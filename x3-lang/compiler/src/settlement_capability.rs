//! Offline native settlement plan, never permission to lock or release funds.
use crate::{parser::capability::parse_capability_source, policy_encoding::encode_grant};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use x3_lang_ast::policy_ir::{AmountLimit, Requirement, Resource, SettlementOperation};
use x3_lang_common::{Span, X3Error};

#[derive(Debug, Clone)]
pub struct WindowTerms {
    pub bsc_chain_id: u64,
    pub escrow: [u8; 20],
    pub depositor: [u8; 20],
    pub lp: [u8; 20],
    pub bnb_wei: u128,
    pub salt: [u8; 32],
    pub near_vault: String,
    pub near_initiator: String,
    pub near_recipient: String,
    pub hashlock: [u8; 32],
    pub near_yocto: u128,
    pub funding_deadline_ns: u64,
    pub expiry_ns: u64,
    pub near_genesis: [u8; 32],
    pub near_vault_code_hash: [u8; 32],
}
#[derive(Debug, PartialEq, Eq)]
pub enum PlanStatus {
    BlockedAwaitingNativeVerifier,
}
#[derive(Debug)]
pub struct Plan {
    pub source_binding: [u8; 32],
    pub window_id: [u8; 32],
    pub outcome_commitment: [u8; 32],
    pub outcome: u8,
    pub status: PlanStatus,
    pub plan_digest: [u8; 32],
}
fn refuse(message: &str) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: Span::DUMMY,
    }
}
fn account(name: &str) -> bool {
    name.len() <= 64
        && name.ends_with(".testnet")
        && name.split('.').all(|part| {
            part.split(['-', '_'])
                .all(|word| !word.is_empty() && word.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
        })
}
fn validate(t: &WindowTerms) -> Result<(), X3Error> {
    if !matches!(t.bsc_chain_id, 97 | 31337)
        || t.escrow == [0; 20]
        || t.depositor == [0; 20]
        || t.lp == [0; 20]
        || t.bnb_wei == 0
        || t.near_yocto == 0
        || t.hashlock == [0; 32]
        || t.funding_deadline_ns == 0
        || t.funding_deadline_ns >= t.expiry_ns
        || t.near_genesis == [0; 32]
        || t.near_vault_code_hash == [0; 32]
        || ![&t.near_vault, &t.near_initiator, &t.near_recipient]
            .iter()
            .all(|s| account(s))
    {
        return Err(refuse("invalid testnet settlement terms"));
    }
    Ok(())
}
fn ids(t: &WindowTerms) -> ([u8; 32], [u8; 32]) {
    let mut h = Sha256::new();
    h.update(b"X3-BSC-SOURCE-V1");
    h.update([0; 24]);
    h.update(t.bsc_chain_id.to_be_bytes());
    h.update(t.escrow);
    h.update(t.depositor);
    h.update(t.lp);
    h.update([0; 16]);
    h.update(t.bnb_wei.to_be_bytes());
    h.update(t.salt);
    let binding: [u8; 32] = h.finalize().into();
    let mut h = Sha256::new();
    h.update(b"X3-NEAR-BSC-WINDOW-V1");
    for s in [&t.near_vault, &t.near_initiator, &t.near_recipient] {
        h.update((s.len() as u32).to_be_bytes());
        h.update(s.as_bytes());
    }
    h.update(t.hashlock);
    h.update(t.near_yocto.to_be_bytes());
    h.update(t.funding_deadline_ns.to_be_bytes());
    h.update(t.expiry_ns.to_be_bytes());
    h.update(binding);
    (binding, h.finalize().into())
}
/// Produce a blocked, owner-reviewable plan only. Trusted terms are selected
/// independently of the source; they are not authenticated chain observations.
/// No finality support or proof can be asserted through this API.
pub fn compile_settlement_plan(source: &str, trusted_scope: &str, now: u64, t: &WindowTerms) -> Result<Plan, X3Error> {
    let g = parse_capability_source(source)?;
    let required = BTreeSet::from([Requirement::ExactCodeIdentity, Requirement::AuthenticatedFinality]);
    if g.scope != trusted_scope
        || g.parent.is_some()
        || g.delegation_depth != 0
        || g.amount != AmountLimit::NoAsset
        || g.expires_at <= now
        || g.requirements != required
    {
        return Err(refuse("unsupported settlement plan or context"));
    }
    let Resource::NearBscWindow { window_id, operation } = &g.resource else {
        return Err(refuse("settlement window resource required"));
    };
    validate(t)?;
    let (binding, window) = ids(t);
    if *window_id != window {
        return Err(refuse("native settlement terms mismatch"));
    }
    let outcome = match operation {
        SettlementOperation::Lock => {
            if u128::from(now) * 1_000_000_000 >= u128::from(t.funding_deadline_ns) {
                return Err(refuse("funding deadline passed"));
            }
            1
        }
        SettlementOperation::Claim => 2,
        SettlementOperation::Refund => 3,
    };
    let mut h = Sha256::new();
    h.update(b"X3-NEAR-SETTLEMENT-V1");
    h.update(window);
    h.update([outcome]);
    let commitment: [u8; 32] = h.finalize().into();
    let grant = encode_grant(&g).map_err(|_| refuse("invalid typed settlement grant"))?;
    let mut h = Sha256::new();
    h.update(b"X3/PIR/SETTLEMENT-PLAN/1\0");
    h.update(Sha256::digest(source.as_bytes()));
    h.update(grant.digest);
    h.update(t.near_genesis);
    h.update(t.near_vault_code_hash);
    h.update(commitment);
    Ok(Plan {
        source_binding: binding,
        window_id: window,
        outcome_commitment: commitment,
        outcome,
        status: PlanStatus::BlockedAwaitingNativeVerifier,
        plan_digest: h.finalize().into(),
    })
}
