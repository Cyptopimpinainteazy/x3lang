//! Generated .x3 initialization and same-block local RPC configuration observations.
//! The local node is trusted; this is not an authenticated public-chain proof.
use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine as _};
use near_api::{types::CryptoHash, NetworkConfig};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::io::Write as _;
use std::net::IpAddr;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_BYTES: usize = 131_072;
static OBSERVATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut result = String::with_capacity(64);
    for byte in CryptoHash::hash(bytes).0 {
        write!(&mut result, "{byte:02x}").expect("String writing is infallible");
    }
    result
}

fn helper(mode: &str, input: &Value) -> Result<Value> {
    let script =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../scripts/hackathon/native_vault_binding.py");
    let raw = serde_json::to_vec(input)?;
    ensure!(raw.len() <= MAX_BYTES, "binding input is oversized");
    let mut child = Command::new("python3")
        .arg("-I")
        .arg(script)
        .arg(mode)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot start mandatory .x3 binding compiler/verifier")?;
    let written = child.stdin.take().context("binding stdin missing")?.write_all(&raw);
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "binding helper failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    ensure!(output.stdout.len() <= MAX_BYTES, "binding output is oversized");
    Ok(serde_json::from_slice(&output.stdout)?)
}

pub fn compile(source: &[u8], wasm: &[u8], vault_account: &str, compiled_at_ms: u64) -> Result<Value> {
    ensure!(!source.is_empty() && source.len() <= 65_536, "invalid .x3 source size");
    helper(
        "compile",
        &json!({
            "source_base64": BASE64_STANDARD.encode(source),
            "context": {"network":"sandbox", "vault_account":vault_account,
                "wasm_sha256":sha256_hex(wasm), "policy_version":1, "compiled_at_ms":compiled_at_ms}
        }),
    )
}

pub fn constructor(binding: &Value) -> Result<Value> {
    let value = binding.get("constructor").context("compiler returned no constructor")?;
    ensure!(value.is_object(), "compiler constructor is not an object");
    Ok(value.clone())
}

pub fn source_with_lower_cap(source: &[u8]) -> Result<Vec<u8>> {
    let source = std::str::from_utf8(source)?;
    let old = "per_payment 200000000000000000000000";
    ensure!(
        source.matches(old).count() == 1,
        "expected one reviewed demonstration cap clause"
    );
    Ok(source
        .replacen(old, "per_payment 50000000000000000000000", 1)
        .into_bytes())
}

async fn rpc(network: &NetworkConfig, method: &str, params: Value) -> Result<Value> {
    let url = network
        .rpc_endpoints
        .first()
        .context("missing Sandbox RPC")?
        .url
        .clone();
    ensure!(
        url.scheme() == "http"
            && url
                .host_str()
                .and_then(|host| host.parse::<IpAddr>().ok())
                .is_some_and(|ip| ip.is_loopback())
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "policy readback requires a literal loopback Sandbox endpoint"
    );
    let sequence = OBSERVATION_SEQUENCE
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| anyhow::anyhow!("observation sequence exhausted"))?;
    let id = format!("x3-policy-observation-{sequence}");
    let mut response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(30))
        .build()?
        .post(url)
        .json(&json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params}))
        .send()
        .await?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "unexpected policy-readback HTTP status"
    );
    let mut raw = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= MAX_BYTES - raw.len(),
            "oversized policy-readback RPC response"
        );
        raw.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&raw)?;
    ensure!(
        value["id"] == id && value["jsonrpc"] == "2.0" && value.get("error").is_none(),
        "policy observation RPC failed: {value}"
    );
    let result = value.get("result").context("policy observation lacks a result")?;
    ensure!(result.get("error").is_none(), "policy view execution failed: {result}");
    Ok(result.clone())
}

/// Use the same bounded loopback RPC client for time advancement. The SDK's
/// blocking send precedes its polling timeout, so it cannot bound this call.
pub async fn advance_for_expiry(network: &NetworkConfig) -> Result<()> {
    println!("X3_VAULT_SANDBOX_ADVANCE_BEGIN local_only=true requested_blocks=50000 rpc_timeout_seconds=30");
    rpc(network, "sandbox_fast_forward", json!({"delta_height":50_000})).await?;
    println!("X3_VAULT_SANDBOX_ADVANCE_ACK local_only=true clock_not_yet_verified=true");
    Ok(())
}

pub async fn block(network: &NetworkConfig) -> Result<Value> {
    let result = rpc(network, "block", json!({"finality":"final"})).await?;
    let header = result.get("header").context("missing observed block header")?.clone();
    ensure!(
        header["hash"].is_string() && header["height"].as_u64().is_some(),
        "malformed block identity"
    );
    timestamp_ms(&header)?;
    Ok(header)
}

pub fn timestamp_ms(header: &Value) -> Result<u64> {
    let timestamp = header["timestamp_nanosec"]
        .as_str()
        .context("missing block nanosecond timestamp")?;
    Ok(timestamp.parse::<u64>()? / 1_000_000)
}

pub async fn observe(network: &NetworkConfig, binding: &Value) -> Result<Value> {
    let vault = binding["context"]["vault_account"]
        .as_str()
        .context("binding vault missing")?;
    let header = block(network).await?;
    let hash = &header["hash"];
    let account = rpc(
        network,
        "query",
        json!({"request_type":"view_account", "account_id":vault,
        "block_id":hash}),
    )
    .await?;
    let config = rpc(
        network,
        "query",
        json!({"request_type":"call_function", "account_id":vault,
        "block_id":hash, "method_name":"configuration", "args_base64":"e30="}),
    )
    .await?;
    for observed in [&account, &config] {
        ensure!(
            observed["block_hash"] == *hash && observed["block_height"] == header["height"],
            "readback did not use the pinned common block"
        );
    }
    let bytes: Vec<u8> = serde_json::from_value(config["result"].clone()).context("configuration view lacks bytes")?;
    let observed = json!({
        "vault_account":vault, "block_hash":hash, "block_height":header["height"],
        "code_hash":account["code_hash"], "configuration":serde_json::from_slice::<Value>(&bytes)?
    });
    let report = helper(
        "verify",
        &json!({"binding":binding,"observation":observed,
        "now_ms":timestamp_ms(&header)?}),
    )?;
    ensure!(
        report["configuration_matches"] == true
            && report["code_hash_matches"] == true
            && report["cryptographic_proof"] == false
            && report["public_chain"] == false,
        "invalid configuration verification report"
    );
    println!(
        "X3_VAULT_SANDBOX_GENERATED_POLICY {}",
        serde_json::to_string(&json!({
            "binding":binding,"observation":observed,"now_ms":timestamp_ms(&header)?,"report":report
        }))?
    );
    Ok(observed)
}

#[cfg(test)]
mod tests {
    use super::*;
    const SOURCE: &[u8] = include_bytes!("../../../examples/near_native_vault_supplier.x3");

    #[test]
    fn source_change_only_changes_cap_in_generated_constructor() {
        let base = compile(
            SOURCE,
            b"\0asm fixture not deployed",
            "vault.testnet",
            1_900_000_000_000,
        )
        .unwrap();
        let narrow = compile(
            &source_with_lower_cap(SOURCE).unwrap(),
            b"\0asm fixture not deployed",
            "tight.testnet",
            1_900_000_000_000,
        )
        .unwrap();
        let mut expected = constructor(&base).unwrap();
        expected["cap"] = json!("50000000000000000000000");
        assert_eq!(constructor(&narrow).unwrap(), expected);
        assert_ne!(base["manifest"]["source_sha256"], narrow["manifest"]["source_sha256"]);
    }

    #[test]
    fn compiler_failure_and_wrong_source_clause_fail_closed() {
        assert!(compile(b"garbage", b"\0asm", "vault.testnet", 1).is_err());
        assert!(source_with_lower_cap(b"per_payment 1").is_err());
        assert!(source_with_lower_cap(&[SOURCE, SOURCE].concat()).is_err());
    }

    #[test]
    fn readback_helper_rejects_wrong_code_before_any_success_claim() {
        let binding = compile(SOURCE, b"fixture", "vault.testnet", 1_900_000_000_000).unwrap();
        let observed = json!({"vault_account":"vault.testnet", "block_hash":"11111111111111111111111111111111",
            "block_height":10, "code_hash":"wrong", "configuration":constructor(&binding).unwrap()});
        assert!(helper(
            "verify",
            &json!({"binding":binding, "observation":observed,
            "now_ms":1_900_000_000_001u64})
        )
        .is_err());
    }
}
