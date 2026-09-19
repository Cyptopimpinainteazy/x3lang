//! AST -> X3IR lowering pipeline.
//!
//! This module lowers X3 AST into X3 Intermediate Representation (X3IR),
//! which is a semantic representation suitable for verification, optimization,
//! and code generation.

use crate::arbitrage;
use crate::hedge;
use crate::intent_emit;
use crate::ir::{
    self, ChainMetricKind, Condition, CrdtKind as IrCrdtKind, EmergencyKind, LifecycleKind, Operation, ProofKind,
    SerialFormat, StorageKind, VectorOp, X3IR,
};
use crate::liquidation;
use crate::rebalance;
use crate::semantic::CompilationMode;
use crate::trading_lowering;
use crate::trading_semantic;
use crate::trading_verify;
use x3_lang_ast::ast;
use x3_lang_ast::ast::*;
use x3_lang_common::Span;

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
                ir.push(Operation::Call {
                    function: "charge_subscription".to_string(),
                    args: vec![sub.name.as_str().to_string(), sub.amount.to_string()],
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
                // Lower strategy as constrained execution
                ir.push(Operation::AtomicBegin);

                // Add requires guards first
                for require in &strategy.requires {
                    ir.push(Operation::Require {
                        kind: require_kind_to_ir(&require.kind),
                        subject: require.subject.as_ref().map(|s| s.as_str().to_string()),
                        condition: guard_condition(require)?,
                        error_msg: None,
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
                    comparison: None,
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
                    weights: portfolio.weights.clone(),
                    criterion: portfolio.criterion.name().to_string(),
                });
            }
            Item::Arb(arb_decl) => {
                // `contract` runs the same filter the search runs, so calling it here
                // repeats no decision: it is what carries the decided bounds and the
                // venues they admit into the artifact, where `x3c lower` shows them.
                let contract = arbitrage::contract(program, arb_decl).map_err(|reason| semantic(&reason))?;
                ir.push(Operation::ArbPlan {
                    chains: contract.chains.clone(),
                    max_hops: contract.max_hops,
                    depth_floor: contract.committed_depth.clone(),
                    flash_max: contract.flash_max.clone(),
                    parallel: contract.parallel,
                    private: contract.private,
                    min_profit_bps: contract.min_profit_bps,
                    max_slippage_bps: contract.max_slippage_bps,
                    max_total_fee_bps: contract.max_total_fee_bps,
                    deadline_ms: contract.deadline_ms,
                    admitted: contract.admitted.clone(),
                });
            }
            Item::AtomicLiquidation(liquidation_decl) => {
                // The `ledger` call repeats nothing the verifier decided: it is what
                // carries the decided figures into the artifact, so a replayer can
                // re-check the plan from it.
                let ledger = liquidation::ledger(liquidation_decl).map_err(|reason| semantic(&reason))?;
                ir.push(Operation::Liquidation {
                    position: ledger.position.clone(),
                    debt_asset: ledger.debt_asset.clone(),
                    collateral_asset: ledger.collateral_asset.clone(),
                    capital: ledger.capital,
                    collateral: ledger.collateral,
                    min_output: ledger.min_output,
                    repaid: ledger.repaid,
                    profit_floor: ledger.profit_floor,
                });
            }
            Item::AtomicHedge(hedge_decl) => {
                // The legs are already known to net: `hedge::verify` runs over the
                // AST before lowering and refuses a hedge that does not. Resolving
                // them here again is what carries the *checked* net into the
                // artifact, not a second opinion about it.
                let exposure = hedge::exposure(hedge_decl).map_err(|reason| semantic(&reason))?;
                ir.push(Operation::Hedge {
                    asset: exposure.asset.clone(),
                    long: exposure.long,
                    short: exposure.short,
                    delta_bps: exposure.delta_bps(),
                    delta_bound_bps: hedge_decl.delta_bound_bps,
                });
            }
            _ => {} // Other items (types, imports, ErrorDecl, etc.) don't generate operations
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
            let cond_ir = expression_to_condition(cond)?;
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
            let _cond_ir = expression_to_condition(cond)?;
            let body_ops = {
                let mut temp_ir = X3IR::new();
                lower_function_body(body, &mut temp_ir)?;
                temp_ir.operations
            };

            ir.push(Operation::Loop {
                max_iterations: 1000, // Safe default limit
                body: body_ops,
            });
        }
        Statement::Atomic(atomic) => {
            ir.push(Operation::AtomicBegin);
            lower_function_body(&atomic.body, ir)?;
            ir.push(Operation::AtomicEnd);
        }
        Statement::Emit(event) => {
            let mut data = std::collections::HashMap::new();
            for (i, arg) in event.payload.iter().enumerate() {
                data.insert(format!("arg{}", i), format!("{:?}", arg));
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
            // release CHAIN.ASSET to ADDR
            ir.push(Operation::Release {
                chain: chain_to_string(chain),
                asset: asset.name.as_str().to_string(),
                to: expression_to_string(to),
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
                input_amount: amount.as_ref().map(expression_to_u128).transpose()?.unwrap_or(0),
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
            ir.push(Operation::Require {
                kind: require_kind_to_ir(&guard.kind),
                subject: guard.subject.as_ref().map(|s| s.as_str().to_string()),
                condition: guard_condition(guard)?,
                error_msg: None,
                comparison: guard.comparison,
            });
        }
        Statement::RouteFallback { replacements, .. } => {
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
        _ => {
            // Other statement types (return, break, etc.)
            ir.push(Operation::Nop);
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
            Annotation::Subscription(amount, period) => ir.push(Operation::Call {
                function: "charge_subscription".to_string(),
                args: vec![amount.to_string(), period.to_string()],
            }),
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
            }),
            Annotation::Whitelist(entries) => ir.push(Operation::Require {
                kind: ir::RequireKind::Custom("whitelist".to_string()),
                subject: None,
                comparison: None,
                condition: Condition::Expression {
                    expr: entries.iter().map(|sym| sym.as_str()).collect::<Vec<_>>().join(","),
                },
                error_msg: Some("call target not whitelisted".to_string()),
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
            Annotation::GasAdaptive => ir.push(Operation::GasAdaptive {
                high_gas_ops: vec![Operation::Nop],
                low_gas_ops: vec![Operation::Nop],
            }),
            Annotation::NoHeap
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
fn expression_to_condition(expr: &Expression) -> Result<Condition, x3_lang_common::X3Error> {
    match expr {
        Expression::Literal(LiteralExpr::Bool(true)) => Ok(Condition::True),
        Expression::Literal(LiteralExpr::Bool(false)) => Ok(Condition::False),
        Expression::Call { callee, args } => condition_from_call(callee, args),
        _ => Ok(Condition::Expression {
            expr: expression_to_string(expr),
        }),
    }
}

/// Convert AST RequireKind to IR RequireKind
fn require_kind_to_ir(kind: &ast::RequireKind) -> ir::RequireKind {
    match kind {
        ast::RequireKind::CanonicalSupply => ir::RequireKind::CanonicalSupply,
        ast::RequireKind::Nonce => ir::RequireKind::NonceUnused,
        ast::RequireKind::BridgeLiquidity => ir::RequireKind::BridgeLiquidity,
        ast::RequireKind::Slippage => ir::RequireKind::SlippageTolerance,
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
    };

    // A criterion that could only rank some of the paths has not ranked the
    // branch set: the winner would be "best of the paths we could measure".
    if ranked.len() != choice.paths.len() {
        return None;
    }

    let better = |candidate: u128, incumbent: u128| match choice.criterion {
        ChoiceCriterion::HighestNetOutput => candidate > incumbent,
        ChoiceCriterion::FewestHops => candidate < incumbent,
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

fn expression_to_string(expr: &Expression) -> String {
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
        Expression::Binary { op, lhs, rhs } => {
            format!("{} {:?} {}", expression_to_string(lhs), op, expression_to_string(rhs))
        }
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
