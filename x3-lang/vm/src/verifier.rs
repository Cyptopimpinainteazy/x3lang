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
    let mut pc = first_instruction_pc(bytes);
    while pc + 4 <= bytes.len() {
        if bytes[pc..].iter().all(|byte| *byte == 0) {
            break;
        }
        boundaries.insert(pc);
        let opcode = bytes[pc];
        if !valid_opcode(opcode) {
            return Err(VerifyError::InvalidOpcode(opcode, pc));
        }
        if is_payload_opcode(opcode, compiler_stream) {
            let payload = read_payload(bytes, pc)?;
            validate_payload_opcode(opcode, payload, pc)?;
            pc = align4(pc + 3 + payload.len());
            continue;
        }

        let _flags = bytes[pc + 1];
        let operand = u16::from_le_bytes([bytes[pc + 2], bytes[pc + 3]]);
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
        pc += 4;
    }
    Ok(boundaries)
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
    let mut pc = 1usize;
    loop {
        if pc + 3 > bytes.len() {
            return pc;
        }
        match bytes[pc] {
            META_NONCE => {
                // nonce metadata: 2-byte length followed by UTF-8 nonce.
                let len = u16::from_le_bytes([bytes[pc + 1], bytes[pc + 2]]) as usize;
                // Deliberately no alignment. `emit_x3ir` writes this record
                // unpadded and the executor's own `first_instruction_pc` walks
                // it unpadded, so the verifier has to agree with both. It used
                // to round up to a 4-byte boundary, which desynchronised the
                // walk for any nonce whose record length was 1 or 3 bytes short
                // of a multiple of four: the validator then read metadata bytes
                // as opcodes and rejected bytecode the executor runs fine.
                pc += 3 + len;
            }
            META_CHAIN_ID => {
                // chain_id metadata: an 8-byte u64 payload, so the record is
                // nine bytes. This read a u32 and advanced five, which put the
                // cursor three bytes into the next instruction.
                pc += 9;
            }
            _ => return pc,
        }
    }
}

fn has_compiler_header(bytes: &[u8]) -> bool {
    // Same rule as the executor's `has_compiler_header`: a version byte
    // followed by a real record. A stream of `[0x01][0x00..]` is raw bytecode
    // that happens to start with the version byte, not a compiler stream.
    bytes.first() == Some(&BYTECODE_VERSION_1) && bytes.get(1).copied().unwrap_or(NOP) != NOP
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

fn validate_payload_opcode(opcode: u8, payload: &[u8], pc: usize) -> Result<(), VerifyError> {
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
            AssetOpPayload::Release { chain, asset, to } => {
                if chain.is_empty() || asset.is_empty() || to.is_empty() {
                    return Err(VerifyError::InvalidOperand(pc));
                }
            }
            AssetOpPayload::Swap {
                from_chain,
                from_asset,
                to_asset,
                input_amount,
                ..
            } => {
                if from_chain.is_empty() || from_asset.is_empty() || to_asset.is_empty() || input_amount == 0 {
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
        _ => {}
    }
    Ok(())
}

fn valid_opcode(op: u8) -> bool {
    // Accept every opcode the emitter can produce, including
    // asset ops (0x20-0x24), control (0x30-0x33), guards
    // (0x40-0x44), atomic (0x50-0x52), emit/call (0x60-0x66),
    // vector (0x70-0x73), capability payloads (0x80-0x9B),
    // extras (0xA0-0xAB) and the trading core (0xB0-0xBA). Halt (0xFF)
    // and reserved (0x00-0x18) are also valid. Anything outside
    // 0x00-0xFF is impossible.
    //
    // The trading range was missing, so `verify` rejected every bytecode a
    // trading-core program produces: `x3c run examples/trading_core_v1.x3`
    // failed with `X3_VERIFY_FAILED: InvalidOpcode(176, 1)` — 0xB0 is
    // TRADING_BEGIN, the first instruction of the stream. The compiler's
    // disassembler already carries this range, and carries a comment about
    // having been fixed for the same reason.
    op <= 0xAB || (TRADING_BEGIN..=TRADING_BRIDGE).contains(&op) || op == HALT
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
        let mut bytes = vec![BYTECODE_VERSION_1];
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
        assert_eq!(expected, 19, "the record must be 18 bytes plus the version byte");

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
    fn chain_id_metadata_is_walked_as_nine_bytes() {
        // The record is `[0x11][u64]`; reading it as a u32 advanced only five
        // bytes and put the cursor three bytes into the next instruction.
        let (code, expected) = compiler_stream_with(Some("nonce_1"), Some(0x0123_4567_89AB_CDEF));
        assert_eq!(expected, 1 + 10 + 9, "nonce record then nine bytes of chain id");

        let boundaries = verify(&code).expect("a stream with a chain id must verify");
        assert!(
            boundaries.contains(&expected),
            "the first instruction must be found at {expected}, got {boundaries:?}"
        );
    }
}
