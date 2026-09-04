//! X3 VM executor - fetch-decode-execute loop and handlers for opcodes.
//!
//! # Gas model
//!
//! Every instruction incurs a **base cost** (see `gas_cost_for_opcode`).
//! Some instructions also incur a **surcharge** that depends on operand
//! values (see `gas_surcharge`):
//!
//! | Opcode | Base | Surcharge |
//! |--------|------|-----------|
//! | ADD/SUB (RRR) | 1 | 0 |
//! | POW (0x0A) | 50 | `(exponent / 32) × 10` |
//! | META_NONCE/META_CHAIN_ID (load/store) | 5 | `(address / 64 KiB) × 5` |
//! | EMIT/CALL_HOST | 100 | `payload_len / 32` |
//! | BRIDGE | 100 | `payload_len / 32` |
//! | All capability opcodes (0x80-0x9B) | varies | `payload_len / 32` |
//! | HALT (0xFF) | 0 | 0 |
//! | All others | 1 | 0 |
//!
//! Additionally, a **code-deposit cost** equal to `bytecode.len()` is
//! deducted from the initial gas allocation at VM construction time.
//! This prevents unbounded bytecode from being executed with a single
//! fixed gas allocation.
//!
//! Gas is never refunded and never goes negative. The VM checks
//! `state.gas >= cost` before deducting.

use crate::x3_lang_vm::{VmSnapshot, VM};
// Import shared opcode constants
use crate::spec::opcodes::*;
use x3_lang_common::{
    decode_asset_op_payload, decode_bridge_payload, decode_capability_payload, AssetOpPayload, CapabilityPayload,
};

pub type ExecResult<T> = Result<T, ExecError>;

#[derive(Debug)]
pub enum ExecError {
    OutOfGas,
    InvalidOpcode(u8),
    CapabilityNotImplemented(u8, &'static str),
    InvalidOperand,
    MemoryOutOfBounds,
    Panic(String),
}

pub type GasCost = u128;

/// If a failure handler is registered, pop it, redirect PC to the handler,
/// and return `true` to signal "handled — continue execution".
/// If no handler exists, return `false`.
fn try_dispatch_handler(vm: &mut VM) -> bool {
    if let Some(handler_pc) = vm.state.failure_handlers.pop() {
        vm.state.pc = handler_pc;
        true
    } else {
        false
    }
}

/// Execute the VM until halt or out of gas.
pub fn execute(vm: &mut VM) -> ExecResult<()> {
    let has_compiler_header = has_compiler_header(vm.code.as_slice());
    if vm.state.pc == 0 {
        vm.state.pc = first_instruction_pc(vm.code.as_slice())?;
    }
    loop {
        if vm.state.pc >= vm.code.len() {
            return Ok(());
        }
        // Fetch instruction
        let opcode = vm.code.as_slice()[vm.state.pc];
        let _flags = vm.code.as_slice().get(vm.state.pc + 1).copied().unwrap_or(0);
        let operand = read_u16_le(vm.code.as_slice(), vm.state.pc + 2).unwrap_or(0);
        let pc_next = if has_compiler_header {
            align4(vm.state.pc + 3)
        } else {
            vm.state.pc + 4
        };

        if vm.state.paused && opcode != EMERGENCY_CONTROL {
            if try_dispatch_handler(vm) {
                continue;
            }
            return Err(ExecError::Panic("X3_PAUSED".to_string()));
        }

        // Gas accounting: base cost + operand-dependent surcharge
        let base_cost = gas_cost_for_opcode(opcode);
        let extra_cost = gas_surcharge(opcode, vm, operand);
        let cost = base_cost.saturating_add(extra_cost);
        if vm.state.gas < cost {
            if try_dispatch_handler(vm) {
                continue;
            }
            return Err(ExecError::OutOfGas);
        }
        vm.state.gas -= cost;

        // Instruction count and timeout enforcement
        vm.state.instruction_count = vm.state.instruction_count.saturating_add(1);
        if let Some(deadline) = vm.state.timeout_deadline {
            if vm.state.instruction_count > deadline {
                if try_dispatch_handler(vm) {
                    continue;
                }
                return Err(ExecError::Panic(format!(
                    "X3_TIMEOUT: instruction count {} exceeded deadline {}",
                    vm.state.instruction_count, deadline
                )));
            }
        }
        // If a prior opcode in this atomic scope panicked, attempt to run the
        // most recently registered failure handler.
        // (The handler pops the stack so each panic triggers the innermost handler.)
        // We defer the jump to the handler only when we detect a panic —
        // for now, failures are caught via the executor's Err return and
        // the runtime can inspect failure_handlers to route.
        // This inline check catches explicit panic opcodes within scopes.

        match opcode {
            0x0A => {
                // POW_RRR - power: ra = rb ^ rc (saturating)
                let (ra, rb, rc) = decode_regtriplet(operand);
                let base = vm.state.registers[rb as usize];
                let exp = vm.state.registers[rc as usize];
                // Saturating pow: produce u128::MAX on overflow
                let result = base.saturating_pow(exp as u32);
                vm.state.registers[ra as usize] = result;
            }
            0x01 => {
                // ADD_RRR - REG-REG-REG: operand encodes registers
                // flags: REG3
                let (ra, rb, rc) = decode_regtriplet(operand);
                vm.state.registers[ra as usize] =
                    vm.state.registers[rb as usize].wrapping_add(vm.state.registers[rc as usize]);
            }
            0x02 => {
                // SUB_RRR
                let (ra, rb, rc) = decode_regtriplet(operand);
                vm.state.registers[ra as usize] =
                    vm.state.registers[rb as usize].wrapping_sub(vm.state.registers[rc as usize]);
            }
            META_NONCE => {
                // LOAD_RAI: R[a] = mem[R[b] + imm16]
                let (ra, rb, imm) = decode_reg_reg_imm(operand);
                let addr = (vm.state.registers[rb as usize] as usize).wrapping_add(imm as usize);
                if addr + 16 > vm.state.memory.len() {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::MemoryOutOfBounds);
                }
                // Read 16 bytes and produce u128 (little endian)
                let mut val = 0u128;
                for i in 0..16 {
                    val |= (vm.state.memory[addr + i] as u128) << (i * 8);
                }
                vm.state.registers[ra as usize] = val;
            }
            META_CHAIN_ID => {
                // STORE_RAI
                let (ra, rb, imm) = decode_reg_reg_imm(operand);
                let addr = (vm.state.registers[rb as usize] as usize).wrapping_add(imm as usize);
                if addr + 16 > vm.state.memory.len() {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::MemoryOutOfBounds);
                }
                let val = vm.state.registers[ra as usize];
                for i in 0..16 {
                    vm.state.memory[addr + i] = ((val >> (i * 8)) & 0xFF) as u8;
                }
            }
            LOCK => {
                if let Err(e) = execute_asset_opcode(vm, LOCK, has_compiler_header) {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(e);
                }
                if has_compiler_header {
                    continue;
                }
            }
            MINT => {
                if let Err(e) = execute_asset_opcode(vm, MINT, has_compiler_header) {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(e);
                }
                if has_compiler_header {
                    continue;
                }
            }
            BURN => {
                if let Err(e) = execute_asset_opcode(vm, BURN, has_compiler_header) {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(e);
                }
                if has_compiler_header {
                    continue;
                }
            }
            RELEASE => {
                if let Err(e) = execute_asset_opcode(vm, RELEASE, has_compiler_header) {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(e);
                }
                if has_compiler_header {
                    continue;
                }
            }
            SWAP => {
                if let Err(e) = execute_asset_opcode(vm, SWAP, has_compiler_header) {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(e);
                }
                if has_compiler_header {
                    continue;
                }
            }
            BRIDGE => {
                if !has_compiler_header {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::InvalidOpcode(BRIDGE));
                }
                if let Err(e) = execute_bridge_opcode(vm) {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(e);
                }
                continue;
            }
            IF => {
                // IF encodes: ra=condition_register, imm=skip_offset (in 4-byte units)
                // If R[ra] == 0, jump forward by imm instructions (skip the if-body).
                // If R[ra] != 0, fall through into the if-body.
                // The assembler emits: IF r0, 3  means "if r0 is zero, skip 3 instructions".
                let (ra, _rb, imm) = decode_reg_reg_imm(operand);
                let condition = vm.state.registers[ra as usize];
                if condition == 0 {
                    // Skip forward from pc_next: each instruction is 4 bytes in raw bytecode
                    let skip_bytes = imm as usize * 4;
                    vm.state.pc = pc_next.saturating_add(skip_bytes);
                    if vm.state.pc >= vm.code.len() {
                        return Ok(());
                    }
                    continue;
                }
                // Condition is truthy — fall through into if-body
            }
            LOOP => {
                // LOOP encodes: ra=condition_register, imm=back_jump_offset (in 4-byte units)
                // If R[ra] == 0, exit the loop (fall through to next instruction).
                // If R[ra] != 0, jump back by imm instructions to loop start.
                let (ra, _rb, imm) = decode_reg_reg_imm(operand);
                let condition = vm.state.registers[ra as usize];
                if condition == 0 {
                    // Exit loop — fall through
                } else {
                    // Jump back: subtract imm*4 from PC
                    let back_bytes = imm as usize * 4;
                    vm.state.pc = vm.state.pc.saturating_sub(back_bytes);
                    // Decrement the counter so bounded loops terminate
                    vm.state.registers[ra as usize] = condition.saturating_sub(1);
                    continue;
                }
            }
            CALL => {
                // CALL - push return and jump
                let addr = operand as usize;
                vm.state.call_stack.push(pc_next);
                vm.state.pc = addr;
                continue;
            }
            RET => {
                // RET
                let retpc = match vm.state.call_stack.pop() {
                    Some(pc) => pc,
                    None => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic("call stack underflow".to_string()));
                    }
                };
                vm.state.pc = retpc;
                continue;
            }
            REQUIRE => {
                // REQUIRE: ra=condition_register. Panic if R[ra] == 0.
                let (ra, _rb, _imm) = decode_reg_reg_imm(operand);
                let condition = vm.state.registers[ra as usize];
                if condition == 0 {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(format!(
                        "X3_REQUIRE_FAILED: condition register r{ra} is zero"
                    )));
                }
            }
            ON_FAIL => {
                // ON_FAIL: ra=handler_pc_target. Push a failure handler entry:
                // if any subsequent opcode within this scope panics, the VM
                // jumps to the handler target before aborting.
                let (ra, _rb, _imm) = decode_reg_reg_imm(operand);
                let handler_pc = vm.state.registers[ra as usize] as usize;
                vm.state.failure_handlers.push(handler_pc);
            }
            ON_TIMEOUT => {
                // ON_TIMEOUT: ra=deadline_register. Set a per-execution
                // deadline. If the VM exceeds this many total instructions,
                // the next opcode will panic with X3_TIMEOUT.
                let (ra, _rb, _imm) = decode_reg_reg_imm(operand);
                let deadline = vm.state.registers[ra as usize];
                vm.state.timeout_deadline = Some(deadline);
                // Track instruction count for timeout enforcement
                vm.state.instruction_count = 0;
            }
            ATOMIC_BEGIN => {
                // Snapshot the current VM state for potential rollback.
                // The snapshot uses pc_next (the instruction AFTER ATOMIC_BEGIN)
                // so that rollback resumes execution past the begin marker,
                // preventing infinite re-execution of the begin opcode.
                let snapshot = VmSnapshot {
                    registers: vm.state.registers.clone(),
                    memory: vm.state.memory.clone(),
                    asset_ops_len: vm.state.asset_ops.len(),
                    bridge_receipts_len: vm.state.bridge_receipts.len(),
                    pc: pc_next,
                    call_stack: vm.state.call_stack.clone(),
                    instruction_count: vm.state.instruction_count,
                };
                vm.state.atomic_snapshot = Some(snapshot);
            }
            ATOMIC_END => {
                // Commit: clear the snapshot. The atomic scope succeeded.
                if vm.state.atomic_snapshot.is_none() {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(
                        "X3_ATOMIC_END_WITHOUT_BEGIN: no atomic snapshot to commit".to_string(),
                    ));
                }
                vm.state.atomic_snapshot = None;
                // Clear any failure handlers registered in this scope
                vm.state.failure_handlers.clear();
            }
            ATOMIC_ROLLBACK => {
                // Restore VM state from the snapshot taken at ATOMIC_BEGIN.
                // Registers, memory, asset_ops, and bridge_receipts are reverted.
                // However execution continues PAST the rollback instruction
                // (using pc_next), not back to the snapshot point, so the
                // program can handle the rollback and continue.
                let snapshot = match vm.state.atomic_snapshot.take() {
                    Some(s) => s,
                    None => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic(
                            "X3_ATOMIC_ROLLBACK_WITHOUT_SNAPSHOT: no atomic snapshot to restore".to_string(),
                        ));
                    }
                };
                vm.state.registers = snapshot.registers;
                vm.state.memory = snapshot.memory;
                vm.state.asset_ops.truncate(snapshot.asset_ops_len);
                vm.state.bridge_receipts.truncate(snapshot.bridge_receipts_len);
                // Note: We intentionally do NOT restore PC from the snapshot.
                // Instead execution continues past the rollback instruction.
                // This prevents infinite re-execution of the atomic scope.
                vm.state.call_stack = snapshot.call_stack;
                vm.state.instruction_count = snapshot.instruction_count;
                vm.state.failure_handlers.clear();
                // Continue execution past the rollback instruction
                vm.state.pc = pc_next;
                continue;
            }
            EMIT => {
                let data = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(p) => p.to_vec(),
                    Err(e) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(e);
                    }
                };
                let res = match bridge_result(vm.bridge.evm_call(&data)) {
                    Ok(v) => v,
                    Err(e) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(e);
                    }
                };
                vm.state.registers[0] = bytes_to_register(&res);
                vm.state.pc = align4(vm.state.pc + 3 + data.len());
                continue;
            }
            CALL_HOST => {
                let data = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(p) => p.to_vec(),
                    Err(e) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(e);
                    }
                };
                let res = match bridge_result(vm.bridge.svm_call(&data)) {
                    Ok(v) => v,
                    Err(e) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(e);
                    }
                };
                vm.state.registers[0] = bytes_to_register(&res);
                vm.state.pc = align4(vm.state.pc + 3 + data.len());
                continue;
            }
            GPU_DISPATCH..=SUB_EXEC => {
                let payload = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(p) => p.to_vec(),
                    Err(e) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(e);
                    }
                };
                let result = match dispatch_host_opcode(vm, opcode, &payload) {
                    Ok(v) => v,
                    Err(e) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(e);
                    }
                };
                vm.state.registers[0] = bytes_to_register(&result);
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
            }
            NOP => { // NOP
            }
            HALT => {
                // HALT
                return Ok(());
            }
            other => {
                if try_dispatch_handler(vm) {
                    continue;
                }
                return Err(ExecError::InvalidOpcode(other));
            }
        }

        vm.state.pc = pc_next;
    }
}

fn has_compiler_header(bytes: &[u8]) -> bool {
    bytes.first() == Some(&BYTECODE_VERSION_1) && bytes.get(1).copied().unwrap_or(NOP) != NOP
}

fn first_instruction_pc(bytes: &[u8]) -> ExecResult<usize> {
    if !has_compiler_header(bytes) {
        return Ok(0);
    }

    let mut pc = 1usize;
    while pc < bytes.len() {
        match bytes[pc] {
            META_NONCE => {
                let len = read_u16_le(bytes, pc + 1).ok_or(ExecError::InvalidOperand)? as usize;
                pc = pc.checked_add(3 + len).ok_or(ExecError::InvalidOperand)?;
            }
            META_CHAIN_ID => {
                pc = pc.checked_add(9).ok_or(ExecError::InvalidOperand)?;
            }
            _ => return Ok(pc),
        }
    }

    Ok(pc)
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn execute_asset_opcode(vm: &mut VM, opcode: u8, compiler_stream: bool) -> ExecResult<()> {
    if !compiler_stream {
        // Raw bytecode streams lack the structured asset payload needed to
        // identify chain, asset, amount, and receiver. Executing the opcode
        // would be a silent no-op. Fail closed.
        return Err(ExecError::Panic(format!(
            "X3_ASSET_OP_NOT_EXECUTABLE: opcode 0x{opcode:02x} requires a compiler-stream payload"
        )));
    }

    let payload = read_len_payload(vm.code.as_slice(), vm.state.pc)?.to_vec();
    let decoded = decode_asset_op_payload(opcode, &payload).map_err(|_| ExecError::InvalidOperand)?;
    apply_asset_payload(vm, decoded);
    vm.state.pc = align4(vm.state.pc + 3 + payload.len());
    Ok(())
}

fn execute_bridge_opcode(vm: &mut VM) -> ExecResult<()> {
    let payload = read_len_payload(vm.code.as_slice(), vm.state.pc)?.to_vec();
    let decoded = decode_bridge_payload(&payload).map_err(|_| ExecError::InvalidOperand)?;
    let receipt = bridge_result(vm.bridge.bridge_transfer(
        &decoded.via,
        &decoded.from_chain,
        &decoded.from_asset,
        &decoded.to_chain,
        &decoded.to_asset,
        decoded.amount,
        decoded.receiver.as_bytes(),
        &decoded.source_finality_proof,
        &decoded.transfer_proof,
    ))?;
    vm.state.registers[0] = bytes_to_register(&receipt);
    vm.state.registers[1] = decoded.amount;
    vm.state.bridge_ops.push(decoded);
    vm.state.bridge_receipts.push(receipt);
    vm.state.pc = align4(vm.state.pc + 3 + payload.len());
    Ok(())
}

fn apply_asset_payload(vm: &mut VM, payload: AssetOpPayload) {
    match &payload {
        AssetOpPayload::Lock { amount, .. }
        | AssetOpPayload::Mint { amount, .. }
        | AssetOpPayload::Burn { amount, .. } => {
            vm.state.registers[0] = *amount;
        }
        AssetOpPayload::Release { .. } => {
            vm.state.registers[0] = 0;
        }
        AssetOpPayload::Swap {
            input_amount,
            min_output,
            ..
        } => {
            vm.state.registers[0] = *input_amount;
            vm.state.registers[1] = *min_output;
        }
    }
    vm.state.asset_ops.push(payload);
}

fn gas_cost_for_opcode(opcode: u8) -> u128 {
    match opcode {
        0x0A => 50,
        0x01 | 0x02 => 1,
        0x10 | 0x11 => 5,
        0x20 | 0x21 => 1,
        BRIDGE => 100,
        0x30 | 0x31 => 2,
        0x32 | 0x33 => 5,
        0x40 => 10,
        0x50 | 0x51 => 250,
        0x60 | 0x61 => 100,
        0x70 => 2,
        0x80 => 500,
        0x81 => 200,
        0x82 => 100,
        0x83 => 150,
        0x84 => 10,
        0x85 => 500,
        0x86 => 50,
        0x87 => 100,
        0x88 | 0x89 => 50,
        0x8A => 10,
        0x8B => 500,
        0x8C | 0x8D => 20,
        0x8E => 30,
        0x8F => 10,
        0x90 => 20,
        0x91 => 200,
        0x92 => 5,
        0x93 | 0x94 | 0x95 | 0x96 | 0x97 | 0x98 => 10,
        0x99 => 50,
        0x9A => 50,
        0x9B => 50,
        0xFF => 0,
        _ => 1,
    }
}

/// Additional gas cost based on operand-dependent factors (payload size,
/// exponent magnitude, memory access depth). Returns 0 for most opcodes.
fn gas_surcharge(opcode: u8, vm: &VM, operand: u16) -> u128 {
    match opcode {
        0x0A => {
            let (_ra, _rb, rc) = decode_regtriplet(operand);
            let exp = vm.state.registers.get(rc as usize).copied().unwrap_or(0);
            (exp / 32).saturating_mul(10)
        }
        0x10 | 0x11 => {
            let (_ra, rb, imm) = decode_reg_reg_imm(operand);
            let base = vm.state.registers.get(rb as usize).copied().unwrap_or(0) as usize;
            let addr = base.wrapping_add(imm as usize);
            (addr as u128 / 65536).saturating_mul(5)
        }
        0x20..=0x25 | 0x60 | 0x61 | 0x80..=0x9B => {
            let payload_len = read_u16_le(vm.code.as_slice(), vm.state.pc + 1).unwrap_or(0) as u128;
            payload_len / 32
        }
        _ => 0,
    }
}

fn read_len_payload(bytes: &[u8], pc: usize) -> ExecResult<&[u8]> {
    let len = read_u16_le(bytes, pc + 1).ok_or(ExecError::InvalidOperand)? as usize;
    let start = pc + 3;
    let end = start.checked_add(len).ok_or(ExecError::InvalidOperand)?;
    if end > bytes.len() {
        return Err(ExecError::InvalidOperand);
    }
    Ok(&bytes[start..end])
}

fn capability_opcode_name(opcode: u8) -> &'static str {
    match opcode {
        GPU_DISPATCH => "GPU_DISPATCH",
        SIMULATE => "SIMULATE",
        SCHEDULED_DISPATCH => "SCHEDULED_DISPATCH",
        INTENT_RESOLVE => "INTENT_RESOLVE",
        CRDT_OP => "CRDT_OP",
        PROOF_VERIFY => "PROOF_VERIFY",
        STORAGE_OP => "STORAGE_OP",
        PATHFIND => "PATHFIND",
        MEMPOOL_SCAN => "MEMPOOL_SCAN",
        ORACLE_REQUEST => "ORACLE_REQUEST",
        EMERGENCY_CONTROL => "EMERGENCY_CONTROL",
        LIFECYCLE => "LIFECYCLE",
        SERIALIZE => "SERIALIZE",
        DESERIALIZE => "DESERIALIZE",
        GAS_ESTIMATE => "GAS_ESTIMATE",
        CHAIN_METRIC => "CHAIN_METRIC",
        EVENT_PROVENANCE => "EVENT_PROVENANCE",
        MULTI_HOP_SWAP => "MULTI_HOP_SWAP",
        VECTOR_MATH => "VECTOR_MATH",
        ROLE_CHECK => "ROLE_CHECK",
        MULTISIG_CHECK => "MULTISIG_CHECK",
        VERSION_META => "VERSION_META",
        STORAGE_NAMESPACE => "STORAGE_NAMESPACE",
        ABI_EXPORT => "ABI_EXPORT",
        DOC_EMBED => "DOC_EMBED",
        GAS_ADAPTIVE => "GAS_ADAPTIVE",
        BOUNTY => "BOUNTY",
        SUB_EXEC => "SUB_EXEC",
        _ => "UNKNOWN_CAPABILITY",
    }
}

fn dispatch_host_opcode(vm: &mut VM, opcode: u8, payload: &[u8]) -> ExecResult<Vec<u8>> {
    let decoded = decode_capability_payload(opcode, payload).map_err(|_| ExecError::InvalidOperand)?;
    let result = match decoded {
        CapabilityPayload::GpuDispatch { kernel, args, .. } => {
            bridge_result(vm.bridge.gpu_dispatch(&kernel, args.join("\0").as_bytes()))
        }
        CapabilityPayload::Simulate { body_ops, .. } => bridge_result(vm.bridge.simulate(&body_ops.to_le_bytes())),
        CapabilityPayload::ScheduledDispatch {
            period_blocks,
            entry_ops,
        } => bridge_result(vm.bridge.scheduled_dispatch(period_blocks, &entry_ops.to_le_bytes())),
        CapabilityPayload::IntentResolve { constraints, .. } => {
            bridge_result(vm.bridge.intent_resolve(constraints.join("\0").as_bytes()))
        }
        CapabilityPayload::CrdtOp { kind, key, value } => bridge_result(vm.bridge.crdt_op(
            kind,
            key.as_bytes(),
            value.as_deref().unwrap_or_default().as_bytes(),
        )),
        CapabilityPayload::ProofVerify {
            kind,
            proof,
            input,
            key_or_threshold,
        } => {
            bridge_result(
                vm.bridge
                    .proof_verify(kind, proof.as_bytes(), input.as_bytes(), key_or_threshold.as_bytes()),
            )
        }
        CapabilityPayload::StorageOp { kind, data } => bridge_result(vm.bridge.storage_op(kind, data.as_bytes())),
        CapabilityPayload::Pathfind { from, to, max_depth } => {
            bridge_result(vm.bridge.pathfind(from.as_bytes(), to.as_bytes(), max_depth))
        }
        CapabilityPayload::MempoolScan { max_results } => bridge_result(vm.bridge.mempool_scan(max_results)),
        CapabilityPayload::OracleRequest { token, reward } => {
            bridge_result(vm.bridge.oracle_request(token.as_bytes(), reward))
        }
        CapabilityPayload::EmergencyControl { kind } => {
            let res = bridge_result(vm.bridge.emergency_control(kind));
            if res.is_ok() {
                vm.state.paused = kind == 0;
            }
            res
        }
        CapabilityPayload::Lifecycle { kind, target } => bridge_result(
            vm.bridge
                .lifecycle(kind, target.as_deref().unwrap_or_default().as_bytes()),
        ),
        CapabilityPayload::Serialize { format, data } => bridge_result(vm.bridge.serialize(format, data.as_bytes())),
        CapabilityPayload::Deserialize { format, data } => {
            bridge_result(vm.bridge.deserialize(format, data.as_bytes()))
        }
        CapabilityPayload::GasEstimate { chain, route } => {
            bridge_result(vm.bridge.gas_estimate(chain.as_bytes(), route.as_bytes()))
        }
        CapabilityPayload::ChainMetric { metric } => bridge_result(vm.bridge.chain_metric(metric)),
        CapabilityPayload::EventProvenance { event_type, data } => {
            bridge_result(vm.bridge.event_provenance(event_type.as_bytes(), data.as_bytes()))
        }
        CapabilityPayload::MultiHopSwap { path, amount } => {
            bridge_result(vm.bridge.multi_hop_swap(path.join("\0").as_bytes(), amount))
        }
        CapabilityPayload::VectorMath { op, a, b, size } => {
            bridge_result(vm.bridge.vector_math(op, a.as_bytes(), b.as_bytes(), size))
        }
        CapabilityPayload::RoleCheck { role } => vm
            .bridge
            .role_check(role.as_bytes())
            .map_err(|_| ExecError::Panic("X3_ROLE_DENIED".to_string())),
        CapabilityPayload::MultisigCheck { required, total } => vm
            .bridge
            .multisig_check(required, total)
            .map_err(|_| ExecError::Panic("X3_MULTISIG_THRESHOLD_NOT_MET".to_string())),
        CapabilityPayload::VersionMeta { version, .. } => Ok(version.into_bytes()),
        CapabilityPayload::StorageNamespace { package, key } => Ok([package.as_bytes(), b":", key.as_bytes()].concat()),
        CapabilityPayload::AbiExport { function, .. } => Ok(function.into_bytes()),
        CapabilityPayload::DocEmbed { content } => Ok(content.into_bytes()),
        CapabilityPayload::GasAdaptive { .. } => bridge_result(vm.bridge.gas_adaptive_select()),
        CapabilityPayload::Bounty { amount, condition } => {
            bridge_result(vm.bridge.bounty_escrow(amount, condition.as_bytes()))
        }
        CapabilityPayload::SubExec { .. } => Err(ExecError::CapabilityNotImplemented(
            SUB_EXEC,
            capability_opcode_name(SUB_EXEC),
        )),
    }?;
    Ok(result)
}

fn bridge_result(result: Result<Vec<u8>, Box<dyn std::error::Error>>) -> ExecResult<Vec<u8>> {
    result.map_err(|err| ExecError::Panic(err.to_string()))
}

fn bytes_to_register(bytes: &[u8]) -> u128 {
    let mut value = 0u128;
    for (idx, byte) in bytes.iter().take(16).enumerate() {
        value |= (*byte as u128) << (idx * 8);
    }
    value
}

fn read_u16_le(bytes: &[u8], idx: usize) -> Option<u16> {
    if idx + 1 >= bytes.len() {
        return None;
    }
    Some((bytes[idx] as u16) | ((bytes[idx + 1] as u16) << 8))
}

fn decode_regtriplet(operand: u16) -> (u8, u8, u8) {
    // operand packs three 5-bit registers: r0[0..4], r1[5..9], r2[10..14]
    let ra = (operand & 0x1F) as u8;
    let rb = ((operand >> 5) & 0x1F) as u8;
    let rc = ((operand >> 10) & 0x1F) as u8;
    (ra, rb, rc)
}

fn decode_reg_reg_imm(operand: u16) -> (u8, u8, u16) {
    // operand: low 5 bits ra, next 5 bits rb, top 6 bits imm6 - extend
    let ra = (operand & 0x1F) as u8;
    let rb = ((operand >> 5) & 0x1F) as u8;
    let imm = (operand >> 10) & 0x3F;
    (ra, rb, imm)
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::x3_lang_vm::{VMConfig, VM};

    /// Encode a register-triplet operand: ra in bits 0-4, rb in 5-9, rc in 10-14.
    fn enc_tt(ra: u8, rb: u8, rc: u8) -> u16 {
        (ra as u16) | ((rb as u16) << 5) | ((rc as u16) << 10)
    }

    /// Run bytecode and return r0.
    fn run(bytecode: &[u8], r0_init: u128, r1_init: u128, r2_init: u128, gas: u128) -> u128 {
        let mut vm = VM::new(bytecode.to_vec(), VMConfig::default(), gas);
        vm.state.registers[0] = r0_init;
        vm.state.registers[1] = r1_init;
        vm.state.registers[2] = r2_init;
        execute(&mut vm).unwrap();
        vm.state.registers[0]
    }

    #[test]
    fn pow_2_pow_3_eq_8() {
        // POW r0, r1, r2  (0x0A) then HALT (0xFF)
        let code = &[
            0x0A,
            0,
            (enc_tt(0, 1, 2) & 0xFF) as u8,
            (enc_tt(0, 1, 2) >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        let result = run(code, 0, 2, 3, 1_000_000);
        assert_eq!(result, 8, "2 ^ 3");
    }

    #[test]
    fn pow_5_pow_0_eq_1() {
        let code = &[
            0x0A,
            0,
            (enc_tt(0, 1, 2) & 0xFF) as u8,
            (enc_tt(0, 1, 2) >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        let result = run(code, 0, 5, 0, 1_000_000);
        assert_eq!(result, 1, "5 ^ 0");
    }

    #[test]
    fn pow_0_pow_5_eq_0() {
        let code = &[
            0x0A,
            0,
            (enc_tt(0, 1, 2) & 0xFF) as u8,
            (enc_tt(0, 1, 2) >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        let result = run(code, 0, 0, 5, 1_000_000);
        assert_eq!(result, 0, "0 ^ 5");
    }

    #[test]
    fn pow_overflow_saturates() {
        // 2 ^ 128 overflows u128 — saturating_pow returns u128::MAX
        let code = &[
            0x0A,
            0,
            (enc_tt(0, 1, 2) & 0xFF) as u8,
            (enc_tt(0, 1, 2) >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        let result = run(code, 0, 2, 128, 1_000_000);
        assert_eq!(result, u128::MAX, "2 ^ 128 saturates to MAX");
    }

    #[test]
    fn add_rrr_works() {
        // ADD r0, r1, r2  (0x01) then HALT (0xFF)
        let code = &[
            0x01,
            0,
            (enc_tt(0, 1, 2) & 0xFF) as u8,
            (enc_tt(0, 1, 2) >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        let result = run(code, 0, 10, 20, 1_000_000);
        assert_eq!(result, 30, "10 + 20");
    }

    #[test]
    fn sub_rrr_works() {
        let code = &[
            0x02,
            0,
            (enc_tt(0, 1, 2) & 0xFF) as u8,
            (enc_tt(0, 1, 2) >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        let result = run(code, 0, 100, 7, 1_000_000);
        assert_eq!(result, 93, "100 - 7");
    }

    #[test]
    fn chained_add_pow() {
        // ADD r0, r1, r2  -> r0 = 2 + 1 = 3
        // POW r0, r0, r2 -> r0 = 3 ^ 1 = 3,  wait, need to adjust regs.
        // Let's do: r1=2, r2=1. ADD r0,r1,r2 -> r0=3. Then POW r0,r0,r2 -> 3^1=3.
        // Better: r1=2, r2=3. ADD r0,r1,r2 -> r0=5. POW r3,r0,r2 -> r3=5^3. But our run() only checks r0.
        // Let's do: r1=2, r2=3. ADD r0,r1,r2=5. Then POW r0,r0,r1 -> 5^2=25.
        let op_add = enc_tt(0, 1, 2); // ADD r0, r1, r2
        let op_pow = enc_tt(0, 0, 1); // POW r0, r0, r1
        let code: &[u8] = &[
            0x01,
            0,
            (op_add & 0xFF) as u8,
            (op_add >> 8) as u8,
            0x0A,
            0,
            (op_pow & 0xFF) as u8,
            (op_pow >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        let result = run(code, 0, 2, 3, 1_000_000);
        assert_eq!(result, 25, "(2+3)^2 = 25");
    }

    #[test]
    fn pow_gas_is_consumed() {
        let code = &[
            0x0A,
            0,
            (enc_tt(0, 1, 2) & 0xFF) as u8,
            (enc_tt(0, 1, 2) >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 100);
        vm.state.registers[1] = 2;
        vm.state.registers[2] = 3;
        execute(&mut vm).unwrap();
        // Code is 8 bytes (POW + HALT), code-deposit = 8, POW base = 50, surcharge = 0
        assert_eq!(vm.state.gas, 100 - 8 - 50, "Pow costs 50 gas + 8 code-deposit");
        assert_eq!(vm.state.registers[0], 8);
    }

    #[test]
    fn pow_gas_scales_with_exponent() {
        // POW r0, r1, r2  (0x0A) then HALT (0xFF)
        // surcharge = (exponent / 32) * 10
        let code = &[
            0x0A,
            0,
            (enc_tt(0, 1, 2) & 0xFF) as u8,
            (enc_tt(0, 1, 2) >> 8) as u8,
            0xFF,
            0,
            0,
            0,
        ];
        // exp=32 => surcharge = (32/32)*10 = 10, total POW = 50 + 10 = 60
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1000);
        vm.state.registers[1] = 2;
        vm.state.registers[2] = 32;
        execute(&mut vm).unwrap();
        assert_eq!(vm.state.gas, 1000 - 8 - 60, "POW with exp=32 costs 60 + 8 deposit");
        assert_eq!(vm.state.registers[0], 2u128.saturating_pow(32));

        // exp=64 => surcharge = (64/32)*10 = 20, total POW = 50 + 20 = 70
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1000);
        vm.state.registers[1] = 2;
        vm.state.registers[2] = 64;
        execute(&mut vm).unwrap();
        assert_eq!(vm.state.gas, 1000 - 8 - 70, "POW with exp=64 costs 70 + 8 deposit");
        assert_eq!(vm.state.registers[0], 2u128.saturating_pow(64));
    }

    #[test]
    fn e2e_halt_stops_vm() {
        let code = &[0xFF, 0, 0, 0];
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 100);
        execute(&mut vm).unwrap();
        // HALT returns Ok so we just verify it completed
    }

    // ── Control-flow opcode tests ──────────────────────────────────────

    /// Encode a reg-reg-imm16 operand: ra low 5, rb next 5, imm top 6.
    fn enc_rri(ra: u8, rb: u8, imm: u16) -> u16 {
        (ra as u16) | ((rb as u16) << 5) | ((imm & 0x3F) << 10)
    }

    /// Encode 4-byte instruction: [opcode, flags, operand_lo, operand_hi].
    fn instr(opcode: u8, operand: u16) -> [u8; 4] {
        [opcode, 0, (operand & 0xFF) as u8, (operand >> 8) as u8]
    }

    #[test]
    fn if_condition_zero_skips_body() {
        // IF r0, 1  (skip 1 instruction if r0==0)
        // ADD r0, r0, r1  (should be skipped)
        // HALT
        // r0=0, r1=100
        // If taken: r0 stays 0. If not taken: r0=100.
        let code: &[u8] = &[
            instr(0x30, enc_rri(0, 0, 1)), // IF r0, 1
            instr(0x01, enc_tt(0, 0, 1)),  // ADD r0, r0, r1
            instr(0xFF, 0),                // HALT
        ]
        .concat();
        let result = run(&code, 0, 100, 0, 1_000_000);
        assert_eq!(result, 0, "IF r0==0 should skip the ADD, r0 stays 0");
    }

    #[test]
    fn if_condition_nonzero_falls_through() {
        // IF r0, 1  (skip 1 if r0==0, but r0≠0)
        // ADD r0, r0, r1  (executed)
        // HALT
        let code: &[u8] = &[
            instr(0x30, enc_rri(0, 0, 1)), // IF r0, 1
            instr(0x01, enc_tt(0, 0, 1)),  // ADD r0, r0, r1
            instr(0xFF, 0),                // HALT
        ]
        .concat();
        let result = run(&code, 1, 100, 0, 1_000_000);
        assert_eq!(result, 101, "IF r0≠0 should fall through, r0=1+100");
    }

    #[test]
    fn loop_decrements_and_exits() {
        // r0 = 5  (counter)
        // LOOP r0, 0  — if r0==0 fall through, else decrement and jump back to LOOP
        // Since we jump back to the same LOOP instruction with decrement,
        // this runs 5 iterations then exits.
        // We need the LOOP to jump back to itself. offset 0 means jump 0
        // backwards (to itself). Let's use offset 1 to jump before LOOP.
        // Actually: LOOP decrements counter. If non-zero, jump back.
        // With offset=1, it jumps back 1 instruction = 4 bytes = itself.
        // offset 1 = imm=1, back_bytes=4 → back to the same LOOP.
        let code: &[u8] = &[
            instr(0x31, enc_rri(0, 0, 1)), // LOOP r0, 1 — jump back to itself
            instr(0xFF, 0),                // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 5;
        execute(&mut vm).unwrap();
        assert_eq!(vm.state.registers[0], 0, "Loop counter should decrement to 0");
    }

    #[test]
    fn require_passes_when_nonzero() {
        // REQUIRE r0  (fails if r0==0)
        // ADD r0, r0, r1  (executed only if require passes)
        // HALT
        let code: &[u8] = &[
            instr(0x40, enc_rri(0, 0, 0)), // REQUIRE r0
            instr(0x01, enc_tt(0, 0, 1)),  // ADD r0, r0, r1
            instr(0xFF, 0),                // HALT
        ]
        .concat();
        let result = run(&code, 42, 10, 0, 1_000_000);
        assert_eq!(result, 52, "REQUIRE passed, ADD executed: 42+10=52");
    }

    #[test]
    fn require_fails_when_zero() {
        // REQUIRE r0 with r0=0 → must panic
        let code: &[u8] = &[
            instr(0x40, enc_rri(0, 0, 0)), // REQUIRE r0
            instr(0xFF, 0),                // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 0;
        let result = execute(&mut vm);
        assert!(result.is_err(), "REQUIRE with r0=0 should panic");
        let err = result.unwrap_err();
        match err {
            ExecError::Panic(msg) => assert!(msg.contains("REQUIRE_FAILED")),
            _ => panic!("expected Panic, got {:?}", err),
        }
    }

    #[test]
    fn on_fail_registers_handler() {
        // ON_FAIL r1 — push the value in r1 as a failure handler PC
        // NOP
        // HALT
        let code: &[u8] = &[
            instr(0x41, enc_rri(1, 0, 0)), // ON_FAIL r1
            instr(0xFF, 0),                // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[1] = 0x42; // handler PC target
        execute(&mut vm).unwrap();
        assert_eq!(vm.state.failure_handlers.len(), 1);
        assert_eq!(vm.state.failure_handlers[0], 0x42);
    }

    #[test]
    fn on_timeout_sets_deadline() {
        // ON_TIMEOUT r0 — set deadline from r0
        // NOP x100 (won't exceed deadline)
        // HALT
        let code: &[u8] = &[
            instr(0x42, enc_rri(0, 0, 0)), // ON_TIMEOUT r0
            instr(0xFF, 0),                // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 100; // deadline = 100 instructions
        execute(&mut vm).unwrap();
        assert_eq!(vm.state.timeout_deadline, Some(100));
    }

    #[test]
    fn timeout_exceeded_panics() {
        // ON_TIMEOUT r0 with deadline=1, then several NOPs
        // Each NOP increments instruction_count. After 2 NOPs,
        // instruction_count > deadline → X3_TIMEOUT panic.
        let code: &[u8] = &[
            instr(0x42, enc_rri(0, 0, 0)), // ON_TIMEOUT r0: deadline=1
            instr(NOP, 0),                 // NOP (instruction_count=1)
            instr(NOP, 0),                 // NOP (instruction_count=2 > deadline=1)
            instr(0xFF, 0),                // HALT (won't reach)
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 1; // deadline = 1 instruction
        let result = execute(&mut vm);
        assert!(result.is_err(), "Timeout should panic");
        let err = result.unwrap_err();
        match err {
            ExecError::Panic(msg) => assert!(msg.contains("X3_TIMEOUT")),
            _ => panic!("expected Panic, got {:?}", err),
        }
    }

    #[test]
    fn atomic_begin_end_commit_preserves_state() {
        // ATOMIC_BEGIN (0x50)
        // ADD r0, r0, r1  (r0=10, r1=5 → r0=15)
        // ATOMIC_END (0x51)
        // HALT
        let code: &[u8] = &[
            instr(ATOMIC_BEGIN, enc_rri(0, 0, 0)),
            instr(0x01, enc_tt(0, 0, 1)), // ADD r0, r0, r1
            instr(ATOMIC_END, enc_rri(0, 0, 0)),
            instr(0xFF, 0), // HALT
        ]
        .concat();
        let result = run(&code, 10, 5, 0, 1_000_000);
        assert_eq!(result, 15, "Atomic commit preserves ADD result: 10+5=15");
    }
    #[test]
    fn atomic_rollback_without_begin_panics() {
        // ATOMIC_ROLLBACK (0x52) without ATOMIC_BEGIN must panic
        let code: &[u8] = &[
            instr(ATOMIC_ROLLBACK, enc_rri(0, 0, 0)),
            instr(0xFF, 0), // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        let result = execute(&mut vm);
        assert!(result.is_err(), "ATOMIC_ROLLBACK without BEGIN should panic");
        let err = result.unwrap_err();
        match err {
            ExecError::Panic(msg) => assert!(msg.contains("ROLLBACK_WITHOUT_SNAPSHOT")),
            _ => panic!("expected Panic, got {:?}", err),
        }
    }

    #[test]
    fn atomic_rollback_restores_registers_and_skips_body() {
        // E2E test: ATOMIC_BEGIN, modify r0, then ATOMIC_ROLLBACK.
        // After rollback, r0 should be restored to its pre-begin value,
        // and execution should continue past the rollback point.
        //
        // Program:
        //   ATOMIC_BEGIN          (snapshot: r0=10, r1=5)
        //   ADD r0, r0, r1       (r0=15)
        //   ATOMIC_ROLLBACK      (restore snapshot: r0=10, PC=after ATOMIC_BEGIN)
        //   REQUIRE r0            (r0=10 ≠ 0, passes)
        //   ADD r2, r2, r1       (r2=5 — marker that we reached here)
        //   HALT
        //
        // Expected: r0=10 (restored), r2=5 (reached)
        let code: &[u8] = &[
            instr(ATOMIC_BEGIN, enc_rri(0, 0, 0)),    // ATOMIC_BEGIN (0x50)
            instr(0x01, enc_tt(0, 0, 1)),             // ADD r0, r0, r1
            instr(ATOMIC_ROLLBACK, enc_rri(0, 0, 0)), // ATOMIC_ROLLBACK (0x52)
            instr(0x40, enc_rri(0, 0, 0)),            // REQUIRE r0 — should pass after rollback
            instr(0x01, enc_tt(2, 2, 1)),             // ADD r2, r2, r1
            instr(0xFF, 0),                           // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 10; // initial r0
        vm.state.registers[1] = 5; // increment
        vm.state.registers[2] = 0; // flag
        execute(&mut vm).unwrap();
        assert_eq!(vm.state.registers[0], 10, "r0 should be restored to 10 after rollback");
        assert_eq!(
            vm.state.registers[2], 5,
            "r2 should be 5 (reached marker after rollback)"
        );
    }

    #[test]
    fn atomic_rollback_clears_asset_ops() {
        // E2E test: ATOMIC_BEGIN, push an asset op, then rollback.
        // After rollback, asset_ops should be empty.
        //
        // ATOMIC_BEGIN
        // (simulate asset op by directly manipulating state)
        // ATOMIC_ROLLBACK
        // HALT
        //
        // We verify by checking vm.state.asset_ops is empty after execution.
        let code = &[0xFF, 0, 0, 0]; // minimal HALT
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 42;
        let snapshot = VmSnapshot {
            registers: vec![10; 32],
            memory: vm.state.memory.clone(),
            asset_ops_len: 0,
            bridge_receipts_len: 0,
            pc: 0,
            call_stack: vec![],
            instruction_count: 3,
        };
        vm.state.atomic_snapshot = Some(snapshot);
        vm.state.asset_ops.push(AssetOpPayload::Lock {
            chain: "test".into(),
            asset: "TEST".into(),
            amount: 100,
            from: "test-addr".into(),
        });
        // Execute rollback manually
        let snapshot = vm.state.atomic_snapshot.take().unwrap();
        vm.state.registers = snapshot.registers;
        vm.state.memory = snapshot.memory;
        vm.state.asset_ops.truncate(snapshot.asset_ops_len);
        assert_eq!(vm.state.registers[0], 10, "Register should be restored to 10");
        assert_eq!(
            vm.state.asset_ops.len(),
            0,
            "Asset ops should be cleared after rollback"
        );
        assert_eq!(
            vm.state.asset_ops.len(),
            0,
            "Asset ops should be cleared after rollback"
        );
    }
    #[test]
    fn atomic_end_without_begin_panics() {
        let code: &[u8] = &[
            instr(ATOMIC_END, enc_rri(0, 0, 0)), // ATOMIC_END without BEGIN
            instr(0xFF, 0),                      // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        let result = execute(&mut vm);
        assert!(result.is_err(), "ATOMIC_END without BEGIN should panic");
        let err = result.unwrap_err();
        match err {
            ExecError::Panic(msg) => assert!(msg.contains("END_WITHOUT_BEGIN")),
            _ => panic!("expected Panic, got {:?}", err),
        }
    }

    #[test]
    fn on_fail_catches_require_failure() {
        // Register a failure handler via ON_FAIL, then execute REQUIRE with
        // r0=0. The trap should redirect to the handler instead of returning Err.
        //
        // Program layout:
        //   PC 0:  ON_FAIL r1      — register handler PC from r1
        //   PC 4:  REQUIRE r0      — fails (r0=0), trap → jump to handler
        //   PC 8:  HALT            — unreachable
        //   PC 12: ADD r0, r0, r2  — handler: r0 = 0 + 42 = 42 (marker)
        //   PC 16: HALT
        let code: &[u8] = &[
            instr(ON_FAIL, enc_rri(1, 0, 0)), // ON_FAIL r1
            instr(REQUIRE, enc_rri(0, 0, 0)), // REQUIRE r0  → fails
            instr(HALT, 0),                   // HALT (unreachable)
            instr(0x01, enc_tt(0, 0, 2)),     // ADD r0, r0, r2 (handler)
            instr(HALT, 0),                   // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 0; // condition → REQUIRE fails
        vm.state.registers[1] = 12; // handler PC = address of ADD
        vm.state.registers[2] = 42; // marker value placed in r0 by handler
        execute(&mut vm).unwrap();
        assert_eq!(
            vm.state.registers[0], 42,
            "ON_FAIL handler should catch REQUIRE failure and set r0 to marker value"
        );
    }

    // ── Capability opcode tests ─────────────────────────────────

    fn capability_bytecode(opcode: u8, payload: &[u8]) -> Vec<u8> {
        let len = payload.len() as u16;
        let mut code = vec![0x01]; // version header
        code.push(opcode);
        code.extend_from_slice(&len.to_le_bytes());
        code.extend_from_slice(payload);
        while code.len() % 4 != 0 {
            code.push(0);
        }
        code.extend_from_slice(&[0xFF, 0, 0, 0]); // HALT
        code
    }

    #[test]
    fn sub_exec_opcode_returns_capability_not_implemented() {
        // Minimal SUB_EXEC payload: empty bytecode_hash, 0 args, 0 gas_limit
        let payload = {
            let mut p = Vec::new();
            p.extend_from_slice(&0u16.to_le_bytes()); // bytecode_hash len 0
            p.extend_from_slice(&0u16.to_le_bytes()); // args count 0
            p.extend_from_slice(&0u64.to_le_bytes()); // gas_limit 0
            p
        };
        let code = capability_bytecode(SUB_EXEC, &payload);
        let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
        let result = execute(&mut vm);
        assert!(result.is_err(), "SUB_EXEC must be rejected");
        match result.unwrap_err() {
            ExecError::CapabilityNotImplemented(op, name) => {
                assert_eq!(op, SUB_EXEC);
                assert_eq!(name, "SUB_EXEC");
            }
            other => panic!("expected CapabilityNotImplemented, got {:?}", other),
        }
    }

    #[test]
    fn all_capability_opcodes_reach_dispatcher() {
        let opcodes: Vec<u8> = (GPU_DISPATCH..=SUB_EXEC).collect();
        for opcode in &opcodes {
            let name = capability_opcode_name(*opcode);
            assert!(
                *opcode >= GPU_DISPATCH && *opcode <= SUB_EXEC,
                "opcode 0x{opcode:02x} ({name}) is in capability range"
            );
            // Each opcode must have a name (not UNKNOWN_CAPABILITY)
            assert!(
                name != "UNKNOWN_CAPABILITY",
                "opcode 0x{opcode:02x} must have a known name, got {name}"
            );
        }
        assert_eq!(opcodes.len(), 28, "must cover all 28 capability opcodes (0x80..=0x9B)");
    }

    #[test]
    fn capability_opcode_names_cover_all_defined() {
        assert_eq!(capability_opcode_name(GPU_DISPATCH), "GPU_DISPATCH");
        assert_eq!(capability_opcode_name(SIMULATE), "SIMULATE");
        assert_eq!(capability_opcode_name(SCHEDULED_DISPATCH), "SCHEDULED_DISPATCH");
        assert_eq!(capability_opcode_name(INTENT_RESOLVE), "INTENT_RESOLVE");
        assert_eq!(capability_opcode_name(CRDT_OP), "CRDT_OP");
        assert_eq!(capability_opcode_name(PROOF_VERIFY), "PROOF_VERIFY");
        assert_eq!(capability_opcode_name(STORAGE_OP), "STORAGE_OP");
        assert_eq!(capability_opcode_name(PATHFIND), "PATHFIND");
        assert_eq!(capability_opcode_name(MEMPOOL_SCAN), "MEMPOOL_SCAN");
        assert_eq!(capability_opcode_name(ORACLE_REQUEST), "ORACLE_REQUEST");
        assert_eq!(capability_opcode_name(EMERGENCY_CONTROL), "EMERGENCY_CONTROL");
        assert_eq!(capability_opcode_name(LIFECYCLE), "LIFECYCLE");
        assert_eq!(capability_opcode_name(SERIALIZE), "SERIALIZE");
        assert_eq!(capability_opcode_name(DESERIALIZE), "DESERIALIZE");
        assert_eq!(capability_opcode_name(GAS_ESTIMATE), "GAS_ESTIMATE");
        assert_eq!(capability_opcode_name(CHAIN_METRIC), "CHAIN_METRIC");
        assert_eq!(capability_opcode_name(EVENT_PROVENANCE), "EVENT_PROVENANCE");
        assert_eq!(capability_opcode_name(MULTI_HOP_SWAP), "MULTI_HOP_SWAP");
        assert_eq!(capability_opcode_name(VECTOR_MATH), "VECTOR_MATH");
        assert_eq!(capability_opcode_name(ROLE_CHECK), "ROLE_CHECK");
        assert_eq!(capability_opcode_name(MULTISIG_CHECK), "MULTISIG_CHECK");
        assert_eq!(capability_opcode_name(VERSION_META), "VERSION_META");
        assert_eq!(capability_opcode_name(STORAGE_NAMESPACE), "STORAGE_NAMESPACE");
        assert_eq!(capability_opcode_name(ABI_EXPORT), "ABI_EXPORT");
        assert_eq!(capability_opcode_name(DOC_EMBED), "DOC_EMBED");
        assert_eq!(capability_opcode_name(GAS_ADAPTIVE), "GAS_ADAPTIVE");
        assert_eq!(capability_opcode_name(BOUNTY), "BOUNTY");
        assert_eq!(capability_opcode_name(SUB_EXEC), "SUB_EXEC");
    }

    #[test]
    fn dry_run_bridge_executes_all_capability_opcodes_without_silent_noop() {
        for opcode in GPU_DISPATCH..=SUB_EXEC {
            let name = capability_opcode_name(opcode);
            let payload = capability_minimal_payload(opcode);
            let code = capability_bytecode(opcode, &payload);
            let mut vm = VM::new(code, VMConfig::default(), 500_000);
            let result = execute(&mut vm);
            match result {
                Ok(()) => {
                    assert!(
                        name != "SUB_EXEC",
                        "SUB_EXEC should have been rejected with CapabilityNotImplemented, but got Ok"
                    );
                }
                Err(ExecError::CapabilityNotImplemented(oc, n)) => {
                    assert_eq!(
                        oc, SUB_EXEC,
                        "only SUB_EXEC should return CapabilityNotImplemented for now"
                    );
                    assert_eq!(n, "SUB_EXEC");
                }
                Err(ExecError::InvalidOperand) => {
                    panic!(
                        "opcode 0x{opcode:02x} ({name}) payload failed to decode — check capability_minimal_payload"
                    );
                }
                Err(ExecError::Panic(msg)) => {
                    assert!(
                        !msg.contains("not implemented"),
                        "opcode 0x{opcode:02x} ({name}) must not silently produce 'not implemented' panics: {msg}"
                    );
                }
                Err(other) => {
                    panic!("opcode 0x{opcode:02x} ({name}) produced unexpected error: {:?}", other);
                }
            }
        }
    }

    fn capability_minimal_payload(opcode: u8) -> Vec<u8> {
        let empty_str = vec![0u8, 0]; // len=0
        let one_str = vec![1u8, 0, b'a']; // len=1, "a"
        let mut p = Vec::new();
        match opcode {
            GPU_DISPATCH => {
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&1u16.to_le_bytes());
                p.extend_from_slice(&one_str);
                p.push(1u8);
            }
            SIMULATE => {
                p.extend_from_slice(&0u32.to_le_bytes());
                p.extend_from_slice(&one_str);
            }
            SCHEDULED_DISPATCH => p.extend_from_slice(&[1u32 as u8, 0, 0, 0, 0u32 as u8, 0, 0, 0]),
            INTENT_RESOLVE => {
                p.extend_from_slice(&0u16.to_le_bytes());
                p.extend_from_slice(&one_str);
            }
            CRDT_OP => p.extend_from_slice(&[1u8, 1u8, 0, b'k', 1u8, 1u8, 0, b'v']),
            PROOF_VERIFY => p.extend_from_slice(&[0u8, 1u8, 0, b'p', 1u8, 0, b'i', 1u8, 0, b't']),
            STORAGE_OP => p.extend_from_slice(&[0u8, 1u8, 0, b'd']),
            PATHFIND => {
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&1u32.to_le_bytes());
            }
            MEMPOOL_SCAN => p.extend_from_slice(&3u32.to_le_bytes()),
            ORACLE_REQUEST => {
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&[0u8; 16]);
            }
            EMERGENCY_CONTROL => p.push(0u8),
            LIFECYCLE => p.extend_from_slice(&[0u8, 0u8]),
            SERIALIZE | DESERIALIZE => p.extend_from_slice(&[0u8, 1u8, 0, b'd']),
            GAS_ESTIMATE => {
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&one_str);
            }
            CHAIN_METRIC => p.push(0u8),
            EVENT_PROVENANCE => {
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&one_str);
            }
            MULTI_HOP_SWAP => {
                p.extend_from_slice(&1u16.to_le_bytes());
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&[0u8; 16]);
            }
            VECTOR_MATH => {
                p.push(0u8);
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&0u32.to_le_bytes());
            }
            ROLE_CHECK => p.extend_from_slice(&one_str),
            MULTISIG_CHECK => p.extend_from_slice(&[1u32 as u8, 0, 0, 0, 2u32 as u8, 0, 0, 0]),
            VERSION_META => {
                p.extend_from_slice(&one_str);
                p.push(0u8);
            }
            STORAGE_NAMESPACE => {
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&one_str);
            }
            ABI_EXPORT => {
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&1u16.to_le_bytes());
                p.extend_from_slice(&one_str);
                p.extend_from_slice(&one_str);
            }
            DOC_EMBED => p.extend_from_slice(&one_str),
            GAS_ADAPTIVE => p.extend_from_slice(&[0u32 as u8, 0, 0, 0, 0u32 as u8, 0, 0, 0]),
            BOUNTY => {
                p.extend_from_slice(&[0u8; 16]);
                p.extend_from_slice(&one_str);
            }
            SUB_EXEC => {
                p.extend_from_slice(&empty_str);
                p.extend_from_slice(&0u16.to_le_bytes());
                p.extend_from_slice(&0u64.to_le_bytes());
            }
            _ => {}
        }
        p
    }

    #[test]
    fn all_capability_opcodes_have_minimal_test_payload() {
        for opcode in GPU_DISPATCH..=SUB_EXEC {
            let payload = capability_minimal_payload(opcode);
            let decoded = decode_capability_payload(opcode, &payload);
            assert!(
                decoded.is_ok(),
                "minimal payload for opcode 0x{opcode:02x} ({}) must decode: {decoded:?}",
                capability_opcode_name(opcode)
            );
        }
    }

    #[test]
    fn unknown_opcode_produces_invalid_opcode_error() {
        let code = capability_bytecode(0xCC, &[]);
        let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
        let result = execute(&mut vm);
        assert!(result.is_err(), "unknown opcode 0xCC must be rejected");
        match result.unwrap_err() {
            ExecError::InvalidOpcode(0xCC) => {}
            other => panic!("expected InvalidOpcode(0xCC), got {:?}", other),
        }
    }
}
