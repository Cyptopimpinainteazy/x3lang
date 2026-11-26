//! X3-PIR-03 canonical offline encodings. No authorization or execution.
use sha2::{Digest, Sha256};
use x3_lang_ast::policy_ir::{
    check_support, AmountLimit, Grant, PolicyError, Requirement, Resource, SettlementOperation,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingProfile {
    EvmAction,
    AgentAdmission,
    NearBscWindow,
}
impl BindingProfile {
    pub fn name(self) -> &'static str {
        match self {
            Self::EvmAction => "evm.action.v1",
            Self::AgentAdmission => "agent.admission.v1",
            Self::NearBscWindow => "near-bsc.window.v1",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commitment {
    pub encoded: Vec<u8>,
    pub digest: [u8; 32],
}

fn finish(encoded: Vec<u8>) -> Commitment {
    let digest = Sha256::digest(&encoded).into();
    Commitment { encoded, digest }
}

/// Exact compatibility with the existing X3-PIR-01 Python binding format.
/// The scope and native identifier must be selected independently by a
/// trusted caller. No native semantics are verified here.
pub fn encode_binding(
    profile: BindingProfile,
    source: &[u8],
    scope: &str,
    native_id: [u8; 32],
) -> Result<Commitment, PolicyError> {
    if source.is_empty()
        || source.len() > 65536
        || scope.is_empty()
        || scope.len() > 128
        || !scope.as_bytes()[0].is_ascii_alphanumeric()
        || !scope
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":._/-".contains(&b))
    {
        return Err(PolicyError::Invalid);
    }
    let mut out = b"X3/PIR/BINDING/1\0".to_vec();
    let p = profile.name().as_bytes();
    out.push(p.len() as u8);
    out.extend_from_slice(p);
    out.push(scope.len() as u8);
    out.extend_from_slice(scope.as_bytes());
    out.extend_from_slice(&Sha256::digest(source));
    out.extend_from_slice(&native_id);
    Ok(finish(out))
}

/// Length-prefixed, exact UTF-8. No normalization, alias resolution or
/// truncation. The maximum byte length is fixed by the v1 wire contract.
fn text(out: &mut Vec<u8>, value: &str) -> Result<(), PolicyError> {
    if value.is_empty() || value.len() > 256 {
        return Err(PolicyError::Invalid);
    }
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}

fn requirement_id(r: Requirement) -> u8 {
    match r {
        Requirement::ExactCodeIdentity => 1,
        Requirement::AtomicRollback => 2,
        Requirement::AuthenticatedFinality => 3,
    }
}

/// Deterministic commitment to a structurally valid typed grant. An expired
/// nonzero grant can still be encoded for historical audit; enforce time,
/// trusted requirements, delegation and actual effects separately.
pub fn encode_grant(grant: &Grant) -> Result<Commitment, PolicyError> {
    // Reuse the AST structural validation at epoch zero, not a claim that
    // any adapter supports these requirements or that the grant is live.
    check_support(grant, &grant.requirements, 0)?;
    let mut out = b"X3/PIR/GRANT/1\0".to_vec();
    text(&mut out, &grant.subject)?;
    match &grant.parent {
        None => out.push(0),
        Some(parent) => {
            out.push(1);
            text(&mut out, parent)?;
        }
    }
    text(&mut out, &grant.scope)?;
    match &grant.resource {
        Resource::EvmCall {
            chain_id,
            executor,
            target,
            selector,
            calldata_hash,
        } => {
            out.push(1);
            out.extend_from_slice(&chain_id.to_be_bytes());
            out.extend_from_slice(executor);
            out.extend_from_slice(target);
            out.extend_from_slice(selector);
            out.extend_from_slice(calldata_hash);
        }
        Resource::Tool { adapter, artifact_hash } => {
            out.push(2);
            text(&mut out, adapter)?;
            out.extend_from_slice(artifact_hash);
        }
        Resource::NearBscWindow { window_id, operation } => {
            out.push(3);
            out.extend_from_slice(window_id);
            out.push(match operation {
                SettlementOperation::Lock => 1,
                SettlementOperation::Claim => 2,
                SettlementOperation::Refund => 3,
            });
        }
    }
    match &grant.amount {
        AmountLimit::NoAsset => out.push(0),
        AmountLimit::Asset { asset, max_base_units } => {
            out.push(1);
            text(&mut out, asset.vm_family.as_str())?;
            text(&mut out, asset.chain.as_str())?;
            text(&mut out, asset.canonical_id.as_str())?;
            text(&mut out, asset.symbol.as_str())?;
            out.push(asset.decimals);
            out.extend_from_slice(&max_base_units.to_be_bytes());
        }
    }
    out.extend_from_slice(&grant.expires_at.to_be_bytes());
    out.push(grant.delegation_depth);
    let mut requirements: Vec<u8> = grant.requirements.iter().copied().map(requirement_id).collect();
    requirements.sort_unstable();
    out.push(requirements.len() as u8);
    out.extend_from_slice(&requirements);
    Ok(finish(out))
}
