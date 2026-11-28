use std::{
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};
use x3_lang_common::liquidation::LiquidationPolicy;
use x3_lang_vm::liquidation::{Batch, LiquidationHost, Snapshot};
use x3_lang_vm::liquidation_evm::{EvmTransport, EvmTransportConfig, TransportMode};

fn empty_batch() -> Batch {
    Batch {
        snapshot: Snapshot {
            chain_id: 31337,
            block_number: 0,
            block_hash: format!("0x{}", "11".repeat(32)),
            nonce: 0,
        },
        current_block: 0,
        candidates: vec![],
    }
}

fn policy() -> LiquidationPolicy {
    let address = |n: u8| format!("0x{n:02x}{}", "00".repeat(19));
    LiquidationPolicy {
        chain_id: 31337,
        executor: address(1),
        executor_code_hash: format!("0x{}", "11".repeat(32)),
        keeper: address(2),
        owner: address(3),
        morpho: address(4),
        loan_token: address(5),
        markets: vec![format!("0x{}", "22".repeat(32))],
        routers: vec![address(6)],
        min_repay: 1,
        max_repay: 100,
        min_profit: 1,
        max_gas: 500_000,
        max_fee_per_gas: 10,
        max_priority_fee_per_gas: 1,
        max_total_fee_wei: 5_000_000,
        max_age_blocks: 1,
        expires_at_block: 1000,
        max_candidates: 4,
    }
}

#[test]
fn native_evm_cannot_replace_the_approved_policy_through_the_host_trait() {
    let config = EvmTransportConfig {
        read_url: "http://127.0.0.1:8545".into(),
        submit_url: None,
        mode: TransportMode::PrepareOnly,
        timeout: Duration::from_secs(1),
    };
    let mut host = EvmTransport::new(
        config,
        &artifact(),
        digest(),
        empty_batch(),
        None,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let mut altered = policy();
    altered.max_repay = 1000;
    assert!(
        host.candidates(&altered).is_err(),
        "a valid but more permissive policy must not acquire the approved artifact's authority"
    );
}

#[test]
fn native_evm_broadcast_requires_local_endpoint_and_ephemeral_signer() {
    let config = EvmTransportConfig {
        read_url: "https://mainnet.base.org".into(),
        submit_url: Some("https://mainnet-sequencer.base.org".into()),
        mode: TransportMode::LocalBroadcast,
        timeout: Duration::from_secs(1),
    };
    let error = EvmTransport::new(
        config,
        &artifact(),
        digest(),
        empty_batch(),
        None,
        Arc::new(AtomicBool::new(false)),
    )
    .err()
    .expect("public broadcast must be refused");
    assert!(error.contains("loopback"), "{error}");
}

#[test]
fn native_evm_prepare_has_no_signer_and_local_signing_cannot_target_public_chain() {
    let config = EvmTransportConfig {
        read_url: "https://mainnet.base.org".into(),
        submit_url: None,
        mode: TransportMode::PrepareOnly,
        timeout: Duration::from_secs(1),
    };
    assert!(EvmTransport::new(
        config,
        &artifact(),
        digest(),
        empty_batch(),
        None,
        Arc::new(AtomicBool::new(false))
    )
    .is_ok());
    let secret = secp256k1::SecretKey::new(&mut rand::thread_rng());
    let config = EvmTransportConfig {
        read_url: "https://mainnet.base.org".into(),
        submit_url: None,
        mode: TransportMode::PrepareOnly,
        timeout: Duration::from_secs(1),
    };
    let error = EvmTransport::new(
        config,
        &artifact(),
        digest(),
        empty_batch(),
        Some(x3_lang_vm::liquidation_evm::EphemeralLocalSigner::from_declared_secret(
            secret,
        )),
        Arc::new(AtomicBool::new(false)),
    )
    .err()
    .unwrap();
    assert!(error.contains("UNSIGNED_MODE_REFUSES_SIGNER"), "{error}");
    let config = EvmTransportConfig {
        read_url: "http://127.0.0.1:8545".into(),
        submit_url: Some("http://127.0.0.1:8545".into()),
        mode: TransportMode::LocalBroadcast,
        timeout: Duration::from_secs(1),
    };
    let mut batch = empty_batch();
    batch.snapshot.chain_id = 8453;
    let error = EvmTransport::new(
        config,
        &artifact(),
        digest(),
        batch,
        None,
        Arc::new(AtomicBool::new(false)),
    )
    .err()
    .unwrap();
    assert!(error.contains("LOCAL_CHAIN_ONLY"), "{error}");
}

#[test]
fn native_evm_rejects_unbounded_timeout_and_unbound_artifact() {
    for (timeout, artifact, expected) in [
        (Duration::ZERO, [1; 32], "TIMEOUT_BOUND"),
        (Duration::from_secs(31), [1; 32], "TIMEOUT_BOUND"),
        (Duration::from_secs(1), [0; 32], "UNBOUND_ARTIFACT"),
    ] {
        let config = EvmTransportConfig {
            read_url: "http://127.0.0.1:8545".into(),
            submit_url: None,
            mode: TransportMode::PrepareOnly,
            timeout,
        };
        let error = EvmTransport::new(
            config,
            &self::artifact(),
            artifact,
            empty_batch(),
            None,
            Arc::new(AtomicBool::new(false)),
        )
        .err()
        .unwrap();
        assert!(error.contains(expected), "{error}");
    }
}

fn artifact() -> Vec<u8> {
    let fields = serde_json::to_value(policy())
        .unwrap()
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| format!("{k}: {v};"))
        .collect::<Vec<_>>()
        .join("\n");
    x3_lang_compiler::compile_source(&format!("native_liquidation Test {{\n{fields}\n}}")).unwrap()
}
fn digest() -> [u8; 32] {
    x3_lang_vm::liquidation::artifact_hash(&artifact())
}
