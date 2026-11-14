//! Unit tests. `vectors` checks the cryptography against published vectors; `flow` and
//! `adversarial` drive the contract with near-sdk's mocked blockchain and a local stand-in for
//! the MPC network (a known root secret), so no network, key file or real signer is involved.

mod adversarial;
mod flow;
mod vectors;

use std::panic::{catch_unwind, AssertUnwindSafe};

use k256::ecdsa::SigningKey;
use k256::elliptic_curve::sec1::ToEncodedPoint;
use k256::elliptic_curve::PrimeField;
use k256::{ProjectivePoint, Scalar};
use near_sdk::json_types::U128;
use near_sdk::mock::MockAction;
use near_sdk::test_utils::{get_created_receipts, VMContextBuilder};
use near_sdk::{testing_env, AccountId, Gas, NearToken, PromiseResult};

use crate::authority::{EvmAsset, SigningMandateTerms, SigningRequest};
use crate::derivation::{derive_tweak, format_near_secp256k1_key};
use crate::*;

pub const GUARD: &str = "x3-mpc-guard.testnet";
pub const SIGNER: &str = "v1.signer-prod.testnet";
pub const OWNER: &str = "owner.testnet";
pub const AGENT: &str = "agent.testnet";
pub const ALICE: &str = "alice.testnet";
pub const BOB: &str = "bob.testnet";
pub const MALLORY: &str = "mallory.testnet";
pub const PROGRAM: &str = "1111111111111111111111111111111111111111111111111111111111111111";
pub const RECIPIENT: &str = "0x2222222222222222222222222222222222222222";
pub const RECIPIENT_2: &str = "0x3333333333333333333333333333333333333333";
pub const ATTACKER: &str = "0xbadbadbadbadbadbadbadbadbadbadbadbadbad0";
pub const SEPOLIA: u64 = 11_155_111;
pub const T0_MS: u64 = 1_800_000_000_000;
pub const NEAR: u128 = 1_000_000_000_000_000_000_000_000;
pub const GWEI: u128 = 1_000_000_000;

pub fn acc(name: &str) -> AccountId {
    name.parse().unwrap()
}

/// The test MPC network's root secret. Stands in for the threshold key; never a real key.
pub fn root_secret() -> Scalar {
    Scalar::from_repr([7u8; 32].into()).unwrap()
}

pub fn uncompressed_of(point: ProjectivePoint) -> [u8; 64] {
    let encoded = point.to_affine().to_encoded_point(false);
    encoded.as_bytes()[1..].try_into().unwrap()
}

pub fn root_public_key() -> String {
    format_near_secp256k1_key(&uncompressed_of(ProjectivePoint::GENERATOR * root_secret()))
}

/// The derived secret the MPC network would use for `(GUARD, path)`: root + tweak, as
/// `derive_secret_key` in near/mpc `crates/contract/src/api/sign.rs` tests.
pub fn derived_secret(predecessor: &str, path: &str) -> Scalar {
    root_secret() + Scalar::from_repr(derive_tweak(predecessor, path).into()).unwrap()
}

/// What the signer's `derived_public_key` view returns, computed from the secret side.
pub fn signer_derived_key(predecessor: &str, path: &str) -> String {
    format_near_secp256k1_key(&uncompressed_of(
        ProjectivePoint::GENERATOR * derived_secret(predecessor, path),
    ))
}

/// A `SignatureResponse::Secp256k1` JSON body for `hash` under `secret`.
pub fn mpc_response(secret: &Scalar, hash: &[u8; 32]) -> Vec<u8> {
    let key = SigningKey::from_bytes(&secret.to_bytes()).unwrap();
    let (sig, recid) = key.sign_prehash_recoverable(hash).unwrap();
    let r: [u8; 32] = sig.r().to_bytes().into();
    let s: [u8; 32] = sig.s().to_bytes().into();
    let mut big_r = vec![0x02 | (recid.to_byte() & 1)];
    big_r.extend_from_slice(&r);
    near_sdk::serde_json::to_vec(&near_sdk::serde_json::json!({
        "scheme": "Secp256k1",
        "big_r": {"affine_point": hex::encode(&big_r)},
        "s": {"scalar": hex::encode(&s)},
        "recovery_id": recid.to_byte(),
    }))
    .unwrap()
}

pub fn ctx(predecessor: &str, deposit: u128, now_ms: u64) -> near_sdk::VMContext {
    VMContextBuilder::new()
        .current_account_id(acc(GUARD))
        .predecessor_account_id(acc(predecessor))
        .signer_account_id(acc(predecessor))
        .attached_deposit(NearToken::from_yoctonear(deposit))
        .block_timestamp(now_ms * 1_000_000)
        .prepaid_gas(Gas::from_tgas(300))
        .account_balance(NearToken::from_near(100))
        .build()
}

/// Drop all mocked storage so a test can build a second, independent contract.
pub fn fresh_chain() {
    let _ = near_sdk::mock::with_mocked_blockchain(|b| b.take_storage());
}

pub fn set(predecessor: &str, deposit: u128, now_ms: u64) {
    testing_env!(ctx(predecessor, deposit, now_ms));
}

pub fn callback(now_ms: u64, result: PromiseResult) {
    testing_env!(
        ctx(GUARD, 0, now_ms),
        near_sdk::test_vm_config(),
        near_sdk::RuntimeFeesConfig::test(),
        Default::default(),
        vec![result],
    );
}

pub fn terms() -> SigningMandateTerms {
    SigningMandateTerms {
        program_sha256: PROGRAM.into(),
        chain_id: SEPOLIA,
        key_index: 0,
        asset: EvmAsset::Native,
        recipients: vec![RECIPIENT.into(), RECIPIENT_2.into()],
        per_payment_cap: U128(1_000_000),
        daily_cap: U128(2_500_000),
        max_fee_per_gas: U128(100 * GWEI),
        max_priority_fee_per_gas: U128(2 * GWEI),
        max_gas_limit: 100_000,
        max_total_fee: U128(5_000_000 * GWEI),
        approval_above: Some(U128(500_000)),
        approvers: vec![acc(ALICE), acc(BOB)],
        approval_threshold: 2,
        expires_at_ms: T0_MS + 7 * 86_400_000,
        max_intent_ttl_ms: 3_600_000,
    }
}

pub fn request(intent_nonce: &str, amount: u128, evm_nonce: u64) -> SigningRequest {
    SigningRequest {
        intent_nonce: intent_nonce.into(),
        program_sha256: PROGRAM.into(),
        chain_id: SEPOLIA,
        key_index: 0,
        asset: EvmAsset::Native,
        recipient: RECIPIENT.into(),
        amount: U128(amount),
        evm_nonce,
        max_priority_fee_per_gas: U128(GWEI),
        max_fee_per_gas: U128(30 * GWEI),
        gas_limit: 21_000,
        expires_at_ms: T0_MS + 1_800_000,
    }
}

/// Initialized contract with a verified foreign account 0 and a mandate for the agent.
pub fn world_with(t: SigningMandateTerms) -> Contract {
    fresh_chain();
    set(OWNER, 0, T0_MS);
    let mut c = Contract::new(acc(SIGNER), root_public_key(), 0);
    set(OWNER, NEAR / 10, T0_MS);
    let _ = c.register_foreign_account(0);
    let path = derivation_path(&acc(OWNER), 0);
    let answer = near_sdk::serde_json::to_vec(&signer_derived_key(GUARD, &path)).unwrap();
    callback(T0_MS, PromiseResult::Successful(answer));
    assert_eq!(c.on_foreign_account_checked(acc(OWNER), 0), AccountStatus::Verified);
    set(OWNER, NEAR / 2, T0_MS);
    c.create_mandate(acc(AGENT), t);
    c
}

pub fn world() -> Contract {
    world_with(terms())
}

pub fn propose(c: &mut Contract, req: SigningRequest, now_ms: u64) -> SigningRecord {
    set(AGENT, 0, now_ms);
    c.agent_propose_signing(acc(OWNER), req)
}

/// Request a signature as the agent and return the `sign` call it created.
pub fn request_signature(c: &mut Contract, nonce: &str, now_ms: u64) -> near_sdk::serde_json::Value {
    set(AGENT, 0, now_ms);
    let _ = c.agent_request_signature(acc(OWNER), nonce.into());
    sign_calls()
        .pop()
        .expect("an authorized request creates exactly one sign call")
}

/// Every `sign` function call to the MPC signer created in the current mocked transaction,
/// with its deposit and gas checked against the pinned interface.
pub fn sign_calls() -> Vec<near_sdk::serde_json::Value> {
    let mut out = Vec::new();
    for receipt in get_created_receipts() {
        for action in &receipt.actions {
            if let MockAction::FunctionCallWeight {
                method_name,
                args,
                attached_deposit,
                prepaid_gas,
                ..
            } = action
            {
                if method_name == b"sign" {
                    assert_eq!(receipt.receiver_id.as_str(), SIGNER);
                    assert_eq!(attached_deposit.as_yoctonear(), SIGN_DEPOSIT_YOCTO);
                    assert_eq!(*prepaid_gas, SIGN_GAS);
                    out.push(near_sdk::serde_json::from_slice(args).unwrap());
                }
            }
        }
    }
    out
}

/// No promise of any kind, in particular none to the signer, exists after a refusal.
pub fn assert_no_promise() {
    let receipts = get_created_receipts();
    assert!(
        receipts.iter().all(|r| r.receiver_id.as_str() != SIGNER),
        "a refused call reached the signer: {receipts:?}"
    );
    assert!(receipts.is_empty(), "a refused call created a promise: {receipts:?}");
}

/// Run a call that must be refused; return its panic message after checking no promise exists.
pub fn refused<F: FnOnce()>(f: F) -> String {
    let outcome = catch_unwind(AssertUnwindSafe(f));
    let payload = outcome.expect_err("the call should have been refused");
    assert_no_promise();
    if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else {
        String::from("<non-string panic>")
    }
}

pub fn sign_and_return(c: &mut Contract, nonce: &str, now_ms: u64) -> SigningStatus {
    let rec = c.get_signing_record(acc(OWNER), nonce.into()).unwrap();
    let hash: [u8; 32] = hex::decode_array(&rec.intent.signing_hash).unwrap();
    let secret = derived_secret(GUARD, &rec.intent.derivation_path);
    callback(now_ms, PromiseResult::Successful(mpc_response(&secret, &hash)));
    c.on_signature(acc(OWNER), nonce.into(), rec.attempts)
}

pub fn approve(c: &mut Contract, approver: &str, nonce: &str, now_ms: u64) -> SigningRecord {
    let hash = c.get_signing_record(acc(OWNER), nonce.into()).unwrap().intent_hash;
    set(approver, 0, now_ms);
    c.approve_signing(acc(OWNER), nonce.into(), hash)
}
