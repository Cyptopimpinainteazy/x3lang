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
    let upgrade_from = std::env::args().nth(2).map(std::fs::read).transpose()?;
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
        .use_code(upgrade_from.clone().unwrap_or_else(|| wasm.clone()))
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
        method_names: [
            "state_version",
            "agent_register_intent",
            "agent_accept_quote",
            "agent_record_outcome",
        ]
        .map(str::to_owned)
        .to_vec(),
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
            ensure!(
                p.method_names
                    == [
                        "state_version",
                        "agent_register_intent",
                        "agent_accept_quote",
                        "agent_record_outcome"
                    ],
                "method scope not enforced"
            );
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

    // Real zero-deposit scoped signer against owner-funded Guard storage.
    let policy = json!({
        "program_sha256": "a".repeat(64), "origin_asset": "nep141:usdc.near",
        "destination_asset": "nep141:usdc.near", "amount": "25000000",
        "min_receive": "24800000", "slippage_bps": 50, "fee_cap_bps": 100,
        "recipient": "x3-demo.near", "refund_to": "refund.sandbox",
        "deadline_ms": 3_900_000_000_000u64,
    });
    guard
        .call_function(
            "create_mandate",
            json!({"agent": agent_id, "terms": {
                "program_sha256": "a".repeat(64), "asset": "nep141:usdc.near",
                "per_payment_cap": "50000000", "daily_cap": "50000000",
                "destination_assets": ["nep141:usdc.near"], "recipients": ["x3-demo.near"],
                "refund_to": "refund.sandbox", "max_slippage_bps": 100, "max_fee_bps": 100,
                "approval_above": null, "approvers": [], "approval_threshold": 0,
                "expires_at_ms": 4_000_000_000_000u64,
            }}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(guard_id.clone(), administrator.clone())
        .send_to(&network)
        .await?
        .assert_success();
    if upgrade_from.is_some() {
        // Rehearse deployment over the pinned public binary, preserving records.
        guard
            .call_function(
                "agent_register_intent",
                json!({"owner": guard_id, "nonce": "before_upgrade", "policy": policy}),
            )
            .transaction()
            .gas(NearGas::from_tgas(100))
            .deposit(NearToken::from_near(1))
            .with_signer(agent_id.clone(), administrator.clone())
            .send_to(&network)
            .await?
            .assert_success();
        let before: serde_json::Value = guard
            .call_function("get_intent", json!({"owner": guard_id, "nonce": "before_upgrade"}))
            .read_only()
            .fetch_from(&network)
            .await?
            .data;
        let mandate_before: serde_json::Value = guard
            .call_function("get_mandate", json!({"owner": guard_id, "agent": agent_id}))
            .read_only()
            .fetch_from(&network)
            .await?
            .data;
        Contract::deploy(guard_id.clone())
            .use_code(wasm)
            .without_init_call()
            .with_signer(administrator.clone())
            .send_to(&network)
            .await?
            .assert_success();
        let after: serde_json::Value = guard
            .call_function("get_intent", json!({"owner": guard_id, "nonce": "before_upgrade"}))
            .read_only()
            .fetch_from(&network)
            .await?
            .data;
        let mandate_after: serde_json::Value = guard
            .call_function("get_mandate", json!({"owner": guard_id, "agent": agent_id}))
            .read_only()
            .fetch_from(&network)
            .await?
            .data;
        ensure!(
            before == after && !after.is_null() && mandate_before == mandate_after && !mandate_after.is_null(),
            "upgrade changed existing intent or mandate"
        );
        println!("X3_SANDBOX_STORAGE_UPGRADE_OK local_only=true existing_intent_preserved=true existing_mandate_preserved=true no_migration=true");
    }
    let args = json!({"owner": guard_id, "nonce": "scoped_storage", "policy": policy});
    let unfunded = guard
        .call_function("agent_register_intent", args.clone())
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent_id.clone(), scope_key.clone())
        .send_to(&network)
        .await?;
    ensure!(
        !unfunded.is_success() && format!("{unfunded:?}").contains("X3_STORAGE_BUDGET_EXHAUSTED"),
        "unfunded scoped write did not fail at storage budget: {unfunded:?}"
    );
    let missing: serde_json::Value = guard
        .call_function("get_intent", json!({"owner": guard_id, "nonce": "scoped_storage"}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(missing.is_null(), "unfunded failed receipt left an intent");
    guard
        .call_function("fund_agent_storage", json!({"agent": agent_id}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_millinear(100))
        .with_signer(guard_id.clone(), administrator.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let funded: String = guard
        .call_function(
            "get_agent_storage_balance",
            json!({"owner": guard_id, "agent": agent_id}),
        )
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(
        funded.parse::<u128>()? > 0 && funded.parse::<u128>()? < 100_000_000_000_000_000_000_000u128,
        "funding did not charge credit-record storage"
    );
    guard
        .call_function("agent_register_intent", args)
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent_id.clone(), scope_key.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let remaining: String = guard
        .call_function(
            "get_agent_storage_balance",
            json!({"owner": guard_id, "agent": agent_id}),
        )
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(
        remaining.parse::<u128>()? < funded.parse::<u128>()?,
        "registration did not debit owner credit"
    );
    let mut forbidden = policy.clone();
    forbidden["recipient"] = json!("attacker.sandbox");
    let refused = guard
        .call_function(
            "agent_register_intent",
            json!({"owner": guard_id, "nonce": "scoped_denied", "policy": forbidden}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent_id.clone(), scope_key.clone())
        .send_to(&network)
        .await?;
    ensure!(
        !refused.is_success() && format!("{refused:?}").contains("X3_MANDATE_DENIED"),
        "mandate bypass: {refused:?}"
    );
    let unchanged: String = guard
        .call_function(
            "get_agent_storage_balance",
            json!({"owner": guard_id, "agent": agent_id}),
        )
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(unchanged == remaining, "refusal debited storage credit");
    guard
        .call_function("withdraw_agent_storage", json!({"agent": agent_id}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(guard_id.clone(), administrator.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let zero: String = guard
        .call_function(
            "get_agent_storage_balance",
            json!({"owner": guard_id, "agent": agent_id}),
        )
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(zero == "0", "withdrawal did not settle credit");
    let exhausted = guard
        .call_function(
            "agent_register_intent",
            json!({"owner": guard_id, "nonce": "scoped_exhausted", "policy": policy}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent_id.clone(), scope_key.clone())
        .send_to(&network)
        .await?;
    ensure!(
        !exhausted.is_success() && format!("{exhausted:?}").contains("X3_STORAGE_BUDGET_EXHAUSTED"),
        "exhausted budget accepted: {exhausted:?}"
    );
    let missing: serde_json::Value = guard
        .call_function("get_intent", json!({"owner": guard_id, "nonce": "scoped_exhausted"}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(missing.is_null(), "budget-exhausted receipt left an intent");
    println!("X3_SANDBOX_OWNER_FUNDED_STORAGE_OK local_only=true scoped_zero_deposit_registration=true exhausted_receipt_rollback=true mandate_refusal=true withdrawal_callback=true");

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
