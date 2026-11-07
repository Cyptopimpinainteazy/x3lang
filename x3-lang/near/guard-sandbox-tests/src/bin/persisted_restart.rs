//! Real persisted-home NEAR Sandbox restart regression; no public chain use.
//! The only node process controlled here is spawned by this executable.
use anyhow::{bail, ensure, Context, Result};
use near_api::{Account, AccountId, Contract, NearGas, NearToken, NetworkConfig, Signer};
use serde_json::{json, Value};
use std::{
    env,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::{
    process::{Child, Command},
    time::sleep,
};

const RPC: &str = "http://127.0.0.1:38273";
const PINNED_MANAGER: &str = "ed25519:reYaWhvwu8Jzo3WUM3zhn6VrhuMEF4eADL17qtRVifc";

fn policy() -> Value {
    json!({
        "program_sha256": "a".repeat(64),
        "origin_asset": "nep141:usdc.near",
        "destination_asset": "nep141:usdc.near",
        "amount": "25000000", "min_receive": "24800000",
        "slippage_bps": 50, "fee_cap_bps": 100,
        "recipient": "x3-demo.near", "refund_to": "refund.sandbox",
        "deadline_ms": 3_900_000_000_000u64
    })
}

async fn await_node(network: &NetworkConfig, owner: &AccountId) -> Result<String> {
    let mut last = String::new();
    for _ in 0..100 {
        match Account(owner.clone()).view().fetch_from(network).await {
            Ok(view) => return Ok(format!("{:?}", view.data.contract_state)),
            Err(err) => last = err.to_string(),
        }
        sleep(Duration::from_millis(250)).await;
    }
    bail!("isolated node not ready: {last}")
}

fn start_node(bin: &Path, home: &Path, phase: &str) -> Result<Child> {
    let log = File::create(home.parent().unwrap().join(format!("{phase}.log")))?;
    Ok(Command::new(bin)
        .arg("--home")
        .arg(home)
        .arg("run")
        .arg("--rpc-addr")
        .arg("127.0.0.1:38273")
        .arg("--network-addr")
        .arg("127.0.0.1:38274")
        .stdout(Stdio::from(log.try_clone()?))
        .stderr(Stdio::from(log))
        .kill_on_drop(true)
        .spawn()
        .context("could not start owned sandbox child")?)
}

async fn view(contract: &Contract, net: &NetworkConfig, f: &str, args: Value) -> Result<Value> {
    Ok(contract.call_function(f, args).read_only().fetch_from(net).await?.data)
}

async fn used(contract: &Contract, net: &NetworkConfig, owner: &AccountId, nonce: &str) -> Result<bool> {
    view(contract, net, "is_nonce_used", json!({"owner": owner, "nonce":nonce}))
        .await?
        .as_bool()
        .context("nonce use query not boolean")
}

async fn record(contract: &Contract, net: &NetworkConfig, owner: &AccountId, nonce: &str) -> Result<Value> {
    view(contract, net, "get_intent", json!({"owner": owner, "nonce":nonce})).await
}

async fn register(
    contract: &Contract,
    net: &NetworkConfig,
    owner: &AccountId,
    signer: Arc<Signer>,
    nonce: &str,
) -> Result<bool> {
    let outcome = contract
        .call_function("register_intent", json!({"nonce":nonce, "policy":policy()}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .deposit(NearToken::from_near(1))
        .with_signer(owner.clone(), signer)
        .send_to(net)
        .await?;
    Ok(outcome.is_success())
}

async fn markers(contract: &Contract, net: &NetworkConfig) -> Result<()> {
    for method in [
        "is_fresh_quote_ledger_initialized",
        "is_fresh_deposit_ledger_initialized",
    ] {
        ensure!(
            view(contract, net, method, json!({})).await? == json!(true),
            "missing persistent replay marker {method}"
        );
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut args = env::args_os().skip(1);
    let wasm = PathBuf::from(args.next().context("missing WASM path")?);
    let home = PathBuf::from(args.next().context("missing isolated home")?);
    ensure!(args.next().is_none(), "unexpected argument");
    ensure!(
        home == Path::new("/x3-persist/guard-node-home"),
        "refusing unsafe node directory"
    );
    ensure!(!home.exists(), "refusing to reuse an existing node home");
    let bin = PathBuf::from(env::var("NEAR_SANDBOX_BIN_PATH")?);
    ensure!(
        bin == Path::new("/opt/x3-near-sandbox"),
        "refusing unpinned node binary"
    );
    let wasm_bytes = fs::read(&wasm)?;
    ensure!(wasm_bytes.starts_with(b"\0asm"), "not WASM");
    fs::create_dir_all(&home)?;
    let init = Command::new(&bin)
        .arg("--home")
        .arg(&home)
        .args(["init", "--fast"])
        .output()
        .await?;
    ensure!(
        init.status.success(),
        "sandbox init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    near_sandbox::config::set_sandbox_genesis(&home)?;

    let owner: AccountId = "sandbox".parse()?;
    let signer = Signer::from_secret_key(near_sandbox::config::DEFAULT_GENESIS_ACCOUNT_PRIVATE_KEY.parse()?)?;
    let network = NetworkConfig::from_rpc_url("x3-local-restart", RPC.parse()?);
    let contract = Account(owner.clone()).as_contract();

    let mut node = start_node(&bin, &home, "initial")?;
    await_node(&network, &owner).await?;
    Contract::deploy(owner.clone())
        .use_code(wasm_bytes)
        .without_init_call()
        .with_signer(signer.clone())
        .send_to(&network)
        .await?
        .assert_success();
    contract
        .call_function("new", json!({}))
        .transaction()
        .gas(NearGas::from_tgas(100))
        .with_signer(owner.clone(), signer.clone())
        .send_to(&network)
        .await?
        .assert_success();
    ensure!(
        view(&contract, &network, "get_manager_public_key", json!({})).await? == json!(PINNED_MANAGER),
        "manager public key changed"
    );
    markers(&contract, &network).await?;
    ensure!(
        register(&contract, &network, &owner, signer.clone(), "persisted_a").await?,
        "initial intent registration failed"
    );
    let first = record(&contract, &network, &owner, "persisted_a").await?;
    ensure!(
        first["owner"] == json!(owner)
            && first["nonce"] == "persisted_a"
            && first["quote"].is_null()
            && first["approvals"] == json!([]),
        "invalid initial record: {first}"
    );
    let code_hash = await_node(&network, &owner).await?;
    println!("X3_RESTART_INITIAL_WRITES_CONFIRMED nonce=persisted_a markers=2");

    // Terminate only the directly spawned sandbox process, simulating a crash.
    node.kill().await?;
    node.wait().await?;
    let mut node = start_node(&bin, &home, "restart1")?;
    ensure!(
        await_node(&network, &owner).await? == code_hash,
        "contract code identity changed after restart"
    );
    markers(&contract, &network).await?;
    ensure!(
        used(&contract, &network, &owner, "persisted_a").await?,
        "confirmed nonce disappeared on restart"
    );
    ensure!(
        record(&contract, &network, &owner, "persisted_a").await? == first,
        "Guard record changed after restart"
    );
    ensure!(
        !register(&contract, &network, &owner, signer.clone(), "persisted_a").await?,
        "confirmed nonce replay accepted"
    );
    println!("X3_RESTART_PERSISTENT_STATE_PASS code_hash=true markers=2 duplicate_refused=true");

    // Try a local tx but deliberately kill its sandbox before waiting for
    // final observation. Reconcile state after restart; do not guess outcome.
    let pending_owner = owner.clone();
    let pending_signer = signer.clone();
    let pending_net = NetworkConfig::from_rpc_url("x3-local-restart", RPC.parse()?);
    let handle = tokio::spawn(async move {
        let pending_contract = Account(pending_owner.clone()).as_contract();
        register(
            &pending_contract,
            &pending_net,
            &pending_owner,
            pending_signer,
            "interrupted_b",
        )
        .await
    });
    sleep(Duration::from_millis(100)).await;
    let in_flight = !handle.is_finished();
    node.kill().await?;
    node.wait().await?;
    handle.abort();

    let mut node = start_node(&bin, &home, "restart2")?;
    ensure!(
        await_node(&network, &owner).await? == code_hash,
        "code hash changed across second restart"
    );
    markers(&contract, &network).await?;
    ensure!(
        used(&contract, &network, &owner, "persisted_a").await?,
        "initial state lost on second restart"
    );
    let committed = used(&contract, &network, &owner, "interrupted_b").await?;
    if !committed {
        ensure!(
            register(&contract, &network, &owner, signer.clone(), "interrupted_b").await?,
            "unknown transaction could not be recovered safely"
        );
    }
    ensure!(
        used(&contract, &network, &owner, "interrupted_b").await?,
        "recovered nonce is not used"
    );
    let recovered = record(&contract, &network, &owner, "interrupted_b").await?;
    ensure!(
        recovered["owner"] == json!(owner) && recovered["nonce"] == "interrupted_b",
        "recovered record identity is invalid: {recovered}"
    );
    ensure!(
        !register(&contract, &network, &owner, signer.clone(), "interrupted_b").await?,
        "reconciled nonce replay was accepted"
    );
    println!("X3_RESTART_UNKNOWN_TX_RECONCILED in_flight={in_flight} committed_before_retry={committed} exactly_one_record=true");
    println!("X3_RESTART_LOCAL_NODE_PASS restarts=2 public_chain=false test_only_signer=true");
    node.kill().await?;
    node.wait().await?;
    Ok(())
}
