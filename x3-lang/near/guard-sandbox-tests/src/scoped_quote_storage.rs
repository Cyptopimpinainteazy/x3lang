//! Real Sandbox rollback with SYNTHETIC quote signatures and unchanged production WASM.
//! Only this ephemeral node's manager-key state is patched. No official 1Click
//! acceptance, settlement or public-chain authorization is established here.
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, SecondsFormat, Utc};
use ed25519_dalek::{Signer as _, SigningKey};
use near_api::{Account, AccountId, Chain, Contract, NearGas, NearToken, NetworkConfig, Signer};
use near_sandbox::{sandbox::patch::StateRecord, FetchData, Sandbox};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::Arc;

const PRODUCTION_MANAGER: &str = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc";
const NONCE: &str = "quote_storage_retry";
const REPLAY_NONCE: &str = "quote_storage_replay";
const DEPOSIT: &str = "synthetic-deposit.sandbox";

async fn view(guard: &Contract, network: &NetworkConfig, method: &str, args: Value) -> Result<Value> {
    Ok(guard
        .call_function(method, args)
        .read_only()
        .fetch_from(network)
        .await?
        .data)
}

/// The sole internal fixture seam: replace exactly one known public key in the
/// existing root state. Refuse unfamiliar layouts instead of guessing an offset.
/// PatchState is attached to the newly started Sandbox, never an operator URL.
async fn patch_local_manager(
    sandbox: &Sandbox,
    network: &NetworkConfig,
    owner: &AccountId,
    from: &str,
    to: &str,
) -> Result<()> {
    let guard = Account(owner.clone()).as_contract();
    ensure!(view(&guard, network, "get_manager_public_key", json!({})).await? == from);
    let before = bs58::decode(from.strip_prefix("ed25519:").context("manager prefix")?).into_vec()?;
    let after = bs58::decode(to.strip_prefix("ed25519:").context("manager prefix")?).into_vec()?;
    ensure!(before.len() == 32 && after.len() == 32, "fixture public-key size");
    let fetched = sandbox
        .patch_state(owner.as_str().parse()?)
        .fetch_from(&sandbox.rpc_addr, FetchData::NONE.storage())
        .await?;
    let roots: Vec<_> = fetched
        .state
        .iter()
        .filter_map(|row| match row {
            StateRecord::Data {
                data_key_base64,
                value_base64,
                ..
            } if data_key_base64 == &STANDARD.encode(b"STATE") => Some(value_base64),
            _ => None,
        })
        .collect();
    ensure!(roots.len() == 1, "expected one Sandbox contract root state");
    let mut state = STANDARD.decode(roots[0])?;
    let offsets: Vec<_> = state
        .windows(32)
        .enumerate()
        .filter(|(_, b)| *b == before)
        .map(|(i, _)| i)
        .collect();
    ensure!(offsets.len() == 1, "manager public key is not unique in root state");
    state[offsets[0]..offsets[0] + 32].copy_from_slice(&after);
    sandbox
        .patch_state(owner.as_str().parse()?)
        .storage(STANDARD.encode(b"STATE"), STANDARD.encode(state))
        .send()
        .await?;
    // Sandbox state patches may become visible asynchronously.
    for _ in 0..20 {
        if view(&guard, network, "get_manager_public_key", json!({})).await? == to {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    anyhow::bail!("Sandbox synthetic manager fixture did not become visible")
}

fn signed_quote(now_ms: u64, key: &SigningKey) -> Result<(String, String, String)> {
    let utc = |ms| -> Result<String> {
        Ok(DateTime::<Utc>::from_timestamp_millis(ms)
            .context("fixture timestamp")?
            .to_rfc3339_opts(SecondsFormat::Millis, true))
    };
    let payload = json!({
        "dry": false, "swapType": "EXACT_INPUT", "depositType": "ORIGIN_CHAIN",
        "refundType": "ORIGIN_CHAIN", "recipientType": "DESTINATION_CHAIN",
        "slippageTolerance": 50, "originAsset": "nep141:usdc.near",
        "destinationAsset": "nep141:usdc.near", "amount": "25000000",
        "refundTo": "refund.sandbox", "recipient": "x3-demo.near",
        "amountIn": "25000000", "amountOut": "25000000", "minAmountOut": "24900000",
        "amountInUsd": "25.0", "amountOutUsd": "25.0", "refundFee": "0",
        "depositAddress": DEPOSIT, "deadline": utc((now_ms + 600_000).try_into()?)?,
        "timestamp": utc(now_ms.try_into()?)?, "timeEstimate": 1
    })
    .to_string();
    let hash = bs58::encode(Sha256::digest(payload.as_bytes())).into_string();
    let signature = format!(
        "ed25519:{}",
        bs58::encode(key.sign(hash.as_bytes()).to_bytes()).into_string()
    );
    Ok((payload, signature, hash))
}

/// Compare public contract state across a real failed receipt. Account balance
/// is deliberately excluded because executing a refused transaction costs gas.
async fn snapshot(
    guard: &Contract,
    network: &NetworkConfig,
    owner: &AccountId,
    agent: &AccountId,
    hash: &str,
) -> Result<Value> {
    Ok(json!({
        "intent": view(guard, network, "get_intent", json!({"owner": owner, "nonce": NONCE})).await?,
        "replay_intent": view(guard, network, "get_intent", json!({"owner": owner, "nonce": REPLAY_NONCE})).await?,
        "mandate": view(guard, network, "get_mandate", json!({"owner": owner, "agent": agent})).await?,
        "credit": view(guard, network, "get_agent_storage_balance", json!({"owner": owner, "agent": agent})).await?,
        "quote_consumer": view(guard, network, "get_quote_consumer", json!({"quote_hash": hash})).await?,
        "deposit_consumer": view(guard, network, "get_deposit_consumer", json!({"deposit_address": DEPOSIT})).await?,
        "storage_usage": Account(owner.clone()).view().fetch_from(network).await?.data.storage_usage
    }))
}

#[allow(clippy::too_many_arguments)]
pub async fn verify(
    sandbox: &Sandbox,
    network: &NetworkConfig,
    owner: &AccountId,
    agent: &AccountId,
    administrator: Arc<Signer>,
    scoped: Arc<Signer>,
    policy: Value,
) -> Result<()> {
    let guard = Account(owner.clone()).as_contract();
    let code_before = Account(owner.clone())
        .view()
        .fetch_from(network)
        .await?
        .data
        .contract_state;
    // Fixed PUBLIC TEST-ONLY seed; never a wallet or production manager secret.
    let synthetic_key = SigningKey::from_bytes(&[42u8; 32]);
    let synthetic_public = format!(
        "ed25519:{}",
        bs58::encode(synthetic_key.verifying_key().to_bytes()).into_string()
    );
    let now_ms = Chain::block()
        .fetch_from(network)
        .await?
        .header
        .timestamp_nanosec
        .parse::<u64>()?
        / 1_000_000;
    let (payload, signature, hash) = signed_quote(now_ms, &synthetic_key)?;
    for nonce in [NONCE, REPLAY_NONCE] {
        guard
            .call_function(
                "agent_register_intent",
                json!({"owner": owner, "nonce": nonce, "policy": policy}),
            )
            .transaction()
            .gas(NearGas::from_tgas(100))
            .deposit(NearToken::from_millinear(100))
            .with_signer(agent.clone(), administrator.clone())
            .send_to(network)
            .await?
            .assert_success();
    }
    let quote_args = json!({"owner": owner, "nonce": NONCE, "signed_payload": payload, "signature": signature});
    // Negative control proves the production manager does NOT accept our fixture.
    let initial = snapshot(&guard, network, owner, agent, &hash).await?;
    ensure!(initial["credit"] == "0" && initial["mandate"]["spends"] == json!([]));
    ensure!(
        initial["intent"]["quote"].is_null()
            && initial["quote_consumer"].is_null()
            && initial["deposit_consumer"].is_null()
    );
    let invalid = guard
        .call_function("agent_accept_quote", quote_args.clone())
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(network)
        .await?;
    ensure!(
        !invalid.is_success() && format!("{invalid:?}").contains("X3_NEAR_INVALID_SIGNATURE"),
        "production manager accepted synthetic quote"
    );
    ensure!(
        snapshot(&guard, network, owner, agent, &hash).await? == initial,
        "invalid signature changed contract state"
    );

    patch_local_manager(sandbox, network, owner, PRODUCTION_MANAGER, &synthetic_public).await?;
    ensure!(
        Account(owner.clone())
            .view()
            .fetch_from(network)
            .await?
            .data
            .contract_state
            == code_before,
        "fixture changed production WASM"
    );
    let checks = view(&guard, network, "check_agent_quote", quote_args.clone()).await?;
    let checks = checks.as_array().context("quote checks must be an array")?;
    ensure!(
        !checks.is_empty() && checks.iter().all(|c| c["ok"] == true),
        "synthetic quote failed preconditions: {checks:?}"
    );
    ensure!(
        checks
            .iter()
            .any(|c| c["guard"] == "quote_signature" && c["ok"] == true),
        "missing signature positive control"
    );

    let before = snapshot(&guard, network, owner, agent, &hash).await?;
    let exhausted = guard
        .call_function("agent_accept_quote", quote_args.clone())
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(network)
        .await?;
    ensure!(
        !exhausted.is_success() && format!("{exhausted:?}").contains("X3_STORAGE_BUDGET_EXHAUSTED"),
        "expected storage refusal: {exhausted:?}"
    );
    ensure!(
        snapshot(&guard, network, owner, agent, &hash).await? == before,
        "failed acceptance retained intent, replay, spend or storage changes"
    );
    println!("X3_SANDBOX_QUOTE_STORAGE_ROLLBACK_OK local_only=true synthetic_quote_signature=true unchanged_production_wasm=true quote_and_deposit_ledgers_unchanged=true rolling_spend_unchanged=true credit_unchanged=true");

    guard
        .call_function("fund_agent_storage", json!({"agent": agent}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_millinear(100))
        .with_signer(owner.clone(), administrator)
        .send_to(network)
        .await?
        .assert_success();
    let funded = snapshot(&guard, network, owner, agent, &hash).await?;
    guard
        .call_function("agent_accept_quote", quote_args)
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped.clone())
        .send_to(network)
        .await?
        .assert_success();
    let accepted = snapshot(&guard, network, owner, agent, &hash).await?;
    let consumer = format!("{owner}/{NONCE}");
    ensure!(
        accepted["intent"]["quote"]["facts"]["quote_hash"] == hash,
        "retry did not store original quote"
    );
    ensure!(
        accepted["quote_consumer"] == consumer && accepted["deposit_consumer"] == consumer,
        "retry did not consume both replay ledgers"
    );
    let spends = accepted["mandate"]["spends"].as_array().context("mandate spends")?;
    ensure!(
        spends.len() == 1 && spends[0]["amount"] == "25000000" && spends[0]["nonce"] == NONCE,
        "retry was not counted exactly once"
    );
    let credit = |value: &Value| -> Result<u128> { Ok(value["credit"].as_str().context("credit")?.parse()?) };
    ensure!(
        credit(&accepted)? < credit(&funded)?,
        "retry failed to charge storage credit"
    );
    ensure!(
        accepted["replay_intent"]["quote"].is_null(),
        "retry changed unrelated intent"
    );
    let replay = guard
        .call_function(
            "agent_accept_quote",
            json!({"owner": owner, "nonce": REPLAY_NONCE, "signed_payload": payload, "signature": signature}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(agent.clone(), scoped)
        .send_to(network)
        .await?;
    ensure!(
        !replay.is_success() && format!("{replay:?}").contains("X3_NEAR_QUOTE_REPLAY"),
        "retry allowed cross-nonce replay: {replay:?}"
    );
    ensure!(
        snapshot(&guard, network, owner, agent, &hash).await? == accepted,
        "replay refusal changed state or charged storage"
    );
    patch_local_manager(sandbox, network, owner, &synthetic_public, PRODUCTION_MANAGER).await?;
    ensure!(
        Account(owner.clone())
            .view()
            .fetch_from(network)
            .await?
            .data
            .contract_state
            == code_before,
        "production WASM changed"
    );
    println!("X3_SANDBOX_QUOTE_STORAGE_RETRY_OK local_only=true synthetic_quote_signature=true original_quote_accepted_once=true cross_nonce_replay_refused=true production_manager_restored=true official_oneclick_acceptance_proven=false settlement_proven=false");
    Ok(())
}
