//! Local NEAR Sandbox integration test for the deployed x3-guard WASM.
//!
//! This starts a local `near-sandbox` node, funds sandbox-only accounts,
//! deploys the real contract WASM, and exercises the mandate accept/refuse
//! path against a real NEAR runtime.
//!
//! It is deliberately NOT a public testnet or mainnet test. Every account,
//! transaction hash and receipt below is sandbox-local and must never be
//! presented as public chain evidence.
//!
//! Run from this directory:
//!     cargo test -- --nocapture

use near_account_id::AccountId as SandboxAccountId;
use near_api::{AccountId, NearGas, NearToken};
use near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY;
use serde_json::{json, Value};

const EXPIRES_MS: u64 = 4_000_000_000_000;
const DEADLINE_MS: u64 = 3_900_000_000_000;

fn program_hash() -> String {
    "a".repeat(64)
}

fn terms() -> Value {
    json!({
        "program_sha256": program_hash(),
        "asset": "nep141:usdc.near",
        "per_payment_cap": "50000000",
        "daily_cap": "200000000",
        "destination_assets": ["nep141:usdc.near"],
        "recipients": ["x3-demo.near"],
        "refund_to": "refund.sandbox",
        "max_slippage_bps": 100,
        "max_fee_bps": 100,
        "approval_above": null,
        "approvers": [],
        "approval_threshold": 0,
        "expires_at_ms": EXPIRES_MS,
    })
}

fn policy(recipient: &str) -> Value {
    json!({
        "program_sha256": program_hash(),
        "origin_asset": "nep141:usdc.near",
        "destination_asset": "nep141:usdc.near",
        "amount": "25000000",
        "min_receive": "24800000",
        "slippage_bps": 50,
        "fee_cap_bps": 100,
        "recipient": recipient,
        "refund_to": "refund.sandbox",
        "deadline_ms": DEADLINE_MS,
    })
}

async fn create_account(sandbox: &near_sandbox::Sandbox, name: &str) -> testresult::TestResult<near_api::Account> {
    let sandbox_id: SandboxAccountId = name.parse().unwrap();
    sandbox
        .create_account(sandbox_id)
        .initial_balance(NearToken::from_near(10))
        .send()
        .await?;
    let api_id: AccountId = name.parse().unwrap();
    Ok(near_api::Account(api_id))
}

async fn guard_checks(
    contract: &near_api::Contract,
    network: &near_api::NetworkConfig,
    owner: &AccountId,
    agent: &AccountId,
    policy: Value,
) -> testresult::TestResult<Vec<Value>> {
    let checks: Value = contract
        .call_function(
            "check_agent_intent",
            json!({"owner": owner, "agent": agent, "policy": policy}),
        )
        .read_only()
        .fetch_from(network)
        .await?
        .data;
    Ok(checks
        .as_array()
        .expect("check_agent_intent returns a guard array")
        .clone())
}

fn guard_ok(checks: &[Value], guard: &str) -> bool {
    checks
        .iter()
        .find(|check| check["guard"] == guard)
        .and_then(|check| check["ok"].as_bool())
        .unwrap_or_else(|| panic!("missing guard {guard} in {checks:?}"))
}

#[tokio::test]
async fn guard_mandate_accepts_refuses_and_revokes_on_deployed_wasm() -> testresult::TestResult<()> {
    // The exact guard contract this test deploys. Build it first with:
    //   (cd x3-lang/near/x3-guard && cargo build --locked --release --target wasm32-unknown-unknown)
    // or use scripts/guard-sandbox-test.sh from the repository root.
    let wasm_path = std::env::var("X3_GUARD_WASM")
        .unwrap_or_else(|_| "../target/wasm32-unknown-unknown/release/x3_guard.wasm".to_string());
    let wasm = std::fs::read(&wasm_path).unwrap_or_else(|error| {
        panic!(
            "read guard wasm at {wasm_path}: {error}; build it with `cargo build --locked \
             --release --target wasm32-unknown-unknown` in x3-lang/near/x3-guard"
        )
    });

    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = near_api::NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);

    let owner = create_account(&sandbox, "owner.sandbox").await?;
    let agent = create_account(&sandbox, "agent.sandbox").await?;

    let contract_sandbox_id: SandboxAccountId = "guard.sandbox".parse().unwrap();
    sandbox
        .create_account(contract_sandbox_id)
        .initial_balance(NearToken::from_near(30))
        .send()
        .await?;
    let contract_id: AccountId = "guard.sandbox".parse().unwrap();
    let contract = near_api::Account(contract_id.clone()).as_contract();

    let signer = near_api::Signer::from_secret_key(DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse().unwrap())?;

    let deployment = near_api::Contract::deploy(contract_id.clone())
        .use_code(wasm)
        .with_init_call("new", json!({}))?
        .with_signer(signer.clone())
        .send_to(&network)
        .await?;
    deployment.clone().assert_success();
    println!("deploy (sandbox-only): {deployment:#?}");

    let created = contract
        .call_function("create_mandate", json!({"agent": agent.account_id(), "terms": terms()}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?;
    created.clone().assert_success();
    println!("create_mandate (sandbox-only): {created:#?}");

    let allowed = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": owner.account_id(),
                "nonce": "sandbox-allow-001",
                "policy": policy("x3-demo.near"),
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(agent.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?;
    allowed.clone().assert_success();
    println!("allowed registration (sandbox-only): {allowed:#?}");

    // The read-only view names the exact guard; the transaction below then
    // proves the deployed WASM cannot register the forbidden intent.
    let forbidden_checks = guard_checks(
        &contract,
        &network,
        owner.account_id(),
        agent.account_id(),
        policy("attacker.near"),
    )
    .await?;
    assert!(
        !guard_ok(&forbidden_checks, "mandate_recipient"),
        "the read-only view did not refuse attacker.near: {forbidden_checks:?}"
    );

    let refused = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": owner.account_id(),
                "nonce": "sandbox-refuse-001",
                "policy": policy("attacker.near"),
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?;
    // NEAR's WASM trap does not carry the Rust panic string; the view above
    // names mandate_recipient, and this failed transaction is the on-chain
    // refusal. The allowed registration above is the control.
    assert!(!refused.is_success(), "forbidden recipient was unexpectedly accepted");
    println!("refused registration (sandbox-only, wasm trap): {refused:#?}");

    let revoked = contract
        .call_function("revoke_mandate", json!({"agent": agent.account_id()}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(owner.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?;
    revoked.assert_success();

    let mandate: Value = contract
        .call_function(
            "get_mandate",
            json!({"owner": owner.account_id(), "agent": agent.account_id()}),
        )
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    assert_eq!(mandate["revoked"], Value::Bool(true), "mandate not revoked: {mandate}");

    let revoked_checks = guard_checks(
        &contract,
        &network,
        owner.account_id(),
        agent.account_id(),
        policy("x3-demo.near"),
    )
    .await?;
    assert!(
        !guard_ok(&revoked_checks, "mandate_active"),
        "the read-only view still considers the revoked mandate active: {revoked_checks:?}"
    );

    let after_revoke = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": owner.account_id(),
                "nonce": "sandbox-after-revoke-001",
                "policy": policy("x3-demo.near"),
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.account_id().clone(), signer.clone())
        .send_to(&network)
        .await?;
    assert!(
        !after_revoke.is_success(),
        "registration after revocation was unexpectedly accepted"
    );
    println!("post-revocation refusal (sandbox-only, wasm trap): {after_revoke:#?}");

    Ok(())
}
