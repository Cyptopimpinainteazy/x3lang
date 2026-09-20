//! The version byte is a function of the opcode set (TICKET-097).
//!
//! Before this, `BYTECODE_VERSION_1` was `0x01`, it was written as the first byte of every
//! artifact, and it did **not** move when an opcode was added: `ROUTE_FALLBACK`, `PARALLEL_PLAN`,
//! `FEATURE_ALLOW`, `STRATEGY_LICENSE`, `VENUE_SETTLEMENT`, the whole `0x80..=0xAB` capability
//! block and the trading block all travelled inside the same version. A reader that predates an
//! opcode therefore had no way to refuse it — the version it would have checked was unchanged — so
//! it misparsed: `is_payload_opcode`'s own words are "A reader that classifies a payload-carrying
//! instruction as fixed-width advances four bytes and then reads bytes that are not instructions,
//! which is how the same defect has surfaced four times in this format." A refusal is visible; a
//! misparse is a different program.
//!
//! Three things are held here, and they are one claim: that every opcode is registered against the
//! version that introduced it (so an opcode cannot be added without the decision being stated),
//! that the version the writer writes is the greatest version in that table, and that a reader
//! refuses both a version it does not know and an opcode the artifact's own version does not
//! contain — rather than walking either.

use x3_lang_compiler::emitter::{emit_x3ir, first_instruction_offset, instructions};
use x3_lang_compiler::semantic::CompilationMode;
use x3_lang_compiler::spec::opcodes::{
    is_supported_version, max_opcode_version, opcode_version, BYTECODE_VERSION_1, BYTECODE_VERSION_2,
    CURRENT_BYTECODE_VERSION, IF_MEASURED, OPCODE_SET,
};
use x3_lang_vm::verifier::{verify, VerifyError};
use x3_lang_vm::x3_lang_vm::InstructionStream;

/// The spec file itself, so the gate reads what a future opcode would be added to rather than a
/// copy of it.
const SPEC_SOURCE: &str = include_str!("../../spec/opcodes.rs");

/// The `pub const`s in the spec that are `u8`s with a hex value but are **not** opcodes.
///
/// Each one is here with its reason, and the test below proves each name still exists in the file:
/// an exemption that outlives the constant it exempted is how this kind of list goes quietly
/// stale. Adding an opcode is *not* how you add to this list — registering it in `OPCODE_SET` is.
const NON_OPCODE_BYTE_CONSTANTS: &[(&str, &str)] = &[
    ("BYTECODE_VERSION_1", "the first bytecode version's own value"),
    ("BYTECODE_VERSION_2", "the second bytecode version's own value"),
    ("REQUIRE_COMPARE_MASK", "a bit mask over `REQUIRE`'s flags byte"),
    (
        "CAPABILITY_REPLY_MEASURED_TAG",
        "the tag of a measured figure in a host's reply, not an instruction",
    ),
    (
        "MEASURED_UNIT_CODE_MASK",
        "a bit mask over a measured guard's flags byte",
    ),
];

/// A program's worth of IR, lowered from a source that emits at least one instruction.
fn artifact() -> Vec<u8> {
    let source = r#"strategy Gate {
    input ethereum.USDC amount 25_000_000 max 50_000_000
    output ethereum.ETH
    effects [swap]
    guarantees [min_profit]
    domains [ethereum]
    risk { max_slippage_bps 50 max_total_fee_bps 8 }
    bounds { max_steps 10 max_gas 200_000 }
    execute {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1
        require slippage <= 50
        require profit >= 5
        on_fail refund ethereum.USDC to sender
    }
}
"#;
    let (_, ir, outcome) = x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev)
        .expect("the gate program must lower");
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    emit_x3ir(&ir).expect("the gate program must emit")
}

#[test]
fn every_opcode_constant_in_the_spec_is_registered_against_a_version() {
    let mut unregistered = Vec::new();
    let mut checked = 0usize;
    for line in SPEC_SOURCE.lines() {
        let Some(rest) = line.strip_prefix("pub const ") else {
            continue;
        };
        let Some((name, value)) = rest.split_once(": u8 = 0x") else {
            continue;
        };
        let Some(value) = value.strip_suffix(';') else {
            continue;
        };
        let Ok(value) = u8::from_str_radix(value, 16) else {
            continue;
        };
        checked += 1;
        if opcode_version(value).is_some() {
            continue;
        }
        if NON_OPCODE_BYTE_CONSTANTS.iter().any(|(exempt, _)| *exempt == name) {
            continue;
        }
        unregistered.push(format!("{name} (0x{value:02X})"));
    }
    assert!(
        checked > 50,
        "the gate must actually read the file: it saw {checked} constants"
    );
    assert!(
        unregistered.is_empty(),
        "these constants are neither registered in OPCODE_SET nor exempted as non-opcodes: \
         {unregistered:?}. An opcode added without a version is an opcode a reader of the artifact's \
         version cannot know the width of (TICKET-097)"
    );
}

#[test]
fn the_exemption_list_holds_only_constants_that_exist() {
    for (name, reason) in NON_OPCODE_BYTE_CONSTANTS {
        assert!(
            SPEC_SOURCE.contains(&format!("pub const {name}: u8 = 0x")),
            "`{name}` is exempted as \"{reason}\" and the spec no longer declares it, so the \
             exemption is stale and would hide a future constant of the same name"
        );
    }
}

#[test]
fn the_version_the_writer_writes_is_the_greatest_version_in_the_opcode_set() {
    // The same statement `spec/opcodes.rs` makes in a `const` assertion, held here where a reader
    // of the tests can find it. If the two ever disagree the compiler has already refused to
    // build, so this is the visible half of a compile-time gate rather than a second rule.
    assert_eq!(
        CURRENT_BYTECODE_VERSION,
        max_opcode_version(),
        "the version byte and the opcode set are one fact"
    );
    assert!(
        is_supported_version(CURRENT_BYTECODE_VERSION),
        "a pipeline that writes a version it cannot read back is a pipeline with no round trip"
    );
    assert!(
        OPCODE_SET
            .iter()
            .any(|(opcode, version)| *opcode == IF_MEASURED && *version == BYTECODE_VERSION_2),
        "and the opcode that moved it is registered against the version it was introduced in: that \
         is the whole of TICKET-105, and it is a *statement* rather than something a later reader \
         has to infer from the writer's byte"
    );
    assert!(
        OPCODE_SET
            .iter()
            .all(|(_, version)| *version == BYTECODE_VERSION_1 || *version == BYTECODE_VERSION_2),
        "no opcode may be registered against a version above the one this writer may write"
    );
}

#[test]
fn an_artifact_whose_version_this_reader_does_not_know_is_refused_by_name() {
    let mut bytes = artifact();
    // A version from a *future* build. Version 2 is one this reader supports, and the point of the
    // reservation is that a byte the format claims but does not implement is still refused rather
    // than read as an instruction — the hole that opened when version 2 became supported (TICKET-105).
    let future_version = BYTECODE_VERSION_2 + 1;
    bytes[0] = future_version;

    // The compiler's walker — `x3c explain`, `x3c inspect`.
    let error = instructions(&bytes)
        .err()
        .expect("a version this reader does not know must be refused");
    assert!(
        error.to_string().contains("version 3"),
        "the refusal must name the version, or its reader cannot tell which one to rebuild for: \
         {error}"
    );

    // And the VM's, which must refuse it *before* walking rather than reading the version byte as
    // an instruction. The message is the same one, from `spec::opcodes::version_refusal`.
    let error = verify(&InstructionStream::new(bytes)).expect_err("version 3 must not be executed");
    assert!(
        matches!(error, VerifyError::UnsupportedBytecodeVersion(3)),
        "a reserved version this reader does not know has its own refusal: {error:?}"
    );
    assert!(error.to_string().contains("version 3"), "{error}");
}

#[test]
fn an_opcode_the_artifacts_version_does_not_contain_is_refused_rather_than_advanced_over() {
    // A synthetic future opcode: `0x03` is not an instruction in any version this format defines,
    // which is what an opcode added by a *later* build looks like to this one. The artifact is a
    // real one, so the header, the version binding and the framing are all the writer's own —
    // only the instruction byte is the future's.
    let mut bytes = artifact();
    let at = first_instruction_offset(&bytes);
    assert!(
        at + 4 <= bytes.len(),
        "the artifact must have an instruction to replace"
    );
    bytes[at] = 0x03;

    let error = instructions(&bytes)
        .err()
        .expect("0x03 is not an instruction this format defines");
    assert!(
        error.to_string().contains("0x03"),
        "the refusal must name the opcode it could not place: {error}"
    );

    let error = verify(&InstructionStream::new(bytes)).expect_err("0x03 must not be walked");
    assert!(
        matches!(error, VerifyError::InvalidOpcode(0x03, _)),
        "an opcode no version defines is a malformed artifact: {error:?}"
    );
}

#[test]
fn an_artifact_this_reader_does_know_is_still_walked() {
    // The pair that stops the refusals above from passing vacuously: the same artifact, unpatched,
    // walks and verifies.
    let bytes = artifact();
    assert!(
        instructions(&bytes).is_ok(),
        "the writer's own artifact must be walkable by the writer's own walker"
    );
    assert!(
        verify(&InstructionStream::new(bytes)).is_ok(),
        "and executable by the VM it was written for"
    );
}
