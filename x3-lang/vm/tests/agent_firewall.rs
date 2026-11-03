//! Cryptographic owner-approved payments: no AI prompt is allowed to authorize funds.
use ed25519_dalek::{Signer, SigningKey};
use std::collections::BTreeSet;
use x3_lang_vm::agent_firewall::{
    approval_digest, AgentFirewall, AgentPolicy, FirewallError, PaymentIntent, SignedPaymentIntent,
};

fn signer(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn policy(owner: &SigningKey) -> AgentPolicy {
    AgentPolicy {
        chain: "near".into(),
        asset: "usdc.near".into(),
        allowed_recipients: BTreeSet::from(["vendor.near".into()]),
        allowed_contracts: BTreeSet::from(["usdc.near".into()]),
        allowed_methods: BTreeSet::from(["ft_transfer".into()]),
        allowed_owner_keys: BTreeSet::from([owner.verifying_key().to_bytes()]),
        max_per_payment: 100,
        max_per_day: 150,
    }
}

fn intent(nonce: u64, amount: u128) -> PaymentIntent {
    PaymentIntent {
        chain: "near".into(),
        asset: "usdc.near".into(),
        recipient: "vendor.near".into(),
        contract: "usdc.near".into(),
        method: "ft_transfer".into(),
        amount,
        nonce,
        day: 20,
        expiry_block: 1000,
    }
}

fn signed(owner: &SigningKey, value: PaymentIntent) -> SignedPaymentIntent {
    let digest = approval_digest(&value);
    SignedPaymentIntent {
        intent: value,
        public_key: owner.verifying_key().to_bytes(),
        signature: owner.sign(&digest).to_bytes(),
    }
}

#[test]
fn human_signed_payment_records_limit_and_never_authorizes_a_second_nonce() {
    let owner = signer(11);
    let mut firewall = AgentFirewall::new(policy(&owner)).unwrap();
    let order = signed(&owner, intent(1, 75));
    let permit = firewall.record_approved_request(&order, 20, 600).unwrap();
    assert_eq!(permit.amount, 75);
    assert_eq!(permit.authorization, "local_preflight_only");
    assert_eq!(firewall.spent_on_day(20), 75);
    assert_eq!(
        firewall.record_approved_request(&order, 20, 600),
        Err(FirewallError::Replay)
    );
    assert_eq!(firewall.spent_on_day(20), 75);
}

#[test]
fn tampering_after_human_signature_fails_before_spending_counter_changes() {
    let owner = signer(11);
    let mut firewall = AgentFirewall::new(policy(&owner)).unwrap();
    let mut order = signed(&owner, intent(2, 75));
    order.intent.recipient = "attacker.near".into();
    assert_eq!(
        firewall.record_approved_request(&order, 20, 600),
        Err(FirewallError::BadSignature)
    );
    assert_eq!(firewall.spent_on_day(20), 0);
}

#[test]
fn unknown_signer_and_unapproved_contract_method_or_asset_are_denied() {
    let owner = signer(11);
    let stranger = signer(12);
    let mut firewall = AgentFirewall::new(policy(&owner)).unwrap();
    assert_eq!(
        firewall.record_approved_request(&signed(&stranger, intent(1, 50)), 20, 600),
        Err(FirewallError::UnknownSigner)
    );
    let mut changed = intent(2, 50);
    changed.method = "storage_withdraw".into();
    assert_eq!(
        firewall.record_approved_request(&signed(&owner, changed), 20, 600),
        Err(FirewallError::UnapprovedMethod)
    );
    let mut changed = intent(3, 50);
    changed.contract = "evil.near".into();
    assert_eq!(
        firewall.record_approved_request(&signed(&owner, changed), 20, 600),
        Err(FirewallError::UnapprovedContract)
    );
    let mut changed = intent(4, 50);
    changed.asset = "near".into();
    assert_eq!(
        firewall.record_approved_request(&signed(&owner, changed), 20, 600),
        Err(FirewallError::UnapprovedAsset)
    );
    assert_eq!(firewall.spent_on_day(20), 0);
}

#[test]
fn per_payment_and_daily_amount_limits_fail_closed() {
    let owner = signer(11);
    let mut firewall = AgentFirewall::new(policy(&owner)).unwrap();
    assert_eq!(
        firewall.record_approved_request(&signed(&owner, intent(1, 101)), 20, 600),
        Err(FirewallError::PaymentLimitExceeded)
    );
    firewall
        .record_approved_request(&signed(&owner, intent(2, 90)), 20, 600)
        .unwrap();
    assert_eq!(
        firewall.record_approved_request(&signed(&owner, intent(3, 70)), 20, 600),
        Err(FirewallError::DailyLimitExceeded)
    );
    assert_eq!(firewall.spent_on_day(20), 90);
    firewall
        .record_approved_request(&signed(&owner, intent(4, 60)), 20, 600)
        .unwrap();
    assert_eq!(firewall.spent_on_day(20), 150);
}

#[test]
fn stale_approval_wrong_day_and_invalid_policy_are_denied() {
    let owner = signer(11);
    let mut firewall = AgentFirewall::new(policy(&owner)).unwrap();
    assert_eq!(
        firewall.record_approved_request(&signed(&owner, intent(1, 10)), 21, 600),
        Err(FirewallError::WrongDay)
    );
    assert_eq!(
        firewall.record_approved_request(&signed(&owner, intent(2, 10)), 20, 1001),
        Err(FirewallError::Expired)
    );
    let mut policy = policy(&owner);
    policy.max_per_payment = 200;
    assert!(matches!(AgentFirewall::new(policy), Err(FirewallError::InvalidPolicy)));
}
