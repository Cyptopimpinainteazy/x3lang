//! Genuine NEAR receipt outcome/Merkle inspection against an UNTRUSTED head root.
//!
//! A valid Merkle path NEVER authenticates RPC-supplied chain state. This module
//! contains no trusted checkpoint, validator root, certificate or signer API.
//! Results are read-only observations and must not authorize any settlement.
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NearReceiptContext {
    pub receipt_id: String,
    pub receiver_id: String,
    pub light_client_head: String,
    pub untrusted_head_block_merkle_root: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NearReceiptObservation {
    pub receipt_id: String,
    pub receiver_id: String,
    pub block_hash: String,
    pub shard_outcome_root: String,
    pub block_outcome_root: String,
    pub outcome_merkle_matches: bool,
    pub block_merkle_matches: bool,
    pub authenticated: bool,
    pub authorized_to_sign: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearProofError(pub &'static str);

impl std::fmt::Display for NearProofError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for NearProofError {}

type ProofResult<T> = Result<T, NearProofError>;

fn sha(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn field<'a>(node: &'a Value, name: &str) -> ProofResult<&'a Value> {
    node.get(name).ok_or(NearProofError("missing NEAR proof field"))
}
fn string<'a>(node: &'a Value, name: &str) -> ProofResult<&'a str> {
    field(node, name)?
        .as_str()
        .ok_or(NearProofError("invalid NEAR string field"))
}
fn uint64(node: &Value, name: &str) -> ProofResult<u64> {
    field(node, name)?
        .as_u64()
        .ok_or(NearProofError("invalid NEAR u64 field"))
}
fn hash(text: &str) -> ProofResult<[u8; 32]> {
    if !(32..=48).contains(&text.len()) {
        return Err(NearProofError("invalid NEAR hash length"));
    }
    let raw = bs58::decode(text)
        .into_vec()
        .map_err(|_| NearProofError("malformed NEAR base58"))?;
    let array: [u8; 32] = raw.try_into().map_err(|_| NearProofError("hash is not 32 bytes"))?;
    if bs58::encode(array).into_string() != text {
        return Err(NearProofError("non-canonical NEAR hash"));
    }
    Ok(array)
}
fn hash_field(node: &Value, name: &str) -> ProofResult<[u8; 32]> {
    hash(string(node, name)?)
}

fn merkle(start: [u8; 32], path: &Value) -> ProofResult<[u8; 32]> {
    let path = path.as_array().ok_or(NearProofError("malformed NEAR Merkle path"))?;
    if path.len() > 128 {
        return Err(NearProofError("NEAR Merkle depth exceeds limit"));
    }
    let mut current = start;
    for step in path {
        let node = step.as_object().ok_or(NearProofError("malformed NEAR Merkle node"))?;
        if node.len() != 2 || !node.contains_key("hash") || !node.contains_key("direction") {
            return Err(NearProofError("unknown Merkle node fields"));
        }
        let sibling = hash_field(step, "hash")?;
        let mut data = [0u8; 64];
        match string(step, "direction")? {
            "Left" => {
                data[..32].copy_from_slice(&sibling);
                data[32..].copy_from_slice(&current);
            }
            "Right" => {
                data[..32].copy_from_slice(&current);
                data[32..].copy_from_slice(&sibling);
            }
            _ => return Err(NearProofError("invalid Merkle direction")),
        }
        current = sha(&data);
    }
    Ok(current)
}

fn lite_header_hash(node: &Value) -> ProofResult<[u8; 32]> {
    let inner = field(node, "inner_lite")?;
    let height = uint64(inner, "height")?;
    if height == 0 {
        return Err(NearProofError("zero NEAR header height"));
    }
    let timestamp = uint64(inner, "timestamp")?;
    if let Some(other) = inner.get("timestamp_nanosec") {
        if other.as_str().and_then(|v| v.parse::<u64>().ok()) != Some(timestamp) {
            return Err(NearProofError("inconsistent header timestamps"));
        }
    }
    let mut borsh = Vec::with_capacity(8 * 2 + 32 * 6);
    borsh.extend_from_slice(&height.to_le_bytes());
    for name in ["epoch_id", "next_epoch_id", "prev_state_root", "outcome_root"] {
        borsh.extend_from_slice(&hash_field(inner, name)?);
    }
    borsh.extend_from_slice(&timestamp.to_le_bytes());
    for name in ["next_bp_hash", "block_merkle_root"] {
        borsh.extend_from_slice(&hash_field(inner, name)?);
    }
    let inner_hash = sha(&borsh);
    let mut joined = [0u8; 64];
    joined[..32].copy_from_slice(&inner_hash);
    joined[32..].copy_from_slice(&hash_field(node, "inner_rest_hash")?);
    let inner_combined = sha(&joined);
    joined[..32].copy_from_slice(&inner_combined);
    joined[32..].copy_from_slice(&hash_field(node, "prev_block_hash")?);
    Ok(sha(&joined))
}

fn u32_len(len: usize) -> ProofResult<[u8; 4]> {
    Ok(u32::try_from(len)
        .map_err(|_| NearProofError("oversized NEAR Borsh collection"))?
        .to_le_bytes())
}

fn outcome_leaf(proof: &Value) -> ProofResult<[u8; 32]> {
    let outcome = field(proof, "outcome")?;
    let receipts = field(outcome, "receipt_ids")?
        .as_array()
        .ok_or(NearProofError("invalid NEAR receipts"))?;
    let logs = field(outcome, "logs")?
        .as_array()
        .ok_or(NearProofError("invalid NEAR execution logs"))?;
    if receipts.len() > 4096 || logs.len() > 4096 {
        return Err(NearProofError("unbounded NEAR outcome"));
    }
    let mut partial = Vec::new();
    partial.extend_from_slice(&u32_len(receipts.len())?);
    for id in receipts {
        partial.extend_from_slice(&hash(id.as_str().ok_or(NearProofError("invalid receipt hash"))?)?);
    }
    partial.extend_from_slice(&uint64(outcome, "gas_burnt")?.to_le_bytes());
    let tokens = string(outcome, "tokens_burnt")?;
    if tokens.len() > 39 || (tokens.len() > 1 && tokens.starts_with('0')) || !tokens.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(NearProofError("invalid Borsh tokens burnt"));
    }
    partial.extend_from_slice(
        &tokens
            .parse::<u128>()
            .map_err(|_| NearProofError("Borsh token count overflow"))?
            .to_le_bytes(),
    );
    let executor = string(outcome, "executor_id")?.as_bytes();
    if executor.len() > 64 {
        return Err(NearProofError("invalid NEAR executor"));
    }
    partial.extend_from_slice(&u32_len(executor.len())?);
    partial.extend_from_slice(executor);
    let status = field(outcome, "status")?
        .as_object()
        .ok_or(NearProofError("invalid NEAR outcome status"))?;
    if status.len() != 1 {
        return Err(NearProofError("unknown NEAR execution status"));
    }
    let encoded = status
        .get("SuccessValue")
        .and_then(Value::as_str)
        .ok_or(NearProofError("NEAR execution did not succeed"))?;
    if encoded.len() > 1_340_000 {
        return Err(NearProofError("oversized NEAR return value"));
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|_| NearProofError("invalid NEAR return value encoding"))?;
    if decoded.len() > 1_000_000 {
        return Err(NearProofError("oversized NEAR return value"));
    }
    partial.push(2); // Borsh ExecutionStatus::SuccessValue variant
    partial.extend_from_slice(&u32_len(decoded.len())?);
    partial.extend_from_slice(&decoded);
    let mut hashes = vec![hash_field(proof, "id")?, sha(&partial)];
    for log in logs {
        let value = log.as_str().ok_or(NearProofError("invalid NEAR log"))?.as_bytes();
        if value.len() > 1_000_000 {
            return Err(NearProofError("oversized NEAR execution log"));
        }
        hashes.push(sha(value));
    }
    let mut encoded = Vec::with_capacity(4 + 32 * hashes.len());
    encoded.extend_from_slice(&u32_len(hashes.len())?);
    for h in hashes {
        encoded.extend_from_slice(&h);
    }
    Ok(sha(&encoded))
}

/// Verify Borsh execution outcome and both Merkle paths against *caller supplied*
/// historical RPC head root. Not a consensus verifier. No setter for authority.
/// Never feed this result into an execution or signing eligibility decision.
pub fn inspect_receipt_untrusted(raw_rpc: &[u8], ctx: &NearReceiptContext) -> ProofResult<NearReceiptObservation> {
    if raw_rpc.is_empty() || raw_rpc.len() > 1_000_000 {
        return Err(NearProofError("NEAR proof response size invalid"));
    }
    let rpc: Value = serde_json::from_slice(raw_rpc).map_err(|_| NearProofError("invalid NEAR proof JSON"))?;
    if string(&rpc, "jsonrpc")? != "2.0"
        || string(&rpc, "id")? != "x3-near-light-proof"
        || rpc.get("error").is_some_and(|x| !x.is_null())
    {
        return Err(NearProofError("unexpected NEAR proof RPC envelope"));
    }
    let _head_id = hash(&ctx.light_client_head)?;
    let expected_root = hash(&ctx.untrusted_head_block_merkle_root)?;
    let _id = hash(&ctx.receipt_id)?;
    if ctx.receiver_id.is_empty()
        || ctx.receiver_id.len() > 64
        || !ctx
            .receiver_id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(NearProofError("invalid NEAR expected receiver"));
    }
    let data = field(&rpc, "result")?;
    let outcome_proof = field(data, "outcome_proof")?;
    if string(outcome_proof, "id")? != ctx.receipt_id
        || string(field(outcome_proof, "outcome")?, "executor_id")? != ctx.receiver_id
    {
        return Err(NearProofError("NEAR receipt ID or executor mismatch"));
    }
    let header = field(data, "block_header_lite")?;
    let block_hash = lite_header_hash(header)?;
    if block_hash != hash_field(outcome_proof, "block_hash")? {
        return Err(NearProofError("NEAR outcome block hash disagrees with lite header"));
    }
    let shard_root = merkle(outcome_leaf(outcome_proof)?, field(outcome_proof, "proof")?)?;
    let block_outcome_root = merkle(sha(&shard_root), field(data, "outcome_root_proof")?)?;
    if block_outcome_root != hash_field(field(header, "inner_lite")?, "outcome_root")? {
        return Err(NearProofError("NEAR execution outcome not included in block header"));
    }
    if merkle(block_hash, field(data, "block_proof")?)? != expected_root {
        return Err(NearProofError(
            "NEAR block not included in untrusted supplied head root",
        ));
    }
    Ok(NearReceiptObservation {
        receipt_id: ctx.receipt_id.clone(),
        receiver_id: ctx.receiver_id.clone(),
        block_hash: bs58::encode(block_hash).into_string(),
        shard_outcome_root: bs58::encode(shard_root).into_string(),
        block_outcome_root: bs58::encode(block_outcome_root).into_string(),
        outcome_merkle_matches: true,
        block_merkle_matches: true,
        authenticated: false,
        authorized_to_sign: false,
    })
}
