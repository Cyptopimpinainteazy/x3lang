//! Positive controls: the authorized path creates exactly the pinned `sign` call, and the
//! returned signature becomes a broadcastable transaction from the verified derived address.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};

use super::*;
use crate::authority::EvmAsset;
use crate::derivation::{evm_address, keccak256};

#[test]
fn foreign_account_is_derived_from_the_guard_and_confirmed_by_the_signer() {
    set(OWNER, 0, T0_MS);
    let mut c = Contract::new(acc(SIGNER), root_public_key(), 0);
    set(OWNER, NEAR / 10, T0_MS);
    let _ = c.register_foreign_account(0);
    // The contract asked the signer for the key of (this contract, the derived path).
    let receipts = get_created_receipts();
    let view = receipts
        .iter()
        .find(|r| r.receiver_id.as_str() == SIGNER)
        .expect("a derived_public_key call to the signer");
    let MockAction::FunctionCallWeight { method_name, args, .. } = &view.actions[0] else {
        panic!("expected a function call")
    };
    assert_eq!(method_name, b"derived_public_key");
    let args: near_sdk::serde_json::Value = near_sdk::serde_json::from_slice(args).unwrap();
    assert_eq!(
        args,
        near_sdk::serde_json::json!({
            "path": "x3-mpc-guard/v1/evm/owner.testnet/0",
            "predecessor": GUARD,
            "domain_id": 0,
        })
    );
    let account = c.get_foreign_account(acc(OWNER), 0).unwrap();
    assert_eq!(account.status, AccountStatus::PendingSignerCheck);
    // Same derived key as the secret the MPC network would sign with.
    assert_eq!(
        account.derived_public_key,
        signer_derived_key(GUARD, &account.derivation_path)
    );

    let answer = near_sdk::serde_json::to_vec(&account.derived_public_key).unwrap();
    callback(T0_MS, PromiseResult::Successful(answer));
    assert_eq!(c.on_foreign_account_checked(acc(OWNER), 0), AccountStatus::Verified);
}

#[test]
fn an_agent_calling_the_signer_itself_derives_an_unrelated_address() {
    let path = derivation_path(&acc(OWNER), 0);
    let root = crate::derivation::parse_near_secp256k1_key(&root_public_key()).unwrap();
    let as_guard = crate::derivation::derive_public_key(&root, &derive_tweak(GUARD, &path)).unwrap();
    let as_agent = crate::derivation::derive_public_key(&root, &derive_tweak(AGENT, &path)).unwrap();
    assert_ne!(evm_address(&as_guard), evm_address(&as_agent));
}

#[test]
fn an_authorized_request_creates_exactly_the_pinned_sign_call() {
    let mut c = world();
    let rec = propose(&mut c, request("pay-1", 400_000, 0), T0_MS);
    assert_eq!(rec.status, SigningStatus::Proposed);
    // Proposal alone never reaches the signer.
    assert!(sign_calls().is_empty());

    let call = request_signature(&mut c, "pay-1", T0_MS + 1);
    assert_eq!(
        call,
        near_sdk::serde_json::json!({
            "request": {
                "path": "x3-mpc-guard/v1/evm/owner.testnet/0",
                "payload_v2": {"Ecdsa": rec.intent.signing_hash},
                "domain_id": 0,
            }
        })
    );
    assert_eq!(
        get_created_receipts()
            .iter()
            .filter(|r| r.receiver_id.as_str() == SIGNER)
            .count(),
        1
    );
    let after = c.get_signing_record(acc(OWNER), "pay-1".into()).unwrap();
    assert_eq!(after.status, SigningStatus::SignRequested);
    assert_eq!(c.get_next_evm_nonce(acc(OWNER), 0, SEPOLIA), 1);
    assert_eq!(c.get_mandate(acc(OWNER), acc(AGENT)).unwrap().spends.len(), 1);
}

#[test]
fn a_returned_signature_becomes_a_transaction_from_the_derived_address() {
    let mut c = world();
    propose(&mut c, request("pay-1", 400_000, 0), T0_MS);
    request_signature(&mut c, "pay-1", T0_MS + 1);
    assert_eq!(sign_and_return(&mut c, "pay-1", T0_MS + 2), SigningStatus::Signed);

    let rec = c.get_signing_record(acc(OWNER), "pay-1".into()).unwrap();
    let account = c.get_foreign_account(acc(OWNER), 0).unwrap();
    assert!(account.in_flight.is_none());
    let signed = hex::decode(rec.signed_tx.as_ref().unwrap()).unwrap();
    assert_eq!(rec.tx_hash.unwrap(), hex::encode(&keccak256(&signed)));
    // Independent recovery from the stored signature over the intent's signing hash.
    let sig = rec.signature.unwrap();
    let signature = Signature::from_scalars(
        hex::decode_array::<32>(&sig.r).unwrap(),
        hex::decode_array::<32>(&sig.s).unwrap(),
    )
    .unwrap();
    let hash: [u8; 32] = hex::decode_array(&rec.intent.signing_hash).unwrap();
    let recovered =
        VerifyingKey::recover_from_prehash(&hash, &signature, RecoveryId::from_byte(sig.y_parity).unwrap()).unwrap();
    let point = recovered.to_encoded_point(false);
    let key: [u8; 64] = point.as_bytes()[1..].try_into().unwrap();
    assert_eq!(hex::format_evm_address(&evm_address(&key)), account.address);
    assert_eq!(rec.intent.from, account.address);
    // The signed bytes are the unsigned fields plus (y_parity, r, s).
    assert_eq!(signed[0], 0x02);
    assert_eq!(
        hex::encode(&hash),
        hex::encode(&keccak256(&hex::decode(&rec.intent.unsigned_tx).unwrap()))
    );
}

#[test]
fn a_payment_above_the_threshold_signs_after_two_distinct_approvals() {
    let mut c = world();
    propose(&mut c, request("big", 800_000, 0), T0_MS);
    approve(&mut c, ALICE, "big", T0_MS + 1);
    approve(&mut c, BOB, "big", T0_MS + 2);
    request_signature(&mut c, "big", T0_MS + 3);
    assert_eq!(sign_and_return(&mut c, "big", T0_MS + 4), SigningStatus::Signed);
}

#[test]
fn an_erc20_intent_signs_a_transfer_call_to_the_token() {
    let token = "0x1c7d4b196cb0c7b01d743fbc6116a902379c7238";
    let mut t = terms();
    t.asset = EvmAsset::Erc20 { token: token.into() };
    let mut c = world_with(t);
    let mut req = request("usdc-1", 250_000, 0);
    req.asset = EvmAsset::Erc20 { token: token.into() };
    req.gas_limit = 65_000;
    let rec = propose(&mut c, req, T0_MS);
    assert_eq!(rec.intent.tx_to, token);
    assert_eq!(rec.intent.tx_value.0, 0);
    assert_eq!(
        rec.intent.tx_data,
        hex::encode(&crate::evm::erc20_transfer_data(
            &hex::parse_evm_address(RECIPIENT).unwrap(),
            250_000
        ))
    );
    request_signature(&mut c, "usdc-1", T0_MS + 1);
    assert_eq!(sign_and_return(&mut c, "usdc-1", T0_MS + 2), SigningStatus::Signed);
}

#[test]
fn consecutive_intents_sign_consecutive_evm_nonces() {
    let mut c = world();
    propose(&mut c, request("a", 100_000, 0), T0_MS);
    propose(&mut c, request("b", 100_000, 1), T0_MS);
    request_signature(&mut c, "a", T0_MS + 1);
    sign_and_return(&mut c, "a", T0_MS + 2);
    request_signature(&mut c, "b", T0_MS + 3);
    assert_eq!(sign_and_return(&mut c, "b", T0_MS + 4), SigningStatus::Signed);
    assert_eq!(c.get_next_evm_nonce(acc(OWNER), 0, SEPOLIA), 2);
}

#[test]
fn owner_can_withdraw_unspent_credit_only_after_revoking() {
    let mut c = world();
    set(OWNER, 0, T0_MS);
    assert!(refused(|| {
        let _ = c.withdraw_credit(acc(AGENT));
    })
    .contains("revoke the mandate"));
    set(OWNER, 0, T0_MS);
    c.revoke_mandate(acc(AGENT));
    set(OWNER, 0, T0_MS);
    let _ = c.withdraw_credit(acc(AGENT));
    assert_eq!(c.get_mandate(acc(OWNER), acc(AGENT)).unwrap().storage_credit.0, 0);
}
