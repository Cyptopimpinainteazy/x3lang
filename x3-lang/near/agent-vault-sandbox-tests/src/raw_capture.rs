//! Local Sandbox wire capture. Byte correlation is not node or signature authentication.
use anyhow::{ensure, Context, Result};
use base64::{prelude::BASE64_STANDARD, Engine as _};
use near_api::{types::transaction::SignedTransaction, NetworkConfig};
use serde_json::{json, Value};
use std::fmt::Write as _;
use std::io::Write as _;
use std::net::IpAddr;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

const MAX_RESPONSE_BYTES: usize = 1_048_576;
const MAX_SIGNED_BYTES: usize = 131_072;
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn prepare_request(signed_bytes: &[u8], expectation: &mut Value) -> Result<Vec<u8>> {
    ensure!(
        !signed_bytes.is_empty() && signed_bytes.len() <= MAX_SIGNED_BYTES,
        "invalid signed transaction size"
    );
    let mut digest = String::with_capacity(64);
    for byte in near_api::types::CryptoHash::hash(signed_bytes).0 {
        write!(&mut digest, "{byte:02x}")?;
    }
    let sequence = REQUEST_SEQUENCE
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .map_err(|_| anyhow::anyhow!("raw request sequence exhausted"))?;
    let request_id = format!("x3-vault-{digest}-{sequence}");
    expectation
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("permission expectation must be an object"))?
        .insert("request_id".into(), json!(request_id));
    Ok(serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "send_tx",
        "params": {"signed_tx_base64": BASE64_STANDARD.encode(signed_bytes), "wait_until": "FINAL"}
    }))?)
}

fn classify_original_bytes(request: &[u8], response: &[u8], expectation: &Value) -> Result<Value> {
    ensure!(response.len() <= MAX_RESPONSE_BYTES, "oversized raw RPC response");
    let input = serde_json::to_vec(&json!({
        "request_base64": BASE64_STANDARD.encode(request),
        "response_base64": BASE64_STANDARD.encode(response),
        "expectation": expectation
    }))?;
    let classifier = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../scripts/hackathon/near_permission_refusal.py");
    let mut child = Command::new("python3")
        .arg("-I")
        .arg(classifier)
        .arg("--capture-stdin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("cannot start mandatory raw-byte classifier")?;
    let write_result = (|| -> Result<()> {
        let mut stdin = child.stdin.take().context("classifier stdin missing")?;
        stdin.write_all(&input)?;
        Ok(())
    })();
    if let Err(error) = write_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.context("raw-byte classifier input failed"));
    }
    let output = child.wait_with_output()?;
    ensure!(
        output.status.success(),
        "raw-byte refusal classification failed: {}",
        String::from_utf8_lossy(&output.stderr).chars().take(4096).collect::<String>()
    );
    let report: Value = serde_json::from_slice(&output.stdout)?;
    ensure!(
        report["classification"] == "EXPECTED_PERMISSION_ERROR_IN_RPC_RESPONSE"
            && report["request_id"] == expectation["request_id"]
            && report["request_payload_bound"] == true
            && report["cryptographic_proof"] == false,
        "invalid classifier report"
    );
    Ok(report)
}

pub async fn send_signed_raw(
    network: &NetworkConfig,
    signed: impl Into<SignedTransaction>,
    mut expectation: Value,
) -> Result<Value> {
    let url = network.rpc_endpoints.first().context("sandbox RPC endpoint missing")?.url.clone();
    ensure!(
        url.scheme() == "http"
            && url.host_str().and_then(|h| h.parse::<IpAddr>().ok()).is_some_and(|ip| ip.is_loopback())
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "raw refusal collection is restricted to a literal loopback Sandbox endpoint"
    );
    let signed: SignedTransaction = signed.into();
    let request = prepare_request(&borsh::to_vec(&signed)?, &mut expectation)?;
    let mut response = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(60))
        .build()?
        .post(url)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(request.clone())
        .send()
        .await?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "unexpected Sandbox HTTP status: {}",
        response.status()
    );
    if let Some(length) = response.content_length() {
        ensure!(length <= MAX_RESPONSE_BYTES as u64, "oversized Sandbox content length");
    }
    let mut raw = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= MAX_RESPONSE_BYTES - raw.len(),
            "oversized chunked Sandbox response"
        );
        raw.extend_from_slice(&chunk);
    }
    // Never deserialize the untrusted response before the duplicate-key checks.
    let report = classify_original_bytes(&request, &raw, &expectation)?;
    let observed = report.get("rpc_response").context("classified response missing")?.clone();
    // Only disposable local transactions and raw node errors; no secret key is captured.
    println!("X3_VAULT_SANDBOX_RAW_CAPTURE {}", serde_json::to_string(&report)?);
    Ok(observed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_fixture() -> (Vec<u8>, Value, Value) {
        let mut expected = json!({"case":"direct_transfer"});
        let request = prepare_request(b"synthetic fixture; not a signed transaction", &mut expected).unwrap();
        let response = json!({
            "jsonrpc":"2.0", "id":expected["request_id"],
            "error":{"code":-32000,"name":"HANDLER_ERROR", "cause":{"name":"INVALID_TRANSACTION","info":{}},
                "data":{"TxExecutionError":{"InvalidTxError":{"InvalidAccessKeyError":"RequiresFullAccess"}}}}
        });
        (request, expected, response)
    }

    #[test]
    fn request_ids_are_bound_and_distinct_even_for_identical_bytes() {
        let mut first = json!({"case":"direct_transfer"});
        let mut second = first.clone();
        prepare_request(b"identical fixture", &mut first).unwrap();
        prepare_request(b"identical fixture", &mut second).unwrap();
        assert_ne!(first["request_id"], second["request_id"]);
    }

    #[test]
    fn original_response_goes_through_python_without_claiming_chain_proof() {
        let (request, expected, response) = capture_fixture();
        let report = classify_original_bytes(&request, &serde_json::to_vec(&response).unwrap(), &expected).unwrap();
        assert_eq!(report["rpc_response"], response);
        assert_eq!(report["signed_transaction_verified"], false);
    }

    #[test]
    fn mismatched_id_and_duplicate_wire_keys_fail() {
        let (request, expected, mut response) = capture_fixture();
        response["id"] = json!("unrelated request");
        assert!(classify_original_bytes(&request, &serde_json::to_vec(&response).unwrap(), &expected).is_err());
        response["id"] = expected["request_id"].clone();
        assert!(classify_original_bytes(&request, &serde_json::to_vec(&response).unwrap(), &expected).is_ok());
        let text = serde_json::to_string(&response).unwrap();
        let duplicate = format!("{{\"id\":\"forged\",{}", &text[1..]);
        assert!(classify_original_bytes(&request, duplicate.as_bytes(), &expected).is_err());
    }

    #[test]
    fn oversize_response_fails_before_starting_a_classifier() {
        assert!(classify_original_bytes(b"{}", &vec![0; MAX_RESPONSE_BYTES + 1], &json!({})).is_err());
    }
}
