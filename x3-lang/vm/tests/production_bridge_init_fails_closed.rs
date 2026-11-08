//! Subprocess-level configuration regression: exercise the real production
//! bridge initializer without mutating process-global environment in parallel
//! Rust tests or contacting a live RPC.
use std::process::Command;
use x3_lang_vm::bridge::init_production_backend;

#[test]
fn isolated_bridge_init_probe() {
    let Ok(expected_field) = std::env::var("X3_BRIDGE_TEST_EXPECT_REJECTION") else {
        return;
    };
    match init_production_backend() {
        Err(err) => {
            assert_eq!(err.code, "X3_BRIDGE_INIT_FAILED");
            assert!(
                err.message.contains(&expected_field),
                "expected config error naming {expected_field}, got: {}",
                err.message
            );
        }
        Ok(_) => panic!("production backend accepted missing or malformed {expected_field}"),
    }
}

#[test]
fn production_backend_requires_rpc_urls_and_rejects_bad_thresholds() {
    // (verifier, field that must be rejected, explicitly supplied env pairs)
    let cases: &[(&str, &str, &[(&str, &str)])] = &[
        ("evm-rpc", "X3_EVM_RPC_URL", &[]),
        ("svm-rpc", "X3_SVM_RPC_URL", &[]),
        (
            "evm-rpc",
            "X3_EVM_MIN_CONFIRMATIONS",
            &[
                ("X3_EVM_RPC_URL", "http://127.0.0.1:8545"),
                ("X3_EVM_MIN_CONFIRMATIONS", "twelve"),
            ],
        ),
        (
            "evm-rpc",
            "X3_EVM_MIN_CONFIRMATIONS",
            &[
                ("X3_EVM_RPC_URL", "http://127.0.0.1:8545"),
                ("X3_EVM_MIN_CONFIRMATIONS", "0"),
            ],
        ),
        (
            "evm-light-client",
            "X3_EVM_MIN_BLOCK_NUMBER",
            &[
                ("X3_EVM_TRUSTED_HEADER_HASH", "0xdeadbeef"),
                ("X3_EVM_MIN_BLOCK_NUMBER", "not-a-block"),
            ],
        ),
        (
            "svm-light-client",
            "X3_SVM_MIN_SIGNATURES",
            &[
                ("X3_SVM_TRUSTED_BANK_HASH", "fake-local-test-hash"),
                ("X3_SVM_MIN_SIGNATURES", "wrong"),
            ],
        ),
        (
            "svm-light-client",
            "X3_SVM_MIN_STAKE_BPS",
            &[
                ("X3_SVM_TRUSTED_BANK_HASH", "fake-local-test-hash"),
                ("X3_SVM_MIN_STAKE_BPS", "garbled"),
            ],
        ),
        (
            "svm-light-client",
            "X3_SVM_MIN_STAKE_BPS",
            &[
                ("X3_SVM_TRUSTED_BANK_HASH", "fake-local-test-hash"),
                ("X3_SVM_MIN_STAKE_BPS", "10001"),
            ],
        ),
    ];

    let current_exe = std::env::current_exe().expect("test binary");
    let names = [
        "X3_EVM_RPC_URL",
        "X3_SVM_RPC_URL",
        "X3_EVM_MIN_CONFIRMATIONS",
        "X3_EVM_MIN_BLOCK_NUMBER",
        "X3_EVM_TRUSTED_HEADER_HASH",
        "X3_SVM_TRUSTED_BANK_HASH",
        "X3_SVM_MIN_SIGNATURES",
        "X3_SVM_MIN_STAKE_BPS",
    ];
    for (verifier, expected, overrides) in cases {
        let mut child = Command::new(&current_exe);
        child
            .arg("--exact")
            .arg("isolated_bridge_init_probe")
            .env("X3_BRIDGE_VERIFIER", verifier)
            .env("X3_BRIDGE_TEST_EXPECT_REJECTION", expected);
        for name in names {
            child.env_remove(name);
        }
        for (key, value) in *overrides {
            child.env(key, value);
        }
        let output = child.output().expect("isolated bridge config test");
        assert!(
            output.status.success(),
            "{verifier}/{expected}: initialized unsafely or reported wrong error\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
