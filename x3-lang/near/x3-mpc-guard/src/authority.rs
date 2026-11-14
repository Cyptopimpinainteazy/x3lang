//! # The signing intent and the authority check that must pass before the MPC signer is called
//!
//! An owner gives an agent a [`SigningMandate`]: one EVM chain, one owner-controlled derived
//! account (`key_index`), one asset, an allowlist of recipients, per-payment and rolling 24-hour
//! caps, fee and gas ceilings, approvals above a threshold, an expiry and a pinned program hash.
//! The agent can only *propose* a [`SigningRequest`]: structured fields, never bytes, a hash, a
//! derivation path or a key version. The contract builds the canonical unsigned transaction
//! from those fields and the mandate ([`build_intent`]), and records the result as a versioned
//! [`X3SigningIntent`]. [`check_signing_authority`] is the pure term-by-term check; the contract
//! runs it when the intent is proposed and again, bindingly, before it creates the sign promise.
//!
//! The mandate types follow `x3-lang/near/x3-guard/src/mandate.rs` (copied, not imported: the
//! deployed Guard is frozen and this crate must not change it). Differences: the asset is an EVM
//! asset, recipients are EVM addresses, the fee cap is in gas units and wei instead of basis
//! points, and `program_sha256` is enforced, not only recorded.

use near_sdk::json_types::U128;
use near_sdk::{near, AccountId};

use crate::evm::{erc20_transfer_data, Eip1559Tx, MIN_GAS_LIMIT};
use crate::hex;

/// Version of the [`X3SigningIntent`] layout. A new field or meaning needs a new version.
pub const SIGNING_INTENT_VERSION: u16 = 1;
/// CAIP-2 namespace of every chain this version can sign for.
pub const EVM_NAMESPACE: &str = "eip155";
pub const WINDOW_MS: u64 = 86_400_000;
pub const MAX_PAYMENTS_PER_WINDOW: usize = 64;
pub const MAX_LIST: usize = 16;
pub const MAX_APPROVERS: usize = 8;
pub const MAX_NONCE_LEN: usize = 128;
/// Gas a mandate may allow in one transaction; above any current L1 block limit.
pub const MAX_GAS_LIMIT: u64 = 30_000_000;
/// ERC-20 `transfer` cannot run in the 21 000 intrinsic gas.
pub const MIN_ERC20_GAS_LIMIT: u64 = 30_000;

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct GuardResult {
    pub guard: String,
    pub ok: bool,
    pub detail: String,
}

/// What the transaction moves. The agent cannot choose it; it must equal the mandate's.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum EvmAsset {
    /// The chain's native coin, sent as the transaction value with empty calldata.
    Native,
    /// An ERC-20 token, sent as `transfer(recipient, amount)` to `token` with zero value.
    Erc20 { token: String },
}

/// What an owner allows. Amounts are base units of `asset`; fees are wei.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct SigningMandateTerms {
    /// SHA-256 of the `.x3` payment program the agent must run, lowercase hex. Enforced.
    pub program_sha256: String,
    /// EIP-155 chain ID of the only chain the agent may sign for.
    pub chain_id: u64,
    /// The owner's derived foreign account the agent may spend from.
    pub key_index: u32,
    pub asset: EvmAsset,
    /// Allowlisted recipients, `0x` + 40 lowercase hex.
    pub recipients: Vec<String>,
    pub per_payment_cap: U128,
    /// Ceiling over any rolling 24 hours.
    pub daily_cap: U128,
    pub max_fee_per_gas: U128,
    pub max_priority_fee_per_gas: U128,
    pub max_gas_limit: u64,
    /// Ceiling on `gas_limit * max_fee_per_gas`, the most one transaction can pay in fees.
    pub max_total_fee: U128,
    /// Payments above this amount need `approval_threshold` distinct `approvers`.
    pub approval_above: Option<U128>,
    pub approvers: Vec<AccountId>,
    pub approval_threshold: u8,
    pub expires_at_ms: u64,
    /// Longest lifetime of one intent, so an approval cannot be used long after it was given.
    pub max_intent_ttl_ms: u64,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct Spend {
    pub at_ms: u64,
    pub amount: U128,
    pub intent_nonce: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct SigningMandate {
    pub owner: AccountId,
    pub agent: AccountId,
    pub terms: SigningMandateTerms,
    /// Starts at 1, increases with every replacement; an intent is valid only under its version.
    pub version: u32,
    pub revoked: bool,
    pub created_at_ms: u64,
    /// Signature requests made in the current window (older ones are pruned on write).
    pub spends: Vec<Spend>,
    /// yoctoNEAR the owner prepaid for the agent's storage and the signer's 1 yocto deposits.
    pub storage_credit: U128,
}

impl SigningMandate {
    pub fn spent_in_window(&self, now_ms: u64) -> (u128, usize) {
        let since = now_ms.saturating_sub(WINDOW_MS);
        self.spends
            .iter()
            .filter(|s| s.at_ms > since)
            .fold((0u128, 0usize), |(sum, n), s| (sum.saturating_add(s.amount.0), n + 1))
    }

    pub fn approvals_needed(&self, amount: u128) -> u8 {
        match self.terms.approval_above {
            Some(limit) if amount > limit.0 => self.terms.approval_threshold,
            _ => 0,
        }
    }
}

/// Everything an agent may say. No bytes, hash, path, key version or domain: unknown JSON
/// fields are refused, so a payload smuggled in as an extra field fails to parse.
#[near(serializers = [borsh, json])]
#[serde(deny_unknown_fields)]
#[derive(Clone, Debug, PartialEq)]
pub struct SigningRequest {
    /// One-use X3 nonce, unique per owner.
    pub intent_nonce: String,
    pub program_sha256: String,
    pub chain_id: u64,
    pub key_index: u32,
    pub asset: EvmAsset,
    pub recipient: String,
    pub amount: U128,
    /// The foreign account's EVM nonce; must equal the next one this contract has not signed.
    pub evm_nonce: u64,
    pub max_priority_fee_per_gas: U128,
    pub max_fee_per_gas: U128,
    pub gas_limit: u64,
    pub expires_at_ms: u64,
}

/// What happens if the transaction does not complete. Versioned with the intent.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum RefundStrategy {
    /// A direct EVM transfer is atomic: it either executes in full or moves nothing (the fee is
    /// still spent if it is mined and reverts). No refund leg exists; the spend stays counted.
    AtomicTransferNoRefund,
}

/// The exact thing an owner authorizes and the MPC network signs. Borsh-serialized in field
/// order, it is the canonical preimage of [`intent_hash`]. Every byte string is lowercase hex.
#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct X3SigningIntent {
    pub version: u16,
    pub program_sha256: String,
    pub owner: AccountId,
    pub agent: AccountId,
    pub mandate_version: u32,
    pub intent_nonce: String,
    /// CAIP-2 namespace (`eip155`) and chain ID.
    pub chain_namespace: String,
    pub chain_id: u64,
    /// The MPC signer contract and key domain, fixed at this contract's initialization.
    pub mpc_signer: AccountId,
    pub mpc_domain_id: u64,
    /// The account that calls `sign`, which is this contract: the derivation predecessor.
    pub derivation_predecessor: AccountId,
    /// Derived by the contract from the owner and `key_index`, never supplied by the agent.
    pub derivation_path: String,
    pub key_index: u32,
    /// The derived foreign sending address (`0x…`), verified against the MPC contract.
    pub from: String,
    pub recipient: String,
    pub asset: EvmAsset,
    pub amount: U128,
    /// Fields of the transaction as built: `to`, `value`, `data`.
    pub tx_to: String,
    pub tx_value: U128,
    pub tx_data: String,
    pub evm_nonce: u64,
    pub max_priority_fee_per_gas: U128,
    pub max_fee_per_gas: U128,
    pub gas_limit: u64,
    /// `gas_limit * max_fee_per_gas`: the fee ceiling of this exact transaction.
    pub max_total_fee: U128,
    /// The canonical unsigned EIP-1559 transaction (`0x02 || rlp(...)`), the signing preimage.
    pub unsigned_tx: String,
    /// `keccak256(unsigned_tx)`, the only payload sent to the signer.
    pub signing_hash: String,
    pub expires_at_ms: u64,
    pub refund: RefundStrategy,
}

/// Where the intent will be signed: fixed by contract state, not by the agent.
pub struct SigningContext<'a> {
    pub mpc_signer: &'a AccountId,
    pub mpc_domain_id: u64,
    pub predecessor: &'a AccountId,
    pub derivation_path: &'a str,
    pub from: &'a str,
}

fn record(out: &mut Vec<GuardResult>, guard: &str, ok: bool, detail: String) {
    out.push(GuardResult {
        guard: guard.into(),
        ok,
        detail,
    });
}

fn has_duplicates<T: PartialEq>(items: &[T]) -> bool {
    items.iter().enumerate().any(|(i, a)| items[..i].contains(a))
}

fn asset_ok(asset: &EvmAsset) -> bool {
    match asset {
        EvmAsset::Native => true,
        EvmAsset::Erc20 { token } => hex::parse_evm_address(token).is_some_and(|a| a != [0u8; 20]),
    }
}

/// Terms that would not mean what they say. Returns the first reason.
pub fn validate_terms(
    owner: &AccountId,
    agent: &AccountId,
    t: &SigningMandateTerms,
    now_ms: u64,
) -> Result<(), String> {
    let fail = |msg: &str| Err(msg.to_string());
    if agent == owner {
        return fail("the agent must be a different account from the owner");
    }
    if !hex::is_hex64(&t.program_sha256) {
        return fail("program_sha256 must be 64 lowercase hex characters");
    }
    if t.chain_id == 0 {
        return fail("chain_id must be positive");
    }
    if !asset_ok(&t.asset) {
        return fail("an ERC-20 token must be a non-zero 0x-lowercase address");
    }
    if t.recipients.is_empty() || t.recipients.len() > MAX_LIST {
        return Err(format!("recipients must list 1..={MAX_LIST} addresses"));
    }
    if has_duplicates(&t.recipients) {
        return fail("recipients has duplicates");
    }
    if t.recipients.iter().any(|r| hex::parse_evm_address(r).is_none()) {
        return fail("every recipient must be 0x + 40 lowercase hex characters");
    }
    if t.per_payment_cap.0 == 0 || t.per_payment_cap.0 > t.daily_cap.0 {
        return fail("per_payment_cap must be positive and not exceed daily_cap");
    }
    if t.max_fee_per_gas.0 == 0 || t.max_priority_fee_per_gas.0 > t.max_fee_per_gas.0 {
        return fail("max_fee_per_gas must be positive and at least max_priority_fee_per_gas");
    }
    if !(MIN_GAS_LIMIT..=MAX_GAS_LIMIT).contains(&t.max_gas_limit) {
        return Err(format!("max_gas_limit must be {MIN_GAS_LIMIT}..={MAX_GAS_LIMIT}"));
    }
    if t.max_total_fee.0 == 0 {
        return fail("max_total_fee must be positive");
    }
    match t.approval_above {
        Some(limit) => {
            if limit.0 >= t.per_payment_cap.0 {
                return fail("approval_above must be below per_payment_cap");
            }
            if t.approvers.is_empty() || t.approvers.len() > MAX_APPROVERS {
                return Err(format!("approvers must list 1..={MAX_APPROVERS} accounts"));
            }
            if has_duplicates(&t.approvers) {
                return fail("approvers has duplicates");
            }
            if t.approvers.contains(agent) {
                return fail("the agent cannot approve its own payments");
            }
            if t.approval_threshold == 0 || t.approval_threshold as usize > t.approvers.len() {
                return fail("approval_threshold must be 1..=the number of approvers");
            }
        }
        None => {
            if !t.approvers.is_empty() || t.approval_threshold != 0 {
                return fail("approvers and approval_threshold need approval_above");
            }
        }
    }
    if t.expires_at_ms <= now_ms {
        return fail("the mandate's expiry has already passed");
    }
    if t.max_intent_ttl_ms == 0 || t.max_intent_ttl_ms > WINDOW_MS {
        return fail("max_intent_ttl_ms must be 1..=86400000");
    }
    Ok(())
}

pub fn valid_nonce(nonce: &str) -> bool {
    !nonce.is_empty()
        && nonce.len() <= MAX_NONCE_LEN
        && nonce.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

/// Every mandate term against a request at `now_ms`. Pure. `spend_already_counted` is true only
/// when re-requesting a signature for the identical intent after a failed attempt, whose
/// amount is already in the rolling window.
pub fn check_signing_authority(
    m: &SigningMandate,
    req: &SigningRequest,
    now_ms: u64,
    spend_already_counted: bool,
) -> Vec<GuardResult> {
    let t = &m.terms;
    let mut out = Vec::new();
    record(
        &mut out,
        "mandate_active",
        !m.revoked && now_ms < t.expires_at_ms,
        format!(
            "version {}, revoked {}, expires {}, now {now_ms}",
            m.version, m.revoked, t.expires_at_ms
        ),
    );
    record(
        &mut out,
        "intent_nonce",
        valid_nonce(&req.intent_nonce),
        format!("{:?}", req.intent_nonce),
    );
    record(
        &mut out,
        "program_sha256",
        req.program_sha256 == t.program_sha256,
        format!("runs {}, mandate pins {}", req.program_sha256, t.program_sha256),
    );
    record(
        &mut out,
        "chain_id",
        req.chain_id == t.chain_id,
        format!(
            "{EVM_NAMESPACE}:{}, mandate allows {EVM_NAMESPACE}:{}",
            req.chain_id, t.chain_id
        ),
    );
    record(
        &mut out,
        "key_index",
        req.key_index == t.key_index,
        format!("account {}, mandate allows {}", req.key_index, t.key_index),
    );
    record(
        &mut out,
        "asset",
        req.asset == t.asset,
        format!("{:?}, mandate allows {:?}", req.asset, t.asset),
    );
    record(
        &mut out,
        "recipient",
        hex::parse_evm_address(&req.recipient).is_some() && t.recipients.contains(&req.recipient),
        format!("{}, allowed {:?}", req.recipient, t.recipients),
    );
    let amount = req.amount.0;
    record(
        &mut out,
        "per_payment",
        amount > 0 && amount <= t.per_payment_cap.0,
        format!("{amount} of at most {}", t.per_payment_cap.0),
    );
    let (spent, count) = m.spent_in_window(now_ms);
    let (spent_after, count_after) = if spend_already_counted {
        (Some(spent), count)
    } else {
        (spent.checked_add(amount), count + 1)
    };
    record(
        &mut out,
        "rolling_24h",
        spent_after.is_some_and(|total| total <= t.daily_cap.0) && count_after <= MAX_PAYMENTS_PER_WINDOW,
        format!(
            "{spent} in {count} requests over 24h, adding {}, cap {}",
            if spend_already_counted { 0 } else { amount },
            t.daily_cap.0
        ),
    );
    record(
        &mut out,
        "max_fee_per_gas",
        req.max_fee_per_gas.0 > 0 && req.max_fee_per_gas.0 <= t.max_fee_per_gas.0,
        format!("{} of at most {}", req.max_fee_per_gas.0, t.max_fee_per_gas.0),
    );
    record(
        &mut out,
        "max_priority_fee_per_gas",
        req.max_priority_fee_per_gas.0 <= t.max_priority_fee_per_gas.0
            && req.max_priority_fee_per_gas.0 <= req.max_fee_per_gas.0,
        format!(
            "{} of at most {} and at most max_fee_per_gas {}",
            req.max_priority_fee_per_gas.0, t.max_priority_fee_per_gas.0, req.max_fee_per_gas.0
        ),
    );
    let min_gas = match t.asset {
        EvmAsset::Native => MIN_GAS_LIMIT,
        EvmAsset::Erc20 { .. } => MIN_ERC20_GAS_LIMIT,
    };
    record(
        &mut out,
        "gas_limit",
        req.gas_limit >= min_gas && req.gas_limit <= t.max_gas_limit,
        format!("{} in {min_gas}..={}", req.gas_limit, t.max_gas_limit),
    );
    let total_fee = (req.gas_limit as u128).checked_mul(req.max_fee_per_gas.0);
    record(
        &mut out,
        "max_total_fee",
        total_fee.is_some_and(|fee| fee <= t.max_total_fee.0),
        format!("{total_fee:?} wei of at most {}", t.max_total_fee.0),
    );
    record(
        &mut out,
        "intent_expiry",
        now_ms < req.expires_at_ms
            && req.expires_at_ms <= t.expires_at_ms
            && req.expires_at_ms - now_ms <= t.max_intent_ttl_ms,
        format!(
            "expires {}, now {now_ms}, mandate expires {}, ttl at most {}",
            req.expires_at_ms, t.expires_at_ms, t.max_intent_ttl_ms
        ),
    );
    out
}

/// Approvals that count: distinct accounts still listed as approvers on the current mandate.
pub fn count_approvals(m: &SigningMandate, approvals: &[AccountId]) -> usize {
    approvals
        .iter()
        .enumerate()
        .filter(|(i, a)| m.terms.approvers.contains(a) && !approvals[..*i].contains(a))
        .count()
}

/// Build the exact transaction a request describes. The caller has already run
/// [`check_signing_authority`]; this refuses only what cannot be encoded.
pub fn build_transaction(req: &SigningRequest) -> Result<Eip1559Tx, String> {
    let recipient = hex::parse_evm_address(&req.recipient).ok_or("recipient is not an address")?;
    let (to, value, data) = match &req.asset {
        EvmAsset::Native => (recipient, req.amount.0, Vec::new()),
        EvmAsset::Erc20 { token } => (
            hex::parse_evm_address(token).ok_or("token is not an address")?,
            0,
            erc20_transfer_data(&recipient, req.amount.0),
        ),
    };
    Ok(Eip1559Tx {
        chain_id: req.chain_id,
        nonce: req.evm_nonce,
        max_priority_fee_per_gas: req.max_priority_fee_per_gas.0,
        max_fee_per_gas: req.max_fee_per_gas.0,
        gas_limit: req.gas_limit,
        to,
        value,
        data,
        access_list: Vec::new(),
    })
}

/// The versioned intent for a request under mandate `m`, signed through `ctx`.
pub fn build_intent(
    m: &SigningMandate,
    req: &SigningRequest,
    ctx: &SigningContext,
) -> Result<(X3SigningIntent, Eip1559Tx), String> {
    let tx = build_transaction(req)?;
    let total_fee = (req.gas_limit as u128)
        .checked_mul(req.max_fee_per_gas.0)
        .ok_or("fee overflow")?;
    let intent = X3SigningIntent {
        version: SIGNING_INTENT_VERSION,
        program_sha256: req.program_sha256.clone(),
        owner: m.owner.clone(),
        agent: m.agent.clone(),
        mandate_version: m.version,
        intent_nonce: req.intent_nonce.clone(),
        chain_namespace: EVM_NAMESPACE.into(),
        chain_id: req.chain_id,
        mpc_signer: ctx.mpc_signer.clone(),
        mpc_domain_id: ctx.mpc_domain_id,
        derivation_predecessor: ctx.predecessor.clone(),
        derivation_path: ctx.derivation_path.into(),
        key_index: req.key_index,
        from: ctx.from.into(),
        recipient: req.recipient.clone(),
        asset: req.asset.clone(),
        amount: req.amount,
        tx_to: hex::format_evm_address(&tx.to),
        tx_value: U128(tx.value),
        tx_data: hex::encode(&tx.data),
        evm_nonce: req.evm_nonce,
        max_priority_fee_per_gas: req.max_priority_fee_per_gas,
        max_fee_per_gas: req.max_fee_per_gas,
        gas_limit: req.gas_limit,
        max_total_fee: U128(total_fee),
        unsigned_tx: hex::encode(&tx.unsigned_bytes()),
        signing_hash: hex::encode(&tx.signing_hash()),
        expires_at_ms: req.expires_at_ms,
        refund: RefundStrategy::AtomicTransferNoRefund,
    };
    Ok((intent, tx))
}

/// The canonical Borsh bytes of an intent: the preimage of [`intent_hash`].
pub fn canonical_bytes(intent: &X3SigningIntent) -> Vec<u8> {
    near_sdk::borsh::to_vec(intent).expect("borsh serialization of an in-memory intent")
}

/// SHA-256 of the canonical intent, lowercase hex. Approvers sign off on this value.
pub fn intent_hash(intent: &X3SigningIntent) -> String {
    hex::encode(&near_sdk::env::sha256_array(canonical_bytes(intent)))
}

pub fn failed(results: &[GuardResult]) -> Vec<String> {
    results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| format!("{} ({})", r.guard, r.detail))
        .collect()
}
