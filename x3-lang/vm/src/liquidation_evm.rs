//! Native EVM preparation and an explicitly local-only signing/submission proof.
//!
//! RPC and fee valuation are trusted host inputs. Exact executor simulation and
//! a successful nested Morpho call are required before a request is prepared.
//! Public chains remain unsigned: this module does not establish their complete
//! L1/operator fee accounting, latency, inclusion, or production key isolation.

use std::{
    collections::BTreeSet,
    io::Read,
    net::IpAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::eip2718::Encodable2718;
use alloy_primitives::{keccak256, Address, Bytes, Signature, TxKind, U256};
use reqwest::{blocking::Client, redirect::Policy, Url};
use secp256k1::{Message, PublicKey, Secp256k1, SecretKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use x3_lang_common::liquidation::LiquidationPolicy;
use x3_lang_common::{decode_capability_payload, CapabilityPayload};

use crate::liquidation::{
    authorize, encode_market, hex_bytes, hex_string, AuthorizedLiquidation, Batch, EffectReceipt, EffectStatus,
    LiquidationHost,
};

const MAX_RPC_RESPONSE: u64 = 8 * 1024 * 1024;

/// In-memory local-test credential supplied explicitly by the embedding host.
/// Entropy generation belongs outside the deterministic VM source tree. There
/// is no file/string loading, serialization, Debug or persistence API. Curve
/// operations use libsecp256k1; Alloy encodes transactions and public signatures.
#[derive(Clone)]
pub struct EphemeralLocalSigner {
    secret: SecretKey,
    address: Address,
}

impl EphemeralLocalSigner {
    pub fn from_declared_secret(secret: SecretKey) -> Self {
        let context = Secp256k1::new();
        let public = PublicKey::from_secret_key(&context, &secret).serialize_uncompressed();
        let digest = keccak256(&public[1..]);
        Self {
            secret,
            address: Address::from_slice(&digest[12..]),
        }
    }
    pub fn address(&self) -> Address {
        self.address
    }
    fn sign_hash(&self, digest: &[u8; 32]) -> Result<Signature, String> {
        let message = Message::from_digest_slice(digest).map_err(|_| "X3_EVM_SIGNING_DIGEST")?;
        let (recovery, compact) = Secp256k1::new()
            .sign_ecdsa_recoverable(&message, &self.secret)
            .serialize_compact();
        let parity = match recovery.to_i32() {
            0 => false,
            1 => true,
            _ => return Err("X3_EVM_SIGNATURE_PARITY".into()),
        };
        Ok(Signature::new(
            U256::from_be_slice(&compact[..32]),
            U256::from_be_slice(&compact[32..]),
            parity,
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    /// Verify the exact request, without producing a signature or sending it.
    PrepareOnly,
    /// Ephemeral local-node proof only: both URLs must be loopback, chain 31337.
    LocalBroadcast,
}

/// Deliberately not Debug: provider URLs may contain credentials.
pub struct EvmTransportConfig {
    pub read_url: String,
    pub submit_url: Option<String>,
    pub mode: TransportMode,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreparedRequest {
    pub request_hash: [u8; 32],
    pub chain_id: u64,
    pub nonce: u64,
    pub executor: String,
    pub calldata: Vec<u8>,
    pub block_hash: String,
    pub gas_limit: u64,
    pub max_fee_per_gas: u128,
    pub max_priority_fee_per_gas: u128,
    pub simulated_gas_used: u64,
    pub simulated_seized: u128,
    pub simulated_repaid: u128,
    pub simulated_profit: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastAttempt {
    pub chain_id: u64,
    pub nonce: u64,
    /// Computed locally before the single submission attempt.
    pub transaction_hash: String,
    /// A matching RPC hash is acceptance evidence, never an inclusion proof.
    pub acknowledged: bool,
}

/// Owns its nonce reservations and cannot be cloned. A timed-out or rejected
/// submission is never retried, and its nonce stays reserved for this instance.
/// Restart recovery requires an external durable nonce journal before any
/// production signing path could be enabled.
pub struct EvmTransport {
    client: Client,
    config: EvmTransportConfig,
    artifact: [u8; 32],
    approved_policy: LiquidationPolicy,
    batch: Batch,
    signer: Option<EphemeralLocalSigner>,
    kill_switch: Arc<AtomicBool>,
    reserved_nonces: BTreeSet<(u64, u64)>,
    prepared: Option<PreparedRequest>,
    last_attempt: Option<BroadcastAttempt>,
}

impl EvmTransport {
    pub fn new(
        config: EvmTransportConfig,
        approved_artifact: &[u8],
        artifact: [u8; 32],
        batch: Batch,
        signer: Option<EphemeralLocalSigner>,
        kill_switch: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        validate_url(&config.read_url, config.mode == TransportMode::LocalBroadcast)?;
        if config.timeout.is_zero() || config.timeout > Duration::from_secs(30) {
            return Err("X3_EVM_TIMEOUT_BOUND".into());
        }
        if artifact == [0; 32] {
            return Err("X3_EVM_UNBOUND_ARTIFACT".into());
        }
        if config.mode == TransportMode::PrepareOnly && signer.is_some() {
            return Err("X3_EVM_UNSIGNED_MODE_REFUSES_SIGNER".into());
        }
        if config.mode == TransportMode::LocalBroadcast {
            validate_url(config.submit_url.as_deref().ok_or("X3_EVM_SUBMIT_URL")?, true)?;
            if batch.snapshot.chain_id != 31337 {
                return Err("X3_EVM_LOCAL_CHAIN_ONLY".into());
            }
            if signer.is_none() {
                return Err("X3_EVM_LOCAL_SIGNER_REQUIRED".into());
            }
        }
        let approved_policy = policy_from_artifact(approved_artifact, artifact)?;
        let client = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(config.timeout)
            .redirect(Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| "X3_EVM_HTTP_CLIENT".to_owned())?;
        Ok(Self {
            client,
            config,
            artifact,
            approved_policy,
            batch,
            signer,
            kill_switch,
            reserved_nonces: BTreeSet::new(),
            prepared: None,
            last_attempt: None,
        })
    }

    pub fn prepared_request(&self) -> Option<&PreparedRequest> {
        self.prepared.as_ref()
    }
    pub fn last_attempt(&self) -> Option<&BroadcastAttempt> {
        self.last_attempt.as_ref()
    }
    pub fn nonce_reserved(&self, chain_id: u64, nonce: u64) -> bool {
        self.reserved_nonces.contains(&(chain_id, nonce))
    }

    fn armed(&self) -> Result<(), String> {
        if self.kill_switch.load(Ordering::SeqCst) {
            Err("X3_EVM_KILLED".into())
        } else {
            Ok(())
        }
    }

    fn rpc(&self, submit: bool, method: &str, params: Value) -> Result<Value, String> {
        let url = if submit {
            self.config.submit_url.as_deref().ok_or("X3_EVM_SUBMIT_URL")?
        } else {
            &self.config.read_url
        };
        // No retries, redirects, or provider-controlled error strings. In
        // particular, an HTTP failure may occur after the upstream accepted a tx.
        let response = self
            .client
            .post(url)
            .json(&json!({"jsonrpc":"2.0", "id":1, "method":method, "params":params}))
            .send()
            .map_err(|_| format!("X3_EVM_RPC_TRANSPORT {method}"))?;
        if !response.status().is_success() {
            return Err(format!("X3_EVM_RPC_HTTP {method} {}", response.status().as_u16()));
        }
        let mut bytes = Vec::new();
        response
            .take(MAX_RPC_RESPONSE + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| format!("X3_EVM_RPC_BODY {method}"))?;
        if bytes.len() as u64 > MAX_RPC_RESPONSE {
            return Err("X3_EVM_RPC_SIZE".into());
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "X3_EVM_RPC_JSON".to_owned())?;
        if value.get("jsonrpc") != Some(&json!("2.0"))
            || value.get("id") != Some(&json!(1))
            || value.get("error").is_some()
            || value.get("result").is_none()
        {
            return Err(format!("X3_EVM_RPC_RESPONSE {method}"));
        }
        Ok(value["result"].clone())
    }

    fn context(&self, a: &AuthorizedLiquidation) -> Result<(), String> {
        self.context_at(a, false)
    }

    fn context_at(&self, a: &AuthorizedLiquidation, submit: bool) -> Result<(), String> {
        let chain = quantity(&self.rpc(submit, "eth_chainId", json!([]))?)?;
        if chain != u128::from(a.policy.chain_id) {
            return Err("X3_EVM_CHAIN".into());
        }
        let block = self.rpc(submit, "eth_getBlockByNumber", json!(["latest", false]))?;
        if quantity(&block["number"])? != u128::from(a.snapshot.block_number)
            || block["hash"].as_str() != Some(&a.snapshot.block_hash)
            || a.snapshot.block_number > a.policy.expires_at_block
        {
            return Err("X3_EVM_HEAD_CHANGED".into());
        }
        if quantity(&block["baseFeePerGas"])? > a.candidate.max_fee_per_gas {
            return Err("X3_EVM_BASE_FEE".into());
        }
        let nonce = self.rpc(submit, "eth_getTransactionCount", json!([a.policy.keeper, "pending"]))?;
        if quantity(&nonce)? != u128::from(a.snapshot.nonce) {
            return Err("X3_EVM_NONCE_CHANGED".into());
        }
        Ok(())
    }

    fn preflight(&self, a: &AuthorizedLiquidation) -> Result<PreparedRequest, String> {
        self.armed()?;
        self.context(a)?;
        let block = json!({"blockHash": a.snapshot.block_hash, "requireCanonical": true});
        let code = data(&self.rpc(false, "eth_getCode", json!([a.policy.executor, block]))?)?;
        if code.is_empty() || hex_string(keccak256(&code).as_slice()) != a.policy.executor_code_hash {
            return Err("X3_EVM_EXECUTOR_CODE_HASH".into());
        }
        for (signature, expected) in [
            ("owner()", &a.policy.owner),
            ("keeper()", &a.policy.keeper),
            ("MORPHO()", &a.policy.morpho),
        ] {
            let output = self.getter(a, &block, &selector(signature))?;
            if output.len() != 32 || output[..12] != [0; 12] || output[12..] != hex_bytes::<20>(expected)? {
                return Err(format!("X3_EVM_IDENTITY {signature}"));
            }
        }
        let mut router_getter = selector("isRouter(address)");
        router_getter.extend([0; 12]);
        router_getter.extend(hex_bytes::<20>(&a.candidate.router)?);
        if decode_word(&self.getter(a, &block, &router_getter)?)? != 1 {
            return Err("X3_EVM_ROUTER_NOT_ALLOWED".into());
        }
        let cost = a.candidate.cost.as_ref().ok_or("X3_EVM_UNKNOWN_COST")?;
        let balance = quantity(&self.rpc(false, "eth_getBalance", json!([a.policy.keeper, block]))?)?;
        if balance < cost.native_wei {
            return Err("X3_EVM_NATIVE_BALANCE".into());
        }
        let call = json!({
            "from":a.policy.keeper,"to":a.policy.executor,"data":hex_string(&a.calldata),
            "gas":format!("0x{:x}",a.candidate.gas_limit),"value":"0x0",
            "maxFeePerGas":format!("0x{:x}",a.candidate.max_fee_per_gas),
            "maxPriorityFeePerGas":format!("0x{:x}",a.candidate.max_priority_fee_per_gas),
        });
        let output = data(&self.rpc(false, "eth_call", json!([call, block]))?)?;
        let profit = decode_word(&output)?;
        let trace = self.rpc(false, "debug_traceCall", json!([call, block, {"tracer":"callTracer"}]))?;
        if trace.get("error").is_some()
            || trace["type"] != "CALL"
            || !same_address(&trace["from"], &a.policy.keeper)
            || !same_address(&trace["to"], &a.policy.executor)
            || data(&trace["input"])? != a.calldata
            || data(&trace["output"])? != output
            || quantity(&trace["gasUsed"])? > u128::from(a.candidate.gas_limit)
        {
            return Err("X3_EVM_EXECUTOR_TRACE".into());
        }
        let mut morpho_calls = Vec::new();
        find_morpho_calls(&trace, &a.policy.executor, &a.policy.morpho, &mut morpho_calls, 0)?;
        if morpho_calls.len() != 1 {
            return Err("X3_EVM_MORPHO_TRACE_COUNT".into());
        }
        let morpho = morpho_calls[0];
        let mut expected =
            selector("liquidate((address,address,address,address,uint256),address,uint256,uint256,bytes)");
        expected.extend(encode_market(&a.candidate.market)?);
        expected.extend([0; 12]);
        expected.extend(hex_bytes::<20>(&a.candidate.borrower)?);
        expected.extend(U256::from(a.candidate.seized_assets).to_be_bytes::<32>());
        expected.extend([0; 32]); // repaidShares = 0
        let input = data(&morpho["input"])?;
        if morpho["type"] != "CALL" || morpho.get("error").is_some() || !input.starts_with(&expected) {
            return Err("X3_EVM_MORPHO_TRACE_INPUT".into());
        }
        let output = data(&morpho["output"])?;
        if output.len() != 64 {
            return Err("X3_EVM_MORPHO_TRACE_OUTPUT".into());
        }
        let seized = decode_word(&output[..32])?;
        let repaid = decode_word(&output[32..])?;
        if seized != a.candidate.seized_assets
            || repaid != a.candidate.repay_assets
            || repaid < a.policy.min_repay
            || repaid > a.policy.max_repay
        {
            return Err("X3_EVM_REPAY_OR_SEIZE_MISMATCH".into());
        }
        let floor = a
            .policy
            .min_profit
            .checked_add(cost.loan_units)
            .ok_or("X3_EVM_COST_OVERFLOW")?;
        if profit < a.candidate.gross_profit || profit < floor {
            return Err("X3_EVM_SIMULATED_PROFIT".into());
        }
        self.context(a)?;
        self.armed()?;
        let tx = unsigned(a)?;
        Ok(PreparedRequest {
            request_hash: tx.signature_hash().into(),
            chain_id: a.policy.chain_id,
            nonce: a.snapshot.nonce,
            executor: a.policy.executor.clone(),
            calldata: a.calldata.clone(),
            block_hash: a.snapshot.block_hash.clone(),
            gas_limit: a.candidate.gas_limit,
            max_fee_per_gas: a.candidate.max_fee_per_gas,
            max_priority_fee_per_gas: a.candidate.max_priority_fee_per_gas,
            simulated_gas_used: quantity(&trace["gasUsed"])? as u64,
            simulated_seized: seized,
            simulated_repaid: repaid,
            simulated_profit: profit,
        })
    }

    fn getter(&self, a: &AuthorizedLiquidation, block: &Value, input: &[u8]) -> Result<Vec<u8>, String> {
        data(&self.rpc(
            false,
            "eth_call",
            json!([{"to":a.policy.executor,"data":hex_string(input)}, block]),
        )?)
    }
}

impl LiquidationHost for EvmTransport {
    fn artifact_hash(&self) -> [u8; 32] {
        self.artifact
    }
    fn candidates(&mut self, policy: &LiquidationPolicy) -> Result<Batch, String> {
        self.armed()?;
        if *policy != self.approved_policy {
            return Err("X3_EVM_POLICY_NOT_APPROVED".into());
        }
        Ok(self.batch.clone())
    }
    fn execute(&mut self, a: &AuthorizedLiquidation) -> Result<EffectReceipt, String> {
        self.prepared = None;
        self.armed()?;
        // Recompute the complete authorization at the signing boundary. Public
        // callers cannot modify calldata, a winner, or a compiled bound in transit.
        if a.policy != self.approved_policy || *a != authorize(&self.approved_policy, self.artifact, &self.batch)? {
            return Err("X3_EVM_AUTHORIZATION_CHANGED".into());
        }
        if self.nonce_reserved(a.policy.chain_id, a.snapshot.nonce) {
            return Err("X3_EVM_NONCE_RESERVED_NO_RESEND".into());
        }
        let request = self.preflight(a)?;
        let request_hash = request.request_hash;
        self.prepared = Some(request);
        if self.config.mode == TransportMode::PrepareOnly {
            return Ok(EffectReceipt {
                request_hash,
                transaction_hash: None,
                status: EffectStatus::Prepared,
            });
        }
        if a.policy.chain_id != 31337 {
            return Err("X3_EVM_LOCAL_CHAIN_ONLY".into());
        }
        let signer = self.signer.as_ref().ok_or("X3_EVM_LOCAL_SIGNER_REQUIRED")?;
        if signer.address() != Address::from(hex_bytes::<20>(&a.policy.keeper)?) {
            return Err("X3_EVM_SIGNER_IDENTITY".into());
        }
        // Validate the submission endpoint independently; a separate URL must
        // not silently route a chain-bound signature onto a different chain.
        self.context_at(a, true)?;
        self.context(a)?;
        let tx = unsigned(a)?;
        self.armed()?; // immediately before native signing
        let signature = signer.sign_hash(tx.signature_hash().as_ref())?;
        let raw = TxEnvelope::Eip1559(tx.into_signed(signature)).encoded_2718();
        let hash = hex_string(keccak256(&raw).as_slice());
        self.armed()?;
        // Reserve BEFORE attempting the POST. An uncertain response is not
        // permission to reuse or replace this nonce.
        self.reserved_nonces.insert((a.policy.chain_id, a.snapshot.nonce));
        self.last_attempt = Some(BroadcastAttempt {
            chain_id: a.policy.chain_id,
            nonce: a.snapshot.nonce,
            transaction_hash: hash.clone(),
            acknowledged: false,
        });
        let result = self
            .rpc(true, "eth_sendRawTransaction", json!([hex_string(&raw)]))
            .map_err(|error| format!("{error}; X3_EVM_BROADCAST_UNCERTAIN_NONCE_RESERVED"))?;
        if result.as_str() != Some(&hash) {
            return Err("X3_EVM_BROADCAST_HASH_MISMATCH_NONCE_RESERVED".into());
        }
        if let Some(attempt) = &mut self.last_attempt {
            attempt.acknowledged = true;
        }
        Ok(EffectReceipt {
            request_hash,
            transaction_hash: Some(hash),
            status: EffectStatus::Submitted,
        })
    }
}

fn policy_from_artifact(bytes: &[u8], expected: [u8; 32]) -> Result<LiquidationPolicy, String> {
    use crate::{
        spec::opcodes::{HALT, NATIVE_LIQUIDATION},
        InstructionStream,
    };
    if bytes.len() > 1_048_576 || crate::liquidation::artifact_hash(bytes) != expected {
        return Err("X3_EVM_ARTIFACT_NOT_APPROVED".into());
    }
    let boundaries = crate::verifier::verify(&InstructionStream::new(bytes.to_vec()))
        .map_err(|_| "X3_EVM_ARTIFACT_INVALID".to_owned())?;
    if boundaries.len() != 2 || !boundaries.iter().any(|pc| bytes[*pc] == HALT) {
        return Err("X3_EVM_ARTIFACT_STANDALONE".into());
    }
    let pc = boundaries
        .into_iter()
        .find(|pc| bytes[*pc] == NATIVE_LIQUIDATION)
        .ok_or("X3_EVM_ARTIFACT_NATIVE_REQUIRED")?;
    let length = u16::from_le_bytes(
        bytes[pc + 1..pc + 3]
            .try_into()
            .map_err(|_| "X3_EVM_ARTIFACT_PAYLOAD")?,
    ) as usize;
    let payload = bytes.get(pc + 3..pc + 3 + length).ok_or("X3_EVM_ARTIFACT_PAYLOAD")?;
    match decode_capability_payload(NATIVE_LIQUIDATION, payload).map_err(|_| "X3_EVM_ARTIFACT_POLICY")? {
        CapabilityPayload::NativeLiquidation { policy } => Ok(policy),
        _ => Err("X3_EVM_ARTIFACT_POLICY".into()),
    }
}

fn unsigned(a: &AuthorizedLiquidation) -> Result<TxEip1559, String> {
    Ok(TxEip1559 {
        chain_id: a.policy.chain_id,
        nonce: a.snapshot.nonce,
        gas_limit: a.candidate.gas_limit,
        max_fee_per_gas: a.candidate.max_fee_per_gas,
        max_priority_fee_per_gas: a.candidate.max_priority_fee_per_gas,
        to: TxKind::Call(Address::from(hex_bytes::<20>(&a.policy.executor)?)),
        value: U256::ZERO,
        access_list: Default::default(),
        input: Bytes::copy_from_slice(&a.calldata),
    })
}

fn validate_url(raw: &str, loopback: bool) -> Result<(), String> {
    let url = Url::parse(raw).map_err(|_| "X3_EVM_RPC_URL".to_owned())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || url.fragment().is_some() {
        return Err("X3_EVM_RPC_URL".into());
    }
    if loopback {
        let host = url.host_str().unwrap().trim_matches(['[', ']']);
        if !(host == "localhost" || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback()))
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err("X3_EVM_LOCAL_BROADCAST_REQUIRES_loopback".into());
        }
    }
    Ok(())
}

fn selector(signature: &str) -> Vec<u8> {
    keccak256(signature.as_bytes())[..4].to_vec()
}
fn same_address(value: &Value, expected: &str) -> bool {
    value
        .as_str()
        .is_some_and(|address| address.eq_ignore_ascii_case(expected))
}
fn quantity(value: &Value) -> Result<u128, String> {
    let raw = value
        .as_str()
        .and_then(|s| s.strip_prefix("0x"))
        .ok_or("X3_EVM_QUANTITY")?;
    if raw.is_empty() || raw.len() > 32 || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("X3_EVM_QUANTITY".into());
    }
    u128::from_str_radix(raw, 16).map_err(|_| "X3_EVM_QUANTITY".into())
}
fn data(value: &Value) -> Result<Vec<u8>, String> {
    let raw = value.as_str().and_then(|s| s.strip_prefix("0x")).ok_or("X3_EVM_DATA")?;
    if raw.len() % 2 != 0 || raw.len() > MAX_RPC_RESPONSE as usize {
        return Err("X3_EVM_DATA".into());
    }
    raw.as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let nibble = |b: u8| (b as char).to_digit(16).ok_or("X3_EVM_DATA".to_owned());
            Ok((nibble(pair[0])? * 16 + nibble(pair[1])?) as u8)
        })
        .collect()
}
fn decode_word(bytes: &[u8]) -> Result<u128, String> {
    if bytes.len() != 32 || bytes[..16] != [0; 16] {
        return Err("X3_EVM_ABI_WORD".into());
    }
    Ok(u128::from_be_bytes(
        bytes[16..].try_into().map_err(|_| "X3_EVM_ABI_WORD")?,
    ))
}
fn find_morpho_calls<'a>(
    frame: &'a Value,
    executor: &str,
    morpho: &str,
    out: &mut Vec<&'a Value>,
    depth: usize,
) -> Result<(), String> {
    if depth > 64 {
        return Err("X3_EVM_TRACE_DEPTH".into());
    }
    if same_address(&frame["from"], executor) && same_address(&frame["to"], morpho) {
        let input = data(&frame["input"])?;
        if input.starts_with(&selector(
            "liquidate((address,address,address,address,uint256),address,uint256,uint256,bytes)",
        )) {
            out.push(frame);
        }
    }
    if let Some(calls) = frame.get("calls") {
        for child in calls.as_array().ok_or("X3_EVM_TRACE_CALLS")? {
            find_morpho_calls(child, executor, morpho, out, depth + 1)?;
        }
    }
    Ok(())
}
