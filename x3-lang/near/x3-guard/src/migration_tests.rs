//! Upgrade tests for persisted Guard state written before schema versioning.

use super::*;
use crate::mandate::Mandate;
use near_sdk::test_utils::{accounts, VMContextBuilder};
use near_sdk::testing_env;

const NOW_MS: u64 = 1_793_679_484_780;
const DEADLINE_MS: u64 = 1_793_680_084_780;
const PROGRAM: &str = "0b5ef1d2f6c4b0a8e8d1c1f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2a1b0";
const SOL_USDC: &str = "nep141:sol-5ce3bf3a31af18be40ba30f721101b4341690186.omft.near";
const NEAR_USDC: &str = "nep141:17208628f84f5d6ad33f0da3bbbeb27ffcb398eac501a31bd6ad2011e36133a1";
const RECIPIENT: &str = "f464e8aa90661507c667b20b39bdc9e97ccfb06822030f2ecbf3741af9d85f02";
const REFUND_TO: &str = "4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD";
const TEST_MANAGER_KEY: [u8; 32] = [7; 32];

fn ctx() {
    let builder = VMContextBuilder::new()
        .current_account_id(accounts(0))
        .predecessor_account_id(accounts(0))
        .block_timestamp(NOW_MS * 1_000_000)
        .build();
    testing_env!(builder);
}

fn policy() -> Policy {
    Policy {
        program_sha256: PROGRAM.into(),
        origin_asset: SOL_USDC.into(),
        destination_asset: NEAR_USDC.into(),
        amount: U128(25_000_000),
        min_receive: Some(U128(24_800_000)),
        slippage_bps: 50,
        fee_cap_bps: Some(100),
        recipient: RECIPIENT.into(),
        refund_to: REFUND_TO.into(),
        deadline_ms: DEADLINE_MS,
    }
}

#[near(serializers = [borsh])]
struct IntentRecordV11 {
    owner: AccountId,
    nonce: String,
    policy: Policy,
    registered_at_ms: u64,
    quote: Option<AcceptedQuote>,
    outcome: Option<Outcome>,
}

#[near(serializers = [borsh])]
struct ContractV11 {
    intents: LookupMap<String, IntentRecordV11>,
    manager_public_key: [u8; 32],
}

#[near(serializers = [borsh])]
struct ContractV12 {
    intents: LookupMap<String, IntentRecord>,
    manager_public_key: [u8; 32],
    mandates: LookupMap<String, Mandate>,
}

fn legacy_intent_v11() -> IntentRecordV11 {
    IntentRecordV11 {
        owner: accounts(0),
        nonce: "legacy_v11".into(),
        policy: policy(),
        registered_at_ms: NOW_MS,
        quote: None,
        outcome: None,
    }
}

fn write_v11() -> [u8; 32] {
    let manager_public_key = TEST_MANAGER_KEY;
    let mut intents = LookupMap::new(StorageKey::Intents);
    intents.insert(key(&accounts(0), "legacy_v11"), legacy_intent_v11());
    intents.flush();
    env::state_write(&ContractV11 {
        intents,
        manager_public_key,
    });
    manager_public_key
}

fn write_v12() -> [u8; 32] {
    let manager_public_key = TEST_MANAGER_KEY;
    let mut intents = LookupMap::new(StorageKey::Intents);
    intents.insert(
        key(&accounts(0), "legacy_v12"),
        IntentRecord {
            owner: accounts(0),
            nonce: "legacy_v12".into(),
            policy: policy(),
            registered_at_ms: NOW_MS,
            quote: None,
            outcome: None,
            agent: Some(accounts(1)),
            mandate_version: Some(4),
            approvals: vec![accounts(2)],
        },
    );
    intents.flush();
    let mut mandates = LookupMap::new(StorageKey::Mandates);
    mandates.insert(
        format!("{}/{}", accounts(0), accounts(1)),
        Mandate {
            owner: accounts(0),
            agent: accounts(1),
            terms: mandate_terms(),
            version: 4,
            revoked: false,
            created_at_ms: NOW_MS,
            spends: vec![],
        },
    );
    mandates.flush();
    env::state_write(&ContractV12 {
        intents,
        manager_public_key,
        mandates,
    });
    manager_public_key
}

#[test]
fn v11_record_bytes_decode_with_default_agent_fields() {
    let bytes = near_sdk::borsh::to_vec(&legacy_intent_v11()).unwrap();
    let decoded = IntentRecord::try_from_slice(&bytes).expect("the legacy record trailer defaults");
    assert_eq!(decoded.agent, None);
    assert_eq!(decoded.mandate_version, None);
    assert!(decoded.approvals.is_empty());
}

#[test]
fn truncated_or_inconsistent_new_trailers_fail_closed() {
    let legacy = near_sdk::borsh::to_vec(&legacy_intent_v11()).unwrap();

    let mut missing_agent = legacy.clone();
    missing_agent.push(1);
    assert!(
        IntentRecord::try_from_slice(&missing_agent).is_err(),
        "a partial agent binding must not fall back to the v1.1 layout"
    );

    let mut unbound_with_version = legacy;
    unbound_with_version.extend_from_slice(&[0, 1, 4, 0, 0, 0, 0, 0, 0, 0]);
    assert!(
        IntentRecord::try_from_slice(&unbound_with_version).is_err(),
        "a mandate version without an agent must be refused"
    );
}

fn mandate_terms() -> mandate::MandateTerms {
    mandate::MandateTerms {
        program_sha256: PROGRAM.into(),
        asset: SOL_USDC.into(),
        per_payment_cap: U128(50_000_000),
        daily_cap: U128(100_000_000),
        destination_assets: vec![NEAR_USDC.into()],
        recipients: vec![RECIPIENT.into()],
        refund_to: REFUND_TO.into(),
        max_slippage_bps: 50,
        max_fee_bps: 100,
        approval_above: None,
        approvers: vec![],
        approval_threshold: 0,
        expires_at_ms: DEADLINE_MS,
    }
}

#[test]
fn v11_state_migrates_without_losing_intents_or_the_manager_key() {
    ctx();
    let expected_key = write_v11();
    let contract = Contract::migrate();

    assert_eq!(contract.state_version(), STATE_VERSION);
    assert_eq!(contract.manager_public_key, expected_key);
    let intent = contract
        .get_intent(accounts(0), "legacy_v11".into())
        .expect("the pre-mandate intent survives");
    assert_eq!(intent.owner, accounts(0));
    assert_eq!(intent.agent, None);
    assert_eq!(intent.mandate_version, None);
    assert!(intent.approvals.is_empty());

    env::state_write(&contract);
    let reloaded = env::state_read::<Contract>().expect("migrated state is readable");
    assert_eq!(reloaded.state_version(), STATE_VERSION);
}

#[test]
fn v12_state_keeps_agent_bindings_approvals_and_mandates() {
    ctx();
    let expected_key = write_v12();
    let contract = Contract::migrate();

    assert_eq!(contract.state_version(), STATE_VERSION);
    assert_eq!(contract.manager_public_key, expected_key);
    let intent = contract
        .get_intent(accounts(0), "legacy_v12".into())
        .expect("the mandate-bound intent survives");
    assert_eq!(intent.agent, Some(accounts(1)));
    assert_eq!(intent.mandate_version, Some(4));
    assert_eq!(intent.approvals, vec![accounts(2)]);
    let mandate = contract
        .get_mandate(accounts(0), accounts(1))
        .expect("the mandate survives");
    assert_eq!(mandate.version, 4);
}

#[test]
#[should_panic(expected = "Method is private")]
fn migration_can_only_be_called_by_the_contract_account() {
    ctx();
    write_v11();
    let builder = VMContextBuilder::new()
        .current_account_id(accounts(0))
        .predecessor_account_id(accounts(1))
        .block_timestamp(NOW_MS * 1_000_000)
        .build();
    testing_env!(builder);
    let _ = Contract::migrate();
}

#[test]
#[should_panic(expected = "already uses schema version")]
fn migration_refuses_state_that_is_already_current() {
    ctx();
    let contract = Contract::new();
    env::state_write(&contract);
    let _ = Contract::migrate();
}
