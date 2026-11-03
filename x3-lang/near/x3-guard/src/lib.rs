//! # x3-guard — the X3 policy of a NEAR Intents swap, enforced on chain
//!
//! `x3-lang/near_intents.py` lowers an X3 `intent` to a 1Click quote request and checks the
//! returned quote against the program's guards. This contract makes three of those decisions
//! public and permanent:
//!
//! 1. **The policy.** [`Contract::register_intent`] stores what the program allows — the
//!    program's SHA-256, assets, amount, `min_receive` floor, slippage bound, fee cap, receivers
//!    and deadline — under `(caller, nonce)`.
//! 2. **The quote.** [`Contract::accept_quote`] re-runs every quote guard *here*, against the
//!    stored policy, and records the accepted quote (its response hash, 1Click signature and
//!    deposit address). A policy accepts **one** quote, so a nonce opens one deposit address:
//!    replay protection that anyone can check, not a file on one machine.
//! 3. **The outcome.** [`Contract::record_outcome`] records how the swap ended, once.
//!
//! ## Trust model, stated so it does not have to be inferred
//!
//! - The quote facts are submitted by the policy's owner. The contract proves they satisfy
//!   the policy and binds them to the 1Click response by hash and signature; anyone holding
//!   the receipt can recompute the hash and check the signature off chain. 1Click does not
//!   publish the signed message format, so the signature is stored, not verified here.
//! - The outcome is the owner's attestation of what `GET /v0/status` reported. The contract
//!   checks it is consistent (terminal, once, after a quote) and records whether the delivered
//!   amount met the guaranteed floor — it does not observe the destination chain.
//!
//! ## Storage
//!
//! Every write is paid by the caller: attach NEAR to cover the bytes added, and the excess is
//! refunded in the same call.

use near_sdk::json_types::U128;
use near_sdk::serde_json::{self, json};
use near_sdk::store::LookupMap;
use near_sdk::{env, near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault, Promise};

/// NEP-297 event standard name.
pub const EVENT_STANDARD: &str = "x3guard";
pub const EVENT_VERSION: &str = "1.0.0";

const MAX_TEXT: usize = 256;
const MAX_NONCE: usize = 128;
const MAX_TX_HASHES: usize = 16;
const BPS: u128 = 10_000;
/// Fixed-point scale for 1Click's decimal USD strings (it sends 12 fractional digits).
const USD_DECIMALS: usize = 18;

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum StorageKey {
    Intents,
}

/// What an X3 program allows, as lowered by `near_intents.py`.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Policy {
    /// SHA-256 of the `.x3` source, lowercase hex.
    pub program_sha256: String,
    /// 1Click asset IDs (`nep141:…`).
    pub origin_asset: String,
    pub destination_asset: String,
    /// Exact input, in origin base units.
    pub amount: U128,
    /// Floor on the quote's guaranteed output (`minAmountOut`), destination base units.
    pub min_receive: Option<U128>,
    /// `require slippage`, whole basis points.
    pub slippage_bps: u32,
    /// `require fees`, whole basis points; bounds the swap's cost and the refund fee.
    pub fee_cap_bps: Option<u32>,
    pub recipient: String,
    pub refund_to: String,
    /// The intent's `timeout` as a Unix time in milliseconds.
    pub deadline_ms: u64,
}

/// The facts of a 1Click quote that the guards read, copied from the quote response.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct QuoteFacts {
    /// SHA-256 of the canonical quote response JSON, lowercase hex (`quote_response_sha256` in
    /// the adapter's receipt).
    pub quote_response_sha256: String,
    /// 1Click's signature over the quote (`ed25519:…`).
    pub signature: String,
    pub deposit_address: String,
    // The request as 1Click echoed it.
    pub origin_asset: String,
    pub destination_asset: String,
    pub recipient: String,
    pub refund_to: String,
    pub slippage_tolerance_bps: u32,
    // The quote.
    pub amount_in: U128,
    pub amount_out: U128,
    pub min_amount_out: U128,
    pub amount_in_usd: String,
    pub amount_out_usd: String,
    pub refund_fee: Option<U128>,
    /// The quote's own deadline, Unix milliseconds.
    pub quote_deadline_ms: u64,
    /// 1Click's estimate of execution time after the deposit, seconds.
    pub time_estimate_s: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Settled,
    Refunded,
    Failed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Outcome {
    pub status: Status,
    pub amount_out: Option<U128>,
    pub refunded_amount: Option<U128>,
    pub tx_hashes: Vec<String>,
    /// For `Settled`: whether `amount_out` met the accepted quote's guaranteed minimum.
    pub floor_met: Option<bool>,
    pub recorded_at_ms: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct IntentRecord {
    pub owner: AccountId,
    pub nonce: String,
    pub policy: Policy,
    pub registered_at_ms: u64,
    pub quote: Option<QuoteFacts>,
    pub quote_accepted_at_ms: Option<u64>,
    pub outcome: Option<Outcome>,
}

/// One guard's verdict, the same shape the adapter reports.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq)]
pub struct GuardResult {
    pub guard: String,
    pub ok: bool,
    pub detail: String,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    intents: LookupMap<String, IntentRecord>,
}

fn key(owner: &AccountId, nonce: &str) -> String {
    format!("{owner}/{nonce}")
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn require_text(field: &str, value: &str) {
    require!(!value.is_empty(), format!("{field} must not be empty"));
    require!(
        value.len() <= MAX_TEXT,
        format!("{field} is longer than {MAX_TEXT} bytes")
    );
}

fn require_nonce(nonce: &str) {
    require!(
        !nonce.is_empty() && nonce.len() <= MAX_NONCE,
        format!("nonce must be 1..={MAX_NONCE} bytes")
    );
    require!(
        nonce
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.'),
        "nonce may contain only ASCII letters, digits, '_', '-' and '.'"
    );
}

/// Parse a non-negative decimal string into a fixed-point integer with [`USD_DECIMALS`].
/// More fractional digits than that are refused rather than rounded.
pub fn parse_usd(field: &str, text: &str) -> u128 {
    let (whole, frac) = match text.split_once('.') {
        Some((w, f)) => (w, f),
        None => (text, ""),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    require!(
        digits(whole) && (frac.is_empty() || digits(frac)),
        format!("{field} is not a decimal: {text:?}")
    );
    require!(
        frac.len() <= USD_DECIMALS,
        format!("{field} has more than {USD_DECIMALS} fractional digits")
    );
    let scale = 10u128.pow(USD_DECIMALS as u32);
    let whole: u128 = whole
        .parse()
        .unwrap_or_else(|_| env::panic_str(&format!("{field} is too large")));
    let frac_padded = format!("{frac:0<USD_DECIMALS$}");
    let frac: u128 = frac_padded.parse().unwrap_or(0);
    whole
        .checked_mul(scale)
        .and_then(|w| w.checked_add(frac))
        .unwrap_or_else(|| env::panic_str(&format!("{field} is too large")))
}

/// `a * BPS <= cap * b`, without overflow panics on honest inputs.
fn within_bps(numerator: u128, denominator: u128, cap_bps: u128) -> bool {
    match (numerator.checked_mul(BPS), denominator.checked_mul(cap_bps)) {
        (Some(lhs), Some(rhs)) => lhs <= rhs,
        // Values this large are not quotes; refuse rather than guess.
        _ => false,
    }
}

/// Run every guard of `policy` over `quote` at `now_ms`. Pure, so it is unit-tested directly
/// and exposed as a view ([`Contract::check_quote`]).
pub fn evaluate(policy: &Policy, quote: &QuoteFacts, now_ms: u64) -> Vec<GuardResult> {
    let mut out = Vec::new();
    let mut record = |guard: &str, ok: bool, detail: String| {
        out.push(GuardResult {
            guard: guard.into(),
            ok,
            detail,
        })
    };

    let mismatched: Vec<&str> = [
        ("originAsset", quote.origin_asset == policy.origin_asset),
        ("destinationAsset", quote.destination_asset == policy.destination_asset),
        ("recipient", quote.recipient == policy.recipient),
        ("refundTo", quote.refund_to == policy.refund_to),
        ("slippageTolerance", quote.slippage_tolerance_bps == policy.slippage_bps),
    ]
    .into_iter()
    .filter(|(_, same)| !same)
    .map(|(name, _)| name)
    .collect();
    record(
        "quote_request_echo",
        mismatched.is_empty(),
        format!("mismatched: {mismatched:?}"),
    );

    let amount = policy.amount.0;
    let (amount_in, amount_out, min_out) = (quote.amount_in.0, quote.amount_out.0, quote.min_amount_out.0);
    record(
        "amount_in",
        amount_in == amount,
        format!("expected {amount}, quoted {amount_in}"),
    );

    if let Some(floor) = policy.min_receive {
        record(
            "min_receive",
            min_out >= floor.0,
            format!("floor {}, guaranteed {min_out}", floor.0),
        );
    }

    if amount_out == 0 || min_out > amount_out {
        record(
            "slippage",
            false,
            format!("amountOut {amount_out}, minAmountOut {min_out}"),
        );
    } else {
        // One bps of headroom covers the solver rounding minAmountOut down to a whole unit.
        let bound = policy.slippage_bps as u128 + 1;
        record(
            "slippage",
            within_bps(amount_out - min_out, amount_out, bound),
            format!(
                "spread {} of {amount_out}, bound {} bps",
                amount_out - min_out,
                policy.slippage_bps
            ),
        );
    }

    if let Some(cap) = policy.fee_cap_bps {
        let usd_in = parse_usd("amount_in_usd", &quote.amount_in_usd);
        let usd_out = parse_usd("amount_out_usd", &quote.amount_out_usd);
        if usd_in == 0 {
            record(
                "fees",
                false,
                "no USD price for the input; cost cannot be measured".into(),
            );
        } else {
            let cost = usd_in.saturating_sub(usd_out);
            record(
                "fees",
                within_bps(cost, usd_in, cap as u128),
                format!("cost {cost} of {usd_in} (1e-18 USD), cap {cap} bps"),
            );
        }
        match quote.refund_fee {
            None => record(
                "refund_fee",
                false,
                "quote states no refundFee; refund cost unknown".into(),
            ),
            Some(fee) => record(
                "refund_fee",
                within_bps(fee.0, amount, cap as u128),
                format!("refund fee {} of {amount}, cap {cap} bps", fee.0),
            ),
        }
    }

    record(
        "deposit_address",
        !quote.deposit_address.is_empty(),
        quote.deposit_address.clone(),
    );
    record("quote_signature", !quote.signature.is_empty(), String::new());
    record(
        "quote_response_hash",
        is_hex64(&quote.quote_response_sha256),
        "64 lowercase hex characters".into(),
    );

    let remaining_ms = quote.quote_deadline_ms.saturating_sub(now_ms);
    let needed_ms = quote.time_estimate_s.saturating_mul(1000);
    record(
        "deadline",
        now_ms < policy.deadline_ms
            && now_ms < quote.quote_deadline_ms
            && quote.quote_deadline_ms <= policy.deadline_ms
            && remaining_ms >= needed_ms,
        format!(
            "now {now_ms}, quote deadline {}, policy deadline {}, needs {needed_ms} ms",
            quote.quote_deadline_ms, policy.deadline_ms
        ),
    );
    out
}

fn emit(event: &str, data: serde_json::Value) {
    let payload = json!({
        "standard": EVENT_STANDARD,
        "version": EVENT_VERSION,
        "event": event,
        "data": [data],
    });
    env::log_str(&format!("EVENT_JSON:{payload}"));
}

/// Charge the caller for storage added since `initial_usage` and refund the rest of the deposit.
fn settle_storage(initial_usage: u64) {
    let used = env::storage_usage().saturating_sub(initial_usage);
    let cost = env::storage_byte_cost().saturating_mul(used as u128);
    let attached = env::attached_deposit();
    require!(
        attached >= cost,
        format!(
            "attach at least {} yoctoNEAR to pay for {used} bytes of storage",
            cost.as_yoctonear()
        )
    );
    let refund = attached.saturating_sub(cost);
    if refund > NearToken::from_yoctonear(0) {
        Promise::new(env::predecessor_account_id()).transfer(refund).detach();
    }
}

#[near]
impl Contract {
    #[init]
    pub fn new() -> Self {
        Self {
            intents: LookupMap::new(StorageKey::Intents),
        }
    }

    /// Store an X3 program's policy under `(caller, nonce)`. A nonce is registered once.
    #[payable]
    pub fn register_intent(&mut self, nonce: String, policy: Policy) -> IntentRecord {
        let initial = env::storage_usage();
        let owner = env::predecessor_account_id();
        require_nonce(&nonce);
        require!(
            is_hex64(&policy.program_sha256),
            "program_sha256 must be 64 lowercase hex characters"
        );
        for (field, value) in [
            ("origin_asset", &policy.origin_asset),
            ("destination_asset", &policy.destination_asset),
            ("recipient", &policy.recipient),
            ("refund_to", &policy.refund_to),
        ] {
            require_text(field, value);
        }
        require!(policy.amount.0 > 0, "amount must be positive");
        require!(policy.slippage_bps as u128 <= BPS, "slippage_bps must be at most 10000");
        if let Some(cap) = policy.fee_cap_bps {
            require!(cap as u128 <= BPS, "fee_cap_bps must be at most 10000");
        }
        require!(
            policy.deadline_ms > env::block_timestamp_ms(),
            "the policy's deadline has already passed"
        );

        let k = key(&owner, &nonce);
        require!(
            !self.intents.contains_key(&k),
            format!("nonce {nonce:?} is already registered by {owner}")
        );

        let record = IntentRecord {
            owner: owner.clone(),
            nonce: nonce.clone(),
            policy,
            registered_at_ms: env::block_timestamp_ms(),
            quote: None,
            quote_accepted_at_ms: None,
            outcome: None,
        };
        self.intents.insert(k, record.clone());
        self.intents.flush();
        emit(
            "intent_registered",
            json!({ "owner": owner, "nonce": nonce, "program_sha256": record.policy.program_sha256 }),
        );
        settle_storage(initial);
        record
    }

    /// Re-run every quote guard against the stored policy and, if all pass, record the quote.
    /// A policy accepts one quote; a failing quote changes nothing and may be retried.
    #[payable]
    pub fn accept_quote(&mut self, nonce: String, quote: QuoteFacts) -> Vec<GuardResult> {
        let initial = env::storage_usage();
        let owner = env::predecessor_account_id();
        let k = key(&owner, &nonce);
        let mut record = self
            .intents
            .get(&k)
            .cloned()
            .unwrap_or_else(|| env::panic_str(&format!("no intent {nonce:?} registered by {owner}")));
        require!(
            record.quote.is_none(),
            format!(
                "intent {nonce:?} already accepted quote {}",
                record
                    .quote
                    .as_ref()
                    .map(|q| q.quote_response_sha256.as_str())
                    .unwrap_or("")
            )
        );
        for (field, value) in [
            ("signature", &quote.signature),
            ("deposit_address", &quote.deposit_address),
            ("amount_in_usd", &quote.amount_in_usd),
            ("amount_out_usd", &quote.amount_out_usd),
        ] {
            require_text(field, value);
        }

        let now = env::block_timestamp_ms();
        let results = evaluate(&record.policy, &quote, now);
        let failed: Vec<&str> = results.iter().filter(|r| !r.ok).map(|r| r.guard.as_str()).collect();
        require!(
            failed.is_empty(),
            format!("quote rejected by {}; do not deposit", failed.join(", "))
        );

        emit(
            "quote_accepted",
            json!({
                "owner": owner,
                "nonce": nonce,
                "quote_response_sha256": quote.quote_response_sha256,
                "deposit_address": quote.deposit_address,
                "min_amount_out": quote.min_amount_out,
            }),
        );
        record.quote = Some(quote);
        record.quote_accepted_at_ms = Some(now);
        self.intents.insert(k, record);
        self.intents.flush();
        settle_storage(initial);
        results
    }

    /// Record how the swap ended, once, after a quote was accepted.
    #[payable]
    pub fn record_outcome(
        &mut self,
        nonce: String,
        status: Status,
        amount_out: Option<U128>,
        refunded_amount: Option<U128>,
        tx_hashes: Vec<String>,
    ) -> Outcome {
        let initial = env::storage_usage();
        let owner = env::predecessor_account_id();
        let k = key(&owner, &nonce);
        let mut record = self
            .intents
            .get(&k)
            .cloned()
            .unwrap_or_else(|| env::panic_str(&format!("no intent {nonce:?} registered by {owner}")));
        let min_out = record
            .quote
            .as_ref()
            .map(|q| q.min_amount_out.0)
            .unwrap_or_else(|| env::panic_str("no quote has been accepted for this intent"));
        require!(record.outcome.is_none(), "this intent's outcome is already recorded");
        require!(
            tx_hashes.len() <= MAX_TX_HASHES,
            format!("at most {MAX_TX_HASHES} transaction hashes")
        );
        for hash in &tx_hashes {
            require_text("tx_hashes[]", hash);
        }
        let floor_met = match status {
            Status::Settled => {
                let delivered = amount_out.unwrap_or_else(|| env::panic_str("a settled outcome states amount_out"));
                Some(delivered.0 >= min_out)
            }
            Status::Refunded | Status::Failed => None,
        };

        let outcome = Outcome {
            status,
            amount_out,
            refunded_amount,
            tx_hashes,
            floor_met,
            recorded_at_ms: env::block_timestamp_ms(),
        };
        emit(
            "outcome_recorded",
            json!({ "owner": owner, "nonce": nonce, "status": outcome.status, "floor_met": outcome.floor_met }),
        );
        record.outcome = Some(outcome.clone());
        self.intents.insert(k, record);
        self.intents.flush();
        settle_storage(initial);
        outcome
    }

    pub fn get_intent(&self, owner: AccountId, nonce: String) -> Option<IntentRecord> {
        self.intents.get(&key(&owner, &nonce)).cloned()
    }

    pub fn is_nonce_used(&self, owner: AccountId, nonce: String) -> bool {
        self.intents.contains_key(&key(&owner, &nonce))
    }

    /// Dry-run the guards for a registered intent without changing anything.
    pub fn check_quote(&self, owner: AccountId, nonce: String, quote: QuoteFacts) -> Vec<GuardResult> {
        let record = self
            .intents
            .get(&key(&owner, &nonce))
            .unwrap_or_else(|| env::panic_str(&format!("no intent {nonce:?} registered by {owner}")));
        evaluate(&record.policy, &quote, env::block_timestamp_ms())
    }
}

#[cfg(test)]
mod tests;
