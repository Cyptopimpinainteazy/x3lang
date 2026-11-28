//! Compiled authority bounds for a native liquidation host effect.
//!
//! All loan amounts are raw units of `loan_token`; fee amounts are wei. This
//! declaration authorizes neither a signature nor a public transaction by itself.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LiquidationPolicy {
    pub chain_id: u64,
    pub executor: String,
    pub executor_code_hash: String,
    pub keeper: String,
    pub owner: String,
    pub morpho: String,
    pub loan_token: String,
    pub markets: Vec<String>,
    pub routers: Vec<String>,
    pub min_repay: u128,
    pub max_repay: u128,
    pub min_profit: u128,
    pub max_gas: u64,
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
    pub max_total_fee_wei: u128,
    pub max_age_blocks: u64,
    pub expires_at_block: u64,
    pub max_candidates: u32,
}

impl LiquidationPolicy {
    /// Validate independently at every untrusted source/IR/bytecode boundary.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("chain_id", u128::from(self.chain_id)),
            ("min_repay", self.min_repay),
            ("max_repay", self.max_repay),
            ("min_profit", self.min_profit),
            ("max_gas", u128::from(self.max_gas)),
            ("max_fee_per_gas", self.max_fee_per_gas),
            ("max_total_fee_wei", self.max_total_fee_wei),
            ("expires_at_block", u128::from(self.expires_at_block)),
        ] {
            if value == 0 {
                return Err(format!("native liquidation {field} must be greater than zero"));
            }
        }
        if self.min_repay > self.max_repay {
            return Err("native liquidation min_repay exceeds max_repay".into());
        }
        if self.max_priority_fee_per_gas > self.max_fee_per_gas {
            return Err("native liquidation priority fee exceeds max_fee_per_gas".into());
        }
        if !(1..=64).contains(&self.max_candidates) {
            return Err("native liquidation max_candidates must be in 1..=64".into());
        }
        if self.max_age_blocks > 64 {
            return Err("native liquidation max_age_blocks must be at most 64".into());
        }
        for (field, value) in [
            ("executor", &self.executor),
            ("keeper", &self.keeper),
            ("owner", &self.owner),
            ("morpho", &self.morpho),
            ("loan_token", &self.loan_token),
        ] {
            canonical_hex(field, value, 20)?;
        }
        canonical_hex("executor_code_hash", &self.executor_code_hash, 32)?;
        if self.markets.is_empty() || self.markets.len() > 32 {
            return Err("native liquidation markets must contain 1..=32 entries".into());
        }
        if self.routers.is_empty() || self.routers.len() > 32 {
            return Err("native liquidation routers must contain 1..=32 entries".into());
        }
        validate_list("markets", &self.markets, 32)?;
        validate_list("routers", &self.routers, 20)?;
        Ok(())
    }
}

fn canonical_hex(field: &str, value: &str, bytes: usize) -> Result<(), String> {
    let Some(raw) = value.strip_prefix("0x") else {
        return Err(format!("native liquidation {field} requires lowercase 0x hex"));
    };
    if raw.len() != bytes * 2
        || !raw.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || raw.bytes().all(|b| b == b'0')
    {
        return Err(format!(
            "native liquidation {field} must be a nonzero canonical lowercase {bytes}-byte hex value"
        ));
    }
    Ok(())
}

fn validate_list(field: &str, values: &[String], bytes: usize) -> Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        canonical_hex(field, value, bytes)?;
        if !seen.insert(value) {
            return Err(format!("native liquidation {field} contains a duplicate"));
        }
    }
    Ok(())
}
