//! Bytecode verifier for X3 VM
//!
//! Performs deterministic checks on the bytecode to ensure safety:
//! - valid opcodes
//! - instruction boundaries
//! - jump targets
//! - memory and immediate operand ranges

use crate::x3_lang_vm::InstructionStream;
// Import shared opcode constants
use crate::spec::opcodes::*;
use std::collections::HashSet;
use x3_lang_common::{
    decode_asset_op_payload, decode_bridge_payload, decode_capability_payload, AssetOpPayload, CapabilityPayload,
};

#[derive(Debug)]
pub enum VerifyError {
    InvalidOpcode(u8, usize),
    InvalidOperand(usize),
    JumpToNonBoundary(usize, usize),
    OutOfBounds(usize),
    /// The artifact's version binding does not match this runtime's — spec PHASE 45.
    ///
    /// Its own variant rather than an `InvalidOperand` because the two say different
    /// things: an operand that does not decode is a malformed artifact, and a version
    /// that does not match is a *well-formed* artifact this runtime must not run. A
    /// loader that could not tell them apart would report the wrong thing to whoever has
    /// to fix it.
    VersionMismatch {
        field: &'static str,
        bound: u16,
        supported: u16,
    },
    /// The artifact states a bytecode version this reader does not know.
    ///
    /// Its own variant rather than reading one byte as an opcode: a version this reader does not
    /// know says the *opcode set* is not one it knows, so every width after the first byte is a
    /// guess. Refusing is the only answer that cannot be a misparse (TICKET-097), and the message
    /// comes from `spec::opcodes::version_refusal` so the reader and the writer's own refusal name
    /// the same version.
    UnsupportedBytecodeVersion(u8),
    /// An opcode that the version the artifact states does not contain — either because this format
    /// does not define it at all, or because it was introduced after that version.
    ///
    /// Separate from `InvalidOpcode` because the two say different things to whoever has to fix
    /// it: an opcode no version defines is a malformed artifact, and an opcode from a *later*
    /// version is a well-formed artifact this reader must not walk — the same distinction
    /// `VersionMismatch` draws one level up.
    OpcodeNotInVersion {
        opcode: u8,
        version: u8,
        pc: usize,
    },
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::VersionMismatch {
                field,
                bound,
                supported,
            } => write!(
                f,
                "X3_VERSION_MISMATCH: the artifact binds {field} version {bound} and this runtime \
                 supports {supported}, so it is not one this VM may execute"
            ),
            VerifyError::UnsupportedBytecodeVersion(version) => write!(
                f,
                "X3_BYTECODE_VERSION_UNSUPPORTED: {}",
                crate::spec::opcodes::version_refusal(*version).unwrap_or_else(|| format!(
                    "the artifact states bytecode version {version} and this reader does not know it"
                ))
            ),
            VerifyError::OpcodeNotInVersion { opcode, version, pc } => write!(
                f,
                "X3_OPCODE_NOT_IN_VERSION: at pc {pc}, {}",
                crate::spec::opcodes::opcode_version_refusal(*opcode, *version).unwrap_or_else(|| format!(
                    "opcode 0x{opcode:02X} is not one this reader may walk in an artifact stating \
                     bytecode version {version}"
                ))
            ),
            other => write!(f, "{other:?}"),
        }
    }
}

/// Validate that `code` is valid bytecode and return set of instruction boundaries.
pub fn verify(code: &InstructionStream) -> Result<HashSet<usize>, VerifyError> {
    // Emitted X3 bytecode is padded to 4-byte alignment, while capability
    // operations themselves use opcode + u16 payload length + payload.
    if code.len() % 4 != 0 {
        return Err(VerifyError::OutOfBounds(code.len()));
    }
    let mut boundaries = HashSet::new();
    let bytes = code.as_slice();
    let compiler_stream = has_compiler_header(bytes);
    // Which opcode set the artifact's own version promises. A raw stream (no version byte) is not
    // an artifact and has no other version to bind to than this build's, which is what an
    // in-process caller that assembles instructions by hand means by them.
    let artifact_version = if compiler_stream {
        bytes[0]
    } else {
        crate::spec::opcodes::CURRENT_BYTECODE_VERSION
    };
    // The version binding is checked before anything is read: an artifact this runtime
    // must not execute is refused whether or not the rest of it decodes (PHASE 45).
    if compiler_stream {
        // A *defined* version this reader does not know is refused by name. It is not treated as
        // raw bytecode: the first byte of an artifact is a version, and walking it as an
        // instruction is the misparse — with a newer version it would be a different instruction,
        // and with a payload opcode it would be `3 + whatever the next two bytes say`.
        if crate::spec::opcodes::version_refusal(bytes[0]).is_some() {
            return Err(VerifyError::UnsupportedBytecodeVersion(bytes[0]));
        }
        verify_version_binding(bytes)?;
    }
    let mut pc = first_instruction_pc(bytes);
    while pc + 4 <= bytes.len() {
        if bytes[pc..].iter().all(|byte| *byte == 0) {
            break;
        }
        boundaries.insert(pc);
        let opcode = bytes[pc];
        // The acceptance set is `spec/opcodes.rs`'s one table of registered opcodes, not a list of
        // ranges kept here. The range list this replaced is why: the trading range was missing from
        // it, so `verify` rejected every artifact a trading-core program produces with
        // `X3_VERIFY_FAILED: InvalidOpcode(176, 1)`, and it accepted every unassigned byte in
        // 0x00..=0xAB — leaving the executor to refuse them one instruction later, at the wrong
        // place and for the wrong reason (TICKET-097).
        match crate::spec::opcodes::opcode_version(opcode) {
            // Not an instruction at any version: a malformed artifact.
            None => return Err(VerifyError::InvalidOpcode(opcode, pc)),
            // An instruction from a version this artifact does not state, so this reader has no
            // width for it. Refused by name rather than guessed at (TICKET-097).
            Some(version) if version > artifact_version => {
                return Err(VerifyError::OpcodeNotInVersion {
                    opcode,
                    version: artifact_version,
                    pc,
                })
            }
            Some(_) => {}
        }
        if is_payload_opcode(opcode, compiler_stream) {
            let payload = read_payload(bytes, pc)?;
            validate_payload_opcode(opcode, payload, pc, bytes.len())?;
            pc = align4(pc + 3 + payload.len());
            continue;
        }

        let _flags = bytes[pc + 1];
        // A fixed frame's operand is as wide as the frame: four bytes for
        // `REQUIRE`, whose operand is a real `u16`, and one byte for every other
        // fixed frame, whose high half is the padding the emitter writes. The
        // width comes from `spec/opcodes.rs` so it cannot drift from the writer.
        let operand = fixed_frame_operand(opcode, compiler_stream, bytes[pc + 2], bytes[pc + 3]);
        // check flags & operand ranges depending on opcode (simplified)
        // for branches ensure destination is inside code and aligned
        match opcode {
            IF | LOOP => {
                // relative or absolute jumps; compute target
                let rel = operand as i16;
                let target = (pc + 4) as i32 + rel as i32; // relative
                if target < 0 || (target as usize) >= bytes.len() {
                    return Err(VerifyError::InvalidOperand(pc));
                }
                if ((target as usize) % 4) != 0 {
                    return Err(VerifyError::JumpToNonBoundary(pc, target as usize));
                }
            }
            REQUIRE => {
                // Bits 0-1 are the comparison mode; bits 2-4 the guard's own
                // operator. A byte that says "test a run-time comparison this VM
                // does not implement", or records an operator the language does
                // not define, is a guard whose meaning nobody can state.
                // Every mode the language defines, not only the first: this check used
                // to accept `STATIC` and `GE` alone, so the first artifact carrying a
                // *measured* guard was rejected as an invalid operand before the executor
                // could refuse it for the honest reason. A whitelist that names the
                // members of a set has to be updated with the set, and `spec/opcodes.rs`
                // is the single source for it — the same two-layer failure the choice
                // criteria hit in `89a15ccc5`.
                let mode = bytes[pc + 1] & REQUIRE_COMPARE_MASK;
                if mode > REQUIRE_COMPARE_MEASURED_SLIPPAGE {
                    return Err(VerifyError::InvalidOperand(pc));
                }
                // A measured guard names *which* quantity it compares in the flags' high
                // bits, and the set is closed: a code outside it would be compared against
                // whichever field the executor's fall-through happened to read, so a
                // hand-assembled artifact could ask for a measurement the language does not
                // have and be answered with a different one. The code is only meaningful
                // for a measured guard, and code 0 is "no unit stated" — the profit for
                // mode 2, and what every artifact written before the field existed carries.
                let unit_code = require_measured_unit_code(bytes[pc + 1]);
                if mode == REQUIRE_COMPARE_MEASURED_PROFIT {
                    if !is_known_measured_unit_code(unit_code) {
                        return Err(VerifyError::InvalidOperand(pc));
                    }
                } else if mode == REQUIRE_COMPARE_STATIC {
                    // A static guard's operand carries the figure it was *checked against* — a
                    // bond, a score, a depth — and the same three bits say what that figure
                    // counts. The set is closed for the reason the measured units' is: a code
                    // outside it would be printed by a reader as a quantity the guard is not
                    // about. Zero means "no figure carried", which is what every artifact
                    // written before the figure was carried reads as.
                    if !is_known_guard_quantity(unit_code) {
                        return Err(VerifyError::InvalidOperand(pc));
                    }
                } else if unit_code != MEASURED_UNIT_CODE_PROFIT_BPS {
                    return Err(VerifyError::InvalidOperand(pc));
                }
                if require_guard_operator(bytes[pc + 1]) > GUARD_OP_NE {
                    return Err(VerifyError::InvalidOperand(pc));
                }
            }
            FEATURE_ALLOW => {
                // A fixed three-byte frame whose operand is the feature code.
                // The set is closed: consent to an unknown mode is not consent,
                // and a byte that happens to decode as a feature must still name
                // one the language defines.
                if operand != u16::from(FEATURE_INTENT_FUSION) {
                    return Err(VerifyError::InvalidOperand(pc));
                }
            }
            CALL => {
                let target = operand as usize;
                if target >= bytes.len() {
                    return Err(VerifyError::InvalidOperand(pc));
                }
                if target % 4 != 0 {
                    return Err(VerifyError::JumpToNonBoundary(pc, target));
                }
            }
            RET => { /* RET - valid */ }
            _ => {}
        }
        // The same advance the executor makes, from the same table: in a
        // compiler stream the first instruction sits at offset 1 (the version
        // byte is byte 0), so what bounds it is the next *absolute* multiple of
        // four after the frame's own bytes — which is what the emitter pads for.
        // `pc + 4` is that only when `pc` is a multiple of four itself, and for
        // a frame whose content is four bytes rather than three the boundary is
        // two alignments away, so both the width and the alignment are read
        // rather than assumed. Getting this wrong is how the verifier refused an
        // artifact the executor runs (`X3_VERIFY_FAILED: OutOfBounds(73)`) and
        // how `x3c explain` printed a six-instruction program as eighteen lines
        // of `UNKNOWN`.
        // A compiler stream frames a fixed instruction in three bytes (four for
        // `REQUIRE`) and pads; a raw stream's fixed instructions are four bytes
        // with no padding. The two agree today only because raw streams start at
        // zero and stay aligned, so the bound is written out rather than
        // inferred from the offset.
        let content_len = if compiler_stream {
            fixed_frame_content_len(opcode)
        } else {
            4
        };
        pc = align4(pc + content_len);
    }
    Ok(boundaries)
}

/// Read the version binding and refuse an artifact this runtime must not run.
///
/// Two rejections, and both are the fail-closed direction: an artifact whose language,
/// IR or VM version is not this runtime's, and a *compiler stream with no binding at all*
/// — which cannot be shown to be one this runtime may execute. The compiler's own format
/// version and the policy version are carried and not compared: they say which build
/// produced the artifact, and neither decides whether it may run.
fn verify_version_binding(bytes: &[u8]) -> Result<(), VerifyError> {
    let Some((language, _compiler, ir, vm, _policy)) = crate::spec::opcodes::version_binding(bytes) else {
        // A compiler stream that binds to nothing cannot be shown to be one this runtime
        // may execute, so it is refused rather than run on the assumption that it is.
        return Err(VerifyError::InvalidOperand(0));
    };
    for (field, bound, supported) in [
        ("language", language, LANGUAGE_VERSION),
        ("IR", ir, IR_VERSION),
        ("VM", vm, VM_VERSION),
    ] {
        if bound != supported {
            return Err(VerifyError::VersionMismatch {
                field,
                bound,
                supported,
            });
        }
    }
    Ok(())
}

fn first_instruction_pc(bytes: &[u8]) -> usize {
    if has_compiler_header(bytes) {
        // The compiler-stream header is 0x01 followed by an arbitrary
        // sequence of metadata records (currently 0x10=nonce and
        // 0x11=chain_id, but the format is open). Walk past them so
        // the verifier does not mistake metadata bytes for opcodes.
        skip_compiler_metadata(bytes)
    } else {
        0
    }
}

fn skip_compiler_metadata(bytes: &[u8]) -> usize {
    // One walker for the whole set (`spec::opcodes::metadata_record`), so a tag added there
    // cannot be read as instructions here.
    let mut pc = 1usize;
    while let Some((len, _, _)) = crate::spec::opcodes::metadata_record(bytes, pc) {
        pc += len;
    }
    pc
}

fn has_compiler_header(bytes: &[u8]) -> bool {
    // Same rule as the executor's `has_compiler_header`: a version byte
    // followed by a real record. A stream of `[0x01][0x00..]` is raw bytecode
    // that happens to start with the version byte, not a compiler stream.
    //
    // The first byte must be a version this format *defines* rather than only the one this reader
    // supports: a version-2 artifact has to arrive here so `verify` can refuse it by name, and if
    // it were not recognised as a stream it would be walked as raw bytecode instead — the misparse
    // this whole file exists to prevent (TICKET-097). `is_reserved_version_byte` answers the framing
    // question — it claims the space, so a version from a *future* build is a stream to refuse rather
    // than raw bytecode to walk — and `is_supported_version` answers the compatibility one, which
    // `version_refusal` applies before anything is read (TICKET-105).
    is_reserved_version_byte(bytes.first().copied().unwrap_or(0)) && bytes.get(1).copied().unwrap_or(NOP) != NOP
}

// The classification comes from `spec/opcodes.rs`, shared with the compiler's
// disassembler. Local copies had drifted: this one omitted `EMIT` and
// `CALL_HOST`, which carry payloads, so the verifier walked four bytes into them.

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn read_payload(bytes: &[u8], pc: usize) -> Result<&[u8], VerifyError> {
    if pc + 3 > bytes.len() {
        return Err(VerifyError::OutOfBounds(pc));
    }
    let len = u16::from_le_bytes([bytes[pc + 1], bytes[pc + 2]]) as usize;
    let start = pc + 3;
    let end = start.checked_add(len).ok_or(VerifyError::InvalidOperand(pc))?;
    if end > bytes.len() {
        return Err(VerifyError::OutOfBounds(pc));
    }
    Ok(&bytes[start..end])
}

fn validate_payload_opcode(opcode: u8, payload: &[u8], pc: usize, stream_len: usize) -> Result<(), VerifyError> {
    if matches!(opcode, LOCK | MINT | BURN | RELEASE | SWAP) {
        let payload = decode_asset_op_payload(opcode, payload).map_err(|_| VerifyError::InvalidOperand(pc))?;
        match payload {
            AssetOpPayload::Lock {
                chain,
                asset,
                amount,
                from,
            }
            | AssetOpPayload::Mint {
                chain,
                asset,
                amount,
                to: from,
            }
            | AssetOpPayload::Burn {
                chain,
                asset,
                amount,
                from,
            } => {
                if chain.is_empty() || asset.is_empty() || from.is_empty() || amount == 0 {
                    return Err(VerifyError::InvalidOperand(pc));
                }
            }
            AssetOpPayload::Release { chain, asset, to, .. } => {
                if chain.is_empty() || asset.is_empty() || to.is_empty() {
                    return Err(VerifyError::InvalidOperand(pc));
                }
            }
            AssetOpPayload::Swap {
                from_chain,
                from_asset,
                to_chain,
                to_asset,
                input_amount,
                ..
            } => {
                if from_chain.is_empty()
                    || from_asset.is_empty()
                    || to_chain.is_empty()
                    || to_asset.is_empty()
                    || input_amount == 0
                {
                    return Err(VerifyError::InvalidOperand(pc));
                }
            }
        }
        return Ok(());
    }

    if opcode == BRIDGE {
        let payload = decode_bridge_payload(payload).map_err(|_| VerifyError::InvalidOperand(pc))?;
        if payload.via.is_empty()
            || payload.from_chain.is_empty()
            || payload.from_asset.is_empty()
            || payload.to_chain.is_empty()
            || payload.to_asset.is_empty()
            || payload.receiver.is_empty()
            || payload.amount == 0
        {
            return Err(VerifyError::InvalidOperand(pc));
        }
        return Ok(());
    }

    if (TRADING_BEGIN..=TRADING_BRIDGE).contains(&opcode) {
        // Trading payloads are their own encoding (see the compiler's
        // `decode_trading_operation`), not a `CapabilityPayload`, so the
        // capability decoder below cannot read them: it rejects the opcode
        // outright, which made every trading-core program fail verification
        // with `InvalidOperand` even though the executor runs it. Decoding here
        // keeps the verifier's promise that a payload it accepts has actually
        // been checked against its opcode.
        x3_lang_compiler::emitter::decode_trading_operation(opcode, payload)
            .map_err(|_| VerifyError::InvalidOperand(pc))?;
        return Ok(());
    }

    if opcode == STRATEGY_LICENSE {
        // The distribution reads this record, so the rule the compiler enforces
        // is enforced again where it is read: a split that does not total 10,000
        // is refused rather than distributed on a best-effort basis.
        let text = std::str::from_utf8(payload).map_err(|_| VerifyError::InvalidOperand(pc))?;
        let field =
            |name: &str| -> Option<&str> { text.split(';').find_map(|part| part.strip_prefix(&format!("{name}="))) };
        let creator = field("creator").ok_or(VerifyError::InvalidOperand(pc))?;
        let royalty: u32 = field("royalty_bps")
            .and_then(|value| value.parse().ok())
            .ok_or(VerifyError::InvalidOperand(pc))?;
        if royalty > 10_000 {
            return Err(VerifyError::InvalidOperand(pc));
        }
        // A royalty paid to nobody is the only way an empty creator is wrong; a
        // module may split its profit without being licensed.
        if royalty > 0 && creator.is_empty() {
            return Err(VerifyError::InvalidOperand(pc));
        }
        let split = field("split").ok_or(VerifyError::InvalidOperand(pc))?;
        let mut total = 0u32;
        let mut shares = 0usize;
        for entry in split.split(',').filter(|entry| !entry.is_empty()) {
            let Some((recipient, bps)) = entry.split_once(':') else {
                return Err(VerifyError::InvalidOperand(pc));
            };
            let bps: u32 = bps.parse().map_err(|_| VerifyError::InvalidOperand(pc))?;
            if recipient.is_empty() || bps == 0 {
                return Err(VerifyError::InvalidOperand(pc));
            }
            total = total.saturating_add(bps);
            shares += 1;
        }
        if shares == 0 || total != 10_000 {
            return Err(VerifyError::InvalidOperand(pc));
        }
        return Ok(());
    }

    if opcode == PARALLEL_PLAN {
        // `legs=<n>;waves=a,b|c;edges=a->c`. The verifier checks the record is a
        // plan that could have been built: at least two legs, no empty wave, a
        // declared leg count that matches the waves, and no edge naming a leg
        // outside them. Beyond that the artifact is trusted, because a plan is
        // not executable state — it is the compiler's conclusion about which
        // legs are independent, recorded so it can be reviewed.
        let text = std::str::from_utf8(payload).map_err(|_| VerifyError::InvalidOperand(pc))?;
        let field =
            |name: &str| -> Option<&str> { text.split(';').find_map(|part| part.strip_prefix(&format!("{name}="))) };
        let legs: usize = field("legs")
            .and_then(|value| value.parse().ok())
            .ok_or(VerifyError::InvalidOperand(pc))?;
        let waves_field = field("waves").ok_or(VerifyError::InvalidOperand(pc))?;
        let waves: Vec<&str> = waves_field.split('|').collect();
        if legs < 2
            || waves.is_empty()
            || waves.iter().any(|wave| wave.is_empty())
            || waves.iter().map(|wave| wave.split(',').count()).sum::<usize>() != legs
        {
            return Err(VerifyError::InvalidOperand(pc));
        }
        let declared: Vec<&str> = waves.iter().flat_map(|wave| wave.split(',')).collect();
        if declared.len() != legs || declared.iter().any(|leg| leg.is_empty()) {
            return Err(VerifyError::InvalidOperand(pc));
        }
        if let Some(edges) = field("edges") {
            for edge in edges.split(',').filter(|edge| !edge.is_empty()) {
                let Some((from, to)) = edge.split_once("->") else {
                    return Err(VerifyError::InvalidOperand(pc));
                };
                if from == to || !declared.contains(&from) || !declared.contains(&to) {
                    return Err(VerifyError::InvalidOperand(pc));
                }
            }
        }
        // Every leg must name the domain it executes on: a plan that does not
        // say which VM runs a leg is not a multi-VM plan, it is a list of legs
        // with a multi-VM claim attached.
        let domains = field("domains").ok_or(VerifyError::InvalidOperand(pc))?;
        let mut with_domain = 0usize;
        for entry in domains.split(',').filter(|entry| !entry.is_empty()) {
            let Some((leg, leg_domains)) = entry.split_once(':') else {
                return Err(VerifyError::InvalidOperand(pc));
            };
            if !declared.contains(&leg) || leg_domains.is_empty() {
                return Err(VerifyError::InvalidOperand(pc));
            }
            with_domain += 1;
        }
        if with_domain != legs {
            return Err(VerifyError::InvalidOperand(pc));
        }
        // The settlement section is what a coordinator acts on, so a plan
        // without one, or with a record that does not line up with the waves,
        // is refused rather than accepted as "no obligations".
        let settle = field("settle").ok_or(VerifyError::InvalidOperand(pc))?;
        let records: Vec<&str> = settle.split('|').collect();
        if records.len() != waves.len() {
            return Err(VerifyError::InvalidOperand(pc));
        }
        for (index, record) in records.iter().enumerate() {
            let parts: Vec<&str> = record.split(':').collect();
            if parts.len() != 4 || parts[0].parse::<usize>().ok() != Some(index) {
                return Err(VerifyError::InvalidOperand(pc));
            }
            if parts[1].is_empty() || !matches!(parts[3], "local" | "coordinated") {
                return Err(VerifyError::InvalidOperand(pc));
            }
            // The rule the compiler enforces, checked again where it is read: a
            // wave over more than one domain is not locally recoverable.
            if parts[1] != "-" && parts[1].contains('+') && parts[3] == "local" {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        return Ok(());
    }

    if opcode == IF_MEASURED {
        // `unit:invert:threshold:skip`. Every field is checked, and the skip is checked against where
        // it lands: a branch whose target is outside the stream, or inside the middle of an
        // instruction, is a record no execution could follow, and the executor would compute it from
        // the same figures — so this is where it is caught rather than at the jump.
        let Some((_, _, _, skip)) = parse_if_measured(payload) else {
            return Err(VerifyError::InvalidOperand(pc));
        };
        let after = align4(pc + 3 + payload.len());
        let target = after.saturating_add((skip as usize).saturating_mul(4));
        if target > stream_len {
            return Err(VerifyError::InvalidOperand(pc));
        }
        return Ok(());
    }

    if opcode == ATOMIC_CHOICE {
        // `criterion:paths:selected`. The verifier checks the record describes a
        // branch set that could have been verified — a known criterion, at least
        // two paths, and a selected index that names one — because a record that
        // does not is a body whose provenance the artifact cannot state.
        let text = std::str::from_utf8(payload).map_err(|_| VerifyError::InvalidOperand(pc))?;
        let fields: Vec<&str> = text.split(':').collect();
        if fields.len() != 3 {
            return Err(VerifyError::InvalidOperand(pc));
        }
        let criterion: u8 = fields[0].parse().map_err(|_| VerifyError::InvalidOperand(pc))?;
        let paths: u32 = fields[1].parse().map_err(|_| VerifyError::InvalidOperand(pc))?;
        let selected: u32 = fields[2].parse().map_err(|_| VerifyError::InvalidOperand(pc))?;
        let known_criterion = matches!(
            criterion,
            CHOICE_CRITERION_HIGHEST_NET_OUTPUT | CHOICE_CRITERION_FEWEST_HOPS | CHOICE_CRITERION_LOWEST_DECLARED_FEE
        );
        if !known_criterion || paths < 2 || selected >= paths {
            return Err(VerifyError::InvalidOperand(pc));
        }
        return Ok(());
    }

    if opcode == ROUTE_FALLBACK {
        // The approved-venue list is a plain comma-separated payload, not a
        // `CapabilityPayload`, so the decoder below cannot read it: without
        // this branch the fall-through rejected the opcode outright and every
        // program that declared a fallback failed verification with
        // `InvalidOperand` while the executor could run it. The same shape as
        // the trading range above, in the same function.
        let text = std::str::from_utf8(payload).map_err(|_| VerifyError::InvalidOperand(pc))?;
        let approved: Vec<&str> = text.split(',').collect();
        if approved.is_empty() || approved.iter().any(|venue| venue.is_empty()) || approved.len() > MAX_ROUTE_FALLBACKS
        {
            return Err(VerifyError::InvalidOperand(pc));
        }
        return Ok(());
    }

    if opcode == VENUE_SETTLEMENT {
        // `venue:shape`, a plain payload the capability decoder below cannot read — the
        // same shape as the branch above, and it needs the same handling for the same
        // reason: without it the fall-through rejects the opcode and every program that
        // declares a venue fails verification while the executor can run it.
        //
        // The shape vocabulary is checked here as well as in the executor, against the
        // compiler's own `SettlementGuarantee`, so a hand-assembled artifact carrying
        // `settlement vibes` is refused at the door rather than at the leg it describes.
        let text = std::str::from_utf8(payload).map_err(|_| VerifyError::InvalidOperand(pc))?;
        let (venue, shape) = text.split_once(':').ok_or(VerifyError::InvalidOperand(pc))?;
        // `trim()` to match the compiler's rule (`semantic`/`verify` refuse a name that
        // is blank rather than only one that is empty), so the two cannot disagree about a
        // name the other refuses to emit.
        if venue.trim().is_empty() || shape.contains(':') {
            return Err(VerifyError::InvalidOperand(pc));
        }
        if !shape.is_empty() && x3_lang_compiler::ir::SettlementGuarantee::parse(shape).is_none() {
            return Err(VerifyError::InvalidOperand(pc));
        }
        return Ok(());
    }

    let payload = decode_capability_payload(opcode, payload).map_err(|_| VerifyError::InvalidOperand(pc))?;
    match payload {
        CapabilityPayload::ScheduledDispatch { period_blocks, .. } => {
            if period_blocks == 0 {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        CapabilityPayload::ProofVerify { proof, input, .. } => {
            if proof.is_empty() || input.is_empty() {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        CapabilityPayload::MultisigCheck { required, total } => {
            if required > total {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        CapabilityPayload::GasAdaptive {
            high_gas_ops,
            low_gas_ops,
        } => {
            if high_gas_ops == 0 || low_gas_ops == 0 {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        CapabilityPayload::SubExec {
            bytecode_hash,
            gas_limit,
            ..
        } => {
            if bytecode_hash.is_empty() || gas_limit == 0 {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        CapabilityPayload::NonceUnused { nonce } => {
            // The same rule the compiler states: an empty identifier would test
            // and record nothing while the guard after it passes, i.e. a
            // replay-protection instruction that protects against no replay.
            if nonce.is_empty() {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        CapabilityPayload::EmitEvent { name, fields } => {
            // An event with no name is an event no consumer can subscribe to, and an
            // argument with no name is one the event's own reader cannot address — the
            // record would carry a value nobody could bind to a parameter.
            if name.is_empty() || fields.iter().any(|(key, value)| key.is_empty() || value.is_empty()) {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        CapabilityPayload::HostCall { function, .. } => {
            // The host is asked for a function *by name*: `CALL_HOST` carries no other
            // selector, so an empty one is a call the host cannot route and must not
            // receive as a no-op.
            if function.is_empty() {
                return Err(VerifyError::InvalidOperand(pc));
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use x3_lang_common::encode_capability_payload;

    fn payload_code(opcode: u8, payload: CapabilityPayload) -> InstructionStream {
        let payload = encode_capability_payload(&payload).expect("test payload should encode");
        let mut bytes = vec![opcode];
        bytes.extend_from_slice(&(payload.len() as u16).to_le_bytes());
        bytes.extend_from_slice(&payload);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        InstructionStream::new(bytes)
    }

    /// A measured `REQUIRE` names *which* quantity it compares in the flags' high bits, and
    /// the set is closed — a code outside it would be compared against whichever field the
    /// executor's fall-through happened to read, so a hand-assembled artifact could ask for
    /// a measurement the language does not have and be answered with a different one
    /// (TICKET-068).
    #[test]
    fn verifier_rejects_a_measured_guard_naming_a_quantity_the_language_does_not_have() {
        // A `REQUIRE` frame is `[opcode][flags][operand lo][operand hi]`.
        let require = |flags: u8| InstructionStream::new(vec![REQUIRE, flags, 1, 0]);

        // The profit's own code is zero, which is what every artifact written before the
        // field existed carries — so this is the backward-compatibility assertion as much
        // as it is the acceptance of the code.
        assert!(
            verify(&require(require_flags(REQUIRE_COMPARE_MEASURED_PROFIT, GUARD_OP_LE))).is_ok(),
            "a measured profit guard carries no unit code and must still verify"
        );
        assert!(
            verify(&require(require_flags_measured(
                REQUIRE_COMPARE_MEASURED_PROFIT,
                GUARD_OP_LE,
                MEASURED_UNIT_CODE_DELTA_BPS
            )))
            .is_ok(),
            "and a measured delta guard names its quantity and must verify"
        );

        assert!(
            verify(&require(require_flags_measured(
                REQUIRE_COMPARE_MEASURED_PROFIT,
                GUARD_OP_LE,
                5
            )))
            .is_err(),
            "a code outside the set is not a measurement this VM can answer"
        );
        assert!(
            verify(&require(require_flags_measured(
                REQUIRE_COMPARE_MEASURED_SLIPPAGE,
                GUARD_OP_LE,
                MEASURED_UNIT_CODE_DELTA_BPS
            )))
            .is_err(),
            "a unit code on a mode whose quantity is already named is a second answer to a \
             question that has one"
        );
    }

    #[test]
    fn verifier_accepts_structurally_valid_capability_payloads() {
        for (opcode, payload) in [
            (
                0x82,
                CapabilityPayload::ScheduledDispatch {
                    period_blocks: 5,
                    entry_ops: 1,
                },
            ),
            (
                0x85,
                CapabilityPayload::ProofVerify {
                    kind: 0,
                    proof: "proof".into(),
                    input: "input".into(),
                    key_or_threshold: "vk".into(),
                },
            ),
            (0x94, CapabilityPayload::MultisigCheck { required: 2, total: 3 }),
            (
                0x99,
                CapabilityPayload::GasAdaptive {
                    high_gas_ops: 1,
                    low_gas_ops: 1,
                },
            ),
        ] {
            assert!(
                verify(&payload_code(opcode, payload)).is_ok(),
                "opcode 0x{opcode:02x} should verify"
            );
        }
    }

    #[test]
    fn verifier_rejects_invalid_capability_payloads() {
        for (opcode, payload) in [
            (
                0x82,
                CapabilityPayload::ScheduledDispatch {
                    period_blocks: 0,
                    entry_ops: 1,
                },
            ),
            (
                0x85,
                CapabilityPayload::ProofVerify {
                    kind: 0,
                    proof: "".into(),
                    input: "input".into(),
                    key_or_threshold: "vk".into(),
                },
            ),
            (0x94, CapabilityPayload::MultisigCheck { required: 4, total: 3 }),
            (
                0x99,
                CapabilityPayload::GasAdaptive {
                    high_gas_ops: 0,
                    low_gas_ops: 1,
                },
            ),
        ] {
            assert!(
                verify(&payload_code(opcode, payload)).is_err(),
                "opcode 0x{opcode:02x} should reject malformed payload"
            );
        }
    }

    /// A compiler stream: version byte, metadata records, then `ATOMIC_BEGIN`.
    ///
    /// `ATOMIC_BEGIN` encodes as `[opcode][u16 0]` padded to four bytes, so the
    /// expected first boundary is the metadata length.
    fn compiler_stream_with(nonce: Option<&str>, chain_id: Option<u64>) -> (InstructionStream, usize) {
        // The version binding comes first, because `verify` refuses a compiler stream that
        // carries none (PHASE 45) — and a hand-built stream is exactly the case that rule
        // is about.
        let mut bytes = vec![BYTECODE_VERSION_1];
        bytes.push(META_VERSIONS);
        for version in [
            LANGUAGE_VERSION,
            COMPILER_FORMAT_VERSION,
            IR_VERSION,
            VM_VERSION,
            POLICY_VERSION,
        ] {
            bytes.extend_from_slice(&version.to_le_bytes());
        }
        if let Some(nonce) = nonce {
            bytes.push(META_NONCE);
            bytes.extend_from_slice(&(nonce.len() as u16).to_le_bytes());
            bytes.extend_from_slice(nonce.as_bytes());
        }
        if let Some(chain_id) = chain_id {
            bytes.push(META_CHAIN_ID);
            bytes.extend_from_slice(&chain_id.to_le_bytes());
        }
        let first_instruction = bytes.len();
        bytes.extend_from_slice(&[ATOMIC_BEGIN, 0, 0, 0]);
        // `emit_x3ir` pads the whole stream to a multiple of four, and `verify`
        // requires that.
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        (InstructionStream::new(bytes), first_instruction)
    }

    #[test]
    fn a_nonce_record_that_is_not_a_multiple_of_four_does_not_shift_the_walk() {
        // `[0x10][u16 15]["simple_swap_001"]` is 18 bytes, so the first
        // instruction begins at 19. The verifier used to round that up to 20 and
        // start reading the metadata's last byte as an opcode, which is how a
        // stream the executor runs fine could fail validation.
        let (code, expected) = compiler_stream_with(Some("simple_swap_001"), None);
        assert_eq!(
            expected,
            1 + VERSIONS_RECORD_LEN + 18,
            "the version byte, the binding, then the record's 18 bytes"
        );

        let boundaries = verify(&code).expect("a stream the executor runs must verify");
        assert!(
            boundaries.contains(&expected),
            "the first instruction must be found at {expected}, got {boundaries:?}"
        );
        assert!(
            !boundaries.contains(&(expected + 1)),
            "the walk must not be a byte late, got {boundaries:?}"
        );
    }

    #[test]
    fn an_artifact_that_binds_another_version_is_refused() {
        // PHASE 45's requirement, and the reason it is fail-closed: an artifact this runtime
        // must not execute has to be refused whether or not the rest of it decodes.
        let (code, _) = compiler_stream_with(Some("nonce_1"), None);
        let mut bytes = code.as_slice().to_vec();
        // The binding is the first record: tag, then language, compiler, IR, VM, policy.
        // Patch the VM version (offset 1 + tag + 3 u16s).
        let vm_version_at = 1 + 1 + 3 * 2;
        bytes[vm_version_at] = VM_VERSION.wrapping_add(1) as u8;
        bytes[vm_version_at + 1] = 0;

        let error = verify(&InstructionStream::new(bytes)).expect_err("a wrong VM version must be refused");
        match error {
            VerifyError::VersionMismatch {
                field,
                bound,
                supported,
            } => {
                assert_eq!(field, "VM");
                assert_eq!(supported, VM_VERSION);
                assert_ne!(bound, supported, "the artifact bound a version this runtime is not");
            }
            other => panic!("the refusal must name the version, got {other:?}"),
        }
        // And the message says which artifact this is and what the runtime supports, so a
        // human can act on it.
        let rendered = format!("{error}");
        assert!(
            rendered.contains("X3_VERSION_MISMATCH") && rendered.contains("VM version"),
            "the refusal must name the field and the code: {rendered}"
        );
    }

    #[test]
    fn a_compiler_stream_that_binds_no_version_is_refused() {
        // Fail closed: an artifact that binds to nothing cannot be shown to be one this
        // runtime may execute, so it is refused rather than run on the assumption that it is.
        let mut bytes = vec![BYTECODE_VERSION_1, ATOMIC_BEGIN, 0, 0, 0];
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        verify(&InstructionStream::new(bytes)).expect_err("a compiler stream with no binding must be refused");
    }

    #[test]
    fn chain_id_metadata_is_walked_as_nine_bytes() {
        // The record is `[0x11][u64]`; reading it as a u32 advanced only five
        // bytes and put the cursor three bytes into the next instruction.
        let (code, expected) = compiler_stream_with(Some("nonce_1"), Some(0x0123_4567_89AB_CDEF));
        assert_eq!(
            expected,
            1 + VERSIONS_RECORD_LEN + 10 + 9,
            "the binding, then the nonce record, then nine bytes of chain id"
        );

        let boundaries = verify(&code).expect("a stream with a chain id must verify");
        assert!(
            boundaries.contains(&expected),
            "the first instruction must be found at {expected}, got {boundaries:?}"
        );
    }

    /// An event a host cannot name is refused where it is read.
    ///
    /// The compiler states the same rule, so no artifact it writes can carry an empty name — this
    /// is the hand-assembled case, and it is the one that matters: `EMIT`'s only routing
    /// information is the name, so a host receiving an unnamed event has nothing to dispatch on
    /// while the VM reports that it emitted one.
    #[test]
    fn verifier_refuses_an_event_that_names_nothing() {
        let empty_name = payload_code(
            EMIT,
            CapabilityPayload::EmitEvent {
                name: String::new(),
                fields: vec![("arg0".to_string(), "1".to_string())],
            },
        );
        assert!(
            matches!(verify(&empty_name), Err(VerifyError::InvalidOperand(_))),
            "an event with no name must be refused as an invalid operand"
        );

        // The same record with a name verifies, so the refusal above is the name and not the
        // record's shape.
        let named = payload_code(
            EMIT,
            CapabilityPayload::EmitEvent {
                name: "TransferDone".to_string(),
                fields: vec![("arg0".to_string(), "1".to_string())],
            },
        );
        assert!(verify(&named).is_ok(), "the same event with a name must verify");

        let unnamed_argument = payload_code(
            EMIT,
            CapabilityPayload::EmitEvent {
                name: "TransferDone".to_string(),
                fields: vec![(String::new(), "1".to_string())],
            },
        );
        assert!(
            matches!(verify(&unnamed_argument), Err(VerifyError::InvalidOperand(_))),
            "an argument no reader can bind to a parameter must be refused"
        );
    }

    /// `CALL_HOST` carries no selector other than the function's name.
    #[test]
    fn verifier_refuses_a_host_call_that_names_nothing() {
        let empty_function = payload_code(
            CALL_HOST,
            CapabilityPayload::HostCall {
                function: String::new(),
                args: vec![],
            },
        );
        assert!(
            matches!(verify(&empty_function), Err(VerifyError::InvalidOperand(_))),
            "a call naming no function must be refused rather than answered as a no-op"
        );

        let named = payload_code(
            CALL_HOST,
            CapabilityPayload::HostCall {
                function: "charge_subscription".to_string(),
                args: vec!["keeper".to_string(), "100".to_string()],
            },
        );
        assert!(verify(&named).is_ok(), "a named call with arguments must verify");
    }

    /// The bytes the emitter used to write for these two opcodes are refused, not reinterpreted.
    ///
    /// Before this record existed the payload was `"{name}:{data:?}"` — a formatted string with no
    /// length prefix for the name and no field count. It could never be executed (the decoder had no
    /// arm for either opcode), so no artifact anywhere depends on it; the assertion is here because
    /// "nothing depended on it" should be a checked statement rather than an assumption, and because
    /// the new record's first field is also a string, which is exactly the shape a lenient decoder
    /// would read the old payload's prefix as.
    #[test]
    fn the_old_hand_written_event_payload_is_refused_rather_than_misread() {
        let old_form = b"TransferDone:{\"arg0\": \"Literal(Int { value: 1, base: Decimal, suffix: None })\"}";
        let mut bytes = vec![EMIT];
        bytes.extend_from_slice(&(old_form.len() as u16).to_le_bytes());
        bytes.extend_from_slice(old_form);
        while bytes.len() % 4 != 0 {
            bytes.push(0);
        }
        assert!(
            matches!(
                verify(&InstructionStream::new(bytes)),
                Err(VerifyError::InvalidOperand(_))
            ),
            "the old payload is not an `EmitEvent` and must not be read as one"
        );
    }
}

#[cfg(test)]
mod static_guard_quantity_tests {
    use super::*;

    fn require(flags: u8) -> InstructionStream {
        InstructionStream::new(vec![REQUIRE, flags, 1, 0])
    }

    /// A static guard's figure is only readable if the code that says *what it counts* is one this
    /// format defines.
    ///
    /// The set is closed for the reason the measured units' is: a reader that printed an unknown
    /// code would name a quantity the guard is not about. Zero is "no figure carried", which is
    /// what every artifact written before the figure was carried reads as — so this is the
    /// backward-compatibility assertion as much as it is the acceptance of the codes.
    #[test]
    fn verifier_accepts_the_quantities_a_static_guard_can_count_and_refuses_the_rest() {
        assert!(
            verify(&require(require_flags(REQUIRE_COMPARE_STATIC, GUARD_OP_GE))).is_ok(),
            "a static guard carrying no figure must still verify"
        );
        for code in [
            GUARD_QUANTITY_AMOUNT,
            GUARD_QUANTITY_SCORE,
            GUARD_QUANTITY_COUNT,
            GUARD_QUANTITY_BLOCKS,
        ] {
            assert!(
                verify(&require(require_flags_measured(
                    REQUIRE_COMPARE_STATIC,
                    GUARD_OP_GE,
                    code
                )))
                .is_ok(),
                "code {code} is one this format defines"
            );
        }
        // 7 is the highest the three bits can hold and no quantity at all.
        assert!(
            verify(&require(require_flags_measured(REQUIRE_COMPARE_STATIC, GUARD_OP_GE, 7))).is_err(),
            "a code outside the set names a quantity the guard is not about"
        );
        // And a measured guard may not borrow a static quantity's code: the two sets are separate
        // because a measurement and a compile-time figure are different claims.
        assert!(
            verify(&require(require_flags_measured(
                REQUIRE_COMPARE_MEASURED_PROFIT,
                GUARD_OP_GE,
                GUARD_QUANTITY_SCORE
            )))
            .is_err(),
            "a measured guard's code is a measured quantity, not a score"
        );
    }
}
