//! Property-based tests for VM determinism and rollback safety.
//!
//! Invariants tested:
//! 1. Determinism — same bytecode + same initial state → same final state
//! 2. Atomic rollback — ATOMIC_ROLLBACK restores state to pre-begin snapshot
//! 3. Gas accounting — gas consumed never exceeds initial allocation

use proptest::prelude::*;
use x3_lang_vm::spec::opcodes::*;
use x3_lang_vm::{VMConfig, VM};

fn instr(opcode: u8, operand: u16) -> Vec<u8> {
    vec![opcode, 0, (operand & 0xFF) as u8, (operand >> 8) as u8]
}

fn enc_tt(ra: u8, rb: u8, rc: u8) -> u16 {
    (ra as u16) | ((rb as u16) << 5) | ((rc as u16) << 10)
}

fn enc_rri_simple(ra: u8) -> u16 {
    ra as u16
}

fn build_vm(code: Vec<u8>, regs: &[u128], gas: u128) -> VM {
    let mut vm = VM::new(code, VMConfig::default(), gas);
    for (i, &v) in regs.iter().enumerate() {
        if i < vm.state.registers.len() {
            vm.state.registers[i] = v;
        }
    }
    vm
}

fn exec_twice(code: Vec<u8>, regs: Vec<u128>, gas: u128) -> (Result<(), x3_lang_vm::executor::ExecError>, VM, VM) {
    let mut vm1 = build_vm(code.clone(), &regs, gas);
    let r1 = vm1.execute();
    let mut vm2 = build_vm(code, &regs, gas);
    let r2 = vm2.execute();
    assert_eq!(r1.is_ok(), r2.is_ok(), "determinism violation: result differs");
    (r1, vm1, vm2)
}

/// ROLLBACK without BEGIN must always fail (deterministic error).
#[test]
fn prop_rollback_without_begin_always_fails() {
    let code = [instr(ATOMIC_ROLLBACK, 0), instr(HALT, 0)].concat();
    let (r1, vm1, vm2) = exec_twice(code, vec![], 10000);
    assert!(r1.is_err(), "ROLLBACK without BEGIN must always fail");
    assert_eq!(vm1.state.pc, vm2.state.pc, "pc after error must match across runs");
}

/// Gas consumed never exceeds initial allocation.
#[test]
fn prop_gas_never_exceeds_initial() {
    let code = [
        instr(0x01, enc_tt(0, 0, 1)),
        instr(0x02, enc_tt(2, 0, 1)),
        instr(HALT, 0),
    ]
    .concat();
    let mut vm = build_vm(code, &[0, 0], u128::MAX);
    let _ = vm.execute();
    // Code deposit: 12 bytes. ADD: 1, SUB: 1. Total: 12 + 1 + 1 = 14.
    assert_eq!(vm.state.gas, u128::MAX - 14);
}

proptest! {

    #[test]
    fn prop_add_determinism(a: u128, b: u128) {
        let code = [
            instr(0x01, enc_tt(0, 0, 1)),
            instr(HALT, 0),
        ].concat();
        let (r1, vm1, vm2) = exec_twice(code, vec![a, b], 1000);
        prop_assert!(r1.is_ok());
        prop_assert_eq!(vm1.state.registers, vm2.state.registers);
        prop_assert_eq!(vm1.state.gas, vm2.state.gas);
        prop_assert_eq!(vm1.state.pc, vm2.state.pc);
    }

    #[test]
    fn prop_sub_determinism(a: u128, b: u128) {
        let code = [
            instr(0x02, enc_tt(0, 0, 1)),
            instr(HALT, 0),
        ].concat();
        let (r1, vm1, vm2) = exec_twice(code, vec![a, b], 1000);
        prop_assert!(r1.is_ok());
        prop_assert_eq!(vm1.state.registers, vm2.state.registers);
        prop_assert_eq!(vm1.state.gas, vm2.state.gas);
    }

    #[test]
    fn prop_pow_determinism(base: u128, exp: u128) {
        let code = [
            instr(0x0A, enc_tt(0, 0, 1)),
            instr(HALT, 0),
        ].concat();
        let (r1, vm1, vm2) = exec_twice(code, vec![base, exp], u128::MAX);
        prop_assert!(r1.is_ok());
        prop_assert_eq!(vm1.state.registers, vm2.state.registers);
        prop_assert_eq!(vm1.state.gas, vm2.state.gas);
    }

    #[test]
    fn prop_chained_rrr_determinism(a: u128, b: u128, c: u128) {
        let code = [
            instr(0x01, enc_tt(0, 0, 1)),
            instr(0x02, enc_tt(0, 0, 2)),
            instr(HALT, 0),
        ].concat();
        let (r1, vm1, vm2) = exec_twice(code, vec![a, b, c], 1000);
        prop_assert!(r1.is_ok());
        prop_assert_eq!(vm1.state.registers, vm2.state.registers);
        prop_assert_eq!(vm1.state.gas, vm2.state.gas);
    }

    #[test]
    fn prop_atomic_rollback_restores_registers(a: u128, b: u128) {
        prop_assume!(a != 0);
        let code = [
            instr(ATOMIC_BEGIN, 0),
            instr(0x01, enc_tt(0, 0, 1)),
            instr(ATOMIC_ROLLBACK, 0),
            instr(REQUIRE, enc_rri_simple(0)),
            instr(HALT, 0),
        ].concat();
        let mut vm = build_vm(code, &[a, b], 10000);
        let result = vm.execute();
        prop_assert!(result.is_ok());
        prop_assert_eq!(vm.state.registers[0], a);
        prop_assert_eq!(vm.state.registers[1], b);
    }

    #[test]
    fn prop_atomic_commit_preserves_changes(a: u128, b: u128) {
        let code = [
            instr(ATOMIC_BEGIN, 0),
            instr(0x01, enc_tt(0, 0, 1)),
            instr(ATOMIC_END, 0),
            instr(HALT, 0),
        ].concat();
        let mut vm = build_vm(code, &[a, b], 10000);
        let result = vm.execute();
        prop_assert!(result.is_ok());
        prop_assert_eq!(vm.state.registers[0], a.wrapping_add(b));
    }

    #[test]
    fn prop_out_of_gas_deterministic(a: u128) {
        let code = [
            instr(0x0A, enc_tt(0, 0, 1)),
            instr(HALT, 0),
        ].concat();
        let (r1, _, _) = exec_twice(code, vec![a, 2], 10);
        prop_assert!(r1.is_err());
    }

}
