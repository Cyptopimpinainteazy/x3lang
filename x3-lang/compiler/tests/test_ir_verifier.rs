use std::collections::HashMap;

use x3_lang_compiler::diagnostic::DiagnosticCode;
use x3_lang_compiler::ir::{FailureAction, Operation, ProgramMetadata, X3IR};
use x3_lang_compiler::verify::verify_ir;

fn codes(ir: &X3IR) -> Vec<DiagnosticCode> {
    verify_ir(ir)
        .err()
        .unwrap_or_default()
        .into_iter()
        .map(|diagnostic| diagnostic.code)
        .collect()
}

fn ir_with(operations: Vec<Operation>) -> X3IR {
    X3IR {
        operations,
        metadata: ProgramMetadata {
            nonce: Some("nonce-1".to_owned()),
            chain_id: Some(1),
            timeout_blocks: Some(10),
        },
    }
}

#[test]
fn accepts_minimal_well_formed_atomic_sequence() {
    let ir = ir_with(vec![
        Operation::AtomicBegin,
        Operation::Lock {
            chain: "ethereum".to_owned(),
            asset: "USDC".to_owned(),
            amount: 100,
            from: "sender".to_owned(),
        },
        Operation::OnFail {
            action: FailureAction::Rollback,
        },
        Operation::AtomicEnd,
    ]);

    assert!(verify_ir(&ir).is_ok());
}

#[test]
fn rejects_atomic_end_without_begin() {
    let ir = ir_with(vec![Operation::AtomicEnd]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_unclosed_atomic_begin() {
    let ir = ir_with(vec![Operation::AtomicBegin, Operation::Nop]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_nested_atomic_scope() {
    let ir = ir_with(vec![
        Operation::AtomicBegin,
        Operation::AtomicBegin,
        Operation::AtomicEnd,
        Operation::AtomicEnd,
    ]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_zero_iteration_loop() {
    let ir = ir_with(vec![Operation::Loop {
        max_iterations: 0,
        body: vec![Operation::Nop],
    }]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_empty_host_call_identifier() {
    let ir = ir_with(vec![Operation::Call {
        function: String::new(),
        args: Vec::new(),
    }]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn rejects_invalid_multisig_threshold() {
    let ir = ir_with(vec![Operation::MultisigCheck { required: 3, total: 2 }]);
    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}

#[test]
fn recursively_rejects_unsafe_nested_control_flow() {
    let ir = ir_with(vec![Operation::If {
        condition: x3_lang_compiler::ir::Condition::True,
        then_ops: vec![Operation::ScheduledDispatch {
            period_blocks: 0,
            entry: vec![Operation::Nop],
        }],
        else_ops: Some(vec![Operation::Emit {
            name: "ok".to_owned(),
            data: HashMap::new(),
        }]),
    }]);

    assert_eq!(codes(&ir), vec![DiagnosticCode::UnsafeIr]);
}
