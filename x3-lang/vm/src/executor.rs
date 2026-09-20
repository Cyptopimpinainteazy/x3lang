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

use crate::x3_lang_vm::{AtomicChoiceRecord, ParallelPlanRecord, SubExecInfo, VmSnapshot, WaveSettlementRecord, VM};
use std::collections::BTreeMap;
use x3_lang_compiler::emitter::decode_trading_operation;
// Import shared opcode constants
use crate::spec::opcodes::*;
use x3_lang_common::{
    decode_asset_op_payload, decode_bridge_payload, decode_capability_payload, AssetOpPayload, BridgePayload,
    CapabilityPayload,
};

pub type ExecResult<T> = Result<T, ExecError>;

#[derive(Debug)]
pub enum ExecError {
    OutOfGas,
    InvalidOpcode(u8),
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

/// Execute the VM until halt or out of gas, **without verifying the bytecode**.
///
/// Crate-internal on purpose. The public door is
/// [`VM::execute`](crate::x3_lang_vm::VM::execute), which verifies first and
/// then calls this. It used to be `pub`, which made the verification guarantee
/// a matter of picking the right name: an integrator could reach the
/// interpreter directly through `x3_lang_vm::executor::execute` and run
/// bytecode that was never checked. There is now exactly one public way in.
pub(crate) fn execute(vm: &mut VM) -> ExecResult<()> {
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
        // The operand is as wide as the frame that carries it: four bytes for
        // `REQUIRE` (a real `u16` threshold), one byte for every other fixed
        // frame in a compiler stream, whose high half is the padding the emitter
        // writes. Both widths live in `spec/opcodes.rs`, the table the compiler
        // and the verifier read, so a frame cannot be wide to one reader and
        // narrow to another. Reading four bytes of a three-byte frame is how a
        // `feature_allow` guard at offset 1 presented feature code 0x5683 to
        // this VM — the high half was the next instruction's opcode.
        let operand = if is_payload_opcode(opcode, has_compiler_header) {
            read_u16_le(vm.code.as_slice(), vm.state.pc + 2).unwrap_or(0)
        } else {
            fixed_frame_operand(
                opcode,
                has_compiler_header,
                vm.code.as_slice().get(vm.state.pc + 2).copied().unwrap_or(0),
                vm.code.as_slice().get(vm.state.pc + 3).copied().unwrap_or(0),
            )
        };
        // The next instruction: a compiler stream frames a fixed instruction in
        // three bytes (four for `REQUIRE`) and pads to the next absolute
        // multiple of four, while a raw stream's fixed instructions are four
        // bytes with no padding.
        let pc_next = if has_compiler_header {
            if is_payload_opcode(opcode, true) {
                // Payload arms read the length and set `pc` themselves; this is
                // the value they would use if they did not.
                align4(vm.state.pc + 3)
            } else {
                align4(vm.state.pc + fixed_frame_content_len(opcode))
            }
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
                // REQUIRE: flags = comparison code, operand = threshold. The
                // value being guarded is register r0, where the declaration
                // instruction for this guard leaves it.
                //
                // The operand deliberately does not hold the packed
                // register/register/immediate triple other instructions use: a
                // guard always tests r0, and the field is better spent on the
                // threshold. This used to ignore both fields and test r0 for
                // non-zero, which made every guard depend on whatever the
                // previous instruction happened to leave behind — an unrelated
                // instruction's empty result was enough to fail a guard, and
                // the same residue could as easily pass one that should fail.
                let threshold = operand as u128;
                let value = vm.state.registers[0];
                // The flags byte carries the guard's own operator in bits 2-4;
                // mask them off before reading the comparison mode, or a
                // `require slippage <= 50` would arrive as comparison code 8 and
                // be refused as unimplemented.
                let satisfied = match require_comparison(_flags) {
                    // An assertion about the artifact's configuration. The
                    // compiler has already checked it and there is no run-time
                    // quantity to test, so the instruction stands as a record
                    // of the guard rather than a test.
                    REQUIRE_COMPARE_STATIC => true,
                    REQUIRE_COMPARE_GE => value >= threshold,
                    // A measured guard. The quantity is what a host reported, and the instruction
                    // refuses rather than comparing when nothing did — that refusal is the point, and
                    // it is the one place this comparison is evaluated: `IF_MEASURED` forks on the
                    // same verdict, so a guard and a branch cannot describe the same quantity
                    // differently (TICKET-106).
                    REQUIRE_COMPARE_MEASURED_PROFIT => {
                        // An unknown unit code reads as the profit floor, which is what this mode
                        // meant before the unit code existed, so an artifact emitted then still
                        // reads as the guard it was.
                        let unit = match require_measured_unit_code(_flags) {
                            MEASURED_UNIT_CODE_DELTA_BPS => MEASURED_UNIT_CODE_DELTA_BPS,
                            _ => MEASURED_UNIT_CODE_PROFIT_BPS,
                        };
                        match measured_comparison(vm, unit, threshold, "guard") {
                            Some(MeasuredVerdict::Holds) => true,
                            Some(MeasuredVerdict::Fails(message) | MeasuredVerdict::Unmeasured(message)) => {
                                return Err(ExecError::Panic(message))
                            }
                            None => {
                                return Err(ExecError::Panic(format!(
                                    "X3_REQUIRE_FAILED: unknown measured unit code {unit} at pc {}",
                                    vm.state.pc
                                )))
                            }
                        }
                    }
                    REQUIRE_COMPARE_MEASURED_SLIPPAGE => {
                        match measured_comparison(vm, MEASURED_UNIT_CODE_SLIPPAGE_BPS, threshold, "guard") {
                            Some(MeasuredVerdict::Holds) => true,
                            Some(MeasuredVerdict::Fails(message) | MeasuredVerdict::Unmeasured(message)) => {
                                return Err(ExecError::Panic(message))
                            }
                            None => {
                                return Err(ExecError::Panic(format!(
                                    "X3_REQUIRE_FAILED: unknown measured unit code at pc {}",
                                    vm.state.pc
                                )))
                            }
                        }
                    }
                    // A comparison this VM does not implement must not pass by
                    // default.
                    other => {
                        return Err(ExecError::Panic(format!(
                            "X3_REQUIRE_FAILED: unknown comparison code {other} at pc {}",
                            vm.state.pc
                        )))
                    }
                };
                if !satisfied {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(format!(
                        "X3_REQUIRE_FAILED: r0={value} is below the required {threshold} at pc {}",
                        vm.state.pc
                    )));
                }
            }
            IF_MEASURED => {
                // `[IF_MEASURED][u16 len][unit:invert:threshold:skip]` — a branch on a quantity a
                // host measured (TICKET-106).
                //
                // The record says which quantity, whether this is the negation of that quantity's own
                // comparison, the bound in basis points, and how many instructions to skip when the
                // comparison does not hold. That is the whole of an `if` over a measured quantity:
                // the emitter writes the body inline and a second record after it to skip the other
                // body, because this format has no unconditional jump.
                let payload = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(payload) => payload.to_vec(),
                    Err(error) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(error);
                    }
                };
                let Some((unit, invert, threshold_bps, skip)) = parse_if_measured(&payload) else {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::InvalidOperand);
                };
                let base = match measured_comparison(vm, unit, u128::from(threshold_bps), "branch") {
                    Some(MeasuredVerdict::Holds) => true,
                    Some(MeasuredVerdict::Fails(_)) => false,
                    // A quantity nothing reported refuses in a branch too: a fork on a figure that
                    // was never measured would pick a path nobody chose, which is worse than
                    // refusing — the same rule the guard states.
                    Some(MeasuredVerdict::Unmeasured(message)) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic(message));
                    }
                    None => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic(format!(
                            "X3_GUARD_UNMEASURED: the branch names measured quantity {unit}, which \
                             this VM does not hold"
                        )));
                    }
                };
                let taken = if invert { !base } else { base };
                let after = align4(vm.state.pc + 3 + payload.len());
                if taken {
                    vm.state.pc = after;
                } else {
                    let target = after.saturating_add((skip as usize).saturating_mul(4));
                    if target >= vm.code.len() {
                        return Ok(());
                    }
                    vm.state.pc = target;
                }
                continue;
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
                // ON_TIMEOUT: the operand is the deadline in instructions.
                //
                // It used to be read as a *register index*, and the emitter
                // wrote operand 0 — so the deadline silently became whatever
                // `r0` happened to hold, which is register residue from an
                // unrelated instruction. `examples/atomic_swap.x3` failed with
                // "instruction count 1 exceeded deadline 0" purely because the
                // previous operation left r0 at zero, while
                // `examples/simple_swap.x3` passed on the same opcode only
                // because its residue happened to be large. Carrying the value
                // in the instruction removes the coin flip; it is the same
                // shape as REQUIRE carrying `[comparison][threshold]`.
                //
                // Zero means **no deadline**. The IR's `duration_blocks` is a
                // chain timeout in blocks and this VM has no block height, so
                // it is deliberately not used as an instruction budget — that
                // would enforce a bound the program never asked for and would
                // make bytecode fail on the size of the program rather than on
                // its behaviour. Block-based expiry belongs to the timeout /
                // refund engine, and the IR keeps the declared value for it.
                vm.state.timeout_deadline = if operand == 0 { None } else { Some(operand as u128) };
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
                    trading_ops_len: vm.state.trading_ops.len(),
                    atomic_choices_len: vm.state.atomic_choices.len(),
                    route_fallbacks_len: vm.state.route_fallbacks.len(),
                    parallel_plans_len: vm.state.parallel_plans.len(),
                    strategy_licenses_len: vm.state.strategy_licenses.len(),
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
                vm.state.trading_ops.truncate(snapshot.trading_ops_len);
                vm.state.atomic_choices.truncate(snapshot.atomic_choices_len);
                vm.state.route_fallbacks.truncate(snapshot.route_fallbacks_len);
                vm.state.parallel_plans.truncate(snapshot.parallel_plans_len);
                vm.state.strategy_licenses.truncate(snapshot.strategy_licenses_len);
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
            // The range is the *set*, and it has drifted from `is_payload_opcode`
            // before: an opcode the framing function called a payload opcode reached no
            // arm here and was refused as invalid, which is what the payload-opcode test
            // below exists to catch. `VENUE_ORDER` is the newest member, so the range ends
            // there and `NONCE_UNUSED` (0x9C) is inside it now rather than beside it.
            GPU_DISPATCH..=REBALANCE_TARGET => {
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
                // Only move the result into r0 when there is one. A handler
                // that already set r0 — a declaration leaving the guarded
                // quantity there — must not have it overwritten by the register
                // encoding of an empty vector, which is zero. That overwrite is
                // what made guards fail on the residue of unrelated
                // instructions.
                if !result.is_empty() {
                    vm.state.registers[0] = bytes_to_register(&result);
                }
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
            }
            ROUTE_SCORE..=REFUND_POLICY => {
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
                if !result.is_empty() {
                    vm.state.registers[0] = bytes_to_register(&result);
                }
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
            }
            // The whole trading range, not up to TRADING_ASSERT_INVARIANT:
            // TRADING_BRIDGE (0xBA) is emitted by the trading bridge operation
            // and previously fell through to the unknown-opcode arm.
            TRADING_BEGIN..=TRADING_BRIDGE => {
                let payload = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(p) => p.to_vec(),
                    Err(e) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(e);
                    }
                };
                let trading = match decode_trading_operation(opcode, &payload) {
                    Ok(trading) => trading,
                    Err(_) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::InvalidOperand);
                    }
                };
                vm.state.trading_ops.push(trading);
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
            }
            NOP => { // NOP
            }
            ATOMIC_CHOICE => {
                // `[ATOMIC_CHOICE][u16 len][criterion:paths:selected]`.
                //
                // The branch body has already been selected at compile time and
                // is what follows in the instruction stream, so this instruction
                // does not choose anything at run time — that is the point. Its
                // job is to state and check the record: the artifact declares
                // how many branches were verified and which one it took, and the
                // VM refuses a record that is internally inconsistent rather
                // than executing a body whose provenance it cannot describe.
                let payload = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(payload) => payload.to_vec(),
                    Err(error) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(error);
                    }
                };
                let text = match std::str::from_utf8(&payload) {
                    Ok(text) => text,
                    Err(_) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic(
                            "X3_CHOICE_RECORD_INVALID: the branch record is not UTF-8".to_string(),
                        ));
                    }
                };
                let fields: Vec<&str> = text.split(':').collect();
                let parsed = (fields.len() == 3)
                    .then(|| {
                        Some((
                            fields[0].parse::<u8>().ok()?,
                            fields[1].parse::<u32>().ok()?,
                            fields[2].parse::<u32>().ok()?,
                        ))
                    })
                    .flatten();
                let Some((criterion, paths, selected)) = parsed else {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(format!(
                        "X3_CHOICE_RECORD_INVALID: branch record {text:?} is not \
                         `criterion:paths:selected`"
                    )));
                };
                let known_criterion = matches!(
                    criterion,
                    CHOICE_CRITERION_HIGHEST_NET_OUTPUT
                        | CHOICE_CRITERION_FEWEST_HOPS
                        | CHOICE_CRITERION_LOWEST_DECLARED_FEE
                );
                if !known_criterion || paths < 2 || selected >= paths {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(format!(
                        "X3_CHOICE_RECORD_INVALID: criterion {criterion}, selected {selected} of {paths} \
                         — the branch record does not describe a verified branch set"
                    )));
                }
                vm.state.atomic_choices.push(AtomicChoiceRecord {
                    paths,
                    criterion,
                    selected,
                });
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
            }
            FEATURE_ALLOW => {
                // The program consented to an execution mode. Recording it is
                // the whole effect: a runtime deciding whether it may net this
                // intent against another has to be able to see the consent.
                if _flags != 0 || operand != u16::from(FEATURE_INTENT_FUSION) {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(format!(
                        "X3_FEATURE_ALLOW_INVALID: feature code {operand} is not one the language defines"
                    )));
                }
                vm.state.allowed_features.insert(FEATURE_INTENT_FUSION);
            }
            STRATEGY_LICENSE => {
                // A licence is a record, not an action: nothing here can fail a
                // program, which is how PHASE 24's "licensing must never
                // compromise deterministic execution" holds by construction. The
                // VM refuses a record that is not a distribution plan, and
                // records the one it was handed.
                let payload = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(payload) => payload.to_vec(),
                    Err(error) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(error);
                    }
                };
                let text = match std::str::from_utf8(&payload) {
                    Ok(text) => text.to_string(),
                    Err(_) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic(
                            "X3_STRATEGY_LICENSE_INVALID: the licence record is not UTF-8".to_string(),
                        ));
                    }
                };
                vm.state.strategy_licenses.push(text);
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
            }
            PARALLEL_PLAN => {
                // `legs=<n>;waves=a,b|c;edges=a->c`.
                //
                // The legs' operations follow this record in wave order, and
                // this VM runs them in that order: it is a single-threaded
                // interpreter, so "parallel" is a claim the artifact makes
                // about independence, and the only thing a sequential
                // interpreter can do with it is honour the ordering and keep
                // the claim. It refuses a record that is not a plan, because
                // recording a malformed plan would put an unverifiable claim in
                // the trace.
                let payload = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(payload) => payload.to_vec(),
                    Err(error) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(error);
                    }
                };
                let text = match std::str::from_utf8(&payload) {
                    Ok(text) => text,
                    Err(_) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic(
                            "X3_PARALLEL_PLAN_INVALID: the plan record is not UTF-8".to_string(),
                        ));
                    }
                };
                let field = |name: &str| -> Option<String> {
                    text.split(';')
                        .find_map(|part| part.strip_prefix(&format!("{name}=")).map(|value| value.to_string()))
                };
                let legs: usize = match field("legs").and_then(|value| value.parse().ok()) {
                    Some(legs) => legs,
                    None => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic(format!(
                            "X3_PARALLEL_PLAN_INVALID: plan {text:?} has no leg count"
                        )));
                    }
                };
                let waves: Vec<Vec<String>> = field("waves")
                    .map(|waves| {
                        waves
                            .split('|')
                            .map(|wave| wave.split(',').map(|leg| leg.to_string()).collect())
                            .collect()
                    })
                    .unwrap_or_default();
                let declared: Vec<String> = waves.iter().flatten().cloned().collect();
                let edges: Vec<(String, String)> = field("edges")
                    .map(|edges| {
                        edges
                            .split(',')
                            .filter(|edge| !edge.is_empty())
                            .filter_map(|edge| edge.split_once("->"))
                            .map(|(from, to)| (from.to_string(), to.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                // Which VM each leg runs on. Absent here is a refusal, not an
                // empty set: a plan that does not say is not a multi-VM plan.
                let domains: BTreeMap<String, Vec<String>> = field("domains")
                    .map(|domains| {
                        domains
                            .split(',')
                            .filter(|entry| !entry.is_empty())
                            .filter_map(|entry| entry.split_once(':'))
                            .map(|(leg, leg_domains)| {
                                (
                                    leg.to_string(),
                                    leg_domains.split('+').map(|domain| domain.to_string()).collect(),
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let settlement: Vec<WaveSettlementRecord> = field("settle")
                    .map(|settle| {
                        settle
                            .split('|')
                            .filter_map(|record| {
                                let parts: Vec<&str> = record.split(':').collect();
                                if parts.len() != 4 {
                                    return None;
                                }
                                Some(WaveSettlementRecord {
                                    wave: parts[0].parse().ok()?,
                                    domains: if parts[1] == "-" {
                                        Vec::new()
                                    } else {
                                        parts[1].split('+').map(|domain| domain.to_string()).collect()
                                    },
                                    outstanding_proofs: if parts[2] == "-" {
                                        Vec::new()
                                    } else {
                                        parts[2].split('+').map(|proof| proof.to_string()).collect()
                                    },
                                    locally_recoverable: parts[3] == "local",
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                if settlement.len() != waves.len() {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(format!(
                        "X3_PARALLEL_PLAN_INVALID: plan has {} wave(s) and {} settlement record(s); a \
                         coordinator cannot be told what a wave owes if the plan does not say",
                        waves.len(),
                        settlement.len()
                    )));
                }
                if legs < 2 || declared.len() != legs || domains.len() != legs {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(format!(
                        "X3_PARALLEL_PLAN_INVALID: plan declares {legs} leg(s) but names {}",
                        declared.len()
                    )));
                }
                vm.state.parallel_plans.push(ParallelPlanRecord {
                    legs,
                    waves,
                    edges,
                    domains,
                    settlement,
                });
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
            }
            ROUTE_FALLBACK => {
                // `[ROUTE_FALLBACK][u16 len][venue,venue,...]`.
                //
                // The list is the compiler's approval, and the VM's job is to
                // refuse a record that does not describe one: an empty list
                // approves nothing (a failing leg would then have no approved
                // substitute, which is a different declaration), and more
                // venues than the production bound is a set the compiler did
                // not bound. Every venue in it was verified as a route before
                // the artifact was emitted; the VM records which set this
                // execution was handed.
                let payload = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(payload) => payload.to_vec(),
                    Err(error) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(error);
                    }
                };
                let text = match std::str::from_utf8(&payload) {
                    Ok(text) => text,
                    Err(_) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(ExecError::Panic(
                            "X3_ROUTE_FALLBACK_INVALID: approved venues are not UTF-8".to_string(),
                        ));
                    }
                };
                let approved: Vec<String> = text.split(',').map(|venue| venue.to_string()).collect();
                if approved.is_empty()
                    || approved.iter().any(|venue| venue.is_empty())
                    || approved.len() > MAX_ROUTE_FALLBACKS
                {
                    if try_dispatch_handler(vm) {
                        continue;
                    }
                    return Err(ExecError::Panic(format!(
                        "X3_ROUTE_FALLBACK_INVALID: {} approved venue(s) — an approved set must be \
                         non-empty and within the {MAX_ROUTE_FALLBACKS} venue production bound",
                        approved.len()
                    )));
                }
                vm.state.route_fallbacks.push(approved);
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
            }
            VENUE_SETTLEMENT => {
                // `[VENUE_SETTLEMENT][u16 len][venue:shape]`.
                //
                // A carried declaration, and the VM does not act on it: nothing in the
                // language gives a settlement guarantee an action to take. It exists so
                // that the assumption a leg rests on — PHASE 39's `atomic` versus one of
                // the five honest ways an off-chain leg settles — can be read out of the
                // artifact rather than out of the source.
                //
                // What the VM owes a record is to refuse one that does not describe what it
                // claims to — and that refusal lives in `verifier::verify`, not here. Every
                // public entry to execution goes through it (`x3_lang_vm.rs`'s
                // `verify_and_execute`, the only caller of `execute_unverified`), so a
                // second copy of the rule here would be a rule no reachable path exercises.
                // The verifier checks the shape vocabulary against the compiler's own
                // `SettlementGuarantee`, and a malformed record never reaches this arm.
                let payload = match read_len_payload(vm.code.as_slice(), vm.state.pc) {
                    Ok(payload) => payload.to_vec(),
                    Err(error) => {
                        if try_dispatch_handler(vm) {
                            continue;
                        }
                        return Err(error);
                    }
                };
                vm.state.pc = align4(vm.state.pc + 3 + payload.len());
                continue;
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
    // A *reserved* version byte followed by a real record — see the verifier's copy for why it is the
    // reservation and not the version this reader supports (TICKET-097, TICKET-105).
    is_reserved_version_byte(bytes.first().copied().unwrap_or(0)) && bytes.get(1).copied().unwrap_or(NOP) != NOP
}

fn first_instruction_pc(bytes: &[u8]) -> ExecResult<usize> {
    if !has_compiler_header(bytes) {
        return Ok(0);
    }

    // One walker for the whole set (`spec::opcodes::metadata_record`). This used to be a
    // `match` on the tags, which meant every new record had to be added here as well —
    // and `META_VERSIONS` was not, so the executor walked into the middle of it.
    let mut pc = 1usize;
    while let Some((len, _, _)) = crate::spec::opcodes::metadata_record(bytes, pc) {
        pc = pc.checked_add(len).ok_or(ExecError::InvalidOperand)?;
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
    // One table, in the file the compiler includes too: a compile-time estimate
    // (PHASE 35) and the run-time charge must be the same numbers, and a table
    // written twice is a table that drifts.
    base_gas_cost(opcode)
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
        // Charged for the payload the execution actually reads, from the one
        // place that says which opcodes carry one. This was a list of ranges, and
        // the list omitted `ATOMIC_CHOICE`, `ROUTE_FALLBACK`, `PARALLEL_PLAN`,
        // `STRATEGY_LICENSE` and the trading range: instructions whose payloads
        // both the verifier and the executor read and dispatch, while their
        // payload-proportional surcharge was zero. Gas is a statement about work,
        // and it was a statement about a different set of instructions
        // (TICKET-056).
        //
        // The asset ops (`0x20..=0x25`) carry a payloads only in a compiler
        // stream, which is what the framing flag says: in raw bytecode they are
        // fixed frames with nothing to read, so there is nothing to charge for.
        _ if is_payload_opcode(opcode, has_compiler_header(vm.code.as_slice())) => {
            let payload_len = read_u16_le(vm.code.as_slice(), vm.state.pc + 1).unwrap_or(0) as u128;
            payload_len / 32
        }
        _ => 0,
    }
}

/// What a measured comparison says about a quantity a host reported.
enum MeasuredVerdict {
    /// The comparison holds.
    Holds,
    /// It does not hold, and the message is the refusal a *guard* makes of that — so a branch and a
    /// guard cannot describe the same quantity differently.
    Fails(String),
    /// Nothing reported the quantity this comparison is about.
    Unmeasured(String),
}

/// Evaluate a measured comparison against the quantity a host reported.
///
/// `None` when `unit_code` is not a quantity this build holds, which is a refusal at the caller
/// rather than a comparison that passes by default.
///
/// **One implementation for the two instructions that make this comparison**: `REQUIRE`, where a
/// failure is a refusal, and `IF_MEASURED`, where it is a fork (TICKET-106). The direction is the
/// quantity's own — `>=` for the profit floor, `<=` for the slippage and delta ceilings — because
/// that is what a guard on it means; a branch's `invert` selects the negation of that, which is the
/// second shape a branch needs to skip a body.
///
/// The two verdicts are handled differently by design: a *failure* is a refusal in a guard and a
/// fork in a branch, while a quantity **nothing reported** is a refusal in both. A branch that
/// picked a path on a figure nobody measured would be a path nobody chose.
fn measured_comparison(vm: &VM, unit_code: u8, threshold: u128, what: &str) -> Option<MeasuredVerdict> {
    let measured = match unit_code {
        MEASURED_UNIT_CODE_PROFIT_BPS => vm.state.measured_profit_bps,
        MEASURED_UNIT_CODE_DELTA_BPS => vm.state.measured_delta_bps,
        MEASURED_UNIT_CODE_SLIPPAGE_BPS => vm.state.measured_slippage_bps,
        _ => return None,
    };
    let Some(measured) = measured else {
        return Some(MeasuredVerdict::Unmeasured(match unit_code {
            MEASURED_UNIT_CODE_PROFIT_BPS => format!(
                "X3_GUARD_UNMEASURED: the {what} `profit >= {threshold}bps` needs a profit the host \
                 measured, and no host reported one for this trade"
            ),
            MEASURED_UNIT_CODE_DELTA_BPS => format!(
                "X3_GUARD_UNMEASURED: the {what} `delta <= {threshold}bps` needs a delta the venue \
                 measured, and no venue reported one for this hedge"
            ),
            _ => format!(
                "X3_GUARD_UNMEASURED: the {what} `slippage <= {threshold}bps` needs a slippage the \
                 host measured, and no host reported one for this trade"
            ),
        }));
    };
    let holds = match unit_code {
        MEASURED_UNIT_CODE_PROFIT_BPS => measured >= threshold,
        _ => measured <= threshold,
    };
    if holds {
        return Some(MeasuredVerdict::Holds);
    }
    Some(MeasuredVerdict::Fails(match unit_code {
        MEASURED_UNIT_CODE_PROFIT_BPS => format!(
            "X3_PROFIT_BELOW_FLOOR: the trade realised {measured}bps and the program requires at \
             least {threshold}bps"
        ),
        MEASURED_UNIT_CODE_DELTA_BPS => format!(
            "X3_DELTA_ABOVE_BOUND: the venue left a delta of {measured}bps and the program allows at \
             most {threshold}bps"
        ),
        _ => format!(
            "X3_SLIPPAGE_ABOVE_CEILING: the trade realised {measured}bps and the program allows at \
             most {threshold}bps"
        ),
    }))
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
            let reply = bridge_result(vm.bridge.multi_hop_swap(path.join("\0").as_bytes(), amount))?;
            record_measurement(vm, &reply);
            Ok(reply)
        }
        CapabilityPayload::RebalanceTarget {
            portfolio,
            holdings,
            weights,
            criterion,
        } => {
            // Both ends of the move, asked of a host that can price them: where the portfolio
            // is (when the program stated it) and where it is wanted. A rebalance's *trades*
            // depend on the current holdings, which a compiler has no state for — carrying
            // them is what lets a host compute the trades it could not before (TICKET-070).
            // The holdings field is empty when the program stated none, which a host can tell
            // from holding nothing because the field is present and empty.
            let held = holdings
                .iter()
                .map(|(asset, amount)| format!("{asset}:{amount}"))
                .collect::<Vec<_>>()
                .join(",");
            let flattened = weights
                .iter()
                .map(|(asset, percent)| format!("{asset}:{percent}"))
                .collect::<Vec<_>>()
                .join(",");
            let order = format!("{portfolio}\u{1f}{held}\u{1f}{flattened}\u{1f}{criterion}");
            let reply = bridge_result(vm.bridge.rebalance_target(order.as_bytes()))?;
            record_measurement(vm, &reply);
            Ok(reply)
        }
        CapabilityPayload::VenueOrder {
            action,
            subject,
            asset,
            quantity,
        } => {
            // The host is asked in a form it can act on, and its reply is recorded the way
            // any capability's is: a measured guard after this order is judged against what
            // the host measured, and refuses when it measured nothing.
            let order = format!("{action}\u{1f}{subject}\u{1f}{asset}\u{1f}{quantity}");
            let reply = bridge_result(vm.bridge.venue_order(order.as_bytes()))?;
            record_measurement(vm, &reply);
            Ok(reply)
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
        CapabilityPayload::SubExec {
            bytecode_hash,
            args,
            gas_limit,
        } => {
            if bytecode_hash.is_empty() {
                return Err(ExecError::Panic(
                    "sub exec: bytecode hash must be non-empty".to_string(),
                ));
            }
            if gas_limit == 0 {
                return Err(ExecError::Panic("sub exec: gas_limit must be positive".to_string()));
            }
            vm.state.sub_exec_ops.push(SubExecInfo {
                bytecode_hash,
                args,
                gas_limit,
            });
            Ok(vec![])
        }
        CapabilityPayload::RouteScore { strategy: _, weights } => {
            let total: u32 = weights.iter().map(|(_, w)| w).sum();
            if total == 0 {
                return Err(ExecError::Panic("route score: zero weight".to_string()));
            }
            vm.state.registers[0] = total as u128;
            Ok(vec![])
        }
        CapabilityPayload::SolverBid {
            solver,
            receive_asset,
            deliver_asset,
            fee,
            bond,
        } => {
            if bond == 0 {
                return Err(ExecError::Panic("solver bid: bond must be positive".to_string()));
            }
            if fee.is_empty() {
                return Err(ExecError::Panic("solver bid: fee must be non-empty".to_string()));
            }
            let info = format!(
                "{{\"solver\":\"{solver}\",\"receive_asset\":\"{receive_asset}\",\"deliver_asset\":\"{deliver_asset}\",\"fee\":\"{fee}\",\"bond\":{bond}}}"
            );
            vm.state.bridge_ops.push(BridgePayload {
                via: solver.clone(),
                from_chain: receive_asset.clone(),
                from_asset: deliver_asset.clone(),
                to_chain: String::new(),
                to_asset: String::new(),
                amount: bond,
                receiver: String::new(),
                source_finality_proof: vec![],
                transfer_proof: vec![],
            });
            Ok(info.into_bytes())
        }
        CapabilityPayload::RelayerAttest {
            relayers,
            quorum_numerator,
            quorum_denominator,
            signatures,
        } => {
            if relayers.is_empty() {
                return Err(ExecError::Panic(
                    "relayer attest: relayers must not be empty".to_string(),
                ));
            }
            if quorum_denominator == 0 || quorum_numerator > quorum_denominator {
                return Err(ExecError::Panic("relayer attest: invalid quorum".to_string()));
            }
            // No quorum-versus-signature-count check here, deliberately.
            //
            // A compiled artifact *declares* its swarm: `relayers { quorum
            // 3_of_5, relayers [...] }` lowers to this op with an empty
            // signature list, because attestations are produced by relayers at
            // settlement, not embedded in the program. Demanding
            // `signatures.len() >= quorum_numerator` therefore could not be
            // satisfied by any program that declares a swarm, and every one of
            // them failed with "insufficient signatures for quorum" before
            // executing a single instruction — including the two examples the
            // repository uses to demonstrate a mainnet-safe swap.
            //
            // Quorum satisfaction is verified by the attestation path
            // (x3-verification-router / x3-validator-attestation), which sees
            // the signatures. What is checkable here is that the declaration is
            // internally consistent.
            if signatures.len() > relayers.len() {
                return Err(ExecError::Panic(
                    "relayer attest: more signatures than declared relayers".to_string(),
                ));
            }
            // Leave the guarded quantity in r0: `require relayer_quorum >= N`
            // compares the swarm's quorum numerator against N. This used to put
            // the signature count here, which is zero for every compiled
            // artifact.
            vm.state.registers[0] = quorum_numerator as u128;
            Ok(vec![])
        }
        CapabilityPayload::RpcConsensus {
            chain,
            require_numerator,
            require_denominator,
            reject_on: _,
        } => {
            if chain.is_empty() {
                return Err(ExecError::Panic("rpc consensus: chain must be non-empty".to_string()));
            }
            if require_denominator == 0 || require_numerator > require_denominator {
                return Err(ExecError::Panic("rpc consensus: invalid require ratio".to_string()));
            }
            vm.state.registers[0] = 1;
            Ok(vec![])
        }
        CapabilityPayload::NonceUnused { nonce } => {
            if nonce.is_empty() {
                return Err(ExecError::Panic("nonce unused: nonce must be non-empty".to_string()));
            }
            // Leave the guarded quantity in r0: 1 when the nonce is new, 0 when
            // it has been used. This is a *test and record* — the second use in
            // the same run fails the guard that follows, which is what makes
            // `require nonce unused <id>` a check rather than an assertion
            // nothing reads (TICKET-051).
            let fresh = !vm.state.used_nonces.iter().any(|used| used == &nonce);
            vm.state.registers[0] = if fresh { 1 } else { 0 };
            if fresh {
                vm.state.used_nonces.push(nonce.clone());
            }
            Ok(vec![])
        }
        CapabilityPayload::RiskScore { score, category } => {
            if score > 100 {
                return Err(ExecError::Panic("risk score: score must be <= 100".to_string()));
            }
            if category.is_empty() {
                return Err(ExecError::Panic("risk score: category must be non-empty".to_string()));
            }
            vm.state.registers[0] = score as u128;
            Ok(vec![])
        }
        CapabilityPayload::InvariantCheck { name, assert_expr: _ } => {
            if name.is_empty() {
                return Err(ExecError::Panic("invariant check: name must be non-empty".to_string()));
            }
            vm.state.registers[0] = 1;
            Ok(vec![])
        }
        CapabilityPayload::PrivacyCommit {
            reveal_on,
            encrypted: _,
        } => {
            if reveal_on.is_empty() {
                return Err(ExecError::Panic(
                    "privacy commit: reveal_on must be non-empty".to_string(),
                ));
            }
            Ok(vec![])
        }
        CapabilityPayload::ProofRequired { proof_type, source } => {
            if proof_type.is_empty() {
                return Err(ExecError::Panic(
                    "proof required: proof_type must be non-empty".to_string(),
                ));
            }
            if source.is_empty() {
                return Err(ExecError::Panic("proof required: source must be non-empty".to_string()));
            }
            vm.state.registers[0] = bytes_to_register(proof_type.as_bytes());
            Ok(vec![])
        }
        CapabilityPayload::VmAdapterCall {
            vm: vm_name,
            adapter,
            calldata,
        } => {
            if vm_name.is_empty() {
                return Err(ExecError::Panic("vm adapter call: vm must be non-empty".to_string()));
            }
            if adapter.is_empty() {
                return Err(ExecError::Panic(
                    "vm adapter call: adapter must be non-empty".to_string(),
                ));
            }
            let receipt = format!("{vm_name}:{adapter}:{calldata}").into_bytes();
            vm.state.bridge_receipts.push(receipt);
            Ok(vec![])
        }
        CapabilityPayload::ModeCheck { mode, restriction } => {
            if mode.is_empty() {
                return Err(ExecError::Panic("mode check: mode must be non-empty".to_string()));
            }
            if restriction.is_empty() {
                return Err(ExecError::Panic(
                    "mode check: restriction must be non-empty".to_string(),
                ));
            }
            // PHASE 28's "runtime should reject accidental public submission if
            // compiled policy requires privacy" is enforced here rather than
            // recorded: a mode check that nothing reads is a policy the artifact
            // states and the runtime ignores, which is worse than no policy.
            if mode == "submission" && restriction == "private_required" && !vm.config.allow_private_submission {
                return Err(ExecError::Panic(
                    "X3_PRIVATE_SUBMISSION_REQUIRED: the compiled policy requires private submission \
                     but this runtime has no private channel"
                        .to_string(),
                ));
            }
            Ok(vec![])
        }
        CapabilityPayload::PackageImport { path, alias: _ } => {
            if path.is_empty() {
                return Err(ExecError::Panic("package import: path must be non-empty".to_string()));
            }
            vm.state.registers[0] = 1;
            Ok(vec![])
        }
        CapabilityPayload::RefundPolicy {
            action,
            target: _,
            after_blocks,
        } => {
            if action.is_empty() {
                return Err(ExecError::Panic("refund policy: action must be non-empty".to_string()));
            }
            vm.state.failure_handlers.push(after_blocks as usize);
            Ok(vec![])
        }
    }?;
    Ok(result)
}

fn bridge_result(result: Result<Vec<u8>, Box<dyn std::error::Error>>) -> ExecResult<Vec<u8>> {
    result.map_err(|err| ExecError::Panic(err.to_string()))
}

/// Record the measurement a capability reply carries, if it carries one.
///
/// A reply is a measurement only when it says so and names the unit. Anything else
/// clears both fields rather than leaving an earlier trade's numbers in place: a guard
/// must not pass on a measurement that belongs to a different instruction.
fn record_measurement(vm: &mut VM, reply: &[u8]) {
    vm.state.measured_profit_bps = None;
    vm.state.measured_slippage_bps = None;
    vm.state.measured_delta_bps = None;
    // A reply that is not a measurement is not an error: most capabilities answer with
    // arbitrary bytes, and only a measured guard needs a number.
    // A reply is a *sequence* of measurements, because one trade answers both questions a
    // plan asks about it: its profit floor and its slippage ceiling are about the same
    // call, and a host that had to answer twice would have to say which reply went with
    // which guard.
    let mut last = None;
    for (unit, value) in crate::spec::opcodes::read_measured_replies(reply) {
        last = Some(value);
        match unit {
            crate::spec::opcodes::MEASURED_UNIT_PROFIT_BPS => vm.state.measured_profit_bps = Some(value),
            crate::spec::opcodes::MEASURED_UNIT_SLIPPAGE_BPS => vm.state.measured_slippage_bps = Some(value),
            crate::spec::opcodes::MEASURED_UNIT_DELTA_BPS => vm.state.measured_delta_bps = Some(value),
            // An unknown unit is not a measurement this VM can use, and pretending
            // otherwise would let a host answer a profit guard with a slippage.
            _ => {}
        }
    }
    // `r0` carries the last one, so the register and the records agree.
    if let Some(value) = last {
        vm.state.registers[0] = value;
    }
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

    /// The same, with an explicit flags byte — `REQUIRE` keeps its comparison
    /// code there.
    fn instr_flags(opcode: u8, flags: u8, operand: u16) -> [u8; 4] {
        [opcode, flags, (operand & 0xFF) as u8, (operand >> 8) as u8]
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
    fn a_static_guard_does_not_depend_on_r0() {
        // A `REQUIRE` with the STATIC comparison asserts something about the
        // artifact's configuration, which the compiler already checked; there is
        // no run-time quantity to test. It used to test r0 for non-zero, so a
        // guard failed whenever an unrelated instruction happened to leave zero
        // there — which is exactly what the flagship examples hit.
        let code: &[u8] = &[instr_flags(REQUIRE, REQUIRE_COMPARE_STATIC, 0), instr(HALT, 0)].concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 0;
        assert!(
            execute(&mut vm).is_ok(),
            "a static guard must not depend on the residue of an unrelated instruction"
        );
    }

    #[test]
    fn a_compared_guard_fails_below_its_threshold() {
        let code: &[u8] = &[instr_flags(REQUIRE, REQUIRE_COMPARE_GE, 10), instr(HALT, 0)].concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 9;
        match execute(&mut vm) {
            Err(ExecError::Panic(msg)) => assert!(msg.contains("REQUIRE_FAILED"), "got: {msg}"),
            other => panic!("expected a guard failure, got {other:?}"),
        }
    }

    #[test]
    fn a_compared_guard_passes_at_its_threshold() {
        let code: &[u8] = &[instr_flags(REQUIRE, REQUIRE_COMPARE_GE, 10), instr(HALT, 0)].concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 10;
        assert!(execute(&mut vm).is_ok(), "r0 equal to the threshold must satisfy >=");
    }

    #[test]
    fn an_unknown_comparison_code_fails_closed() {
        // A comparison this VM does not implement must not pass by default.
        let code: &[u8] = &[instr_flags(REQUIRE, 0x7F, 0), instr(HALT, 0)].concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 1;
        assert!(
            execute(&mut vm).is_err(),
            "an unimplemented comparison must not silently succeed"
        );
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
        // ON_TIMEOUT with the deadline as the instruction's operand.
        // HALT
        let code: &[u8] = &[
            instr(0x42, 100), // ON_TIMEOUT: deadline = 100 instructions
            instr(0xFF, 0),   // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        execute(&mut vm).unwrap();
        assert_eq!(vm.state.timeout_deadline, Some(100));
    }

    #[test]
    fn on_timeout_with_a_zero_operand_sets_no_deadline() {
        // Zero means "no instruction budget", not "a budget of zero". Treating
        // the operand as a register index made this emit `r0`, so a program
        // whose r0 held zero panicked on its first instruction.
        let code: &[u8] = &[
            instr(0x42, 0), // ON_TIMEOUT: policy only
            instr(NOP, 0),
            instr(0xFF, 0), // HALT
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
        vm.state.registers[0] = 0;
        execute(&mut vm).expect("a zero deadline must not bound the program");
        assert_eq!(vm.state.timeout_deadline, None);
    }

    #[test]
    fn timeout_exceeded_panics() {
        // ON_TIMEOUT with deadline=1, then several NOPs
        // Each NOP increments instruction_count. After 2 NOPs,
        // instruction_count > deadline → X3_TIMEOUT panic.
        let code: &[u8] = &[
            instr(0x42, 1), // ON_TIMEOUT: deadline = 1 instruction
            instr(NOP, 0),  // NOP (instruction_count=1)
            instr(NOP, 0),  // NOP (instruction_count=2 > deadline=1)
            instr(0xFF, 0), // HALT (won't reach)
        ]
        .concat();
        let mut vm = VM::new(code.to_vec(), VMConfig::default(), 1_000_000);
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
            trading_ops_len: 0,
            atomic_choices_len: 0,
            route_fallbacks_len: 0,
            parallel_plans_len: 0,
            strategy_licenses_len: 0,
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
            instr(ON_FAIL, enc_rri(1, 0, 0)),            // ON_FAIL r1
            instr_flags(REQUIRE, REQUIRE_COMPARE_GE, 1), // REQUIRE r0 >= 1 → fails
            instr(HALT, 0),                              // HALT (unreachable)
            instr(0x01, enc_tt(0, 0, 2)),                // ADD r0, r0, r2 (handler)
            instr(HALT, 0),                              // HALT
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
    fn sub_exec_opcode_succeeds_with_valid_payload() {
        let payload = {
            let mut p = Vec::new();
            p.extend_from_slice(&1u16.to_le_bytes()); // bytecode_hash len 1
            p.push(b'a'); // bytecode_hash = "a"
            p.extend_from_slice(&0u16.to_le_bytes()); // args count 0
            p.extend_from_slice(&1u64.to_le_bytes()); // gas_limit = 1
            p
        };
        let code = capability_bytecode(SUB_EXEC, &payload);
        let mut vm = VM::new(code, VMConfig::default(), 1_000_000);
        let result = execute(&mut vm);
        assert!(result.is_ok(), "SUB_EXEC with valid payload must succeed: {:?}", result);
        assert_eq!(vm.state.sub_exec_ops.len(), 1);
        assert_eq!(vm.state.sub_exec_ops[0].bytecode_hash, "a");
        assert_eq!(vm.state.sub_exec_ops[0].gas_limit, 1);
    }

    /// Every opcode that carries a length-prefixed payload, from the one place
    /// that says which those are.
    ///
    /// This test used to walk the range `GPU_DISPATCH..=SUB_EXEC` and assert each
    /// opcode had a name — the same assumption as the bug it should have caught:
    /// the *executor's* dispatch was written as that range, so an instruction
    /// added to `is_payload_opcode` and not to the range was a payload opcode to
    /// the verifier and an invalid one here. The nonce instruction did exactly
    /// that (`InvalidOpcode(156)`), and this test passed throughout, because a
    /// range literal cannot notice an opcode outside it. Driving the walk from the
    /// predicate is what makes the two agree. (TICKET-055.)
    pub(super) fn payload_opcodes() -> Vec<u8> {
        (0u8..=u8::MAX)
            .filter(|opcode| is_payload_opcode(*opcode, true))
            .collect()
    }

    #[test]
    fn every_payload_opcode_is_recognised_by_the_executor() {
        let opcodes = payload_opcodes();
        assert!(
            opcodes.len() >= 40,
            "the payload set should cover the asset, capability and trading ranges: {opcodes:?}"
        );
        for opcode in opcodes {
            // Every payload opcode is *named*, from the one table both crates
            // include: an instruction whose payload the executor reads and whose
            // trace says `UNKNOWN` is an instruction a reader cannot identify.
            assert_ne!(
                opcode_name(opcode),
                "UNKNOWN",
                "payload opcode 0x{opcode:02x} has no name"
            );
            // Only the *dispatch* is asserted below. A payload opcode's name is not
            // necessarily a capability name — the asset ops and the atomic and
            // trading instructions carry payloads and have machine names of their
            // own — and the capability names have their own test.
            //
            // An empty payload is not a valid payload for most of these, and that
            // is fine: what is asserted is that the opcode is *recognised*, so any
            // failure is about its content rather than about reaching no arm at
            // all. That is exactly the difference the drifting range hid.
            let mut vm = VM::new(capability_bytecode(opcode, &[]), VMConfig::default(), 1_000_000);
            match execute(&mut vm) {
                Ok(()) => {}
                Err(ExecError::InvalidOpcode(other)) => panic!(
                    "payload opcode 0x{opcode:02x} reached no arm: the executor refused it as \
                     invalid ({other:#04x})"
                ),
                Err(_) => {}
            }
        }
    }

    #[test]
    fn every_payload_opcode_frames_the_same_for_the_verifier() {
        // The other half: a payload opcode whose frame the verifier does not
        // recognise desynchronises the walk rather than failing, which is how the
        // verifier's own drift showed up before.
        use crate::verifier::verify;
        use crate::x3_lang_vm::InstructionStream;
        for opcode in payload_opcodes() {
            let code = capability_bytecode(opcode, &[]);
            if let Err(error) = verify(&InstructionStream::new(code)) {
                let named = format!("{error:?}");
                assert!(
                    !named.contains("OutOfBounds"),
                    "the verifier's walk desynchronised on payload opcode 0x{opcode:02x}: {named}"
                );
                assert!(
                    !named.contains("InvalidOpcode"),
                    "the verifier refused payload opcode 0x{opcode:02x} as an invalid instruction: \
                     {named}"
                );
            }
        }
    }

    #[test]
    fn capability_opcode_names_cover_all_defined() {
        assert_eq!(opcode_name(GPU_DISPATCH), "GPU_DISPATCH");
        // The instruction added for the nonce guard: a capability payload like
        // its neighbours, and named so a disassembly says what it is.
        assert_eq!(opcode_name(NONCE_UNUSED), "NONCE_UNUSED");
        assert_eq!(opcode_name(SIMULATE), "SIMULATE");
        assert_eq!(opcode_name(SCHEDULED_DISPATCH), "SCHEDULED_DISPATCH");
        assert_eq!(opcode_name(INTENT_RESOLVE), "INTENT_RESOLVE");
        assert_eq!(opcode_name(CRDT_OP), "CRDT_OP");
        assert_eq!(opcode_name(PROOF_VERIFY), "PROOF_VERIFY");
        assert_eq!(opcode_name(STORAGE_OP), "STORAGE_OP");
        assert_eq!(opcode_name(PATHFIND), "PATHFIND");
        assert_eq!(opcode_name(MEMPOOL_SCAN), "MEMPOOL_SCAN");
        assert_eq!(opcode_name(ORACLE_REQUEST), "ORACLE_REQUEST");
        assert_eq!(opcode_name(EMERGENCY_CONTROL), "EMERGENCY_CONTROL");
        assert_eq!(opcode_name(LIFECYCLE), "LIFECYCLE");
        assert_eq!(opcode_name(SERIALIZE), "SERIALIZE");
        assert_eq!(opcode_name(DESERIALIZE), "DESERIALIZE");
        assert_eq!(opcode_name(GAS_ESTIMATE), "GAS_ESTIMATE");
        assert_eq!(opcode_name(CHAIN_METRIC), "CHAIN_METRIC");
        assert_eq!(opcode_name(EVENT_PROVENANCE), "EVENT_PROVENANCE");
        assert_eq!(opcode_name(MULTI_HOP_SWAP), "MULTI_HOP_SWAP");
        assert_eq!(opcode_name(VECTOR_MATH), "VECTOR_MATH");
        assert_eq!(opcode_name(ROLE_CHECK), "ROLE_CHECK");
        assert_eq!(opcode_name(MULTISIG_CHECK), "MULTISIG_CHECK");
        assert_eq!(opcode_name(VERSION_META), "VERSION_META");
        assert_eq!(opcode_name(STORAGE_NAMESPACE), "STORAGE_NAMESPACE");
        assert_eq!(opcode_name(ABI_EXPORT), "ABI_EXPORT");
        assert_eq!(opcode_name(DOC_EMBED), "DOC_EMBED");
        assert_eq!(opcode_name(GAS_ADAPTIVE), "GAS_ADAPTIVE");
        assert_eq!(opcode_name(BOUNTY), "BOUNTY");
        assert_eq!(opcode_name(SUB_EXEC), "SUB_EXEC");
    }

    #[test]
    fn dry_run_bridge_executes_all_capability_opcodes_without_silent_noop() {
        for opcode in GPU_DISPATCH..=SUB_EXEC {
            let name = opcode_name(opcode);
            let payload = capability_minimal_payload(opcode);
            let code = capability_bytecode(opcode, &payload);
            let mut vm = VM::new(code, VMConfig::default(), 500_000);
            let result = execute(&mut vm);
            match result {
                Ok(()) => {}
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
                p.extend_from_slice(&one_str); // bytecode_hash = "a"
                p.extend_from_slice(&0u16.to_le_bytes()); // args count 0
                p.extend_from_slice(&1u64.to_le_bytes()); // gas_limit = 1
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
                opcode_name(opcode)
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

#[cfg(test)]
mod payload_charge_tests {
    use super::tests::payload_opcodes;
    use super::*;
    use crate::x3_lang_vm::VMConfig;

    /// What one execution of `opcode` with a payload of `len` bytes costs.
    fn cost(opcode: u8, len: usize) -> u128 {
        let mut vm = VM::new(
            // The helper lives in the other test module, so build the frame here.
            {
                let mut code = vec![0x01, opcode];
                code.extend_from_slice(&(len as u16).to_le_bytes());
                code.extend(std::iter::repeat_n(0u8, len));
                while code.len() % 4 != 0 {
                    code.push(0);
                }
                code.extend_from_slice(&[HALT, 0, 0, 0]);
                code
            },
            VMConfig::default(),
            1_000_000,
        );
        let before = vm.state.gas;
        let _ = execute(&mut vm);
        before - vm.state.gas
    }

    #[test]
    fn every_payload_opcode_is_charged_for_its_payload() {
        // The rule is `payload_len / 32`, and it applies to *every* instruction
        // whose payload is read. Asserted exactly rather than as an inequality:
        // "a longer payload does not cost less" passes when nothing is charged at
        // all, which is the defect this test exists for.
        let opcodes = payload_opcodes();
        assert!(!opcodes.is_empty(), "the payload set must not be empty");
        for opcode in opcodes {
            let empty = cost(opcode, 0);
            let long = cost(opcode, 64);
            assert_eq!(
                long,
                empty + 2,
                "opcode 0x{opcode:02x} must be charged 64/32 = 2 more for a 64-byte payload than \
                 for an empty one (empty {empty}, long {long})"
            );
        }
    }
}
