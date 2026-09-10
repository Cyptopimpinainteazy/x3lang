//! Atomic trading execution, guard, and rollback tests.

use std::collections::BTreeSet;

use x3_lang_compiler::ir::{AssetKey, TradingOperation, ValueRef};
use x3_lang_vm::trading::{
    fixture_manifest, BorrowRequest, BorrowResult, CapabilityManifest, CapabilityMode, CommittedCost, ExecutionLimits,
    ExecutionMode, HostError, RepayRequest, RepayResult, SwapRequest, SwapResult, TradeExecutionContext, TradingHost,
    TradingVm,
};

const COMMITMENT: [u8; 32] = [7u8; 32];

fn asset(symbol: &str) -> AssetKey {
    AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: format!("0x{symbol}"),
        symbol: symbol.to_string(),
        decimals: 6,
    }
}

fn manifest() -> CapabilityManifest {
    let mut manifest = fixture_manifest(COMMITMENT);
    manifest.providers.insert("aave_v3".to_string());
    manifest.venues.insert("uniswap_v3".to_string());
    manifest
}

struct FixtureHost {
    manifest: CapabilityManifest,
    swap_output: u128,
    borrow_fee: u128,
    commitment: [u8; 32],
}

impl FixtureHost {
    fn new() -> Self {
        Self {
            manifest: manifest(),
            swap_output: 2_000_000,
            borrow_fee: 0,
            commitment: COMMITMENT,
        }
    }
}

impl TradingHost for FixtureHost {
    fn capabilities(&self) -> &CapabilityManifest {
        &self.manifest
    }

    fn open_debt(&mut self, request: BorrowRequest) -> Result<BorrowResult, HostError> {
        Ok(BorrowResult {
            asset: request.asset,
            principal: request.principal,
            fee: self.borrow_fee,
            state_commitment: self.commitment,
        })
    }

    fn swap(&mut self, request: SwapRequest) -> Result<SwapResult, HostError> {
        Ok(SwapResult {
            from: request.from,
            to: request.to.clone(),
            input: request.input,
            output: self.swap_output,
            fee: 0,
            fee_asset: request.to,
            state_commitment: self.commitment,
        })
    }

    fn close_debt(&mut self, request: RepayRequest) -> Result<RepayResult, HostError> {
        Ok(RepayResult {
            debt_id: request.debt_id,
            asset: request.asset,
            amount_paid: request.amount,
            fee: 0,
            state_commitment: self.commitment,
        })
    }

    fn execution_costs(&self) -> Result<Vec<CommittedCost>, HostError> {
        Ok(Vec::new())
    }
}

fn ops() -> Vec<TradingOperation> {
    vec![
        TradingOperation::BeginAtomicTrade {
            trade_id: "T".to_string(),
            policy_id: "P".to_string(),
        },
        TradingOperation::OpenDebt {
            debt_id: "debt".to_string(),
            provider: "aave_v3".to_string(),
            asset: asset("USDC"),
            principal: 1_000_000,
        },
        TradingOperation::ExecuteSwap {
            binding: "weth".to_string(),
            venue: "uniswap_v3".to_string(),
            from: asset("USDC"),
            to: asset("WETH"),
            input: ValueRef::Binding("debt.amount".to_string()),
            min_output: 1_000_000,
        },
        TradingOperation::ExecuteSwap {
            binding: "returned".to_string(),
            venue: "uniswap_v3".to_string(),
            from: asset("WETH"),
            to: asset("USDC"),
            input: ValueRef::Binding("weth".to_string()),
            min_output: 1_000_000,
        },
        TradingOperation::CloseDebt {
            debt_id: "debt".to_string(),
        },
        TradingOperation::AssertMinNetProfit {
            settlement_asset: asset("USDC"),
            minimum: 1_000_000,
        },
        TradingOperation::AssertAllDebtsClosed,
        TradingOperation::EmitTradeReceipt,
        TradingOperation::CommitAtomicTrade,
    ]
}

fn context(mode: ExecutionMode) -> TradeExecutionContext {
    TradeExecutionContext {
        mode,
        limits: ExecutionLimits {
            max_flash_fee_bps: 10,
            max_gas: 1_000_000,
            minimum_net_profit: None,
        },
        current_block: 1,
        deadline_block: 10,
    }
}

#[test]
fn successful_trade_commits_and_closes_debt() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    let execution = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect("well-formed trade must commit");
    assert!(execution.committed_state.committed);
    assert!(execution.committed_state.open_debts.is_empty());
    assert!(execution.committed_state.receipt_emitted);
}

#[test]
fn output_below_min_out_rolls_back() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.swap_output = 1;
    let before = vm.trading_state.clone();
    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("below-min_out output must abort");
    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::OutputBelowMinOut { .. }
    ));
    assert_eq!(vm.trading_state, before);
}

#[test]
fn excessive_flash_fee_rolls_back() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.borrow_fee = 100_000; // 10,000 bps of a 1,000,000 principal
    let before = vm.trading_state.clone();
    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("fee above ceiling must abort");
    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::FeeCeilingExceeded { .. }
    ));
    assert_eq!(vm.trading_state, before);
}

#[test]
fn expired_deadline_aborts_before_execution() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    let before = vm.trading_state.clone();
    let mut ctx = context(ExecutionMode::Development);
    ctx.current_block = 11;
    let err = vm
        .execute_atomic(&ops(), &mut host, ctx)
        .expect_err("expired deadline must abort");
    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::DeadlineExpired { .. }
    ));
    assert_eq!(vm.trading_state, before);
}

#[test]
fn state_commitment_mismatch_rolls_back() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.commitment = [9u8; 32];
    let before = vm.trading_state.clone();
    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("state commitment mismatch must abort");
    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::StateCommitmentMismatch
    ));
    assert_eq!(vm.trading_state, before);
}

#[test]
fn unknown_venue_is_rejected_before_execution() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.manifest.venues.clear();
    let before = vm.trading_state.clone();
    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("unknown venue must abort");
    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::UnknownCapability(_)
    ));
    assert_eq!(vm.trading_state, before);
}

#[test]
fn production_rejects_fixture_capabilities() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    assert_eq!(host.manifest.mode, CapabilityMode::Fixture);
    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Production))
        .expect_err("production must reject fixtures");
    assert_eq!(err, x3_lang_vm::trading::TradingExecError::NonProductionCapability);
}

#[test]
fn low_net_profit_rolls_back() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.swap_output = 1_500_000;
    let before = vm.trading_state.clone();
    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("profit below floor must abort");
    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::NetProfitBelowFloor { .. }
    ));
    assert_eq!(vm.trading_state, before);
}
