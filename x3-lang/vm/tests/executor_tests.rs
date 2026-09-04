use x3_lang_common::capability::encode_capability_payload;
use x3_lang_common::CapabilityPayload;
use x3_lang_vm::executor::ExecError;
use x3_lang_vm::spec::opcodes::*;
use x3_lang_vm::{VMConfig, VM};

fn b52_bytecode(opcode: u8, payload: &[u8]) -> Vec<u8> {
    let len = payload.len() as u16;
    let mut code = vec![0x01]; // version header
    code.push(opcode);
    code.extend_from_slice(&len.to_le_bytes());
    code.extend_from_slice(payload);
    while code.len() % 4 != 0 {
        code.push(0);
    }
    code.extend_from_slice(&[0xFF, 0, 0, 0]); // HALT
    code
}

fn b52_execute(opcode: u8, payload: &CapabilityPayload, gas: u128) -> Result<VM, ExecError> {
    let payload_bytes = encode_capability_payload(payload).expect("encode should succeed");
    let code = b52_bytecode(opcode, &payload_bytes);
    let config = VMConfig::default();
    let mut vm = VM::new(code, config, gas);
    vm.execute()?;
    Ok(vm)
}

fn b52_execute_ok(opcode: u8, payload: &CapabilityPayload, gas: u128) -> VM {
    b52_execute(opcode, payload, gas).expect("execute should succeed")
}

#[test]
fn test_route_score_happy_path() {
    let vm = b52_execute_ok(
        ROUTE_SCORE,
        &CapabilityPayload::RouteScore {
            strategy: "fastest".into(),
            weights: vec![("speed".into(), 70u32), ("cost".into(), 30u32)],
        },
        100_000,
    );
    assert!(vm.state.gas < 100_000, "route score should consume gas");
}

#[test]
fn test_solver_bid_validates_bond() {
    let vm = b52_execute_ok(
        SOLVER_BID,
        &CapabilityPayload::SolverBid {
            solver: "solver1".into(),
            receive_asset: "USDC".into(),
            deliver_asset: "SOL".into(),
            fee: "0.1%".into(),
            bond: 10000,
        },
        100_000,
    );
    assert!(!vm.state.bridge_ops.is_empty(), "solver bid should add a bridge op");
    assert_eq!(
        vm.state.bridge_ops[0].amount, 10000,
        "bond should be stored in bridge op amount"
    );
}

#[test]
fn test_solver_bid_zero_bond_rejected() {
    let result = b52_execute(
        SOLVER_BID,
        &CapabilityPayload::SolverBid {
            solver: "solver1".into(),
            receive_asset: "USDC".into(),
            deliver_asset: "SOL".into(),
            fee: "0.1%".into(),
            bond: 0,
        },
        100_000,
    );
    assert!(result.is_err(), "zero bond should be rejected");
    match result.unwrap_err() {
        ExecError::Panic(msg) => assert!(msg.contains("bond"), "expected bond error, got: {msg}"),
        other => panic!("expected ExecError::Panic, got {other:?}"),
    }
}

#[test]
fn test_relayer_attest_quorum() {
    let vm = b52_execute_ok(
        RELAYER_ATTEST,
        &CapabilityPayload::RelayerAttest {
            relayers: vec!["relayer_a".into(), "relayer_b".into(), "relayer_c".into()],
            quorum_numerator: 3,
            quorum_denominator: 5,
            signatures: vec!["sig1".into(), "sig2".into(), "sig3".into()],
        },
        100_000,
    );
    assert!(vm.state.gas < 100_000, "relayer attest should consume gas");
}

#[test]
fn test_rpc_consensus_valid() {
    let vm = b52_execute_ok(
        RPC_CONSENSUS,
        &CapabilityPayload::RpcConsensus {
            chain: "arbitrum".into(),
            require_numerator: 2,
            require_denominator: 3,
            reject_on: vec!["receipt_disagree".into()],
        },
        100_000,
    );
    assert!(vm.state.gas < 100_000, "rpc consensus should consume gas");
}

#[test]
fn test_risk_score_happy_path() {
    let vm = b52_execute_ok(
        RISK_SCORE,
        &CapabilityPayload::RiskScore {
            score: 75,
            category: "medium".into(),
        },
        100_000,
    );
    assert!(vm.state.gas < 100_000, "risk score should consume gas");
}

#[test]
fn test_risk_score_over_100_rejected() {
    let result = b52_execute(
        RISK_SCORE,
        &CapabilityPayload::RiskScore {
            score: 150,
            category: "high".into(),
        },
        100_000,
    );
    assert!(result.is_err(), "risk score > 100 should be rejected");
    match result.unwrap_err() {
        ExecError::Panic(msg) => assert!(msg.contains("score"), "expected score error, got: {msg}"),
        other => panic!("expected ExecError::Panic, got {other:?}"),
    }
}

#[test]
fn test_proof_required_happy_path() {
    let vm = b52_execute_ok(
        PROOF_REQUIRED,
        &CapabilityPayload::ProofRequired {
            proof_type: "lock_proof".into(),
            source: "ethereum".into(),
        },
        100_000,
    );
    assert!(vm.state.gas < 100_000, "proof required should consume gas");
}

#[test]
fn test_vm_adapter_call_happy_path() {
    let vm = b52_execute_ok(
        VM_ADAPTER_CALL,
        &CapabilityPayload::VmAdapterCall {
            vm: "evm".into(),
            adapter: "evm_adapter".into(),
            calldata: "0xdeadbeef".into(),
        },
        100_000,
    );
    assert!(
        !vm.state.bridge_receipts.is_empty(),
        "should produce at least one receipt"
    );
}

#[test]
fn test_refund_policy_happy_path() {
    let vm = b52_execute_ok(
        REFUND_POLICY,
        &CapabilityPayload::RefundPolicy {
            action: "refund".into(),
            target: "sender".into(),
            after_blocks: 100,
        },
        100_000,
    );
    assert!(
        !vm.state.failure_handlers.is_empty(),
        "refund policy should register a failure handler"
    );
}

#[test]
fn test_privacy_commit_happy_path() {
    b52_execute_ok(
        PRIVACY_COMMIT,
        &CapabilityPayload::PrivacyCommit {
            reveal_on: "claim".into(),
            encrypted: true,
        },
        100_000,
    );
}

#[test]
fn test_mode_check_happy_path() {
    b52_execute_ok(
        MODE_CHECK,
        &CapabilityPayload::ModeCheck {
            mode: "mainnet".into(),
            restriction: "slippage<=5".into(),
        },
        100_000,
    );
}

#[test]
fn test_package_import_happy_path() {
    let vm = b52_execute_ok(
        PACKAGE_IMPORT,
        &CapabilityPayload::PackageImport {
            path: vec!["x3".into(), "std".into()],
            alias: Some("std".into()),
        },
        100_000,
    );
    assert!(vm.state.gas < 100_000, "package import should consume gas");
}

#[test]
fn test_invariant_check_happy_path() {
    let vm = b52_execute_ok(
        INVARIANT_CHECK,
        &CapabilityPayload::InvariantCheck {
            name: "no_double_claim".into(),
            assert_expr: "count(Release) <= 1".into(),
        },
        100_000,
    );
    assert!(vm.state.gas < 100_000, "invariant check should consume gas");
}

#[test]
fn test_route_score_zero_weight_rejected() {
    let result = b52_execute(
        ROUTE_SCORE,
        &CapabilityPayload::RouteScore {
            strategy: "empty".into(),
            weights: vec![],
        },
        100_000,
    );
    assert!(result.is_err(), "zero-weight route score should be rejected");
}

#[test]
fn test_relayer_attest_empty_relayers_rejected() {
    let result = b52_execute(
        RELAYER_ATTEST,
        &CapabilityPayload::RelayerAttest {
            relayers: vec![],
            quorum_numerator: 1,
            quorum_denominator: 1,
            signatures: vec!["sig1".into()],
        },
        100_000,
    );
    assert!(result.is_err(), "empty relayers should be rejected");
}

#[test]
fn test_relayer_attest_invalid_quorum_rejected() {
    let result = b52_execute(
        RELAYER_ATTEST,
        &CapabilityPayload::RelayerAttest {
            relayers: vec!["relayer_a".into()],
            quorum_numerator: 2,
            quorum_denominator: 1,
            signatures: vec!["sig1".into()],
        },
        100_000,
    );
    assert!(result.is_err(), "numerator > denominator should be rejected");
}

#[test]
fn test_privacy_commit_empty_reveal_on_rejected() {
    let result = b52_execute(
        PRIVACY_COMMIT,
        &CapabilityPayload::PrivacyCommit {
            reveal_on: "".into(),
            encrypted: false,
        },
        100_000,
    );
    assert!(result.is_err(), "empty reveal_on should be rejected");
}

#[test]
fn test_mode_check_empty_mode_rejected() {
    let result = b52_execute(
        MODE_CHECK,
        &CapabilityPayload::ModeCheck {
            mode: "".into(),
            restriction: "test".into(),
        },
        100_000,
    );
    assert!(result.is_err(), "empty mode should be rejected");
}

#[test]
fn test_package_import_empty_path_rejected() {
    let result = b52_execute(
        PACKAGE_IMPORT,
        &CapabilityPayload::PackageImport {
            path: vec![],
            alias: None,
        },
        100_000,
    );
    assert!(result.is_err(), "empty path should be rejected");
}

#[test]
fn test_solver_bid_empty_fee_rejected() {
    let result = b52_execute(
        SOLVER_BID,
        &CapabilityPayload::SolverBid {
            solver: "solver1".into(),
            receive_asset: "USDC".into(),
            deliver_asset: "SOL".into(),
            fee: "".into(),
            bond: 1000,
        },
        100_000,
    );
    assert!(result.is_err(), "empty fee should be rejected");
}

#[test]
fn test_rpc_consensus_empty_chain_rejected() {
    let result = b52_execute(
        RPC_CONSENSUS,
        &CapabilityPayload::RpcConsensus {
            chain: "".into(),
            require_numerator: 2,
            require_denominator: 3,
            reject_on: vec![],
        },
        100_000,
    );
    assert!(result.is_err(), "empty chain should be rejected");
}

#[test]
fn test_rpc_consensus_invalid_require_rejected() {
    let result = b52_execute(
        RPC_CONSENSUS,
        &CapabilityPayload::RpcConsensus {
            chain: "arbitrum".into(),
            require_numerator: 3,
            require_denominator: 2,
            reject_on: vec![],
        },
        100_000,
    );
    assert!(result.is_err(), "numerator > denominator should be rejected");
}

#[test]
fn test_risk_score_empty_category_rejected() {
    let result = b52_execute(
        RISK_SCORE,
        &CapabilityPayload::RiskScore {
            score: 50,
            category: "".into(),
        },
        100_000,
    );
    assert!(result.is_err(), "empty category should be rejected");
}

#[test]
fn test_invariant_check_empty_name_rejected() {
    let result = b52_execute(
        INVARIANT_CHECK,
        &CapabilityPayload::InvariantCheck {
            name: "".into(),
            assert_expr: "true".into(),
        },
        100_000,
    );
    assert!(result.is_err(), "empty name should be rejected");
}

#[test]
fn test_proof_required_empty_proof_type_rejected() {
    let result = b52_execute(
        PROOF_REQUIRED,
        &CapabilityPayload::ProofRequired {
            proof_type: "".into(),
            source: "ethereum".into(),
        },
        100_000,
    );
    assert!(result.is_err(), "empty proof_type should be rejected");
}

#[test]
fn test_proof_required_empty_source_rejected() {
    let result = b52_execute(
        PROOF_REQUIRED,
        &CapabilityPayload::ProofRequired {
            proof_type: "lock_proof".into(),
            source: "".into(),
        },
        100_000,
    );
    assert!(result.is_err(), "empty source should be rejected");
}

#[test]
fn test_vm_adapter_call_empty_vm_rejected() {
    let result = b52_execute(
        VM_ADAPTER_CALL,
        &CapabilityPayload::VmAdapterCall {
            vm: "".into(),
            adapter: "evm_adapter".into(),
            calldata: "0xdead".into(),
        },
        100_000,
    );
    assert!(result.is_err(), "empty vm should be rejected");
}

#[test]
fn test_vm_adapter_call_empty_adapter_rejected() {
    let result = b52_execute(
        VM_ADAPTER_CALL,
        &CapabilityPayload::VmAdapterCall {
            vm: "evm".into(),
            adapter: "".into(),
            calldata: "0xdead".into(),
        },
        100_000,
    );
    assert!(result.is_err(), "empty adapter should be rejected");
}

#[test]
fn test_refund_policy_empty_action_rejected() {
    let result = b52_execute(
        REFUND_POLICY,
        &CapabilityPayload::RefundPolicy {
            action: "".into(),
            target: "sender".into(),
            after_blocks: 100,
        },
        100_000,
    );
    assert!(result.is_err(), "empty action should be rejected");
}
