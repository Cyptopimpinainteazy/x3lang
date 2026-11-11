//! A configured production bridge is not a mandate/intent resolver.
//! Exercise real compiled source against a local counting backend; no RPC,
//! wallet, signature, transfer or deployed-chain operation is performed.

use std::cell::Cell;
use std::path::PathBuf;
use std::rc::Rc;

use x3_lang_compiler::compile_source;
use x3_lang_vm::bridge::{
    BridgeError, BridgeTransferRequest, ProductionBridgeAdapter, ProductionBridgeBackend, SettlementReceipt,
};
use x3_lang_vm::{BackendMode, BridgeConfig, InstructionStream, VMConfig, VM};

#[derive(Clone, Default)]
struct EffectCounters {
    finality: Rc<Cell<usize>>,
    proof: Rc<Cell<usize>>,
    receipt: Rc<Cell<usize>>,
}

fn unexpected_effect() -> BridgeError {
    BridgeError {
        code: "X3_TEST_UNEXPECTED_BRIDGE_EFFECT",
        message: "unresolved intent must be refused before bridge effects".into(),
    }
}

impl ProductionBridgeBackend for EffectCounters {
    fn verify_source_finality(&self, _request: &BridgeTransferRequest) -> Result<Vec<u8>, BridgeError> {
        self.finality.set(self.finality.get() + 1);
        Err(unexpected_effect())
    }

    fn verify_transfer_proof(
        &self,
        _request: &BridgeTransferRequest,
        _finality_proof: &[u8],
    ) -> Result<Vec<u8>, BridgeError> {
        self.proof.set(self.proof.get() + 1);
        Err(unexpected_effect())
    }

    fn persist_receipt(&self, _receipt: &SettlementReceipt) -> Result<(), BridgeError> {
        self.receipt.set(self.receipt.get() + 1);
        Err(unexpected_effect())
    }
}

fn assert_unresolved_mandate_has_no_bridge_effects(example: &str) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../examples")
        .join(example);
    let source = std::fs::read_to_string(path).expect("canonical mandate-bound example");
    let bytecode = compile_source(&source).expect("source may compile without granting authority");
    x3_lang_vm::verifier::verify(&InstructionStream::new(bytecode.clone())).expect("valid bytecode");

    let calls = EffectCounters::default();
    let config = BridgeConfig {
        mode: BackendMode::Production,
        adapter: Some(Box::new(ProductionBridgeAdapter::new(calls.clone()))),
    };
    let mut vm = VM::with_bridge(bytecode, VMConfig::default(), 100_000, config).expect("local configured backend");
    let error = vm
        .execute()
        .expect_err("a production bridge cannot authorize unresolved mandate constraints");
    let diagnostic = format!("{error:?}");
    assert!(
        diagnostic.contains("X3_FEATURE_NOT_AVAILABLE") && diagnostic.contains("solver marketplace"),
        "the refusal must identify the missing intent resolver: {diagnostic}"
    );
    assert_eq!(
        calls.finality.get(),
        0,
        "no finality request before authority resolution"
    );
    assert_eq!(calls.proof.get(), 0, "no proof request before authority resolution");
    assert_eq!(
        calls.receipt.get(),
        0,
        "no receipt persisted before authority resolution"
    );
    assert!(
        vm.state.asset_ops.is_empty(),
        "no asset operation before authority resolution"
    );
    assert!(
        vm.state.bridge_ops.is_empty(),
        "no bridge operation before authority resolution"
    );
    assert!(
        vm.state.bridge_receipts.is_empty(),
        "no settlement receipt before authority resolution"
    );
}

#[test]
fn payment_intent_requires_a_resolver_before_production_bridge_effects() {
    assert_unresolved_mandate_has_no_bridge_effects("near_intents_agent_payment.x3");
}

#[test]
fn denied_intent_cannot_use_a_production_bridge_as_mandate_authority() {
    assert_unresolved_mandate_has_no_bridge_effects("near_intents_agent_denied.x3");
}
