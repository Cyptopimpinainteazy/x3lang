//! Unit tests run against near-sdk's mocked blockchain. The quote numbers are the ones 1Click
//! returned for a live dry quote on 2026-11-03 (25 USDC Solana -> USDC NEAR), so the on-chain
//! guards are tested on a real quote, not an invented one.

use super::*;
use near_sdk::test_utils::{accounts, get_logs, VMContextBuilder};
use near_sdk::testing_env;

/// 2026-11-03T04:28:04.780Z, the deadline the adapter sent.
const DEADLINE_MS: u64 = 1_793_680_084_780;
/// Ten minutes earlier: when the quote was requested.
const NOW_MS: u64 = DEADLINE_MS - 600_000;
const PROGRAM: &str = "0b5ef1d2f6c4b0a8e8d1c1f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a1b0";
const QUOTE_HASH: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90";
const SOL_USDC: &str = "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near";
const NEAR_USDC: &str = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
const RECIPIENT: &str = "f464e8aa90661507c667b20b39bdc9e97ccfb06822030f2ecbf3741af9d85f02";
const REFUND_TO: &str = "4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD";

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

fn live_quote() -> QuoteFacts {
    QuoteFacts {
        quote_response_sha256: QUOTE_HASH.into(),
        signature: "ed25519:212wrwFvuT85fZ1fzj3noxkSEFanw13um9tRyko9VXZhrkjAU9jEiqsEtH2QgpZAzyfDGcdfNqVxim3ucAceLdqw"
            .into(),
        deposit_address: "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin".into(),
        origin_asset: SOL_USDC.into(),
        destination_asset: NEAR_USDC.into(),
        recipient: RECIPIENT.into(),
        refund_to: REFUND_TO.into(),
        slippage_tolerance_bps: 50,
        amount_in: U128(25_000_000),
        amount_out: U128(24_993_591),
        min_amount_out: U128(24_868_623),
        amount_in_usd: "24.983000000000".into(),
        amount_out_usd: "24.976595358120".into(),
        refund_fee: Some(U128(244_770)),
        quote_deadline_ms: DEADLINE_MS,
        time_estimate_s: 20,
    }
}

fn registered() -> Contract {
    ctx(accounts(0), NOW_MS, one_near());
    let mut c = Contract::new();
    c.register_intent("n_001".into(), policy());
    c
}

fn failed(results: &[GuardResult]) -> Vec<String> {
    results.iter().filter(|r| !r.ok).map(|r| r.guard.clone()).collect()
}

// --- pure guard evaluation ----------------------------------------------------------------

#[test]
fn the_captured_live_quote_passes_every_guard() {
    let results = evaluate(&policy(), &live_quote(), NOW_MS);
    assert!(failed(&results).is_empty(), "{results:?}");
    let names: Vec<_> = results.iter().map(|r| r.guard.as_str()).collect();
    assert_eq!(
        names,
        [
            "quote_request_echo",
            "amount_in",
            "min_receive",
            "slippage",
            "fees",
            "refund_fee",
            "deposit_address",
            "quote_signature",
            "quote_response_hash",
            "deadline"
        ]
    );
}

fn rejected_by(mutate: impl FnOnce(&mut Policy, &mut QuoteFacts), now_ms: u64) -> Vec<String> {
    let (mut p, mut q) = (policy(), live_quote());
    mutate(&mut p, &mut q);
    failed(&evaluate(&p, &q, now_ms))
}

type Mutation = Box<dyn FnOnce(&mut Policy, &mut QuoteFacts)>;

#[test]
fn each_guard_rejects_the_quote_that_breaks_it() {
    let cases: Vec<(&str, Mutation)> = vec![
        (
            "quote_request_echo",
            Box::new(|_, q| q.recipient = "attacker.near".into()),
        ),
        ("quote_request_echo", Box::new(|_, q| q.slippage_tolerance_bps = 300)),
        ("amount_in", Box::new(|_, q| q.amount_in = U128(20_000_000))),
        ("min_receive", Box::new(|p, _| p.min_receive = Some(U128(24_900_000)))),
        ("slippage", Box::new(|_, q| q.min_amount_out = U128(24_000_000))),
        ("slippage", Box::new(|_, q| q.min_amount_out = U128(30_000_000))),
        ("fees", Box::new(|_, q| q.amount_out_usd = "24.60".into())),
        ("fees", Box::new(|_, q| q.amount_in_usd = "0".into())),
        ("refund_fee", Box::new(|_, q| q.refund_fee = Some(U128(250_001)))),
        ("refund_fee", Box::new(|_, q| q.refund_fee = None)),
        ("deposit_address", Box::new(|_, q| q.deposit_address = String::new())),
        ("quote_signature", Box::new(|_, q| q.signature = String::new())),
        (
            "quote_response_hash",
            Box::new(|_, q| q.quote_response_sha256 = "ABC".into()),
        ),
        ("deadline", Box::new(|_, q| q.time_estimate_s = 900)),
        ("deadline", Box::new(|_, q| q.quote_deadline_ms = DEADLINE_MS + 1)),
    ];
    for (guard, mutate) in cases {
        let failed = rejected_by(mutate, NOW_MS);
        assert!(
            failed.contains(&guard.to_string()),
            "expected {guard} to fail, failed: {failed:?}"
        );
    }
    assert!(rejected_by(|_, _| {}, DEADLINE_MS).contains(&"deadline".to_string()));
}

#[test]
fn the_slippage_bound_is_exact_to_one_bps_of_headroom() {
    // 51 bps of 24_993_591 is 127_467.31; 127_467 passes, 127_468 does not.
    let at = |spread: u128| rejected_by(move |_, q| q.min_amount_out = U128(24_993_591 - spread), NOW_MS);
    assert!(!at(127_467).contains(&"slippage".to_string()));
    assert!(at(127_468).contains(&"slippage".to_string()));
}

#[test]
fn no_fee_cap_means_no_fee_guards() {
    let results = {
        let mut p = policy();
        p.fee_cap_bps = None;
        evaluate(&p, &live_quote(), NOW_MS)
    };
    assert!(!results.iter().any(|r| r.guard == "fees" || r.guard == "refund_fee"));
}

#[test]
fn usd_parsing_is_exact_and_strict() {
    assert_eq!(parse_usd("x", "24.983"), 24_983_000_000_000_000_000);
    assert_eq!(parse_usd("x", "1"), 1_000_000_000_000_000_000);
}

#[test]
#[should_panic(expected = "is not a decimal")]
fn usd_parsing_refuses_signs() {
    parse_usd("x", "-1.0");
}

#[test]
#[should_panic(expected = "fractional digits")]
fn usd_parsing_refuses_excess_precision() {
    parse_usd("x", "1.0000000000000000001");
}

// --- contract lifecycle -------------------------------------------------------------------

#[test]
fn register_accept_and_settle() {
    let mut c = registered();
    assert!(c.is_nonce_used(accounts(0), "n_001".into()));
    assert!(!c.is_nonce_used(accounts(1), "n_001".into()), "nonces are per owner");

    ctx(accounts(0), NOW_MS, one_near());
    let results = c.accept_quote("n_001".into(), live_quote());
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
    assert_eq!(record.quote.unwrap().quote_response_sha256, QUOTE_HASH);
    assert_eq!(record.outcome.unwrap().status, Status::Settled);
}

#[test]
fn an_underdelivered_settlement_is_recorded_as_missing_its_floor() {
    let mut c = registered();
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), live_quote());
    ctx(accounts(0), NOW_MS, one_near());
    let outcome = c.record_outcome("n_001".into(), Status::Settled, Some(U128(24_000_000)), None, vec![]);
    assert_eq!(outcome.floor_met, Some(false));
}

#[test]
fn a_rejected_quote_changes_nothing_and_a_good_one_can_follow() {
    let mut c = registered();
    let mut bad = live_quote();
    bad.amount_out_usd = "20.0".into();
    ctx(accounts(0), NOW_MS, one_near());
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| c.accept_quote("n_001".into(), bad)));
    assert!(outcome.is_err());
    // A panicking call is rolled back on chain; in the mocked VM the record was never written.
    assert!(c.get_intent(accounts(0), "n_001".into()).unwrap().quote.is_none());
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), live_quote());
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
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), live_quote());
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), live_quote());
}

#[test]
#[should_panic(expected = "no intent")]
fn only_the_owner_can_accept_a_quote() {
    let mut c = registered();
    ctx(accounts(1), NOW_MS, one_near());
    c.accept_quote("n_001".into(), live_quote());
}

#[test]
#[should_panic(expected = "quote rejected by deadline")]
fn a_quote_after_the_deadline_is_rejected() {
    let mut c = registered();
    ctx(accounts(0), DEADLINE_MS, one_near());
    c.accept_quote("n_001".into(), live_quote());
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
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), live_quote());
    ctx(accounts(0), NOW_MS, one_near());
    c.record_outcome("n_001".into(), Status::Refunded, None, Some(U128(24_755_230)), vec![]);
    ctx(accounts(0), NOW_MS, one_near());
    c.record_outcome("n_001".into(), Status::Settled, Some(U128(24_993_591)), None, vec![]);
}

#[test]
#[should_panic(expected = "states amount_out")]
fn a_settled_outcome_states_what_was_delivered() {
    let mut c = registered();
    ctx(accounts(0), NOW_MS, one_near());
    c.accept_quote("n_001".into(), live_quote());
    ctx(accounts(0), NOW_MS, one_near());
    c.record_outcome("n_001".into(), Status::Settled, None, None, vec![]);
}

#[test]
#[should_panic(expected = "to pay for")]
fn storage_must_be_paid() {
    ctx(accounts(0), NOW_MS, NearToken::from_yoctonear(0));
    let mut c = Contract::new();
    c.register_intent("n_001".into(), policy());
}

#[test]
#[should_panic(expected = "deadline has already passed")]
fn an_expired_policy_cannot_be_registered() {
    ctx(accounts(0), DEADLINE_MS + 1, one_near());
    let mut c = Contract::new();
    c.register_intent("n_001".into(), policy());
}

#[test]
#[should_panic(expected = "program_sha256")]
fn the_program_hash_must_be_a_sha256() {
    ctx(accounts(0), NOW_MS, one_near());
    let mut c = Contract::new();
    let mut p = policy();
    p.program_sha256 = "not-a-hash".into();
    c.register_intent("n_001".into(), p);
}

#[test]
#[should_panic(expected = "nonce may contain only")]
fn nonces_are_plain_identifiers() {
    ctx(accounts(0), NOW_MS, one_near());
    let mut c = Contract::new();
    c.register_intent("n/001".into(), policy());
}

#[test]
fn check_quote_is_a_dry_run() {
    let c = registered();
    let mut q = live_quote();
    q.amount_in = U128(1);
    assert_eq!(failed(&c.check_quote(accounts(0), "n_001".into(), q)), ["amount_in"]);
    assert!(c.get_intent(accounts(0), "n_001".into()).unwrap().quote.is_none());
}

// --- the adapter's own output -------------------------------------------------------------

/// `x3-lang/tests/fixtures/x3_guard_args.json` is generated by `near_intents.py guard-args`
/// and asserted by `test_guard_args_match_the_fixture_the_contract_tests_read`. Running it
/// through the contract here means the Python adapter and this contract cannot drift apart:
/// a field renamed on either side fails one of the two suites.
#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct AdapterArgs {
    register: RegisterArgs,
    quote: QuoteArgs,
    outcome: OutcomeArgs,
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
    quote: QuoteFacts,
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

#[test]
fn the_adapters_generated_arguments_run_the_full_lifecycle() {
    let fixture: AdapterArgs =
        near_sdk::serde_json::from_str(include_str!("../../../tests/fixtures/x3_guard_args.json")).unwrap();
    // The adapter quoted at 03:00:00Z with `timeout 10m`.
    let now = fixture.register.policy.deadline_ms - 600_000;

    ctx(accounts(0), now, one_near());
    let mut c = Contract::new();
    c.register_intent(fixture.register.nonce, fixture.register.policy);

    ctx(accounts(0), now + 5_000, one_near());
    let results = c.accept_quote(fixture.quote.nonce, fixture.quote.quote);
    assert!(results.iter().all(|r| r.ok), "{results:?}");

    ctx(accounts(0), now + 60_000, one_near());
    let o = fixture.outcome;
    let outcome = c.record_outcome(o.nonce.clone(), o.status, o.amount_out, o.refunded_amount, o.tx_hashes);
    assert_eq!(outcome.status, Status::Settled);
    assert_eq!(outcome.floor_met, Some(true));
    assert!(c.get_intent(accounts(0), o.nonce).unwrap().outcome.is_some());
}
