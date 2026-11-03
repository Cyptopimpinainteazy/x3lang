//! Bytecode Format Helpers
//!
//! Test helpers for assembling minimal X3BC modules for unit tests.
//! These helpers produce bytes that can be parsed by `BytecodeModule::from_bytes()`
//! and executed by the VM.
//!
//! # Binary Format Layout
//!
//! ```text
//! [MAGIC: "X3BC" (4 bytes)]
//! [VERSION: u32]
//! [FLAGS: u32]
//! [CHECKSUM: u32]
//! [MIN_VERSION: u32]
//! [FEATURES: u32]
//! [CONST_POOL: count(u32) + entries...]
//! [FUNCTIONS: count(u32) + FunctionEntry*]
//! [GLOBALS: count(u32) + GlobalEntry*]
//! [CODE: len(u32) + bytes...]
//! [DEBUG_INFO: marker(u8) + optional data]
//! [METADATA: marker(u8) + optional data]
//! ```

#![allow(clippy::vec_init_then_push)] // The assemblers spell instructions out byte by byte, on purpose.

use crate::format::{ConstValue, FunctionEntry, MAGIC, VERSION};
use crate::opcode::Opcode;
use crate::prelude::*;

/// Assemble a simple module that adds two constants and returns the result.
///
/// The module contains:
/// - Constant pool: [42_i64, 7_i64]
/// - One function "add_test" at entry_point 0
/// - Code:
///   ```text
///   LoadConst r0, #0    ; r0 = 42
///   LoadConst r1, #1    ; r1 = 7
///   AddI      r2, r0, r1 ; r2 = 49
///   Ret       r2         ; return 49
///   ```
///
/// The envelope's checksum, over the body — everything after the 24-byte header.
///
/// This file used to carry its own CRC32 implementation while the writer used a wrapping
/// multiply-and-add, so fixtures built here were not bytecode the reader would accept. The
/// definition lives in `x3-common::bytecode` and is shared with the writer, the std reader and the
/// no-std reader (TICKET-108).
fn compute_bytecode_checksum(body: &[u8]) -> u32 {
    crate::bytecode::checksum(body)
}

/// Write the 24-byte X3BC header (magic + version + flags + checksum_placeholder + min_version + features).
fn write_header(out: &mut Vec<u8>) {
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // flags
    out.extend_from_slice(&0u32.to_le_bytes()); // checksum placeholder
    out.extend_from_slice(&VERSION.to_le_bytes()); // min_version
    out.extend_from_slice(&0u32.to_le_bytes()); // features
}

/// Patch the checksum field (bytes 12-15) with the body's checksum.
fn patch_checksum(module: &mut [u8]) {
    if module.len() < 24 {
        return;
    }
    let body = &module[24..];
    let checksum = compute_bytecode_checksum(body);
    module[12..16].copy_from_slice(&checksum.to_le_bytes());
}

/// # Returns
///
/// `Vec<u8>` - Valid X3BC module bytes ready for `BytecodeModule::from_bytes()`.
pub fn assemble_simple_module() -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();

    // === Header (24 bytes) ===
    write_header(&mut out);

    // === Constant Pool ===
    // Two integer constants: 42 and 7
    // Format: count(u32) + [tag(u8) + payload...]
    // Tags: 0=Integer(i64), 1=Float(f64), 2=String, 3=Bool, 4=Bytes
    let consts = vec![ConstValue::Integer(42), ConstValue::Integer(7)];
    out.extend_from_slice(&(consts.len() as u32).to_le_bytes());
    for c in &consts {
        match c {
            ConstValue::Integer(i) => {
                out.push(0u8); // tag = 0 for Integer
                out.extend_from_slice(&i.to_le_bytes());
            }
            ConstValue::Float(f) => {
                out.push(1u8);
                out.extend_from_slice(&f.to_le_bytes());
            }
            ConstValue::String(s) => {
                out.push(2u8);
                let bytes = s.as_bytes();
                out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                out.extend_from_slice(bytes);
            }
            ConstValue::Bool(b) => {
                out.push(3u8);
                out.push(if *b { 1 } else { 0 });
            }
            ConstValue::Bytes(b) => {
                out.push(4u8);
                out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                out.extend_from_slice(b);
            }
        }
    }

    // === Build code section first (need size for function entry) ===
    let mut code: Vec<u8> = Vec::new();

    // Instruction format from emit.rs:
    // LoadConst: opcode(u8) + dst(u8) + idx(u32) = 6 bytes
    // AddI: opcode(u8) + dst(u8) + a(u8) + b(u8) = 4 bytes
    // Ret: opcode(u8) + src(u8) = 2 bytes

    // LoadConst r0, #0 (load const[0] = 42 into r0)
    code.push(Opcode::LoadConst as u8); // 0x10
    code.push(0u8); // dst = r0
    code.extend_from_slice(&0u32.to_le_bytes()); // const idx 0

    // LoadConst r1, #1 (load const[1] = 7 into r1)
    code.push(Opcode::LoadConst as u8); // 0x10
    code.push(1u8); // dst = r1
    code.extend_from_slice(&1u32.to_le_bytes()); // const idx 1

    // AddI r2, r0, r1 (r2 = r0 + r1 = 49)
    code.push(Opcode::AddI as u8); // 0x20
    code.push(2u8); // dst = r2
    code.push(0u8); // a = r0
    code.push(1u8); // b = r1

    // Ret r2 (return r2)
    code.push(Opcode::Ret as u8); // 0x05
    code.push(2u8); // src = r2

    // === Functions Table ===
    // Format: count(u32) + [name_len(u16) + name + entry_point(u32) +
    //         param_count(u8) + local_count(u16) + max_stack(u16) + return_type_tag(u8)]
    let func = FunctionEntry {
        name: "add_test".to_string(),
        entry_point: 0, // starts at beginning of code section
        param_count: 0,
        local_count: 3, // r0, r1, r2
        max_stack: 4,
        return_type_tag: 1, // 1 = int
    };

    out.extend_from_slice(&1u32.to_le_bytes()); // function count
    out.extend_from_slice(&(func.name.len() as u16).to_le_bytes());
    out.extend_from_slice(func.name.as_bytes());
    out.extend_from_slice(&func.entry_point.to_le_bytes());
    out.push(func.param_count);
    out.extend_from_slice(&func.local_count.to_le_bytes());
    out.extend_from_slice(&func.max_stack.to_le_bytes());
    out.push(func.return_type_tag);

    // === Globals Table (empty) ===
    out.extend_from_slice(&0u32.to_le_bytes());

    // === Code Section ===
    out.extend_from_slice(&(code.len() as u32).to_le_bytes());
    out.extend_from_slice(&code);

    // Debug info: not present (marker byte 0)
    out.push(0u8);
    // Metadata: not present (marker byte 0)
    out.push(0u8);

    // Patch the CRC32 checksum over the body (bytes 24..end).
    patch_checksum(&mut out);

    out
}

/// Assemble a module with parameters: adds two i64 parameters and returns sum.
///
/// Function signature: `fn add_params(a: i64, b: i64) -> i64`
/// Code:
/// ```text
/// AddI r2, r0, r1  ; r2 = param0 + param1
/// Ret  r2          ; return r2
/// ```
pub fn assemble_param_module() -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();

    // Header (24 bytes)
    write_header(&mut out);

    // Empty const pool
    out.extend_from_slice(&0u32.to_le_bytes());

    // Build code
    let mut code: Vec<u8> = Vec::new();
    // AddI r2, r0, r1 (params in r0, r1)
    code.push(Opcode::AddI as u8);
    code.push(2u8);
    code.push(0u8);
    code.push(1u8);
    // Ret r2
    code.push(Opcode::Ret as u8);
    code.push(2u8);

    // Function table
    let func_name = "add_params";
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(func_name.len() as u16).to_le_bytes());
    out.extend_from_slice(func_name.as_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // entry_point
    out.push(2u8); // param_count
    out.extend_from_slice(&3u16.to_le_bytes()); // local_count
    out.extend_from_slice(&4u16.to_le_bytes()); // max_stack
    out.push(1u8); // return_type_tag = int

    // Empty globals
    out.extend_from_slice(&0u32.to_le_bytes());

    // Code section
    out.extend_from_slice(&(code.len() as u32).to_le_bytes());
    out.extend_from_slice(&code);

    // Debug info: not present
    out.push(0u8);
    // Metadata: not present
    out.push(0u8);

    // Patch the CRC32 checksum over the body.
    patch_checksum(&mut out);

    out
}

/// Assemble a module that uses a conditional jump.
///
/// Function: returns a if a > 0, else returns 0
/// Code:
/// ```text
/// LoadImm  r1, 0       ; r1 = 0
/// CmpGtI   r2, r0, r1  ; r2 = (r0 > 0)
/// JumpIf   r2, #8      ; if true, jump to Ret r0
/// Ret      r1          ; return 0
/// Ret      r0          ; return a
/// ```
pub fn assemble_branch_module() -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();

    // Header (24 bytes)
    write_header(&mut out);

    // Empty const pool
    out.extend_from_slice(&0u32.to_le_bytes());

    // Build code
    let mut code: Vec<u8> = Vec::new();

    // LoadImm r1, 0 (2 bytes)
    code.push(Opcode::LoadImm as u8); // 0x18
    code.push(1u8); // dst = r1
    code.push(0u8); // val = 0 (i8)

    // GtI r2, r0, r1 (4 bytes) - r2 = (r0 > r1)
    code.push(Opcode::GtI as u8); // 0x44
    code.push(2u8); // dst
    code.push(0u8); // a
    code.push(1u8); // b

    // JumpIf r2, target (6 bytes) - jump to offset 15 (Ret r0)
    code.push(Opcode::JumpIf as u8); // 0x02
    code.push(2u8); // cond
                    // Offsets: LoadImm=3, GtI=4, JumpIf=6, Ret=2, Ret=2
                    // 0-2: LoadImm, 3-6: GtI, 7-12: JumpIf, 13-14: Ret r1, 15-16: Ret r0
    let target_offset = 15u32; // offset of "Ret r0"
    code.extend_from_slice(&target_offset.to_le_bytes());

    // Ret r1 (2 bytes) - at offset 11
    code.push(Opcode::Ret as u8);
    code.push(1u8);

    // Ret r0 (2 bytes) - at offset 13
    code.push(Opcode::Ret as u8);
    code.push(0u8);

    // Function table
    let func_name = "abs_or_zero";
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(func_name.len() as u16).to_le_bytes());
    out.extend_from_slice(func_name.as_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // entry_point
    out.push(1u8); // param_count
    out.extend_from_slice(&3u16.to_le_bytes()); // local_count
    out.extend_from_slice(&4u16.to_le_bytes()); // max_stack
    out.push(1u8); // return_type_tag = int

    // Empty globals
    out.extend_from_slice(&0u32.to_le_bytes());

    // Code section
    out.extend_from_slice(&(code.len() as u32).to_le_bytes());
    out.extend_from_slice(&code);

    // Debug info: not present
    out.push(0u8);
    // Metadata: not present
    out.push(0u8);

    // Patch the CRC32 checksum over the body.
    patch_checksum(&mut out);

    out
}

/// Assemble a minimal module with just Halt.
pub fn assemble_halt_module() -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();

    // Header (24 bytes)
    write_header(&mut out);

    // Empty const pool
    out.extend_from_slice(&0u32.to_le_bytes());

    // Code: just Halt
    let code = vec![Opcode::Halt as u8];

    // Function table
    let func_name = "main";
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&(func_name.len() as u16).to_le_bytes());
    out.extend_from_slice(func_name.as_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // entry_point
    out.push(0u8); // param_count
    out.extend_from_slice(&0u16.to_le_bytes()); // local_count
    out.extend_from_slice(&1u16.to_le_bytes()); // max_stack
    out.push(0u8); // return_type_tag = void

    // Empty globals
    out.extend_from_slice(&0u32.to_le_bytes());

    // Code section
    out.extend_from_slice(&(code.len() as u32).to_le_bytes());
    out.extend_from_slice(&code);

    // Debug info: not present
    out.push(0u8);
    // Metadata: not present
    out.push(0u8);

    // Patch the CRC32 checksum over the body.
    patch_checksum(&mut out);

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::BytecodeModule;

    #[test]
    fn simple_module_parses() {
        let bytes = assemble_simple_module();
        let module = BytecodeModule::from_bytes(&bytes).expect("should parse");

        assert_eq!(module.const_pool.entries.len(), 2);
        assert_eq!(module.const_pool.entries[0], ConstValue::Integer(42));
        assert_eq!(module.const_pool.entries[1], ConstValue::Integer(7));
        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, "add_test");
        assert_eq!(module.functions[0].entry_point, 0);
        // Code: 6 + 6 + 4 + 2 = 18 bytes
        assert_eq!(module.code.len(), 18);
    }

    #[test]
    fn param_module_parses() {
        let bytes = assemble_param_module();
        let module = BytecodeModule::from_bytes(&bytes).expect("should parse");

        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, "add_params");
        assert_eq!(module.functions[0].param_count, 2);
    }

    #[test]
    fn branch_module_parses() {
        let bytes = assemble_branch_module();
        let module = BytecodeModule::from_bytes(&bytes).expect("should parse");

        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, "abs_or_zero");
        assert_eq!(module.functions[0].param_count, 1);
    }

    #[test]
    fn halt_module_parses() {
        let bytes = assemble_halt_module();
        let module = BytecodeModule::from_bytes(&bytes).expect("should parse");

        assert_eq!(module.functions.len(), 1);
        assert_eq!(module.functions[0].name, "main");
        assert_eq!(module.code.len(), 1);
        assert_eq!(module.code[0], Opcode::Halt as u8);
    }
}
