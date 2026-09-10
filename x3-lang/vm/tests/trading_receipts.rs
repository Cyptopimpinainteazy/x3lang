//! Deterministic receipt encoding, hashing, and tamper-detection tests.

use x3_lang_compiler::ir::{AssetKey, TradingOperation};
use x3_lang_vm::trading::{
    build_receipt, canonical_receipt_bytes, finalize_receipt, verify_receipt, DebtRecord, ReceiptError, TradeOutcome,
    TradingState,
};

fn asset(symbol: &str) -> AssetKey {
    AssetKey {
        vm_family: "evm".to_string(),
        chain: "ethereum".to_string(),
        canonical_id: format!("0x{symbol}"),
        symbol: symbol.to_string(),
        decimals: 6,
    }
}

fn operations() -> Vec<TradingOperation> {
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
        TradingOperation::CloseDebt {
            debt_id: "debt".to_string(),
        },
        TradingOperation::AssertMinNetProfit {
            settlement_asset: asset("USDC"),
            minimum: 1,
        },
        TradingOperation::CommitAtomicTrade,
    ]
}

fn committed_state() -> TradingState {
    let mut state = TradingState {
        committed: true,
        receipt_emitted: true,
        ..TradingState::default()
    };
    state.closed_debts.insert("debt".to_string());
    state.closed_debt_records.insert(
        "debt".to_string(),
        DebtRecord {
            asset: asset("USDC"),
            principal: 1_000_000,
            fee: 0,
        },
    );
    state.net_deltas.insert(asset("USDC"), 2_000_000);
    state
}

fn sample_receipt() -> x3_lang_vm::trading::TradeReceipt {
    build_receipt(
        "0.1.0",
        [1u8; 32],
        "T",
        "P",
        [2u8; 32],
        &operations(),
        &committed_state(),
        Some(&asset("USDC")),
        TradeOutcome::Success,
    )
    .expect("receipt must build")
}

#[test]
fn receipt_encoding_is_deterministic() {
    let first = sample_receipt();
    let second = sample_receipt();
    assert_eq!(
        canonical_receipt_bytes(&first).unwrap(),
        canonical_receipt_bytes(&second).unwrap()
    );
    assert_eq!(first.receipt_hash, second.receipt_hash);
    verify_receipt(&first).expect("valid receipt must verify");
}

#[test]
fn one_bit_tampering_fails_verification() {
    let mut receipt = sample_receipt();
    receipt.trade_id.push('X');
    assert!(matches!(
        verify_receipt(&receipt),
        Err(ReceiptError::HashMismatch { .. })
    ));
}

#[test]
fn open_debt_in_successful_receipt_fails_verification() {
    let mut state = committed_state();
    state.closed_debts.clear();
    state.closed_debt_records.clear();
    state.open_debts.insert(
        "debt".to_string(),
        DebtRecord {
            asset: asset("USDC"),
            principal: 1_000_000,
            fee: 0,
        },
    );
    let receipt = build_receipt(
        "0.1.0",
        [1u8; 32],
        "T",
        "P",
        [2u8; 32],
        &operations(),
        &state,
        Some(&asset("USDC")),
        TradeOutcome::Success,
    )
    .unwrap();
    assert!(matches!(
        verify_receipt(&receipt),
        Err(ReceiptError::OpenDebtInSuccessfulReceipt(_))
    ));
}

#[test]
fn failed_receipt_cannot_report_profit() {
    let mut receipt = sample_receipt();
    receipt.outcome = TradeOutcome::Failure {
        reason: "host rejected".to_string(),
    };
    let receipt = finalize_receipt(receipt).unwrap();
    assert!(matches!(
        verify_receipt(&receipt),
        Err(ReceiptError::ProfitInFailedReceipt)
    ));
}

#[test]
fn differing_state_commitments_change_the_hash() {
    let first = sample_receipt();
    let state = committed_state();
    let second = build_receipt(
        "0.1.0",
        [1u8; 32],
        "T",
        "P",
        [3u8; 32],
        &operations(),
        &state,
        Some(&asset("USDC")),
        TradeOutcome::Success,
    )
    .unwrap();
    assert_ne!(first.receipt_hash, second.receipt_hash);
}

#[test]
fn deltas_and_costs_use_ordered_vectors() {
    let receipt = sample_receipt();
    assert_eq!(receipt.deltas.len(), 1);
    assert_eq!(receipt.deltas[0].asset, asset("USDC"));
    assert_eq!(receipt.deltas[0].delta, 2_000_000);
    assert!(receipt.costs.is_empty());
}
