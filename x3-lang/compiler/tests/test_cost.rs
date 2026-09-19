//! The compiler cost model (PHASE 35).
//
//
// The estimate has to be the same number the VM charges, or it is a second
// opinion about cost. These tests pin the two properties that make it trustworthy:
// the weight is the sum of the shared table over the emitted instructions, and the
// components the compiler cannot estimate are named rather than guessed.

#[test]
fn the_estimate_weighs_the_instructions_with_the_table_the_vm_charges_from() {
    let source = "intent cost_probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        \
                  swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    require \
                  slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}\n";
    let bytecode = x3_lang_compiler::compile_source(source).expect("compiles");
    let estimate = x3_lang_compiler::cost::estimate_artifact(&bytecode).expect("estimates");

    // Recompute the weight from the disassembly, independently of the estimate.
    let walked = x3_lang_compiler::emitter::instructions(&bytecode).expect("walks");
    let expected: u128 = walked
        .iter()
        .map(|instruction| x3_lang_compiler::spec::opcodes::base_gas_cost(instruction.opcode))
        .sum();
    assert_eq!(estimate.base_weight, expected, "{estimate:?}");
    assert_eq!(estimate.instruction_count, walked.len(), "{estimate:?}");
    assert_eq!(estimate.artifact_bytes, bytecode.len(), "{estimate:?}");

    // The report states every figure's basis, and names what it will not guess.
    let report = estimate.report();
    assert!(report.contains("base weight"), "{report}");
    assert!(report.contains("the table vm/src/executor.rs charges from"), "{report}");
    for component in ["EVM gas", "SVM compute", "cross-domain latency", "expected fees"] {
        assert!(report.contains(component), "{component} must be named: {report}");
    }
    assert!(report.contains("Not estimated, and why"), "{report}");
}

#[test]
fn the_estimate_counts_payload_bytes_and_host_facing_instructions() {
    // A bridging program: one BRIDGE (host-facing, and the only frame that carries
    // proof bytes) and payload-carrying asset frames.
    let source = "finality_policy strict {\n    chain ethereum\n    requirement finalized\n    blocks \
                  12\n}\n\nproofs required {\n    source_lock_proof\n    destination_fill_proof\n}\n\nintent \
                  cost_bridge {\n    from ethereum.USDC amount 100 receiver 0x1\n    to solana.SOL \
                  receiver 0x2\n    route {\n        bridge X3 ethereum.USDC -> solana.SOL receiver \
                  0x2\n    }\n    require nonce unused cost_bridge_1\n    require finality.ethereum \
                  >= 12\n    timeout 30 refund ethereum.USDC to sender\n    on_fail rollback\n}\n";
    let bytecode = x3_lang_compiler::compile_source(source).expect("compiles");
    let estimate = x3_lang_compiler::cost::estimate_artifact(&bytecode).expect("estimates");

    assert_eq!(estimate.host_facing, 1, "{estimate:?}");
    assert!(estimate.payload_bytes > 0, "{estimate:?}");
    // The artifact carries empty proof fields in this fixture; the figure counts
    // what is *there*, not what a chain will ask for.
    assert_eq!(estimate.proof_bytes, 0, "{estimate:?}");
    assert!(
        estimate.per_opcode.iter().any(|entry| entry.name == "BRIDGE"),
        "the bridge must appear in the histogram: {estimate:?}"
    );
}
