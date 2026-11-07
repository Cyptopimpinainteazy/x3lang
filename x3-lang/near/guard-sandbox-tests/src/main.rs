//! Exact production x3-guard WASM, deployed to a LOCAL NEAR sandbox node only.
//! Never contacts testnet, imports production wallet keys or moves real funds.
use anyhow::{Context, Result};
use near_account_id::AccountId as SandboxAccountId;
use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const MANDATE_EXPIRES_MS: u64 = 4_000_000_000_000;
const MANDATE_DEADLINE_MS: u64 = 3_900_000_000_000;

// Sandbox state patches can be asynchronously observable. Never assume an
// account's default signing key is visible just because patch_state succeeded.
async fn wait_for_local_signing_key(account: &AccountId, network: &NetworkConfig) -> Result<()> {
    let public_key: near_api::PublicKey = near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PUBLIC_KEY.parse()?;
    let mut last_error = String::new();
    for _ in 0..15 {
        match Account(account.clone())
            .access_key(public_key.clone())
            .fetch_from(network)
            .await
        {
            Ok(_) => return Ok(()),
            Err(e) => last_error = e.to_string(),
        }
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    anyhow::bail!("local Sandbox access key unavailable for {account}: {last_error}")
}

async fn create_funded_account(
    sandbox: &near_sandbox::Sandbox,
    network: &NetworkConfig,
    name: &str,
) -> Result<AccountId> {
    let sandbox_id: SandboxAccountId = name.parse()?;
    sandbox
        .create_account(sandbox_id)
        .initial_balance(NearToken::from_near(10))
        .send()
        .await?;
    let account_id: AccountId = name.parse()?;
    wait_for_local_signing_key(&account_id, network).await?;
    Ok(account_id)
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
    wait_for_local_signing_key(&contract_id, &network).await?;
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

    // Verify that changing the signed bytes or signature invalidates the
    // genuine 1Click manager's signature on the deployed contract.
    let signed_text = signed["signed_payload"].as_str().context("missing signed payload")?;
    let original_sig = signed["signature"].as_str().context("missing signature")?;
    let changed_text = format!("{signed_text} ");
    let mut changed_sig = original_sig.to_string();
    let final_char = changed_sig.pop().context("empty signature")?;
    changed_sig.push(if final_char == '1' { '2' } else { '1' });
    for (case, payload, signature) in [
        ("signed_bytes", changed_text.as_str(), original_sig),
        ("signature_bytes", signed_text, changed_sig.as_str()),
    ] {
        let checks: Vec<Value> = contract
            .call_function(
                "check_quote",
                json!({"owner": contract_id.clone(), "nonce": nonce,
                    "signed_payload": payload, "signature": signature}),
            )
            .read_only()
            .fetch_from(&network)
            .await?
            .data;
        anyhow::ensure!(
            !guard_ok(&checks, "quote_signature"),
            "modified {case} passed signature verification: {checks:?}"
        );
        let tx = contract
            .call_function(
                "accept_quote",
                json!({"nonce": nonce, "signed_payload": payload, "signature": signature}),
            )
            .transaction()
            .gas(NearGas::from_tgas(180))
            .deposit(NearToken::from_near(1))
            .with_signer(contract_id.clone(), signer.clone())
            .send_to(&network)
            .await?;
        anyhow::ensure!(!tx.is_success(), "modified {case} was accepted: {tx:?}");
        let current: Value = contract
            .call_function("get_intent", json!({"owner": contract_id.clone(), "nonce": nonce}))
            .read_only()
            .fetch_from(&network)
            .await?
            .data;
        anyhow::ensure!(
            current["quote"].is_null(),
            "modified {case} persisted authorization: {current}"
        );
    }
    println!("X3_SANDBOX_TAMPERED_QUOTE_REFUSED local_only=true signature_mismatch=true stored_quote_unchanged=true");

    // Mandate lifecycle against the same deployed WASM: an owner writes a
    // mandate, the agent registers an allowed intent, a forbidden recipient is
    // refused, and revocation stops the agent. Sandbox-local only.
    let owner = create_funded_account(&sandbox, &network, "owner.sandbox").await?;
    let agent = create_funded_account(&sandbox, &network, "agent.sandbox").await?;

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

    // Before the patch, an approval after mandate expiry was recorded.
    // Verify the patched deployed WASM refuses and preserves state in real Sandbox.
    let short_now_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    let short_expiry_ms = short_now_ms + 90_000;
    let mut expiring_terms = mandate_terms();
    expiring_terms["expires_at_ms"] = json!(short_expiry_ms);
    expiring_terms["approval_above"] = json!("20000000");
    expiring_terms["approvers"] = json!([owner.clone()]);
    expiring_terms["approval_threshold"] = json!(1);

    let renewed = contract
        .call_function(
            "create_mandate",
            json!({"agent": agent.clone(), "terms": expiring_terms}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(owner.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(renewed.is_success(), "sandbox renewed mandate failed: {renewed:?}");
    let mut expiring_policy = mandate_policy("x3-demo.near");
    expiring_policy["deadline_ms"] = json!(short_expiry_ms);
    let registered_for_approval = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": owner.clone(),
                "nonce": "sandbox_expired_approval",
                "policy": expiring_policy,
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(agent.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(
        registered_for_approval.is_success(),
        "sandbox registration before mandate expiry failed: {registered_for_approval:?}"
    );
    // Sandbox block time is not guaranteed to be one wall-clock second.
    // Observe the actual contract guard after each bounded time advancement;
    // a still-active mandate is a test failure, never proof of expiry.
    let mut observed_expired = false;
    for _ in 0..8 {
        sandbox.fast_forward(180).await?;
        let expired_checks =
            check_agent_intent(&contract, &network, &owner, &agent, mandate_policy("x3-demo.near")).await?;
        if !guard_ok(&expired_checks, "mandate_active") {
            observed_expired = true;
            break;
        }
    }
    anyhow::ensure!(
        observed_expired,
        "sandbox clock never expired mandate in bounded rounds"
    );
    let late_approval = contract
        .call_function(
            "approve_intent",
            json!({"owner": owner.clone(), "nonce": "sandbox_expired_approval"}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(owner.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(
        !late_approval.is_success(),
        "EXPIRED MANDATE VULNERABILITY: sandbox accepted approval: {late_approval:?}"
    );
    let after_late: Value = contract
        .call_function(
            "get_intent",
            json!({"owner": owner.clone(), "nonce": "sandbox_expired_approval"}),
        )
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    anyhow::ensure!(
        after_late["approvals"] == json!([]),
        "expired Sandbox mandate mutated approvals: {after_late}"
    );
    println!("X3_SANDBOX_EXPIRED_APPROVAL_REFUSED local_only=true block_timestamp_expired=true stored_approvals_unchanged=true");

    // Different ephemeral agent accounts submit the same owner+nonce
    // concurrently. NEAR must serialize them to exactly one stored intent.
    let race_owner = create_funded_account(&sandbox, &network, "race-owner.sandbox").await?;
    let race_agent_a = create_funded_account(&sandbox, &network, "race-agent-a.sandbox").await?;
    let race_agent_b = create_funded_account(&sandbox, &network, "race-agent-b.sandbox").await?;
    let mut race_terms = mandate_terms();
    race_terms["approval_above"] = json!("20000000");
    race_terms["approvers"] = json!([race_owner.clone()]);
    race_terms["approval_threshold"] = json!(1);
    for actor in [&race_agent_a, &race_agent_b] {
        let tx = contract
            .call_function("create_mandate", json!({"agent": actor, "terms": race_terms.clone()}))
            .transaction()
            .gas(NearGas::from_tgas(120))
            .deposit(NearToken::from_near(1))
            .with_signer(race_owner.clone(), signer.clone())
            .send_to(&network)
            .await?;
        anyhow::ensure!(tx.is_success(), "race mandate creation failed for {actor}: {tx:?}");
    }
    let race_nonce = "same_owner_same_nonce";
    let future_a = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": race_owner.clone(), "nonce": race_nonce, "policy": mandate_policy("x3-demo.near")
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(120))
        .deposit(NearToken::from_near(1))
        .with_signer(race_agent_a.clone(), signer.clone())
        .send_to(&network);
    let future_b = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": race_owner.clone(), "nonce": race_nonce, "policy": mandate_policy("x3-demo.near")
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(120))
        .deposit(NearToken::from_near(1))
        .with_signer(race_agent_b.clone(), signer.clone())
        .send_to(&network);
    let (outcome_a, outcome_b) = tokio::join!(future_a, future_b);
    let outcome_a = outcome_a.context("agent A submission did not produce a receipt")?;
    let outcome_b = outcome_b.context("agent B submission did not produce a receipt")?;
    let successes = u8::from(outcome_a.is_success()) + u8::from(outcome_b.is_success());
    anyhow::ensure!(
        successes == 1,
        "two concurrent agent registrations must yield one success: A={outcome_a:?} B={outcome_b:?}"
    );
    let winning_agent = if outcome_a.is_success() {
        &race_agent_a
    } else {
        &race_agent_b
    };
    let persisted: Value = contract
        .call_function("get_intent", json!({"owner": race_owner.clone(), "nonce": race_nonce}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    anyhow::ensure!(
        persisted["owner"] == json!(race_owner)
            && persisted["agent"] == json!(winning_agent)
            && persisted["approvals"] == json!([]),
        "racing agents changed owner/nonce identity unexpectedly: {persisted}"
    );
    println!("X3_SANDBOX_CONCURRENT_NONCE_REFUSED local_only=true competing_agents=2 success_count=1 owner_nonce_isolated=true");

    // Replay the successful owner's approval and try a non-approver. Neither
    // must increase the persistent approval count beyond one.
    let approved = contract
        .call_function(
            "approve_intent",
            json!({"owner": race_owner.clone(), "nonce": race_nonce}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(race_owner.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(approved.is_success(), "first legitimate approval failed: {approved:?}");
    let replayed = contract
        .call_function(
            "approve_intent",
            json!({"owner": race_owner.clone(), "nonce": race_nonce}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(race_owner.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(!replayed.is_success(), "duplicate approval accepted: {replayed:?}");
    let stranger = contract
        .call_function(
            "approve_intent",
            json!({"owner": race_owner.clone(), "nonce": race_nonce}),
        )
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(race_agent_b.clone(), signer.clone())
        .send_to(&network)
        .await?;
    anyhow::ensure!(!stranger.is_success(), "unauthorized approver accepted: {stranger:?}");
    let after_replay: Value = contract
        .call_function("get_intent", json!({"owner": race_owner.clone(), "nonce": race_nonce}))
        .read_only()
        .fetch_from(&network)
        .await?
        .data;
    anyhow::ensure!(
        after_replay["approvals"] == json!([race_owner]),
        "duplicate or unauthorized approval modified stored state: {after_replay}"
    );
    println!("X3_SANDBOX_APPROVAL_REPLAY_REFUSED local_only=true first_accepted=true duplicate_refused=true stranger_refused=true stored_count=1");

    multi_agent_stress(&sandbox, &network, &contract, signer.clone(), &race_owner).await?;
    Ok(())
}

// Bounded, deterministic 8-way / 4-round contention. Uses actual Sandbox
// transactions from distinct account IDs and checks chain state after each round.
// No public RPC, no wallet funds, no simulated transaction outcomes.
async fn multi_agent_stress(
    sandbox: &near_sandbox::Sandbox,
    network: &NetworkConfig,
    contract: &Contract,
    signer: Arc<Signer>,
    previous_owner: &AccountId,
) -> Result<()> {
    const AGENTS: usize = 8;
    const ROUNDS: usize = 4;
    let owner = create_funded_account(sandbox, network, "stress-owner.sandbox").await?;
    let mut agents = Vec::with_capacity(AGENTS);
    for i in 0..AGENTS {
        let actor = create_funded_account(sandbox, network, &format!("stress-agent-{i}.sandbox")).await?;
        let tx = contract
            .call_function(
                "create_mandate",
                json!({
                    "agent": actor.clone(), "terms": mandate_terms()
                }),
            )
            .transaction()
            .gas(NearGas::from_tgas(120))
            .deposit(NearToken::from_near(1))
            .with_signer(owner.clone(), signer.clone())
            .send_to(network)
            .await?;
        anyhow::ensure!(tx.is_success(), "stress mandate setup failed for {actor}: {tx:?}");
        agents.push(actor);
    }

    // Each round uses a fresh nonce but exactly the same owner. All eight
    // already-authorized agents are eligible; no two should win one nonce.
    for round in 0..ROUNDS {
        let nonce = format!("stress_nonce_{round:02}");
        let requests = agents.iter().map(|agent| {
            contract
                .call_function(
                    "agent_register_intent",
                    json!({
                        "owner": owner.clone(),
                        "nonce": nonce.clone(),
                        "policy": mandate_policy("x3-demo.near")
                    }),
                )
                .transaction()
                .gas(NearGas::from_tgas(120))
                .deposit(NearToken::from_near(1))
                .with_signer(agent.clone(), signer.clone())
                .send_to(network)
        });
        // All requests are submitted without waiting for each other's receipts.
        let outcomes = futures::future::join_all(requests).await;
        let outcomes: Vec<_> = outcomes
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .with_context(|| format!("stress round {round} lacked a transaction receipt"))?;
        let winners: Vec<usize> = outcomes
            .iter()
            .enumerate()
            .filter_map(|(i, outcome)| outcome.is_success().then_some(i))
            .collect();
        anyhow::ensure!(
            winners.len() == 1,
            "stress round {round}: expected one successful receipt of {AGENTS}, got {}: {outcomes:?}",
            winners.len()
        );
        let winning_agent = &agents[winners[0]];
        let stored: Value = contract
            .call_function("get_intent", json!({"owner": owner.clone(), "nonce": nonce.clone()}))
            .read_only()
            .fetch_from(network)
            .await?
            .data;
        anyhow::ensure!(
            stored["owner"] == json!(owner)
                && stored["nonce"] == json!(nonce)
                && stored["agent"] == json!(winning_agent)
                && stored["approvals"] == json!([])
                && stored["quote"].is_null(),
            "stress round {round}: owner/nonce/agent state mismatch: {stored}"
        );
        // Both the winning signer and a losing signer must now be refused.
        // A failed transaction must preserve every stored byte of the intent.
        for attacker in [winning_agent, &agents[(winners[0] + 1) % AGENTS]] {
            let replay = contract
                .call_function(
                    "agent_register_intent",
                    json!({
                        "owner": owner.clone(), "nonce": nonce.clone(),
                        "policy": mandate_policy("x3-demo.near")
                    }),
                )
                .transaction()
                .gas(NearGas::from_tgas(120))
                .deposit(NearToken::from_near(1))
                .with_signer(attacker.clone(), signer.clone())
                .send_to(network)
                .await?;
            anyhow::ensure!(
                !replay.is_success(),
                "stress round {round}: agent {attacker} replayed existing nonce: {replay:?}"
            );
        }
        let unchanged: Value = contract
            .call_function("get_intent", json!({"owner": owner.clone(), "nonce": nonce.clone()}))
            .read_only()
            .fetch_from(network)
            .await?
            .data;
        anyhow::ensure!(
            unchanged == stored,
            "stress round {round}: rejected replay mutated persistent intent: {unchanged}"
        );
    }
    println!(
        "X3_SANDBOX_EIGHT_AGENT_RACES local_only=true agents={AGENTS} rounds={ROUNDS} attempts={} winners={ROUNDS} replay_rejections={} state_unchanged=true",
        AGENTS * ROUNDS,
        ROUNDS * 2
    );

    // A nonce is unique PER OWNER, not globally. A second owner must be able
    // to reuse a nonce previously consumed by a different owner.
    let nonce = "same_owner_same_nonce";
    let previous: Value = contract
        .call_function("get_intent", json!({"owner": previous_owner, "nonce": nonce}))
        .read_only()
        .fetch_from(network)
        .await?
        .data;
    anyhow::ensure!(
        previous["owner"] == json!(previous_owner) && previous["nonce"] == json!(nonce),
        "prior owner's distinct nonce record was not present"
    );
    let registered = contract
        .call_function(
            "agent_register_intent",
            json!({
                "owner": owner.clone(), "nonce": nonce,
                "policy": mandate_policy("x3-demo.near")
            }),
        )
        .transaction()
        .gas(NearGas::from_tgas(120))
        .deposit(NearToken::from_near(1))
        .with_signer(agents[0].clone(), signer.clone())
        .send_to(network)
        .await?;
    anyhow::ensure!(
        registered.is_success(),
        "a separate owner was blocked from its own nonce: {registered:?}"
    );
    let first_after: Value = contract
        .call_function("get_intent", json!({"owner": previous_owner, "nonce": nonce}))
        .read_only()
        .fetch_from(network)
        .await?
        .data;
    let second: Value = contract
        .call_function("get_intent", json!({"owner": owner.clone(), "nonce": nonce}))
        .read_only()
        .fetch_from(network)
        .await?
        .data;
    anyhow::ensure!(
        previous == first_after
            && second["owner"] == json!(owner)
            && second["agent"] == json!(agents[0])
            && second["quote"].is_null(),
        "cross-owner nonce reuse corrupted identity or state: {second}"
    );
    println!("X3_SANDBOX_CROSS_OWNER_NONCE_ISOLATED local_only=true owners=2 same_nonce=true separate_state=true");
    Ok(())
}
