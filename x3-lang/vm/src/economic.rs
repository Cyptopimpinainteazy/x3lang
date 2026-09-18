//! Canonical economic policy, snapshot, and plan commitments.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use x3_lang_compiler::ir::{
    AssetKey, CompiledTradingPolicy, CostKind, StateBindingMode, SubmissionProfile, TradingOperation,
};

const ECONOMIC_SCHEMA_VERSION: u16 = 1;
const SNAPSHOT_DOMAIN: &[u8] = b"X3:ECONOMIC_SNAPSHOT:V1";
const PLAN_DOMAIN: &[u8] = b"X3:ECONOMIC_PLAN:V1";
const POLICY_DOMAIN: &[u8] = b"X3:ECONOMIC_POLICY:V1";

/// Stable failures emitted while validating or committing economic objects.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EconomicError {
    UnsupportedVersion { object: &'static str, version: u16 },
    CanonicalEncoding,
    PolicyWeakening(&'static str),
    InconsistentPolicy(&'static str),
}

impl fmt::Display for EconomicError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { object, version } => {
                write!(formatter, "unsupported {object} version {version}")
            }
            Self::CanonicalEncoding => formatter.write_str("canonical economic encoding failed"),
            Self::PolicyWeakening(field) => {
                write!(formatter, "runtime policy weakens compiled field {field}")
            }
            Self::InconsistentPolicy(field) => {
                write!(formatter, "compiled policy has inconsistent field {field}")
            }
        }
    }
}

impl std::error::Error for EconomicError {}

/// Domain-separated SHA-256 commitment over canonical bincode bytes.
pub trait CanonicalCommitment {
    fn domain(&self) -> &'static [u8];
    fn canonical_bytes(&self) -> Result<Vec<u8>, EconomicError>;

    fn commitment(&self) -> Result<[u8; 32], EconomicError> {
        let mut hasher = Sha256::new();
        hasher.update(self.domain());
        hasher.update(self.canonical_bytes()?);
        Ok(hasher.finalize().into())
    }
}

/// Immutable economic limits bound into a compiled artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicPolicy {
    pub version: u16,
    pub policy_id: String,
    pub chain: String,
    pub settlement_asset: AssetKey,
    pub minimum_net_profit: u128,
    pub max_total_cost: u128,
    pub max_slippage_bps: u16,
    pub max_price_impact_bps: u16,
    pub max_mev_leakage_bps: u16,
    pub quote_freshness_blocks: u64,
    pub deadline_blocks: u64,
    pub submission_profile: SubmissionProfile,
    pub state_binding: StateBindingMode,
    pub allowed_cost_kinds: BTreeSet<CostKind>,
    pub allow_mint: bool,
    pub allow_burn: bool,
}

impl EconomicPolicy {
    pub fn from_compiled(compiled: &CompiledTradingPolicy, settlement_asset: AssetKey) -> Result<Self, EconomicError> {
        if compiled.policy_version != ECONOMIC_SCHEMA_VERSION {
            return Err(EconomicError::UnsupportedVersion {
                object: "compiled_policy",
                version: compiled.policy_version,
            });
        }
        if !compiled.submission_profile_is_consistent() {
            return Err(EconomicError::InconsistentPolicy("submission_profile"));
        }

        Ok(Self {
            version: compiled.policy_version,
            policy_id: compiled.policy_id.clone(),
            chain: compiled.chain.clone(),
            settlement_asset,
            minimum_net_profit: compiled.minimum_net_profit.unwrap_or(0),
            max_total_cost: compiled.max_total_cost,
            max_slippage_bps: compiled.max_slippage_bps,
            max_price_impact_bps: compiled.max_price_impact_bps,
            max_mev_leakage_bps: compiled.max_mev_leakage_bps,
            quote_freshness_blocks: compiled.quote_freshness_blocks,
            deadline_blocks: compiled.deadline_blocks,
            submission_profile: compiled.submission_profile,
            state_binding: compiled.state_binding,
            allowed_cost_kinds: compiled.allowed_cost_kinds.clone(),
            allow_mint: compiled.allow_mint,
            allow_burn: compiled.allow_burn,
        })
    }

    pub fn validate_version(&self) -> Result<(), EconomicError> {
        validate_version("policy", self.version)
    }

    /// Runtime policy may only reduce ceilings, increase floors, or add restrictions.
    pub fn validate_not_weaker_than(&self, compiled: &Self) -> Result<(), EconomicError> {
        self.validate_version()?;
        compiled.validate_version()?;

        if self.policy_id != compiled.policy_id {
            return Err(EconomicError::PolicyWeakening("policy_id"));
        }
        if self.chain != compiled.chain {
            return Err(EconomicError::PolicyWeakening("chain"));
        }
        if self.settlement_asset != compiled.settlement_asset {
            return Err(EconomicError::PolicyWeakening("settlement_asset"));
        }
        if self.minimum_net_profit < compiled.minimum_net_profit {
            return Err(EconomicError::PolicyWeakening("minimum_net_profit"));
        }
        if self.max_total_cost > compiled.max_total_cost {
            return Err(EconomicError::PolicyWeakening("max_total_cost"));
        }
        if self.max_slippage_bps > compiled.max_slippage_bps {
            return Err(EconomicError::PolicyWeakening("max_slippage_bps"));
        }
        if self.max_price_impact_bps > compiled.max_price_impact_bps {
            return Err(EconomicError::PolicyWeakening("max_price_impact_bps"));
        }
        if self.max_mev_leakage_bps > compiled.max_mev_leakage_bps {
            return Err(EconomicError::PolicyWeakening("max_mev_leakage_bps"));
        }
        if self.quote_freshness_blocks > compiled.quote_freshness_blocks {
            return Err(EconomicError::PolicyWeakening("quote_freshness_blocks"));
        }
        if self.deadline_blocks > compiled.deadline_blocks {
            return Err(EconomicError::PolicyWeakening("deadline_blocks"));
        }
        if self.submission_profile < compiled.submission_profile {
            return Err(EconomicError::PolicyWeakening("submission_profile"));
        }
        if self.state_binding < compiled.state_binding {
            return Err(EconomicError::PolicyWeakening("state_binding"));
        }
        if !self.allowed_cost_kinds.is_subset(&compiled.allowed_cost_kinds) {
            return Err(EconomicError::PolicyWeakening("allowed_cost_kinds"));
        }
        if self.allow_mint && !compiled.allow_mint {
            return Err(EconomicError::PolicyWeakening("allow_mint"));
        }
        if self.allow_burn && !compiled.allow_burn {
            return Err(EconomicError::PolicyWeakening("allow_burn"));
        }

        Ok(())
    }
}

impl CanonicalCommitment for EconomicPolicy {
    fn domain(&self) -> &'static [u8] {
        POLICY_DOMAIN
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, EconomicError> {
        bincode::serialize(self).map_err(|_| EconomicError::CanonicalEncoding)
    }
}

/// Deterministic assumptions against which an economic plan is evaluated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicSnapshot {
    pub version: u16,
    pub chain: String,
    pub vm_family: String,
    pub block_height: u64,
    pub block_hash: Option<[u8; 32]>,
    pub host_state_commitment: [u8; 32],
    pub oracle_observations: BTreeMap<String, u128>,
    pub oracle_provenance: BTreeMap<String, String>,
    pub quote_observation_blocks: BTreeMap<String, u64>,
    pub balances: BTreeMap<AssetKey, u128>,
    pub debt_state: BTreeMap<String, u128>,
    pub route_commitment: [u8; 32],
    pub operations_commitment: [u8; 32],
    pub expected_outputs: BTreeMap<String, u128>,
    pub predicted_costs: BTreeMap<CostKind, u128>,
    pub observed_at_unix_seconds: u64,
}

impl EconomicSnapshot {
    pub fn validate_version(&self) -> Result<(), EconomicError> {
        validate_version("snapshot", self.version)
    }
}

impl CanonicalCommitment for EconomicSnapshot {
    fn domain(&self) -> &'static [u8] {
        SNAPSHOT_DOMAIN
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, EconomicError> {
        bincode::serialize(self).map_err(|_| EconomicError::CanonicalEncoding)
    }
}

/// Exact ordered operations and bindings authorized for economic execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EconomicPlan {
    pub version: u16,
    pub artifact_hash: [u8; 32],
    pub operations: Vec<TradingOperation>,
    pub asset_bindings: BTreeMap<String, AssetKey>,
    pub amount_bindings: BTreeMap<String, u128>,
    pub venues: BTreeSet<String>,
    pub providers: BTreeSet<String>,
    pub minimum_outputs: BTreeMap<u32, u128>,
    pub deadlines: BTreeMap<u32, u64>,
    pub expected_costs: BTreeMap<CostKind, u128>,
    pub expected_value_flow_commitment: [u8; 32],
    pub snapshot_commitment: [u8; 32],
    pub policy_commitment: [u8; 32],
}

impl EconomicPlan {
    pub fn validate_version(&self) -> Result<(), EconomicError> {
        validate_version("plan", self.version)
    }
}

impl CanonicalCommitment for EconomicPlan {
    fn domain(&self) -> &'static [u8] {
        PLAN_DOMAIN
    }

    fn canonical_bytes(&self) -> Result<Vec<u8>, EconomicError> {
        bincode::serialize(self).map_err(|_| EconomicError::CanonicalEncoding)
    }
}

fn validate_version(object: &'static str, version: u16) -> Result<(), EconomicError> {
    if version == ECONOMIC_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(EconomicError::UnsupportedVersion { object, version })
    }
}
