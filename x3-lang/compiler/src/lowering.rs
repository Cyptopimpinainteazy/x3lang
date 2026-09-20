//! AST -> X3IR lowering pipeline.
//!
//! This module lowers X3 AST into X3 Intermediate Representation (X3IR),
//! which is a semantic representation suitable for verification, optimization,
//! and code generation.

use crate::arb;
use crate::hedge;
use crate::hyperarb;
use crate::intent_emit;
use crate::ir::{
    self, ChainMetricKind, Condition, CrdtKind as IrCrdtKind, EmergencyKind, LifecycleKind, Operation, ProofKind,
    ReleaseAct, SerialFormat, StorageKind, VectorOp, X3IR,
};
use crate::liquidation;
use crate::netting;
use crate::rebalance;
use crate::semantic::CompilationMode;
use crate::trading_lowering;
use crate::trading_semantic;
use crate::trading_verify;
use x3_lang_ast::ast;
use x3_lang_ast::ast::*;
use x3_lang_common::{BinOp, Span, UnOp};

pub type LoweredInstr = Operation;

/// The IR condition a guard states.
///
/// A guard with a value compares against it. A guard without one asserts a
/// property, and what the property is *of* is its subject — or, when it names
/// nothing at all (`require mainnet_safe`), the kind itself. Nothing is dropped
/// either way: what must hold is recorded in the IR's own shape, and the readers
/// that compare a value (`verify_mainnet_solver_bond`, `verify_slippage_safe`)
/// only look at kinds the parser refuses to write without one.
fn guard_condition(guard: &ast::RequireGuard) -> Result<Condition, x3_lang_common::X3Error> {
    if let Some(value) = &guard.value {
        return expression_to_condition(value);
    }
    let named = guard
        .subject
        .as_ref()
        .map(|subject| subject.as_str())
        .unwrap_or_else(|| guard.kind.as_str());
    Ok(Condition::Expression {
        expr: named.to_string(),
    })
}

/// Context for lowering operations
pub struct LowerCtx {
    /// Unique nonce for replay protection
    pub nonce: Option<String>,
    /// Chain ID for this context
    pub chain_id: Option<u64>,
}

impl LowerCtx {
    pub fn new() -> Self {
        LowerCtx {
            nonce: None,
            chain_id: None,
        }
    }
}

/// Lower an entire program to X3IR
pub fn lower_program(program: &Program, ctx: LowerCtx) -> Result<X3IR, x3_lang_common::X3Error> {
    lower_program_with_mode(program, ctx, CompilationMode::Dev)
}

/// Lower an entire program using the requested compilation mode for every
/// trading analysis and verification pass.
pub fn lower_program_with_mode(
    program: &Program,
    ctx: LowerCtx,
    mode: CompilationMode,
) -> Result<X3IR, x3_lang_common::X3Error> {
    let mut ir = X3IR::new();
    ir.metadata.nonce = ctx.nonce;
    ir.metadata.chain_id = ctx.chain_id;

    let trading_symbols = if program.items.iter().any(|item| {
        matches!(
            item.node,
            Item::AssetDecl(_) | Item::TradeRiskPolicy(_) | Item::AtomicTrade(_)
        )
    }) {
        let symbols = trading_semantic::analyze_trading(program, mode).map_err(|errors| {
            errors
                .into_iter()
                .next()
                .unwrap_or_else(|| x3_lang_common::X3Error::InternalError {
                    message: "trading analysis failed without a diagnostic".to_string(),
                })
        })?;
        let errors = trading_verify::verify_trading_program(program, &symbols, mode);
        if let Some(error) = errors.into_iter().next() {
            return Err(error);
        }
        Some(symbols)
    } else {
        None
    };

    // Lower top-level declarations into operations
    for item in &program.items {
        // Route fallbacks are verified before they are lowered: each approved
        // substitution is turned back into the route it would produce and run
        // through the same layers, so "compiler-approved" means the substitute
        // was actually checked rather than merely enumerated.
        for body in item_bodies(&item.node) {
            verify_route_fallbacks_in(body, &ir.metadata, mode)?;
        }
        match &item.node {
            Item::AtomicTrade(trade) => {
                let symbols = trading_symbols
                    .as_ref()
                    .ok_or_else(|| x3_lang_common::X3Error::SemanticError {
                        message: "trading symbols unavailable during lowering".to_string(),
                        span: Span::DUMMY,
                    })?;
                let operations =
                    trading_lowering::lower_atomic_trade(trade, symbols).map_err(x3_lang_common::X3Error::from)?;
                ir.operations.extend(operations);
            }
            Item::Function(func) => {
                // Lower function body as a sequence of operations
                lower_annotations_prefix(&func.annotations, &mut ir)?;
                lower_function_body(&func.body, &mut ir)?;
                lower_annotations_suffix(&func.annotations, &mut ir)?;
            }
            Item::GpuBlock(block) => {
                ir.push(Operation::GpuDispatch {
                    kernel: "inline_gpu_block".to_string(),
                    args: vec![format!("{:?}", block.body)],
                    is_simd: block.is_simd,
                });
            }
            Item::SimulateDecl(sim) => {
                let mut body_ir = X3IR::new();
                lower_function_body(&sim.body, &mut body_ir)?;
                ir.push(Operation::Simulate {
                    body: body_ir.operations,
                    receipt_slot: sim
                        .receipt
                        .as_ref()
                        .map(|sym| sym.as_str().to_string())
                        .unwrap_or_else(|| format!("{}_receipt", sim.name.as_str())),
                });
            }
            Item::ScheduledTask(task) => {
                let mut entry_ir = X3IR::new();
                lower_function_body(&task.body, &mut entry_ir)?;
                ir.push(Operation::ScheduledDispatch {
                    period_blocks: task.period_blocks as u32,
                    entry: entry_ir.operations,
                });
            }
            Item::IntentDecl(intent) => {
                ir.push(Operation::IntentResolve {
                    constraints: intent.constraints.iter().map(expression_to_string).collect(),
                    resolver: intent.name.as_str().to_string(),
                });
                lower_function_body(&intent.body, &mut ir)?;
            }
            Item::SubscriptionDecl(sub) => {
                // The cadence travels with the charge. It was read off the declaration and dropped
                // here — `subscription keeper: 100, 30 { … }` charged `keeper` 100 with nothing
                // saying how often, so the one fact that makes a subscription periodic was the one
                // fact the host could not see. A host call is a list of strings, so the period is
                // the third: name, amount, period in blocks.
                ir.push(Operation::Call {
                    function: "charge_subscription".to_string(),
                    args: vec![
                        sub.name.as_str().to_string(),
                        sub.amount.to_string(),
                        sub.period_blocks.to_string(),
                    ],
                });
                lower_function_body(&sub.body, &mut ir)?;
            }
            Item::ParallelDecl(parallel) => {
                // Lower and verify every leg, then let the dependency DAG decide
                // what may run concurrently. The legs are verified first because
                // the DAG is built from what their operations *do* — a leg that
                // failed verification would contribute a misleading read/write
                // set to the analysis that decides the plan.
                let mut lowered: Vec<(String, Vec<Operation>)> = Vec::with_capacity(parallel.legs.len());
                for leg in &parallel.legs {
                    let mut body = X3IR::new();
                    body.metadata = ir.metadata.clone();
                    body.push(Operation::AtomicBegin);
                    for statement in &leg.body {
                        lower_statement(statement, &mut body)?;
                    }
                    body.push(Operation::AtomicEnd);

                    let mut errors = crate::ir_level_errors(&body);
                    errors.extend(
                        crate::semantic::verify_collect(
                            &body,
                            crate::semantic::DEFAULT_MAX_ATOMIC_OPS,
                            crate::semantic::DEFAULT_MAX_ROUTE_HOPS,
                            Some(mode),
                        )
                        .errors,
                    );
                    if let Some(first) = errors.into_iter().next() {
                        return Err(semantic(&format!(
                            "parallel '{}' leg '{}': {first}",
                            parallel.name.as_str(),
                            leg.name.as_str()
                        )));
                    }
                    // Replay protection is a property of the program, and a leg
                    // is the only place a nonce guard can be written — so a leg
                    // that declares one declares it for the program. Two legs
                    // disagreeing about it is ambiguous rather than ordered:
                    // there is one artifact, and it cannot carry two nonces.
                    if let Some(nonce) = body.metadata.nonce.clone() {
                        match &ir.metadata.nonce {
                            None => ir.metadata.nonce = Some(nonce),
                            Some(existing) if *existing == nonce => {}
                            Some(existing) => {
                                return Err(semantic(&format!(
                                    "parallel '{}' leg '{}' declares nonce '{nonce}' but the program \
                                     already carries '{existing}'; one artifact cannot carry two \
                                     replay-protection nonces",
                                    parallel.name.as_str(),
                                    leg.name.as_str()
                                )))
                            }
                        }
                    }
                    lowered.push((leg.name.as_str().to_string(), body.operations));
                }

                // Two legs that produce the same asset have a race nothing in
                // the program resolves, so the plan is refused rather than
                // ordered arbitrarily. A dependency the program *does* express —
                // one leg consuming what another produces — becomes an edge.
                // Which VM family each chain runs on, from the program's own
                // declarations. Two declarations disagreeing about one chain
                // would make the plan's answer to "which VM runs this leg"
                // ambiguous, so that is refused rather than picked between.
                let mut chain_domains: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
                for item in &program.items {
                    let (chain, domain) = match &item.node {
                        Item::VmDecl(vm) => (vm.chain.as_str().to_string(), vm.adapter.as_str().to_string()),
                        Item::VenueDecl(venue) => (venue.chain.as_str().to_string(), venue.domain.as_str().to_string()),
                        _ => continue,
                    };
                    if let Some(existing) = chain_domains.get(&chain) {
                        if existing != &domain {
                            return Err(semantic(&format!(
                                "chain '{chain}' is declared on two domains ('{existing}' and '{domain}'); \
                                 the plan could not say which VM executes a leg on it"
                            )));
                        }
                    }
                    chain_domains.insert(chain, domain);
                }

                let legs: Vec<crate::dag::Leg> = lowered
                    .iter()
                    .map(|(name, operations)| crate::dag::leg_from_operations(name, operations))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|error| {
                        semantic(&format!(
                            "parallel '{}': {}",
                            parallel.name.as_str(),
                            describe_race(&error)
                        ))
                    })?;
                let plan = crate::dag::plan(&legs, &chain_domains).map_err(|race| {
                    semantic(&format!(
                        "parallel '{}': {}",
                        parallel.name.as_str(),
                        describe_race(&race)
                    ))
                })?;

                let order: Vec<&str> = plan.waves.iter().flatten().map(|name| name.as_str()).collect();
                ir.push(Operation::ParallelPlan {
                    waves: plan.waves.clone(),
                    edges: plan.edges.clone(),
                    domains: plan.domains.clone(),
                    settlement: plan.settlement.clone(),
                });
                for name in order {
                    let (_, operations) = lowered
                        .iter()
                        .find(|(leg_name, _)| leg_name == name)
                        .expect("the plan only names legs that were lowered");
                    ir.operations.extend(operations.iter().cloned());
                }
            }
            Item::AtomicChoice(choice) => {
                // Select the branch, then emit that branch. This is where
                // "bounded" becomes real: the artifact contains one path's
                // operations and a record of the whole branch set, so there is
                // no run-time code path that can reach a path the compiler did
                // not verify. Selection is compile-time because the ranking
                // data (`net_output`, hop counts) is, which is also why the
                // verifier refuses a `net_output` it cannot evaluate.
                let selected = select_choice_path(choice).ok_or_else(|| {
                    semantic(&format!(
                        "atomic_choice '{}': no path could be selected from {} declared path(s)",
                        choice.name.as_str(),
                        choice.paths.len()
                    ))
                })?;

                // Every branch is lowered and verified, not only the winner.
                // "Type-check every branch" is the contract, and a branch set
                // where the compiler checked only the path it happened to pick
                // would mean its promise covers code nobody looked at. Each
                // branch is verified inside the same atomic scope it will
                // execute in, with the program's own metadata, so a branch's
                // checks see exactly the context the program has — including
                // the per-branch atomic operation bound, which is per branch
                // precisely because only one branch runs.
                let mut verified: Vec<Vec<Operation>> = Vec::with_capacity(choice.paths.len());
                for path in &choice.paths {
                    let mut branch = X3IR::new();
                    branch.metadata = ir.metadata.clone();
                    branch.push(Operation::AtomicBegin);
                    for statement in &path.body {
                        lower_statement(statement, &mut branch)?;
                    }
                    branch.push(Operation::AtomicEnd);

                    let mut errors = crate::ir_level_errors(&branch);
                    errors.extend(
                        crate::semantic::verify_collect(
                            &branch,
                            crate::semantic::DEFAULT_MAX_ATOMIC_OPS,
                            crate::semantic::DEFAULT_MAX_ROUTE_HOPS,
                            Some(mode),
                        )
                        .errors,
                    );
                    if let Some(first) = errors.into_iter().next() {
                        return Err(semantic(&format!(
                            "atomic_choice '{}' path '{}': {first}",
                            choice.name.as_str(),
                            path.name.as_str()
                        )));
                    }
                    verified.push(branch.operations);
                }

                ir.push(Operation::AtomicChoice {
                    paths: choice.paths.len() as u32,
                    criterion: choice.criterion,
                    selected: selected as u32,
                });
                ir.operations.extend(verified.swap_remove(selected));
            }
            Item::AtomicSwap(atomic) => {
                // ── Semantic validation ──────────────────────────────────
                // Validate hash function name if hashlock is specified
                if let Some(hashlock) = &atomic.hashlock {
                    let hash_fn = hashlock.hash_fn.as_str();
                    match hash_fn {
                        "sha256" | "blake2b" | "keccak256" | "ripemd160" => {}
                        _ => {
                            return Err(semantic(&format!(
                                "unsupported hash function '{hash_fn}' in atomic swap \
                                 hashlock; supported: sha256, blake2b, keccak256, ripemd160"
                            )));
                        }
                    }
                }

                // Validate source and destination chains map to known VM families
                // via parse_vm_family (called during parsing, but verify non-None)
                let src_chain = atomic.from_asset.chain.as_str();
                let dst_chain = atomic.to_asset.chain.as_str();
                let src_known = KNOWN_CHAIN_PREFIXES.iter().any(|p| src_chain.starts_with(p));
                let dst_known = KNOWN_CHAIN_PREFIXES.iter().any(|p| dst_chain.starts_with(p));
                if !src_known {
                    return Err(semantic(&format!(
                        "source chain '{src_chain}' is not a known chain prefix; \
                         expected one of: {}",
                        KNOWN_CHAIN_PREFIXES.join(", ")
                    )));
                }
                if !dst_known {
                    return Err(semantic(&format!(
                        "destination chain '{dst_chain}' is not a known chain prefix; \
                         expected one of: {}",
                        KNOWN_CHAIN_PREFIXES.join(", ")
                    )));
                }

                // Validate timeout ranges (non-zero, not absurdly large)
                if let Some(timeout) = &atomic.timeout_source {
                    let blocks = expression_to_blocks(timeout)?;
                    if blocks == 0 {
                        return Err(semantic("source timeout must be greater than 0 blocks"));
                    }
                    if blocks > MAX_TIMEOUT_BLOCKS {
                        return Err(semantic(&format!(
                            "source timeout of {blocks} blocks exceeds maximum {}",
                            MAX_TIMEOUT_BLOCKS
                        )));
                    }
                }
                if let Some(timeout) = &atomic.timeout_destination {
                    let blocks = expression_to_blocks(timeout)?;
                    if blocks == 0 {
                        return Err(semantic("destination timeout must be greater than 0 blocks"));
                    }
                    if blocks > MAX_TIMEOUT_BLOCKS {
                        return Err(semantic(&format!(
                            "destination timeout of {blocks} blocks exceeds maximum {}",
                            MAX_TIMEOUT_BLOCKS
                        )));
                    }
                }

                // Validate source and destination chains are different
                let src_chain_str = atomic.from_asset.chain.as_str();
                let dst_chain_str = atomic.to_asset.chain.as_str();
                if src_chain_str == dst_chain_str {
                    return Err(semantic(&format!(
                        "atomic swap source and destination chains are the same \
                         ('{src_chain_str}'); cross-chain swap must target a different chain"
                    )));
                }

                // Validate amount is positive if specified
                if let Some(amt_expr) = &atomic.amount {
                    let amount_val = expression_to_u128(amt_expr)?;
                    if amount_val == 0 {
                        return Err(semantic("atomic swap amount must be greater than 0"));
                    }
                }

                // Wrap in atomic block and lower body
                ir.push(Operation::AtomicBegin);

                // Add requires guards first
                for require in &atomic.requires {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&require.kind),
                        subject: require.subject.as_ref().map(|s| s.as_str().to_string()),
                        condition: guard_condition(require)?,
                        error_msg: None,
                        measured: false,
                        comparison: require.comparison,
                    });
                }

                // If amount is specified, add a lock operation for the source asset
                if let Some(amt) = &atomic.amount {
                    let amount_val = expression_to_u128(amt)?;
                    let receiver_str = atomic
                        .receiver
                        .as_ref()
                        .map(expression_to_string)
                        .unwrap_or_else(|| "receiver".to_string());
                    // Lock on source chain: funds come from the sender, not the receiver.
                    ir.push(Operation::Lock {
                        chain: atomic.from_asset.chain.as_str().to_string(),
                        asset: atomic.from_asset.name.as_str().to_string(),
                        amount: amount_val,
                        from: "sender".to_string(),
                    });
                    // Release on destination chain to the receiver.
                    ir.push(Operation::Release {
                        chain: atomic.to_asset.chain.as_str().to_string(),
                        asset: atomic.to_asset.name.as_str().to_string(),
                        to: receiver_str,
                        // A **payout**, not a claim: this pays out the asset the route
                        // delivered. The escrow the source lock created is claimed by the
                        // route's own settlement, and reading this as a claim is what made
                        // `no_refund_after_claim` warn on every canonical example (TICKET-035).
                        act: ReleaseAct::Payout,
                    });
                }

                // Lower body statements
                for stmt in &atomic.body {
                    lower_statement(stmt, &mut ir)?;
                }

                // Add source timeout handling if specified
                if let Some(timeout_expr) = &atomic.timeout_source {
                    ir.push(Operation::OnTimeout {
                        duration_blocks: expression_to_blocks(timeout_expr)?,
                        action: ir::FailureAction::Refund {
                            chain: atomic.from_asset.chain.as_str().to_string(),
                            asset: atomic.from_asset.name.as_str().to_string(),
                            to: "sender".to_string(),
                        },
                    });
                }

                // Add destination timeout handling if specified
                if let Some(timeout_expr) = &atomic.timeout_destination {
                    ir.push(Operation::OnTimeout {
                        duration_blocks: expression_to_blocks(timeout_expr)?,
                        action: ir::FailureAction::Refund {
                            chain: atomic.to_asset.chain.as_str().to_string(),
                            asset: atomic.to_asset.name.as_str().to_string(),
                            to: "sender".to_string(),
                        },
                    });
                }

                // Add failure handling if specified
                if let Some(failure) = &atomic.on_fail {
                    ir.push(Operation::OnFail {
                        action: failure_action_to_ir(failure),
                    });
                }
                ir.push(Operation::AtomicEnd);
            }
            Item::Bridge(bridge) => {
                // Lower bridge as atomic sequence with requires
                ir.push(Operation::AtomicBegin);

                // Add requires guards first
                for require in &bridge.requires {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&require.kind),
                        subject: require.subject.as_ref().map(|s| s.as_str().to_string()),
                        condition: guard_condition(require)?,
                        error_msg: None,
                        measured: false,
                        comparison: require.comparison,
                    });
                }

                // Lower bridge body
                for stmt in &bridge.body {
                    lower_statement(stmt, &mut ir)?;
                }

                // Add failure handling if specified
                if let Some(failure) = &bridge.on_fail {
                    ir.push(Operation::OnFail {
                        action: failure_action_to_ir(failure),
                    });
                }

                // Add timeout handling if specified
                if let Some(timeout_expr) = &bridge.timeout {
                    ir.push(Operation::OnTimeout {
                        duration_blocks: expression_to_blocks(timeout_expr)?,
                        action: ir::FailureAction::Refund {
                            chain: bridge.from_asset.chain.as_str().to_string(),
                            asset: bridge.from_asset.name.as_str().to_string(),
                            to: "sender".to_string(),
                        },
                    });
                }

                ir.push(Operation::AtomicEnd);
            }
            Item::Strategy(strategy) => {
                // The licence is a record, and it is pushed before the body so a
                // reader meets the terms before the work.
                if strategy.license.is_some() || strategy.split.is_some() {
                    let (creator, royalty_bps, executions, expires_block) = strategy
                        .license
                        .as_ref()
                        .map(|license| {
                            (
                                license.creator.as_str().to_string(),
                                license.profit_share_bps,
                                license.executions,
                                license.expires_block,
                            )
                        })
                        .unwrap_or_else(|| (String::new(), 0, None, None));
                    ir.push(Operation::StrategyLicense {
                        creator,
                        royalty_bps,
                        executions,
                        expires_block,
                        split: strategy
                            .split
                            .as_ref()
                            .map(|split| {
                                split
                                    .shares
                                    .iter()
                                    .map(|(recipient, bps)| (recipient.as_str().to_string(), *bps))
                                    .collect()
                            })
                            .unwrap_or_default(),
                    });
                }
                if let Some(submission) = &strategy.submission {
                    // The policy becomes a mode check rather than a comment: the
                    // runtime refuses a program whose compiled policy requires
                    // privacy when it has no private channel, which is what
                    // PHASE 28 asks for.
                    ir.push(Operation::ModeCheck {
                        mode: "submission".to_string(),
                        restriction: format!("private_{}", submission.private.as_str()),
                    });
                }
                if let Some(risk) = &strategy.risk {
                    // The fee ceiling the module states travels with it. The *slippage* ceiling
                    // reaches the artifact through the guard the body writes (`require slippage <=
                    // 50` lowers to a `SlippageTolerance` record whose figure the emitter carries),
                    // but no statement writes a fee ceiling, so a runtime reading only the artifact
                    // could not tell what the module accepts. PHASE 7: "risk policy must compile
                    // into the artifact". Same shape as the finality policy's depth: a declaration,
                    // recorded with the figure it states, and decided at compile time by the check
                    // that compares it against the venues the body routes through.
                    ir.push(Operation::Require {
                        kind: ir::RequireKind::FeeCeiling,
                        subject: None,
                        condition: ir::Condition::Expression {
                            expr: risk.max_total_fee_bps.to_string(),
                        },
                        error_msg: None,
                        measured: false,
                        comparison: Some(ir::ComparisonOp::LessOrEqual),
                    });
                }
                // Lower strategy as constrained execution
                ir.push(Operation::AtomicBegin);

                // Add requires guards first
                for require in &strategy.requires {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&require.kind),
                        subject: require.subject.as_ref().map(|s| s.as_str().to_string()),
                        condition: guard_condition(require)?,
                        error_msg: None,
                        measured: false,
                        comparison: require.comparison,
                    });
                }

                // Lower strategy body (limited by max_steps)
                for stmt in &strategy.body {
                    lower_statement(stmt, &mut ir)?;
                }

                // Add failure handling if specified
                if let Some(failure) = &strategy.on_fail {
                    ir.push(Operation::OnFail {
                        action: failure_action_to_ir(failure),
                    });
                }

                ir.push(Operation::AtomicEnd);
            }
            Item::VmDecl(vm) => {
                ir.push(Operation::VmAdapterCall {
                    vm: vm.chain.as_str().to_string(),
                    adapter: vm.adapter.as_str().to_string(),
                    calldata: String::new(),
                });
                if let Some(finality) = &vm.finality {
                    ir.push(Operation::ModeCheck {
                        mode: "finality".to_string(),
                        restriction: finality.as_str().to_string(),
                    });
                }
            }
            // `solver_market { mode, min_reputation }` configures the
            // marketplace; it is not a bid, and there is nothing for the VM to
            // execute.
            //
            // This used to lower into an executable `SolverBid` built entirely
            // from neighbouring fields — `solver: mode`, empty receive/deliver
            // assets, an empty fee, and `bond: min_reputation`. A reputation
            // threshold is not a bond, and the executor rightly refused the
            // result ("solver bid: fee must be non-empty"), which made every
            // program that configured a solver market fail at run time. The
            // bond a program actually relies on is the guard it writes,
            // `require solver_bond >= N`, which `verify_solver_bond` reads.
            Item::SolverMarket(_market) => {}
            Item::RelayerSwarm(swarm) => {
                ir.push(Operation::RelayerAttest {
                    relayers: swarm.relayers.iter().map(|r| r.as_str().to_string()).collect(),
                    quorum: (swarm.quorum_numerator, swarm.quorum_denominator),
                    signatures: Vec::new(),
                });
            }
            Item::RpcQuorum(quorum) => {
                ir.push(Operation::RpcConsensus {
                    chain: quorum.source.as_str().to_string(),
                    require: (quorum.require_numerator, quorum.require_denominator),
                    reject_on: quorum.reject_on.iter().map(|r| r.as_str().to_string()).collect(),
                });
            }
            Item::RiskPolicy(policy) => {
                // A slippage bound is a slippage bound. This used to emit
                // `RiskScore { score: max_slippage }` — a *different* quantity
                // whose own range is 0..=100, so `risk_policy { max_slippage 120 }`
                // (a 120% slippage bound, which is what the corpus's guards write)
                // became a risk score the VM refuses and the program stopped
                // running. As a requirement it is (a) the same kind as
                // `require slippage <= n`, so the policy and the guard are in one
                // unit, and (b) visible to the mainnet ceiling, which reads
                // exactly this operation and could not see the policy at all
                // before. (TICKET-052.)
                if policy.max_slippage > 0 {
                    ir.push(Operation::Require {
                        kind: ir::RequireKind::SlippageTolerance,
                        subject: None,
                        condition: Condition::Expression {
                            expr: policy.max_slippage.to_string(),
                        },
                        error_msg: None,
                        measured: false,
                        comparison: Some(ir::ComparisonOp::LessOrEqual),
                    });
                }
            }
            Item::PrivacyBlock(privacy) => {
                ir.push(Operation::PrivacyCommit {
                    reveal_on: privacy.reveal_on.as_str().to_string(),
                    encrypted: privacy.encrypted,
                });
            }
            Item::InvariantDecl(inv) => {
                ir.push(Operation::InvariantCheck {
                    name: inv.name.as_str().to_string(),
                    assert_expr: inv.assert_expr.as_str().to_string(),
                });
            }
            Item::FinalityPolicy(fp) => {
                ir.push(Operation::Require {
                    kind: ir::RequireKind::FinalityExplicit,
                    subject: Some(fp.chain.as_str().to_string()),
                    // Typed rather than a rendered string, so the depth the
                    // declaration states reaches the artifact (TICKET-059).
                    condition: ir::Condition::FinalityPolicy {
                        name: fp.mode.as_str().to_string(),
                        requirement: fp.requirement.as_str().to_string(),
                        blocks: fp.blocks,
                    },
                    error_msg: None,
                    measured: false,
                    comparison: None,
                });
            }
            Item::VenueDecl(venue) => {
                // A venue's *attributes* reach the artifact through the plan a route
                // produced, and they always did. Its **settlement guarantee** did not:
                // the plan carries which legs run, not how each one finally settles, so
                // a reader of the artifact could not tell a leg the VM executes both
                // sides of from one an off-chain venue fills and something else makes
                // whole. PHASE 39's whole point is that distinction, so it travels here
                // the way the finality depth does (TICKET-059).
                ir.push(Operation::VenueSettlement {
                    venue: venue.name.as_str().to_string(),
                    guarantee: venue.settlement,
                });
            }
            Item::ProofsRequired(proofs) => {
                for proof in &proofs.proofs {
                    ir.push(Operation::ProofRequired {
                        proof_type: proof.as_str().to_string(),
                        source: "intent".to_string(),
                    });
                }
            }
            Item::VmTarget(target) => {
                ir.push(Operation::VmAdapterCall {
                    vm: target.vm.as_str().to_string(),
                    adapter: target.adapter.as_str().to_string(),
                    calldata: target
                        .contract
                        .as_ref()
                        .map(|c| c.as_str().to_string())
                        .unwrap_or_default(),
                });
            }
            Item::Proposal(proposal) => {
                ir.push(Operation::AtomicBegin);
                for stmt in &proposal.body {
                    lower_statement(stmt, &mut ir)?;
                }
                for req in &proposal.requires {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&req.kind),
                        subject: req.subject.as_ref().map(|s| s.as_str().to_string()),
                        condition: guard_condition(req)?,
                        error_msg: None,
                        measured: false,
                        comparison: req.comparison,
                    });
                }
                ir.push(Operation::AtomicEnd);
            }
            Item::Agent(agent) => {
                for method in &agent.methods {
                    lower_annotations_prefix(&method.node.annotations, &mut ir)?;
                    lower_function_body(&method.node.body, &mut ir)?;
                    lower_annotations_suffix(&method.node.annotations, &mut ir)?;
                }
                for strategy in &agent.strategies {
                    lower_function_body(&strategy.node.body, &mut ir)?;
                }
            }
            Item::Rebalance(rebalance_decl) => {
                // The decided portfolio travels in the operation: `x3c lower` is where
                // the targets and the criterion are visible today, because no plan can
                // be generated for them yet.
                let portfolio = rebalance::portfolio(rebalance_decl).map_err(|reason| semantic(&reason))?;
                ir.push(Operation::Rebalance {
                    name: portfolio.name.clone(),
                    // What the account holds now, when the program states it. The trades to
                    // the target are computed from where the portfolio starts, so this is
                    // the input the target alone leaves a host without (TICKET-070).
                    holdings: portfolio.holdings.clone(),
                    weights: portfolio.weights.clone(),
                    criterion: portfolio.criterion.name().to_string(),
                });
            }
            Item::Netting(netting_decl) => {
                // The book lowers to the transfers that settle it: one lock and one
                // release per residual, inside one atomic block, against the accounts the
                // book binds each party to. The offsets are decided (`netting::verify`)
                // and this is what executes them.
                let settled = netting::settlement(netting_decl).map_err(|reason| semantic(&reason))?;
                // **One atomic route for the whole book**, which is what makes netting valid:
                // if some residual transfers settle and others do not, the positions that
                // result are not the positions the offsetting preserved.
                //
                // It used to be one route per transfer, because a `Release` named its claim by
                // asset alone — so two transfers of one asset in a route were two claims no
                // reader could tell apart, and `no_double_claim` refused them. A book nets
                // *within* one asset, so that was the normal case rather than a corner. Each
                // release names its own lock's position among this route's locks now, so the
                // claims are distinguishable and one route settles the set (TICKET-080).
                ir.push(Operation::AtomicBegin);
                for (index, transfer) in settled.transfers.iter().enumerate() {
                    // The debtor's value is locked before it is released: a release with
                    // nothing locked in front of it is a mint, and the pair is the idiom
                    // every other settlement path in this language uses.
                    ir.push(Operation::Lock {
                        chain: transfer.domain.clone(),
                        asset: transfer.asset.clone(),
                        amount: transfer.amount,
                        from: transfer.debtor_account.clone(),
                    });
                    ir.push(Operation::Release {
                        chain: transfer.domain.clone(),
                        asset: transfer.asset.clone(),
                        to: transfer.creditor_account.clone(),
                        // This transfer's own lock, counted among this route's locks in the
                        // order they are written — a claim of the escrow written just above.
                        act: ReleaseAct::Claims(
                            u32::try_from(index)
                                .map_err(|_| semantic("a book has more transfers than a claim index can name"))?,
                        ),
                    });
                }
                // No refund handler, and none is wanted: a route that fails rolls back, so no
                // lock takes effect and no value left. A handler here would refund an escrow
                // this route claims, which `no_refund_after_claim` refuses — correctly,
                // because the handler would describe a path the route cannot reach.
                ir.push(Operation::AtomicEnd);
            }
            Item::Arb(arb_decl) => {
                // The declaration lowers to the *plan*: an atomic block holding the asset
                // cycle the search found, the venues it may use, and the floors the
                // runtime enforces. There is no marker operation, because the plan is
                // what a reader needs — the scope's job was to filter, and the filter's
                // result is the cycle.
                let planned = arb::plan(program, arb_decl).map_err(|reason| semantic(&reason))?;

                ir.push(Operation::AtomicBegin);
                // A choice is recorded only when there was one: with a single candidate
                // the compiler decided nothing, and an `AtomicChoice` over one path
                // would be the verifier's "a choice with fewer than two paths is not a
                // choice".
                if planned.candidates > 1 {
                    ir.push(Operation::AtomicChoice {
                        paths: planned.candidates as u32,
                        criterion: planned.criterion,
                        selected: 0,
                    });
                }
                // The input amount is the one the declaration states; the per-hop outputs
                // are the host's, because every intermediate amount depends on a price
                // this compiler does not have.
                ir.push(Operation::MultiHopSwap {
                    path: planned.cycle.assets.clone(),
                    amount: planned.capital.0,
                });
                // The venues the compiler approved for this route, carried rather than
                // implied: a runtime can only restrict itself to the compiler's
                // approvals if the approvals are in the artifact.
                ir.push(Operation::RouteFallback {
                    approved: planned.cycle.venues.clone(),
                });
                for guard in &planned.guards {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&guard.kind),
                        subject: None,
                        condition: guard_condition(&bps_guard(guard))?,
                        error_msg: None,
                        // The plan's floors are post-conditions: judged against what the
                        // host reports for the trade, in basis points, and refused when
                        // nothing reported one. A guard a program writes is a constraint the
                        // compiler checks against declarations and stays `measured: false`.
                        measured: true,
                        comparison: Some(guard.comparison),
                    });
                }
                ir.push(Operation::AtomicEnd);
            }
            // Explicit rather than left to the catch-all below: `Item` has an arm for
            // "other items generate no operations", and a declaration that fell into it
            // would vanish from the artifact silently (TICKET-078).
            Item::Hyperarb(hyperarb_decl) => {
                // The declaration lowers to the route its `choose` clause selects: an
                // atomic block holding the selection, the chosen leg's host call, the
                // venues the compiler approved, and the net-profit floor as a runtime
                // guard.
                let planned = hyperarb::plan(program, hyperarb_decl).map_err(|reason| semantic(&reason))?;
                let leg = planned
                    .legs
                    .get(planned.selected)
                    .ok_or_else(|| semantic("the hyperarb selected no leg"))?
                    .clone();

                ir.push(Operation::AtomicBegin);
                ir.push(Operation::AtomicChoice {
                    paths: planned.legs.len() as u32,
                    criterion: planned.criterion,
                    selected: planned.selected as u32,
                });
                // The input amount is the capital the declaration commits; the leg's
                // outputs are the host's, because every output depends on a price this
                // compiler does not have.
                ir.push(Operation::MultiHopSwap {
                    path: leg.path.clone(),
                    amount: planned.capital.0,
                });
                ir.push(Operation::RouteFallback {
                    approved: leg.approved.clone(),
                });
                // The floor travels as a guard the runtime evaluates, for the same reason
                // the arb plan's does: the compiler can bound the trade and cannot measure
                // it.
                let floor = arb::Guard {
                    kind: ast::RequireKind::Profit,
                    comparison: ast::ComparisonOp::GreaterOrEqual,
                    bps: planned.net_profit_bps,
                };
                ir.push(Operation::Require {
                    kind: require_kind_to_ir(&floor.kind),
                    subject: None,
                    condition: guard_condition(&bps_guard(&floor))?,
                    error_msg: None,
                    // A post-condition, like the arb plan's floor (TICKET-027).
                    measured: true,
                    comparison: Some(floor.comparison),
                });
                ir.push(Operation::AtomicEnd);
            }
            Item::AtomicLiquidation(liquidation_decl) => {
                // The declaration lowers to the *plan*: the two venue calls, the conversion
                // with the floor the declaration states, and the net-profit guard. Every
                // figure comes from the ledger the verifier decided, so a replayer can
                // re-check the plan from the artifact.
                //
                // Unlike a hedge's legs, the conversion's amounts are known: the declaration
                // states the swap's input and its `min_output`, which is what makes this a
                // `Swap` rather than a route the compiler would have to resolve. The venue is
                // not named, and `dex: None` says so — picking one would be choosing a market
                // the program never wrote down.
                let ledger = liquidation::ledger(liquidation_decl).map_err(|reason| semantic(&reason))?;

                // The conversion is a swap, and `verify_slippage_explicit` refuses a swap leg
                // in an artifact with no explicit slippage bound. A liquidation states a
                // profit floor and no ceiling, so the ceiling has to come from the program —
                // refused here with the rule named rather than emitted into an artifact the
                // next layer rejects (the same shape as a hyperarb's plan).
                let bounded = crate::semantic::require_guards(program)
                    .into_iter()
                    .any(|(_, guard)| guard.kind == ast::RequireKind::Slippage);
                if !bounded {
                    return Err(semantic(&format!(
                        "the liquidation of '{}' plans a conversion and this program declares no \
                         `require slippage <= <n>` bound; a swap without an explicit ceiling is a \
                         leg whose price is unconstrained, so write the bound where the trade is \
                         bounded",
                        ledger.position
                    )));
                }

                let (collateral_chain, collateral_name) = ledger.collateral_parts();
                let (debt_chain, debt_name) = ledger.debt_parts();

                ir.push(Operation::AtomicBegin);
                // The two calls the VM has no native form for, asked of a host in the one
                // shape that can carry them: an action, what it is about (the position), the
                // asset, and the quantity.
                ir.push(Operation::VenueOrder {
                    action: "liquidate".to_string(),
                    subject: ledger.position.clone(),
                    asset: ledger.debt_asset.clone(),
                    quantity: ledger.capital,
                });
                ir.push(Operation::VenueOrder {
                    action: "receive_collateral".to_string(),
                    subject: ledger.position.clone(),
                    asset: ledger.collateral_asset.clone(),
                    quantity: ledger.collateral,
                });
                ir.push(Operation::Swap {
                    from_chain: collateral_chain,
                    from_asset: collateral_name,
                    to_chain: debt_chain,
                    to_asset: debt_name,
                    input_amount: ledger.swapped_in,
                    min_output: ledger.min_output,
                    dex: None,
                });
                // The floor travels as a **post-condition**. It used to be a constraint,
                // because the conversion is a `Swap` — an asset-op *record* the executor
                // resolves locally from the declaration's own amounts, so no reply could
                // carry what was seized (TICKET-069).
                //
                // It does not need the conversion to change shape. The quantity the floor is
                // about is the **net the seizure realised**, and the venue orders above are
                // the calls that did the seizing: `liquidate` and `receive_collateral` reach
                // the host, and a venue that reports a net answers this guard. The compile-time
                // check stays what it was — `liquidation::verify` refuses a swap whose declared
                // minimum cannot repay — so the compiler bounds the plan and the runtime
                // measures it, the same division of labour as a plan's profit floor
                // (TICKET-027) and a hedge's delta (TICKET-068).
                if let Some(floor) = ledger.profit_floor {
                    let guard = arb::Guard {
                        kind: ast::RequireKind::Profit,
                        comparison: ast::ComparisonOp::GreaterOrEqual,
                        bps: u16::try_from(floor)
                            .map_err(|_| semantic("a liquidation's profit floor does not fit the guard's operand"))?,
                    };
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&guard.kind),
                        subject: Some(ledger.debt_asset.clone()),
                        condition: guard_condition(&bps_guard(&guard))?,
                        error_msg: None,
                        // Measured: the quantity is the net the venue's orders realised, and
                        // the executor refuses when no venue reported one.
                        measured: true,
                        comparison: Some(guard.comparison),
                    });
                }
                ir.push(Operation::AtomicEnd);
            }
            Item::AtomicHedge(hedge_decl) => {
                // The declaration lowers to the *orders*: one atomic block asking a venue
                // for each leg, with the net the verifier checked and the bound it checked
                // it against. The legs are already known to net (`hedge::verify` runs over
                // the AST before lowering and refuses a hedge that does not), so resolving
                // them here carries the *checked* figures into the artifact rather than
                // stating a second opinion about them.
                let (exposure, orders) = hedge::orders(hedge_decl).map_err(|reason| semantic(&reason))?;

                ir.push(Operation::AtomicBegin);
                for order in &orders {
                    ir.push(Operation::VenueOrder {
                        action: order.action.to_string(),
                        subject: order.subject().to_string(),
                        asset: order.asset.clone(),
                        quantity: order.quantity,
                    });
                }
                // The bound travels as a **post-condition**. It used to be a constraint
                // on the declaration, because whether the venue filled what was asked is a
                // question the artifact could not answer — the venue reports it now, in
                // the reply to the orders above (`MEASURED_UNIT_DELTA_BPS`), and a venue
                // that reports nothing makes the guard refuse rather than pass.
                if let Some(bound) = hedge_decl.delta_bound_bps {
                    let guard = arb::Guard {
                        kind: ast::RequireKind::Custom(x3_lang_common::Symbol::new(hedge::DELTA_GUARD_SUBJECT)),
                        comparison: ast::ComparisonOp::LessOrEqual,
                        bps: u16::try_from(bound)
                            .map_err(|_| semantic("a hedge's delta bound does not fit the guard's operand"))?,
                    };
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&guard.kind),
                        subject: Some(exposure.asset.clone()),
                        condition: guard_condition(&bps_guard(&guard))?,
                        error_msg: None,
                        // Measured: the quantity is the delta the venue reported for the
                        // orders above, and the executor refuses when no venue reported one.
                        measured: true,
                        comparison: Some(guard.comparison),
                    });
                }
                ir.push(Operation::AtomicEnd);
            }
            // Every item that generates no operations, named rather than caught by a
            // wildcard. The wildcard was a trap: a *new* declaration added to `Item`
            // fell into it silently, so `x3c lower` and `x3c build` both succeeded and
            // the declaration was simply absent from the artifact — a program whose
            // plan is invisible in the very document a reader checks it against. PHASE
            // 38 hit it for real: `Item::Hyperarb` would have vanished that way, and the
            // explicit arm above is the only reason it did not. With the list written
            // out, adding a variant fails to compile until somebody decides what it
            // lowers to (TICKET-078).
            //
            // Every arm that lowers something is above; what is left is exactly the
            // items that lower to nothing, and the compiler checks that this list is
            // *complete* rather than trusting a comment.
            Item::Struct(_) | Item::Enum(_) | Item::Use(_) | Item::Mod(_) | Item::Import(_) => {
                // Type and module machinery: the compiler reads it, the artifact runs
                // operations.
            }
            Item::Const(_) => {
                // A constant is evaluated where it is used, so the artifact carries the
                // value rather than the name.
            }
            Item::ErrorDecl(_) => {
                // A name a program may raise. Raising it is a `Statement`, which
                // lowers; the declaration is the name.
            }
            Item::ObjectiveDecl(_) => {
                // What the optimizer should rank by. The ranking happens at compile
                // time, and the choice it produced is what the artifact carries.
            }
            Item::AssetDecl(_) | Item::TradeRiskPolicy(_) => {
                // Declarations the verifier and the opportunity graph read. A venue is
                // an edge in a graph, not an instruction, and its attributes reach the
                // artifact through the plan a route produced. Its settlement guarantee
                // does not — see the `Item::VenueDecl` arm above.
            }
        }
    }

    Ok(ir)
}

/// Lower a statement to IR operations
fn lower_statement(stmt: &Statement, ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    match stmt {
        Statement::Expr(expr) => {
            // Lower expression (may produce multiple operations)
            lower_expression(expr, ir)?;
        }
        Statement::If {
            cond,
            then_block,
            else_block,
        } => {
            // A condition the program makes decidable is decided *here*, and the decision is
            // what travels: `Condition::True`/`Condition::False` say the compiler knows which
            // branch runs, and the emitter writes that branch's body inline. Everything else
            // stays `Condition::Expression` and is refused downstream — this VM branches on a
            // register and the compiler emits no arithmetic to put one there (TICKET-058).
            //
            // Both bodies are still lowered into the IR, so `x3c lower` shows the branch that
            // was *not* taken next to the one that was. Dropping it would leave an artifact
            // whose reader cannot tell a folded branch from straight-line code.
            let cond_ir = match fold_condition(cond) {
                Some(true) => Condition::True,
                Some(false) => Condition::False,
                None => expression_to_condition(cond)?,
            };
            let then_ops = {
                let mut temp_ir = X3IR::new();
                lower_function_body(then_block, &mut temp_ir)?;
                temp_ir.operations
            };

            let else_ops = if let Some(else_blk) = else_block {
                let mut temp_ir = X3IR::new();
                lower_function_body(else_blk, &mut temp_ir)?;
                Some(temp_ir.operations)
            } else {
                None
            };

            ir.push(Operation::If {
                condition: cond_ir,
                then_ops,
                else_ops,
            });
        }
        Statement::While { cond, body } => {
            // The condition is decided by the same folder `if` uses, and it travels with the loop
            // either way. `while false { … }` is a body that never runs — the language's meaning,
            // and a decision the compiler may make because `fold_condition` only decides literals,
            // arithmetic on them and logical combinations, so nothing in the decision is a call
            // with a side effect. Everything else keeps its condition as `Condition::Expression`,
            // which is what lets the verifier and the emitter name the guard they refuse
            // (TICKET-098).
            let cond_ir = match fold_condition(cond) {
                Some(true) => Condition::True,
                Some(false) => Condition::False,
                None => expression_to_condition(cond)?,
            };
            let body_ops = {
                let mut temp_ir = X3IR::new();
                lower_function_body(body, &mut temp_ir)?;
                temp_ir.operations
            };

            ir.push(Operation::Loop {
                max_iterations: 1000, // Safe default limit
                condition: cond_ir,
                body: body_ops,
            });
        }
        Statement::Atomic(atomic) => {
            ir.push(Operation::AtomicBegin);
            lower_function_body(&atomic.body, ir)?;
            ir.push(Operation::AtomicEnd);
        }
        Statement::Emit(event) => {
            // `BTreeMap`: this is one of the two maps in the IR that reach the artifact's
            // bytes, so its order has to come from the program rather than from a
            // per-instance hash seed (PHASE 42).
            let mut data = std::collections::BTreeMap::new();
            for (i, arg) in event.payload.iter().enumerate() {
                // The source text of the argument, not its `Debug` form: this string is the
                // event's payload in the artifact, and `{:?}` put the compiler's own AST
                // representation in it (`Literal(Int { value: 1, base: Decimal, suffix: None })`),
                // which no consumer of an event could agree on. Same renderer every other
                // payload field uses.
                data.insert(format!("arg{}", i), expression_to_string(arg));
            }
            ir.push(Operation::Emit {
                name: event.name.as_str().to_string(),
                data,
            });
        }
        Statement::Lock {
            chain,
            asset,
            amount,
            from,
        } => {
            // lock CHAIN.ASSET amount VALUE from ADDR
            ir.push(Operation::Lock {
                chain: chain_to_string(chain),
                asset: asset.name.as_str().to_string(),
                amount: expression_to_u128(amount)?,
                from: expression_to_string(from),
            });
        }
        Statement::Mint { asset, amount, to } => {
            // mint ASSET amount VALUE to ADDR
            ir.push(Operation::Mint {
                chain: chain_to_string(&asset.chain),
                asset: asset.name.as_str().to_string(),
                amount: expression_to_u128(amount)?,
                to: expression_to_string(to),
            });
        }
        Statement::Burn { asset, amount, from } => {
            // burn ASSET amount VALUE from ADDR
            ir.push(Operation::Burn {
                chain: chain_to_string(&asset.chain),
                asset: asset.name.as_str().to_string(),
                amount: expression_to_u128(amount)?,
                from: expression_to_string(from),
            });
        }
        Statement::Release { chain, asset, to } => {
            // release CHAIN.ASSET to ADDR — a payout of an asset the program holds or a route
            // delivered, which claims no escrow (TICKET-101).
            ir.push(Operation::Release {
                chain: chain_to_string(chain),
                asset: asset.name.as_str().to_string(),
                to: expression_to_string(to),
                act: ReleaseAct::Payout,
            });
        }
        Statement::Swap {
            from,
            to,
            amount,
            min_output,
            dex,
        } => {
            // swap FROM -> TO [route ...] [dex ...]
            ir.push(Operation::Swap {
                from_chain: chain_to_string(&from.chain),
                from_asset: from.name.as_str().to_string(),
                to_chain: chain_to_string(&to.chain),
                to_asset: to.name.as_str().to_string(),
                // The *erroring* conversion: `expression_to_u128_opt` swallowed
                // the refusal a fractional amount gets and wrote 0, so
                // `min_output 0.09` reached the verifier as "min_output must be
                // greater than zero" — a report about a zero, three passes after
                // the literal that caused it, naming neither the literal nor the
                // reason. A missing amount is still 0 (nothing was written); an
                // amount that cannot be converted is an error.
                // A step that reaches lowering with no amount has nothing to be filled
                // from — the pass that fills from the intent's source endpoint
                // (`parser::fill_route_bridge_amounts`) has already run — so this is a
                // step that states no amount whose `from` is not the source. It used to
                // write 0 and let two later passes report "swap input_amount must be
                // greater than zero", which names neither the step nor the reason. Same
                // principle as the conversion comment above: a report about a zero should
                // be made where the zero was written, and this one is not a conversion
                // failure at all.
                input_amount: match amount.as_ref() {
                    Some(expr) => expression_to_u128(expr)?,
                    None => {
                        return Err(semantic(
                            "this route step states no `amount` and the compiler cannot infer one: \
                             a step takes the intent's source amount only when its `from` is that \
                             source asset, and any other step's input is what the previous leg \
                             returns — a market outcome rather than a constant. Write `amount <n>` \
                             on this step.",
                        ))
                    }
                },
                min_output: min_output.as_ref().map(expression_to_u128).transpose()?.unwrap_or(0),
                dex: dex.as_ref().map(expression_to_string),
            });
        }
        Statement::Bridge {
            via,
            from,
            to,
            amount,
            receiver,
            source_finality_proof,
            transfer_proof,
        } => {
            ir.push(Operation::Bridge {
                via: via.as_str().to_string(),
                from_chain: chain_to_string(&from.chain),
                from_asset: from.name.as_str().to_string(),
                to_chain: chain_to_string(&to.chain),
                to_asset: to.name.as_str().to_string(),
                amount: expression_to_u128(amount)?,
                receiver: expression_to_string(receiver),
                source_finality_proof: source_finality_proof
                    .as_ref()
                    .map(expression_to_string)
                    .unwrap_or_default()
                    .into_bytes(),
                transfer_proof: transfer_proof
                    .as_ref()
                    .map(expression_to_string)
                    .unwrap_or_default()
                    .into_bytes(),
            });
        }
        Statement::Require(guard) => {
            // Surface `require nonce <subject> <value>` into the
            // program-level metadata so the replay-protection rule in
            // the semantic verifier sees it. The value is the nonce
            // string the consumer is committing to.
            if matches!(guard.kind, RequireKind::Nonce) {
                // `require nonce unused <id>` — the identifier is what the
                // program commits to. The parser refuses a nonce guard without
                // one; a hand-built AST that has none is refused here rather
                // than recorded as a nonce of nothing.
                let nonce = match &guard.value {
                    Some(value) => expression_to_string(value),
                    None => {
                        return Err(x3_lang_common::X3Error::SemanticError {
                            message: "a `nonce` guard must name the nonce it commits to \
                                      (`require nonce unused <id>`); without one the replay-protection \
                                      check has nothing to look for"
                                .to_string(),
                            span: Span::DUMMY,
                        })
                    }
                };
                ir.metadata.nonce = Some(nonce.clone());
                // And put the quantity the guard reads into `r0`: whether this
                // nonce has been used. This is the one guard in the language
                // whose condition is a *run-time* fact, so it is the one that
                // emits a comparison rather than a `STATIC` assertion
                // (TICKET-027/051). The instruction goes immediately before the
                // guard because the guard tests `r0` and nothing else writes it.
                ir.push(Operation::NonceUnused { nonce });
            }
            // An economic guard is enforced by the VM against what the host measured — that is
            // what the language says it is for ("economic constraints enforced by the VM"), and
            // it is what the plan-generated floors have done since TICKET-106. Without this the
            // guard was a `STATIC` record with threshold **zero**: measured on the corpus,
            // `require slippage <= 7` and `require slippage <= 99` compiled to byte-identical
            // artifacts, so the bound reached neither the artifact nor the runtime.
            //
            // The bound travels in the instruction's operand, which is where a measured guard's
            // threshold goes, so it has to be basis points — the same reading the linter and the
            // risk-policy check already give a guard's literal. A guard whose direction is not the
            // one its quantity means (`slippage >= n`, `profit <= n`) is left as the static record
            // it was: those are refused where they matter, and inverting the comparison here would
            // enforce the opposite of what the program wrote.
            let enforced = enforceable_economic_guard(guard);
            ir.push(Operation::Require {
                kind: require_kind_to_ir(&guard.kind),
                subject: guard.subject.as_ref().map(|s| s.as_str().to_string()),
                condition: guard_condition(enforced.as_ref().unwrap_or(guard))?,
                error_msg: None,
                measured: enforced.is_some(),
                comparison: guard.comparison,
            });
        }
        Statement::RouteFallback { replacements, requires } => {
            // The approvals are the record. Each one was verified as a route in
            // its own right by `verify_route_fallbacks` before lowering ran, so
            // this arm only materialises the list the artifact has to carry —
            // it does not decide what is approved.
            ir.push(Operation::RouteFallback {
                approved: replacements
                    .iter()
                    .map(|replacement| replacement.venue.as_str().to_string())
                    .collect(),
            });
            // The block's own bounds are guards, and they were lowered to **nothing**: this arm
            // destructured the list away with `..`, so `fallback { require profit >= 0 }` reached
            // neither the artifact nor the runtime. The slippage half is still enforced where it
            // is decided — every approved venue's declared slippage is checked against the bound
            // before the list is admitted (`verify_route_fallbacks`) — but a bound on what the
            // substitution *realises* is a post-condition on the trade, and the same guard is a
            // measured one outside the block, so dropping it here made the block the one place a
            // guard meant less.
            for guard in requires {
                let enforced = enforceable_economic_guard(guard);
                ir.push(Operation::Require {
                    kind: require_kind_to_ir(&guard.kind),
                    subject: guard.subject.as_ref().map(|s| s.as_str().to_string()),
                    condition: guard_condition(enforced.as_ref().unwrap_or(guard))?,
                    error_msg: None,
                    measured: enforced.is_some(),
                    comparison: guard.comparison,
                });
            }
        }
        Statement::Allow { feature } => {
            // The consent is the point, so it goes in the artifact. An opt-in
            // the bytecode does not carry is a permission only the source knows
            // about, and a runtime deciding whether it may net this intent
            // against another has nothing to check it against.
            let code = match feature.as_str() {
                "intent_fusion" => crate::spec::opcodes::FEATURE_INTENT_FUSION,
                _ => {
                    return Err(semantic(&format!(
                        "unknown feature '{}' in `allow`; the set is closed so a misspelling cannot be \
                         read as consent",
                        feature.as_str()
                    )))
                }
            };
            ir.push(Operation::FeatureAllow {
                feature: code,
                name: feature.as_str().to_string(),
            });
        }
        Statement::OnFail(action) => {
            ir.push(Operation::OnFail {
                action: failure_action_to_ir(action),
            });
        }
        Statement::OnTimeout { duration, action } => {
            // One converter for every timeout in the language: a bare number is
            // blocks, a number with a unit is time. This used to have its own
            // inline rule that clamped an oversized literal to `u32::MAX`, so a
            // deadline of 4_294_967_297 blocks became 4_294_967_295 — a silently
            // different deadline rather than a refusal.
            let dur_blocks: u32 = expression_to_blocks(&duration)?;
            // The ceiling applies wherever a timeout is written, not only in the
            // `atomic swap` clauses and the mainnet pass: a window longer than a
            // day is not a window, and a program should be told so in dev mode
            // rather than at release.
            if dur_blocks == 0 {
                return Err(semantic("timeout must be greater than 0 blocks"));
            }
            if dur_blocks > MAX_TIMEOUT_BLOCKS {
                return Err(semantic(&format!(
                    "timeout of {dur_blocks} blocks exceeds maximum {MAX_TIMEOUT_BLOCKS} \
                     (24 hours at {SECONDS_PER_BLOCK}s/block)"
                )));
            }
            ir.push(Operation::OnTimeout {
                duration_blocks: dur_blocks,
                action: failure_action_to_ir(action),
            });
            if let ast::FailureAction::Refund(expr) = action {
                if let crate::ir::FailureAction::Refund { chain, asset, to } = refund_expression_to_ir(expr) {
                    ir.push(Operation::Release {
                        chain: chain.to_ascii_lowercase(),
                        asset,
                        to,
                        // A refund *returns* the escrow to the payer: the inverse of a lock,
                        // and its own act — it is what the `OnTimeout` above describes, emitted
                        // as the instruction that performs it (TICKET-001).
                        act: ReleaseAct::Refund,
                    });
                }
            }
        }
        Statement::Snapshot => {
            ir.push(Operation::ChainMetric {
                metric: ChainMetricKind::Snapshot,
            });
        }
        Statement::Diff { before, after } => {
            ir.push(Operation::Call {
                function: "diff".to_string(),
                args: vec![expression_to_string(before), expression_to_string(after)],
            });
        }
        Statement::CrdtOp(op) => {
            ir.push(Operation::CrdtOp {
                kind: crdt_kind_to_ir(&op.kind),
                key: expression_to_string(&op.key),
                value: op.value.as_ref().map(expression_to_string),
            });
        }
        Statement::ZkVerify {
            proof,
            public_input,
            key,
        } => {
            ir.push(Operation::ProofVerify {
                kind: ProofKind::Zk,
                proof: expression_to_string(proof),
                input: expression_to_string(public_input),
                key_or_threshold: expression_to_string(key),
            });
        }
        Statement::MpcVerify {
            result,
            signatures,
            threshold,
        } => {
            ir.push(Operation::ProofVerify {
                kind: ProofKind::Mpc,
                proof: expression_to_string(result),
                input: expression_to_string(signatures),
                key_or_threshold: expression_to_string(threshold),
            });
        }
        Statement::StorageRef { op, data } => {
            ir.push(Operation::StorageOp {
                kind: storage_kind_to_ir(op),
                data: expression_to_string(data),
            });
        }
        Statement::Pathfind { from, to, max_depth } => {
            ir.push(Operation::Pathfind {
                from: expression_to_string(from),
                to: expression_to_string(to),
                max_depth: expression_to_blocks(max_depth)?,
            });
        }
        Statement::MempoolScan { max_results } => {
            ir.push(Operation::MempoolScan {
                max_results: expression_to_blocks(max_results)?,
            });
        }
        Statement::OracleRequest { token, reward } => {
            ir.push(Operation::OracleRequest {
                token: expression_to_string(token),
                reward: expression_to_u128(reward)?,
            });
        }
        Statement::Pause => {
            ir.push(Operation::EmergencyControl {
                kind: EmergencyKind::Pause,
            });
        }
        Statement::Resume => {
            ir.push(Operation::EmergencyControl {
                kind: EmergencyKind::Resume,
            });
        }
        Statement::SelfDestruct => {
            ir.push(Operation::Lifecycle {
                kind: LifecycleKind::Destroy,
                target: None,
            });
        }
        Statement::Migrate { new_contract } => {
            ir.push(Operation::Lifecycle {
                kind: LifecycleKind::Migrate,
                target: Some(expression_to_string(new_contract)),
            });
        }
        // ===== Statements this compiler cannot lower, refused rather than dropped =====
        //
        // There is deliberately no catch-all arm. One used to sit here — `_ => ir.push(Nop)` under
        // the comment "Other statement types (return, break, etc.)" — and it is why
        // `tests/sketches/arithmetic.x3` — which sat in `tests/` until TICKET-109, where it
        // *passed* the corpus gate — whose whole subject is arithmetic,
        //```
        //    let a = 1; let b = 2; let c = a + b;
        //```
        // lowered to **one `Nop` per statement** and checked clean with no warnings. `NOP` is
        // written as four zero bytes, which the compiler's own instruction walker skips as padding
        // and the VM's verifier breaks on as the end of the stream: the record is invisible to every
        // reader, so the artifact of a program whose every statement was dropped is
        // indistinguishable from the artifact of an empty program. "A `.x3` file in a directory the
        // tooling walks is a claim that it is a program" — the corpus gate's own words — and this
        // is where that claim was being satisfied by an artifact of the drop (TICKET-109).
        //
        // Each arm names the construct and what is missing, because the alternative a program's
        // author has to know about is what the compiler *can* do: the const/declaration surface and
        // the operation statements above.
        Statement::Let { name, .. } => {
            return Err(semantic(&format!(
                "`let {name} = …` binds a name this compiler has no place to keep: it emits no \
                 arithmetic and no register holds a source-level binding, so the value would be \
                 dropped and every *use* of `{name}` already refuses. Write the value where it is \
                 used. (The trading dialect's `let <name> = <swap …>` is a different construct and \
                 does lower — it binds an operation's result.)"
            )));
        }
        Statement::Return(_) => {
            return Err(semantic(
                "`return` would be dropped: this compiler does not emit a statement's return, and an \
                 artifact has no frame for one, so a program that returns a value would reach the VM \
                 saying nothing about it",
            ));
        }
        Statement::Break => {
            return Err(semantic(
                "`break` would be dropped: the artifact has no loop the VM can execute — a `while` \
                 is refused unless the compiler can decide it — so there is no loop to break out of",
            ));
        }
        Statement::Continue => {
            return Err(semantic(
                "`continue` would be dropped: the artifact has no loop the VM can execute — a \
                 `while` is refused unless the compiler can decide it — so there is no loop to \
                 continue",
            ));
        }
        Statement::For { iterable, .. } => {
            return Err(semantic(&format!(
                "`for … in {}` would be dropped: this compiler emits no iteration codegen and this VM \
                 branches on a register, so the body could not run even once",
                expression_to_string(iterable)
            )));
        }
        // A bare `loop` is the unbounded loop `while true` spells, so it takes the same path: it
        // lowers with a decided-true condition and the refusal that names it comes from the verifier
        // and the emitter, where `while true`'s comes from. A second refusal here would be a second
        // answer to a question TICKET-098 settled in one place.
        Statement::Loop(body) => {
            let body_ops = {
                let mut temp_ir = X3IR::new();
                lower_function_body(body, &mut temp_ir)?;
                temp_ir.operations
            };
            ir.push(Operation::Loop {
                max_iterations: 1000,
                condition: Condition::True,
                body: body_ops,
            });
        }
    }
    Ok(())
}

/// Lower a function body (block of statements)
fn lower_function_body(block: &Block, ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    for stmt in &block.stmts {
        lower_statement(stmt, ir)?;
    }
    Ok(())
}

/// Lower an expression to IR (may produce operations or just return values)
fn lower_expression(expr: &Expression, ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    match expr {
        Expression::Literal(_) => {
            // Literals don't produce operations, just values
            Ok(())
        }
        Expression::Ident(_) => {
            // Variables don't produce operations
            Ok(())
        }
        Expression::Call { callee, args } => {
            lower_builtin_call(callee, args, ir)?;
            Ok(())
        }
        Expression::Binary { lhs: _, op: _, rhs: _ } => {
            // Binary operations don't produce direct IR ops (used in conditions)
            Ok(())
        }
        _ => Ok(()),
    }
}

fn lower_annotations_prefix(annotations: &[Annotation], ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    for annotation in annotations {
        match annotation {
            Annotation::Role(role) => ir.push(Operation::RoleCheck {
                role: role.as_str().to_string(),
            }),
            Annotation::Multisig(required, total) => {
                if required > total {
                    return Err(semantic("@multisig requires required <= total"));
                }
                ir.push(Operation::MultisigCheck {
                    required: *required,
                    total: *total,
                });
            }
            Annotation::Subscribe(event) => ir.push(Operation::Call {
                function: "subscribe_event".to_string(),
                args: vec![event.as_str().to_string()],
            }),
            Annotation::Sponsor => ir.push(Operation::Call {
                function: "deduct_sponsor_fee".to_string(),
                args: vec![],
            }),
            Annotation::Sandbox => ir.push(Operation::Require {
                kind: ir::RequireKind::Custom("sandbox_gas_limit".to_string()),
                subject: None,
                comparison: None,
                condition: Condition::True,
                error_msg: Some("sandbox gas limit exceeded".to_string()),
                measured: false,
            }),
            Annotation::Whitelist(entries) => ir.push(Operation::Require {
                kind: ir::RequireKind::Custom("whitelist".to_string()),
                subject: None,
                comparison: None,
                condition: Condition::Expression {
                    expr: entries.iter().map(|sym| sym.as_str()).collect::<Vec<_>>().join(","),
                },
                error_msg: Some("call target not whitelisted".to_string()),
                measured: false,
            }),
            Annotation::Hot => ir.push(Operation::Emit {
                name: "hot_enter".to_string(),
                data: Default::default(),
            }),
            Annotation::Audit => ir.push(Operation::Emit {
                name: "audit_enter".to_string(),
                data: Default::default(),
            }),
            Annotation::Extern => ir.push(Operation::AbiExport {
                function: "extern".to_string(),
                params: vec![],
                ret: "()".to_string(),
            }),
            // `@gas_adaptive` states two gas paths, and the *annotation* has no way to name them —
            // it takes no arguments. The artifact's record demands two non-empty bodies
            // (`verify_ir`: "gas-adaptive branches must not be empty"), so this arm used to satisfy
            // that rule with `vec![Operation::Nop]` on each side: a record claiming the program has
            // two paths, neither of which is one, and both of whose bodies were four zero bytes no
            // reader can see (TICKET-110).
            //
            // So it lowers to nothing, like every other modifier the artifact has no form for
            // (`NoHeap`, `OnChain`, `Payable`, `Simd`, … below). The opcode stays — it is a real VM
            // capability (`GAS_ADAPTIVE`, and `bridge.gas_adaptive_select()` is what answers it) and a
            // hand-built IR can still carry it — but a *source* surface for it needs syntax the
            // annotation does not have, which is TICKET-111's question rather than this arm's.
            Annotation::NoHeap
            | Annotation::GasAdaptive
            | Annotation::NoRecursion(_)
            | Annotation::OnChain
            | Annotation::OffChain
            | Annotation::Concurrent
            | Annotation::Scheduled(_)
            | Annotation::Version(_)
            | Annotation::UpgradeFrom(_)
            | Annotation::Payable
            | Annotation::Simd => {}
        }
    }
    Ok(())
}

fn lower_annotations_suffix(annotations: &[Annotation], ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    let mut version = None;
    let mut upgrade_from = None;
    for annotation in annotations {
        match annotation {
            Annotation::Version(value) => version = Some(value.as_str().to_string()),
            Annotation::UpgradeFrom(value) => upgrade_from = Some(value.as_str().to_string()),
            Annotation::Hot => ir.push(Operation::Emit {
                name: "hot_exit".to_string(),
                data: Default::default(),
            }),
            Annotation::Audit => ir.push(Operation::Emit {
                name: "audit_exit".to_string(),
                data: Default::default(),
            }),
            Annotation::Scheduled(period) => {
                let entry = std::mem::take(&mut ir.operations);
                ir.push(Operation::ScheduledDispatch {
                    period_blocks: *period as u32,
                    entry,
                });
            }
            Annotation::Simd => {
                let args = std::mem::take(&mut ir.operations)
                    .into_iter()
                    .map(|op| format!("{:?}", op))
                    .collect();
                ir.push(Operation::GpuDispatch {
                    kernel: "simd_function".to_string(),
                    args,
                    is_simd: true,
                });
            }
            _ => {}
        }
    }
    if let Some(version) = version {
        ir.push(Operation::VersionMeta { version, upgrade_from });
    }
    Ok(())
}

fn lower_builtin_call(callee: &Expression, args: &[Expression], ir: &mut X3IR) -> Result<(), x3_lang_common::X3Error> {
    let name = expression_to_string(callee);
    match name.as_str() {
        "encode_rlp" => emit_serialize(ir, SerialFormat::Rlp, args),
        "decode_rlp" => emit_deserialize(ir, SerialFormat::Rlp, args),
        "encode_cbor" => emit_serialize(ir, SerialFormat::Cbor, args),
        "decode_cbor" => emit_deserialize(ir, SerialFormat::Cbor, args),
        "encode_json" => emit_serialize(ir, SerialFormat::Json, args),
        "decode_json" => emit_deserialize(ir, SerialFormat::Json, args),
        "encode_ssz" => emit_serialize(ir, SerialFormat::Ssz, args),
        "decode_ssz" => emit_deserialize(ir, SerialFormat::Ssz, args),
        "estimate_evm_gas" => emit_gas_estimate(ir, "evm", args),
        "estimate_svm_gas" => emit_gas_estimate(ir, "svm", args),
        "estimate_x3_gas" => emit_gas_estimate(ir, "x3", args),
        "get_chain_congestion" => emit_metric(ir, ChainMetricKind::Congestion),
        "get_base_fee" => emit_metric(ir, ChainMetricKind::BaseFee),
        "get_finality_lag" => emit_metric(ir, ChainMetricKind::FinalityLag),
        "get_block_time" => emit_metric(ir, ChainMetricKind::BlockTime),
        "generate_event_proof" => ir.push(Operation::EventProvenance {
            event_type: arg_string(args, 0),
            data: arg_string(args, 1),
        }),
        "multi_hop_swap" => ir.push(Operation::MultiHopSwap {
            path: vec![arg_string(args, 0)],
            amount: arg_u128(args, 1)?,
        }),
        "resolve_intent" => ir.push(Operation::IntentResolve {
            constraints: vec![arg_string(args, 0)],
            resolver: "default".to_string(),
        }),
        "run_ai_model" => ir.push(Operation::GpuDispatch {
            kernel: arg_string(args, 0),
            args: args.iter().skip(1).map(expression_to_string).collect(),
            is_simd: false,
        }),
        "get_crdt" => ir.push(Operation::CrdtOp {
            kind: IrCrdtKind::Get,
            key: arg_string(args, 0),
            value: None,
        }),
        "set_crdt" => ir.push(Operation::CrdtOp {
            kind: IrCrdtKind::Set,
            key: arg_string(args, 0),
            value: Some(arg_string(args, 1)),
        }),
        "storage_store" => ir.push(Operation::StorageOp {
            kind: StorageKind::Store,
            data: arg_string(args, 0),
        }),
        "storage_load" => ir.push(Operation::StorageOp {
            kind: StorageKind::Load,
            data: arg_string(args, 0),
        }),
        "pathfind" => ir.push(Operation::Pathfind {
            from: arg_string(args, 0),
            to: arg_string(args, 1),
            max_depth: arg_u128(args, 2)? as u32,
        }),
        "mempool_scan" => ir.push(Operation::MempoolScan {
            max_results: arg_u128(args, 0)? as u32,
        }),
        "oracle_request" => ir.push(Operation::OracleRequest {
            token: arg_string(args, 0),
            reward: arg_u128(args, 1)?,
        }),
        "pause" => ir.push(Operation::EmergencyControl {
            kind: EmergencyKind::Pause,
        }),
        "resume" => ir.push(Operation::EmergencyControl {
            kind: EmergencyKind::Resume,
        }),
        "self_destruct" => ir.push(Operation::Lifecycle {
            kind: LifecycleKind::Destroy,
            target: None,
        }),
        "verify_zk" => ir.push(Operation::ProofVerify {
            kind: ProofKind::Zk,
            proof: arg_string(args, 0),
            input: arg_string(args, 1),
            key_or_threshold: arg_string(args, 2),
        }),
        "verify_mpc" => ir.push(Operation::ProofVerify {
            kind: ProofKind::Mpc,
            proof: arg_string(args, 0),
            input: arg_string(args, 1),
            key_or_threshold: arg_string(args, 2),
        }),
        "calculate_portfolio_value" => ir.push(Operation::VectorMath {
            op: VectorOp::DotProduct,
            a: arg_string(args, 0),
            b: arg_string(args, 1),
            size: args.len() as u32,
        }),
        _ => ir.push(Operation::Call {
            function: name,
            args: args.iter().map(expression_to_string).collect(),
        }),
    }
    Ok(())
}

/// Convert an AST expression to an IR Condition
/// The value of an integer expression, when the program states one.
///
/// Arithmetic is evaluated with `checked_*`, so an overflow or a division by zero returns `None`
/// and the branch that depended on it stays undecided. A branch decided from a wrapped number is
/// a branch decided wrongly, which is worse than one that refuses.
///
/// Narrow on purpose: only the operations whose result a `u128` can hold are folded. The shifts
/// and the bitwise operators are `None` — a left shift that discards high bits wraps, and
/// checking that costs more than the case is worth when the fallback is a refusal that names why.
fn fold_int(expr: &Expression) -> Option<u128> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Some(*value),
        Expression::Binary { op, lhs, rhs } => {
            let left = fold_int(lhs)?;
            let right = fold_int(rhs)?;
            match op {
                BinOp::Plus => left.checked_add(right),
                BinOp::Minus => left.checked_sub(right),
                BinOp::Star => left.checked_mul(right),
                BinOp::Slash => left.checked_div(right),
                BinOp::Percent => left.checked_rem(right),
                BinOp::Power => u32::try_from(right)
                    .ok()
                    .and_then(|exponent| left.checked_pow(exponent)),
                _ => None,
            }
        }
        // `u128` has no negative value, so a negation has none either — `-5 < 0` is undecided
        // rather than false, and the branch that needs it is refused.
        _ => None,
    }
}

/// Decide a branch condition at compile time, when the program made it decidable.
///
/// `Some(..)` means the compiler knows which branch runs and the emitter writes that one.
/// `None` means it does not — and that is exactly the case this VM cannot execute, because it
/// branches on a register and the compiler emits no arithmetic to put a value there, so the
/// branch is refused downstream with that reason rather than guessed at (TICKET-058).
///
/// `&&` and `||` keep the language's short-circuit meaning: a decidable `false` on either side of
/// `&&` decides the whole condition even when the other side is undecided, and likewise a
/// decidable `true` for `||`.
fn fold_condition(expr: &Expression) -> Option<bool> {
    match expr {
        Expression::Literal(LiteralExpr::Bool(value)) => Some(*value),
        Expression::Unary { op: UnOp::Not, expr } => fold_condition(expr).map(|value| !value),
        Expression::Binary { op, lhs, rhs } => match op {
            BinOp::AndAnd => match (fold_condition(lhs), fold_condition(rhs)) {
                (Some(false), _) | (_, Some(false)) => Some(false),
                (Some(true), Some(true)) => Some(true),
                _ => None,
            },
            BinOp::OrOr => match (fold_condition(lhs), fold_condition(rhs)) {
                (Some(true), _) | (_, Some(true)) => Some(true),
                (Some(false), Some(false)) => Some(false),
                _ => None,
            },
            BinOp::EqEq => Some(fold_int(lhs)? == fold_int(rhs)?),
            BinOp::Ne => Some(fold_int(lhs)? != fold_int(rhs)?),
            BinOp::Lt => Some(fold_int(lhs)? < fold_int(rhs)?),
            BinOp::Le => Some(fold_int(lhs)? <= fold_int(rhs)?),
            BinOp::Gt => Some(fold_int(lhs)? > fold_int(rhs)?),
            BinOp::Ge => Some(fold_int(lhs)? >= fold_int(rhs)?),
            _ => None,
        },
        _ => None,
    }
}

fn expression_to_condition(expr: &Expression) -> Result<Condition, x3_lang_common::X3Error> {
    match expr {
        Expression::Literal(LiteralExpr::Bool(true)) => Ok(Condition::True),
        Expression::Literal(LiteralExpr::Bool(false)) => Ok(Condition::False),
        Expression::Call { callee, args } => condition_from_call(callee, args),
        // `if profit >= 20` — a comparison of a quantity a host measures, which is the one class of
        // condition this VM can decide without arithmetic: it holds the quantity and has a mode for
        // the comparison. Anything else stays `Condition::Expression` and is refused downstream,
        // which is TICKET-106's boundary and not a gap in this arm.
        Expression::Binary { op, lhs, rhs } => match measured_condition(op, lhs, rhs)? {
            Some(condition) => Ok(condition),
            None => Ok(Condition::Expression {
                expr: expression_to_string(expr),
            }),
        },
        _ => Ok(Condition::Expression {
            expr: expression_to_string(expr),
        }),
    }
}

/// `Ident(<measured quantity>) <comparison> <bound>` as a [`Condition::Measured`], or `None` when
/// the expression is not that shape.
///
/// `Err` — rather than `None` — when the left-hand side *is* a measured quantity and the comparison
/// is one the VM has no direction for: `if profit == 20` names a quantity the runtime holds and
/// asks a question nothing executes, and answering it with the generic "the condition is not
/// decidable at compile time" would send its author looking for a way to make it decidable. The
/// bound is read by the same exact converter the guards use, so `if profit >= 0.05%` means the same
/// figure as `require profit >= 0.05%` or neither does.
fn measured_condition(
    op: &x3_lang_common::BinOp,
    lhs: &Expression,
    rhs: &Expression,
) -> Result<Option<Condition>, x3_lang_common::X3Error> {
    let Some(quantity) = measured_quantity_of(lhs) else {
        return Ok(None);
    };
    let Some(comparison) = comparison_of(op) else {
        return Ok(None);
    };
    if !quantity.supports(comparison) {
        return Err(semantic(&format!(
            "`{} {} …` is not a comparison this runtime makes about the {} a host measures: a branch \
             on it is `{} {} <bound>` or `{} {} <bound>`",
            quantity.spelling(),
            comparison.as_str(),
            quantity.spelling(),
            quantity.spelling(),
            quantity.guard_comparison().as_str(),
            quantity.spelling(),
            quantity.complement_comparison().as_str(),
        )));
    }
    let Some(bps) = crate::semantic::bound_bps_from_expr(rhs) else {
        return Err(semantic(&format!(
            "`{} {} {}` — the bound has to be a whole number of basis points, a percentage \
             (`0.05%`), or a bare fraction of one; a sub-basis-point bound is not a figure this \
             runtime can compare and is refused rather than rounded",
            quantity.spelling(),
            comparison.as_str(),
            expression_to_string(rhs)
        )));
    };
    let threshold_bps = u16::try_from(bps).map_err(|_| {
        semantic(&format!(
            "the bound {bps}bps is larger than a measured comparison's operand, which is a `u16`"
        ))
    })?;
    Ok(Some(Condition::Measured {
        quantity,
        comparison,
        threshold_bps,
    }))
}

/// The measured quantity an expression names, if it names one at all.
fn measured_quantity_of(expr: &Expression) -> Option<ir::MeasuredQuantity> {
    match expr {
        Expression::Ident(name) => ir::MeasuredQuantity::from_name(name.as_str()),
        _ => None,
    }
}

/// The IR comparison an operator is, if it is one.
///
/// `&&` and `||` are not: a branch on two measurements at once is two branches, and combining them
/// would need a value in a register, which is the thing this compiler cannot make.
fn comparison_of(op: &x3_lang_common::BinOp) -> Option<ir::ComparisonOp> {
    match op {
        x3_lang_common::BinOp::Lt => Some(ir::ComparisonOp::Less),
        x3_lang_common::BinOp::Le => Some(ir::ComparisonOp::LessOrEqual),
        x3_lang_common::BinOp::Gt => Some(ir::ComparisonOp::Greater),
        x3_lang_common::BinOp::Ge => Some(ir::ComparisonOp::GreaterOrEqual),
        x3_lang_common::BinOp::EqEq => Some(ir::ComparisonOp::Equal),
        x3_lang_common::BinOp::Ne => Some(ir::ComparisonOp::NotEqual),
        _ => None,
    }
}

/// Convert AST RequireKind to IR RequireKind
/// A guard the compiler generated, as the AST guard the rest of the pipeline reads.
///
/// The scope's floors travel the same path a written `require` does, so the condition
/// they become is produced by one converter rather than by a second that could disagree
/// about what `profit >= 20` means.
fn bps_guard(guard: &arb::Guard) -> ast::RequireGuard {
    ast::RequireGuard {
        kind: guard.kind.clone(),
        subject: None,
        comparison: Some(guard.comparison),
        value: Some(Expression::Literal(LiteralExpr::Int {
            value: u128::from(guard.bps),
            base: x3_lang_common::IntBase::Decimal,
            suffix: None,
        })),
    }
}

/// The guard with its bound stated in basis points, when the VM is the thing that enforces it.
///
/// `None` means "the static record it already was". Two quantities are enforced by the VM — a
/// slippage ceiling and a profit floor — because they are the two the host measures and reports
/// (PHASE 7's native risk policy, and the spec's "economic constraints enforced by the VM"). Every
/// other guard kind is a claim about the artifact's *configuration*, which the compile-time pass
/// for that kind decides, and its bound stays where the pass read it.
///
/// The unit is basis points because the bound becomes the instruction's operand and the executor
/// compares it against a measurement in basis points. The reading of a bare number is the one the
/// linter and the risk-policy check already use, so `require slippage <= 50` and
/// `risk_policy { max_slippage 50 }` mean the same figure rather than two.
fn enforceable_economic_guard(guard: &ast::RequireGuard) -> Option<ast::RequireGuard> {
    let comparison = guard.comparison?;
    let is_ceiling = comparison.is_upper_bound();
    let wanted_ceiling = match guard.kind {
        ast::RequireKind::Slippage => true,
        ast::RequireKind::Profit => false,
        _ => return None,
    };
    if is_ceiling != wanted_ceiling {
        return None;
    }
    let bound = crate::semantic::slippage_bps_from_text(&expression_to_string(guard.value.as_ref()?))?;
    let bound = u16::try_from(bound).ok()?;
    Some(ast::RequireGuard {
        value: Some(Expression::Literal(LiteralExpr::Int {
            value: u128::from(bound),
            base: x3_lang_common::IntBase::Decimal,
            suffix: None,
        })),
        ..guard.clone()
    })
}

fn require_kind_to_ir(kind: &ast::RequireKind) -> ir::RequireKind {
    match kind {
        ast::RequireKind::CanonicalSupply => ir::RequireKind::CanonicalSupply,
        ast::RequireKind::Nonce => ir::RequireKind::NonceUnused,
        ast::RequireKind::BridgeLiquidity => ir::RequireKind::BridgeLiquidity,
        ast::RequireKind::Slippage => ir::RequireKind::SlippageTolerance,
        ast::RequireKind::Fees => ir::RequireKind::Fees,
        ast::RequireKind::Profit => ir::RequireKind::ProfitThreshold,
        ast::RequireKind::Finality => ir::RequireKind::Finality,
        ast::RequireKind::Custom(name) => ir::RequireKind::Custom(name.as_str().to_string()),
        ast::RequireKind::InvariantCheck => ir::RequireKind::Custom("invariant".to_string()),
        ast::RequireKind::RiskScore => ir::RequireKind::RiskScore,
        ast::RequireKind::AuditGate => ir::RequireKind::Custom("audit_gate".to_string()),
        ast::RequireKind::RelayerQuorum => ir::RequireKind::RelayerQuorum,
        ast::RequireKind::RouteScore => ir::RequireKind::RouteScore,
        ast::RequireKind::SolverBond => ir::RequireKind::SolverBond,
        ast::RequireKind::ProofComplete => ir::RequireKind::ProofComplete,
        ast::RequireKind::RefundPath => ir::RequireKind::RefundPath,
        ast::RequireKind::FinalityExplicit => ir::RequireKind::FinalityExplicit,
        ast::RequireKind::VmSupported => ir::RequireKind::VmSupported,
        ast::RequireKind::MainnetSafe => ir::RequireKind::MainnetSafe,
    }
}

/// Convert AST FailureAction to IR FailureAction
fn failure_action_to_ir(action: &ast::FailureAction) -> ir::FailureAction {
    match action {
        ast::FailureAction::Rollback => ir::FailureAction::Rollback,
        ast::FailureAction::Refund(expr) => refund_expression_to_ir(expr),
        ast::FailureAction::Halt => ir::FailureAction::Halt,
        ast::FailureAction::Quarantine => ir::FailureAction::Quarantine,
    }
}

/// Pick the path an `atomic_choice` will emit.
///
/// Ties go to the earliest declared path, which makes the selection a function
/// of the program text alone. Returns `None` when no path can be ranked — the
/// AST-level verifier rejects those programs, and lowering refuses rather than
/// defaulting to path 0, because a default would turn "the compiler could not
/// decide" into "the compiler decided the first one".
fn select_choice_path(choice: &x3_lang_ast::ast::AtomicChoiceDecl) -> Option<usize> {
    use x3_lang_ast::ast::ChoiceCriterion;

    let ranked: Vec<(usize, u128)> = match choice.criterion {
        ChoiceCriterion::HighestNetOutput => choice
            .paths
            .iter()
            .enumerate()
            .filter_map(|(index, path)| {
                let output = path.net_output.as_ref()?;
                let amount = crate::semantic::extract_int_from_expr(&output.value)?;
                Some((index, amount))
            })
            .collect(),
        ChoiceCriterion::FewestHops => choice
            .paths
            .iter()
            .enumerate()
            .filter_map(|(index, path)| crate::semantic::path_hop_count(path).map(|hops| (index, u128::from(hops))))
            .collect(),
        // Refused by `verify_atomic_choice_decls` before lowering is reached, because
        // the criterion needs the venue chain a plan resolves to and a path body does
        // not carry one. Ranking nothing here leaves `ranked.len() != paths.len()`,
        // so a caller that somehow got past the verifier is refused rather than
        // given path 0.
        ChoiceCriterion::LowestDeclaredFee => Vec::new(),
    };

    // A criterion that could only rank some of the paths has not ranked the
    // branch set: the winner would be "best of the paths we could measure".
    if ranked.len() != choice.paths.len() {
        return None;
    }

    let better = |candidate: u128, incumbent: u128| match choice.criterion {
        ChoiceCriterion::HighestNetOutput => candidate > incumbent,
        ChoiceCriterion::FewestHops => candidate < incumbent,
        // Unreachable: the arm above ranks no path, so `ranked` is empty and this is
        // never called. Stated so the direction is not silently wrong if it ever is.
        ChoiceCriterion::LowestDeclaredFee => false,
    };

    let mut best: Option<(usize, u128)> = None;
    for (index, score) in ranked {
        match best {
            Some((_, incumbent)) if !better(score, incumbent) => {}
            _ => best = Some((index, score)),
        }
    }
    best.map(|(index, _)| index)
}

/// One sentence for why a `parallel` block has no plan. The messages live here
/// rather than inline so the two call sites — the per-leg check and the
/// whole-plan check — cannot describe the same failure differently.
fn describe_race(race: &crate::dag::RaceError) -> String {
    match race {
        crate::dag::RaceError::WriteWrite { asset, first, second } => format!(
            "legs '{first}' and '{second}' both produce {asset}, so the program does not say which \
             write wins"
        ),
        crate::dag::RaceError::Cycle { legs } => format!(
            "the dependencies form a cycle ({}), so there is no execution order",
            legs.join(" -> ")
        ),
        crate::dag::RaceError::TooFewLegs { legs } => {
            format!("{legs} leg(s) declared; a parallel block needs at least two")
        }
        crate::dag::RaceError::TooManyLegs { legs, bound } => {
            format!("{legs} legs declared, above the {bound}-leg production bound")
        }
        crate::dag::RaceError::DuplicateLeg { name } => format!("leg '{name}' is declared twice"),
        crate::dag::RaceError::ImplicitCrossChain { leg, chains, bridges } => format!(
            "leg '{leg}' touches {} chains ({}) but contains {bridges} cross-chain step(s); moving \
             value between chains takes a step that says so",
            chains.len(),
            chains.join(", ")
        ),
    }
}

/// The statement bodies a top-level item may hold, for passes that need to see
/// the route as a whole rather than one statement at a time.
fn item_bodies(item: &Item) -> Vec<&Vec<Statement>> {
    match item {
        Item::IntentDecl(intent) => vec![&intent.body.stmts],
        Item::AtomicSwap(swap) => vec![&swap.body],
        Item::Strategy(strategy) => vec![&strategy.body],
        Item::Bridge(bridge) => vec![&bridge.body],
        Item::Proposal(proposal) => vec![&proposal.body],
        _ => Vec::new(),
    }
}

/// Rebuild the program's statements with every swap leg re-routed through
/// `venue` and the fallback block removed.
///
/// Rebuilt from the *whole* item body, not just the route block, because the
/// guards that make a route valid sit at the intent level: verifying a
/// substitute against the route alone would report every approved venue as
/// missing a slippage bound. This is the program the runtime would run if it
/// took that substitution, which is the only thing worth verifying — approving
/// a venue without checking the route it produces approves on the strength of
/// the venue's name.
fn substituted_route(statements: &[Statement], venue: &str) -> Vec<Statement> {
    fn map(statement: &Statement, venue: &str) -> Option<Statement> {
        match statement {
            Statement::RouteFallback { .. } => None,
            Statement::Swap {
                from,
                to,
                amount,
                min_output,
                ..
            } => Some(Statement::Swap {
                from: from.clone(),
                to: to.clone(),
                amount: amount.clone(),
                min_output: min_output.clone(),
                dex: Some(Expression::Literal(LiteralExpr::String(x3_lang_common::Symbol::new(
                    venue,
                )))),
            }),
            Statement::Atomic(block) => Some(Statement::Atomic(AtomicBlock {
                meta: block.meta.clone(),
                body: Block::new(block.body.stmts.iter().filter_map(|inner| map(inner, venue)).collect()),
            })),
            Statement::If {
                cond,
                then_block,
                else_block,
            } => Some(Statement::If {
                cond: cond.clone(),
                then_block: Block::new(then_block.stmts.iter().filter_map(|inner| map(inner, venue)).collect()),
                else_block: else_block
                    .as_ref()
                    .map(|block| Block::new(block.stmts.iter().filter_map(|inner| map(inner, venue)).collect())),
            }),
            other => Some(other.clone()),
        }
    }

    statements
        .iter()
        .filter_map(|statement| map(statement, venue))
        .collect()
}

/// Verify every approved substitution in `statements` as a route.
///
/// The route's own steps stay in place — the guards and the refund path that
/// make it valid are exactly what the substitute has to satisfy too — so a
/// replacement that would produce an invalid route is refused here, before it
/// can be written into the artifact as approved.
fn verify_route_fallbacks_in(
    statements: &[Statement],
    metadata: &crate::ir::ProgramMetadata,
    mode: CompilationMode,
) -> Result<(), x3_lang_common::X3Error> {
    fn collect_replacements<'a>(statements: &'a [Statement], out: &mut Vec<&'a FallbackReplacement>) {
        for statement in statements {
            match statement {
                Statement::RouteFallback { replacements, .. } => out.extend(replacements.iter()),
                Statement::Atomic(block) => collect_replacements(&block.body.stmts, out),
                Statement::If {
                    then_block, else_block, ..
                } => {
                    collect_replacements(&then_block.stmts, out);
                    if let Some(block) = else_block {
                        collect_replacements(&block.stmts, out);
                    }
                }
                _ => {}
            }
        }
    }

    fn has_swap_leg(statements: &[Statement]) -> bool {
        statements.iter().any(|statement| match statement {
            Statement::Swap { .. } => true,
            Statement::Atomic(block) => has_swap_leg(&block.body.stmts),
            Statement::If {
                then_block, else_block, ..
            } => has_swap_leg(&then_block.stmts) || else_block.as_ref().is_some_and(|block| has_swap_leg(&block.stmts)),
            _ => false,
        })
    }

    let mut replacements: Vec<&FallbackReplacement> = Vec::new();
    collect_replacements(statements, &mut replacements);
    if replacements.is_empty() {
        return Ok(());
    }
    if !has_swap_leg(statements) {
        return Err(semantic(
            "fallback declares approved replacements but the route has no swap leg to replace",
        ));
    }

    for replacement in replacements {
        let venue = replacement.venue.as_str();
        let route = substituted_route(statements, venue);
        let mut verified = X3IR::new();
        verified.metadata = metadata.clone();
        for step in &route {
            lower_statement(step, &mut verified)?;
        }

        let mut errors = crate::ir_level_errors(&verified);
        errors.extend(
            crate::semantic::verify_collect(
                &verified,
                crate::semantic::DEFAULT_MAX_ATOMIC_OPS,
                crate::semantic::DEFAULT_MAX_ROUTE_HOPS,
                Some(mode),
            )
            .errors,
        );
        if let Some(first) = errors.into_iter().next() {
            return Err(semantic(&format!(
                "fallback approves venue '{venue}', but re-routing the leg through it does not \
                 produce a valid route: {first}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn expression_to_string(expr: &Expression) -> String {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => value.to_string(),
        Expression::Literal(LiteralExpr::Float { raw, .. }) => raw.as_str().to_string(),
        Expression::Literal(LiteralExpr::String(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Address(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Hash(s)) => s.as_str().to_string(),
        Expression::Literal(LiteralExpr::Percentage { value }) => value.as_str().to_string(),
        Expression::Literal(LiteralExpr::Duration { value, unit }) => {
            format!("{}{:?}", value, unit)
        }
        Expression::Literal(LiteralExpr::Bool(b)) => b.to_string(),
        Expression::Ident(s) => s.as_str().to_string(),
        // `receiver sol.wallet.owner` is a dotted path, and it is the form the
        // language's own examples use. It used to fall through to `{:?}` and
        // become `FieldAccess { target: FieldAccess { ... } }` — a Rust debug
        // string, which the name-safety check then rejected for "unsafe
        // characters" and for exceeding 64 characters. The path is what the
        // program wrote; print that.
        Expression::FieldAccess { target, field } => {
            format!("{}.{}", expression_to_string(target), field.as_str())
        }
        // `{:?}` on the operator renders its Rust *variant* name, so `while steps < 10` reached the
        // IR, `x3c lower` and every diagnostic that names a condition as `steps Lt 10` — a guard
        // the program never wrote, in a spelling no reader of this language can parse back. `BinOp`
        // already implements `Display` with the language's own symbols, which is the one fact that
        // makes this a rendering choice rather than a second grammar (TICKET-098).
        Expression::Binary { op, lhs, rhs } => {
            format!("{} {} {}", expression_to_string(lhs), op, expression_to_string(rhs))
        }
        // The same defect one arm down: a unary condition was rendered as `{:?}`, so `while !ready`
        // would have arrived as `Unary { op: Not, … }` — a Rust debug string in a diagnostic that
        // is supposed to name the guard the program wrote.
        Expression::Unary { op, expr } => format!("{}{}", op, expression_to_string(expr)),
        Expression::Call { callee, args } => format!(
            "{}({})",
            expression_to_string(callee),
            args.iter().map(expression_to_string).collect::<Vec<_>>().join(",")
        ),
        _ => format!("{:?}", expr),
    }
}

fn expression_to_u128(expr: &Expression) -> Result<u128, x3_lang_common::X3Error> {
    match expr {
        Expression::Literal(LiteralExpr::Int { value, .. }) => Ok(*value),
        // A float literal used as an amount used to be parsed as `f64` and cast
        // to `u128`, which is a silent wrong value: `amount 0.5` became 0, and a
        // large fractional value could land anywhere. Amounts are money, PHASE 43
        // asks for exact conversion or none, and the exact path exists — the
        // trading policy converts `0.02 ETH` to base units through the asset's
        // declared decimals. So the answer here is a refusal that says where the
        // exact conversion is, not a truncation.
        Expression::Literal(LiteralExpr::Float { raw, .. }) => Err(semantic(&format!(
            "expected an amount, but `{}` is a fractional literal; an amount is converted only when \
             the conversion is exact, so write it in base units or use a field that carries its \
             asset's decimals (as `max_gas: 0.02 ETH` does)",
            raw.as_str()
        ))),
        _ => expression_to_string(expr)
            .parse::<u128>()
            .map_err(|_| semantic("expected numeric expression")),
    }
}
fn chain_to_string(chain: &ChainRef) -> String {
    chain.as_str().to_ascii_lowercase()
}
/// The block count a timeout expression denotes.
///
/// Timeouts are written with a unit suffix — `180s`, `40m` — and the lexer
/// hands those back as *identifiers*, not numbers, so an integer parse rejects
/// them. The `intent` path has always read the digits off the front;
/// `atomic swap` and `bridge` rejected the identical syntax with "expected
/// numeric expression", which made `timeout source 40m` — the form the
/// language's own documentation and examples use — impossible to lower. Both
/// paths read timeouts through here now, so they cannot disagree again.
///
/// The suffix is **not** converted: `40m` is 40 blocks, not 40 minutes. That is
/// the convention already baked into every compiled program (`timeout 180s`
/// lowers to 180 blocks today), and changing it would silently move the
/// timeout-ordering invariant for every existing program, which is a security
/// decision rather than a bug fix. See TICKET-033.
pub(crate) fn timeout_expression_to_blocks(expr: &Expression) -> Option<u32> {
    match expr {
        // A bare number is a count of blocks.
        Expression::Literal(LiteralExpr::Int { value, .. }) => u32::try_from(*value).ok(),
        // A fractional duration is not a number of blocks. This used to parse as
        // `f64` and truncate, so `timeout 40.9m` became 40 — a wrong value rather
        // than a refusal, in a field the timeout-ordering invariant reads.
        Expression::Literal(LiteralExpr::Float { .. }) => None,
        Expression::Literal(LiteralExpr::Duration { value, unit }) => blocks_from_duration(*value, *unit),
        // `40m` is one identifier when it was not classified where it was read
        // (a hand-built AST, or a duration in a position the parser does not
        // treat as one). The unit is read rather than dropped: this used to take
        // the leading digits and ignore the rest, so `40m` meant forty *blocks*.
        Expression::Ident(sym) => {
            let text = sym.as_str();
            let digits = text.trim_end_matches(|ch: char| ch.is_ascii_alphabetic());
            if !digits.is_empty() && digits.len() != text.len() {
                let suffix = &text[digits.len()..];
                let value: u64 = digits.trim_end_matches('_').replace('_', "").parse().ok()?;
                if suffix == "blocks" {
                    return u32::try_from(value).ok();
                }
                return crate::parser::duration_unit_from_suffix(suffix)
                    .and_then(|unit| blocks_from_duration(value, unit));
            }
            numeric_prefix_u32(text)
        }
        _ => None,
    }
}

/// Seconds per block.
///
/// Named once, because two things depend on it: what a program's `40m` means,
/// and `MAX_TIMEOUT_BLOCKS`'s reading of "24 hours". A timeout that says minutes
/// and a ceiling that assumes a block time have to agree about the block time.
pub const SECONDS_PER_BLOCK: u64 = 6;

/// The blocks a duration of `value` of `unit` denotes.
///
/// Rounded **up**: an HTLC window shorter than the program asked for is the
/// dangerous direction — the source-side claim is the one that needs the time —
/// so a duration that is not a whole number of blocks gets the extra block. A
/// sub-second duration therefore becomes one block rather than none, which is
/// also what keeps `timeout 500ms` from lowering to a deadline of zero (and
/// being refused as a zero-duration timeout, which it is not).
fn blocks_from_duration(value: u64, unit: x3_lang_common::DurationUnit) -> Option<u32> {
    use x3_lang_common::DurationUnit;
    let seconds = match unit {
        DurationUnit::Nanoseconds => value.div_ceil(1_000_000_000),
        DurationUnit::Microseconds => value.div_ceil(1_000_000),
        DurationUnit::Milliseconds => value.div_ceil(1_000),
        DurationUnit::Seconds => value,
        DurationUnit::Minutes => value.saturating_mul(60),
        DurationUnit::Hours => value.saturating_mul(3_600),
        DurationUnit::Days => value.saturating_mul(86_400),
    };
    if seconds == 0 {
        return Some(0);
    }
    u32::try_from(seconds.div_ceil(SECONDS_PER_BLOCK)).ok()
}

fn numeric_prefix_u32(value: &str) -> Option<u32> {
    let digits: String = value.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse::<u32>().ok()
    }
}

pub(crate) fn expression_to_blocks(expr: &Expression) -> Result<u32, x3_lang_common::X3Error> {
    // `u32::try_from` rather than a cast: a bare `as u32` used to wrap a large
    // literal, so `timeout 4294967297` became a one-block timeout — a
    // too-short window is a safety problem in an HTLC, not a cosmetic one.
    timeout_expression_to_blocks(expr).ok_or_else(|| {
        semantic("expected a timeout duration — e.g. `40m`, `180s`, or a block count that fits in 32 bits")
    })
}
fn semantic(message: &str) -> x3_lang_common::X3Error {
    x3_lang_common::X3Error::SemanticError {
        message: message.to_string(),
        span: Span::DUMMY,
    }
}

/// Known chain prefixes (from parser VM family mapping).
const KNOWN_CHAIN_PREFIXES: &[&str] = &[
    "eth",
    "ethereum",
    "polygon",
    "arb",
    "optimism",
    "base",
    "bsc",
    "avax",
    "sol",
    "solana",
    "svm",
    "substrate",
    "dot",
    "ksm",
    "polkadot",
    "kusama",
    "btc",
    "bitcoin",
    "x3",
    "x3vm",
    "move",
    "sui",
    "aptos",
    "cosmwasm",
    "cosmos",
    "atom",
    "osmo",
    "cairo",
    "starknet",
    "ada",
    "cardano",
    "plutus",
    "ton",
    "fuel",
    "near",
    "xlm",
    "stellar",
    "soroban",
    "ink",
    "pvm",
    "zk",
    "zkvm",
    "risc0",
    "sp1",
];

/// Maximum allowed timeout, from the block time rather than beside it: 24 hours
/// of blocks. Derived, so a change to `SECONDS_PER_BLOCK` moves the ceiling with
/// it instead of leaving two numbers that disagree about what a day is.
pub const MAX_TIMEOUT_BLOCKS: u32 = (24 * 60 * 60 / SECONDS_PER_BLOCK) as u32;
fn crdt_kind_to_ir(kind: &CrdtOpKind) -> IrCrdtKind {
    match kind {
        CrdtOpKind::Get => IrCrdtKind::Get,
        CrdtOpKind::Set => IrCrdtKind::Set,
        CrdtOpKind::Append => IrCrdtKind::Append,
        CrdtOpKind::Merge => IrCrdtKind::Merge,
    }
}
fn storage_kind_to_ir(kind: &StorageRefOp) -> StorageKind {
    match kind {
        StorageRefOp::Store => StorageKind::Store,
        StorageRefOp::Load => StorageKind::Load,
    }
}
fn arg_string(args: &[Expression], idx: usize) -> String {
    args.get(idx).map(expression_to_string).unwrap_or_default()
}
fn arg_u128(args: &[Expression], idx: usize) -> Result<u128, x3_lang_common::X3Error> {
    args.get(idx).map(expression_to_u128).unwrap_or(Ok(0))
}
fn emit_serialize(ir: &mut X3IR, format: SerialFormat, args: &[Expression]) {
    ir.push(Operation::Serialize {
        format,
        data: arg_string(args, 0),
    });
}
fn emit_deserialize(ir: &mut X3IR, format: SerialFormat, args: &[Expression]) {
    ir.push(Operation::Deserialize {
        format,
        data: arg_string(args, 0),
    });
}
fn emit_gas_estimate(ir: &mut X3IR, chain: &str, args: &[Expression]) {
    ir.push(Operation::GasEstimate {
        chain: chain.to_string(),
        route: arg_string(args, 0),
    });
}
fn emit_metric(ir: &mut X3IR, metric: ChainMetricKind) {
    ir.push(Operation::ChainMetric { metric });
}
fn condition_from_call(callee: &Expression, args: &[Expression]) -> Result<Condition, x3_lang_common::X3Error> {
    let name = expression_to_string(callee);
    if name == "verify_proof" && args.len() >= 2 {
        return Ok(Condition::ProofValid {
            proof: expression_to_string(&args[0]),
            expected_hash: expression_to_string(&args[1]),
        });
    }
    if name == "nonce" && args.len() >= 2 {
        return Ok(Condition::NonceEq {
            account: expression_to_string(&args[0]),
            expected: expression_to_u128(&args[1])? as u64,
        });
    }
    Ok(Condition::Expression {
        expr: format!(
            "{}({})",
            name,
            args.iter().map(expression_to_string).collect::<Vec<_>>().join(",")
        ),
    })
}
fn refund_expression_to_ir(expr: &Expression) -> ir::FailureAction {
    let value = expression_to_string(expr);
    let mut parts = value.split(':');
    let asset = parts.next().unwrap_or("unknown.UNKNOWN");
    let to = parts.next().unwrap_or("sender").to_string();
    let mut asset_parts = asset.split('.');
    ir::FailureAction::Refund {
        chain: asset_parts.next().unwrap_or("unknown").to_string(),
        asset: asset_parts.next().unwrap_or(asset).to_string(),
        to,
    }
}
