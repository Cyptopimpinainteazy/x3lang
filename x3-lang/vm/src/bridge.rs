//! Bridge adapter abstraction for cross-chain calls.
//! Production adapters must verify finality/proofs; dry-run is explicit.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sha3::Keccak256;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::error::Error;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub type BridgeResult = Result<Vec<u8>, Box<dyn Error>>;

const ERC20_TRANSFER_TOPIC: &str = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef";
const EVM_HEADER_PROOF_TYPE: &str = "ethereum-header-rlp-v1";
const EVM_RECEIPT_PROOF_TYPE: &str = "ethereum-receipt-trie-v1";
const SVM_BANK_PROOF_TYPE: &str = "solana-bank-hash-v1";
const SVM_EPOCH_PROOF_TYPE: &str = "solana-epoch-stake-v1";
const SVM_TRANSACTION_PROOF_TYPE: &str = "solana-transaction-proof-v1";
const SVM_EPOCH_TRANSITION_PROOF_TYPE: &str = "solana-epoch-transition-v1";
const SVM_STAKE_ACCOUNT_DATA_TYPE: &str = "solana-stake-account-v1";
const SVM_STAKE_ACCOUNT_LEGACY_FIXTURE_TYPE: &str = "solana-stake-account-fixture-v1";
const DEFAULT_SVM_STAKE_THRESHOLD_BPS: u64 = 6_667;

#[derive(Debug)]
pub struct BridgeError {
    pub code: &'static str,
    pub message: String,
}
impl fmt::Display for BridgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl Error for BridgeError {}

fn decode_hex(hex: &str) -> Result<Vec<u8>, BridgeError> {
    let hex = hex.strip_prefix("0x").unwrap_or(hex);
    if hex.len() % 2 != 0 {
        return Err(BridgeError {
            code: "X3_HEX_DECODE_FAILED",
            message: "hex string has odd length".into(),
        });
    }
    (0..hex.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&hex[i..i + 2], 16).map_err(|err| BridgeError {
                code: "X3_HEX_DECODE_FAILED",
                message: format!("invalid hex at position {i}: {err}"),
            })
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeTransferRequest {
    pub via: String,
    pub from_chain: String,
    pub from_asset: String,
    pub to_chain: String,
    pub to_asset: String,
    pub amount: u128,
    pub receiver: Vec<u8>,
    pub source_finality_proof: Vec<u8>,
    pub transfer_proof: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettlementReceipt {
    pub receipt_id: String,
    pub via: String,
    pub from_chain: String,
    pub from_asset: String,
    pub to_chain: String,
    pub to_asset: String,
    pub amount: u128,
    pub receiver: Vec<u8>,
    pub source_finality_proof_input: Vec<u8>,
    pub transfer_proof_input: Vec<u8>,
    pub finality_proof: Vec<u8>,
    pub transfer_proof: Vec<u8>,
}

impl SettlementReceipt {
    pub fn verified(request: &BridgeTransferRequest, finality_proof: Vec<u8>, transfer_proof: Vec<u8>) -> Self {
        let receipt_id = settlement_receipt_id(request, &finality_proof, &transfer_proof);
        Self {
            receipt_id,
            via: request.via.clone(),
            from_chain: request.from_chain.clone(),
            from_asset: request.from_asset.clone(),
            to_chain: request.to_chain.clone(),
            to_asset: request.to_asset.clone(),
            amount: request.amount,
            receiver: request.receiver.clone(),
            source_finality_proof_input: request.source_finality_proof.clone(),
            transfer_proof_input: request.transfer_proof.clone(),
            finality_proof,
            transfer_proof,
        }
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        [
            b"x3-settlement-receipt:v1:".as_slice(),
            self.receipt_id.as_bytes(),
            b":".as_slice(),
            self.via.as_bytes(),
            b":".as_slice(),
            self.from_chain.as_bytes(),
            b".".as_slice(),
            self.from_asset.as_bytes(),
            b"->".as_slice(),
            self.to_chain.as_bytes(),
            b".".as_slice(),
            self.to_asset.as_bytes(),
            b":".as_slice(),
            self.amount.to_string().as_bytes(),
            b":".as_slice(),
            &self.receiver,
            b":source_finality_proof_input=".as_slice(),
            &self.source_finality_proof_input,
            b":transfer_proof_input=".as_slice(),
            &self.transfer_proof_input,
        ]
        .concat()
    }
}

pub trait ProductionBridgeBackend {
    fn verify_source_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError>;

    fn verify_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError>;

    fn persist_receipt(&self, receipt: &SettlementReceipt) -> Result<(), BridgeError>;

    fn call_evm(&self, _data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: "EVM call requires an EVM-capable bridge backend".into(),
        })
    }

    fn call_svm(&self, _data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: "SVM call requires an SVM-capable bridge backend".into(),
        })
    }

    fn gas_estimate(&self, _chain: &[u8], _route: &[u8]) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: "gas estimation requires an RPC-enabled bridge backend".into(),
        })
    }

    fn chain_metric(&self, _metric: u8) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: "chain metrics require a configured bridge backend".into(),
        })
    }
}

pub trait ReceiptStore {
    fn persist(&self, receipt: &SettlementReceipt) -> Result<(), BridgeError>;
}

#[derive(Clone, Debug)]
pub struct FileReceiptStore {
    path: PathBuf,
}

impl FileReceiptStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl ReceiptStore for FileReceiptStore {
    fn persist(&self, receipt: &SettlementReceipt) -> Result<(), BridgeError> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| BridgeError {
                code: "X3_RECEIPT_STORE_CREATE_FAILED",
                message: err.to_string(),
            })?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|err| BridgeError {
                code: "X3_RECEIPT_STORE_OPEN_FAILED",
                message: err.to_string(),
            })?;
        let encoded = serde_json::to_vec(receipt).map_err(|err| BridgeError {
            code: "X3_RECEIPT_ENCODE_FAILED",
            message: err.to_string(),
        })?;
        file.write_all(&encoded).map_err(|err| BridgeError {
            code: "X3_RECEIPT_STORE_WRITE_FAILED",
            message: err.to_string(),
        })?;
        file.write_all(b"\n").map_err(|err| BridgeError {
            code: "X3_RECEIPT_STORE_WRITE_FAILED",
            message: err.to_string(),
        })?;
        file.sync_data().map_err(|err| BridgeError {
            code: "X3_RECEIPT_STORE_SYNC_FAILED",
            message: err.to_string(),
        })
    }
}

pub trait EvmFinalityVerifier {
    fn verify_evm_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError>;

    fn verify_evm_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError>;

    fn evm_raw_call(&self, _data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError {
            code: "X3_RPC_NOT_AVAILABLE",
            message: "EVM raw call requires an RPC-enabled EVM verifier".into(),
        })
    }

    fn evm_gas_estimate(&self, _data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError {
            code: "X3_RPC_NOT_AVAILABLE",
            message: "EVM gas estimate requires an RPC-enabled EVM verifier".into(),
        })
    }
}

pub trait SvmFinalityVerifier {
    fn verify_svm_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError>;

    fn verify_svm_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError>;

    fn svm_raw_call(&self, _data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError {
            code: "X3_RPC_NOT_AVAILABLE",
            message: "SVM raw call requires an RPC-enabled SVM verifier".into(),
        })
    }

    fn svm_gas_estimate(&self, _data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        Err(BridgeError {
            code: "X3_RPC_NOT_AVAILABLE",
            message: "SVM gas estimate requires an RPC-enabled SVM verifier".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// Production bridge capability matrix (2026-06-06)
//
// This block pins the production-readiness surface of the bridge
// module. The matrix is intentionally explicit so callers and
// reviewers can see at a glance which verifier pairs are wired today
// and which are still gated.
//
// Verifier              Source chain   Finality mechanism        Status
// --------------------  -------------  ------------------------  ------
// EthereumLightClientVerifier  ethereum  trusted header hash +    wired
//                                                min-block check
// SolanaLightClientVerifier    solana    trusted bank hash +      wired
//                                                validator pubkey
//                                                set + min sigs +
//                                                epoch + stake bps
// EthereumRpcFinalityVerifier  ethereum  JSON-RPC `eth_getTrans-  wired
//                                                actionReceipt` with
//                                                min-confirmations
// SolanaRpcFinalityVerifier    solana    JSON-RPC `getSignatur-   wired
//                                                eStatuses` with
//                                                expected program id
//
// Receipt store:
//   FileReceiptStore — append-only JSONL on local disk.            wired
//   (Distributed/DB-backed stores are a future concern; not
//   claimed today.)
//
// Gated (not wired today — explicitly NOT production):
//   * Merkle receipt trie over EVM log proofs
//   * Stake-account fixture loader for Solana
//   * Distributed receipt store (multi-host, crash-recovery)
//
// Production callers must wire one of the four verifier pairs above
// and a `FileReceiptStore`. No other configuration is production-safe.
// ---------------------------------------------------------------------------

pub struct EvmProductionBridgeBackend<V, S> {
    verifier: V,
    store: S,
}

impl<V, S> EvmProductionBridgeBackend<V, S> {
    pub fn new(verifier: V, store: S) -> Self {
        Self { verifier, store }
    }
}

impl<V, S> ProductionBridgeBackend for EvmProductionBridgeBackend<V, S>
where
    V: EvmFinalityVerifier,
    S: ReceiptStore,
{
    fn verify_source_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
        ensure_source_chain(request, &["ethereum", "evm"])?;
        self.verifier.verify_evm_finality(request)
    }

    fn verify_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        self.verifier.verify_evm_transfer_proof(request, finality_proof)
    }

    fn persist_receipt(&self, receipt: &SettlementReceipt) -> Result<(), BridgeError> {
        self.store.persist(receipt)
    }

    fn call_evm(&self, data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        self.verifier.evm_raw_call(data)
    }

    fn gas_estimate(&self, chain: &[u8], _route: &[u8]) -> Result<Vec<u8>, BridgeError> {
        if chain.eq_ignore_ascii_case(b"ethereum") || chain.eq_ignore_ascii_case(b"evm") {
            self.verifier
                .evm_gas_estimate(b"{\"to\":\"0x0000000000000000000000000000000000000000\"}")
        } else {
            Err(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!(
                    "gas estimation not available for chain '{}'",
                    String::from_utf8_lossy(chain)
                ),
            })
        }
    }

    fn chain_metric(&self, metric: u8) -> Result<Vec<u8>, BridgeError> {
        match metric {
            0 => Ok(b"ethereum".to_vec()),
            1 => {
                let block = self.verifier.evm_raw_call(
                    b"{\"to\":\"0x0000000000000000000000000000000000000000\",\"data\":\"0x\",\"block\":\"latest\"}",
                );
                match block {
                    Ok(_) => Ok(b"connected".to_vec()),
                    Err(_) => Ok(b"unknown".to_vec()),
                }
            }
            _ => Err(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!("chain metric {metric} not available"),
            }),
        }
    }
}

pub struct SvmProductionBridgeBackend<V, S> {
    verifier: V,
    store: S,
}

impl<V, S> SvmProductionBridgeBackend<V, S> {
    pub fn new(verifier: V, store: S) -> Self {
        Self { verifier, store }
    }
}

impl<V, S> ProductionBridgeBackend for SvmProductionBridgeBackend<V, S>
where
    V: SvmFinalityVerifier,
    S: ReceiptStore,
{
    fn verify_source_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
        ensure_source_chain(request, &["solana", "svm"])?;
        self.verifier.verify_svm_finality(request)
    }

    fn verify_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        self.verifier.verify_svm_transfer_proof(request, finality_proof)
    }

    fn persist_receipt(&self, receipt: &SettlementReceipt) -> Result<(), BridgeError> {
        self.store.persist(receipt)
    }

    fn call_svm(&self, data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        self.verifier.svm_raw_call(data)
    }

    fn gas_estimate(&self, chain: &[u8], _route: &[u8]) -> Result<Vec<u8>, BridgeError> {
        if chain.eq_ignore_ascii_case(b"solana") || chain.eq_ignore_ascii_case(b"svm") {
            self.verifier.svm_gas_estimate(b"{}")
        } else {
            Err(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!(
                    "gas estimation not available for chain '{}'",
                    String::from_utf8_lossy(chain)
                ),
            })
        }
    }

    fn chain_metric(&self, metric: u8) -> Result<Vec<u8>, BridgeError> {
        match metric {
            0 => Ok(b"solana".to_vec()),
            1 => Ok(b"connected".to_vec()),
            _ => Err(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!("chain metric {metric} not available"),
            }),
        }
    }
}

#[derive(Clone, Debug)]
pub struct EthereumLightClientVerifier {
    trusted_header_hash: String,
    min_block_number: Option<u64>,
    expected_log_address: Option<String>,
    require_erc20_transfer: bool,
}

impl EthereumLightClientVerifier {
    pub fn new(trusted_header_hash: impl Into<String>) -> Self {
        Self {
            trusted_header_hash: normalize_hex_string(trusted_header_hash.into()),
            min_block_number: None,
            expected_log_address: None,
            require_erc20_transfer: false,
        }
    }

    pub fn with_min_block_number(mut self, min_block_number: u64) -> Self {
        self.min_block_number = Some(min_block_number);
        self
    }

    pub fn with_erc20_transfer_event(mut self, token_address: impl Into<String>) -> Self {
        self.expected_log_address = Some(token_address.into().to_ascii_lowercase());
        self.require_erc20_transfer = true;
        self
    }
}

impl EvmFinalityVerifier for EthereumLightClientVerifier {
    fn verify_evm_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
        let proof = parse_json_proof(&request.source_finality_proof, "EVM finality")?;
        let header = verify_evm_header_proof(&proof, &self.trusted_header_hash, self.min_block_number)?;
        serde_json::to_vec(&json!({
            "chain": "ethereum",
            "proof_type": EVM_HEADER_PROOF_TYPE,
            "header_hash": header.header_hash,
            "block_number": header.block_number,
            "receipts_root": header.receipts_root,
        }))
        .map_err(|err| BridgeError {
            code: "X3_EVM_FINALITY_PROOF_ENCODE_FAILED",
            message: err.to_string(),
        })
    }

    fn verify_evm_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        let finality: Value = serde_json::from_slice(finality_proof).map_err(|err| BridgeError {
            code: "X3_EVM_FINALITY_PROOF_DECODE_FAILED",
            message: err.to_string(),
        })?;
        let receipts_root = expect_str(&finality, "receipts_root")?;
        let proof = parse_json_proof(&request.transfer_proof, "EVM transfer")?;
        let receipt = verify_evm_receipt_proof(&proof, receipts_root)?;
        if receipt.status != 1 {
            return Err(BridgeError {
                code: "X3_EVM_TX_FAILED",
                message: "receipt status is not successful".into(),
            });
        }
        if self.require_erc20_transfer {
            let log = expect_object(&proof, "log")?;
            let receipt_json = json!({"logs": [log.clone()]});
            if !evm_receipt_has_erc20_transfer(&receipt_json, request, self.expected_log_address.as_deref())? {
                return Err(BridgeError {
                    code: "X3_EVM_TRANSFER_EVENT_MISMATCH",
                    message: "receipt proof log does not match token, receiver, and amount".into(),
                });
            }
        }
        serde_json::to_vec(&json!({
            "chain": "ethereum",
            "proof_type": EVM_RECEIPT_PROOF_TYPE,
            "receipt_root": receipts_root,
            "receipt_hash": receipt.receipt_hash,
            "amount": request.amount.to_string(),
            "asset": request.from_asset,
            "receiver": String::from_utf8_lossy(&request.receiver),
        }))
        .map_err(|err| BridgeError {
            code: "X3_EVM_TRANSFER_PROOF_ENCODE_FAILED",
            message: err.to_string(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct SolanaLightClientVerifier {
    trusted_bank_hash: String,
    validator_pubkeys: Vec<String>,
    min_signatures: usize,
    trusted_epoch: Option<TrustedSvmEpoch>,
    min_stake_threshold_bps: u64,
    expected_program_id: Option<String>,
}

impl SolanaLightClientVerifier {
    pub fn new(trusted_bank_hash: impl Into<String>) -> Self {
        Self {
            trusted_bank_hash: normalize_hex_string(trusted_bank_hash.into()),
            validator_pubkeys: Vec::new(),
            min_signatures: 1,
            trusted_epoch: None,
            min_stake_threshold_bps: DEFAULT_SVM_STAKE_THRESHOLD_BPS,
            expected_program_id: None,
        }
    }

    pub fn with_validator_pubkeys(mut self, validator_pubkeys: Vec<String>) -> Self {
        self.validator_pubkeys = validator_pubkeys.into_iter().map(normalize_hex_string).collect();
        self
    }

    pub fn with_min_signatures(mut self, min_signatures: usize) -> Self {
        self.min_signatures = min_signatures;
        self
    }

    pub fn with_trusted_epoch_hash(mut self, epoch: u64, epoch_hash: impl Into<String>) -> Self {
        self.trusted_epoch = Some(TrustedSvmEpoch {
            epoch,
            epoch_hash: normalize_hex_string(epoch_hash.into()),
        });
        self
    }

    pub fn with_min_stake_threshold_bps(mut self, threshold_bps: u64) -> Self {
        self.min_stake_threshold_bps = threshold_bps;
        self
    }

    pub fn with_expected_program_id(mut self, program_id: impl Into<String>) -> Self {
        self.expected_program_id = Some(program_id.into());
        self
    }
}

impl SvmFinalityVerifier for SolanaLightClientVerifier {
    fn verify_svm_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
        let proof = parse_json_proof(&request.source_finality_proof, "SVM finality")?;
        let bank = verify_svm_bank_proof(
            &proof,
            &self.trusted_bank_hash,
            &self.validator_pubkeys,
            self.min_signatures,
            self.trusted_epoch.as_ref(),
            self.min_stake_threshold_bps,
        )?;
        serde_json::to_vec(&json!({
            "chain": "solana",
            "proof_type": SVM_BANK_PROOF_TYPE,
            "slot": bank.slot,
            "bank_hash": bank.bank_hash,
            "epoch": bank.epoch,
            "epoch_hash": bank.epoch_hash,
            "signed_stake": bank.signed_stake.to_string(),
            "total_active_stake": bank.total_active_stake.to_string(),
        }))
        .map_err(|err| BridgeError {
            code: "X3_SVM_FINALITY_PROOF_ENCODE_FAILED",
            message: err.to_string(),
        })
    }

    fn verify_svm_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        let finality: Value = serde_json::from_slice(finality_proof).map_err(|err| BridgeError {
            code: "X3_SVM_FINALITY_PROOF_DECODE_FAILED",
            message: err.to_string(),
        })?;
        let slot = finality
            .get("slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| BridgeError {
                code: "X3_SVM_FINALITY_SLOT_MISSING",
                message: "SVM finality proof receipt has no slot".into(),
            })?;
        let bank_hash = expect_str(&finality, "bank_hash")?;
        let proof = parse_json_proof(&request.transfer_proof, "SVM transfer")?;
        let tx = verify_svm_transaction_proof(&proof, request, slot, bank_hash, self.expected_program_id.as_deref())?;
        serde_json::to_vec(&json!({
            "chain": "solana",
            "proof_type": SVM_TRANSACTION_PROOF_TYPE,
            "slot": slot,
            "bank_hash": bank_hash,
            "transaction_hash": tx.transaction_hash,
            "amount": request.amount.to_string(),
            "asset": request.from_asset,
            "receiver": String::from_utf8_lossy(&request.receiver),
        }))
        .map_err(|err| BridgeError {
            code: "X3_SVM_TRANSFER_PROOF_ENCODE_FAILED",
            message: err.to_string(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct EthereumRpcFinalityVerifier {
    rpc_url: String,
    tx_hash: String,
    min_confirmations: u64,
    expected_log_address: Option<String>,
    expected_log_topic: Option<String>,
    require_erc20_transfer: bool,
}

impl EthereumRpcFinalityVerifier {
    pub fn new(rpc_url: impl Into<String>, tx_hash: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            tx_hash: tx_hash.into(),
            min_confirmations: 12,
            expected_log_address: None,
            expected_log_topic: None,
            require_erc20_transfer: false,
        }
    }

    pub fn with_min_confirmations(mut self, min_confirmations: u64) -> Self {
        self.min_confirmations = min_confirmations;
        self
    }

    pub fn with_expected_log_address(mut self, address: impl Into<String>) -> Self {
        self.expected_log_address = Some(address.into().to_ascii_lowercase());
        self
    }

    pub fn with_expected_log_topic(mut self, topic: impl Into<String>) -> Self {
        self.expected_log_topic = Some(topic.into().to_ascii_lowercase());
        self
    }

    pub fn with_erc20_transfer_event(mut self, token_address: impl Into<String>) -> Self {
        self.expected_log_address = Some(token_address.into().to_ascii_lowercase());
        self.expected_log_topic = Some(ERC20_TRANSFER_TOPIC.to_string());
        self.require_erc20_transfer = true;
        self
    }

    fn rpc(&self, method: &str, params: Value) -> Result<Value, BridgeError> {
        json_rpc_call(&self.rpc_url, method, params)
    }

    fn transaction_receipt(&self) -> Result<Value, BridgeError> {
        let receipt = self.rpc("eth_getTransactionReceipt", json!([self.tx_hash]))?;
        if receipt.is_null() {
            return Err(BridgeError {
                code: "X3_EVM_RECEIPT_MISSING",
                message: format!("transaction receipt not found for {}", self.tx_hash),
            });
        }
        Ok(receipt)
    }
}

impl EvmFinalityVerifier for EthereumRpcFinalityVerifier {
    fn verify_evm_finality(&self, _request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
        let latest = self.rpc("eth_blockNumber", json!([]))?;
        let latest = parse_hex_u64(latest.as_str().ok_or_else(|| BridgeError {
            code: "X3_EVM_BAD_BLOCK_NUMBER",
            message: "eth_blockNumber did not return a hex string".into(),
        })?)?;

        let receipt = self.transaction_receipt()?;
        let tx_block =
            parse_hex_u64(
                receipt
                    .get("blockNumber")
                    .and_then(Value::as_str)
                    .ok_or_else(|| BridgeError {
                        code: "X3_EVM_RECEIPT_UNMINED",
                        message: "transaction receipt has no blockNumber".into(),
                    })?,
            )?;
        let confirmations = latest.saturating_sub(tx_block).saturating_add(1);
        if confirmations < self.min_confirmations {
            return Err(BridgeError {
                code: "X3_EVM_FINALITY_INSUFFICIENT",
                message: format!(
                    "transaction has {confirmations} confirmations; need {}",
                    self.min_confirmations
                ),
            });
        }

        serde_json::to_vec(&json!({
            "chain": "ethereum",
            "tx_hash": self.tx_hash,
            "latest_block": latest,
            "tx_block": tx_block,
            "confirmations": confirmations
        }))
        .map_err(|err| BridgeError {
            code: "X3_EVM_FINALITY_PROOF_ENCODE_FAILED",
            message: err.to_string(),
        })
    }

    fn verify_evm_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        _finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        let receipt = self.transaction_receipt()?;
        if receipt.get("status").and_then(Value::as_str) != Some("0x1") {
            return Err(BridgeError {
                code: "X3_EVM_TX_FAILED",
                message: format!("transaction {} did not succeed", self.tx_hash),
            });
        }
        if let Some(address) = &self.expected_log_address {
            let found = receipt.get("logs").and_then(Value::as_array).is_some_and(|logs| {
                logs.iter().any(|log| {
                    log.get("address")
                        .and_then(Value::as_str)
                        .map(|value| value.to_ascii_lowercase() == *address)
                        .unwrap_or(false)
                })
            });
            if !found {
                return Err(BridgeError {
                    code: "X3_EVM_LOG_ADDRESS_MISSING",
                    message: format!("expected log address {address} not found"),
                });
            }
        }
        if let Some(topic) = &self.expected_log_topic {
            let found = receipt.get("logs").and_then(Value::as_array).is_some_and(|logs| {
                logs.iter().any(|log| {
                    log.get("topics").and_then(Value::as_array).is_some_and(|topics| {
                        topics.iter().any(|value| {
                            value
                                .as_str()
                                .map(|value| value.to_ascii_lowercase() == *topic)
                                .unwrap_or(false)
                        })
                    })
                })
            });
            if !found {
                return Err(BridgeError {
                    code: "X3_EVM_LOG_TOPIC_MISSING",
                    message: format!("expected log topic {topic} not found"),
                });
            }
        }
        if self.require_erc20_transfer
            && !evm_receipt_has_erc20_transfer(&receipt, request, self.expected_log_address.as_deref())?
        {
            return Err(BridgeError {
                code: "X3_EVM_TRANSFER_EVENT_MISMATCH",
                message: "receipt does not contain an ERC-20 Transfer log matching token, receiver, and amount".into(),
            });
        }

        serde_json::to_vec(&json!({
            "chain": "ethereum",
            "tx_hash": self.tx_hash,
            "from_chain": request.from_chain,
            "to_chain": request.to_chain,
            "asset": request.from_asset,
            "amount": request.amount.to_string(),
            "receipt": receipt
        }))
        .map_err(|err| BridgeError {
            code: "X3_EVM_TRANSFER_PROOF_ENCODE_FAILED",
            message: err.to_string(),
        })
    }

    fn evm_raw_call(&self, data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        let params: Value = serde_json::from_slice(data).map_err(|err| BridgeError {
            code: "X3_EVM_CALL_INVALID_PARAMS",
            message: format!("EVM call params must be valid JSON: {err}"),
        })?;
        let to = params.get("to").and_then(Value::as_str).ok_or_else(|| BridgeError {
            code: "X3_EVM_CALL_MISSING_TO",
            message: "EVM call params must include 'to' address".into(),
        })?;
        let call_data = params.get("data").and_then(Value::as_str).unwrap_or("0x");
        let tx_object = json!({
            "to": to,
            "data": call_data,
        });
        let block = params.get("block").and_then(Value::as_str).unwrap_or("latest");
        let result = self.rpc("eth_call", json!([tx_object, block]))?;
        let hex_str = result.as_str().ok_or_else(|| BridgeError {
            code: "X3_EVM_CALL_BAD_RESPONSE",
            message: "eth_call did not return a hex string".into(),
        })?;
        decode_hex(hex_str)
    }

    fn evm_gas_estimate(&self, data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        let params: Value = serde_json::from_slice(data).map_err(|err| BridgeError {
            code: "X3_EVM_GAS_PARAMS_INVALID",
            message: format!("gas estimate params must be valid JSON: {err}"),
        })?;
        let to = params.get("to").and_then(Value::as_str).ok_or_else(|| BridgeError {
            code: "X3_EVM_GAS_MISSING_TO",
            message: "gas estimate params must include 'to' address".into(),
        })?;
        let tx_object = json!({ "to": to, "data": params.get("data") });
        let result = self.rpc("eth_estimateGas", json!([tx_object]))?;
        let hex_str = result.as_str().ok_or_else(|| BridgeError {
            code: "X3_EVM_GAS_BAD_RESPONSE",
            message: "eth_estimateGas did not return a hex string".into(),
        })?;
        decode_hex(hex_str)
    }
}

#[derive(Clone, Debug)]
pub struct SolanaRpcFinalityVerifier {
    rpc_url: String,
    signature: String,
    expected_program_id: Option<String>,
    require_parsed_transfer: bool,
}

impl SolanaRpcFinalityVerifier {
    pub fn new(rpc_url: impl Into<String>, signature: impl Into<String>) -> Self {
        Self {
            rpc_url: rpc_url.into(),
            signature: signature.into(),
            expected_program_id: None,
            require_parsed_transfer: false,
        }
    }

    pub fn with_expected_program_id(mut self, program_id: impl Into<String>) -> Self {
        self.expected_program_id = Some(program_id.into());
        self
    }

    pub fn require_parsed_token_transfer(mut self) -> Self {
        self.require_parsed_transfer = true;
        self
    }

    fn rpc(&self, method: &str, params: Value) -> Result<Value, BridgeError> {
        json_rpc_call(&self.rpc_url, method, params)
    }
}

impl SvmFinalityVerifier for SolanaRpcFinalityVerifier {
    fn verify_svm_finality(&self, _request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
        let statuses = self.rpc(
            "getSignatureStatuses",
            json!([[self.signature], {"searchTransactionHistory": true}]),
        )?;
        let status = statuses
            .get("value")
            .and_then(Value::as_array)
            .and_then(|values| values.first())
            .and_then(Value::as_object)
            .ok_or_else(|| BridgeError {
                code: "X3_SVM_SIGNATURE_MISSING",
                message: format!("signature status not found for {}", self.signature),
            })?;
        if !status.get("err").unwrap_or(&Value::Null).is_null() {
            return Err(BridgeError {
                code: "X3_SVM_TX_FAILED",
                message: format!("signature {} has an error status", self.signature),
            });
        }
        if status.get("confirmationStatus").and_then(Value::as_str) != Some("finalized") {
            return Err(BridgeError {
                code: "X3_SVM_NOT_FINALIZED",
                message: format!("signature {} is not finalized", self.signature),
            });
        }
        serde_json::to_vec(&json!({
            "chain": "solana",
            "signature": self.signature,
            "status": status
        }))
        .map_err(|err| BridgeError {
            code: "X3_SVM_FINALITY_PROOF_ENCODE_FAILED",
            message: err.to_string(),
        })
    }

    fn verify_svm_transfer_proof(
        &self,
        request: &BridgeTransferRequest,
        _finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        let tx = self.rpc(
            "getTransaction",
            json!([
                self.signature,
                {
                    "encoding": "jsonParsed",
                    "commitment": "finalized",
                    "maxSupportedTransactionVersion": 0
                }
            ]),
        )?;
        if tx.is_null() {
            return Err(BridgeError {
                code: "X3_SVM_TRANSACTION_MISSING",
                message: format!("transaction not found for {}", self.signature),
            });
        }
        if !tx
            .get("meta")
            .and_then(|meta| meta.get("err"))
            .unwrap_or(&Value::Null)
            .is_null()
        {
            return Err(BridgeError {
                code: "X3_SVM_TX_FAILED",
                message: format!("transaction {} failed", self.signature),
            });
        }
        let tx_json = tx.to_string();
        if !tx_json.contains(&self.signature) {
            return Err(BridgeError {
                code: "X3_SVM_SIGNATURE_NOT_IN_TRANSACTION",
                message: format!("transaction does not include signature {}", self.signature),
            });
        }
        let receiver = std::str::from_utf8(&request.receiver).map_err(|_| BridgeError {
            code: "X3_SVM_RECEIVER_NOT_UTF8",
            message: "receiver is not a UTF-8 Solana address".into(),
        })?;
        if !tx_json.contains(receiver) {
            return Err(BridgeError {
                code: "X3_SVM_RECEIVER_MISSING",
                message: format!("transaction does not reference receiver {receiver}"),
            });
        }
        if let Some(program_id) = &self.expected_program_id {
            if self.require_parsed_transfer {
                if !svm_transaction_has_parsed_transfer(&tx, request, Some(program_id))? {
                    return Err(BridgeError {
                        code: "X3_SVM_TRANSFER_INSTRUCTION_MISMATCH",
                        message: "transaction does not contain a parsed transfer matching program, receiver, mint, and amount".into(),
                    });
                }
            } else if !tx_json.contains(program_id) {
                return Err(BridgeError {
                    code: "X3_SVM_PROGRAM_MISSING",
                    message: format!("transaction does not reference program {program_id}"),
                });
            }
        } else if self.require_parsed_transfer && !svm_transaction_has_parsed_transfer(&tx, request, None)? {
            return Err(BridgeError {
                code: "X3_SVM_TRANSFER_INSTRUCTION_MISMATCH",
                message: "transaction does not contain a parsed transfer matching receiver, mint, and amount".into(),
            });
        }
        serde_json::to_vec(&json!({
            "chain": "solana",
            "signature": self.signature,
            "from_chain": request.from_chain,
            "to_chain": request.to_chain,
            "asset": request.from_asset,
            "amount": request.amount.to_string(),
            "transaction": tx
        }))
        .map_err(|err| BridgeError {
            code: "X3_SVM_TRANSFER_PROOF_ENCODE_FAILED",
            message: err.to_string(),
        })
    }

    fn svm_raw_call(&self, data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        let params: Value = serde_json::from_slice(data).map_err(|err| BridgeError {
            code: "X3_SVM_CALL_INVALID_PARAMS",
            message: format!("SVM call params must be valid JSON: {err}"),
        })?;
        let method = params
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| BridgeError {
                code: "X3_SVM_CALL_MISSING_METHOD",
                message: "SVM call params must include 'method'".into(),
            })?;
        let call_params = params.get("params").cloned().unwrap_or(json!([]));
        let result = self.rpc(method, json!([call_params]))?;
        serde_json::to_vec(&result).map_err(|err| BridgeError {
            code: "X3_SVM_CALL_ENCODE_FAILED",
            message: format!("failed to encode SVM call result: {err}"),
        })
    }

    fn svm_gas_estimate(&self, _data: &[u8]) -> Result<Vec<u8>, BridgeError> {
        let budget: u64 = 200_000;
        Ok(budget.to_le_bytes().to_vec())
    }
}

pub struct ProductionBridgeAdapter<B> {
    backend: B,
}

impl<B> ProductionBridgeAdapter<B> {
    pub fn new(backend: B) -> Self {
        Self { backend }
    }
}

fn settlement_receipt_id(request: &BridgeTransferRequest, finality_proof: &[u8], transfer_proof: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(request.via.as_bytes());
    hasher.update(request.from_chain.as_bytes());
    hasher.update(request.from_asset.as_bytes());
    hasher.update(request.to_chain.as_bytes());
    hasher.update(request.to_asset.as_bytes());
    hasher.update(request.amount.to_le_bytes());
    hasher.update(&request.receiver);
    hasher.update(&request.source_finality_proof);
    hasher.update(&request.transfer_proof);
    hasher.update(finality_proof);
    hasher.update(transfer_proof);
    let digest = hasher.finalize();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn ensure_source_chain(request: &BridgeTransferRequest, allowed: &[&str]) -> Result<(), BridgeError> {
    let source = request.from_chain.to_ascii_lowercase();
    if allowed.iter().any(|chain| *chain == source) {
        Ok(())
    } else {
        Err(BridgeError {
            code: "X3_BRIDGE_SOURCE_CHAIN_MISMATCH",
            message: format!("backend does not verify source chain '{}'", request.from_chain),
        })
    }
}

fn json_rpc_call(rpc_url: &str, method: &str, params: Value) -> Result<Value, BridgeError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|err| BridgeError {
            code: "X3_RPC_CLIENT_BUILD_FAILED",
            message: err.to_string(),
        })?;
    let response = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        }))
        .send()
        .map_err(|err| BridgeError {
            code: "X3_RPC_REQUEST_FAILED",
            message: err.to_string(),
        })?;
    if !response.status().is_success() {
        return Err(BridgeError {
            code: "X3_RPC_HTTP_ERROR",
            message: response.status().to_string(),
        });
    }
    let body: Value = response.json().map_err(|err| BridgeError {
        code: "X3_RPC_RESPONSE_DECODE_FAILED",
        message: err.to_string(),
    })?;
    if let Some(error) = body.get("error") {
        return Err(BridgeError {
            code: "X3_RPC_ERROR",
            message: error.to_string(),
        });
    }
    body.get("result").cloned().ok_or_else(|| BridgeError {
        code: "X3_RPC_RESULT_MISSING",
        message: format!("{method} response did not include result"),
    })
}

fn parse_hex_u64(value: &str) -> Result<u64, BridgeError> {
    u64::from_str_radix(value.trim_start_matches("0x"), 16).map_err(|err| BridgeError {
        code: "X3_HEX_U64_DECODE_FAILED",
        message: err.to_string(),
    })
}

#[derive(Debug)]
struct VerifiedEvmHeader {
    header_hash: String,
    receipts_root: String,
    block_number: u64,
}

#[derive(Debug)]
struct VerifiedEvmReceipt {
    receipt_hash: String,
    status: u8,
}

#[derive(Debug)]
struct VerifiedSvmBank {
    slot: u64,
    bank_hash: String,
    epoch: Option<u64>,
    epoch_hash: Option<String>,
    signed_stake: u128,
    total_active_stake: u128,
}

#[derive(Debug)]
struct VerifiedSvmTransaction {
    transaction_hash: String,
}

#[derive(Clone, Debug)]
struct TrustedSvmEpoch {
    epoch: u64,
    epoch_hash: String,
}

#[derive(Clone, Debug)]
struct SvmValidatorStake {
    public_key: String,
    stake: u128,
    active: bool,
}

#[derive(Debug, Deserialize)]
struct SvmStakeAccountData {
    voter_pubkey: String,
    delegated_stake: String,
    activation_epoch: u64,
    deactivation_epoch: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
enum SolanaStakeStateV2Wire {
    Uninitialized,
    Initialized(SolanaStakeMetaWire),
    Stake(SolanaStakeMetaWire, SolanaStakeWire, SolanaStakeFlagsWire),
    RewardsPool,
}

#[derive(Debug, Deserialize, Serialize)]
struct SolanaStakeMetaWire {
    rent_exempt_reserve: u64,
    authorized: SolanaAuthorizedWire,
    lockup: SolanaLockupWire,
}

#[derive(Debug, Deserialize, Serialize)]
struct SolanaAuthorizedWire {
    staker: [u8; 32],
    withdrawer: [u8; 32],
}

#[derive(Debug, Deserialize, Serialize)]
struct SolanaLockupWire {
    unix_timestamp: i64,
    epoch: u64,
    custodian: [u8; 32],
}

#[derive(Debug, Deserialize, Serialize)]
struct SolanaStakeWire {
    delegation: SolanaDelegationWire,
    credits_observed: u64,
}

#[derive(Debug, Deserialize, Serialize)]
struct SolanaDelegationWire {
    voter_pubkey: [u8; 32],
    stake: u64,
    activation_epoch: u64,
    deactivation_epoch: u64,
    warmup_cooldown_rate: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct SolanaStakeFlagsWire {
    bits: u8,
}

#[derive(Debug)]
struct VerifiedSvmEpoch {
    epoch: u64,
    epoch_hash: String,
    active_stakes: BTreeMap<String, u128>,
    total_active_stake: u128,
}

fn parse_json_proof(bytes: &[u8], name: &str) -> Result<Value, BridgeError> {
    if bytes.is_empty() {
        return Err(BridgeError {
            code: "X3_LIGHT_CLIENT_PROOF_MISSING",
            message: format!("{name} proof input is empty"),
        });
    }
    serde_json::from_slice(bytes).map_err(|err| BridgeError {
        code: "X3_LIGHT_CLIENT_PROOF_DECODE_FAILED",
        message: format!("{name} proof is not valid JSON: {err}"),
    })
}

fn verify_evm_header_proof(
    proof: &Value,
    trusted_header_hash: &str,
    min_block_number: Option<u64>,
) -> Result<VerifiedEvmHeader, BridgeError> {
    require_proof_type(proof, EVM_HEADER_PROOF_TYPE)?;
    let rlp_header = hex_to_bytes(expect_str(proof, "rlp_header")?, "rlp_header")?;
    let computed_hash = hex_prefixed(&keccak256(&rlp_header));
    let supplied_hash = normalize_hex_string(expect_str(proof, "header_hash")?);
    if computed_hash != supplied_hash {
        return Err(BridgeError {
            code: "X3_EVM_HEADER_HASH_MISMATCH",
            message: "header_hash does not match keccak256(rlp_header)".into(),
        });
    }
    if supplied_hash != trusted_header_hash {
        return Err(BridgeError {
            code: "X3_EVM_HEADER_NOT_TRUSTED",
            message: "header hash is not the configured trusted finalized header".into(),
        });
    }

    let header_fields = rlp_list_items(&rlp_header)?;
    let receipts_root = header_fields.get(5).ok_or_else(|| BridgeError {
        code: "X3_EVM_HEADER_RECEIPTS_ROOT_MISSING",
        message: "RLP header does not contain receiptsRoot field".into(),
    })?;
    if receipts_root.len() != 32 {
        return Err(BridgeError {
            code: "X3_EVM_HEADER_RECEIPTS_ROOT_INVALID",
            message: "RLP header receiptsRoot is not 32 bytes".into(),
        });
    }
    let block_number = header_fields
        .get(8)
        .map(|value| rlp_uint_to_u64(value))
        .transpose()?
        .ok_or_else(|| BridgeError {
            code: "X3_EVM_HEADER_NUMBER_MISSING",
            message: "RLP header does not contain number field".into(),
        })?;
    if let Some(min_block_number) = min_block_number {
        if block_number < min_block_number {
            return Err(BridgeError {
                code: "X3_EVM_HEADER_TOO_OLD",
                message: format!("header block {block_number} is below required {min_block_number}"),
            });
        }
    }

    let receipts_root = hex_prefixed(receipts_root);
    if let Some(expected_root) = proof.get("receipts_root").and_then(Value::as_str) {
        if normalize_hex_string(expected_root) != receipts_root {
            return Err(BridgeError {
                code: "X3_EVM_RECEIPTS_ROOT_MISMATCH",
                message: "proof receipts_root does not match RLP header receiptsRoot".into(),
            });
        }
    }

    Ok(VerifiedEvmHeader {
        header_hash: supplied_hash,
        receipts_root,
        block_number,
    })
}

fn verify_evm_receipt_proof(proof: &Value, receipts_root: &str) -> Result<VerifiedEvmReceipt, BridgeError> {
    require_proof_type(proof, EVM_RECEIPT_PROOF_TYPE)?;
    let receipt_rlp = hex_to_bytes(expect_str(proof, "receipt_rlp")?, "receipt_rlp")?;
    let receipt_hash = keccak256(&receipt_rlp);
    let receipt_hash_hex = hex_prefixed(&receipt_hash);
    if let Some(expected_hash) = proof.get("receipt_hash").and_then(Value::as_str) {
        if normalize_hex_string(expected_hash) != receipt_hash_hex {
            return Err(BridgeError {
                code: "X3_EVM_RECEIPT_HASH_MISMATCH",
                message: "receipt_hash does not match keccak256(receipt_rlp)".into(),
            });
        }
    }
    verify_evm_receipt_trie_proof(proof, receipts_root, &receipt_rlp)?;
    if let Some(legacy_proof) = proof.get("receipt_proof") {
        if legacy_proof.as_array().is_some_and(|proof| !proof.is_empty()) {
            return Err(BridgeError {
                code: "X3_EVM_RECEIPT_PROOF_LEGACY_UNSUPPORTED",
                message: "receipt_proof hash paths are not accepted; provide trie_nodes MPT proof".into(),
            });
        }
    }
    if let Some(proof_root) = proof.get("receipts_root").and_then(Value::as_str) {
        if normalize_hex_string(proof_root) != normalize_hex_string(receipts_root) {
            return Err(BridgeError {
                code: "X3_EVM_RECEIPT_ROOT_MISMATCH",
                message: "proof receipts_root does not match finalized receiptsRoot".into(),
            });
        }
    }
    let status = evm_receipt_status(&receipt_rlp)?;
    Ok(VerifiedEvmReceipt {
        receipt_hash: receipt_hash_hex,
        status,
    })
}

fn verify_svm_bank_proof(
    proof: &Value,
    trusted_bank_hash: &str,
    validator_pubkeys: &[String],
    min_signatures: usize,
    trusted_epoch: Option<&TrustedSvmEpoch>,
    min_stake_threshold_bps: u64,
) -> Result<VerifiedSvmBank, BridgeError> {
    require_proof_type(proof, SVM_BANK_PROOF_TYPE)?;
    let slot = proof.get("slot").and_then(Value::as_u64).ok_or_else(|| BridgeError {
        code: "X3_SVM_BANK_SLOT_MISSING",
        message: "SVM bank proof has no slot".into(),
    })?;
    let bank_hash = normalize_hex_string(expect_str(proof, "bank_hash")?);
    if bank_hash != trusted_bank_hash {
        return Err(BridgeError {
            code: "X3_SVM_BANK_HASH_NOT_TRUSTED",
            message: "bank hash is not the configured trusted finalized bank".into(),
        });
    }
    let parent_bank_hash = normalize_hex_string(expect_str(proof, "parent_bank_hash")?);
    let message = format!("{SVM_BANK_PROOF_TYPE}:{slot}:{bank_hash}:{parent_bank_hash}");

    if trusted_epoch.is_some() || proof.get("epoch_proof").is_some() {
        let epoch_proof = proof.get("epoch_proof").ok_or_else(|| BridgeError {
            code: "X3_SVM_EPOCH_PROOF_MISSING",
            message: "SVM bank proof must include epoch_proof for stake-weighted verification".into(),
        })?;
        let epoch = verify_svm_epoch_proof(epoch_proof, trusted_epoch, min_stake_threshold_bps)?;
        let signed_stake = verify_stake_weighted_signature_set(
            proof.get("signatures").and_then(Value::as_array),
            message.as_bytes(),
            &epoch.active_stakes,
        )?;
        let required_stake = stake_threshold(epoch.total_active_stake, min_stake_threshold_bps)?;
        if signed_stake < required_stake {
            return Err(BridgeError {
                code: "X3_SVM_BANK_STAKE_THRESHOLD_NOT_MET",
                message: format!("verified {signed_stake} bank signature stake; need {required_stake}"),
            });
        }
        return Ok(VerifiedSvmBank {
            slot,
            bank_hash,
            epoch: Some(epoch.epoch),
            epoch_hash: Some(epoch.epoch_hash),
            signed_stake,
            total_active_stake: epoch.total_active_stake,
        });
    }

    let verified = verify_signature_set(
        proof.get("signatures").and_then(Value::as_array),
        message.as_bytes(),
        validator_pubkeys,
    )?;
    if verified < min_signatures {
        return Err(BridgeError {
            code: "X3_SVM_BANK_SIGNATURE_THRESHOLD_NOT_MET",
            message: format!("verified {verified} bank signatures; need {min_signatures}"),
        });
    }
    Ok(VerifiedSvmBank {
        slot,
        bank_hash,
        epoch: None,
        epoch_hash: None,
        signed_stake: verified as u128,
        total_active_stake: validator_pubkeys.len() as u128,
    })
}

fn verify_svm_epoch_proof(
    proof: &Value,
    trusted_epoch: Option<&TrustedSvmEpoch>,
    min_stake_threshold_bps: u64,
) -> Result<VerifiedSvmEpoch, BridgeError> {
    require_proof_type(proof, SVM_EPOCH_PROOF_TYPE)?;
    let epoch = proof.get("epoch").and_then(Value::as_u64).ok_or_else(|| BridgeError {
        code: "X3_SVM_EPOCH_MISSING",
        message: "SVM epoch proof has no epoch".into(),
    })?;
    let parent_epoch_hash =
        normalize_hex_string(proof.get("parent_epoch_hash").and_then(Value::as_str).unwrap_or("0x"));
    let validators = parse_svm_validator_set(proof)?;
    let epoch_hash = svm_epoch_hash(epoch, &parent_epoch_hash, &validators);
    if let Some(supplied_hash) = proof.get("epoch_hash").and_then(Value::as_str) {
        if normalize_hex_string(supplied_hash) != epoch_hash {
            return Err(BridgeError {
                code: "X3_SVM_EPOCH_HASH_MISMATCH",
                message: "epoch_hash does not match epoch validator set".into(),
            });
        }
    }
    if let Some(trusted) = trusted_epoch {
        if trusted.epoch != epoch || trusted.epoch_hash != epoch_hash {
            return Err(BridgeError {
                code: "X3_SVM_EPOCH_NOT_TRUSTED",
                message: "epoch proof does not match the configured trusted epoch hash".into(),
            });
        }
    } else {
        return Err(BridgeError {
            code: "X3_SVM_EPOCH_TRUST_ANCHOR_MISSING",
            message: "stake-weighted SVM epoch proof requires a configured trusted epoch hash".into(),
        });
    }

    if let Some(transition) = proof.get("transition") {
        verify_svm_epoch_transition(
            transition,
            epoch,
            &epoch_hash,
            &parent_epoch_hash,
            min_stake_threshold_bps,
        )?;
    }

    let mut active_stakes = BTreeMap::new();
    let mut total_active_stake = 0u128;
    for validator in validators {
        if validator.active {
            total_active_stake = total_active_stake
                .checked_add(validator.stake)
                .ok_or_else(|| BridgeError {
                    code: "X3_SVM_STAKE_OVERFLOW",
                    message: "validator stake total overflowed u128".into(),
                })?;
            active_stakes.insert(validator.public_key, validator.stake);
        }
    }
    if total_active_stake == 0 {
        return Err(BridgeError {
            code: "X3_SVM_ACTIVE_STAKE_EMPTY",
            message: "SVM epoch proof has no active stake".into(),
        });
    }
    Ok(VerifiedSvmEpoch {
        epoch,
        epoch_hash,
        active_stakes,
        total_active_stake,
    })
}

fn verify_svm_epoch_transition(
    transition: &Value,
    epoch: u64,
    epoch_hash: &str,
    parent_epoch_hash: &str,
    min_stake_threshold_bps: u64,
) -> Result<(), BridgeError> {
    require_proof_type(transition, SVM_EPOCH_TRANSITION_PROOF_TYPE)?;
    let parent_validators = parse_svm_validator_set(transition)?;
    let mut parent_stakes = BTreeMap::new();
    let mut total_parent_stake = 0u128;
    for validator in parent_validators {
        if validator.active {
            total_parent_stake = total_parent_stake
                .checked_add(validator.stake)
                .ok_or_else(|| BridgeError {
                    code: "X3_SVM_STAKE_OVERFLOW",
                    message: "parent validator stake total overflowed u128".into(),
                })?;
            parent_stakes.insert(validator.public_key, validator.stake);
        }
    }
    if total_parent_stake == 0 {
        return Err(BridgeError {
            code: "X3_SVM_PARENT_ACTIVE_STAKE_EMPTY",
            message: "SVM epoch transition has no active parent stake".into(),
        });
    }
    let expected_parent = transition
        .get("parent_epoch_hash")
        .and_then(Value::as_str)
        .map(normalize_hex_string)
        .unwrap_or_else(|| parent_epoch_hash.to_string());
    if expected_parent != parent_epoch_hash {
        return Err(BridgeError {
            code: "X3_SVM_PARENT_EPOCH_HASH_MISMATCH",
            message: "transition parent_epoch_hash does not match epoch proof parent".into(),
        });
    }
    let message = format!("{SVM_EPOCH_TRANSITION_PROOF_TYPE}:{epoch}:{parent_epoch_hash}:{epoch_hash}");
    let signed_stake = verify_stake_weighted_signature_set(
        transition.get("signatures").and_then(Value::as_array),
        message.as_bytes(),
        &parent_stakes,
    )?;
    let required_stake = stake_threshold(total_parent_stake, min_stake_threshold_bps)?;
    if signed_stake < required_stake {
        return Err(BridgeError {
            code: "X3_SVM_EPOCH_TRANSITION_STAKE_THRESHOLD_NOT_MET",
            message: format!("verified {signed_stake} epoch transition stake; need {required_stake}"),
        });
    }
    Ok(())
}

fn verify_svm_transaction_proof(
    proof: &Value,
    request: &BridgeTransferRequest,
    finalized_slot: u64,
    finalized_bank_hash: &str,
    expected_program_id: Option<&str>,
) -> Result<VerifiedSvmTransaction, BridgeError> {
    require_proof_type(proof, SVM_TRANSACTION_PROOF_TYPE)?;
    let slot = proof.get("slot").and_then(Value::as_u64).ok_or_else(|| BridgeError {
        code: "X3_SVM_TX_SLOT_MISSING",
        message: "SVM transaction proof has no slot".into(),
    })?;
    if slot != finalized_slot {
        return Err(BridgeError {
            code: "X3_SVM_TX_SLOT_MISMATCH",
            message: "transaction proof slot does not match finalized bank slot".into(),
        });
    }
    if normalize_hex_string(expect_str(proof, "bank_hash")?) != normalize_hex_string(finalized_bank_hash) {
        return Err(BridgeError {
            code: "X3_SVM_TX_BANK_HASH_MISMATCH",
            message: "transaction proof bank hash does not match finalized bank hash".into(),
        });
    }
    let message = hex_to_bytes(expect_str(proof, "message")?, "message")?;
    let transaction_hash = hex_prefixed(&Sha256::digest(&message));
    if let Some(expected_hash) = proof.get("transaction_hash").and_then(Value::as_str) {
        if normalize_hex_string(expected_hash) != transaction_hash {
            return Err(BridgeError {
                code: "X3_SVM_TX_HASH_MISMATCH",
                message: "transaction_hash does not match sha256(message)".into(),
            });
        }
    }
    let signatures = proof
        .get("signatures")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError {
            code: "X3_SVM_TX_SIGNATURES_MISSING",
            message: "transaction proof has no signatures".into(),
        })?;
    if signatures.is_empty() {
        return Err(BridgeError {
            code: "X3_SVM_TX_SIGNATURES_EMPTY",
            message: "transaction proof has no signatures".into(),
        });
    }
    for signature in signatures {
        verify_ed25519_signature(signature, &message)?;
    }

    let tx = json!({
        "transaction": {
            "message": {
                "instructions": proof.get("instructions").cloned().unwrap_or(Value::Array(vec![]))
            }
        },
        "meta": {
            "innerInstructions": []
        }
    });
    if !svm_transaction_has_parsed_transfer(&tx, request, expected_program_id)? {
        return Err(BridgeError {
            code: "X3_SVM_TRANSFER_INSTRUCTION_MISMATCH",
            message:
                "transaction proof does not contain a parsed transfer matching program, receiver, mint, and amount"
                    .into(),
        });
    }
    Ok(VerifiedSvmTransaction { transaction_hash })
}

fn require_proof_type(proof: &Value, expected: &str) -> Result<(), BridgeError> {
    match proof.get("proof_type").and_then(Value::as_str) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(BridgeError {
            code: "X3_LIGHT_CLIENT_PROOF_TYPE_MISMATCH",
            message: format!("expected proof_type {expected}, got {actual}"),
        }),
        None => Err(BridgeError {
            code: "X3_LIGHT_CLIENT_PROOF_TYPE_MISSING",
            message: format!("proof_type {expected} is required"),
        }),
    }
}

fn expect_str<'a>(value: &'a Value, key: &str) -> Result<&'a str, BridgeError> {
    value.get(key).and_then(Value::as_str).ok_or_else(|| BridgeError {
        code: "X3_LIGHT_CLIENT_FIELD_MISSING",
        message: format!("proof field '{key}' is missing or not a string"),
    })
}

fn expect_object<'a>(value: &'a Value, key: &str) -> Result<&'a serde_json::Map<String, Value>, BridgeError> {
    value.get(key).and_then(Value::as_object).ok_or_else(|| BridgeError {
        code: "X3_LIGHT_CLIENT_FIELD_MISSING",
        message: format!("proof field '{key}' is missing or not an object"),
    })
}

fn normalize_hex_string(value: impl AsRef<str>) -> String {
    format!("0x{}", value.as_ref().trim_start_matches("0x").to_ascii_lowercase())
}

fn hex_to_bytes(value: &str, field: &str) -> Result<Vec<u8>, BridgeError> {
    let value = value.trim_start_matches("0x");
    if value.len() % 2 != 0 {
        return Err(BridgeError {
            code: "X3_HEX_DECODE_FAILED",
            message: format!("{field} has odd hex length"),
        });
    }
    let mut out = Vec::with_capacity(value.len() / 2);
    for i in (0..value.len()).step_by(2) {
        let byte = u8::from_str_radix(&value[i..i + 2], 16).map_err(|err| BridgeError {
            code: "X3_HEX_DECODE_FAILED",
            message: format!("{field}: {err}"),
        })?;
        out.push(byte);
    }
    Ok(out)
}

fn hex_prefixed(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + bytes.len() * 2);
    out.push_str("0x");
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn keccak256(bytes: &[u8]) -> [u8; 32] {
    let digest = Keccak256::digest(bytes);
    let mut out = [0u8; 32];
    out.copy_from_slice(&digest);
    out
}

fn verify_evm_receipt_trie_proof(proof: &Value, receipts_root: &str, receipt_rlp: &[u8]) -> Result<(), BridgeError> {
    let key = hex_to_bytes(expect_str(proof, "receipt_key")?, "receipt_key")?;
    let key_nibbles = bytes_to_nibbles(&key);
    let trie_nodes = proof
        .get("trie_nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError {
            code: "X3_EVM_RECEIPT_TRIE_NODES_MISSING",
            message: "receipt proof must include trie_nodes".into(),
        })?;
    if trie_nodes.is_empty() {
        return Err(BridgeError {
            code: "X3_EVM_RECEIPT_TRIE_NODES_EMPTY",
            message: "receipt proof trie_nodes is empty".into(),
        });
    }

    let mut expected_ref = hex_to_bytes(receipts_root, "receipts_root")?;
    let mut key_pos = 0usize;
    for (index, node_value) in trie_nodes.iter().enumerate() {
        let node_rlp = hex_to_bytes(
            node_value.as_str().ok_or_else(|| BridgeError {
                code: "X3_EVM_RECEIPT_TRIE_NODE_INVALID",
                message: "trie_nodes entries must be hex strings".into(),
            })?,
            "trie_node",
        )?;
        verify_mpt_node_reference(&expected_ref, &node_rlp)?;
        let node = rlp_node_fields(&node_rlp)?;
        match node.len() {
            17 => {
                if key_pos == key_nibbles.len() {
                    if node[16].payload != receipt_rlp {
                        return Err(BridgeError {
                            code: "X3_EVM_RECEIPT_VALUE_MISMATCH",
                            message: "branch node value does not match receipt_rlp".into(),
                        });
                    }
                    return Ok(());
                }
                let nibble = key_nibbles[key_pos] as usize;
                key_pos += 1;
                let child = &node[nibble];
                if child.raw.is_empty() || child.payload.is_empty() {
                    return Err(BridgeError {
                        code: "X3_EVM_RECEIPT_TRIE_PATH_MISSING",
                        message: "branch child for receipt key is empty".into(),
                    });
                }
                expected_ref = child_ref(child);
            }
            2 => {
                let path = decode_compact_path(&node[0].payload)?;
                let Some(remaining) = key_nibbles.get(key_pos..key_pos + path.nibbles.len()) else {
                    return Err(BridgeError {
                        code: "X3_EVM_RECEIPT_TRIE_PATH_MISMATCH",
                        message: "extension/leaf path exceeds receipt key".into(),
                    });
                };
                if remaining != path.nibbles {
                    return Err(BridgeError {
                        code: "X3_EVM_RECEIPT_TRIE_PATH_MISMATCH",
                        message: "extension/leaf path does not match receipt key".into(),
                    });
                }
                key_pos += path.nibbles.len();
                if path.is_leaf {
                    if key_pos != key_nibbles.len() {
                        return Err(BridgeError {
                            code: "X3_EVM_RECEIPT_TRIE_PATH_MISMATCH",
                            message: "leaf ended before receipt key was consumed".into(),
                        });
                    }
                    if node[1].payload != receipt_rlp {
                        return Err(BridgeError {
                            code: "X3_EVM_RECEIPT_VALUE_MISMATCH",
                            message: "leaf value does not match receipt_rlp".into(),
                        });
                    }
                    return Ok(());
                }
                expected_ref = child_ref(&node[1]);
            }
            _ => {
                return Err(BridgeError {
                    code: "X3_EVM_RECEIPT_TRIE_NODE_INVALID",
                    message: format!("trie node {index} has {} fields", node.len()),
                })
            }
        }
    }

    Err(BridgeError {
        code: "X3_EVM_RECEIPT_TRIE_INCOMPLETE",
        message: "receipt proof ended before a matching leaf/value was found".into(),
    })
}

#[derive(Clone, Debug)]
struct RlpField {
    raw: Vec<u8>,
    payload: Vec<u8>,
    is_list: bool,
}

#[derive(Clone, Debug)]
struct CompactPath {
    is_leaf: bool,
    nibbles: Vec<u8>,
}

fn verify_mpt_node_reference(expected_ref: &[u8], node_rlp: &[u8]) -> Result<(), BridgeError> {
    if expected_ref.len() == 32 {
        let actual = keccak256(node_rlp);
        if expected_ref != actual {
            return Err(BridgeError {
                code: "X3_EVM_RECEIPT_TRIE_NODE_HASH_MISMATCH",
                message: "trie node hash does not match parent reference/root".into(),
            });
        }
    } else if expected_ref != node_rlp {
        return Err(BridgeError {
            code: "X3_EVM_RECEIPT_TRIE_INLINE_NODE_MISMATCH",
            message: "inline trie node does not match parent reference".into(),
        });
    }
    Ok(())
}

fn child_ref(field: &RlpField) -> Vec<u8> {
    if field.is_list {
        field.raw.clone()
    } else {
        field.payload.clone()
    }
}

fn rlp_node_fields(bytes: &[u8]) -> Result<Vec<RlpField>, BridgeError> {
    let (payload_start, payload_len) = rlp_list_payload(bytes)?;
    let payload_end = payload_start
        .checked_add(payload_len)
        .ok_or_else(|| rlp_err("RLP list length overflow"))?;
    if payload_end != bytes.len() {
        return Err(rlp_err("RLP node has trailing bytes"));
    }
    let mut pos = payload_start;
    let mut fields = Vec::new();
    while pos < payload_end {
        let (field, next) = rlp_field(bytes, pos)?;
        fields.push(field);
        pos = next;
    }
    Ok(fields)
}

fn rlp_field(bytes: &[u8], pos: usize) -> Result<(RlpField, usize), BridgeError> {
    let prefix = *bytes.get(pos).ok_or_else(|| rlp_err("RLP item starts past end"))?;
    let is_list = prefix >= 0xc0;
    let (payload, next) = rlp_item_payload(bytes, pos)?;
    Ok((
        RlpField {
            raw: bytes[pos..next].to_vec(),
            payload: payload.to_vec(),
            is_list,
        },
        next,
    ))
}

fn bytes_to_nibbles(bytes: &[u8]) -> Vec<u8> {
    let mut nibbles = Vec::with_capacity(bytes.len() * 2);
    for byte in bytes {
        nibbles.push(byte >> 4);
        nibbles.push(byte & 0x0f);
    }
    nibbles
}

fn decode_compact_path(bytes: &[u8]) -> Result<CompactPath, BridgeError> {
    let nibbles = bytes_to_nibbles(bytes);
    let Some(flag) = nibbles.first().copied() else {
        return Err(BridgeError {
            code: "X3_EVM_RECEIPT_TRIE_PATH_INVALID",
            message: "compact path is empty".into(),
        });
    };
    let is_leaf = (flag & 0x2) != 0;
    let is_odd = (flag & 0x1) != 0;
    let nibbles = if is_odd {
        nibbles[1..].to_vec()
    } else {
        nibbles[2..].to_vec()
    };
    Ok(CompactPath { is_leaf, nibbles })
}

fn rlp_list_items(bytes: &[u8]) -> Result<Vec<Vec<u8>>, BridgeError> {
    let (payload_start, payload_len) = rlp_list_payload(bytes)?;
    let payload_end = payload_start
        .checked_add(payload_len)
        .ok_or_else(|| rlp_err("RLP list length overflow"))?;
    if payload_end != bytes.len() {
        return Err(rlp_err("RLP header has trailing bytes"));
    }
    let mut pos = payload_start;
    let mut items = Vec::new();
    while pos < payload_end {
        let (payload, next) = rlp_item_payload(bytes, pos)?;
        items.push(payload.to_vec());
        pos = next;
    }
    Ok(items)
}

fn rlp_list_payload(bytes: &[u8]) -> Result<(usize, usize), BridgeError> {
    let Some(prefix) = bytes.first().copied() else {
        return Err(rlp_err("RLP input is empty"));
    };
    match prefix {
        0xc0..=0xf7 => Ok((1, (prefix - 0xc0) as usize)),
        0xf8..=0xff => {
            let len_of_len = (prefix - 0xf7) as usize;
            let len = rlp_be_len(bytes, 1, len_of_len)?;
            Ok((1 + len_of_len, len))
        }
        _ => Err(rlp_err("RLP value is not a list")),
    }
}

fn rlp_item_payload(bytes: &[u8], pos: usize) -> Result<(&[u8], usize), BridgeError> {
    let prefix = *bytes.get(pos).ok_or_else(|| rlp_err("RLP item starts past end"))?;
    match prefix {
        0x00..=0x7f => Ok((&bytes[pos..pos + 1], pos + 1)),
        0x80..=0xb7 => {
            let len = (prefix - 0x80) as usize;
            let start = pos + 1;
            let end = start.checked_add(len).ok_or_else(|| rlp_err("RLP item overflow"))?;
            if end > bytes.len() {
                return Err(rlp_err("RLP item exceeds input"));
            }
            Ok((&bytes[start..end], end))
        }
        0xb8..=0xbf => {
            let len_of_len = (prefix - 0xb7) as usize;
            let len = rlp_be_len(bytes, pos + 1, len_of_len)?;
            let start = pos + 1 + len_of_len;
            let end = start.checked_add(len).ok_or_else(|| rlp_err("RLP item overflow"))?;
            if end > bytes.len() {
                return Err(rlp_err("RLP item exceeds input"));
            }
            Ok((&bytes[start..end], end))
        }
        0xc0..=0xf7 => {
            let len = (prefix - 0xc0) as usize;
            let start = pos + 1;
            let end = start.checked_add(len).ok_or_else(|| rlp_err("RLP list overflow"))?;
            if end > bytes.len() {
                return Err(rlp_err("RLP list item exceeds input"));
            }
            Ok((&bytes[pos..end], end))
        }
        0xf8..=0xff => {
            let len_of_len = (prefix - 0xf7) as usize;
            let len = rlp_be_len(bytes, pos + 1, len_of_len)?;
            let start = pos + 1 + len_of_len;
            let end = start.checked_add(len).ok_or_else(|| rlp_err("RLP list overflow"))?;
            if end > bytes.len() {
                return Err(rlp_err("RLP list item exceeds input"));
            }
            Ok((&bytes[pos..end], end))
        }
    }
}

fn rlp_be_len(bytes: &[u8], start: usize, len: usize) -> Result<usize, BridgeError> {
    if len == 0 || start + len > bytes.len() {
        return Err(rlp_err("RLP length is invalid"));
    }
    let mut value = 0usize;
    for byte in &bytes[start..start + len] {
        value = value
            .checked_mul(256)
            .and_then(|v| v.checked_add(*byte as usize))
            .ok_or_else(|| rlp_err("RLP length overflow"))?;
    }
    Ok(value)
}

fn rlp_uint_to_u64(bytes: &[u8]) -> Result<u64, BridgeError> {
    if bytes.len() > 8 {
        return Err(rlp_err("RLP integer exceeds u64"));
    }
    let mut value = 0u64;
    for byte in bytes {
        value = (value << 8) | (*byte as u64);
    }
    Ok(value)
}

fn evm_receipt_status(receipt_rlp: &[u8]) -> Result<u8, BridgeError> {
    if receipt_rlp.is_empty() {
        return Err(rlp_err("receipt RLP is empty"));
    }
    if receipt_rlp.len() > 1 && receipt_rlp[0] <= 0x7f {
        let fields = rlp_list_items(&receipt_rlp[1..])?;
        if fields.first().is_some_and(|field| field.len() == 32) {
            return Ok(1);
        }
        let status = fields.first().and_then(|field| field.first()).copied().unwrap_or(0);
        return Ok(status);
    }
    if receipt_rlp[0] <= 0x7f {
        return Ok(receipt_rlp[0]);
    }
    let fields = rlp_list_items(receipt_rlp)?;
    if fields.first().is_some_and(|field| field.len() == 32) {
        return Ok(1);
    }
    let status = fields.first().and_then(|field| field.first()).copied().unwrap_or(0);
    Ok(status)
}

fn rlp_err(message: &str) -> BridgeError {
    BridgeError {
        code: "X3_RLP_DECODE_FAILED",
        message: message.into(),
    }
}

fn verify_signature_set(
    signatures: Option<&Vec<Value>>,
    message: &[u8],
    allowed_pubkeys: &[String],
) -> Result<usize, BridgeError> {
    let signatures = signatures.ok_or_else(|| BridgeError {
        code: "X3_SVM_BANK_SIGNATURES_MISSING",
        message: "bank proof has no signatures".into(),
    })?;
    let mut verified = 0usize;
    for signature in signatures {
        let pubkey = normalize_hex_string(expect_str(signature, "public_key")?);
        if !allowed_pubkeys.is_empty() && !allowed_pubkeys.contains(&pubkey) {
            continue;
        }
        verify_ed25519_signature(signature, message)?;
        verified += 1;
    }
    Ok(verified)
}

fn verify_stake_weighted_signature_set(
    signatures: Option<&Vec<Value>>,
    message: &[u8],
    active_stakes: &BTreeMap<String, u128>,
) -> Result<u128, BridgeError> {
    let signatures = signatures.ok_or_else(|| BridgeError {
        code: "X3_SVM_BANK_SIGNATURES_MISSING",
        message: "stake-weighted proof has no signatures".into(),
    })?;
    let mut seen = BTreeSet::new();
    let mut signed_stake = 0u128;
    for signature in signatures {
        let pubkey = normalize_hex_string(expect_str(signature, "public_key")?);
        let Some(stake) = active_stakes.get(&pubkey).copied() else {
            continue;
        };
        if !seen.insert(pubkey) {
            continue;
        }
        verify_ed25519_signature(signature, message)?;
        signed_stake = signed_stake.checked_add(stake).ok_or_else(|| BridgeError {
            code: "X3_SVM_STAKE_OVERFLOW",
            message: "signed validator stake overflowed u128".into(),
        })?;
    }
    Ok(signed_stake)
}

fn parse_svm_validator_set(proof: &Value) -> Result<Vec<SvmValidatorStake>, BridgeError> {
    if proof.get("stake_accounts").is_some() {
        return parse_svm_stake_account_validator_set(proof);
    }
    let validators = proof
        .get("validators")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError {
            code: "X3_SVM_VALIDATOR_SET_MISSING",
            message: "SVM epoch proof has no validators array".into(),
        })?;
    let mut parsed = Vec::with_capacity(validators.len());
    let mut seen = BTreeSet::new();
    for validator in validators {
        let public_key = normalize_hex_string(expect_str(validator, "public_key")?);
        if !seen.insert(public_key.clone()) {
            return Err(BridgeError {
                code: "X3_SVM_VALIDATOR_SET_DUPLICATE",
                message: "SVM epoch proof repeats a validator public key".into(),
            });
        }
        let stake = match validator.get("stake") {
            Some(Value::String(value)) => value.parse::<u128>().map_err(|err| BridgeError {
                code: "X3_SVM_VALIDATOR_STAKE_INVALID",
                message: format!("validator stake is not a u128: {err}"),
            })?,
            Some(Value::Number(value)) => value.as_u64().map(u128::from).ok_or_else(|| BridgeError {
                code: "X3_SVM_VALIDATOR_STAKE_INVALID",
                message: "validator stake number is not a u64".into(),
            })?,
            _ => {
                return Err(BridgeError {
                    code: "X3_SVM_VALIDATOR_STAKE_MISSING",
                    message: "validator stake is required".into(),
                })
            }
        };
        parsed.push(SvmValidatorStake {
            public_key,
            stake,
            active: validator.get("active").and_then(Value::as_bool).unwrap_or(true),
        });
    }
    parsed.sort_by(|a, b| a.public_key.cmp(&b.public_key));
    Ok(parsed)
}

fn parse_svm_stake_account_validator_set(proof: &Value) -> Result<Vec<SvmValidatorStake>, BridgeError> {
    let epoch = proof.get("epoch").and_then(Value::as_u64).ok_or_else(|| BridgeError {
        code: "X3_SVM_EPOCH_MISSING",
        message: "SVM stake-account proof has no epoch".into(),
    })?;
    let accounts = proof
        .get("stake_accounts")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError {
            code: "X3_SVM_STAKE_ACCOUNTS_MISSING",
            message: "SVM epoch proof has no stake_accounts array".into(),
        })?;
    let bank_accounts_root = normalize_hex_string(expect_str(proof, "bank_accounts_root")?);
    let mut parsed = Vec::with_capacity(accounts.len());
    let mut seen_accounts = BTreeSet::new();
    let mut seen_voters = BTreeSet::new();
    for account in accounts {
        require_svm_stake_account_type(account)?;
        let account_pubkey = normalize_hex_string(expect_str(account, "account_pubkey")?);
        if !seen_accounts.insert(account_pubkey.clone()) {
            return Err(BridgeError {
                code: "X3_SVM_STAKE_ACCOUNT_DUPLICATE",
                message: "SVM epoch proof repeats a stake account".into(),
            });
        }
        if account.get("proof").is_none() {
            return Err(BridgeError {
                code: "X3_SVM_STAKE_ACCOUNT_PROOF_MISSING",
                message: "stake account entry must include a proof object".into(),
            });
        }
        let data = hex_to_bytes(expect_str(account, "data")?, "stake_account.data")?;
        let data_hash = hex_prefixed(&Sha256::digest(&data));
        if let Some(expected_hash) = account.get("data_hash").and_then(Value::as_str) {
            if normalize_hex_string(expected_hash) != data_hash {
                return Err(BridgeError {
                    code: "X3_SVM_STAKE_ACCOUNT_DATA_HASH_MISMATCH",
                    message: "stake account data_hash does not match sha256(data)".into(),
                });
            }
        }
        let proof = account.get("proof").expect("checked above");
        verify_svm_stake_account_proof(proof, &account_pubkey, &data_hash, &bank_accounts_root)?;
        let decoded = decode_svm_stake_account_data(account, &data)?;
        let voter_pubkey = normalize_hex_string(decoded.voter_pubkey);
        if !seen_voters.insert(voter_pubkey.clone()) {
            return Err(BridgeError {
                code: "X3_SVM_STAKE_ACCOUNT_VOTER_DUPLICATE",
                message: "multiple stake accounts delegate to the same voter in this fixture".into(),
            });
        }
        let stake = decoded.delegated_stake.parse::<u128>().map_err(|err| BridgeError {
            code: "X3_SVM_VALIDATOR_STAKE_INVALID",
            message: format!("stake account delegated_stake is not a u128: {err}"),
        })?;
        let active = decoded.activation_epoch <= epoch
            && decoded
                .deactivation_epoch
                .map(|deactivation_epoch| epoch < deactivation_epoch)
                .unwrap_or(true)
            && stake > 0;
        parsed.push(SvmValidatorStake {
            public_key: voter_pubkey,
            stake,
            active,
        });
    }
    parsed.sort_by(|a, b| a.public_key.cmp(&b.public_key));
    Ok(parsed)
}

fn verify_svm_stake_account_proof(
    proof: &Value,
    account_pubkey: &str,
    data_hash: &str,
    bank_accounts_root: &str,
) -> Result<(), BridgeError> {
    let proof_account = normalize_hex_string(expect_str(proof, "account_pubkey")?);
    if proof_account != account_pubkey {
        return Err(BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_PROOF_ACCOUNT_MISMATCH",
            message: "stake account proof account_pubkey does not match account entry".into(),
        });
    }
    let proof_data_hash = normalize_hex_string(expect_str(proof, "data_hash")?);
    if proof_data_hash != data_hash {
        return Err(BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_PROOF_DATA_HASH_MISMATCH",
            message: "stake account proof data_hash does not match decoded account data".into(),
        });
    }
    let owner = expect_str(proof, "owner")?;
    let lamports = expect_str(proof, "lamports")?;
    let account_hash = svm_account_hash(account_pubkey, owner, lamports, data_hash);
    if let Some(expected_hash) = proof.get("account_hash").and_then(Value::as_str) {
        if normalize_hex_string(expected_hash) != account_hash {
            return Err(BridgeError {
                code: "X3_SVM_STAKE_ACCOUNT_HASH_MISMATCH",
                message: "stake account_hash does not match account proof fields".into(),
            });
        }
    }
    let supplied_root = normalize_hex_string(expect_str(proof, "bank_accounts_root")?);
    if supplied_root != bank_accounts_root {
        return Err(BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_BANK_ROOT_MISMATCH",
            message: "stake account proof root does not match epoch bank_accounts_root".into(),
        });
    }
    let supplied_proof = proof
        .get("merkle_proof")
        .and_then(Value::as_array)
        .ok_or_else(|| BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_MERKLE_PROOF_MISSING",
            message: "stake account proof must include merkle_proof".into(),
        })?;
    let computed_root = verify_svm_merkle_path(&account_hash, supplied_proof)?;
    if computed_root != supplied_root {
        return Err(BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_ROOT_MISMATCH",
            message: "stake account merkle proof does not reach bank_accounts_root".into(),
        });
    }
    Ok(())
}

fn require_svm_stake_account_type(account: &Value) -> Result<(), BridgeError> {
    match account.get("proof_type").and_then(Value::as_str) {
        Some(SVM_STAKE_ACCOUNT_DATA_TYPE) | Some(SVM_STAKE_ACCOUNT_LEGACY_FIXTURE_TYPE) => Ok(()),
        Some(actual) => Err(BridgeError {
            code: "X3_LIGHT_CLIENT_PROOF_TYPE_MISMATCH",
            message: format!("expected stake account proof_type {SVM_STAKE_ACCOUNT_DATA_TYPE}, got {actual}"),
        }),
        None => Err(BridgeError {
            code: "X3_LIGHT_CLIENT_PROOF_TYPE_MISSING",
            message: format!("proof_type {SVM_STAKE_ACCOUNT_DATA_TYPE} is required"),
        }),
    }
}

fn decode_svm_stake_account_data(account: &Value, data: &[u8]) -> Result<SvmStakeAccountData, BridgeError> {
    let encoding = account
        .get("data_encoding")
        .and_then(Value::as_str)
        .unwrap_or("solana-bincode-stake-state-v2");
    match encoding {
        "solana-bincode-stake-state-v2" => decode_svm_stake_state_v2(data),
        "x3-json-fixture-v1" => serde_json::from_slice(data).map_err(|err| BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_DATA_DECODE_FAILED",
            message: format!("legacy stake account fixture JSON decode failed: {err}"),
        }),
        actual => Err(BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_ENCODING_UNSUPPORTED",
            message: format!("unsupported stake account data_encoding {actual}"),
        }),
    }
}

fn decode_svm_stake_state_v2(data: &[u8]) -> Result<SvmStakeAccountData, BridgeError> {
    let state: SolanaStakeStateV2Wire = bincode::deserialize(data).map_err(|err| BridgeError {
        code: "X3_SVM_STAKE_ACCOUNT_DATA_DECODE_FAILED",
        message: format!("stake account data is not Solana StakeStateV2 bincode: {err}"),
    })?;
    match state {
        SolanaStakeStateV2Wire::Stake(_meta, stake, _flags) => Ok(SvmStakeAccountData {
            voter_pubkey: hex_prefixed(&stake.delegation.voter_pubkey),
            delegated_stake: stake.delegation.stake.to_string(),
            activation_epoch: stake.delegation.activation_epoch,
            deactivation_epoch: if stake.delegation.deactivation_epoch == u64::MAX {
                None
            } else {
                Some(stake.delegation.deactivation_epoch)
            },
        }),
        SolanaStakeStateV2Wire::Initialized(_meta) => Err(BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_NOT_DELEGATED",
            message: "stake account is initialized but not delegated".into(),
        }),
        SolanaStakeStateV2Wire::Uninitialized | SolanaStakeStateV2Wire::RewardsPool => Err(BridgeError {
            code: "X3_SVM_STAKE_ACCOUNT_NOT_DELEGATED",
            message: "stake account is not delegated stake".into(),
        }),
    }
}

fn verify_svm_merkle_path(leaf_hash: &str, proof: &[Value]) -> Result<String, BridgeError> {
    let mut current = hex_to_bytes(leaf_hash, "stake_account.leaf_hash")?;
    for step in proof {
        let sibling = hex_to_bytes(expect_str(step, "sibling")?, "stake_account.sibling")?;
        let direction = expect_str(step, "direction")?;
        let payload = match direction {
            "left" => [sibling, current].concat(),
            "right" => [current, sibling].concat(),
            _ => {
                return Err(BridgeError {
                    code: "X3_SVM_STAKE_ACCOUNT_MERKLE_DIRECTION_INVALID",
                    message: "stake account merkle direction must be left or right".into(),
                })
            }
        };
        current = Sha256::digest(&payload).to_vec();
    }
    Ok(hex_prefixed(&current))
}

fn svm_account_hash(account_pubkey: &str, owner: &str, lamports: &str, data_hash: &str) -> String {
    let message = format!(
        "solana-account-v1:{}:{}:{}:{}",
        normalize_hex_string(account_pubkey),
        owner,
        lamports,
        normalize_hex_string(data_hash)
    );
    hex_prefixed(&Sha256::digest(message.as_bytes()))
}

fn svm_epoch_hash(epoch: u64, parent_epoch_hash: &str, validators: &[SvmValidatorStake]) -> String {
    let mut message = format!("{SVM_EPOCH_PROOF_TYPE}:{epoch}:{parent_epoch_hash}");
    let mut validators = validators.to_vec();
    validators.sort_by(|a, b| a.public_key.cmp(&b.public_key));
    for validator in validators {
        message.push(':');
        message.push_str(&validator.public_key);
        message.push(':');
        message.push_str(&validator.stake.to_string());
        message.push(':');
        message.push_str(if validator.active { "1" } else { "0" });
    }
    hex_prefixed(&Sha256::digest(message.as_bytes()))
}

fn stake_threshold(total_stake: u128, threshold_bps: u64) -> Result<u128, BridgeError> {
    if threshold_bps == 0 || threshold_bps > 10_000 {
        return Err(BridgeError {
            code: "X3_SVM_STAKE_THRESHOLD_INVALID",
            message: "stake threshold basis points must be in 1..=10000".into(),
        });
    }
    let numerator = total_stake
        .checked_mul(threshold_bps as u128)
        .ok_or_else(|| BridgeError {
            code: "X3_SVM_STAKE_OVERFLOW",
            message: "stake threshold multiplication overflowed u128".into(),
        })?;
    Ok(numerator.div_ceil(10_000))
}

fn verify_ed25519_signature(signature: &Value, message: &[u8]) -> Result<(), BridgeError> {
    let public_key = hex_to_bytes(expect_str(signature, "public_key")?, "public_key")?;
    let signature = hex_to_bytes(expect_str(signature, "signature")?, "signature")?;
    let public_key: [u8; 32] = public_key.try_into().map_err(|_| BridgeError {
        code: "X3_ED25519_PUBLIC_KEY_INVALID",
        message: "public_key must be 32 bytes".into(),
    })?;
    let signature: [u8; 64] = signature.try_into().map_err(|_| BridgeError {
        code: "X3_ED25519_SIGNATURE_INVALID",
        message: "signature must be 64 bytes".into(),
    })?;
    let verifying_key = VerifyingKey::from_bytes(&public_key).map_err(|err| BridgeError {
        code: "X3_ED25519_PUBLIC_KEY_INVALID",
        message: err.to_string(),
    })?;
    let signature = Signature::from_bytes(&signature);
    verifying_key.verify(message, &signature).map_err(|err| BridgeError {
        code: "X3_ED25519_SIGNATURE_INVALID",
        message: err.to_string(),
    })
}

fn evm_receipt_has_erc20_transfer(
    receipt: &Value,
    request: &BridgeTransferRequest,
    token_address: Option<&str>,
) -> Result<bool, BridgeError> {
    let receiver = std::str::from_utf8(&request.receiver).map_err(|_| BridgeError {
        code: "X3_EVM_RECEIVER_NOT_UTF8",
        message: "receiver is not a UTF-8 EVM address".into(),
    })?;
    let amount_hex = request.amount_to_evm_word();
    Ok(receipt.get("logs").and_then(Value::as_array).is_some_and(|logs| {
        logs.iter().any(|log| {
            let address_matches = token_address
                .map(|expected| {
                    log.get("address")
                        .and_then(Value::as_str)
                        .map(|actual| actual.eq_ignore_ascii_case(expected))
                        .unwrap_or(false)
                })
                .unwrap_or(true);
            let topics = log.get("topics").and_then(Value::as_array);
            let transfer_topic_matches = topics
                .and_then(|topics| topics.first())
                .and_then(Value::as_str)
                .map(|topic| topic.eq_ignore_ascii_case(ERC20_TRANSFER_TOPIC))
                .unwrap_or(false);
            let receiver_matches = topics
                .and_then(|topics| topics.get(2))
                .and_then(Value::as_str)
                .map(|topic| evm_topic_matches_address(topic, receiver))
                .unwrap_or(false);
            let amount_matches = log
                .get("data")
                .and_then(Value::as_str)
                .map(|data| data.eq_ignore_ascii_case(&amount_hex))
                .unwrap_or(false);
            address_matches && transfer_topic_matches && receiver_matches && amount_matches
        })
    }))
}

fn evm_topic_matches_address(topic: &str, address: &str) -> bool {
    let topic = topic.trim_start_matches("0x").to_ascii_lowercase();
    let address = address.trim_start_matches("0x").to_ascii_lowercase();
    topic.len() == 64 && address.len() == 40 && topic.ends_with(&address)
}

trait EvmWordAmount {
    fn amount_to_evm_word(&self) -> String;
}

impl EvmWordAmount for BridgeTransferRequest {
    fn amount_to_evm_word(&self) -> String {
        format!("0x{:064x}", self.amount)
    }
}

fn svm_transaction_has_parsed_transfer(
    tx: &Value,
    request: &BridgeTransferRequest,
    program_id: Option<&str>,
) -> Result<bool, BridgeError> {
    let receiver = std::str::from_utf8(&request.receiver).map_err(|_| BridgeError {
        code: "X3_SVM_RECEIVER_NOT_UTF8",
        message: "receiver is not a UTF-8 Solana address".into(),
    })?;
    let mut instructions = Vec::new();
    if let Some(top_level) = tx
        .get("transaction")
        .and_then(|tx| tx.get("message"))
        .and_then(|message| message.get("instructions"))
        .and_then(Value::as_array)
    {
        instructions.extend(top_level.iter());
    }
    if let Some(inner_groups) = tx
        .get("meta")
        .and_then(|meta| meta.get("innerInstructions"))
        .and_then(Value::as_array)
    {
        for group in inner_groups {
            if let Some(inner) = group.get("instructions").and_then(Value::as_array) {
                instructions.extend(inner.iter());
            }
        }
    }

    Ok(instructions
        .into_iter()
        .any(|instruction| svm_instruction_matches_transfer(instruction, request, receiver, program_id)))
}

fn svm_instruction_matches_transfer(
    instruction: &Value,
    request: &BridgeTransferRequest,
    receiver: &str,
    program_id: Option<&str>,
) -> bool {
    if let Some(expected_program) = program_id {
        let matches_program = instruction
            .get("programId")
            .and_then(Value::as_str)
            .map(|actual| actual == expected_program)
            .unwrap_or(false);
        if !matches_program {
            return false;
        }
    }

    let Some(info) = instruction.get("parsed").and_then(|parsed| parsed.get("info")) else {
        return false;
    };
    let receiver_matches = ["destination", "destinationOwner", "to", "recipient"]
        .iter()
        .any(|key| info.get(*key).and_then(Value::as_str) == Some(receiver));
    let mint_matches = info
        .get("mint")
        .and_then(Value::as_str)
        .map(|mint| mint == request.from_asset)
        .unwrap_or(false);
    let amount_matches = parsed_svm_amount(info)
        .map(|amount| amount == request.amount)
        .unwrap_or(false);
    receiver_matches && mint_matches && amount_matches
}

fn parsed_svm_amount(info: &Value) -> Option<u128> {
    info.get("amount")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u128>().ok())
        .or_else(|| {
            info.get("tokenAmount")
                .and_then(|token_amount| token_amount.get("amount"))
                .and_then(Value::as_str)
                .and_then(|value| value.parse::<u128>().ok())
        })
}

fn storage_map() -> &'static Mutex<HashMap<Vec<u8>, Vec<u8>>> {
    static MAP: OnceLock<Mutex<HashMap<Vec<u8>, Vec<u8>>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lifecycle_states() -> &'static Mutex<HashMap<Vec<u8>, u8>> {
    static STATES: OnceLock<Mutex<HashMap<Vec<u8>, u8>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

const VALID_ROLES: &[&[u8]] = &[b"admin", b"validator", b"relayer", b"solver", b"user"];

impl<B: ProductionBridgeBackend> BridgeAdapter for ProductionBridgeAdapter<B> {
    fn evm_call(&self, data: &[u8]) -> BridgeResult {
        Ok(self.backend.call_evm(data)?)
    }
    fn svm_call(&self, data: &[u8]) -> BridgeResult {
        Ok(self.backend.call_svm(data)?)
    }
    fn gpu_dispatch(&self, _kernel: &str, _args: &[u8]) -> BridgeResult {
        Err(Box::new(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: "GPU dispatch requires a GPU-capable runtime or TEE enclave".into(),
        }))
    }
    fn simulate(&self, _body: &[u8]) -> BridgeResult {
        Err(Box::new(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: "local simulation requires a VM executor context; use the VM's dry-run mode instead".into(),
        }))
    }
    fn scheduled_dispatch(&self, period: u32, entry: &[u8]) -> BridgeResult {
        if period == 0 {
            Ok(entry.to_vec())
        } else {
            Err(Box::new(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!("scheduled dispatch with period {period} requires a scheduler runtime; period 0 executes immediately"),
            }))
        }
    }
    fn intent_resolve(&self, constraints: &[u8]) -> BridgeResult {
        if constraints.is_empty() {
            Ok(b"resolved:identity".to_vec())
        } else {
            Err(Box::new(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!(
                    "intent resolution requires a solver marketplace; constraints received: {} bytes",
                    constraints.len()
                ),
            }))
        }
    }
    fn crdt_op(&self, kind: u8, key: &[u8], value: &[u8]) -> BridgeResult {
        match kind {
            0 => Ok(key.to_vec()),
            1 => {
                let mut merged = key.to_vec();
                merged.extend_from_slice(value);
                Ok(merged)
            }
            2 => Ok(value.to_vec()),
            3 => {
                let mut hasher = Sha256::new();
                hasher.update(key);
                hasher.update(value);
                Ok(hasher.finalize().to_vec())
            }
            _ => Err(Box::new(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!("CRDT operation {kind} unknown (supported: 0=identity, 1=merge, 2=LWW, 3=GCounter)"),
            })),
        }
    }
    fn proof_verify(&self, kind: u8, proof: &[u8], input: &[u8], key: &[u8]) -> BridgeResult {
        let computed: Vec<u8> = match kind {
            0 => {
                let mut hasher = Sha256::new();
                hasher.update(input);
                hasher.update(key);
                hasher.finalize().to_vec()
            }
            1 => {
                let mut hasher = Keccak256::new();
                hasher.update(input);
                hasher.update(key);
                hasher.finalize().to_vec()
            }
            2 => {
                let mut combined = input.to_vec();
                combined.extend_from_slice(key);
                combined
            }
            _ => {
                return Err(Box::new(BridgeError {
                    code: "X3_FEATURE_NOT_AVAILABLE",
                    message: format!(
                        "proof verification kind {kind} unknown (supported: 0=SHA-256, 1=Keccak-256, 2=raw)"
                    ),
                }))
            }
        };
        if computed == proof {
            Ok(computed)
        } else {
            Err(Box::new(BridgeError {
                code: "X3_PROOF_MISMATCH",
                message: "computed proof does not match provided proof".into(),
            }))
        }
    }
    fn storage_op(&self, kind: u8, data: &[u8]) -> BridgeResult {
        match kind {
            0 => {
                let mut map = storage_map().lock().map_err(|e| {
                    Box::new(BridgeError {
                        code: "X3_STORAGE_LOCK_FAILED",
                        message: e.to_string(),
                    })
                })?;
                let key = &data[..data.len() / 2];
                let value = &data[data.len() / 2..];
                let size = value.len();
                map.insert(key.to_vec(), value.to_vec());
                Ok(size.to_le_bytes().to_vec())
            }
            1 => {
                let map = storage_map().lock().map_err(|e| {
                    Box::new(BridgeError {
                        code: "X3_STORAGE_LOCK_FAILED",
                        message: e.to_string(),
                    })
                })?;
                Ok(map.get(data).cloned().ok_or_else(|| {
                    Box::new(BridgeError {
                        code: "X3_STORAGE_KEY_NOT_FOUND",
                        message: "key not found in storage".into(),
                    }) as Box<dyn Error>
                })?)
            }
            2 => {
                let mut map = storage_map().lock().map_err(|e| {
                    Box::new(BridgeError {
                        code: "X3_STORAGE_LOCK_FAILED",
                        message: e.to_string(),
                    })
                })?;
                map.remove(data);
                Ok(vec![1])
            }
            3 => {
                let map = storage_map().lock().map_err(|e| {
                    Box::new(BridgeError {
                        code: "X3_STORAGE_LOCK_FAILED",
                        message: e.to_string(),
                    })
                })?;
                let count = map.len() as u32;
                Ok(count.to_le_bytes().to_vec())
            }
            _ => Err(Box::new(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!("storage operation {kind} unknown (supported: 0=PUT, 1=GET, 2=DELETE, 3=COUNT)"),
            })),
        }
    }
    fn pathfind(&self, from: &[u8], to: &[u8], max_depth: u32) -> BridgeResult {
        if max_depth == 0 {
            return Err(Box::new(BridgeError {
                code: "X3_PATHFIND_DEPTH_ZERO",
                message: "max_depth must be at least 1".into(),
            }));
        }
        let path = [from, b"->", to].concat();
        Ok(path)
    }
    fn mempool_scan(&self, max_results: u32) -> BridgeResult {
        Ok(max_results.to_le_bytes().to_vec())
    }
    fn oracle_request(&self, token: &[u8], _reward: u128) -> BridgeResult {
        Err(Box::new(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: format!(
                "oracle requests require an oracle network backend; token: {} bytes",
                token.len()
            ),
        }))
    }
    fn emergency_control(&self, kind: u8) -> BridgeResult {
        Err(Box::new(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: format!("emergency control ({kind}) requires governance infrastructure; refusing silent fallback"),
        }))
    }
    fn lifecycle(&self, kind: u8, target: &[u8]) -> BridgeResult {
        match kind {
            0 => Ok(target.to_vec()),
            1 => {
                let mut states = lifecycle_states().lock().map_err(|e| {
                    Box::new(BridgeError {
                        code: "X3_LIFECYCLE_LOCK_FAILED",
                        message: e.to_string(),
                    })
                })?;
                states.insert(target.to_vec(), 1);
                Ok(vec![1])
            }
            2 => {
                let mut states = lifecycle_states().lock().map_err(|e| {
                    Box::new(BridgeError {
                        code: "X3_LIFECYCLE_LOCK_FAILED",
                        message: e.to_string(),
                    })
                })?;
                states.insert(target.to_vec(), 2);
                Ok(vec![2])
            }
            3 => {
                let mut states = lifecycle_states().lock().map_err(|e| {
                    Box::new(BridgeError {
                        code: "X3_LIFECYCLE_LOCK_FAILED",
                        message: e.to_string(),
                    })
                })?;
                states.remove(target);
                Ok(vec![0])
            }
            _ => Err(Box::new(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!("lifecycle operation {kind} unknown (supported: 0=query, 1=init, 2=start, 3=stop)"),
            })),
        }
    }
    fn serialize(&self, format: u8, data: &[u8]) -> BridgeResult {
        match format {
            0 => Ok(data.to_vec()),
            1 => {
                let value: serde_json::Value = serde_json::from_slice(data).map_err(|err| {
                    Box::new(BridgeError {
                        code: "X3_SERIALIZE_FAILED",
                        message: format!("JSON encoding failed: {err}"),
                    })
                })?;
                Ok(serde_json::to_vec(&value).map_err(|err| {
                    Box::new(BridgeError {
                        code: "X3_SERIALIZE_FAILED",
                        message: format!("JSON encoding failed: {err}"),
                    }) as Box<dyn Error>
                })?)
            }
            _ => Err(Box::new(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!("serialization format {format} unknown (supported: 0=identity, 1=JSON)"),
            })),
        }
    }
    fn deserialize(&self, format: u8, data: &[u8]) -> BridgeResult {
        match format {
            0 => Ok(data.to_vec()),
            1 => {
                let value: serde_json::Value = serde_json::from_slice(data).map_err(|err| {
                    Box::new(BridgeError {
                        code: "X3_DESERIALIZE_FAILED",
                        message: format!("JSON decoding failed: {err}"),
                    })
                })?;
                Ok(value.to_string().into_bytes())
            }
            _ => Err(Box::new(BridgeError {
                code: "X3_FEATURE_NOT_AVAILABLE",
                message: format!("deserialization format {format} unknown (supported: 0=identity, 1=JSON)"),
            })),
        }
    }
    fn gas_estimate(&self, chain: &[u8], route: &[u8]) -> BridgeResult {
        Ok(self.backend.gas_estimate(chain, route)?)
    }
    fn chain_metric(&self, metric: u8) -> BridgeResult {
        Ok(self.backend.chain_metric(metric)?)
    }
    fn event_provenance(&self, event_type: &[u8], data: &[u8]) -> BridgeResult {
        let mut hasher = Sha256::new();
        hasher.update(b"x3-event-provenance:v1:");
        hasher.update(event_type);
        hasher.update(b":");
        hasher.update(data);
        let digest = hasher.finalize();
        let mut result = event_type.to_vec();
        result.push(b':');
        result.extend_from_slice(&digest);
        Ok(result)
    }
    fn multi_hop_swap(&self, _path: &[u8], _amount: u128) -> BridgeResult {
        Err(Box::new(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: "multi-hop swaps require a DEX aggregation backend with connected liquidity sources".into(),
        }))
    }
    fn bridge_transfer(
        &self,
        via: &str,
        from_chain: &str,
        from_asset: &str,
        to_chain: &str,
        to_asset: &str,
        amount: u128,
        receiver: &[u8],
        source_finality_proof: &[u8],
        transfer_proof: &[u8],
    ) -> BridgeResult {
        let request = BridgeTransferRequest {
            via: via.to_string(),
            from_chain: from_chain.to_string(),
            from_asset: from_asset.to_string(),
            to_chain: to_chain.to_string(),
            to_asset: to_asset.to_string(),
            amount,
            receiver: receiver.to_vec(),
            source_finality_proof: source_finality_proof.to_vec(),
            transfer_proof: transfer_proof.to_vec(),
        };
        let finality_proof = self.backend.verify_source_finality(&request)?;
        if finality_proof.is_empty() {
            return Err(Box::new(BridgeError {
                code: "X3_FINALITY_PROOF_EMPTY",
                message: "bridge backend returned an empty finality proof".into(),
            }));
        }
        let transfer_proof = self.backend.verify_transfer_proof(&request, &finality_proof)?;
        if transfer_proof.is_empty() {
            return Err(Box::new(BridgeError {
                code: "X3_TRANSFER_PROOF_EMPTY",
                message: "bridge backend returned an empty transfer proof".into(),
            }));
        }
        let receipt = SettlementReceipt::verified(&request, finality_proof, transfer_proof);
        self.backend.persist_receipt(&receipt)?;
        Ok(receipt.to_bytes())
    }
    fn vector_math(&self, op: u8, a: &[u8], b: &[u8], size: u32) -> BridgeResult {
        let size = size as usize;
        let a_padded = if a.len() < size {
            let mut v = a.to_vec();
            v.resize(size, 0);
            v
        } else {
            a[..size].to_vec()
        };
        let b_padded = if b.len() < size {
            let mut v = b.to_vec();
            v.resize(size, 0);
            v
        } else {
            b[..size].to_vec()
        };
        let result: Vec<u8> = match op {
            0 => a_padded
                .iter()
                .zip(&b_padded)
                .map(|(x, y)| x.wrapping_add(*y))
                .collect(),
            1 => a_padded
                .iter()
                .zip(&b_padded)
                .map(|(x, y)| x.wrapping_sub(*y))
                .collect(),
            2 => a_padded
                .iter()
                .zip(&b_padded)
                .map(|(x, y)| x.wrapping_mul(*y))
                .collect(),
            3 => a_padded.iter().zip(&b_padded).map(|(x, y)| x & y).collect(),
            4 => a_padded.iter().zip(&b_padded).map(|(x, y)| x | y).collect(),
            5 => a_padded.iter().zip(&b_padded).map(|(x, y)| x ^ y).collect(),
            _ => {
                return Err(Box::new(BridgeError {
                    code: "X3_FEATURE_NOT_AVAILABLE",
                    message: format!(
                        "vector math operation {op} unknown (supported: 0=add, 1=sub, 2=mul, 3=and, 4=or, 5=xor)"
                    ),
                }))
            }
        };
        Ok(result)
    }
    fn role_check(&self, role: &[u8]) -> BridgeResult {
        if role.is_empty() {
            return Err(Box::new(BridgeError {
                code: "X3_ROLE_EMPTY",
                message: "role name must not be empty".into(),
            }));
        }
        if VALID_ROLES.contains(&role) {
            Ok(role.to_vec())
        } else {
            Err(Box::new(BridgeError {
                code: "X3_ROLE_UNKNOWN",
                message: format!(
                    "unknown role '{}'; valid roles: admin, validator, relayer, solver, user",
                    String::from_utf8_lossy(role)
                ),
            }))
        }
    }
    fn multisig_check(&self, required: u32, total: u32) -> BridgeResult {
        if total == 0 {
            return Err(Box::new(BridgeError {
                code: "X3_MULTISIG_TOTAL_ZERO",
                message: "total signers must be greater than 0".into(),
            }));
        }
        if required == 0 {
            return Err(Box::new(BridgeError {
                code: "X3_MULTISIG_REQUIRED_ZERO",
                message: "required signers must be greater than 0".into(),
            }));
        }
        if required > total {
            return Err(Box::new(BridgeError {
                code: "X3_MULTISIG_REQUIRED_EXCEEDS_TOTAL",
                message: format!("required ({required}) exceeds total signers ({total})"),
            }));
        }
        Ok(required
            .to_le_bytes()
            .iter()
            .chain(total.to_le_bytes().iter())
            .copied()
            .collect())
    }
    fn vrf_seed(&self) -> BridgeResult {
        let mut hasher = Sha256::new();
        hasher.update(b"x3-vrf-seed");
        hasher.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .to_le_bytes(),
        );
        Ok(hasher.finalize().to_vec())
    }
    fn gas_adaptive_select(&self) -> BridgeResult {
        Ok(vec![0])
    }
    fn bounty_escrow(&self, amount: u128, condition: &[u8]) -> BridgeResult {
        Err(Box::new(BridgeError {
            code: "X3_FEATURE_NOT_AVAILABLE",
            message: format!(
                "bounty escrow for {amount} requires an escrow pallet backend; condition: {} bytes",
                condition.len()
            ),
        }))
    }
}

pub trait BridgeAdapter {
    fn evm_call(&self, data: &[u8]) -> BridgeResult;
    fn svm_call(&self, data: &[u8]) -> BridgeResult;
    fn gpu_dispatch(&self, kernel: &str, args: &[u8]) -> BridgeResult;
    fn simulate(&self, body: &[u8]) -> BridgeResult;
    fn scheduled_dispatch(&self, period: u32, entry: &[u8]) -> BridgeResult;
    fn intent_resolve(&self, constraints: &[u8]) -> BridgeResult;
    fn crdt_op(&self, kind: u8, key: &[u8], value: &[u8]) -> BridgeResult;
    fn proof_verify(&self, kind: u8, proof: &[u8], input: &[u8], key: &[u8]) -> BridgeResult;
    fn storage_op(&self, kind: u8, data: &[u8]) -> BridgeResult;
    fn pathfind(&self, from: &[u8], to: &[u8], max_depth: u32) -> BridgeResult;
    fn mempool_scan(&self, max_results: u32) -> BridgeResult;
    fn oracle_request(&self, token: &[u8], reward: u128) -> BridgeResult;
    fn emergency_control(&self, kind: u8) -> BridgeResult;
    fn lifecycle(&self, kind: u8, target: &[u8]) -> BridgeResult;
    fn serialize(&self, format: u8, data: &[u8]) -> BridgeResult;
    fn deserialize(&self, format: u8, data: &[u8]) -> BridgeResult;
    fn gas_estimate(&self, chain: &[u8], route: &[u8]) -> BridgeResult;
    fn chain_metric(&self, metric: u8) -> BridgeResult;
    fn event_provenance(&self, event_type: &[u8], data: &[u8]) -> BridgeResult;
    fn multi_hop_swap(&self, path: &[u8], amount: u128) -> BridgeResult;
    fn bridge_transfer(
        &self,
        via: &str,
        from_chain: &str,
        from_asset: &str,
        to_chain: &str,
        to_asset: &str,
        amount: u128,
        receiver: &[u8],
        source_finality_proof: &[u8],
        transfer_proof: &[u8],
    ) -> BridgeResult;
    fn vector_math(&self, op: u8, a: &[u8], b: &[u8], size: u32) -> BridgeResult;
    fn role_check(&self, role: &[u8]) -> BridgeResult;
    fn multisig_check(&self, required: u32, total: u32) -> BridgeResult;
    fn vrf_seed(&self) -> BridgeResult;
    fn gas_adaptive_select(&self) -> BridgeResult;
    fn bounty_escrow(&self, amount: u128, condition: &[u8]) -> BridgeResult;
}

pub struct UnconfiguredBridge;
impl BridgeAdapter for UnconfiguredBridge {
    fn evm_call(&self, _data: &[u8]) -> BridgeResult {
        Err(Box::new(BridgeError {
            code: "X3_BACKEND_REQUIRED",
            message: "production EVM bridge backend is not configured".into(),
        }))
    }
    fn svm_call(&self, _data: &[u8]) -> BridgeResult {
        Err(Box::new(BridgeError {
            code: "X3_BACKEND_REQUIRED",
            message: "production SVM bridge backend is not configured".into(),
        }))
    }
    fn gpu_dispatch(&self, _kernel: &str, _args: &[u8]) -> BridgeResult {
        backend_required("GPU dispatch")
    }
    fn simulate(&self, _body: &[u8]) -> BridgeResult {
        backend_required("simulation")
    }
    fn scheduled_dispatch(&self, _period: u32, _entry: &[u8]) -> BridgeResult {
        backend_required("scheduled dispatch")
    }
    fn intent_resolve(&self, _constraints: &[u8]) -> BridgeResult {
        backend_required("intent resolver")
    }
    fn crdt_op(&self, _kind: u8, _key: &[u8], _value: &[u8]) -> BridgeResult {
        backend_required("CRDT")
    }
    fn proof_verify(&self, _kind: u8, _proof: &[u8], _input: &[u8], _key: &[u8]) -> BridgeResult {
        backend_required("proof verifier")
    }
    fn storage_op(&self, _kind: u8, _data: &[u8]) -> BridgeResult {
        backend_required("storage")
    }
    fn pathfind(&self, _from: &[u8], _to: &[u8], _max_depth: u32) -> BridgeResult {
        backend_required("pathfinder")
    }
    fn mempool_scan(&self, _max_results: u32) -> BridgeResult {
        backend_required("mempool scanner")
    }
    fn oracle_request(&self, _token: &[u8], _reward: u128) -> BridgeResult {
        backend_required("oracle")
    }
    fn emergency_control(&self, _kind: u8) -> BridgeResult {
        backend_required("emergency control")
    }
    fn lifecycle(&self, _kind: u8, _target: &[u8]) -> BridgeResult {
        backend_required("lifecycle")
    }
    fn serialize(&self, _format: u8, _data: &[u8]) -> BridgeResult {
        backend_required("serializer")
    }
    fn deserialize(&self, _format: u8, _data: &[u8]) -> BridgeResult {
        backend_required("deserializer")
    }
    fn gas_estimate(&self, _chain: &[u8], _route: &[u8]) -> BridgeResult {
        backend_required("gas estimator")
    }
    fn chain_metric(&self, _metric: u8) -> BridgeResult {
        backend_required("chain metrics")
    }
    fn event_provenance(&self, _event_type: &[u8], _data: &[u8]) -> BridgeResult {
        backend_required("event provenance")
    }
    fn multi_hop_swap(&self, _path: &[u8], _amount: u128) -> BridgeResult {
        backend_required("multi-hop swap")
    }
    fn bridge_transfer(
        &self,
        _via: &str,
        _from_chain: &str,
        _from_asset: &str,
        _to_chain: &str,
        _to_asset: &str,
        _amount: u128,
        _receiver: &[u8],
        _source_finality_proof: &[u8],
        _transfer_proof: &[u8],
    ) -> BridgeResult {
        backend_required("bridge transfer")
    }
    fn vector_math(&self, _op: u8, _a: &[u8], _b: &[u8], _size: u32) -> BridgeResult {
        backend_required("vector math")
    }
    fn role_check(&self, _role: &[u8]) -> BridgeResult {
        backend_required("role check")
    }
    fn multisig_check(&self, _required: u32, _total: u32) -> BridgeResult {
        backend_required("multisig check")
    }
    fn vrf_seed(&self) -> BridgeResult {
        backend_required("VRF")
    }
    fn gas_adaptive_select(&self) -> BridgeResult {
        backend_required("gas adaptive selector")
    }
    fn bounty_escrow(&self, _amount: u128, _condition: &[u8]) -> BridgeResult {
        backend_required("bounty escrow")
    }
}

pub struct DryRunBridge;

impl Default for DryRunBridge {
    fn default() -> Self {
        Self
    }
}

impl BridgeAdapter for DryRunBridge {
    fn evm_call(&self, data: &[u8]) -> BridgeResult {
        Ok([b"dry-run-evm:".as_slice(), data].concat())
    }
    fn svm_call(&self, data: &[u8]) -> BridgeResult {
        Ok([b"dry-run-svm:".as_slice(), data].concat())
    }
    fn gpu_dispatch(&self, kernel: &str, args: &[u8]) -> BridgeResult {
        Ok([format!("dry-run-gpu_dispatch:{kernel}:").as_bytes(), args].concat())
    }
    fn simulate(&self, body: &[u8]) -> BridgeResult {
        dry_run("simulate", body)
    }
    fn scheduled_dispatch(&self, period: u32, entry: &[u8]) -> BridgeResult {
        Ok([format!("dry-run-scheduled_dispatch:{period}:").as_bytes(), entry].concat())
    }
    fn intent_resolve(&self, constraints: &[u8]) -> BridgeResult {
        dry_run("intent_resolve", constraints)
    }
    fn crdt_op(&self, kind: u8, key: &[u8], value: &[u8]) -> BridgeResult {
        Ok([format!("dry-run-crdt_op:{kind}:").as_bytes(), key, b":", value].concat())
    }
    fn proof_verify(&self, kind: u8, proof: &[u8], input: &[u8], key: &[u8]) -> BridgeResult {
        Ok([
            format!("dry-run-proof_verify:{kind}:").as_bytes(),
            proof,
            b":",
            input,
            b":",
            key,
        ]
        .concat())
    }
    fn storage_op(&self, kind: u8, data: &[u8]) -> BridgeResult {
        Ok([format!("dry-run-storage_op:{kind}:").as_bytes(), data].concat())
    }
    fn pathfind(&self, from: &[u8], to: &[u8], max_depth: u32) -> BridgeResult {
        Ok([format!("dry-run-pathfind:{max_depth}:").as_bytes(), from, b":", to].concat())
    }
    fn mempool_scan(&self, max_results: u32) -> BridgeResult {
        Ok(format!("dry-run-mempool_scan:{max_results}").into_bytes())
    }
    fn oracle_request(&self, token: &[u8], reward: u128) -> BridgeResult {
        Ok([format!("dry-run-oracle_request:{reward}:").as_bytes(), token].concat())
    }
    fn emergency_control(&self, kind: u8) -> BridgeResult {
        Ok(format!("dry-run-emergency_control:{kind}").into_bytes())
    }
    fn lifecycle(&self, kind: u8, target: &[u8]) -> BridgeResult {
        Ok([format!("dry-run-lifecycle:{kind}:").as_bytes(), target].concat())
    }
    fn serialize(&self, format: u8, data: &[u8]) -> BridgeResult {
        Ok([format!("dry-run-serialize:{format}:").as_bytes(), data].concat())
    }
    fn deserialize(&self, format: u8, data: &[u8]) -> BridgeResult {
        Ok([format!("dry-run-deserialize:{format}:").as_bytes(), data].concat())
    }
    fn gas_estimate(&self, chain: &[u8], route: &[u8]) -> BridgeResult {
        Ok([b"dry-run-gas_estimate:".as_slice(), chain, b":", route].concat())
    }
    fn chain_metric(&self, metric: u8) -> BridgeResult {
        Ok(format!("dry-run-chain_metric:{metric}").into_bytes())
    }
    fn event_provenance(&self, event_type: &[u8], data: &[u8]) -> BridgeResult {
        Ok([b"dry-run-event_provenance:".as_slice(), event_type, b":", data].concat())
    }
    fn multi_hop_swap(&self, path: &[u8], amount: u128) -> BridgeResult {
        Ok([format!("dry-run-multi_hop_swap:{amount}:").as_bytes(), path].concat())
    }
    fn bridge_transfer(
        &self,
        via: &str,
        from_chain: &str,
        from_asset: &str,
        to_chain: &str,
        to_asset: &str,
        amount: u128,
        receiver: &[u8],
        source_finality_proof: &[u8],
        transfer_proof: &[u8],
    ) -> BridgeResult {
        Ok([
            format!("dry-run-bridge_transfer:{via}:{from_chain}.{from_asset}->{to_chain}.{to_asset}:{amount}:")
                .as_bytes(),
            receiver,
            b":finality_proof=",
            source_finality_proof,
            b":transfer_proof=",
            transfer_proof,
        ]
        .concat())
    }
    fn vector_math(&self, op: u8, a: &[u8], b: &[u8], size: u32) -> BridgeResult {
        Ok([format!("dry-run-vector_math:{op}:{size}:").as_bytes(), a, b":", b].concat())
    }
    fn role_check(&self, role: &[u8]) -> BridgeResult {
        dry_run("role_check", role)
    }
    fn multisig_check(&self, required: u32, total: u32) -> BridgeResult {
        Ok(format!("dry-run-multisig_check:{required}:{total}").into_bytes())
    }
    fn vrf_seed(&self) -> BridgeResult {
        Ok(b"dry-run-vrf_seed:00000000000000000000000000000000".to_vec())
    }
    fn gas_adaptive_select(&self) -> BridgeResult {
        Ok(vec![0])
    }
    fn bounty_escrow(&self, amount: u128, condition: &[u8]) -> BridgeResult {
        Ok([format!("dry-run-bounty_escrow:{amount}:").as_bytes(), condition].concat())
    }
}

#[deprecated(note = "Use DryRunBridge explicitly for simulations or a real verifier-backed adapter for production")]
pub type MockBridge = DryRunBridge;

/// Select the bridge backend based on the `X3_BACKEND` environment variable.
///
/// # Backend selection
/// - `X3_BACKEND=prod` → requires a wired production adapter. Fails closed
///   (returns an error) when no production adapter is configured, rather
///   than silently falling back to dry-run simulation.
/// - `X3_BACKEND=dry` (or unset) → use `DryRunBridge` for simulations.
///
/// Production callers must configure a backend via
/// `resolve_bridge_backend_with()`.
pub fn resolve_bridge_backend() -> Result<Box<dyn BridgeAdapter>, BridgeError> {
    match std::env::var("X3_BACKEND").as_deref() {
        Ok("prod") => Err(BridgeError {
            code: "X3_BACKEND_PROD_NOT_CONFIGURED",
            message: concat!(
                "X3_BACKEND=prod requires a wired production adapter. ",
                "No production backend is registered. ",
                "Call resolve_bridge_backend_with() to wire a real backend, ",
                "or set X3_BACKEND=dry for simulation."
            )
            .to_string(),
        }),
        _ => Ok(Box::new(DryRunBridge)),
    }
}

/// Wire a specific production bridge backend.
///
/// When `X3_BACKEND=prod`, returns `Some(backend)`. Otherwise returns `None`
/// (caller should fall back to `resolve_bridge_backend()` for the dry-run
/// path).
pub fn resolve_bridge_backend_with(backend: Box<dyn BridgeAdapter>) -> Option<Box<dyn BridgeAdapter>> {
    if std::env::var("X3_BACKEND").as_deref() == Ok("prod") {
        Some(backend)
    } else {
        None
    }
}

/// Initialize a production bridge backend from environment configuration.
///
/// Reads `X3_BRIDGE_VERIFIER` to select the verifier family and sources
/// verifier-specific parameters from environment variables. Returns a
/// `ProductionBridgeAdapter` wrapping the configured verifier and a
/// `FileReceiptStore` for persistent receipt storage.
///
/// # Verifier families
/// - `evm-light-client` → `EthereumLightClientVerifier`
///   - `X3_EVM_TRUSTED_HEADER_HASH` (required)
///   - `X3_EVM_MIN_BLOCK_NUMBER` (optional)
///   - `X3_EVM_ERC20_CHECK_ADDRESS` (optional, enables ERC20 transfer event verification)
/// - `svm-light-client` → `SolanaLightClientVerifier`
///   - `X3_SVM_TRUSTED_BANK_HASH` (required)
///   - `X3_SVM_VALIDATOR_PUBKEYS` (comma-separated, optional)
///   - `X3_SVM_MIN_SIGNATURES` (optional, default 1)
///   - `X3_SVM_MIN_STAKE_BPS` (optional, default 6667)
/// - `evm-rpc` → `EthereumRpcFinalityVerifier`
///   - `X3_EVM_RPC_URL` (required)
///   - `X3_EVM_MIN_CONFIRMATIONS` (optional, default 12)
/// - `svm-rpc` → `SolanaRpcFinalityVerifier`
///   - `X3_SVM_RPC_URL` (required)
///   - `X3_SVM_EXPECTED_PROGRAM_ID` (optional)
///
/// # Receipt store
/// Uses `X3_RECEIPT_STORE_PATH` for the `FileReceiptStore` location.
/// Defaults to `.autoclaw/bridge-receipts.jsonl` in the current directory.
///
/// # Failure modes
/// Returns `BridgeError` with code `X3_BRIDGE_INIT_FAILED` when required
/// environment variables are missing or when the verifier family is unrecognized.
/// Never silently falls back to `DryRunBridge`. Callers that don't set
/// `X3_BRIDGE_VERIFIER` should explicitly pass `None` and use
/// `resolve_bridge_backend()` for the dry-run path.
pub fn init_production_backend() -> Result<Option<Box<dyn BridgeAdapter>>, BridgeError> {
    let verifier: Box<dyn BridgeAdapter> = match std::env::var("X3_BRIDGE_VERIFIER").as_deref() {
        Ok("evm-light-client") => {
            let trusted_hash = std::env::var("X3_EVM_TRUSTED_HEADER_HASH").map_err(|_| BridgeError {
                code: "X3_BRIDGE_INIT_FAILED",
                message: "X3_BRIDGE_VERIFIER=evm-light-client requires X3_EVM_TRUSTED_HEADER_HASH".to_string(),
            })?;
            let mut verifier = EthereumLightClientVerifier::new(&trusted_hash);
            if let Ok(min_block) = std::env::var("X3_EVM_MIN_BLOCK_NUMBER") {
                if let Ok(n) = min_block.parse::<u64>() {
                    verifier = verifier.with_min_block_number(n);
                }
            }
            if let Ok(token_addr) = std::env::var("X3_EVM_ERC20_CHECK_ADDRESS") {
                verifier = verifier.with_erc20_transfer_event(token_addr);
            }
            let store_path = std::env::var("X3_RECEIPT_STORE_PATH")
                .unwrap_or_else(|_| ".autoclaw/bridge-receipts.jsonl".to_string());
            let store = FileReceiptStore::new(&store_path);
            Box::new(ProductionBridgeAdapter::new(EvmProductionBridgeBackend::new(
                verifier, store,
            )))
        }
        Ok("svm-light-client") => {
            let trusted_hash = std::env::var("X3_SVM_TRUSTED_BANK_HASH").map_err(|_| BridgeError {
                code: "X3_BRIDGE_INIT_FAILED",
                message: "X3_BRIDGE_VERIFIER=svm-light-client requires X3_SVM_TRUSTED_BANK_HASH".to_string(),
            })?;
            let mut verifier = SolanaLightClientVerifier::new(&trusted_hash);
            if let Ok(pubkeys) = std::env::var("X3_SVM_VALIDATOR_PUBKEYS") {
                verifier = verifier.with_validator_pubkeys(pubkeys.split(',').map(|s| s.trim().to_string()).collect());
            }
            if let Ok(min_sigs) = std::env::var("X3_SVM_MIN_SIGNATURES") {
                if let Ok(n) = min_sigs.parse::<usize>() {
                    verifier = verifier.with_min_signatures(n);
                }
            }
            if let Ok(stake_bps) = std::env::var("X3_SVM_MIN_STAKE_BPS") {
                if let Ok(n) = stake_bps.parse::<u64>() {
                    verifier = verifier.with_min_stake_threshold_bps(n);
                }
            }
            let store_path = std::env::var("X3_RECEIPT_STORE_PATH")
                .unwrap_or_else(|_| ".autoclaw/bridge-receipts.jsonl".to_string());
            let store = FileReceiptStore::new(&store_path);
            Box::new(ProductionBridgeAdapter::new(SvmProductionBridgeBackend::new(
                verifier, store,
            )))
        }
        Ok("evm-rpc") => {
            let rpc_url = std::env::var("X3_EVM_RPC_URL").unwrap_or_else(|_| "http://localhost:8545".to_string());
            let min_confirmations = std::env::var("X3_EVM_MIN_CONFIRMATIONS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(12);
            let verifier = EthereumRpcFinalityVerifier::new(&rpc_url, "").with_min_confirmations(min_confirmations);
            let store_path = std::env::var("X3_RECEIPT_STORE_PATH")
                .unwrap_or_else(|_| ".autoclaw/bridge-receipts.jsonl".to_string());
            let store = FileReceiptStore::new(&store_path);
            Box::new(ProductionBridgeAdapter::new(EvmProductionBridgeBackend::new(
                verifier, store,
            )))
        }
        Ok("svm-rpc") => {
            let rpc_url = std::env::var("X3_SVM_RPC_URL").unwrap_or_else(|_| "http://localhost:8899".to_string());
            let mut verifier = SolanaRpcFinalityVerifier::new(&rpc_url, "");
            if let Ok(program_id) = std::env::var("X3_SVM_EXPECTED_PROGRAM_ID") {
                verifier = verifier.with_expected_program_id(program_id);
            }
            let store_path = std::env::var("X3_RECEIPT_STORE_PATH")
                .unwrap_or_else(|_| ".autoclaw/bridge-receipts.jsonl".to_string());
            let store = FileReceiptStore::new(&store_path);
            Box::new(ProductionBridgeAdapter::new(SvmProductionBridgeBackend::new(
                verifier, store,
            )))
        }
        Ok(other) => {
            return Err(BridgeError {
                code: "X3_BRIDGE_INIT_FAILED",
                message: format!(
                    "Unrecognized X3_BRIDGE_VERIFIER value: '{}'. \
                     Supported values: evm-light-client, svm-light-client, evm-rpc, svm-rpc",
                    other
                ),
            });
        }
        Err(_) => {
            // X3_BRIDGE_VERIFIER not set — caller should decide whether to
            // use dry-run or fail. Return None to signal "no verifier configured".
            return Ok(None);
        }
    };
    Ok(Some(verifier))
}

fn backend_required(name: &str) -> BridgeResult {
    Err(Box::new(BridgeError {
        code: "X3_BACKEND_REQUIRED",
        message: format!("production {name} backend is not configured"),
    }))
}

fn dry_run(method: &str, data: &[u8]) -> BridgeResult {
    Ok([format!("dry-run-{method}:").as_bytes(), data].concat())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[derive(Clone)]
    struct TestEvmVerifier;

    impl EvmFinalityVerifier for TestEvmVerifier {
        fn verify_evm_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
            assert_eq!(request.from_chain, "ethereum");
            Ok(b"evm-finality-proof".to_vec())
        }

        fn verify_evm_transfer_proof(
            &self,
            request: &BridgeTransferRequest,
            finality_proof: &[u8],
        ) -> Result<Vec<u8>, BridgeError> {
            assert_eq!(request.amount, 42);
            assert_eq!(finality_proof, b"evm-finality-proof");
            Ok(b"evm-transfer-proof".to_vec())
        }
    }

    #[derive(Clone)]
    struct TestSvmVerifier;

    impl SvmFinalityVerifier for TestSvmVerifier {
        fn verify_svm_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
            assert_eq!(request.from_chain, "solana");
            Ok(b"svm-finality-proof".to_vec())
        }

        fn verify_svm_transfer_proof(
            &self,
            request: &BridgeTransferRequest,
            finality_proof: &[u8],
        ) -> Result<Vec<u8>, BridgeError> {
            assert!(!request.receiver.is_empty());
            assert_eq!(finality_proof, b"svm-finality-proof");
            Ok(b"svm-transfer-proof".to_vec())
        }
    }

    fn request(from_chain: &str) -> BridgeTransferRequest {
        BridgeTransferRequest {
            via: "X3".into(),
            from_chain: from_chain.into(),
            from_asset: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            to_chain: "x3".into(),
            to_asset: "USDC".into(),
            amount: 42,
            receiver: b"0x1111111111111111111111111111111111111111".to_vec(),
            source_finality_proof: b"embedded-finality-proof".to_vec(),
            transfer_proof: b"embedded-transfer-proof".to_vec(),
        }
    }

    #[test]
    fn evm_backend_verifies_and_persists_jsonl_receipt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("receipts.jsonl");
        let store = FileReceiptStore::new(&path);
        let backend = EvmProductionBridgeBackend::new(TestEvmVerifier, store.clone());
        let req = request("ethereum");

        let finality = backend.verify_source_finality(&req).expect("finality verifies");
        let proof = backend
            .verify_transfer_proof(&req, &finality)
            .expect("transfer proof verifies");
        let receipt = SettlementReceipt::verified(&req, finality, proof);
        backend.persist_receipt(&receipt).expect("receipt persists");

        let encoded = std::fs::read_to_string(store.path()).expect("receipt file");
        let decoded: SettlementReceipt = serde_json::from_str(encoded.trim()).expect("json receipt");
        assert_eq!(decoded.receipt_id, receipt.receipt_id);
        assert_eq!(decoded.finality_proof, b"evm-finality-proof");
        assert_eq!(decoded.transfer_proof, b"evm-transfer-proof");
        assert_eq!(decoded.source_finality_proof_input, b"embedded-finality-proof");
        assert_eq!(decoded.transfer_proof_input, b"embedded-transfer-proof");
    }

    #[test]
    fn svm_backend_rejects_non_svm_source_chain() {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = SvmProductionBridgeBackend::new(
            TestSvmVerifier,
            FileReceiptStore::new(dir.path().join("receipts.jsonl")),
        );
        let err = backend
            .verify_source_finality(&request("ethereum"))
            .expect_err("wrong source chain must fail");
        assert_eq!(err.code, "X3_BRIDGE_SOURCE_CHAIN_MISMATCH");
    }

    #[test]
    fn svm_backend_verifies_transfer_proof() {
        let dir = tempfile::tempdir().expect("tempdir");
        let backend = SvmProductionBridgeBackend::new(
            TestSvmVerifier,
            FileReceiptStore::new(dir.path().join("receipts.jsonl")),
        );
        let req = request("solana");
        let finality = backend.verify_source_finality(&req).expect("svm finality verifies");
        let proof = backend
            .verify_transfer_proof(&req, &finality)
            .expect("svm transfer proof verifies");
        assert_eq!(proof, b"svm-transfer-proof");
    }

    #[test]
    fn evm_erc20_transfer_decoder_matches_exact_receiver_and_amount() {
        let req = request("ethereum");
        let receipt = json!({
            "logs": [{
                "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                "topics": [
                    ERC20_TRANSFER_TOPIC,
                    "0x0000000000000000000000002222222222222222222222222222222222222222",
                    "0x0000000000000000000000001111111111111111111111111111111111111111"
                ],
                "data": "0x000000000000000000000000000000000000000000000000000000000000002a"
            }]
        });

        assert!(
            evm_receipt_has_erc20_transfer(&receipt, &req, Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"))
                .expect("log decodes")
        );

        let mut wrong_amount = req.clone();
        wrong_amount.amount = 43;
        assert!(!evm_receipt_has_erc20_transfer(
            &receipt,
            &wrong_amount,
            Some("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48")
        )
        .expect("log decodes"));
    }

    #[test]
    fn svm_parsed_instruction_decoder_matches_exact_mint_receiver_and_amount() {
        let req = BridgeTransferRequest {
            from_chain: "solana".into(),
            from_asset: "Mint111111111111111111111111111111111111111".into(),
            receiver: b"Receiver11111111111111111111111111111111111".to_vec(),
            ..request("solana")
        };
        let tx = json!({
            "transaction": {
                "signatures": ["sig111"],
                "message": {
                    "instructions": [{
                        "programId": "Tokenkeg1111111111111111111111111111111111",
                        "parsed": {
                            "type": "transferChecked",
                            "info": {
                                "mint": "Mint111111111111111111111111111111111111111",
                                "destination": "Receiver11111111111111111111111111111111111",
                                "tokenAmount": {
                                    "amount": "42"
                                }
                            }
                        }
                    }]
                }
            },
            "meta": {
                "innerInstructions": []
            }
        });

        assert!(
            svm_transaction_has_parsed_transfer(&tx, &req, Some("Tokenkeg1111111111111111111111111111111111"))
                .expect("parsed instruction decodes")
        );

        let mut wrong_receiver = req.clone();
        wrong_receiver.receiver = b"OtherReceiver111111111111111111111111111".to_vec();
        assert!(!svm_transaction_has_parsed_transfer(
            &tx,
            &wrong_receiver,
            Some("Tokenkeg1111111111111111111111111111111111")
        )
        .expect("parsed instruction decodes"));
    }

    #[test]
    fn evm_light_client_verifier_checks_header_receipts_root_and_transfer_log() {
        let mut req = request("ethereum");
        let receipt_rlp = rlp_list(vec![
            rlp_bytes(&[1]),
            rlp_bytes(&[0]),
            rlp_bytes(&vec![0; 256]),
            rlp_list(vec![]),
        ]);
        let receipt_hash = keccak256(&receipt_rlp);
        let receipt_key = vec![0x01];
        let receipt_leaf = rlp_list(vec![
            rlp_bytes(&compact_leaf_path(&bytes_to_nibbles(&receipt_key))),
            rlp_bytes(&receipt_rlp),
        ]);
        let receipts_root = keccak256(&receipt_leaf);
        let header_rlp = evm_test_header_rlp(receipts_root, 42);
        let header_hash = hex_prefixed(&keccak256(&header_rlp));
        req.source_finality_proof = serde_json::to_vec(&json!({
            "proof_type": EVM_HEADER_PROOF_TYPE,
            "rlp_header": hex_prefixed(&header_rlp),
            "header_hash": header_hash,
            "receipts_root": hex_prefixed(&receipts_root)
        }))
        .expect("finality proof JSON");
        req.transfer_proof = serde_json::to_vec(&json!({
            "proof_type": EVM_RECEIPT_PROOF_TYPE,
            "receipt_key": hex_prefixed(&receipt_key),
            "receipt_rlp": hex_prefixed(&receipt_rlp),
            "receipt_hash": hex_prefixed(&receipt_hash),
            "receipts_root": hex_prefixed(&receipts_root),
            "trie_nodes": [hex_prefixed(&receipt_leaf)],
            "log": {
                "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                "topics": [
                    ERC20_TRANSFER_TOPIC,
                    "0x0000000000000000000000002222222222222222222222222222222222222222",
                    "0x0000000000000000000000001111111111111111111111111111111111111111"
                ],
                "data": "0x000000000000000000000000000000000000000000000000000000000000002a"
            }
        }))
        .expect("transfer proof JSON");

        let verifier = EthereumLightClientVerifier::new(header_hash)
            .with_min_block_number(40)
            .with_erc20_transfer_event("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
        let finality = verifier.verify_evm_finality(&req).expect("header proof verifies");
        let transfer = verifier
            .verify_evm_transfer_proof(&req, &finality)
            .expect("receipt proof verifies");
        let transfer: Value = serde_json::from_slice(&transfer).expect("transfer receipt JSON");
        assert_eq!(
            transfer.get("receipt_root").and_then(Value::as_str),
            Some(hex_prefixed(&receipts_root).as_str())
        );

        let mut wrong = req.clone();
        wrong.amount = 43;
        let err = verifier
            .verify_evm_transfer_proof(&wrong, &finality)
            .expect_err("wrong amount must fail transfer log validation");
        assert_eq!(err.code, "X3_EVM_TRANSFER_EVENT_MISMATCH");

        let mut wrong_root = req.clone();
        let mut wrong_transfer: Value =
            serde_json::from_slice(&wrong_root.transfer_proof).expect("transfer proof JSON");
        wrong_transfer["trie_nodes"] = json!([hex_prefixed(&rlp_list(vec![
            rlp_bytes(&compact_leaf_path(&bytes_to_nibbles(&receipt_key))),
            rlp_bytes(b"different-receipt"),
        ]))]);
        wrong_root.transfer_proof = serde_json::to_vec(&wrong_transfer).expect("wrong transfer proof JSON");
        let err = verifier
            .verify_evm_transfer_proof(&wrong_root, &finality)
            .expect_err("tampered trie node must fail root binding");
        assert_eq!(err.code, "X3_EVM_RECEIPT_TRIE_NODE_HASH_MISMATCH");
    }

    // Note: env-var-dependent init_production_backend and resolve_bridge_backend_with
    // tests are not included here because parallel test execution causes race conditions
    // on environment variables. Those code paths are exercised by the verifier backend
    // tests above (evm_backend_verifies_and_persists_jsonl_receipt, etc.) which construct
    // backends directly without env vars.

    #[test]
    fn production_bridge_adapter_delegates_evm_call_to_backend() {
        // Integration test: ProductionBridgeAdapter wrapping a TestEvmVerifier
        // should successfully delegate bridge_transfer to the verifier's path
        let dir = tempfile::tempdir().expect("tempdir");
        let store = FileReceiptStore::new(dir.path().join("receipts.jsonl"));
        let backend = EvmProductionBridgeBackend::new(TestEvmVerifier, store);
        let adapter: Box<dyn BridgeAdapter> = Box::new(ProductionBridgeAdapter::new(backend));

        let req = request("ethereum");
        let result = adapter.bridge_transfer(
            &req.via,
            &req.from_chain,
            &req.from_asset,
            &req.to_chain,
            &req.to_asset,
            req.amount,
            &req.receiver,
            &req.source_finality_proof,
            &req.transfer_proof,
        );
        assert!(result.is_ok(), "Production bridge adapter should process transfer");
        let receipt_bytes = result.unwrap();
        assert!(!receipt_bytes.is_empty(), "Receipt should not be empty");
    }

    #[test]
    fn evm_light_client_verifier_accepts_branch_extension_and_typed_receipt_fixture() {
        let mut req = request("ethereum");
        let legacy_payload = rlp_list(vec![
            rlp_bytes(&[1]),
            rlp_bytes(&[0]),
            rlp_bytes(&vec![0; 256]),
            rlp_list(vec![]),
        ]);
        let typed_receipt = [vec![0x02], legacy_payload].concat();
        let receipt_hash = keccak256(&typed_receipt);
        let receipt_key = vec![0xab, 0xcd];
        let leaf = rlp_list(vec![rlp_bytes(&compact_leaf_path(&[0x0d])), rlp_bytes(&typed_receipt)]);
        let leaf_hash = keccak256(&leaf);
        let mut branch_items = Vec::new();
        for index in 0..16 {
            if index == 0x0c {
                branch_items.push(rlp_bytes(&leaf_hash));
            } else {
                branch_items.push(rlp_bytes(&[]));
            }
        }
        branch_items.push(rlp_bytes(&[]));
        let branch = rlp_list(branch_items);
        let branch_hash = keccak256(&branch);
        let extension = rlp_list(vec![
            rlp_bytes(&compact_extension_path(&[0x0a, 0x0b])),
            rlp_bytes(&branch_hash),
        ]);
        let receipts_root = keccak256(&extension);
        let header_rlp = evm_test_header_rlp(receipts_root, 123);
        let header_hash = hex_prefixed(&keccak256(&header_rlp));

        req.source_finality_proof = serde_json::to_vec(&json!({
            "proof_type": EVM_HEADER_PROOF_TYPE,
            "rlp_header": hex_prefixed(&header_rlp),
            "header_hash": header_hash,
            "receipts_root": hex_prefixed(&receipts_root)
        }))
        .expect("finality proof JSON");
        req.transfer_proof = serde_json::to_vec(&json!({
            "proof_type": EVM_RECEIPT_PROOF_TYPE,
            "receipt_key": hex_prefixed(&receipt_key),
            "receipt_rlp": hex_prefixed(&typed_receipt),
            "receipt_hash": hex_prefixed(&receipt_hash),
            "receipts_root": hex_prefixed(&receipts_root),
            "trie_nodes": [
                hex_prefixed(&extension),
                hex_prefixed(&branch),
                hex_prefixed(&leaf)
            ],
            "log": {
                "address": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48",
                "topics": [
                    ERC20_TRANSFER_TOPIC,
                    "0x0000000000000000000000002222222222222222222222222222222222222222",
                    "0x0000000000000000000000001111111111111111111111111111111111111111"
                ],
                "data": "0x000000000000000000000000000000000000000000000000000000000000002a"
            }
        }))
        .expect("transfer proof JSON");

        let verifier = EthereumLightClientVerifier::new(header_hash)
            .with_min_block_number(100)
            .with_erc20_transfer_event("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
        let finality = verifier.verify_evm_finality(&req).expect("header proof verifies");
        let transfer = verifier
            .verify_evm_transfer_proof(&req, &finality)
            .expect("typed branch/extension receipt proof verifies");
        let transfer: Value = serde_json::from_slice(&transfer).expect("transfer proof JSON");
        assert_eq!(
            transfer.get("receipt_hash").and_then(Value::as_str),
            Some(hex_prefixed(&receipt_hash).as_str())
        );

        let mut wrong = req.clone();
        let mut wrong_transfer: Value = serde_json::from_slice(&wrong.transfer_proof).expect("transfer proof JSON");
        wrong_transfer["receipt_key"] = json!("0xabce");
        wrong.transfer_proof = serde_json::to_vec(&wrong_transfer).expect("wrong proof JSON");
        let err = verifier
            .verify_evm_transfer_proof(&wrong, &finality)
            .expect_err("wrong branch key must fail trie path validation");
        assert_eq!(err.code, "X3_EVM_RECEIPT_TRIE_PATH_MISMATCH");
    }

    #[test]
    fn ethereum_fixture_generator_validates_archived_schema_fixture() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|path| path.parent())
            .expect("vm crate lives under x3-lang/vm");
        let script = repo_root.join("scripts/proof/generate_eth_bridge_fixture.py");
        let fixture = repo_root.join("docs/x3-lang/fixtures/ethereum-receipt-trie-proof.fixture.json");
        let output = std::process::Command::new("python3")
            .arg(&script)
            .arg("--validate-only")
            .arg(&fixture)
            .output()
            .expect("fixture generator should run");
        assert!(
            output.status.success(),
            "fixture validation failed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn ethereum_mainnet_archive_fixture_runs_through_generator_and_vm_verifier() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|path| path.parent())
            .expect("vm crate lives under x3-lang/vm");
        let script = repo_root.join("scripts/proof/generate_eth_bridge_fixture.py");
        let archive = repo_root.join("docs/x3-lang/fixtures/ethereum-mainnet-46147-receipt-proof.archive.json");
        let output = std::process::Command::new("python3")
            .arg(&script)
            .arg("--from-archive-only")
            .arg(&archive)
            .output()
            .expect("fixture generator should run");
        assert!(
            output.status.success(),
            "mainnet archive fixture generation failed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let fixture: Value = serde_json::from_slice(&output.stdout).expect("generated fixture JSON");
        let finality = fixture
            .get("source_finality_proof")
            .cloned()
            .expect("source_finality_proof");
        let transfer = fixture.get("transfer_proof").cloned().expect("transfer_proof");
        let mut req = request("ethereum");
        req.source_finality_proof = serde_json::to_vec(&finality).expect("finality JSON");
        req.transfer_proof = serde_json::to_vec(&transfer).expect("transfer JSON");

        let verifier =
            EthereumLightClientVerifier::new("0x4e3a3754410177e6937ef1f84bba68ea139e8d1a2258c5f85db9f1cd715a1bdd")
                .with_min_block_number(46147);
        let finality = verifier
            .verify_evm_finality(&req)
            .expect("mainnet header proof verifies");
        let transfer = verifier
            .verify_evm_transfer_proof(&req, &finality)
            .expect("mainnet receipt trie proof verifies");
        let transfer: Value = serde_json::from_slice(&transfer).expect("transfer receipt JSON");
        assert_eq!(
            transfer.get("receipt_root").and_then(Value::as_str),
            Some("0xfe2bf2a941abf41d72637e5b91750332a30283efd40c424dc522b77e6f0ed8c4")
        );
    }

    #[test]
    fn ethereum_modern_usdc_archive_verifies_erc20_transfer_event() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|path| path.parent())
            .expect("vm crate lives under x3-lang/vm");
        let script = repo_root.join("scripts/proof/generate_eth_bridge_fixture.py");
        let archive = repo_root.join("docs/x3-lang/fixtures/ethereum-mainnet-17000000-usdc-receipt-proof.archive.json");
        let output = std::process::Command::new("python3")
            .arg(&script)
            .arg("--from-archive-only")
            .arg(&archive)
            .output()
            .expect("fixture generator should run");
        assert!(
            output.status.success(),
            "modern USDC archive generation failed\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let fixture: Value = serde_json::from_slice(&output.stdout).expect("generated fixture JSON");
        let mut req = BridgeTransferRequest {
            via: "X3".into(),
            from_chain: "ethereum".into(),
            from_asset: "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48".into(),
            to_chain: "x3".into(),
            to_asset: "USDC".into(),
            amount: 18_000_000,
            receiver: b"0xc45143c530e9dc0c3895c458c160144a3129955b".to_vec(),
            source_finality_proof: serde_json::to_vec(
                fixture.get("source_finality_proof").expect("source_finality_proof"),
            )
            .expect("finality JSON"),
            transfer_proof: serde_json::to_vec(fixture.get("transfer_proof").expect("transfer_proof"))
                .expect("transfer JSON"),
        };
        let verifier =
            EthereumLightClientVerifier::new("0x96cfa0fb5e50b0a3f6cc76f3299cfbf48f17e8b41798d1394474e67ec8a97e9f")
                .with_min_block_number(17_000_000)
                .with_erc20_transfer_event("0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48");
        let finality = verifier
            .verify_evm_finality(&req)
            .expect("modern header proof verifies");
        let transfer = verifier
            .verify_evm_transfer_proof(&req, &finality)
            .expect("modern USDC receipt proof verifies");
        let transfer: Value = serde_json::from_slice(&transfer).expect("transfer receipt JSON");
        assert_eq!(
            transfer.get("receipt_root").and_then(Value::as_str),
            Some("0xdafc7e17d609503a08b1406eb69c714cb3e7ba51e84580977c449999068ae513")
        );

        req.amount = 18_000_001;
        let err = verifier
            .verify_evm_transfer_proof(&req, &finality)
            .expect_err("wrong amount must fail event validation");
        assert_eq!(err.code, "X3_EVM_TRANSFER_EVENT_MISMATCH");
    }

    #[test]
    fn ethereum_failed_receipt_archive_proves_inclusion_then_rejects_status() {
        let (fixture, mut req) = fixture_from_archive("ethereum-mainnet-17000000-failed-receipt-proof.archive.json");
        req.source_finality_proof =
            serde_json::to_vec(fixture.get("source_finality_proof").expect("source_finality_proof"))
                .expect("finality JSON");
        req.transfer_proof =
            serde_json::to_vec(fixture.get("transfer_proof").expect("transfer_proof")).expect("transfer JSON");
        let verifier =
            EthereumLightClientVerifier::new("0x96cfa0fb5e50b0a3f6cc76f3299cfbf48f17e8b41798d1394474e67ec8a97e9f")
                .with_min_block_number(17_000_000);
        let finality = verifier
            .verify_evm_finality(&req)
            .expect("failed receipt header proof verifies");
        let err = verifier
            .verify_evm_transfer_proof(&req, &finality)
            .expect_err("failed receipt must be rejected after trie proof");
        assert_eq!(err.code, "X3_EVM_TX_FAILED");
    }

    #[test]
    fn ethereum_multilog_archive_verifies_first_transfer_log() {
        let (fixture, mut req) = fixture_from_archive("ethereum-mainnet-17000000-multilog-receipt-proof.archive.json");
        let transfer = fixture.get("transfer_proof").expect("transfer_proof");
        let log = transfer.get("log").expect("transfer log");
        let receiver = log
            .get("topics")
            .and_then(Value::as_array)
            .and_then(|topics| topics.get(2))
            .and_then(Value::as_str)
            .expect("receiver topic");
        req.from_asset = log
            .get("address")
            .and_then(Value::as_str)
            .expect("token address")
            .to_string();
        req.receiver = format!("0x{}", &receiver[receiver.len() - 40..]).into_bytes();
        req.amount = u128::from_str_radix(
            log.get("data")
                .and_then(Value::as_str)
                .expect("transfer amount")
                .trim_start_matches("0x"),
            16,
        )
        .expect("amount parses");
        req.source_finality_proof =
            serde_json::to_vec(fixture.get("source_finality_proof").expect("source_finality_proof"))
                .expect("finality JSON");
        req.transfer_proof = serde_json::to_vec(transfer).expect("transfer JSON");
        let verifier =
            EthereumLightClientVerifier::new("0x96cfa0fb5e50b0a3f6cc76f3299cfbf48f17e8b41798d1394474e67ec8a97e9f")
                .with_min_block_number(17_000_000)
                .with_erc20_transfer_event(req.from_asset.clone());
        let finality = verifier
            .verify_evm_finality(&req)
            .expect("multi-log header proof verifies");
        verifier
            .verify_evm_transfer_proof(&req, &finality)
            .expect("multi-log receipt proof verifies");
    }

    fn fixture_from_archive(name: &str) -> (Value, BridgeTransferRequest) {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|path| path.parent())
            .expect("vm crate lives under x3-lang/vm");
        let script = repo_root.join("scripts/proof/generate_eth_bridge_fixture.py");
        let archive = repo_root.join("docs/x3-lang/fixtures").join(name);
        let output = std::process::Command::new("python3")
            .arg(&script)
            .arg("--from-archive-only")
            .arg(&archive)
            .output()
            .expect("fixture generator should run");
        assert!(
            output.status.success(),
            "archive fixture generation failed for {name}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        (
            serde_json::from_slice(&output.stdout).expect("generated fixture JSON"),
            request("ethereum"),
        )
    }

    #[test]
    fn svm_light_client_verifier_checks_bank_and_transaction_signatures() {
        let bank_signing_key = SigningKey::from_bytes(&[7u8; 32]);
        let tx_signing_key = SigningKey::from_bytes(&[9u8; 32]);
        let bank_hash = hex_prefixed(&[0xabu8; 32]);
        let parent_bank_hash = hex_prefixed(&[0xcdu8; 32]);
        let slot = 123u64;
        let bank_message = format!("{SVM_BANK_PROOF_TYPE}:{slot}:{bank_hash}:{parent_bank_hash}");
        let bank_signature = bank_signing_key.sign(bank_message.as_bytes());
        let tx_message = b"compiled-solana-transaction-message";
        let tx_signature = tx_signing_key.sign(tx_message);
        let mut req = BridgeTransferRequest {
            from_chain: "solana".into(),
            from_asset: "Mint111111111111111111111111111111111111111".into(),
            receiver: b"Receiver11111111111111111111111111111111111".to_vec(),
            ..request("solana")
        };
        req.source_finality_proof = serde_json::to_vec(&json!({
            "proof_type": SVM_BANK_PROOF_TYPE,
            "slot": slot,
            "bank_hash": bank_hash,
            "parent_bank_hash": parent_bank_hash,
            "signatures": [{
                "public_key": hex_prefixed(bank_signing_key.verifying_key().as_bytes()),
                "signature": hex_prefixed(&bank_signature.to_bytes())
            }]
        }))
        .expect("bank proof JSON");
        req.transfer_proof = serde_json::to_vec(&json!({
            "proof_type": SVM_TRANSACTION_PROOF_TYPE,
            "slot": slot,
            "bank_hash": bank_hash,
            "message": hex_prefixed(tx_message),
            "transaction_hash": hex_prefixed(&Sha256::digest(tx_message)),
            "signatures": [{
                "public_key": hex_prefixed(tx_signing_key.verifying_key().as_bytes()),
                "signature": hex_prefixed(&tx_signature.to_bytes())
            }],
            "instructions": [{
                "programId": "Tokenkeg1111111111111111111111111111111111",
                "parsed": {
                    "type": "transferChecked",
                    "info": {
                        "mint": "Mint111111111111111111111111111111111111111",
                        "destination": "Receiver11111111111111111111111111111111111",
                        "tokenAmount": {
                            "amount": "42"
                        }
                    }
                }
            }]
        }))
        .expect("transaction proof JSON");

        let verifier = SolanaLightClientVerifier::new(bank_hash)
            .with_validator_pubkeys(vec![hex_prefixed(bank_signing_key.verifying_key().as_bytes())])
            .with_expected_program_id("Tokenkeg1111111111111111111111111111111111");
        let finality = verifier.verify_svm_finality(&req).expect("bank proof verifies");
        let transfer = verifier
            .verify_svm_transfer_proof(&req, &finality)
            .expect("transaction proof verifies");
        let transfer: Value = serde_json::from_slice(&transfer).expect("transfer proof JSON");
        assert_eq!(
            transfer.get("transaction_hash").and_then(Value::as_str),
            Some(hex_prefixed(&Sha256::digest(tx_message)).as_str())
        );

        let mut wrong = req.clone();
        wrong.receiver = b"OtherReceiver111111111111111111111111111".to_vec();
        let err = verifier
            .verify_svm_transfer_proof(&wrong, &finality)
            .expect_err("wrong receiver must fail parsed transfer validation");
        assert_eq!(err.code, "X3_SVM_TRANSFER_INSTRUCTION_MISMATCH");
    }

    #[test]
    fn svm_light_client_verifier_checks_epoch_stake_weighted_finality() {
        let parent_a = SigningKey::from_bytes(&[1u8; 32]);
        let parent_b = SigningKey::from_bytes(&[2u8; 32]);
        let active_a = SigningKey::from_bytes(&[3u8; 32]);
        let active_b = SigningKey::from_bytes(&[4u8; 32]);
        let inactive = SigningKey::from_bytes(&[5u8; 32]);
        let bank_hash = hex_prefixed(&[0xabu8; 32]);
        let parent_bank_hash = hex_prefixed(&[0xcdu8; 32]);
        let parent_epoch_hash = hex_prefixed(&[0x11u8; 32]);
        let epoch = 99u64;
        let validators = vec![
            SvmValidatorStake {
                public_key: hex_prefixed(active_a.verifying_key().as_bytes()),
                stake: 70,
                active: true,
            },
            SvmValidatorStake {
                public_key: hex_prefixed(active_b.verifying_key().as_bytes()),
                stake: 30,
                active: true,
            },
            SvmValidatorStake {
                public_key: hex_prefixed(inactive.verifying_key().as_bytes()),
                stake: 500,
                active: false,
            },
        ];
        let epoch_hash = svm_epoch_hash(epoch, &parent_epoch_hash, &validators);
        let transition_message = format!("{SVM_EPOCH_TRANSITION_PROOF_TYPE}:{epoch}:{parent_epoch_hash}:{epoch_hash}");
        let bank_message = format!("{SVM_BANK_PROOF_TYPE}:123:{bank_hash}:{parent_bank_hash}");
        let active_a_account =
            svm_stake_account_fixture(0x31, &hex_prefixed(active_a.verifying_key().as_bytes()), 70, 90, None);
        let active_b_account =
            svm_stake_account_fixture(0x32, &hex_prefixed(active_b.verifying_key().as_bytes()), 30, 90, None);
        let inactive_account = svm_stake_account_fixture(
            0x33,
            &hex_prefixed(inactive.verifying_key().as_bytes()),
            500,
            90,
            Some(95),
        );
        let parent_a_account =
            svm_stake_account_fixture(0x41, &hex_prefixed(parent_a.verifying_key().as_bytes()), 60, 80, None);
        let parent_b_account =
            svm_stake_account_fixture(0x42, &hex_prefixed(parent_b.verifying_key().as_bytes()), 40, 80, None);
        let active_accounts = svm_stake_accounts_with_root(vec![active_a_account, active_b_account, inactive_account]);
        let active_accounts_root = active_accounts
            .first()
            .and_then(|account| account.get("proof"))
            .and_then(|proof| proof.get("bank_accounts_root"))
            .cloned()
            .expect("active accounts root");
        let parent_accounts = svm_stake_accounts_with_root(vec![parent_a_account, parent_b_account]);
        let parent_accounts_root = parent_accounts
            .first()
            .and_then(|account| account.get("proof"))
            .and_then(|proof| proof.get("bank_accounts_root"))
            .cloned()
            .expect("parent accounts root");
        let req = BridgeTransferRequest {
            from_chain: "solana".into(),
            source_finality_proof: serde_json::to_vec(&json!({
                "proof_type": SVM_BANK_PROOF_TYPE,
                "slot": 123,
                "bank_hash": bank_hash,
                "parent_bank_hash": parent_bank_hash,
                "signatures": [{
                    "public_key": hex_prefixed(active_a.verifying_key().as_bytes()),
                    "signature": hex_prefixed(&active_a.sign(bank_message.as_bytes()).to_bytes())
                }],
                "epoch_proof": {
                    "proof_type": SVM_EPOCH_PROOF_TYPE,
                    "epoch": epoch,
                    "parent_epoch_hash": parent_epoch_hash,
                    "epoch_hash": epoch_hash,
                    "bank_accounts_root": active_accounts_root,
                    "stake_accounts": active_accounts,
                    "transition": {
                        "proof_type": SVM_EPOCH_TRANSITION_PROOF_TYPE,
                        "epoch": epoch - 1,
                        "parent_epoch_hash": parent_epoch_hash,
                        "bank_accounts_root": parent_accounts_root,
                        "stake_accounts": parent_accounts,
                        "signatures": [
                            {
                                "public_key": hex_prefixed(parent_a.verifying_key().as_bytes()),
                                "signature": hex_prefixed(&parent_a.sign(transition_message.as_bytes()).to_bytes())
                            },
                            {
                                "public_key": hex_prefixed(parent_b.verifying_key().as_bytes()),
                                "signature": hex_prefixed(&parent_b.sign(transition_message.as_bytes()).to_bytes())
                            }
                        ]
                    }
                }
            }))
            .expect("bank proof JSON"),
            ..request("solana")
        };
        let verifier = SolanaLightClientVerifier::new(&bank_hash)
            .with_trusted_epoch_hash(epoch, &epoch_hash)
            .with_min_stake_threshold_bps(6_667);
        let finality = verifier
            .verify_svm_finality(&req)
            .expect("stake-weighted bank proof verifies");
        let finality: Value = serde_json::from_slice(&finality).expect("finality JSON");
        assert_eq!(finality.get("epoch").and_then(Value::as_u64), Some(epoch));
        assert_eq!(finality.get("signed_stake").and_then(Value::as_str), Some("70"));
    }

    #[test]
    fn svm_light_client_rejects_insufficient_epoch_stake_and_bad_transition() {
        let parent_a = SigningKey::from_bytes(&[10u8; 32]);
        let parent_b = SigningKey::from_bytes(&[11u8; 32]);
        let active_a = SigningKey::from_bytes(&[12u8; 32]);
        let active_b = SigningKey::from_bytes(&[13u8; 32]);
        let bank_hash = hex_prefixed(&[0xeeu8; 32]);
        let parent_bank_hash = hex_prefixed(&[0xddu8; 32]);
        let parent_epoch_hash = hex_prefixed(&[0x22u8; 32]);
        let epoch = 100u64;
        let validators = vec![
            SvmValidatorStake {
                public_key: hex_prefixed(active_a.verifying_key().as_bytes()),
                stake: 40,
                active: true,
            },
            SvmValidatorStake {
                public_key: hex_prefixed(active_b.verifying_key().as_bytes()),
                stake: 60,
                active: true,
            },
        ];
        let epoch_hash = svm_epoch_hash(epoch, &parent_epoch_hash, &validators);
        let bank_message = format!("{SVM_BANK_PROOF_TYPE}:321:{bank_hash}:{parent_bank_hash}");
        let transition_message = format!("{SVM_EPOCH_TRANSITION_PROOF_TYPE}:{epoch}:{parent_epoch_hash}:{epoch_hash}");
        let base_epoch_proof = json!({
            "proof_type": SVM_EPOCH_PROOF_TYPE,
            "epoch": epoch,
            "parent_epoch_hash": parent_epoch_hash,
            "epoch_hash": epoch_hash,
            "validators": [
                {
                    "public_key": hex_prefixed(active_a.verifying_key().as_bytes()),
                    "stake": "40",
                    "active": true
                },
                {
                    "public_key": hex_prefixed(active_b.verifying_key().as_bytes()),
                    "stake": "60",
                    "active": true
                }
            ],
            "transition": {
                "proof_type": SVM_EPOCH_TRANSITION_PROOF_TYPE,
                "parent_epoch_hash": parent_epoch_hash,
                "validators": [
                    {
                        "public_key": hex_prefixed(parent_a.verifying_key().as_bytes()),
                        "stake": "60",
                        "active": true
                    },
                    {
                        "public_key": hex_prefixed(parent_b.verifying_key().as_bytes()),
                        "stake": "40",
                        "active": true
                    }
                ],
                "signatures": [
                    {
                        "public_key": hex_prefixed(parent_a.verifying_key().as_bytes()),
                        "signature": hex_prefixed(&parent_a.sign(transition_message.as_bytes()).to_bytes())
                    },
                    {
                        "public_key": hex_prefixed(parent_b.verifying_key().as_bytes()),
                        "signature": hex_prefixed(&parent_b.sign(transition_message.as_bytes()).to_bytes())
                    }
                ]
            }
        });
        let mut req = BridgeTransferRequest {
            from_chain: "solana".into(),
            source_finality_proof: serde_json::to_vec(&json!({
                "proof_type": SVM_BANK_PROOF_TYPE,
                "slot": 321,
                "bank_hash": bank_hash,
                "parent_bank_hash": parent_bank_hash,
                "signatures": [{
                    "public_key": hex_prefixed(active_a.verifying_key().as_bytes()),
                    "signature": hex_prefixed(&active_a.sign(bank_message.as_bytes()).to_bytes())
                }],
                "epoch_proof": base_epoch_proof
            }))
            .expect("bank proof JSON"),
            ..request("solana")
        };
        let verifier = SolanaLightClientVerifier::new(&bank_hash)
            .with_trusted_epoch_hash(epoch, &epoch_hash)
            .with_min_stake_threshold_bps(6_667);
        let err = verifier
            .verify_svm_finality(&req)
            .expect_err("40% signed stake must fail threshold");
        assert_eq!(err.code, "X3_SVM_BANK_STAKE_THRESHOLD_NOT_MET");

        let bad_transition_message = b"wrong-transition-message";
        req.source_finality_proof = serde_json::to_vec(&json!({
            "proof_type": SVM_BANK_PROOF_TYPE,
            "slot": 321,
            "bank_hash": bank_hash,
            "parent_bank_hash": parent_bank_hash,
            "signatures": [
                {
                    "public_key": hex_prefixed(active_a.verifying_key().as_bytes()),
                    "signature": hex_prefixed(&active_a.sign(bank_message.as_bytes()).to_bytes())
                },
                {
                    "public_key": hex_prefixed(active_b.verifying_key().as_bytes()),
                    "signature": hex_prefixed(&active_b.sign(bank_message.as_bytes()).to_bytes())
                }
            ],
            "epoch_proof": {
                "proof_type": SVM_EPOCH_PROOF_TYPE,
                "epoch": epoch,
                "parent_epoch_hash": parent_epoch_hash,
                "epoch_hash": epoch_hash,
                "validators": [
                    {
                        "public_key": hex_prefixed(active_a.verifying_key().as_bytes()),
                        "stake": "40",
                        "active": true
                    },
                    {
                        "public_key": hex_prefixed(active_b.verifying_key().as_bytes()),
                        "stake": "60",
                        "active": true
                    }
                ],
                "transition": {
                    "proof_type": SVM_EPOCH_TRANSITION_PROOF_TYPE,
                    "parent_epoch_hash": parent_epoch_hash,
                    "validators": [
                        {
                            "public_key": hex_prefixed(parent_a.verifying_key().as_bytes()),
                            "stake": "100",
                            "active": true
                        }
                    ],
                    "signatures": [{
                        "public_key": hex_prefixed(parent_a.verifying_key().as_bytes()),
                        "signature": hex_prefixed(&parent_a.sign(bad_transition_message).to_bytes())
                    }]
                }
            }
        }))
        .expect("bad transition proof JSON");
        let err = verifier
            .verify_svm_finality(&req)
            .expect_err("bad transition signature must fail");
        assert_eq!(err.code, "X3_ED25519_SIGNATURE_INVALID");
    }

    #[test]
    fn svm_epoch_proof_rejects_tampered_stake_account_data_hash() {
        let active = SigningKey::from_bytes(&[20u8; 32]);
        let bank_hash = hex_prefixed(&[0x44u8; 32]);
        let parent_bank_hash = hex_prefixed(&[0x45u8; 32]);
        let parent_epoch_hash = hex_prefixed(&[0x46u8; 32]);
        let epoch = 7u64;
        let validators = vec![SvmValidatorStake {
            public_key: hex_prefixed(active.verifying_key().as_bytes()),
            stake: 100,
            active: true,
        }];
        let epoch_hash = svm_epoch_hash(epoch, &parent_epoch_hash, &validators);
        let bank_message = format!("{SVM_BANK_PROOF_TYPE}:77:{bank_hash}:{parent_bank_hash}");
        let stake_account =
            svm_stake_account_fixture(0x55, &hex_prefixed(active.verifying_key().as_bytes()), 100, 1, None);
        let mut stake_accounts = svm_stake_accounts_with_root(vec![stake_account]);
        let stake_accounts_root = stake_accounts
            .first()
            .and_then(|account| account.get("proof"))
            .and_then(|proof| proof.get("bank_accounts_root"))
            .cloned()
            .expect("stake accounts root");
        let mut stake_account = stake_accounts.remove(0);
        stake_account["data_hash"] = json!(hex_prefixed(&[0x99u8; 32]));
        let req = BridgeTransferRequest {
            from_chain: "solana".into(),
            source_finality_proof: serde_json::to_vec(&json!({
                "proof_type": SVM_BANK_PROOF_TYPE,
                "slot": 77,
                "bank_hash": bank_hash,
                "parent_bank_hash": parent_bank_hash,
                "signatures": [{
                    "public_key": hex_prefixed(active.verifying_key().as_bytes()),
                    "signature": hex_prefixed(&active.sign(bank_message.as_bytes()).to_bytes())
                }],
                "epoch_proof": {
                    "proof_type": SVM_EPOCH_PROOF_TYPE,
                    "epoch": epoch,
                    "parent_epoch_hash": parent_epoch_hash,
                    "epoch_hash": epoch_hash,
                    "bank_accounts_root": stake_accounts_root,
                    "stake_accounts": [stake_account]
                }
            }))
            .expect("bank proof JSON"),
            ..request("solana")
        };
        let verifier = SolanaLightClientVerifier::new(&bank_hash).with_trusted_epoch_hash(epoch, &epoch_hash);
        let err = verifier
            .verify_svm_finality(&req)
            .expect_err("tampered stake account data hash must fail");
        assert_eq!(err.code, "X3_SVM_STAKE_ACCOUNT_DATA_HASH_MISMATCH");
    }

    #[test]
    fn svm_epoch_stake_account_fixture_decodes_to_active_stake() {
        let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|path| path.parent())
            .expect("vm crate lives under x3-lang/vm");
        let fixture_path = repo_root.join("docs/x3-lang/fixtures/solana-epoch-stake-account-proof.fixture.json");
        let fixture: Value = serde_json::from_slice(&std::fs::read(&fixture_path).expect("solana stake fixture reads"))
            .expect("solana stake fixture JSON");
        let epoch_hash = expect_str(&fixture, "epoch_hash").expect("epoch_hash").to_string();
        let epoch = verify_svm_epoch_proof(
            &fixture,
            Some(&TrustedSvmEpoch { epoch: 99, epoch_hash }),
            DEFAULT_SVM_STAKE_THRESHOLD_BPS,
        )
        .expect("fixture stake accounts verify");
        assert_eq!(epoch.total_active_stake, 100);
        assert_eq!(epoch.active_stakes.get(&hex_prefixed(&[0x03u8; 32])), Some(&70));
        assert!(!epoch.active_stakes.contains_key(&hex_prefixed(&[0x05u8; 32])));
    }

    fn svm_stake_account_fixture(
        account_seed: u8,
        voter_pubkey: &str,
        delegated_stake: u128,
        activation_epoch: u64,
        deactivation_epoch: Option<u64>,
    ) -> Value {
        let account_pubkey = hex_prefixed(&[account_seed; 32]);
        let data = solana_stake_state_data(voter_pubkey, delegated_stake, activation_epoch, deactivation_epoch);
        let data_hash = hex_prefixed(&Sha256::digest(&data));
        let owner = "Stake11111111111111111111111111111111111111";
        let lamports = delegated_stake.to_string();
        let account_hash = svm_account_hash(&account_pubkey, owner, &lamports, &data_hash);
        json!({
            "proof_type": SVM_STAKE_ACCOUNT_DATA_TYPE,
            "account_pubkey": account_pubkey,
            "data_encoding": "solana-bincode-stake-state-v2",
            "data": hex_prefixed(&data),
            "data_hash": data_hash,
            "proof": {
                "account_pubkey": account_pubkey,
                "owner": owner,
                "lamports": lamports,
                "data_hash": data_hash,
                "account_hash": account_hash
            }
        })
    }

    fn svm_stake_accounts_with_root(mut accounts: Vec<Value>) -> Vec<Value> {
        let mut levels: Vec<Vec<Vec<u8>>> = Vec::new();
        levels.push(
            accounts
                .iter()
                .map(|account| {
                    let account_hash = account
                        .get("proof")
                        .and_then(|proof| proof.get("account_hash"))
                        .and_then(Value::as_str)
                        .expect("account_hash");
                    hex_to_bytes(account_hash, "account_hash").expect("account hash hex")
                })
                .collect(),
        );
        while levels.last().expect("levels").len() > 1 {
            let current = levels.last().expect("current level");
            let mut next = Vec::new();
            for pair in current.chunks(2) {
                let left = &pair[0];
                let right = pair.get(1).unwrap_or(left);
                next.push(Sha256::digest([left.clone(), right.clone()].concat()).to_vec());
            }
            levels.push(next);
        }
        let root = hex_prefixed(&levels.last().expect("root level")[0]);
        for (account_index, account) in accounts.iter_mut().enumerate() {
            let mut path = Vec::new();
            let mut index = account_index;
            for level in &levels[..levels.len() - 1] {
                let is_left = index % 2 == 0;
                let sibling_index = if is_left {
                    (index + 1).min(level.len() - 1)
                } else {
                    index - 1
                };
                path.push(json!({
                    "direction": if is_left { "right" } else { "left" },
                    "sibling": hex_prefixed(&level[sibling_index]),
                }));
                index /= 2;
            }
            account["proof"]["bank_accounts_root"] = json!(root);
            account["proof"]["merkle_proof"] = Value::Array(path);
        }
        accounts
    }

    fn solana_stake_state_data(
        voter_pubkey: &str,
        delegated_stake: u128,
        activation_epoch: u64,
        deactivation_epoch: Option<u64>,
    ) -> Vec<u8> {
        let voter_bytes: [u8; 32] = hex_to_bytes(voter_pubkey, "test voter pubkey")
            .expect("voter pubkey hex")
            .try_into()
            .expect("test voter pubkey is 32 bytes");
        let stake = SolanaStakeStateV2Wire::Stake(
            SolanaStakeMetaWire {
                rent_exempt_reserve: 0,
                authorized: SolanaAuthorizedWire {
                    staker: [0x77u8; 32],
                    withdrawer: [0x77u8; 32],
                },
                lockup: SolanaLockupWire {
                    unix_timestamp: 0,
                    epoch: 0,
                    custodian: [0u8; 32],
                },
            },
            SolanaStakeWire {
                delegation: SolanaDelegationWire {
                    voter_pubkey: voter_bytes,
                    stake: delegated_stake.try_into().expect("test delegated stake fits u64"),
                    activation_epoch,
                    deactivation_epoch: deactivation_epoch.unwrap_or(u64::MAX),
                    warmup_cooldown_rate: 0.25,
                },
                credits_observed: 0,
            },
            SolanaStakeFlagsWire { bits: 0 },
        );
        bincode::serialize(&stake).expect("StakeStateV2 serializes")
    }

    fn evm_test_header_rlp(receipts_root: [u8; 32], number: u64) -> Vec<u8> {
        rlp_list(vec![
            rlp_bytes(&[0; 32]),
            rlp_bytes(&[1; 32]),
            rlp_bytes(&[2; 20]),
            rlp_bytes(&[3; 32]),
            rlp_bytes(&[4; 32]),
            rlp_bytes(&receipts_root),
            rlp_bytes(&vec![0; 256]),
            rlp_bytes(&[1]),
            rlp_bytes(&[number as u8]),
            rlp_bytes(&[0x0f, 0x42, 0x40]),
            rlp_bytes(&[0]),
            rlp_bytes(&[1]),
            rlp_bytes(&[]),
            rlp_bytes(&[5; 32]),
            rlp_bytes(&[6; 8]),
        ])
    }

    fn rlp_bytes(bytes: &[u8]) -> Vec<u8> {
        if bytes.len() == 1 && bytes[0] < 0x80 {
            return bytes.to_vec();
        }
        let mut out = rlp_len_prefix(bytes.len(), 0x80);
        out.extend_from_slice(bytes);
        out
    }

    fn rlp_list(items: Vec<Vec<u8>>) -> Vec<u8> {
        let payload = items.concat();
        let mut out = rlp_len_prefix(payload.len(), 0xc0);
        out.extend_from_slice(&payload);
        out
    }

    fn compact_leaf_path(nibbles: &[u8]) -> Vec<u8> {
        let mut prefixed = if nibbles.len() % 2 == 1 {
            vec![0x03]
        } else {
            vec![0x02, 0x00]
        };
        prefixed.extend_from_slice(nibbles);
        nibbles_to_bytes(&prefixed)
    }

    fn compact_extension_path(nibbles: &[u8]) -> Vec<u8> {
        let mut prefixed = if nibbles.len() % 2 == 1 {
            vec![0x01]
        } else {
            vec![0x00, 0x00]
        };
        prefixed.extend_from_slice(nibbles);
        nibbles_to_bytes(&prefixed)
    }

    fn nibbles_to_bytes(nibbles: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(nibbles.len().div_ceil(2));
        for chunk in nibbles.chunks(2) {
            let high = chunk[0] << 4;
            let low = chunk.get(1).copied().unwrap_or(0);
            out.push(high | low);
        }
        out
    }

    fn rlp_len_prefix(len: usize, offset: u8) -> Vec<u8> {
        if len < 56 {
            return vec![offset + len as u8];
        }
        let len_bytes = len.to_be_bytes();
        let first_nonzero = len_bytes
            .iter()
            .position(|byte| *byte != 0)
            .unwrap_or(len_bytes.len() - 1);
        let encoded_len = &len_bytes[first_nonzero..];
        let mut out = vec![offset + 55 + encoded_len.len() as u8];
        out.extend_from_slice(encoded_len);
        out
    }
}
