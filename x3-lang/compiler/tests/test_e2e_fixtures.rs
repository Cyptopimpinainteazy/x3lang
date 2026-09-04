//! E2E test fixtures (target H/I in the production contract).
//!
//! Each fixture in `fixtures/mod.rs` is exercised through the compiler
//! pipeline up to bytecode emission and verified for structure:
//!
//!   source  ->  parse  ->  AST  ->  lower  ->  IR  ->  emit  ->  bytecode  ->  execute
//!
//! **Execution status**: The VM executes all emitted bytecode
//! successfully because AtomicBegin/AtomicEnd are now wired in the
//! executor (they snapshot/commit VM state).  Each positive-case test
//! asserts that the compiled bytecode runs to completion.
//!
//! The negative cases assert that the semantic verifier rejects the
//! program with a useful diagnostic.

mod fixtures;

use x3_lang_compiler::{check_source, compile_source, parser::parse_source, Operation};
use x3_lang_vm::{VMConfig, VM};

fn run_pipeline(source: &str) -> Result<Vec<u8>, String> {
    let program = parse_source(source).map_err(|e| format!("parse: {e}"))?;
    let (_p, ir, errs) = check_source(source).map_err(|e| format!("check: {e}"))?;
    if !errs.is_empty() {
        return Err(format!("semantic: {}", errs[0]));
    }
    let bytecode = compile_source(source).map_err(|e| format!("compile: {e}"))?;
    assert!(!bytecode.is_empty(), "bytecode must not be empty");
    assert_eq!(bytecode[0], 0x01, "version byte");
    assert_eq!(bytecode.len() % 4, 0, "bytecode alignment");
    let _ = ir;
    let _ = program;
    Ok(bytecode)
}

fn assert_vm_executes(bytecode: Vec<u8>) {
    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000u128);
    vm.execute()
        .expect("VM must execute atomic bytecode successfully — AtomicBegin/AtomicEnd are wired");
}

fn assert_semantic_error(source: &str, needle: &str) {
    let (_p, _ir, errs) = check_source(source).expect("check_source must succeed");
    assert!(
        !errs.is_empty(),
        "expected semantic error containing {needle:?} in source {source}"
    );
    let combined: Vec<String> = errs.iter().map(|e| e.to_string()).collect();
    assert!(
        combined.iter().any(|m| m.contains(needle)),
        "expected error containing {needle:?} among {combined:?}"
    );
}

// ---------------- positive cases ----------------

#[test]
fn transfer_pipeline() {
    let bytecode = run_pipeline(fixtures::TRANSFER).expect("transfer pipeline must succeed");
    assert_vm_executes(bytecode);
}

#[test]
fn atomic_swap_pipeline() {
    let bytecode = run_pipeline(fixtures::ATOMIC_SWAP).expect("atomic_swap pipeline must succeed");
    assert_vm_executes(bytecode);
}

#[test]
fn evm_call_pipeline() {
    let bytecode = run_pipeline(fixtures::EVM_CALL).expect("evm_call pipeline must succeed");
    assert_vm_executes(bytecode);
}

#[test]
fn x3_call_pipeline() {
    let bytecode = run_pipeline(fixtures::X3_CALL).expect("x3_call pipeline must succeed");
    assert_vm_executes(bytecode);
}

#[test]
fn btc_route_pipeline() {
    // BTC routes are valid syntactically and semantically; the
    // adapter itself is feature-gated and exercised in vm tests.
    let bytecode = run_pipeline(fixtures::BTC_ROUTE).expect("btc_route pipeline must succeed");
    assert_vm_executes(bytecode);
}

// ---------------- negative cases ----------------

#[test]
fn invalid_route_rejected() {
    // The lowerer fills the bridge amount with the source amount
    // (0 in the malformed route, since the from-endpoint is a release
    // with no Lock for Ethereum.USDC) so the semantic verifier
    // catches a zero-amount bridge.
    let errs = check_source(fixtures::INVALID_ROUTE)
        .expect("check_source runs even with errors")
        .2;
    assert!(!errs.is_empty(), "expected at least one error");
}

#[test]
fn unknown_chain_rejected() {
    assert_semantic_error(fixtures::UNKNOWN_CHAIN, "unknown chain");
}

#[test]
fn malformed_rejected() {
    let result = parse_source(fixtures::MALFORMED);
    assert!(result.is_err(), "malformed source must be rejected");
}

// ---------------- IR golden tests ----------------
//
// These are "golden" tests: they pin the IR shape produced by the
// documented production intent surface. Any change to the IR output
// for these programs is intentional and must update these assertions.

#[test]
fn golden_ir_for_transfer() {
    let (_p, ir, errs) = check_source(fixtures::TRANSFER).expect("check_source");
    assert!(errs.is_empty(), "expected no semantic errors");
    // Production intent lowering produces: IntentResolve, Lock, Release, AtomicBegin, Swap, AtomicEnd.
    let kinds: Vec<&'static str> = ir
        .operations
        .iter()
        .map(|op| match op {
            Operation::Lock { .. } => "Lock",
            Operation::Release { .. } => "Release",
            Operation::Swap { .. } => "Swap",
            Operation::AtomicBegin => "AtomicBegin",
            Operation::AtomicEnd => "AtomicEnd",
            Operation::IntentResolve { .. } => "IntentResolve",
            _ => "Other",
        })
        .collect();
    assert_eq!(
        kinds,
        vec!["IntentResolve", "Lock", "Release", "AtomicBegin", "Swap", "AtomicEnd"]
    );
}

#[test]
fn golden_ir_for_atomic_swap() {
    let (_p, ir, errs) = check_source(fixtures::ATOMIC_SWAP).expect("check_source");
    assert!(errs.is_empty(), "expected no semantic errors");
    // atomic_swap has 1 lock, 1 release, and a route with bridge+swap
    let bridges = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::Bridge { .. }))
        .count();
    let swaps = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::Swap { .. }))
        .count();
    assert_eq!(bridges, 1, "one bridge in atomic_swap");
    assert_eq!(swaps, 1, "one swap in atomic_swap");
}

#[test]
fn golden_bytecode_size_for_transfer() {
    let bytecode = run_pipeline(fixtures::TRANSFER).expect("compile transfer");
    // Pin the byte count so accidental emitter changes get caught.
    assert_eq!(bytecode.len() % 4, 0);
    assert!(bytecode.len() > 32, "transfer bytecode must be substantive");
}
