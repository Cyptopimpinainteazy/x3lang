//! Real LOCAL NEAR Sandbox access-key authorization boundary.
//!
//! Checks that a disposable agent with *only* a function-call key cannot
//! transfer NEAR, call an unrelated contract, call unapproved methods, or
//! attach a deposit. This is not a public-chain test or an X3 custody proof.
//! The owner full-access key remains in this trusted *test harness* only.
use anyhow::{bail, ensure, Context, Result};
use near_api::{
    signer::generate_secret_key,
    types::{
        transaction::actions::{Action, FunctionCallPermission, TransferAction},
        AccessKeyPermission,
    },
    Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer, Transaction,
};
use serde_json::json;

// Never accept success or a generic transport error as an authorization refusal.
// Require a typed node transaction-rejection, not arbitrary RPC timeout.
// near-api 0.8.6 currently suppresses the specific InvalidAccessKey subtype.
fn require_protocol_denial<T: std::fmt::Debug, E: std::fmt::Debug>(
    result: std::result::Result<T, E>,
    expected_reason: &str,
) -> Result<()> {
    match result {
        Ok(success) => bail!("authorization bypass: node accepted forbidden scoped-key transaction: {success:?}"),
        Err(error) => {
            let reason = format!("{error:?}");
            ensure!(
                reason.contains("ServerError(InvalidTransaction"),
                "expected a node InvalidTransaction rejection for {expected_reason}, got {reason}"
            );
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let wasm_path = std::env::args().nth(1).context("pass exact local Guard WASM path")?;
    let wasm = std::fs::read(&wasm_path)?;
    ensure!(wasm.starts_with(b"\0asm"), "invalid compiled Guard WASM");
    let sandbox = near_sandbox::Sandbox::start_sandbox().await?;
    let network = NetworkConfig::from_rpc_url("sandbox", sandbox.rpc_addr.parse()?);

    let guard_id: AccountId = "guard.sandbox".parse()?;
    let agent_id: AccountId = "agent.sandbox".parse()?;
    let attacker_id: AccountId = "attacker.sandbox".parse()?;
    for id in [&guard_id, &agent_id, &attacker_id] {
        sandbox
            .create_account(id.to_string().parse()?)
            .initial_balance(NearToken::from_near(20))
            .send()
            .await?;
    }

    // Only the privileged harness receives this Sandbox's throwaway genesis
    // administrator key; it is never printed, stored or handed to the agent.
    let administrator = Signer::from_secret_key(near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?)?;

    Contract::deploy(guard_id.clone())
        .use_code(wasm)
        .without_init_call()
        .with_signer(administrator.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let guard = Account(guard_id.clone()).as_contract();
    guard
        .call_function("new", json!({}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(guard_id.clone(), administrator.clone())
        .send_to(&network)
        .await?
        .assert_success();

    let scope_key = Signer::from_secret_key(generate_secret_key()?)?;
    let public_key = scope_key.get_public_key().await?;
    let permission = AccessKeyPermission::FunctionCall(FunctionCallPermission {
        allowance: Some(NearToken::from_near(5)),
        receiver_id: guard_id.to_string(),
        method_names: vec!["state_version".to_owned()],
    });
    Account(agent_id.clone())
        .add_key(permission, public_key)
        .with_signer(administrator.clone())
        .send_to(&network)
        .await?
        .assert_success();

    // Query the *real local node* to verify key permissions rather than
    // assuming a builder constructed the intended on-chain access key.
    let installed = Account(agent_id.clone())
        .access_key(public_key)
        .fetch_from(&network)
        .await?
        .data;
    match installed.permission {
        AccessKeyPermission::FunctionCall(p) => {
            ensure!(p.receiver_id == guard_id.as_str(), "incorrect receiver");
            ensure!(p.method_names == ["state_version"], "method scope not enforced");
            ensure!(p.allowance == Some(NearToken::from_near(5)), "unbounded gas allowance");
        }
        _ => bail!("agent key unexpectedly has full access"),
    }

    // Positive control: a permitted zero-deposit contract method succeeds
    // using exactly the same scoped key tested in each adversarial attempt.
    let allowed = guard
        .call_function("state_version", json!({}))
        .transaction()
        .gas(NearGas::from_tgas(10))
        .with_signer(agent_id.clone(), scope_key.clone())
        .send_to(&network)
        .await?;
    ensure!(allowed.is_success(), "permitted scoped method was rejected");
    println!("X3_SANDBOX_SCOPED_ALLOWED_METHOD_OK local_only=true guard_method=state_version");

    let before = Account(attacker_id.clone())
        .view()
        .fetch_from(&network)
        .await?
        .data
        .amount;

    // Raw transaction bypass attempt: no X3 Python or frontend check runs.
    require_protocol_denial(
        Transaction::construct(agent_id.clone(), attacker_id.clone())
            .add_action(Action::Transfer(TransferAction {
                deposit: NearToken::from_near(1),
            }))
            .with_signer(scope_key.clone())
            .send_to(&network)
            .await,
        "RequiresFullAccess",
    )?;

    // Same scoped key; real contract method, forbidden by this access key.
    require_protocol_denial(
        guard
            .call_function("get_manager_public_key", json!({}))
            .transaction()
            .gas(NearGas::from_tgas(10))
            .with_signer(agent_id.clone(), scope_key.clone())
            .send_to(&network)
            .await,
        "MethodNameMismatch",
    )?;

    // Permitted method name, wrong receiver account.
    let wrong_contract = Account(attacker_id.clone()).as_contract();
    require_protocol_denial(
        wrong_contract
            .call_function("state_version", json!({}))
            .transaction()
            .gas(NearGas::from_tgas(10))
            .with_signer(agent_id.clone(), scope_key.clone())
            .send_to(&network)
            .await,
        "ReceiverMismatch",
    )?;

    // A function-call key cannot attach even one yoctoNEAR.
    require_protocol_denial(
        guard
            .call_function("state_version", json!({}))
            .transaction()
            .gas(NearGas::from_tgas(10))
            .deposit(NearToken::from_yoctonear(1))
            .with_signer(agent_id.clone(), scope_key.clone())
            .send_to(&network)
            .await,
        "DepositWithFunctionCall",
    )?;

    let after = Account(attacker_id).view().fetch_from(&network).await?.data.amount;
    ensure!(before == after, "attacker balance changed after forbidden transactions");

    // Trusted administrator revokes the disposable agent access key on the
    // actual local node. The previously permitted method must now fail even
    // when called with the identical signed key, bypassing all frontends.
    Account(agent_id.clone())
        .delete_key(public_key)
        .with_signer(administrator)
        .send_to(&network)
        .await?
        .assert_success();
    let revoked_result = guard
        .call_function("state_version", json!({}))
        .transaction()
        .gas(NearGas::from_tgas(10))
        .with_signer(agent_id, scope_key)
        .send_to(&network)
        .await;
    match revoked_result {
        Ok(result) => bail!("revoked key still signed an allowed method: {result:?}"),
        Err(error) => {
            let text = format!("{error:?}");
            ensure!(
                text.contains("FetchNonceError") && text.contains("access key") && text.contains("does not exist"),
                "revoked signer did not receive a verified missing-access-key failure: {text}"
            );
        }
    }
    println!(
        "X3_SANDBOX_SCOPED_KEY_BYPASS_REFUSED local_only=true raw_transfer=true wrong_method=true wrong_receiver=true deposit=true attacker_balance_unchanged=true revoked_key_rejected=true owner_key_remains_in_harness=true"
    );
    println!(
        "X3_SANDBOX_SCOPED_KEY_LIMITS local_only=true scoped_guard_method_only=true guard_custody_proven=false cross_vm_enforcement_proven=false public_testnet_transactions=false"
    );
    Ok(())
}
