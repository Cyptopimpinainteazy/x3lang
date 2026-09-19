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

use crate::ir::{Condition, FailureAction, Operation, X3IR};
use std::collections::{HashMap, HashSet};
use x3_lang_ast::ast::{AtomicSwapDecl, Expression, Item, LiteralExpr, Program};
use x3_lang_common::{ErrorAccumulator, Span, Spanned, X3Error};

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
        SemanticPass::ProofRequirements => verify_proof_requirements(ir, acc),
        SemanticPass::RouteScore => verify_route_score(ir, acc),
        SemanticPass::Invariants => verify_invariants_structured(ir, context.invariants, acc),
        SemanticPass::MainnetSafe => {
            if context.mode == Some(CompilationMode::Mainnet) {
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
            Operation::Release { chain, asset, to } => {
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
fn require_guards(program: &Program) -> Vec<(&str, &x3_lang_ast::ast::RequireGuard)> {
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
                                .then(|| extract_int_from_expr(&guard.value))
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
                        if extract_int_from_expr(&guard.value).is_none() {
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

        // `>=`, not `>`: 10_000 bps *is* the whole amount, so a fee that high is
        // not a venue. The message said "at or above" while the code checked
        // strictly above, which is how the boundary case went unchecked.
        if venue.fee_bps >= 10_000 {
            acc.add_error(err(format!(
                "venue '{name}' declares a fee of {} bps; a fee at or above 10_000 bps is the whole \
                 amount",
                venue.fee_bps
            )));
        }
        if venue.slippage_bps > 10_000 {
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
                "declaration '{owner}' states `require solver_bond` without a `>=` bound; a solver                  bond guard is a floor, and the check reads it as one"
            )));
            continue;
        }
        let required = extract_int_from_expr(&guard.value).unwrap_or(0);
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
                "declaration '{owner}' states `require relayer_quorum` without a `>=` bound; a quorum                  guard is a floor, and the check reads it as one"
            )));
            continue;
        }
        let required = extract_int_from_expr(&guard.value).unwrap_or(0);
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

/// Extract a duration in seconds from an expression.
///
/// - `Literal(Int(n))` → bare number treated as seconds → `Some(n)`
/// - `Literal(Duration { value, unit })` → converts to seconds
/// - Otherwise → `None`
fn extract_seconds_from_expr(expr: &Expression) -> Option<u64> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Some(*value as u64),
        Expression::Literal(LiteralExpr::Duration { value, unit }) => {
            use x3_lang_common::DurationUnit;
            let secs = match unit {
                DurationUnit::Seconds => *value,
                DurationUnit::Minutes => value.saturating_mul(60),
                DurationUnit::Hours => value.saturating_mul(3600),
                DurationUnit::Days => value.saturating_mul(86400),
                DurationUnit::Milliseconds => value / 1000,
                DurationUnit::Microseconds => value / 1_000_000,
                DurationUnit::Nanoseconds => value / 1_000_000_000,
            };
            Some(secs)
        }
        _ => None,
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
        if let (Some(src_secs), Some(dst_secs)) =
            (extract_seconds_from_expr(src_expr), extract_seconds_from_expr(dst_expr))
        {
            if src_secs <= dst_secs {
                acc.add_error(err(format!(
                    "Source timeout ({src_secs}s) must be greater than destination timeout ({dst_secs}s) in atomic swap"
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
            if let Some(n) = extract_int_from_expr(&require.value) {
                if n == 0 {
                    acc.add_error(err("require relayer_quorum must be positive"));
                }
            }
        }
        _ => {}
    }
}

/// Verify that every cross-chain operation (Bridge, Swap, Lock) has a
/// corresponding refund path via OnFail with a Refund action or OnTimeout.
pub fn verify_refund_path_exists(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let mut has_bridge = false;
    let mut has_refund = false;
    for op in &ir.operations {
        if matches!(
            op,
            Operation::Bridge { .. } | Operation::Swap { .. } | Operation::Lock { .. }
        ) {
            has_bridge = true;
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
    if has_bridge && !has_refund {
        acc.add_error(err(
            "cross-chain operation present without a refund path — add an OnFail or OnTimeout with Refund action",
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
pub fn verify_proof_requirements(ir: &X3IR, acc: &mut ErrorAccumulator) {
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
    if has_bridge && !declared.contains(&ProofCategory::SourceLock) {
        acc.add_warning(X3Error::SemanticError {
            message: "Bridge operation present without a source-lock proof — add `proofs required { \
                      source_lock_proof }`"
                .into(),
            span: span(),
        });
    }
    if has_bridge && !declared.contains(&ProofCategory::DestinationFill) {
        acc.add_warning(X3Error::SemanticError {
            message: "Bridge operation present without a destination-fill proof — add `proofs \
                      required { destination_fill_proof }`"
                .into(),
            span: span(),
        });
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

/// The lock a claim releases, as `(chain, asset)`.
fn release_lock(op: &Operation) -> Option<(&str, &str)> {
    match op {
        Operation::Release { chain, asset, .. } => Some((chain.as_str(), asset.as_str())),
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
                let claims: Vec<&Operation> = atomic_scoped_operations(ir)
                    .into_iter()
                    .filter(|op| matches!(op, Operation::Release { .. }))
                    .collect();
                if claims.len() > 1 {
                    return Err("multiple Release (claim) operations execute inside the same atomic route".into());
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
                // Per lock, for the same reason as `no_double_refund`: releasing
                // the destination leg does not stop the source leg from being
                // refunded on its own timeout, and a structural scan that
                // ignored the lock would call that a violation.
                let mut claimed: Vec<(&str, &str)> = Vec::new();
                for op in atomic_scoped_operations(ir) {
                    if let Some(lock) = release_lock(op) {
                        claimed.push(lock);
                    }
                    if let Some(lock) = refund_lock(op) {
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

// ───── Mainnet safety checks ─────────────────────────────────────────────

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
            if let Some(pct) = extract_slippage_percent(expr).filter(|p| *p > 5.0) {
                acc.add_error(err(format!("mainnet: slippage tolerance {pct}% exceeds maximum 5%")));
            }
        }
    }
}

fn extract_slippage_percent(expr: &str) -> Option<f64> {
    let cleaned: String = expr
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.' || *c == '-' || *c == '+' || *c == 'e' || *c == 'E')
        .collect();
    cleaned.parse::<f64>().ok()
}

fn verify_deadline_bounded(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let max_allowed_blocks: u32 = 14400; // 24h at 6s/block
    for op in &ir.operations {
        if let Operation::OnTimeout { duration_blocks, .. } = op {
            if *duration_blocks > max_allowed_blocks {
                acc.add_error(err(format!(
                    "mainnet: timeout {duration_blocks} blocks exceeds maximum 14400 (24h)"
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
        Operation::OnTimeout { duration_blocks, .. } => *duration_blocks <= 14400,
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
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "a".into(),
            },
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "b".into(),
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
        let mut ir = empty_ir();
        ir.operations = atomic(vec![
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "a".into(),
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
            "a refund after a claim in one atomic route must be reported"
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
            value: Expression::Literal(LiteralExpr::Int {
                value: 12,
                base: x3_lang_common::IntBase::Decimal,
                suffix: None,
            }),
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
            value: Expression::Literal(LiteralExpr::Int {
                value: 3,
                base: x3_lang_common::IntBase::Decimal,
                suffix: None,
            }),
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
            value: Expression::Literal(LiteralExpr::Int {
                value: 0,
                base: x3_lang_common::IntBase::Decimal,
                suffix: None,
            }),
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
        let mut weights = std::collections::HashMap::new();
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
        let mut weights = std::collections::HashMap::new();
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
                condition: Condition::Expression { expr: "10.0".into() },
                error_msg: Some("slippage".into()),
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
            },
            Operation::Release {
                chain: "solana".into(),
                asset: "USDC".into(),
                to: "bob".into(),
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
        let mut weights = std::collections::HashMap::new();
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
                comparison: None,
            },
            Operation::Require {
                kind: RequireKind::Finality,
                subject: Some("solana".into()),
                condition: Condition::True,
                error_msg: None,
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
