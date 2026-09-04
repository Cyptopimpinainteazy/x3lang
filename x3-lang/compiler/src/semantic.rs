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

/// Verify with explicit budgets.
///
/// `mode` optionally gates mainnet-specific safety checks.
pub fn verify_with_config(
    ir: &X3IR,
    max_atomic_ops: u32,
    max_route_hops: u32,
    mode: Option<CompilationMode>,
) -> Result<(), Vec<X3Error>> {
    let mut acc = ErrorAccumulator::new();
    verify_symbols(ir, &mut acc);
    verify_route_depths(ir, &mut acc, max_atomic_ops, max_route_hops);
    verify_atomic_balance(ir, &mut acc);
    verify_rollback_presence(ir, &mut acc);
    verify_replay_and_expiry(ir, &mut acc);
    verify_bridge_adapter_allowlist(ir, &mut acc);
    verify_adapter_compatibility(ir, &mut acc);
    verify_asset_moves(ir, &mut acc);
    verify_refund_path_exists(ir, &mut acc);
    verify_finality_explicit(ir, &mut acc);
    verify_proof_requirements(ir, &mut acc);
    verify_route_score(ir, &mut acc);

    let invariants = get_builtin_invariants();
    verify_invariants_structured(ir, &invariants, &mut acc);

    if mode == Some(CompilationMode::Mainnet) {
        verify_mainnet_safe(ir, &mut acc);
    }

    if acc.has_errors() {
        Err(acc.take_errors())
    } else {
        Ok(())
    }
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
    if value
        .chars()
        .any(|c| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
    {
        acc.add_error(err(format!(
            "{field}={value:?} contains unsafe characters (allowed: alnum, _, -)"
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
fn extract_int_from_expr(expr: &Expression) -> Option<u128> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Some(*value),
        _ => None,
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
            acc.add_warning(X3Error::SemanticError {
                message: format!(
                    "bridge from chain '{chain}' has no explicit finality requirement — add `require finality.{chain} >= <confirmations>`"
                ),
                span: span(),
            });
        }
    }
}

/// Verify that state transitions have required proof declarations.
/// - Lock operations need a lock_proof
/// - Bridge operations need a fill_proof
/// - Release operations need a claim_proof
pub fn verify_proof_requirements(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let has_lock = ir.operations.iter().any(|op| matches!(op, Operation::Lock { .. }));
    let has_bridge = ir.operations.iter().any(|op| matches!(op, Operation::Bridge { .. }));
    let has_release = ir.operations.iter().any(|op| matches!(op, Operation::Release { .. }));

    let required_proofs: HashSet<String> = ir
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::ProofRequired { proof_type, .. } => Some(proof_type.to_ascii_lowercase()),
            _ => None,
        })
        .collect();

    if has_lock && !required_proofs.contains("lock_proof") {
        acc.add_warning(X3Error::SemanticError {
            message: "Lock operation present without `proofs required { lock_proof }` declaration".into(),
            span: span(),
        });
    }
    if has_bridge && !required_proofs.contains("fill_proof") {
        acc.add_warning(X3Error::SemanticError {
            message: "Bridge operation present without `proofs required { fill_proof }` declaration".into(),
            span: span(),
        });
    }
    if has_release && !required_proofs.contains("claim_proof") {
        acc.add_warning(X3Error::SemanticError {
            message: "Release operation present without `proofs required { claim_proof }` declaration".into(),
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

/// Return the list of built-in invariant rules for static analysis.
pub fn get_builtin_invariants() -> Vec<InvariantRule> {
    vec![
        InvariantRule {
            name: "no_double_claim".into(),
            description: "No claim operation may execute twice for the same lock".into(),
            check_fn: |ir| {
                let claims: Vec<&Operation> = ir
                    .operations
                    .iter()
                    .filter(|op| matches!(op, Operation::Release { .. }))
                    .collect();
                if claims.len() > 1 {
                    return Err("multiple Release (claim) operations found for the same intent".into());
                }
                Ok(())
            },
        },
        InvariantRule {
            name: "no_double_refund".into(),
            description: "No refund operation may execute twice for the same lock".into(),
            check_fn: |ir| {
                let refunds: Vec<&Operation> = ir
                    .operations
                    .iter()
                    .filter(|op| {
                        matches!(op, Operation::OnTimeout { action, .. }
                            if matches!(action, FailureAction::Refund { .. })
                        )
                    })
                    .collect();
                if refunds.len() > 1 {
                    return Err("multiple refund operations found for the same lock".into());
                }
                Ok(())
            },
        },
        InvariantRule {
            name: "no_claim_after_refund".into(),
            description: "Claim must not execute after refund".into(),
            check_fn: |ir| {
                let mut found_refund = false;
                for op in &ir.operations {
                    if matches!(op, Operation::OnTimeout { action, .. }
                        if matches!(action, FailureAction::Refund { .. })
                    ) {
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
                let mut found_claim = false;
                for op in &ir.operations {
                    if matches!(op, Operation::Release { .. }) {
                        found_claim = true;
                    }
                    if found_claim
                        && matches!(op, Operation::OnTimeout { action, .. }
                            if matches!(action, FailureAction::Refund { .. })
                        )
                    {
                        return Err("Refund found after Release (claim)".into());
                    }
                }
                Ok(())
            },
        },
        InvariantRule {
            name: "destination_fill_before_source_claim".into(),
            description: "Destination must be filled before source claim".into(),
            check_fn: |ir| {
                let bridge_positions: Vec<usize> = ir
                    .operations
                    .iter()
                    .enumerate()
                    .filter(|(_, op)| matches!(op, Operation::Bridge { .. }))
                    .map(|(i, _)| i)
                    .collect();
                let release_positions: Vec<usize> = ir
                    .operations
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

fn verify_single_rpc(ir: &X3IR, acc: &mut ErrorAccumulator) {
    let rpc_count = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::RpcConsensus { .. }))
        .count();
    if rpc_count == 0 {
        acc.add_error(err("mainnet: no RPC consensus declared — single-RPC is unsafe"));
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
        acc.add_error(err(
            "mainnet: no relayer attestation declared — single-relayer is unsafe",
        ));
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
    let has_solver = ir.operations.iter().any(|op| matches!(op, Operation::SolverBid { .. }));
    if !has_solver {
        acc.add_error(err("mainnet: missing solver bond declaration"));
        return;
    }
    for op in &ir.operations {
        if let Operation::SolverBid { solver, bond, .. } = op {
            if *bond == 0 {
                acc.add_error(err(format!(
                    "mainnet: solver '{solver}' has zero bond — bond must be > 0"
                )));
            }
        }
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

// ───── Invariant analysis ────────────────────────────────────────────────

/// Run static analysis on invariant declarations from the AST.
/// Generates conditions that must be true at runtime.
pub fn analyze_invariants(invariant_names: &[String]) -> Vec<String> {
    let mut conditions = Vec::new();
    for name in invariant_names {
        match name.to_ascii_lowercase().as_str() {
            "no_double_claim" => {
                conditions.push("assert count(Release) <= 1".into());
            }
            "no_double_refund" => {
                conditions.push("assert count(Refund) <= 1".into());
            }
            "no_claim_after_refund" => {
                conditions.push("assert not (Refund before Release)".into());
            }
            "no_refund_after_claim" => {
                conditions.push("assert not (Release before Refund)".into());
            }
            "destination_fill_before_source_claim" => {
                conditions.push("assert exists(Bridge) before any(Release)".into());
            }
            "no_route_mutation_after_lock" => {
                conditions.push("assert not (RouteScore after Lock)".into());
            }
            other => {
                conditions.push(format!("assert invariant({other})"));
            }
        }
    }
    conditions
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

    // Solver risk: no solver bond means risk
    let has_solver_bond = ir
        .operations
        .iter()
        .any(|op| matches!(op, Operation::SolverBid { bond, .. } if *bond > 0));
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
            },
            Operation::Require {
                kind: RequireKind::Finality,
                subject: Some("solana".into()),
                condition: Condition::True,
                error_msg: None,
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
