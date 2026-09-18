//! Deterministic receipt encoding, hashing, and tamper-detection tests.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::SigningKey;
use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile, TradingOperation,
};
use x3_lang_vm::trading::{
    build_receipt, canonical_receipt_bytes, finalize_receipt, sign_receipt, verify_receipt, verify_receipt_economics,
    verify_receipt_trusted, CommittedCost, DebtRecord, ReceiptError, ReceiptReplayLedger, TradeOutcome, TradingState,
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

fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

fn trusted_keys(key: &SigningKey) -> BTreeMap<String, [u8; 32]> {
    BTreeMap::from([("executor-1".to_string(), key.verifying_key().to_bytes())])
}

#[test]
fn signed_receipt_verifies_against_explicit_trust_store() {
    let key = signing_key();
    let receipt = sign_receipt(sample_receipt(), "executor-1", &key).expect("receipt must sign");

    verify_receipt_trusted(&receipt, &trusted_keys(&key)).expect("trusted signed receipt must verify");
}

#[test]
fn tampering_after_signing_invalidates_attestation() {
    let key = signing_key();
    let mut receipt = sign_receipt(sample_receipt(), "executor-1", &key).expect("receipt must sign");
    receipt.trade_id.push('X');

    assert!(verify_receipt_trusted(&receipt, &trusted_keys(&key)).is_err());
}

#[test]
fn untrusted_attestor_is_rejected() {
    let key = signing_key();
    let receipt = sign_receipt(sample_receipt(), "executor-1", &key).expect("receipt must sign");
    let empty = BTreeMap::new();

    assert!(matches!(
        verify_receipt_trusted(&receipt, &empty),
        Err(ReceiptError::UntrustedAttestor(_))
    ));
}

#[test]
fn economic_replay_rejects_forged_reported_profit_even_with_rehashed_receipt() {
    let mut receipt = sample_receipt();
    receipt.realized_net_profit.as_mut().unwrap().amount += 1;
    let receipt = finalize_receipt(receipt).expect("attacker can recompute a plain checksum");

    assert!(
        verify_receipt(&receipt).is_ok(),
        "plain checksum alone cannot prove economics"
    );
    assert!(matches!(
        verify_receipt_economics(&receipt),
        Err(ReceiptError::EconomicReplayMismatch(_))
    ));
}

#[test]
fn trusted_verification_rejects_resigned_economically_invalid_receipt() {
    let key = signing_key();
    let mut receipt = sample_receipt();
    receipt.realized_net_profit.as_mut().unwrap().amount += 1;
    let receipt = sign_receipt(receipt, "executor-1", &key).expect("receipt can be signed");

    assert!(matches!(
        verify_receipt_trusted(&receipt, &trusted_keys(&key)),
        Err(ReceiptError::EconomicReplayMismatch(_))
    ));
}

#[test]
fn replay_ledger_rejects_the_identical_receipt_presented_twice() {
    // Without a ReceiptReplayLedger, verify_receipt_trusted alone accepts
    // the exact same signed receipt every time it's checked — it's a pure
    // function with no memory of what it has already verified. This is
    // exactly the gap a settlement layer needs closed: the same trade must
    // not be settleable twice just because its receipt is still valid.
    let key = signing_key();
    let receipt = sign_receipt(sample_receipt(), "executor-1", &key).expect("receipt must sign");
    let mut ledger = ReceiptReplayLedger::new();

    ledger
        .verify_and_record(&receipt, &trusted_keys(&key))
        .expect("first presentation of a valid receipt must be accepted");
    assert!(ledger.has_settled(&receipt.receipt_hash));

    let err = ledger
        .verify_and_record(&receipt, &trusted_keys(&key))
        .expect_err("presenting the identical receipt again must be rejected as a replay");
    assert_eq!(err, ReceiptError::ReceiptAlreadySettled(receipt.receipt_hash));
}

#[test]
fn replay_ledger_accepts_two_genuinely_different_receipts() {
    let key = signing_key();
    let first = sign_receipt(sample_receipt(), "executor-1", &key).expect("receipt must sign");

    // A receipt's trade_id must match its own operations' BeginAtomicTrade,
    // so a genuinely different trade needs its own operations, not just a
    // relabeled copy of the first receipt.
    let mut second_operations = operations();
    if let TradingOperation::BeginAtomicTrade { trade_id, .. } = &mut second_operations[0] {
        *trade_id = "T2".to_string();
    }
    let second_unsigned = build_receipt(
        "0.1.0",
        [1u8; 32],
        "T2",
        "P",
        [2u8; 32],
        &second_operations,
        &committed_state(),
        Some(&asset("USDC")),
        TradeOutcome::Success,
    )
    .expect("second receipt must build");
    let second = sign_receipt(second_unsigned, "executor-1", &key).expect("receipt must sign");
    assert_ne!(
        first.receipt_hash, second.receipt_hash,
        "a different trade_id must produce a different receipt hash"
    );
    let mut ledger = ReceiptReplayLedger::new();

    ledger
        .verify_and_record(&first, &trusted_keys(&key))
        .expect("first trade's receipt must settle");
    ledger
        .verify_and_record(&second, &trusted_keys(&key))
        .expect("a genuinely different trade's receipt must settle independently of the first");
}

#[test]
fn replay_ledger_does_not_record_a_receipt_that_fails_verification() {
    // A receipt that is rejected for an unrelated reason (here: tampered
    // after signing) must not get recorded into the ledger — otherwise a
    // forged/garbage receipt could poison the ledger and block the
    // legitimate receipt that shares its hash from ever settling. Since a
    // tampered receipt's hash almost certainly differs from any real
    // receipt's hash, the direct risk is more about not silently marking
    // something as "settled" that never actually passed verification.
    let key = signing_key();
    let mut receipt = sign_receipt(sample_receipt(), "executor-1", &key).expect("receipt must sign");
    receipt.trade_id.push('X'); // invalidates the attestation signature
    let mut ledger = ReceiptReplayLedger::new();

    assert!(ledger.verify_and_record(&receipt, &trusted_keys(&key)).is_err());
    assert!(
        !ledger.has_settled(&receipt.receipt_hash),
        "a receipt that failed verification must not be recorded as settled"
    );
}

/// Build a receipt whose committed-state cost ledger carries `kind`, so the
/// replay path can be exercised with a specific cost category.
fn receipt_with_cost_kind(kind: &str) -> x3_lang_vm::trading::TradeReceipt {
    let mut state = committed_state();
    state.cost_ledger.push(CommittedCost {
        asset: asset("USDC"),
        amount: 10,
        kind: kind.to_string(),
    });
    build_receipt(
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
    .expect("receipt must build")
}

#[test]
fn receipt_with_an_allowed_cost_kind_passes_economic_replay() {
    // Establishes that the cost-kind check is not vacuous: a category the
    // compiled policy does list must replay cleanly.
    let receipt = receipt_with_cost_kind("gas");
    verify_receipt(&receipt).expect("hash must verify");
    verify_receipt_economics(&receipt).expect("an allowlisted cost kind must replay");
}

#[test]
fn receipt_with_an_unknown_cost_kind_fails_economic_replay() {
    // An unclassifiable category cannot be checked against any allowlist, so
    // a receipt carrying one must not verify.
    let receipt = receipt_with_cost_kind("totally_made_up");
    verify_receipt(&receipt).expect("hash must verify");
    let err = verify_receipt_economics(&receipt).expect_err("an unknown cost kind must fail replay");
    assert!(
        matches!(err, ReceiptError::EconomicReplayMismatch(ref message) if message.contains("unknown cost kind")),
        "expected an unknown-cost-kind replay mismatch, got {err:?}"
    );
}

#[test]
fn receipt_with_a_disallowed_cost_kind_fails_economic_replay() {
    // `solver_infrastructure_fee` is a real `CostKind` that the compiled
    // policy in `operations()` does not allow. A receipt claiming it was
    // charged must be rejected by replay, not silently totalled.
    let receipt = receipt_with_cost_kind("solver_infrastructure_fee");
    verify_receipt(&receipt).expect("hash must verify");
    let err = verify_receipt_economics(&receipt).expect_err("a disallowed cost kind must fail replay");
    assert!(
        matches!(err, ReceiptError::EconomicReplayMismatch(ref message) if message.contains("allowed_cost_kinds")),
        "expected an allowlist replay mismatch, got {err:?}"
    );
}
