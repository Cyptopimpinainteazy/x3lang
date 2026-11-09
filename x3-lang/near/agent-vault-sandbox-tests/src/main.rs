//! Real offline NEAR Sandbox transactions; no public wallets, no real funds.
use anyhow::{ensure, Result};
use near_api::types::{transaction::actions::FunctionCallPermission, AccessKeyPermission};
use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer, Tokens, Transaction};
use near_openapi_types::{ActionErrorKind, FunctionCallError, TxExecutionError};
use serde_json::{json, Value};

mod raw_capture;
use raw_capture::send_signed_raw;

fn require_access_key_error<'a>(response: &'a Value, expected: &Value) -> Result<&'a Value> {
    ensure!(
        response.get("result").is_none() && response.get("error").is_some(),
        "raw transaction unexpectedly returned a result: {response}"
    );
    let actual = response
        .pointer("/error/data/TxExecutionError/InvalidTxError/InvalidAccessKeyError")
        .ok_or_else(|| anyhow::anyhow!("raw rejection lacked InvalidAccessKeyError: {response}"))?;
    ensure!(
        actual == expected,
        "unexpected access-key rejection: expected {expected}, got {actual}"
    );
    Ok(actual)
}

fn policy_reason_from_tx_error(error: TxExecutionError) -> Option<String> {
    match error {
        TxExecutionError::ActionError(error) => match error.kind {
            ActionErrorKind::FunctionCallError(FunctionCallError::ExecutionError(reason)) => Some(reason),
            _ => None,
        },
        TxExecutionError::InvalidTxError(_) => None,
    }
}

fn require_single_policy_reason(errors: Vec<TxExecutionError>, expected_reason: &str) -> Result<String> {
    ensure!(
        errors.len() == 1,
        "expected exactly one contract failure, got {}",
        errors.len()
    );
    let contract_reason = errors
        .into_iter()
        .next()
        .and_then(policy_reason_from_tx_error)
        .ok_or_else(|| anyhow::anyhow!("contract failure lacked an execution reason"))?;
    let actual_reason = contract_reason
        .strip_prefix("Smart contract panicked: ")
        .unwrap_or(&contract_reason);
    ensure!(
        actual_reason == expected_reason,
        "unexpected policy reason: expected {expected_reason:?}, got {actual_reason:?}"
    );
    Ok(actual_reason.to_string())
}

fn require_policy_failure(
    result: near_api::types::transaction::result::TransactionResult,
    expected_reason: &str,
    contract_account: &AccountId,
) -> Result<()> {
    let execution = result
        .into_full()
        .ok_or_else(|| anyhow::anyhow!("transaction did not reach a full execution result"))?;
    ensure!(
        execution.is_failure(),
        "expected contract policy failure, got {execution:?}"
    );
    let failures = execution.failures();
    ensure!(
        failures.len() == 1,
        "expected exactly one failed execution outcome, got {}: {execution:?}",
        failures.len()
    );
    let failure = failures[0];
    ensure!(
        &failure.executor_id == contract_account,
        "policy failure executed on {:?}, expected {:?}",
        failure.executor_id,
        contract_account
    );
    let error = match failure.clone().into_result() {
        Err(near_api::types::errors::ExecutionError::TransactionExecutionFailed(error)) => *error,
        other => anyhow::bail!("failure outcome was not a typed transaction error: {other:?}"),
    };
    require_single_policy_reason(vec![error], expected_reason)?;
    println!(
        "X3_VAULT_SANDBOX_POLICY_REFUSAL local_only=true reason={:?}",
        expected_reason
    );
    Ok(())
}

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
            pubkey,
        )
        .with_signer(full.clone())
        .send_to(&network)
        .await?
        .assert_success();
    let recorded = Account(agent.clone()).access_key(pubkey).fetch_from(&network).await?;
    match &recorded.data.permission {
        AccessKeyPermission::FunctionCall(permission) => {
            ensure!(
                permission.receiver_id.as_str() == vault.as_str()
                    && permission.method_names == vec!["request_payment".to_string()],
                "agent key grants broader methods or receiver than approved"
            );
        }
        _ => anyhow::bail!("agent key has full access instead of FunctionCall-only"),
    }
    println!("X3_VAULT_SANDBOX_FUNCTIONCALL_KEY_CONFIRMED scope=vault.testnet::request_payment local_only=true");

    // The scoped key can sign bytes; the node must reject unauthorized actions.
    // The separate full-access credential remains in this local test harness.
    let attacker_balance_before = Tokens::account(attacker.clone())
        .near_balance()
        .fetch_from(&network)
        .await?
        .total
        .as_yoctonear();
    let direct_signed = Transaction::construct(agent.clone(), attacker.clone())
        .add_action(near_api::types::transaction::actions::Action::Transfer(
            near_api::types::transaction::actions::TransferAction {
                deposit: NearToken::from_yoctonear(10u128.pow(22)),
            },
        ))
        .with_signer(scoped.clone())
        .presign_with(&network)
        .await?
        .transaction
        .signed()
        .ok_or_else(|| anyhow::anyhow!("direct transfer transaction was not signed"))?;
    let direct_observation = send_signed_raw(&network, direct_signed, json!({"case":"direct_transfer"})).await?;
    require_access_key_error(&direct_observation, &json!("RequiresFullAccess"))?;
    let attacker_balance_after = Tokens::account(attacker.clone())
        .near_balance()
        .fetch_from(&network)
        .await?
        .total
        .as_yoctonear();
    ensure!(
        attacker_balance_after == attacker_balance_before,
        "direct transfer changed attacker balance"
    );
    println!("X3_VAULT_SANDBOX_SCOPED_DIRECT_TRANSFER_REFUSED local_only=true rejection=RequiresFullAccess balance_unchanged=true");

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
    require_policy_failure(wrong, "X3_VAULT_DENIED: recipient not allowlisted", &vault)?;
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
    require_policy_failure(oversized, "X3_VAULT_DENIED: per-payment cap exceeded", &vault)?;
    let available_after_denials: Value = contract
        .call_function("available", json!({}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(
        available_after_denials == "1000000000000000000000000",
        "denied requests changed escrow: {available_after_denials:?}"
    );
    println!("X3_VAULT_SANDBOX_RECIPIENT_AND_CAP_REFUSED local_only=true reasons_verified=true balance_unchanged=true");

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
    require_policy_failure(replay, "X3_VAULT_DENIED: nonce already used", &vault)?;

    // Even the owner cannot recover live escrow while the mandate is active.
    let active_recovery = contract
        .call_function("recover_available", json!({"amount":"500000000000000000000000"}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(owner.clone(), full.clone())
        .send_to(&network)
        .await?;
    require_policy_failure(
        active_recovery,
        "X3_VAULT_DENIED: revoke or expire mandate before recovery",
        &vault,
    )?;
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
    require_policy_failure(revoked, "X3_VAULT_DENIED: owner revoked mandate", &vault)?;
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
    println!("X3_VAULT_SANDBOX_REPLAY_AND_REVOCATION_REFUSED local_only=true reasons_verified=true terminal_payment_preserved=true");

    // The compromised scoped key has no permission for owner recovery.
    let agent_recovery_signed = contract
        .call_function("recover_available", json!({"amount":"100000000000000000000000"}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .presign_with(&network)
        .await?
        .transaction
        .signed()
        .ok_or_else(|| anyhow::anyhow!("agent recovery transaction was not signed"))?;
    let recovery_observation = send_signed_raw(
        &network,
        agent_recovery_signed,
        json!({"case":"wrong_method","method_name":"recover_available"}),
    )
    .await?;
    require_access_key_error(
        &recovery_observation,
        &json!({"MethodNameMismatch":{"method_name":"recover_available"}}),
    )?;
    println!("X3_VAULT_SANDBOX_AGENT_RECOVERY_REFUSED local_only=true rejection=MethodNameMismatch:recover_available");

    let available_before: Value = contract
        .call_function("available", json!({}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(
        available_before == "900000000000000000000000",
        "unexpected escrow before owner recovery: {available_before:?}"
    );
    // Observe real Sandbox account balances, not just an internal ledger delta.
    let owner_balance_before = Tokens::account(owner.clone())
        .near_balance()
        .fetch_from(&network)
        .await?
        .total
        .as_yoctonear();
    let vault_balance_before = Tokens::account(vault.clone())
        .near_balance()
        .fetch_from(&network)
        .await?
        .total
        .as_yoctonear();
    // Actual local chain Promise transfer and final callback must both succeed.
    let owner_recovery = contract
        .call_function("recover_available", json!({"amount":"500000000000000000000000"}))
        .transaction()
        .gas(NearGas::from_tgas(140))
        .with_signer(owner.clone(), full.clone())
        .send_to(&network)
        .await?;
    ensure!(
        owner_recovery.is_success(),
        "owner recovery failed on real Sandbox node: {owner_recovery:?}"
    );
    let available_after: Value = contract
        .call_function("available", json!({}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    ensure!(
        available_after == "400000000000000000000000",
        "owner recovery did not debit exactly 0.5 test NEAR: {available_after:?}"
    );
    let owner_balance_after = Tokens::account(owner.clone())
        .near_balance()
        .fetch_from(&network)
        .await?
        .total
        .as_yoctonear();
    let vault_balance_after = Tokens::account(vault.clone())
        .near_balance()
        .fetch_from(&network)
        .await?
        .total
        .as_yoctonear();
    let owner_net = owner_balance_after
        .checked_sub(owner_balance_before)
        .ok_or_else(|| anyhow::anyhow!("owner balance decreased during recovery"))?;
    ensure!(
        owner_net >= 4u128 * 10u128.pow(23),
        "owner did not receive the 0.5 NEAR recovery after gas: net={owner_net}"
    );
    ensure!(
        vault_balance_before >= vault_balance_after + 5u128 * 10u128.pow(23),
        "vault account balance did not fund owner recovery"
    );
    println!("X3_VAULT_SANDBOX_OWNER_RECOVERY_SUCCESS local_only=true owner_only=true active_mandate_refusal_verified=true recovered_yoctonear=500000000000000000000000 pending_payout_protected_by_unit_tests=true owner_net_yoctonear={owner_net} vault_account_balance_decreased=true");
    println!("X3_VAULT_SANDBOX_COMPLETE agent_key_restricted=true funds_vault_custodied=true public_chain=false");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_transfer_requires_the_exact_access_key_variant() {
        let response = json!({
            "error": {
                "data": {
                    "TxExecutionError": {
                        "InvalidTxError": {
                            "InvalidAccessKeyError": "RequiresFullAccess"
                        }
                    }
                }
            }
        });
        assert_eq!(
            require_access_key_error(&response, &json!("RequiresFullAccess")).unwrap(),
            &json!("RequiresFullAccess")
        );
        assert!(require_access_key_error(&response, &json!("ReceiverMismatch")).is_err());
    }

    #[test]
    fn empty_or_generic_errors_are_not_access_key_proof() {
        for response in [
            json!({"error": {}}),
            json!({"error": {"data": {"TxExecutionError": {"InvalidTxError": {"InvalidAccessKeyError": {}}}}}}),
            json!({"result": {"status": {"Failure": "unknown"}}}),
        ] {
            assert!(require_access_key_error(&response, &json!("RequiresFullAccess")).is_err());
        }
    }

    #[test]
    fn policy_reason_comes_from_typed_execution_failure() {
        let error = TxExecutionError::ActionError(near_openapi_types::ActionError {
            index: Some(0),
            kind: ActionErrorKind::FunctionCallError(FunctionCallError::ExecutionError(
                "Smart contract panicked: X3_VAULT_DENIED: recipient not allowlisted".into(),
            )),
        });
        assert_eq!(
            policy_reason_from_tx_error(error).as_deref(),
            Some("Smart contract panicked: X3_VAULT_DENIED: recipient not allowlisted")
        );
    }

    #[test]
    fn invalid_transaction_error_is_not_a_contract_policy_reason() {
        let error = TxExecutionError::InvalidTxError(near_openapi_types::InvalidTxError::InvalidSignature);
        assert_eq!(policy_reason_from_tx_error(error), None);
    }

    fn policy_error(reason: &str) -> TxExecutionError {
        TxExecutionError::ActionError(near_openapi_types::ActionError {
            index: Some(0),
            kind: ActionErrorKind::FunctionCallError(FunctionCallError::ExecutionError(format!(
                "Smart contract panicked: {reason}"
            ))),
        })
    }

    #[test]
    fn every_policy_failure_must_be_the_single_expected_reason() {
        let expected = "X3_VAULT_DENIED: recipient not allowlisted";
        assert_eq!(
            require_single_policy_reason(vec![policy_error(expected)], expected).unwrap(),
            expected
        );
        assert!(require_single_policy_reason(
            vec![
                policy_error(expected),
                policy_error("X3_VAULT_DENIED: owner revoked mandate")
            ],
            expected,
        )
        .is_err());
        assert!(
            require_single_policy_reason(vec![policy_error("X3_VAULT_DENIED: owner revoked mandate")], expected,)
                .is_err()
        );
    }
}
