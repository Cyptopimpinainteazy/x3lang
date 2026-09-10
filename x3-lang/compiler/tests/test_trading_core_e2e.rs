//! End-to-end Trading Core v1 pipeline: parse -> type -> verify -> lower ->
//! execute through an explicitly marked fixture host -> verify receipt.

use std::collections::BTreeSet;

use x3_lang_ast::Item;
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::{
    analyze_trading, check_source_with_mode, compile_program, lower_atomic_trade, verify_trading_program,
    CompilationMode,
};
use x3_lang_vm::trading::{
    build_receipt, fixture_manifest, verify_receipt, BorrowRequest, BorrowResult, CapabilityManifest, CommittedCost,
    ExecutionLimits, ExecutionMode, HostError, RepayRequest, RepayResult, SwapRequest, SwapResult,
    TradeExecutionContext, TradeOutcome, TradingHost, TradingVm,
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
}

fn execution_context(mode: ExecutionMode) -> TradeExecutionContext {
    TradeExecutionContext {
        mode,
        limits: ExecutionLimits {
            max_flash_fee_bps: 10,
            max_gas: 1_000_000_000,
            minimum_net_profit: Some(1_000_000_000),
        },
        current_block: 1,
        deadline_block: 2,
    }
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

    let mut vm = TradingVm::new();
    let mut host = FixtureVenueHost::new();
    let execution = vm
        .execute_atomic(
            operations
                .iter()
                .map(|operation| match operation {
                    x3_lang_compiler::Operation::Trading(trading) => trading.clone(),
                    _ => panic!("lowered trading sequence only contains trading operations"),
                })
                .collect::<Vec<_>>()
                .as_slice(),
            &mut host,
            execution_context(ExecutionMode::Development),
        )
        .expect("fixture execution must commit");

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
        &operations
            .iter()
            .map(|operation| match operation {
                x3_lang_compiler::Operation::Trading(trading) => trading.clone(),
                _ => panic!("unexpected operation"),
            })
            .collect::<Vec<_>>(),
        &execution.committed_state,
        Some(&settlement),
        TradeOutcome::Success,
    )
    .expect("receipt must build");
    verify_receipt(&receipt).expect("receipt must verify");
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
