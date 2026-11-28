//! A bounded native liquidation effect. Amounts are raw loan-token units, never bps.
//!
//! The VM owns filtering, ranking and exact calldata construction. The host owns
//! RPC evidence, fee valuation and signing; it must independently re-simulate the
//! selected transaction. Neither a scanner assertion nor a VM rollback is chain proof.

use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::BTreeSet;
use x3_lang_common::{capability::encode_capability_payload, liquidation::LiquidationPolicy, CapabilityPayload};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub chain_id: u64,
    pub block_number: u64,
    pub block_hash: String,
    pub nonce: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketParams {
    pub loan_token: String,
    pub collateral_token: String,
    pub oracle: String,
    pub irm: String,
    pub lltv: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionCost {
    /// Conservative total native fee, including gas_limit * max_fee_per_gas.
    pub native_wei: u128,
    /// Conservative conversion of the total fee into the policy's loan token.
    /// This conversion is trusted host evidence, not a price proved by the VM.
    pub loan_units: u128,
    /// L1 data/operator/other native fees. None means unknown, never zero.
    pub extra_native_wei: Option<u128>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub id: String,
    pub market: MarketParams,
    pub borrower: String,
    /// This first adapter supports seize-asset orders only (repaidShares = 0).
    pub seized_assets: u128,
    pub repay_assets: u128,
    /// Scanner simulation result. The native effect must measure it again.
    pub gross_profit: u128,
    pub router: String,
    pub swap_data: Vec<u8>,
    pub gas_limit: u64,
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
    pub cost: Option<ExecutionCost>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Batch {
    pub snapshot: Snapshot,
    pub current_block: u64,
    pub candidates: Vec<Candidate>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizedLiquidation {
    pub policy: LiquidationPolicy,
    pub policy_hash: [u8; 32],
    pub artifact_hash: [u8; 32],
    pub snapshot: Snapshot,
    pub candidate: Candidate,
    pub calldata: Vec<u8>,
    pub net_profit: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffectStatus {
    Prepared,
    Submitted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectReceipt {
    /// Hash of the canonical unsigned EIP-1559 request, not an inclusion proof.
    pub request_hash: [u8; 32],
    pub transaction_hash: Option<String>,
    pub status: EffectStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionReceipt {
    pub authorization: AuthorizedLiquidation,
    pub effect: EffectReceipt,
}

/// Installed explicitly. There is deliberately no successful dry-run default.
/// A host must pin an owner-reviewed artifact, rather than trusting a scanner to
/// supply a newly compiled, more permissive mandate at execution time.
pub trait LiquidationHost {
    fn artifact_hash(&self) -> [u8; 32];
    fn candidates(&mut self, policy: &LiquidationPolicy) -> Result<Batch, String>;
    fn execute(&mut self, authorized: &AuthorizedLiquidation) -> Result<EffectReceipt, String>;
}

pub fn artifact_hash(bytes: &[u8]) -> [u8; 32] {
    Keccak256::digest(bytes).into()
}

pub fn policy_hash(policy: &LiquidationPolicy) -> Result<[u8; 32], String> {
    let encoded = encode_capability_payload(&CapabilityPayload::NativeLiquidation { policy: policy.clone() })
        .map_err(|e| e.to_string())?;
    Ok(artifact_hash(&encoded))
}

/// Deterministic selection, independent of input order. Equal net profits use
/// the candidate identifier's byte order. An invalid candidate is filtered; an
/// ambiguous, oversized or stale batch is refused before any effect.
pub fn authorize(
    policy: &LiquidationPolicy,
    artifact: [u8; 32],
    batch: &Batch,
) -> Result<AuthorizedLiquidation, String> {
    policy.validate()?;
    if hex_bytes::<32>(&batch.snapshot.block_hash)? == [0; 32] {
        return Err("X3_LIQUIDATION_ZERO_SNAPSHOT".into());
    }
    if batch.snapshot.chain_id != policy.chain_id {
        return Err("X3_LIQUIDATION_CHAIN".into());
    }
    if batch.current_block > policy.expires_at_block
        || batch.snapshot.block_number > batch.current_block
        || batch.current_block - batch.snapshot.block_number > policy.max_age_blocks
    {
        return Err("X3_LIQUIDATION_STALE".into());
    }
    if batch.candidates.is_empty() || batch.candidates.len() > policy.max_candidates as usize {
        return Err("X3_LIQUIDATION_BATCH_BOUND".into());
    }
    let mut ids = BTreeSet::new();
    let mut best: Option<(&Candidate, u128)> = None;
    for candidate in &batch.candidates {
        if candidate.id.is_empty() || candidate.id.len() > 128 || !ids.insert(&candidate.id) {
            return Err("X3_LIQUIDATION_CANDIDATE_ID".into());
        }
        let Ok(net) = candidate_net(policy, candidate) else {
            continue;
        };
        if best.is_none_or(|(previous, previous_net)| {
            net > previous_net || (net == previous_net && candidate.id < previous.id)
        }) {
            best = Some((candidate, net));
        }
    }
    let (candidate, net_profit) = best.ok_or("X3_LIQUIDATION_NO_ELIGIBLE_CANDIDATE")?;
    let floor = policy
        .min_profit
        .checked_add(candidate.cost.as_ref().unwrap().loan_units)
        .ok_or("X3_LIQUIDATION_AMOUNT_OVERFLOW")?;
    Ok(AuthorizedLiquidation {
        policy: policy.clone(),
        policy_hash: policy_hash(policy)?,
        artifact_hash: artifact,
        snapshot: batch.snapshot.clone(),
        candidate: candidate.clone(),
        calldata: encode_liquidation_call(candidate, floor)?,
        net_profit,
    })
}

fn candidate_net(policy: &LiquidationPolicy, c: &Candidate) -> Result<u128, String> {
    if hex_bytes::<20>(&c.borrower)? == [0; 20] {
        return Err("X3_LIQUIDATION_ZERO_BORROWER".into());
    }
    hex_bytes::<20>(&c.router)?;
    if c.market.loan_token != policy.loan_token
        || !policy.routers.contains(&c.router)
        || !policy.markets.contains(&market_id(&c.market)?)
        || c.repay_assets < policy.min_repay
        || c.repay_assets > policy.max_repay
        || c.seized_assets == 0
        || c.gas_limit == 0
        || c.gas_limit > policy.max_gas
        || c.max_fee_per_gas == 0
        || c.max_fee_per_gas > policy.max_fee_per_gas
        || c.max_priority_fee_per_gas > policy.max_priority_fee_per_gas
        || c.max_priority_fee_per_gas > c.max_fee_per_gas
        || c.swap_data.len() < 4
        || c.swap_data.len() > 16_384
    {
        return Err("X3_LIQUIDATION_BOUND".into());
    }
    let cost = c.cost.as_ref().ok_or("X3_LIQUIDATION_UNKNOWN_COST")?;
    let extra = cost.extra_native_wei.ok_or("X3_LIQUIDATION_UNKNOWN_EXTRA_FEE")?;
    let fee_bound = u128::from(c.gas_limit)
        .checked_mul(c.max_fee_per_gas)
        .and_then(|fee| fee.checked_add(extra))
        .ok_or("X3_LIQUIDATION_FEE_OVERFLOW")?;
    if cost.native_wei < fee_bound
        || cost.native_wei > policy.max_total_fee_wei
        || (cost.native_wei > 0 && cost.loan_units == 0)
    {
        return Err("X3_LIQUIDATION_FEE_BOUND".into());
    }
    let net = c
        .gross_profit
        .checked_sub(cost.loan_units)
        .ok_or("X3_LIQUIDATION_LOSS")?;
    if net < policy.min_profit {
        return Err("X3_LIQUIDATION_PROFIT_FLOOR".into());
    }
    Ok(net)
}

pub fn market_id(market: &MarketParams) -> Result<String, String> {
    Ok(hex_string(&artifact_hash(&encode_market(market)?)))
}

pub fn encode_market(market: &MarketParams) -> Result<Vec<u8>, String> {
    if market.lltv == 0 || market.lltv > 1_000_000_000_000_000_000 {
        return Err("X3_LIQUIDATION_LLTV".into());
    }
    let mut out = Vec::with_capacity(160);
    for value in [
        &market.loan_token,
        &market.collateral_token,
        &market.oracle,
        &market.irm,
    ] {
        out.extend_from_slice(&[0; 12]);
        out.extend_from_slice(&hex_bytes::<20>(value)?);
    }
    word(&mut out, market.lltv);
    Ok(out)
}

/// Canonical MonkeyLiquidatorSwap call. Caller-provided transaction calldata is
/// never accepted. The dynamic router payload is still untrusted and is checked
/// through the pinned executor's complete simulation before signing.
pub fn encode_liquidation_call(candidate: &Candidate, min_profit: u128) -> Result<Vec<u8>, String> {
    let signature =
        b"liquidate((address,address,address,address,uint256),(address,uint256,uint256,uint256,address,bytes))";
    let mut out = artifact_hash(signature)[..4].to_vec();
    out.extend(encode_market(&candidate.market)?);
    word(&mut out, 192); // offset of dynamic Order after the five market words
    address_word(&mut out, &candidate.borrower)?;
    word(&mut out, candidate.seized_assets);
    word(&mut out, 0); // repaidShares: seize-asset mode only
    word(&mut out, min_profit);
    address_word(&mut out, &candidate.router)?;
    word(&mut out, 192); // bytes offset relative to Order
    word(&mut out, candidate.swap_data.len() as u128);
    out.extend_from_slice(&candidate.swap_data);
    out.resize(out.len() + (32 - candidate.swap_data.len() % 32) % 32, 0);
    Ok(out)
}

fn address_word(out: &mut Vec<u8>, address: &str) -> Result<(), String> {
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(&hex_bytes::<20>(address)?);
    Ok(())
}

fn word(out: &mut Vec<u8>, amount: u128) {
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&amount.to_be_bytes());
}

pub fn hex_bytes<const N: usize>(value: &str) -> Result<[u8; N], String> {
    let bytes = value.as_bytes();
    if bytes.len() != 2 + N * 2 || !value.starts_with("0x") {
        return Err("X3_LIQUIDATION_HEX_WIDTH".into());
    }
    let nibble = |c: u8| match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        _ => Err("X3_LIQUIDATION_HEX_CANONICAL".to_owned()),
    };
    let mut result = [0; N];
    for (index, slot) in result.iter_mut().enumerate() {
        *slot = nibble(bytes[2 + index * 2])? * 16 + nibble(bytes[3 + index * 2])?;
    }
    Ok(result)
}

pub fn hex_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        use std::fmt::Write;
        write!(out, "{byte:02x}").unwrap();
    }
    out
}
