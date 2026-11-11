//! A simulation cannot authorize an external mandate it cannot inspect.
use x3_lang_vm::bridge::{BridgeAdapter, DryRunBridge};
use x3_lang_vm::{VMConfig, VM};

#[test]
fn dry_run_refuses_mandates_in_any_constraint_position() {
    let adapter = DryRunBridge::default();
    for constraints in [
        b"under mandate x3lang.near".as_slice(),
        b"under mandate x3lang.near\0slippage <= 90",
        b"slippage <= 90\0under mandate x3lang.near",
        b"\0under mandate x3lang.near\0",
    ] {
        let error = adapter
            .intent_resolve(constraints)
            .expect_err("a dry run has no mandate authority");
        assert!(error.to_string().contains("X3_FEATURE_NOT_AVAILABLE"));
    }
    for constraints in [b"".as_slice(), b"slippage <= 90", b"slippage <= 90\0fees <= 100 bps"] {
        adapter
            .intent_resolve(constraints)
            .expect("unbound simulation remains available");
    }
}

#[test]
fn exact_payment_source_is_refused_before_any_effects() {
    let source = include_str!("../../tests/fixtures/near_intents_named_nonce_payment.x3");
    let program = x3_lang_compiler::parser::parse_source(source).unwrap();
    let bytecode = x3_lang_compiler::compile_program(&program).unwrap();
    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    vm.report_outcome(None, Some(90), None);
    let error = vm
        .execute()
        .expect_err("generic dry run must not authorize this payment");
    assert!(format!("{error:?}").contains("X3_FEATURE_NOT_AVAILABLE"));
    assert!(vm.state.asset_ops.is_empty());
    assert!(vm.state.bridge_ops.is_empty());
    assert!(vm.state.bridge_receipts.is_empty());
    assert!(vm.state.used_nonces.is_empty());
}
