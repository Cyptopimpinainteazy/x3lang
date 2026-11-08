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

    // Structured raw-node rejection evidence is still required. Until the exact
    // access-key subtype and contract failure reasons are captured, observations
    // are inconclusive and must prevent the final proof-success marker.
    let mut inconclusive = Vec::new();

    // The scoped key can sign bytes; the node must reject unauthorized actions.
    // The separate full-access credential remains in this local test harness.
    let raw_attempt = Tokens::account(agent.clone())
        .send_to(attacker.clone())
        .near(NearToken::from_yoctonear(10u128.pow(22)))
        .with_signer(scoped.clone())
        .send_to(&network)
        .await;
    let direct_observation = match raw_attempt {
        Ok(o) if o.is_success() => {
            anyhow::bail!("compromised scoped key transferred native NEAR directly")
        }
        Ok(_) => "non_success_outcome_unclassified",
        Err(error) => {
            eprintln!("X3_VAULT_SANDBOX_DIRECT_TRANSFER_UNCLASSIFIED_ERROR {error}");
            "sdk_or_rpc_error_unclassified"
        }
    };
    inconclusive.push("direct_transfer_permission");
    eprintln!("X3_VAULT_SANDBOX_DIRECT_TRANSFER_INCONCLUSIVE local_only=true observation={direct_observation} permission_proven=false");

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
    inconclusive.push("wrong_recipient_reason");
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
    inconclusive.push("payment_cap_reason");
    eprintln!("X3_VAULT_SANDBOX_RECIPIENT_AND_CAP_FAILURES_OBSERVED local_only=true reasons_verified=false");

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
    inconclusive.push("nonce_replay_reason");

    // Even the owner cannot recover live escrow while the mandate is active.
    let active_recovery = contract
        .call_function("recover_available", json!({"amount":"500000000000000000000000"}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(owner.clone(), full.clone())
        .send_to(&network)
        .await?;
    ensure!(
        !active_recovery.is_success(),
        "active owner mandate allowed escrow recovery"
    );
    inconclusive.push("active_recovery_reason");
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
    inconclusive.push("revocation_reason");
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
    eprintln!("X3_VAULT_SANDBOX_REPLAY_AND_REVOCATION_FAILURES_OBSERVED local_only=true reasons_verified=false");

    // The compromised scoped key has no permission for owner recovery.
    let agent_recovery = contract
        .call_function("recover_available", json!({"amount":"100000000000000000000000"}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(&network)
        .await;
    let recovery_observation = match agent_recovery {
        Ok(result) if result.is_success() => {
            anyhow::bail!("scoped agent was able to recover owner escrow")
        }
        Ok(_) => "non_success_outcome_unclassified",
        Err(error) => {
            eprintln!("X3_VAULT_SANDBOX_AGENT_RECOVERY_UNCLASSIFIED_ERROR {error}");
            "sdk_or_rpc_error_unclassified"
        }
    };
    inconclusive.push("agent_recovery_permission");
    eprintln!("X3_VAULT_SANDBOX_AGENT_RECOVERY_INCONCLUSIVE local_only=true observation={recovery_observation} permission_proven=false");

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
    println!("X3_VAULT_SANDBOX_OWNER_RECOVERY_SUCCESS local_only=true owner_only=true active_mandate_failure_observed=true active_mandate_reason_verified=false recovered_yoctonear=500000000000000000000000 pending_payout_protected_by_unit_tests=true owner_net_yoctonear={owner_net} vault_account_balance_decreased=true");
    ensure!(
        inconclusive.is_empty(),
        "X3_VAULT_SANDBOX_EVIDENCE_INCONCLUSIVE unresolved={inconclusive:?}; structured permission and policy rejection reasons are required"
    );
    println!("X3_VAULT_SANDBOX_COMPLETE agent_key_restricted=true funds_vault_custodied=true public_chain=false");
    Ok(())
}
