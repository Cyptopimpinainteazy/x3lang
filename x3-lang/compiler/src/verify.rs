//! Structural verifier for X3IR.
//!
//! This pass runs after AST lowering and before bytecode emission. It validates
//! invariants that should never be delegated to an emitter or runtime decoder.

use crate::diagnostic::{CompilerDiagnostic, DiagnosticCode};
use crate::ir::{Operation, X3IR};
use x3_lang_common::Span;

/// Verify structural and safety invariants of lowered X3IR.
pub fn verify_ir(ir: &X3IR) -> Result<(), Vec<CompilerDiagnostic>> {
    let mut diagnostics = Vec::new();

    if matches!(ir.metadata.nonce.as_deref(), Some("")) {
        push_unsafe(&mut diagnostics, "IR metadata nonce must not be empty");
    }
    if matches!(ir.metadata.timeout_blocks, Some(0)) {
        push_unsafe(
            &mut diagnostics,
            "IR timeout_blocks must be greater than zero when present",
        );
    }

    verify_sequence(&ir.operations, "program", &mut diagnostics);

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(diagnostics)
    }
}

fn push_unsafe(diagnostics: &mut Vec<CompilerDiagnostic>, message: impl Into<String>) {
    diagnostics.push(
        CompilerDiagnostic::error(DiagnosticCode::UnsafeIr, message, Span::DUMMY)
            .with_help("fix the lowering invariant before bytecode emission"),
    );
}

fn require_non_empty(diagnostics: &mut Vec<CompilerDiagnostic>, context: &str, field: &str, value: &str) {
    if value.trim().is_empty() {
        push_unsafe(diagnostics, format!("{context}: {field} must not be empty"));
    }
}

fn verify_sequence(ops: &[Operation], context: &str, diagnostics: &mut Vec<CompilerDiagnostic>) {
    let mut atomic_depth: i32 = 0;

    for (index, op) in ops.iter().enumerate() {
        let op_context = format!("{context}[{index}]");
        match op {
            Operation::AtomicBegin => {
                if atomic_depth > 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: nested atomic scopes are not allowed"),
                    );
                }
                atomic_depth += 1;
            }
            Operation::AtomicEnd => {
                if atomic_depth == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: AtomicEnd has no matching AtomicBegin"),
                    );
                } else {
                    atomic_depth -= 1;
                }
            }
            Operation::If { then_ops, else_ops, .. } => {
                verify_sequence(then_ops, &format!("{op_context}.then"), diagnostics);
                if let Some(else_ops) = else_ops {
                    verify_sequence(else_ops, &format!("{op_context}.else"), diagnostics);
                }
            }
            Operation::Loop { max_iterations, body } => {
                if *max_iterations == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: loop max_iterations must be greater than zero"),
                    );
                }
                if body.is_empty() {
                    push_unsafe(diagnostics, format!("{op_context}: loop body must not be empty"));
                }
                verify_sequence(body, &format!("{op_context}.loop"), diagnostics);
            }
            Operation::Lock {
                chain,
                asset,
                amount,
                from,
            }
            | Operation::Mint {
                chain,
                asset,
                amount,
                to: from,
            }
            | Operation::Burn {
                chain,
                asset,
                amount,
                from,
            } => {
                require_non_empty(diagnostics, &op_context, "chain", chain);
                require_non_empty(diagnostics, &op_context, "asset", asset);
                require_non_empty(diagnostics, &op_context, "account", from);
                if *amount == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: asset move amount must be greater than zero"),
                    );
                }
            }
            Operation::Release { chain, asset, to } => {
                require_non_empty(diagnostics, &op_context, "chain", chain);
                require_non_empty(diagnostics, &op_context, "asset", asset);
                require_non_empty(diagnostics, &op_context, "to", to);
            }
            Operation::Swap {
                from_chain,
                from_asset,
                to_asset,
                input_amount,
                min_output,
                dex,
            } => {
                require_non_empty(diagnostics, &op_context, "from_chain", from_chain);
                require_non_empty(diagnostics, &op_context, "from_asset", from_asset);
                require_non_empty(diagnostics, &op_context, "to_asset", to_asset);
                if let Some(dex) = dex {
                    require_non_empty(diagnostics, &op_context, "dex", dex);
                }
                if *input_amount == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: swap input_amount must be greater than zero"),
                    );
                }
                if *min_output == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: swap min_output must be greater than zero"),
                    );
                }
            }
            Operation::Bridge {
                via,
                from_chain,
                from_asset,
                to_chain,
                to_asset,
                amount,
                receiver,
                ..
            } => {
                require_non_empty(diagnostics, &op_context, "via", via);
                require_non_empty(diagnostics, &op_context, "from_chain", from_chain);
                require_non_empty(diagnostics, &op_context, "from_asset", from_asset);
                require_non_empty(diagnostics, &op_context, "to_chain", to_chain);
                require_non_empty(diagnostics, &op_context, "to_asset", to_asset);
                require_non_empty(diagnostics, &op_context, "receiver", receiver);
                if *amount == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: bridge amount must be greater than zero"),
                    );
                }
            }
            Operation::Call { function, .. } => {
                require_non_empty(diagnostics, &op_context, "function", function);
            }
            Operation::GpuDispatch { kernel, .. } => {
                require_non_empty(diagnostics, &op_context, "kernel", kernel);
            }
            Operation::Simulate { body, receipt_slot } => {
                require_non_empty(diagnostics, &op_context, "receipt_slot", receipt_slot);
                verify_sequence(body, &format!("{op_context}.simulate"), diagnostics);
            }
            Operation::ScheduledDispatch { period_blocks, entry } => {
                if *period_blocks == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: scheduled period_blocks must be greater than zero"),
                    );
                }
                if entry.is_empty() {
                    push_unsafe(diagnostics, format!("{op_context}: scheduled entry must not be empty"));
                }
                verify_sequence(entry, &format!("{op_context}.scheduled"), diagnostics);
            }
            Operation::IntentResolve { resolver, .. } => {
                require_non_empty(diagnostics, &op_context, "resolver", resolver);
            }
            Operation::Pathfind { from, to, max_depth } => {
                require_non_empty(diagnostics, &op_context, "from", from);
                require_non_empty(diagnostics, &op_context, "to", to);
                if *max_depth == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: pathfind max_depth must be greater than zero"),
                    );
                }
            }
            Operation::MempoolScan { max_results } => {
                if *max_results == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: mempool max_results must be greater than zero"),
                    );
                }
            }
            Operation::OracleRequest { token, .. } => {
                require_non_empty(diagnostics, &op_context, "token", token);
            }
            Operation::Lifecycle {
                target: Some(target), ..
            } => {
                require_non_empty(diagnostics, &op_context, "target", target);
            }
            Operation::GasEstimate { chain, route } => {
                require_non_empty(diagnostics, &op_context, "chain", chain);
                require_non_empty(diagnostics, &op_context, "route", route);
            }
            Operation::EventProvenance { event_type, .. } => {
                require_non_empty(diagnostics, &op_context, "event_type", event_type);
            }
            Operation::MultiHopSwap { path, amount } => {
                if path.len() < 2 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: multi-hop path must contain at least two assets"),
                    );
                }
                if path.iter().any(|part| part.trim().is_empty()) {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: multi-hop path contains an empty asset"),
                    );
                }
                if *amount == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: multi-hop amount must be greater than zero"),
                    );
                }
            }
            Operation::VectorMath { size, .. } => {
                if *size == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: vector size must be greater than zero"),
                    );
                }
            }
            Operation::RoleCheck { role } => {
                require_non_empty(diagnostics, &op_context, "role", role);
            }
            Operation::MultisigCheck { required, total } => {
                if *required == 0 || *total == 0 || required > total {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: multisig requires 0 < required <= total"),
                    );
                }
            }
            Operation::VersionMeta { version, .. } => {
                require_non_empty(diagnostics, &op_context, "version", version);
            }
            Operation::StorageNamespace { package, key } => {
                require_non_empty(diagnostics, &op_context, "package", package);
                require_non_empty(diagnostics, &op_context, "key", key);
            }
            Operation::AbiExport { function, .. } => {
                require_non_empty(diagnostics, &op_context, "function", function);
            }
            Operation::GasAdaptive {
                high_gas_ops,
                low_gas_ops,
            } => {
                if high_gas_ops.is_empty() || low_gas_ops.is_empty() {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: gas-adaptive branches must not be empty"),
                    );
                }
                verify_sequence(high_gas_ops, &format!("{op_context}.high_gas"), diagnostics);
                verify_sequence(low_gas_ops, &format!("{op_context}.low_gas"), diagnostics);
            }
            Operation::Bounty { amount, condition } => {
                if *amount == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: bounty amount must be greater than zero"),
                    );
                }
                require_non_empty(diagnostics, &op_context, "condition", condition);
            }
            Operation::Emit { name, .. } => require_non_empty(diagnostics, &op_context, "name", name),
            Operation::OnTimeout { duration_blocks, .. } => {
                if *duration_blocks == 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: timeout duration must be greater than zero"),
                    );
                }
            }
            Operation::Nop
            | Operation::Require { .. }
            | Operation::OnFail { .. }
            | Operation::CrdtOp { .. }
            | Operation::ProofVerify { .. }
            | Operation::StorageOp { .. }
            | Operation::EmergencyControl { .. }
            | Operation::Lifecycle { target: None, .. }
            | Operation::Serialize { .. }
            | Operation::Deserialize { .. }
            | Operation::ChainMetric { .. }
            | Operation::DocEmbed { .. }
            | Operation::RouteScore { .. }
            | Operation::SolverBid { .. }
            | Operation::RelayerAttest { .. }
            | Operation::RpcConsensus { .. }
            | Operation::RiskScore { .. }
            | Operation::InvariantCheck { .. }
            | Operation::PrivacyCommit { .. }
            | Operation::ProofRequired { .. }
            | Operation::VmAdapterCall { .. }
            | Operation::ModeCheck { .. }
            | Operation::PackageImport { .. }
            | Operation::RefundPolicy { .. } => {}
        }
    }

    if atomic_depth > 0 {
        push_unsafe(
            diagnostics,
            format!("{context}: {atomic_depth} AtomicBegin operation(s) are not closed by AtomicEnd"),
        );
    }
}
