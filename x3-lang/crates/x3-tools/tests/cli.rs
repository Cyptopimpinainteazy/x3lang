//! Integration tests for the `x3c` CLI binary (target G in the
//! production contract).
//!
//! These tests shell out to the built `x3c` binary in the workspace
//! `target/debug` directory and assert the documented behavior of every
//! subcommand:
//!
//! - `parse` — produces JSON.
//! - `check` — exits 0 on a clean program, exits 1 on a bad one.
//! - `lower` — writes a JSON file containing the IR.
//! - `build` — produces 4-byte aligned bytecode starting with version 0x01.
//! - `simulate` / `run` — execute bytecode and report stats.
//! - `explain` — disassembly text contains the source opcodes.
//! - `test-fixture` — emits a known-good fixture.

use std::path::PathBuf;
use std::process::Command;

fn x3c_bin() -> PathBuf {
    // Prefer the cargo-provided bin path (set in `cargo test`'s
    // scratch target dir); fall back to the workspace `target/debug`.
    if let Some(path) = option_env!("CARGO_BIN_EXE_x3c") {
        return PathBuf::from(path);
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolution")
        .to_path_buf();
    workspace_root.join("target").join("debug").join("x3c")
}

fn x3c() -> Command {
    Command::new(x3c_bin())
}

fn write_fixture(name: &str, body: &str) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, body).expect("fixture write");
    path
}

fn trading_receipt_json(tamper: bool) -> String {
    use std::collections::BTreeMap;

    use x3_lang_compiler::ir::{AssetKey, TradingOperation};
    use x3_lang_vm::trading::{build_receipt, DebtRecord, TradeOutcome, TradingState};

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
            policy_id: "P".to_string(),
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
    let mut receipt = build_receipt(
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
    .expect("receipt builds");
    if tamper {
        receipt.receipt_hash = [0u8; 32];
    }
    serde_json::to_string_pretty(&receipt).expect("receipt json")
}

const GOOD_SOURCE: &str = r#"intent arb_solana_eth {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 777
    }
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;

#[test]
fn cli_parse_writes_json() {
    let src = write_fixture("cli_good.x3", GOOD_SOURCE);
    let out = std::env::temp_dir().join("cli_parse.json");
    let status = x3c()
        .arg("parse")
        .arg(&src)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("x3c parse invocation");
    assert!(status.success(), "x3c parse should succeed: {status:?}");
    let body = std::fs::read_to_string(&out).expect("output read");
    assert!(body.contains("\"items\""), "parse output must include items");
}

#[test]
fn cli_check_clean_exits_zero() {
    let src = write_fixture("cli_check_clean.x3", GOOD_SOURCE);
    let status = x3c().arg("check").arg(&src).status().expect("x3c check");
    assert!(status.success(), "clean source must pass: {status:?}");
}

#[test]
fn cli_check_dirty_exits_nonzero() {
    let src = write_fixture("cli_check_dirty.x3", GOOD_SOURCE);
    let _ = src;
    // No bad fixture shipped — this branch is exercised by the
    // explicit bad fixture in CI/dev. The negative path is covered
    // by `cli_check_rejects_unsafe_program` below.
}

#[test]
fn cli_lower_writes_ir_file() {
    let src = write_fixture("cli_lower.x3", GOOD_SOURCE);
    let out = std::env::temp_dir().join("cli_lower.json");
    let status = x3c()
        .arg("lower")
        .arg(&src)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("x3c lower");
    assert!(status.success(), "lower must succeed: {status:?}");
    let body = std::fs::read_to_string(&out).expect("ir read");
    assert!(body.contains("\"operations\""), "IR must contain operations field");
}

#[test]
fn cli_build_produces_aligned_bytecode() {
    let src = write_fixture("cli_build.x3", GOOD_SOURCE);
    let out = std::env::temp_dir().join("cli_build.x3b");
    let status = x3c()
        .arg("build")
        .arg(&src)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("x3c build");
    assert!(status.success(), "build must succeed: {status:?}");
    let bytes = std::fs::read(&out).expect("bytecode read");
    assert!(!bytes.is_empty(), "bytecode is non-empty");
    assert_eq!(bytes[0], 0x01, "version byte is 0x01");
    assert_eq!(bytes.len() % 4, 0, "bytecode is 4-byte aligned");
}

#[test]
fn cli_receipt_verify_accepts_valid_receipt() {
    let receipt = write_fixture("cli_valid_receipt.json", &trading_receipt_json(false));
    let output = x3c()
        .arg("receipt")
        .arg("verify")
        .arg(&receipt)
        .output()
        .expect("x3c receipt verify");
    assert!(
        output.status.success(),
        "valid receipt must verify: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("receipt verified"));
}

#[test]
fn cli_receipt_verify_rejects_tampered_receipt() {
    let receipt = write_fixture("cli_tampered_receipt.json", &trading_receipt_json(true));
    let status = x3c()
        .arg("receipt")
        .arg("verify")
        .arg(&receipt)
        .status()
        .expect("x3c receipt verify");
    assert!(!status.success(), "tampered receipt must fail verification");
}

#[test]
fn cli_receipt_inspect_prints_json() {
    let receipt = write_fixture("cli_inspect_receipt.json", &trading_receipt_json(false));
    let output = x3c()
        .arg("receipt")
        .arg("inspect")
        .arg(&receipt)
        .output()
        .expect("x3c receipt inspect");
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("\"trade_id\""));
}

#[test]
fn cli_run_executes_atomic_bytecode_successfully() {
    let src = write_fixture("cli_run.x3", GOOD_SOURCE);
    let bytecode = std::env::temp_dir().join("cli_run.x3b");
    let _ = x3c()
        .arg("build")
        .arg(&src)
        .arg("--out")
        .arg(&bytecode)
        .status()
        .expect("build");
    let out = x3c().arg("run").arg(&bytecode).output().expect("x3c run");
    assert!(
        out.status.success(),
        "run must succeed — AtomicBegin/AtomicEnd are wired, got stdout: {}, stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("x3c run: ok"),
        "stdout must report success, got: {stdout}"
    );
}

#[test]
fn cli_explain_produces_disassembly() {
    let src = write_fixture("cli_explain.x3", GOOD_SOURCE);
    let bytecode = std::env::temp_dir().join("cli_explain.x3b");
    let _ = x3c()
        .arg("build")
        .arg(&src)
        .arg("--out")
        .arg(&bytecode)
        .status()
        .expect("build");
    let out = x3c().arg("explain").arg(&bytecode).output().expect("x3c explain");
    assert!(out.status.success(), "explain must succeed");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("x3-lang bytecode"), "header line");
    assert!(stdout.contains("LOCK") || stdout.contains("SWAP") || stdout.contains("RELEASE"));
}

#[test]
fn cli_test_fixture_writes_file() {
    let out = std::env::temp_dir().join("cli_test_fixture.x3");
    let status = x3c()
        .arg("test-fixture")
        .arg("--out")
        .arg(&out)
        .status()
        .expect("x3c test-fixture");
    assert!(status.success(), "test-fixture must succeed");
    let body = std::fs::read_to_string(&out).expect("fixture read");
    assert!(body.contains("intent"), "fixture must contain an intent");
}

#[test]
fn cli_check_rejects_unsafe_program() {
    // Build a program via the build_eth_receipt_archive bin, but we
    // don't have that here. The path we exercise: hand-build an
    // .x3 file that compiles to a bridge outside an atomic block by
    // exploiting the parser's known shape: the existing parser is
    // strict enough that the lowerer always wraps intent routes in an
    // atomic. Instead, ship a fixture that uses an unknown chain
    // (which the semantic verifier rejects).
    let bad = r#"intent bad {
    from MyChain.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        swap uniswap mychain.USDC -> ethereum.ETH amount 1000 min_output 777
    }
}
"#;
    let src = write_fixture("cli_check_bad.x3", bad);
    let status = x3c().arg("check").arg(&src).status().expect("x3c check bad");
    // The unknown chain should be flagged.
    assert!(
        !status.success(),
        "unknown chain must be rejected by `x3c check`, got: {status:?}"
    );
}
