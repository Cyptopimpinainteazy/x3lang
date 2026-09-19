//! `require nonce unused <id>` is a check, not an assertion (TICKET-051).
//!
//! It was the one guard whose quantity is a *chain* fact rather than a program
//! fact: the identifier reached `metadata.nonce` and nothing consulted it, while
//! the guard lowered to a `REQUIRE` the executor treats as true. Fourteen corpus
//! programs carry such a guard, so replay protection was a claim the artifact
//! recorded and nothing enforced.
//!
//! Now the compiler emits a `NONCE_UNUSED` instruction immediately before the
//! guard — test-and-record, leaving 1 in `r0` when the nonce is new — and the
//! guard compares (`r0 >= 1`) instead of asserting. The VM's state carries the
//! nonces this run has seen, so a host with a longer memory starts it with the
//! ones it already knows.

use x3_lang_vm::verifier::verify;
use x3_lang_vm::{InstructionStream, VMConfig, VM};

fn intent(guards: &str) -> String {
    format!(
        "intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {{\n        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }}\n{guards}\n    require slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}}\n"
    )
}

fn compiled(source: &str) -> Vec<u8> {
    let program = x3_lang_compiler::parser::parse_source(source).expect("it must parse");
    x3_lang_compiler::compile_program(&program).expect("it must compile")
}

fn vm_with(bytecode: Vec<u8>, used: &[&str]) -> VM {
    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    vm.state.used_nonces = used.iter().map(|nonce| nonce.to_string()).collect();
    vm
}

#[test]
fn a_fresh_nonce_passes_and_is_recorded() {
    let bytecode = compiled(&intent("    require nonce unused probe_001"));
    verify(&InstructionStream::new(bytecode.clone())).expect("the artifact must verify");

    let mut vm = vm_with(bytecode, &[]);
    vm.execute().expect("a fresh nonce is unused");
    assert_eq!(
        vm.state.used_nonces,
        vec!["probe_001".to_string()],
        "the guard tested the nonce and recorded it"
    );
}

#[test]
fn the_same_nonce_twice_fails_at_the_second_guard() {
    // The replay the guard exists to catch, inside one run.
    let bytecode = compiled(&intent(
        "    require nonce unused probe_001\n    require nonce unused probe_001",
    ));
    let error = vm_with(bytecode, &[])
        .execute()
        .expect_err("the second use is a replay");
    let error = format!("{error:?}");
    assert!(
        error.contains("X3_REQUIRE_FAILED"),
        "the guard is what fails, not a later instruction: {error}"
    );
    assert!(
        error.contains("r0=0 is below the required 1"),
        "and it says what it compared: {error}"
    );
}

#[test]
fn a_nonce_the_host_already_used_fails() {
    // The cross-run case: replay protection is a host fact, so the VM is given
    // what the host has seen. A guard that ignored this would pass here.
    let bytecode = compiled(&intent("    require nonce unused probe_001"));
    let error = vm_with(bytecode.clone(), &["probe_001"])
        .execute()
        .expect_err("the host has seen this nonce");
    let error = format!("{error:?}");
    assert!(error.contains("X3_REQUIRE_FAILED"), "got: {error}");

    // And a host that has seen a *different* nonce is not in the way.
    vm_with(bytecode, &["something_else"])
        .execute()
        .expect("only the nonce the guard names matters");
}

#[test]
fn distinct_nonces_do_not_interfere() {
    let bytecode = compiled(&intent(
        "    require nonce unused probe_001\n    require nonce unused probe_002",
    ));
    let mut vm = vm_with(bytecode, &[]);
    vm.execute().expect("two different nonces are both unused");
    assert_eq!(
        vm.state.used_nonces,
        vec!["probe_001".to_string(), "probe_002".to_string()],
        "and both are recorded, in the order they were tested"
    );
}
