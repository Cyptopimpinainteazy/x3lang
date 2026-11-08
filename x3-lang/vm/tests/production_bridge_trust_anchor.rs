//! Black-box production initialization checks in separate processes so no
//! process-global environment mutation can contaminate concurrent Rust tests.
use std::process::Command;
use x3_lang_vm::bridge::init_production_backend;

#[test]
fn isolated_admission_probe() {
    let Ok(field) = std::env::var("X3_BRIDGE_EXPECT_REJECTION") else {
        return;
    };
    match init_production_backend() {
        Err(err) => {
            assert_eq!(err.code, "X3_BRIDGE_INIT_FAILED");
            assert!(err.message.contains(&field), "missing field {field}: {}", err.message);
        }
        Ok(_) => panic!("unsafe production config accepted for {field}"),
    }
}

#[test]
fn empty_production_trust_anchors_are_rejected() {
    let cases = [
        ("evm-light-client", "X3_EVM_TRUSTED_HEADER_HASH"),
        ("svm-light-client", "X3_SVM_TRUSTED_BANK_HASH"),
    ];
    for (kind, field) in cases {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "isolated_admission_probe"])
            .env("X3_BRIDGE_VERIFIER", kind)
            .env("X3_BRIDGE_EXPECT_REJECTION", field)
            .env("X3_BACKEND", "prod")
            .env_remove("X3_EVM_TRUSTED_HEADER_HASH")
            .env_remove("X3_SVM_TRUSTED_BANK_HASH")
            .env(field, "")
            .output()
            .expect("run real backend initialization");
        assert!(
            output.status.success(),
            "{kind}/{field} accepted empty trusted anchor:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(unix)]
#[test]
fn non_utf8_verifier_selector_cannot_be_treated_as_unset() {
    use std::os::unix::ffi::OsStringExt;
    let output = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "isolated_admission_probe"])
        .env("X3_BRIDGE_VERIFIER", std::ffi::OsString::from_vec(vec![0xff, 0x41]))
        .env_remove("X3_BACKEND")
        .env("X3_BRIDGE_EXPECT_REJECTION", "X3_BRIDGE_VERIFIER")
        .output()
        .expect("run real backend initialization");
    assert!(
        output.status.success(),
        "invalid selector was interpreted as absent:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
