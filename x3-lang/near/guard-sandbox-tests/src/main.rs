//! Exact production x3-guard WASM, deployed to a LOCAL NEAR sandbox node only.
//! Never contacts testnet, imports production wallet keys or moves real funds.
use anyhow::{Context, Result};
use near_account_id::AccountId as SandboxAccountId;
use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

const MANDATE_EXPIRES_MS: u64 = 4_000_000_000_000;
const MANDATE_DEADLINE_MS: u64 = 3_900_000_000_000;

async fn create_funded_account(sandbox: &near_sandbox::Sandbox, name: &str) -> Result<AccountId> {
    let sandbox_id: SandboxAccountId = name.parse()?;
    sandbox
        .create_account(sandbox_id)
        .initial_balance(NearToken::from_near(10))
        .send()
        .await?;
    Ok(name.parse()?)
}

fn mandate_terms() -> Value {
    json!({
        "program_sha256": "a".repeat(64),
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
        "expires_at_ms": MANDATE_EXPIRES_MS,
    })
}

fn mandate_policy(recipient: &str) -> Value {
    json!({
        "program_sha256": "a".repeat(64),
        "origin_asset": "nep141:usdc.near",
        "destination_asset": "nep141:usdc.near",
        "amount": "25000000",
        "min_receive": "24800000",
        "slippage_bps": 50,
        "fee_cap_bps": 100,
        "recipient": recipient,
        "refund_to": "refund.sandbox",
        "deadline_ms": MANDATE_DEADLINE_MS,
    })
}

async fn check_agent_intent(
    contract: &Contract,
    network: &NetworkConfig,
    owner: &AccountId,
    agent: &AccountId,
    policy: Value,
) -> Result<Vec<Value>> {
    let checks: Value = contract
        .call_function(
            "check_agent_intent",
            json!({"owner": owner, "agent": agent, "policy": policy}),
        )
        .read_only()
        .fetch_from(network)
        .await?
        .data;
    checks
        .as_array()
        .cloned()
        .context("check_agent_intent did not return a guard array")
}

fn guard_ok(checks: &[Value], guard: &str) -> bool {
    checks
        .iter()
        .find(|check| check["guard"] == guard)
        .and_then(|check| check["ok"].as_bool())
        .unwrap_or_else(|| panic!("missing guard {guard} in {checks:?}"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let wasm_path = std::env::args().nth(1).context("pass local x3_guard.wasm as arg")?;
    let wasm = std::fs::read(&wasm_path)?;
    anyhow::ensure!(wasm.starts_with(b"\0asm"), "not WASM");
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);
    let contract_id: AccountId = "guard.sandbox".parse()?;
    sandbox
        .create_account("guard.sandbox".parse()?)
        .initial_balance(NearToken::from_near(20))
        .send()
        .await?;
    let signer = Signer::from_secret_key(near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?)?;
    Contract::deploy(contract_id.clone())
        .use_code(wasm)
        .without_init_call()
        .with_signer(signer.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let contract = Account(contract_id.clone()).as_contract();
    contract
        .call_function("new", json!({}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(contract_id.clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let manager_key: String = contract
        .call_function("get_manager_public_key", json!({}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    anyhow::ensure!(
        manager_key == "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc",
        "unexpected pinned manager key"
    );
    let schema: u16 = contract
        .call_function("state_version", json!({}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    anyhow::ensure!(schema == 2, "unexpected Guard schema: {schema}");
    println!("X3_SANDBOX_GUARD_DEPLOY_AND_INIT_SUCCESS local_only=true ephemeral_accounts=true schema={schema}");

    // GENUINE archived, publicly signed 1Click DRY quote: real production
    // manager signature, but NEVER a live deposit address or swap authority.
    // We deliberately reproduce signature acceptance + policy refusal through
    // the actual compiled WASM on a sandbox, not near-sdk mock state.
    let fixtures_path = std::env::args()
        .nth(2)
        .context("pass x3_guard_args.json fixtures path")?;
    let fixture: Value = serde_json::from_slice(&std::fs::read(fixtures_path)?)?;
    let mut policy = fixture["register"]["policy"].clone();
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    policy["deadline_ms"] = json!(now_ms + 86_400_000); // register future-dated owner policy
    let nonce = "sandbox_real_dry_quote";
    let registered = contract
        .call_function("register_intent", json!({"nonce":nonce,"policy":policy}))
        .transaction()
        .gas(NearGas::from_tgas(180))
        .deposit(NearToken::from_near(1))
        .with_signer(contract_id.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(
        registered.is_success(),
        "sandbox policy registration failed: {registered:?}"
    );
    let signed = &fixture["real_1click_dry_quotes"][0];
    let args = json!({
        "owner":contract_id, "nonce":nonce,
        "signed_payload":signed["signed_payload"],
        "signature":signed["signature"],
    });
    let checks: Vec<Value> = contract
        .call_function("check_quote", args.clone())
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    let verified = checks
        .iter()
        .any(|r| r["guard"] == "quote_signature" && r["ok"] == true);
    let dry_refused = checks.iter().any(|r| r["guard"] == "quote_kind" && r["ok"] == false);
    anyhow::ensure!(
        verified,
        "genuine manager signature not verified by deployed WASM: {checks:?}"
    );
    anyhow::ensure!(
        dry_refused,
        "a genuine DRY quote must never authorize deposit: {checks:?}"
    );
    let rejected = contract
        .call_function(
            "accept_quote",
            json!({
                "nonce":nonce,
                "signed_payload":signed["signed_payload"],
                "signature":signed["signature"],
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(200))
        .deposit(NearToken::from_near(1))
        .with_signer(contract_id.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(
        !rejected.is_success(),
        "Guard accepted a dry quote as a payable deposit!"
    );
    let record: Value = contract
        .call_function(
            "get_intent",
            json!({
                "owner":contract_id, "nonce":nonce,
            }),
        )
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    anyhow::ensure!(
        record["quote"].is_null(),
        "rejected dry quote persisted deposit authorization"
    );
    println!("X3_SANDBOX_REAL_MANAGER_SIGNATURE_VALID_DRY_QUOTE_REFUSED local_only=true no_deposit_address=true persisted_quote=false");

    // Mandate lifecycle against the same deployed WASM: an owner writes a
    // mandate, the agent registers an allowed intent, a forbidden recipient is
    // refused, and revocation stops the agent. Sandbox-local only.
    let owner = create_funded_account(&sandbox, "owner.sandbox").await?;
    let agent = create_funded_account(&sandbox, "agent.sandbox").await?;

    let created = contract
        .call_function(
            "create_mandate",
            json!({"agent": agent.clone(), "terms": mandate_terms()}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(owner.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(created.is_success(), "sandbox create_mandate failed: {created:?}");

    let allowed = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": owner.clone(),
                "nonce": "sandbox_mandate_allow",
                "policy": mandate_policy("x3-demo.near"),
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(agent.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(
        allowed.is_success(),
        "sandbox allowed agent registration failed: {allowed:?}"
    );

    let forbidden_checks =
        check_agent_intent(&contract, &network, &owner, &agent, mandate_policy("attacker.near")).await?;
    anyhow::ensure!(
        !guard_ok(&forbidden_checks, "mandate_recipient"),
        "sandbox view accepted attacker.near: {forbidden_checks:?}"
    );

    let refused = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": owner.clone(),
                "nonce": "sandbox_mandate_refuse",
                "policy": mandate_policy("attacker.near"),
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(agent.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(
        !refused.is_success(),
        "sandbox accepted a forbidden recipient: {refused:?}"
    );

    let revoked = contract
        .call_function("revoke_mandate", json!({"agent": agent.clone()}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(owner.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(revoked.is_success(), "sandbox revoke_mandate failed: {revoked:?}");

    let mandate_record: Value = contract
        .call_function("get_mandate", json!({"owner": owner.clone(), "agent": agent.clone()}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    anyhow::ensure!(
        mandate_record["revoked"] == Value::Bool(true),
        "sandbox mandate was not marked revoked: {mandate_record}"
    );

    let revoked_checks =
        check_agent_intent(&contract, &network, &owner, &agent, mandate_policy("x3-demo.near")).await?;
    anyhow::ensure!(
        !guard_ok(&revoked_checks, "mandate_active"),
        "sandbox view still treats the revoked mandate as active: {revoked_checks:?}"
    );

    let after_revoke = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": owner.clone(),
                "nonce": "sandbox_mandate_post_revoke",
                "policy": mandate_policy("x3-demo.near"),
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(agent.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(
        !after_revoke.is_success(),
        "sandbox accepted a registration after revocation: {after_revoke:?}"
    );

    println!("X3_SANDBOX_MANDATE_ACCEPT_REFUSE_REVOKE local_only=true allowed_registration=true forbidden_recipient_refused=true revoked_registration_refused=true");
    Ok(())
}
