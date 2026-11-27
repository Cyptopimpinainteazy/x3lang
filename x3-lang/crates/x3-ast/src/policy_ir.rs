//! X3-PIR-02 typed capability model.
//!
//! Structural checks only. Callers must authenticate the parent and scope,
//! reserve shared budgets, enforce revocation, and prove runtime requirements.
//! Neither a grant nor a successful check is an execution authorization.

use crate::AssetId;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettlementOperation {
    Lock,
    Claim,
    Refund,
}

/// Exact resource, not a wildcard or an ABI-derived permission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resource {
    EvmCall {
        chain_id: u64,
        executor: [u8; 20],
        target: [u8; 20],
        selector: [u8; 4],
        calldata_hash: [u8; 32],
    },
    Tool {
        adapter: String,
        artifact_hash: [u8; 32],
    },
    NearBscWindow {
        window_id: [u8; 32],
        operation: SettlementOperation,
    },
}

/// Per-action ceiling only. NoAsset means no asset authority, never unlimited.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmountLimit {
    NoAsset,
    Asset { asset: AssetId, max_base_units: u128 },
}

/// No implicit ordering or equivalence between distinct guarantees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Requirement {
    ExactCodeIdentity,
    AtomicRollback,
    AuthenticatedFinality,
}

#[derive(Debug, Clone)]
pub struct Grant {
    pub subject: String,
    pub parent: Option<String>,
    pub scope: String,
    pub resource: Resource,
    pub amount: AmountLimit,
    /// Exclusive Unix-second deadline.
    pub expires_at: u64,
    pub delegation_depth: u8,
    pub requirements: BTreeSet<Requirement>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyError {
    Invalid,
    Identity,
    Resource,
    Amount,
    Expiry,
    Depth,
    Requirements,
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.as_bytes()[0].is_ascii_alphanumeric()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b":._/-".contains(&c))
}

fn valid(g: &Grant, now: u64) -> Result<(), PolicyError> {
    if !identifier(&g.subject)
        || !identifier(&g.scope)
        || g.parent.as_ref().is_some_and(|p| !identifier(p) || p == &g.subject)
    {
        return Err(PolicyError::Invalid);
    }
    match &g.resource {
        Resource::EvmCall {
            chain_id,
            executor,
            target,
            ..
        } if *chain_id == 0 || *executor == [0; 20] || *target == [0; 20] || executor == target => {
            return Err(PolicyError::Invalid);
        }
        Resource::Tool { adapter, .. } if !identifier(adapter) => return Err(PolicyError::Invalid),
        _ => {}
    }
    if let AmountLimit::Asset { asset, .. } = &g.amount {
        if asset.vm_family.is_empty()
            || asset.chain.as_str().is_empty()
            || asset.canonical_id.is_empty()
            || asset.symbol.is_empty()
        {
            return Err(PolicyError::Invalid);
        }
    }
    if now >= g.expires_at {
        return Err(PolicyError::Expiry);
    }
    Ok(())
}

/// Check one edge against an independently authenticated parent.
/// No budget reservation, signature verification, or global lineage check.
pub fn check_delegation(parent: &Grant, child: &Grant, now: u64) -> Result<(), PolicyError> {
    valid(parent, now)?;
    if child.subject == parent.subject || child.parent.as_deref() != Some(parent.subject.as_str()) {
        return Err(PolicyError::Identity);
    }
    valid(child, now)?;
    if child.scope != parent.scope || child.resource != parent.resource {
        return Err(PolicyError::Resource);
    }
    match (&parent.amount, &child.amount) {
        (AmountLimit::NoAsset, AmountLimit::NoAsset) => {}
        (
            AmountLimit::Asset {
                asset: p,
                max_base_units: p_max,
            },
            AmountLimit::Asset {
                asset: c,
                max_base_units: c_max,
            },
        ) if p == c && c_max <= p_max => {}
        _ => return Err(PolicyError::Amount),
    }
    if child.expires_at > parent.expires_at {
        return Err(PolicyError::Expiry);
    }
    if child.delegation_depth >= parent.delegation_depth {
        return Err(PolicyError::Depth);
    }
    if !parent.requirements.is_subset(&child.requirements) {
        return Err(PolicyError::Requirements);
    }
    Ok(())
}

/// Reject unsupported requirements before lowering. The trusted caller must
/// select support for this exact resource; support is not evidence of success.
pub fn check_support(grant: &Grant, supported: &BTreeSet<Requirement>, now: u64) -> Result<(), PolicyError> {
    valid(grant, now)?;
    if !grant.requirements.is_subset(supported) {
        return Err(PolicyError::Requirements);
    }
    Ok(())
}
