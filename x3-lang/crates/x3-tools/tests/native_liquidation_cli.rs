use std::{fs, process::Command};
use x3_lang_vm::liquidation::{artifact_hash, hex_string};

#[test]
fn native_cli_requires_an_independently_pinned_artifact_before_reading_rpc_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let example = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/native_liquidation.x3");
    let code = x3_lang_compiler::compile_source(&fs::read_to_string(example).unwrap()).unwrap();
    let artifact = dir.path().join("policy.x3b");
    fs::write(&artifact, &code).unwrap();
    let batch = dir.path().join("batch.json");
    fs::write(
        &batch,
        serde_json::json!({
            "snapshot":{"chain_id":8453,"block_number":1,"block_hash":format!("0x{}","11".repeat(32)),"nonce":0},
            "current_block":1,"candidates":[]
        })
        .to_string(),
    )
    .unwrap();
    let run = |hash: &str| {
        Command::new(env!("CARGO_BIN_EXE_x3_liquidation"))
            .args([
                "--artifact",
                artifact.to_str().unwrap(),
                "--batch",
                batch.to_str().unwrap(),
                "--approved-artifact-hash",
                hash,
                "--rpc-url-env",
                "X3_NATIVE_TEST_UNSET_RPC",
            ])
            .env_remove("X3_NATIVE_TEST_UNSET_RPC")
            .output()
            .unwrap()
    };
    let wrong = run(&format!("0x{}", "aa".repeat(32)));
    assert!(!wrong.status.success());
    assert!(String::from_utf8_lossy(&wrong.stderr).contains("ARTIFACT_NOT_APPROVED"));
    let approved = run(&hex_string(&artifact_hash(&code)));
    assert!(!approved.status.success());
    assert!(String::from_utf8_lossy(&approved.stderr).contains("RPC URL environment variable is unset"));
    assert!(wrong.stdout.is_empty() && approved.stdout.is_empty());
}

#[test]
fn public_native_cli_has_no_key_or_broadcast_option() {
    let output = Command::new(env!("CARGO_BIN_EXE_x3_liquidation"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("--approved-artifact-hash"));
    assert!(help.contains("unsigned EVM preflight"));
    assert!(!help.contains("--private-key") && !help.contains("--broadcast"));
}
