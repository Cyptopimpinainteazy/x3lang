//! # x3-guard: the X3 policy of a NEAR Intents swap, enforced on chain
//!
//! `x3-lang/near_intents.py` lowers an X3 `intent` to a 1Click quote request and checks the
//! returned quote against the program's guards. This contract makes those decisions public and
//! permanent:
//!
//! 1. **The policy.** [`Contract::register_intent`] stores what the program allows (the
//!    program's SHA-256, assets, amount, `min_receive` floor, slippage bound, fee cap, receivers
//!    and deadline) under `(caller, nonce)`.
//! 2. **The quote.** [`Contract::accept_quote`] takes the exact payload 1Click signed and its
//!    signature, verifies the signature **on chain** against the pinned 1Click manager key, and
//!    only then reads the quote out of the verified bytes and runs every guard against the
//!    stored policy. A policy accepts **one** quote. Signed quote hashes are also
//!    consumed once contract-wide, to prevent cross-nonce authorization replay.
//! 3. **The outcome.** [`Contract::record_outcome`] records how the swap ended, once.
//! 4. **Agents.** An owner can let an AI agent's NEAR account swap on its behalf under a
//!    [`mandate::Mandate`]: per-payment and rolling-24-hour caps, allowlisted recipients and
//!    destination assets, a refund address the owner pins, slippage and fee ceilings, owner
//!    approvals above a threshold, an expiry, and instant revocation. See [`mandate`].
//!
//! ## Trust model
//!
//! - Every number a quote guard reads comes from the payload whose Ed25519 signature verified
//!   under the 1Click manager key. The caller cannot alter an amount, receiver, deposit address
//!   or deadline without the signature failing. The signed message is the one the official
//!   SDK verifies: `base58(sha256(payload))` as UTF-8 bytes
//!   (`@defuse-protocol/one-click-sdk-typescript` 0.1.26, `verifyQuoteSignature`).
//! - The manager key is set at initialization to the SDK's published key and has no setter.
//!   The contract has no owner and no admin. Rotating the key means deploying new code.
//! - The outcome is the policy owner's attestation of what `GET /v0/status` reported: 1Click
//!   does not sign status responses. The contract checks it is consistent (terminal, once,
//!   after a quote) and records whether a settlement met the signed guaranteed minimum.
//!
//! ## Storage
//!
//! Every write is paid by the caller: attach NEAR to cover the bytes added, and the excess is
//! refunded in the same call.

use near_sdk::borsh::io::{ErrorKind, Read, Write};
use near_sdk::borsh::{BorshDeserialize, BorshSerialize};
use near_sdk::json_types::U128;
use near_sdk::serde::Deserialize;
use near_sdk::serde_json::{self, json};
use near_sdk::store::LookupMap;
use near_sdk::{bs58, env, near, require, AccountId, BorshStorageKey, NearToken, PanicOnDefault, Promise};

/// NEP-297 event standard name.
pub const EVENT_STANDARD: &str = "x3guard";
pub const EVENT_VERSION: &str = "1.3.0";

/// Current persisted contract state layout.
pub const STATE_VERSION: u16 = 2;

/// The 1Click manager's Ed25519 key, from `@defuse-protocol/one-click-sdk-typescript` 0.1.26.
pub const ONE_CLICK_MANAGER_PUBLIC_KEY: &str = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc";
const ED25519_PREFIX: &str = "ed25519:";

const MAX_TEXT: usize = 256;
const MAX_NONCE: usize = 128;
const MAX_TX_HASHES: usize = 16;
const MAX_SIGNED_PAYLOAD: usize = 4096;
const BPS: u128 = 10_000;
/// Fixed-point scale for 1Click's decimal USD strings (it sends 12 fractional digits).
const USD_DECIMALS: usize = 18;
/// The same freshness window as `near_intents.py` (`MAX_QUOTE_AGE_S`, `MAX_CLOCK_SKEW_S`).
pub const MAX_QUOTE_AGE_MS: u64 = 300_000;
pub const MAX_CLOCK_SKEW_MS: u64 = 60_000;

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum StorageKey {
    Intents,
    Mandates,
    // Append-only storage prefix: preserve the existing Intents/Mandates keys.
    ConsumedQuoteHashes,
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

/// The payload 1Click signs, as `json-stable-stringify` writes it. Unknown fields are ignored;
/// a missing required field, or one of the wrong type, refuses the quote.
#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde", rename_all = "camelCase")]
struct SignedPayload {
    dry: bool,
    swap_type: String,
    deposit_type: String,
    refund_type: String,
    recipient_type: String,
    slippage_tolerance: u32,
    origin_asset: String,
    destination_asset: String,
    amount: String,
    refund_to: String,
    recipient: String,
    amount_in: String,
    amount_out: String,
    min_amount_out: String,
    amount_in_usd: String,
    amount_out_usd: String,
    #[serde(default)]
    deposit_address: Option<String>,
    #[serde(default)]
    deadline: Option<String>,
    #[serde(default)]
    time_estimate: Option<u64>,
    #[serde(default)]
    refund_fee: Option<String>,
    timestamp: String,
}

/// A quote as read from a verified 1Click payload: every field here was signed.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct QuoteFacts {
    /// The signed message: base58 SHA-256 of the payload (the SDK's `quoteHash`).
    pub quote_hash: String,
    pub signature: String,
    pub dry: bool,
    pub swap_type: String,
    pub deposit_type: String,
    pub refund_type: String,
    pub recipient_type: String,
    pub origin_asset: String,
    pub destination_asset: String,
    pub recipient: String,
    pub refund_to: String,
    pub slippage_tolerance_bps: u32,
    /// The request's `amount`.
    pub amount: U128,
    pub amount_in: U128,
    pub amount_out: U128,
    pub min_amount_out: U128,
    pub amount_in_usd: String,
    pub amount_out_usd: String,
    pub refund_fee: Option<U128>,
    /// Empty when the payload has none (dry quotes).
    pub deposit_address: String,
    /// 0 when the payload has none (dry quotes).
    pub quote_deadline_ms: u64,
    pub time_estimate_s: u64,
    pub issued_at_ms: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct AcceptedQuote {
    pub facts: QuoteFacts,
    /// The exact bytes 1Click signed, kept so anyone can re-verify the acceptance.
    pub signed_payload: String,
    pub accepted_at_ms: u64,
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
    /// For `Settled`: whether `amount_out` met the accepted quote's signed guaranteed minimum.
    pub floor_met: Option<bool>,
    pub recorded_at_ms: u64,
}

#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq)]
pub struct IntentRecord {
    pub owner: AccountId,
    pub nonce: String,
    pub policy: Policy,
    pub registered_at_ms: u64,
    pub quote: Option<AcceptedQuote>,
    pub outcome: Option<Outcome>,
    /// The agent that registered this intent under the owner's mandate; `None` for the owner's own.
    pub agent: Option<AccountId>,
    /// The mandate version it was registered under. A changed or re-created mandate voids it.
    pub mandate_version: Option<u32>,
    /// Approvers who signed off on it (only intents above the mandate's approval threshold).
    pub approvals: Vec<AccountId>,
}

impl BorshSerialize for IntentRecord {
    fn serialize<W: Write>(&self, writer: &mut W) -> near_sdk::borsh::io::Result<()> {
        self.owner.serialize(writer)?;
        self.nonce.serialize(writer)?;
        self.policy.serialize(writer)?;
        self.registered_at_ms.serialize(writer)?;
        self.quote.serialize(writer)?;
        self.outcome.serialize(writer)?;
        self.agent.serialize(writer)?;
        self.mandate_version.serialize(writer)?;
        self.approvals.serialize(writer)?;
        Ok(())
    }
}

/// Read a Borsh option tag without using `Option<T>`'s decoder, whose
/// truncation error does not distinguish a legacy record boundary from a
/// malformed new field.
fn read_optional_presence<R: Read>(reader: &mut R) -> near_sdk::borsh::io::Result<Option<bool>> {
    let mut tag = [0u8; 1];
    match reader.read_exact(&mut tag) {
        Ok(()) => match tag[0] {
            0 => Ok(Some(false)),
            1 => Ok(Some(true)),
            _ => Err(near_sdk::borsh::io::Error::new(
                ErrorKind::InvalidData,
                "invalid persisted option tag",
            )),
        },
        Err(error) if error.kind() == ErrorKind::UnexpectedEof => Ok(None),
        Err(error) => Err(error),
    }
}

impl BorshDeserialize for IntentRecord {
    fn deserialize_reader<R: Read>(reader: &mut R) -> near_sdk::borsh::io::Result<Self> {
        let owner = AccountId::deserialize_reader(reader)?;
        let nonce = String::deserialize_reader(reader)?;
        let policy = Policy::deserialize_reader(reader)?;
        let registered_at_ms = u64::deserialize_reader(reader)?;
        let quote = Option::<AcceptedQuote>::deserialize_reader(reader)?;
        let outcome = Option::<Outcome>::deserialize_reader(reader)?;

        // v1.1 records ended at `outcome`; v1.2 appended the agent fields.
        let agent = match read_optional_presence(reader)? {
            None => {
                return Ok(Self {
                    owner,
                    nonce,
                    policy,
                    registered_at_ms,
                    quote,
                    outcome,
                    agent: None,
                    mandate_version: None,
                    approvals: Vec::new(),
                });
            }
            Some(false) => None,
            Some(true) => Some(AccountId::deserialize_reader(reader)?),
        };
        let mandate_version = match read_optional_presence(reader)? {
            Some(false) => None,
            Some(true) => Some(u32::deserialize_reader(reader)?),
            None => {
                return Err(near_sdk::borsh::io::Error::new(
                    ErrorKind::InvalidData,
                    "persisted agent binding is missing its mandate version",
                ));
            }
        };
        let approvals = Vec::<AccountId>::deserialize_reader(reader)?;
        if agent.is_some() != mandate_version.is_some() || (agent.is_none() && !approvals.is_empty()) {
            return Err(near_sdk::borsh::io::Error::new(
                ErrorKind::InvalidData,
                "inconsistent persisted agent mandate fields",
            ));
        }
        Ok(Self {
            owner,
            nonce,
            policy,
            registered_at_ms,
            quote,
            outcome,
            agent,
            mandate_version,
            approvals,
        })
    }
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
    manager_public_key: [u8; 32],
    mandates: LookupMap<String, mandate::Mandate>,
    state_version: u16,
}

#[near(serializers = [borsh])]
struct ContractV11 {
    intents: LookupMap<String, IntentRecord>,
    manager_public_key: [u8; 32],
}

#[near(serializers = [borsh])]
struct ContractV12 {
    intents: LookupMap<String, IntentRecord>,
    manager_public_key: [u8; 32],
    mandates: LookupMap<String, mandate::Mandate>,
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

/// `ed25519:<base58>` -> exactly `N` bytes.
pub fn decode_ed25519<const N: usize>(value: &str) -> Result<[u8; N], String> {
    let body = value
        .strip_prefix(ED25519_PREFIX)
        .ok_or_else(|| format!("expected an {ED25519_PREFIX}<base58> value"))?;
    let raw = bs58::decode(body)
        .into_vec()
        .map_err(|e| format!("invalid base58: {e}"))?;
    raw.as_slice()
        .try_into()
        .map_err(|_| format!("expected {N} bytes, decoded {}", raw.len()))
}

/// The SDK's `quoteHash`: base58 of SHA-256 over the signed payload.
pub fn quote_hash(signed_payload: &str) -> String {
    bs58::encode(env::sha256(signed_payload.as_bytes())).into_string()
}

/// Parse a non-negative decimal string into a fixed-point integer with [`USD_DECIMALS`].
/// More fractional digits than that are refused rather than rounded.
pub fn parse_usd(field: &str, text: &str) -> Result<u128, String> {
    let (whole, frac) = text.split_once('.').unwrap_or((text, ""));
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !(digits(whole) && (frac.is_empty() || digits(frac))) {
        return Err(format!("{field} is not a decimal: {text:?}"));
    }
    if frac.len() > USD_DECIMALS {
        return Err(format!("{field} has more than {USD_DECIMALS} fractional digits"));
    }
    let scale = 10u128.pow(USD_DECIMALS as u32);
    let whole: u128 = whole.parse().map_err(|_| format!("{field} is too large"))?;
    let frac: u128 = format!("{frac:0<USD_DECIMALS$}").parse().unwrap_or(0);
    whole
        .checked_mul(scale)
        .and_then(|w| w.checked_add(frac))
        .ok_or_else(|| format!("{field} is too large"))
}

fn parse_units(field: &str, text: &str) -> Result<u128, String> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("{field} is not an integer amount: {text:?}"));
    }
    text.parse().map_err(|_| format!("{field} is too large"))
}

/// `YYYY-MM-DDTHH:MM:SS[.fff…]Z` (UTC, as 1Click writes it) -> Unix milliseconds.
pub fn parse_utc_ms(text: &str) -> Result<u64, String> {
    let bad = || format!("unreadable UTC timestamp {text:?}");
    let body = text.strip_suffix('Z').ok_or_else(bad)?;
    let (main, frac) = body.split_once('.').unwrap_or((body, ""));
    let b = main.as_bytes();
    if b.len() != 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return Err(bad());
    }
    if !frac.bytes().all(|c| c.is_ascii_digit()) {
        return Err(bad());
    }
    let num = |r: std::ops::Range<usize>| -> Result<u64, String> { main[r].parse::<u64>().map_err(|_| bad()) };
    let (y, mo, d, h, mi, s) = (
        num(0..4)?,
        num(5..7)?,
        num(8..10)?,
        num(11..13)?,
        num(14..16)?,
        num(17..19)?,
    );
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    if !(1970..=9999).contains(&y)
        || !(1..=12).contains(&mo)
        || d == 0
        || d > month_days[(mo - 1) as usize]
        || h > 23
        || mi > 59
        || s > 59
    {
        return Err(bad());
    }
    // Days from the civil date (H. Hinnant), for years >= 1970.
    let (yy, mm) = if mo <= 2 { (y - 1, mo + 9) } else { (y, mo - 3) };
    let era = yy / 400;
    let yoe = yy - era * 400;
    let doy = (153 * mm + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let ms: u64 = format!("{:0<3}", &frac[..frac.len().min(3)]).parse().unwrap_or(0);
    Ok(((days * 86_400 + h * 3_600 + mi * 60 + s) * 1_000) + ms)
}

/// Verify a 1Click signature over `signed_payload` and read the quote out of it.
/// Nothing is parsed until the signature verifies.
pub fn verify_signed_quote(
    manager_public_key: &[u8; 32],
    signed_payload: &str,
    signature: &str,
) -> Result<QuoteFacts, String> {
    if signed_payload.is_empty() || signed_payload.len() > MAX_SIGNED_PAYLOAD {
        return Err(format!("signed payload must be 1..={MAX_SIGNED_PAYLOAD} bytes"));
    }
    let signature_bytes = decode_ed25519::<64>(signature)?;
    let hash = quote_hash(signed_payload);
    if !env::ed25519_verify(&signature_bytes, hash.as_bytes(), manager_public_key) {
        return Err("signature does not verify under the 1Click manager key".into());
    }
    let p: SignedPayload =
        serde_json::from_str(signed_payload).map_err(|e| format!("signed payload is not a 1Click quote: {e}"))?;
    Ok(QuoteFacts {
        quote_hash: hash,
        signature: signature.to_string(),
        dry: p.dry,
        swap_type: p.swap_type,
        deposit_type: p.deposit_type,
        refund_type: p.refund_type,
        recipient_type: p.recipient_type,
        origin_asset: p.origin_asset,
        destination_asset: p.destination_asset,
        recipient: p.recipient,
        refund_to: p.refund_to,
        slippage_tolerance_bps: p.slippage_tolerance,
        amount: U128(parse_units("amount", &p.amount)?),
        amount_in: U128(parse_units("amountIn", &p.amount_in)?),
        amount_out: U128(parse_units("amountOut", &p.amount_out)?),
        min_amount_out: U128(parse_units("minAmountOut", &p.min_amount_out)?),
        amount_in_usd: p.amount_in_usd,
        amount_out_usd: p.amount_out_usd,
        refund_fee: p
            .refund_fee
            .as_deref()
            .map(|f| parse_units("refundFee", f))
            .transpose()?
            .map(U128),
        deposit_address: p.deposit_address.unwrap_or_default(),
        quote_deadline_ms: p.deadline.as_deref().map(parse_utc_ms).transpose()?.unwrap_or(0),
        // The SDK omits a zero `timeEstimate` from the payload.
        time_estimate_s: p.time_estimate.unwrap_or(0),
        issued_at_ms: parse_utc_ms(&p.timestamp)?,
    })
}

/// `a * BPS <= cap * b`, without overflow panics on honest inputs.
fn within_bps(numerator: u128, denominator: u128, cap_bps: u128) -> bool {
    match (numerator.checked_mul(BPS), denominator.checked_mul(cap_bps)) {
        (Some(lhs), Some(rhs)) => lhs <= rhs,
        // Values this large are not quotes; refuse rather than guess.
        _ => false,
    }
}

/// Run every guard of `policy` over a verified quote at `now_ms`. Pure, so it is unit-tested
/// directly and also exposed through the view [`Contract::check_quote`].
pub fn evaluate(policy: &Policy, quote: &QuoteFacts, now_ms: u64) -> Vec<GuardResult> {
    let mut out = Vec::new();
    let mut record = |guard: &str, ok: bool, detail: String| {
        out.push(GuardResult {
            guard: guard.into(),
            ok,
            detail,
        })
    };

    record(
        "quote_kind",
        !quote.dry
            && quote.swap_type == "EXACT_INPUT"
            && quote.deposit_type == "ORIGIN_CHAIN"
            && quote.refund_type == "ORIGIN_CHAIN"
            && quote.recipient_type == "DESTINATION_CHAIN",
        format!(
            "dry {}, {} / {} / {} / {}: a deposit needs a live EXACT_INPUT ORIGIN_CHAIN quote",
            quote.dry, quote.swap_type, quote.deposit_type, quote.refund_type, quote.recipient_type
        ),
    );

    let age_ok = now_ms.saturating_sub(quote.issued_at_ms) <= MAX_QUOTE_AGE_MS
        && quote.issued_at_ms <= now_ms.saturating_add(MAX_CLOCK_SKEW_MS);
    record(
        "quote_freshness",
        age_ok,
        format!("issued {}, now {now_ms}", quote.issued_at_ms),
    );

    let mismatched: Vec<&str> = [
        ("originAsset", quote.origin_asset == policy.origin_asset),
        ("destinationAsset", quote.destination_asset == policy.destination_asset),
        ("recipient", quote.recipient == policy.recipient),
        ("refundTo", quote.refund_to == policy.refund_to),
        ("slippageTolerance", quote.slippage_tolerance_bps == policy.slippage_bps),
        ("amount", quote.amount == policy.amount),
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
        match (
            parse_usd("amountInUsd", &quote.amount_in_usd),
            parse_usd("amountOutUsd", &quote.amount_out_usd),
        ) {
            (Ok(usd_in), Ok(usd_out)) if usd_in > 0 => {
                let cost = usd_in.saturating_sub(usd_out);
                record(
                    "fees",
                    within_bps(cost, usd_in, cap as u128),
                    format!("cost {cost} of {usd_in} (1e-18 USD), cap {cap} bps"),
                );
            }
            (Ok(_), Ok(_)) => record(
                "fees",
                false,
                "no USD price for the input; cost cannot be measured".into(),
            ),
            (Err(e), _) | (_, Err(e)) => record("fees", false, e),
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
    /// Initialize with the 1Click manager key the official SDK pins. There is no setter.
    #[init]
    pub fn new() -> Self {
        Self {
            intents: LookupMap::new(StorageKey::Intents),
            manager_public_key: decode_ed25519::<32>(ONE_CLICK_MANAGER_PUBLIC_KEY)
                .unwrap_or_else(|e| env::panic_str(&e)),
            mandates: LookupMap::new(StorageKey::Mandates),
            state_version: STATE_VERSION,
        }
    }

    /// Upgrade a Guard deployed with the pre-mandate (v1.1) or unversioned
    /// mandate (v1.2) state layout. The existing map prefixes and manager key
    /// are preserved; v1.1 records decode their missing agent fields as `None`.
    #[init(ignore_state)]
    pub fn migrate() -> Self {
        let bytes = env::storage_read(b"STATE")
            .unwrap_or_else(|| env::panic_str("state has no recognized x3-guard migration layout"));
        if let Ok(current) = Contract::try_from_slice(&bytes) {
            require!(
                current.state_version != STATE_VERSION,
                format!("state already uses schema version {STATE_VERSION}")
            );
            require!(
                current.state_version < STATE_VERSION,
                format!(
                    "cannot downgrade state version {} to {STATE_VERSION}",
                    current.state_version
                )
            );
            let migrated = Self {
                intents: current.intents,
                manager_public_key: current.manager_public_key,
                mandates: current.mandates,
                state_version: STATE_VERSION,
            };
            emit(
                "state_migrated",
                json!({ "from_version": current.state_version, "to_version": STATE_VERSION }),
            );
            return migrated;
        }

        let (intents, manager_public_key, mandates, from_version) =
            if let Ok(legacy) = ContractV12::try_from_slice(&bytes) {
                (legacy.intents, legacy.manager_public_key, legacy.mandates, 1u16)
            } else if let Ok(legacy) = ContractV11::try_from_slice(&bytes) {
                (
                    legacy.intents,
                    legacy.manager_public_key,
                    LookupMap::new(StorageKey::Mandates),
                    0u16,
                )
            } else {
                env::panic_str("state has no recognized x3-guard migration layout")
            };
        let migrated = Self {
            intents,
            manager_public_key,
            mandates,
            state_version: STATE_VERSION,
        };
        emit(
            "state_migrated",
            json!({ "from_version": from_version, "to_version": STATE_VERSION }),
        );
        migrated
    }

    pub fn state_version(&self) -> u16 {
        self.state_version
    }

    /// Store an X3 program's policy under `(caller, nonce)`. A nonce is registered once.
    #[payable]
    pub fn register_intent(&mut self, nonce: String, policy: Policy) -> IntentRecord {
        let initial = env::storage_usage();
        let owner = env::predecessor_account_id();
        let record = self.insert_intent(owner, nonce, policy, None);
        settle_storage(initial);
        record
    }

    /// Verify a 1Click quote's signature on chain, read the quote from the signed bytes, run
    /// every guard against the stored policy and, if all pass, record it. A policy accepts one
    /// quote. A forged or failing quote changes nothing.
    #[payable]
    pub fn accept_quote(&mut self, nonce: String, signed_payload: String, signature: String) -> Vec<GuardResult> {
        let initial = env::storage_usage();
        let owner = env::predecessor_account_id();
        let record = self.load_intent(&owner, &nonce);
        if let Some(agent) = &record.agent {
            env::panic_str(&format!(
                "intent {nonce:?} was registered by agent {agent} under a mandate; it is accepted with agent_accept_quote"
            ));
        }
        let results = self.accept_verified(record, signed_payload, signature, None);
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
        let record = self.load_intent(&owner, &nonce);
        let outcome = self.record_outcome_for(record, status, amount_out, refunded_amount, tx_hashes);
        settle_storage(initial);
        outcome
    }

    pub fn get_intent(&self, owner: AccountId, nonce: String) -> Option<IntentRecord> {
        self.intents.get(&key(&owner, &nonce)).cloned()
    }

    pub fn is_nonce_used(&self, owner: AccountId, nonce: String) -> bool {
        self.intents.contains_key(&key(&owner, &nonce))
    }

    /// The 1Click manager key quotes are verified against, as `ed25519:<base58>`.
    pub fn get_manager_public_key(&self) -> String {
        format!(
            "{ED25519_PREFIX}{}",
            bs58::encode(self.manager_public_key).into_string()
        )
    }

    /// Read the on-chain owner/nonce that first consumed a signed quote hash.
    /// Only records accepted *after* this replay ledger was introduced exist
    /// here; pre-upgrade receipts require a separately authenticated backfill.
    pub fn get_quote_consumer(&self, quote_hash: String) -> Option<String> {
        let consumed: LookupMap<String, String> = LookupMap::new(StorageKey::ConsumedQuoteHashes);
        consumed.get(&quote_hash).cloned()
    }

    /// Dry-run the signature check and the guards for a registered intent. Changes nothing.
    pub fn check_quote(
        &self,
        owner: AccountId,
        nonce: String,
        signed_payload: String,
        signature: String,
    ) -> Vec<GuardResult> {
        let record = self
            .intents
            .get(&key(&owner, &nonce))
            .unwrap_or_else(|| env::panic_str(&format!("no intent {nonce:?} registered by {owner}")));
        // The legacy quote view also serves agent-registered intents. Recheck the
        // current mandate and approval state instead of returning quote-only green.
        // This is non-authorizing and must never be used as wallet signing proof.
        if record.agent.is_some() {
            return self.check_agent_quote(owner, nonce, signed_payload, signature);
        }
        match verify_signed_quote(&self.manager_public_key, &signed_payload, &signature) {
            Err(reason) => vec![GuardResult {
                guard: "quote_signature".into(),
                ok: false,
                detail: reason,
            }],
            Ok(facts) => {
                let mut results = vec![GuardResult {
                    guard: "quote_signature".into(),
                    ok: true,
                    detail: facts.quote_hash.clone(),
                }];
                results.extend(evaluate(&record.policy, &facts, env::block_timestamp_ms()));
                results
            }
        }
    }
}

/// Internals shared by the owner's methods and the agent methods in [`mandate`].
impl Contract {
    fn load_intent(&self, owner: &AccountId, nonce: &str) -> IntentRecord {
        self.intents
            .get(&key(owner, nonce))
            .cloned()
            .unwrap_or_else(|| env::panic_str(&format!("no intent {nonce:?} registered by {owner}")))
    }

    fn store_intent(&mut self, record: IntentRecord) {
        self.intents.insert(key(&record.owner, &record.nonce), record);
        self.intents.flush();
    }

    /// Validate and store a new policy under `(owner, nonce)`. `agent` is `(agent, mandate
    /// version)` when an agent registers it under the owner's mandate.
    fn insert_intent(
        &mut self,
        owner: AccountId,
        nonce: String,
        policy: Policy,
        agent: Option<(AccountId, u32)>,
    ) -> IntentRecord {
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

        let (agent, mandate_version) = match agent {
            Some((a, v)) => (Some(a), Some(v)),
            None => (None, None),
        };
        let record = IntentRecord {
            owner: owner.clone(),
            nonce: nonce.clone(),
            policy,
            registered_at_ms: env::block_timestamp_ms(),
            quote: None,
            outcome: None,
            agent,
            mandate_version,
            approvals: Vec::new(),
        };
        self.intents.insert(k, record.clone());
        self.intents.flush();
        emit(
            "intent_registered",
            json!({
                "owner": owner,
                "nonce": nonce,
                "program_sha256": record.policy.program_sha256,
                "agent": record.agent,
            }),
        );
        record
    }

    /// Verify the quote, run every guard, and record it if all pass; panics (changing nothing)
    /// otherwise. `spent` adds the agent's rolling spend to the event.
    fn accept_verified(
        &mut self,
        mut record: IntentRecord,
        signed_payload: String,
        signature: String,
        spent: Option<u128>,
    ) -> Vec<GuardResult> {
        if let Some(accepted) = &record.quote {
            env::panic_str(&format!(
                "intent {:?} already accepted quote {}",
                record.nonce, accepted.facts.quote_hash
            ));
        }
        let facts =
            verify_signed_quote(&self.manager_public_key, &signed_payload, &signature).unwrap_or_else(|reason| {
                env::panic_str(&format!(
                    "X3_NEAR_INVALID_SIGNATURE: Cryptographic quote verification failed; deposit prohibited ({reason})"
                ))
            });
        // Quote signatures authenticate terms but do not bind an owner's
        // arbitrary intent nonce. A separate, contract-wide consumed-hash
        // namespace is therefore required to prevent duplicate authorizations.
        // This is intentionally NOT a Contract struct field, preserving the
        // existing Borsh state layout and original map prefixes.
        let mut consumed: LookupMap<String, String> = LookupMap::new(StorageKey::ConsumedQuoteHashes);
        require!(
            consumed.get(&facts.quote_hash).is_none(),
            format!(
                "X3_NEAR_QUOTE_REPLAY: signed quote {} already accepted by another intent",
                facts.quote_hash
            )
        );
        let now = env::block_timestamp_ms();
        let results = evaluate(&record.policy, &facts, now);
        let failed: Vec<&str> = results.iter().filter(|r| !r.ok).map(|r| r.guard.as_str()).collect();
        require!(
            failed.is_empty(),
            format!("quote rejected by {}; do not deposit", failed.join(", "))
        );
        let consumed_hash = facts.quote_hash.clone();
        let consuming_intent = key(&record.owner, &record.nonce);
        emit(
            "quote_accepted",
            json!({
                "owner": record.owner,
                "nonce": record.nonce,
                "quote_hash": facts.quote_hash,
                "deposit_address": facts.deposit_address,
                "min_amount_out": facts.min_amount_out,
                "agent": record.agent,
                "agent_spent_24h": spent.map(U128),
            }),
        );
        record.quote = Some(AcceptedQuote {
            facts,
            signed_payload,
            accepted_at_ms: now,
        });
        self.store_intent(record);
        // No consumption occurs on a failed signature or rejected policy.
        // The map write and the intent write are in the same NEAR transaction.
        consumed.insert(consumed_hash, consuming_intent);
        consumed.flush();
        results
    }

    fn record_outcome_for(
        &mut self,
        mut record: IntentRecord,
        status: Status,
        amount_out: Option<U128>,
        refunded_amount: Option<U128>,
        tx_hashes: Vec<String>,
    ) -> Outcome {
        let min_out = record
            .quote
            .as_ref()
            .map(|q| q.facts.min_amount_out.0)
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
            json!({
                "owner": record.owner,
                "nonce": record.nonce,
                "status": outcome.status,
                "floor_met": outcome.floor_met,
                "agent": record.agent,
            }),
        );
        record.outcome = Some(outcome.clone());
        self.store_intent(record);
        outcome
    }
}

pub mod mandate;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod migration_tests;
