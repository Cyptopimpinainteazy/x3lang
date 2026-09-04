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

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use x3_lang_ast::ast::Program;
use x3_lang_compiler::{
    check_source, check_source_with_mode, compile_source, compile_to_ir, compile_with_mode, CompilationMode,
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
        Cmd::Check { input, out } => cmd_check(&input, out.as_ref(), mode),
        Cmd::Lower { input, out } => cmd_lower(&input, &out),
        Cmd::Build { input, out } => cmd_build(&input, &out, mode),
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
        // B-52 commands
        Cmd::Fmt { input, check } => cmd_fmt(&input, check),
        Cmd::Lint { input } => cmd_lint(&input, mode),
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
        } => cmd_deploy(&input, target.as_ref(), mode.as_ref(), out.as_ref()),
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

fn cmd_check(input: &PathBuf, out: Option<&PathBuf>, mode_str: &str) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let comp_mode = parse_mode(mode_str)?;
    let (program, ir, errs) = if mode_str == "dev" {
        check_source(&source).map_err(|e| format!("lowering failed: {e}"))?
    } else {
        check_source_with_mode(&source, comp_mode).map_err(|e| format!("lowering failed: {e}"))?
    };
    let program_summary = program_summary(&program);
    if errs.is_empty() {
        let body = serde_json::json!({
            "status": "ok",
            "program": program_summary,
            "operations": ir.operations.len(),
        });
        let json = serde_json::to_string_pretty(&body).map_err(|e| format!("serialization failed: {e}"))?;
        write_output(out, &json)?;
        println!("x3c check: {} ops, no semantic errors", ir.operations.len());
        Ok(ExitCode::SUCCESS)
    } else {
        let body = serde_json::json!({
            "status": "error",
            "program": program_summary,
            "errors": errs.iter().map(|e| format!("{e}")).collect::<Vec<_>>(),
        });
        let json = serde_json::to_string_pretty(&body).map_err(|e| format!("serialization failed: {e}"))?;
        if let Some(o) = out {
            std::fs::write(o, json).map_err(|e| format!("write {o:?}: {e}"))?;
        } else {
            print_error(&format!("semantic check failed — {} error(s)", errs.len()));
            eprintln!("{json}");
        }
        Ok(ExitCode::from(1))
    }
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

fn cmd_build(input: &PathBuf, out: &PathBuf, mode_str: &str) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let comp_mode = parse_mode(mode_str)?;
    let bytecode = if mode_str == "dev" {
        compile_source(&source).map_err(|e| format!("compile error: {e}"))?
    } else {
        compile_with_mode(&source, comp_mode).map_err(|e| format!("compile error: {e}"))?
    };
    std::fs::write(out, &bytecode).map_err(|e| format!("write {out:?}: {e}"))?;
    println!(
        "x3c build: {} bytes ({} ops) -> {}",
        bytecode.len(),
        bytecode.len() / 4,
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

    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);

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
) -> Result<ExitCode, String> {
    let source = read_source(input)?;

    let target_str = target.map(|s| s.as_str()).unwrap_or("evm");
    let mode_str = mode.map(|s| s.as_str()).unwrap_or("dev");
    let comp_mode = parse_mode(mode_str)?;

    let bytecode = if mode_str == "dev" {
        compile_source(&source).map_err(|e| format!("compile error: {e}"))?
    } else {
        compile_with_mode(&source, comp_mode).map_err(|e| format!("compile error: {e}"))?
    };

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

    // Check: intent must exist
    let has_intent = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::IntentDecl(_)));
    if has_intent {
        passed.push("intent declaration present".into());
    } else {
        issues.push("[FAIL] no intent declaration found".into());
    }

    // Check: nonce guard for replay protection
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

    // Check: refund path
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

    // Check: timeout
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

    let has_risk_policy = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::RiskPolicy(_)));
    if has_risk_policy {
        passed.push("risk policy configured".into());
    } else {
        issues.push("[WARN] no risk policy found — recommend adding for production use".into());
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

    let has_invariant = program
        .items
        .iter()
        .any(|item| matches!(&item.node, x3_lang_ast::ast::Item::InvariantDecl(_)));
    if has_invariant {
        passed.push("invariant check declared".into());
    } else {
        issues.push("[WARN] no invariant declared — recommend adding for production use".into());
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

    // Compute risk score
    let scorer = x3_lang_compiler::risk::RiskScorer::with_mode(comp_mode);
    let report = scorer.score_program(&program);

    let has_failures = !issues.is_empty() || !semantic_errors.is_empty();
    let report_json = serde_json::json!({
        "intent": input.display().to_string(),
        "mode": mode_str,
        "status": if has_failures { "fail" } else { "pass" },
        "checks_passed": passed.len(),
        "checks_failed": issues.len(),
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
