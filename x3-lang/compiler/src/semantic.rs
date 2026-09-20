//! Semantic verifier for X3 programs (target C in the production contract).
//!
//! Runs over the X3IR produced by [`crate::lowering::lower_program`] and
//! catches conditions that the bytecode verifier alone cannot catch:
//!
//! - **Symbols**: every chain, asset, via, and dex symbol is non-empty and
//!   contains only safe characters (it is a portable identifier).
//! - **Assets**: lock/mint/burn/release/swap/bridge carry matching
//!   chain/asset values; numeric amounts are non-zero for moves, zero for
//!   release.
//! - **VM routes**: bridge operations target a known chain family
//!   (Ethereum, Solana, X3, BTC/UTXO) and the source/target chains are
//!   distinct for any value-move.
//! - **Adapter compatibility**: a bridge `via` is one of the supported
//!   adapter names — refuses to silently route through an unknown bridge
//!   without an explicit allow-list.
//! - **Atomic rollback**: every cross-VM value move is inside an
//!   `AtomicBegin`/`AtomicEnd` pair, and every `AtomicBegin` is closed by
//!   a matching `AtomicEnd`.
//! - **Replay protection**: an `OnTimeout` policy is present when an
//!   external (cross-VM) call is present.
//! - **Adapter safety**: a `Ref` style asset operation with a non-`X3`
//!   source chain and an unknown target chain is rejected — the adapter
//!   surface for unknown targets must be feature-gated and explicit.
//! - **Route depth**: a single atomic block is bounded to a maximum
//!   number of cross-VM operations (default: 8).
//!
//! B-52 feature lock additions:
//! - **Compilation mode**: Dev / Testnet / Mainnet gating
//! - **Refund path**: every cross-chain operation must have a refund path
//! - **Finality explicit**: every cross-chain op must declare finality reqs
//! - **Proof requirements**: lock/fill/claim proofs required
//! - **Invariant analysis**: built-in invariant rules checked statically
//! - **Route scoring**: weights must sum to 100
//! - **Mainnet safety**: rejects single-RPC, single-relayer, unbounded
//!   deadlines, unsafe slippage, unknown assets, etc.
//! - **Risk scoring**: computes risk score 0-100 with component breakdown
//!
//! Diagnostics accumulate via [`ErrorAccumulator`] so a single `check`
//! call reports every problem rather than failing on the first one.

use crate::diagnostic::{CompilerDiagnostic, DiagnosticSeverity};
use crate::ir::{Condition, FailureAction, Operation, ReleaseAct, X3IR};
use std::collections::{HashMap, HashSet};
use x3_lang_ast::ast::{AtomicSwapDecl, Expression, Item, LiteralExpr, Program};
use x3_lang_common::{Bps, ErrorAccumulator, Span, Spanned, X3Error};

/// Maximum number of cross-VM operations allowed in a single atomic block.
/// This is a hard production safety limit: 8 is the contract default.
pub const DEFAULT_MAX_ATOMIC_OPS: u32 = 8;

/// Maximum number of hops (bridge operations) allowed in a single route.
pub const DEFAULT_MAX_ROUTE_HOPS: u32 = 4;

/// Maximum number of paths an `atomic_choice` may declare.
///
/// "Bounded branch execution" is the construct's whole purpose, so the bound is
/// a named production limit the verifier enforces rather than whatever the
/// parser happens to accept.
pub const MAX_ATOMIC_CHOICE_PATHS: u32 = 8;

/// Hard-coded allow-list of chains the production adapters know about.
/// Adding a new chain here is an explicit, auditable action.
pub const KNOWN_CHAINS: &[&str] = &[
    "eth",
    "ethereum",
    "sol",
    "solana",
    "x3",
    "btc",
    "bitcoin",
    "utxo",
    "polygon",
    "arbitrum",
    "optimism",
    "base",
    "bsc",
    "avalanche",
];

/// Hard-coded allow-list of bridge adapter names. Anything else must be
/// added explicitly via the feature gate (target F in the production
/// contract).
pub const KNOWN_BRIDGE_ADAPTERS: &[&str] = &["x3", "wormhole", "layerzero", "axelar", "native", "btc-relay"];

/// Compilation mode that gates which safety checks are enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationMode {
    Dev,
    Testnet,
    Mainnet,
}

/// A built-in or user-defined invariant rule with a static check function.
pub struct InvariantRule {
    pub name: String,
    pub description: String,
    pub check_fn: fn(&X3IR) -> Result<(), String>,
}

/// Structure capturing a risk score assessment (0-100, lower = safer).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RiskScore {
    pub total: u32,
    pub chain_risk: u32,
    pub bridge_risk: u32,
    pub solver_risk: u32,
    pub relayer_risk: u32,
    pub rpc_risk: u32,
    pub liquidity_risk: u32,
    pub finality_risk: u32,
    pub mev_risk: u32,
    pub timeout_risk: u32,
    pub refund_risk: u32,
}

/// Run the semantic verifier on an X3IR program.
///
/// `max_atomic_ops` and `max_route_hops` are knobs for tests; production
/// callers should accept the defaults via [`verify_with_defaults`].
pub fn verify(ir: &X3IR) -> Result<(), Vec<X3Error>> {
    verify_with_config(ir, DEFAULT_MAX_ATOMIC_OPS, DEFAULT_MAX_ROUTE_HOPS, None)
}

/// Verify with default safety budgets.
pub fn verify_with_defaults(ir: &X3IR) -> Result<(), Vec<X3Error>> {
    verify(ir)
}

/// Everything the semantic verifier found, with warnings kept separate from
/// errors instead of being discarded.
///
/// This exists because `verify_with_config` returns `Ok(())` whenever no
/// *error* was accumulated, so a warning-based check was collected and then
/// thrown away. That is how a bridge with no source-finality requirement, a
/// program violating a builtin invariant, and a bridging program with no proof
/// declaration each compiled silently until the check was promoted to an error.
/// Callers that only care about pass/fail keep using `verify_with_config`;
/// tooling should use `verify_collect` so warnings are visible.
#[derive(Debug, Clone, Default)]
pub struct VerifyOutcome {
    pub errors: Vec<X3Error>,
    pub warnings: Vec<X3Error>,
}

impl VerifyOutcome {
    /// True when nothing rejected the program. Warnings do not fail a program.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// File a coded diagnostic in the vector its own severity names.
    ///
    /// The two vectors are this accumulator's severity channel — `X3Error` carries no level — so
    /// "which vector" is the severity decision, and a caller that picks it by hand has decided the
    /// same thing twice: once when it built the diagnostic and once when it chose the vector. This
    /// is the single place that choice is made, and it is made from the diagnostic's own field.
    /// Before it, a warning-severity diagnostic could not be built at all, so nothing suffered; the
    /// moment one could, the hand-chosen vector was the drift severity-as-a-field exists to prevent
    /// (TICKET-104).
    ///
    /// Note that the *ticket* that asked for this said the accumulator "already accepts"
    /// `x3-common`'s `Diagnostic`. It does not: both vectors are `Vec<X3Error>`. That is why this
    /// method exists in the shape it does — the severity is carried by the channel and the faithful
    /// conversion (`Diagnostic::from`) is what a consumer with a richer type takes.
    pub fn push_diagnostic(&mut self, diagnostic: CompilerDiagnostic) {
        if diagnostic.severity == DiagnosticSeverity::Error {
            self.errors.push(diagnostic.into_error());
        } else {
            self.warnings.push(diagnostic.into_warning());
        }
    }

    pub fn into_result(self) -> Result<(), Vec<X3Error>> {
        if self.errors.is_empty() {
            Ok(())
        } else {
            Err(self.errors)
        }
    }
}

/// Run every semantic safety pass and return errors *and* warnings.
pub fn verify_collect(
    ir: &X3IR,
    max_atomic_ops: u32,
    max_route_hops: u32,
    mode: Option<CompilationMode>,
) -> VerifyOutcome {
    let mut acc = ErrorAccumulator::new();
    let invariants = get_builtin_invariants();
    let context = SemanticPassContext {
        max_atomic_ops,
        max_route_hops,
        mode,
        invariants: &invariants,
    };

    for (_, pass) in SEMANTIC_PASSES {
        run_semantic_pass(*pass, ir, &mut acc, &context);
    }

    VerifyOutcome {
        errors: acc.errors().to_vec(),
        warnings: acc.warnings().to_vec(),
    }
}

/// Everything a semantic pass may need beyond the IR itself.
struct SemanticPassContext<'a> {
    max_atomic_ops: u32,
    max_route_hops: u32,
    mode: Option<CompilationMode>,
    invariants: &'a [InvariantRule],
}

/// A step in the semantic pipeline.
///
/// An enum rather than a function pointer because the passes do not share a
/// signature — two take budgets, one takes the invariant rule set, one is gated
/// on the compilation mode — and a table of `fn` pointers would need an adapter
/// per pass anyway. The enum makes the table and the dispatch mutually
/// exhaustive: the compiler will not let a variant exist without an arm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SemanticPass {
    Symbols,
    RouteDepths,
    AtomicBalance,
    RollbackPresence,
    ReplayAndExpiry,
    BridgeAdapterAllowlist,
    AdapterCompatibility,
    AssetMoves,
    RefundPathExists,
    FinalityExplicit,
    SlippageExplicit,
    ProofRequirements,
    RouteScore,
    /// Decided against the program's computed risk score (`compute_risk_score`).
    RiskScore,
    Invariants,
    MainnetSafe,
}

/// Every whole-program semantic pass, in the order it runs.
///
/// This is data rather than a sequence of calls so that "which passes exist" and
/// "which passes run" are the same list. A pass that is written and never added
/// here runs against nothing while looking wired, which is the failure this
/// module keeps producing; the test `every_semantic_pass_is_in_the_registry`
/// compares the table against the `fn verify_*` definitions in this file.
const SEMANTIC_PASSES: &[(&str, SemanticPass)] = &[
    ("verify_symbols", SemanticPass::Symbols),
    ("verify_route_depths", SemanticPass::RouteDepths),
    ("verify_atomic_balance", SemanticPass::AtomicBalance),
    ("verify_rollback_presence", SemanticPass::RollbackPresence),
    ("verify_replay_and_expiry", SemanticPass::ReplayAndExpiry),
    ("verify_bridge_adapter_allowlist", SemanticPass::BridgeAdapterAllowlist),
    ("verify_adapter_compatibility", SemanticPass::AdapterCompatibility),
    ("verify_asset_moves", SemanticPass::AssetMoves),
    ("verify_refund_path_exists", SemanticPass::RefundPathExists),
    ("verify_finality_explicit", SemanticPass::FinalityExplicit),
    ("verify_slippage_explicit", SemanticPass::SlippageExplicit),
    ("verify_proof_requirements", SemanticPass::ProofRequirements),
    ("verify_route_score", SemanticPass::RouteScore),
    ("verify_risk_score_guards", SemanticPass::RiskScore),
    ("verify_invariants_structured", SemanticPass::Invariants),
    ("verify_mainnet_safe", SemanticPass::MainnetSafe),
];

fn run_semantic_pass(pass: SemanticPass, ir: &X3IR, acc: &mut ErrorAccumulator, context: &SemanticPassContext<'_>) {
    match pass {
        SemanticPass::Symbols => verify_symbols(ir, acc),
        SemanticPass::RouteDepths => verify_route_depths(ir, acc, context.max_atomic_ops, context.max_route_hops),
        SemanticPass::AtomicBalance => verify_atomic_balance(ir, acc),
        SemanticPass::RollbackPresence => verify_rollback_presence(ir, acc),
        SemanticPass::ReplayAndExpiry => verify_replay_and_expiry(ir, acc),
        SemanticPass::BridgeAdapterAllowlist => verify_bridge_adapter_allowlist(ir, acc),
        SemanticPass::AdapterCompatibility => verify_adapter_compatibility(ir, acc),
        SemanticPass::AssetMoves => verify_asset_moves(ir, acc),
        SemanticPass::RefundPathExists => verify_refund_path_exists(ir, acc),
        SemanticPass::FinalityExplicit => verify_finality_explicit(ir, acc),
        SemanticPass::SlippageExplicit => verify_slippage_explicit(ir, acc),
        SemanticPass::ProofRequirements => verify_proof_requirements(ir, context.mode, acc),
        SemanticPass::RouteScore => verify_route_score(ir, acc),
        SemanticPass::RiskScore => verify_risk_score_guards(ir, acc),
        SemanticPass::Invariants => verify_invariants_structured(ir, context.invariants, acc),
        SemanticPass::MainnetSafe => {
            // Two reasons to run the checks: the build is for mainnet, or the
            // program asks for them with `require mainnet_safe`. The second is
            // what makes that guard a claim rather than a comment — a guard that
            // was only honoured under `--mode mainnet` could never be written by a
            // program under development, and one that was recorded without the
            // checks running would assert something nothing established.
            if context.mode == Some(CompilationMode::Mainnet) || claims_mainnet_safe(ir) {
                verify_mainnet_safe(ir, acc);
            }
        }
    }
}

/// Verify with explicit budgets.
///
/// `mode` optionally gates mainnet-specific safety checks.
pub fn verify_with_config(
    ir: &X3IR,
    max_atomic_ops: u32,
    max_route_hops: u32,
    mode: Option<CompilationMode>,
) -> Result<(), Vec<X3Error>> {
    verify_collect(ir, max_atomic_ops, max_route_hops, mode).into_result()
}

fn span() -> Span {
    Span::DUMMY
}

fn err(message: impl Into<String>) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: span(),
    }
}

/// A `require invariant <name>` guard needs that invariant to be declared.
///
/// An invariant is checked because the program *declares* it — `invariant
/// <name>` is the rule the invariants pass reads — so a guard naming one the
/// program never declares asks for a check nothing provides. The same shape as
/// `proof_complete` against `proofs required`, and decidable for the same reason:
/// the claim is about the artifact's own configuration.
pub fn verify_invariant_guards_declared(program: &Program, acc: &mut ErrorAccumulator) {
    let declared: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::InvariantDecl(invariant) => Some(invariant.name.as_str()),
            _ => None,
        })
        .collect();

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::InvariantCheck {
            continue;
        }
        let Some(named) = guard.subject.as_ref() else {
            acc.add_error(err(format!(
                "declaration '{owner}' requires `invariant` without naming it; there is nothing to have checked — write `require invariant <name>`"
            )));
            continue;
        };
        if !declared.iter().any(|name| *name == named.as_str()) {
            acc.add_error(err(if declared.is_empty() {
                format!(
                    "declaration '{owner}' requires the invariant '{}', and the program declares no `invariant` at all — a guard names a rule the artifact has to state",
                    named.as_str()
                )
            } else {
                format!(
                    "declaration '{owner}' requires the invariant '{}', and the program declares \
                     {{{}}} — an invariant it never declares is one nothing checks",
                    named.as_str(),
                    declared.join(", ")
                )
            }));
        }
    }
}

/// A `require bridge_liquidity >= N` guard needs bridges that declare that depth.
///
/// The guard asserts the bridges this program uses can absorb N. A `venue { kind
/// bridge … liquidity … }` declaration is where a program states a bridge's depth,
/// so the claim is decidable against it: every declared bridge must have at least
/// the required liquidity, and a program that declares no bridge at all has
/// nothing the guard could be about — the same shape as `solver_bond` against
/// `solver_market` and `relayer_quorum` against `relayers`.
///
/// Every declared bridge rather than any one of them: a route may take whichever
/// the planner finds, so a depth requirement that held for one and not another
/// would be a claim the program cannot keep.
pub fn verify_bridge_liquidity_declared(program: &Program, acc: &mut ErrorAccumulator) {
    let declared: Vec<(&str, u128)> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::VenueDecl(venue) if venue.kind == x3_lang_ast::ast::VenueKind::Bridge => {
                Some((venue.name.as_str(), venue.liquidity))
            }
            _ => None,
        })
        .collect();

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::BridgeLiquidity {
            continue;
        }
        // A floor, like the other liquidity claims: written as a ceiling it would
        // say the bridges may not be deep.
        if !guard.comparison.is_some_and(|op| op.is_lower_bound()) {
            acc.add_error(err(format!(
                "declaration '{owner}' states `require bridge_liquidity` without a `>=` bound; a \
                 liquidity guard is a floor, and the check reads it as one"
            )));
            continue;
        }
        let Some(required) = guard.value.as_ref().and_then(extract_int_from_expr) else {
            acc.add_error(err(format!(
                "declaration '{owner}' requires bridge liquidity of a value the compiler cannot read \
                 as a number; the check compares it against what a venue declares"
            )));
            continue;
        };
        if declared.is_empty() {
            acc.add_error(err(format!(
                "declaration '{owner}' requires {required} of bridge liquidity, and the program \
                 declares no `venue {{ kind bridge … }}` — the guard has nothing to compare against"
            )));
            continue;
        }
        for (venue, liquidity) in &declared {
            if *liquidity < required {
                acc.add_error(err(format!(
                    "declaration '{owner}' requires {required} of bridge liquidity, and the bridge \
                     '{venue}' declares {liquidity}"
                )));
            }
        }
    }
}

/// A `require vm_supported <vm>` guard needs a declaration that uses that VM.
///
/// The guard asserts the artifact runs on a VM family; a program says which
/// families it uses in its `vm`, `target` and `venue` declarations. So the guard
/// is a claim the *compiler* can decide — TICKET-027's other branch ("or is
/// evaluated by the compiler") — rather than an assertion the executor treats as
/// true.
///
/// Compared through the family map the parser already uses for chain prefixes, so
/// a guard's `solana` matches a declared `svm` and a declared `sol` matches either:
/// the families are the language's closed set, and comparing the spelling would
/// refuse a correct program over a word.
pub fn verify_vm_supported_declared(program: &Program, acc: &mut ErrorAccumulator) {
    let mut declared: Vec<String> = Vec::new();
    for item in &program.items {
        match &item.node {
            Item::VmDecl(vm) => declared.push(vm.adapter.as_str().to_string()),
            Item::VmTarget(target) => {
                declared.push(target.vm.as_str().to_string());
                declared.push(target.adapter.as_str().to_string());
            }
            Item::VenueDecl(venue) => declared.push(venue.domain.as_str().to_string()),
            _ => {}
        }
    }

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::VmSupported {
            continue;
        }
        let Some(named) = guard.subject.as_ref() else {
            acc.add_error(err(format!(
                "declaration '{owner}' requires `vm_supported` without naming the VM; every program \
                 runs on some VM, so the guard says nothing — write `require vm_supported <vm>`"
            )));
            continue;
        };
        let wanted = vm_family(named.as_str());
        if !declared.iter().any(|declaration| vm_family(declaration) == wanted) {
            acc.add_error(err(if declared.is_empty() {
                format!(
                    "declaration '{owner}' requires the VM '{}' to be supported, and the program \
                     declares no `vm`, `target` or `venue` at all — the guard has nothing to be \
                     backed by",
                    named.as_str()
                )
            } else {
                format!(
                    "declaration '{owner}' requires the VM '{}' to be supported, and the program \
                     declares {{{}}} — a VM no declaration uses is one no adapter here provides",
                    named.as_str(),
                    declared.join(", ")
                )
            }));
        }
    }
}

/// The family a VM or chain name belongs to, or the name itself.
fn vm_family(name: &str) -> String {
    crate::parser::parse_vm_family(name)
        .map(str::to_string)
        .unwrap_or_else(|| name.to_ascii_lowercase())
}

/// A `risk_policy { max_slippage M }` is a ceiling the guards must respect.
///
/// The policy says "no route of mine slips more than M percent"; a guard says
/// "this operation must slip no more than N percent". A guard looser than the
/// policy therefore permits what the policy forbids, and the program has said two
/// different things about the same quantity. Compared in one unit, which is the
/// whole point of the field: `max_slippage` and the guard's literal are both
/// percentages (the linter normalises a guard literal to basis points with the
/// same reading).
///
/// A policy of zero is "unstated": the field is not optional in the AST, so zero
/// is the absence, and a program with no policy cannot contradict one.
pub fn verify_risk_policy_bounds_guards(program: &Program, acc: &mut ErrorAccumulator) {
    let max_slippage = program.items.iter().find_map(|item| match &item.node {
        Item::RiskPolicy(policy) if policy.max_slippage > 0 => Some(policy.max_slippage),
        _ => None,
    });
    let Some(policy) = max_slippage else {
        return;
    };

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::Slippage {
            continue;
        }
        // A ceiling is the claim the policy makes. A guard written with a floor
        // is a different claim, and the existing strategy check already refuses
        // it where it matters; comparing it here would compare two directions.
        if !guard.comparison.is_some_and(|op| op.is_upper_bound()) {
            continue;
        }
        let Some(bound) = guard.value.as_ref().and_then(bound_bps_from_expr) else {
            continue;
        };
        // Both sides in basis points: the policy's field is a bare number in the
        // same unit as a guard's (TICKET-054).
        if bound > u32::try_from(policy).unwrap_or(u32::MAX) {
            acc.add_error(err(format!(
                "declaration '{owner}' permits a slippage of {bound} while the risk policy accepts \
                 at most {policy}; the guard allows what the policy forbids"
            )));
        }
    }
}

/// A `require route_score >= N` guard needs a score to compare against.
///
/// The guard is a claim about the route; `risk_policy { min_route_score M }` is
/// where a program states the score it accepts. So a guard demanding more than
/// the declared score claims something the configuration never does, and a guard
/// with no policy at all claims something nothing backs — the same shape as the
/// solver bond and the relayer quorum. Six corpus programs required
/// `route_score >= 85..90` with no declaration anywhere, which is what closed this
/// as a hole rather than a formality.
pub fn verify_route_score_declared(program: &Program, acc: &mut ErrorAccumulator) {
    let declared = program.items.iter().find_map(|item| match &item.node {
        Item::RiskPolicy(policy) => policy.min_route_score,
        _ => None,
    });

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::RouteScore {
            continue;
        }
        // `route_score >= N` is a floor. Written as a ceiling it says the route
        // may not score *well*, and comparing that against a declared minimum
        // would answer a question nobody asked.
        if !guard.comparison.is_some_and(|op| op.is_lower_bound()) {
            acc.add_error(err(format!(
                "declaration '{owner}' states `require route_score` without a `>=` bound; a route \
                 score guard is a floor, and the check reads it as one"
            )));
            continue;
        }
        let required = guard.value.as_ref().and_then(extract_int_from_expr).unwrap_or(0);
        match declared {
            None => acc.add_error(err(format!(
                "declaration '{owner}' requires a route score of {required} but the program declares \
                 no `risk_policy {{ min_route_score <n> }}` — the guard has nothing to compare \
                 against"
            ))),
            Some(declared) if required > u128::from(declared) => acc.add_error(err(format!(
                "declaration '{owner}' requires a route score of {required}, but the declared \
                 minimum is {declared}"
            ))),
            Some(_) => {}
        }
    }
}

/// A `require finality.<chain> >= N` guard needs a declared depth for that chain.
///
/// The guard is a claim about settlement: "this program will not treat a fill as
/// final until `<chain>` is at least N blocks deep". `finality_policy { chain
/// <chain> blocks N }` is the depth the program requires of that chain, so a
/// guard *below* it is refused — it would pass at a depth the program's own
/// policy says is not final, which is the blur between confirmation depths this
/// pass exists to prevent — while a guard above it is strictly more conservative
/// and allowed. A guard naming a chain no policy declares is refused too:
/// nothing backs it, and it lowered to a `REQUIRE` the executor treats as true,
/// which is the defect that made every corpus program with a depth guard
/// unchecked.
///
/// The mode form (`require finality.sol == finalized`) is decided the same way
/// against the declaration's `requirement` word. Chain names compare
/// case-insensitively, because eight of the corpus's twelve depth guards spell
/// their chain differently from the way the declaration does (`finality
/// Ethereum`) and a chain's name is not an identifier whose case is part of its
/// meaning.
pub fn verify_finality_guards_declared(program: &Program, acc: &mut ErrorAccumulator) {
    let policies: Vec<&x3_lang_ast::ast::FinalityPolicy> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::FinalityPolicy(policy) => Some(policy),
            _ => None,
        })
        .collect();

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::Finality {
            continue;
        }
        // `require source_finality` / `dest_finality` name a side of the program
        // rather than a chain, and no declaration states what such a side must
        // reach; they are decided by the program's own finality pass.
        let Some(chain) = guard.subject.as_ref() else {
            continue;
        };
        let matching: Vec<&&x3_lang_ast::ast::FinalityPolicy> = policies
            .iter()
            .filter(|policy| policy.chain.as_str().eq_ignore_ascii_case(chain.as_str()))
            .collect();
        if matching.is_empty() {
            acc.add_error(err(format!(
                "declaration '{owner}' requires `finality.{}`, but no `finality_policy` names that \
                 chain — the guard has nothing to compare against. Write `finality_policy <mode> {{ \
                 chain {} requirement finalized blocks <n> }}`",
                chain.as_str(),
                chain.as_str()
            )));
            continue;
        }
        if matching.len() > 1 {
            acc.add_error(err(format!(
                "{} `finality_policy` declarations name chain '{}' while '{owner}' guards it; one \
                 chain has one policy, and a guard read against the first of two depths is read \
                 against whichever happened to be written first",
                matching.len(),
                chain.as_str()
            )));
            continue;
        }
        let policy = matching[0];

        let Some(value) = guard.value.as_ref() else {
            acc.add_error(err(format!(
                "declaration '{owner}' states `require finality.{}` with no depth and no mode; write \
                 a floor (`require finality.{} >= 32`) or a mode (`require finality.{} == \
                 finalized`)",
                chain.as_str(),
                chain.as_str(),
                chain.as_str()
            )));
            continue;
        };

        if let Some(required) = extract_int_from_expr(value) {
            // A depth is a floor — "at least N blocks deep". A ceiling says the
            // chain may not be *well* settled, and comparing that against a
            // declared depth answers a question nobody asked.
            if !guard.comparison.is_some_and(|op| op.is_lower_bound()) {
                acc.add_error(err(format!(
                    "declaration '{owner}' states `require finality.{}` without a `>=` bound; a \
                     finality depth is a floor — write `require finality.{} >= {required}`",
                    chain.as_str(),
                    chain.as_str()
                )));
                continue;
            }
            match policy.blocks {
                None => acc.add_error(err(format!(
                    "declaration '{owner}' requires {required} blocks of finality on '{}', but the \
                     `finality_policy` for that chain states no `blocks`; add `blocks {required}` \
                     (or more) to the policy",
                    chain.as_str()
                ))),
                Some(declared) if required < u128::from(declared) => acc.add_error(err(format!(
                    "declaration '{owner}' requires only {required} blocks of finality on '{}', while \
                     the policy declared for that chain requires {declared}: the guard would pass at \
                     a depth the program itself says is not final. Write `require finality.{} >= \
                     {declared}`, or lower the policy if {required} is what the program means",
                    chain.as_str(),
                    chain.as_str()
                ))),
                Some(_) => {}
            }
            continue;
        }

        let Some(mode) = guard_word(value) else {
            acc.add_error(err(format!(
                "declaration '{owner}' compares `finality.{}` against something that is neither a \
                 depth nor a mode; write `require finality.{} >= <blocks>` or `require finality.{} \
                 == <mode>`",
                chain.as_str(),
                chain.as_str(),
                chain.as_str()
            )));
            continue;
        };
        if !policy.requirement.as_str().eq_ignore_ascii_case(mode) {
            acc.add_error(err(format!(
                "declaration '{owner}' requires finality mode '{mode}' on '{}', but the declared \
                 policy for that chain requires '{}'",
                chain.as_str(),
                policy.requirement.as_str()
            )));
        }
    }
}

/// The name a guard's right-hand side writes, when it is a bare identifier.
///
/// `require finality.sol == finalized` compares a chain against a *word*, and the
/// parser stores that word as an identifier expression. Returning `None` for
/// anything else keeps the caller from reading a compound expression as a name.
fn guard_word(expr: &Expression) -> Option<&str> {
    match expr {
        Expression::Ident(name) => Some(name.as_str()),
        _ => None,
    }
}

/// Refuse a guard kind the compiler cannot decide.
///
/// Two kinds of guard are refused here, and the set is closed by name rather than
/// by accident:
///
/// - **an unknown word.** `require_kind_from_str` carries it as `Custom`, which is
///   how `require proof verified` parses — a condition nothing can check: no
///   declaration to compare against, no run-time quantity, no verifier reading it.
///   The language's rule everywhere else is that a construct the compiler does not
///   understand is one it cannot check ("permissions" are a closed set for exactly
///   this reason), so the word has to be one of `REQUIRE_KIND_NAMES` and the
///   diagnostic lists them.
/// - **`audit_gate`.** A known *name* behind which there is nothing: an audit is
///   evidence about the delivery process, not a property of the artifact, so no
///   clause in a program can state one and no pass can read one. Recording the
///   guard would make the artifact assert a condition that is true because nothing
///   looked, which is the defect this whole family of checks exists to remove.
///
/// Every other name in `REQUIRE_KIND_NAMES` is decided by a pass — against a
/// declaration the program writes, against the program's own operations, or, for
/// `nonce`, at run time against the VM's nonce registry.
pub fn verify_guard_kinds_are_checkable(program: &Program, acc: &mut ErrorAccumulator) {
    for (owner, guard) in require_guards(program) {
        match &guard.kind {
            x3_lang_ast::ast::RequireKind::Custom(_) => acc.add_error(err(format!(
                "declaration '{owner}' requires `{}`, which is not a guard kind this compiler knows, \
                 so nothing would ever check it. The kinds it knows are: {}",
                guard.kind.as_str(),
                crate::parser::REQUIRE_KIND_NAMES.join(", ")
            ))),
            x3_lang_ast::ast::RequireKind::AuditGate => acc.add_error(err(format!(
                "declaration '{owner}' requires `audit_gate`, which this language cannot back: no \
                 clause in a program declares that an audit ran, so the guard would be recorded as a \
                 condition that is true because nothing looked. An audit is evidence about the \
                 delivery process rather than a property of the artifact — keep it where it can be \
                 verified, and remove the guard"
            ))),
            _ => {}
        }
    }
}

/// A `require proof_complete <name>` guard needs that proof to be declared.
///
/// The guard asserts a proof of the named type was completed;
/// `proofs required { … }` is where a program declares which proofs its
/// operations need. So a guard naming a proof the program never declares claims
/// something no configuration backs — the same shape as the solver bond and the
/// relayer quorum, and the same reason it is a compile-time check rather than a
/// `STATIC` assertion.
pub fn verify_proof_complete_declared(program: &Program, acc: &mut ErrorAccumulator) {
    let declared: Vec<&str> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::ProofsRequired(proofs) => Some(proofs.proofs.iter().map(|proof| proof.as_str())),
            _ => None,
        })
        .flatten()
        .collect();

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::ProofComplete {
            continue;
        }
        let Some(proof) = guard.subject.as_ref() else {
            acc.add_error(err(format!(
                "declaration '{owner}' requires `proof_complete` without naming the proof; there is \
                 nothing to have completed — write `require proof_complete <proof_type>`"
            )));
            continue;
        };
        if !declared.iter().any(|name| *name == proof.as_str()) {
            acc.add_error(err(if declared.is_empty() {
                format!(
                    "declaration '{owner}' requires the proof '{}' to be complete, but the program \
                     declares no `proofs required {{ … }}` at all — the guard has nothing to be \
                     backed by",
                    proof.as_str()
                )
            } else {
                format!(
                    "declaration '{owner}' requires the proof '{}' to be complete, and the program \
                     declares {{{}}} — a proof it never declares is one no operation can carry",
                    proof.as_str(),
                    declared.join(", ")
                )
            }));
        }
    }
}

/// Evaluate `require canonical_supply <ASSET>` against what the program does.
///
/// The guard claims the canonical supply of an asset is preserved, and that is a
/// claim about this program's *own* operations, so the compiler can decide it
/// rather than record an assertion nothing evaluates. The IR is the source of
/// truth for what the program does: it is flat, so a mint inside a nested
/// `atomic` block or a choice path is the same operation as one at the top.
///
/// Preserved means *net* zero: a program that mints and burns the same amount of
/// an asset has not changed its supply, and one that does either alone has. The
/// guard is a statement about the artifact, so a program whose own operations
/// contradict it is rejected — which is the compile-time answer for a guard kind
/// that has no run-time quantity to compare against (TICKET-027).
pub fn verify_canonical_supply(program: &Program, ir: &X3IR) -> Vec<X3Error> {
    let mut errors = Vec::new();
    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::CanonicalSupply {
            continue;
        }
        let Some(asset) = guard.subject.as_ref() else {
            errors.push(err(format!(
                "declaration '{owner}' requires `canonical_supply` without naming the asset; there \
                 is nothing to hold the supply of — write `require canonical_supply USDC`"
            )));
            continue;
        };
        let wanted = asset.as_str();
        let (minted, burned) = supply_totals(ir, wanted);
        if minted != burned {
            errors.push(err(format!(
                "declaration '{owner}' requires the canonical supply of {wanted} to be preserved, \
                 but the program mints {minted} and burns {burned} of it; the guard is a claim the \
                 program's own operations contradict"
            )));
        }
    }
    errors
}

/// What a program adds to and removes from circulation for one asset.
fn supply_totals(ir: &X3IR, asset: &str) -> (u128, u128) {
    let mut minted = 0u128;
    let mut burned = 0u128;
    for op in &ir.operations {
        match op {
            Operation::Mint {
                asset: op_asset,
                amount,
                ..
            } if same_asset(op_asset, asset) => minted = minted.saturating_add(*amount),
            Operation::Burn {
                asset: op_asset,
                amount,
                ..
            } if same_asset(op_asset, asset) => burned = burned.saturating_add(*amount),
            _ => {}
        }
    }
    (minted, burned)
}

/// Whether an operation's asset is the asset a guard named.
///
/// A guard writes the asset's own name (`USDC`), while an operation carries what
/// its statement wrote, which may be qualified (`ethereum.USDC`). Compared on the
/// last segment, case-insensitively, which is the same rule the asset-move checks
/// use.
fn same_asset(operation_asset: &str, guard_asset: &str) -> bool {
    let tail = |text: &str| text.rsplit('.').next().unwrap_or(text).to_ascii_uppercase();
    tail(operation_asset) == tail(guard_asset)
}

fn verify_symbols(ir: &X3IR, acc: &mut ErrorAccumulator) {
    for op in &ir.operations {
        match op {
            Operation::Lock { chain, asset, from, .. } => {
                check_safe_symbol("chain", chain, acc);
                check_safe_symbol("asset", asset, acc);
                check_safe_symbol("from", from, acc);
            }
            Operation::Mint { chain, asset, to, .. } => {
                check_safe_symbol("chain", chain, acc);
                check_safe_symbol("asset", asset, acc);
                check_safe_symbol("to", to, acc);
            }
            Operation::Burn { chain, asset, from, .. } => {
                check_safe_symbol("chain", chain, acc);
                check_safe_symbol("asset", asset, acc);
                check_safe_symbol("from", from, acc);
            }
            Operation::Release { chain, asset, to, .. } => {
                check_safe_symbol("chain", chain, acc);
                check_safe_symbol("asset", asset, acc);
                check_safe_symbol("to", to, acc);
            }
            Operation::Swap {
                from_chain,
                from_asset,
                to_asset,
                dex,
                ..
            } => {
                check_safe_symbol("from_chain", from_chain, acc);
                check_safe_symbol("from_asset", from_asset, acc);
                check_safe_symbol("to_asset", to_asset, acc);
                if let Some(d) = dex {
                    check_safe_symbol("dex", d, acc);
                }
            }
            Operation::Bridge {
                via,
                from_chain,
                from_asset,
                to_chain,
                to_asset,
                receiver,
                ..
            } => {
                check_safe_symbol("via", via, acc);
                check_safe_symbol("from_chain", from_chain, acc);
                check_safe_symbol("from_asset", from_asset, acc);
                check_safe_symbol("to_chain", to_chain, acc);
                check_safe_symbol("to_asset", to_asset, acc);
                check_safe_symbol("receiver", receiver, acc);
            }
            _ => {}
        }
    }
}

fn check_safe_symbol(field: &str, value: &str, acc: &mut ErrorAccumulator) {
    if value.is_empty() {
        acc.add_error(err(format!("{field} is empty")));
        return;
    }
    // `.` is allowed because a receiver is written as a dotted path —
    // `receiver sol.wallet.owner` — and that is the form the language's own
    // examples use. A bare allowlist extension would also have admitted `.`,
    // `..` and `a..b`, so the dot is allowed only *between* non-empty segments
    // and a `..` segment is refused: a `.`-bearing identifier that later gets
    // treated as a path is a traversal, and this check is the only thing
    // standing between the program text and whatever consumes it.
    let unsafe_char = value
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.');
    let unsafe_segment = value.split('.').any(|segment| segment.is_empty() || segment == "..");
    if unsafe_char || unsafe_segment {
        acc.add_error(err(format!(
            "{field}={value:?} contains unsafe characters (allowed: alnum, _, -, and single dots \
             between non-empty segments)"
        )));
    }
    if value.len() > 64 {
        acc.add_error(err(format!("{field}={value:?} exceeds 64-character safety limit")));
    }
}

fn verify_route_depths(ir: &X3IR, acc: &mut ErrorAccumulator, max_atomic_ops: u32, max_route_hops: u32) {
    // Reject nested atomic blocks first.
    let mut atomic_depth: u32 = 0;
    for op in &ir.operations {
        if matches!(op, Operation::AtomicBegin) {
            atomic_depth += 1;
        }
        if matches!(op, Operation::AtomicEnd) {
            atomic_depth = atomic_depth.saturating_sub(1);
        }
        if atomic_depth > 1 {
            acc.add_error(err("nested atomic blocks are not allowed"));
        }
    }

    // Validate op count and cross-VM hop count per block.
    let mut current_block: u32 = 0;
    let mut current_hops: u32 = 0;
    let mut inside_atomic = false;
    for op in &ir.operations {
        if matches!(op, Operation::AtomicBegin) {
            inside_atomic = true;
            current_block = 0;
            current_hops = 0;
        } else if matches!(op, Operation::AtomicEnd) {
            if inside_atomic {
                if current_block > max_atomic_ops {
                    acc.add_error(err(format!(
                        "atomic block has {current_block} operations (max {max_atomic_ops})"
                    )));
                }
                if current_hops > max_route_hops {
                    acc.add_error(err(format!(
                        "atomic block has {current_hops} cross-VM hops (max {max_route_hops})"
                    )));
                }
            }
            inside_atomic = false;
        } else if inside_atomic {
            current_block += 1;
            if is_cross_vm_op(op) {
                current_hops += 1;
            }
        }
    }
}

fn is_cross_vm_op(op: &Operation) -> bool {
    matches!(op, Operation::Bridge { .. } | Operation::Swap { .. })
}

fn verify_atomic_balance(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let mut depth: i32 = 0;
    for op in ir.operations.iter() {
        if matches!(op, Operation::AtomicBegin) {
            depth += 1;
        }
        if matches!(op, Operation::AtomicEnd) {
            depth -= 1;
            if depth < 0 {
                acc.add_error(err("AtomicEnd without matching AtomicBegin"));
            }
        }
    }
    if depth > 0 {
        acc.add_error(err(format!("{} unmatched AtomicBegin (missing AtomicEnd)", depth)));
    }
}

fn verify_rollback_presence(ir: &X3IR, acc: &mut ErrorAccumulator) {
    // Any cross-VM operation must be inside an atomic block.
    let mut inside_atomic = false;
    for op in &ir.operations {
        if matches!(op, Operation::AtomicBegin) {
            inside_atomic = true;
        }
        if matches!(op, Operation::AtomicEnd) {
            inside_atomic = false;
        }
        if !inside_atomic && is_cross_vm_op(op) {
            acc.add_error(err(format!(
                "cross-VM operation {op:?} is not inside an atomic block — rollback cannot be \
                 guaranteed"
            )));
        }
    }
}

fn verify_replay_and_expiry(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let has_bridge = ir.operations.iter().any(|op| matches!(op, Operation::Bridge { .. }));
    if !has_bridge {
        return;
    }
    let has_timeout = ir
        .operations
        .iter()
        .any(|op| matches!(op, Operation::OnTimeout { duration_blocks, .. } if *duration_blocks > 0));
    if !has_timeout {
        acc.add_error(err(
            "bridge operation present without an OnTimeout policy — expiry/deadline required for \
             replay protection",
        ));
    }
    if ir.metadata.nonce.is_none() {
        acc.add_error(err(
            "bridge operation present without a nonce in program metadata — replay protection \
             required",
        ));
    }
}

fn verify_bridge_adapter_allowlist(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let allow: HashSet<&str> = KNOWN_BRIDGE_ADAPTERS.iter().copied().collect();
    for op in &ir.operations {
        if let Operation::Bridge { via, .. } = op {
            if !allow.contains(via.to_ascii_lowercase().as_str()) {
                acc.add_error(err(format!(
                    "bridge via={via:?} is not in the production adapter allow-list \
                     ({}); add the adapter explicitly or use a known one",
                    KNOWN_BRIDGE_ADAPTERS.join(", ")
                )));
            }
        }
    }
}

fn verify_adapter_compatibility(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let known: HashSet<&str> = KNOWN_CHAINS.iter().copied().collect();
    for op in &ir.operations {
        match op {
            Operation::Bridge {
                from_chain, to_chain, ..
            } => {
                let from_norm = from_chain.to_ascii_lowercase();
                if !known.contains(from_norm.as_str()) {
                    acc.add_error(err(format!(
                        "source chain {from_chain:?} is not a known production chain; refuse to \
                         silently route through an unknown adapter"
                    )));
                }
                if from_chain == to_chain {
                    acc.add_error(err(format!(
                        "bridge from_chain == to_chain ({from_chain:?}); cross-VM bridge must \
                         target a different chain"
                    )));
                }
            }
            Operation::Swap { from_chain, .. } => {
                let from_norm = from_chain.to_ascii_lowercase();
                if !known.contains(from_norm.as_str()) {
                    acc.add_error(err(format!(
                        "swap on unknown chain {from_chain:?}; add the chain to the production \
                         allow-list or use a known chain"
                    )));
                }
            }
            Operation::Lock { chain, .. }
            | Operation::Mint { chain, .. }
            | Operation::Burn { chain, .. }
            | Operation::Release { chain, .. } => {
                if !known.contains(chain.to_ascii_lowercase().as_str()) {
                    acc.add_error(err(format!(
                        "asset operation on unknown chain {chain:?}; add the chain to the \
                         production allow-list or use a known chain"
                    )));
                }
            }
            _ => {}
        }
    }
}

fn verify_asset_moves(ir: &X3IR, acc: &mut ErrorAccumulator) {
    for op in &ir.operations {
        match op {
            Operation::Lock { amount, .. } | Operation::Mint { amount, .. } | Operation::Burn { amount, .. } => {
                if *amount == 0 {
                    acc.add_error(err(format!("asset move operation has zero amount: {op:?}")));
                }
            }
            Operation::Swap { input_amount, .. } => {
                if *input_amount == 0 {
                    acc.add_error(err("swap has zero input_amount"));
                }
            }
            Operation::Bridge { amount, .. } => {
                if *amount == 0 {
                    acc.add_error(err("bridge has zero amount"));
                }
            }
            Operation::Require {
                kind: _,
                subject: _,
                condition,
                error_msg,
                ..
            } => {
                if matches!(condition, Condition::False) {
                    acc.add_error(err(format!(
                        "require is statically false: {}",
                        error_msg.clone().unwrap_or_else(|| "<no message>".into())
                    )));
                }
            }
            Operation::OnTimeout { duration_blocks, .. } => {
                if *duration_blocks == 0 {
                    acc.add_error(err("OnTimeout with zero duration"));
                }
            }
            Operation::OnFail { action } => {
                if matches!(action, FailureAction::Halt) {
                    // Halt is an explicit operator-controlled safety action; we don't
                    // reject it but we do require a timeout in the same program
                    // (handled in verify_replay_and_expiry).
                }
            }
            _ => {}
        }
    }
}

/// Known chain names — mirrors the production allow-list.
/// Defined here (also in [`KNOWN_CHAINS`]) so AST-level validation
/// does not depend on the IR lowering pass.
pub const SWAP_KNOWN_CHAINS: &[&str] = &[
    "eth",
    "ethereum",
    "sol",
    "solana",
    "x3",
    "btc",
    "bitcoin",
    "utxo",
    "polygon",
    "arbitrum",
    "optimism",
    "base",
    "bsc",
    "avalanche",
];

/// Supported hash function names for hashlock in atomic swaps.
pub const SUPPORTED_HASH_FUNCTIONS: &[&str] = &["sha256", "blake2b"];

/// Extract a `u128` integer value from an expression, if it is a literal integer.
pub(crate) fn extract_int_from_expr(expr: &Expression) -> Option<u128> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Some(*value),
        _ => None,
    }
}

/// Every `require` guard in the program, paired with the name of the
/// declaration that owns it.
///
/// Guards do not live in one shape. An `intent` body carries them as
/// statements, while `bridge`, `atomic swap`, `strategy` and `proposal`
/// collect them into a `requires` list. A verifier that walks one shape skips
/// the others silently, so the guard-versus-declaration checks share this
/// enumeration instead of each one re-deriving it.
/// Every `require` guard in a program, with the declaration that owns it.
///
/// `pub(crate)` because PHASE 37's `arb` analysis asks the same question the risk
/// policy check asks — "is this declared bound enforced by a guard, or is it a
/// label nothing acts on?" — and one walk of the guards is what keeps the two
/// answers from drifting apart.
pub(crate) fn require_guards(program: &Program) -> Vec<(&str, &x3_lang_ast::ast::RequireGuard)> {
    let mut guards: Vec<(&str, &x3_lang_ast::ast::RequireGuard)> = Vec::new();
    for item in &program.items {
        match &item.node {
            Item::IntentDecl(intent) => {
                for statement in &intent.body.stmts {
                    if let x3_lang_ast::ast::Statement::Require(guard) = statement {
                        guards.push((intent.name.as_str(), guard));
                    }
                }
            }
            Item::Bridge(decl) => {
                guards.extend(decl.requires.iter().map(|guard| (decl.name.as_str(), guard)));
            }
            Item::AtomicSwap(decl) => {
                guards.extend(decl.requires.iter().map(|guard| (decl.name.as_str(), guard)));
            }
            Item::Strategy(decl) => {
                guards.extend(decl.requires.iter().map(|guard| (decl.name.as_str(), guard)));
            }
            Item::Proposal(decl) => {
                guards.extend(decl.requires.iter().map(|guard| (decl.name.as_str(), guard)));
            }
            _ => {}
        }
    }
    guards
}

/// How many hops a path's body represents, when it is written as statements.
///
/// A `swap` or `bridge` statement is one hop. `None` means the path declares
/// neither a hop chain nor a route statement, so the compiler has no hop count
/// to rank by — which is an error for `choose fewest_hops`, not a zero.
pub(crate) fn path_hop_count(path: &x3_lang_ast::ast::ChoicePath) -> Option<u32> {
    if !path.hops.is_empty() {
        return Some(path.hops.len() as u32);
    }
    let mut hops = 0u32;
    for statement in &path.body {
        if matches!(
            statement,
            x3_lang_ast::ast::Statement::Swap { .. } | x3_lang_ast::ast::Statement::Bridge { .. }
        ) {
            hops += 1;
        }
    }
    if hops == 0 {
        None
    } else {
        Some(hops)
    }
}

/// Verify an `atomic_choice` is a bounded, type-consistent branch set.
///
/// The construct's promise is that the compiler enumerated every permitted
/// branch, checked each one, and that the artifact cannot run a branch it did
/// not verify. Each clause below is one of those promises:
///
/// - **enumerate permitted branches** — at least two, at most
///   [`MAX_ATOMIC_CHOICE_PATHS`], with distinct names. A set of one is not a
///   choice, and an unbounded set is not bounded execution.
/// - **type-check every branch** — each path must have an executable body;
///   lowering and the IR passes then run over the selected path, and the
///   bounds check is what keeps "every branch was checked" from being a claim
///   about branches the parser silently dropped.
/// - **equivalent required output type** — every path must declare
///   `net_output <amount> <ASSET>` and all of them must name the same asset.
///   Without this, "choose the best branch" compares things that are not the
///   same kind of thing.
/// - **prohibit arbitrary runtime code mutation** — the criterion is a closed
///   enum, and the data it ranks must be evaluable at compile time. A path
///   whose `net_output` is not an integer literal cannot be ranked, so it is
///   refused rather than defaulted.
pub fn verify_atomic_choice_decls(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::AtomicChoice(choice) = &item.node else {
            continue;
        };
        let name = choice.name.as_str();

        if choice.paths.len() < 2 {
            acc.add_error(err(format!(
                "atomic_choice '{name}' declares {} path(s); a choice needs at least two branches",
                choice.paths.len()
            )));
        }
        if choice.paths.len() as u32 > MAX_ATOMIC_CHOICE_PATHS {
            acc.add_error(err(format!(
                "atomic_choice '{name}' declares {} paths, above the {MAX_ATOMIC_CHOICE_PATHS}-path \
                 production bound for bounded branch execution",
                choice.paths.len()
            )));
        }

        // `lowest_declared_fee` ranks a venue chain, and an `atomic_choice`'s paths
        // are bodies plus optional asset hops — a hop does not say *which* venue
        // serves it, so the declared fee of a path is not computable from the path
        // alone. The criterion is the one an `arb` scope's generated plan uses, where
        // the venues are resolved; here it is refused with that reason rather than
        // ranked by a number the declaration never contained.
        if choice.criterion == x3_lang_ast::ast::ChoiceCriterion::LowestDeclaredFee {
            acc.add_error(err(format!(
                "atomic_choice '{name}' chooses by `lowest_declared_fee`, which ranks the venues a                  plan resolves a route to; a path body names hops, not venues, so its declared fee                  is not computable here. Write `fewest_hops` or `highest_net_output`, or let an \
                 `arb` scope rank its cycles"
            )));
        }

        let mut seen: Vec<&str> = Vec::new();
        let mut outputs: Vec<(&str, &str)> = Vec::new();
        for path in &choice.paths {
            let path_name = path.name.as_str();
            if seen.contains(&path_name) {
                acc.add_error(err(format!(
                    "atomic_choice '{name}' declares path '{path_name}' twice; branches are selected \
                     by index and a duplicate name makes the set ambiguous"
                )));
            }
            seen.push(path_name);

            if path.body.is_empty() {
                acc.add_error(err(format!(
                    "atomic_choice '{name}' path '{path_name}' has no executable body — a hop chain \
                     alone names a route but no venue to execute it against, so there is nothing to \
                     verify"
                )));
            }

            match &path.net_output {
                None => acc.add_error(err(format!(
                    "atomic_choice '{name}' path '{path_name}' declares no `net_output <amount> \
                     <ASSET>`; the compiler cannot compare a branch whose output it does not know"
                ))),
                Some(output) => {
                    if extract_int_from_expr(&output.value).is_none() {
                        acc.add_error(err(format!(
                            "atomic_choice '{name}' path '{path_name}' has a `net_output` that is not \
                             an integer literal; choosing between branches needs values the compiler \
                             can evaluate, not expressions it must defer"
                        )));
                    }
                    outputs.push((path_name, output.asset.as_str()));
                }
            }
        }

        if let Some((first_path, first_asset)) = outputs.first() {
            for (path_name, asset) in outputs.iter().skip(1) {
                if asset != first_asset {
                    acc.add_error(err(format!(
                        "atomic_choice '{name}' paths do not require the same output asset: path \
                         '{first_path}' produces {first_asset} and path '{path_name}' produces \
                         {asset}; a choice must compare equivalent outputs"
                    )));
                }
            }
        }

        if choice.criterion == x3_lang_ast::ast::ChoiceCriterion::FewestHops {
            for path in &choice.paths {
                if path_hop_count(path).is_none() {
                    acc.add_error(err(format!(
                        "atomic_choice '{name}' path '{}' has no hops to count — declare a hop chain \
                         or a swap/bridge statement for `choose fewest_hops`",
                        path.name.as_str()
                    )));
                }
            }
        }
    }
}

/// The guard kinds that can bound a substitution.
///
/// A `fallback` block's `require` lines exist to bound what a replacement may
/// cost. `require nonce unused ...` is not a bound on a substitution, so
/// accepting it would let the block look like it constrains the runtime while
/// constraining nothing.
fn guard_bounds_a_substitution(kind: &x3_lang_ast::ast::RequireKind) -> bool {
    use x3_lang_ast::ast::RequireKind;
    matches!(
        kind,
        RequireKind::Slippage | RequireKind::Profit | RequireKind::Finality | RequireKind::BridgeLiquidity
    )
}

/// Verify every route `fallback` is a closed, bounded, statically checked set of
/// substitutions.
///
/// The construct's promise is "the runtime may choose only among
/// compiler-approved routes". Each clause below is one part of that promise:
///
/// - **closed** — at least one replacement, at most
///   [`MAX_ROUTE_FALLBACKS`](crate::spec::opcodes::MAX_ROUTE_FALLBACKS), each
///   venue named once. There is no wildcard and no "any venue": a substitution
///   the compiler cannot enumerate is a substitution it cannot verify.
/// - **a real substitution** — a replacement whose venue is already a venue of
///   the same route replaces nothing, and the route must contain a `swap` leg
///   for a fallback to replace in the first place.
/// - **bounded** — every `require` inside the block must be a guard that
///   actually bounds a substitution, and its value must be an integer literal
///   the compiler can evaluate. A bound the compiler defers is not a bound.
///
/// The per-substitution *route* verification (does the replacement travel the
/// same assets, is the resulting route valid?) happens in lowering, where the
/// route's other steps are in scope.
pub fn verify_route_fallbacks(program: &Program, acc: &mut ErrorAccumulator) {
    // Declared venues, so a fallback's bounds have something to be checked
    // against. Before the opportunity graph existed a venue was only a name, so
    // `require slippage <= 7` inside a fallback bounded a quantity nothing
    // knew: the number was read, checked to be a literal, and then had nothing
    // to compare with. A venue that declares its slippage turns that bound into
    // a real check.
    let declared: Vec<(&str, u32)> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::VenueDecl(venue) => Some((venue.name.as_str(), venue.slippage_bps)),
            _ => None,
        })
        .collect();

    fn walk(statements: &[x3_lang_ast::ast::Statement], declared: &[(&str, u32)], acc: &mut ErrorAccumulator) {
        for statement in statements {
            match statement {
                x3_lang_ast::ast::Statement::RouteFallback { replacements, requires } => {
                    if replacements.is_empty() {
                        acc.add_error(err(
                            "fallback block approves no replacements; an empty approval set would let a \
                             failing leg be re-routed by something the compiler never checked",
                        ));
                    }
                    if replacements.len() > crate::spec::opcodes::MAX_ROUTE_FALLBACKS {
                        acc.add_error(err(format!(
                            "fallback approves {} replacements, above the {}-venue production bound for \
                             statically bounded fallbacks",
                            replacements.len(),
                            crate::spec::opcodes::MAX_ROUTE_FALLBACKS
                        )));
                    }
                    let mut seen: Vec<&str> = Vec::new();
                    for replacement in replacements {
                        let venue = replacement.venue.as_str();
                        if seen.contains(&venue) {
                            acc.add_error(err(format!(
                                "fallback approves venue '{venue}' twice; the approved set is a set, and a \
                                 duplicate would make it ambiguous which entry was verified"
                            )));
                        }
                        seen.push(venue);

                        // If the venue is declared, its own slippage must fit
                        // inside the bound the fallback promises. Approving a
                        // venue that declares more slippage than the block
                        // allows would make the block a claim the program does
                        // not satisfy.
                        // The bound is a ceiling: `require slippage <= N`. A
                        // fallback written with `>=` inside it is claiming a
                        // floor, and comparing it against a venue's declared
                        // slippage as a ceiling would invert the check.
                        let bound = requires.iter().find_map(|guard| {
                            (guard.kind == x3_lang_ast::ast::RequireKind::Slippage)
                                .then(|| guard.value.as_ref().and_then(extract_int_from_expr))
                                .flatten()
                        });
                        for guard in requires {
                            if guard.kind == x3_lang_ast::ast::RequireKind::Slippage
                                && !guard.comparison.is_some_and(|op| op.is_upper_bound())
                            {
                                acc.add_error(err("a fallback's slippage bound must be a ceiling (`<=`); the check \
                                     compares it against what the approved venue declares, which only \
                                     means something if the guard names an upper bound"
                                    .to_string()));
                            }
                        }
                        if let (Some(bound), Some((_, venue_slippage))) =
                            (bound, declared.iter().find(|(name, _)| *name == venue).copied())
                        {
                            if u128::from(venue_slippage) > bound {
                                acc.add_error(err(format!(
                                    "fallback approves venue '{venue}', which declares {venue_slippage} \
                                     bps of slippage, but the fallback bounds slippage at {bound} bps; \
                                     the approval promises something the venue does not offer"
                                )));
                            }
                        }
                    }
                    for guard in requires {
                        if !guard_bounds_a_substitution(&guard.kind) {
                            acc.add_error(err(format!(
                                "fallback contains `require {}`, which does not bound a substitution; a \
                                 fallback may only constrain what a replacement may cost",
                                format!("{:?}", guard.kind).to_lowercase()
                            )));
                        }
                        if guard.value.as_ref().and_then(extract_int_from_expr).is_none() {
                            acc.add_error(err(
                                "fallback bound is not an integer literal; a bound the compiler cannot \
                                 evaluate does not bound the runtime",
                            ));
                        }
                    }
                }
                x3_lang_ast::ast::Statement::Atomic(block) => walk(&block.body.stmts, declared, acc),
                x3_lang_ast::ast::Statement::If {
                    then_block, else_block, ..
                } => {
                    walk(&then_block.stmts, declared, acc);
                    if let Some(else_block) = else_block {
                        walk(&else_block.stmts, declared, acc);
                    }
                }
                _ => {}
            }
        }
    }

    for item in &program.items {
        match &item.node {
            Item::IntentDecl(intent) => walk(&intent.body.stmts, &declared, acc),
            Item::AtomicSwap(swap) => walk(&swap.body, &declared, acc),
            Item::Strategy(strategy) => walk(&strategy.body, &declared, acc),
            Item::Bridge(bridge) => walk(&bridge.body, &declared, acc),
            _ => {}
        }
    }
}

/// Verify a `parallel` block is a set of legs the DAG can reason about.
///
/// The interesting decisions — which legs are independent, and which race — are
/// made by the dependency DAG in lowering, because they need the legs' lowered
/// operations rather than their source. What is checked here is what the source
/// alone can settle: that there are legs to analyse, that they are distinct, and
/// that each one is a body.
pub fn verify_parallel_decls(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::ParallelDecl(parallel) = &item.node else {
            continue;
        };
        let name = parallel.name.as_str();
        if parallel.legs.len() < 2 {
            acc.add_error(err(format!(
                "parallel '{name}' declares {} leg(s); a parallel block with one leg is not \
                 parallel, and accepting it would make the artifact's claim of concurrent \
                 execution false",
                parallel.legs.len()
            )));
        }
        if parallel.legs.len() > crate::dag::MAX_PARALLEL_LEGS {
            acc.add_error(err(format!(
                "parallel '{name}' declares {} legs, above the {}-leg production bound",
                parallel.legs.len(),
                crate::dag::MAX_PARALLEL_LEGS
            )));
        }
        let mut seen: Vec<&str> = Vec::new();
        for leg in &parallel.legs {
            let leg_name = leg.name.as_str();
            if seen.contains(&leg_name) {
                acc.add_error(err(format!(
                    "parallel '{name}' declares leg '{leg_name}' twice; the plan names legs, so a \
                     duplicate makes the plan ambiguous"
                )));
            }
            seen.push(leg_name);
            if leg.body.is_empty() {
                acc.add_error(err(format!(
                    "parallel '{name}' leg '{leg_name}' is empty; it would contribute no dependencies \
                     and no work"
                )));
            }
        }
    }
}

/// The privacy levels this language implements, labelled.
///
/// PHASE 27 asks for two things at once: support private execution, and "do not
/// invent cryptographic guarantees that are not implemented", with each privacy
/// level clearly labelled. A table is how both are satisfied: what a program can
/// declare is exactly what a mechanism here can deliver, and the levels that are
/// not implemented are named so the refusal can say what *is*.
pub fn implemented_privacy_levels() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "public (no privacy block)",
            "nothing is hidden; every operation is visible in the artifact",
        ),
        (
            "hide_route_until_commit true + reveal_on <event>",
            "the route is committed to and revealed at <event>; the commitment is real \
             (PRIVACY_COMMIT), the hiding is until the reveal point",
        ),
        (
            "submission { private = required }",
            "the artifact refuses to run without a private channel; enforced against the \
             runtime's capability, the same gate the trading path uses",
        ),
    ]
}

/// Verify a `privacy` block only declares what a mechanism here provides.
///
/// The block has an `encrypted` flag that nothing implements — the executor's
/// `PrivacyCommit` arm reads it as `encrypted: _` — so `encrypted true` compiled
/// into an artifact that carried a claim of encryption with nothing behind it.
/// That is the specific failure PHASE 27 forbids, and a declaration that cannot
/// be honoured has to be refused rather than recorded.
pub fn verify_privacy_decls(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::PrivacyBlock(privacy) = &item.node else {
            continue;
        };
        if privacy.encrypted {
            let implemented: Vec<String> = implemented_privacy_levels()
                .iter()
                .map(|(level, what)| format!("\n  - {level}: {what}"))
                .collect();
            acc.add_error(err(format!(
                "privacy declares `encrypted true`, and no mechanism in this language implements \
                 encryption: the artifact would carry a claim of encryption that nothing provides. \
                 The levels that are implemented are:{}",
                implemented.join("")
            )));
        }
    }
}

/// Verify every `venue` declaration is a node the graph can actually use.
///
/// A venue's declared attributes are what the planner reads, so a declaration
/// that is internally inconsistent is worse than a missing one: it is a graph
/// edge that looks traversable and is not.
pub fn verify_venue_decls(program: &Program, acc: &mut ErrorAccumulator) {
    use x3_lang_ast::ast::VenueKind;

    let mut seen: Vec<&str> = Vec::new();
    for item in &program.items {
        let Item::VenueDecl(venue) = &item.node else {
            continue;
        };
        let name = venue.name.as_str();
        if seen.contains(&name) {
            acc.add_error(err(format!(
                "venue '{name}' is declared twice; the graph addresses venues by name, so a \
                 duplicate makes an edge ambiguous"
            )));
        }
        seen.push(name);

        // `>=`, not `>`: the whole (`Bps::WHOLE`) *is* the whole amount, so a fee
        // that high is not a venue. The message said "at or above" while the code
        // checked strictly above, which is how the boundary case went unchecked.
        if venue.fee_bps >= Bps::WHOLE.raw() {
            acc.add_error(err(format!(
                "venue '{name}' declares a fee of {} bps; a fee at or above 10_000 bps is the whole \
                 amount",
                venue.fee_bps
            )));
        }
        if !Bps::from_raw(venue.slippage_bps).is_within_whole() {
            acc.add_error(err(format!(
                "venue '{name}' declares {} bps of slippage, above 10_000 bps",
                venue.slippage_bps
            )));
        }
        if venue.risk > 100 {
            acc.add_error(err(format!(
                "venue '{name}' declares risk {}; the scale is 0 (safest) to 100",
                venue.risk
            )));
        }
        if venue.liquidity == 0 {
            acc.add_error(err(format!(
                "venue '{name}' declares zero liquidity; the graph would offer a route no size can \
                 use"
            )));
        }

        // An off-chain venue has no enforceable settlement, so a leg on it has to
        // say where the guarantee actually comes from. This is PHASE 39's whole
        // point: "Do not claim atomic CEX execution unless the external venue
        // exposes enforceable settlement semantics."
        if venue.kind == VenueKind::Orderbook {
            match venue.settlement {
                None => acc.add_error(err(format!(
                    "venue '{name}' is an `orderbook` venue and declares no `settlement`; a venue \
                     that matches off-chain does not settle both sides or neither by itself, so the \
                     program has to say what does: one of trusted_adapter, escrow, pre_funded, \
                     attested or compensating"
                ))),
                Some(guarantee) if guarantee.is_atomic() => acc.add_error(err(format!(
                    "venue '{name}' is an `orderbook` venue and claims `settlement atomic`; an \
                     off-chain venue does not expose enforceable settlement semantics, so the claim \
                     would be false. Say where the guarantee really comes from: trusted_adapter, \
                     escrow, pre_funded, attested or compensating"
                ))),
                Some(_) => {}
            }
        }

        let asset_in = format!("{}.{}", venue.asset_in.chain.as_str(), venue.asset_in.name.as_str());
        let asset_out = format!("{}.{}", venue.asset_out.chain.as_str(), venue.asset_out.name.as_str());
        if asset_in == asset_out && venue.kind != VenueKind::Lending && venue.kind != VenueKind::Flash {
            acc.add_error(err(format!(
                "venue '{name}' takes in and gives out the same asset ({asset_in}); only a lending or \
                 flash venue moves one asset, and a {} venue that does would be an edge from an asset \
                 to itself, which no path can use",
                venue.kind.as_str()
            )));
        }

        // A venue that settles on a chain it does not trade on would put the
        // path on a chain the graph never names.
        let chain = venue.chain.as_str();
        if asset_in.split('.').next() != Some(chain) && venue.kind != VenueKind::Bridge {
            acc.add_error(err(format!(
                "venue '{name}' trades {asset_in} but settles on chain '{chain}'; only a bridge \
                 adapter moves an asset to another chain"
            )));
        }
    }
}

/// A `require solver_bond >= N` guard needs a bond to compare against.
///
/// The guard asserts something about the program's configuration — "the solver
/// backing this trade has posted at least N" — so it is a compile-time check,
/// not a run-time one. Without a `solver_market { bond <amount> <ASSET> }`
/// declaration there is no bond anywhere in the program, and the guard asserted
/// nothing in either the compiler or the VM. This is the compile-time half; the
/// run-time half is TICKET-027.
pub fn verify_solver_bond_declared(program: &Program, acc: &mut ErrorAccumulator) {
    let declared = program.items.iter().find_map(|item| match &item.node {
        Item::SolverMarket(market) => market.bond.as_ref().and_then(|bond| extract_int_from_expr(&bond.value)),
        _ => None,
    });

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::SolverBond {
            continue;
        }
        // `require solver_bond >= N` claims a floor. Written with a ceiling the
        // guard says something else, and comparing it against the declared bond
        // as though it were a floor would answer a question nobody asked.
        if !guard.comparison.is_some_and(|op| op.is_lower_bound()) {
            acc.add_error(err(format!(
                "declaration '{owner}' states `require solver_bond` without a `>=` bound; a solver bond guard is a floor, and the check reads it as one"
            )));
            continue;
        }
        let required = guard.value.as_ref().and_then(extract_int_from_expr).unwrap_or(0);
        match declared {
            None => acc.add_error(err(format!(
                "declaration '{owner}' requires a solver bond of {required} but the program declares \
                 no `solver_market {{ bond <amount> <ASSET> }}` — the guard has nothing to compare \
                 against"
            ))),
            Some(bond) if required > bond => acc.add_error(err(format!(
                "declaration '{owner}' requires a solver bond of {required}, but the declared bond is {bond}"
            ))),
            Some(_) => {}
        }
    }
}

/// A `require relayer_quorum >= N` guard needs a quorum to compare against.
///
/// The guard asserts "at least N relayers attest this operation";
/// `relayers { quorum N_of_M }` is where the swarm states how many must
/// actually attest. So a guard demanding more relayers than the declared
/// quorum claims something the configuration never does, and a guard with no
/// swarm at all claims something no configuration backs. Same shape as the
/// solver bond, and the same reason it has to be a compile-time check.
pub fn verify_relayer_quorum_declared(program: &Program, acc: &mut ErrorAccumulator) {
    let declared = program.items.iter().find_map(|item| match &item.node {
        Item::RelayerSwarm(swarm) => Some(swarm.quorum_numerator),
        _ => None,
    });

    for (owner, guard) in require_guards(program) {
        if guard.kind != x3_lang_ast::ast::RequireKind::RelayerQuorum {
            continue;
        }
        if !guard.comparison.is_some_and(|op| op.is_lower_bound()) {
            acc.add_error(err(format!(
                "declaration '{owner}' states `require relayer_quorum` without a `>=` bound; a quorum guard is a floor, and the check reads it as one"
            )));
            continue;
        }
        let required = guard.value.as_ref().and_then(extract_int_from_expr).unwrap_or(0);
        match declared {
            None => acc.add_error(err(format!(
                "declaration '{owner}' requires a relayer quorum of {required} but the program \
                 declares no `relayers {{ quorum N_of_M }}` — the guard has nothing to compare against"
            ))),
            Some(quorum) if required > u128::from(quorum) => acc.add_error(err(format!(
                "declaration '{owner}' requires a relayer quorum of {required}, but the declared \
                 swarm attests with a quorum of {quorum}"
            ))),
            Some(_) => {}
        }
    }
}

/// Check that a chain name is in the known-chains allow-list (case-insensitive).
fn is_known_chain(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SWAP_KNOWN_CHAINS.iter().any(|k| *k == lower)
}

/// Run AST-level validation on every `AtomicSwap` declaration in the program.
///
/// Catches problems that are invisible after lowering:
/// - unknown chain names
/// - same source/destination chain
/// - unsupported hash functions
/// - zero amounts
/// - timeout ordering violations
pub fn verify_atomic_swap_decls(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        if let Spanned {
            node: Item::AtomicSwap(decl),
            ..
        } = item
        {
            validate_atomic_swap(decl, acc);
        }
    }
}

fn validate_atomic_swap(decl: &AtomicSwapDecl, acc: &mut ErrorAccumulator) {
    // Validate 1: Known chain names
    let from_chain = decl.from_asset.chain.as_str();
    let to_chain = decl.to_asset.chain.as_str();

    if !is_known_chain(from_chain) {
        acc.add_error(err(format!("Unknown chain '{from_chain}' in atomic swap")));
    }
    if !is_known_chain(to_chain) {
        acc.add_error(err(format!("Unknown chain '{to_chain}' in atomic swap")));
    }

    // Validate 2: Cross-chain (different source/dest)
    if from_chain.to_ascii_lowercase() == to_chain.to_ascii_lowercase() {
        acc.add_error(err("Atomic swap must be between different chains"));
    }

    // Validate 3: Valid hash function
    if let Some(hashlock) = &decl.hashlock {
        let hash_fn = hashlock.hash_fn.as_str();
        if !SUPPORTED_HASH_FUNCTIONS
            .iter()
            .any(|h| *h == hash_fn.to_ascii_lowercase())
        {
            acc.add_error(err(format!(
                "Unknown hash function '{hash_fn}' in atomic swap. Supported: sha256, blake2b"
            )));
        }
    }

    // Validate 4: Positive amount
    if let Some(amount_expr) = &decl.amount {
        if let Some(n) = extract_int_from_expr(amount_expr) {
            if n == 0 {
                acc.add_error(err("Atomic swap amount must be positive"));
            }
        }
    }

    // Validate 5: Timeout ordering
    if let (Some(src_expr), Some(dst_expr)) = (&decl.timeout_source, &decl.timeout_destination) {
        // Compared in the unit that is enforced — blocks, converted by the one
        // function lowering uses. This read the same expression as *seconds* and
        // skipped an expression it could not read, so a program that wrote its
        // units (`timeout source 40m`) had no ordering check at all.
        if let (Some(src_blocks), Some(dst_blocks)) = (
            crate::lowering::timeout_expression_to_blocks(src_expr),
            crate::lowering::timeout_expression_to_blocks(dst_expr),
        ) {
            if src_blocks <= dst_blocks {
                acc.add_error(err(format!(
                    "Source timeout ({src_blocks} blocks) must be greater than destination timeout \
                     ({dst_blocks} blocks) in atomic swap"
                )));
            }
        }
    }

    // Validate 6: Require guards
    for require in &decl.requires {
        validate_atomic_swap_require(require, acc);
    }
}

fn validate_atomic_swap_require(require: &x3_lang_ast::ast::RequireGuard, acc: &mut ErrorAccumulator) {
    match &require.kind {
        x3_lang_ast::ast::RequireKind::Finality => {
            // finality requires a subject (chain name)
            if require.subject.is_none() {
                acc.add_error(err(
                    "require finality needs a chain subject (e.g. 'finality.eth >= 12')",
                ));
            }
        }
        x3_lang_ast::ast::RequireKind::RelayerQuorum => {
            // relayer_quorum must be a positive integer
            if let Some(n) = require.value.as_ref().and_then(extract_int_from_expr) {
                if n == 0 {
                    acc.add_error(err("require relayer_quorum must be positive"));
                }
            }
        }
        _ => {}
    }
}

/// Verify that value which leaves the program's control has a way back.
///
/// Two mechanisms count, and they are not interchangeable: an explicit
/// `OnFail`/`OnTimeout` with a `Refund`, which is the only thing that can bring value
/// back from another chain; and the atomic rollback for a `Lock` whose escrow the same
/// route claims, which means the lock never took effect. Requiring a handler there as
/// well was not stricter, it was contradictory — a handler refunding an escrow the same
/// route claims is refused by `no_refund_after_claim`, so a same-asset
/// lock-and-release, which is what settling a netting book is, could not be written.
pub fn verify_refund_path_exists(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let mut has_cross_chain = false;
    let mut has_lock = false;
    let mut has_refund = false;
    for op in &ir.operations {
        if matches!(op, Operation::Bridge { .. } | Operation::Swap { .. }) {
            has_cross_chain = true;
        }
        if matches!(op, Operation::Lock { .. }) {
            has_lock = true;
        }
        if let Operation::OnFail { action } = op {
            if matches!(action, FailureAction::Refund { .. }) {
                has_refund = true;
            }
        }
        if let Operation::OnTimeout { action, .. } = op {
            if matches!(action, FailureAction::Refund { .. }) {
                has_refund = true;
            }
        }
    }
    // The rollback is a refund path for a lock whose escrow the same route claims:
    // `AtomicBegin` snapshots and a failed route truncates the recorded asset
    // operations (`vm/src/executor.rs`), so the lock never takes effect. It is *not*
    // a path back from another chain, which is why bridges and swaps still need an
    // explicit handler — a rollback restores this VM's state and cannot reach a
    // bridge's far side.
    let rollback_covers_locks = escrows_claimed_in_their_own_route(ir);
    if has_cross_chain && !has_refund {
        acc.add_error(err(
            "cross-chain operation present without a refund path — add an OnFail or OnTimeout with Refund action",
        ));
    }
    if has_lock && !has_refund && !rollback_covers_locks {
        acc.add_error(err(
            "a `Lock` leaves the program's control with no way back — add an OnFail or OnTimeout              with a Refund action, or release the escrow inside the same atomic route, where a              failed route's rollback means the lock never took effect",
        ));
    }
    // A `require refund_path` guard makes the same claim this pass makes about
    // the program, so it is decided here rather than recorded as `STATIC`: a
    // guard demanding a refund path the program does not have is a false
    // statement about the artifact. The check above only fires for a program with
    // a cross-chain operation; this one fires for the guard wherever it is
    // written.
    let guards_refund_path = ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Require {
                kind: crate::ir::RequireKind::RefundPath,
                ..
            }
        )
    });
    if guards_refund_path && !has_refund {
        acc.add_error(err(
            "`require refund_path` demands a refund path, and this program has none — add an OnFail \
             or OnTimeout with a Refund action, or drop the guard",
        ));
    }
}

/// Verify that every cross-chain operation has explicit finality
/// requirements declared via Require with Finality kind.
///
/// This is an error, not a warning. `verify_with_config` returns `Ok(())`
/// whenever no *error* was accumulated, so a warning here was collected and
/// then dropped on the floor: a bridging program with no finality
/// requirement compiled silently. That is the exact shape of the classic
/// cross-chain loss — minting against a source-chain lock that a reorg can
/// still erase — so the requirement is enforced at compile time instead.
pub fn verify_finality_explicit(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let bridge_chains: HashSet<String> = ir
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Bridge { from_chain, .. } => Some(from_chain.to_ascii_lowercase()),
            _ => None,
        })
        .collect();

    if bridge_chains.is_empty() {
        return;
    }

    let finality_chains: HashSet<String> = ir
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Require {
                kind: crate::ir::RequireKind::Finality,
                subject,
                ..
            } => subject.as_ref().map(|s| s.to_ascii_lowercase()),
            _ => None,
        })
        .collect();

    for chain in &bridge_chains {
        if !finality_chains.contains(chain) {
            acc.add_error(err(format!(
                "bridge from chain '{chain}' has no explicit finality requirement — add `require finality.{chain} >= <confirmations>`"
            )));
        }
    }
}

/// Verify that every swap leg carries an explicit slippage bound.
///
/// `min_output` is not a substitute. It is a single absolute floor baked into
/// one route at compile time; it says nothing about how far the market may
/// move between the quote and the fill, and it cannot adapt when the route's
/// liquidity does. A program with a swap leg and no `require slippage <= N`
/// therefore has no bound at all on that leg's execution quality, which is
/// how a "profitable" cross-chain trade settles at a loss. Same fail-closed
/// shape as `verify_finality_explicit`: the bound must be declared, not
/// assumed.
pub fn verify_slippage_explicit(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let has_swap_leg = ir
        .operations
        .iter()
        .any(|op| matches!(op, Operation::Swap { .. } | Operation::MultiHopSwap { .. }));
    if !has_swap_leg {
        return;
    }
    let has_slippage_bound = ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Require {
                kind: crate::ir::RequireKind::SlippageTolerance,
                ..
            }
        )
    });
    if !has_slippage_bound {
        acc.add_error(err(
            "swap leg present without an explicit slippage bound — add `require slippage <= <percent>`",
        ));
    }
}

/// A proof category a cross-chain program has to account for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum ProofCategory {
    /// Evidence that the source chain locked the asset.
    SourceLock,
    /// Evidence that the destination chain filled the transfer.
    DestinationFill,
}

/// Classify a declared proof name into the category it satisfies.
///
/// Proof names are free-form symbols — `parse_proofs_required_item` accepts any
/// identifier — so matching is a naming convention, done by substring so a
/// project may call its lock proof `eth_lock_attestation`. The vocabulary this
/// recognises is the one the repository's own programs use:
/// `source_lock_proof`, `source_finality_proof`, `destination_fill_proof`,
/// `destination_finality_proof`, `solver_signature`.
fn proof_category(name: &str) -> Option<ProofCategory> {
    let name = name.to_ascii_lowercase();
    if name.contains("lock") {
        Some(ProofCategory::SourceLock)
    } else if name.contains("fill") {
        Some(ProofCategory::DestinationFill)
    } else {
        None
    }
}

/// Verify that state transitions have required proof declarations.
///
/// This used to demand `lock_proof`, `fill_proof` and `claim_proof`. Those
/// exact names appear in **zero** `.x3` files in the repository, so no correct
/// program could satisfy the check: every bridging program reported warnings it
/// could do nothing about. The requirement now matches the vocabulary programs
/// actually use.
///
/// The `claim_proof` requirement is gone rather than renamed. No program
/// declares one, and the destination-fill proof is what authorises the release,
/// so requiring the name was the same defect in a third place. What a release
/// must *actually* prove — a destination fill, a receipt, a validator quorum —
/// is a design question; see TICKET-018.
pub fn verify_proof_requirements(ir: &X3IR, mode: Option<CompilationMode>, acc: &mut ErrorAccumulator) {
    let has_bridge = ir.operations.iter().any(|op| matches!(op, Operation::Bridge { .. }));

    let declared: HashSet<ProofCategory> = ir
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::ProofRequired { proof_type, .. } => proof_category(proof_type),
            _ => None,
        })
        .collect();

    // Gated on bridging, not on `Lock`.
    //
    // A lock proof exists to convince the *destination* chain that the funds
    // are locked on the source. With no `Bridge` there is no destination to
    // convince, and lowering emits a `Lock` for a same-chain transfer — so a
    // program that never leaves a chain was told to prove it had locked funds
    // on a chain it never left. Measured on a same-chain intent: one warning,
    // and nothing the program could do about it. That is the same defect the
    // vocabulary fix above addressed, one layer down: a warning a correct
    // program cannot act on trains its reader to ignore the check.
    // Severity by mode, and the decision is written down because the two tickets
    // that asked for it (TICKET-020, TICKET-024) said either answer is acceptable
    // as long as it is stated. The obligation is real — a bridge whose lock is not
    // proven is a bridge the destination fills against nothing — but it is a
    // *mainnet* requirement, which is where every other rule of this shape lives
    // (`verify_mainnet_safe` runs single-RPC, single-relayer, refund-path,
    // finality, solver-bond and the rest as errors and only for mainnet). Tolerating
    // it in dev keeps the warning actionable while a program is being written;
    // tolerating it on mainnet would ship the hole. Every bridging program in the
    // corpus declares both proofs already, so promotion costs the corpus nothing.
    let report = |acc: &mut ErrorAccumulator, obligation: &str| {
        let message = format!("Bridge operation present without {obligation}");
        if mode == Some(CompilationMode::Mainnet) {
            acc.add_error(err(format!("mainnet: {message}")));
        } else {
            acc.add_warning(X3Error::SemanticError { message, span: span() });
        }
    };

    if has_bridge && !declared.contains(&ProofCategory::SourceLock) {
        report(acc, "a source-lock proof — add `proofs required { source_lock_proof }`");
    }
    if has_bridge && !declared.contains(&ProofCategory::DestinationFill) {
        report(
            acc,
            "a destination-fill proof — add `proofs required { destination_fill_proof }`",
        );
    }
}

/// Verify that invariant rules are respected by the IR.
pub fn verify_invariants_on_intent(ir: &X3IR, invariants: &[InvariantRule]) -> Vec<String> {
    let mut violations = Vec::new();
    for rule in invariants {
        match (rule.check_fn)(ir) {
            Ok(()) => {}
            Err(msg) => violations.push(format!("invariant '{}' violated: {}", rule.name, msg)),
        }
    }
    violations
}

/// Verify invariants and emit structured warnings via the ErrorAccumulator.
///
/// NOTE (2026-09-18): these rules are unsound for `intent` programs and must
/// not be promoted to errors yet. They reason about linear position in
/// `ir.operations`, but intent lowering emits the `from`/`to` endpoints
/// *before* the route body, and it uses `Release` for the destination endpoint
/// rather than for a source-chain claim. A well-formed intent such as
/// `internal_swap.x3` therefore trips four of the six rules — double-claim,
/// both claim/refund orderings, and destination-fill-before-claim — which
/// makes them false positives against the intent surface rather than findings.
/// See `.ai/reports/x3lang-intent-guards-20260918.md`.
pub fn verify_invariants_structured(ir: &X3IR, invariants: &[InvariantRule], acc: &mut ErrorAccumulator) {
    for rule in invariants {
        match (rule.check_fn)(ir) {
            Ok(()) => {}
            Err(msg) => {
                acc.add_warning(X3Error::SemanticError {
                    message: format!("invariant '{}' violated: {}", rule.name, msg),
                    span: Span::DUMMY,
                });
            }
        }
    }
}

/// Operations that execute *inside* an atomic route block, in order.
///
/// Intent lowering emits the `from`/`to` endpoints before the route body and a
/// timeout/on-fail handler after it, so a `Lock`, `Release` or refund handler
/// sitting outside an atomic block is a *declaration*: its position in the
/// operation list is not its position in the execution order. Positional
/// reasoning is only sound over operations that actually execute together, so
/// the ordering rules below use this view.
///
/// Bodies of `If`, `Loop`, `Simulate`, `ScheduledDispatch` and `GasAdaptive` are
/// walked too, so a claim nested inside a branch or loop is still seen.
fn atomic_scoped_operations(ir: &X3IR) -> Vec<&Operation> {
    let mut scoped = Vec::new();
    collect_atomic_scoped(&ir.operations, 0, &mut scoped);
    scoped
}

fn collect_atomic_scoped<'a>(ops: &'a [Operation], depth: usize, out: &mut Vec<&'a Operation>) {
    let mut depth = depth;
    for op in ops {
        match op {
            Operation::AtomicBegin => {
                depth += 1;
                continue;
            }
            Operation::AtomicEnd => {
                depth = depth.saturating_sub(1);
                continue;
            }
            _ => {}
        }
        if depth > 0 {
            out.push(op);
        }
        match op {
            Operation::If { then_ops, else_ops, .. } => {
                collect_atomic_scoped(then_ops, depth, out);
                if let Some(else_ops) = else_ops {
                    collect_atomic_scoped(else_ops, depth, out);
                }
            }
            Operation::Loop { body, .. } | Operation::Simulate { body, .. } => {
                collect_atomic_scoped(body, depth, out);
            }
            Operation::ScheduledDispatch { entry, .. } => collect_atomic_scoped(entry, depth, out),
            Operation::GasAdaptive {
                high_gas_ops,
                low_gas_ops,
            } => {
                collect_atomic_scoped(high_gas_ops, depth, out);
                collect_atomic_scoped(low_gas_ops, depth, out);
            }
            _ => {}
        }
    }
}

/// Whether `op` is a guard that refunds on failure or timeout.
fn is_refund_action(op: &Operation) -> bool {
    refund_lock(op).is_some()
}

/// The lock a refund handler refunds, as `(chain, asset)`.
///
/// A refund handler does not refund "the program"; it refunds one specific
/// lock. Two handlers that name different chains and assets are two different
/// locks, which is exactly what a two-legged atomic swap has.
fn refund_lock(op: &Operation) -> Option<(&str, &str)> {
    match op {
        Operation::OnTimeout {
            action: FailureAction::Refund { chain, asset, .. },
            ..
        }
        | Operation::OnFail {
            action: FailureAction::Refund { chain, asset, .. },
        } => Some((chain.as_str(), asset.as_str())),
        _ => None,
    }
}

/// Whether some atomic route locks an escrow and claims it again inside itself.
///
/// That route's refund path is its own rollback, which is a real mechanism and not an
/// exemption: a route that fails records no asset operations at all.
fn escrows_claimed_in_their_own_route(ir: &X3IR) -> bool {
    // Escrow-level, deliberately: the question is whether *this program's* escrow of an asset
    // is claimed again inside the route that locked it, and a refund path is keyed by asset
    // rather than by which of a route's locks it names. The claim *index* is what
    // `no_double_claim` needs, and it is read there (TICKET-080).
    let mut locks: Vec<(&str, &str)> = Vec::new();
    let mut claims: Vec<(&str, &str)> = Vec::new();
    let mut depth = 0usize;
    for op in &ir.operations {
        match op {
            Operation::AtomicBegin => {
                depth += 1;
                locks.clear();
                claims.clear();
            }
            Operation::AtomicEnd => {
                if locks.iter().any(|lock| claims.contains(lock)) {
                    return true;
                }
                depth = depth.saturating_sub(1);
                locks.clear();
                claims.clear();
            }
            _ if depth > 0 => match op {
                Operation::Lock { chain, asset, .. } => locks.push((chain.as_str(), asset.as_str())),
                Operation::Release { chain, asset, .. } => claims.push((chain.as_str(), asset.as_str())),
                _ => {}
            },
            _ => {}
        }
    }
    false
}

/// The escrow a claim releases, as `(chain, asset)`, **for a release that claims one**.
///
/// A payout is not a claim and does not appear here. That distinction used to be inferred —
/// `no_refund_after_claim` kept a `locked_escrows` guard because a payout of an asset nobody
/// locked was being read as a claim, and the canonical two-legged swap warned on every check and
/// build for it (TICKET-035). The IR says which a release is now, so the rule reads it
/// (TICKET-101).
fn release_lock(op: &Operation) -> Option<(&str, &str)> {
    match op {
        Operation::Release {
            chain,
            asset,
            act: ReleaseAct::Claims(_),
            ..
        } => Some((chain.as_str(), asset.as_str())),
        _ => None,
    }
}

/// Which of its route's locks a claim names, or `None` for a release that claims nothing.
///
/// This is the identity `no_double_claim` needs: two releases of one asset in one route name
/// two different locks, and without the index they were the same claim to every reader — which
/// is what forced a book to settle one transfer per route (TICKET-080).
fn claimed_lock(op: &Operation) -> Option<u32> {
    match op {
        Operation::Release {
            act: ReleaseAct::Claims(index),
            ..
        } => Some(*index),
        _ => None,
    }
}

/// Return the list of built-in invariant rules for static analysis.
///
/// The duplicate-claim and claim/refund-ordering rules below reason about
/// *execution order*, so they only look at operations that execute together —
/// see [`atomic_scoped_operations`]. They used to scan the whole operation list,
/// which reported four violations on every well-formed bridging intent:
/// `tests/conformance/valid/intents/internal_swap.x3`,
/// `examples/mainnet_safe_swap.x3`, `examples/timeout_refund.x3` and
/// `examples/flagship_b52.x3` all tripped them, because intent lowering emits
/// the `from`/`to` endpoints *before* the route body and a timeout/on-fail
/// handler *after* it. Positional rules are meaningless over declarations whose
/// order is not execution order, so the rules looked protective while actually
/// being noise.
///
/// `no_double_refund` and `no_route_mutation_after_lock` stay program-wide:
/// handler cardinality and "no route mutation after a lock" are properties of
/// the program, not of a position within one block.
pub fn get_builtin_invariants() -> Vec<InvariantRule> {
    vec![
        InvariantRule {
            name: "no_double_claim".into(),
            description: "No claim operation may execute twice for the same lock".into(),
            check_fn: |ir| {
                // Counted **per lock, within one route**, which is what the rule's own
                // description says ("twice for the same lock"). It used to count every
                // `Release` in the program, so a program with two routes was reported as
                // one lock claimed twice — the defect `no_double_refund` below was fixed
                // for, with the same reason: two claims against two locks are not one
                // lock claimed twice. The `release_lock` helper that names a claim's lock
                // already existed for this.
                //
                // Per route rather than per program because a route is the unit that
                // settles: claims in two routes are two settlements, not one claim made
                // twice.
                let mut claims: Vec<(&str, &str, u32)> = Vec::new();
                let mut depth = 0usize;
                for op in &ir.operations {
                    match op {
                        Operation::AtomicBegin => {
                            depth += 1;
                            claims.clear();
                        }
                        Operation::AtomicEnd => {
                            depth = depth.saturating_sub(1);
                            claims.clear();
                        }
                        _ if depth > 0 => {
                            let (Some((chain, asset)), Some(index)) = (release_lock(op), claimed_lock(op)) else {
                                continue;
                            };
                            let lock = (chain, asset, index);
                            if claims.contains(&lock) {
                                return Err(format!(
                                    "multiple Release (claim) operations found for the same lock \
                                     ({chain}.{asset}, lock #{} of its route) inside one atomic route",
                                    lock.2
                                ));
                            }
                            claims.push(lock);
                        }
                        _ => {}
                    }
                }
                Ok(())
            },
        },
        InvariantRule {
            name: "no_double_refund".into(),
            description: "No refund operation may execute twice for the same lock".into(),
            check_fn: |ir| {
                // Counted per lock, which is what the rule's own description
                // says. Counting globally reported every two-legged atomic
                // swap: `examples/atomic_swap.x3` refunds `eth.USDC` on the
                // source timeout and `sol.SOL` on the destination timeout, and
                // those are two different locks, not one lock refunded twice.
                let mut refunds: Vec<(&str, &str)> = Vec::new();
                for op in &ir.operations {
                    let Some(lock) = refund_lock(op) else {
                        continue;
                    };
                    if refunds.contains(&lock) {
                        return Err(format!(
                            "multiple refund operations found for the same lock ({}.{})",
                            lock.0, lock.1
                        ));
                    }
                    refunds.push(lock);
                }
                Ok(())
            },
        },
        InvariantRule {
            name: "no_claim_after_refund".into(),
            description: "Claim must not execute after refund".into(),
            check_fn: |ir| {
                let mut found_refund = false;
                for op in atomic_scoped_operations(ir) {
                    if is_refund_action(op) {
                        found_refund = true;
                    }
                    if found_refund && matches!(op, Operation::Release { .. }) {
                        return Err("Release (claim) found after refund".into());
                    }
                }
                Ok(())
            },
        },
        InvariantRule {
            name: "no_refund_after_claim".into(),
            description: "Refund must not execute after claim".into(),
            check_fn: |ir| {
                // Per escrow, and only for an escrow this program locked.
                //
                // `Release` means two things in this IR: claiming an escrow the
                // program locked, and paying out the asset a route delivered. A
                // two-legged swap pays the destination asset out and, on the other
                // settlement path, refunds that same asset on timeout — so a scan
                // that read the payout as a claim called it a claim-then-refund.
                // Measured: `examples/atomic_swap.x3`, the roadmap's canonical
                // example, warned on every check and build (TICKET-035).
                //
                // A refund can only double-spend an escrow that exists, so the rule
                // requires the asset to have been locked by this program: the
                // payout of an asset nobody locked is not a claim this rule is
                // about, while a claim of a locked escrow that is refunded later in
                // the same route is still caught.
                let mut claimed: Vec<(&str, &str)> = Vec::new();
                for op in atomic_scoped_operations(ir) {
                    if let Some(lock) = release_lock(op) {
                        claimed.push(lock);
                    }
                    if let Some(lock) = refund_lock(op) {
                        // No `locked` guard: a release that appears in `claimed` **is** a claim,
                        // and a claim names a lock its route holds — so the escrow exists by
                        // construction rather than by a second lookup. The guard was here
                        // because a payout of an unlocked asset used to be indistinguishable
                        // from a claim; the IR says which is which now (TICKET-101).
                        if claimed.contains(&lock) {
                            return Err(format!(
                                "Refund of {}.{} found after its Release (claim)",
                                lock.0, lock.1
                            ));
                        }
                    }
                }
                Ok(())
            },
        },
        InvariantRule {
            name: "destination_fill_before_source_claim".into(),
            description: "Destination must be filled before source claim".into(),
            check_fn: |ir| {
                let scoped = atomic_scoped_operations(ir);
                let bridge_positions: Vec<usize> = scoped
                    .iter()
                    .enumerate()
                    .filter(|(_, op)| matches!(op, Operation::Bridge { .. }))
                    .map(|(i, _)| i)
                    .collect();
                // Nothing to order against when the route does not bridge: a
                // same-chain route releasing its own escrow is not this rule's
                // business, and flagging it would be the same false positive in
                // a different disguise.
                if bridge_positions.is_empty() {
                    return Ok(());
                }
                let release_positions: Vec<usize> = scoped
                    .iter()
                    .enumerate()
                    .filter(|(_, op)| matches!(op, Operation::Release { .. }))
                    .map(|(i, _)| i)
                    .collect();
                for &ri in &release_positions {
                    if !bridge_positions.iter().any(|&bi| bi < ri) {
                        return Err(
                            "Release (claim) found before any Bridge fill — destination must be filled first".into(),
                        );
                    }
                }
                Ok(())
            },
        },
        InvariantRule {
            name: "no_route_mutation_after_lock".into(),
            description: "Route may not change after lock".into(),
            check_fn: |ir| {
                let mut found_lock = false;
                for op in &ir.operations {
                    if matches!(op, Operation::Lock { .. }) {
                        found_lock = true;
                    }
                    if found_lock && matches!(op, Operation::RouteScore { .. }) {
                        return Err("RouteScore found after Lock — route mutation not allowed after lock".into());
                    }
                    if found_lock && matches!(op, Operation::Pathfind { .. }) {
                        return Err("Pathfind found after Lock — route mutation not allowed after lock".into());
                    }
                }
                Ok(())
            },
        },
    ]
}

/// Verify that route scoring weights sum to 100 and are reasonable.
pub fn verify_route_score(ir: &X3IR, acc: &mut ErrorAccumulator) {
    for op in &ir.operations {
        if let Operation::RouteScore { strategy, weights } = op {
            let total: u32 = weights.values().sum();
            if total != 100 {
                acc.add_error(err(format!(
                    "route score strategy '{strategy}' weights sum to {total}, expected 100"
                )));
            }
            for (key, &val) in weights {
                if val > 100 {
                    acc.add_error(err(format!(
                        "route score strategy '{strategy}' weight '{key}' is {val}, exceeds 100"
                    )));
                }
            }
        }
    }
}

/// `require risk <= N` is decided against the score this compiler computes for
/// the program itself.
///
/// The guard names a quantity no clause declares: `compute_risk_score` reads the
/// program's own operations, so the check has the same shape as
/// `canonical_supply` — evaluated against what the program does rather than
/// compared against a declaration — and a program that under-states its own risk
/// is refused with the parts that produced the number.
///
/// The bound is a ceiling. `require risk >= N` says the program must be at least
/// that risky, which is not a property to ask a program to satisfy, and comparing
/// a floor against a score below it is how a guard meant as a ceiling becomes one
/// that cannot fail.
pub fn verify_risk_score_guards(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let mut guards: Vec<(Option<crate::ir::ComparisonOp>, Option<u128>)> = Vec::new();
    for op in &ir.operations {
        if let Operation::Require {
            kind: crate::ir::RequireKind::RiskScore,
            condition,
            comparison,
            ..
        } = op
        {
            let value = match condition {
                Condition::Expression { expr } => expr.trim().parse::<u128>().ok(),
                _ => None,
            };
            guards.push((*comparison, value));
        }
    }
    if guards.is_empty() {
        return;
    }

    let score = compute_risk_score(ir);
    for (comparison, limit) in guards {
        if !comparison.is_some_and(|op| op.is_upper_bound()) {
            acc.add_error(err(format!(
                "`require risk` states a risk score without a ceiling; the score is a risk, so the \
                 bound is an upper one — write `require risk <= {}`",
                limit
                    .map(|limit| limit.to_string())
                    .unwrap_or_else(|| "<n>".to_string())
            )));
            continue;
        }
        let Some(limit) = limit else {
            acc.add_error(err(
                "`require risk` states a bound the compiler cannot read as a number; the score it is \
                 checked against is computed, so the guard has to state an integer"
                    .to_string(),
            ));
            continue;
        };
        if u128::from(score.total) > limit {
            acc.add_error(err(format!(
                "`require risk <= {limit}` claims a risk score of at most {limit}, but this program's \
                 computed score is {} (chain {}, bridge {}, solver {}, relayer {}, rpc {}, liquidity \
                 {}, finality {}, mev {}, timeout {}, refund {})",
                score.total,
                score.chain_risk,
                score.bridge_risk,
                score.solver_risk,
                score.relayer_risk,
                score.rpc_risk,
                score.liquidity_risk,
                score.finality_risk,
                score.mev_risk,
                score.timeout_risk,
                score.refund_risk
            )));
        }
    }
}

// ───── Mainnet safety checks ─────────────────────────────────────────────

/// Whether the program claims `mainnet_safe`.
///
/// The guard is a *request for the mainnet checks* rather than a claim about a
/// compilation mode, and the pass below honours it by running them. That is what
/// makes the guard's claim true rather than recorded: refusing it outside mainnet
/// mode would make it unusable where programs are actually written, and recording
/// it while the checks did not run would make the artifact assert something no run
/// established — the defect this family of checks exists to remove.
fn claims_mainnet_safe(ir: &X3IR) -> bool {
    ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Require {
                kind: crate::ir::RequireKind::MainnetSafe,
                ..
            }
        )
    })
}

/// Run all mainnet-specific safety checks. Rejects the program if any
/// production-safety rule is violated.
pub fn verify_mainnet_safe(ir: &X3IR, acc: &mut ErrorAccumulator) {
    verify_single_rpc(ir, acc);
    verify_single_relayer(ir, acc);
    verify_refund_path_exists(ir, acc);
    verify_finality_explicit(ir, acc);
    verify_solver_bond(ir, acc);
    verify_known_assets(ir, acc);
    verify_slippage_safe(ir, acc);
    verify_deadline_bounded(ir, acc);
    verify_bridge_adapter_allowlist(ir, acc);
    verify_manual_recovery(ir, acc);
}

/// Whether the program contains any general-VM cross-chain/bridge-shaped
/// operation. RPC-consensus, relayer-attestation, and solver-bond safety
/// only mean anything for a program actually relying on that
/// infrastructure — a program with none of these operations at all (e.g.
/// a Trading Core v1 atomic trade, which lowers entirely into
/// `Operation::Trading(..)` and has no concept of an RPC/relayer/solver
/// layer to begin with) has nothing here to be unsafe about. Mirrors the
/// same condition `verify_refund_path_exists` already uses.
fn has_cross_chain_operation(ir: &X3IR) -> bool {
    ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Bridge { .. } | Operation::Swap { .. } | Operation::Lock { .. }
        )
    })
}

fn verify_single_rpc(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let rpc_count = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::RpcConsensus { .. }))
        .count();
    if rpc_count == 0 {
        if has_cross_chain_operation(ir) {
            acc.add_error(err("mainnet: no RPC consensus declared — single-RPC is unsafe"));
        }
        return;
    }
    for op in &ir.operations {
        if let Operation::RpcConsensus { chain, require, .. } = op {
            if require.0 < 2 || require.1 < 2 {
                acc.add_error(err(format!(
                    "mainnet: chain '{chain}' RPC quorum {}/{} is unsafe — minimum 2_of_3 required",
                    require.0, require.1
                )));
            }
        }
    }
}

fn verify_single_relayer(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let relayer_count = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::RelayerAttest { .. }))
        .count();
    if relayer_count == 0 {
        if has_cross_chain_operation(ir) {
            acc.add_error(err(
                "mainnet: no relayer attestation declared — single-relayer is unsafe",
            ));
        }
        return;
    }
    for op in &ir.operations {
        if let Operation::RelayerAttest { relayers, quorum, .. } = op {
            if quorum.0 < 2 || quorum.1 < 2 {
                acc.add_error(err(format!(
                    "mainnet: relayer quorum {}/{} is unsafe — minimum 2_of_3 required",
                    quorum.0, quorum.1
                )));
            }
            if relayers.len() < 3 {
                acc.add_error(err(format!(
                    "mainnet: only {} relayers declared — minimum 3 required for quorum safety",
                    relayers.len()
                )));
            }
        }
    }
}

fn verify_solver_bond(ir: &X3IR, acc: &mut ErrorAccumulator) {
    // Read the bond the program actually declares: `require solver_bond >= N`.
    //
    // This used to look for a lowered `SolverBid`, which `solver_market
    // { mode, min_reputation }` produced by mapping a reputation threshold onto
    // `bond`. That op is no longer fabricated (see `lowering.rs`), so the
    // declaration the source writes is what gets checked.
    let mut saw_bond = false;
    for op in &ir.operations {
        if let Operation::Require {
            kind: crate::ir::RequireKind::SolverBond,
            condition,
            ..
        } = op
        {
            saw_bond = true;
            let declared = match condition {
                Condition::Expression { expr } => expr.trim().parse::<u128>().ok(),
                _ => None,
            };
            if declared == Some(0) {
                acc.add_error(err("mainnet: solver bond must be greater than zero"));
            }
        }
    }
    if !saw_bond && has_cross_chain_operation(ir) {
        acc.add_error(err(
            "mainnet: missing solver bond declaration — add `require solver_bond >= <amount>`",
        ));
    }
}

fn verify_known_assets(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let known: HashSet<&str> = ["USDC", "USDT", "WETH", "WBTC", "SOL", "ETH", "BTC", "X3"]
        .iter()
        .copied()
        .collect();
    for op in &ir.operations {
        match op {
            Operation::Lock { asset, .. } | Operation::Mint { asset, .. } | Operation::Burn { asset, .. } => {
                if !known.contains(asset.to_uppercase().as_str()) {
                    acc.add_error(err(format!(
                        "mainnet: unknown asset '{asset}' — must be one of: USDC, USDT, WETH, WBTC, SOL, ETH, BTC, X3"
                    )));
                }
            }
            Operation::Bridge {
                from_asset, to_asset, ..
            } => {
                if !known.contains(from_asset.to_uppercase().as_str()) {
                    acc.add_error(err(format!("mainnet: unknown from_asset '{from_asset}' in bridge")));
                }
                if !known.contains(to_asset.to_uppercase().as_str()) {
                    acc.add_error(err(format!("mainnet: unknown to_asset '{to_asset}' in bridge")));
                }
            }
            _ => {}
        }
    }
}

fn verify_slippage_safe(ir: &X3IR, acc: &mut ErrorAccumulator) {
    for op in &ir.operations {
        if let Operation::Require {
            kind: crate::ir::RequireKind::SlippageTolerance,
            condition: Condition::Expression { ref expr },
            ..
        } = op
        {
            // Compared in basis points. A `f64` comparison decides a mainnet
            // rejection, and a value one ulp either side of 5.0 would decide it
            // differently on a different runtime.
            if let Some(bps) = slippage_bps_from_text(expr).filter(|bps| *bps > SLIPPAGE_CEILING_BPS) {
                acc.add_error(err(format!(
                    "mainnet: slippage tolerance {}.{:02}% exceeds maximum 5%",
                    bps / 100,
                    bps % 100
                )));
            }
        }
    }
}

/// The slippage ceiling a mainnet run will accept, in basis points.
const SLIPPAGE_CEILING_BPS: u32 = 500;

/// Read a slippage figure as basis points, or `None` if it is not a whole
/// number of them.
///
/// This used to parse to `f64` and compare against `5.0`. The comparison decides
/// whether a program is rejected on mainnet, and a floating-point comparison
/// that decides a rejection is the ambiguity PHASE 43 asks us not to have.
/// The bound a guard states, in basis points — the unit rule every bound shares.
///
/// Used by slippage guards (`require slippage <= 50`) and by a hedge's delta guard
/// (`require delta <= 0.01%`): the quantity differs, the unit rule does not. One
/// quantity, one unit, and the two ways of writing it *agree*: a bare number is
/// **basis points** (`<= 50` is 0.5%) and a percent literal is a **percentage**
/// (`<= 0.5%` is also 50 bps). The readers did not agree — the
/// mainnet gate read a bare number as bps, the risk scorer multiplied it by 100 as
/// though it were a percent, and the strategy check compared it against a
/// `max_slippage_bps` field directly — so the corpus's most common bound,
/// `require slippage <= 50`, was 0.5% to one reader and 50% to another, and the
/// scorer reported it as `high slippage (5000bps / 50.00%)` (TICKET-054).
///
/// The rule matches the field name every declaration already uses
/// (`max_slippage_bps`), so a guard and the policy it sits under are in one unit.
pub fn bound_bps_from_expr(expr: &Expression) -> Option<u32> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => u32::try_from(*value).ok(),
        Expression::Literal(LiteralExpr::Percentage { value }) => percent_to_bps(value.as_str().trim_end_matches('%')),
        // A *bare* fractional number is basis points too: `50.0` is fifty of them,
        // and half a basis point is not a bound the VM can compare — `None`
        // rather than a rounding nobody chose.
        Expression::Literal(LiteralExpr::Float { raw, .. }) => slippage_bps_from_text(raw.as_str()),
        _ => None,
    }
}

/// The same rule for a rendered expression, which is what the IR carries.
pub fn slippage_bps_from_text(text: &str) -> Option<u32> {
    let text = text.trim();
    if let Some(percent) = text.strip_suffix('%') {
        return percent_to_bps(percent);
    }
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    if !fraction.is_empty() && fraction.chars().any(|ch| ch != '0') {
        return None;
    }
    whole.parse().ok()
}

/// `<n>[.<fraction>]` as basis points.
///
/// At most two fractional digits: `0.5` is 50 bps, `0.05` is 5 bps, and `0.005`
/// is half a basis point, which is not representable and is refused rather than
/// rounded in a direction nobody wrote down.
fn percent_to_bps(percent: &str) -> Option<u32> {
    let percent = percent.trim();
    let (whole, fraction) = percent.split_once('.').unwrap_or((percent, ""));
    let whole: u32 = whole.parse().ok()?;
    let fraction = match fraction.len() {
        0 => 0,
        1 => fraction.parse::<u32>().ok()? * 10,
        2 => fraction.parse::<u32>().ok()?,
        _ => return None,
    };
    whole.checked_mul(100)?.checked_add(fraction)
}

fn verify_deadline_bounded(ir: &X3IR, acc: &mut ErrorAccumulator) {
    // One ceiling for the whole language, from the block time that also decides
    // what a program's `40m` means.
    let max_allowed_blocks = crate::lowering::MAX_TIMEOUT_BLOCKS;
    for op in &ir.operations {
        if let Operation::OnTimeout { duration_blocks, .. } = op {
            if *duration_blocks > max_allowed_blocks {
                acc.add_error(err(format!(
                    "mainnet: timeout {duration_blocks} blocks exceeds the maximum of \
                     {max_allowed_blocks} (24 hours at {}s/block)",
                    crate::lowering::SECONDS_PER_BLOCK
                )));
            }
        }
    }
}

fn verify_manual_recovery(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let has_auto_recovery = ir.operations.iter().any(|op| match op {
        Operation::OnTimeout { action, .. } => matches!(action, FailureAction::Refund { .. }),
        Operation::OnFail { action } => matches!(action, FailureAction::Refund { .. }),
        _ => false,
    });
    let has_manual_only = ir.operations.iter().any(|op| match op {
        Operation::OnFail { action } => matches!(action, FailureAction::Halt | FailureAction::Quarantine),
        _ => false,
    });
    if has_manual_only && !has_auto_recovery {
        acc.add_error(err(
            "mainnet: manual-only recovery paths (Halt/Quarantine) without automatic refund — unsafe",
        ));
    }
}

// ───── Risk scoring ──────────────────────────────────────────────────────

/// Compute a risk score (0-100, lower = safer) for an X3IR program based
/// on its operations, metadata, and configuration.
pub fn compute_risk_score(ir: &X3IR) -> RiskScore {
    let mut score = RiskScore::default();

    // Chain risk: known chains are safer
    let known: HashSet<&str> = KNOWN_CHAINS.iter().copied().collect();
    let unknown_chains: usize = ir
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Bridge {
                from_chain, to_chain, ..
            } => {
                if !known.contains(from_chain.to_ascii_lowercase().as_str())
                    || !known.contains(to_chain.to_ascii_lowercase().as_str())
                {
                    Some(())
                } else {
                    None
                }
            }
            _ => None,
        })
        .count();
    score.chain_risk = if unknown_chains > 0 { 15 } else { 0 };

    // Bridge risk: unknown adapters
    let known_adapters: HashSet<&str> = KNOWN_BRIDGE_ADAPTERS.iter().copied().collect();
    let unknown_adapters: usize = ir
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Bridge { via, .. } if !known_adapters.contains(via.to_ascii_lowercase().as_str()) => Some(()),
            _ => None,
        })
        .count();
    score.bridge_risk = if unknown_adapters > 0 { 15 } else { 0 };

    // Solver risk: no solver bond means risk. Read the guard the program
    // writes, not the `SolverBid` op that `solver_market` used to fabricate.
    let has_solver_bond = ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Require {
                kind: crate::ir::RequireKind::SolverBond,
                ..
            }
        )
    });
    score.solver_risk = if has_solver_bond { 0 } else { 10 };

    // Relayer risk: quorum check
    let has_good_quorum = ir.operations.iter().any(|op| match op {
        Operation::RelayerAttest { quorum, .. } => quorum.0 >= 2 && quorum.1 >= 3,
        _ => false,
    });
    score.relayer_risk = if has_good_quorum { 0 } else { 10 };

    // RPC risk: quorum check
    let has_good_rpc = ir.operations.iter().any(|op| match op {
        Operation::RpcConsensus { require, .. } => require.0 >= 2 && require.1 >= 3,
        _ => false,
    });
    score.rpc_risk = if has_good_rpc { 0 } else { 10 };

    // Liquidity risk: no bridge liquidity require increases risk
    let has_liquidity_check = ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Require {
                kind: crate::ir::RequireKind::BridgeLiquidity,
                ..
            }
        )
    });
    score.liquidity_risk = if has_liquidity_check { 0 } else { 10 };

    // Finality risk: explicit finality check
    let has_finality_check = ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Require {
                kind: crate::ir::RequireKind::Finality,
                ..
            }
        )
    });
    score.finality_risk = if has_finality_check { 0 } else { 10 };

    // MEV risk: no privacy or hashlock
    let has_privacy = ir
        .operations
        .iter()
        .any(|op| matches!(op, Operation::PrivacyCommit { .. }));
    score.mev_risk = if has_privacy { 0 } else { 5 };

    // Timeout risk: bounded timeout
    let has_bounded_timeout = ir.operations.iter().any(|op| match op {
        Operation::OnTimeout { duration_blocks, .. } => *duration_blocks <= crate::lowering::MAX_TIMEOUT_BLOCKS,
        _ => false,
    });
    score.timeout_risk = if has_bounded_timeout { 0 } else { 5 };

    // Refund risk: has refund path
    let has_refund = ir.operations.iter().any(|op| match op {
        Operation::OnTimeout { action, .. } | Operation::OnFail { action } => {
            matches!(action, FailureAction::Refund { .. })
        }
        _ => false,
    });
    score.refund_risk = if has_refund { 0 } else { 10 };

    score.total = score.chain_risk
        + score.bridge_risk
        + score.solver_risk
        + score.relayer_risk
        + score.rpc_risk
        + score.liquidity_risk
        + score.finality_risk
        + score.mev_risk
        + score.timeout_risk
        + score.refund_risk;

    if score.total > 100 {
        score.total = 100;
    }

    score
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Condition, FailureAction, Operation, ProgramMetadata, RequireKind, X3IR};

    fn empty_ir() -> X3IR {
        let mut ir = X3IR::new();
        ir.metadata = ProgramMetadata {
            nonce: Some("nonce-1".into()),
            chain_id: Some(1),
            timeout_blocks: Some(30),
        };
        ir
    }

    fn atomic(ops: Vec<Operation>) -> Vec<Operation> {
        let mut v = vec![Operation::AtomicBegin];
        v.extend(ops);
        v.push(Operation::AtomicEnd);
        v
    }

    #[test]
    fn happy_path_minimal_bridge_passes() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            // A bridging program must state how final the source chain has to
            // be before the transfer is trusted.
            Operation::Require {
                kind: RequireKind::Finality,
                subject: Some("solana".into()),
                condition: Condition::Expression {
                    expr: "finality >= 12".into(),
                },
                error_msg: None,
                measured: false,
                comparison: None,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);
        assert!(verify(&ir).is_ok());
    }

    #[test]
    fn swap_without_slippage_bound_is_rejected() {
        // `min_output` is an absolute floor chosen at compile time, not a
        // tolerance for how far the market may move, so it does not satisfy
        // this guard. The refund path is present so the only thing this
        // program is missing is the slippage bound.
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Swap {
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".to_string(),
                to_asset: "ETH".into(),
                input_amount: 1_000,
                min_output: 500,
                dex: Some("uniswap".into()),
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);

        let errs = verify(&ir).expect_err("a swap with no slippage bound must not compile");
        assert!(
            errs.iter().any(|e| e.to_string().contains("explicit slippage bound")),
            "expected a slippage-bound error, got: {errs:?}"
        );
    }

    #[test]
    fn swap_with_slippage_bound_passes() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Swap {
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".to_string(),
                to_asset: "ETH".into(),
                input_amount: 1_000,
                min_output: 500,
                dex: Some("uniswap".into()),
            },
            Operation::Require {
                kind: RequireKind::SlippageTolerance,
                subject: None,
                condition: Condition::Expression { expr: "50".into() },
                error_msg: None,
                measured: false,
                comparison: None,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);

        let result = verify(&ir);
        assert!(result.is_ok(), "expected ok, got: {:?}", result.err());
    }

    #[test]
    fn bridge_without_finality_requirement_is_rejected() {
        // Regression: this used to be an `add_warning`, and `verify_with_config`
        // returns `Ok(())` whenever no *error* was accumulated — so the warning
        // was collected and then dropped. A bridge that never states a source
        // finality requirement compiled silently, which is precisely how a
        // reorged source lock turns into an unbacked destination mint.
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);

        let errs = verify(&ir).expect_err("a bridge with no finality requirement must not compile");
        assert!(
            errs.iter()
                .any(|e| e.to_string().contains("no explicit finality requirement")),
            "expected a finality-requirement error, got: {errs:?}"
        );
    }

    /// The operation order intent lowering actually produces: the `from`/`to`
    /// endpoints and the failure handler sit *outside* the atomic route.
    fn intent_shaped_ir() -> X3IR {
        let mut ir = empty_ir();
        ir.operations = vec![
            Operation::IntentResolve {
                constraints: vec![],
                resolver: "swap_demo".into(),
            },
            Operation::Lock {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                amount: 100,
                from: "0x1111".into(),
            },
            // The `to` endpoint.
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "4Nd1".into(),
                act: ReleaseAct::Claims(0),
            },
            Operation::AtomicBegin,
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "ethereum".into(),
                from_asset: "USDC".into(),
                to_chain: "solana".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "4Nd1".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::AtomicEnd,
            Operation::Require {
                kind: RequireKind::Finality,
                subject: Some("ethereum".into()),
                condition: Condition::Expression { expr: "12".into() },
                error_msg: None,
                measured: false,
                comparison: None,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "ethereum".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
            // The refund's release target.
            Operation::Release {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                to: "sender".into(),
                act: ReleaseAct::Payout,
            },
            Operation::OnFail {
                action: FailureAction::Rollback,
            },
        ];
        ir
    }

    /// Run every builtin invariant rule and collect the violations.
    fn invariant_violations(ir: &X3IR) -> Vec<String> {
        let mut violations = Vec::new();
        for rule in get_builtin_invariants() {
            if let Err(msg) = (rule.check_fn)(ir) {
                violations.push(format!("{}: {msg}", rule.name));
            }
        }
        violations
    }

    #[test]
    fn builtin_invariants_accept_the_intent_ir_shape() {
        // Regression: before the rules were scoped to atomic route bodies, this
        // shape tripped four of the six rules. Every well-formed bridging intent
        // in the repo looked broken, which is the same as the rules being
        // broken.
        let violations = invariant_violations(&intent_shaped_ir());
        assert!(
            violations.is_empty(),
            "a well-formed intent must not violate any builtin invariant, got: {violations:?}"
        );
    }

    #[test]
    fn invariants_still_catch_two_claims_inside_one_route() {
        // Non-vacuous: scoping the rules must not turn them off.
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            // Two claims of the *same* lock: one lock claimed twice is the violation the rule
            // names, and two claims of two different locks are what TICKET-080 made possible.
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "a".into(),
                act: ReleaseAct::Claims(0),
            },
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "b".into(),
                act: ReleaseAct::Claims(0),
            },
        ]);
        assert!(
            invariant_violations(&ir)
                .iter()
                .any(|v| v.starts_with("no_double_claim")),
            "two claims in one atomic route must be reported"
        );
    }

    #[test]
    fn invariants_still_catch_a_refund_after_a_claim_inside_one_route() {
        // The escrow is locked first, because that is the scenario the rule is
        // about: a *claim* releases an escrow this program created, and refunding
        // that escrow afterwards is the double-spend. The fixture used to omit the
        // `Lock`, which made it indistinguishable from the two-legged payout the
        // rule used to false-positive on (TICKET-035) — the assertion was right and
        // the program it was asserted against was not.
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Lock {
                chain: "solana".into(),
                asset: "USDC".into(),
                amount: 100,
                from: "0x1111".into(),
            },
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "a".into(),
                act: ReleaseAct::Claims(0),
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);
        assert!(
            invariant_violations(&ir)
                .iter()
                .any(|v| v.starts_with("no_refund_after_claim")),
            "a refund of a claimed escrow in one atomic route must be reported"
        );
    }

    #[test]
    fn a_two_legged_swap_pays_out_and_refunds_different_assets() {
        // The shape `examples/atomic_swap.x3` has: one escrow is locked and can be
        // refunded, the destination asset is paid out with no lock of its own, and
        // each leg's timeout refund is the other settlement path of that leg. The
        // payout is not a claim of an escrow, and the rule must not read it as one
        // — it warned on every build of the roadmap's canonical example
        // (TICKET-035).
        let mut ir = empty_ir();
        ir.operations = vec![
            Operation::Lock {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                amount: 100,
                from: "0x1111".into(),
            },
            Operation::AtomicBegin,
            Operation::Release {
                chain: "solana".into(),
                asset: "SOL".into(),
                to: "4Nd1".into(),
                // The destination asset's payout, which claims no lock — which is what this
                // fixture exists to say, in the IR's own terms now rather than by proxy.
                act: ReleaseAct::Payout,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "ethereum".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "SOL".into(),
                    to: "sender".into(),
                },
            },
            Operation::AtomicEnd,
        ];
        assert_eq!(
            invariant_violations(&ir)
                .iter()
                .filter(|violation| violation.starts_with("no_refund_after_claim"))
                .count(),
            0,
            "a two-legged swap is not a claim-then-refund: {:?}",
            invariant_violations(&ir)
        );
    }

    #[test]
    fn claim_before_bridge_is_only_flagged_when_the_route_bridges() {
        // A same-chain route releasing its own escrow has no fill to order
        // against, so the rule has nothing to say about it...
        let mut same_chain = empty_ir();
        same_chain.operations = atomic(vec![Operation::Release {
            chain: "solana".into(),
            asset: "USDC".into(),
            to: "a".into(),
            act: ReleaseAct::Payout,
        }]);
        assert!(
            !invariant_violations(&same_chain)
                .iter()
                .any(|v| v.starts_with("destination_fill_before_source_claim")),
            "a route with no bridge must not be flagged for fill ordering"
        );

        // ...but a claim that precedes the route's bridge still is.
        let mut bridged = empty_ir();
        bridged.operations = atomic(vec![
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "a".into(),
                act: ReleaseAct::Payout,
            },
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "ethereum".into(),
                from_asset: "USDC".into(),
                to_chain: "solana".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "a".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
        ]);
        assert!(
            invariant_violations(&bridged)
                .iter()
                .any(|v| v.starts_with("destination_fill_before_source_claim")),
            "a claim before the route's bridge must be reported"
        );
    }

    #[test]
    fn no_double_refund_counts_on_fail_refunds_as_well_as_timeout_refunds() {
        // "At most one refund handler" is the property; counting only timeout
        // handlers missed an on-fail refund alongside a timeout refund.
        let mut ir = empty_ir();
        ir.operations = vec![
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
            Operation::OnFail {
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ];
        assert!(
            invariant_violations(&ir)
                .iter()
                .any(|v| v.starts_with("no_double_refund")),
            "two refund handlers must be reported regardless of which handler kind they are"
        );
    }

    /// A locking + bridging program, optionally declaring proofs.
    fn cross_chain_with_proofs(declared: &[&str]) -> X3IR {
        let mut ir = empty_ir();
        ir.operations = vec![
            Operation::Lock {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                amount: 100,
                from: "0x1".into(),
            },
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "ethereum".into(),
                from_asset: "USDC".into(),
                to_chain: "solana".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "4Nd1".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
        ];
        for proof in declared {
            ir.operations.push(Operation::ProofRequired {
                proof_type: (*proof).to_string(),
                source: "intent".into(),
            });
        }
        ir
    }

    #[test]
    fn proof_requirement_accepts_the_vocabulary_real_programs_use() {
        // The check demanded `lock_proof` / `fill_proof` / `claim_proof`, exact
        // names that appear in zero `.x3` files, so every bridging program got
        // warnings it could not act on. The names the repository's own programs
        // declare must satisfy it.
        let outcome = verify_collect(
            &cross_chain_with_proofs(&["source_lock_proof", "destination_fill_proof"]),
            DEFAULT_MAX_ATOMIC_OPS,
            DEFAULT_MAX_ROUTE_HOPS,
            None,
        );
        let proof_warnings: Vec<String> = outcome
            .warnings
            .iter()
            .map(|w| w.to_string())
            .filter(|w| w.contains("lock proof") || w.contains("destination-fill proof"))
            .collect();
        assert!(
            proof_warnings.is_empty(),
            "declaring source_lock_proof and destination_fill_proof must satisfy the check, got: {proof_warnings:?}"
        );
    }

    #[test]
    fn proof_requirement_still_warns_when_nothing_is_declared() {
        // Non-vacuous: a program that depends on a lock and a fill but declares
        // no proof must still be told.
        let outcome = verify_collect(
            &cross_chain_with_proofs(&[]),
            DEFAULT_MAX_ATOMIC_OPS,
            DEFAULT_MAX_ROUTE_HOPS,
            None,
        );
        let messages: Vec<String> = outcome.warnings.iter().map(|w| w.to_string()).collect();
        assert!(
            messages.iter().any(|w| w.contains("lock proof")),
            "a missing lock proof must be reported, got: {messages:?}"
        );
        assert!(
            messages.iter().any(|w| w.contains("destination-fill proof")),
            "a missing destination-fill proof must be reported, got: {messages:?}"
        );
    }

    #[test]
    fn the_proof_obligation_is_an_error_on_mainnet_and_a_warning_elsewhere() {
        // TICKET-020 and TICKET-024 asked for this decision to be made and
        // written down. It is the same program in two modes, so the difference is
        // the severity and nothing else.
        let ir = cross_chain_with_proofs(&[]);

        let dev = verify_collect(&ir, DEFAULT_MAX_ATOMIC_OPS, DEFAULT_MAX_ROUTE_HOPS, None);
        assert!(
            dev.errors.iter().all(|error| !error.to_string().contains("lock proof")),
            "a development build must not fail on a missing proof declaration: {:?}",
            dev.errors
        );
        // The fixture is minimal, so it has other errors; this test is about the
        // proof obligation's severity, and asks about that alone.
        let warnings: Vec<String> = dev.warnings.iter().map(|warning| warning.to_string()).collect();
        assert!(
            warnings.iter().any(|warning| warning.contains("lock proof")),
            "but it must still say so: {warnings:?}"
        );

        let mainnet = verify_collect(
            &ir,
            DEFAULT_MAX_ATOMIC_OPS,
            DEFAULT_MAX_ROUTE_HOPS,
            Some(CompilationMode::Mainnet),
        );
        let errors: Vec<String> = mainnet.errors.iter().map(|error| error.to_string()).collect();
        assert!(
            errors
                .iter()
                .any(|error| error.contains("mainnet:") && error.contains("lock proof")),
            "on mainnet the obligation is a requirement: {errors:?}"
        );
        assert!(
            mainnet
                .warnings
                .iter()
                .all(|warning| !warning.to_string().contains("lock proof")),
            "and it is not also a warning there — one finding, one severity: {:?}",
            mainnet.warnings
        );
    }

    #[test]
    fn a_same_chain_lock_does_not_demand_a_cross_chain_proof() {
        // The requirement used to trigger on `Lock`, and lowering emits a `Lock`
        // for a same-chain transfer. So an intent that never leaves a chain was
        // told to prove it had locked funds on a chain it never left — a warning
        // no correct program could act on, which is how a check stops being read
        // (TICKET-024).
        let mut ir = empty_ir();
        ir.operations = vec![
            Operation::Lock {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                amount: 100,
                from: "0x1".into(),
            },
            Operation::Release {
                chain: "ethereum".into(),
                asset: "ETH".into(),
                to: "0x1".into(),
                act: ReleaseAct::Claims(0),
            },
        ];
        let outcome = verify_collect(&ir, DEFAULT_MAX_ATOMIC_OPS, DEFAULT_MAX_ROUTE_HOPS, None);
        let messages: Vec<String> = outcome.warnings.iter().map(|w| w.to_string()).collect();
        assert!(
            !messages.iter().any(|w| w.contains("proof")),
            "a same-chain lock has no cross-chain proof to require, got: {messages:?}"
        );
    }

    #[test]
    fn a_bridging_program_needs_both_a_lock_and_a_fill_proof() {
        // The other half of the gate: gating on `has_bridge` must not have
        // silenced the requirement for programs that do cross a chain.
        let outcome = verify_collect(
            &cross_chain_with_proofs(&[]),
            DEFAULT_MAX_ATOMIC_OPS,
            DEFAULT_MAX_ROUTE_HOPS,
            None,
        );
        let messages: Vec<String> = outcome.warnings.iter().map(|w| w.to_string()).collect();
        assert!(
            messages.iter().any(|w| w.contains("source-lock proof")),
            "a bridge without a source-lock proof must be reported, got: {messages:?}"
        );
        assert!(
            messages.iter().any(|w| w.contains("destination-fill proof")),
            "a bridge without a destination-fill proof must be reported, got: {messages:?}"
        );
    }

    #[test]
    fn an_unrelated_proof_does_not_satisfy_the_lock_or_fill_requirement() {
        // A declaration that names neither a lock nor a fill must not silence
        // the check.
        let outcome = verify_collect(
            &cross_chain_with_proofs(&["solver_signature"]),
            DEFAULT_MAX_ATOMIC_OPS,
            DEFAULT_MAX_ROUTE_HOPS,
            None,
        );
        let messages: Vec<String> = outcome.warnings.iter().map(|w| w.to_string()).collect();
        assert!(
            messages.iter().any(|w| w.contains("lock proof"))
                && messages.iter().any(|w| w.contains("destination-fill proof")),
            "an unrelated proof declaration must not satisfy either requirement, got: {messages:?}"
        );
    }

    /// A bridging program with every error-level guard satisfied but no
    /// `proofs required` declaration.
    fn fully_guarded_bridge() -> X3IR {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::Require {
                kind: RequireKind::Finality,
                subject: Some("solana".into()),
                condition: Condition::Expression {
                    expr: "finality >= 12".into(),
                },
                error_msg: None,
                measured: false,
                comparison: None,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);
        ir
    }

    #[test]
    fn every_semantic_pass_is_in_the_registry() {
        // "Which passes exist" and "which passes run" have to be the same list.
        // A pass that is written and never registered runs against nothing while
        // looking wired, and no test of that pass would notice — it would be
        // testing a function the pipeline never calls.
        //
        // A `fn verify_*` that is *not* registered is allowed only if something
        // else in this file calls it: the passes run by `verify_mainnet_safe`,
        // and the entry points themselves.
        let source = include_str!("semantic.rs");
        let registered: Vec<&str> = SEMANTIC_PASSES.iter().map(|(name, _)| *name).collect();

        // References are looked for across the whole crate, because an entry
        // point or an AST-level pass is legitimately called from `lib.rs`, not
        // from this file. Test functions inside `mod tests` are not passes, so
        // definitions are only read above that marker.
        let src_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut other_sources = String::new();
        if let Ok(entries) = std::fs::read_dir(&src_root) {
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|extension| extension == "rs") {
                    other_sources.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
                    other_sources.push('\n');
                }
            }
        }
        let definitions = source.split("#[cfg(test)]").next().unwrap_or(source);

        let mut orphans = Vec::new();
        for line in definitions.lines() {
            let trimmed = line.trim_start();
            let Some(rest) = trimmed.strip_prefix("pub fn ").or_else(|| trimmed.strip_prefix("fn ")) else {
                continue;
            };
            if !rest.starts_with("verify_") {
                continue;
            }
            let name = rest.split(['(', '<']).next().unwrap_or_default().trim();
            if name.is_empty() || registered.contains(&name) {
                continue;
            }

            let references = other_sources
                .lines()
                .filter(|other| !other.trim_start().starts_with("//"))
                .filter(|other| other.contains(name))
                .count();
            if references == 0 {
                orphans.push(name.to_string());
            }
        }

        assert!(
            orphans.is_empty(),
            "these passes exist, are named like pipeline passes, and are in neither the registry \
             nor any caller's body: {orphans:?}"
        );
    }

    #[test]
    fn verify_collect_surfaces_warnings_that_verify_hides() {
        // `verify_with_config` returns `Ok(())` whenever no error was
        // accumulated, so every warning it collects is discarded. This program
        // passes `verify` while the proof-requirement pass has something to
        // say; `verify_collect` is the entry point that keeps that signal.
        let ir = fully_guarded_bridge();

        assert!(verify(&ir).is_ok(), "this program has no error-level violation");

        let outcome = verify_collect(&ir, DEFAULT_MAX_ATOMIC_OPS, DEFAULT_MAX_ROUTE_HOPS, None);
        assert!(outcome.is_ok(), "warnings must not fail the program");
        assert!(
            outcome.warnings.iter().any(|w| w.to_string().contains("fill_proof")),
            "the unfulfilled proof requirement must be visible as a warning, got: {:?}",
            outcome.warnings
        );
    }

    #[test]
    fn verify_collect_reports_errors_separately_from_warnings() {
        // Same program, minus the finality requirement: the violation must land
        // in `errors`, not be softened into a warning.
        let mut ir = fully_guarded_bridge();
        ir.operations.retain(|op| {
            !matches!(
                op,
                Operation::Require {
                    kind: RequireKind::Finality,
                    ..
                }
            )
        });

        let outcome = verify_collect(&ir, DEFAULT_MAX_ATOMIC_OPS, DEFAULT_MAX_ROUTE_HOPS, None);
        assert!(!outcome.is_ok(), "a missing finality requirement must fail the program");
        assert!(
            outcome
                .errors
                .iter()
                .any(|e| e.to_string().contains("no explicit finality requirement")),
            "expected a finality error, got: {:?}",
            outcome.errors
        );
    }

    #[test]
    fn finality_requirement_for_the_wrong_chain_does_not_satisfy_the_guard() {
        // The chain named in the requirement is what counts — declaring
        // finality for some other chain must not wave the bridge through.
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::Require {
                kind: RequireKind::Finality,
                subject: Some("ethereum".into()),
                condition: Condition::Expression {
                    expr: "finality >= 12".into(),
                },
                error_msg: None,
                measured: false,
                comparison: None,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);

        let errs = verify(&ir).expect_err("finality for an unrelated chain must not satisfy the guard");
        assert!(
            errs.iter()
                .any(|e| e.to_string().contains("no explicit finality requirement")),
            "expected a finality-requirement error, got: {errs:?}"
        );
    }

    #[test]
    fn cross_vm_outside_atomic_is_rejected() {
        let mut ir = empty_ir();
        ir.operations = vec![Operation::Bridge {
            via: "x3".into(),
            from_chain: "solana".into(),
            from_asset: "USDC".into(),
            to_chain: "ethereum".into(),
            to_asset: "USDC".into(),
            amount: 100,
            receiver: "0xabc".into(),
            source_finality_proof: vec![],
            transfer_proof: vec![],
        }];
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs
            .iter()
            .any(|e| e.to_string().contains("not inside an atomic block")));
    }

    #[test]
    fn bridge_without_timeout_is_rejected() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![Operation::Bridge {
            via: "x3".into(),
            from_chain: "solana".into(),
            from_asset: "USDC".into(),
            to_chain: "ethereum".into(),
            to_asset: "USDC".into(),
            amount: 100,
            receiver: "0xabc".into(),
            source_finality_proof: vec![],
            transfer_proof: vec![],
        }]);
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("OnTimeout")));
    }

    #[test]
    fn bridge_without_nonce_is_rejected() {
        let mut ir = X3IR::new(); // no nonce
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Rollback,
            },
        ]);
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("nonce")));
    }

    #[test]
    fn unknown_bridge_via_is_rejected() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "rogue-bridge".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Rollback,
            },
        ]);
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("adapter allow-list")));
    }

    #[test]
    fn same_chain_bridge_is_rejected() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "ethereum".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Rollback,
            },
        ]);
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("from_chain == to_chain")));
    }

    #[test]
    fn unmatched_atomic_begin_is_rejected() {
        let mut ir = empty_ir();
        ir.operations = vec![
            Operation::AtomicBegin,
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
        ];
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("unmatched AtomicBegin")));
    }

    #[test]
    fn zero_amount_bridge_is_rejected() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 0,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Rollback,
            },
        ]);
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("zero amount")));
    }

    #[test]
    fn require_with_statically_false_condition_is_rejected() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Require {
                kind: RequireKind::Finality,
                subject: None,
                condition: Condition::False,
                error_msg: Some("never reachable".into()),
                measured: false,
                comparison: None,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Rollback,
            },
        ]);
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("statically false")));
    }

    #[test]
    fn unsafe_symbol_is_rejected() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC; rm -rf /".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Rollback,
            },
        ]);
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("unsafe characters")));
    }

    #[test]
    fn route_depth_over_limit_is_rejected() {
        let mut ir = empty_ir();
        // Six bridges in one block — should be fine (limit 8).
        // But seven in one block must be fine too. Make it 9 to trip the
        // max=8 default.
        let mut ops = vec![];
        for _ in 0..9 {
            ops.push(Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 1,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            });
        }
        ops.push(Operation::OnTimeout {
            duration_blocks: 30,
            action: FailureAction::Rollback,
        });
        ir.operations = atomic(ops);
        let errs = verify(&ir).expect_err("must fail");
        assert!(errs
            .iter()
            .any(|e| e.to_string().contains("max 8") || e.to_string().contains("max 4")));
    }

    // ========== Atomic swap AST-level validation tests ==========

    use x3_lang_ast::ast::{AssetRef, AtomicSwapDecl, ChainRef, HashlockSpec, Item, Program};
    use x3_lang_common::{Span, Spanned};

    fn make_swap_program(decl: AtomicSwapDecl) -> Program {
        Program {
            items: vec![Spanned::dummy(Item::AtomicSwap(decl))],
        }
    }

    fn valid_atomic_swap() -> AtomicSwapDecl {
        AtomicSwapDecl {
            name: "test_swap".into(),
            from_asset: AssetRef::new(ChainRef("eth".into()), "USDC".into()),
            to_asset: AssetRef::new(ChainRef("sol".into()), "USDC".into()),
            source_vm: None,
            dest_vm: None,
            amount: Some(Expression::Literal(LiteralExpr::Int {
                value: 100,
                base: x3_lang_common::IntBase::Decimal,
                suffix: None,
            })),
            receiver: None,
            hashlock: Some(HashlockSpec {
                hash_fn: "sha256".into(),
                secret: Box::new(Expression::Literal(LiteralExpr::String("my_secret".into()))),
            }),
            body: vec![],
            requires: vec![],
            on_fail: None,
            timeout_source: Some(Expression::Literal(LiteralExpr::Duration {
                value: 3600,
                unit: x3_lang_common::DurationUnit::Seconds,
            })),
            timeout_destination: Some(Expression::Literal(LiteralExpr::Duration {
                value: 1800,
                unit: x3_lang_common::DurationUnit::Seconds,
            })),
        }
    }

    #[test]
    fn test_valid_atomic_swap_passes() {
        let mut acc = ErrorAccumulator::new();
        let program = make_swap_program(valid_atomic_swap());
        verify_atomic_swap_decls(&program, &mut acc);
        assert!(!acc.has_errors(), "expected no errors, got: {:?}", acc.take_errors());
    }

    #[test]
    fn test_unknown_source_chain_rejected() {
        let mut decl = valid_atomic_swap();
        decl.from_asset = AssetRef::new(ChainRef("unknown_chain".into()), "USDC".into());
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(errs.iter().any(|e| e.to_string().contains("Unknown chain")));
    }

    #[test]
    fn test_unknown_dest_chain_rejected() {
        let mut decl = valid_atomic_swap();
        decl.to_asset = AssetRef::new(ChainRef("not_a_chain".into()), "USDC".into());
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(errs.iter().any(|e| e.to_string().contains("Unknown chain")));
    }

    #[test]
    fn test_same_chain_rejected() {
        let mut decl = valid_atomic_swap();
        decl.to_asset = AssetRef::new(ChainRef("eth".into()), "USDC".into());
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(errs
            .iter()
            .any(|e| e.to_string().contains("must be between different chains")));
    }

    #[test]
    fn test_unknown_hash_function_rejected() {
        let mut decl = valid_atomic_swap();
        decl.hashlock = Some(HashlockSpec {
            hash_fn: "md5".into(),
            secret: Box::new(Expression::Literal(LiteralExpr::String("secret".into()))),
        });
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(errs.iter().any(|e| e.to_string().contains("Unknown hash function")));
    }

    #[test]
    fn test_zero_amount_rejected() {
        let mut decl = valid_atomic_swap();
        decl.amount = Some(Expression::Literal(LiteralExpr::Int {
            value: 0,
            base: x3_lang_common::IntBase::Decimal,
            suffix: None,
        }));
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(errs.iter().any(|e| e.to_string().contains("amount must be positive")));
    }

    #[test]
    fn test_timeout_ordering_rejected() {
        let mut decl = valid_atomic_swap();
        decl.timeout_source = Some(Expression::Literal(LiteralExpr::Duration {
            value: 100,
            unit: x3_lang_common::DurationUnit::Seconds,
        }));
        decl.timeout_destination = Some(Expression::Literal(LiteralExpr::Duration {
            value: 500,
            unit: x3_lang_common::DurationUnit::Seconds,
        }));
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(errs.iter().any(|e| e.to_string().contains("Source timeout")));
    }

    #[test]
    fn test_timeout_equal_rejected() {
        let mut decl = valid_atomic_swap();
        decl.timeout_source = Some(Expression::Literal(LiteralExpr::Duration {
            value: 300,
            unit: x3_lang_common::DurationUnit::Seconds,
        }));
        decl.timeout_destination = Some(Expression::Literal(LiteralExpr::Duration {
            value: 300,
            unit: x3_lang_common::DurationUnit::Seconds,
        }));
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(errs.iter().any(|e| e.to_string().contains("Source timeout")));
    }

    #[test]
    fn test_finality_require_missing_subject_rejected() {
        let mut decl = valid_atomic_swap();
        decl.requires = vec![x3_lang_ast::ast::RequireGuard {
            kind: x3_lang_ast::ast::RequireKind::Finality,
            subject: None,
            comparison: None,
            value: Some(Expression::Literal(LiteralExpr::Int {
                value: 12,
                base: x3_lang_common::IntBase::Decimal,
                suffix: None,
            })),
        }];
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(
            errs.iter().any(|e| e.to_string().contains("chain subject")),
            "expected 'needs a chain subject', got: {errs:?}"
        );
    }

    #[test]
    fn test_relayer_quorum_require_accepts_valid() {
        let mut decl = valid_atomic_swap();
        decl.requires = vec![x3_lang_ast::ast::RequireGuard {
            kind: x3_lang_ast::ast::RequireKind::RelayerQuorum,
            subject: None,
            comparison: None,
            value: Some(Expression::Literal(LiteralExpr::Int {
                value: 3,
                base: x3_lang_common::IntBase::Decimal,
                suffix: None,
            })),
        }];
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        assert!(
            !acc.has_errors(),
            "expected no errors for valid relayer_quorum, got: {:?}",
            acc.take_errors()
        );
    }

    #[test]
    fn test_relayer_quorum_zero_rejected() {
        let mut decl = valid_atomic_swap();
        decl.requires = vec![x3_lang_ast::ast::RequireGuard {
            kind: x3_lang_ast::ast::RequireKind::RelayerQuorum,
            subject: None,
            comparison: None,
            value: Some(Expression::Literal(LiteralExpr::Int {
                value: 0,
                base: x3_lang_common::IntBase::Decimal,
                suffix: None,
            })),
        }];
        let mut acc = ErrorAccumulator::new();
        verify_atomic_swap_decls(&make_swap_program(decl), &mut acc);
        let errs = acc.take_errors();
        assert!(
            errs.iter()
                .any(|e| e.to_string().contains("relayer_quorum must be positive")),
            "expected 'relayer_quorum must be positive', got: {errs:?}"
        );
    }

    // ───── B-52 B-52 feature lock tests ─────────────────────────────────

    #[test]
    fn refund_path_missing_is_detected() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            // No OnFail or OnTimeout with Refund
        ]);
        let errs = verify_with_config(&ir, 8, 4, None).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("refund path")));
    }

    #[test]
    fn refund_path_present_passes() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::Require {
                kind: RequireKind::Finality,
                subject: Some("solana".into()),
                condition: Condition::Expression {
                    expr: "finality >= 12".into(),
                },
                error_msg: None,
                measured: false,
                comparison: None,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);
        let result = verify_with_config(&ir, 8, 4, None);
        assert!(result.is_ok(), "expected ok, got: {:?}", result.err());
    }

    #[test]
    fn route_score_weights_sum_to_100() {
        let mut ir = empty_ir();
        let mut weights = std::collections::BTreeMap::new();
        weights.insert("speed".to_string(), 50);
        weights.insert("cost".to_string(), 50);
        ir.operations = atomic(vec![
            Operation::RouteScore {
                strategy: "best".into(),
                weights,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Rollback,
            },
        ]);
        let result = verify_with_config(&ir, 8, 4, None);
        assert!(result.is_ok(), "expected ok, got: {:?}", result.err());
    }

    #[test]
    fn route_score_weights_wrong_total_rejected() {
        let mut ir = empty_ir();
        let mut weights = std::collections::BTreeMap::new();
        weights.insert("speed".to_string(), 30);
        ir.operations = atomic(vec![
            Operation::RouteScore {
                strategy: "bad".into(),
                weights,
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Rollback,
            },
        ]);
        let errs = verify_with_config(&ir, 8, 4, None).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("sum to 30")));
    }

    // ───── Mainnet safety tests ──────────────────────────────────────────

    #[test]
    fn mainnet_rejects_single_rpc() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
            // No RpcConsensus
        ]);
        let errs = verify_with_config(&ir, 8, 4, Some(CompilationMode::Mainnet)).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("no RPC consensus")));
    }

    #[test]
    fn mainnet_rejects_missing_refund_path() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::RpcConsensus {
                chain: "solana".into(),
                require: (2, 3),
                reject_on: vec![],
            },
            // No OnFail/OnTimeout with Refund
        ]);
        let errs = verify_with_config(&ir, 8, 4, Some(CompilationMode::Mainnet)).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("refund path")));
    }

    #[test]
    fn mainnet_rejects_unsafe_slippage() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::Require {
                kind: RequireKind::SlippageTolerance,
                subject: None,
                // 600 basis points = 6%, over the 5% ceiling. This said `10.0`,
                // which was read as 10% when the gate took a bare number for a
                // percentage; a bare number is basis points (TICKET-054), so the
                // fixture names a bound that is over the ceiling either way.
                condition: Condition::Expression { expr: "600".into() },
                error_msg: Some("slippage".into()),
                measured: false,
                comparison: None,
            },
            Operation::RpcConsensus {
                chain: "solana".into(),
                require: (2, 3),
                reject_on: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);
        let errs = verify_with_config(&ir, 8, 4, Some(CompilationMode::Mainnet)).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("slippage")));
    }

    #[test]
    fn mainnet_rejects_unbounded_deadline() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::RpcConsensus {
                chain: "solana".into(),
                require: (2, 3),
                reject_on: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 999999,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);
        let errs = verify_with_config(&ir, 8, 4, Some(CompilationMode::Mainnet)).expect_err("must fail");
        assert!(errs.iter().any(|e| e.to_string().contains("timeout")));
    }

    // ───── Invariant detection tests ─────────────────────────────────────

    #[test]
    fn invariant_no_double_claim_detects_violation() {
        let invariants = get_builtin_invariants();
        let mut ir = empty_ir();
        ir.operations = vec![
            Operation::AtomicBegin,
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "alice".into(),
                act: ReleaseAct::Claims(0),
            },
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "bob".into(),
                // The same lock, claimed a second time — which is the violation, and the
                // index is how the rule can now tell that from two claims of two locks.
                act: ReleaseAct::Claims(0),
            },
            Operation::AtomicEnd,
        ];
        let violations = verify_invariants_on_intent(&ir, &invariants);
        assert!(violations.iter().any(|v| v.contains("no_double_claim")));
    }

    #[test]
    fn invariant_no_claim_after_refund_detects_violation() {
        let invariants = get_builtin_invariants();
        let mut ir = empty_ir();
        ir.operations = vec![
            Operation::AtomicBegin,
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::OnTimeout {
                duration_blocks: 30,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "alice".into(),
                act: ReleaseAct::Payout,
            },
            Operation::AtomicEnd,
        ];
        let violations = verify_invariants_on_intent(&ir, &invariants);
        assert!(violations.iter().any(|v| v.contains("no_claim_after_refund")));
    }

    // ───── Risk score tests ──────────────────────────────────────────────

    #[test]
    fn risk_score_safe_intent_is_low() {
        let mut ir = empty_ir();
        let mut weights = std::collections::BTreeMap::new();
        weights.insert("speed".to_string(), 50);
        weights.insert("cost".to_string(), 50);
        ir.operations = atomic(vec![
            Operation::Bridge {
                via: "x3".into(),
                from_chain: "solana".into(),
                from_asset: "USDC".into(),
                to_chain: "ethereum".into(),
                to_asset: "USDC".into(),
                amount: 100,
                receiver: "0xabc".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::RouteScore {
                strategy: "best".into(),
                weights,
            },
            Operation::SolverBid {
                solver: "solver1".into(),
                receive_asset: "USDC".into(),
                deliver_asset: "USDC".into(),
                fee: "0.1%".into(),
                bond: 1000,
            },
            Operation::RelayerAttest {
                relayers: vec!["a".into(), "b".into(), "c".into()],
                quorum: (2, 3),
                signatures: vec![],
            },
            Operation::RpcConsensus {
                chain: "solana".into(),
                require: (2, 3),
                reject_on: vec![],
            },
            Operation::Require {
                kind: RequireKind::BridgeLiquidity,
                subject: None,
                condition: Condition::True,
                error_msg: None,
                measured: false,
                comparison: None,
            },
            Operation::Require {
                kind: RequireKind::Finality,
                subject: Some("solana".into()),
                condition: Condition::True,
                error_msg: None,
                measured: false,
                comparison: None,
            },
            Operation::PrivacyCommit {
                reveal_on: "fill".into(),
                encrypted: true,
            },
            Operation::OnTimeout {
                duration_blocks: 100,
                action: FailureAction::Refund {
                    chain: "solana".into(),
                    asset: "USDC".into(),
                    to: "sender".into(),
                },
            },
        ]);
        let score = compute_risk_score(&ir);
        assert!(
            score.total <= 10,
            "expected safe intent total <= 10, got {}",
            score.total
        );
    }

    #[test]
    fn risk_score_risky_intent_is_high() {
        let mut ir = empty_ir();
        ir.operations = atomic(vec![Operation::Bridge {
            via: "unknown-bridge".into(),
            from_chain: "unknown-chain".into(),
            from_asset: "SHITCOIN".into(),
            to_chain: "ethereum".into(),
            to_asset: "USDC".into(),
            amount: 100,
            receiver: "0xabc".into(),
            source_finality_proof: vec![],
            transfer_proof: vec![],
        }]);
        let score = compute_risk_score(&ir);
        assert!(
            score.total >= 50,
            "expected risky intent total >= 50, got {}",
            score.total
        );
    }
}

#[cfg(test)]
mod refund_path_tests {
    use super::*;

    fn violations(operations: Vec<Operation>) -> Vec<String> {
        // Built here rather than through the `tests` module's helper, which is private
        // to that module.
        let ir = crate::ir::X3IR {
            operations,
            metadata: crate::ir::ProgramMetadata {
                nonce: Some("nonce-1".to_owned()),
                chain_id: Some(1),
                timeout_blocks: Some(10),
            },
        };
        let mut acc = ErrorAccumulator::new();
        verify_refund_path_exists(&ir, &mut acc);
        acc.errors().iter().map(|error| format!("{error}")).collect()
    }

    /// A route that locks an escrow and claims it again has its refund path in its own
    /// rollback: a failed route records no asset operations, so the lock never took
    /// effect. Requiring a handler there as well is not stricter, it is contradictory —
    /// a handler refunding an escrow the same route claims is refused by
    /// `no_refund_after_claim`.
    #[test]
    fn a_lock_claimed_inside_its_own_route_needs_no_refund_handler() {
        let found = violations(vec![
            Operation::AtomicBegin,
            Operation::Lock {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                amount: 120,
                from: "0xA1".into(),
            },
            Operation::Release {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                to: "0xB1".into(),
                act: ReleaseAct::Claims(0),
            },
            Operation::AtomicEnd,
        ]);
        assert!(found.is_empty(), "the rollback is the refund path: {found:?}");
    }

    /// Non-vacuous: the exemption is for a lock *claimed in its own route*, and a lock
    /// that is neither handled nor claimed is still refused — it leaves the program's
    /// control with nothing to bring it back.
    #[test]
    fn a_lock_with_neither_handler_nor_claim_in_its_route_is_still_refused() {
        let found = violations(vec![
            Operation::AtomicBegin,
            Operation::Lock {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                amount: 120,
                from: "0xA1".into(),
            },
            Operation::AtomicEnd,
        ]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("no way back") && found[0].contains("Refund"),
            "the refusal must say what to add: {found:?}"
        );
    }

    /// A release in a *different* asset is not a claim of this lock, so the lock is
    /// still unclaimed and still needs a way back.
    #[test]
    fn a_release_of_another_asset_does_not_cover_the_lock() {
        let found = violations(vec![
            Operation::AtomicBegin,
            Operation::Lock {
                chain: "ethereum".into(),
                asset: "USDC".into(),
                amount: 120,
                from: "0xA1".into(),
            },
            Operation::Release {
                chain: "ethereum".into(),
                asset: "ETH".into(),
                to: "0xB1".into(),
                act: ReleaseAct::Claims(0),
            },
            Operation::AtomicEnd,
        ]);
        assert_eq!(found.len(), 1, "a different escrow is a different escrow: {found:?}");
    }

    /// A bridge's far side is another chain's state and a rollback cannot reach it, so
    /// the explicit handler is still required. This is the half of the rule the
    /// rollback must not absorb.
    #[test]
    fn a_bridge_still_needs_an_explicit_refund_handler() {
        let found = violations(vec![
            Operation::AtomicBegin,
            Operation::Bridge {
                via: "X3".into(),
                from_chain: "ethereum".into(),
                from_asset: "USDC".into(),
                to_chain: "solana".into(),
                to_asset: "USDC".into(),
                amount: 120,
                receiver: "0xB1".into(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            },
            Operation::AtomicEnd,
        ]);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].contains("cross-chain operation present without a refund path"),
            "the refusal must be the cross-chain one: {found:?}"
        );
    }
}
