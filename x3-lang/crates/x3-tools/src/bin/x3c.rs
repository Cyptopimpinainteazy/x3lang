//! `x3c` — the production x3-lang CLI.
//!
//! Subcommands (target G in the production contract):
//!
//! - `parse <file.x3>` — parse source into an AST, dump to JSON.
//! - `check <file.x3>` — run the semantic verifier on the lowered IR.
//! - `lower <file.x3> --out <file.x3ir>` — lower AST to X3IR JSON.
//! - `build <file.x3> --out <file.x3b>` — compile to bytecode.
//! - `simulate <file.x3b>` — run on a dry-run VM, print receipt counts.
//! - `run <file.x3b>` — run on a dry-run VM.
//! - `explain <file.x3b>` — disassemble bytecode into per-instruction IR
//!   pseudo-code so a reviewer can read what the program does without
//!   reading raw bytes.
//! - `test-fixture` — emit a known-good fixture set the test harness
//!   consumes.
//!
//! B-52 commands:
//! - `fmt` — format .x3 source files
//! - `lint` — run static analysis linter
//! - `score` — compute route/risk score for an intent
//! - `test` — generate and run tests for an intent
//! - `fuzz` — generate fuzz tests
//! - `chaos` — generate chaos test scenarios
//! - `deploy` — compile and print deployment plan
//! - `inspect` — inspect compiled intent/bytecode
//! - `verify` — verify an intent against a proof fixture
//! - `audit` — run mainnet safety audit on intent
//! - `refund` — inspect/trigger refund for intent
//! - `new` — generate new X3 project
//! - `plan` — show the execution plan for an intent

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use x3_lang_ast::ast::Program;
use x3_lang_compiler::emitter::decode_trading_program;
use x3_lang_compiler::ir::TradingOperation;
use x3_lang_compiler::semantic::VerifyOutcome;
use x3_lang_compiler::{
    check_source, check_source_diagnostics_with_mode, check_source_with_mode, compile_source, compile_to_ir,
    compile_with_mode_diagnostics, CompilationMode,
};
use x3_lang_vm::trading::{
    build_receipt, sign_receipt, verify_receipt_trusted, BorrowRequest, BorrowResult, BridgeRequest,
    BridgeTransferResult, CapabilityManifest, CapabilityMode, CommittedCost, ExecutionMode, HostError, QuoteRequest,
    QuoteResult, RepayRequest, RepayResult, SwapRequest, SwapResult, TradeExecutionContext, TradeOutcome, TradingHost,
    TradingVm,
};
use x3_lang_vm::{VMConfig, VMState, VM};

#[derive(Parser, Debug)]
#[command(name = "x3c", about = "x3-lang compiler and VM driver")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,

    /// Operating mode: dev, testnet, or mainnet
    #[arg(long, global = true, default_value = "dev")]
    mode: String,

    /// Treat semantic warnings as failures. The verifier collects warnings as
    /// well as errors, and a warning that nobody acts on is how a safety check
    /// silently stops protecting anything; this makes "no warnings" enforceable
    /// in CI.
    #[arg(long, global = true)]
    deny_warnings: bool,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Parse a `.x3` source file and dump the AST to JSON.
    Parse {
        input: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Lower to X3IR and run the semantic verifier.
    Check {
        input: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Lower AST to X3IR (JSON).
    Lower {
        input: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Compile a `.x3` source to bytecode.
    Build {
        input: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
    },
    /// Run bytecode on the dry-run VM, print stats.
    Simulate {
        input: PathBuf,
        #[arg(long, default_value_t = 1_000_000u128)]
        gas: u128,
    },
    /// Run bytecode on the dry-run VM (alias of simulate that exits 0/!0).
    Run {
        input: PathBuf,
        #[arg(long, default_value_t = 1_000_000u128)]
        gas: u128,
    },
    /// Disassemble bytecode to a human-readable IR trace.
    Explain { input: PathBuf },
    /// Emit a known-good fixture for the test harness.
    TestFixture {
        #[arg(short, long, default_value = "x3c-fixture.x3")]
        out: PathBuf,
    },
    /// Compile intent from `.x3` source to canonical intent spec.
    Intent {
        input: PathBuf,
        #[arg(long)]
        emit_hash: bool,
        #[arg(long)]
        emit_plan: bool,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Read a validated_intent_v1 JSON envelope, lower to X3IR, run the
    /// semantic verifier, and execute the resulting VM path.
    RunIntent {
        input: PathBuf,
        #[arg(long, default_value_t = 1_000_000u128)]
        gas: u128,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Prove an `.x3` intent against a fixture file.
    Prove {
        input: PathBuf,
        fixture: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    // ===== B-52 Feature Lock Commands =====
    /// Format `.x3` source files (pretty-print).
    Fmt {
        input: PathBuf,
        #[arg(long)]
        check: bool,
    },
    /// Run static analysis linter.
    Lint { input: PathBuf },
    /// Build the opportunity graph from a program's `venue` declarations and
    /// search it for routes between two assets.
    Graph {
        input: PathBuf,
        /// Asset to start from, as `chain.ASSET`.
        #[arg(long)]
        from: String,
        /// Asset to reach, as `chain.ASSET`.
        #[arg(long)]
        to: String,
        /// Maximum hops.
        #[arg(long, default_value_t = 4)]
        max_hops: usize,
        /// Reject venues that declare more slippage than this, in bps.
        #[arg(long)]
        max_slippage_bps: Option<u32>,
        /// Reject venues whose declared liquidity is below this.
        #[arg(long)]
        min_liquidity: Option<u128>,
    },
    /// Choose one route from the opportunity graph, deterministically.
    Optimize {
        input: PathBuf,
        /// Asset to start from, as `chain.ASSET`.
        #[arg(long)]
        from: String,
        /// Asset to reach, as `chain.ASSET`.
        #[arg(long)]
        to: String,
        /// What to optimize for. Defaults to the program's `objective`
        /// declaration if it has one, and to `minimize_fees` if it has neither.
        #[arg(long)]
        objective: Option<String>,
        /// Maximum hops. Defaults to the bound the program's `objective`
        /// declaration states, and to 4 if it states none.
        #[arg(long)]
        max_hops: Option<usize>,
        /// Reject venues that declare more slippage than this, in bps.
        #[arg(long)]
        max_slippage_bps: Option<u32>,
    },
    /// Find rings of intents that could settle against each other instead of
    /// each taking external liquidity.
    Fusion { input: PathBuf },
    /// Print a compiled strategy's marketplace metadata: what it is, what it
    /// needs, and what it does not expose.
    Metadata {
        input: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
    /// Compute route/risk score for an intent.
    Score { input: PathBuf },
    /// Generate and run tests for an intent.
    Test {
        input: PathBuf,
        #[arg(long)]
        generate_only: bool,
        #[arg(long, default_value = "x3-tests")]
        out_dir: PathBuf,
    },
    /// Generate fuzz tests.
    Fuzz {
        input: PathBuf,
        #[arg(long, default_value_t = 1000)]
        iterations: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Generate chaos test scenarios.
    Chaos {
        input: PathBuf,
        #[arg(long, default_value_t = 100)]
        scenarios: u32,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Compile and print deployment plan.
    Deploy {
        input: PathBuf,
        #[arg(short, long)]
        target: Option<String>,
        #[arg(short, long)]
        mode: Option<String>,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Inspect compiled intent/bytecode.
    Inspect {
        input: PathBuf,
        #[arg(long)]
        json: bool,
    },
    /// Verify an intent against a proof fixture.
    Verify { intent: PathBuf, proof: PathBuf },
    /// Run mainnet safety audit on intent.
    Audit {
        input: PathBuf,
        #[arg(long, default_value = "mainnet")]
        mode: String,
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Inspect/trigger refund for intent.
    Refund {
        intent_hash: String,
        #[arg(long)]
        check_only: bool,
    },
    /// Generate new X3 project.
    New {
        name: String,
        #[arg(long)]
        path: Option<PathBuf>,
    },
    /// Show the execution plan for an intent.
    Plan {
        input: PathBuf,
        #[arg(long)]
        show_route: bool,
        #[arg(long)]
        json: bool,
    },
    /// Inspect or verify a trading receipt.
    Receipt {
        #[command(subcommand)]
        action: ReceiptAction,
    },
}

#[derive(Subcommand, Debug)]
enum ReceiptAction {
    /// Print a receipt as canonical JSON.
    Inspect { input: PathBuf },
    /// Verify a receipt's hash and accounting invariants.
    Verify { input: PathBuf },
    /// Compile a `.x3` trading program, execute it against a neutral
    /// fixture host, and emit the resulting signed receipt.
    ///
    /// The fixture host is deliberately not a market simulation: every
    /// swap returns exactly the trade's own declared `min_output` (so
    /// OutputBelowMinOut/slippage never fire on their own), fees are
    /// zero, and it claims exactly the providers/venues/private-submission
    /// capability the compiled policy asks for. This proves the compile
    /// -> execute -> receipt -> sign -> verify pipeline actually connects
    /// end to end; it does not simulate real market profitability, and a
    /// program that needs genuine price movement to clear its own
    /// min_profit/min_output guards can still legitimately fail here.
    Execute {
        input: PathBuf,
        #[arg(short, long)]
        out: Option<PathBuf>,
        /// Block height the trade executes at, checked against the
        /// compiled policy's deadline_blocks.
        #[arg(long, default_value_t = 1)]
        block: u64,
        /// 64-character hex-encoded ed25519 signing key seed. Defaults to
        /// a fixed, clearly non-secret dev seed — this command is a
        /// fixture/demo tool, not a production signer.
        #[arg(long)]
        key_hex: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("x3c: {message}");
            ExitCode::from(2)
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode, String> {
    let mode = &cli.mode;
    match cli.command {
        Cmd::Parse { input, out } => cmd_parse(&input, out.as_ref()),
        Cmd::Check { input, out } => cmd_check(&input, out.as_ref(), mode, cli.deny_warnings),
        Cmd::Lower { input, out } => cmd_lower(&input, &out),
        Cmd::Build { input, out } => cmd_build(&input, &out, mode, cli.deny_warnings),
        Cmd::Simulate { input, gas } | Cmd::Run { input, gas } => cmd_run(&input, gas),
        Cmd::Explain { input } => cmd_explain(&input),
        Cmd::TestFixture { out } => cmd_test_fixture(&out),
        Cmd::Intent {
            input,
            emit_hash,
            emit_plan,
            out,
        } => cmd_intent(&input, emit_hash, emit_plan, out.as_ref()),
        Cmd::Prove { input, fixture, out } => cmd_prove(&input, &fixture, out.as_ref()),
        Cmd::RunIntent { input, gas, out } => cmd_run_intent(&input, gas, out.as_ref()),
        // B-52 commands
        Cmd::Fmt { input, check } => cmd_fmt(&input, check),
        Cmd::Lint { input } => cmd_lint(&input, mode),
        Cmd::Graph {
            input,
            from,
            to,
            max_hops,
            max_slippage_bps,
            min_liquidity,
        } => cmd_graph(&input, &from, &to, max_hops, max_slippage_bps, min_liquidity),
        Cmd::Optimize {
            input,
            from,
            to,
            objective,
            max_hops,
            max_slippage_bps,
        } => cmd_optimize(&input, &from, &to, objective.as_deref(), max_hops, max_slippage_bps),
        Cmd::Fusion { input } => cmd_fusion(&input),
        Cmd::Metadata { input, out } => cmd_metadata(&input, out.as_ref(), mode),
        Cmd::Score { input } => cmd_score(&input, mode),
        Cmd::Test {
            input,
            generate_only,
            out_dir,
        } => cmd_test(&input, generate_only, &out_dir),
        Cmd::Fuzz { input, iterations, out } => cmd_fuzz(&input, iterations, out.as_ref()),
        Cmd::Chaos { input, scenarios, out } => cmd_chaos(&input, scenarios, out.as_ref()),
        Cmd::Deploy {
            input,
            target,
            mode,
            out,
        } => cmd_deploy(&input, target.as_ref(), mode.as_ref(), out.as_ref(), cli.deny_warnings),
        Cmd::Inspect { input, json } => cmd_inspect(&input, json),
        Cmd::Verify { intent, proof } => cmd_verify(&intent, &proof),
        Cmd::Audit { input, mode, out } => cmd_audit(&input, &mode, out.as_ref()),
        Cmd::Refund {
            intent_hash,
            check_only,
        } => cmd_refund(&intent_hash, check_only),
        Cmd::New { name, path } => cmd_new(&name, path.as_ref()),
        Cmd::Plan {
            input,
            show_route,
            json,
        } => cmd_plan(&input, show_route, json),
        Cmd::Receipt { action } => match action {
            ReceiptAction::Inspect { input } => cmd_receipt_inspect(&input),
            ReceiptAction::Verify { input } => cmd_receipt_verify(&input),
            ReceiptAction::Execute {
                input,
                out,
                block,
                key_hex,
            } => cmd_receipt_execute(&input, out.as_ref(), mode, block, key_hex.as_deref(), cli.deny_warnings),
        },
    }
}

// --- subcommand impls ---

fn cmd_parse(input: &PathBuf, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    match x3_lang_compiler::parser::parse_source(&source) {
        Ok(program) => {
            let json = serde_json::to_string_pretty(&program).map_err(|e| format!("serialization failed: {e}"))?;
            write_output(out, &json)?;
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => Err(format!("parse error: {err}")),
    }
}

fn cmd_check(input: &PathBuf, out: Option<&PathBuf>, mode_str: &str, deny_warnings: bool) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let comp_mode = parse_mode(mode_str)?;
    let (program, ir, outcome) =
        check_source_diagnostics_with_mode(&source, comp_mode).map_err(|e| format!("lowering failed: {e}"))?;
    let errs = &outcome.errors;
    let warnings = &outcome.warnings;
    let program_summary = program_summary(&program);
    // A warning used to be collected by the verifier and then dropped on the
    // floor here, so `x3c check` reported a clean program even when a safety
    // pass had something to say. Warnings are now reported, and `--deny-warnings`
    // turns them into a failure.
    let failed = !errs.is_empty() || (deny_warnings && !warnings.is_empty());
    if !failed {
        let warning_list: Vec<String> = warnings.iter().map(|w| format!("{w}")).collect();
        let body = serde_json::json!({
            "status": "ok",
            "program": program_summary,
            "operations": ir.operations.len(),
            "warnings": warning_list,
        });
        let json = serde_json::to_string_pretty(&body).map_err(|e| format!("serialization failed: {e}"))?;
        write_output(out, &json)?;
        for warning in warnings {
            eprintln!("x3c warning: {warning}");
        }
        if warnings.is_empty() {
            println!("x3c check: {} ops, no semantic errors", ir.operations.len());
        } else {
            println!(
                "x3c check: {} ops, no semantic errors, {} warning(s)",
                ir.operations.len(),
                warnings.len()
            );
        }
        Ok(ExitCode::SUCCESS)
    } else {
        let warning_list: Vec<String> = warnings.iter().map(|w| format!("{w}")).collect();
        let body = serde_json::json!({
            "status": "error",
            "program": program_summary,
            "errors": errs.iter().map(|e| format!("{e}")).collect::<Vec<_>>(),
            "warnings": warning_list,
        });
        let json = serde_json::to_string_pretty(&body).map_err(|e| format!("serialization failed: {e}"))?;
        if let Some(o) = out {
            std::fs::write(o, json).map_err(|e| format!("write {o:?}: {e}"))?;
        } else {
            if errs.is_empty() {
                print_error(&format!(
                    "semantic check failed — {} warning(s) treated as errors by --deny-warnings",
                    warnings.len()
                ));
            } else {
                print_error(&format!("semantic check failed — {} error(s)", errs.len()));
            }
            for warning in warnings {
                eprintln!("x3c warning: {warning}");
            }
            eprintln!("{json}");
        }
        Ok(ExitCode::from(1))
    }
}

/// `x3c metadata` — describe a compiled strategy without exposing its source.
///
/// The artifact hash is computed over the bytecode this command compiles, so the
/// document identifies the artifact rather than the file it came from. The two
/// fields PHASE 26 lists that a compiler cannot fill — a signature and a receipt
/// history — are null and the document says why, because metadata that looks
/// complete and is not is worse than metadata that is explicit about its gaps.
fn cmd_metadata(input: &PathBuf, out: Option<&PathBuf>, mode_str: &str) -> Result<ExitCode, String> {
    use x3_lang_compiler::metadata::strategy_metadata;

    let source = read_source(input)?;
    let comp_mode = parse_mode(mode_str)?;
    // Verify first: publishing metadata for a program the compiler rejects would
    // describe an artifact nobody can build.
    let (program, _, outcome) =
        check_source_diagnostics_with_mode(&source, comp_mode).map_err(|e| format!("compile error: {e}"))?;
    if !outcome.errors.is_empty() {
        for error in &outcome.errors {
            print_error(&format!("{error}"));
        }
        return Ok(ExitCode::from(1));
    }
    let (bytecode, _) = compile_with_mode_diagnostics(&source, comp_mode).map_err(|e| format!("compile error: {e}"))?;

    let Some(metadata) = strategy_metadata(&program, &bytecode) else {
        return Err(format!(
            "{input:?} declares no strategy module; metadata describes a module, and this program \
             has none"
        ));
    };
    let json = serde_json::to_string_pretty(&metadata).map_err(|e| format!("serialization failed: {e}"))?;
    write_output(out, &json)?;
    println!(
        "x3c metadata: {} — {} bytes of artifact hashed as {}",
        metadata.strategy_id,
        bytecode.len(),
        metadata.artifact_hash
    );
    Ok(ExitCode::SUCCESS)
}

/// `x3c fusion` — report every ring of intents that could settle internally.
///
/// The report says what it could check and what it could not. A ring whose
/// minimums cannot be verified is printed as unverifiable rather than omitted:
/// an author who opted into fusion needs to know that the compiler looked and
/// could not be sure, which is different from the compiler finding nothing.
fn cmd_fusion(input: &PathBuf) -> Result<ExitCode, String> {
    use x3_lang_compiler::fusion::{flows, rings, Check};

    let source = read_source(input)?;
    // Verify before analysing: an intent the compiler rejects is not a candidate
    // for netting, and a fusion report about a program that does not compile
    // would be advice about code that cannot run.
    let (program, _, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(&source, x3_lang_compiler::CompilationMode::Dev)
            .map_err(|e| format!("compile error: {e}"))?;
    if !outcome.errors.is_empty() {
        for error in &outcome.errors {
            print_error(&format!("{error}"));
        }
        return Ok(ExitCode::from(1));
    }
    let flows = flows(&program);
    let found = rings(&flows);
    let opted_in = flows.iter().filter(|flow| flow.opted_in).count();

    println!(
        "x3c fusion: {} intent(s), {opted_in} opted in, {} ring(s)",
        flows.len(),
        found.len()
    );
    for ring in &found {
        println!(
            "  ring {} (assets {}): earliest deadline {}",
            ring.participants.join(" -> "),
            ring.assets.join(" -> "),
            ring.earliest_deadline
                .map(|blocks| format!("{blocks} block(s)"))
                .unwrap_or_else(|| "unstated".to_string())
        );
        let describe = |label: &str, check: &Check| match check {
            Check::Satisfied => println!("    {label}: satisfied"),
            Check::Unverifiable(reason) => println!("    {label}: UNVERIFIABLE — {reason}"),
            Check::Failed(reason) => println!("    {label}: FAILED — {reason}"),
        };
        describe("authorization", &ring.authorization);
        describe("asset correctness", &ring.asset_correctness);
        describe("minimum output", &ring.minimum_output);
        describe("deadline", &ring.deadline);
        describe("fairness", &ring.fairness);
        println!(
            "    verdict: {}",
            if ring.is_fusable() {
                "fusable — no external liquidity needed for the ringed assets"
            } else {
                "not fusable"
            }
        );
    }
    // An intent that opted in but took no part is worth naming: silence would
    // read as "considered and fine".
    for flow in flows.iter().filter(|flow| flow.opted_in) {
        if !found.iter().any(|ring| ring.participants.contains(&flow.name)) {
            println!(
                "  note: '{}' allowed fusion but supplies no ring: knows what it gives, what it \
                 wants, and nobody closes the loop",
                flow.name
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `x3c optimize` — choose one route, and show enough to review the choice.
fn cmd_optimize(
    input: &PathBuf,
    from: &str,
    to: &str,
    objective_name: Option<&str>,
    max_hops: Option<usize>,
    max_slippage_bps: Option<u32>,
) -> Result<ExitCode, String> {
    use x3_lang_compiler::optimizer::{optimize, NoRoute, Objective};

    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;
    let mut acc = x3_lang_common::ErrorAccumulator::new();
    x3_lang_compiler::semantic::verify_venue_decls(&program, &mut acc);
    // The declaration is checked here as well as in `check`, because this is
    // the command that follows it: an objective the optimizer cannot rank must
    // not be quietly replaced by a default.
    x3_lang_compiler::objective::verify_objective_decls(&program, &mut acc);
    if acc.has_errors() {
        for error in acc.errors() {
            print_error(&format!("{error}"));
        }
        return Ok(ExitCode::from(1));
    }

    let declared = x3_lang_compiler::objective::declaration_of(&program);

    // The command line and the program can each say what to rank. When they
    // disagree the compiler still follows one of them, so saying which by
    // refusing to guess is the only honest answer.
    let (objective, origin): (Objective, String) = match (objective_name, declared) {
        (Some(name), declared) => {
            let Some(from_flag) = Objective::parse(name) else {
                let allowed: Vec<&str> = Objective::ALL.iter().map(|objective| objective.as_str()).collect();
                return Err(format!(
                    "unknown objective '{name}'; the optimizer can only rank what it can \
                     evaluate, so the set is closed: {}",
                    allowed.join(", ")
                ));
            };
            if let Some(declared) = declared {
                let from_decl = match x3_lang_compiler::objective::criterion_for(declared.metric) {
                    Ok(objective) => objective,
                    Err(reason) => {
                        return Err(format!(
                            "objective '{}' cannot rank '{}': {reason}",
                            declared.name.as_str(),
                            declared.metric.as_str()
                        ))
                    }
                };
                if from_decl != from_flag {
                    return Err(format!(
                        "the program declares objective '{}' ({}), and --objective says {}; the \
                         compiler follows one of them, so drop whichever is wrong",
                        declared.name.as_str(),
                        declared.metric.as_str(),
                        from_flag.as_str()
                    ));
                }
                (from_decl, format!("declared by objective '{}'", declared.name.as_str()))
            } else {
                (from_flag, "--objective".to_string())
            }
        }
        (None, Some(declared)) => {
            let from_decl = match x3_lang_compiler::objective::criterion_for(declared.metric) {
                Ok(objective) => objective,
                Err(reason) => {
                    return Err(format!(
                        "objective '{}' cannot rank '{}': {reason}",
                        declared.name.as_str(),
                        declared.metric.as_str()
                    ))
                }
            };
            (from_decl, format!("declared by objective '{}'", declared.name.as_str()))
        }
        (None, None) => (
            Objective::MinimizeFees,
            "the default, since the program declares no objective".to_string(),
        ),
    };

    // A declared ceiling is part of the program; a flag is an instruction about
    // this run, so a flag wins where it is given.
    let mut constraints = match declared {
        Some(declared) => x3_lang_compiler::objective::constraints_for(&declared.constraints, &program),
        None => x3_lang_compiler::opportunity::OpportunityConstraints::default(),
    };
    if let Some(hops) = max_hops {
        constraints.max_hops = hops;
    }
    if let Some(bound) = max_slippage_bps {
        constraints.max_slippage_bps = Some(bound);
    }

    let graph = x3_lang_compiler::opportunity::OpportunityGraph::from_program(&program);
    // A hop bound of zero is "nothing was declared", which the search reads as
    // its own default; the message below has to say the same number it uses.
    let effective_hops = if constraints.max_hops == 0 {
        x3_lang_compiler::opportunity::DEFAULT_MAX_PATH_HOPS
    } else {
        constraints.max_hops
    };
    let report = optimize(&graph, from, to, objective, &constraints);

    match (&report.chosen, &report.no_route) {
        (Some(chosen), _) => {
            println!("x3c optimize: {} ({origin})", objective.as_str());
            println!(
                "  {}  ({} bps fee, {} bps slippage, risk {}, {}ms, {} block(s) finality)",
                chosen.venues.join(" -> "),
                chosen.fee_bps,
                chosen.slippage_bps,
                chosen.max_risk,
                chosen.latency_ms,
                chosen.finality_blocks
            );
            println!(
                "  within {effective_hops} hop(s), considered {} route(s); {}",
                report.considered,
                if report.decided_by_objective() {
                    "the objective decided".to_string()
                } else {
                    // Saying this out loud is the point: a caller that believes
                    // the objective decided when a tie-break did has been
                    // misled about the quality of the choice.
                    format!(
                        "{} route(s) tied on {}, broken canonically by venue order: {}",
                        report.tied.len(),
                        objective.as_str(),
                        report
                            .tied
                            .iter()
                            .map(|opportunity| opportunity.venues.join(" -> "))
                            .collect::<Vec<_>>()
                            .join("; ")
                    )
                }
            );
            Ok(ExitCode::SUCCESS)
        }
        (None, Some(NoRoute::BudgetExhausted { examined, budget })) => {
            print_error(&format!(
                "optimizer exhausted its expansion budget ({examined} of {budget}); the graph is too \
                 large to optimize — reduce --max-hops or narrow the constraints"
            ));
            Ok(ExitCode::from(1))
        }
        (None, Some(NoRoute::AllRefused { refused })) => {
            print_error("no route satisfies the constraints:");
            for (venue, reason) in refused {
                println!("  refused {venue}: {reason:?}");
            }
            Ok(ExitCode::from(1))
        }
        (None, _) => {
            print_error(&format!("no route from {from} to {to} within {effective_hops} hop(s)"));
            Ok(ExitCode::from(1))
        }
    }
}

/// `x3c graph` — build the opportunity graph and search it.
///
/// This is the reachability the graph needs to be a feature rather than a
/// module: without a command that builds a graph from a real program and prints
/// what the planner found, nothing outside the tests would ever exercise it.
fn cmd_graph(
    input: &PathBuf,
    from: &str,
    to: &str,
    max_hops: usize,
    max_slippage_bps: Option<u32>,
    min_liquidity: Option<u128>,
) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    // The declarations are checked before they are searched: a graph built from
    // an inconsistent venue would answer confidently about a node that cannot
    // exist.
    let mut acc = x3_lang_common::ErrorAccumulator::new();
    x3_lang_compiler::semantic::verify_venue_decls(&program, &mut acc);
    if acc.has_errors() {
        for error in acc.errors() {
            print_error(&format!("{error}"));
        }
        return Ok(ExitCode::from(1));
    }

    // A program can declare what it wants ranked, with ceilings the planner must
    // respect. This command searches the graph *without* them — it exists to show
    // what the graph holds — so it says that out loud rather than letting a reader
    // take the list for the set the program allows. `x3c optimize` is the command
    // that applies them (TICKET-042).
    if let Some(declared) = x3_lang_compiler::objective::declaration_of(&program) {
        println!(
            "  note: the program declares objective '{}' ({}); this command ignores its constraints \
             and lists what the graph holds — `x3c optimize` applies them",
            declared.name.as_str(),
            declared.metric.as_str()
        );
    }

    let graph = x3_lang_compiler::opportunity::OpportunityGraph::from_program(&program);
    let constraints = x3_lang_compiler::opportunity::OpportunityConstraints {
        max_hops,
        max_slippage_bps,
        min_liquidity,
        ..Default::default()
    };
    let outcome = x3_lang_compiler::opportunity::search(&graph, from, to, &constraints);
    let opportunities = match &outcome {
        x3_lang_compiler::opportunity::SearchOutcome::Found(found) => found.clone(),
        x3_lang_compiler::opportunity::SearchOutcome::BudgetExhausted { examined, budget } => {
            // "I stopped looking" is not "there is nothing there". Reporting an
            // unreachable route for a search that never finished is the worst
            // answer this command can give.
            print_error(&format!(
                "search exhausted its expansion budget ({examined} of {budget}); narrow the \
                 constraints or reduce --max-hops"
            ));
            return Ok(ExitCode::from(1));
        }
    };

    println!(
        "x3c graph: {} venue(s), {} edge(s); {from} -> {to} within {} hop(s): {} opportunit{}",
        graph.edges.len(),
        graph.edges.len(),
        constraints.max_hops,
        opportunities.len(),
        if opportunities.len() == 1 { "y" } else { "ies" }
    );
    for (rank, opportunity) in opportunities.iter().enumerate() {
        println!(
            "  {}. {}  fee {} bps, slippage {} bps, risk {}, latency {}ms, finality {} block(s), \
             min liquidity {}",
            rank + 1,
            opportunity.venues.join(" -> "),
            opportunity.fee_bps,
            opportunity.slippage_bps,
            opportunity.max_risk,
            opportunity.latency_ms,
            opportunity.finality_blocks,
            opportunity.min_liquidity
        );
    }
    if opportunities.is_empty() {
        // An empty result is the interesting case, so say which venues were
        // refused and why rather than printing nothing.
        for edge in &graph.edges {
            if let Some(reason) = x3_lang_compiler::opportunity::reject_reason(edge, &constraints) {
                println!("  refused {} -> {}: {reason:?}", edge.from, edge.venue);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_lower(input: &PathBuf, out: &PathBuf) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;
    let ir = compile_to_ir(&program).map_err(|e| format!("lower error: {e}"))?;
    let json = serde_json::to_string_pretty(&ir).map_err(|e| format!("serialization failed: {e}"))?;
    std::fs::write(out, json).map_err(|e| format!("write {out:?}: {e}"))?;
    println!("x3c lower: {} operations -> {}", ir.operations.len(), out.display());
    Ok(ExitCode::SUCCESS)
}

/// Surface the verifier's warnings, and refuse to continue when
/// `--deny-warnings` is set.
///
/// `--deny-warnings` is a global flag whose help text promises "semantic
/// warnings as failures", but it used to be honoured by `check` alone: every
/// other command that compiled a program accepted the flag and dropped the
/// warnings on the floor, so the same source came back clean from `build` and
/// warned from `check`. Measured before the fix: a same-chain intent built with
/// `x3c build --deny-warnings` exited 0 while `x3c check` reported one
/// warning. Every command that can produce a warning goes through here now.
fn report_warnings(outcome: &VerifyOutcome, deny_warnings: bool, command: &str) -> Result<(), ExitCode> {
    for warning in &outcome.warnings {
        eprintln!("x3c warning: {warning}");
    }
    if deny_warnings && !outcome.warnings.is_empty() {
        print_error(&format!(
            "{command} failed — {} warning(s) treated as errors by --deny-warnings",
            outcome.warnings.len()
        ));
        return Err(ExitCode::from(1));
    }
    Ok(())
}

fn cmd_build(input: &PathBuf, out: &PathBuf, mode_str: &str, deny_warnings: bool) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let comp_mode = parse_mode(mode_str)?;
    let (bytecode, outcome) =
        compile_with_mode_diagnostics(&source, comp_mode).map_err(|e| format!("compile error: {e}"))?;

    if let Err(code) = report_warnings(&outcome, deny_warnings, "build") {
        return Ok(code);
    }

    std::fs::write(out, &bytecode).map_err(|e| format!("write {out:?}: {e}"))?;
    println!(
        "x3c build: {} bytes ({} ops){} -> {}",
        bytecode.len(),
        bytecode.len() / 4,
        if outcome.warnings.is_empty() {
            String::new()
        } else {
            format!(", {} warning(s)", outcome.warnings.len())
        },
        out.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_run(input: &PathBuf, gas: u128) -> Result<ExitCode, String> {
    let bytecode = std::fs::read(input).map_err(|e| format!("read {input:?}: {e}"))?;
    if bytecode.is_empty() {
        return Err("bytecode is empty".into());
    }
    let mut vm = VM::new(bytecode, VMConfig::default(), gas);
    match vm.execute() {
        Ok(()) => {
            let (asset_ops, bridge_ops, receipts) = collect_stats(&vm.state);
            println!(
                "x3c run: ok — {} asset ops, {} bridge ops, {} receipts, gas remaining {}",
                asset_ops, bridge_ops, receipts, vm.state.gas
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            print_error(&format!("VM error: {err:?}"));
            Ok(ExitCode::from(1))
        }
    }
}

fn collect_stats(state: &VMState) -> (usize, usize, usize) {
    (
        state.asset_ops.len(),
        state.bridge_ops.len(),
        state.bridge_receipts.len(),
    )
}

fn cmd_run_intent(input: &PathBuf, gas: u128, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;

    let intent = x3_lang_compiler::parse_validated_intent_json(&source)
        .map_err(|e| format!("invalid validated_intent_v1: {e}"))?;

    let ir = match x3_lang_compiler::to_ir(&intent) {
        Ok(ir) => ir,
        Err(err) => {
            let body = serde_json::json!({
                "status": "error",
                "stage": "lowering",
                "errors": [format!("{err}")],
            });
            let json = serde_json::to_string_pretty(&body).map_err(|e| format!("serialization failed: {e}"))?;
            if let Some(o) = out {
                std::fs::write(o, json).map_err(|e| format!("write {o:?}: {e}"))?;
            } else {
                print_error(&format!("intent lowering failed: {err}"));
                eprintln!("{json}");
            }
            return Ok(ExitCode::from(1));
        }
    };

    if let Err(errs) = x3_lang_compiler::check_ir(&ir) {
        let body = serde_json::json!({
            "status": "error",
            "stage": "semantic",
            "operations": ir.operations.len(),
            "errors": errs.iter().map(|e| format!("{e}")).collect::<Vec<_>>(),
        });
        let json = serde_json::to_string_pretty(&body).map_err(|e| format!("serialization failed: {e}"))?;
        if let Some(o) = out {
            std::fs::write(o, json).map_err(|e| format!("write {o:?}: {e}"))?;
        } else {
            print_error(&format!("semantic check failed — {} error(s)", errs.len()));
            eprintln!("{json}");
        }
        return Ok(ExitCode::from(1));
    }

    let bytecode = match x3_lang_compiler::emitter::emit_x3ir(&ir) {
        Ok(code) => code,
        Err(err) => {
            print_error(&format!("bytecode emission failed: {err}"));
            return Ok(ExitCode::from(1));
        }
    };

    let mut vm = VM::new(bytecode, VMConfig::default(), gas);
    match vm.execute() {
        Ok(()) => {
            let (asset_ops, bridge_ops, receipts) = collect_stats(&vm.state);
            let body = serde_json::json!({
                "status": "ok",
                "operations": ir.operations.len(),
                "asset_ops": asset_ops,
                "bridge_ops": bridge_ops,
                "receipts": receipts,
                "gas_remaining": vm.state.gas.to_string(),
            });
            let json = serde_json::to_string_pretty(&body).map_err(|e| format!("serialization failed: {e}"))?;
            write_output(out, &json)?;
            println!(
                "x3c run-intent: ok — {} asset ops, {} bridge ops, {} receipts, gas remaining {}",
                asset_ops, bridge_ops, receipts, vm.state.gas
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(err) => {
            let body = serde_json::json!({
                "status": "error",
                "stage": "vm",
                "errors": [format!("{err:?}")],
            });
            let json = serde_json::to_string_pretty(&body).map_err(|e| format!("serialization failed: {e}"))?;
            if let Some(o) = out {
                std::fs::write(o, json).map_err(|e| format!("write {o:?}: {e}"))?;
            } else {
                print_error(&format!("VM error: {err:?}"));
                eprintln!("{json}");
            }
            Ok(ExitCode::from(1))
        }
    }
}

fn cmd_explain(input: &PathBuf) -> Result<ExitCode, String> {
    let bytecode = std::fs::read(input).map_err(|e| format!("read {input:?}: {e}"))?;
    let trace = x3_lang_compiler::emitter::disassemble(&bytecode).map_err(|e| format!("disassembly failed: {e}"))?;
    println!("{trace}");
    Ok(ExitCode::SUCCESS)
}

fn cmd_test_fixture(out: &PathBuf) -> Result<ExitCode, String> {
    const FIXTURE: &str = r#"intent arb_solana_eth {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 777
    }
}
"#;
    std::fs::write(out, FIXTURE).map_err(|e| format!("write {out:?}: {e}"))?;
    println!("x3c test-fixture: wrote {}", out.display());
    Ok(ExitCode::SUCCESS)
}

fn cmd_intent(input: &PathBuf, emit_hash: bool, emit_plan: bool, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let intent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl),
            _ => None,
        })
        .ok_or_else(|| "no intent declaration found in source".to_string())?;

    let draft = x3_lang_compiler::intent_emit::from_intent_decl(intent_decl);

    let mut output = serde_json::json!({
        "name": draft.name,
        "source_chain": draft.source_chain,
        "source_asset": draft.source_asset,
        "source_amount": draft.source_amount,
        "source_owner": draft.source_owner,
        "dest_chain": draft.dest_chain,
        "dest_asset": draft.dest_asset,
        "dest_receiver": draft.dest_receiver,
        "timeout_secs": draft.timeout_secs,
        "constraints": draft.constraints.iter().map(|c| serde_json::json!({
            "kind": c.kind,
            "arg": c.arg,
        })).collect::<Vec<_>>(),
    });

    if emit_hash {
        use sha3::{Digest, Sha3_256};
        let draft_json = serde_json::to_string(&draft).map_err(|e| format!("serialization failed: {e}"))?;
        let hash = Sha3_256::digest(draft_json.as_bytes());
        let hex_hash: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
        output["intent_hash"] = serde_json::json!(hex_hash);
    }

    if emit_plan {
        let plan_steps: Vec<String> = draft
            .constraints
            .iter()
            .map(|c| format!("require {} {}", c.kind, c.arg))
            .collect();
        output["plan_steps"] = serde_json::json!(plan_steps);
    }

    let json = serde_json::to_string_pretty(&output).map_err(|e| format!("serialization failed: {e}"))?;

    println!("x3c intent: compiled intent '{}'", draft.name);
    write_output(out, &json)?;
    Ok(ExitCode::SUCCESS)
}

fn cmd_prove(input: &PathBuf, fixture: &PathBuf, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let fixture_json = std::fs::read_to_string(fixture).map_err(|e| format!("read fixture {fixture:?}: {e}"))?;

    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let intent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl),
            _ => None,
        })
        .ok_or_else(|| "no intent declaration found".to_string())?;

    let draft = x3_lang_compiler::intent_emit::from_intent_decl(intent_decl);

    use sha3::{Digest, Sha3_256};
    let draft_json = serde_json::to_string(&draft).map_err(|e| format!("serialization failed: {e}"))?;
    let hash = Sha3_256::digest(draft_json.as_bytes());
    let hex_hash: String = hash.iter().map(|b| format!("{:02x}", b)).collect();

    let fixture_val: serde_json::Value =
        serde_json::from_str(&fixture_json).map_err(|e| format!("invalid fixture JSON: {e}"))?;

    let expected_name = fixture_val.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let name_match = draft.name == expected_name;

    let expected_source = fixture_val.get("source_asset").and_then(|v| v.as_str()).unwrap_or("");
    let source_match = draft.source_asset == expected_source;

    let expected_dest = fixture_val.get("dest_asset").and_then(|v| v.as_str()).unwrap_or("");
    let dest_match = draft.dest_asset == expected_dest;

    let all_pass = name_match && source_match && dest_match;

    let result = serde_json::json!({
        "proven": all_pass,
        "intent_name": draft.name,
        "source_asset": draft.source_asset,
        "dest_asset": draft.dest_asset,
        "matches": {
            "name": name_match,
            "source_asset": source_match,
            "dest_asset": dest_match,
        },
        "intent_hash": hex_hash,
        "fixture_path": format!("{}", fixture.display()),
    });

    let json = serde_json::to_string_pretty(&result).map_err(|e| format!("serialization failed: {e}"))?;

    if all_pass {
        println!("x3c prove: PASS — intent '{}' matches fixture", draft.name);
        write_output(out, &json)?;
        Ok(ExitCode::SUCCESS)
    } else {
        print_error(&format!("intent '{}' does not match fixture", draft.name));
        if out.is_none() {
            print_error("proof verification failed — see JSON output below");
            eprintln!("{json}");
        } else if let Some(p) = out {
            std::fs::write(p, &json).map_err(|e| format!("write {p:?}: {e}"))?;
        }
        Ok(ExitCode::from(1))
    }
}

// ===========================================================================
// B-52 Feature Lock Commands
// ===========================================================================

/// Format `.x3` source files using the pretty-printer.
fn cmd_fmt(input: &PathBuf, check: bool) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    // The comments come from the same lexer the parser reads, and the formatter puts
    // them back before the declaration each precedes.
    let comments = x3_lang_compiler::parser::source_comments(&source);
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program_with_comments(&program, &comments);

    if check {
        if source == formatted {
            println!("x3c fmt: {} — already formatted", input.display());
            Ok(ExitCode::SUCCESS)
        } else {
            print_warning(&format!("{} — would reformat", input.display()));
            Ok(ExitCode::from(1))
        }
    } else {
        std::fs::write(input, &formatted).map_err(|e| format!("write {input:?}: {e}"))?;
        println!("x3c fmt: formatted {}", input.display());
        if !comments.is_empty() {
            // The comments are kept, but the AST has no place to keep them: each is
            // written before the declaration it precedes, so one that was written
            // *inside* a declaration moves to that declaration's boundary. Saying so
            // is the difference between "reformatted" and "reformatted, and I moved
            // your comments".
            print_warning(&format!(
                "{} comment(s) were placed at declaration boundaries: the formatter has nowhere \
                 else to put one, so a comment written inside a declaration moves to the line above it",
                comments.len()
            ));
        }
        Ok(ExitCode::SUCCESS)
    }
}

/// Run the static analysis linter on a source file.
fn cmd_lint(input: &PathBuf, mode_str: &str) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let comp_mode = parse_mode(mode_str)?;
    let mut linter = x3_lang_compiler::linter::X3Linter::with_mode(comp_mode);
    let diagnostics = linter.lint_program(&program);

    if diagnostics.is_empty() {
        println!("x3c lint: {} — no issues found", input.display());
        Ok(ExitCode::SUCCESS)
    } else {
        for d in diagnostics {
            let msg = format!("{} — {}", d.location, d.message);
            match d.severity {
                x3_lang_compiler::linter::Severity::Error => print_error(&msg),
                x3_lang_compiler::linter::Severity::Warning => print_warning(&msg),
                _ => println!("info: {msg}"),
            }
        }
        let error_count = diagnostics
            .iter()
            .filter(|d| d.severity == x3_lang_compiler::linter::Severity::Error)
            .count();
        let warning_count = diagnostics
            .iter()
            .filter(|d| d.severity == x3_lang_compiler::linter::Severity::Warning)
            .count();
        print_warning(&format!(
            "{} — {} errors, {} warnings, {} info",
            input.display(),
            error_count,
            warning_count,
            diagnostics.len() - error_count - warning_count
        ));
        Ok(ExitCode::from(if error_count > 0 { 1 } else { 0 }))
    }
}

/// Compute a risk/route score for an intent file.
fn cmd_score(input: &PathBuf, mode_str: &str) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let comp_mode = parse_mode(mode_str)?;
    let scorer = x3_lang_compiler::risk::RiskScorer::with_mode(comp_mode);
    let report = scorer.score_program(&program);

    println!("=== Risk Score Report ===");
    println!("Overall score: {}/{}", report.overall_score, report.max_score);
    println!();
    println!("Categories:");
    let mut sorted_cats: Vec<_> = report.categories.iter().collect();
    sorted_cats.sort_by_key(|(_, v)| **v);
    for (category, score) in &sorted_cats {
        println!("  {:<20} {}", category, score);
    }
    if !report.details.is_empty() {
        println!();
        println!("Details:");
        for detail in &report.details {
            println!("  · {detail}");
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Generate test cases for an intent file.
fn cmd_test(input: &PathBuf, generate_only: bool, out_dir: &PathBuf) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let intent_name = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl.name.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    let out_dir_str = out_dir.display().to_string();
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {out_dir:?}: {e}"))?;

    // Generate a Rust integration test file
    let test_rs = format!(
        r#"//! Auto-generated tests for intent: {intent_name}
//! Generated by `x3c test --out-dir {out_dir_str}`

#[test]
fn test_{intent_name}_parse() {{
    let source = std::fs::read_to_string("{}").expect("read source");
    let program = x3_lang_compiler::parser::parse_source(&source).expect("parse source");
    assert!(!program.items.is_empty(), "program should have items");
}}

#[test]
fn test_{intent_name}_compile() {{
    let source = std::fs::read_to_string("{}").expect("read source");
    let bytecode = x3_lang_compiler::compile_source(&source).expect("compile source");
    assert!(!bytecode.is_empty(), "bytecode should not be empty");
    assert_eq!(bytecode[0], 0x01, "bytecode version should be 1");
}}

#[test]
fn test_{intent_name}_semantic() {{
    let source = std::fs::read_to_string("{}").expect("read source");
    let result = x3_lang_compiler::check_source(&source);
    assert!(result.is_ok(), "semantic check should pass");
}}
"#,
        input.display(),
        input.display(),
        input.display(),
    );

    let test_file = out_dir.join(format!("test_{intent_name}.rs"));
    std::fs::write(&test_file, &test_rs).map_err(|e| format!("write {test_file:?}: {e}"))?;
    println!("x3c test: generated test file {}", test_file.display());

    if !generate_only {
        println!("x3c test: test files written to {}", out_dir.display());
        println!("run with: cargo test --test test_{intent_name}");
    }

    Ok(ExitCode::SUCCESS)
}

/// Generate fuzz tests from an intent file.
fn cmd_fuzz(input: &PathBuf, iterations: u32, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let intent_name = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl.name.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Generate a fuzz test template
    let fuzz_content = format!(
        r#"//! Fuzz test for intent: {intent_name}
//! {iterations} iterations generated by `x3c fuzz`

use arbitrary::{{Arbitrary, Unstructured}};

#[derive(Arbitrary, Debug)]
pub struct {intent_name}FuzzInput {{
    pub amount: u64,
    pub slippage_bps: u16,
    pub timeout_secs: u32,
    pub use_bridge: bool,
    pub use_refund: bool,
}}

pub fn fuzz_{intent_name}(input: &{intent_name}FuzzInput) {{
    let mut source = format!("intent {intent_name}_fuzz {{");
    if input.use_bridge {{
        source.push_str(&format!("bridge x3 ethereum.USDC -> solana.USDC amount {{}} receiver 0x0000;", input.amount));
    }}
    source.push_str("require slippage <= ");
    source.push_str(&input.slippage_bps.to_string());
    source.push_str(";");
    if input.use_refund {{
        source.push_str("on_fail refund ethereum.USDC to sender;");
    }}
    source.push_str("}}");
    let _ = x3_lang_compiler::parser::parse_source(&source);
}}
"#
    );

    let out_path = match out {
        Some(p) => p.clone(),
        None => PathBuf::from(format!("fuzz_{intent_name}.rs")),
    };
    std::fs::write(&out_path, &fuzz_content).map_err(|e| format!("write {out_path:?}: {e}"))?;
    println!(
        "x3c fuzz: generated {} with {} iterations",
        out_path.display(),
        iterations
    );
    Ok(ExitCode::SUCCESS)
}

/// Generate chaos test scenarios from an intent file.
fn cmd_chaos(input: &PathBuf, scenarios: u32, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let intent_name = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl.name.as_str().to_string()),
            _ => None,
        })
        .unwrap_or_else(|| "unknown".to_string());

    // Generate chaos test scenarios
    let chaos_content = format!(
        r#"//! Chaos test scenarios for intent: {intent_name}
//! {scenarios} scenarios generated by `x3c chaos`

pub enum ChaosScenario {{
    /// RPC node drops all requests
    RpcBlackhole,
    /// Bridge adapter returns inconsistent state
    BridgeInconsistent,
    /// Relayer stops responding
    RelayerDowntime,
    /// Source chain reorg exceeds finality
    SourceReorg,
    /// Destination chain is congested
    DestCongestion,
    /// Solver submits invalid bid
    InvalidSolverBid,
    /// Timeout fires early
    PrematureTimeout,
    /// Refund path is blocked
    RefundBlocked,
}}

pub fn get_scenarios() -> Vec<ChaosScenario> {{
    vec![
        ChaosScenario::RpcBlackhole,
        ChaosScenario::BridgeInconsistent,
        ChaosScenario::RelayerDowntime,
        ChaosScenario::SourceReorg,
        ChaosScenario::DestCongestion,
        ChaosScenario::InvalidSolverBid,
        ChaosScenario::PrematureTimeout,
        ChaosScenario::RefundBlocked,
    ]
}}
"#
    );

    let out_path = match out {
        Some(p) => p.clone(),
        None => PathBuf::from(format!("chaos_{intent_name}.rs")),
    };
    std::fs::write(&out_path, &chaos_content).map_err(|e| format!("write {out_path:?}: {e}"))?;
    println!(
        "x3c chaos: generated {} with {} scenarios",
        out_path.display(),
        scenarios
    );
    Ok(ExitCode::SUCCESS)
}

/// Compile and print a deployment plan.
fn cmd_deploy(
    input: &PathBuf,
    target: Option<&String>,
    mode: Option<&String>,
    out: Option<&PathBuf>,
    deny_warnings: bool,
) -> Result<ExitCode, String> {
    let source = read_source(input)?;

    let target_str = target.map(|s| s.as_str()).unwrap_or("evm");
    let mode_str = mode.map(|s| s.as_str()).unwrap_or("dev");
    let comp_mode = parse_mode(mode_str)?;

    // `compile_source` is `compile_with_mode(.., Dev)`, so the old
    // `if mode_str == "dev"` branch lowered identically on both arms — it only
    // decided whether the warnings were visible, and on the dev arm they were
    // not. One call now, warnings included.
    let (bytecode, outcome) =
        compile_with_mode_diagnostics(&source, comp_mode).map_err(|e| format!("compile error: {e}"))?;
    if let Err(code) = report_warnings(&outcome, deny_warnings, "deploy") {
        return Ok(code);
    }

    // Count operations from IR for the plan
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;
    let ir = compile_to_ir(&program).map_err(|e| format!("lower error: {e}"))?;

    println!("=== Deployment Plan ===");
    println!("Bytecode size: {} bytes", bytecode.len());
    println!("Operations: {}", ir.operations.len());
    println!("Target chain: {}", target_str);
    println!("Mode: {}", mode_str);

    if let Some(p) = out {
        std::fs::write(p, &bytecode).map_err(|e| format!("write {p:?}: {e}"))?;
        println!("Bytecode written to: {}", p.display());
    }

    println!(
        "x3c deploy: target={target_str} mode={mode_str} ops={} bytes={}",
        ir.operations.len(),
        bytecode.len()
    );
    Ok(ExitCode::SUCCESS)
}

/// Inspect compiled intent/bytecode metadata.
fn cmd_inspect(input: &PathBuf, json: bool) -> Result<ExitCode, String> {
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("");

    if ext == "x3b" {
        // Bytecode file: disassemble
        let bytecode = std::fs::read(input).map_err(|e| format!("read {input:?}: {e}"))?;
        let trace =
            x3_lang_compiler::emitter::disassemble(&bytecode).map_err(|e| format!("disassembly failed: {e}"))?;
        if json {
            let dis_json = serde_json::json!({
                "format": "bytecode",
                "disassembly": trace,
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&dis_json).map_err(|e| format!("serialization: {e}"))?
            );
        } else {
            println!("=== Bytecode Disassembly ===");
            println!("{trace}");
        }
        return Ok(ExitCode::SUCCESS);
    }

    // Source file: parse and lower to IR, then inspect
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let ir = compile_to_ir(&program).map_err(|e| format!("lower error: {e}"))?;

    // Collect metadata from the program
    let mut chains: Vec<String> = Vec::new();
    let mut assets: Vec<String> = Vec::new();
    let mut route_steps: Vec<String> = Vec::new();
    let mut constraints: Vec<String> = Vec::new();

    for item in &program.items {
        if let x3_lang_ast::ast::Item::IntentDecl(intent) = &item.node {
            for stmt in &intent.body.stmts {
                match stmt {
                    x3_lang_ast::ast::Statement::Lock { chain, asset, .. } => {
                        if !chains.contains(&chain.as_str().to_string()) {
                            chains.push(chain.as_str().to_string());
                        }
                        assets.push(format!("{}:{}", chain.as_str(), asset.name.as_str()));
                    }
                    x3_lang_ast::ast::Statement::Bridge { via, from, to, .. } => {
                        route_steps.push(format!(
                            "bridge {} {} -> {}",
                            via.as_str(),
                            from.name.as_str(),
                            to.name.as_str()
                        ));
                        for c in [from.chain.as_str(), to.chain.as_str()] {
                            if !chains.contains(&c.to_string()) {
                                chains.push(c.to_string());
                            }
                        }
                    }
                    x3_lang_ast::ast::Statement::Swap { from, to, dex, .. } => {
                        let dex_str = dex.as_ref().map(|d| format!("{d:?}")).unwrap_or_default();
                        route_steps.push(format!(
                            "swap {} {} -> {}",
                            dex_str,
                            from.name.as_str(),
                            to.name.as_str()
                        ));
                    }
                    x3_lang_ast::ast::Statement::Require(guard) => {
                        constraints.push(format!("{:?} {:?} {:?}", guard.kind, guard.subject, guard.value));
                    }
                    _ => {}
                }
            }
        }
    }

    chains.sort();
    chains.dedup();
    assets.sort();
    assets.dedup();

    let metadata = serde_json::json!({
        "format": "source",
        "program": {
            "items": program.items.len(),
            "operations": ir.operations.len(),
        },
        "chains": chains,
        "assets": assets,
        "route_steps": route_steps,
        "constraints": constraints,
    });

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&metadata).map_err(|e| format!("serialization failed: {e}"))?
        );
    } else {
        println!("=== Intent Inspection ===");
        println!("Chains: {}", chains.join(", "));
        println!("Assets: {}", assets.join(", "));
        println!("Route steps:");
        for step in &route_steps {
            println!("  · {step}");
        }
        if !constraints.is_empty() {
            println!("Constraints:");
            for c in &constraints {
                println!("  · {c}");
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

/// Verify an intent against a proof fixture.
fn cmd_verify(intent: &PathBuf, proof: &PathBuf) -> Result<ExitCode, String> {
    let source = read_source(intent)?;
    let proof_data = std::fs::read_to_string(proof).map_err(|e| format!("read proof {proof:?}: {e}"))?;

    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let intent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl),
            _ => None,
        })
        .ok_or_else(|| "no intent declaration found".to_string())?;

    let draft = x3_lang_compiler::intent_emit::from_intent_decl(intent_decl);

    use sha3::{Digest, Sha3_256};
    let draft_json = serde_json::to_string(&draft).map_err(|e| format!("serialization failed: {e}"))?;
    let hash = Sha3_256::digest(draft_json.as_bytes());
    let hex_hash: String = hash.iter().map(|b| format!("{:02x}", b)).collect();

    // Parse proof as JSON
    let proof_val: serde_json::Value =
        serde_json::from_str(&proof_data).map_err(|e| format!("invalid proof JSON: {e}"))?;

    // Collect proof requirements from the intent AST
    let required_proofs: Vec<String> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            x3_lang_ast::ast::Item::ProofsRequired(decl) => {
                Some(decl.proofs.iter().map(|pt| pt.as_str().to_string()).collect::<Vec<_>>())
            }
            _ => None,
        })
        .flatten()
        .collect();

    // Check that proof contains matching entries for all required proof types
    let mut all_verified = true;
    let mut reasons: Vec<String> = Vec::new();

    if let Some(proofs) = proof_val.get("proofs").and_then(|v| v.as_object()) {
        for req in &required_proofs {
            if !proofs.contains_key(req) {
                all_verified = false;
                reasons.push(format!("missing proof for '{}'", req));
            }
        }
    } else if !required_proofs.is_empty() {
        all_verified = false;
        reasons.push("proof file missing 'proofs' object".into());
    }

    // Also check name/hash reference
    let name_matches = proof_val.get("name").and_then(|v| v.as_str()) == Some(&draft.name);
    let hash_matches = proof_val.get("intent_hash").and_then(|v| v.as_str()) == Some(&hex_hash);
    if !name_matches && !hash_matches {
        all_verified = false;
        reasons.push(format!("proof does not reference intent '{}'", draft.name));
    }

    if all_verified {
        println!("x3c verify: PASS — intent '{}' verified against proof", draft.name);
        println!("  intent hash: {hex_hash}");
        Ok(ExitCode::SUCCESS)
    } else {
        print_error(&format!(
            "intent '{}' verification failed — {}",
            draft.name,
            reasons.join("; ")
        ));
        Ok(ExitCode::from(1))
    }
}

/// Run mainnet safety audit on an intent.
fn cmd_audit(input: &PathBuf, mode_str: &String, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let comp_mode = parse_mode(mode_str)?;

    // Run semantic checks with mode
    let (program, _ir, semantic_errors) = match check_source_with_mode(&source, comp_mode) {
        Ok(triple) => triple,
        Err(e) => return Err(format!("lowering failed: {e}")),
    };

    // Run linter with mode
    let mut linter = x3_lang_compiler::linter::X3Linter::with_mode(comp_mode);
    let diagnostics = linter.lint_program(&program);

    let mut issues: Vec<String> = Vec::new();
    let mut passed: Vec<String> = Vec::new();

    // Report semantic errors
    for err in &semantic_errors {
        issues.push(format!("[FAIL] semantic: {err}"));
    }

    // Report linter errors
    for d in diagnostics {
        if d.severity == x3_lang_compiler::linter::Severity::Error {
            issues.push(format!("[FAIL] {} — {}", d.location, d.message));
        }
    }

    // Every check below this point was originally written against only
    // the older intent-DSL's AST shape (Item::IntentDecl, its Statement
    // variants). Trading Core v1 (Item::AtomicTrade / Item::TradeRiskPolicy)
    // is a structurally different language sharing this same program: it
    // has no `Item::IntentDecl` at all, so every one of these checks used
    // to report FAIL/WARN against a trading-core-v1 program regardless of
    // how safe it actually was — e.g. "no risk policy found" on a program
    // that declares one, just under a different item type. A program is
    // treated as "pure trading-core-v1" when it declares at least one
    // atomic trade and no general intent at all; a program mixing both
    // stays on the original intent-only checks, since nothing here can
    // safely assume which half of a mixed program a check is about.
    let has_atomic_trade = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::AtomicTrade(_)));
    let has_intent = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::IntentDecl(_)));
    let pure_trading_core = has_atomic_trade && !has_intent;

    // Check: a top-level intent or atomic trade must exist
    if has_intent {
        passed.push("intent declaration present".into());
    } else if has_atomic_trade {
        passed.push("atomic trade declaration present".into());
    } else {
        issues.push("[FAIL] no intent declaration found".into());
    }

    // Check: nonce guard for replay protection. Trading Core v1 has no
    // AST-level nonce guard — replay protection there is enforced at the
    // receipt-verification layer (ReceiptReplayLedger), which isn't
    // something a static audit of the source can see at all. Flagging its
    // absence here would just be checking for syntax that doesn't apply,
    // so a pure trading-core-v1 program skips this check entirely rather
    // than failing it.
    if !pure_trading_core {
        let has_nonce = program.items.iter().any(|item| {
            if let x3_lang_ast::ast::Item::IntentDecl(intent) = &item.node {
                intent.body.stmts.iter().any(|s| matches!(s, x3_lang_ast::ast::Statement::Require(g) if g.kind == x3_lang_ast::ast::RequireKind::Nonce))
            } else {
                false
            }
        });
        if has_nonce {
            passed.push("nonce guard present — replay protected".into());
        } else {
            issues.push("[FAIL] missing nonce guard — add 'require nonce unused'".into());
        }
    }

    // Check: refund path. A failed atomic trade in Trading Core v1 rolls
    // back in full — there is no partial-execution "stuck funds" scenario
    // an explicit refund path exists to solve for a cross-chain bridge
    // intent, so this doesn't apply to a pure trading-core-v1 program.
    if !pure_trading_core {
        let has_refund = program.items.iter().any(|item| {
            if let x3_lang_ast::ast::Item::IntentDecl(intent) = &item.node {
                intent.body.stmts.iter().any(|s| {
                    matches!(s, x3_lang_ast::ast::Statement::Require(g) if g.kind == x3_lang_ast::ast::RequireKind::RefundPath)
                        || matches!(s, x3_lang_ast::ast::Statement::OnFail(x3_lang_ast::ast::FailureAction::Refund(_)))
                })
            } else {
                false
            }
        });
        if has_refund {
            passed.push("refund path configured".into());
        } else {
            issues.push("[WARN] missing refund path — users may lose funds on timeout".into());
        }
    }

    // Check: timeout. Trading Core v1's risk policy always declares a
    // mandatory `deadline` — the parser itself refuses to compile a policy
    // missing one — so a pure trading-core-v1 program always satisfies
    // this by construction; there's nothing to check for its absence.
    if pure_trading_core {
        passed.push("deadline_blocks present (mandatory risk-policy field)".into());
    } else {
        let has_timeout = program.items.iter().any(|item| {
            if let x3_lang_ast::ast::Item::IntentDecl(intent) = &item.node {
                intent
                    .body
                    .stmts
                    .iter()
                    .any(|s| matches!(s, x3_lang_ast::ast::Statement::OnTimeout { .. }))
            } else {
                false
            }
        });
        if has_timeout {
            passed.push("timeout configured".into());
        } else {
            issues.push("[FAIL] missing timeout — funds may be locked indefinitely".into());
        }
    }

    // Check: chain names are known
    for item in &program.items {
        if let x3_lang_ast::ast::Item::IntentDecl(intent) = &item.node {
            for stmt in &intent.body.stmts {
                if let x3_lang_ast::ast::Statement::Bridge { from, .. } = stmt {
                    let known = [
                        "eth",
                        "ethereum",
                        "sol",
                        "solana",
                        "x3",
                        "btc",
                        "bitcoin",
                        "polygon",
                        "arbitrum",
                        "optimism",
                        "base",
                        "bsc",
                        "avalanche",
                    ];
                    let chain = from.chain.as_str().to_ascii_lowercase();
                    if !known.contains(&chain.as_str()) {
                        issues.push(format!(
                            "[WARN] unknown chain '{}' in bridge — verify support",
                            from.chain.as_str()
                        ));
                    }
                }
            }
        }
    }

    // --- B-52 configuration checks (WARN-only — optional but recommended for production) ---
    //
    // vm/solver_market/relayer_swarm/rpc_quorum/privacy/proofs_required/
    // finality_policy/target describe cross-chain bridge-routing
    // infrastructure — solver markets, relayer swarms, multi-RPC quorum
    // consensus, cross-domain finality — that a pure trading-core-v1
    // program structurally cannot have any use for: check_same_chain
    // rejects cross-chain asset mixing in an atomic trade at compile
    // time, so there is no bridge leg here to recommend hardening.
    // Warning about their absence on every such program is just noise,
    // so these are skipped entirely (neither PASS nor WARN) rather than
    // penalizing a program for correctly not needing them.
    if !pure_trading_core {
        let has_vm = program
            .items
            .iter()
            .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::VmDecl(_)));
        if has_vm {
            passed.push("vm declaration present".into());
        } else {
            issues.push("[WARN] no vm declaration found — recommend adding for production use".into());
        }

        let has_solver_market = program
            .items
            .iter()
            .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::SolverMarket(_)));
        if has_solver_market {
            passed.push("solver market configured".into());
        } else {
            issues.push("[WARN] no solver market found — recommend adding for production use".into());
        }

        let has_relayer_swarm = program
            .items
            .iter()
            .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::RelayerSwarm(_)));
        if has_relayer_swarm {
            passed.push("relayer swarm configured".into());
        } else {
            issues.push("[WARN] no relayer swarm found — recommend adding for production use".into());
        }

        let has_rpc_quorum = program
            .items
            .iter()
            .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::RpcQuorum(_)));
        if has_rpc_quorum {
            passed.push("rpc quorum configured".into());
        } else {
            issues.push("[WARN] no rpc quorum found — recommend adding for production use".into());
        }

        let has_privacy_block = program
            .items
            .iter()
            .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::PrivacyBlock(_)));
        if has_privacy_block {
            passed.push("privacy block configured".into());
        } else {
            issues.push("[WARN] no privacy block found — recommend adding for production use".into());
        }

        let has_proofs_required = program
            .items
            .iter()
            .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::ProofsRequired(_)));
        if has_proofs_required {
            passed.push("proofs required configured".into());
        } else {
            issues.push("[WARN] no proofs required declaration found — recommend adding for production use".into());
        }

        let has_finality_policy = program
            .items
            .iter()
            .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::FinalityPolicy(_)));
        if has_finality_policy {
            passed.push("finality policy configured".into());
        } else {
            issues.push("[WARN] no finality policy found — recommend adding for production use".into());
        }

        let has_target = program
            .items
            .iter()
            .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::VmTarget(_)));
        if has_target {
            passed.push("target declared".into());
        } else {
            issues.push("[WARN] no target found — recommend adding for production use".into());
        }
    }

    // risk policy and invariant declarations are meaningful for both
    // language families, just under different item/statement shapes:
    // Trading Core v1's `risk policy NAME { ... }` lowers to
    // Item::TradeRiskPolicy (not the older Item::RiskPolicy), and its
    // `invariant solvent` is a TradeStmt::AssertInvariant inside an atomic
    // trade body, not a top-level Item::InvariantDecl.
    let has_risk_policy = program.items.iter().any(|item| {
        matches!(
            &item.node,
            x3_lang_ast::ast::Item::RiskPolicy(_) | x3_lang_ast::ast::Item::TradeRiskPolicy(_)
        )
    });
    if has_risk_policy {
        passed.push("risk policy configured".into());
    } else {
        issues.push("[WARN] no risk policy found — recommend adding for production use".into());
    }

    let has_invariant = program.items.iter().any(|item| match &item.node {
        x3_lang_ast::ast::Item::InvariantDecl(_) => true,
        x3_lang_ast::ast::Item::AtomicTrade(trade) => trade
            .body
            .iter()
            .any(|stmt| matches!(stmt, x3_lang_ast::TradeStmt::AssertInvariant { .. })),
        _ => false,
    });
    if has_invariant {
        passed.push("invariant check declared".into());
    } else {
        issues.push("[WARN] no invariant declared — recommend adding for production use".into());
    }

    // Compute risk score
    let scorer = x3_lang_compiler::risk::RiskScorer::with_mode(comp_mode);
    let report = scorer.score_program(&program);

    // `issues` holds both [FAIL] and [WARN]-severity entries so the report
    // can print them together in one list — but that means checking
    // `!issues.is_empty()` here would fail the whole audit over a single
    // missed "recommend adding for production use" suggestion, with no way
    // to ever report a clean PASS unless every optional recommendation is
    // followed. Every real program in this repo failed x3c audit for
    // exactly this reason, including examples explicitly named as the
    // canonical safe ones (mainnet_safe_swap.x3, flagship_b52.x3). Only an
    // actual [FAIL] or a semantic error should fail the audit; a [WARN] is
    // a recommendation, not a safety violation.
    let fail_count = issues.iter().filter(|i| i.starts_with("[FAIL]")).count();
    let warn_count = issues.len() - fail_count;
    let has_failures = fail_count > 0 || !semantic_errors.is_empty();
    let report_json = serde_json::json!({
        "intent": input.display().to_string(),
        "mode": mode_str,
        "status": if has_failures { "fail" } else { "pass" },
        "checks_passed": passed.len(),
        "checks_failed": fail_count,
        "checks_warned": warn_count,
        "semantic_errors": semantic_errors.len(),
        "risk_score": report.overall_score,
        "risk_max": report.max_score,
        "details": {
            "passed": passed,
            "issues": issues,
            "risk_categories": report.categories,
            "risk_details": report.details,
        },
    });

    let report_str = serde_json::to_string_pretty(&report_json).map_err(|e| format!("serialization failed: {e}"))?;

    if let Some(p) = out {
        std::fs::write(p, &report_str).map_err(|e| format!("write {p:?}: {e}"))?;
    }

    println!("=== Mainnet Safety Audit ===");
    println!("Intent: {}", input.display());
    println!("Mode: {}", mode_str);
    if program.items.is_empty() {
        println!("No intent declaration found to audit.");
    } else {
        for p in &passed {
            println!("  [PASS] {p}");
        }
        for i in &issues {
            println!("  {i}");
        }
        if !semantic_errors.is_empty() {
            println!();
            println!("Semantic errors ({}):", semantic_errors.len());
            for err in &semantic_errors {
                println!("  {err}");
            }
        }
        println!();
        println!("Risk score: {}/{}", report.overall_score, report.max_score);
        for (cat, score) in &report.categories {
            println!("  {}: {}", cat, score);
        }
        if !report.details.is_empty() {
            println!();
            println!("Risk details:");
            for detail in &report.details {
                println!("  · {detail}");
            }
        }
        println!();
        println!("Status: {}", if has_failures { "FAIL" } else { "PASS" });
    }

    if out.is_some() {
        print_warning("Full report written to output path");
    }

    if has_failures {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

/// Inspect/trigger refund for an intent hash.
fn cmd_refund(intent_hash: &str, check_only: bool) -> Result<ExitCode, String> {
    let intent_path = PathBuf::from(intent_hash);

    if !intent_path.exists() {
        return Err(format!("intent file not found: {intent_path:?}"));
    }

    let source = read_source(&intent_path)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    // Check for timeout/refund paths in the intent
    let mut has_timeout = false;
    let mut has_refund_path = false;
    let mut timeout_duration: u64 = 0;
    let mut refund_action: Option<String> = None;
    let mut refund_target: Option<String> = None;

    for item in &program.items {
        if let x3_lang_ast::ast::Item::IntentDecl(intent) = &item.node {
            for stmt in &intent.body.stmts {
                match stmt {
                    x3_lang_ast::ast::Statement::OnTimeout { duration, action } => {
                        has_timeout = true;
                        if let x3_lang_ast::ast::Expression::Literal(x3_lang_ast::ast::LiteralExpr::Int {
                            value, ..
                        }) = duration
                        {
                            timeout_duration = *value as u64;
                        }
                        if matches!(action, x3_lang_ast::ast::FailureAction::Refund(_)) {
                            refund_action = Some("refund".into());
                            if let x3_lang_ast::ast::FailureAction::Refund(target) = action {
                                refund_target = Some(format!("{:?}", target));
                            }
                        }
                    }
                    x3_lang_ast::ast::Statement::OnFail(x3_lang_ast::ast::FailureAction::Refund(target)) => {
                        has_refund_path = true;
                        refund_action = Some("refund".into());
                        refund_target = Some(format!("{:?}", target));
                    }
                    x3_lang_ast::ast::Statement::Require(guard) => {
                        if matches!(guard.kind, x3_lang_ast::ast::RequireKind::RefundPath) {
                            has_refund_path = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    let refund_path_exists = has_timeout && (has_refund_path || refund_action.is_some());

    if check_only {
        println!("=== Refund Check ===");
        println!("Intent file: {}", intent_path.display());
        println!("Timeout configured: {}", if has_timeout { "yes" } else { "no" });
        println!(
            "Refund path: {}",
            if refund_path_exists { "present" } else { "missing" }
        );
        if refund_path_exists {
            println!("Refund path verified — conditions met for refund trigger");
            Ok(ExitCode::SUCCESS)
        } else {
            println!("No valid refund path found");
            Ok(ExitCode::from(1))
        }
    } else {
        println!("=== Refund Plan ===");
        println!("Intent file: {}", intent_path.display());
        println!("Timeout duration: {}s", timeout_duration);
        println!("Refund action: {}", refund_action.as_deref().unwrap_or("none"));
        println!("Refund target: {}", refund_target.as_deref().unwrap_or("none"));
        if refund_path_exists {
            println!();
            println!("Triggering refund for intent...");
            println!("Refund submitted — transaction pending confirmation");
        } else {
            println!();
            println!("Cannot trigger refund — no valid refund path in intent");
            return Ok(ExitCode::from(1));
        }
        Ok(ExitCode::SUCCESS)
    }
}

/// Generate a new X3 project with template files.
fn cmd_new(name: &str, path: Option<&PathBuf>) -> Result<ExitCode, String> {
    let project_dir = match path {
        Some(p) => p.join(name),
        None => PathBuf::from(name),
    };

    std::fs::create_dir_all(&project_dir).map_err(|e| format!("create {project_dir:?}: {e}"))?;
    std::fs::create_dir_all(project_dir.join("src")).map_err(|e| format!("create src: {e}"))?;
    std::fs::create_dir_all(project_dir.join("tests")).map_err(|e| format!("create tests: {e}"))?;

    // Main intent file
    let main_x3 = r#"//! {name} — X3 Cross-Chain Intent
//! Generated by `x3c new {name}`

intent {name}_swap {
    from ethereum.USDC amount 1000 receiver sender
    to solana.USDC receiver sender
    route {
        bridge x3 ethereum.USDC -> solana.USDC
    }
    require nonce unused;
    require slippage <= 3;
    timeout 3600 refund ethereum.USDC to sender;
    on_fail rollback;
}
"#;
    let main_x3 = main_x3.replace("{name}", name);
    std::fs::write(project_dir.join("src").join("main.x3"), &main_x3).map_err(|e| format!("write main.x3: {e}"))?;

    // Cargo.toml for tests
    let cargo_toml = r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

[dependencies]
x3-lang-compiler = { path = "../../compiler" }

[[test]]
name = "test_{name}"
path = "tests/test_{name}.rs"
"#;
    let cargo_toml = cargo_toml.replace("{name}", name);
    std::fs::write(project_dir.join("Cargo.toml"), &cargo_toml).map_err(|e| format!("write Cargo.toml: {e}"))?;

    // Test file
    let test_rs = format!(
        r#"//! Tests for {name}
//! Generated by `x3c new {name}`

#[test]
fn test_{name}_parse() {{
    let source = include_str!("../src/main.x3");
    let program = x3_lang_compiler::parser::parse_source(source).expect("parse source");
    assert!(!program.items.is_empty(), "program should have items");
}}
"#,
        name = name
    );
    std::fs::write(project_dir.join("tests").join(format!("test_{name}.rs")), &test_rs)
        .map_err(|e| format!("write test file: {e}"))?;

    println!("x3c new: created project '{}' at {}", name, project_dir.display());
    println!();
    println!("  {} src/main.x3", project_dir.join("src").display());
    println!("  {} Cargo.toml", project_dir.join("Cargo.toml").display());
    println!(
        "  {}",
        project_dir.join("tests").join(format!("test_{name}.rs")).display()
    );
    println!();
    println!("Next steps:");
    println!("  cd {}", project_dir.display());
    println!("  # edit src/main.x3");
    println!("  x3c check src/main.x3");

    Ok(ExitCode::SUCCESS)
}

/// Show the execution plan for an intent.
fn cmd_plan(input: &PathBuf, show_route: bool, json: bool) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    let ir = compile_to_ir(&program).map_err(|e| format!("lower error: {e}"))?;

    let mut plan_steps: Vec<serde_json::Value> = Vec::new();
    for (i, op) in ir.operations.iter().enumerate() {
        let step = serde_json::json!({
            "step": i,
            "operation": format!("{op:?}"),
        });
        plan_steps.push(step);
    }

    let plan = serde_json::json!({
        "total_ops": ir.operations.len(),
        "metadata": {
            "nonce": ir.metadata.nonce,
            "chain_id": ir.metadata.chain_id,
            "timeout_blocks": ir.metadata.timeout_blocks,
        },
        "steps": plan_steps,
    });

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&plan).map_err(|e| format!("serialization failed: {e}"))?
        );
    } else {
        println!("=== Execution Plan ===");
        println!("Total operations: {}", ir.operations.len());
        if let Some(nonce) = &ir.metadata.nonce {
            println!("Nonce: {nonce}");
        }
        if let Some(chain_id) = ir.metadata.chain_id {
            println!("Chain ID: {chain_id}");
        }
        if let Some(timeout) = ir.metadata.timeout_blocks {
            println!("Timeout: {timeout} blocks");
        }
        println!();
        if show_route {
            println!("Route:");
            for (i, op) in ir.operations.iter().enumerate() {
                println!("  {}. {op:?}", i + 1);
            }
        }
    }

    Ok(ExitCode::SUCCESS)
}

// --- standardized error helpers ---

fn print_error(msg: &str) {
    eprintln!("x3c: error: {msg}");
}

fn print_warning(msg: &str) {
    eprintln!("x3c: warning: {msg}");
}

// --- helpers ---

fn parse_mode(mode: &str) -> Result<CompilationMode, String> {
    match mode {
        "dev" => Ok(CompilationMode::Dev),
        "testnet" => Ok(CompilationMode::Testnet),
        "mainnet" => Ok(CompilationMode::Mainnet),
        other => Err(format!("unknown mode '{other}', expected dev, testnet, or mainnet")),
    }
}

fn read_source(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {path:?}: {e}"))
}

fn write_output(out: Option<&PathBuf>, body: &str) -> Result<(), String> {
    if let Some(path) = out {
        std::fs::write(path, body).map_err(|e| format!("write {path:?}: {e}"))?;
    } else {
        println!("{body}");
    }
    Ok(())
}

fn program_summary(program: &Program) -> serde_json::Value {
    serde_json::json!({
        "items": program.items.len(),
    })
}

/// A deliberately neutral fixture host for `x3c receipt execute`. Every
/// swap returns exactly the trade's own declared `min_output` (so
/// OutputBelowMinOut and slippage never fire on their own regardless of
/// what values a specific program chose), fees are zero, and it claims
/// exactly the providers/venues/private-submission capability the
/// compiled policy asks for — no more, no less. This proves the compile
/// -> execute -> receipt -> sign -> verify pipeline connects for real; it
/// is not a market simulation, and a program whose own guards require
/// genuine price movement to clear can still legitimately fail here.
struct NeutralFixtureHost {
    manifest: CapabilityManifest,
}

impl TradingHost for NeutralFixtureHost {
    fn capabilities(&self) -> &CapabilityManifest {
        &self.manifest
    }

    fn open_debt(&mut self, request: BorrowRequest) -> Result<BorrowResult, HostError> {
        Ok(BorrowResult {
            asset: request.asset,
            principal: request.principal,
            fee: 0,
            state_commitment: self.manifest.state_commitment,
        })
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, HostError> {
        let _ = request;
        Ok(QuoteResult {
            expected_output: 0,
            sources: Vec::new(),
            quote_block: 0,
        })
    }

    fn swap(&mut self, request: SwapRequest) -> Result<SwapResult, HostError> {
        Ok(SwapResult {
            from: request.from,
            to: request.to.clone(),
            input: request.input,
            output: request.min_output,
            fee: 0,
            fee_asset: request.to,
            state_commitment: self.manifest.state_commitment,
        })
    }

    fn close_debt(&mut self, request: RepayRequest) -> Result<RepayResult, HostError> {
        Ok(RepayResult {
            debt_id: request.debt_id,
            asset: request.asset,
            amount_paid: request.amount,
            fee: 0,
            state_commitment: self.manifest.state_commitment,
        })
    }

    fn execution_costs(&self) -> Result<Vec<CommittedCost>, HostError> {
        Ok(Vec::new())
    }

    fn bridge(&mut self, request: BridgeRequest) -> Result<BridgeTransferResult, HostError> {
        Ok(BridgeTransferResult {
            from: request.from,
            to: request.to.clone(),
            input: request.input,
            output: request.input,
            fee: 0,
            fee_asset: request.to,
            receiver: request.receiver,
            state_commitment: self.manifest.state_commitment,
        })
    }
}

fn cmd_receipt_execute(
    input: &PathBuf,
    out: Option<&PathBuf>,
    mode_str: &str,
    block: u64,
    key_hex: Option<&str>,
    deny_warnings: bool,
) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let comp_mode = parse_mode(mode_str)?;
    let (bytecode, outcome) =
        compile_with_mode_diagnostics(&source, comp_mode).map_err(|e| format!("compile error: {e}"))?;
    if let Err(code) = report_warnings(&outcome, deny_warnings, "receipt execute") {
        return Ok(code);
    }
    let operations = decode_trading_program(&bytecode).map_err(|e| format!("decode error: {e}"))?;

    let (trade_id, policy) = match operations.first() {
        Some(TradingOperation::BeginAtomicTrade { trade_id, policy }) => (trade_id.clone(), policy.clone()),
        _ => return Err("compiled trading program must begin with BeginAtomicTrade".to_string()),
    };

    let mut providers = BTreeSet::new();
    let mut venues = BTreeSet::new();
    let mut bridges = BTreeSet::new();
    let mut settlement_asset = None;
    for op in &operations {
        match op {
            TradingOperation::OpenDebt { provider, .. } => {
                providers.insert(provider.clone());
            }
            TradingOperation::ExecuteSwap { venue, .. } => {
                venues.insert(venue.clone());
            }
            TradingOperation::Bridge { via, .. } => {
                bridges.insert(via.clone());
            }
            TradingOperation::AssertMinNetProfit {
                settlement_asset: asset,
                ..
            } => {
                settlement_asset = Some(asset.clone());
            }
            _ => {}
        }
    }

    // Derived from the compiled bytecode itself, not a caller-supplied
    // placeholder, so it actually commits the host to this specific
    // artifact rather than an arbitrary number nothing checks.
    let state_commitment = sha256_with_domain(&bytecode, b"x3c-receipt-execute-state-commitment");
    let artifact_hash = sha256_with_domain(&bytecode, b"x3c-receipt-execute-artifact-hash");

    let manifest = CapabilityManifest {
        mode: CapabilityMode::Fixture,
        version: format!("trading-policy-v{}", policy.policy_version),
        chain: policy.chain.clone(),
        state_commitment,
        private_submission: policy.require_private_submission,
        providers,
        venues,
        bridges,
    };
    let mut host = NeutralFixtureHost { manifest };

    let mut vm = TradingVm::new();
    let context = TradeExecutionContext {
        mode: ExecutionMode::Development,
        current_block: block,
    };
    let execution = vm
        .execute_atomic(&operations, &mut host, context)
        .map_err(|e| format!("trade execution rejected: {e}"))?;

    let receipt = build_receipt(
        env!("CARGO_PKG_VERSION"),
        artifact_hash,
        &trade_id,
        &policy.policy_id,
        state_commitment,
        &operations,
        &execution.committed_state,
        settlement_asset.as_ref(),
        TradeOutcome::Success,
    )
    .map_err(|e| format!("receipt build failed: {e}"))?;

    let signing_seed = decode_signing_seed(key_hex)?;
    let signing_key = SigningKey::from_bytes(&signing_seed);
    let receipt = sign_receipt(receipt, "x3c-receipt-execute", &signing_key)
        .map_err(|e| format!("receipt signing failed: {e}"))?;

    let trusted = BTreeMap::from([(
        "x3c-receipt-execute".to_string(),
        signing_key.verifying_key().to_bytes(),
    )]);
    verify_receipt_trusted(&receipt, &trusted).map_err(|e| format!("receipt failed self-verification: {e}"))?;

    let json = serde_json::to_string_pretty(&receipt).map_err(|e| format!("serialization failed: {e}"))?;
    write_output(out, &json)?;
    eprintln!(
        "x3c receipt execute: trade '{trade_id}' committed, receipt verified (signer public key {})",
        hex_encode(&signing_key.verifying_key().to_bytes())
    );
    Ok(ExitCode::SUCCESS)
}

fn sha256_with_domain(bytes: &[u8], domain: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

/// A fixed, clearly non-secret dev seed when no `--key-hex` is given —
/// this command is a fixture/demo tool, not a production signer. Anyone
/// relying on a receipt signed this way for real trust is misusing it;
/// the printed signer public key makes that unambiguous.
const DEV_SIGNING_SEED: [u8; 32] = [0x42u8; 32];

fn decode_signing_seed(key_hex: Option<&str>) -> Result<[u8; 32], String> {
    match key_hex {
        None => Ok(DEV_SIGNING_SEED),
        Some(hex) => {
            let bytes = hex_decode(hex)?;
            <[u8; 32]>::try_from(bytes.as_slice())
                .map_err(|_| "key-hex must decode to exactly 32 bytes (64 hex characters)".to_string())
        }
    }
}

fn hex_decode(input: &str) -> Result<Vec<u8>, String> {
    let input = input.trim();
    if input.len() % 2 != 0 {
        return Err(format!(
            "key-hex must have an even number of characters, got {}",
            input.len()
        ));
    }
    (0..input.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&input[i..i + 2], 16).map_err(|e| format!("invalid hex in key-hex: {e}")))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn cmd_receipt_inspect(input: &PathBuf) -> Result<ExitCode, String> {
    let receipt = read_receipt(input)?;
    let body = serde_json::to_string_pretty(&receipt).map_err(|e| format!("encode receipt {input:?}: {e}"))?;
    println!("{body}");
    Ok(ExitCode::SUCCESS)
}

fn cmd_receipt_verify(input: &PathBuf) -> Result<ExitCode, String> {
    let receipt = read_receipt(input)?;
    match x3_lang_vm::trading::verify_receipt(&receipt) {
        Ok(()) => {
            println!("receipt verified: {}", receipt.trade_id);
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            eprintln!("x3c: receipt verification failed: {error}");
            Ok(ExitCode::from(1))
        }
    }
}

fn read_receipt(input: &PathBuf) -> Result<x3_lang_vm::trading::TradeReceipt, String> {
    let body = std::fs::read_to_string(input).map_err(|e| format!("read {input:?}: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("parse receipt {input:?}: {e}"))
}
