use std::collections::BTreeSet;
use x3_lang_ast::policy_ir::Requirement;
use x3_lang_compiler::settlement_capability::*;
use x3_lang_compiler::{compile_capability_source, compile_source};

const SOURCE: &str = include_str!("fixtures/pir_settlement_claim.x3");
const NOW: u64 = 1_700_000_000;
const SCOPE: &str = "sandbox:pir07";
fn hex(b: &[u8]) -> String {
    b.iter().map(|v| format!("{v:02x}")).collect::<Vec<_>>().join("")
}
fn terms() -> WindowTerms {
    let mut escrow = [0; 20];
    escrow[17..].copy_from_slice(&[0x0e, 0x5c, 0x40]);
    let mut depositor = [0; 20];
    depositor[17..].copy_from_slice(&[0x0a, 0x11, 0xce]);
    let mut lp = [0; 20];
    lp[18..].copy_from_slice(&[0xbe, 0xef]);
    WindowTerms {
        bsc_chain_id: 31337,
        escrow,
        depositor,
        lp,
        bnb_wei: 10u128.pow(18),
        salt: [0x11; 32],
        near_vault: "x3-vault.testnet".into(),
        near_initiator: "lp.testnet".into(),
        near_recipient: "alice.testnet".into(),
        hashlock: sha2::Sha256::digest([3; 32]).into(),
        near_yocto: 10u128.pow(24),
        funding_deadline_ns: 1_700_000_600_000_000_000,
        expiry_ns: 1_700_001_200_000_000_000,
        near_genesis: [5; 32],
        near_vault_code_hash: [6; 32],
    }
}
use sha2::Digest;
#[test]
fn native_vectors_and_blocked_execution() {
    let p = compile_settlement_plan(SOURCE, SCOPE, NOW, &terms()).unwrap();
    assert_eq!(
        hex(&p.source_binding),
        "f9ceb35c05cbdd513aa7342a5ba33dd8538a9580668cea14017b5ef7f91d05aa"
    );
    assert_eq!(
        hex(&p.window_id),
        "9d66ddfe92e56e993dd87e2f7ca53b87e39ceb35325cc44b404c0a8eb87de110"
    );
    assert_eq!(
        hex(&p.outcome_commitment),
        "c956f53937ecff906f42672b636a1b8216fd7beade6dd7eeaaa2074e52d9177e"
    );
    assert_eq!(p.outcome, 2);
    assert_eq!(p.status, PlanStatus::BlockedAwaitingNativeVerifier);
    println!("PIR07_BINDING=0x{}", hex(&p.source_binding));
    println!("PIR07_WINDOW=0x{}", hex(&p.window_id));
    println!("PIR07_CLAIM=0x{}", hex(&p.outcome_commitment));
    let q = compile_settlement_plan(&SOURCE.replace("\"claim\"", "\"refund\""), SCOPE, NOW, &terms()).unwrap();
    assert_eq!(q.outcome, 3);
    assert_eq!(
        hex(&q.outcome_commitment),
        "561cc941f2d1e81c86d2d428753acae16de924e836c2d9b589dbbc4a303bbb17"
    );
    assert_ne!(p.plan_digest, q.plan_digest);
    println!("PIR07_REFUND=0x{}", hex(&q.outcome_commitment));
    let q = compile_settlement_plan(&SOURCE.replace("\"claim\"", "\"lock\""), SCOPE, NOW, &terms()).unwrap();
    assert_eq!(q.outcome, 1);
    assert_eq!(
        hex(&q.outcome_commitment),
        "4b7b258b6db8ae6b3979c0ef505ecf625c0ab59727439d980ce017b01fa04808"
    );
    println!("PIR07_LOCK=0x{}", hex(&q.outcome_commitment));
}
#[test]
fn all_native_terms_are_bound() {
    let mut variants = vec![];
    let mut t = terms();
    t.bsc_chain_id = 97;
    variants.push(t);
    let mut t = terms();
    t.escrow[0] = 9;
    variants.push(t);
    let mut t = terms();
    t.depositor[0] = 9;
    variants.push(t);
    let mut t = terms();
    t.lp[0] = 9;
    variants.push(t);
    let mut t = terms();
    t.bnb_wei += 1;
    variants.push(t);
    let mut t = terms();
    t.salt[0] = 9;
    variants.push(t);
    let mut t = terms();
    t.near_vault = "other.testnet".into();
    variants.push(t);
    let mut t = terms();
    t.near_initiator = "other.testnet".into();
    variants.push(t);
    let mut t = terms();
    t.near_recipient = "other.testnet".into();
    variants.push(t);
    let mut t = terms();
    t.hashlock[0] ^= 1;
    variants.push(t);
    let mut t = terms();
    t.near_yocto += 1;
    variants.push(t);
    let mut t = terms();
    t.funding_deadline_ns += 1;
    variants.push(t);
    let mut t = terms();
    t.expiry_ns += 1;
    variants.push(t);
    for t in variants {
        assert!(compile_settlement_plan(SOURCE, SCOPE, NOW, &t).is_err());
    }
}
#[test]
fn no_silent_finality_or_unsupported_policy() {
    for source in [
        SOURCE.replace(", authenticated_finality", ""),
        SOURCE.replace("exact_code_identity, ", ""),
        SOURCE.replace("authenticated_finality", "atomic_rollback"),
        SOURCE.replace("\"claim\"", "\"not_funded\""),
        SOURCE.replace("parent none", "parent \"root\""),
        SOURCE.replace("depth 0", "depth 1"),
        SOURCE.replace("no_asset;", "asset \"BNB\";"),
    ] {
        assert!(
            compile_settlement_plan(&source, SCOPE, NOW, &terms()).is_err(),
            "{source}"
        );
    }
    let supported = BTreeSet::from([Requirement::ExactCodeIdentity, Requirement::AuthenticatedFinality]);
    assert!(compile_capability_source(SOURCE, SCOPE, &supported, NOW).is_err());
    assert!(compile_source(SOURCE).is_err());
    assert!(compile_settlement_plan(SOURCE, "other", NOW, &terms()).is_err());
    assert!(compile_settlement_plan(SOURCE, SCOPE, 1_700_001_500, &terms()).is_err());
}
#[test]
fn chain_identity_and_code_context_bind_plan_not_native_window() {
    let p = compile_settlement_plan(SOURCE, SCOPE, NOW, &terms()).unwrap();
    let mut t = terms();
    t.near_genesis = [7; 32];
    let q = compile_settlement_plan(SOURCE, SCOPE, NOW, &t).unwrap();
    assert_eq!(p.window_id, q.window_id);
    assert_ne!(p.plan_digest, q.plan_digest);
    let mut t = terms();
    t.near_vault_code_hash = [7; 32];
    let q = compile_settlement_plan(SOURCE, SCOPE, NOW, &t).unwrap();
    assert_eq!(p.window_id, q.window_id);
    assert_ne!(p.plan_digest, q.plan_digest);
    let q = compile_settlement_plan(&format!("// reviewed\n{SOURCE}"), SCOPE, NOW, &terms()).unwrap();
    assert_ne!(p.plan_digest, q.plan_digest);
    let mut t = terms();
    t.near_genesis = [0; 32];
    assert!(compile_settlement_plan(SOURCE, SCOPE, NOW, &t).is_err());
}
