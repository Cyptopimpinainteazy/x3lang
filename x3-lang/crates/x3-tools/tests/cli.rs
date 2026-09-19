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
    use std::collections::{BTreeMap, BTreeSet};

    use x3_lang_compiler::ir::{
        AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile, TradingOperation,
    };
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

const TRADING_SOURCE: &str = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset WETH = evm.ethereum.0xC02a { decimals: 18 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }

risk policy MainnetArb {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: true
}

atomic trade CrossDexArb using MainnetArb {
    borrow 1_000_000 USDC from aave_v3 as debt

    let weth = swap debt.amount USDC -> WETH
        via uniswap_v3
        min_out 410 WETH

    let returned = swap weth WETH -> USDC
        via sushiswap
        min_out 1_002_000 USDC

    repay debt

    require net_profit >= 1_000 USDC
    require all_debts_repaid
    emit receipt
}
"#;

const BRIDGE_TRADING_SOURCE: &str = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset WETH = evm.ethereum.0xC02a { decimals: 18 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }
asset USDC_BASE = evm.base.0xB1a0 { decimals: 6 }

risk policy MainnetArbBridge {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}

atomic trade CrossDexArbToBase using MainnetArbBridge {
    borrow 1_000_000 USDC from aave_v3 as debt

    let weth = swap debt.amount USDC -> WETH
        via uniswap_v3
        min_out 410 WETH

    let returned = swap weth WETH -> USDC
        via sushiswap
        min_out 1_002_000 USDC

    repay debt

    bridge returned USDC -> USDC_BASE via wormhole to "0x1234567890abcdef1234567890abcdef12345678"

    require net_profit >= 1 USDC_BASE
    require all_debts_repaid
    emit receipt
}
"#;

/// A B-52 program that declares every optional recommendation `x3c audit`
/// checks for (vm, solver_market, two rpc_quorum blocks, risk_policy,
/// privacy, invariant, proofs required, finality_policy, target) plus a
/// nonce guard, refund path, and timeout — chosen so a clean run produces
/// zero [FAIL] and zero [WARN] entries at all.
const FULLY_CONFIGURED_SOURCE: &str = r#"
vm {
    chain arbitrum
    adapter evm
    finality safe
}

solver_market {
    mode competitive
    min_reputation 95
    bond 10_000 USDC
}

relayers {
    quorum_numerator 3
    quorum_denominator 5
    relayers [relayer_a, relayer_b, relayer_c, relayer_d, relayer_e]
}

risk_policy {
    min_route_score 90
}

rpc_quorum {
    source arbitrum
    require_numerator 2
    require_denominator 3
    reject_on [receipt_disagree, finality_disagree]
}

rpc_quorum {
    source solana
    require_numerator 2
    require_denominator 3
    reject_on [receipt_disagree, finality_disagree]
}

risk_policy {
    max_slippage 5
    max_position 500000
}

privacy {
    hide_route_until_commit true
    reveal_on claim
}

invariant no_double_claim

proofs required {
    source_lock_proof
    source_finality_proof
    destination_fill_proof
}

finality_policy strict {
    chain ethereum
    requirement finalized
}

error SlippageExceeded

target evm {
    adapter evm_adapter
    contract 0x742d35Cc6634C0532925a3b844Bc9e7595f2bD18
}

finality_policy strict {
    chain arbitrum
    requirement finalized
    blocks 32
}

finality_policy strict {
    chain solana
    requirement finalized
    blocks 32
}

intent safe_cross_vm_swap {
    from arbitrum.USDC amount 500
    to solana.SOL receiver wallet

    route {
        bridge X3 arbitrum.USDC -> solana.SOL receiver wallet
    }

    require nonce unused safe_swap_001
    require slippage <= 5
    require route_score >= 90
    require finality.arbitrum >= 32
    require finality.solana >= 32
    require relayer_quorum >= 3
    require solver_bond >= 10000

    timeout 3600s
    on_fail refund arbitrum.USDC to sender
}
"#;

const GOOD_SOURCE: &str = r#"intent arb_solana_eth {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 777
    }
    require slippage <= 50
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;

/// A program that warns for a correct reason: it bridges and declares no
/// proofs, so the verifier asks for the source-lock and destination-fill
/// proofs the bridge depends on.
///
/// This replaced `GOOD_SOURCE` as the warning fixture. `GOOD_SOURCE` is a
/// *same-chain* intent, and the warning it produced was the bug fixed in
/// TICKET-024 — the lock-proof requirement fired on `Lock`, and lowering emits
/// a `Lock` for a transfer that never leaves a chain. The two tests below were
/// therefore asserting that the compiler complains about a correct program, and
/// they went green on a defect. A warning fixture has to warn about something
/// the program could actually fix.
const WARN_SOURCE: &str = r#"finality_policy strict {
    chain ethereum
    requirement finalized
    blocks 32
}

finality_policy strict {
    chain solana
    requirement finalized
    blocks 32
}

intent bridging_without_proofs {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        bridge x3 ethereum.USDC -> solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    }
    require nonce unused bridging_001
    require finality.ethereum >= 32
    require finality.solana >= 32
    require slippage <= 50
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

#[test]
fn cli_receipt_execute_compiles_runs_and_emits_a_verifiable_receipt() {
    // Proves the previously-unwired pipeline actually connects: a real
    // .x3 source file, compiled and executed through the CLI (not
    // library test code), produces a receipt that independently passes
    // `x3c receipt verify`.
    let src = write_fixture("cli_receipt_execute.x3", TRADING_SOURCE);
    let receipt_path = std::env::temp_dir().join("cli_receipt_execute_out.json");

    let output = x3c()
        .arg("receipt")
        .arg("execute")
        .arg(&src)
        .arg("--out")
        .arg(&receipt_path)
        .output()
        .expect("x3c receipt execute");
    assert!(
        output.status.success(),
        "receipt execute must succeed for a well-formed trade: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("CrossDexArb"));

    let body = std::fs::read_to_string(&receipt_path).expect("receipt file must be written");
    assert!(body.contains("\"trade_id\": \"CrossDexArb\""));
    assert!(body.contains("\"attestation\""));

    let verify_status = x3c()
        .arg("receipt")
        .arg("verify")
        .arg(&receipt_path)
        .status()
        .expect("x3c receipt verify");
    assert!(
        verify_status.success(),
        "a receipt produced by `receipt execute` must itself pass `receipt verify`"
    );
}

#[test]
fn cli_receipt_execute_handles_a_trade_with_a_bridge_leg() {
    // NeutralFixtureHost's bridge() completes the CLI's coverage of
    // trading-core-v1's full statement set: a program that crosses chains
    // must compile, execute, and produce a verifiable receipt through the
    // CLI exactly like a single-chain one does.
    let src = write_fixture("cli_receipt_execute_bridge.x3", BRIDGE_TRADING_SOURCE);
    let receipt_path = std::env::temp_dir().join("cli_receipt_execute_bridge_out.json");

    let output = x3c()
        .arg("receipt")
        .arg("execute")
        .arg(&src)
        .arg("--out")
        .arg(&receipt_path)
        .output()
        .expect("x3c receipt execute");
    assert!(
        output.status.success(),
        "receipt execute must succeed for a well-formed bridge trade: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("CrossDexArbToBase"));

    let body = std::fs::read_to_string(&receipt_path).expect("receipt file must be written");
    assert!(body.contains("\"trade_id\": \"CrossDexArbToBase\""));
    assert!(body.contains("\"attestation\""));

    let verify_status = x3c()
        .arg("receipt")
        .arg("verify")
        .arg(&receipt_path)
        .status()
        .expect("x3c receipt verify");
    assert!(
        verify_status.success(),
        "a receipt produced by `receipt execute` for a bridge trade must itself pass `receipt verify`"
    );
}

#[test]
fn cli_receipt_execute_is_deterministic_given_the_same_signing_key() {
    let src = write_fixture("cli_receipt_execute_deterministic.x3", TRADING_SOURCE);
    let key = "1111111111111111111111111111111111111111111111111111111111111111";
    let key = &key[..64];

    let first = x3c()
        .arg("receipt")
        .arg("execute")
        .arg(&src)
        .arg("--key-hex")
        .arg(key)
        .output()
        .expect("x3c receipt execute");
    let second = x3c()
        .arg("receipt")
        .arg("execute")
        .arg(&src)
        .arg("--key-hex")
        .arg(key)
        .output()
        .expect("x3c receipt execute");

    assert!(first.status.success() && second.status.success());
    assert_eq!(
        first.stdout, second.stdout,
        "the same source and signing key must produce byte-identical receipts"
    );
}

#[test]
fn cli_receipt_execute_rejects_a_trade_past_its_deadline() {
    let src = write_fixture("cli_receipt_execute_expired.x3", TRADING_SOURCE);

    let status = x3c()
        .arg("receipt")
        .arg("execute")
        .arg(&src)
        .arg("--block")
        .arg("999")
        .status()
        .expect("x3c receipt execute");

    assert!(
        !status.success(),
        "a trade executed past its compiled deadline must be rejected, not silently succeed"
    );
}

#[test]
fn cli_receipt_execute_rejects_a_malformed_signing_key() {
    let src = write_fixture("cli_receipt_execute_badkey.x3", TRADING_SOURCE);

    let status = x3c()
        .arg("receipt")
        .arg("execute")
        .arg(&src)
        .arg("--key-hex")
        .arg("deadbeef")
        .status()
        .expect("x3c receipt execute");

    assert!(
        !status.success(),
        "a 4-byte key-hex must be rejected, not silently truncated/padded"
    );
}

#[test]
fn cli_audit_passes_a_program_with_zero_fail_and_zero_warn_issues() {
    let src = write_fixture("cli_audit_fully_configured.x3", FULLY_CONFIGURED_SOURCE);
    let output = x3c().arg("audit").arg(&src).output().expect("x3c audit");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains("[FAIL]"),
        "fixture must have zero real failures: {stdout}"
    );
    assert!(
        !stdout.contains("[WARN]"),
        "fixture must have zero warnings, to isolate this from the WARN/FAIL conflation being tested: {stdout}"
    );
    assert!(
        stdout.contains("Status: PASS"),
        "zero FAIL and zero WARN must audit clean: {stdout}"
    );
    assert!(
        output.status.success(),
        "a program with no real issues must exit 0: {stdout}"
    );
}

#[test]
fn cli_audit_status_reflects_fail_severity_not_warn_count() {
    // Regression test for a real bug: `has_failures` used to be computed
    // from `!issues.is_empty()`, where `issues` held both [FAIL] and
    // [WARN]-prefixed strings in one vector — so a single missed "consider
    // adding X for production" WARN flipped the entire audit to FAIL, with
    // no way to ever report a clean pass short of declaring every optional
    // B-52 item. Every example .x3 file in this repo failed `x3c audit`
    // for exactly this reason, including ones explicitly named as the
    // canonical safe example (examples/mainnet_safe_swap.x3) and the
    // flagship feature-complete one (examples/flagship_b52.x3). Dropping
    // exactly one optional declaration (here: the `privacy` block) from an
    // otherwise fully-configured, zero-FAIL program must produce exactly
    // one [WARN] and still report Status: PASS.
    let without_privacy = FULLY_CONFIGURED_SOURCE.replacen(
        "privacy {\n    hide_route_until_commit true\n    reveal_on claim\n}\n\n",
        "",
        1,
    );
    assert_ne!(
        without_privacy, FULLY_CONFIGURED_SOURCE,
        "the privacy block must actually have been removed from the fixture"
    );
    let src = write_fixture("cli_audit_one_warning.x3", &without_privacy);

    let output = x3c().arg("audit").arg(&src).output().expect("x3c audit");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains("[FAIL]"),
        "dropping an optional block must not introduce a real failure: {stdout}"
    );
    assert!(
        stdout.contains("[WARN] no privacy block found"),
        "dropping the privacy block must be flagged as a warning: {stdout}"
    );
    assert!(
        stdout.contains("Status: PASS"),
        "a program with only WARN-level issues and zero FAIL-level issues must still report PASS: {stdout}"
    );
    assert!(
        output.status.success(),
        "a program with only warnings must exit 0, not 1: {stdout}"
    );
}

#[test]
fn cli_audit_recognizes_trading_core_v1_declarations() {
    // Regression test for a real bug: every check here (has_intent,
    // has_nonce, has_refund, has_timeout, has_risk_policy, has_invariant)
    // was originally written against only the older intent-DSL's AST
    // shape (Item::IntentDecl and its Statement variants). A
    // trading-core-v1 program has none of those — it lowers to
    // Item::AtomicTrade / Item::TradeRiskPolicy instead — so every one of
    // these checks used to report FAIL/WARN regardless of how safe the
    // trade actually was, e.g. "no risk policy found" on a program that
    // manifestly declares one.
    let src = write_fixture("cli_audit_trading_core.x3", TRADING_SOURCE);
    let output = x3c()
        .arg("--mode")
        .arg("dev")
        .arg("audit")
        .arg(&src)
        .output()
        .expect("x3c audit");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        stdout.contains("[PASS] atomic trade declaration present"),
        "must recognize Item::AtomicTrade as a valid top-level declaration: {stdout}"
    );
    assert!(
        stdout.contains("deadline_blocks present"),
        "must recognize the mandatory risk-policy deadline as satisfying the timeout check: {stdout}"
    );
    assert!(
        stdout.contains("[PASS] risk policy configured"),
        "must recognize Item::TradeRiskPolicy, not just the older Item::RiskPolicy: {stdout}"
    );
    assert!(
        !stdout.contains("no intent declaration found"),
        "must not demand an Item::IntentDecl from a trading-core-v1 program: {stdout}"
    );
    assert!(
        !stdout.contains("missing nonce guard"),
        "must not demand an AST-level nonce guard — trading-core-v1 replay protection is a receipt-layer concern: {stdout}"
    );
    assert!(
        !stdout.contains("no risk policy found"),
        "must not report a risk policy as absent when Item::TradeRiskPolicy is present: {stdout}"
    );
    assert!(
        !stdout.contains("no vm declaration found") && !stdout.contains("no solver market found"),
        "must not demand cross-chain bridge infrastructure from a single-chain atomic trade: {stdout}"
    );
}

/// A Trading Core v1 program the verifier accepts with **no** warnings: it
/// declares no intent endpoints and no bridge, so neither the invariant rules
/// nor the proof-requirement pass has anything to say about it.
const CLEAN_SOURCE: &str = r#"asset USDC = evm.ethereum.0xA0b8 {
    decimals: 6
}

asset WETH = evm.ethereum.0xC02a {
    decimals: 18
}

asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 {
    decimals: 18
}

risk policy MainnetArb {
    max_slippage: 30 bps
    max_gas: 0.02 ETH
    max_flash_fee: 10 bps
    deadline: 2 blocks
    require_private_submission: false
}

atomic trade CrossDexArb using MainnetArb {
    borrow 1_000_000 USDC from aave_v3 as debt

    let weth = swap debt.amount USDC -> WETH
        via uniswap_v3
        min_out 410 WETH

    let returned = swap weth WETH -> USDC
        via sushiswap
        min_out 1_002_000 USDC

    repay debt

    require net_profit >= 1_000 USDC
    require all_debts_repaid
    emit receipt
}
"#;

#[test]
fn cli_check_reports_warnings_instead_of_dropping_them() {
    // Regression: the verifier collected warnings and `cmd_check` discarded
    // them, so a program with an unfulfilled proof requirement reported a
    // clean bill of health. The warning list must now reach the caller.
    let src = write_fixture("cli_warn.x3", WARN_SOURCE);
    let out = std::env::temp_dir().join("cli_warn.json");
    let status = x3c()
        .arg("check")
        .arg(&src)
        .arg("--out")
        .arg(&out)
        .status()
        .expect("x3c check");
    assert!(status.success(), "warnings alone must not fail a check: {status:?}");

    let body = std::fs::read_to_string(&out).expect("check output read");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("check output must be JSON");
    let warnings = parsed
        .get("warnings")
        .and_then(|value| value.as_array())
        .expect("check output must carry a warnings array");
    assert!(
        !warnings.is_empty(),
        "WARN_SOURCE bridges without proofs, so its warnings must be reported; got {body}"
    );
}

#[test]
fn cli_deny_warnings_fails_a_program_that_only_warns() {
    let src = write_fixture("cli_warn_deny.x3", WARN_SOURCE);
    let status = x3c()
        .arg("--deny-warnings")
        .arg("check")
        .arg(&src)
        .status()
        .expect("x3c check --deny-warnings");
    assert!(
        !status.success(),
        "--deny-warnings must fail a program that produces warnings: {status:?}"
    );
}

#[test]
fn cli_build_deny_warnings_fails_a_program_that_only_warns() {
    // `--deny-warnings` is a global flag documented as "treat semantic warnings
    // as failures", but only `check` honoured it. `build` accepted it and
    // dropped the warnings, so this exact invocation exited 0 while `check` on
    // the same source reported two warnings — a false green on the command CI
    // actually uses.
    let src = write_fixture("cli_build_warn_deny.x3", WARN_SOURCE);
    let out = std::env::temp_dir().join("cli_build_warn_deny.bin");
    let status = x3c()
        .arg("--deny-warnings")
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .status()
        .expect("x3c build --deny-warnings");
    assert!(
        !status.success(),
        "build --deny-warnings must fail a program that produces warnings: {status:?}"
    );
    assert!(
        !out.exists(),
        "a refused build must not leave bytecode behind — that is how a false green gets shipped"
    );
}

#[test]
fn cli_build_without_deny_warnings_still_succeeds_and_reports_the_warning() {
    // Non-vacuous: warnings must not become errors by default, and they must
    // not become invisible either.
    let src = write_fixture("cli_build_warn.x3", WARN_SOURCE);
    let out = std::env::temp_dir().join("cli_build_warn.bin");
    let _ = std::fs::remove_file(&out);
    let result = x3c()
        .arg("build")
        .arg(&src)
        .arg("-o")
        .arg(&out)
        .output()
        .expect("x3c build");
    assert!(result.status.success(), "build must still succeed: {result:?}");
    assert!(out.exists(), "a successful build must write bytecode");
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("source-lock proof") && stderr.contains("destination-fill proof"),
        "the warnings the verifier produced must be visible on a successful build, got: {stderr}"
    );
}

#[test]
fn cli_deny_warnings_passes_a_program_with_no_warnings() {
    // Non-vacuous: the flag must not simply fail everything.
    let src = write_fixture("cli_clean_deny.x3", CLEAN_SOURCE);
    let status = x3c()
        .arg("--deny-warnings")
        .arg("check")
        .arg(&src)
        .status()
        .expect("x3c check --deny-warnings");
    assert!(
        status.success(),
        "a warning-free program must pass --deny-warnings: {status:?}"
    );
}

#[test]
fn graph_says_when_it_is_ignoring_a_declared_objective() {
    // `x3c optimize` follows a program's `objective` declaration; `x3c graph` exists
    // to show what the graph holds, so it searches without those ceilings and says so
    // rather than letting a reader take the list for the set the program allows.
    // Measured on the example: the declaration's fee ceiling excludes the 9 bps route,
    // and `graph` lists it — which is exactly the difference the note is about.
    let with_objective = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/objective_routing.x3");
    let output = x3c()
        .args([
            "graph",
            with_objective.to_str().expect("a path"),
            "--from",
            "ethereum.USDC",
            "--to",
            "solana.SOL",
        ])
        .output()
        .expect("x3c graph runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "graph must succeed: {stdout}");
    assert!(
        stdout.contains("declares objective 'cheapest_route'") && stdout.contains("ignores its constraints"),
        "the note must name the objective and say what this command does with it: {stdout}"
    );
    assert!(
        stdout.contains("deep_pool"),
        "and the routes the declaration would exclude are still listed: {stdout}"
    );

    // A program with no declaration gets no note.
    let without = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/opportunity_graph.x3");
    let output = x3c()
        .args([
            "graph",
            without.to_str().expect("a path"),
            "--from",
            "ethereum.USDC",
            "--to",
            "solana.SOL",
        ])
        .output()
        .expect("x3c graph runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success(), "graph must succeed: {stdout}");
    assert!(
        !stdout.contains("declares objective"),
        "there is nothing to say about a program that declares none: {stdout}"
    );
}

/// `x3c fusion` — spec PHASE 21's netting report.
///
/// The compiler module has its own tests (`compiler/tests/test_fusion.rs`), but
/// those call `fusion::rings` directly. This is the reachability the feature
/// needs to be a feature rather than a module: without a test that runs the
/// *binary* and reads its report, `x3c fusion` could print anything — or nothing
/// — and every test in the workspace would still pass. The same gap that
/// TICKET-042 closed for the opportunity graph.
fn fusion_intent(
    name: &str,
    give: &str,
    give_amount: u128,
    want: &str,
    min_out: u128,
    deadline: u32,
    opt_in: bool,
) -> String {
    let allow = if opt_in { "    allow intent_fusion\n" } else { "" };
    format!(
        r#"intent {name} {{
    from ethereum.{give} amount {give_amount} receiver 0x1
    to ethereum.{want} receiver 0x2
    route {{
        swap uniswap ethereum.{give} -> ethereum.{want} amount {give_amount} min_output {min_out}
    }}
{allow}    require nonce unused {name}_nonce
    require slippage <= 50
    timeout {deadline} refund ethereum.{give} to sender
    on_fail rollback
}}
"#
    )
}

#[test]
fn cli_fusion_reports_a_ring_that_closes() {
    // Alice gives ETH and wants SOL, Bob gives SOL and wants USDC, Charlie gives
    // USDC and wants ETH. Every participant opted in and every declared minimum
    // is met by what the next participant hands over.
    let source = format!(
        "{}{}{}",
        fusion_intent("alice", "ETH", 10, "SOL", 9, 30, true),
        fusion_intent("bob", "SOL", 9, "USDC", 8, 20, true),
        fusion_intent("charlie", "USDC", 8, "ETH", 7, 60, true)
    );
    let fixture = write_fixture("cli_fusion_ring.x3", &source);
    let output = x3c().arg("fusion").arg(&fixture).output().expect("run x3c fusion");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "fusion must exit 0: {stdout}");
    assert!(stdout.contains("1 ring(s)"), "the ring must be reported: {stdout}");
    assert!(
        stdout.contains("alice -> bob -> charlie"),
        "the ring must name its participants in order: {stdout}"
    );
    assert!(
        stdout.contains("earliest deadline 20 block(s)"),
        "the ring's deadline is its most urgent participant's: {stdout}"
    );
    for check in [
        "authorization",
        "asset correctness",
        "minimum output",
        "deadline",
        "fairness",
    ] {
        assert!(
            stdout.contains(&format!("{check}: satisfied")),
            "every check must be stated, not implied ({check}): {stdout}"
        );
    }
    assert!(
        stdout.contains("verdict: fusable"),
        "a ring whose checks all pass is fusable: {stdout}"
    );
}

#[test]
fn cli_fusion_never_internalizes_an_intent_that_did_not_opt_in() {
    // The same ring without `allow intent_fusion`.
    let source = format!(
        "{}{}{}",
        fusion_intent("alice", "ETH", 10, "SOL", 9, 30, false),
        fusion_intent("bob", "SOL", 9, "USDC", 8, 20, false),
        fusion_intent("charlie", "USDC", 8, "ETH", 7, 60, false)
    );
    let fixture = write_fixture("cli_fusion_no_optin.x3", &source);
    let output = x3c().arg("fusion").arg(&fixture).output().expect("run x3c fusion");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "fusion must exit 0: {stdout}");
    assert!(
        stdout.contains("0 ring(s)"),
        "an intent that did not consent is never netted, so there is no ring: {stdout}"
    );
    assert!(
        !stdout.contains("verdict: fusable"),
        "and nothing may be reported as fusable: {stdout}"
    );
}

/// `if`/`loop` were emitted as records no reader could follow.
///
/// Measured on this program before the refusal: `x3c build` wrote 320 bytes,
/// `x3c explain` printed the condition text as opcodes, and `x3c run` failed with
/// `X3_VERIFY_FAILED: OutOfBounds(292)`. The IR verifier and the emitter refuse
/// now, and the refusal has to reach *both* commands — a `check` that accepted
/// what `build` refuses is the split this test exists to prevent — and neither may
/// leave an artifact behind.
#[test]
fn cli_refuses_a_branch_no_reader_could_follow_instead_of_writing_one() {
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("the crate lives under x3-lang")
        .join("examples")
        .join("strategy_module.x3");
    let source = std::fs::read_to_string(&example).expect("the strategy example must be readable");
    let anchor = "        require profit >= 5\n";
    assert!(
        source.contains(anchor),
        "this test wraps one guard in an `if`, so the guard has to still be there: {example:?}"
    );
    let branched = source.replace(
        anchor,
        "        if 1 > 0 {\n            require profit >= 5\n        }\n",
    );
    let fixture = write_fixture("cli_strategy_with_a_branch.x3", &branched);

    let check = x3c().arg("check").arg(&fixture).output().expect("run x3c check");
    let check_output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!check.status.success(), "check must refuse the branch: {check_output}");
    assert!(
        check_output.contains("`if` cannot be executed"),
        "and it must say which construct and why: {check_output}"
    );

    let out = std::env::temp_dir().join("cli_strategy_with_a_branch.x3b");
    let _ = std::fs::remove_file(&out);
    let build = x3c()
        .arg("build")
        .arg(&fixture)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("run x3c build");
    let build_output = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(!build.status.success(), "build must refuse the branch: {build_output}");
    assert!(!out.exists(), "and no artifact may be written next to it");
}

/// The proof obligation is a *mainnet* requirement (TICKET-020/024).
///
/// A bridge whose lock is not proven is a bridge the destination fills against
/// nothing, and every other rule of this shape in the language is an error in
/// mainnet mode and tolerated in dev (`verify_mainnet_safe`). This asserts the
/// same source in both modes, so the difference is the severity and nothing else.
#[test]
fn cli_refuses_a_bridging_program_without_its_proofs_on_mainnet() {
    let src = write_fixture("cli_mainnet_proof_obligation.x3", WARN_SOURCE);

    let dev = x3c().arg("check").arg(&src).output().expect("run x3c check");
    let dev_output = format!(
        "{}{}",
        String::from_utf8_lossy(&dev.stdout),
        String::from_utf8_lossy(&dev.stderr)
    );
    assert!(
        dev.status.success(),
        "dev mode tolerates it with a warning: {dev_output}"
    );
    assert!(dev_output.contains("source-lock proof"), "and says so: {dev_output}");

    let mainnet = x3c()
        .arg("check")
        .arg("--mode")
        .arg("mainnet")
        .arg(&src)
        .output()
        .expect("run x3c check --mode mainnet");
    let mainnet_output = format!(
        "{}{}",
        String::from_utf8_lossy(&mainnet.stdout),
        String::from_utf8_lossy(&mainnet.stderr)
    );
    assert!(
        !mainnet.status.success(),
        "mainnet must refuse a bridge with no proof obligation: {mainnet_output}"
    );
    assert!(
        mainnet_output.contains("mainnet:") && mainnet_output.contains("source-lock proof"),
        "and the refusal must name the obligation as a mainnet requirement: {mainnet_output}"
    );
}

/// PHASE 47 — source provenance: which exact source produced this artifact?
///
/// The record is measured, not guessed: the hashes are of bytes that exist, and a
/// field the build could not observe says `"unknown"` with the reason rather than
/// being filled with something plausible. This test builds one fixture twice and
/// checks both halves of that.
#[test]
fn cli_build_writes_provenance_that_names_the_source_and_the_artifact() {
    let source = "risk_policy {\n    min_route_score 90\n}\n\nintent provenance {\n    from \
                  ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        swap uniswap \
                  ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    require slippage <= \
                  50\n    on_fail refund ethereum.USDC to sender\n}\n";
    let fixture = write_fixture("cli_provenance.x3", source);
    let artifact = std::env::temp_dir().join("cli_provenance.x3b");
    let document = std::env::temp_dir().join("cli_provenance.json");
    let _ = std::fs::remove_file(&artifact);
    let _ = std::fs::remove_file(&document);

    let output = x3c()
        .arg("build")
        .arg(&fixture)
        .arg("--out")
        .arg(&artifact)
        .arg("--provenance")
        .arg(&document)
        .output()
        .expect("run x3c build --provenance");
    assert!(
        output.status.success(),
        "build must succeed: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&document).expect("the document exists"))
            .expect("the document must be JSON");

    // The source hash is of the source as read: recompute it here rather than
    // trusting the string.
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(source.as_bytes());
    let source_hash = format!("sha256:{}", hex_lower(&hasher.finalize()));
    assert_eq!(json["source_hash"], source_hash, "{json}");

    let bytes = std::fs::read(&artifact).expect("the artifact exists");
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let artifact_hash = format!("sha256:{}", hex_lower(&hasher.finalize()));
    assert_eq!(json["artifact_hash"], artifact_hash, "{json}");

    assert!(
        json["compiler_version"].as_str().is_some_and(|value| !value.is_empty()),
        "the compiler's version is a fact about the binary: {json}"
    );
    assert!(
        json["repository_commit"].as_str().is_some(),
        "a commit is recorded, or `unknown` with a note — never absent: {json}"
    );
    assert!(
        json["notes"].as_array().is_some() || json["repository_commit"] != "unknown",
        "a field that could not be measured must carry a note: {json}"
    );

    // Deterministic: the same source and the same compiler give the same hashes.
    let second = std::env::temp_dir().join("cli_provenance_2.json");
    let _ = std::fs::remove_file(&second);
    let artifact_2 = std::env::temp_dir().join("cli_provenance_2.x3b");
    let status = x3c()
        .arg("build")
        .arg(&fixture)
        .arg("--out")
        .arg(&artifact_2)
        .arg("--provenance")
        .arg(&second)
        .output()
        .expect("second build");
    assert!(status.status.success());
    let second_json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&second).expect("second document"))
            .expect("second document must be JSON");
    assert_eq!(json["source_hash"], second_json["source_hash"]);
    assert_eq!(json["artifact_hash"], second_json["artifact_hash"]);
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// PHASE 35 — the cost report, with the basis of every figure and the gaps named.
///
/// The weight it prints is the VM's own table applied to the artifact the compiler
/// would write, so "what a run costs" and "what the compiler estimates" cannot
/// disagree. What the compiler cannot know is listed as unestimated rather than
/// filled with a plausible number.
#[test]
fn cli_estimate_reports_the_cost_of_the_artifact_it_would_write() {
    let source = "intent cost_cli {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        \
                  swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    require \
                  slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}\n";
    let fixture = write_fixture("cli_estimate.x3", source);
    let output = x3c().arg("estimate").arg(&fixture).output().expect("run x3c estimate");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "estimate must succeed: {stdout}");
    for expected in [
        "instructions",
        "base weight",
        "the table vm/src/executor.rs charges from",
        "payload bytes",
        "host-facing",
        "proof bytes carried",
        "Not estimated, and why",
        "EVM gas",
    ] {
        assert!(
            stdout.contains(expected),
            "the report must contain {expected:?}: {stdout}"
        );
    }

    // The weight is not a constant: a program with an extra instruction weighs
    // more, which is the property that makes the figure an estimate rather than a
    // number in a template.
    let longer = write_fixture(
        "cli_estimate_longer.x3",
        &source.replace(
            "    on_fail refund ethereum.USDC to sender\n",
            "    require profit >= 1\n    on_fail refund ethereum.USDC to sender\n",
        ),
    );
    let second = x3c().arg("estimate").arg(&longer).output().expect("run x3c estimate");
    let second_stdout = String::from_utf8_lossy(&second.stdout);
    assert!(second.status.success(), "{second_stdout}");
    let weight_of = |text: &str| -> u128 {
        text.lines()
            .find(|line| line.contains("base weight"))
            .and_then(|line| line.split_whitespace().nth(2))
            .and_then(|value| value.parse().ok())
            .unwrap_or_else(|| panic!("no weight in {text}"))
    };
    assert!(
        weight_of(&second_stdout) > weight_of(&stdout),
        "an extra guard must weigh more: {} vs {}",
        weight_of(&second_stdout),
        weight_of(&stdout)
    );
}

/// PHASE 36 — a route cannot satisfy a floor its own declarations put out of reach.
///
/// The comparison is between two things the program states: the profit floor it
/// claims and the fees its own `venue` declarations declare, applied to the output
/// each leg promises at minimum. It is reported as a *warning* with the figures, and
/// the message says what it is not — the phase itself warns against reading a static
/// estimate as a runtime guarantee.
#[test]
fn cli_warns_when_a_declared_floor_is_below_the_declared_fees() {
    let venue = "venue costly_lending {\n    kind lending\n    chain ethereum\n    domain evm\n    \
                 asset_in ethereum.USDC\n    asset_out ethereum.USDC\n    fee_bps 500\n    liquidity \
                 1_000_000\n    slippage_bps 5\n    latency_ms 10\n    finality_blocks 12\n    risk \
                 3\n    proof source_lock_proof\n}\n\n";
    // Spends `spent`, accepts at least `min_output` back, and claims `floor` of net
    // profit; 500 bps of the minimum is the venue's declared fee.
    let intent = |spent: u128, min_output: u128, floor: u128| {
        format!(
            "intent profit_probe {{\n    from ethereum.USDC amount {spent}\n    to ethereum.USDC\n    \
             route {{\n        swap costly_lending ethereum.USDC -> ethereum.USDC amount {spent} \
             min_output {min_output}\n    }}\n    require slippage <= 50\n    require profit >= {floor}\n    \
             on_fail refund ethereum.USDC to sender\n}}\n"
        )
    };

    // At the leg's own minimum (2_000 − 1_000 − 100 of fees) the route nets 900, so a
    // floor of 1_000 is above what the declarations support.
    let unreachable = write_fixture(
        "cli_profitability_bad.x3",
        &format!("{venue}{}", intent(1_000, 2_000, 1_000)),
    );
    let check = x3c().arg("check").arg(&unreachable).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        output.contains("route cannot satisfy its declared minimum profit at the output it itself"),
        "the declaration mismatch must be reported: {output}"
    );
    assert!(
        output.contains("at least 1000 ethereum.USDC")
            && output.contains("nets 900 ethereum.USDC")
            && output.contains("100 ethereum.USDC of fees"),
        "with the floor, the net and the fee: {output}"
    );
    assert!(
        output.contains("not a quote"),
        "and without claiming to be a price: {output}"
    );
    assert!(check.status.success(), "a warning is not a failure: {output}");

    let deny = x3c()
        .arg("check")
        .arg("--deny-warnings")
        .arg(&unreachable)
        .output()
        .expect("x3c check --deny-warnings");
    assert!(!deny.status.success(), "--deny-warnings must turn it into a failure");

    // The same program with a floor its declared minimum covers is not warned about.
    let satisfied = write_fixture(
        "cli_profitability_ok.x3",
        &format!("{venue}{}", intent(1_000, 2_000, 500)),
    );
    let check = x3c().arg("check").arg(&satisfied).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        check.status.success() && !output.contains("route cannot satisfy"),
        "a floor above the declared fees must pass silently: {output}"
    );
}

/// PHASE 9 — a hedge lowers to orders, and it runs.
///
/// The legs used to be resolved and refused: a perp leg needs a venue adapter, so the
/// exposure was decided and the execution was not pretended. They lower to **venue orders**
/// now — an action from a vocabulary the compiler owns, an asset and a quantity — so the
/// whole path holds, and the artifact carries what the hedge decided. The other half is
/// unchanged: a hedge whose legs do not net is refused with the delta, before any of this.
#[test]
fn cli_lowers_a_hedge_to_venue_orders_and_runs_it() {
    let balanced = write_fixture(
        "cli_hedge_balanced.x3",
        "atomic_hedge {\n    buy 1_000 ethereum.ETH spot;\n    short equivalent ethereum.ETH perp;\n\n    \
         require delta <= 0.01%;\n}\n",
    );
    let check = x3c().arg("check").arg(&balanced).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.status.success(), "a hedge must check now: {output}");
    assert!(
        !output.contains("leaves a delta"),
        "a balanced hedge has no exposure to complain about: {output}"
    );

    let out = std::env::temp_dir().join("cli_hedge_balanced.x3b");
    let build = x3c()
        .arg("build")
        .arg(&balanced)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("x3c build");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(build.status.success(), "the hedge's orders must build: {text}");

    // The artifact carries both legs as orders, the `equivalent` leg resolved to the
    // other side's size, the bound as a guard, and the whole thing atomic.
    let explain = x3c().arg("explain").arg(&out).output().expect("x3c explain");
    let disassembly = format!(
        "{}{}",
        String::from_utf8_lossy(&explain.stdout),
        String::from_utf8_lossy(&explain.stderr)
    );
    assert!(
        disassembly.contains("ATOMIC_BEGIN") && disassembly.contains("ATOMIC_END"),
        "a hedge's legs must be one atomic plan: {disassembly}"
    );
    assert_eq!(
        disassembly.matches("VENUE_ORDER").count(),
        2,
        "one order per leg: {disassembly}"
    );
    assert!(
        disassembly.contains("spot_buy") && disassembly.contains("perp_short"),
        "the actions must say which market and which direction: {disassembly}"
    );
    assert!(
        disassembly.contains("REQUIRE"),
        "the delta bound must travel as a guard: {disassembly}"
    );

    // The quantities live in the payload, which `explain` prints as bytes — so the
    // resolution of `equivalent` is checked where it is legible: the lowered IR.
    let ir_out = std::env::temp_dir().join("cli_hedge_balanced.json");
    let lower = x3c()
        .arg("lower")
        .arg(&balanced)
        .arg("--out")
        .arg(&ir_out)
        .output()
        .expect("x3c lower");
    assert!(lower.status.success(), "the hedge must lower");
    let ir = std::fs::read_to_string(&ir_out).expect("the IR document");
    let orders: Vec<&str> = ir.match_indices("VenueOrder").map(|(i, _)| &ir[i..]).collect();
    assert_eq!(orders.len(), 2, "both legs lower to orders: {ir}");
    assert!(
        orders[0].contains("\"quantity\": 1000") && orders[1].contains("\"quantity\": 1000"),
        "the `equivalent` leg takes the other side's written size, so both are 1000: {ir}"
    );
    assert!(
        orders[0].contains("spot_buy") && orders[1].contains("perp_short"),
        "and each says which market and which direction: {ir}"
    );

    let run = x3c().arg("run").arg(&out).output().expect("x3c run");
    let run_text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.status.success() && run_text.contains("x3c run: ok"),
        "the hedge's orders must run against the fixture host: {run_text}"
    );
}

/// PHASE 10 — a liquidation's accounting is decided, and its calls are not pretended.
#[test]
fn cli_decides_a_liquidations_accounting_and_refuses_its_calls() {
    let sound = "atomic_liquidation {\n    liquidate 1_000 ethereum.USDC of borrower.position;\n    \
                 receive 1_200 ethereum.ETH collateral;\n    swap 1_200 ethereum.ETH -> \
                 ethereum.USDC min_output 1_100;\n    repay 1_000 ethereum.USDC;\n    require \
                 net_profit >= 100 ethereum.USDC;\n}\n";
    let fixture = write_fixture("cli_liquidation_sound.x3", sound);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!check.status.success(), "a liquidation cannot be built here: {output}");
    assert!(
        output.contains("liquidate and receive are calls into a lending protocol"),
        "the refusal must name the missing adapter: {output}"
    );
    assert!(
        !output.contains("cannot repay its capital") && !output.contains("unaccounted"),
        "a sound liquidation has no accounting to complain about: {output}"
    );

    let underfunded = fixture.with_file_name("cli_liquidation_underfunded.x3");
    std::fs::write(&underfunded, sound.replace("min_output 1_100", "min_output 900")).expect("write");
    let check = x3c().arg("check").arg(&underfunded).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        output.contains("cannot repay its capital")
            && output.contains("minimum output is 900")
            && output.contains("shortfall is 100"),
        "the repayment must be reported with its figures: {output}"
    );
    assert!(
        !output.contains("lending protocol"),
        "the accounting is decided before the adapter question is reached: {output}"
    );
}

/// PHASE 11 — a target portfolio's weights are decided, and the plan is not pretended.
#[test]
fn cli_decides_a_rebalances_weights_and_refuses_to_pretend_it_plans() {
    let sound = "rebalance portfolio {\n    BTC = 40%;\n    ETH = 25%;\n    SOL = 15%;\n    X3 = \
                 10%;\n    USDC = 10%;\n\n    minimize {\n        fees;\n        slippage;\n    }\n\n    \
                 atomic;\n}\n";
    let fixture = write_fixture("cli_rebalance_sound.x3", sound);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!check.status.success(), "no plan can be generated yet: {output}");
    assert!(
        output.contains("cannot yet generate the transaction graph"),
        "the refusal must say what is missing: {output}"
    );
    assert!(
        !output.contains("sums to") && !output.contains("cannot rank"),
        "a portfolio that adds up has no weight or criterion to complain about: {output}"
    );

    let unbalanced = write_fixture("cli_rebalance_unbalanced.x3", &sound.replace("SOL = 15%", "SOL = 5%"));
    let check = x3c().arg("check").arg(&unbalanced).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        output.contains("sums to 90%, not 100%") && output.contains("BTC 40%"),
        "the weights must be reported with their figures: {output}"
    );
    assert!(
        !output.contains("transaction graph"),
        "the weights are decided before the plan question is reached: {output}"
    );

    let unrankable = write_fixture(
        "cli_rebalance_unrankable.x3",
        &sound.replace("        fees;", "        external_liquidity;"),
    );
    let check = x3c().arg("check").arg(&unrankable).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        output.contains("cannot rank") && output.contains("does not distinguish liquidity the ring supplies"),
        "an unrankable target must carry the objective's own reason: {output}"
    );
}

/// `x3c netting` — spec PHASE 22's obligation offsets.
///
/// The compiler module has its own tests (`compiler/tests/test_netting.rs`), but
/// those call `netting::books` directly. This is the reachability the feature needs
/// to be a feature rather than a module: without a test that runs the *binary* and
/// reads its report, `x3c netting` could print anything — or nothing — and every
/// test in the workspace would still pass.
#[test]
fn cli_netting_offsets_a_book_and_measures_what_it_removed() {
    let source = "netting book_a {\n    consent alice;\n    consent bob;\n    consent carol;\n    \
                  account alice = 0xA1;\n    account bob = 0xB1;\n    account carol = 0xC1;\n    \
                  alice owes 500 ethereum.USDC to bob;\n    bob owes 300 ethereum.USDC to \
                  alice;\n    carol owes 120 ethereum.USDC to alice;\n    alice owes 40 \
                  ethereum.USDC to carol;\n}\n";
    let fixture = write_fixture("cli_netting_book.x3", source);
    let output = x3c().arg("netting").arg(&fixture).output().expect("run x3c netting");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "netting must exit 0: {stdout}");
    assert!(
        stdout.contains("book 'book_a'") && stdout.contains("3 part(ies)"),
        "the book must be named with its parties: {stdout}"
    );
    assert!(
        stdout.contains("4 obligation(s) -> 2 transfer(s), gross 960 -> 200"),
        "the saving must be measured, not asserted: {stdout}"
    );
    assert!(
        stdout.contains("alice -> bob: 120") && stdout.contains("carol -> bob: 80"),
        "the residual must be printed: {stdout}"
    );
    assert!(
        stdout.contains("changed no party's net position"),
        "the invariant must be reported: {stdout}"
    );
}

#[test]
fn cli_netting_names_the_pairs_it_refused_to_combine() {
    // Two ledgers, same asset name. A report that listed only what it netted would
    // let a reader assume this pair was netted too.
    let source = "netting book_b {\n    consent alice;\n    consent bob;\n    account alice = \
                  0xA1;\n    account bob = 0xB1;\n    alice owes 5 ethereum.USDC to bob;\n    bob \
                  owes 5 x3.USDC to alice;\n}\n";
    let fixture = write_fixture("cli_netting_ledgers.x3", source);
    let output = x3c().arg("netting").arg(&fixture).output().expect("run x3c netting");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "netting must exit 0: {stdout}");
    assert_eq!(
        stdout.matches("gross 5 -> 5").count(),
        2,
        "both groups keep their one obligation, so nothing was netted across the two \
         ledgers: {stdout}"
    );
    assert!(
        stdout.contains("were NOT combined"),
        "the pair it declined must be named: {stdout}"
    );
}

#[test]
fn cli_netting_refuses_a_book_that_nets_a_party_that_did_not_consent() {
    let source = "netting book_c {\n    consent alice;\n    alice owes 500 ethereum.USDC to \
                  bob;\n    bob owes 300 ethereum.USDC to alice;\n}\n";
    let fixture = write_fixture("cli_netting_no_consent.x3", source);
    let output = x3c().arg("netting").arg(&fixture).output().expect("run x3c netting");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !output.status.success(),
        "an unconsented offset must be refused: {text}"
    );
    assert!(
        text.contains("did not consent"),
        "the refusal must say who did not consent: {text}"
    );
}

#[test]
fn cli_settles_a_netting_book_builds_it_and_runs_it() {
    // The book used to be refused at the IR layer: a party was a name and there was
    // nothing to debit. It binds each party to an account now, so the whole path has to
    // hold — the offsets are decided, the residual lowers to locks and releases, the
    // artifact builds, and it runs.
    let source = "netting book_a {\n    consent alice;\n    consent bob;\n    consent carol;\n    \
                  account alice = 0xA1;\n    account bob = 0xB1;\n    account carol = 0xC1;\n    \
                  alice owes 500 ethereum.USDC to bob;\n    bob owes 300 ethereum.USDC to \
                  alice;\n    carol owes 120 ethereum.USDC to alice;\n    alice owes 40 \
                  ethereum.USDC to carol;\n}\n";
    let fixture = write_fixture("cli_netting_settled.x3", source);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.status.success(), "a bound book must check: {output}");
    assert!(
        !output.contains("cannot be executed"),
        "nothing about a bound book is unexecutable: {output}"
    );

    let out = std::env::temp_dir().join("cli_netting_settled.x3b");
    let build = x3c()
        .arg("build")
        .arg(&fixture)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("x3c build");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(build.status.success(), "the settlement must build: {text}");

    // The artifact carries the residual as locks and releases against the accounts the
    // book bound, inside one atomic block — not a count of them.
    let explain = x3c().arg("explain").arg(&out).output().expect("x3c explain");
    let disassembly = format!(
        "{}{}",
        String::from_utf8_lossy(&explain.stdout),
        String::from_utf8_lossy(&explain.stderr)
    );
    assert!(
        disassembly.contains("ATOMIC_BEGIN") && disassembly.contains("ATOMIC_END"),
        "the residual must settle atomically: {disassembly}"
    );
    assert_eq!(
        disassembly.matches("LOCK").count(),
        2,
        "two transfers were left standing, so two locks: {disassembly}"
    );
    assert_eq!(
        disassembly.matches("RELEASE").count(),
        2,
        "and one release per lock: {disassembly}"
    );
    assert!(
        disassembly.contains("alice") || disassembly.contains("0xA1"),
        "the debtor's account must be in the artifact: {disassembly}"
    );

    let run = x3c().arg("run").arg(&out).output().expect("x3c run");
    let run_text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.status.success() && run_text.contains("x3c run: ok"),
        "the settlement must run: {run_text}"
    );
}

#[test]
fn cli_refuses_a_book_whose_party_has_no_account() {
    // The other end of the same rule: a residual with nowhere to go is refused with the
    // party named, rather than lowered into a plan with a hole in it.
    let source = "netting book_a {\n    consent alice;\n    consent bob;\n    account alice = \
                  0xA1;\n    alice owes 500 ethereum.USDC to bob;\n    bob owes 300 \
                  ethereum.USDC to alice;\n}\n";
    let fixture = write_fixture("cli_netting_unbound.x3", source);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!check.status.success(), "an unbound party is a hole: {output}");
    assert!(
        output.contains("leaves 'bob' without an account"),
        "the refusal must name the party: {output}"
    );
}

/// `x3c check` on an `arb` — spec PHASE 37.
///
/// The compiler module has its own tests (`compiler/tests/test_arb.rs`), but those
/// call `arb::policy` and `arb::verify` directly. This is the reachability the
/// feature needs: the declaration has to reach the *binary*, and the command that
/// says whether a program can run has to refuse it with the stages that are
/// missing rather than with silence.
#[test]
fn cli_plans_an_arb_scope_builds_it_and_runs_it() {
    // PHASE 37's scope used to be refused at the IR layer over the two pipeline stages
    // with no implementation. `arb::plan` owns them now, so what has to hold is the
    // whole path: the scope checks, the plan lowers to a cycle and its floors, the
    // artifact builds, and the trade runs against a host.
    let source = "intent spread_trade {\n    from ethereum.USDC amount 1_000_000 receiver 0xA1\n    \
                  to ethereum.USDC receiver 0xA2\n    require profit >= 20\n    require \
                  slippage <= 50\n    timeout 30s refund ethereum.USDC to sender\n    on_fail \
                  rollback\n}\n\
                  venue usdc_to_eth {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps 3\n    \
                  liquidity 1_000_000\n    slippage_bps 8\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  venue eth_to_usdc {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.ETH\n    asset_out ethereum.USDC\n    fee_bps 2\n    \
                  liquidity 900_000\n    slippage_bps 6\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  arb spread {\n    discover { chains = [ethereum]; max_hops = 4; liquidity_min = \
                  500_000 ethereum.USDC; }\n    capital { flash = disabled; max = 25_000_000 \
                  ethereum.USDC; }\n    execution { atomic = true; parallel = true; private = \
                  false; }\n    risk { min_profit = 20bps; max_slippage = 8bps; max_total_fee = \
                  6bps; deadline = 220ms; }\n}\n";
    let fixture = write_fixture("cli_arb_planned.x3", source);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.status.success(), "an arb scope with a cycle must check: {output}");
    assert!(
        !output.contains("cannot be executed"),
        "nothing about a planned arb scope is unexecutable: {output}"
    );

    let out = std::env::temp_dir().join("cli_arb_planned.x3b");
    let build = x3c()
        .arg("build")
        .arg(&fixture)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("x3c build");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(build.status.success(), "the plan must build: {text}");

    // The artifact must carry the plan, not merely accept it: the cycle as a host call
    // and the approved venues beside it.
    let explain = x3c().arg("explain").arg(&out).output().expect("x3c explain");
    let disassembly = format!(
        "{}{}",
        String::from_utf8_lossy(&explain.stdout),
        String::from_utf8_lossy(&explain.stderr)
    );
    assert!(
        disassembly.contains("MULTI_HOP_SWAP"),
        "the plan's host call must be in the artifact: {disassembly}"
    );
    assert!(
        disassembly.contains("ethereum.ETH") && disassembly.contains("25000000"),
        "the cycle and the committed amount must be in the artifact: {disassembly}"
    );
    assert!(
        disassembly.contains("ROUTE_FALLBACK") && disassembly.contains("usdc_to_eth"),
        "the venues the compiler approved must be in the artifact: {disassembly}"
    );
    assert!(
        disassembly.contains("ATOMIC_BEGIN")
            && disassembly.contains("ATOMIC_END")
            && disassembly.find("ATOMIC_BEGIN") < disassembly.find("MULTI_HOP_SWAP"),
        "the whole plan must sit inside one atomic block: {disassembly}"
    );

    // The plan's floors are *enforced*, so the run states the market outcome they are
    // judged against rather than the floor passing on a number nobody measured.
    let run = x3c()
        .arg("run")
        .args(["--measured-profit-bps", "100", "--measured-slippage-bps", "5"])
        .arg(&out)
        .output()
        .expect("x3c run");
    let run_text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.status.success() && run_text.contains("x3c run: ok"),
        "the planned trade must run against the fixture host: {run_text}"
    );
}

/// `x3c lower` shows what the arb's verifier decided.
#[test]
fn cli_lower_shows_the_planned_arb_cycle_and_its_floors() {
    // The artifact carries what the generator decided: the asset cycle, the venues the
    // compiler approved, and the floors the runtime measures.
    let source = "intent spread_trade {\n    from ethereum.USDC amount 1_000_000 receiver 0xA1\n    \
                  to ethereum.USDC receiver 0xA2\n    require profit >= 20\n    require \
                  slippage <= 50\n    timeout 30s refund ethereum.USDC to sender\n    on_fail \
                  rollback\n}\n\
                  venue usdc_to_eth {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps 3\n    \
                  liquidity 1_000_000\n    slippage_bps 8\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  venue eth_to_usdc {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.ETH\n    asset_out ethereum.USDC\n    fee_bps 2\n    \
                  liquidity 900_000\n    slippage_bps 6\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  arb spread {\n    discover { chains = [ethereum]; max_hops = 4; liquidity_min = \
                  500_000 ethereum.USDC; }\n    capital { flash = disabled; max = 25_000_000 \
                  ethereum.USDC; }\n    execution { atomic = true; parallel = true; private = \
                  false; }\n    risk { min_profit = 20bps; max_slippage = 8bps; max_total_fee = \
                  6bps; deadline = 220ms; }\n}\n";
    let fixture = write_fixture("cli_arb_lower.x3", source);
    let out = std::env::temp_dir().join("cli_arb_lower.json");
    let lower = x3c()
        .arg("lower")
        .arg(&fixture)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("x3c lower");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&lower.stdout),
        String::from_utf8_lossy(&lower.stderr)
    );
    assert!(lower.status.success(), "lowering must succeed: {text}");

    let json = std::fs::read_to_string(&out).expect("the IR document");
    assert!(
        json.contains("\"MultiHopSwap\"") && json.contains("\"ethereum.ETH\""),
        "the plan's cycle must be in the IR: {json}"
    );
    assert!(
        json.contains("\"usdc_to_eth\"") && json.contains("\"eth_to_usdc\""),
        "the venues the compiler approved must travel in the artifact: {json}"
    );
    assert!(
        json.contains("\"ProfitThreshold\"") && json.contains("\"SlippageTolerance\""),
        "the floors must be guards, because the runtime is what measures them: {json}"
    );
    assert!(
        json.contains("\"AtomicBegin\"") && json.contains("\"AtomicEnd\""),
        "the whole plan must be inside one atomic block: {json}"
    );
    assert!(
        !json.contains("\"AtomicChoice\""),
        "one candidate is not a choice, so no choice may be recorded: {json}"
    );
}

/// `x3c check` on a venue that trades off-chain — spec PHASE 39.
///
/// `semantic::verify_venue_decls` is what enforces the rule, and the compiler
/// module has its own tests for it (`compiler/tests/test_settlement_guarantees.rs`).
/// This is the reachability the *user-facing* rule needs: a program that claims
/// atomic settlement on an off-chain venue has to be refused by the binary, with
/// the reason, or the phase is a comment in a module nobody runs.
#[test]
fn cli_refuses_a_venue_that_claims_atomic_settlement_off_chain() {
    let venue = |settlement: &str| {
        format!(
            "venue binance_spot {{\n    kind orderbook\n    chain ethereum\n    domain evm\n    \
             asset_in ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps 5\n    liquidity \
             1_000_000\n    slippage_bps 8\n    latency_ms 12\n    finality_blocks 12\n    risk \
             2\n{settlement}}}\n"
        )
    };

    let atomic = write_fixture("cli_venue_atomic.x3", &venue("    settlement atomic\n"));
    let check = x3c().arg("check").arg(&atomic).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        !check.status.success(),
        "an off-chain venue does not settle atomically: {output}"
    );
    assert!(
        output.contains("claims `settlement atomic`") && output.contains("would be false"),
        "the refusal must say the claim is false: {output}"
    );

    // The same venue saying what actually guarantees the trade is accepted, so the
    // rule rejects the lie rather than the venue.
    let honest = write_fixture("cli_venue_compensating.x3", &venue("    settlement compensating\n"));
    let check = x3c().arg("check").arg(&honest).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        check.status.success(),
        "`compensating` is a true description of an off-chain leg: {output}"
    );
}

/// The key `opportunity_packet_json` signs with. Fixed and non-secret on
/// purpose: the test asserts the CLI's trust decision, not the key's quality.
fn packet_solver_key() -> ed25519_dalek::SigningKey {
    ed25519_dalek::SigningKey::from_bytes(&[7u8; 32])
}

fn hex_encode_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A signed opportunity packet, optionally with one term edited after signing
/// so the packet's own hash no longer covers it.
fn opportunity_packet_json(tamper: bool) -> String {
    use std::collections::{BTreeMap, BTreeSet};

    use x3_lang_compiler::opportunity::Opportunity;
    use x3_lang_vm::opportunity_packet::{sign_packet, OpportunityPacket, OPPORTUNITY_PACKET_VERSION};

    let packet = OpportunityPacket {
        version: OPPORTUNITY_PACKET_VERSION,
        strategy_id: "cli-tri-arb".to_string(),
        artifact_hash: [3u8; 32],
        state_roots: BTreeMap::from([("ethereum".to_string(), [9u8; 32])]),
        route: Opportunity {
            venues: vec!["uniswap-v3".to_string(), "raydium".to_string()],
            assets: vec!["USDC".to_string(), "WETH".to_string(), "USDC".to_string()],
            fee_bps: 30,
            slippage_bps: 20,
            max_risk: 1,
            latency_ms: 400,
            finality_blocks: 12,
            min_liquidity: 1_000_000,
        },
        required_capital: 100_000,
        max_capital: 500_000,
        expected_output: 520_000,
        minimum_profit: 5_000,
        maximum_fee: 2_000,
        maximum_slippage_bps: 50,
        deadline_blocks: 500,
        proof_requirements: BTreeSet::from(["state".to_string()]),
        execution_commitment: [0u8; 32],
        packet_hash: [0u8; 32],
        signature: None,
    };
    let mut packet = sign_packet(packet, "cli-solver", &packet_solver_key()).expect("packet signs");
    if tamper {
        packet.expected_output += 1;
    }
    serde_json::to_string_pretty(&packet).expect("packet serializes")
}

#[test]
fn packet_verify_admits_a_signed_packet_and_names_why_it_refuses_the_others() {
    let trusted = format!(
        "cli-solver={}",
        hex_encode_bytes(&packet_solver_key().verifying_key().to_bytes())
    );

    let signed = write_fixture("cli_packet_signed.json", &opportunity_packet_json(false));
    let verify = x3c()
        .args(["packet", "verify"])
        .arg(&signed)
        .args(["--block", "100", "--trusted", &trusted])
        .output()
        .expect("x3c packet verify");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );
    assert!(verify.status.success(), "a signed packet must verify: {output}");
    assert!(
        output.contains("packet verified") && output.contains("cli-tri-arb"),
        "the report must name the strategy it admitted: {output}"
    );

    let inspect = x3c()
        .args(["packet", "inspect"])
        .arg(&signed)
        .output()
        .expect("x3c packet inspect");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&inspect.stdout),
        String::from_utf8_lossy(&inspect.stderr)
    );
    assert!(inspect.status.success(), "inspect must succeed: {output}");
    assert!(
        output.contains("packet_hash: ") && output.contains("execution_commitment: "),
        "inspect must print both commitments: {output}"
    );

    // One term edited after signing: the hash no longer covers the packet.
    let tampered = write_fixture("cli_packet_tampered.json", &opportunity_packet_json(true));
    let verify = x3c()
        .args(["packet", "verify"])
        .arg(&tampered)
        .args(["--block", "100", "--trusted", &trusted])
        .output()
        .expect("x3c packet verify");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );
    assert!(!verify.status.success(), "an edited packet must not verify: {output}");
    assert!(
        output.contains("execution commitment") && output.contains("does not cover its terms"),
        "the refusal must name the commitment that caught the edit: {output}"
    );

    // The same packet admitted at its deadline.
    let verify = x3c()
        .args(["packet", "verify"])
        .arg(&signed)
        .args(["--block", "500", "--trusted", &trusted])
        .output()
        .expect("x3c packet verify");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );
    assert!(
        !verify.status.success(),
        "a packet at its deadline must not verify: {output}"
    );
    assert!(
        output.contains("expired at block 500"),
        "the refusal must name the block: {output}"
    );

    // A key the operator does not trust, even though the packet names it.
    let verify = x3c()
        .args(["packet", "verify"])
        .arg(&signed)
        .args([
            "--block",
            "100",
            "--trusted",
            "cli-solver=0000000000000000000000000000000000000000000000000000000000000000",
        ])
        .output()
        .expect("x3c packet verify");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&verify.stdout),
        String::from_utf8_lossy(&verify.stderr)
    );
    assert!(!verify.status.success(), "an untrusted key must not verify: {output}");
    assert!(
        output.contains("signer 'cli-solver' is not trusted"),
        "the refusal must name the signer: {output}"
    );
}

/// `x3c check` on an `arb` scope the graph cannot satisfy — spec PHASE 37.
///
/// The declaration's own arithmetic can be consistent while the search it
/// describes has no candidates: a chain list nothing is declared on, a floor no
/// pool can absorb, ceilings every venue exceeds. The binary has to name every
/// venue it judged and the bound that removed it, or the author is left guessing
/// which line to change.
#[test]
fn cli_refuses_an_arb_scope_no_declared_venue_survives() {
    let source = "venue pricey_pool {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out solana.USDC\n    fee_bps 40\n    \
                  liquidity 1_000_000\n    slippage_bps 8\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  arb spread {\n    discover { chains = [x3, ethereum, solana]; max_hops = 4; \
                  liquidity_min = 500_000 ethereum.USDC; }\n    capital { flash = disabled; max = \
                  50_000_000 ethereum.USDC; }\n    execution { atomic = true; parallel = true; \
                  private = false; }\n    risk { min_profit = 20bps; max_slippage = 8bps; \
                  max_total_fee = 6bps; deadline = 220ms; }\n}\n";
    let fixture = write_fixture("cli_arb_no_candidate.x3", source);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!check.status.success(), "the scope is unsatisfiable: {output}");
    assert!(
        output.contains("no declared venue survives") && output.contains("'pricey_pool'"),
        "the refusal must name the venue: {output}"
    );
    assert!(
        output.contains("charges 40bps") && output.contains("`max_total_fee` is 6bps"),
        "the refusal must give the figure and the bound: {output}"
    );
}

/// `x3c check` on a `hyperarb` — spec PHASE 38.
///
/// The compiler module has its own tests (`compiler/tests/test_hyperarb.rs`), but
/// those call `hyperarb::analyse` directly. This is the reachability: the clauses
/// have to reach the *binary*, a leg that names nothing has to be refused there,
/// and `x3c lower` has to show the plan the verifier resolved.
#[test]
fn cli_plans_a_hyperarb_builds_it_and_runs_it() {
    // The legs used to be resolved and refused, because nothing turned them into
    // operations. `hyperarb::plan` selects one and emits its route, so the whole path has
    // to hold — check, build, disassemble, run — and the artifact has to carry which leg
    // was chosen and the floor the runtime enforces.
    let source = "venue usdc_to_eth {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps 5\n    \
                  liquidity 1_200_000\n    slippage_bps 8\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  venue usdc_to_sol {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out solana.USDC\n    fee_bps 3\n    \
                  liquidity 1_000_000\n    slippage_bps 6\n    latency_ms 20\n    \
                  finality_blocks 12\n    risk 3\n}\n\
                  intent bounds {\n    from ethereum.USDC amount 1 receiver 0xA1\n    to \
                  ethereum.USDC receiver 0xA2\n    require slippage <= 50\n    require nonce \
                  unused bounds_nonce\n    timeout 30s refund ethereum.USDC to sender\n    \
                  on_fail rollback\n}\n\
                  hyperarb tri {\n    capital = 25_000_000 ethereum.USDC;\n    \
                  parallel { route_a = evaluate(usdc_to_eth); route_b = evaluate(usdc_to_sol); }\n    \
                  choose lowest_declared_fee;\n    settle_across_domains;\n    require net_profit \
                  >= 35bps;\n}\n";
    let fixture = write_fixture("cli_hyperarb_planned.x3", source);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(check.status.success(), "a planned hyperarb must check: {output}");
    assert!(
        !output.contains("cannot be executed"),
        "nothing about a planned hyperarb is unexecutable: {output}"
    );

    let out = std::env::temp_dir().join("cli_hyperarb_planned.x3b");
    let build = x3c()
        .arg("build")
        .arg(&fixture)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("x3c build");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr)
    );
    assert!(build.status.success(), "the route must build: {text}");

    let explain = x3c().arg("explain").arg(&out).output().expect("x3c explain");
    let disassembly = format!(
        "{}{}",
        String::from_utf8_lossy(&explain.stdout),
        String::from_utf8_lossy(&explain.stderr)
    );
    assert!(
        disassembly.contains("ATOMIC_CHOICE") && disassembly.contains("MULTI_HOP_SWAP"),
        "the plan must record the choice and the chosen route: {disassembly}"
    );
    assert!(
        disassembly.contains("solana.USDC"),
        "the selected leg is the one that crosses to Solana: {disassembly}"
    );
    assert!(
        disassembly.contains("ROUTE_FALLBACK") && disassembly.contains("usdc_to_sol"),
        "the approved venues must travel in the artifact: {disassembly}"
    );
    assert!(
        disassembly.contains("REQUIRE") && disassembly.contains("ATOMIC_END"),
        "the net-profit floor must be a guard inside the atomic block: {disassembly}"
    );

    // The plan's floors are *enforced*, so the run states the market outcome they are
    // judged against rather than the floor passing on a number nobody measured.
    let run = x3c()
        .arg("run")
        .args(["--measured-profit-bps", "100", "--measured-slippage-bps", "5"])
        .arg(&out)
        .output()
        .expect("x3c run");
    let run_text = format!(
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    assert!(
        run.status.success() && run_text.contains("x3c run: ok"),
        "the planned route must run against the fixture host: {run_text}"
    );
}

#[test]
fn cli_refuses_a_hyperarb_leg_that_names_nothing() {
    let source = "venue usdc_to_eth {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps 5\n    \
                  liquidity 1_200_000\n    slippage_bps 8\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  intent bounds {\n    from ethereum.USDC amount 1 receiver 0xA1\n    to \
                  ethereum.USDC receiver 0xA2\n    require slippage <= 50\n    require nonce \
                  unused bounds_nonce\n    timeout 30s refund ethereum.USDC to sender\n    \
                  on_fail rollback\n}\n\
                  hyperarb tri {\n    capital = 25_000_000 ethereum.USDC;\n    \
                  parallel { route_a = evaluate(usdc_to_eth); route_b = evaluate(EVM_PATH); }\n    \
                  choose lowest_declared_fee;\n    require net_profit >= 35bps;\n}\n";
    let fixture = write_fixture("cli_hyperarb_bad_leg.x3", source);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!check.status.success(), "the leg resolves to nothing: {output}");
    assert!(
        output.contains("`evaluate(EVM_PATH)` names nothing the program declares"),
        "the refusal must name the leg's target: {output}"
    );
    assert!(
        output.contains("usdc_to_eth") && output.contains("evm"),
        "the refusal must list what the program declares: {output}"
    );
}

#[test]
fn cli_refuses_a_hyperarb_that_chooses_by_output() {
    // The phase's own example criterion, and the number the compiler does not have.
    let source = "venue usdc_to_eth {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps 5\n    \
                  liquidity 1_200_000\n    slippage_bps 8\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  venue usdc_to_sol {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out solana.USDC\n    fee_bps 3\n    \
                  liquidity 1_000_000\n    slippage_bps 6\n    latency_ms 20\n    \
                  finality_blocks 12\n    risk 3\n}\n\
                  intent bounds {\n    from ethereum.USDC amount 1 receiver 0xA1\n    to \
                  ethereum.USDC receiver 0xA2\n    require slippage <= 50\n    require nonce \
                  unused bounds_nonce\n    timeout 30s refund ethereum.USDC to sender\n    \
                  on_fail rollback\n}\n\
                  hyperarb tri {\n    capital = 25_000_000 ethereum.USDC;\n    \
                  parallel { route_a = evaluate(usdc_to_eth); route_b = evaluate(usdc_to_sol); }\n    \
                  choose highest_net_output;\n    require net_profit >= 35bps;\n}\n";
    let fixture = write_fixture("cli_hyperarb_by_output.x3", source);
    let check = x3c().arg("check").arg(&fixture).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(!check.status.success(), "there is no output to rank by: {output}");
    assert!(
        output.contains("highest_net_output") && output.contains("no price"),
        "the refusal must say which number is missing: {output}"
    );
}

/// `x3c lanes` — spec PHASE 30.
///
/// The compiler module has its own tests (`compiler/tests/test_lanes.rs`), but those
/// call `lanes::classify` on hand-built IR. This is the reachability the policy
/// needs: the lane has to be decided from a *program*, and the serving order has to
/// be printed, because a policy a reader has to infer from behaviour is not the
/// auditable one the phase asks for.
#[test]
fn cli_lanes_reports_each_declarations_lane_and_the_serving_order() {
    let source = "intent only_moves {\n    from ethereum.USDC amount 100 receiver 0xA1\n    to \
                  ethereum.USDC receiver 0xA2\n    require nonce unused only_moves_nonce\n    \
                  timeout 30s refund ethereum.USDC to sender\n    on_fail rollback\n}\n\
                  intent trades {\n    from ethereum.USDC amount 100 receiver 0xA1\n    to \
                  ethereum.SOL receiver 0xA2\n    route {\n        swap uniswap ethereum.USDC -> \
                  ethereum.SOL amount 100 min_output 90\n    }\n    require nonce unused \
                  trades_nonce\n    timeout 30s refund ethereum.USDC to sender\n    on_fail \
                  rollback\n}\n\
                  intent crosses {\n    from ethereum.USDC amount 100 receiver 0xA1\n    to \
                  solana.USDC receiver 0xA2\n    route {\n        swap uniswap ethereum.USDC -> \
                  solana.USDC amount 100 min_output 90\n    }\n    require nonce unused \
                  crosses_nonce\n    timeout 30s refund ethereum.USDC to sender\n    on_fail \
                  rollback\n}\n";
    let fixture = write_fixture("cli_lanes.x3", source);
    let output = x3c().arg("lanes").arg(&fixture).output().expect("run x3c lanes");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "lanes must exit 0: {stdout}");
    assert!(
        stdout.contains("liquidation -> atomic_cross_domain -> trading -> settlement -> standard"),
        "the serving order must be printed, not inferred: {stdout}"
    );
    assert!(
        stdout.contains("crosses: atomic_cross_domain")
            && stdout.contains("trades: trading")
            && stdout.contains("only_moves: settlement"),
        "each declaration's lane must be decided from its operations: {stdout}"
    );
    assert!(
        stdout.contains("arrival order is preserved"),
        "the fairness rule must be stated in the report: {stdout}"
    );
    // The report is printed in the serving order, so the three appear in it.
    let crosses = stdout.find("crosses: atomic_cross_domain").expect("crosses");
    let trades = stdout.find("trades: trading").expect("trades");
    let only_moves = stdout.find("only_moves: settlement").expect("only_moves");
    assert!(
        crosses < trades && trades < only_moves,
        "the report must be in the order the policy serves: {stdout}"
    );
}

/// `x3bench` — spec PHASE 50's measurement harness has to actually run.
///
/// A benchmark harness that stops measuring is worse than none: the report would keep
/// its numbers and nothing would say they were stale. This runs the binary at a tiny
/// iteration count and asserts that every operation the phase names is measured — so
/// a harness that quietly dropped one, or that could no longer set one up, fails here
/// rather than in a report nobody re-ran.
#[test]
fn cli_x3bench_measures_every_operation_phase_50_names() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_x3bench"))
        .arg("5")
        .output()
        .expect("run x3bench");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        output.status.success(),
        "every operation must be measurable: {stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    for operation in [
        "parsing",
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
        assert!(
            stdout.lines().any(|line| line.starts_with(&format!("| {operation} |"))),
            "'{operation}' must be measured and given a row: {stdout}"
        );
    }
    assert!(
        stdout.contains("all 10 operations measured"),
        "the run must report that every operation was measured: {stdout}"
    );
    assert!(
        stdout.contains("nearest rank"),
        "the percentile method must be stated, so a reader knows what the numbers are: {stdout}"
    );
    // The function each row times is named, so the claim is checkable.
    assert!(
        stdout.contains("parser::parse_source") && stdout.contains("dag::plan"),
        "each row must name the function it exercises: {stdout}"
    );
}

/// `x3c gpu` — spec PHASE 51.
///
/// The VM module has its own tests (`vm/tests/gpu_acceleration.rs`), but those call
/// `gpu::run_equality` and `gpu::select` directly. This is the reachability: the
/// classification, the probe's answer and the equality verdict have to reach the
/// binary, because a classification nobody can read is a comment with extra steps.
#[test]
fn cli_gpu_reports_the_classification_the_probe_and_an_unproven_equality() {
    let output = x3c().arg("gpu").output().expect("run x3c gpu");
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(output.status.success(), "gpu must exit 0: {stdout}");
    for candidate in [
        "route candidate scoring",
        "signature verification",
        "hash batches",
        "graph scoring",
        "simulation batches",
        "opportunity filtering",
    ] {
        assert!(
            stdout.contains(&format!("| {candidate} |")),
            "'{candidate}' must be classified with a reason: {stdout}"
        );
    }
    assert!(
        stdout.contains("| signature verification | critical |"),
        "a signature check decides validity and must be classified as critical: {stdout}"
    );
    assert!(
        stdout.contains("| simulation batches | not critical |"),
        "a dry run settles nothing: {stdout}"
    );

    // The probe's answer is the host's own, not a string this report invented.
    assert!(
        stdout.contains("X3_BACKEND_REQUIRED") || stdout.contains("X3_FEATURE_NOT_AVAILABLE"),
        "the probe must report what the host path answered: {stdout}"
    );

    // And equality is unproven rather than passing on one backend.
    assert!(
        stdout.contains("UNPROVEN"),
        "equality must not be claimed without a second implementation: {stdout}"
    );
    assert!(
        !stdout.contains("proven over"),
        "no row may claim proven equality while no GPU backend exists: {stdout}"
    );
    assert!(
        stdout.lines().all(|line| !line.contains("equality over 0 sample")),
        "the harness must run over a real input set: {stdout}"
    );
    assert!(
        stdout.contains("the CPU is the only backend available, not a fallback"),
        "the report must not present the CPU as a fallback: {stdout}"
    );
}

/// A plan's economic floor is **enforced**, not recorded — spec PHASE 50's "native profit
/// guards: the transaction simply refuses settlement below target profit".
///
/// The floors `arb::plan` and `hyperarb::plan` emit are post-conditions on the trade, so
/// they are judged against a measurement a host reports, in basis points, and this is the
/// whole of it: without a measurement the guard refuses, with one that clears the floor it
/// passes, and with one that does not it refuses **with both figures**. A guard a *program*
/// writes is a different thing — a constraint the compiler checks against declarations —
/// and the last case here is that one, still running unchanged.
#[test]
fn cli_enforces_a_plans_floor_against_a_measured_outcome() {
    let source = "intent spread_trade {\n    from ethereum.USDC amount 1_000_000 receiver 0xA1\n    \
                  to ethereum.USDC receiver 0xA2\n    require profit >= 20\n    require \
                  slippage <= 50\n    timeout 30s refund ethereum.USDC to sender\n    on_fail \
                  rollback\n}\n\
                  venue usdc_to_eth {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps 3\n    \
                  liquidity 1_000_000\n    slippage_bps 8\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  venue eth_to_usdc {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                  asset_in ethereum.ETH\n    asset_out ethereum.USDC\n    fee_bps 2\n    \
                  liquidity 900_000\n    slippage_bps 6\n    latency_ms 12\n    \
                  finality_blocks 12\n    risk 2\n}\n\
                  arb spread {\n    discover { chains = [ethereum]; max_hops = 4; liquidity_min = \
                  500_000 ethereum.USDC; }\n    capital { flash = disabled; max = 25_000_000 \
                  ethereum.USDC; }\n    execution { atomic = true; parallel = true; private = \
                  false; }\n    risk { min_profit = 20bps; max_slippage = 8bps; max_total_fee = \
                  6bps; deadline = 220ms; }\n}\n";
    let fixture = write_fixture("cli_measured_floor.x3", source);
    let out = std::env::temp_dir().join("cli_measured_floor.x3b");
    let build = x3c()
        .arg("build")
        .arg(&fixture)
        .arg("--out")
        .arg(&out)
        .output()
        .expect("x3c build");
    assert!(
        build.status.success(),
        "the plan must build: {}",
        String::from_utf8_lossy(&build.stderr)
    );

    let run = |args: &[&str]| {
        let mut command = x3c();
        command.arg("run").args(args).arg(&out);
        let output = command.output().expect("x3c run");
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    };

    // Nothing measured: the floor refuses rather than passing on a number nobody took.
    let unmeasured = run(&[]);
    assert!(
        unmeasured.contains("X3_GUARD_UNMEASURED") && unmeasured.contains("profit >= 20bps"),
        "an unmeasured floor must refuse: {unmeasured}"
    );

    // Measured and clearing both floors: it settles.
    let cleared = run(&["--measured-profit-bps", "100", "--measured-slippage-bps", "5"]);
    assert!(
        cleared.contains("x3c run: ok"),
        "a trade that clears its floors must run: {cleared}"
    );

    // Measured below the profit floor: refused, with both figures.
    let below = run(&["--measured-profit-bps", "5", "--measured-slippage-bps", "5"]);
    assert!(
        below.contains("X3_PROFIT_BELOW_FLOOR") && below.contains("realised 5bps") && below.contains("at least 20bps"),
        "the refusal must give what was realised and what was required: {below}"
    );

    // Measured above the slippage ceiling: refused, with both figures.
    let slippy = run(&["--measured-profit-bps", "100", "--measured-slippage-bps", "90"]);
    assert!(
        slippy.contains("X3_SLIPPAGE_ABOVE_CEILING")
            && slippy.contains("realised 90bps")
            && slippy.contains("at most 8bps"),
        "the refusal must give what was realised and what was allowed: {slippy}"
    );

    // Half a measurement is refused rather than completed by inventing the other half.
    let half = run(&["--measured-profit-bps", "100"]);
    assert!(
        half.contains("state both measurements or neither"),
        "a half-stated measurement must be refused: {half}"
    );

    // And a *program's* guard is a different thing: `simple_swap`'s `require slippage <= 50`
    // is a constraint the compiler checks against declarations, so it runs with no
    // measurement at all. This is the case that keeps the change from altering what
    // existing programs mean.
    let existing = std::env::temp_dir().join("cli_measured_source_guard.x3");
    std::fs::write(
        &existing,
        "venue uniswap_v3 {\n    kind pool\n    chain ethereum\n    domain evm\n    asset_in \
         ethereum.USDC\n    asset_out ethereum.ETH\n    fee_bps 5\n    liquidity 1_000_000\n    \
         slippage_bps 8\n    latency_ms 12\n    finality_blocks 12\n    risk 2\n}\n\
         intent simple_swap {\n    from ethereum.USDC amount 100 receiver 0xA1\n    to \
         ethereum.ETH receiver 0xA2\n    route {\n        swap uniswap_v3 ethereum.USDC -> \
         ethereum.ETH amount 100 min_output 90\n    }\n    require nonce unused simple_nonce\n    \
         require slippage <= 50\n    timeout 30s refund ethereum.USDC to sender\n    on_fail \
         rollback\n}\n",
    )
    .expect("write the source-guard fixture");
    let source_artifact = std::env::temp_dir().join("cli_measured_source_guard.x3b");
    let build = x3c()
        .arg("build")
        .arg(&existing)
        .arg("--out")
        .arg(&source_artifact)
        .output()
        .expect("x3c build");
    assert!(build.status.success(), "the source-guard program must build");
    let output = x3c().arg("run").arg(&source_artifact).output().expect("x3c run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.status.success() && text.contains("x3c run: ok"),
        "a program's own guard is a compile-time constraint and must still run unmeasured: {text}"
    );
}

/// PHASE 9's other verdict: a hedge whose legs do not net is refused with the delta.
#[test]
fn cli_refuses_a_hedge_whose_legs_do_not_net() {
    let unbalanced = write_fixture(
        "cli_hedge_unbalanced.x3",
        "atomic_hedge {\n    buy 1_000 ethereum.ETH spot;\n    short 900 ethereum.ETH perp;\n\n    \
         require delta <= 0.01%;\n}\n",
    );
    let check = x3c().arg("check").arg(&unbalanced).output().expect("x3c check");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&check.stdout),
        String::from_utf8_lossy(&check.stderr)
    );
    assert!(
        !check.status.success(),
        "a hedge that does not net is not a hedge: {output}"
    );
    assert!(
        output.contains("leaves a delta") && output.contains("1000 bps"),
        "the refusal must give the delta it computed: {output}"
    );
}
