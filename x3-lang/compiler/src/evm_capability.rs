//! Bounded offline EVM action proposal; owner approval and live checks are separate.
use crate::compile_capability_source;
use sha2::{Digest, Sha256};
use sha3::Keccak256;
use std::collections::BTreeSet;
use x3_lang_ast::policy_ir::{AmountLimit, Resource};
use x3_lang_common::{Span, X3Error};

#[derive(Debug)]
pub struct Proposal {
    pub abi: Vec<u8>,
    pub action_id: [u8; 32],
    pub policy_hash: [u8; 32],
}

fn refuse(message: &str) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: Span::DUMMY,
    }
}

fn address(subject: &str) -> Result<[u8; 20], X3Error> {
    let s = subject
        .strip_prefix("0x")
        .ok_or_else(|| refuse("EVM subject must be an address"))?;
    if s.len() != 40 || !s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        return Err(refuse("invalid lowercase EVM subject"));
    }
    let mut out = [0; 20];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).map_err(|_| refuse("invalid address"))?;
    }
    if out == [0; 20] {
        return Err(refuse("zero agent"));
    }
    Ok(out)
}
fn uint(out: &mut Vec<u8>, n: u64) {
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&n.to_be_bytes());
}
fn addr(out: &mut Vec<u8>, a: &[u8; 20]) {
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(a);
}
/// Only root exact calls on Base Sepolia, zero native value and no claimed requirements.
/// Caller supplies independently trusted scope/time and reviewed policy epoch/nonce.
/// Calldata must match the declared selector and Keccak digest.
pub fn compile_evm_action_source(
    source: &str,
    trusted_scope: &str,
    now: u64,
    policy_version: u64,
    nonce: u64,
    calldata: &[u8],
) -> Result<Proposal, X3Error> {
    let c = compile_capability_source(source, trusted_scope, &BTreeSet::new(), now)?;
    let g = &c.grant;
    if g.parent.is_some()
        || g.delegation_depth != 0
        || g.amount != AmountLimit::NoAsset
        || policy_version == 0
        || calldata.len() < 4
        || calldata.len() > 516
    {
        return Err(refuse("unsupported EVM grant or action context"));
    }
    let Resource::EvmCall {
        chain_id,
        executor,
        target,
        selector,
        calldata_hash,
    } = &g.resource
    else {
        return Err(refuse("EVM resource required"));
    };
    let hash: [u8; 32] = Keccak256::digest(calldata).into();
    if *chain_id != 84532 || calldata[..4] != *selector || hash != *calldata_hash {
        return Err(refuse("chain, selector or calldata hash mismatch"));
    }
    let agent = address(&g.subject)?;
    let mut policy = Sha256::new();
    policy.update(b"X3/PIR/EVM-POLICY/1\0");
    policy.update(c.source_sha256);
    policy.update(c.commitment.digest);
    let policy_hash: [u8; 32] = policy.finalize().into();
    let mut abi = Vec::with_capacity(352);
    uint(&mut abi, *chain_id);
    addr(&mut abi, executor);
    addr(&mut abi, target);
    abi.extend_from_slice(selector);
    abi.extend_from_slice(&[0; 28]);
    abi.extend_from_slice(calldata_hash);
    uint(&mut abi, 0);
    addr(&mut abi, &agent);
    abi.extend_from_slice(&policy_hash);
    uint(&mut abi, policy_version);
    uint(&mut abi, nonce);
    uint(&mut abi, g.expires_at);
    let mut h = Keccak256::new();
    h.update(Keccak256::digest(b"X3ContractActionV1"));
    h.update(&abi);
    Ok(Proposal {
        abi,
        action_id: h.finalize().into(),
        policy_hash,
    })
}
