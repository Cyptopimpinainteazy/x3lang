//! Candidate NEAR light-client epoch transitions with real cryptography.
//!
//! WARNING: neither a JSON RPC response nor the previous `next_bps` field
//! establishes a trust anchor. This state machine is deliberately named
//! UNTRUSTED and has no way to issue an authenticated settlement certificate.
use crate::{approval_message, reconstruct_next_block_hash, verify_approvals, Validator};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

pub type Hash = [u8; 32];
const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";

fn sha(data: &[u8]) -> Hash {
    Sha256::digest(data).into()
}

fn b58(input: &str, required: usize) -> Result<Vec<u8>, &'static str> {
    if !(required == 32 || required == 64) || input.is_empty() || input.len() > 90 {
        return Err("invalid base58 value length");
    }
    let mut out = vec![0u8; required];
    for c in input.bytes() {
        let val = ALPHABET.iter().position(|x| *x == c).ok_or("invalid base58 alphabet")?;
        let mut carry = val as u32;
        for byte in out.iter_mut().rev() {
            let next = (*byte as u32) * 58 + carry;
            *byte = (next & 255) as u8;
            carry = next >> 8;
        }
        if carry != 0 {
            return Err("base58 value overflow");
        }
    }
    // Leading zero encoded as '1' must agree with canonical byte padding.
    let leading = input.bytes().take_while(|c| *c == b'1').count();
    if leading != out.iter().take_while(|c| **c == 0).count() {
        return Err("non-canonical base58 representation");
    }
    Ok(out)
}

fn hash(s: &str) -> Result<Hash, &'static str> {
    b58(s, 32)?.try_into().map_err(|_| "bad hash")
}

fn key(s: &str) -> Result<Hash, &'static str> {
    hash(s.strip_prefix("ed25519:").ok_or("unsupported validator key type")?)
}

fn signature(s: &str) -> Result<[u8; 64], &'static str> {
    b58(s.strip_prefix("ed25519:").ok_or("unsupported signature scheme")?, 64)?
        .try_into()
        .map_err(|_| "bad signature length")
}

fn field_str<'a>(v: &'a Value, name: &str) -> Result<&'a str, &'static str> {
    v.get(name)
        .and_then(Value::as_str)
        .ok_or("missing or invalid string field")
}

fn field_hash(v: &Value, name: &str) -> Result<Hash, &'static str> {
    hash(field_str(v, name)?)
}

fn u64field(v: &Value, name: &str) -> Result<u64, &'static str> {
    v.get(name).and_then(Value::as_u64).ok_or("invalid u64 field")
}

fn borsh_u32(len: usize) -> Result<[u8; 4], &'static str> {
    Ok(u32::try_from(len).map_err(|_| "borsh length overflow")?.to_le_bytes())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Producer {
    pub account_id: String,
    pub validator: Validator,
}

fn parse_producers(values: &Value) -> Result<Vec<Producer>, &'static str> {
    let arr = values.as_array().ok_or("missing validator array")?;
    if arr.is_empty() || arr.len() > 4096 {
        return Err("unbounded validator array");
    }
    let mut accounts = HashSet::new();
    let mut keys = HashSet::new();
    let mut result = Vec::with_capacity(arr.len());
    for v in arr {
        if field_str(v, "validator_stake_struct_version")? != "V1" {
            return Err("unsupported validator stake version");
        }
        let account = field_str(v, "account_id")?;
        if account.len() > 64
            || account.len() < 2
            || !account.split('.').all(|segment| {
                !segment.is_empty()
                    && segment
                        .bytes()
                        .all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == b'_' || x == b'-')
            })
        {
            return Err("invalid validator account");
        }
        let stake_text = field_str(v, "stake")?;
        if stake_text.is_empty() || stake_text.starts_with('0') || !stake_text.bytes().all(|x| x.is_ascii_digit()) {
            return Err("bad validator stake");
        }
        let stake = stake_text.parse::<u128>().map_err(|_| "stake overflow")?;
        if stake == 0 {
            return Err("zero validator stake");
        }
        let public_key = key(field_str(v, "public_key")?)?;
        if !keys.insert(public_key) || !accounts.insert(account) {
            return Err("duplicate validator");
        }
        result.push(Producer {
            account_id: account.to_owned(),
            validator: Validator { public_key, stake },
        });
    }
    Ok(result)
}

pub fn borsh_validator_hash(producers: &[Producer]) -> Result<Hash, &'static str> {
    if producers.is_empty() || producers.len() > 4096 {
        return Err("bad validator count");
    }
    let mut buf = Vec::new();
    buf.extend_from_slice(&borsh_u32(producers.len())?);
    let mut accounts = HashSet::new();
    let mut keys = HashSet::new();
    for v in producers {
        if !accounts.insert(v.account_id.as_str())
            || !keys.insert(v.validator.public_key)
            || v.validator.stake == 0
            || v.account_id.len() > 64
        {
            return Err("invalid or duplicate validator");
        }
        buf.push(0); // Borsh ValidatorStake::V1
        buf.extend_from_slice(&borsh_u32(v.account_id.len())?);
        buf.extend_from_slice(v.account_id.as_bytes());
        buf.push(0); // Borsh PublicKey::ED25519
        buf.extend_from_slice(&v.validator.public_key);
        buf.extend_from_slice(&v.validator.stake.to_le_bytes());
    }
    Ok(sha(&buf))
}

#[derive(Debug, Clone)]
pub struct CandidateView {
    height: u64,
    timestamp_ns: u64,
    epoch_id: Hash,
    next_epoch_id: Hash,
    next_bp_hash: Hash,
    header_hash: Hash,
    next_block_inner_hash: Hash,
    next_bps: Option<Vec<Producer>>,
    approvals: Vec<Option<[u8; 64]>>,
}

pub fn parse_rpc_light_view(response: &Value) -> Result<CandidateView, &'static str> {
    if response.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || response.get("error").is_some() {
        return Err("bad light client RPC result");
    }
    let r = response.get("result").ok_or("missing RPC result")?;
    let inner = r.get("inner_lite").ok_or("missing lite block header")?;
    let height = u64field(inner, "height")?;
    if height == 0 {
        return Err("zero block height");
    }
    let timestamp_ns = match inner.get("timestamp_nanosec") {
        Some(Value::String(s)) => s.parse::<u64>().map_err(|_| "bad timestamp_nanosec")?,
        Some(_) => return Err("bad timestamp_nanosec type"),
        None => u64field(inner, "timestamp")?,
    };
    if let Some(timestamp_number) = inner.get("timestamp") {
        if timestamp_number.as_u64() != Some(timestamp_ns) {
            return Err("timestamp mismatch");
        }
    }
    let epoch_id = field_hash(inner, "epoch_id")?;
    let next_epoch_id = field_hash(inner, "next_epoch_id")?;
    if epoch_id == next_epoch_id {
        return Err("degenerate epoch identifiers");
    }
    let next_bp_hash = field_hash(inner, "next_bp_hash")?;
    let mut borsh = Vec::with_capacity(208);
    borsh.extend_from_slice(&height.to_le_bytes());
    for field in ["epoch_id", "next_epoch_id", "prev_state_root", "outcome_root"] {
        borsh.extend_from_slice(&field_hash(inner, field)?);
    }
    borsh.extend_from_slice(&timestamp_ns.to_le_bytes());
    for field in ["next_bp_hash", "block_merkle_root"] {
        borsh.extend_from_slice(&field_hash(inner, field)?);
    }
    let inner_hash = sha(&borsh);
    let mut mid = Vec::with_capacity(64);
    mid.extend_from_slice(&inner_hash);
    mid.extend_from_slice(&field_hash(r, "inner_rest_hash")?);
    let mut final_bytes = Vec::with_capacity(64);
    final_bytes.extend_from_slice(&sha(&mid));
    final_bytes.extend_from_slice(&field_hash(r, "prev_block_hash")?);
    let header_hash = sha(&final_bytes);
    let next_block_inner_hash = field_hash(r, "next_block_inner_hash")?;
    let next_bps = match r.get("next_bps") {
        None | Some(Value::Null) => None,
        Some(v) => Some(parse_producers(v)?),
    };
    if let Some(v) = &next_bps {
        if borsh_validator_hash(v)? != next_bp_hash {
            return Err("next_bps header commitment mismatch");
        }
    }
    let signatures = r
        .get("approvals_after_next")
        .and_then(Value::as_array)
        .ok_or("missing approvals")?;
    if signatures.len() > 4096 {
        return Err("unbounded approvals");
    }
    let approvals = signatures
        .iter()
        .map(|value| match value {
            Value::Null => Ok(None),
            Value::String(s) => signature(s).map(Some),
            _ => Err("malformed signature item"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CandidateView {
        height,
        timestamp_ns,
        epoch_id,
        next_epoch_id,
        next_bp_hash,
        header_hash,
        next_block_inner_hash,
        next_bps,
        approvals,
    })
}

/// Result of cryptographic candidate validation **against untrusted initial
/// checkpoint**. Neither flag can become true in this module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UntrustedAdvance {
    pub block_hash: Hash,
    pub height: u64,
    pub epoch_id: Hash,
    pub valid_signatures: usize,
    pub crossed_epoch: bool,
    pub authenticated: bool,
    pub authorized_to_sign: bool,
}

/// Carries verified validator-set commitments forward from a *candidate*
/// checkpoint. The initial light-client block and its validators have NOT
/// been authenticated against a trusted NEAR checkpoint. No method here
/// turns them into trusted or settlement-authorizing evidence.
#[derive(Debug, Clone)]
pub struct UntrustedEpochChain {
    head_hash: Hash,
    head_height: u64,
    head_epoch_id: Hash,
    next_epoch_id: Hash,
    head_timestamp_ns: u64,
    head_next_bp_hash: Hash,
    candidate_epoch_producers: BTreeMap<Hash, Vec<Producer>>,
}

impl CandidateView {
    pub fn height(&self) -> u64 {
        self.height
    }
    pub fn epoch_id(&self) -> Hash {
        self.epoch_id
    }
    pub fn next_epoch_id(&self) -> Hash {
        self.next_epoch_id
    }
}

impl UntrustedEpochChain {
    pub fn head_height(&self) -> u64 {
        self.head_height
    }
    pub fn bootstrap_rpc_candidate(view: &CandidateView) -> Result<Self, &'static str> {
        let next_producers = view
            .next_bps
            .as_ref()
            .ok_or("candidate checkpoint missing next validators")?;
        if borsh_validator_hash(next_producers)? != view.next_bp_hash {
            return Err("candidate checkpoint has uncommitted next validators");
        }
        let mut map = BTreeMap::new();
        map.insert(view.next_epoch_id, next_producers.to_vec());
        Ok(Self {
            head_hash: view.header_hash,
            head_height: view.height,
            head_epoch_id: view.epoch_id,
            next_epoch_id: view.next_epoch_id,
            head_timestamp_ns: view.timestamp_ns,
            head_next_bp_hash: view.next_bp_hash,
            candidate_epoch_producers: map,
        })
    }

    /// Verify actual signatures and 2/3 stake using the validator list
    /// announced in a PRECEDING light-client epoch, never `view.next_bps`.
    /// Transactional: all checks happen before mutating the candidate state.
    pub fn advance_candidate(&mut self, view: &CandidateView) -> Result<UntrustedAdvance, &'static str> {
        if view.height <= self.head_height {
            return Err("stale or replayed candidate height");
        }
        if view.timestamp_ns < self.head_timestamp_ns {
            return Err("candidate timestamp rollback");
        }
        let crossed = view.epoch_id == self.next_epoch_id;
        if view.epoch_id != self.head_epoch_id && !crossed {
            return Err("skipped or unrelated NEAR epoch");
        }
        if crossed && view.next_bps.is_none() {
            return Err("epoch transition omitted next_bps");
        }
        if !crossed && (view.next_epoch_id != self.next_epoch_id || view.next_bp_hash != self.head_next_bp_hash) {
            return Err("intra-epoch next-epoch commitment changed");
        }
        if crossed && view.next_epoch_id == self.head_epoch_id {
            return Err("next epoch points back to previous epoch");
        }
        let producers = self
            .candidate_epoch_producers
            .get(&view.epoch_id)
            .ok_or("no preceding-epoch validator commitment for candidate epoch")?;
        let next_hash = reconstruct_next_block_hash(view.next_block_inner_hash, view.header_hash);
        let message = approval_message(next_hash, view.height)?;
        if view.approvals.len() > producers.len() {
            return Err("surplus approval slots");
        }
        let validators = producers.iter().map(|p| p.validator.clone()).collect::<Vec<_>>();
        verify_approvals(&validators, &view.approvals, &message)?;
        if let Some(next_producers) = &view.next_bps {
            if borsh_validator_hash(next_producers)? != view.next_bp_hash {
                return Err("invalid next-epoch validator set commitment");
            }
            if let Some(existing) = self.candidate_epoch_producers.get(&view.next_epoch_id) {
                if existing != next_producers {
                    return Err("conflicting next-epoch validator set");
                }
            }
        }
        // State transition occurs only after all checks have passed.
        if let Some(next_producers) = &view.next_bps {
            self.candidate_epoch_producers
                .insert(view.next_epoch_id, next_producers.clone());
        }
        self.head_hash = view.header_hash;
        self.head_height = view.height;
        self.head_epoch_id = view.epoch_id;
        self.next_epoch_id = view.next_epoch_id;
        self.head_timestamp_ns = view.timestamp_ns;
        self.head_next_bp_hash = view.next_bp_hash;
        Ok(UntrustedAdvance {
            block_hash: view.header_hash,
            height: view.height,
            epoch_id: view.epoch_id,
            valid_signatures: view.approvals.iter().filter(|x| x.is_some()).count(),
            crossed_epoch: crossed,
            authenticated: false,
            authorized_to_sign: false,
        })
    }
}
