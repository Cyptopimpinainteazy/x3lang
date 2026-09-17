//! Atomic trading execution, guard, and rollback tests.

use std::collections::BTreeSet;

use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile,
    TradingOperation, ValueRef,
};
use x3_lang_vm::trading::{
    fixture_manifest, BorrowRequest, BorrowResult, CapabilityManifest, CapabilityMode, CommittedCost,
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
    execution_cost: u128,
    began: bool,
    committed: bool,
    rolled_back: bool,
}

impl FixtureHost {
    fn new() -> Self {
        Self {
            manifest: manifest(),
            swap_output: 2_000_000,
            borrow_fee: 0,
            commitment: COMMITMENT,
            execution_cost: 0,
            began: false,
            committed: false,
            rolled_back: false,
        }
    }
}

impl TradingHost for FixtureHost {
    fn capabilities(&self) -> &CapabilityManifest {
        &self.manifest
    }

    fn begin_transaction(&mut self) -> Result<(), HostError> {
        self.began = true;
        Ok(())
    }

    fn commit_transaction(&mut self) -> Result<(), HostError> {
        self.committed = true;
        Ok(())
    }

    fn rollback_transaction(&mut self) -> Result<(), HostError> {
        self.rolled_back = true;
        Ok(())
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
        if self.execution_cost == 0 {
            return Ok(Vec::new());
        }
        Ok(vec![CommittedCost {
            asset: asset("USDC"),
            amount: self.execution_cost,
            kind: "gas".to_string(),
        }])
    }
}

fn ops() -> Vec<TradingOperation> {
    vec![
        TradingOperation::BeginAtomicTrade {
            trade_id: "T".to_string(),
            policy: CompiledTradingPolicy {
                policy_id: "P".to_string(),
                policy_version: 1,
                chain: "ethereum".to_string(),
                max_slippage_bps: 30,
                max_gas: 1_000_000,
                max_flash_fee_bps: 10,
                deadline_blocks: 10,
                require_private_submission: false,
                minimum_net_profit: None,
                max_total_cost: 1_000_000,
                max_price_impact_bps: 30,
                max_mev_leakage_bps: 30,
                quote_freshness_blocks: 10,
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
            },
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
        current_block: 1,
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


#[test]
fn host_transaction_commits_only_after_vm_success() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();

    vm.execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect("valid trade must commit");

    assert!(host.began);
    assert!(host.committed);
    assert!(!host.rolled_back);
}

#[test]
fn host_transaction_rolls_back_on_vm_rejection() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.swap_output = 1;

    vm.execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("invalid trade must fail");

    assert!(host.began);
    assert!(!host.committed);
    assert!(host.rolled_back);
}

#[test]
fn host_execution_costs_are_applied_before_profit_guard() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.execution_cost = 1_100_000;

    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("execution cost must reduce profit before the guard");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::NetProfitBelowFloor { .. }
    ));
    assert!(host.rolled_back);
}


#[test]
fn compiled_policy_chain_mismatch_is_rejected_before_host_transaction() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.manifest.chain = "base".to_string();

    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("host chain must match compiled policy");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::CapabilityChainMismatch { .. }
    ));
    assert!(!host.began);
}

#[test]
fn compiled_policy_version_mismatch_is_rejected_before_host_transaction() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.manifest.version = "trading-policy-v2".to_string();

    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("host policy version must match compiled policy");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::CapabilityVersionMismatch { .. }
    ));
    assert!(!host.began);
}

#[test]
fn compiled_private_submission_requirement_is_enforced() {
    let mut operations = ops();
    if let TradingOperation::BeginAtomicTrade { policy, .. } = &mut operations[0] {
        policy.require_private_submission = true;
    }
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.manifest.private_submission = false;

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("private submission is a compiled capability requirement");

    assert_eq!(
        err,
        x3_lang_vm::trading::TradingExecError::PrivateSubmissionRequired
    );
    assert!(!host.began);
}

#[test]
fn compiled_deadline_cannot_be_extended_by_caller() {
    let mut operations = ops();
    if let TradingOperation::BeginAtomicTrade { policy, .. } = &mut operations[0] {
        policy.deadline_blocks = 2;
    }
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    let ctx = TradeExecutionContext {
        mode: ExecutionMode::Development,
        current_block: 3,
    };

    let err = vm
        .execute_atomic(&operations, &mut host, ctx)
        .expect_err("caller cannot extend compiled deadline");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::DeadlineExpired { deadline: 2, .. }
    ));
}

#[test]
fn compiled_flash_fee_ceiling_cannot_be_relaxed_by_caller() {
    let mut operations = ops();
    if let TradingOperation::BeginAtomicTrade { policy, .. } = &mut operations[0] {
        policy.max_flash_fee_bps = 1;
    }
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.borrow_fee = 1_000;

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("compiled flash fee ceiling must control execution");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::FeeCeilingExceeded { ceiling_bps: 1, .. }
    ));
}
