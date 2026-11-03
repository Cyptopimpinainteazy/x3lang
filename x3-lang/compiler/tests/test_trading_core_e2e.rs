//! End-to-end Trading Core v1 pipeline: parse -> type -> verify -> lower ->
//! execute through an explicitly marked fixture host -> verify receipt.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::SigningKey;

use x3_lang_ast::Item;
use x3_lang_compiler::emitter::decode_trading_program;
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::{
    analyze_trading, check_source_with_mode, compile_program, lower_atomic_trade, verify_trading_program,
    CompilationMode,
};
use x3_lang_vm::trading::{
    build_receipt, fixture_manifest, sign_receipt, verify_receipt_trusted, BorrowRequest, BorrowResult, BridgeRequest,
    BridgeTransferResult, CapabilityManifest, CommittedCost, ExecutionMode, HostError, QuoteRequest, QuoteResult,
    RepayRequest, RepayResult, SwapRequest, SwapResult, TradeExecutionContext, TradeOutcome, TradingHost, TradingVm,
};

const SOURCE: &str = include_str!("../../examples/trading_core_v1.x3");
const COMMITMENT: [u8; 32] = [11u8; 32];

struct FixtureVenueHost {
    manifest: CapabilityManifest,
}

impl FixtureVenueHost {
    fn new() -> Self {
        let mut manifest = fixture_manifest(COMMITMENT);
        manifest.providers = BTreeSet::from(["aave_v3".to_string()]);
        manifest.venues = BTreeSet::from(["uniswap_v3".to_string(), "sushiswap".to_string()]);
        manifest.bridges = BTreeSet::from(["wormhole".to_string()]);
        // MainnetArb (examples/trading_core_v1.x3) declares require_private_submission:
        // true. This fixture claims that capability so the happy-path test can exercise
        // a genuine full commit; the negative case (fixture that does NOT claim it) is
        // already covered by mainnet_audit_reports_missing_private_submission_capability.
        manifest.private_submission = true;
        Self { manifest }
    }
}

impl TradingHost for FixtureVenueHost {
    fn capabilities(&self) -> &CapabilityManifest {
        &self.manifest
    }

    fn open_debt(&mut self, request: BorrowRequest) -> Result<BorrowResult, HostError> {
        Ok(BorrowResult {
            asset: request.asset,
            principal: request.principal,
            fee: 0,
            state_commitment: COMMITMENT,
        })
    }

    fn quote(&self, request: QuoteRequest) -> Result<QuoteResult, HostError> {
        // Slippage enforcement isn't what this E2E test exercises; a floor
        // of 0 always satisfies `actual >= expected`, so it never fires.
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
            state_commitment: COMMITMENT,
        })
    }

    fn close_debt(&mut self, request: RepayRequest) -> Result<RepayResult, HostError> {
        Ok(RepayResult {
            debt_id: request.debt_id,
            asset: request.asset,
            amount_paid: request.amount,
            fee: 0,
            state_commitment: COMMITMENT,
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
            state_commitment: COMMITMENT,
        })
    }
}

fn execution_context(mode: ExecutionMode) -> TradeExecutionContext {
    TradeExecutionContext { mode, current_block: 1 }
}

#[test]
fn trading_core_v1_pipeline_executes_and_verifies_receipt() {
    let program = parse_source(SOURCE).expect("example must parse");
    let symbols = analyze_trading(&program, CompilationMode::Dev).expect("example must type-check");
    assert!(verify_trading_program(&program, &symbols, CompilationMode::Dev).is_empty());

    let trade = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            Item::AtomicTrade(trade) => Some(trade),
            _ => None,
        })
        .expect("example must contain an atomic trade");
    let operations = lower_atomic_trade(trade, &symbols).expect("example must lower");
    assert_eq!(operations.len(), 9);

    let bytecode = compile_program(&program).expect("example must compile to bytecode");
    assert!(!bytecode.is_empty() && bytecode.len() % 4 == 0);

    let decoded = decode_trading_program(&bytecode).expect("emitted trading bytecode must decode");
    assert_eq!(decoded.len(), 9, "every trading operation must survive encode/decode");

    let mut vm = TradingVm::new();
    let mut host = FixtureVenueHost::new();
    let execution = vm
        .execute_atomic(&decoded, &mut host, execution_context(ExecutionMode::Development))
        .expect("decoded-bytecode fixture execution must commit");

    let settlement = x3_lang_compiler::AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: "0xA0b8".to_string(),
        symbol: "USDC".to_string(),
        decimals: 6,
    };
    let receipt = build_receipt(
        env!("CARGO_PKG_VERSION"),
        [5u8; 32],
        trade.name.as_str(),
        trade.risk_policy.as_str(),
        COMMITMENT,
        &decoded,
        &execution.committed_state,
        Some(&settlement),
        TradeOutcome::Success,
    )
    .expect("receipt must build");
    let signing_key = SigningKey::from_bytes(&[9u8; 32]);
    let receipt = sign_receipt(receipt, "e2e-executor", &signing_key).expect("receipt must sign");
    let trusted = BTreeMap::from([("e2e-executor".to_string(), signing_key.verifying_key().to_bytes())]);
    verify_receipt_trusted(&receipt, &trusted).expect("signed receipt must verify");
    assert!(receipt
        .realized_net_profit
        .as_ref()
        .is_some_and(|profit| profit.amount > 0));
}

const BRIDGE_SOURCE: &str = r#"
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

#[test]
fn bridge_pipeline_executes_and_verifies_receipt() {
    // Full pipeline, real bytecode round-trip: proves TRADING_BRIDGE
    // actually survives compile -> emit -> decode (this is exactly the
    // opcode that would have desynced the disassembler before the
    // is_payload_opcode range fix), and that the whole execute -> receipt
    // -> sign -> verify chain works for a trade that crosses chains.
    let program = parse_source(BRIDGE_SOURCE).expect("bridge example must parse");
    let symbols = analyze_trading(&program, CompilationMode::Dev).expect("bridge example must type-check");
    assert!(verify_trading_program(&program, &symbols, CompilationMode::Dev).is_empty());

    let trade = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            Item::AtomicTrade(trade) => Some(trade),
            _ => None,
        })
        .expect("bridge example must contain an atomic trade");
    let operations = lower_atomic_trade(trade, &symbols).expect("bridge example must lower");
    assert_eq!(operations.len(), 10);

    let bytecode = compile_program(&program).expect("bridge example must compile to bytecode");
    assert!(!bytecode.is_empty() && bytecode.len() % 4 == 0);

    let decoded = decode_trading_program(&bytecode).expect("emitted bridge bytecode must decode");
    assert_eq!(
        decoded.len(),
        10,
        "every trading operation, including Bridge, must survive encode/decode"
    );
    assert!(
        decoded
            .iter()
            .any(|op| matches!(op, x3_lang_compiler::ir::TradingOperation::Bridge { .. })),
        "the decoded program must still contain the Bridge operation"
    );

    let mut vm = TradingVm::new();
    let mut host = FixtureVenueHost::new();
    let execution = vm
        .execute_atomic(&decoded, &mut host, execution_context(ExecutionMode::Development))
        .expect("decoded bridge bytecode must execute and commit");

    let settlement = x3_lang_compiler::AssetKey {
        vm_family: "evm".to_string(),
        chain: "base".to_string(),
        canonical_id: "0xB1a0".to_string(),
        symbol: "USDC_BASE".to_string(),
        decimals: 6,
    };
    let receipt = build_receipt(
        env!("CARGO_PKG_VERSION"),
        [5u8; 32],
        trade.name.as_str(),
        trade.risk_policy.as_str(),
        COMMITMENT,
        &decoded,
        &execution.committed_state,
        Some(&settlement),
        TradeOutcome::Success,
    )
    .expect("bridge receipt must build");
    let signing_key = SigningKey::from_bytes(&[9u8; 32]);
    let receipt = sign_receipt(receipt, "e2e-executor", &signing_key).expect("bridge receipt must sign");
    let trusted = BTreeMap::from([("e2e-executor".to_string(), signing_key.verifying_key().to_bytes())]);
    verify_receipt_trusted(&receipt, &trusted).expect("signed bridge receipt must verify");
    assert!(receipt
        .realized_net_profit
        .as_ref()
        .is_some_and(|profit| profit.amount > 0));
}

#[test]
fn production_mode_rejects_fixture_capabilities() {
    let program = parse_source(SOURCE).expect("example must parse");
    let symbols = analyze_trading(&program, CompilationMode::Dev).expect("example must type-check");
    let trade = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            Item::AtomicTrade(trade) => Some(trade),
            _ => None,
        })
        .expect("example must contain an atomic trade");
    let operations: Vec<_> = lower_atomic_trade(trade, &symbols)
        .expect("example must lower")
        .into_iter()
        .map(|operation| match operation {
            x3_lang_compiler::Operation::Trading(trading) => trading,
            _ => panic!("unexpected operation"),
        })
        .collect();
    let mut vm = TradingVm::new();
    let mut host = FixtureVenueHost::new();
    let error = vm
        .execute_atomic(&operations, &mut host, execution_context(ExecutionMode::Production))
        .expect_err("production mode must reject fixture capabilities");
    assert_eq!(error, x3_lang_vm::trading::TradingExecError::NonProductionCapability);
}

#[test]
fn mainnet_audit_reports_missing_private_submission_capability() {
    let (_, _, errors) = check_source_with_mode(SOURCE, CompilationMode::Mainnet).expect("check must parse");
    assert!(
        errors
            .iter()
            .any(|error| error.to_string().contains("private-submission")),
        "mainnet audit must report the missing private-submission capability: {errors:?}"
    );
}

/// Same shape as the canonical fixture but `require_private_submission:
/// false`, so a mainnet-mode check has nothing legitimate left to fail on
/// — isolating the regression below from the pre-existing, correct
/// private-submission check covered above.
const MAINNET_CLEAN_SOURCE: &str = r#"
asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }
asset WETH = evm.ethereum.0xC02a { decimals: 18 }
asset ETH = evm.ethereum.0x0000000000000000000000000000000000000000 { decimals: 18 }

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
fn mainnet_mode_does_not_demand_bridge_infrastructure_from_a_single_chain_trade() {
    // Regression test for a real bug: verify_mainnet_safe's RPC-consensus/
    // relayer-attestation/solver-bond checks used to require their
    // corresponding Operation variant to be present *unconditionally*,
    // unlike every other check in verify_mainnet_safe (e.g.
    // verify_refund_path_exists), which only fires when the program
    // actually contains a bridge/cross-chain operation. Since Trading
    // Core v1 lowers entirely into Operation::Trading(..) and has no
    // concept of an RPC/relayer/solver layer at all, this meant a
    // correctly-hardened trading-core-v1 program could never pass
    // `--mode mainnet` — regardless of how safe the trade itself was.
    let (_, _, errors) =
        check_source_with_mode(MAINNET_CLEAN_SOURCE, CompilationMode::Mainnet).expect("check must parse");
    assert!(
        errors.is_empty(),
        "a well-formed trading-core-v1 program must pass mainnet mode cleanly: {errors:?}"
    );
}

#[test]
fn mainnet_mode_still_demands_bridge_infrastructure_from_a_real_bridge_intent() {
    // The other side of the same fix: a program that *does* use general-VM
    // cross-chain operations must still be held to the full RPC/relayer/
    // solver-bond bar. Proves the fix narrowed the check's scope rather
    // than disabling it.
    let source = r#"intent arb_solana_eth {
    from Ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
    to Solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    route {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 777
    }
    timeout 30s refund ethereum.USDC to sender
    on_fail rollback
}
"#;
    let (_, _, errors) = check_source_with_mode(source, CompilationMode::Mainnet).expect("check must parse");
    let all_msgs: String = errors.iter().map(|e| e.to_string()).collect::<Vec<_>>().join("\n");
    assert!(
        all_msgs.contains("no RPC consensus declared"),
        "a program with a real swap operation must still require RPC consensus: {all_msgs}"
    );
    assert!(
        all_msgs.contains("no relayer attestation declared"),
        "a program with a real swap operation must still require relayer attestation: {all_msgs}"
    );
    assert!(
        all_msgs.contains("missing solver bond declaration"),
        "a program with a real swap operation must still require a solver bond: {all_msgs}"
    );
}

#[test]
fn malformed_trading_bytecode_fails_closed_before_execution() {
    let program = parse_source(SOURCE).expect("example must parse");
    let mut bytecode = compile_program(&program).expect("example must compile");
    let trading_start = bytecode
        .iter()
        .position(|byte| *byte == x3_lang_compiler::spec::opcodes::TRADING_BEGIN)
        .expect("compiled artifact must contain a trading opcode");
    bytecode[trading_start + 1] = 0xff;
    bytecode[trading_start + 2] = 0xff;

    assert!(
        decode_trading_program(&bytecode).is_err(),
        "malformed trading payload length must fail closed"
    );
}
