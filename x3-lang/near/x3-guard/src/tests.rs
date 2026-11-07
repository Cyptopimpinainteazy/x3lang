//! Unit tests on near-sdk's mocked blockchain.
//!
//! - Synthetic quotes are signed with a throwaway key derived from the same seed the Python
//!   tests use, so payloads signed by `near_intents.py` verify here.
//! - Two real quotes signed by 1Click (captured 2026-11-03) are verified against the
//!   production manager key, which shows the on-chain verifier matches 1Click's signing.

use super::*;
use ed25519_dalek::{Signer, SigningKey};
use near_sdk::serde_json::Value;
use near_sdk::test_utils::{accounts, get_logs, VMContextBuilder};
use near_sdk::testing_env;
use sha2::{Digest, Sha256};

/// 2026-11-03T04:28:04.780Z, the deadline the adapter sent for the live capture.
const DEADLINE_MS: u64 = 1_793_680_084_780;
const DEADLINE: &str = "2026-11-03T04:28:04.780Z";
/// Ten minutes earlier: when the quote was requested and signed.
const NOW_MS: u64 = DEADLINE_MS - 600_000;
const ISSUED: &str = "2026-11-03T04:18:04.780Z";
const PROGRAM: &str = "0b5ef1d2f6c4b0a8e8d1c1f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a1b0";
const SOL_USDC: &str = "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near";
const NEAR_USDC: &str = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
const RECIPIENT: &str = "f464e8aa90661507c667b20b39bdc9e97ccfb06822030f2ecbf3741af9d85f02";
const REFUND_TO: &str = "4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD";
const DEPOSIT: &str = "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin";

fn test_key() -> SigningKey {
    SigningKey::from_bytes(&Sha256::digest(b"x3-lang test key: never fund").into())
}

fn test_contract() -> Contract {
    Contract {
        intents: LookupMap::new(StorageKey::Intents),
        manager_public_key: test_key().verifying_key().to_bytes(),
        mandates: LookupMap::new(StorageKey::Mandates),
        state_version: STATE_VERSION,
    }
}

/// Sign `payload` as 1Click does: Ed25519 over the UTF-8 bytes of base58(sha256(payload)).
fn sign(payload: &str) -> String {
    let sig = test_key().sign(quote_hash(payload).as_bytes());
    format!("ed25519:{}", bs58::encode(sig.to_bytes()).into_string())
}

fn ctx(caller: AccountId, now_ms: u64, deposit: NearToken) {
    let mut b = VMContextBuilder::new();
    b.predecessor_account_id(caller)
        .block_timestamp(now_ms * 1_000_000)
        .attached_deposit(deposit);
    testing_env!(b.build());
}

fn one_near() -> NearToken {
    NearToken::from_near(1)
}

fn policy() -> Policy {
    Policy {
        program_sha256: PROGRAM.into(),
        origin_asset: SOL_USDC.into(),
        destination_asset: NEAR_USDC.into(),
        amount: U128(25_000_000),
        min_receive: Some(U128(24_800_000)),
        slippage_bps: 50,
        fee_cap_bps: Some(100),
        recipient: RECIPIENT.into(),
        refund_to: REFUND_TO.into(),
        deadline_ms: DEADLINE_MS,
    }
}

/// A live quote with the real capture's numbers, as a JSON object to mutate before signing.
fn live_payload() -> Value {
    json!({
        "amount": "25000000",
        "amountIn": "25000000",
        "amountInFormatted": "25.0",
        "amountInUsd": "24.983000000000",
        "amountOut": "24993591",
        "amountOutFormatted": "24.993591",
        "amountOutUsd": "24.976595358120",
        "deadline": DEADLINE,
        "depositAddress": DEPOSIT,
        "depositType": "ORIGIN_CHAIN",
        "destinationAsset": NEAR_USDC,
        "dry": false,
        "minAmountIn": "25000000",
        "minAmountOut": "24868623",
        "originAsset": SOL_USDC,
        "recipient": RECIPIENT,
        "recipientType": "DESTINATION_CHAIN",
        "refundFee": "244770",
        "refundTo": REFUND_TO,
        "refundType": "ORIGIN_CHAIN",
        "slippageTolerance": 50,
        "swapType": "EXACT_INPUT",
        "timeEstimate": 20,
        "timeWhenInactive": DEADLINE,
        "timestamp": ISSUED,
        "withdrawFee": "4779"
    })
}

fn signed(payload: &Value) -> (String, String) {
    let text = payload.to_string();
    let sig = sign(&text);
    (text, sig)
}

fn registered() -> Contract {
    ctx(accounts(0), NOW_MS, one_near());
    let mut c = test_contract();
    c.register_intent("n_001".into(), policy());
    c
}

fn failed(results: &[GuardResult]) -> Vec<String> {
    results.iter().filter(|r| !r.ok).map(|r| r.guard.clone()).collect()
}

fn facts(payload: &Value) -> QuoteFacts {
    let (text, sig) = signed(payload);
    let pk = test_key().verifying_key().to_bytes();
    verify_signed_quote(&pk, &text, &sig).unwrap()
}

// --- signature verification ---------------------------------------------------------------

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct RealQuote {
    signed_payload: String,
    signature: String,
    quote_hash: String,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct AdapterArgs {
    register: RegisterArgs,
    quote: QuoteArgs,
    outcome: OutcomeArgs,
    real_1click_dry_quotes: Vec<RealQuote>,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct RegisterArgs {
    nonce: String,
    policy: Policy,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct QuoteArgs {
    nonce: String,
    signed_payload: String,
    signature: String,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct OutcomeArgs {
    nonce: String,
    status: Status,
    amount_out: Option<U128>,
    refunded_amount: Option<U128>,
    tx_hashes: Vec<String>,
}

/// `x3-lang/tests/fixtures/x3_guard_args.json` is generated by `near_intents.py guard-args`
/// and asserted by `test_guard_args_match_the_fixture_the_contract_tests_read`.
fn adapter_args() -> AdapterArgs {
    serde_json::from_str(include_str!("../../../tests/fixtures/x3_guard_args.json")).unwrap()
}

#[test]
fn real_1click_signatures_verify_on_chain_under_the_production_key() {
    let production = decode_ed25519::<32>(ONE_CLICK_MANAGER_PUBLIC_KEY).unwrap();
    let real = adapter_args().real_1click_dry_quotes;
    assert_eq!(real.len(), 2);
    for q in real {
        let facts = verify_signed_quote(&production, &q.signed_payload, &q.signature)
            .unwrap_or_else(|e| panic!("a real 1Click quote failed to verify: {e}"));
        assert_eq!(
            facts.quote_hash, q.quote_hash,
            "on-chain hash must equal the SDK's quoteHash"
        );
        assert!(facts.dry);
        // Changing one byte of the payload breaks the signature.
        let tampered = q.signed_payload.replacen("24", "25", 1);
        assert!(verify_signed_quote(&production, &tampered, &q.signature).is_err());
        // And a dry quote can never be accepted: it signs no deposit address or deadline.
        assert!(failed(&evaluate(&policy(), &facts, facts.issued_at_ms)).contains(&"quote_kind".to_string()));
    }
}

#[test]
fn a_test_key_signature_does_not_verify_under_the_production_key() {
    let production = decode_ed25519::<32>(ONE_CLICK_MANAGER_PUBLIC_KEY).unwrap();
    let (text, sig) = signed(&live_payload());
    let err = verify_signed_quote(&production, &text, &sig).unwrap_err();
    assert!(err.contains("does not verify"), "{err}");
}

#[test]
fn a_new_contract_trusts_the_sdks_manager_key() {
    ctx(accounts(0), NOW_MS, one_near());
    assert_eq!(Contract::new().get_manager_public_key(), ONE_CLICK_MANAGER_PUBLIC_KEY);
}

#[test]
fn tampering_with_any_signed_byte_fails_verification() {
    let (text, sig) = signed(&live_payload());
    let pk = test_key().verifying_key().to_bytes();
    assert!(verify_signed_quote(&pk, &text, &sig).is_ok());
    for (from, to) in [
        ("24868623", "24868624"),               // minAmountOut
        ("24993591", "29993591"),               // amountOut
        (RECIPIENT, "attacker.near"),           // recipient
        (DEPOSIT, "AttackerDeposit1111111111"), // depositAddress
        ("04:28:04.780Z", "05:28:04.780Z"),     // deadline
        ("\"refundFee\":\"244770\"", "\"refundFee\":\"1\""),
        ("\"dry\":false", "\"dry\":true"),
    ] {
        let tampered = text.replacen(from, to, 1);
        assert_ne!(tampered, text, "{from} not found in payload");
        assert!(
            verify_signed_quote(&pk, &tampered, &sig).is_err(),
            "tampered {from} verified"
        );
    }
}

#[test]
fn malformed_signatures_and_payloads_are_refused() {
    let (text, sig) = signed(&live_payload());
    let pk = test_key().verifying_key().to_bytes();
    for bad in [
        "".to_string(),
        "FAKE_SIGNATURE".to_string(),
        sig.trim_start_matches("ed25519:").to_string(),
        "secp256k1:".to_string() + &sig[8..],
        "ed25519:0OIl".to_string(),
        "ed25519:2222".to_string(),
    ] {
        assert!(
            verify_signed_quote(&pk, &text, &bad).is_err(),
            "accepted signature {bad:?}"
        );
    }
    assert!(verify_signed_quote(&pk, "", &sig).is_err());
    assert!(verify_signed_quote(&pk, &"x".repeat(MAX_SIGNED_PAYLOAD + 1), &sig).is_err());
    // Validly signed but not a quote: refused after verification, never half-read.
    let not_a_quote = r#"{"hello":"world"}"#;
    let err = verify_signed_quote(&pk, not_a_quote, &sign(not_a_quote)).unwrap_err();
    assert!(err.contains("not a 1Click quote"), "{err}");
    let mut float_slippage = live_payload();
    float_slippage["slippageTolerance"] = json!(50.5);
    let (t, s) = signed(&float_slippage);
    assert!(verify_signed_quote(&pk, &t, &s).is_err());
}

#[test]
fn utc_timestamps_parse_exactly() {
    assert_eq!(parse_utc_ms(DEADLINE).unwrap(), DEADLINE_MS);
    assert_eq!(parse_utc_ms("1970-01-01T00:00:00Z").unwrap(), 0);
    assert_eq!(parse_utc_ms("2024-02-29T23:59:59.5Z").unwrap(), 1_709_251_199_500);
    assert_eq!(parse_utc_ms("2026-11-03T04:28:04.780123Z").unwrap(), DEADLINE_MS);
    for bad in [
        "2026-11-03T04:28:04",
        "2026-11-03 04:28:04Z",
        "2025-02-29T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-11-03T24:00:00Z",
        "2026-11-03T04:28:04.7a0Z",
    ] {
        assert!(parse_utc_ms(bad).is_err(), "parsed {bad}");
    }
}

// --- guards over verified quotes ----------------------------------------------------------

#[test]
fn a_live_quote_with_the_captured_numbers_passes_every_guard() {
    let results = evaluate(&policy(), &facts(&live_payload()), NOW_MS);
    assert!(failed(&results).is_empty(), "{results:?}");
    let names: Vec<_> = results.iter().map(|r| r.guard.as_str()).collect();
    assert_eq!(
        names,
        [
            "quote_kind",
            "quote_freshness",
            "quote_request_echo",
            "amount_in",
            "min_receive",
            "slippage",
            "fees",
            "refund_fee",
            "deposit_address",
            "deadline"
        ]
    );
}

/// 1Click itself quoting something the program forbids: validly signed, still refused.
#[test]
fn each_guard_rejects_a_validly_signed_quote_that_breaks_it() {
    let cases: Vec<(&str, &str, Value)> = vec![
        ("quote_kind", "dry", json!(true)),
        ("quote_kind", "swapType", json!("EXACT_OUTPUT")),
        ("quote_kind", "depositType", json!("INTENTS")),
        ("quote_request_echo", "recipient", json!("attacker.near")),
        (
            "quote_request_echo",
            "refundTo",
            json!("So11111111111111111111111111111111111111112"),
        ),
        ("quote_request_echo", "originAsset", json!("nep141:wrap.near")),
        ("quote_request_echo", "slippageTolerance", json!(300)),
        ("quote_request_echo", "amount", json!("20000000")),
        ("amount_in", "amountIn", json!("20000000")),
        ("min_receive", "minAmountOut", json!("24700000")),
        ("slippage", "minAmountOut", json!("30000000")),
        ("fees", "amountOutUsd", json!("24.60")),
        ("fees", "amountInUsd", json!("0")),
        ("fees", "amountInUsd", json!("-1")),
        ("refund_fee", "refundFee", json!("250001")),
        ("deadline", "timeEstimate", json!(900)),
        ("deadline", "deadline", json!("2026-11-03T04:28:04.781Z")),
        ("quote_freshness", "timestamp", json!("2026-11-03T04:12:04.779Z")),
        ("quote_freshness", "timestamp", json!("2026-11-03T04:19:04.781Z")),
    ];
    for (guard, field, value) in cases {
        let mut p = live_payload();
        p[field] = value;
        let failed = failed(&evaluate(&policy(), &facts(&p), NOW_MS));
        assert!(
            failed.contains(&guard.to_string()),
            "{field}: expected {guard} to fail, failed: {failed:?}"
        );
    }
    for missing in ["refundFee", "depositAddress", "deadline"] {
        let mut p = live_payload();
        p.as_object_mut().unwrap().remove(missing);
        assert!(
            !failed(&evaluate(&policy(), &facts(&p), NOW_MS)).is_empty(),
            "missing {missing} passed"
        );
    }
    assert!(failed(&evaluate(&policy(), &facts(&live_payload()), DEADLINE_MS)).contains(&"deadline".to_string()));
}

#[test]
fn the_slippage_bound_is_exact_to_one_bps_of_headroom() {
    // 51 bps of 24_993_591 is 127_467.31; a spread of 127_467 passes, 127_468 does not.
    let at = |spread: u128| {
        let mut p = live_payload();
        p["minAmountOut"] = json!((24_993_591 - spread).to_string());
        failed(&evaluate(&policy(), &facts(&p), NOW_MS))
    };
    assert!(!at(127_467).contains(&"slippage".to_string()));
    assert!(at(127_468).contains(&"slippage".to_string()));
}

#[test]
fn usd_parsing_is_exact_and_strict() {
    assert_eq!(parse_usd("x", "24.983").unwrap(), 24_983_000_000_000_000_000);
    assert_eq!(parse_usd("x", "1").unwrap(), 1_000_000_000_000_000_000);
    assert!(parse_usd("x", "-1.0").is_err());
    assert!(parse_usd("x", "1.0000000000000000001").is_err());
}

// --- contract lifecycle -------------------------------------------------------------------

#[test]
fn register_accept_and_settle() {
    let mut c = registered();
    assert!(c.is_nonce_used(accounts(0), "n_001".into()));
    assert!(!c.is_nonce_used(accounts(1), "n_001".into()), "nonces are per owner");

    let (text, sig) = signed(&live_payload());
    ctx(accounts(0), NOW_MS, one_near());
    let results = c.accept_quote("n_001".into(), text.clone(), sig);
    assert!(results.iter().all(|r| r.ok));
    assert!(get_logs()
        .iter()
        .any(|l| l.starts_with("EVENT_JSON:") && l.contains("\"quote_accepted\"")));

    ctx(accounts(0), NOW_MS + 60_000, one_near());
    let outcome = c.record_outcome(
        "n_001".into(),
        Status::Settled,
        Some(U128(24_993_591)),
        None,
        vec!["n1".into()],
    );
    assert_eq!(outcome.floor_met, Some(true));

    let record = c.get_intent(accounts(0), "n_001".into()).unwrap();
    let accepted = record.quote.unwrap();
    assert_eq!(
        accepted.signed_payload, text,
        "the signed bytes are kept for re-verification"
    );
    assert_eq!(accepted.facts.quote_hash, quote_hash(&text));
    assert_eq!(record.outcome.unwrap().status, Status::Settled);
}

#[test]
#[should_panic(expected = "X3_NEAR_INVALID_SIGNATURE: Cryptographic quote verification failed; deposit prohibited")]
fn a_forged_quote_is_refused_on_chain() {
    let mut c = registered();
    let (text, sig) = signed(&live_payload());
    let forged = text.replacen(RECIPIENT, "attacker.near", 1);
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), forged, sig);
}

#[test]
fn a_rejected_quote_changes_nothing_and_a_good_one_can_follow() {
    let mut c = registered();
    let mut bad = live_payload();
    bad["amountOutUsd"] = json!("20.0");
    let (text, sig) = signed(&bad);
    ctx(accounts(0), NOW_MS, one_near());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        c.accept_quote("n_001".into(), text, sig)
    }));
    assert!(outcome.is_err());
    assert!(c.get_intent(accounts(0), "n_001".into()).unwrap().quote.is_none());
    let (text, sig) = signed(&live_payload());
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), text, sig);
}

#[test]
fn an_underdelivered_settlement_is_recorded_as_missing_its_floor() {
    let mut c = registered();
    let (text, sig) = signed(&live_payload());
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), text, sig);
    ctx(accounts(0), NOW_MS, one_near());
    let outcome = c.record_outcome("n_001".into(), Status::Settled, Some(U128(24_000_000)), None, vec![]);
    assert_eq!(outcome.floor_met, Some(false));
}

#[test]
#[should_panic(expected = "already registered")]
fn a_nonce_registers_once() {
    let mut c = registered();
    ctx(accounts(0), NOW_MS, one_near());
    c.register_intent("n_001".into(), policy());
}

#[test]
#[should_panic(expected = "already accepted quote")]
fn a_policy_accepts_one_quote() {
    let mut c = registered();
    let (text, sig) = signed(&live_payload());
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), text.clone(), sig.clone());
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), text, sig);
}

#[test]
#[should_panic(expected = "no intent")]
fn only_the_owner_can_accept_a_quote() {
    let mut c = registered();
    let (text, sig) = signed(&live_payload());
    ctx(accounts(1), NOW_MS, one_near());
    c.accept_quote("n_001".into(), text, sig);
}

#[test]
#[should_panic(expected = "X3_QUOTE_REJECTED: quote rejected by quote_freshness, deadline")]
fn a_quote_after_the_deadline_is_rejected() {
    let mut c = registered();
    let (text, sig) = signed(&live_payload());
    ctx(accounts(0), DEADLINE_MS, one_near());
    c.accept_quote("n_001".into(), text, sig);
}

#[test]
#[should_panic(expected = "no quote has been accepted")]
fn an_outcome_needs_an_accepted_quote() {
    let mut c = registered();
    ctx(accounts(0), NOW_MS, one_near());
    c.record_outcome("n_001".into(), Status::Refunded, None, Some(U128(25_000_000)), vec![]);
}

#[test]
#[should_panic(expected = "already recorded")]
fn an_outcome_is_recorded_once() {
    let mut c = registered();
    let (text, sig) = signed(&live_payload());
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), text, sig);
    ctx(accounts(0), NOW_MS, one_near());
    c.record_outcome("n_001".into(), Status::Refunded, None, Some(U128(24_755_230)), vec![]);
    ctx(accounts(0), NOW_MS, one_near());
    c.record_outcome("n_001".into(), Status::Settled, Some(U128(24_993_591)), None, vec![]);
}

#[test]
#[should_panic(expected = "states amount_out")]
fn a_settled_outcome_states_what_was_delivered() {
    let mut c = registered();
    let (text, sig) = signed(&live_payload());
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), text, sig);
    ctx(accounts(0), NOW_MS, one_near());
    c.record_outcome("n_001".into(), Status::Settled, None, None, vec![]);
}

#[test]
#[should_panic(expected = "to pay for")]
fn storage_must_be_paid() {
    ctx(accounts(0), NOW_MS, NearToken::from_yoctonear(0));
    let mut c = test_contract();
    c.register_intent("n_001".into(), policy());
}

#[test]
#[should_panic(expected = "deadline has already passed")]
fn an_expired_policy_cannot_be_registered() {
    ctx(accounts(0), DEADLINE_MS + 1, one_near());
    let mut c = test_contract();
    c.register_intent("n_001".into(), policy());
}

#[test]
#[should_panic(expected = "program_sha256")]
fn the_program_hash_must_be_a_sha256() {
    ctx(accounts(0), NOW_MS, one_near());
    let mut c = test_contract();
    let mut p = policy();
    p.program_sha256 = "not-a-hash".into();
    c.register_intent("n_001".into(), p);
}

#[test]
#[should_panic(expected = "nonce may contain only")]
fn nonces_are_plain_identifiers() {
    ctx(accounts(0), NOW_MS, one_near());
    let mut c = test_contract();
    c.register_intent("n/001".into(), policy());
}

#[test]
fn check_quote_is_a_dry_run_that_reports_the_signature() {
    let c = registered();
    let mut p = live_payload();
    p["amountIn"] = json!("1");
    let (text, sig) = signed(&p);
    let results = c.check_quote(accounts(0), "n_001".into(), text.clone(), sig);
    assert_eq!(results[0].guard, "quote_signature");
    assert!(results[0].ok);
    assert_eq!(failed(&results), ["amount_in"]);
    let forged = c.check_quote(accounts(0), "n_001".into(), text, sign("something else"));
    assert_eq!(failed(&forged), ["quote_signature"]);
    assert!(c.get_intent(accounts(0), "n_001".into()).unwrap().quote.is_none());
}

// --- the adapter's own output -------------------------------------------------------------

#[test]
fn the_adapters_generated_arguments_run_the_full_lifecycle() {
    let fixture = adapter_args();
    // The adapter quoted at 03:00:00Z with `timeout 10m`.
    let now = fixture.register.policy.deadline_ms - 600_000;

    ctx(accounts(0), now, one_near());
    let mut c = test_contract();
    c.register_intent(fixture.register.nonce, fixture.register.policy);

    ctx(accounts(0), now + 5_000, one_near());
    let results = c.accept_quote(
        fixture.quote.nonce,
        fixture.quote.signed_payload,
        fixture.quote.signature,
    );
    assert!(results.iter().all(|r| r.ok), "{results:?}");

    ctx(accounts(0), now + 60_000, one_near());
    let o = fixture.outcome;
    let outcome = c.record_outcome(o.nonce.clone(), o.status, o.amount_out, o.refunded_amount, o.tx_hashes);
    assert_eq!(outcome.status, Status::Settled);
    assert_eq!(outcome.floor_met, Some(true));
    assert!(c.get_intent(accounts(0), o.nonce).unwrap().outcome.is_some());
}

#[path = "mandate_tests.rs"]
mod mandate_tests;
