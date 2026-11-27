//! # x3-mpc-guard: the only route from an X3 agent to a NEAR Chain Signatures signature
//!
//! **Status: draft, not deployed, no signature has been requested.** This contract is separate
//! from the deployed `x3-guard` (which is frozen) and does not call or change it.
//!
//! NEAR Chain Signatures derives a foreign key from *the account that calls* the MPC signer's
//! `sign` method and a derivation path. This contract is that account. The foreign address an
//! owner funds is therefore derived from this contract's account ID, and only this contract can
//! obtain a signature for it. An agent that calls the signer directly gets signatures for a
//! different, unfunded key derived from its own account.
//!
//! The flow, every step on chain:
//!
//! 1. **Account.** The owner calls [`Contract::register_foreign_account`]. The contract derives
//!    the path (`x3-mpc-guard/v1/evm/<owner>/<key_index>`), the MPC public key and the EVM
//!    address itself, then asks the signer's `derived_public_key` for the same path and
//!    predecessor. The account is usable only if the two agree ([`AccountStatus::Verified`]).
//! 2. **Mandate.** The owner calls [`Contract::create_mandate`] with the agent's terms (see
//!    [`authority`]). [`Contract::revoke_mandate`] stops the agent at once.
//! 3. **Proposal.** The agent calls [`Contract::agent_propose_signing`] with structured fields
//!    only. The contract checks every term, builds the canonical unsigned EIP-1559 transaction
//!    and stores a versioned [`authority::X3SigningIntent`] with its signing hash.
//! 4. **Approval.** Above `approval_above`, distinct approvers call
//!    [`Contract::approve_signing`] with the intent hash they reviewed.
//! 5. **Request.** The agent calls [`Contract::agent_request_signature`]. Every term is checked
//!    again; the spend is counted; the EVM nonce is consumed; then, and only then, the contract
//!    calls `sign` on the MPC signer with the stored hash, the derived path and the pinned
//!    domain. A refusal panics before any promise exists.
//! 6. **Result.** [`Contract::on_signature`] verifies the returned signature recovers to the
//!    derived key and stores the signed transaction for anyone to broadcast. A failed or
//!    timed-out request leaves the intent retryable with the identical payload.
//!
//! The MPC interface is pinned to `near/mpc` commit `686fbb2de1a6b426303c23617ab84773f15bfb19`
//! (workspace 3.19.0): `sign(request: SignRequestArgs { path, payload_v2: {"Ecdsa": hex32},
//! domain_id })`, at least 1 yoctoNEAR attached, at least
//! `sign_call_gas_attachment_requirement_tera_gas` (15 Tgas by default in `config.rs`; the
//! README says 10) prepaid, answered through yield/resume by
//! `return_signature_and_clean_state_on_success` with a `SignatureResponse`, or failed by
//! `fail_on_timeout`.
//!
//! ## Deployment preconditions (not checked by code)
//!
//! - The contract account must hold **no access keys** after deployment and initialization.
//!   Anyone holding a key for it can call `sign` as this account and bypass every check.
//! - The contract has no owner, admin, upgrade or migration method; changing it means a new
//!   account, a new derived address and new funding.
//! - `mpc_root_public_key` must be the signer's `public_key` for `mpc_domain_id`; a wrong value
//!   is caught on chain by step 1 and leaves accounts in [`AccountStatus::Mismatch`].

#[cfg(all(target_arch = "wasm32", not(near)))]
compile_error!(
    "x3-mpc-guard: WASM must be built with `--cfg near` (cargo-near, or this crate's .cargo/config.toml). \
     Without it near-sdk drops every refusal reason."
);

pub mod authority;
pub mod derivation;
pub mod evm;
pub mod hex;

use near_sdk::json_types::U128;
use near_sdk::serde::Deserialize;
use near_sdk::serde_json::{self, json};
use near_sdk::store::LookupMap;
use near_sdk::{env, ext_contract, near, require, AccountId, BorshStorageKey, Gas, NearToken, PanicOnDefault, Promise};

use authority::{
    build_intent, build_transaction, check_signing_authority, count_approvals, failed, intent_hash, valid_nonce,
    validate_terms, GuardResult, SigningContext, SigningMandate, SigningMandateTerms, SigningRequest, Spend,
    X3SigningIntent, WINDOW_MS,
};
use derivation::{
    compressed_point_x, derive_public_key, derive_tweak, evm_address, format_near_secp256k1_key,
    parse_near_secp256k1_key, validate_public_key,
};
use evm::EvmSignature;

pub const EVENT_STANDARD: &str = "x3mpcguard";
pub const EVENT_VERSION: &str = "0.1.0";
/// Derivation-path domain. The full path is `<domain>/<owner>/<key_index>`.
pub const PATH_DOMAIN: &str = "x3-mpc-guard/v1/evm";
/// `near-mpc-contract-interface/src/deposits.rs`: `SIGN_DEPOSIT_YOCTONEAR = 1`.
pub const SIGN_DEPOSIT_YOCTO: u128 = 1;
/// Gas attached to `sign`: above the signer's 15 Tgas requirement, which also pays its
/// 7 Tgas yield callback.
pub const SIGN_GAS: Gas = Gas::from_tgas(50);
pub const SIGN_CALLBACK_GAS: Gas = Gas::from_tgas(30);
/// The request must carry enough gas for this call, `sign` and the callback.
pub const MIN_REQUEST_GAS: Gas = Gas::from_tgas(120);
pub const DERIVED_KEY_GAS: Gas = Gas::from_tgas(30);
pub const DERIVED_KEY_CALLBACK_GAS: Gas = Gas::from_tgas(15);
/// A request with no callback after this long may be marked failed (the MPC yield times out
/// after about 200 blocks).
pub const STALE_REQUEST_MS: u64 = 900_000;
/// Proposals may reserve EVM nonces at most this far ahead of the next unsigned one.
pub const MAX_NONCE_LOOKAHEAD: u64 = 16;
/// Unspent credit an owner may prepay for one agent: 1 NEAR.
pub const MAX_STORAGE_CREDIT: u128 = 1_000_000_000_000_000_000_000_000;
const MAX_SIGNER_RESULT: usize = 1024;

/// The signer's `sign` argument, as `near-mpc-crypto-types/src/sign.rs` serializes it.
#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq)]
pub struct SignRequestArgs {
    pub path: String,
    pub payload_v2: SignPayload,
    pub domain_id: u64,
}

#[near(serializers = [json])]
#[derive(Clone, Debug, PartialEq)]
pub enum SignPayload {
    /// 32 bytes, lowercase hex.
    Ecdsa(String),
}

#[allow(dead_code)]
#[ext_contract(ext_mpc_signer)]
trait MpcSigner {
    fn sign(&mut self, request: SignRequestArgs);
    fn derived_public_key(&self, path: String, predecessor: Option<AccountId>, domain_id: Option<u64>) -> String;
}

#[near(serializers = [borsh])]
#[derive(BorshStorageKey)]
enum StorageKey {
    Mandates,
    Accounts,
    NextNonces,
    Intents,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum AccountStatus {
    /// Derived locally; waiting for the signer's `derived_public_key` to confirm it.
    PendingSignerCheck,
    /// The signer derived the same key. Signing is possible.
    Verified,
    /// The signer derived a different key. This account can never sign.
    Mismatch,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct ForeignAccount {
    pub owner: AccountId,
    pub key_index: u32,
    pub derivation_path: String,
    pub derived_public_key: String,
    pub address: String,
    pub status: AccountStatus,
    /// The intent whose signature request is outstanding. One at a time per account.
    pub in_flight: Option<String>,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub enum SigningStatus {
    Proposed,
    SignRequested,
    Signed,
    SignFailed,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct StoredSignature {
    pub y_parity: u8,
    pub r: String,
    pub s: String,
}

#[near(serializers = [borsh, json])]
#[derive(Clone, Debug, PartialEq)]
pub struct SigningRecord {
    pub intent: X3SigningIntent,
    pub intent_hash: String,
    pub approvals: Vec<AccountId>,
    pub status: SigningStatus,
    /// Signature requests made for this intent; a callback must name the current one.
    pub attempts: u32,
    pub requested_at_ms: Option<u64>,
    /// The spend was counted and the EVM nonce consumed by the first request.
    pub spend_counted: bool,
    pub signature: Option<StoredSignature>,
    /// The broadcastable signed transaction and its hash, lowercase hex.
    pub signed_tx: Option<String>,
    pub tx_hash: Option<String>,
    pub last_failure: Option<String>,
}

#[near(contract_state)]
#[derive(PanicOnDefault)]
pub struct Contract {
    mpc_signer: AccountId,
    mpc_root_public_key: [u8; 64],
    mpc_domain_id: u64,
    mandates: LookupMap<String, SigningMandate>,
    accounts: LookupMap<String, ForeignAccount>,
    next_nonces: LookupMap<String, u64>,
    intents: LookupMap<String, SigningRecord>,
}

#[near(serializers = [json])]
pub struct Config {
    pub mpc_signer: AccountId,
    pub mpc_root_public_key: String,
    pub mpc_domain_id: u64,
    pub path_domain: String,
    pub intent_version: u16,
}

pub fn derivation_path(owner: &AccountId, key_index: u32) -> String {
    format!("{PATH_DOMAIN}/{owner}/{key_index}")
}

fn mandate_key(owner: &AccountId, agent: &AccountId) -> String {
    format!("{owner}/{agent}")
}

fn account_key(owner: &AccountId, key_index: u32) -> String {
    format!("{owner}#{key_index}")
}

fn nonce_key(owner: &AccountId, key_index: u32, chain_id: u64) -> String {
    format!("{owner}#{key_index}#{chain_id}")
}

fn intent_key(owner: &AccountId, nonce: &str) -> String {
    format!("{owner}/{nonce}")
}

fn emit(event: &str, data: serde_json::Value) {
    env::log_str(&format!(
        "EVENT_JSON:{}",
        json!({"standard": EVENT_STANDARD, "version": EVENT_VERSION, "event": event, "data": [data]})
    ));
}

/// `#[private]` is enforced by the generated entry point; this repeats it inside the method so
/// the rule holds for every caller, including native tests that call the method directly.
fn require_self_callback() {
    require!(
        env::predecessor_account_id() == env::current_account_id(),
        "X3_MPC_DENIED: only this contract can deliver a callback"
    );
}

fn refuse(results: &[GuardResult], what: &str) {
    let failures = failed(results);
    require!(
        failures.is_empty(),
        format!("X3_MPC_DENIED: {what} refused by {}", failures.join("; "))
    );
}

fn storage_cost_since(initial: u64) -> u128 {
    env::storage_byte_cost()
        .as_yoctonear()
        .checked_mul(env::storage_usage().saturating_sub(initial) as u128)
        .unwrap_or_else(|| env::panic_str("storage cost overflow"))
}

/// Caller-paid storage: the attached deposit must cover growth; the excess is refunded.
fn settle_storage_deposit(initial: u64) {
    let cost = storage_cost_since(initial);
    let deposit = env::attached_deposit().as_yoctonear();
    require!(deposit >= cost, format!("attach at least {cost} yoctoNEAR for storage"));
    if deposit > cost {
        Promise::new(env::predecessor_account_id())
            .transfer(NearToken::from_yoctonear(deposit - cost))
            .detach();
    }
}

/// The request an intent was built from, for re-checking it at request time.
pub fn request_of(intent: &X3SigningIntent) -> SigningRequest {
    SigningRequest {
        intent_nonce: intent.intent_nonce.clone(),
        program_sha256: intent.program_sha256.clone(),
        chain_id: intent.chain_id,
        key_index: intent.key_index,
        asset: intent.asset.clone(),
        recipient: intent.recipient.clone(),
        amount: intent.amount,
        evm_nonce: intent.evm_nonce,
        max_priority_fee_per_gas: intent.max_priority_fee_per_gas,
        max_fee_per_gas: intent.max_fee_per_gas,
        gas_limit: intent.gas_limit,
        expires_at_ms: intent.expires_at_ms,
    }
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct MpcAffinePoint {
    affine_point: String,
}

#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct MpcScalar {
    scalar: String,
}

/// `SignatureResponse::Secp256k1(K256Signature)`, internally tagged by `scheme`
/// (`near-mpc-crypto-types/src/primitives.rs`).
#[derive(Deserialize)]
#[serde(crate = "near_sdk::serde")]
struct MpcSignatureResponse {
    scheme: String,
    big_r: MpcAffinePoint,
    s: MpcScalar,
    recovery_id: u8,
}

/// Parse the signer's response and accept it only if it is a low-s ECDSA signature of `hash`
/// that recovers, with its recovery ID, to `derived` (the account's verified key).
pub fn verify_signature_response(bytes: &[u8], hash: &[u8; 32], derived: &[u8; 64]) -> Result<EvmSignature, String> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let response: MpcSignatureResponse =
        serde_json::from_slice(bytes).map_err(|e| format!("unreadable signature response: {e}"))?;
    if response.scheme != "Secp256k1" {
        return Err(format!("expected a Secp256k1 signature, got {}", response.scheme));
    }
    let big_r: [u8; 33] = hex::decode_array(&response.big_r.affine_point).ok_or("big_r is not 33 bytes of hex")?;
    let r = compressed_point_x(&big_r)?;
    let s: [u8; 32] = hex::decode_array(&response.s.scalar).ok_or("s is not 32 bytes of hex")?;
    let signature = Signature::from_scalars(r, s).map_err(|_| "r or s is out of range".to_string())?;
    if signature.normalize_s().is_some() {
        return Err("s is not low (EIP-2)".into());
    }
    if response.recovery_id > 1 {
        return Err("recovery_id must be 0 or 1".into());
    }
    let recovery = RecoveryId::from_byte(response.recovery_id).ok_or("invalid recovery_id")?;
    let mut sec1 = [0u8; 65];
    sec1[0] = 0x04;
    sec1[1..].copy_from_slice(derived);
    let expected = VerifyingKey::from_sec1_bytes(&sec1).map_err(|_| "derived key is invalid".to_string())?;
    let recovered = VerifyingKey::recover_from_prehash(hash, &signature, recovery)
        .map_err(|_| "the signature does not recover".to_string())?;
    if recovered != expected {
        return Err("the signature is not by the derived key".into());
    }
    Ok(EvmSignature {
        y_parity: response.recovery_id,
        r,
        s,
    })
}

impl Contract {
    fn load_mandate(&self, owner: &AccountId, agent: &AccountId) -> SigningMandate {
        self.mandates
            .get(&mandate_key(owner, agent))
            .cloned()
            .unwrap_or_else(|| env::panic_str(&format!("X3_MPC_DENIED: {owner} has no signing mandate for {agent}")))
    }

    fn store_mandate(&mut self, m: &SigningMandate) {
        self.mandates.insert(mandate_key(&m.owner, &m.agent), m.clone());
        self.mandates.flush();
    }

    fn load_account(&self, owner: &AccountId, key_index: u32) -> ForeignAccount {
        self.accounts
            .get(&account_key(owner, key_index))
            .cloned()
            .unwrap_or_else(|| env::panic_str(&format!("X3_MPC_DENIED: {owner} has no foreign account {key_index}")))
    }

    fn store_account(&mut self, a: &ForeignAccount) {
        self.accounts.insert(account_key(&a.owner, a.key_index), a.clone());
        self.accounts.flush();
    }

    fn load_record(&self, owner: &AccountId, nonce: &str) -> SigningRecord {
        self.intents
            .get(&intent_key(owner, nonce))
            .cloned()
            .unwrap_or_else(|| env::panic_str(&format!("no signing intent {nonce:?} for {owner}")))
    }

    fn store_record(&mut self, rec: &SigningRecord) {
        self.intents
            .insert(intent_key(&rec.intent.owner, &rec.intent.intent_nonce), rec.clone());
        self.intents.flush();
    }

    fn next_nonce(&self, owner: &AccountId, key_index: u32, chain_id: u64) -> u64 {
        self.next_nonces
            .get(&nonce_key(owner, key_index, chain_id))
            .copied()
            .unwrap_or(0)
    }

    fn derive_account(&self, owner: &AccountId, key_index: u32) -> (String, [u8; 64]) {
        let path = derivation_path(owner, key_index);
        let tweak = derive_tweak(env::current_account_id().as_str(), &path);
        let key = derive_public_key(&self.mpc_root_public_key, &tweak).unwrap_or_else(|e| env::panic_str(e));
        (path, key)
    }

    /// Charge storage growth since `initial`, plus `extra`, to the mandate's prepaid credit.
    fn charge_credit(&mut self, m: &mut SigningMandate, initial: u64, extra: u128) {
        let cost = storage_cost_since(initial)
            .checked_add(extra)
            .unwrap_or_else(|| env::panic_str("cost overflow"));
        m.storage_credit = U128(m.storage_credit.0.checked_sub(cost).unwrap_or_else(|| {
            env::panic_str("X3_MPC_DENIED: the owner's prepaid credit for this agent is exhausted")
        }));
        self.store_mandate(m);
    }

    /// Every check a signature request must pass, recorded term by term. Pure over state.
    pub fn request_checks(&self, rec: &SigningRecord, m: &SigningMandate, now: u64) -> Vec<GuardResult> {
        let intent = &rec.intent;
        let mut out = Vec::new();
        let mut push = |guard: &str, ok: bool, detail: String| {
            out.push(GuardResult {
                guard: guard.into(),
                ok,
                detail,
            })
        };
        push(
            "mandate_version",
            intent.mandate_version == m.version,
            format!(
                "intent under version {}, mandate is version {}",
                intent.mandate_version, m.version
            ),
        );
        push(
            "intent_status",
            matches!(rec.status, SigningStatus::Proposed | SigningStatus::SignFailed),
            format!("{:?}", rec.status),
        );
        let request = request_of(intent);
        let mut results = check_signing_authority(m, &request, now, rec.spend_counted);
        out.append(&mut results);
        let needed = m.approvals_needed(intent.amount.0) as usize;
        let have = count_approvals(m, &rec.approvals);
        out.push(GuardResult {
            guard: "approvals".into(),
            ok: have >= needed,
            detail: format!("{have} of {needed} distinct approvals"),
        });
        let account = self.accounts.get(&account_key(&intent.owner, intent.key_index));
        out.push(GuardResult {
            guard: "foreign_account".into(),
            ok: account.is_some_and(|a| a.status == AccountStatus::Verified && a.address == intent.from),
            detail: format!("{:?}", account.map(|a| (&a.status, &a.address))),
        });
        out.push(GuardResult {
            guard: "no_request_in_flight".into(),
            ok: account.is_some_and(|a| a.in_flight.is_none()),
            detail: format!("{:?}", account.and_then(|a| a.in_flight.clone())),
        });
        let next = self.next_nonce(&intent.owner, intent.key_index, intent.chain_id);
        out.push(GuardResult {
            guard: "evm_nonce".into(),
            // The first request consumes exactly the next nonce; a retry re-signs the same one.
            ok: rec.spend_counted || intent.evm_nonce == next,
            detail: format!("intent nonce {}, next unsigned {next}", intent.evm_nonce),
        });
        let rebuilt =
            build_transaction(&request).map(|tx| (hex::encode(&tx.unsigned_bytes()), hex::encode(&tx.signing_hash())));
        out.push(GuardResult {
            guard: "canonical_transaction".into(),
            ok: rebuilt
                .as_ref()
                .is_ok_and(|(bytes, hash)| *bytes == intent.unsigned_tx && *hash == intent.signing_hash)
                && intent_hash(intent) == rec.intent_hash,
            detail: intent.signing_hash.clone(),
        });
        out.push(GuardResult {
            guard: "payload_is_scalar".into(),
            ok: hex::decode_array::<32>(&intent.signing_hash).is_some_and(|h| {
                Option::<k256::Scalar>::from(<k256::Scalar as k256::elliptic_curve::PrimeField>::from_repr(h.into()))
                    .is_some()
            }),
            detail: "the MPC signer refuses an ECDSA payload not below the group order".into(),
        });
        out.push(GuardResult {
            guard: "derivation".into(),
            ok: intent.mpc_signer == self.mpc_signer
                && intent.mpc_domain_id == self.mpc_domain_id
                && intent.derivation_predecessor == env::current_account_id()
                && intent.derivation_path == derivation_path(&intent.owner, intent.key_index),
            detail: format!(
                "{} domain {} path {}",
                intent.mpc_signer, intent.mpc_domain_id, intent.derivation_path
            ),
        });
        out
    }
}

#[near]
impl Contract {
    /// `mpc_signer`: `v1.signer` (mainnet) or `v1.signer-prod.testnet` (testnet).
    /// `mpc_root_public_key`: the signer's `public_key(domain_id)`, `secp256k1:<base58>`.
    /// `mpc_domain_id`: the signer's secp256k1 ECDSA domain (0 is the legacy ECDSA domain).
    #[init]
    pub fn new(mpc_signer: AccountId, mpc_root_public_key: String, mpc_domain_id: u64) -> Self {
        let key = parse_near_secp256k1_key(&mpc_root_public_key).unwrap_or_else(|e| env::panic_str(e));
        validate_public_key(&key).unwrap_or_else(|e| env::panic_str(e));
        require!(
            mpc_signer != env::current_account_id(),
            "the signer must be a different account"
        );
        Self {
            mpc_signer,
            mpc_root_public_key: key,
            mpc_domain_id,
            mandates: LookupMap::new(StorageKey::Mandates),
            accounts: LookupMap::new(StorageKey::Accounts),
            next_nonces: LookupMap::new(StorageKey::NextNonces),
            intents: LookupMap::new(StorageKey::Intents),
        }
    }

    /// Derive the caller's foreign account `key_index` and ask the signer to confirm it.
    /// Calling again re-asks while the account is still pending.
    #[payable]
    pub fn register_foreign_account(&mut self, key_index: u32) -> Promise {
        let initial = env::storage_usage();
        let owner = env::predecessor_account_id();
        if let Some(existing) = self.accounts.get(&account_key(&owner, key_index)) {
            require!(
                existing.status == AccountStatus::PendingSignerCheck,
                format!("foreign account {key_index} is already {:?}", existing.status)
            );
        }
        let (path, key) = self.derive_account(&owner, key_index);
        let account = ForeignAccount {
            owner: owner.clone(),
            key_index,
            derivation_path: path.clone(),
            derived_public_key: format_near_secp256k1_key(&key),
            address: hex::format_evm_address(&evm_address(&key)),
            status: AccountStatus::PendingSignerCheck,
            in_flight: None,
        };
        self.store_account(&account);
        emit(
            "foreign_account_derived",
            json!({"owner": owner, "key_index": key_index, "path": path, "address": account.address}),
        );
        settle_storage_deposit(initial);
        ext_mpc_signer::ext(self.mpc_signer.clone())
            .with_static_gas(DERIVED_KEY_GAS)
            .derived_public_key(path, Some(env::current_account_id()), Some(self.mpc_domain_id))
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(DERIVED_KEY_CALLBACK_GAS)
                    .on_foreign_account_checked(owner, key_index),
            )
    }

    /// Compare the signer's derivation with ours. Only an exact match enables signing.
    #[private]
    pub fn on_foreign_account_checked(&mut self, owner: AccountId, key_index: u32) -> AccountStatus {
        require_self_callback();
        let mut account = self.load_account(&owner, key_index);
        require!(
            account.status == AccountStatus::PendingSignerCheck && env::promise_results_count() == 1,
            "no pending signer check"
        );
        let answer = env::promise_result_checked(0, MAX_SIGNER_RESULT)
            .ok()
            .map(|bytes| serde_json::from_slice::<String>(&bytes).ok());
        account.status = match answer {
            None => AccountStatus::PendingSignerCheck,
            Some(Some(key)) if key == account.derived_public_key => AccountStatus::Verified,
            Some(_) => AccountStatus::Mismatch,
        };
        self.store_account(&account);
        emit(
            "foreign_account_checked",
            json!({"owner": owner, "key_index": key_index, "status": account.status, "address": account.address}),
        );
        account.status
    }

    /// Create or replace the caller's signing mandate for `agent`. The attached deposit pays
    /// the mandate's storage; the rest is credit for the agent's storage and sign deposits.
    #[payable]
    pub fn create_mandate(&mut self, agent: AccountId, terms: SigningMandateTerms) -> SigningMandate {
        let initial = env::storage_usage();
        let owner = env::predecessor_account_id();
        let now = env::block_timestamp_ms();
        validate_terms(&owner, &agent, &terms, now).unwrap_or_else(|e| env::panic_str(&e));
        require!(
            self.accounts.contains_key(&account_key(&owner, terms.key_index)),
            "register the foreign account before granting a mandate over it"
        );
        let previous = self.mandates.get(&mandate_key(&owner, &agent)).cloned();
        let (version, spends, credit) = match previous {
            Some(p) => (
                p.version
                    .checked_add(1)
                    .unwrap_or_else(|| env::panic_str("mandate version overflow")),
                p.spends,
                p.storage_credit.0,
            ),
            None => (1, Vec::new(), 0),
        };
        let mut m = SigningMandate {
            owner: owner.clone(),
            agent: agent.clone(),
            terms,
            version,
            revoked: false,
            created_at_ms: now,
            spends,
            storage_credit: U128(credit),
        };
        self.store_mandate(&m);
        let cost = storage_cost_since(initial);
        let deposit = env::attached_deposit().as_yoctonear();
        let added = deposit
            .checked_sub(cost)
            .unwrap_or_else(|| env::panic_str(&format!("attach at least {cost} yoctoNEAR for storage")));
        m.storage_credit = U128(
            credit
                .checked_add(added)
                .filter(|c| *c <= MAX_STORAGE_CREDIT)
                .unwrap_or_else(|| env::panic_str("prepaid credit must not exceed 1 NEAR")),
        );
        self.store_mandate(&m);
        emit(
            "mandate_created",
            json!({
                "owner": owner, "agent": agent, "version": version,
                "program_sha256": m.terms.program_sha256, "chain_id": m.terms.chain_id,
                "key_index": m.terms.key_index, "expires_at_ms": m.terms.expires_at_ms,
            }),
        );
        m
    }

    /// Stop the agent at once: no proposal, approval or signature request succeeds after this.
    pub fn revoke_mandate(&mut self, agent: AccountId) -> SigningMandate {
        let owner = env::predecessor_account_id();
        let mut m = self.load_mandate(&owner, &agent);
        require!(!m.revoked, "the mandate is already revoked");
        m.revoked = true;
        self.store_mandate(&m);
        emit(
            "mandate_revoked",
            json!({"owner": owner, "agent": agent, "version": m.version}),
        );
        m
    }

    /// Return unspent credit once the mandate is revoked or expired and nothing is in flight.
    pub fn withdraw_credit(&mut self, agent: AccountId) -> Promise {
        let owner = env::predecessor_account_id();
        let mut m = self.load_mandate(&owner, &agent);
        require!(
            m.revoked || env::block_timestamp_ms() >= m.terms.expires_at_ms,
            "revoke the mandate or wait for it to expire first"
        );
        let account = self.load_account(&owner, m.terms.key_index);
        require!(account.in_flight.is_none(), "a signature request is in flight");
        let amount = m.storage_credit.0;
        require!(amount > 0, "no unspent credit");
        m.storage_credit = U128(0);
        self.store_mandate(&m);
        Promise::new(owner).transfer(NearToken::from_yoctonear(amount))
    }

    /// The agent proposes a payment. The contract checks every term, builds the exact unsigned
    /// transaction and stores the intent. Nothing is signed here.
    pub fn agent_propose_signing(&mut self, owner: AccountId, request: SigningRequest) -> SigningRecord {
        let initial = env::storage_usage();
        let agent = env::predecessor_account_id();
        let now = env::block_timestamp_ms();
        let mut m = self.load_mandate(&owner, &agent);
        let account = self.load_account(&owner, request.key_index);
        let mut results = check_signing_authority(&m, &request, now, false);
        results.push(GuardResult {
            guard: "foreign_account".into(),
            ok: account.status == AccountStatus::Verified,
            detail: format!("{} is {:?}", account.address, account.status),
        });
        results.push(GuardResult {
            guard: "intent_nonce_unused".into(),
            ok: valid_nonce(&request.intent_nonce)
                && !self.intents.contains_key(&intent_key(&owner, &request.intent_nonce)),
            detail: request.intent_nonce.clone(),
        });
        let next = self.next_nonce(&owner, request.key_index, request.chain_id);
        results.push(GuardResult {
            guard: "evm_nonce".into(),
            ok: request.evm_nonce >= next && request.evm_nonce - next < MAX_NONCE_LOOKAHEAD,
            detail: format!("{}, next unsigned {next}", request.evm_nonce),
        });
        refuse(&results, "proposal");

        let current = env::current_account_id();
        let ctx = SigningContext {
            mpc_signer: &self.mpc_signer,
            mpc_domain_id: self.mpc_domain_id,
            predecessor: &current,
            derivation_path: &account.derivation_path,
            from: &account.address,
        };
        let (intent, _) = build_intent(&m, &request, &ctx).unwrap_or_else(|e| env::panic_str(&e));
        let rec = SigningRecord {
            intent_hash: intent_hash(&intent),
            intent,
            approvals: Vec::new(),
            status: SigningStatus::Proposed,
            attempts: 0,
            requested_at_ms: None,
            spend_counted: false,
            signature: None,
            signed_tx: None,
            tx_hash: None,
            last_failure: None,
        };
        self.store_record(&rec);
        self.charge_credit(&mut m, initial, 0);
        emit(
            "signing_proposed",
            json!({
                "owner": owner, "agent": agent, "intent_nonce": rec.intent.intent_nonce,
                "intent_hash": rec.intent_hash, "signing_hash": rec.intent.signing_hash,
                "chain_id": rec.intent.chain_id, "from": rec.intent.from,
                "recipient": rec.intent.recipient, "amount": rec.intent.amount,
            }),
        );
        rec
    }

    /// A listed approver signs off on the exact intent they reviewed, named by its hash.
    pub fn approve_signing(&mut self, owner: AccountId, intent_nonce: String, intent_hash: String) -> SigningRecord {
        let initial = env::storage_usage();
        let approver = env::predecessor_account_id();
        let now = env::block_timestamp_ms();
        let mut rec = self.load_record(&owner, &intent_nonce);
        let mut m = self.load_mandate(&owner, &rec.intent.agent);
        require!(
            rec.intent.mandate_version == m.version,
            "X3_MPC_DENIED: the intent's mandate version is no longer current"
        );
        require!(!m.revoked, "X3_MPC_DENIED: the mandate is revoked");
        require!(now < m.terms.expires_at_ms, "X3_MPC_DENIED: the mandate is expired");
        require!(now < rec.intent.expires_at_ms, "X3_MPC_DENIED: the intent is expired");
        require!(
            rec.status == SigningStatus::Proposed,
            "X3_MPC_DENIED: approvals are taken only before the first signature request"
        );
        require!(
            m.approvals_needed(rec.intent.amount.0) > 0,
            "this payment is within approval_above and needs no approval"
        );
        require!(
            m.terms.approvers.contains(&approver),
            format!("{approver} is not an approver on this mandate")
        );
        require!(
            !rec.approvals.contains(&approver),
            format!("{approver} has already approved")
        );
        require!(
            intent_hash == rec.intent_hash,
            "X3_MPC_DENIED: the approved hash is not this intent's hash"
        );
        rec.approvals.push(approver.clone());
        self.store_record(&rec);
        self.charge_credit(&mut m, initial, 0);
        emit(
            "signing_approved",
            json!({"owner": owner, "intent_nonce": intent_nonce, "approver": approver, "approvals": rec.approvals.len()}),
        );
        rec
    }

    /// The agent asks for the signature. Every term is checked again here; a refusal panics
    /// before any promise to the signer exists. On success the spend is counted, the EVM nonce
    /// is consumed and `sign` is called with the stored hash, the derived path and the pinned
    /// domain. A retry after a failed attempt re-signs the identical payload.
    pub fn agent_request_signature(&mut self, owner: AccountId, intent_nonce: String) -> Promise {
        let initial = env::storage_usage();
        let caller = env::predecessor_account_id();
        let now = env::block_timestamp_ms();
        let mut rec = self.load_record(&owner, &intent_nonce);
        require!(
            caller == rec.intent.agent,
            format!("X3_MPC_DENIED: only {} can request this signature", rec.intent.agent)
        );
        let mut m = self.load_mandate(&owner, &caller);
        let mut results = self.request_checks(&rec, &m, now);
        results.push(GuardResult {
            guard: "prepaid_gas".into(),
            ok: env::prepaid_gas() >= MIN_REQUEST_GAS,
            detail: format!("{} of at least {}", env::prepaid_gas(), MIN_REQUEST_GAS),
        });
        refuse(&results, "signature request");

        let mut account = self.load_account(&owner, rec.intent.key_index);
        if !rec.spend_counted {
            m.spends.retain(|s| s.at_ms > now.saturating_sub(WINDOW_MS));
            m.spends.push(Spend {
                at_ms: now,
                amount: rec.intent.amount,
                intent_nonce: intent_nonce.clone(),
            });
            self.next_nonces.insert(
                nonce_key(&owner, rec.intent.key_index, rec.intent.chain_id),
                rec.intent.evm_nonce + 1,
            );
            self.next_nonces.flush();
            rec.spend_counted = true;
        }
        rec.status = SigningStatus::SignRequested;
        rec.attempts += 1;
        rec.requested_at_ms = Some(now);
        rec.last_failure = None;
        account.in_flight = Some(intent_nonce.clone());
        self.store_record(&rec);
        self.store_account(&account);
        self.store_mandate(&m);
        self.charge_credit(&mut m, initial, SIGN_DEPOSIT_YOCTO);
        emit(
            "signature_requested",
            json!({
                "owner": owner, "intent_nonce": intent_nonce, "attempt": rec.attempts,
                "signing_hash": rec.intent.signing_hash, "path": rec.intent.derivation_path,
                "domain_id": self.mpc_domain_id, "signer": self.mpc_signer,
            }),
        );
        ext_mpc_signer::ext(self.mpc_signer.clone())
            .with_attached_deposit(NearToken::from_yoctonear(SIGN_DEPOSIT_YOCTO))
            .with_static_gas(SIGN_GAS)
            .sign(SignRequestArgs {
                path: rec.intent.derivation_path.clone(),
                payload_v2: SignPayload::Ecdsa(rec.intent.signing_hash.clone()),
                domain_id: self.mpc_domain_id,
            })
            .then(
                Self::ext(env::current_account_id())
                    .with_static_gas(SIGN_CALLBACK_GAS)
                    .on_signature(owner, intent_nonce, rec.attempts),
            )
    }

    /// The signer's answer. Accepts only a signature that recovers to the derived key; anything
    /// else (timeout, failure, wrong key) leaves the intent retryable. A callback for an
    /// earlier attempt changes nothing.
    #[private]
    pub fn on_signature(&mut self, owner: AccountId, intent_nonce: String, attempt: u32) -> SigningStatus {
        require_self_callback();
        let mut rec = self.load_record(&owner, &intent_nonce);
        if rec.status != SigningStatus::SignRequested || rec.attempts != attempt || env::promise_results_count() != 1 {
            emit(
                "stale_signature_callback",
                json!({"owner": owner, "intent_nonce": intent_nonce, "attempt": attempt}),
            );
            return rec.status;
        }
        let mut account = self.load_account(&owner, rec.intent.key_index);
        if account.in_flight.as_deref() == Some(intent_nonce.as_str()) {
            account.in_flight = None;
        }
        let derived = parse_near_secp256k1_key(&account.derived_public_key).unwrap_or_else(|e| env::panic_str(e));
        let hash: [u8; 32] =
            hex::decode_array(&rec.intent.signing_hash).unwrap_or_else(|| env::panic_str("stored hash is corrupt"));
        let outcome = env::promise_result_checked(0, MAX_SIGNER_RESULT)
            .map_err(|_| "the signer returned no signature (failure or yield timeout)".to_string())
            .and_then(|bytes| verify_signature_response(&bytes, &hash, &derived));
        match outcome {
            Ok(sig) => {
                let tx = build_transaction(&request_of(&rec.intent)).unwrap_or_else(|e| env::panic_str(&e));
                let signed = tx.signed_bytes(&sig);
                rec.tx_hash = Some(hex::encode(&derivation::keccak256(&signed)));
                rec.signed_tx = Some(hex::encode(&signed));
                rec.signature = Some(StoredSignature {
                    y_parity: sig.y_parity,
                    r: hex::encode(&sig.r),
                    s: hex::encode(&sig.s),
                });
                rec.status = SigningStatus::Signed;
            }
            Err(reason) => {
                rec.status = SigningStatus::SignFailed;
                rec.last_failure = Some(reason);
            }
        }
        self.store_record(&rec);
        self.store_account(&account);
        emit(
            "signature_result",
            json!({
                "owner": owner, "intent_nonce": intent_nonce, "status": rec.status,
                "tx_hash": rec.tx_hash, "failure": rec.last_failure,
            }),
        );
        rec.status
    }

    /// If a request's callback never arrived (it can run out of gas), the owner or agent may
    /// mark it failed after [`STALE_REQUEST_MS`], freeing the account for a retry.
    pub fn mark_stale_request(&mut self, owner: AccountId, intent_nonce: String) -> SigningRecord {
        let caller = env::predecessor_account_id();
        let mut rec = self.load_record(&owner, &intent_nonce);
        require!(
            caller == owner || caller == rec.intent.agent,
            "only the owner or the agent"
        );
        require!(rec.status == SigningStatus::SignRequested, "no request in flight");
        let requested = rec.requested_at_ms.unwrap_or(0);
        require!(
            env::block_timestamp_ms() >= requested.saturating_add(STALE_REQUEST_MS),
            "the request is not stale yet"
        );
        let mut account = self.load_account(&owner, rec.intent.key_index);
        if account.in_flight.as_deref() == Some(intent_nonce.as_str()) {
            account.in_flight = None;
        }
        rec.status = SigningStatus::SignFailed;
        rec.last_failure = Some("no callback within the stale window".into());
        self.store_record(&rec);
        self.store_account(&account);
        rec
    }

    pub fn get_config(&self) -> Config {
        Config {
            mpc_signer: self.mpc_signer.clone(),
            mpc_root_public_key: format_near_secp256k1_key(&self.mpc_root_public_key),
            mpc_domain_id: self.mpc_domain_id,
            path_domain: PATH_DOMAIN.into(),
            intent_version: authority::SIGNING_INTENT_VERSION,
        }
    }

    /// What `register_foreign_account` would derive, without storing anything.
    pub fn preview_foreign_account(&self, owner: AccountId, key_index: u32) -> ForeignAccount {
        let (path, key) = self.derive_account(&owner, key_index);
        ForeignAccount {
            owner,
            key_index,
            derivation_path: path,
            derived_public_key: format_near_secp256k1_key(&key),
            address: hex::format_evm_address(&evm_address(&key)),
            status: AccountStatus::PendingSignerCheck,
            in_flight: None,
        }
    }

    pub fn get_foreign_account(&self, owner: AccountId, key_index: u32) -> Option<ForeignAccount> {
        self.accounts.get(&account_key(&owner, key_index)).cloned()
    }

    pub fn get_mandate(&self, owner: AccountId, agent: AccountId) -> Option<SigningMandate> {
        self.mandates.get(&mandate_key(&owner, &agent)).cloned()
    }

    pub fn get_signing_record(&self, owner: AccountId, intent_nonce: String) -> Option<SigningRecord> {
        self.intents.get(&intent_key(&owner, &intent_nonce)).cloned()
    }

    pub fn get_next_evm_nonce(&self, owner: AccountId, key_index: u32, chain_id: u64) -> u64 {
        self.next_nonce(&owner, key_index, chain_id)
    }

    /// Dry run of a proposal's mandate checks. Grants nothing; can go stale.
    pub fn check_signing_request(
        &self,
        owner: AccountId,
        agent: AccountId,
        request: SigningRequest,
    ) -> Vec<GuardResult> {
        match self.mandates.get(&mandate_key(&owner, &agent)) {
            None => vec![GuardResult {
                guard: "mandate_active".into(),
                ok: false,
                detail: format!("{owner} has no signing mandate for {agent}"),
            }],
            Some(m) => check_signing_authority(m, &request, env::block_timestamp_ms(), false),
        }
    }
}

#[cfg(test)]
mod tests;
