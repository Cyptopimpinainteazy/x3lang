//! A venue's settlement guarantee reaches the artifact — spec PHASE 39, TICKET-075.
//!
//! PHASE 39's sentence is "do not claim atomic CEX execution unless the external venue
//! exposes enforceable settlement semantics", and the compiler has enforced that at
//! compile time for a while. What it did not do was *carry* it: a venue lowers to a
//! `VENUE_SETTLEMENT` record now, so a counterparty, an auditor or a replayer reading the
//! artifact — with no access to the source — can see whether a leg is atomic or one of the
//! five honest ways an off-chain leg settles.
//!
//! The record executes nothing. These tests pin the two things the VM owes it: an artifact
//! that carries one still runs, and an artifact that carries a malformed or undefined one
//! is refused rather than executed carrying a claim the language does not define.

use x3_lang_vm::verifier::verify;
use x3_lang_vm::{InstructionStream, VMConfig, VM};

fn source(shape_clause: &str) -> String {
    format!(
        r#"intent probe {{
    from ethereum.USDC amount 100 receiver 0x1
    to ethereum.ETH receiver 0x2
    route {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 100 min_output 1
    }}
    require slippage <= 50
    on_fail refund ethereum.USDC to sender
}}

venue probe_venue {{
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps 3
    liquidity 1_000_000
    slippage_bps 8
    latency_ms 12
    finality_blocks 12
    risk 2
{shape_clause}
}}
"#
    )
}

fn compiled(shape_clause: &str) -> Vec<u8> {
    let program = x3_lang_compiler::parser::parse_source(&source(shape_clause)).expect("it must parse");
    x3_lang_compiler::compile_program(&program).expect("it must compile")
}

/// Replace one same-length substring of the artifact.
///
/// Same length on purpose: the record's `u16` payload length and the four-byte frame
/// padding both depend on it, so a mutation that changed the length would be testing the
/// frame reader rather than the record's content.
fn mutated(bytecode: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len(), "a mutation must not change the length");
    let at = bytecode
        .windows(from.len())
        .position(|window| window == from)
        .unwrap_or_else(|| panic!("{:?} must be in the artifact", String::from_utf8_lossy(from)));
    let mut out = bytecode.to_vec();
    out[at..at + to.len()].copy_from_slice(to);
    out
}

#[test]
fn an_artifact_that_carries_a_settlement_record_still_executes() {
    let bytecode = compiled("    settlement escrow");
    assert!(
        bytecode.windows(13).any(|w| w == b"probe_venue:e"),
        "the artifact must carry the record this test is about"
    );
    verify(&InstructionStream::new(bytecode.clone())).expect("a carried declaration must verify");

    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    // The fixture writes `require slippage <= 50`, a measured guard, so the run states the
    // slippage it realised — the same way a host does.
    vm.report_outcome(Some(10), Some(0), None);
    vm.execute()
        .expect("recording how a leg settles must not stop the leg from executing");
}

/// Every malformed-record case, with the control that isolates the record as the cause: the
/// same source, unmutated, must verify, and only the mutation may flip the verdict. A test
/// that asserted the refusal alone would pass against a verifier that refused everything.
#[track_caller]
fn refused_when_mutated_to(from: &[u8], to: &[u8]) -> String {
    let valid = compiled("    settlement atomic");
    verify(&InstructionStream::new(valid.clone()))
        .expect("the unmutated artifact must verify, or the mutation is not what is being tested");

    let bytecode = mutated(&valid, from, to);
    let refusal = verify(&InstructionStream::new(bytecode.clone()))
        .expect_err("the mutation must be refused")
        .to_string();

    // And execution refuses too: `execute` verifies first (`verify_and_execute` is the only
    // caller of `execute_unverified`), so a malformed record cannot be run by skipping the
    // verification step — there is no public way to skip it.
    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    let error = vm.execute().expect_err("a malformed record must not execute");
    let rendered = format!("{error:?}");
    assert!(
        rendered.contains("X3_VERIFY_FAILED") && rendered.contains("InvalidOperand"),
        "the refusal must come from verification and name the operand: {rendered}"
    );
    refusal
}

/// The shape is a closed set, and the artifact is where a claim is made. An artifact whose
/// record says `vibes` must not run: the VM would otherwise execute a leg carrying a
/// settlement claim the language does not define, which is the claim PHASE 39 exists to
/// make unwritable.
#[test]
fn a_shape_the_language_does_not_define_is_refused() {
    refused_when_mutated_to(b"probe_venue:atomic", b"probe_venue:vibes!");
}

/// A record with no separator is not `venue:shape`, so a reader cannot attribute it. The
/// encoder never writes one; a hand-assembled artifact that does must not be read as if the
/// whole string were a venue name.
#[test]
fn a_record_with_no_separator_is_refused() {
    refused_when_mutated_to(b"probe_venue:atomic", b"probe_venue_atomic");
}

/// An unnamed venue makes the record unattributable, which is the whole content of the
/// record: a guarantee about nothing in particular is not a guarantee.
#[test]
fn a_record_with_no_venue_name_is_refused() {
    refused_when_mutated_to(b"probe_venue:atomic", b"           :atomic");
}

/// A second separator would let the shape field carry a venue name, so the record would
/// read back as a different venue than the program declared.
#[test]
fn a_record_with_a_second_separator_is_refused() {
    refused_when_mutated_to(b"probe_venue:atomic", b"probe_venue:at:mic");
}

/// A venue that states no settlement carries a record with an empty shape, and that record
/// is well-formed: "states none" is a fact the artifact is allowed to state, and refusing
/// it would make the honest case unrunnable.
#[test]
fn a_venue_that_states_no_settlement_is_well_formed() {
    let bytecode = compiled("");
    assert!(
        bytecode.windows(12).any(|w| w == b"probe_venue:"),
        "the record must be present with an empty shape"
    );
    verify(&InstructionStream::new(bytecode.clone())).expect("no shape stated is a valid record");
    let mut vm = VM::new(bytecode, VMConfig::default(), 1_000_000);
    vm.report_outcome(Some(10), Some(0), None);
    vm.execute().expect("and it must run");
}
