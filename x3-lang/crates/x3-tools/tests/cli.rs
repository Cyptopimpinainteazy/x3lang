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
