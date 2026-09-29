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
                minimum_net_profit_asset: None,
                quote_freshness_blocks: Some(10),
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
                max_price_impact_bps: None,
                max_mev_leakage_bps: None,
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

/// `operations()` with its `AssertMinNetProfit` floor replaced.
fn operations_with_profit_floor(settlement_asset: AssetKey, minimum: u128) -> Vec<TradingOperation> {
    operations()
        .into_iter()
        .map(|operation| match operation {
            TradingOperation::AssertMinNetProfit { .. } => TradingOperation::AssertMinNetProfit {
                settlement_asset: settlement_asset.clone(),
                minimum,
            },
            other => other,
        })
        .collect()
}

fn receipt_with_profit_floor(settlement_asset: AssetKey, minimum: u128) -> x3_lang_vm::trading::TradeReceipt {
    build_receipt(
        "0.1.0",
        [1u8; 32],
        "T",
        "P",
        [2u8; 32],
        &operations_with_profit_floor(settlement_asset, minimum),
        &committed_state(),
        Some(&asset("USDC")),
        TradeOutcome::Success,
    )
    .expect("receipt must build")
}

/// A receipt may not report a net below the profit floor its own operation sequence states.
///
/// `AssertMinNetProfit { settlement_asset, minimum }` is the floor the trade ran under, and the
/// receipt carries that operation verbatim. Replay used to record only *that a profit guard was
/// present* (`saw_profit_guard`), so a receipt whose realized net sat below the floor replayed
/// cleanly: the receipt was checked against its own numbers rather than against the floor it
/// claims to have been executed under. `committed_state()` nets 2,000,000 USDC, so a floor of
/// 5,000,000 is one this receipt cannot satisfy.
#[test]
fn receipt_below_its_compiled_profit_floor_fails_economic_replay() {
    let receipt = receipt_with_profit_floor(asset("USDC"), 5_000_000);
    verify_receipt(&receipt).expect("the receipt is internally consistent, so its hash must verify");
    let err = verify_receipt_economics(&receipt)
        .expect_err("a receipt that nets below its own compiled floor must not replay");
    let message = err.to_string();
    assert!(
        message.contains("5000000") && message.contains("2000000"),
        "the refusal must name the floor and the realized net, got {message}"
    );
}

/// The floor is enforced against the settlement asset, so a floor whose asset the receipt never
/// touched cannot be satisfied by a surplus in some other asset. The receipt's profit is in USDC;
/// a floor stated in WETH has no delta at all, which is 0, and 0 is below any positive floor.
#[test]
fn profit_floor_in_an_asset_the_receipt_never_touched_fails_economic_replay() {
    let receipt = receipt_with_profit_floor(asset("WETH"), 1);
    verify_receipt(&receipt).expect("the receipt is internally consistent, so its hash must verify");
    let err =
        verify_receipt_economics(&receipt).expect_err("a floor in an asset with no realized delta must not replay");
    let message = err.to_string();
    assert!(
        message.contains("WETH"),
        "the refusal must name the asset whose floor could not be met, got {message}"
    );
}

/// The boundary the previous test brackets: a floor the realized net exactly meets is satisfied,
/// so the check is a floor and not an unstated slack requirement.
#[test]
fn receipt_that_exactly_meets_its_compiled_profit_floor_replays() {
    let receipt = receipt_with_profit_floor(asset("USDC"), 2_000_000);
    verify_receipt(&receipt).expect("hash must verify");
    verify_receipt_economics(&receipt).expect("a net exactly at the floor satisfies it");
}

// ─── The receipt key registry (X3-LANG-003) ─────────────────────────────────────────────────────
//
// `--trusted` named keys per invocation: nothing recorded that a key had been rotated out or
// revoked, so a compromised key stayed trusted wherever an old command line still named it.

mod key_registry {
    use super::*;
    use x3_lang_vm::trading::{ReceiptKeyRegistry, ReceiptKeyStatus, RegisteredReceiptKey};

    fn hex(key: &SigningKey) -> String {
        key.verifying_key()
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn entry(key_id: &str, key: &SigningKey, status: ReceiptKeyStatus) -> RegisteredReceiptKey {
        RegisteredReceiptKey {
            key_id: key_id.to_string(),
            public_key: hex(key),
            status,
        }
    }

    fn old_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn new_key() -> SigningKey {
        SigningKey::from_bytes(&[9u8; 32])
    }

    /// Rotation: `executor-1` was retired when `executor-2` took over. What `executor-1` signed is
    /// genuine and still verifies; the registry says it may no longer sign.
    #[test]
    fn a_rotated_out_key_still_verifies_what_it_signed_but_may_not_sign() {
        let registry = ReceiptKeyRegistry::from_entries(vec![
            entry("executor-1", &old_key(), ReceiptKeyStatus::Retired),
            entry("executor-2", &new_key(), ReceiptKeyStatus::Active),
        ])
        .unwrap();

        let old = sign_receipt(sample_receipt(), "executor-1", &old_key()).unwrap();
        let new = sign_receipt(sample_receipt(), "executor-2", &new_key()).unwrap();
        registry.verify(&old).expect("a retired key's receipts stay valid");
        registry.verify(&new).expect("the active key's receipts verify");
        assert!(!registry.may_sign("executor-1"));
        assert!(registry.may_sign("executor-2"));
    }

    /// Revocation: a compromised key is refused by name, including for receipts it signed before
    /// the revocation, because a forger holding it can produce receipts indistinguishable from
    /// those.
    #[test]
    fn a_revoked_key_verifies_nothing_it_signed() {
        let registry = ReceiptKeyRegistry::from_entries(vec![
            entry("executor-1", &old_key(), ReceiptKeyStatus::Revoked),
            entry("executor-2", &new_key(), ReceiptKeyStatus::Active),
        ])
        .unwrap();
        let receipt = sign_receipt(sample_receipt(), "executor-1", &old_key()).unwrap();
        assert_eq!(
            registry.verify(&receipt),
            Err(ReceiptError::RevokedAttestor("executor-1".to_string()))
        );
        // The same receipt against the plain trust store the registry replaces still verifies:
        // the revocation is the registry's doing, not a broken signature.
        verify_receipt_trusted(&receipt, &trusted_keys(&old_key())).unwrap();
        assert!(!registry.may_sign("executor-1"));
    }

    #[test]
    fn a_registry_does_not_trust_a_key_it_does_not_list_or_a_key_under_the_wrong_id() {
        let registry =
            ReceiptKeyRegistry::from_entries(vec![entry("executor-2", &new_key(), ReceiptKeyStatus::Active)]).unwrap();
        let unlisted = sign_receipt(sample_receipt(), "executor-1", &old_key()).unwrap();
        assert!(matches!(
            registry.verify(&unlisted),
            Err(ReceiptError::UntrustedAttestor(_))
        ));
        // The right id with the wrong key: a receipt that claims to be `executor-2`'s.
        let impostor = sign_receipt(sample_receipt(), "executor-2", &old_key()).unwrap();
        assert!(matches!(
            registry.verify(&impostor),
            Err(ReceiptError::UntrustedAttestor(_))
        ));
        // Tampering after signing still breaks the attestation.
        let mut tampered = sign_receipt(sample_receipt(), "executor-2", &new_key()).unwrap();
        tampered.trade_id.push('X');
        assert!(registry.verify(&tampered).is_err());
    }

    /// A registry a verifier could misread is refused when it is loaded, not when a receipt is.
    #[test]
    fn an_ambiguous_registry_is_refused() {
        let active = |id: &str, key: &SigningKey| entry(id, key, ReceiptKeyStatus::Active);
        let cases: Vec<(&str, Vec<RegisteredReceiptKey>)> = vec![
            ("duplicate id", vec![active("a", &old_key()), active("a", &new_key())]),
            (
                "one key under two ids",
                vec![
                    active("a", &old_key()),
                    entry("b", &old_key(), ReceiptKeyStatus::Revoked),
                ],
            ),
            ("no active key", vec![entry("a", &old_key(), ReceiptKeyStatus::Retired)]),
            ("empty id", vec![active(" ", &old_key())]),
            (
                "short key",
                vec![RegisteredReceiptKey {
                    key_id: "a".into(),
                    public_key: "abcd".into(),
                    status: ReceiptKeyStatus::Active,
                }],
            ),
            (
                "multi-byte characters",
                vec![RegisteredReceiptKey {
                    key_id: "a".into(),
                    public_key: "é".repeat(32),
                    status: ReceiptKeyStatus::Active,
                }],
            ),
        ];
        for (name, entries) in cases {
            assert!(ReceiptKeyRegistry::from_entries(entries).is_err(), "{name}");
        }
    }

    #[test]
    fn a_registry_file_parses_and_refuses_unknown_fields() {
        let json = format!(
            r#"{{"keys": [
                {{"key_id": "executor-1", "public_key": "{}", "status": "retired"}},
                {{"key_id": "executor-2", "public_key": "{}", "status": "active"}}
            ]}}"#,
            hex(&old_key()),
            hex(&new_key())
        );
        let registry = ReceiptKeyRegistry::from_json(&json).unwrap();
        assert_eq!(registry.status("executor-1"), Some(ReceiptKeyStatus::Retired));
        assert_eq!(registry.status("executor-2"), Some(ReceiptKeyStatus::Active));

        // A misspelt field is refused rather than ignored: `stauts` would otherwise default nothing
        // and the entry would be unreadable.
        let misspelt = json.replacen("\"status\": \"retired\"", "\"stauts\": \"retired\"", 1);
        assert!(ReceiptKeyRegistry::from_json(&misspelt).is_err());
        let unknown_status = json.replacen("\"retired\"", "\"paused\"", 1);
        assert!(ReceiptKeyRegistry::from_json(&unknown_status).is_err());
    }
}
