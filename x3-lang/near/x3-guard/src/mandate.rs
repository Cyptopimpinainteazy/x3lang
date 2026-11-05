//! # Agent mandates: what an owner lets an AI agent's NEAR account do
//!
//! An owner calls [`Contract::create_mandate`] naming an agent account and its [`MandateTerms`].
//! From then on that agent, and only that agent, can register intents **under the owner's name**
//! with [`Contract::agent_register_intent`] and accept their quotes with
//! [`Contract::agent_accept_quote`], and only inside the terms:
//!
//! | Term | Enforced when |
//! |---|---|
//! | `asset`, `per_payment_cap` | register and accept |
//! | `daily_cap` over any rolling 24 hours, at most [`MAX_PAYMENTS_PER_WINDOW`] payments | register (early) and accept (binding) |
//! | `recipients`, `destination_assets` allowlists | register and accept |
//! | `refund_to`, pinned by the owner | register and accept |
//! | `max_slippage_bps`, `max_fee_bps` (a fee cap is mandatory) | register and accept |
//! | `approval_above`: N of the listed `approvers` must call [`Contract::approve_intent`] | accept |
//! | `expires_at_ms`, and the intent's deadline must not outlive it | register and accept |
//! | revocation ([`Contract::revoke_mandate`]) or replacement (a new `version`) | immediately, for every intent not yet accepted |
//!
//! Spending is counted when a quote is **accepted**, because that is when a deposit address is
//! authorized. A refunded swap is not credited back: the counter errs towards spending less.
//!
//! ## What this does and does not stop
//!
//! The contract does not hold the funds; the deposit is sent from the owner's wallet on the
//! origin chain. A mandate makes the agent's authority public and binding for any wallet or
//! runtime that only deposits to an address this contract accepted (`near_intents.py` reports
//! that as `eligible_for_wallet_authorization`). It does not stop someone who holds the
//! origin-chain wallet key from sending funds directly. Give the agent a NEAR account, never
//! the wallet key.

use near_sdk::json_types::U128;
use near_sdk::serde_json::json;
use near_sdk::{env, near, require, AccountId};

use crate::*;

/// Rolling spend window for `daily_cap`.
pub const WINDOW_MS: u64 = 86_400_000;
/// Payments an agent can make in one window; also bounds the mandate's storage.
pub const MAX_PAYMENTS_PER_WINDOW: usize = 64;
const MAX_LIST: usize = 16;
const MAX_APPROVERS: usize = 8;

/// What the owner allows. Every amount is in base units of `asset`.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct MandateTerms {
    /// SHA-256 of the `.x3` source that declared the mandate, lowercase hex.
    pub program_sha256: String,
    /// The only origin asset the agent may spend (1Click ID, `nep141:…`).
    pub asset: String,
    pub per_payment_cap: U128,
    /// Ceiling over any rolling 24 hours.
    pub daily_cap: U128,
    pub destination_assets: Vec<String>,
    pub recipients: Vec<String>,
    /// Where every refund goes. The agent cannot choose it.
    pub refund_to: String,
    pub max_slippage_bps: u32,
    pub max_fee_bps: u32,
    /// Payments above this amount need `approval_threshold` of `approvers`.
    pub approval_above: Option<U128>,
    pub approvers: Vec<AccountId>,
    pub approval_threshold: u8,
    pub expires_at_ms: u64,
}

/// One accepted payment, kept while it is inside the rolling window.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Spend {
    pub at_ms: u64,
    pub amount: U128,
    pub nonce: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Mandate {
    pub owner: AccountId,
    pub agent: AccountId,
    pub terms: MandateTerms,
    /// Starts at 1 and increases with every replacement. An intent is accepted only under the
    /// version it was registered with.
    pub version: u32,
    pub revoked: bool,
    pub created_at_ms: u64,
    /// Accepted payments in the current window (older ones are pruned on write).
    pub spends: Vec<Spend>,
}

impl Mandate {
    /// `(total, count)` of payments accepted in the 24 hours before `now_ms`.
    pub fn spent_in_window(&self, now_ms: u64) -> (u128, usize) {
        let since = now_ms.saturating_sub(WINDOW_MS);
        self.spends
            .iter()
            .filter(|s| s.at_ms > since)
            .fold((0u128, 0usize), |(sum, n), s| (sum.saturating_add(s.amount.0), n + 1))
    }

    /// Approvals a payment of `amount` needs (0 at or below the threshold).
    pub fn approvals_needed(&self, amount: u128) -> u8 {
        match self.terms.approval_above {
            Some(limit) if amount > limit.0 => self.terms.approval_threshold,
            _ => 0,
        }
    }
}

fn mandate_key(owner: &AccountId, agent: &AccountId) -> String {
    format!("{owner}/{agent}")
}

fn has_duplicates<T: PartialEq>(items: &[T]) -> bool {
    items.iter().enumerate().any(|(i, a)| items[..i].contains(a))
}

/// Refuse terms that would not mean what they say. Panics with the reason.
pub fn validate_terms(owner: &AccountId, agent: &AccountId, t: &MandateTerms, now_ms: u64) {
    require!(agent != owner, "the agent must be a different account from the owner");
    require!(
        is_hex64(&t.program_sha256),
        "program_sha256 must be 64 lowercase hex characters"
    );
    require_text("asset", &t.asset);
    require_text("refund_to", &t.refund_to);
    require!(t.per_payment_cap.0 > 0, "per_payment_cap must be positive");
    require!(
        t.per_payment_cap.0 <= t.daily_cap.0,
        "per_payment_cap must not exceed daily_cap"
    );
    for (field, list) in [
        ("recipients", &t.recipients),
        ("destination_assets", &t.destination_assets),
    ] {
        require!(
            !list.is_empty() && list.len() <= MAX_LIST,
            format!("{field} must list 1..={MAX_LIST} entries")
        );
        require!(!has_duplicates(list), format!("{field} has duplicates"));
        for item in list {
            require_text(field, item);
        }
    }
    require!(
        t.max_slippage_bps as u128 <= BPS,
        "max_slippage_bps must be at most 10000"
    );
    require!(t.max_fee_bps as u128 <= BPS, "max_fee_bps must be at most 10000");
    match t.approval_above {
        Some(limit) => {
            require!(
                limit.0 < t.per_payment_cap.0,
                "approval_above must be below per_payment_cap, or no payment could need approval"
            );
            require!(
                !t.approvers.is_empty() && t.approvers.len() <= MAX_APPROVERS,
                format!("approvers must list 1..={MAX_APPROVERS} accounts")
            );
            require!(!has_duplicates(&t.approvers), "approvers has duplicates");
            require!(
                !t.approvers.contains(agent),
                "the agent cannot approve its own payments"
            );
            require!(
                t.approval_threshold >= 1 && t.approval_threshold as usize <= t.approvers.len(),
                "approval_threshold must be 1..=the number of approvers"
            );
        }
        None => require!(
            t.approvers.is_empty() && t.approval_threshold == 0,
            "approvers and approval_threshold need approval_above"
        ),
    }
    require!(t.expires_at_ms > now_ms, "the mandate's expiry has already passed");
}

/// Check an intent's policy against a mandate at `now_ms`. Pure, the same checks
/// `near_intents.py` runs before it asks the agent to register.
pub fn check_mandate(m: &Mandate, policy: &Policy, now_ms: u64) -> Vec<GuardResult> {
    let t = &m.terms;
    let mut out = Vec::new();
    let mut record = |guard: &str, ok: bool, detail: String| {
        out.push(GuardResult {
            guard: guard.into(),
            ok,
            detail,
        })
    };
    record(
        "mandate_active",
        !m.revoked && now_ms < t.expires_at_ms,
        format!(
            "version {}, revoked {}, expires {}, now {now_ms}",
            m.version, m.revoked, t.expires_at_ms
        ),
    );
    record(
        "mandate_asset",
        policy.origin_asset == t.asset,
        format!("spends {}, mandate allows {}", policy.origin_asset, t.asset),
    );
    record(
        "mandate_destination_asset",
        t.destination_assets.contains(&policy.destination_asset),
        format!("{}, allowed {:?}", policy.destination_asset, t.destination_assets),
    );
    record(
        "mandate_recipient",
        t.recipients.contains(&policy.recipient),
        format!("{}, allowed {:?}", policy.recipient, t.recipients),
    );
    record(
        "mandate_refund_to",
        policy.refund_to == t.refund_to,
        format!("refunds to {}, mandate pins {}", policy.refund_to, t.refund_to),
    );
    let amount = policy.amount.0;
    record(
        "mandate_per_payment",
        amount <= t.per_payment_cap.0,
        format!("{amount} of at most {}", t.per_payment_cap.0),
    );
    let (spent, count) = m.spent_in_window(now_ms);
    record(
        "mandate_daily",
        spent.checked_add(amount).is_some_and(|total| total <= t.daily_cap.0) && count < MAX_PAYMENTS_PER_WINDOW,
        format!(
            "{spent} spent in {count} payments over 24h, plus {amount}, cap {}",
            t.daily_cap.0
        ),
    );
    record(
        "mandate_slippage",
        policy.slippage_bps <= t.max_slippage_bps,
        format!("{} bps of at most {}", policy.slippage_bps, t.max_slippage_bps),
    );
    record(
        "mandate_fees",
        policy.fee_cap_bps.is_some_and(|cap| cap <= t.max_fee_bps),
        format!(
            "fee cap {:?} bps, mandate requires at most {}",
            policy.fee_cap_bps, t.max_fee_bps
        ),
    );
    record(
        "mandate_deadline",
        policy.deadline_ms <= t.expires_at_ms,
        format!(
            "intent deadline {}, mandate expires {}",
            policy.deadline_ms, t.expires_at_ms
        ),
    );
    out
}

fn refuse_failed(results: &[GuardResult], what: &str) {
    let failed: Vec<String> = results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| format!("{} ({})", r.guard, r.detail))
        .collect();
    require!(
        failed.is_empty(),
        format!("X3_MANDATE_DENIED: {what} refused by {}", failed.join("; "))
    );
}

impl Contract {
    fn load_mandate(&self, owner: &AccountId, agent: &AccountId) -> Mandate {
        self.mandates
            .get(&mandate_key(owner, agent))
            .cloned()
            .unwrap_or_else(|| env::panic_str(&format!("X3_MANDATE_DENIED: {owner} has no mandate for {agent}")))
    }

    fn store_mandate(&mut self, m: Mandate) {
        self.mandates.insert(mandate_key(&m.owner, &m.agent), m);
        self.mandates.flush();
    }

    /// The record an agent may act on: registered by this agent, under the current mandate.
    fn agent_record(&self, owner: &AccountId, nonce: &str) -> (IntentRecord, Mandate) {
        let agent = env::predecessor_account_id();
        let record = self.load_intent(owner, nonce);
        require!(
            record.agent.as_ref() == Some(&agent),
            format!("X3_MANDATE_DENIED: intent {nonce:?} was not registered by {agent}")
        );
        let m = self.load_mandate(owner, &agent);
        require!(
            record.mandate_version == Some(m.version),
            format!(
                "X3_MANDATE_DENIED: intent {nonce:?} was registered under mandate version {:?}; the mandate is now version {}",
                record.mandate_version, m.version
            )
        );
        (record, m)
    }
}

#[near]
impl Contract {
    /// Create or replace the caller's mandate for `agent`. Replacing increases the version, which
    /// voids every intent the agent registered under the old terms and has not had accepted.
    /// Payments already counted in the rolling window stay counted.
    #[payable]
    pub fn create_mandate(&mut self, agent: AccountId, terms: MandateTerms) -> Mandate {
        let initial = env::storage_usage();
        let owner = env::predecessor_account_id();
        let now = env::block_timestamp_ms();
        validate_terms(&owner, &agent, &terms, now);
        let previous = self.mandates.get(&mandate_key(&owner, &agent)).cloned();
        let (version, spends) = match previous {
            Some(p) => (
                p.version
                    .checked_add(1)
                    .unwrap_or_else(|| env::panic_str("mandate version overflow")),
                p.spends,
            ),
            None => (1, Vec::new()),
        };
        let m = Mandate {
            owner: owner.clone(),
            agent: agent.clone(),
            terms,
            version,
            revoked: false,
            created_at_ms: now,
            spends,
        };
        self.store_mandate(m.clone());
        emit(
            "mandate_created",
            json!({
                "owner": owner,
                "agent": agent,
                "version": version,
                "program_sha256": m.terms.program_sha256,
                "per_payment_cap": m.terms.per_payment_cap,
                "daily_cap": m.terms.daily_cap,
                "expires_at_ms": m.terms.expires_at_ms,
            }),
        );
        settle_storage(initial);
        m
    }

    /// Stop the agent at once. Intents it registered and that have no accepted quote can no
    /// longer be accepted; a quote already accepted is not undone (its deposit address exists).
    pub fn revoke_mandate(&mut self, agent: AccountId) -> Mandate {
        let owner = env::predecessor_account_id();
        let mut m = self.load_mandate(&owner, &agent);
        require!(!m.revoked, "the mandate is already revoked");
        m.revoked = true;
        self.store_mandate(m.clone());
        emit(
            "mandate_revoked",
            json!({ "owner": owner, "agent": agent, "version": m.version }),
        );
        m
    }

    /// The agent registers an intent in the owner's name. Every term except approvals is checked
    /// now; the binding spend check runs again when the quote is accepted.
    #[payable]
    pub fn agent_register_intent(&mut self, owner: AccountId, nonce: String, policy: Policy) -> IntentRecord {
        let initial = env::storage_usage();
        let agent = env::predecessor_account_id();
        let m = self.load_mandate(&owner, &agent);
        refuse_failed(&check_mandate(&m, &policy, env::block_timestamp_ms()), "intent");
        let record = self.insert_intent(owner, nonce, policy, Some((agent, m.version)));
        settle_storage(initial);
        record
    }

    /// An approver listed in the mandate signs off on an agent intent above `approval_above`.
    #[payable]
    pub fn approve_intent(&mut self, owner: AccountId, nonce: String) -> IntentRecord {
        let initial = env::storage_usage();
        let approver = env::predecessor_account_id();
        let mut record = self.load_intent(&owner, &nonce);
        let agent = record
            .agent
            .clone()
            .unwrap_or_else(|| env::panic_str("only an agent's intent takes approvals"));
        let m = self.load_mandate(&owner, &agent);
        require!(
            record.mandate_version == Some(m.version),
            "X3_MANDATE_DENIED: the intent's mandate version is no longer current"
        );
        require!(!m.revoked, "X3_MANDATE_DENIED: the mandate is revoked");
        require!(
            m.approvals_needed(record.policy.amount.0) > 0,
            "this payment is within approval_above and needs no approval"
        );
        require!(record.quote.is_none(), "a quote is already accepted for this intent");
        require!(
            m.terms.approvers.contains(&approver),
            format!("{approver} is not an approver on this mandate")
        );
        require!(
            !record.approvals.contains(&approver),
            format!("{approver} has already approved")
        );
        record.approvals.push(approver.clone());
        emit(
            "intent_approved",
            json!({
                "owner": owner,
                "nonce": nonce,
                "approver": approver,
                "approvals": record.approvals.len(),
                "needed": m.approvals_needed(record.policy.amount.0),
            }),
        );
        self.store_intent(record.clone());
        settle_storage(initial);
        record
    }

    /// The agent accepts a quote. The mandate is checked again at this moment (revocation,
    /// expiry, version, approvals, rolling spend), then the quote's signature and guards exactly
    /// as [`Contract::accept_quote`]. On success the payment is counted against the window.
    #[payable]
    pub fn agent_accept_quote(
        &mut self,
        owner: AccountId,
        nonce: String,
        signed_payload: String,
        signature: String,
    ) -> Vec<GuardResult> {
        let initial = env::storage_usage();
        let (record, mut m) = self.agent_record(&owner, &nonce);
        let now = env::block_timestamp_ms();
        let amount = record.policy.amount.0;
        let mut results = check_mandate(&m, &record.policy, now);
        let needed = m.approvals_needed(amount);
        results.push(GuardResult {
            guard: "mandate_approvals".into(),
            ok: record.approvals.len() >= needed as usize,
            detail: format!("{} of {needed} approvals", record.approvals.len()),
        });
        refuse_failed(&results, "quote");

        m.spends.retain(|s| s.at_ms > now.saturating_sub(WINDOW_MS));
        m.spends.push(Spend {
            at_ms: now,
            amount: U128(amount),
            nonce: nonce.clone(),
        });
        let (spent, _) = m.spent_in_window(now);
        results.extend(self.accept_verified(record, signed_payload, signature, Some(spent)));
        self.store_mandate(m);
        settle_storage(initial);
        results
    }

    /// The agent records how its swap ended (the owner can use `record_outcome`).
    #[payable]
    pub fn agent_record_outcome(
        &mut self,
        owner: AccountId,
        nonce: String,
        status: Status,
        amount_out: Option<U128>,
        refunded_amount: Option<U128>,
        tx_hashes: Vec<String>,
    ) -> Outcome {
        let initial = env::storage_usage();
        let agent = env::predecessor_account_id();
        let record = self.load_intent(&owner, &nonce);
        require!(
            record.agent.as_ref() == Some(&agent),
            format!("X3_MANDATE_DENIED: intent {nonce:?} was not registered by {agent}")
        );
        let outcome = self.record_outcome_for(record, status, amount_out, refunded_amount, tx_hashes);
        settle_storage(initial);
        outcome
    }

    pub fn get_mandate(&self, owner: AccountId, agent: AccountId) -> Option<Mandate> {
        self.mandates.get(&mandate_key(&owner, &agent)).cloned()
    }

    /// Read-only preflight for a registered agent quote. Rechecks the same mandate state,
    /// approvals, and signed quote constraints that acceptance checks. This view NEVER grants
    /// wallet authority and its result can go stale before the agent submits a transaction.
    ///
    /// Unlike the owner-only `check_quote` view, this includes revocation, changed mandate
    /// versions, payment counters, and missing approvals. Missing records fail closed as
    /// explicit negative results instead of returning an apparently green quote-only verdict.
    pub fn check_agent_quote(
        &self,
        owner: AccountId,
        nonce: String,
        signed_payload: String,
        signature: String,
    ) -> Vec<GuardResult> {
        let Some(record) = self.intents.get(&key(&owner, &nonce)) else {
            return vec![GuardResult {
                guard: "intent_registered".into(),
                ok: false,
                detail: format!("no intent {nonce:?} registered by {owner}"),
            }];
        };
        let Some(agent) = &record.agent else {
            return vec![GuardResult {
                guard: "mandate_agent".into(),
                ok: false,
                detail: format!("intent {nonce:?} is not agent-registered"),
            }];
        };
        let Some(m) = self.mandates.get(&mandate_key(&owner, agent)) else {
            return vec![GuardResult {
                guard: "mandate_active".into(),
                ok: false,
                detail: format!("{owner} has no mandate for {agent}"),
            }];
        };

        let now = env::block_timestamp_ms();
        let mut out = vec![GuardResult {
            guard: "mandate_version".into(),
            ok: record.mandate_version == Some(m.version),
            detail: format!(
                "intent version {:?}, mandate version {}",
                record.mandate_version, m.version
            ),
        }];
        out.extend(check_mandate(m, &record.policy, now));
        let needed = m.approvals_needed(record.policy.amount.0) as usize;
        let approval_count = record
            .approvals
            .iter()
            .filter(|a| m.terms.approvers.contains(a))
            .count();
        out.push(GuardResult {
            guard: "mandate_approvals".into(),
            ok: approval_count >= needed,
            detail: format!("{approval_count} of {needed} approvals"),
        });
        out.push(GuardResult {
            guard: "quote_unaccepted".into(),
            ok: record.quote.is_none(),
            detail: if record.quote.is_some() {
                "quote already accepted".into()
            } else {
                "quote not yet accepted".into()
            },
        });

        match verify_signed_quote(&self.manager_public_key, &signed_payload, &signature) {
            Err(reason) => out.push(GuardResult {
                guard: "quote_signature".into(),
                ok: false,
                detail: reason,
            }),
            Ok(facts) => {
                out.push(GuardResult {
                    guard: "quote_signature".into(),
                    ok: true,
                    detail: facts.quote_hash.clone(),
                });
                out.extend(evaluate(&record.policy, &facts, now));
            }
        }
        out
    }

    /// Dry-run the mandate for a policy the agent is about to register. Changes nothing.
    pub fn check_agent_intent(&self, owner: AccountId, agent: AccountId, policy: Policy) -> Vec<GuardResult> {
        match self.mandates.get(&mandate_key(&owner, &agent)) {
            None => vec![GuardResult {
                guard: "mandate_active".into(),
                ok: false,
                detail: format!("{owner} has no mandate for {agent}"),
            }],
            Some(m) => check_mandate(m, &policy, env::block_timestamp_ms()),
        }
    }
}
