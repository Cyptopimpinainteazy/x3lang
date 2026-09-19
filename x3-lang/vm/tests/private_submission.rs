//! PHASE 28: "Runtime should reject accidental public submission if compiled
//! policy requires privacy."
//!
//! A mode check that nothing reads is a policy the artifact states and the
//! runtime ignores, which is worse than no policy at all — the program believes
//! it is protected. These tests are the enforcement: the default runtime has no
//! private channel and refuses, and a runtime that has one runs the same
//! bytecode.

use x3_lang_common::capability::CapabilityPayload;
use x3_lang_common::encode_capability_payload;
use x3_lang_vm::executor::ExecError;
use x3_lang_vm::spec::opcodes::MODE_CHECK;
use x3_lang_vm::{VMConfig, VM};

/// `[MODE_CHECK][u16 len][payload]`, padded to four bytes, then HALT.
fn mode_check_bytecode(mode: &str, restriction: &str) -> Vec<u8> {
    let payload = encode_capability_payload(&CapabilityPayload::ModeCheck {
        mode: mode.to_string(),
        restriction: restriction.to_string(),
    })
    .expect("the payload must encode");
    let mut code = vec![0x01]; // version header
    code.push(MODE_CHECK);
    code.extend_from_slice(&(payload.len() as u16).to_le_bytes());
    code.extend_from_slice(&payload);
    while code.len() % 4 != 0 {
        code.push(0);
    }
    code.extend_from_slice(&[0xFF, 0, 0, 0]); // HALT
    code
}

#[test]
fn a_runtime_without_a_private_channel_refuses_required_privacy() {
    let code = mode_check_bytecode("submission", "private_required");
    let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
    match vm.execute() {
        Err(ExecError::Panic(message)) => assert!(
            message.contains("X3_PRIVATE_SUBMISSION_REQUIRED"),
            "the refusal must say what it is, got: {message}"
        ),
        other => panic!("a runtime with no private channel must refuse, got {other:?}"),
    }
}

#[test]
fn a_runtime_with_a_private_channel_runs_the_same_bytecode() {
    // Non-vacuous: the refusal is about the runtime's capability, not about the
    // artifact, so the same bytes run where the capability exists.
    let code = mode_check_bytecode("submission", "private_required");
    let config = VMConfig {
        allow_private_submission: true,
        ..VMConfig::default()
    };
    let mut vm = VM::new(code, config, 1_000_000);
    vm.execute().expect("a capable runtime must run it");
}

#[test]
fn a_preferred_policy_does_not_refuse_a_public_submission() {
    // Only `required` is a demand. A runtime may not refuse on `preferred`,
    // because the policy did not ask it to.
    for restriction in ["private_preferred", "private_allowed"] {
        let code = mode_check_bytecode("submission", restriction);
        let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
        vm.execute()
            .unwrap_or_else(|error| panic!("{restriction} must not refuse: {error:?}"));
    }
}

#[test]
fn the_flag_does_not_affect_other_mode_checks() {
    // The flag is about submission. A finality mode check is a different
    // question and must not start failing because a runtime lacks a private
    // channel.
    let code = mode_check_bytecode("finality", "safe");
    let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
    vm.execute().expect("a finality mode check is not a privacy demand");
}
