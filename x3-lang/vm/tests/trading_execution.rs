//! Atomic trading execution, guard, and rollback tests.

use std::collections::BTreeSet;

use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile, TradingOperation, ValueRef,
};
use x3_lang_vm::trading::{
    fixture_manifest, BorrowRequest, BorrowResult, CapabilityManifest, CapabilityMode, CommittedCost, ExecutionMode,
    HostError, PriceSource, QuoteRequest, QuoteResult, RepayRequest, RepayResult, SwapRequest, SwapResult,
    TradeExecutionContext, TradingHost, TradingVm,
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
    /// Quoted output returned by `quote()`, checked against the real
    /// `swap_output` for slippage. Defaults to `swap_output` itself (zero
    /// slippage) so existing tests are unaffected unless a test explicitly
    /// diverges the two to simulate a stale/moved quote.
    quote_output: Option<u128>,
    /// Independent price sources returned by `quote()` alongside the
    /// primary figure. Empty by default (no oracle-firewall cross-check
    /// data); tests that need `max_oracle_deviation` to actually have
    /// something to check set this explicitly.
    oracle_sources: Vec<PriceSource>,
    borrow_fee: u128,
    commitment: [u8; 32],
    execution_cost: u128,
    /// Asset the fixture-reported execution cost is denominated in.
    /// Defaults to USDC (the settlement asset `ops()` already touches);
    /// tests can point it at an asset the trade never otherwise credits
    /// or debits, to prove costs there are still caught.
    execution_cost_asset: Option<AssetKey>,
    began: bool,
    committed: bool,
    rolled_back: bool,
}

impl FixtureHost {
    fn new() -> Self {
        Self {
            manifest: manifest(),
            swap_output: 2_000_000,
            quote_output: None,
            oracle_sources: Vec::new(),
            borrow_fee: 0,
            commitment: COMMITMENT,
            execution_cost: 0,
            execution_cost_asset: None,
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

    fn quote(&self, _request: QuoteRequest) -> Result<QuoteResult, HostError> {
        Ok(QuoteResult {
            expected_output: self.quote_output.unwrap_or(self.swap_output),
            sources: self.oracle_sources.clone(),
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
            asset: self.execution_cost_asset.clone().unwrap_or_else(|| asset("USDC")),
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
                max_gas_asset: asset("USDC"),
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
                max_oracle_deviation_bps: None,
                max_cumulative_loss: None,
                max_cumulative_loss_asset: None,
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
    TradeExecutionContext { mode, current_block: 1 }
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
fn simulate_never_commits_even_when_every_guard_passes() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    let before = vm.trading_state.clone();

    let execution = vm
        .simulate_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect("a trade that would succeed must project a successful outcome");

    // The projection reflects a real pass — the guards actually ran
    // against the real host — but nothing was allowed to land.
    assert!(execution.committed_state.committed);
    assert!(execution.committed_state.receipt_emitted);
    assert!(host.began);
    assert!(
        !host.committed,
        "simulate_atomic must never call host.commit_transaction()"
    );
    assert!(
        host.rolled_back,
        "simulate_atomic must always call host.rollback_transaction()"
    );
    assert_eq!(
        vm.trading_state, before,
        "simulate_atomic must restore VM state even on a successful projection"
    );
}

#[test]
fn simulate_reports_the_same_failure_a_real_execution_would_hit() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.swap_output = 1;
    let before = vm.trading_state.clone();

    let err = vm
        .simulate_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("a trade that would fail must project the same failure");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::OutputBelowMinOut { .. }
    ));
    assert!(host.began);
    assert!(!host.committed);
    assert!(host.rolled_back);
    assert_eq!(vm.trading_state, before);
}

#[test]
fn simulate_does_not_disturb_a_later_real_execution() {
    let mut vm = TradingVm::new();

    let mut probe_host = FixtureHost::new();
    vm.simulate_atomic(&ops(), &mut probe_host, context(ExecutionMode::Development))
        .expect("projection must succeed");

    // The same VM, reused for a real execution right after simulating,
    // must behave exactly as if the simulation never happened.
    let mut real_host = FixtureHost::new();
    let execution = vm
        .execute_atomic(&ops(), &mut real_host, context(ExecutionMode::Development))
        .expect("a real execution after a simulation must still commit normally");

    assert!(execution.committed_state.committed);
    assert!(real_host.committed);
    assert!(!real_host.rolled_back);
}

#[test]
fn host_execution_costs_are_applied_before_profit_guard() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // ops()'s baseline profit is exactly the 1_000_000 floor with zero extra
    // cost, so any positive cost pushes it below the floor. Kept well under
    // the 1_000_000 max_gas ceiling so this test isolates the profit-guard
    // behavior specifically, not gas_ceiling_exceeded_rejects_* below.
    host.execution_cost = 100;

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
fn inconsistent_submission_profile_is_rejected_before_host_transaction() {
    let mut operations = ops();
    if let TradingOperation::BeginAtomicTrade { policy, .. } = &mut operations[0] {
        policy.submission_profile = SubmissionProfile::Private;
    }
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("legacy flag and submission profile must agree");

    assert_eq!(
        err,
        x3_lang_vm::trading::TradingExecError::InconsistentSubmissionProfile
    );
    assert!(!host.began);
}

#[test]
fn compiled_private_submission_requirement_is_enforced() {
    let mut operations = ops();
    if let TradingOperation::BeginAtomicTrade { policy, .. } = &mut operations[0] {
        policy.require_private_submission = true;
        policy.submission_profile = SubmissionProfile::Private;
    }
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.manifest.private_submission = false;

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("private submission is a compiled capability requirement");

    assert_eq!(err, x3_lang_vm::trading::TradingExecError::PrivateSubmissionRequired);
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

/// `ops()` with an `AssertInvariant { kind: Solvent }` inserted right after
/// the existing guards, before the all-debts/receipt/commit tail.
fn ops_with_solvent_invariant() -> Vec<TradingOperation> {
    let mut operations = ops();
    let insert_at = operations
        .iter()
        .position(|op| matches!(op, TradingOperation::AssertAllDebtsClosed))
        .expect("ops() fixture must contain AssertAllDebtsClosed");
    operations.insert(
        insert_at,
        TradingOperation::AssertInvariant {
            kind: x3_lang_compiler::ir::InvariantKind::Solvent,
        },
    );
    operations
}

#[test]
fn solvent_invariant_passes_when_every_touched_asset_nets_non_negative() {
    let operations = ops_with_solvent_invariant();
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();

    vm.execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect("fully repaid, zero-fee trade must satisfy the solvent invariant");
}

#[test]
fn solvent_invariant_catches_hidden_cost_in_an_asset_the_trade_never_touches() {
    let operations = ops_with_solvent_invariant();
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // Gas paid in ETH — an asset this trade never borrows, swaps, or repays,
    // so nothing else would ever notice it went negative. The named
    // settlement asset (USDC) still clears its own profit floor: this is
    // exactly the gap a single-asset profit check can't see.
    host.execution_cost = 5_000;
    host.execution_cost_asset = Some(asset("ETH"));

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("a hidden cost in an untouched asset must violate the solvent invariant");

    match err {
        x3_lang_vm::trading::TradingExecError::InvariantViolated {
            kind: x3_lang_compiler::ir::InvariantKind::Solvent,
            asset,
            deficit,
        } => {
            assert_eq!(asset.symbol, "ETH");
            assert_eq!(deficit, -5_000);
        }
        other => panic!("expected InvariantViolated for ETH, got {other:?}"),
    }
    assert!(host.rolled_back, "insolvent trade must roll back the host transaction");
}

#[test]
fn gas_ceiling_within_policy_still_commits() {
    // A minimal trade with no profit/debt guards to check, isolating this
    // test to gas-ceiling behavior specifically: cost exactly equal to the
    // ceiling (the check is strictly `>`, not `>=`) must still commit.
    let operations = vec![
        TradingOperation::BeginAtomicTrade {
            trade_id: "T".to_string(),
            policy: CompiledTradingPolicy {
                policy_id: "P".to_string(),
                policy_version: 1,
                chain: "ethereum".to_string(),
                max_slippage_bps: 30,
                max_gas: 100,
                max_gas_asset: asset("USDC"),
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
                max_oracle_deviation_bps: None,
                max_cumulative_loss: None,
                max_cumulative_loss_asset: None,
            },
        },
        TradingOperation::CommitAtomicTrade,
    ];
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.execution_cost = 100; // == policy.max_gas exactly, not over
    host.execution_cost_asset = Some(asset("USDC"));

    vm.execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect("gas cost exactly at the compiled ceiling must still commit");
}

#[test]
fn gas_ceiling_exceeded_rejects_even_though_profit_and_debts_are_fine() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // Compiled ceiling in ops() is max_gas: 1_000_000 in USDC.
    host.execution_cost = 1_000_001;
    host.execution_cost_asset = Some(asset("USDC"));

    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("cost above the compiled max_gas ceiling must be rejected");

    assert_eq!(
        err,
        x3_lang_vm::trading::TradingExecError::GasCeilingExceeded {
            asset: asset("USDC"),
            ceiling: 1_000_000,
            actual: 1_000_001,
        }
    );
    assert!(
        host.rolled_back,
        "over-ceiling trade must roll back the host transaction"
    );
}

#[test]
fn gas_ceiling_is_enforced_at_commit_even_with_no_other_guard_operations() {
    // A trade that skips both AssertMinNetProfit and AssertInvariant never
    // calls accrue_host_execution_costs anywhere except the unconditional
    // check CommitAtomicTrade itself performs. Prove that guarantee is real,
    // not just documented in a comment.
    let operations = vec![
        TradingOperation::BeginAtomicTrade {
            trade_id: "T".to_string(),
            policy: CompiledTradingPolicy {
                policy_id: "P".to_string(),
                policy_version: 1,
                chain: "ethereum".to_string(),
                max_slippage_bps: 30,
                max_gas: 100,
                max_gas_asset: asset("USDC"),
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
                max_oracle_deviation_bps: None,
                max_cumulative_loss: None,
                max_cumulative_loss_asset: None,
            },
        },
        TradingOperation::CommitAtomicTrade,
    ];
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.execution_cost = 101;
    host.execution_cost_asset = Some(asset("USDC"));

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("commit must still enforce the gas ceiling with no other guards present");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::GasCeilingExceeded {
            actual: 101,
            ceiling: 100,
            ..
        }
    ));
}

#[test]
fn output_matching_or_beating_the_quote_is_never_slippage() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // swap_output (what actually executes) stays at the default 2_000_000;
    // quote_output below it means the trade did *better* than quoted.
    host.quote_output = Some(1_900_000);

    vm.execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect("beating the quote must never be reported as slippage");
}

#[test]
fn slippage_within_policy_ceiling_still_commits() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // ops()'s policy allows 30 bps. Quote 5_000 above the actual 2_000_000
    // output is ~24.9 bps — comfortably under the ceiling.
    host.quote_output = Some(2_005_000);

    vm.execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect("slippage under the compiled ceiling must still commit");
}

#[test]
fn slippage_beyond_policy_ceiling_is_rejected() {
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // Quote far above the actual output — unambiguously over the 30 bps
    // ceiling ops()'s policy declares.
    host.quote_output = Some(2_100_000);
    let before = vm.trading_state.clone();

    let err = vm
        .execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect_err("realized output far below the live quote must violate the slippage ceiling");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::SlippageExceeded { ceiling_bps: 30, .. }
    ));
    assert_eq!(vm.trading_state, before, "rejected trade must not leave partial state");
    assert!(host.rolled_back);
}

/// `ops()` with `max_oracle_deviation_bps` set on the compiled policy.
fn ops_with_oracle_deviation(ceiling_bps: u16) -> Vec<TradingOperation> {
    let mut operations = ops();
    if let TradingOperation::BeginAtomicTrade { policy, .. } = &mut operations[0] {
        policy.max_oracle_deviation_bps = Some(ceiling_bps);
    }
    operations
}

#[test]
fn oracle_deviation_not_checked_when_policy_omits_it() {
    // No sources reported and no ceiling declared: nothing to fail closed
    // on, since the policy never asked for cross-checking in the first
    // place.
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    assert!(host.oracle_sources.is_empty());

    vm.execute_atomic(&ops(), &mut host, context(ExecutionMode::Development))
        .expect("a policy with no max_oracle_deviation must never require price sources");
}

#[test]
fn oracle_firewall_fails_closed_when_required_but_host_reports_no_sources() {
    let operations = ops_with_oracle_deviation(50);
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // oracle_sources left empty on purpose.

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("declaring max_oracle_deviation with no host cross-check data must fail closed");

    assert_eq!(err, x3_lang_vm::trading::TradingExecError::OracleFirewallUnsatisfied);
    assert!(host.rolled_back);
}

#[test]
fn oracle_firewall_passes_when_every_source_agrees_within_ceiling() {
    let operations = ops_with_oracle_deviation(50);
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // Primary quote defaults to swap_output (2_000_000). 4_000 above that
    // is 20 bps — under the 50 bps ceiling.
    host.oracle_sources = vec![PriceSource {
        name: "twap".to_string(),
        expected_output: 2_004_000,
    }];

    vm.execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect("a source within the deviation ceiling must not block the trade");
}

#[test]
fn oracle_firewall_rejects_a_source_that_deviates_beyond_ceiling() {
    let operations = ops_with_oracle_deviation(50);
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    // ~526 bps away from the 2_000_000 primary quote — unambiguously past
    // a 50 bps ceiling, exactly the "one source disagrees" manipulation
    // signal this check exists to catch.
    host.oracle_sources = vec![PriceSource {
        name: "suspicious_pool".to_string(),
        expected_output: 1_900_000,
    }];
    let before = vm.trading_state.clone();

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("a source that disagrees with the primary quote must be rejected");

    match err {
        x3_lang_vm::trading::TradingExecError::OracleDeviationExceeded {
            source,
            ceiling_bps: 50,
            ..
        } => assert_eq!(source, "suspicious_pool"),
        other => panic!("expected OracleDeviationExceeded naming the bad source, got {other:?}"),
    }
    assert_eq!(vm.trading_state, before);
    assert!(host.rolled_back);
}

#[test]
fn oracle_firewall_catches_a_source_quoting_higher_too() {
    // Deviation is measured both directions: a source claiming a *better*
    // price than the primary quote is just as much a disagreement as one
    // claiming worse — high deviation in either direction is the signal,
    // not "worse than expected" specifically.
    let operations = ops_with_oracle_deviation(50);
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.oracle_sources = vec![PriceSource {
        name: "inflated_feed".to_string(),
        expected_output: 2_200_000,
    }];

    let err = vm
        .execute_atomic(&operations, &mut host, context(ExecutionMode::Development))
        .expect_err("a source quoting well above the primary must also violate the ceiling");

    assert!(matches!(
        err,
        x3_lang_vm::trading::TradingExecError::OracleDeviationExceeded { ceiling_bps: 50, .. }
    ));
}

/// A minimal trade (no debts, no swaps) whose only USDC movement is the
/// host-reported execution cost, letting a test control exactly how much a
/// single trade "loses" without any of the other guards (profit floor,
/// debt closure) getting in the way. `ceiling` is `None` for a policy that
/// never declares `max_cumulative_loss` at all.
fn minimal_loss_trade(ceiling: Option<u128>) -> Vec<TradingOperation> {
    vec![
        TradingOperation::BeginAtomicTrade {
            trade_id: "T".to_string(),
            policy: CompiledTradingPolicy {
                policy_id: "P".to_string(),
                policy_version: 1,
                chain: "ethereum".to_string(),
                max_slippage_bps: 30,
                max_gas: u128::MAX,
                max_gas_asset: asset("USDC"),
                max_flash_fee_bps: 10,
                deadline_blocks: 10,
                require_private_submission: false,
                minimum_net_profit: None,
                max_total_cost: u128::MAX,
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
                max_oracle_deviation_bps: None,
                max_cumulative_loss: ceiling,
                max_cumulative_loss_asset: ceiling.map(|_| asset("USDC")),
            },
        },
        TradingOperation::CommitAtomicTrade,
    ]
}

#[test]
fn cumulative_loss_ceiling_is_not_checked_when_policy_omits_it() {
    // No ceiling declared: however much a trade loses, there is nothing to
    // fail closed on — mirrors oracle_deviation_not_checked_when_policy_omits_it.
    let mut vm = TradingVm::new();
    let mut host = FixtureHost::new();
    host.execution_cost = 1_000_000_000;
    host.execution_cost_asset = Some(asset("USDC"));

    vm.execute_atomic(
        &minimal_loss_trade(None),
        &mut host,
        context(ExecutionMode::Development),
    )
    .expect("a policy with no max_cumulative_loss must never block on realized losses");
    assert_eq!(vm.cumulative_realized(&asset("USDC")), -1_000_000_000);
}

#[test]
fn cumulative_loss_ceiling_trips_only_once_prior_trades_are_summed_in() {
    // Each individual trade loses 100 — well under a 150 ceiling on its
    // own. The ceiling only bites on the second trade because it is
    // cumulative: -100 (trade 1, committed) + -100 (trade 2) = -200, past
    // -150. This is the property that makes it a cross-trade circuit
    // breaker rather than just a per-trade check in disguise.
    let mut vm = TradingVm::new();
    let operations = minimal_loss_trade(Some(150));

    let mut host1 = FixtureHost::new();
    host1.execution_cost = 100;
    host1.execution_cost_asset = Some(asset("USDC"));
    vm.execute_atomic(&operations, &mut host1, context(ExecutionMode::Development))
        .expect("first 100-loss trade must commit: -100 is within the -150 ceiling");
    assert_eq!(vm.cumulative_realized(&asset("USDC")), -100);

    let mut host2 = FixtureHost::new();
    host2.execution_cost = 100;
    host2.execution_cost_asset = Some(asset("USDC"));
    let err = vm
        .execute_atomic(&operations, &mut host2, context(ExecutionMode::Development))
        .expect_err("second 100-loss trade must be rejected: cumulative -200 breaches the -150 ceiling");

    assert_eq!(
        err,
        x3_lang_vm::trading::TradingExecError::CumulativeLossCeilingExceeded {
            asset: asset("USDC"),
            ceiling: 150,
            projected_loss: 200,
        }
    );
    assert!(host2.rolled_back);
    // The rejected trade must not itself be recorded — the ledger stays at
    // the pre-rejection total, not the projected (and never-landed) one.
    assert_eq!(vm.cumulative_realized(&asset("USDC")), -100);
}

#[test]
fn cumulative_loss_ledger_only_grows_from_trades_that_actually_commit() {
    // A rejected trade (here: rejected for an unrelated reason, an expired
    // deadline, before execution even begins) must leave the cumulative
    // ledger untouched, exactly like it leaves trading_state untouched.
    let mut vm = TradingVm::new();
    let operations = minimal_loss_trade(Some(50));
    let mut host = FixtureHost::new();
    host.execution_cost = 1_000; // would blow the ceiling if it ever landed
    host.execution_cost_asset = Some(asset("USDC"));
    let mut ctx = context(ExecutionMode::Development);
    ctx.current_block = 999; // past deadline_blocks: 10

    vm.execute_atomic(&operations, &mut host, ctx)
        .expect_err("expired deadline must abort before any accounting runs");

    assert_eq!(vm.cumulative_realized(&asset("USDC")), 0);
}

#[test]
fn simulating_a_lossy_trade_never_updates_the_cumulative_ledger() {
    let mut vm = TradingVm::new();
    let operations = minimal_loss_trade(Some(150));

    // Simulate the same 100-loss trade three times over — a real run
    // would trip the ceiling by the second one, exactly as proven above.
    for _ in 0..3 {
        let mut host = FixtureHost::new();
        host.execution_cost = 100;
        host.execution_cost_asset = Some(asset("USDC"));
        vm.simulate_atomic(&operations, &mut host, context(ExecutionMode::Development))
            .expect("simulating a within-ceiling loss must project success");
        assert!(!host.committed);
        assert!(host.rolled_back);
    }

    assert_eq!(
        vm.cumulative_realized(&asset("USDC")),
        0,
        "simulate_atomic must never contribute to the cross-trade ledger, no matter how many times it runs"
    );

    // A real trade right after must start from that same untouched ledger.
    let mut real_host = FixtureHost::new();
    real_host.execution_cost = 100;
    real_host.execution_cost_asset = Some(asset("USDC"));
    vm.execute_atomic(&operations, &mut real_host, context(ExecutionMode::Development))
        .expect("a real trade after only simulations must see a clean ledger and commit");
    assert_eq!(vm.cumulative_realized(&asset("USDC")), -100);
}
