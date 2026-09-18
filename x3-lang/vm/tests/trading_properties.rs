//! Property tests for atomic trading conservation and rollback.

use std::collections::BTreeSet;

use proptest::prelude::*;
use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile, TradingOperation, ValueRef,
};
use x3_lang_vm::trading::{
    fixture_manifest, BorrowRequest, BorrowResult, CapabilityManifest, CommittedCost, ExecutionMode, HostError,
    QuoteRequest, QuoteResult, RepayRequest, RepayResult, SwapRequest, SwapResult, TradeExecutionContext, TradingHost,
    TradingVm,
};

const COMMITMENT: [u8; 32] = [3u8; 32];

fn asset(symbol: &str) -> AssetKey {
    AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: format!("0x{symbol}"),
        symbol: symbol.to_string(),
        decimals: 6,
    }
}

struct Host {
    manifest: CapabilityManifest,
    output: u128,
}

impl TradingHost for Host {
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
        let _ = request;
        Ok(QuoteResult {
            expected_output: self.output,
            sources: Vec::new(),
        })
    }

    fn swap(&mut self, request: SwapRequest) -> Result<SwapResult, HostError> {
        Ok(SwapResult {
            from: request.from,
            to: request.to.clone(),
            input: request.input,
            output: self.output,
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

fn operations() -> Vec<TradingOperation> {
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
                    CostKind::ProofFee,
                    CostKind::CrossDomainFee,
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
            min_output: 1,
        },
        TradingOperation::ExecuteSwap {
            binding: "returned".to_string(),
            venue: "uniswap_v3".to_string(),
            from: asset("WETH"),
            to: asset("USDC"),
            input: ValueRef::Binding("weth".to_string()),
            min_output: 1,
        },
        TradingOperation::CloseDebt {
            debt_id: "debt".to_string(),
        },
        TradingOperation::AssertMinNetProfit {
            settlement_asset: asset("USDC"),
            minimum: 1,
        },
        TradingOperation::AssertAllDebtsClosed,
        TradingOperation::EmitTradeReceipt,
        TradingOperation::CommitAtomicTrade,
    ]
}

fn host(output: u128) -> Host {
    let mut manifest = fixture_manifest(COMMITMENT);
    manifest.providers = BTreeSet::from(["aave_v3".to_string()]);
    manifest.venues = BTreeSet::from(["uniswap_v3".to_string()]);
    Host { manifest, output }
}

fn context() -> TradeExecutionContext {
    TradeExecutionContext {
        mode: ExecutionMode::Development,
        current_block: 1,
    }
}

proptest! {
    #[test]
    fn success_never_has_open_debt_or_missing_commit(output in 1u128..10_000_000u128) {
        let mut vm = TradingVm::new();
        let mut host = host(output);
        let result = vm.execute_atomic(&operations(), &mut host, context());
        if result.is_ok() {
            prop_assert!(vm.trading_state.open_debts.is_empty());
            prop_assert!(vm.trading_state.committed);
            prop_assert!(vm.trading_state.receipt_emitted);
        }
    }

    #[test]
    fn failure_restores_the_pre_execution_state(output in 1u128..10_000_000u128) {
        let mut vm = TradingVm::new();
        let before = vm.trading_state.clone();
        let mut host = host(output);
        let result = vm.execute_atomic(&operations(), &mut host, context());
        if result.is_err() {
            prop_assert_eq!(vm.trading_state, before);
        }
    }
}
