//! Real offline NEAR Sandbox transactions; no public wallets, no real funds.
use anyhow::{ensure, Result};
use near_api::types::{transaction::actions::FunctionCallPermission, AccessKeyPermission};
use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer, Tokens};
use serde_json::{json, Value};

#[tokio::main]
async fn main() -> Result<()> {
    let wasm_path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("missing wasm path"))?;
    let wasm = std::fs::read(wasm_path)?;
    ensure!(wasm.starts_with(b"\0asm"), "not a wasm contract");
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);
    let owner: AccountId = "owner.testnet".parse()?;
    let vault: AccountId = "vault.testnet".parse()?;
    let agent: AccountId = "agent.testnet".parse()?;
    let allowed: AccountId = "allowed.testnet".parse()?;
    let attacker: AccountId = "attacker.testnet".parse()?;
    for id in [
        "owner.testnet",
        "vault.testnet",
        "agent.testnet",
        "allowed.testnet",
        "attacker.testnet",
    ] {
        sandbox
            .create_account(id.parse()?)
            .initial_balance(NearToken::from_near(20))
            .send()
            .await?;
    }
    let full = Signer::from_secret_key(near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?)?;
    Contract::deploy(vault.clone())
        .use_code(wasm)
        .without_init_call()
        .with_signer(full.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let contract = Account(vault.clone()).as_contract();
    contract
        .call_function(
            "new",
            json!({
                "owner":owner,"agent":agent,"recipient":allowed,
                "cap":"200000000000000000000000",
                "budget":"400000000000000000000000",
                "deadline_ms":4000000000000u64
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(80))
        .with_signer(vault.clone(), full.clone())
        .send_to(&network)
        .await?
        .assert_success();

    let fund = contract
        .call_function("fund", json!({}))
        .transaction()
        .deposit(NearToken::from_near(1))
        .gas(NearGas::from_tgas(80))
        .with_signer(owner.clone(), full.clone())
        .send_to(&network)
        .await?;
    ensure!(fund.is_success(), "real Sandbox vault deposit failed: {fund:?}");

    let secret = near_api::signer::generate_secret_key()?;
    let scoped = Signer::from_secret_key(secret)?;
    let pubkey = scoped.get_public_key().await?;
    Account(agent.clone())
        .add_key(
            AccessKeyPermission::FunctionCall(FunctionCallPermission {
                allowance: Some(NearToken::from_near(1)),
                receiver_id: vault.to_string(),
                method_names: vec!["request_payment".into()],
            }),
            pubkey.clone(),
        )
        .with_signer(full.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let recorded = Account(agent.clone())
        .access_key(pubkey.clone())
        .fetch_from(&network)
        .await?;
    match &recorded.data.permission {
        AccessKeyPermission::FunctionCall(permission) => {
            ensure!(
                permission.receiver_id == vault.to_string()
                    && permission.method_names == vec!["request_payment".to_string()],
                "agent key grants broader methods or receiver than approved"
            );
        }
        _ => anyhow::bail!("agent key has full access instead of FunctionCall-only"),
    }
    println!("X3_VAULT_SANDBOX_FUNCTIONCALL_KEY_CONFIRMED scope=vault.testnet::request_payment local_only=true");

    // Attempt to spend native NEAR directly from agent account with the ONLY
    // exposed disposable agent key. A FunctionCall-only key cannot sign Transfer.
    let raw_attempt = Tokens::account(agent.clone())
        .send_to(attacker.clone())
        .near(NearToken::from_yoctonear(10u128.pow(22)))
        .with_signer(scoped.clone())
        .send_to(&network)
        .await;
    let direct_denial_class = match raw_attempt {
        Ok(o) if !o.is_success() => "failed_chain_transaction",
        Ok(_) => anyhow::bail!("compromised scoped key transferred native NEAR directly"),
        Err(error) => {
            println!(
                "X3_VAULT_SANDBOX_DIRECT_TRANSFER_ERROR_CLASS {:?}",
                error.to_string().chars().take(160).collect::<String>()
            );
            "sdk_or_rpc_permission_refusal"
        }
    };
    println!("X3_VAULT_SANDBOX_SCOPED_DIRECT_TRANSFER_REFUSED local_only=true key_permission=FunctionCall rejection_class={direct_denial_class}");

    let call = |nonce: &str, recipient: &AccountId, amount: &str| {
        json!({
            "nonce":nonce,"recipient":recipient,"amount":amount
        })
    };
    let wrong = contract
        .call_function("request_payment", call("wrong", &attacker, "100000000000000000000000"))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(&network)
        .await?;
    ensure!(!wrong.is_success(), "attacker recipient was paid");
    let oversized = contract
        .call_function(
            "request_payment",
            call("oversized", &allowed, "300000000000000000000000"),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(&network)
        .await?;
    ensure!(!oversized.is_success(), "payment over cap was allowed");
    println!("X3_VAULT_SANDBOX_WRONG_RECIPIENT_AND_CAP_REFUSED local_only=true");

    let good = contract
        .call_function("request_payment", call("once", &allowed, "100000000000000000000000"))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(&network)
        .await?;
    ensure!(good.is_success(), "allowed payment failed: {good:?}");
    let receipt: Value = contract
        .call_function("payment", json!({"nonce":"once"}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(
        receipt["state"] == "Paid",
        "asynchronous native payout not confirmed: {receipt:?}"
    );
    println!("X3_VAULT_SANDBOX_ALLOWED_PAYMENT_SUCCESS local_only=true units=100000000000000000000000");

    let replay = contract
        .call_function("request_payment", call("once", &allowed, "100000000000000000000000"))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(&network)
        .await?;
    ensure!(!replay.is_success(), "replayed nonce permitted");
    contract
        .call_function("revoke", json!({}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(owner.clone(), full.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let revoked = contract
        .call_function(
            "request_payment",
            call("after-revoke", &allowed, "100000000000000000000000"),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(&network)
        .await?;
    ensure!(!revoked.is_success(), "revoked agent was allowed");
    let final_state: Value = contract
        .call_function("payment", json!({"nonce":"once"}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(
        final_state["state"] == "Paid",
        "successful payment reverted due to later denial"
    );
    println!("X3_VAULT_SANDBOX_NONCE_REPLAY_AND_REVOCATION_REFUSED local_only=true");
    println!("X3_VAULT_SANDBOX_COMPLETE agent_key_restricted=true funds_vault_custodied=true public_chain=false");
    Ok(())
}
