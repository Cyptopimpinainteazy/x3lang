//! Actual source -> Rust CLI export agrees with the Python NEAR lowering.
//! This is local compilation only; it does not authorize or execute a payment.
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;

#[test]
fn payment_source_exports_the_same_financial_terms_as_python() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let source = root.join("tests/fixtures/near_intents_named_nonce_payment.x3");
    let temp = tempfile::tempdir().expect("temporary outputs");
    let exported = temp.path().join("intent.json");
    let output = Command::new(env!("CARGO_BIN_EXE_x3c"))
        .arg("intent")
        .arg(&source)
        .args(["--emit-hash", "--emit-plan", "--out"])
        .arg(&exported)
        .output()
        .expect("actual x3c process");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let rust: Value = serde_json::from_slice(&std::fs::read(exported).unwrap()).unwrap();
    let python = Command::new("python3")
        .args(["-c", "import json,sys; sys.path.insert(0,sys.argv[1]); import near_intents; print(json.dumps(near_intents.compile_program(sys.argv[2])[1]))"])
        .arg(&root).arg(&source).output().expect("actual Python NEAR lowering");
    assert!(python.status.success(), "{}", String::from_utf8_lossy(&python.stderr));
    let leg: Value = serde_json::from_slice(&python.stdout).unwrap();
    assert_eq!(rust["source_amount"], leg["amount"]);
    assert_eq!(rust["dest_receiver"], leg["recipient"]);
    assert_eq!(rust["mandate_owner"], leg["mandate_owner"]);
    assert_eq!(rust["dest_min_amount"], leg["min_receive"]);
    assert_eq!(rust["timeout_secs"], 3600);
    let constraints = rust["constraints"].as_array().unwrap();
    for (kind, value) in [
        ("nonce", format!("unused {}", leg["nonce"].as_str().unwrap())),
        ("fees", format!("<= {} bps", leg["fee_cap_bps"])),
        ("slippage", "<= 0.9%".to_string()),
        (
            "refund_path",
            format!("Solana.USDC:{}", leg["refund_to"].as_str().unwrap()),
        ),
    ] {
        assert!(
            constraints.iter().any(|c| c["kind"] == kind && c["arg"] == value),
            "missing {kind} {value}: {rust}"
        );
    }
    assert_eq!(leg["slippage_bps"], 90);
    let artifact = temp.path().join("payment.x3b");
    let build = Command::new(env!("CARGO_BIN_EXE_x3c"))
        .arg("build")
        .arg(&source)
        .arg("--out")
        .arg(&artifact)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    assert!(!std::fs::read(&artifact).unwrap().is_empty());
    assert!(
        String::from_utf8_lossy(&build.stderr).contains("source-lock proof"),
        "compiling must not suppress the missing external-chain proof warning"
    );
    let run = Command::new(env!("CARGO_BIN_EXE_x3c"))
        .arg("run")
        .arg(&artifact)
        .args(["--measured-slippage-bps", "90"])
        .output()
        .unwrap();
    assert!(!run.status.success(), "generic dry run cannot authorize the mandate");
    assert!(String::from_utf8_lossy(&run.stderr).contains("X3_FEATURE_NOT_AVAILABLE"));
}
