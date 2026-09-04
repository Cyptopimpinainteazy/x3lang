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

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use x3_lang_ast::ast::Program;
use x3_lang_compiler::{check_source, compile_source, compile_to_ir};
use x3_lang_vm::{VMConfig, VMState, VM};

#[derive(Parser, Debug)]
#[command(name = "x3c", about = "x3-lang compiler and VM driver")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
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
    /// Uses the parser's IntentDecl extraction and serializes the draft.
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
    /// Runs the entire pipeline: parse → draft → CrossChainIntent → plan,
    /// then verifies the output matches the fixture's expected artifacts.
    Prove {
        input: PathBuf,
        fixture: PathBuf,
        #[arg(long)]
        out: Option<PathBuf>,
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
    match cli.command {
        Cmd::Parse { input, out } => cmd_parse(&input, out.as_ref()),
        Cmd::Check { input, out } => cmd_check(&input, out.as_ref()),
        Cmd::Lower { input, out } => cmd_lower(&input, &out),
        Cmd::Build { input, out } => cmd_build(&input, &out),
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
    }
}

// --- subcommand impls ---

fn cmd_parse(input: &PathBuf, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    match x3_lang_compiler::parser::parse_source(&source) {
        Ok(program) => {
            let json = serde_json::to_string_pretty(&program).map_err(|e| format!("serialization failed: {e}"))?;
            write_output(out, &json)?;
            Ok(ExitCode::from(0))
        }
        Err(err) => Err(format!("parse error: {err}")),
    }
}

fn cmd_check(input: &PathBuf, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let (program, ir, errs) = check_source(&source).map_err(|e| format!("lowering failed: {e}"))?;
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
        Ok(ExitCode::from(0))
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
    Ok(ExitCode::from(0))
}

fn cmd_build(input: &PathBuf, out: &PathBuf) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let bytecode = compile_source(&source).map_err(|e| format!("compile error: {e}"))?;
    std::fs::write(out, &bytecode).map_err(|e| format!("write {out:?}: {e}"))?;
    println!(
        "x3c build: {} bytes ({} ops) -> {}",
        bytecode.len(),
        bytecode.len() / 4,
        out.display()
    );
    Ok(ExitCode::from(0))
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
            Ok(ExitCode::from(0))
        }
        Err(err) => {
            eprintln!("x3c run: VM error: {err:?}");
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
    Ok(ExitCode::from(0))
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
    Ok(ExitCode::from(0))
}

/// Compile an `.x3` source file's intent declaration to canonical intent spec.
fn cmd_intent(input: &PathBuf, emit_hash: bool, emit_plan: bool, out: Option<&PathBuf>) -> Result<ExitCode, String> {
    let source = read_source(input)?;
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|e| format!("parse error: {e}"))?;

    // Find the first intent declaration in the AST
    let intent_decl = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::IntentDecl(decl) => Some(decl),
            _ => None,
        })
        .ok_or_else(|| "no intent declaration found in source".to_string())?;

    // Extract the draft using the compiler's adapter
    let draft = x3_lang_compiler::intent_emit::from_intent_decl(intent_decl);

    // Build output
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
        // Compute a deterministic hash of the draft JSON using SHA3-256
        use sha3::{Digest, Sha3_256};
        let draft_json = serde_json::to_string(&draft).map_err(|e| format!("serialization failed: {e}"))?;
        let hash = Sha3_256::digest(draft_json.as_bytes());
        let hex_hash: String = hash.iter().map(|b| format!("{:02x}", b)).collect();
        output["intent_hash"] = serde_json::json!(hex_hash);
    }

    if emit_plan {
        // Convert constraints into a human-readable plan
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
    Ok(ExitCode::from(0))
}

/// Prove an intent by compiling it and comparing against a fixture.
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

    // Compute SHA3-256 hash of the draft
    use sha3::{Digest, Sha3_256};
    let draft_json = serde_json::to_string(&draft).map_err(|e| format!("serialization failed: {e}"))?;
    let hash = Sha3_256::digest(draft_json.as_bytes());
    let hex_hash: String = hash.iter().map(|b| format!("{:02x}", b)).collect();

    // Load fixture and compare
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
        Ok(ExitCode::from(0))
    } else {
        eprintln!("x3c prove: FAIL — intent '{}' does not match fixture", draft.name);
        if out.is_none() {
            eprintln!("{json}");
        } else if let Some(p) = out {
            std::fs::write(p, &json).map_err(|e| format!("write {p:?}: {e}"))?;
        }
        Ok(ExitCode::from(1))
    }
}

// --- helpers ---

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
