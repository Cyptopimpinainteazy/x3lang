//! Structural verifier for X3IR.
//!
//! This pass runs after AST lowering and before bytecode emission. It validates
//! invariants that should never be delegated to an emitter or runtime decoder.

use crate::diagnostic::{CompilerDiagnostic, DiagnosticCode};
use crate::ir::{AssetKey, Operation, TradingOperation, ValueRef, X3IR};
use std::collections::{BTreeMap, BTreeSet};
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
    verify_trading_sequences(&ir.operations, "program", &mut diagnostics);

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
            Operation::NonceUnused { nonce } => {
                // The instruction exists to carry an identifier, and an empty one
                // would test and record nothing while the guard after it passes:
                // a replay-protection instruction that protects against no
                // replay.
                if nonce.trim().is_empty() {
                    push_unsafe(diagnostics, format!("{op_context}: nonce is empty"));
                }
            }
            Operation::AtomicChoice {
                paths,
                criterion,
                selected,
            } => {
                if *paths < 2 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: an atomic choice with fewer than two paths is not a choice"),
                    );
                }
                if *paths == 0 || *selected >= *paths {
                    push_unsafe(
                        diagnostics,
                        format!(
                            "{op_context}: atomic choice selects path {selected} of {paths}; the selected \
                             index must name a declared path"
                        ),
                    );
                }
                let _ = criterion;
            }
            Operation::StrategyLicense {
                creator,
                royalty_bps,
                split,
                ..
            } => {
                // The compiler already checked these; the IR verifier states
                // them again because this record is what a distribution reads,
                // and a record that does not add up is worse than no record.
                // A module may split its profit without being licensed, so an
                // empty creator is only wrong when a royalty is claimed: that is
                // a payment to nobody.
                if *royalty_bps > 0 && creator.is_empty() {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: a strategy licence claims a royalty but names no creator"),
                    );
                }
                if *royalty_bps > 10_000 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: strategy licence royalty {royalty_bps} bps exceeds 10,000"),
                    );
                }
                if split.is_empty() {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: a strategy licence with an empty profit split"),
                    );
                } else {
                    let total: u32 = split.iter().map(|(_, bps)| *bps).sum();
                    if total != 10_000 {
                        push_unsafe(
                            diagnostics,
                            format!("{op_context}: the profit split totals {total} bps, not 10,000"),
                        );
                    }
                    if split.iter().any(|(recipient, _)| recipient.is_empty()) {
                        push_unsafe(
                            diagnostics,
                            format!("{op_context}: the profit split names an empty recipient"),
                        );
                    }
                }
            }
            Operation::FeatureAllow { feature, name } => {
                // The set of features is closed, and this is where the closure
                // is enforced on the IR side: an unknown code means the artifact
                // claims consent to something the language does not define.
                if *feature != crate::spec::opcodes::FEATURE_INTENT_FUSION {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: unknown allowed feature code {feature} ({name})"),
                    );
                }
            }
            Operation::ParallelPlan {
                waves,
                edges,
                domains,
                settlement,
            } => {
                // The compiler built this plan, so what the IR verifier owes is
                // a check that it is a plan: legs that appear exactly once, no
                // empty wave, and no edge naming a leg that is not in it. A
                // plan with a dangling edge is a plan whose ordering nobody
                // enforced.
                let leg_count: usize = waves.iter().map(|wave| wave.len()).sum();
                if leg_count < 2 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: a parallel plan needs at least two legs, has {leg_count}"),
                    );
                }
                if waves.iter().any(|wave| wave.is_empty()) {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: a parallel plan contains an empty wave"),
                    );
                }
                let mut seen: Vec<&str> = Vec::new();
                for leg in waves.iter().flatten() {
                    if seen.contains(&leg.as_str()) {
                        push_unsafe(
                            diagnostics,
                            format!("{op_context}: leg '{leg}' appears in more than one wave"),
                        );
                    }
                    seen.push(leg.as_str());
                }
                for leg in &seen {
                    match domains.get(*leg) {
                        None => push_unsafe(
                            diagnostics,
                            format!("{op_context}: leg '{leg}' has no execution domain"),
                        ),
                        Some(leg_domains) => {
                            if leg_domains.is_empty() {
                                push_unsafe(
                                    diagnostics,
                                    format!("{op_context}: leg '{leg}' has an empty domain set"),
                                );
                            }
                        }
                    }
                }
                for (from, to) in edges {
                    if from == to || !seen.contains(&from.as_str()) || !seen.contains(&to.as_str()) {
                        push_unsafe(
                            diagnostics,
                            format!(
                                "{op_context}: dependency {from}->{to} does not join two distinct legs \
                                 of this plan"
                            ),
                        );
                    }
                }
                // Every wave owes a statement about its settlement. A wave with
                // no entry is a wave whose recoverability nobody decided.
                if settlement.len() != waves.len() {
                    push_unsafe(
                        diagnostics,
                        format!(
                            "{op_context}: {} wave(s) but {} settlement record(s)",
                            waves.len(),
                            settlement.len()
                        ),
                    );
                }
                for (index, record) in settlement.iter().enumerate() {
                    if record.wave != index {
                        push_unsafe(
                            diagnostics,
                            format!("{op_context}: settlement record {index} names wave {}", record.wave),
                        );
                    }
                    if record.domains.is_empty() {
                        push_unsafe(
                            diagnostics,
                            format!("{op_context}: wave {index} settles over no domain"),
                        );
                    }
                    // A wave spanning more than one domain cannot be undone by
                    // this VM alone; claiming otherwise would tell a coordinator
                    // it has a rollback it does not have.
                    if record.domains.len() > 1 && record.locally_recoverable {
                        push_unsafe(
                            diagnostics,
                            format!(
                                "{op_context}: wave {index} spans {} domains but is marked locally \
                                 recoverable",
                                record.domains.len()
                            ),
                        );
                    }
                }
            }
            Operation::RouteFallback { approved } => {
                if approved.is_empty() {
                    push_unsafe(diagnostics, format!("{op_context}: route fallback approves no venues"));
                }
                if approved.len() > crate::spec::opcodes::MAX_ROUTE_FALLBACKS {
                    push_unsafe(
                        diagnostics,
                        format!(
                            "{op_context}: route fallback approves {} venues, above the {}-venue bound",
                            approved.len(),
                            crate::spec::opcodes::MAX_ROUTE_FALLBACKS
                        ),
                    );
                }
                if approved.iter().any(|venue| venue.trim().is_empty()) {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: route fallback approves an unnamed venue"),
                    );
                }
                if approved.iter().any(|venue| venue.contains(',')) {
                    push_unsafe(
                        diagnostics,
                        format!(
                            "{op_context}: route fallback venue contains ',' which is the payload \
                             separator, so the approved set would not round-trip"
                        ),
                    );
                }
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
            // A nested branch is refused rather than walked.
            //
            // The VM branches on a *register* and skips a fixed number of
            // four-byte instructions, while a compiler stream frames instructions
            // with a width that varies (`3 + payload_len`, four for `REQUIRE`) and
            // pads each one to the next absolute multiple of four. Nothing
            // evaluates a condition into a register either — the compiler emits no
            // arithmetic at all. So an emitted branch is an instruction whose
            // operands no reader can follow and no executor can act on: measured on
            // `strategy TriDexArb { execute { if 1 > 0 { require profit >= 5 } } }`,
            // the artifact built, `x3c explain` printed the condition as opcodes,
            // and `x3c run` failed with `X3_VERIFY_FAILED: OutOfBounds(292)`.
            //
            // Refusing here is the fail-closed half of the feature rather than the
            // feature: TICKET-058 carries the work (an explicit branch target in
            // the record, a reader rule for it in `spec/opcodes.rs`, and expression
            // codegen for the condition). Refusing it in this layer and not only in
            // the emitter is what keeps `x3c check` from accepting what `x3c build`
            // then has to refuse.
            // A hedge is *decided*, not executed: its net is checked
            // (`hedge::verify`) and recorded, but a perp leg needs a venue adapter
            // this VM does not have. Refusing in this layer — and not only in the
            // emitter — is what keeps `x3c check` from accepting what `x3c build`
            // then refuses, the split this session keeps finding.
            // A liquidation's figures are decided (`liquidation::verify`), and this
            // layer says what the VM can do with the result: nothing, because
            // `liquidate` and `receive` are calls into a lending protocol it has no
            // adapter for. Same shape as the hedge above, same reason for refusing
            // here rather than only in the emitter.
            Operation::Liquidation { position, .. } => push_unsafe(
                diagnostics,
                format!(
                    "{op_context}: the liquidation of '{position}' cannot be executed — liquidate \
                     and receive are calls into a lending protocol this VM has no adapter for, so \
                     the accounting is decided and the execution is not pretended"
                ),
            ),
            Operation::Hedge { asset, .. } => push_unsafe(
                diagnostics,
                format!(
                    "{op_context}: the hedge on '{asset}' cannot be executed — a perp leg needs a \
                     venue adapter this VM does not have, so the exposure is decided and the \
                     execution is not pretended"
                ),
            ),
            Operation::If { .. } => push_unsafe(
                diagnostics,
                format!(
                    "{op_context}: `if` cannot be executed — this VM branches on a register and skips \
                     four-byte instructions, and a compiler stream is framed with variable widths and \
                     padded, so the branch has no target it could jump to and no condition it could \
                     read"
                ),
            ),
            Operation::Loop { .. } => push_unsafe(
                diagnostics,
                format!(
                    "{op_context}: `loop` cannot be executed — this VM branches on a register and skips \
                     four-byte instructions, and a compiler stream is framed with variable widths and \
                     padded, so the loop has no target it could jump back to"
                ),
            ),
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
                to_chain,
                to_asset,
                input_amount,
                min_output,
                dex,
            } => {
                require_non_empty(diagnostics, &op_context, "from_chain", from_chain);
                require_non_empty(diagnostics, &op_context, "from_asset", from_asset);
                require_non_empty(diagnostics, &op_context, "to_chain", to_chain);
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
            Operation::Trading(trading) => verify_trading_operation(trading, &op_context, diagnostics),
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

#[derive(Default)]
struct TradingSequenceState {
    began: bool,
    committed: bool,
    aborted: bool,
    receipt_seen: bool,
    all_debts_guard_seen: bool,
    profit_guard_seen: bool,
    open_debts: BTreeMap<String, AssetKey>,
    closed_debts: BTreeSet<String>,
    bindings: BTreeMap<String, AssetKey>,
    invariant_guards_seen: BTreeSet<crate::ir::InvariantKind>,
    /// Set once a Bridge operation has run. Nothing that operates on the
    /// source chain (OpenDebt/ExecuteSwap/CloseDebt) may appear after it —
    /// defense in depth for the same invariant trading_verify.rs already
    /// enforces at the AST level.
    bridged: bool,
}

fn verify_trading_sequences(ops: &[Operation], context: &str, diagnostics: &mut Vec<CompilerDiagnostic>) {
    let trading: Vec<&TradingOperation> = ops
        .iter()
        .filter_map(|op| match op {
            Operation::Trading(trading) => Some(trading),
            _ => None,
        })
        .collect();

    if trading.is_empty() {
        return;
    }

    let mut state = TradingSequenceState::default();

    for (index, op) in trading.iter().enumerate() {
        let op_context = format!("{context}.trading[{index}]");

        if state.committed || state.aborted {
            push_unsafe(
                diagnostics,
                format!("{op_context}: trading operation appears after terminal commit/abort"),
            );
            continue;
        }

        match op {
            TradingOperation::BeginAtomicTrade { .. } => {
                if state.began {
                    push_unsafe(diagnostics, format!("{op_context}: duplicate BeginAtomicTrade"));
                }
                if index != 0 {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: BeginAtomicTrade must be the first trading operation"),
                    );
                }
                state.began = true;
            }
            TradingOperation::OpenDebt { debt_id, asset, .. } => {
                require_trade_started(&state, &op_context, diagnostics);
                if state.bridged {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: open debt appears after the trade bridged to another chain"),
                    );
                }
                if state.open_debts.contains_key(debt_id) || state.closed_debts.contains(debt_id) {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: debt '{debt_id}' is opened more than once"),
                    );
                } else {
                    state.open_debts.insert(debt_id.clone(), asset.clone());
                }
            }
            TradingOperation::ExecuteSwap {
                binding,
                from,
                to,
                input,
                ..
            } => {
                require_trade_started(&state, &op_context, diagnostics);

                if state.bridged {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: swap appears after the trade bridged to another chain"),
                    );
                }

                if state.receipt_seen || state.all_debts_guard_seen {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: swap appears after final trading guards/receipt"),
                    );
                }

                match input {
                    ValueRef::Literal(_) => {}
                    ValueRef::Binding(name) => {
                        if let Some((debt_id, field)) = name.split_once('.') {
                            if field != "amount" {
                                push_unsafe(diagnostics, format!("{op_context}: unsupported debt binding '{name}'"));
                            }
                            match state.open_debts.get(debt_id) {
                                Some(asset) if asset == from => {}
                                Some(asset) => push_unsafe(
                                    diagnostics,
                                    format!(
                                        "{op_context}: debt binding '{name}' has asset {} but swap expects {}",
                                        asset.symbol, from.symbol
                                    ),
                                ),
                                None => push_unsafe(
                                    diagnostics,
                                    format!("{op_context}: debt binding '{name}' used before open debt"),
                                ),
                            }
                        } else {
                            match state.bindings.get(name) {
                                Some(asset) if asset == from => {}
                                Some(asset) => push_unsafe(
                                    diagnostics,
                                    format!(
                                        "{op_context}: binding '{name}' has asset {} but swap expects {}",
                                        asset.symbol, from.symbol
                                    ),
                                ),
                                None => push_unsafe(
                                    diagnostics,
                                    format!("{op_context}: binding '{name}' used before creation"),
                                ),
                            }
                        }
                    }
                }

                if let Some(existing) = state.bindings.get(binding) {
                    if existing != to {
                        push_unsafe(
                            diagnostics,
                            format!(
                                "{op_context}: binding '{binding}' reused with conflicting asset {} -> {}",
                                existing.symbol, to.symbol
                            ),
                        );
                    } else {
                        push_unsafe(
                            diagnostics,
                            format!("{op_context}: binding '{binding}' is assigned more than once"),
                        );
                    }
                } else {
                    state.bindings.insert(binding.clone(), to.clone());
                }
            }
            TradingOperation::Bridge { from, to, input, .. } => {
                require_trade_started(&state, &op_context, diagnostics);

                if state.bridged {
                    push_unsafe(diagnostics, format!("{op_context}: duplicate Bridge in one trade"));
                }
                if state.receipt_seen || state.all_debts_guard_seen {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: bridge appears after final trading guards/receipt"),
                    );
                }
                if from == to {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: bridge from and to assets must differ"),
                    );
                }

                match input {
                    ValueRef::Literal(_) => {}
                    ValueRef::Binding(name) => {
                        if let Some((debt_id, field)) = name.split_once('.') {
                            if field != "amount" {
                                push_unsafe(diagnostics, format!("{op_context}: unsupported debt binding '{name}'"));
                            }
                            match state.open_debts.get(debt_id) {
                                Some(asset) if asset == from => {}
                                Some(asset) => push_unsafe(
                                    diagnostics,
                                    format!(
                                        "{op_context}: debt binding '{name}' has asset {} but bridge expects {}",
                                        asset.symbol, from.symbol
                                    ),
                                ),
                                None => push_unsafe(
                                    diagnostics,
                                    format!("{op_context}: debt binding '{name}' used before open debt"),
                                ),
                            }
                        } else {
                            match state.bindings.get(name) {
                                Some(asset) if asset == from => {}
                                Some(asset) => push_unsafe(
                                    diagnostics,
                                    format!(
                                        "{op_context}: binding '{name}' has asset {} but bridge expects {}",
                                        asset.symbol, from.symbol
                                    ),
                                ),
                                None => push_unsafe(
                                    diagnostics,
                                    format!("{op_context}: binding '{name}' used before creation"),
                                ),
                            }
                        }
                    }
                }

                state.bridged = true;
            }
            TradingOperation::CloseDebt { debt_id } => {
                require_trade_started(&state, &op_context, diagnostics);
                if state.bridged {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: close debt appears after the trade bridged to another chain"),
                    );
                }
                if state.closed_debts.contains(debt_id) {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: debt '{debt_id}' is closed more than once"),
                    );
                } else if state.open_debts.remove(debt_id).is_none() {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: debt '{debt_id}' closed before it was opened"),
                    );
                } else {
                    state.closed_debts.insert(debt_id.clone());
                }
            }
            TradingOperation::AssertMinNetProfit { .. } => {
                require_trade_started(&state, &op_context, diagnostics);
                if state.receipt_seen || state.all_debts_guard_seen {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: minimum-profit guard appears after final guards/receipt"),
                    );
                }
                if state.profit_guard_seen {
                    push_unsafe(diagnostics, format!("{op_context}: duplicate minimum-profit guard"));
                }
                state.profit_guard_seen = true;
            }
            TradingOperation::AssertAllDebtsClosed => {
                require_trade_started(&state, &op_context, diagnostics);
                if !state.open_debts.is_empty() {
                    push_unsafe(
                        diagnostics,
                        format!(
                            "{op_context}: all-debts guard reached with open debts: {}",
                            state.open_debts.keys().cloned().collect::<Vec<_>>().join(", ")
                        ),
                    );
                }
                if !state.profit_guard_seen {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: all-debts guard must follow the minimum-profit guard"),
                    );
                }
                if state.all_debts_guard_seen {
                    push_unsafe(diagnostics, format!("{op_context}: duplicate all-debts guard"));
                }
                state.all_debts_guard_seen = true;
            }
            TradingOperation::AssertInvariant { kind } => {
                require_trade_started(&state, &op_context, diagnostics);
                if state.receipt_seen || state.committed {
                    push_unsafe(
                        diagnostics,
                        format!(
                            "{op_context}: invariant '{}' must precede receipt/commit",
                            kind.as_str()
                        ),
                    );
                }
                if !state.invariant_guards_seen.insert(*kind) {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: duplicate invariant guard '{}'", kind.as_str()),
                    );
                }
            }
            TradingOperation::EmitTradeReceipt => {
                require_trade_started(&state, &op_context, diagnostics);
                if !state.profit_guard_seen || !state.all_debts_guard_seen {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: receipt must follow minimum-profit and all-debts guards"),
                    );
                }
                if state.receipt_seen {
                    push_unsafe(diagnostics, format!("{op_context}: duplicate receipt emission"));
                }
                state.receipt_seen = true;
            }
            TradingOperation::CommitAtomicTrade => {
                require_trade_started(&state, &op_context, diagnostics);
                if !state.open_debts.is_empty() {
                    push_unsafe(diagnostics, format!("{op_context}: commit with open debts"));
                }
                if !state.profit_guard_seen {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: commit missing minimum-profit guard"),
                    );
                }
                if !state.all_debts_guard_seen {
                    push_unsafe(diagnostics, format!("{op_context}: commit missing all-debts guard"));
                }
                if !state.receipt_seen {
                    push_unsafe(diagnostics, format!("{op_context}: commit missing receipt"));
                }
                if index + 1 != trading.len() {
                    push_unsafe(
                        diagnostics,
                        format!("{op_context}: CommitAtomicTrade must be the final trading operation"),
                    );
                }
                state.committed = true;
            }
            TradingOperation::AbortAtomicTrade => {
                require_trade_started(&state, &op_context, diagnostics);
                state.aborted = true;
                if index + 1 != trading.len() {
                    push_unsafe(diagnostics, format!("{op_context}: AbortAtomicTrade must be terminal"));
                }
            }
        }
    }

    if !state.began {
        push_unsafe(
            diagnostics,
            format!("{context}: trading sequence is missing BeginAtomicTrade"),
        );
    }
    if !state.committed && !state.aborted {
        push_unsafe(
            diagnostics,
            format!("{context}: trading sequence has no terminal commit/abort"),
        );
    }
}

fn require_trade_started(state: &TradingSequenceState, context: &str, diagnostics: &mut Vec<CompilerDiagnostic>) {
    if !state.began {
        push_unsafe(
            diagnostics,
            format!("{context}: trading operation appears before BeginAtomicTrade"),
        );
    }
}

fn verify_trading_operation(trading: &TradingOperation, context: &str, diagnostics: &mut Vec<CompilerDiagnostic>) {
    match trading {
        TradingOperation::BeginAtomicTrade { trade_id, policy } => {
            require_non_empty(diagnostics, context, "trade_id", trade_id);
            require_non_empty(diagnostics, context, "policy.policy_id", &policy.policy_id);
            require_non_empty(diagnostics, context, "policy.chain", &policy.chain);
            if policy.policy_version == 0 {
                push_unsafe(
                    diagnostics,
                    format!("{context}: policy version must be greater than zero"),
                );
            }
            if policy.max_slippage_bps > 10_000 {
                push_unsafe(diagnostics, format!("{context}: policy max_slippage_bps exceeds 10000"));
            }
            if let Some(deviation_bps) = policy.max_oracle_deviation_bps {
                if deviation_bps > 10_000 {
                    push_unsafe(
                        diagnostics,
                        format!("{context}: policy max_oracle_deviation_bps exceeds 10000"),
                    );
                }
            }
            verify_asset_key(&policy.max_gas_asset, context, "policy.max_gas_asset", diagnostics);
            match (policy.max_cumulative_loss, &policy.max_cumulative_loss_asset) {
                (Some(_), Some(asset)) => {
                    verify_asset_key(asset, context, "policy.max_cumulative_loss_asset", diagnostics);
                }
                (None, None) => {}
                _ => {
                    push_unsafe(
                        diagnostics,
                        format!(
                            "{context}: policy max_cumulative_loss and max_cumulative_loss_asset must both be set or both be absent"
                        ),
                    );
                }
            }
            if policy.max_flash_fee_bps > 10_000 {
                push_unsafe(
                    diagnostics,
                    format!("{context}: policy max_flash_fee_bps exceeds 10000"),
                );
            }
            if policy.deadline_blocks == 0 {
                push_unsafe(
                    diagnostics,
                    format!("{context}: policy deadline_blocks must be greater than zero"),
                );
            }
        }
        TradingOperation::OpenDebt {
            debt_id,
            provider,
            asset,
            principal,
        } => {
            require_non_empty(diagnostics, context, "debt_id", debt_id);
            require_non_empty(diagnostics, context, "provider", provider);
            verify_asset_key(asset, context, "asset", diagnostics);
            if *principal == 0 {
                push_unsafe(
                    diagnostics,
                    format!("{context}: debt principal must be greater than zero"),
                );
            }
        }
        TradingOperation::ExecuteSwap {
            binding,
            venue,
            from,
            to,
            min_output,
            ..
        } => {
            require_non_empty(diagnostics, context, "binding", binding);
            require_non_empty(diagnostics, context, "venue", venue);
            verify_asset_key(from, context, "from", diagnostics);
            verify_asset_key(to, context, "to", diagnostics);
            if *min_output == 0 {
                push_unsafe(diagnostics, format!("{context}: min_output must be greater than zero"));
            }
        }
        TradingOperation::Bridge {
            via,
            from,
            to,
            receiver,
            ..
        } => {
            require_non_empty(diagnostics, context, "via", via);
            verify_asset_key(from, context, "from", diagnostics);
            verify_asset_key(to, context, "to", diagnostics);
            require_non_empty(diagnostics, context, "receiver", receiver);
        }
        TradingOperation::CloseDebt { debt_id } => {
            require_non_empty(diagnostics, context, "debt_id", debt_id);
        }
        TradingOperation::AssertMinNetProfit {
            settlement_asset,
            minimum,
        } => {
            verify_asset_key(settlement_asset, context, "settlement_asset", diagnostics);
            if *minimum == 0 {
                push_unsafe(
                    diagnostics,
                    format!("{context}: minimum net profit must be greater than zero"),
                );
            }
        }
        TradingOperation::AssertAllDebtsClosed
        | TradingOperation::AssertInvariant { .. }
        | TradingOperation::EmitTradeReceipt
        | TradingOperation::CommitAtomicTrade
        | TradingOperation::AbortAtomicTrade => {}
    }
}

fn verify_asset_key(asset: &AssetKey, context: &str, field: &str, diagnostics: &mut Vec<CompilerDiagnostic>) {
    require_non_empty(diagnostics, context, &format!("{field}.vm_family"), &asset.vm_family);
    require_non_empty(diagnostics, context, &format!("{field}.chain"), &asset.chain);
    require_non_empty(
        diagnostics,
        context,
        &format!("{field}.canonical_id"),
        &asset.canonical_id,
    );
    require_non_empty(diagnostics, context, &format!("{field}.symbol"), &asset.symbol);
}
