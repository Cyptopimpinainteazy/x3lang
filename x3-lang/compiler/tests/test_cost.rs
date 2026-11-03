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

/// A trading program is *not* silently skipped by the profitability analysis.
///
/// Its policy states a profit floor (`minimum_net_profit`) together with cost
/// ceilings (`max_gas`, `max_flash_fee`), and a ceiling is not a lower bound on what
/// a trade pays — so the analysis cannot claim anything about the floor, and it says
/// so. The message must describe *that* program, not say it declares nothing.
#[test]
fn a_trading_program_is_described_rather_than_skipped() {
    let source = "asset USDC = evm.ethereum.0xA0b8 { decimals: 6 }\n\
                  asset ETH = evm.ethereum.0x0000 { decimals: 18 }\n\n\
                  risk policy P {\n    max_slippage: 30 bps\n    max_gas: 0.02 ETH\n    \
                  max_flash_fee: 10 bps\n    deadline: 2 blocks\n    \
                  require_private_submission: false\n}\n\n\
                  atomic trade CostProbe using P {\n    let out = swap 100 USDC -> ETH via \
                  uniswap_v3 min_out 1 ETH\n    require net_profit >= 1 USDC\n    \
                  require all_debts_repaid\n    emit receipt\n}\n";
    let program = x3_lang_compiler::parser::parse_source(source).expect("the trading fixture parses");
    let x3_lang_compiler::profitability::Verdict::NotAnalysed(reason) =
        x3_lang_compiler::profitability::analyse(&program)
    else {
        panic!("a trading program's ceilings are not floors");
    };
    assert!(
        reason.contains("ceilings") && reason.contains("not a lower bound"),
        "the analysis must say why it cannot compare: {reason}"
    );
}
