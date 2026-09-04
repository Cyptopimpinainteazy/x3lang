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
            0x10 => {
                // nonce metadata: 2-byte length followed by UTF-8 nonce.
                let len = u16::from_le_bytes([bytes[pc + 1], bytes[pc + 2]]) as usize;
                pc = ((pc + 3 + len) + 3) & !3;
            }
            0x11 => {
                // chain_id metadata: 4-byte u32 payload.
                pc += 5;
            }
            _ => return pc,
        }
    }
}

fn has_compiler_header(bytes: &[u8]) -> bool {
    bytes.first() == Some(&BYTECODE_VERSION_1)
}

fn is_payload_opcode(op: u8, compiler_stream: bool) -> bool {
    (compiler_stream && matches!(op, LOCK | MINT | BURN | RELEASE | SWAP | BRIDGE))
        || (GPU_DISPATCH..=SUB_EXEC).contains(&op)
}

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
    // and extras (0xA0-0xA5). Halt (0xFF) and reserved (0x00-0x18)
    // are also valid. Anything outside 0x00-0xFF is impossible.
    op <= 0xA5 || op == HALT
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
}
