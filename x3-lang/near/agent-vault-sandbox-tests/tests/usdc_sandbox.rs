//! Real local NEAR Sandbox with NEAR's reference NEP-141 contract.
use anyhow::{ensure, Result};
use near_api::types::{
    transaction::actions::{Action, FunctionCallPermission, TransferAction},
    AccessKeyPermission,
};
use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer, Transaction};
use near_openapi_types::{ActionErrorKind, FunctionCallError, TxExecutionError};
use serde_json::{json, Value};

#[path = "../src/raw_capture.rs"]
mod raw_capture;

fn require_denial(
    result: near_api::types::transaction::result::TransactionResult,
    vault: &AccountId,
    reason: &str,
) -> Result<()> {
    let execution = result.into_full().ok_or_else(|| anyhow::anyhow!("execution missing"))?;
    let failures = execution.failures();
    ensure!(
        execution.is_failure() && failures.len() == 1 && &failures[0].executor_id == vault,
        "expected one failure from the vault"
    );
    let Err(near_api::types::errors::ExecutionError::TransactionExecutionFailed(error)) =
        failures[0].clone().into_result()
    else {
        anyhow::bail!("expected a typed execution failure")
    };
    let TxExecutionError::ActionError(error) = *error else {
        anyhow::bail!("expected an action error")
    };
    let ActionErrorKind::FunctionCallError(FunctionCallError::ExecutionError(message)) = error.kind else {
        anyhow::bail!("expected a contract execution error")
    };
    ensure!(
        message.strip_prefix("Smart contract panicked: ").unwrap_or(&message) == reason,
        "wrong denial: {message}"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires pinned local NEAR Sandbox and compiled vault and reference token WASM"]
async fn usdc_custody_and_scoped_payment() -> Result<()> {
    let vault_wasm = std::fs::read(std::env::var("X3_USDC_VAULT_WASM")?)?;
    let token_wasm = std::fs::read(std::env::var("X3_USDC_TOKEN_WASM")?)?;
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);
    let owner: AccountId = "owner.testnet".parse()?;
    let agent: AccountId = "agent.testnet".parse()?;
    let vault: AccountId = "vault.testnet".parse()?;
    let token: AccountId = "token.testnet".parse()?;
    let merchant: AccountId = "merchant.testnet".parse()?;
    for id in [&owner, &agent, &vault, &token, &merchant] {
        sandbox
            .create_account(id.as_str().parse()?)
            .initial_balance(NearToken::from_near(20))
            .send()
            .await?;
    }
    let full = Signer::from_secret_key(near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?)?;
    let gas = NearGas::from_tgas(100);
    let one_yocto = NearToken::from_yoctonear(1);
    let ft = Account(token.clone()).as_contract();
    let escrow = Account(vault.clone()).as_contract();
    Contract::deploy(token.clone())
        .use_code(token_wasm)
        .with_init_call(
            "new",
            json!({"owner_id": owner, "total_supply": "1000000",
            "metadata": {"spec": "ft-1.0.0", "name": "Sandbox test USD", "symbol": "TUSD", "decimals": 6}}),
        )?
        .gas(gas)
        .with_signer(full.clone())
        .send_to(&network)
        .await?
        .assert_success();
    Contract::deploy(vault.clone())
        .use_code(vault_wasm)
        .with_init_call(
            "new",
            json!({"mandate": {"owner": owner, "agent": agent,
            "token": token, "merchant": merchant, "program_sha256": "a".repeat(64),
            "per_payment_cap": "100000", "rolling_cap": "100000", "total_cap": "200000",
            "expires_at_ms": "4000000000000"}}),
        )?
        .gas(gas)
        .with_signer(full.clone())
        .send_to(&network)
        .await?
        .assert_success();

    ft.call_function(
        "storage_deposit",
        json!({"account_id": vault, "registration_only": true}),
    )
    .transaction()
    .deposit(NearToken::from_millinear(10))
    .gas(gas)
    .with_signer(owner.clone(), full.clone())
    .send_to(&network)
    .await?
    .assert_success();
    ft.call_function(
        "ft_transfer_call",
        json!({"receiver_id": vault, "amount": "200000", "msg": ""}),
    )
    .transaction()
    .deposit(one_yocto)
    .gas(gas)
    .with_signer(owner.clone(), full.clone())
    .send_to(&network)
    .await?
    .assert_success();

    let scoped = Signer::from_secret_key(near_api::signer::generate_secret_key()?)?;
    let public_key = scoped.get_public_key().await?;
    Account(agent.clone())
        .add_key(
            AccessKeyPermission::FunctionCall(FunctionCallPermission {
                allowance: Some(NearToken::from_near(1)),
                receiver_id: vault.to_string(),
                method_names: vec!["execute".into()],
            }),
            public_key,
        )
        .with_signer(full.clone())
        .send_to(&network)
        .await?
        .assert_success();
    Account(agent.clone())
        .delete_key(full.get_public_key().await?)
        .with_signer(full.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let keys = Account(agent.clone()).list_keys().fetch_from(&network).await?;
    ensure!(keys.data.len() == 1, "the local agent must have only the scoped key");
    let key = Account(agent.clone())
        .access_key(public_key)
        .fetch_from(&network)
        .await?;
    ensure!(
        matches!(&key.data.permission, AccessKeyPermission::FunctionCall(permission)
        if permission.receiver_id.as_str() == vault.as_str() && permission.method_names == ["execute"]),
        "unexpected agent key scope"
    );

    let direct = Transaction::construct(agent.clone(), merchant.clone())
        .add_action(Action::Transfer(TransferAction { deposit: one_yocto }))
        .with_signer(scoped.clone())
        .presign_with(&network)
        .await?
        .transaction
        .signed()
        .ok_or_else(|| anyhow::anyhow!("missing signed local transfer"))?;
    let rejection = raw_capture::send_signed_raw(&network, direct, json!({"case": "direct_transfer"})).await?;
    ensure!(
        rejection.pointer("/error/data/TxExecutionError/InvalidTxError/InvalidAccessKeyError")
            == Some(&json!("RequiresFullAccess")),
        "expected typed scoped-key rejection"
    );

    let execute = |nonce: &str, recipient: &AccountId, amount: &str| {
        escrow
            .call_function(
                "execute",
                json!({"nonce": nonce, "recipient": recipient, "amount": amount}),
            )
            .transaction()
            .gas(gas)
            .with_signer(agent.clone(), scoped.clone())
    };
    let view = |method: &str| escrow.call_function(method, json!({})).read_only();
    let balance = |id: &AccountId| ft.call_function("ft_balance_of", json!({"account_id": id})).read_only();

    require_denial(
        execute("1", &owner, "1").send_to(&network).await?,
        &vault,
        "X3_USDC_DENIED: recipient",
    )?;
    require_denial(
        execute("1", &merchant, "100001").send_to(&network).await?,
        &vault,
        "X3_USDC_DENIED: payment cap",
    )?;
    let available: Value = view("available").fetch_from(&network).await?.data;
    ensure!(available == "200000", "funding or refusal escrow mismatch");

    // Real token failure. The merchant has not registered storage.
    let failed = execute("1", &merchant, "100000").send_to(&network).await?;
    let execution = failed
        .into_full()
        .ok_or_else(|| anyhow::anyhow!("failed transfer execution missing"))?;
    ensure!(
        execution.failures().iter().any(|failure| failure.executor_id == token),
        "expected failed token receipt"
    );
    let available: Value = view("available").fetch_from(&network).await?.data;
    let pending: Value = view("pending").fetch_from(&network).await?.data;
    ensure!(
        available == "200000" && pending.is_null(),
        "failed token transfer did not restore escrow"
    );
    require_denial(
        execute("1", &merchant, "1").send_to(&network).await?,
        &vault,
        "X3_USDC_DENIED: nonce",
    )?;

    ft.call_function(
        "storage_deposit",
        json!({"account_id": merchant, "registration_only": true}),
    )
    .transaction()
    .deposit(NearToken::from_millinear(10))
    .gas(gas)
    .with_signer(owner.clone(), full.clone())
    .send_to(&network)
    .await?
    .assert_success();
    let paid = execute("2", &merchant, "100000").send_to(&network).await?;
    ensure!(paid.is_success(), "merchant payout failed: {paid:?}");
    let merchant_balance: Value = balance(&merchant).fetch_from(&network).await?.data;
    let vault_balance: Value = balance(&vault).fetch_from(&network).await?.data;
    let spent: Value = view("spent").fetch_from(&network).await?.data;
    ensure!(
        merchant_balance == "100000" && vault_balance == "100000" && spent == "100000",
        "successful payout did not match token and vault ledgers"
    );
    require_denial(
        execute("3", &merchant, "1").send_to(&network).await?,
        &vault,
        "X3_USDC_DENIED: rolling cap",
    )?;

    escrow
        .call_function("revoke", json!({}))
        .transaction()
        .gas(gas)
        .with_signer(owner.clone(), full.clone())
        .send_to(&network)
        .await?
        .assert_success();
    require_denial(
        execute("3", &merchant, "1").send_to(&network).await?,
        &vault,
        "X3_USDC_DENIED: inactive mandate",
    )?;
    escrow
        .call_function("withdraw", json!({"amount": "100000"}))
        .transaction()
        .gas(gas)
        .with_signer(owner.clone(), full)
        .send_to(&network)
        .await?
        .assert_success();
    let owner_balance: Value = balance(&owner).fetch_from(&network).await?.data;
    let vault_balance: Value = balance(&vault).fetch_from(&network).await?.data;
    ensure!(
        owner_balance == "900000" && vault_balance == "0",
        "owner recovery did not settle"
    );
    println!("Local NEAR Sandbox: scoped token payment, typed denials, failed payout restoration and owner recovery verified.");
    Ok(())
}
