//! Exact production x3-guard WASM, deployed to a LOCAL NEAR sandbox node only.
//! Never contacts testnet, imports production wallet keys or moves real funds.
use anyhow::{Context, Result};
use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer};
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

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
    Ok(())
}
