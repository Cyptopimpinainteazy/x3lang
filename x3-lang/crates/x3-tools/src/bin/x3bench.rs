//! `x3bench` — spec PHASE 50's performance targets.
//!
//! Ten operations the phase names, each measured against the *function that does it*
//! in this workspace, reporting **p50, p95 and p99** from a real distribution of
//! per-iteration times.
//!
//! ## Why this is not criterion
//!
//! `criterion` is a good harness, and adding it here would mean a network dependency
//! and a second build configuration for ten measurements that fit in one file. More
//! importantly, the numbers have to be *reproducible by a reader*, and a dependency
//! that pins its own statistics is one more thing between the reader and the
//! distribution. So this computes the percentiles itself, by nearest rank over the
//! sorted samples, and prints the sample count and the machine beside them.
//!
//! ## What it refuses to fake
//!
//! An operation with no callable entry point would be a hole in the table, and the
//! honest answer is to print the hole. Every operation below names the function it
//! times; if that function cannot be set up, the row says so and the process exits
//! non-zero, so a run that measured six of ten cannot be mistaken for a run that
//! measured all ten.
//!
//! "Do not optimize blindly. Profile first" is the phase's own instruction, and this
//! is the profiling step. It makes no claim that any of these numbers is good.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile, TradingOperation, X3IR,
};
use x3_lang_compiler::{lowering, objective, opportunity, optimizer, parser, semantic, verify};
use x3_lang_vm::trading::{build_receipt, DebtRecord, TradeOutcome, TradingState};

/// A program the compiler layers can all run over: venues for the graph and route
/// search, an objective for the constraint solver, and a swap for lowering.
const SOURCE: &str = r#"venue uniswap_v3 {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps 5
    liquidity 1_000_000
    slippage_bps 8
    latency_ms 12
    finality_blocks 12
    risk 2
}

venue curve_v2 {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.ETH
    asset_out solana.SOL
    fee_bps 4
    liquidity 900_000
    slippage_bps 6
    latency_ms 20
    finality_blocks 12
    risk 3
}

objective cheapest {
    minimize fees;

    constraints {
        hops <= 3;
        fees <= 20bps;
        slippage <= 30bps;
    }
}

intent bench_trade {
    from ethereum.USDC amount 100_000 receiver 0xA1
    to ethereum.ETH receiver 0xA2
    route {
        swap uniswap_v3 ethereum.USDC -> ethereum.ETH amount 100_000 min_output 95_000
    }
    require nonce unused bench_nonce
    require profit >= 20
    require slippage <= 50
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;

/// One measured operation.
struct Measurement {
    operation: &'static str,
    /// The function that does the work, named so a reader can check the claim.
    exercises: &'static str,
    samples: Vec<u64>,
}

/// Time `body` `iterations` times after `warmup` runs, in nanoseconds.
fn measure(
    operation: &'static str,
    exercises: &'static str,
    iterations: usize,
    warmup: usize,
    mut body: impl FnMut() -> Result<(), String>,
) -> Result<Measurement, (String, String)> {
    for _ in 0..warmup {
        body().map_err(|reason| (operation.to_string(), reason))?;
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let start = Instant::now();
        body().map_err(|reason| (operation.to_string(), reason))?;
        samples.push(start.elapsed().as_nanos() as u64);
    }
    Ok(Measurement {
        operation,
        exercises,
        samples,
    })
}

/// The sample at `percentile`, by nearest rank over the sorted samples.
///
/// Nearest rank rather than interpolation: with a small sample count an
/// interpolated p99 is a number that was never observed, and a benchmark that
/// reports unobserved values is the kind of measurement this repository does not
/// accept.
fn percentile(samples: &[u64], percentile: u32) -> u64 {
    let mut sorted: Vec<u64> = samples.to_vec();
    sorted.sort_unstable();
    let rank = ((percentile as usize) * sorted.len()).div_ceil(100);
    let index = rank.saturating_sub(1).min(sorted.len().saturating_sub(1));
    sorted[index]
}

/// The ten operations PHASE 50 names, each against its real entry point.
fn measure_all(iterations: usize) -> (Vec<Measurement>, Vec<(String, String)>) {
    let mut measurements = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    let mut record = |result: Result<Measurement, (String, String)>,
                      measurements: &mut Vec<Measurement>,
                      failures: &mut Vec<(String, String)>| match result {
        Ok(measurement) => measurements.push(measurement),
        Err(failure) => failures.push(failure),
    };

    // 1. Parsing.
    record(
        measure("parsing", "parser::parse_source", iterations, 5, || {
            parser::parse_source(SOURCE).map(|_| ()).map_err(|e| format!("{e}"))
        }),
        &mut measurements,
        &mut failures,
    );

    // Everything below needs the parsed program and the lowered IR; a failure here
    // is reported once per affected operation rather than panicking.
    let program = match parser::parse_source(SOURCE) {
        Ok(program) => Some(program),
        Err(error) => {
            for operation in [
                "type checking",
                "IR lowering",
                "graph construction",
                "route scoring",
                "constraint solving",
                "dependency scheduling",
                "economic verification",
                "receipt verification",
                "economic replay",
            ] {
                failures.push((operation.to_string(), format!("the fixture did not parse: {error}")));
            }
            return (measurements, failures);
        }
    };
    let program = program.expect("checked above");
    let ir = lowering::lower_program(&program, lowering::LowerCtx::new()).ok();

    // 2. Type checking: the semantic pass over the lowered IR.
    if let Some(ir) = &ir {
        let ir = ir.clone();
        record(
            measure("type checking", "semantic::verify_collect", iterations, 5, || {
                let outcome = semantic::verify_collect(
                    &ir,
                    semantic::DEFAULT_MAX_ATOMIC_OPS,
                    semantic::DEFAULT_MAX_ROUTE_HOPS,
                    Some(x3_lang_compiler::CompilationMode::Dev),
                );
                if outcome.errors.is_empty() {
                    Ok(())
                } else {
                    Err(format!("{} diagnostic(s)", outcome.errors.len()))
                }
            }),
            &mut measurements,
            &mut failures,
        );
    } else {
        failures.push(("type checking".to_string(), "the fixture did not lower".to_string()));
    }

    // 3. IR lowering.
    {
        let program = program.clone();
        record(
            measure("IR lowering", "lowering::lower_program", iterations, 5, || {
                lowering::lower_program(&program, lowering::LowerCtx::new())
                    .map(|_| ())
                    .map_err(|e| format!("{e}"))
            }),
            &mut measurements,
            &mut failures,
        );
    }

    // 4. Graph construction.
    {
        let program = program.clone();
        record(
            measure(
                "graph construction",
                "opportunity::OpportunityGraph::from_program",
                iterations,
                5,
                || {
                    let graph = opportunity::OpportunityGraph::from_program(&program);
                    if graph.edges.is_empty() {
                        Err("the graph has no edges".to_string())
                    } else {
                        Ok(())
                    }
                },
            ),
            &mut measurements,
            &mut failures,
        );
    }

    let graph = opportunity::OpportunityGraph::from_program(&program);

    // 5. Constraint solving: the objective's constraints lowered against the program.
    {
        let program = program.clone();
        record(
            measure(
                "constraint solving",
                "objective::constraints_for",
                iterations,
                5,
                || {
                    let declaration = objective::declaration_of(&program)
                        .ok_or_else(|| "the fixture declares no objective".to_string())?;
                    let constraints = objective::constraints_for(&declaration.constraints, &program);
                    if constraints.max_hops == 0 {
                        Err("no hop bound was read".to_string())
                    } else {
                        Ok(())
                    }
                },
            ),
            &mut measurements,
            &mut failures,
        );
    }

    let constraints = objective::declaration_of(&program)
        .map(|declaration| objective::constraints_for(&declaration.constraints, &program))
        .unwrap_or_default();

    // 6. Route scoring: the search plus the ranking it reports.
    {
        let graph = graph.clone();
        let constraints = constraints.clone();
        record(
            measure("route scoring", "optimizer::optimize", iterations, 5, || {
                let report = optimizer::optimize(
                    &graph,
                    "ethereum.USDC",
                    "ethereum.ETH",
                    optimizer::Objective::MinimizeFees,
                    &constraints,
                );
                if report.chosen.is_some() {
                    Ok(())
                } else {
                    Err("the search selected no route".to_string())
                }
            }),
            &mut measurements,
            &mut failures,
        );
    }

    // 7. Dependency scheduling: the wave plan over independent legs.
    {
        let (Some(ir), true) = (ir.clone(), true) else {
            failures.push((
                "dependency scheduling".to_string(),
                "the fixture did not lower".to_string(),
            ));
            return finish(measurements, failures);
        };
        let legs = scheduling_legs(&ir);
        match legs {
            Ok(legs) => {
                let domains = BTreeMap::new();
                record(
                    measure("dependency scheduling", "dag::plan", iterations, 5, || {
                        x3_lang_compiler::dag::plan(&legs, &domains)
                            .map(|_| ())
                            .map_err(|e| format!("{e:?}"))
                    }),
                    &mut measurements,
                    &mut failures,
                );
            }
            Err(reason) => failures.push(("dependency scheduling".to_string(), reason)),
        }
    }

    // 8. Economic verification: the IR verifier's structural pass.
    if let Some(ir) = &ir {
        let ir = ir.clone();
        record(
            measure("economic verification", "verify::verify_ir", iterations, 5, || {
                verify::verify_ir(&ir).map_err(|diagnostics| format!("{} diagnostic(s)", diagnostics.len()))
            }),
            &mut measurements,
            &mut failures,
        );
    } else {
        failures.push((
            "economic verification".to_string(),
            "the fixture did not lower".to_string(),
        ));
    }

    // 9 and 10. Receipt verification and economic replay over one built receipt.
    match bench_receipt() {
        Ok(receipt) => {
            let one = receipt.clone();
            record(
                measure("receipt verification", "trading::verify_receipt", iterations, 5, || {
                    x3_lang_vm::trading::verify_receipt(&one).map_err(|e| format!("{e}"))
                }),
                &mut measurements,
                &mut failures,
            );
            let two = receipt;
            record(
                measure(
                    "economic replay",
                    "trading::verify_receipt_economics",
                    iterations,
                    5,
                    || x3_lang_vm::trading::verify_receipt_economics(&two).map_err(|e| format!("{e}")),
                ),
                &mut measurements,
                &mut failures,
            );
        }
        Err(reason) => {
            failures.push(("receipt verification".to_string(), reason.clone()));
            failures.push(("economic replay".to_string(), reason));
        }
    }

    finish(measurements, failures)
}

fn finish(
    measurements: Vec<Measurement>,
    failures: Vec<(String, String)>,
) -> (Vec<Measurement>, Vec<(String, String)>) {
    (measurements, failures)
}

/// Two legs with **disjoint** read and write sets, so the scheduler has a plan to
/// make rather than a cycle to refuse. Legs built from the same operations are not
/// independent, and the planner says so (`Cycle`) — which is what the first version
/// of this fixture measured.
fn scheduling_legs(_ir: &X3IR) -> Result<Vec<x3_lang_compiler::dag::Leg>, String> {
    use x3_lang_compiler::ir::Operation;

    let on_ethereum = vec![Operation::Swap {
        from_chain: "ethereum".to_string(),
        from_asset: "USDC".to_string(),
        to_chain: "ethereum".to_string(),
        to_asset: "ETH".to_string(),
        input_amount: 100_000,
        min_output: 95_000,
        dex: Some("uniswap_v3".to_string()),
    }];
    let on_solana = vec![Operation::Swap {
        from_chain: "solana".to_string(),
        from_asset: "SOL".to_string(),
        to_chain: "solana".to_string(),
        to_asset: "USDC".to_string(),
        input_amount: 1_000,
        min_output: 900,
        dex: Some("jupiter".to_string()),
    }];
    let first =
        x3_lang_compiler::dag::leg_from_operations("leg_a", &on_ethereum).map_err(|e| format!("leg_a: {e:?}"))?;
    let second =
        x3_lang_compiler::dag::leg_from_operations("leg_b", &on_solana).map_err(|e| format!("leg_b: {e:?}"))?;
    Ok(vec![first, second])
}

/// A receipt the two receipt operations can verify. Built once, cloned per call, so
/// the numbers measure verification rather than construction.
fn bench_receipt() -> Result<x3_lang_vm::trading::TradeReceipt, String> {
    let asset = AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: "0xUSDC".to_string(),
        symbol: "USDC".to_string(),
        decimals: 6,
    };
    let operations = vec![
        TradingOperation::BeginAtomicTrade {
            trade_id: "T".to_string(),
            policy: CompiledTradingPolicy {
                policy_id: "P".to_string(),
                policy_version: 1,
                chain: "ethereum".to_string(),
                max_slippage_bps: 30,
                max_gas: 1_000_000,
                max_gas_asset: asset.clone(),
                max_flash_fee_bps: 10,
                deadline_blocks: 10,
                require_private_submission: false,
                minimum_net_profit: None,
                minimum_net_profit_asset: None,
                quote_freshness_blocks: Some(10),
                submission_profile: SubmissionProfile::Public,
                state_binding: StateBindingMode::Exact,
                allowed_cost_kinds: BTreeSet::from([
                    CostKind::Gas,
                    CostKind::LiquidityFee,
                    CostKind::FlashLiquidityFee,
                    CostKind::Slippage,
                    CostKind::PriceImpact,
                    CostKind::MevLeakage,
                ]),
                allow_mint: false,
                allow_burn: false,
                max_oracle_deviation_bps: None,
                max_cumulative_loss: None,
                max_cumulative_loss_asset: None,
            },
        },
        TradingOperation::OpenDebt {
            debt_id: "debt".to_string(),
            provider: "aave_v3".to_string(),
            asset: asset.clone(),
            principal: 1_000_000,
        },
        TradingOperation::CloseDebt {
            debt_id: "debt".to_string(),
        },
        TradingOperation::AssertMinNetProfit {
            settlement_asset: asset.clone(),
            minimum: 1,
        },
        TradingOperation::AssertAllDebtsClosed,
        TradingOperation::EmitTradeReceipt,
        TradingOperation::CommitAtomicTrade,
    ];
    let mut state = TradingState {
        committed: true,
        receipt_emitted: true,
        ..TradingState::default()
    };
    state.closed_debts.insert("debt".to_string());
    state.closed_debt_records.insert(
        "debt".to_string(),
        DebtRecord {
            asset: asset.clone(),
            principal: 1_000_000,
            fee: 0,
        },
    );
    let mut deltas = BTreeMap::new();
    deltas.insert(asset.clone(), 1i128);
    state.net_deltas = deltas;
    build_receipt(
        "0.1.0",
        [1u8; 32],
        "T",
        "P",
        [2u8; 32],
        &operations,
        &state,
        Some(&asset),
        TradeOutcome::Success,
    )
    .map_err(|e| format!("the fixture receipt did not build: {e}"))
}

/// The machine the numbers came from, so a reader can compare their own run.
fn machine() -> String {
    let model = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find(|line| line.starts_with("model name"))
                .and_then(|line| line.split(':').nth(1))
                .map(|name| name.trim().to_string())
        })
        .unwrap_or_else(|| "unknown".to_string());
    let cores = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(0);
    format!("{model}, {cores} logical core(s)")
}

fn main() -> std::process::ExitCode {
    let iterations: usize = std::env::args()
        .nth(1)
        .map(|arg| arg.parse().unwrap_or(200))
        .unwrap_or(200);
    let (measurements, failures) = measure_all(iterations);

    println!("# x3lang PHASE 50 measurements");
    println!();
    println!("| operation | exercises | n | p50 (ns) | p95 (ns) | p99 (ns) |");
    println!("|---|---|---|---|---|---|");
    for measurement in &measurements {
        println!(
            "| {} | `{}` | {} | {} | {} | {} |",
            measurement.operation,
            measurement.exercises,
            measurement.samples.len(),
            percentile(&measurement.samples, 50),
            percentile(&measurement.samples, 95),
            percentile(&measurement.samples, 99),
        );
    }
    println!();
    println!("iterations per operation: {iterations}");
    println!("machine: {}", machine());
    println!("profile: dev (the profile `cargo build` produces)");
    println!("percentiles: nearest rank over the sorted samples, so every printed value was observed");

    if failures.is_empty() {
        println!();
        println!("all {} operations measured", measurements.len());
        std::process::ExitCode::SUCCESS
    } else {
        println!();
        println!("NOT MEASURED:");
        for (operation, reason) in &failures {
            println!("- {operation}: {reason}");
        }
        // Exit non-zero so a partial run cannot be mistaken for a complete one.
        std::process::ExitCode::from(1)
    }
}
