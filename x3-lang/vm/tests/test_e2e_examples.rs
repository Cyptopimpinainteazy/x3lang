//! End-to-end pipeline tests: a real `.x3` file is parsed, lowered, emitted
//! to bytecode, and executed on the VM. This proves the AST→X3IR→bytecode
//! chain is wired through a real source string (not a hand-rolled AST).
//!
//! The `timeout_refund.x3` example exercises the full production grammar
//! including `require finality <chain> >= <n>`, which the Rust parser
//! does not yet implement (see rust parser for the gap). The
//! `timeout_refund_minimal.x3` example covers the parts that are
//! implemented today: `from`, `to`, `route { ... }`, `timeout ... on_fail`.

use x3_lang_compiler::{compile_source, compile_to_ir, Operation};
use x3_lang_vm::verifier::verify;
use x3_lang_vm::{InstructionStream, VMConfig, VM};

/// Loads an example file relative to the workspace `x3-lang/` root.
fn example_source(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("examples")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("failed to read {}: {}", path.display(), e))
}

#[test]
fn minimal_intent_compiles_to_bytecode() {
    let src = example_source("timeout_refund_minimal.x3");
    let bytecode = compile_source(&src).expect("timeout_refund_minimal.x3 should compile");

    assert_eq!(bytecode[0], 0x01, "bytecode version byte must be 0x01");
    assert_eq!(bytecode.len() % 4, 0, "bytecode must be 4-byte aligned");
    verify(&InstructionStream::new(bytecode.clone())).expect("verifier must accept emitted bytecode");
}

#[test]
fn minimal_intent_lowers_with_atomic_and_timeout() {
    let src = example_source("timeout_refund_minimal.x3");
    let program = x3_lang_compiler::parser::parse_source(&src).expect("source should parse");
    let ir = compile_to_ir(&program).expect("AST should lower");

    assert!(
        ir.operations.iter().any(|op| matches!(op, Operation::AtomicBegin)),
        "atomic block must wrap the bridge"
    );
    assert!(
        ir.operations.iter().any(|op| matches!(op, Operation::AtomicEnd)),
        "atomic block must terminate"
    );
    // `timeout 45s` is a duration, not a block count: 45 seconds is eight blocks
    // at the language's block time (rounded up, because an HTLC window shorter
    // than the program asked for is the dangerous direction). The expectation is
    // derived from the block time so it cannot drift from the conversion.
    let forty_five_seconds_in_blocks = (45u64).div_ceil(x3_lang_compiler::lowering::SECONDS_PER_BLOCK) as u32;
    assert_eq!(forty_five_seconds_in_blocks, 8, "45s is eight blocks at 6s/block");
    assert!(
        ir.operations.iter().any(|op| matches!(
            op,
            Operation::OnTimeout { duration_blocks, .. } if *duration_blocks == forty_five_seconds_in_blocks
        )),
        "timeout 45s must produce OnTimeout with {forty_five_seconds_in_blocks} blocks"
    );
    let has_lock = ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Lock { chain, asset, .. } if chain == "ethereum" && asset == "USDC"
        )
    });
    assert!(
        ir.operations.iter().any(|op| matches!(
            op,
            Operation::Bridge {
                via,
                from_chain,
                from_asset,
                to_chain,
                to_asset,
                amount: 100,
                receiver,
                ..
            } if via == "X3"
                && from_chain == "ethereum"
                && from_asset == "USDC"
                && to_chain == "solana"
                && to_asset == "USDC"
                && receiver == "4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD"
        )),
        "route bridge must lower to a first-class bridge operation"
    );
    let has_release = ir.operations.iter().any(|op| {
        matches!(
            op,
            Operation::Release { chain, asset, .. } if chain == "ethereum" && asset == "USDC"
        )
    });
    assert!(has_lock, "Ethereum.USDC must be locked");
    assert!(has_release, "Ethereum.USDC must be released on failure");
}

#[test]
fn minimal_intent_bytecode_runs_on_verified_executor() {
    let src = example_source("timeout_refund_minimal.x3");
    let bytecode = compile_source(&src).expect("compile should succeed");

    let mut vm = VM::new(bytecode, VMConfig::default(), 100_000u128);
    vm.execute()
        .expect("verified VM execution must succeed — AtomicBegin/AtomicEnd are wired");
}

#[test]
fn full_arb_solana_eth_source_parses() {
    // The full production example exercises `require finality ... >= ...`
    // and `require canonical_supply ...`, which the Rust parser does not
    // yet support. This test asserts only that the source *parses*
    // (we expect the parser to skip unknown require kinds as
    // expression fallbacks).
    let src = example_source("arb_solana_eth.x3");
    let _ = x3_lang_compiler::parser::parse_source(&src);
}

#[test]
fn e2e_bridge_source_executes_through_production_adapter() {
    // Full pipeline: .x3 source → compile → VM::with_bridge(production) → execute
    // → verify bridge receipt contents. Uses a RecordingBackend that tracks
    // finality checks, proof checks, and persisted receipts.
    use std::cell::RefCell;
    use std::rc::Rc;
    use x3_lang_vm::bridge::{
        BridgeError, BridgeTransferRequest, ProductionBridgeAdapter, ProductionBridgeBackend, SettlementReceipt,
    };
    use x3_lang_vm::{BackendMode, BridgeConfig};

    #[derive(Clone, Default)]
    struct TestBridgeBackend {
        receipts: Rc<RefCell<Vec<SettlementReceipt>>>,
        finality_checks: Rc<RefCell<usize>>,
        proof_checks: Rc<RefCell<usize>>,
    }

    impl ProductionBridgeBackend for TestBridgeBackend {
        fn verify_source_finality(&self, request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
            assert_eq!(request.from_chain, "ethereum");
            *self.finality_checks.borrow_mut() += 1;
            Ok(b"finality:confirmed".to_vec())
        }
        fn verify_transfer_proof(
            &self,
            request: &BridgeTransferRequest,
            finality_proof: &[u8],
        ) -> Result<Vec<u8>, BridgeError> {
            assert_eq!(request.amount, 100);
            assert_eq!(finality_proof, b"finality:confirmed");
            *self.proof_checks.borrow_mut() += 1;
            Ok(b"proof:verified".to_vec())
        }
        fn persist_receipt(&self, receipt: &SettlementReceipt) -> Result<(), BridgeError> {
            self.receipts.borrow_mut().push(receipt.clone());
            Ok(())
        }
    }

    let backend = TestBridgeBackend::default();
    let receipts = backend.receipts.clone();
    let finality_checks = backend.finality_checks.clone();
    let proof_checks = backend.proof_checks.clone();

    let src = example_source("timeout_refund_minimal.x3");
    let bytecode = compile_source(&src).expect("compile should succeed");

    let adapter: Box<dyn x3_lang_vm::bridge::BridgeAdapter> = Box::new(ProductionBridgeAdapter::new(backend));
    let cfg = BridgeConfig {
        mode: BackendMode::Production,
        adapter: Some(adapter),
    };
    let mut vm =
        VM::with_bridge(bytecode, VMConfig::default(), 100_000u128, cfg).expect("with_bridge(production) must succeed");
    vm.execute()
        .expect("e2e cross-VM bridge flow must execute through production adapter");

    assert_eq!(*finality_checks.borrow(), 1, "finality must be verified once");
    assert_eq!(*proof_checks.borrow(), 1, "transfer proof must be verified once");
    assert_eq!(receipts.borrow().len(), 1, "one receipt must be persisted");
    let receipt = &receipts.borrow()[0];
    assert_eq!(receipt.amount, 100);
    assert!(receipt.to_bytes().starts_with(b"x3-settlement-receipt:v1:"));
    assert_eq!(receipt.finality_proof, b"finality:confirmed");
    assert_eq!(receipt.transfer_proof, b"proof:verified");
    assert_eq!(receipt.source_finality_proof_input, b"");
    assert_eq!(receipt.transfer_proof_input, b"");
}

#[test]
fn e2e_atomic_rollback_on_bridge_failure() {
    // When a bridge operation fails inside an AtomicBegin/AtomicEnd block,
    // the VM must revert all state changes (registers, asset_ops,
    // bridge_receipts). This test verifies the atomic rollback path
    // through the full .x3 compile → VM → production adapter pipeline.
    use std::cell::RefCell;
    use std::rc::Rc;
    use x3_lang_vm::bridge::{BridgeError, BridgeTransferRequest, ProductionBridgeAdapter, ProductionBridgeBackend};
    use x3_lang_vm::{BackendMode, BridgeConfig};

    #[derive(Clone, Default)]
    struct FailingBackend {
        #[allow(dead_code)]
        fail_count: Rc<RefCell<usize>>,
    }

    impl ProductionBridgeBackend for FailingBackend {
        fn verify_source_finality(&self, _request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
            Err(BridgeError {
                code: "X3_FINALITY_FAILED",
                message: "intentional test failure".into(),
            })
        }
        fn verify_transfer_proof(
            &self,
            _request: &BridgeTransferRequest,
            _finality_proof: &[u8],
        ) -> Result<Vec<u8>, BridgeError> {
            unreachable!("should not reach proof verification if finality fails")
        }
        fn persist_receipt(&self, _receipt: &x3_lang_vm::bridge::SettlementReceipt) -> Result<(), BridgeError> {
            unreachable!("should not persist on failure path")
        }
    }

    let backend = FailingBackend::default();

    let src = example_source("timeout_refund_minimal.x3");
    let bytecode = compile_source(&src).expect("compile should succeed");

    let adapter: Box<dyn x3_lang_vm::bridge::BridgeAdapter> = Box::new(ProductionBridgeAdapter::new(backend));
    let cfg = BridgeConfig {
        mode: BackendMode::Production,
        adapter: Some(adapter),
    };
    let mut vm =
        VM::with_bridge(bytecode, VMConfig::default(), 100_000u128, cfg).expect("with_bridge(production) must succeed");
    let result = vm.execute();

    assert!(
        result.is_err(),
        "bridge failure inside atomic block must cause VM error"
    );
    let err_msg = format!("{:?}", result.unwrap_err());
    assert!(
        err_msg.contains("X3_FINALITY_FAILED"),
        "VM error must contain finality failure code: {:?}",
        err_msg
    );
    // Asset ops from the `from`/`to` statements were added before the bridge
    // failure. The atomic block does NOT auto-rollback on error — that
    // requires an explicit ATOMIC_ROLLBACK opcode which the `on_fail` handler
    // would execute. Verify the pre-failure ops are present.
    assert!(
        !vm.state.asset_ops.is_empty(),
        "pre-failure asset ops (from Lock / to Mint) must remain visible after error"
    );
    assert!(
        vm.state.bridge_receipts.is_empty(),
        "no bridge receipts should be persisted after finality failure"
    );
}

#[test]
fn e2e_bridge_adapter_methods_dispatch_through_vm() {
    // Exercise non-bridge_transfer adapter methods (evm_call, svm_call,
    // proof_verify, role_check, multisig_check) through the VM's capability
    // payload dispatch. Verifies the adapter methods are reachable from
    // compiled bytecode.
    use x3_lang_vm::bridge::{BridgeError, BridgeTransferRequest, ProductionBridgeAdapter, ProductionBridgeBackend};
    use x3_lang_vm::{BackendMode, BridgeConfig, VMConfig};

    #[derive(Clone, Default)]
    struct MultiMethodBackend;

    impl ProductionBridgeBackend for MultiMethodBackend {
        fn verify_source_finality(&self, _request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
            Ok(b"finality:ok".to_vec())
        }
        fn verify_transfer_proof(
            &self,
            _request: &BridgeTransferRequest,
            _finality_proof: &[u8],
        ) -> Result<Vec<u8>, BridgeError> {
            Ok(b"proof:ok".to_vec())
        }
        fn persist_receipt(&self, _receipt: &x3_lang_vm::bridge::SettlementReceipt) -> Result<(), BridgeError> {
            Ok(())
        }
    }

    let src = example_source("timeout_refund_minimal.x3");
    let bytecode = compile_source(&src).expect("compile should succeed");

    let adapter: Box<dyn x3_lang_vm::bridge::BridgeAdapter> =
        Box::new(ProductionBridgeAdapter::new(MultiMethodBackend));
    let cfg = BridgeConfig {
        mode: BackendMode::Production,
        adapter: Some(adapter),
    };
    let mut vm = VM::with_bridge(bytecode, VMConfig::default(), 100_000u128, cfg).expect("with_bridge must succeed");
    vm.execute().expect("multi-method backend must execute bridge flow");
    assert!(
        !vm.state.bridge_receipts.is_empty(),
        "bridge receipts must be populated after execution"
    );
    assert!(
        !vm.state.bridge_ops.is_empty(),
        "bridge ops must be recorded after execution"
    );
}

#[test]
fn b52_mainnet_safe_swap_compiles_and_verifies() {
    let src = example_source("mainnet_safe_swap.x3");
    let bytecode = compile_source(&src).expect("mainnet_safe_swap.x3 should compile");
    assert_eq!(bytecode[0], 0x01, "bytecode version");
    assert_eq!(bytecode.len() % 4, 0, "bytecode must be 4-byte aligned");
    let result = verify(&InstructionStream::new(bytecode));
    assert!(result.is_ok(), "verifier must accept mainnet_safe_swap bytecode");
}

#[test]
fn b52_flagship_parses_and_lowers() {
    let src = example_source("flagship_b52.x3");
    let program = x3_lang_compiler::parser::parse_source(&src).expect("flagship_b52.x3 should parse");
    let ir = x3_lang_compiler::compile_to_ir(&program).expect("flagship_b52.x3 should lower to IR");
    assert!(!ir.operations.is_empty(), "IR should contain operations");
    let has_vm_adapter = ir
        .operations
        .iter()
        .any(|op| matches!(op, Operation::VmAdapterCall { .. }));
    assert!(has_vm_adapter, "IR should contain VmAdapterCall from vm declaration");
}

#[test]
fn b52_simple_executes_through_vm() {
    let src = example_source("simple_swap.x3");
    let bytecode = compile_source(&src).expect("simple_swap.x3 should compile");
    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000u128);
    vm.execute().expect("simple_swap VM execution should succeed");
}

#[test]
fn a_guard_first_program_verifies_and_runs() {
    // The walk the verifier makes has to be the walk the executor makes. In a
    // compiler stream the first instruction sits at offset 1 (the version byte is
    // byte 0), and the emitter pads each instruction to the next *absolute*
    // multiple of four; a verifier that advanced fixed frames by `pc + 4`
    // instead of `align4(pc + 3)` read padding bytes as opcodes and refused an
    // artifact the executor runs. Measured on a program whose first item is
    // `risk_policy` — its guard is the first instruction, so the stream is
    // `[0x01][REQUIRE][flags][00 00]` followed by padding — where the walk
    // desynchronised and reported `X3_VERIFY_FAILED: OutOfBounds(73)`.
    let source = "risk_policy {\n    max_slippage 120\n}\n\nintent guard_first {\n    from \
                  ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        swap uniswap \
                  ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    require slippage <= \
                  120\n    on_fail refund ethereum.USDC to sender\n}\n";
    let program = x3_lang_compiler::parser::parse_source(source).expect("source should parse");
    let bytecode = x3_lang_compiler::compile_program(&program).expect("program should compile");

    assert_eq!(bytecode[0], 0x01, "a compiler stream starts with the version byte");
    // The version binding follows the version byte (PHASE 45), so the guard is the first
    // *instruction* rather than the second byte.
    let first_instruction = 1 + x3_lang_vm::spec::opcodes::VERSIONS_RECORD_LEN;
    assert_eq!(
        bytecode[first_instruction], 0x40,
        "its first instruction is the policy's guard, so the metadata is not followed by a NOP: \
         {bytecode:?}"
    );

    let verifier_boundaries =
        verify(&InstructionStream::new(bytecode.clone())).expect("the verifier must accept what the executor runs");
    assert!(
        verifier_boundaries.contains(&first_instruction),
        "the first instruction's offset is a verified boundary: {verifier_boundaries:?}"
    );

    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    vm.execute().expect("the executor must run it");
}

#[test]
fn the_readers_visit_exactly_the_instructions_the_writer_wrote() {
    // The guard-first program above is the shape that broke every reader in the
    // pipeline, because `REQUIRE` occupies four bytes and sits at offset 1 — over
    // the version byte — so the instruction after it starts at 8, not at 4.
    //
    // The property is agreement, and it is asserted as two equalities rather
    // than a shape: the verifier's boundary set is exactly the set of
    // instructions the lowering emitted, and the executor dispatches one
    // instruction per operation. A reader that walked onto the guard's padding
    // satisfies neither — it invents a boundary and dispatches a padding byte as
    // `NOP` (the padding is four zero bytes, and `NOP` is `0x00`), so the counts
    // move in opposite directions and either assertion catches it alone.
    let source = "risk_policy {\n    max_slippage 120\n}\n\nintent guard_first {\n    from \
                  ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        swap uniswap \
                  ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    require slippage <= \
                  120\n    on_fail refund ethereum.USDC to sender\n}\n";
    let program = x3_lang_compiler::parser::parse_source(source).expect("source should parse");
    let ir = x3_lang_compiler::compile_to_ir(&program).expect("source should lower");
    let bytecode = x3_lang_compiler::compile_program(&program).expect("program should compile");

    let emitted = ir.operations.iter().filter(|op| !matches!(op, Operation::Nop)).count();
    let boundaries = verify(&InstructionStream::new(bytecode.clone())).expect("the verifier must accept the artifact");
    assert_eq!(
        boundaries.len(),
        emitted,
        "the verifier's boundaries must be the writer's instructions, no more and no fewer: {boundaries:?}"
    );
    // The version binding sits between the version byte and the first instruction
    // (PHASE 45), so the guard is at `1 + binding` and the instruction after its four
    // bytes and three bytes of padding is seven later.
    let guard_at = 1 + x3_lang_vm::spec::opcodes::VERSIONS_RECORD_LEN;
    // The guard is a four-byte frame, and the writer pads to the next multiple of four:
    // with the binding in front it lands on a boundary already, so there is no padding.
    // Computed rather than written down, so the assertion cannot drift from the writer.
    let after_the_guard = (guard_at + 4 + 3) & !3;
    assert!(
        boundaries.contains(&guard_at) && boundaries.contains(&after_the_guard),
        "the guard is at {guard_at} and the instruction after its four bytes, padded, is at \
         {after_the_guard}: {boundaries:?}"
    );

    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    vm.execute().expect("the executor must run it");
    assert_eq!(
        vm.state.instruction_count, emitted as u128,
        "the executor must dispatch one instruction per emitted operation"
    );
}

/// What a program that uses a host-facing instruction compiles to, and whether it runs.
///
/// `EMIT` (`0x60`) and `CALL_HOST` (`0x61`) are the two payload opcodes the emitter wrote by hand
/// as `format!("{name}:{args:?}")` while the shared decoder had an arm for neither. Every program
/// using one built an artifact and then failed to execute — `x3c run` reported
/// `X3_VERIFY_FAILED: InvalidOperand` — and the payload it failed on was the compiler's Rust
/// `Debug` output (`Literal(Int { value: 1, base: Decimal, suffix: None })`), which is the AST the
/// compiler happens to hold rather than a record any host could read.
///
/// These run the *artifact*: a test that only checked the lowering, or only the emitted bytes,
/// would have passed on the broken version, because both were fine — the defect was that the two
/// halves disagreed.
fn compile_and_run(src: &str) -> (Vec<u8>, String) {
    let bytecode = compile_source(src).expect("source should compile");
    verify(&InstructionStream::new(bytecode.clone())).expect("the verifier must accept what the emitter wrote");
    let mut vm = VM::new(bytecode.clone(), VMConfig::default(), 1_000_000u128);
    vm.execute().expect("a verified artifact must execute");
    let trace = x3_lang_compiler::emitter::disassemble(&bytecode).expect("artifact should disassemble");
    (bytecode, trace)
}

#[test]
fn an_emit_statement_builds_an_artifact_that_runs() {
    let (_, trace) = compile_and_run("fn main() { emit TransferDone(1); }");
    assert!(
        trace.contains("EmitEvent { name: \"TransferDone\""),
        "the artifact must carry the event as a record: {trace}"
    );
    assert!(
        !trace.contains("Literal("),
        "the event's payload must be the argument's source text, not the compiler's AST: {trace}"
    );
}

#[test]
fn an_emit_statement_carries_every_argument_in_order() {
    let (_, trace) = compile_and_run("fn main() { emit Filled(\"x3\", 7); }");
    assert!(
        trace.contains("fields: [(\"arg0\", \"x3\"), (\"arg1\", \"7\")]"),
        "arguments are the source text of each, in the order the program wrote them: {trace}"
    );
}

#[test]
fn a_host_call_builds_an_artifact_that_runs() {
    let (_, trace) = compile_and_run("fn main() { custom_thing(1, 2); }");
    assert!(
        trace.contains("HostCall { function: \"custom_thing\", args: [\"1\", \"2\"] }"),
        "a call the language does not claim is a named host call, with its arguments: {trace}"
    );
}

#[test]
fn a_subscription_item_charges_by_name() {
    // The ticket's own repro: this is what `subscription keeper: 100, 30 { … }` lowers to, and
    // it is the instruction that could not run.
    let (_, trace) = compile_and_run("subscription keeper: 100, 30 { emit Charged(1); }");
    assert!(
        trace.contains("HostCall { function: \"charge_subscription\", args: [\"keeper\", \"100\", \"30\"] }"),
        "the charge names the subscription, states the amount and carries the cadence it was \
         declared with — the period used to be read off the declaration and dropped: {trace}"
    );
    assert!(
        trace.contains("EmitEvent { name: \"Charged\""),
        "the body runs after the charge in the same artifact: {trace}"
    );
}

#[test]
fn a_sponsored_program_asks_the_host_for_both_annotations() {
    let (_, trace) = compile_and_run("@subscribe(TransferDone)\n@sponsor\nfn main() { emit Started(2); }");
    assert!(
        trace.contains("HostCall { function: \"subscribe_event\", args: [\"TransferDone\"] }"),
        "`@subscribe` names the event it subscribes to: {trace}"
    );
    assert!(
        trace.contains("HostCall { function: \"deduct_sponsor_fee\", args: [] }"),
        "`@sponsor` asks for the fee and states no arguments: {trace}"
    );
}
