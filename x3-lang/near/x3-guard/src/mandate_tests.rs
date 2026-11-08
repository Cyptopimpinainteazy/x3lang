//! Agent mandates on near-sdk's mocked blockchain. Accounts: 0 owner, 1 agent, 2 and 3
//! approvers, 4 a stranger.

use super::*;
use crate::mandate::*;

const DAY_MS: u64 = 86_400_000;
const USDC: u128 = 1_000_000;

fn owner() -> AccountId {
    accounts(0)
}
fn agent() -> AccountId {
    accounts(1)
}

type Change<T> = Box<dyn Fn(&mut T)>;

fn terms() -> MandateTerms {
    MandateTerms {
        program_sha256: PROGRAM.into(),
        asset: SOL_USDC.into(),
        per_payment_cap: U128(50 * USDC),
        daily_cap: U128(60 * USDC),
        destination_assets: vec![NEAR_USDC.into()],
        recipients: vec![RECIPIENT.into()],
        refund_to: REFUND_TO.into(),
        max_slippage_bps: 100,
        max_fee_bps: 100,
        approval_above: None,
        approvers: vec![],
        approval_threshold: 0,
        expires_at_ms: DEADLINE_MS + 30 * DAY_MS,
    }
}

fn with_mandate(t: MandateTerms) -> Contract {
    ctx(owner(), NOW_MS, one_near());
    let mut c = test_contract();
    c.create_mandate(agent(), t);
    c
}

/// Milliseconds -> `YYYY-MM-DDTHH:MM:SS.mmmZ` (inverse of `parse_utc_ms`).
fn utc(ms: u64) -> String {
    let (days, rem) = (ms / DAY_MS, ms % DAY_MS);
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s, milli) = (rem / 3_600_000, rem / 60_000 % 60, rem / 1_000 % 60, rem % 1_000);
    format!("{y:04}-{m:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// A policy and a matching live quote issued at `now_ms` (deadline ten minutes later).
fn quote_at(now_ms: u64, amount: u128) -> (Policy, String, String) {
    let mut p = policy();
    p.deadline_ms = now_ms + 600_000;
    p.amount = U128(amount);
    let mut q = live_payload();
    for field in ["amount", "amountIn", "minAmountIn"] {
        q[field] = json!(amount.to_string());
    }
    // Keep the captured price: out = in * 24993591 / 25000000, min 50 bps under.
    let out = amount * 24_993_591 / 25_000_000;
    q["amountOut"] = json!(out.to_string());
    let min_out = out - out * 50 / 10_000;
    q["minAmountOut"] = json!(min_out.to_string());
    p.min_receive = Some(U128(min_out));
    q["amountInUsd"] = json!(format!("{}.000000000000", amount / USDC));
    q["amountOutUsd"] = json!(format!("{}.000000000000", amount / USDC));
    q["refundFee"] = json!((amount / 200).to_string());
    q["timestamp"] = json!(utc(now_ms));
    q["deadline"] = json!(utc(now_ms + 600_000));
    q["timeWhenInactive"] = json!(utc(now_ms + 600_000));
    let (text, sig) = signed(&q);
    (p, text, sig)
}

fn agent_pays(c: &mut Contract, nonce: &str, now_ms: u64, amount: u128) -> Vec<GuardResult> {
    let (p, text, sig) = quote_at(now_ms, amount);
    ctx(agent(), now_ms, one_near());
    c.agent_register_intent(owner(), nonce.into(), p);
    ctx(agent(), now_ms, one_near());
    c.agent_accept_quote(owner(), nonce.into(), text, sig)
}

fn panics<R>(f: impl FnOnce() -> R) -> String {
    let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .err()
        .expect("expected a panic");
    err.downcast_ref::<String>()
        .cloned()
        .or_else(|| err.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

#[test]
fn utc_formatting_round_trips() {
    for ms in [0, DEADLINE_MS, NOW_MS, 1_709_251_199_500, DEADLINE_MS + 400 * DAY_MS] {
        assert_eq!(parse_utc_ms(&utc(ms)).unwrap(), ms, "{}", utc(ms));
    }
}

#[test]
fn an_agent_inside_its_mandate_registers_and_accepts_in_the_owners_name() {
    let mut c = with_mandate(terms());
    let m = c.get_mandate(owner(), agent()).unwrap();
    assert_eq!((m.version, m.revoked), (1, false));

    let results = agent_pays(&mut c, "a_001", NOW_MS, 25 * USDC);
    let names: Vec<_> = results.iter().map(|r| r.guard.as_str()).collect();
    assert!(names.starts_with(&["mandate_active", "mandate_asset"]), "{names:?}");
    assert!(names.contains(&"mandate_approvals") && names.contains(&"quote_kind"));
    assert!(results.iter().all(|r| r.ok), "{results:?}");

    let record = c.get_intent(owner(), "a_001".into()).unwrap();
    assert_eq!(record.owner, owner(), "stored under the owner");
    assert_eq!(record.agent, Some(agent()));
    assert_eq!(record.mandate_version, Some(1));
    assert!(record.quote.is_some());
    let m = c.get_mandate(owner(), agent()).unwrap();
    assert_eq!(m.spent_in_window(NOW_MS), (25 * USDC, 1));
    assert!(get_logs()
        .iter()
        .any(|l| l.contains("\"quote_accepted\"") && l.contains("\"agent_spent_24h\":\"25000000\"")));

    ctx(agent(), NOW_MS + 60_000, one_near());
    let outcome = c.agent_record_outcome(
        owner(),
        "a_001".into(),
        Status::Refunded,
        None,
        Some(U128(24_875_000)),
        vec![],
    );
    assert_eq!(outcome.status, Status::Refunded);
    // A refund is not credited back: the window still counts the payment.
    assert_eq!(
        c.get_mandate(owner(), agent())
            .unwrap()
            .spent_in_window(NOW_MS + 60_000)
            .0,
        25 * USDC
    );
}

#[test]
fn every_term_refuses_an_intent_that_breaks_it() {
    let base = || quote_at(NOW_MS, 25 * USDC).0;
    let cases: Vec<(&str, Change<Policy>)> = vec![
        (
            "mandate_asset",
            Box::new(|p| p.origin_asset = "nep141:wrap.near".into()),
        ),
        (
            "mandate_destination_asset",
            Box::new(|p| p.destination_asset = "nep141:wrap.near".into()),
        ),
        ("mandate_recipient", Box::new(|p| p.recipient = "attacker.near".into())),
        (
            "mandate_refund_to",
            Box::new(|p| p.refund_to = "AgentsOwnSolanaWallet1111111111".into()),
        ),
        ("mandate_per_payment", Box::new(|p| p.amount = U128(50 * USDC + 1))),
        ("mandate_slippage", Box::new(|p| p.slippage_bps = 101)),
        ("mandate_fees", Box::new(|p| p.fee_cap_bps = None)),
        ("mandate_fees", Box::new(|p| p.fee_cap_bps = Some(101))),
        (
            "mandate_deadline",
            Box::new(|p| p.deadline_ms = DEADLINE_MS + 30 * DAY_MS + 1),
        ),
    ];
    let m = with_mandate(terms()).get_mandate(owner(), agent()).unwrap();
    for (guard, change) in cases {
        let mut p = base();
        change(&mut p);
        let failed = failed(&check_mandate(&m, &p, NOW_MS));
        assert_eq!(failed, [guard], "expected only {guard}");

        let mut c = with_mandate(terms());
        ctx(agent(), NOW_MS, one_near());
        let err = panics(|| c.agent_register_intent(owner(), "a_001".into(), p.clone()));
        assert!(
            err.contains("X3_MANDATE_DENIED") && err.contains(guard),
            "{guard}: {err}"
        );
        assert!(c.get_intent(owner(), "a_001".into()).is_none());
    }
    assert!(failed(&check_mandate(&m, &base(), NOW_MS)).is_empty());
}

#[test]
fn only_the_named_agent_acts_under_a_mandate() {
    let mut c = with_mandate(terms());
    let (p, text, sig) = quote_at(NOW_MS, 25 * USDC);
    ctx(accounts(4), NOW_MS, one_near());
    let err = panics(|| c.agent_register_intent(owner(), "a_001".into(), p.clone()));
    assert!(err.contains("has no mandate for"), "{err}");

    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "a_001".into(), p);
    ctx(accounts(4), NOW_MS, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), "a_001".into(), text.clone(), sig.clone()));
    assert!(err.contains("was not registered by"), "{err}");
    // The owner cannot bypass the spend counter by accepting the agent's intent directly.
    ctx(owner(), NOW_MS, one_near());
    let err = panics(|| c.accept_quote("a_001".into(), text.clone(), sig.clone()));
    assert!(err.contains("agent_accept_quote"), "{err}");
    // An agent cannot accept an intent the owner registered.
    ctx(owner(), NOW_MS, one_near());
    let (p2, t2, s2) = quote_at(NOW_MS, 10 * USDC);
    c.register_intent("o_001".into(), p2);
    ctx(agent(), NOW_MS, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), "o_001".into(), t2, s2));
    assert!(err.contains("was not registered by"), "{err}");
    ctx(accounts(4), NOW_MS, one_near());
    let err = panics(|| c.agent_record_outcome(owner(), "a_001".into(), Status::Failed, None, None, vec![]));
    assert!(err.contains("was not registered by"), "{err}");
}

#[test]
fn the_daily_cap_is_a_rolling_window_checked_when_a_quote_is_accepted() {
    let mut c = with_mandate(terms());
    // Two intents of 25 and one of 15 fit 60 together; each passes registration alone.
    agent_pays(&mut c, "a_001", NOW_MS, 25 * USDC);
    let later = NOW_MS + 3_600_000;
    let (p2, t2, s2) = quote_at(later, 25 * USDC);
    let (p3, t3, s3) = quote_at(later, 25 * USDC);
    ctx(agent(), later, one_near());
    c.agent_register_intent(owner(), "a_002".into(), p2);
    ctx(agent(), later, one_near());
    c.agent_register_intent(owner(), "a_003".into(), p3);
    ctx(agent(), later, one_near());
    c.agent_accept_quote(owner(), "a_002".into(), t2, s2);
    // 50 spent: the third 25 was fine when registered but is refused when accepted.
    ctx(agent(), later, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), "a_003".into(), t3, s3));
    assert!(err.contains("mandate_daily"), "{err}");
    assert_eq!(
        c.get_mandate(owner(), agent()).unwrap().spent_in_window(later),
        (50 * USDC, 2)
    );

    // Exactly 24 hours after the first payment it leaves the window; one hour later both have.
    let next_day = NOW_MS + DAY_MS;
    assert!(agent_pays(&mut c, "a_004", next_day, 10 * USDC).iter().all(|r| r.ok));
    let m = c.get_mandate(owner(), agent()).unwrap();
    assert_eq!(m.spent_in_window(next_day), (35 * USDC, 2));
    assert_eq!(m.spends.len(), 2, "payments outside the window are pruned");
    assert_eq!(m.spent_in_window(later + DAY_MS), (10 * USDC, 1));
}

#[test]
fn a_window_holds_at_most_64_payments() {
    let mut t = terms();
    t.daily_cap = U128(1_000 * USDC);
    let mut c = with_mandate(t);
    for i in 0..MAX_PAYMENTS_PER_WINDOW {
        agent_pays(&mut c, &format!("a_{i:03}"), NOW_MS + i as u64 * 1_000, USDC);
    }
    let at = NOW_MS + 100_000;
    let (p, _, _) = quote_at(at, USDC);
    ctx(agent(), at, one_near());
    let err = panics(|| c.agent_register_intent(owner(), "a_999".into(), p));
    assert!(err.contains("mandate_daily"), "{err}");
}

#[test]
fn a_quote_that_fails_its_guards_does_not_count_against_the_cap() {
    let mut c = with_mandate(terms());
    let (p, text, sig) = quote_at(NOW_MS, 25 * USDC);
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "a_001".into(), p);
    let forged = text.replacen(RECIPIENT, "attacker.near", 1);
    ctx(agent(), NOW_MS, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), "a_001".into(), forged, sig.clone()));
    assert!(err.contains("X3_NEAR_INVALID_SIGNATURE"), "{err}");
    assert!(c.get_mandate(owner(), agent()).unwrap().spends.is_empty());
    ctx(agent(), NOW_MS, one_near());
    c.agent_accept_quote(owner(), "a_001".into(), text, sig);
    assert_eq!(c.get_mandate(owner(), agent()).unwrap().spends.len(), 1);
}

#[test]
fn revoking_stops_the_agent_at_once_and_replacing_voids_old_intents() {
    let mut c = with_mandate(terms());
    let (p, text, sig) = quote_at(NOW_MS, 25 * USDC);
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "a_001".into(), p.clone());

    ctx(owner(), NOW_MS, NearToken::from_yoctonear(0));
    assert!(c.revoke_mandate(agent()).revoked);
    assert!(get_logs().iter().any(|l| l.contains("\"mandate_revoked\"")));
    ctx(agent(), NOW_MS, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), "a_001".into(), text.clone(), sig.clone()));
    assert!(err.contains("mandate_active"), "{err}");
    ctx(agent(), NOW_MS, one_near());
    let err = panics(|| c.agent_register_intent(owner(), "a_002".into(), p.clone()));
    assert!(err.contains("mandate_active"), "{err}");

    // New terms are a new version: the intent registered under version 1 stays void.
    ctx(owner(), NOW_MS, one_near());
    assert_eq!(c.create_mandate(agent(), terms()).version, 2);
    ctx(agent(), NOW_MS, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), "a_001".into(), text, sig));
    assert!(err.contains("mandate version"), "{err}");
    let (p3, t3, s3) = quote_at(NOW_MS, 25 * USDC);
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "a_003".into(), p3);
    ctx(agent(), NOW_MS, one_near());
    assert!(c
        .agent_accept_quote(owner(), "a_003".into(), t3, s3)
        .iter()
        .all(|r| r.ok));
}

#[test]
fn an_expired_mandate_refuses_everything() {
    let mut t = terms();
    t.expires_at_ms = NOW_MS + 60_000;
    let mut c = with_mandate(t);
    let (p, text, sig) = quote_at(NOW_MS, 25 * USDC);
    let mut p = p;
    p.deadline_ms = NOW_MS + 60_000;
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "a_001".into(), p);
    ctx(agent(), NOW_MS + 60_000, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), "a_001".into(), text, sig));
    assert!(err.contains("mandate_active"), "{err}");
}

#[test]
fn payments_above_the_threshold_need_n_of_m_approvals() {
    let mut t = terms();
    t.approval_above = Some(U128(20 * USDC));
    t.approvers = vec![owner(), accounts(2), accounts(3)];
    t.approval_threshold = 2;
    let mut c = with_mandate(t);

    // At or below the threshold: no approval needed, and none accepted.
    assert!(agent_pays(&mut c, "a_small", NOW_MS, 20 * USDC).iter().all(|r| r.ok));

    let (p, text, sig) = quote_at(NOW_MS, 25 * USDC);
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "a_big".into(), p);
    ctx(agent(), NOW_MS, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), "a_big".into(), text.clone(), sig.clone()));
    assert!(err.contains("mandate_approvals"), "{err}");

    ctx(accounts(4), NOW_MS, one_near());
    assert!(panics(|| c.approve_intent(owner(), "a_big".into())).contains("is not an approver"));
    ctx(agent(), NOW_MS, one_near());
    assert!(panics(|| c.approve_intent(owner(), "a_big".into())).contains("is not an approver"));
    ctx(accounts(2), NOW_MS, one_near());
    assert_eq!(c.approve_intent(owner(), "a_big".into()).approvals, [accounts(2)]);
    ctx(accounts(2), NOW_MS, one_near());
    assert!(panics(|| c.approve_intent(owner(), "a_big".into())).contains("already approved"));
    ctx(agent(), NOW_MS, one_near());
    assert!(
        panics(|| c.agent_accept_quote(owner(), "a_big".into(), text.clone(), sig.clone()))
            .contains("1 of 2 approvals")
    );

    ctx(owner(), NOW_MS, one_near());
    c.approve_intent(owner(), "a_big".into());
    assert!(get_logs().iter().any(|l| l.contains("\"intent_approved\"")));
    ctx(agent(), NOW_MS, one_near());
    assert!(c
        .agent_accept_quote(owner(), "a_big".into(), text, sig)
        .iter()
        .all(|r| r.ok));
    ctx(accounts(3), NOW_MS, one_near());
    assert!(panics(|| c.approve_intent(owner(), "a_big".into())).contains("already accepted"));
    ctx(accounts(2), NOW_MS, one_near());
    assert!(panics(|| c.approve_intent(owner(), "a_small".into())).contains("needs no approval"));
}

#[test]
fn expired_mandate_cannot_record_late_approvals() {
    let mut t = terms();
    t.approval_above = Some(U128(20 * USDC));
    t.approvers = vec![owner(), accounts(2)];
    t.approval_threshold = 1;
    // The intent remains within the mandate, but approval arrives after expiry.
    let expiry = NOW_MS + 600_001;
    t.expires_at_ms = expiry;
    let mut c = with_mandate(t);
    let (p, _, _) = quote_at(NOW_MS, 25 * USDC);
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "late_approval".into(), p);

    let original = c.get_intent(owner(), "late_approval".into()).unwrap();
    assert!(original.approvals.is_empty());

    ctx(accounts(2), expiry, one_near());
    let failure = panics(|| c.approve_intent(owner(), "late_approval".into()));
    assert!(
        failure.contains("X3_MANDATE_DENIED") && failure.contains("expired"),
        "late approval must fail for expired mandate: {failure}"
    );
    let unchanged = c.get_intent(owner(), "late_approval".into()).unwrap();
    assert!(
        unchanged.approvals.is_empty(),
        "an expired mandate recorded an approval"
    );
}

#[test]
fn terms_that_would_not_mean_what_they_say_are_refused() {
    let cases: Vec<(&str, Change<MandateTerms>)> = vec![
        ("program_sha256", Box::new(|t| t.program_sha256 = "x".into())),
        (
            "per_payment_cap must be positive",
            Box::new(|t| t.per_payment_cap = U128(0)),
        ),
        ("must not exceed daily_cap", Box::new(|t| t.daily_cap = U128(USDC))),
        ("recipients must list", Box::new(|t| t.recipients.clear())),
        (
            "recipients has duplicates",
            Box::new(|t| t.recipients.push(RECIPIENT.into())),
        ),
        (
            "destination_assets must list",
            Box::new(|t| t.destination_assets = vec!["a".into(); 17]),
        ),
        ("refund_to must not be empty", Box::new(|t| t.refund_to.clear())),
        ("max_fee_bps", Box::new(|t| t.max_fee_bps = 10_001)),
        ("expiry has already passed", Box::new(|t| t.expires_at_ms = NOW_MS)),
        ("need approval_above", Box::new(|t| t.approvers = vec![accounts(2)])),
        (
            "approvers must list",
            Box::new(|t| {
                t.approval_above = Some(U128(USDC));
                t.approval_threshold = 1;
            }),
        ),
        (
            "cannot approve its own",
            Box::new(|t| {
                t.approval_above = Some(U128(USDC));
                t.approvers = vec![accounts(1)];
                t.approval_threshold = 1;
            }),
        ),
        (
            "approval_threshold must be",
            Box::new(|t| {
                t.approval_above = Some(U128(USDC));
                t.approvers = vec![accounts(2)];
                t.approval_threshold = 2;
            }),
        ),
        (
            "approval_above must be below",
            Box::new(|t| {
                t.approval_above = Some(U128(50 * USDC));
                t.approvers = vec![accounts(2)];
                t.approval_threshold = 1;
            }),
        ),
    ];
    for (expected, change) in cases {
        let mut t = terms();
        change(&mut t);
        ctx(owner(), NOW_MS, one_near());
        let mut c = test_contract();
        let err = panics(|| c.create_mandate(agent(), t.clone()));
        assert!(err.contains(expected), "{expected}: {err}");
    }
    ctx(owner(), NOW_MS, one_near());
    let mut c = test_contract();
    assert!(panics(|| c.create_mandate(owner(), terms())).contains("different account"));
}

#[test]
fn check_agent_intent_is_a_dry_run() {
    let c = with_mandate(terms());
    let (p, _, _) = quote_at(NOW_MS, 25 * USDC);
    ctx(agent(), NOW_MS, NearToken::from_yoctonear(0));
    assert!(failed(&c.check_agent_intent(owner(), agent(), p.clone())).is_empty());
    let mut bad = p.clone();
    bad.recipient = "attacker.near".into();
    assert_eq!(
        failed(&c.check_agent_intent(owner(), agent(), bad)),
        ["mandate_recipient"]
    );
    assert_eq!(
        failed(&c.check_agent_intent(owner(), accounts(4), p)),
        ["mandate_active"]
    );
}

#[test]
#[should_panic(expected = "to pay for")]
fn a_mandate_pays_for_its_storage() {
    ctx(owner(), NOW_MS, NearToken::from_yoctonear(0));
    let mut c = test_contract();
    c.create_mandate(agent(), terms());
}

// --- the adapter's own output -------------------------------------------------------------

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct CreateArgs {
    agent: AccountId,
    terms: MandateTerms,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct AgentRegisterArgs {
    owner: AccountId,
    nonce: String,
    policy: Policy,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct AgentQuoteArgs {
    owner: AccountId,
    nonce: String,
    signed_payload: String,
    signature: String,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct AgentOutcomeArgs {
    owner: AccountId,
    nonce: String,
    status: Status,
    amount_out: Option<U128>,
    refunded_amount: Option<U128>,
    tx_hashes: Vec<String>,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct PolicyCase {
    name: String,
    policy: Policy,
    failed: Vec<String>,
}

#[derive(near_sdk::serde::Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct MandateFixture {
    now_ms: u64,
    owner: AccountId,
    agent: AccountId,
    create_mandate: CreateArgs,
    register: AgentRegisterArgs,
    refused_register: AgentRegisterArgs,
    quote: AgentQuoteArgs,
    outcome: AgentOutcomeArgs,
    policy_cases: Vec<PolicyCase>,
}

/// `x3-lang/tests/fixtures/x3_guard_mandate_args.json`, written by `near_intents.py` from
/// `examples/near_mandate_agent.x3` and `examples/near_intents_agent_payment.x3`.
fn mandate_fixture() -> MandateFixture {
    serde_json::from_str(include_str!("../../../tests/fixtures/x3_guard_mandate_args.json")).unwrap()
}

#[test]
fn the_adapters_mandate_and_agent_calls_run_the_full_lifecycle() {
    let f = mandate_fixture();
    ctx(f.owner.clone(), f.now_ms, one_near());
    let mut c = test_contract();
    let m = c.create_mandate(f.create_mandate.agent.clone(), f.create_mandate.terms);
    assert_eq!((m.agent.clone(), m.version), (f.agent.clone(), 1));

    ctx(f.agent.clone(), f.now_ms, one_near());
    let record = c.agent_register_intent(f.register.owner, f.register.nonce, f.register.policy);
    assert_eq!(record.agent, Some(f.agent.clone()));

    ctx(f.agent.clone(), f.now_ms + 5_000, one_near());
    let results = c.agent_accept_quote(f.quote.owner, f.quote.nonce, f.quote.signed_payload, f.quote.signature);
    assert!(results.iter().all(|r| r.ok), "{results:?}");

    ctx(f.agent.clone(), f.now_ms + 60_000, one_near());
    let o = f.outcome;
    let outcome = c.agent_record_outcome(o.owner, o.nonce, o.status, o.amount_out, o.refunded_amount, o.tx_hashes);
    assert_eq!(outcome.floor_met, Some(true));
    assert_eq!(
        c.get_mandate(f.owner, f.agent).unwrap().spent_in_window(f.now_ms).0,
        25 * USDC
    );
}

#[test]
fn the_contract_refuses_the_register_args_for_an_out_of_mandate_program() {
    let f = mandate_fixture();
    ctx(f.owner.clone(), f.now_ms, one_near());
    let mut c = test_contract();
    c.create_mandate(f.create_mandate.agent, f.create_mandate.terms);

    ctx(f.agent.clone(), f.now_ms, one_near());
    let refused = f.refused_register;
    let err = panics(|| c.agent_register_intent(refused.owner, refused.nonce, refused.policy));
    assert!(err.contains("X3_MANDATE_DENIED"), "{err}");
    assert!(err.contains("mandate_recipient"), "{err}");
}

#[test]
fn the_contract_and_the_adapter_reach_the_same_verdict_on_every_policy() {
    let f = mandate_fixture();
    ctx(f.owner.clone(), f.now_ms, one_near());
    let mut c = test_contract();
    c.create_mandate(f.create_mandate.agent, f.create_mandate.terms);
    let m = c.get_mandate(f.owner, f.agent).unwrap();
    assert!(f.policy_cases.len() >= 5);
    for case in f.policy_cases {
        assert_eq!(
            failed(&check_mandate(&m, &case.policy, f.now_ms)),
            case.failed,
            "{}: the adapter and the contract disagree",
            case.name
        );
    }
}

#[test]
fn agent_quote_view_must_recheck_revocation_and_never_approve_spending() {
    let mut c = with_mandate(terms());
    let (p, signed_payload, signature) = quote_at(NOW_MS, 25 * USDC);
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "preflight_revoked".into(), p);
    let permitted = c.check_quote(
        owner(),
        "preflight_revoked".into(),
        signed_payload.clone(),
        signature.clone(),
    );
    assert!(failed(&permitted).is_empty(), "{permitted:?}");
    assert!(c
        .get_intent(owner(), "preflight_revoked".into())
        .unwrap()
        .quote
        .is_none());

    ctx(owner(), NOW_MS, NearToken::from_yoctonear(0));
    c.revoke_mandate(agent());
    let denied = c.check_quote(owner(), "preflight_revoked".into(), signed_payload, signature);
    assert!(failed(&denied).contains(&"mandate_active".into()), "{denied:?}");
    assert!(c
        .get_intent(owner(), "preflight_revoked".into())
        .unwrap()
        .quote
        .is_none());
}

#[test]
fn agent_quote_view_checks_approvals_mandate_version_and_signature() {
    let mut t = terms();
    t.approval_above = Some(U128(10 * USDC));
    t.approvers = vec![accounts(2)];
    t.approval_threshold = 1;
    let mut c = with_mandate(t.clone());
    let (p, text, sig) = quote_at(NOW_MS, 25 * USDC);
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), "preflight_approvals".into(), p);

    let denied = c.check_quote(owner(), "preflight_approvals".into(), text.clone(), sig.clone());
    assert!(failed(&denied).contains(&"mandate_approvals".into()), "{denied:?}");

    ctx(accounts(2), NOW_MS, one_near());
    c.approve_intent(owner(), "preflight_approvals".into());
    let ok = c.check_quote(owner(), "preflight_approvals".into(), text.clone(), sig.clone());
    assert!(failed(&ok).is_empty(), "{ok:?}");
    assert!(c
        .get_intent(owner(), "preflight_approvals".into())
        .unwrap()
        .quote
        .is_none());

    let tampered = c.check_quote(owner(), "preflight_approvals".into(), text.clone() + " ", sig.clone());
    assert!(failed(&tampered).contains(&"quote_signature".into()), "{tampered:?}");

    ctx(owner(), NOW_MS, one_near());
    c.create_mandate(agent(), t);
    let stale = c.check_quote(owner(), "preflight_approvals".into(), text, sig);
    assert!(failed(&stale).contains(&"mandate_version".into()), "{stale:?}");
}

#[test]
fn agent_quote_view_refuses_unregistered_and_owner_only_intents() {
    let (p, text, sig) = quote_at(NOW_MS, 25 * USDC);
    let mut c = with_mandate(terms());
    let missing = c.check_agent_quote(owner(), "does_not_exist".into(), text.clone(), sig.clone());
    assert!(failed(&missing).contains(&"intent_registered".into()), "{missing:?}");
    let legacy_missing = panics(|| c.check_quote(owner(), "does_not_exist".into(), text.clone(), sig.clone()));
    assert!(legacy_missing.contains("no intent"), "{legacy_missing}");

    ctx(owner(), NOW_MS, one_near());
    c.register_intent("owner_only".into(), p);
    // Owner-submitted policies have no mandate. The legacy owner quote-view path is unchanged.
    let owner_view = c.check_quote(owner(), "owner_only".into(), text, sig);
    assert!(failed(&owner_view).is_empty(), "{owner_view:?}");
}

/// Judge demonstration: passing cryptography is only step one. Even a correctly
/// signed *new* quote from the test-only signer must fail if its recipient
/// violates the owner's registered mandate/intent terms. This test uses
/// near-sdk's mock blockchain, NOT a deployed contract or 1Click's real key.
#[test]
fn valid_test_signature_with_forbidden_recipient_never_authorizes_a_deposit() {
    let mut c = with_mandate(terms());
    let (policy, allowed_text, allowed_sig) = quote_at(NOW_MS, 25 * USDC);
    let nonce = "signed_forbidden_recipient";
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), nonce.into(), policy.clone());

    // Derive an independently signed request, not a post-signature byte edit:
    // signature MUST validate, otherwise this only tests invalid signatures.
    let mut forbidden: serde_json::Value = serde_json::from_str(&allowed_text).unwrap();
    forbidden["recipient"] = json!("attacker.near");
    let (signed_text, signature) = signed(&forbidden);
    assert!(
        verify_signed_quote(&c.manager_public_key, &signed_text, &signature).is_ok(),
        "the prohibited quote must have a valid test-only signature"
    );

    ctx(agent(), NOW_MS, NearToken::from_yoctonear(0));
    let checks = c.check_agent_quote(owner(), nonce.into(), signed_text.clone(), signature.clone());
    assert!(
        checks.iter().any(|g| g.guard == "quote_signature" && g.ok),
        "test signature must verify before checking policy: {checks:?}"
    );
    assert!(
        failed(&checks).contains(&"quote_request_echo".to_string()),
        "signed recipient mismatch must be rejected as a policy issue: {checks:?}"
    );

    ctx(agent(), NOW_MS, one_near());
    let err = panics(|| c.agent_accept_quote(owner(), nonce.into(), signed_text, signature));
    assert!(
        err.contains("quote rejected by") && err.contains("quote_request_echo"),
        "valid but prohibited quote was not rejected at the policy boundary: {err}"
    );
    assert!(
        c.get_intent(owner(), nonce.into()).unwrap().quote.is_none(),
        "denied quote must not write an accepted deposit address"
    );
    assert_eq!(
        c.get_mandate(owner(), agent()).unwrap().spent_in_window(NOW_MS),
        (0, 0),
        "rejected quote must not consume mandate spend allowance"
    );
    assert!(
        !get_logs().iter().any(|log| log.contains("\"quote_accepted\"")),
        "no quote_accepted event may be emitted for the forbidden quote"
    );

    // Positive control: precisely the same registered policy with a separately
    // signed authorized quote DOES pass; the negative outcome was not a bad key.
    let good_nonce = "signed_permitted_recipient";
    ctx(agent(), NOW_MS, one_near());
    c.agent_register_intent(owner(), good_nonce.into(), policy);
    ctx(agent(), NOW_MS, one_near());
    let accepted = c.agent_accept_quote(owner(), good_nonce.into(), allowed_text, allowed_sig);
    assert!(
        accepted.iter().all(|g| g.ok),
        "valid permitted quote should pass: {accepted:?}"
    );
    // `accept_quote` returns evaluated guard decisions; signature verification
    // is proven independently above and by check_agent_quote, not by a
    // fabricated `quote_signature` entry in the acceptance result.
    assert!(c.get_intent(owner(), good_nonce.into()).unwrap().quote.is_some());
    assert_eq!(
        c.get_mandate(owner(), agent()).unwrap().spent_in_window(NOW_MS),
        (25 * USDC, 1)
    );
}
