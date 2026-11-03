//! Signed, domain-separated AI payment authorization for X3Lang tooling.
//!
//! This is a stateful **local risk-preflight ledger**, not wallet custody or an
//! on-chain spend-limit contract. The runtime must supply a trusted day/block,
//! persist counters, and execute a real token transfer through an authenticated
//! capability before it can be deployed as an authorization boundary.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const DOMAIN: &[u8] = b"X3_AGENT_PAYMENT_AUTHORIZATION_V1\0";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentPolicy {
    pub chain: String,
    pub asset: String,
    pub allowed_recipients: BTreeSet<String>,
    pub allowed_contracts: BTreeSet<String>,
    pub allowed_methods: BTreeSet<String>,
    pub allowed_owner_keys: BTreeSet<[u8; 32]>,
    pub max_per_payment: u128,
    pub max_per_day: u128,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentIntent {
    pub chain: String,
    pub asset: String,
    pub recipient: String,
    pub contract: String,
    pub method: String,
    pub amount: u128,
    /// Owner-controlled nonce; replay checked across all days in this instance.
    pub nonce: u64,
    pub day: u64,
    pub expiry_block: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignedPaymentIntent {
    pub intent: PaymentIntent,
    pub public_key: [u8; 32],
    pub signature: [u8; 64],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPaymentPermit {
    pub amount: u128,
    pub recipient: String,
    pub authorization: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FirewallError {
    InvalidPolicy,
    UnknownSigner,
    BadSignature,
    WrongDay,
    Expired,
    Replay,
    InvalidPayment,
    UnapprovedChain,
    UnapprovedAsset,
    UnapprovedRecipient,
    UnapprovedContract,
    UnapprovedMethod,
    PaymentLimitExceeded,
    DailyLimitExceeded,
    Overflow,
}

/// SHA-256 of a domain-separated, canonical struct serialization. The JSON
/// serializer's field order is fixed by PaymentIntent's Rust declaration;
/// no floats or lossy conversions occur in this signed message.
pub fn approval_digest(intent: &PaymentIntent) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(DOMAIN);
    hash.update(serde_json::to_vec(intent).expect("fixed integer/string fields are serializable"));
    hash.finalize().into()
}

#[derive(Clone, Debug)]
pub struct AgentFirewall {
    policy: AgentPolicy,
    used: BTreeSet<([u8; 32], u64)>,
    daily_spent: BTreeMap<u64, u128>,
}

impl AgentFirewall {
    pub fn new(policy: AgentPolicy) -> Result<Self, FirewallError> {
        if policy.chain.trim().is_empty()
            || policy.asset.trim().is_empty()
            || policy.max_per_payment == 0
            || policy.max_per_payment > policy.max_per_day
            || policy.allowed_recipients.is_empty()
            || policy.allowed_contracts.is_empty()
            || policy.allowed_methods.is_empty()
            || policy.allowed_owner_keys.is_empty()
            || policy.allowed_recipients.iter().any(|s| s.trim().is_empty())
            || policy.allowed_contracts.iter().any(|s| s.trim().is_empty())
            || policy.allowed_methods.iter().any(|s| s.trim().is_empty())
        {
            return Err(FirewallError::InvalidPolicy);
        }
        Ok(Self {
            policy,
            used: BTreeSet::new(),
            daily_spent: BTreeMap::new(),
        })
    }

    pub fn spent_on_day(&self, day: u64) -> u128 {
        self.daily_spent.get(&day).copied().unwrap_or(0)
    }

    /// Validate a human-signed payment request and book a *local* allowance.
    ///
    /// A permit here does **not** sign a wallet transaction, call a contract
    /// or prove global daily spending. Those protections require persistent,
    /// authenticated runtime state; callers cannot treat this as final.
    pub fn record_approved_request(
        &mut self,
        signed: &SignedPaymentIntent,
        current_day: u64,
        current_block: u64,
    ) -> Result<LocalPaymentPermit, FirewallError> {
        if !self.policy.allowed_owner_keys.contains(&signed.public_key) {
            return Err(FirewallError::UnknownSigner);
        }
        let signer = VerifyingKey::from_bytes(&signed.public_key).map_err(|_| FirewallError::BadSignature)?;
        let signature = Signature::from_bytes(&signed.signature);
        signer
            .verify_strict(&approval_digest(&signed.intent), &signature)
            .map_err(|_| FirewallError::BadSignature)?;
        let intent = &signed.intent;
        if intent.day != current_day {
            return Err(FirewallError::WrongDay);
        }
        if current_block > intent.expiry_block {
            return Err(FirewallError::Expired);
        }
        if self.used.contains(&(signed.public_key, intent.nonce)) {
            return Err(FirewallError::Replay);
        }
        if intent.amount == 0 || intent.recipient.trim().is_empty() {
            return Err(FirewallError::InvalidPayment);
        }
        if intent.chain != self.policy.chain {
            return Err(FirewallError::UnapprovedChain);
        }
        if intent.asset != self.policy.asset {
            return Err(FirewallError::UnapprovedAsset);
        }
        if !self.policy.allowed_recipients.contains(&intent.recipient) {
            return Err(FirewallError::UnapprovedRecipient);
        }
        if !self.policy.allowed_contracts.contains(&intent.contract) {
            return Err(FirewallError::UnapprovedContract);
        }
        if !self.policy.allowed_methods.contains(&intent.method) {
            return Err(FirewallError::UnapprovedMethod);
        }
        if intent.amount > self.policy.max_per_payment {
            return Err(FirewallError::PaymentLimitExceeded);
        }
        let total = self
            .spent_on_day(current_day)
            .checked_add(intent.amount)
            .ok_or(FirewallError::Overflow)?;
        if total > self.policy.max_per_day {
            return Err(FirewallError::DailyLimitExceeded);
        }
        self.daily_spent.insert(current_day, total);
        self.used.insert((signed.public_key, intent.nonce));
        Ok(LocalPaymentPermit {
            amount: intent.amount,
            recipient: intent.recipient.clone(),
            authorization: "local_preflight_only",
        })
    }
}
