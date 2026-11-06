//! Real RPC bytes are untrusted observations, not NEAR genesis or checkpoint proof.
//! Tests use actual 2026-11-05 archived testnet light-client responses.
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use x3_near_lightclient_approval_core::epoch::{parse_rpc_light_view, UntrustedEpochChain};

fn read_verified_bytes(path: &Path, expected: &str) -> Value {
    let raw = std::fs::read(path).expect("real NEAR testnet fixture missing");
    let digest = Sha256::digest(&raw);
    let actual = digest.iter().map(|b| format!("{b:02x}")).collect::<String>();
    assert_eq!(actual, expected, "genuine chain fixture changed");
    serde_json::from_slice(&raw).expect("invalid chain fixture JSON")
}

fn fixture(name: &str, sha: &str) -> Value {
    read_verified_bytes(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name),
        sha,
    )
}

fn live_views() -> (Value, Value, Value) {
    let hbm = fixture(
        "hbm-announces-p27.json",
        "acf673ba657bc0b90f75e17bf5fd1f407ae81103f8cbfa6f6c1fd74713f8a5e0",
    );
    let p27 = fixture(
        "p27-announces-gbz.json",
        "dca4ca9cf83d838630cd06ae7ecdb17f23e3639c97c91798b9cf966e6fdc68cd",
    );
    let gbz = read_verified_bytes(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../evidence/near-proof/testnet/sha256/c4/c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7.json"),
        "c47f0c381dbd9a37ac432aa4f77cf9d47ea8ccc452bfe4ff2a67b868953caed7");
    (hbm, p27, gbz)
}

#[test]
fn two_real_near_testnet_epochs_validate_crypto_from_untrusted_bootstrap() {
    let (hbm, p27, gbz) = live_views();
    let first = parse_rpc_light_view(&hbm).expect("HbM parsing");
    let second = parse_rpc_light_view(&p27).expect("p27 parsing");
    let third = parse_rpc_light_view(&gbz).expect("GbZ parsing");
    assert_eq!(first.height(), 276349252);
    assert_eq!(second.height(), 276392452);
    assert_eq!(third.height(), 276420402);
    assert_eq!(first.next_epoch_id(), second.epoch_id());
    assert_eq!(second.next_epoch_id(), third.epoch_id());
    let mut chain = UntrustedEpochChain::bootstrap_rpc_candidate(&first).expect("candidate anchor");
    let step1 = chain.advance_candidate(&second).expect("valid real p27 signatures");
    let step2 = chain.advance_candidate(&third).expect("valid real GbZ signatures");
    assert!(step1.crossed_epoch);
    assert!(step2.crossed_epoch);
    assert_eq!(step2.valid_signatures, 19);
    assert_eq!(chain.head_height(), third.height());
    assert!(!step1.authenticated && !step2.authenticated);
    assert!(!step1.authorized_to_sign && !step2.authorized_to_sign);
}

#[test]
fn fake_next_bp_hash_rejected() {
    let (hbm, mut p27, _) = live_views();
    let first = parse_rpc_light_view(&hbm).unwrap();
    let mut chain = UntrustedEpochChain::bootstrap_rpc_candidate(&first).unwrap();
    p27["result"]["next_bps"][0]["stake"] = json!("1");
    assert!(parse_rpc_light_view(&p27).is_err());
    assert_eq!(chain.head_height(), first.height());
    assert!(chain.advance_candidate(&first).is_err());
}

#[test]
fn forged_signature_fails_without_modifying_candidate_head() {
    let (hbm, mut p27, _) = live_views();
    let first = parse_rpc_light_view(&hbm).unwrap();
    let mut chain = UntrustedEpochChain::bootstrap_rpc_candidate(&first).unwrap();
    p27["result"]["approvals_after_next"][0] =
        json!("ed25519:1111111111111111111111111111111111111111111111111111111111111111");
    let changed = parse_rpc_light_view(&p27).unwrap();
    assert!(chain.advance_candidate(&changed).is_err());
    assert_eq!(chain.head_height(), first.height());
}

#[test]
fn replay_and_skip_epoch_fail_closed_without_state_mutation() {
    let (hbm, p27, gbz) = live_views();
    let first = parse_rpc_light_view(&hbm).unwrap();
    let second = parse_rpc_light_view(&p27).unwrap();
    let third = parse_rpc_light_view(&gbz).unwrap();
    let mut chain = UntrustedEpochChain::bootstrap_rpc_candidate(&first).unwrap();
    assert!(chain.advance_candidate(&third).is_err(), "skipping p27 must fail");
    assert_eq!(chain.head_height(), first.height());
    assert!(chain.advance_candidate(&second).is_ok());
    assert!(chain.advance_candidate(&second).is_err(), "same height replay");
    assert_eq!(chain.head_height(), second.height());
}

#[test]
fn transition_without_next_bps_rejected() {
    let (hbm, p27, _) = live_views();
    let first = parse_rpc_light_view(&hbm).unwrap();
    // Simulate an untrusted RPC omitting mandatory transition validators.
    let mut mutated = p27.clone();
    mutated["result"]["next_bps"] = serde_json::Value::Null;
    let second = parse_rpc_light_view(&mutated).unwrap();
    let mut chain = UntrustedEpochChain::bootstrap_rpc_candidate(&first).unwrap();
    assert!(chain.advance_candidate(&second).is_err());
    assert_eq!(chain.head_height(), first.height());
}

#[test]
fn even_valid_signatures_cannot_authorize_from_rpc_only() {
    let (hbm, p27, _) = live_views();
    let mut chain = UntrustedEpochChain::bootstrap_rpc_candidate(&parse_rpc_light_view(&hbm).unwrap()).unwrap();
    let result = chain.advance_candidate(&parse_rpc_light_view(&p27).unwrap()).unwrap();
    assert!(!result.authenticated);
    assert!(!result.authorized_to_sign);
}

#[test]
fn real_validator_and_message_tampering_cannot_advance_epoch() {
    let (hbm, p27, gbz) = live_views();
    let first = parse_rpc_light_view(&hbm).unwrap();
    let second = parse_rpc_light_view(&p27).unwrap();
    type Mutation = (&'static str, fn(&mut Value));
    let mutations: Vec<Mutation> = vec![
        ("wrong height", |x| {
            x["result"]["inner_lite"]["height"] = json!(276420403)
        }),
        ("wrong epoch", |x| {
            x["result"]["inner_lite"]["epoch_id"] = json!("11111111111111111111111111111111")
        }),
        ("wrong approval hash", |x| {
            x["result"]["next_block_inner_hash"] = json!("11111111111111111111111111111111")
        }),
        ("no stake quorum", |x| x["result"]["approvals_after_next"] = json!([])),
        ("surplus approval slots", |x| {
            let a = x["result"]["approvals_after_next"].as_array_mut().unwrap();
            a.push(Value::Null);
        }),
        ("unsupported key type", |x| {
            x["result"]["next_bps"][0]["public_key"] = json!("secp256k1:invalid")
        }),
        ("unsupported V2 stake", |x| {
            x["result"]["next_bps"][0]["validator_stake_struct_version"] = json!("V2")
        }),
        ("changed validator weight", |x| {
            x["result"]["next_bps"][0]["stake"] = json!("1")
        }),
        ("changed proof timestamp", |x| {
            x["result"]["inner_lite"]["timestamp_nanosec"] = json!("1");
            x["result"]["inner_lite"]["timestamp"] = json!(1);
        }),
    ];
    for (description, mutate) in mutations {
        let mut changed = gbz.clone();
        mutate(&mut changed);
        let mut state = UntrustedEpochChain::bootstrap_rpc_candidate(&first).unwrap();
        state.advance_candidate(&second).unwrap();
        let before = state.head_height();
        let result = parse_rpc_light_view(&changed).and_then(|v| state.advance_candidate(&v).map(|_| ()));
        assert!(result.is_err(), "unexpectedly accepted {description}");
        assert_eq!(state.head_height(), before, "partial state mutation: {description}");
    }
}

#[test]
fn evidence_root_is_not_promoted_to_trusted_merely_by_operator_rpc() {
    let (hbm, p27, gbz) = live_views();
    let mut chain = UntrustedEpochChain::bootstrap_rpc_candidate(&parse_rpc_light_view(&hbm).unwrap()).unwrap();
    for view in [p27, gbz] {
        let verified_against_untrusted = chain.advance_candidate(&parse_rpc_light_view(&view).unwrap()).unwrap();
        assert!(!verified_against_untrusted.authenticated);
        assert!(!verified_against_untrusted.authorized_to_sign);
    }
}
