//! Genuine archived NEAR RPC proof integration tests. Proofs remain untrusted.
use serde_json::{json, Value};
use x3_lang_vm::near_evidence::{inspect_receipt_untrusted, NearReceiptContext};

const REAL: &str = include_str!("../../../evidence/near-proof/testnet/sha256/0f/0fb16971ff172bd30e6ada514a3c9ea688ac77815ea506f28be09b6b3512d5ab.json");
const RECEIPT: &str = "3f9FCzqC8k75AGob2UrCxbKVqADRyv5M5vGfoLEXbZyu";
const RECEIVER: &str = "league-v7.near-post.testnet";
const HEAD: &str = "V7KcHcg16BVWWxhVbw2vQj87D3dJQwXchnU7DCVzo6V";
const ROOT: &str = "6F2mBDUkzB8tRpETPpRTtH5zo1E5yGGpftfyNYbRVDVw";

fn context() -> NearReceiptContext {
    NearReceiptContext {
        receipt_id: RECEIPT.into(),
        receiver_id: RECEIVER.into(),
        light_client_head: HEAD.into(),
        untrusted_head_block_merkle_root: ROOT.into(),
    }
}

#[test]
fn real_near_receipt_outcome_and_block_merkle_paths_match_but_never_authenticate() {
    let observation = inspect_receipt_untrusted(REAL.as_bytes(), &context()).expect("real archived proof");
    assert_eq!(observation.receipt_id, RECEIPT);
    assert_eq!(observation.receiver_id, RECEIVER);
    assert_eq!(observation.block_hash, "DSu44uX9GmwyGXzUhAVK1DWJYGRaUBbfT2tNRGQjEd5i");
    assert_eq!(
        observation.block_outcome_root,
        "7mUR47bB4y7d7MbakknJGSdqmpCEDL5X47Y5LMq9BQnp"
    );
    assert!(observation.outcome_merkle_matches && observation.block_merkle_matches);
    assert!(!observation.authenticated);
    assert!(!observation.authorized_to_sign);
}

#[test]
fn corrupting_any_genuine_proof_branch_or_execution_state_fails_closed() {
    let pristine: Value = serde_json::from_str(REAL).unwrap();
    let mutations: [(&str, fn(&mut Value)); 6] = [
        ("gas", |doc: &mut Value| {
            doc["result"]["outcome_proof"]["outcome"]["gas_burnt"] = json!(1)
        }),
        ("receipt", |doc: &mut Value| {
            doc["result"]["outcome_proof"]["id"] = json!("11111111111111111111111111111111")
        }),
        ("receiver", |doc: &mut Value| {
            doc["result"]["outcome_proof"]["outcome"]["executor_id"] = json!("attacker.testnet")
        }),
        ("outcome branch", |doc: &mut Value| {
            doc["result"]["outcome_proof"]["proof"][0]["direction"] = json!("Right")
        }),
        ("block branch", |doc: &mut Value| {
            doc["result"]["block_proof"][0]["direction"] = json!("Right")
        }),
        ("lite outcome root", |doc: &mut Value| {
            doc["result"]["block_header_lite"]["inner_lite"]["outcome_root"] = json!("11111111111111111111111111111111")
        }),
    ];
    for (name, change) in mutations {
        let mut mutated = pristine.clone();
        change(&mut mutated);
        assert!(
            inspect_receipt_untrusted(&serde_json::to_vec(&mutated).unwrap(), &context()).is_err(),
            "accepted mutated {name}"
        );
    }
}

#[test]
fn a_matching_proof_with_wrong_caller_root_does_not_pass() {
    let mut wrong = context();
    wrong.untrusted_head_block_merkle_root = "11111111111111111111111111111111".into();
    assert!(inspect_receipt_untrusted(REAL.as_bytes(), &wrong).is_err());
}

#[test]
fn real_evidence_reaches_actual_x3vm_proof_host_opcode_but_cannot_authorize() {
    use x3_lang_common::{encode_capability_payload, CapabilityPayload};
    use x3_lang_vm::bridge::{
        EthereumLightClientVerifier, EvmProductionBridgeBackend, FileReceiptStore, ProductionBridgeAdapter,
    };
    use x3_lang_vm::executor::ExecError;
    use x3_lang_vm::spec::opcodes::*;
    use x3_lang_vm::{VMConfig, VM};

    let payload = CapabilityPayload::ProofVerify {
        kind: 3, // distinct read-only NEAR proof observation: NOT ZK, MPC or raw equality
        proof: REAL.to_string(),
        input: serde_json::to_string(&context()).unwrap(),
        key_or_threshold: "UNTRUSTED_HEAD_NO_SIGNING".into(),
    };
    let bytes = encode_capability_payload(&payload).unwrap();
    let mut code = vec![0x01, META_VERSIONS];
    for version in [
        LANGUAGE_VERSION,
        COMPILER_FORMAT_VERSION,
        IR_VERSION,
        VM_VERSION,
        POLICY_VERSION,
    ] {
        code.extend_from_slice(&version.to_le_bytes());
    }
    code.push(PROOF_VERIFY);
    code.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    code.extend_from_slice(&bytes);
    while code.len() % 4 != 0 {
        code.push(0);
    }
    code.extend_from_slice(&[0xFF, 0, 0, 0]);
    let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
    let dir = tempfile::tempdir().unwrap();
    vm.bridge = Box::new(ProductionBridgeAdapter::new(EvmProductionBridgeBackend::new(
        EthereumLightClientVerifier::new("0x0000000000000000000000000000000000000000000000000000000000000000"),
        FileReceiptStore::new(dir.path().join("receipts.jsonl")),
    )));
    // A malicious intent may install an ordinary fallback handler. A missing
    // trusted NEAR checkpoint must be fatal, not swallowed by a handler that
    // jumps straight to HALT and labels the run successful.
    let handler_pc = vm.code.len() - 4;
    vm.state.failure_handlers.push(handler_pc);
    let before_receipts = vm.state.bridge_receipts.len();
    match vm.execute() {
        Err(ExecError::Panic(message)) => assert!(
            message.contains("X3_NEAR_HEAD_UNTRUSTED"),
            "unexpected hostcall error: {message}"
        ),
        other => {
            panic!("never allow production bridge to treat an RPC-rooted proof as settlement authority: {other:?}")
        }
    }
    assert_eq!(
        vm.state.bridge_receipts.len(),
        before_receipts,
        "no settlement receipt may be emitted"
    );
}

#[test]
fn dry_run_bridge_must_not_falsely_claim_near_proof_verified() {
    use x3_lang_vm::bridge::{BridgeAdapter, DryRunBridge};
    assert!(DryRunBridge::default()
        .proof_verify(3, REAL.as_bytes(), b"{}", b"")
        .is_err());
}

#[test]
fn genuine_receipt_can_flow_from_x3_source_through_compiler_to_vm_without_authorizing_settlement() {
    use x3_lang_compiler::compile_source;
    use x3_lang_vm::executor::ExecError;
    use x3_lang_vm::{VMConfig, VM};

    // JSON string literals remain real X3Lang source: compiler must create
    // the typed near proof opcode and preserve the archived RPC evidence.
    let proof_literal = serde_json::to_string(REAL).unwrap();
    let context_literal = serde_json::to_string(&serde_json::to_string(&context()).unwrap()).unwrap();
    let source = format!(
        "fn observe_near_without_signing() {{ verify_near_receipt({proof_literal}, {context_literal}, \"UNTRUSTED_HEAD_NO_SIGNING\"); }}"
    );
    let bytecode = compile_source(&source).expect("actual NEAR .x3 source compiles into X3VM bytecode");
    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    match vm.execute() {
        Err(ExecError::Panic(message)) => assert!(
            message.contains("X3_NEAR_HEAD_UNTRUSTED"),
            "unexpected source-to-VM rejection: {message}"
        ),
        other => panic!("source-compiled proof must never grant settlement authority: {other:?}"),
    }
    assert!(vm.state.bridge_receipts.is_empty());
}
