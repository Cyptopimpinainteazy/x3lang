use std::path::PathBuf;
use std::process::Command;

fn x3c_bin() -> PathBuf {
    if let Some(path) = option_env!("CARGO_BIN_EXE_x3c") {
        return PathBuf::from(path);
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root resolution")
        .to_path_buf();
    workspace_root.join("target").join("debug").join("x3c")
}

fn x3c() -> Command {
    Command::new(x3c_bin())
}

fn write_fixture(name: &str, body: &str) -> PathBuf {
    let path = std::env::temp_dir().join(name);
    std::fs::write(&path, body).expect("fixture write");
    path
}

fn example_path(name: &str) -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
        .join("examples")
        .join(name)
}

#[test]
fn test_cli_inspect_source() {
    let src = example_path("flagship_b52.x3");
    let output = x3c().arg("inspect").arg(&src).output().expect("x3c inspect");
    assert!(output.status.success(), "inspect should succeed: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Chains:"), "inspect should show chains");
    assert!(stdout.contains("Constraints:"), "inspect should show constraints");
}

#[test]
fn test_cli_check_simple_swap() {
    let src = example_path("simple_swap.x3");
    let output = x3c().arg("check").arg(&src).output().expect("x3c check");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{}{}", String::from_utf8_lossy(&output.stdout), &stderr);
    assert!(
        !output.status.success() || combined.contains("ops"),
        "check on simple_swap should fail (missing refund path, no nonce) or report ops, got status={}",
        output.status
    );
}

#[test]
fn test_cli_deploy_simple_swap() {
    let src = example_path("simple_swap.x3");
    let output = x3c().arg("deploy").arg(&src).output().expect("x3c deploy");
    assert!(output.status.success(), "deploy should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Deployment Plan"), "deploy should produce a plan");
    assert!(stdout.contains("Bytecode size"), "deploy should report bytecode size");
    assert!(stdout.contains("Operations:"), "deploy should report operation count");
}

#[test]
fn test_cli_refund_on_timeout_refund() {
    let src = example_path("timeout_refund.x3");
    let output = x3c()
        .arg("refund")
        .arg(&src)
        .arg("--check-only")
        .output()
        .expect("x3c refund");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Refund") || stdout.contains("refund"),
        "refund command should find a refund path"
    );
}

#[test]
fn test_cli_check_mainnet_safe() {
    let src = example_path("mainnet_safe_swap.x3");
    let output = x3c()
        .arg("check")
        .arg("--mode")
        .arg("mainnet")
        .arg(&src)
        .output()
        .expect("x3c check --mode mainnet");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        stdout.contains("ops") || !combined.contains("panic"),
        "mainnet check should not panic, got: {combined}"
    );
}

#[test]
fn test_cli_check_mainnet_rejects_simple_swap() {
    let src = example_path("simple_swap.x3");
    let output = x3c()
        .arg("check")
        .arg("--mode")
        .arg("mainnet")
        .arg(&src)
        .output()
        .expect("x3c check --mode mainnet");
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}\n{stderr}");
    assert!(
        !output.status.success() || combined.contains("slippage") || combined.contains("refund"),
        "mainnet mode should reject simple_swap, got status={} output={combined}",
        output.status
    );
}

#[test]
fn test_cli_inspect_json_flag() {
    let src = write_fixture(
        "cli_inspect_json.x3",
        r#"intent test_json {
        from ethereum.USDC amount 100 receiver 0x1111111111111111111111111111111111111111
        to solana.USDC receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
        route {
            swap uniswap ethereum.USDC -> ethereum.ETH amount 1000 min_output 777
        }
        timeout 30s refund ethereum.USDC to sender
        on_fail rollback
    }
    "#,
    );
    let output = x3c()
        .arg("inspect")
        .arg("--json")
        .arg(&src)
        .output()
        .expect("x3c inspect --json");
    assert!(output.status.success(), "inspect --json should succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("\"format\""), "JSON output should have format field");
    assert!(stdout.contains("\"chains\""), "JSON output should have chains");
}

#[test]
fn test_cli_audit_on_simple_swap() {
    let src = example_path("simple_swap.x3");
    let output = x3c().arg("audit").arg(&src).output().expect("x3c audit");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Mainnet Safety Audit"), "audit should run");
    assert!(stdout.contains("Risk score"), "audit should report risk score");
}

#[test]
fn test_cli_receipt_verify_trusted_signer() {
    let src = example_path("trading_core_v1.x3");
    let receipt = std::env::temp_dir().join("x3c-trusted-receipt.json");

    // Produce a signed receipt and read the signer public key the command
    // reports (the fixed dev seed's key, printed so the caller can trust it).
    let execute = x3c()
        .arg("receipt")
        .arg("execute")
        .arg(&src)
        .arg("-o")
        .arg(&receipt)
        .output()
        .expect("x3c receipt execute");
    assert!(execute.status.success(), "execute should succeed");
    let stderr = String::from_utf8_lossy(&execute.stderr);
    let marker = "signer public key ";
    let public_key = stderr
        .split(marker)
        .nth(1)
        .and_then(|s| s.split_whitespace().next())
        .expect("stderr must report the signer public key")
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .collect::<String>();
    assert_eq!(public_key.len(), 64, "the public key must be 64 hex characters");

    // A receipt verified against the correct trusted key passes.
    let trusted = format!("x3c-receipt-execute={public_key}");
    let ok = x3c()
        .arg("receipt")
        .arg("verify")
        .arg(&receipt)
        .arg("--trusted")
        .arg(&trusted)
        .output()
        .expect("x3c receipt verify --trusted");
    assert!(
        ok.status.success(),
        "verifying against the correct trusted key should succeed: {:?}",
        ok.status
    );
    assert!(
        String::from_utf8_lossy(&ok.stdout).contains("trusted signer attestation"),
        "a trusted verification should say it verified an attestation"
    );

    // The same receipt is refused when the named key id is not trusted.
    let wrong = format!("x3c-receipt-execute={}", "0".repeat(64));
    let refused = x3c()
        .arg("receipt")
        .arg("verify")
        .arg(&receipt)
        .arg("--trusted")
        .arg(&wrong)
        .output()
        .expect("x3c receipt verify --trusted (wrong key)");
    assert!(
        !refused.status.success(),
        "verifying against an untrusted key must fail"
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("not trusted"),
        "an untrusted attestor must be refused by name"
    );

    let _ = std::fs::remove_file(&receipt);
}
