//! X3 Instruction Set
//!
//! The X3 virtual machine uses a register-based instruction set (124 opcodes, see `crate::spec`).
//! Instructions are encoded as variable-length byte sequences:
//!
//! ```text
//! [opcode: u8] [operands...]
//! ```
//!
//! Register operands are encoded as one byte (`u8`): the emitter, the verifier and the interpreter
//! all read one byte, whatever the `Register` newtype below can hold (see `crate::spec`).
//! Immediate values use the constant pool index.
//!
//! # Design Principles
//!
//! 1. **Deterministic**: No undefined behavior, identical inputs → identical outputs
//! 2. **Type-preserving**: Operations match HIR types (i32, i64, f32, f64, bool)
//! 3. **Explicit control flow**: No implicit fallthrough
//! 4. **X3-specific**: Atomic blocks, context access, VM intrinsics are first-class
//! 5. **GPU-friendly**: Avoid divergent branches where possible

use crate::prelude::*;
use core::fmt;
use serde::{Deserialize, Serialize};

/// Virtual register index (0-65535).
/// Register 0 is typically reserved for the return value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Register(pub u16);

impl Register {
    /// Return value register.
    pub const RET: Register = Register(0);

    /// First argument register.
    pub const ARG0: Register = Register(1);

    pub fn index(self) -> u16 {
        self.0
    }

    pub fn from_index(idx: u16) -> Self {
        Register(idx)
    }
}

impl fmt::Display for Register {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// Constant pool index for immediate values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConstIdx(pub u32);

impl fmt::Display for ConstIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "c{}", self.0)
    }
}

/// Function index in the module's function table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FuncIdx(pub u32);

impl fmt::Display for FuncIdx {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fn{}", self.0)
    }
}

/// Jump target (byte offset in instruction stream).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JumpTarget(pub u32);

impl fmt::Display for JumpTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@{}", self.0)
    }
}

/// Atomic block ID for transaction boundaries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AtomicId(pub u16);

/// X3 Virtual Machine Opcodes
///
/// Encoding: `[opcode: u8][operands...]`
///
/// Operand encoding:
/// - `reg`: u16 (register index)
/// - `const`: u32 (constant pool index)
/// - `func`: u32 (function index)
/// - `target`: u32 (jump target offset)
/// - `count`: u16 (argument/element count)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum Opcode {
    // ========================================================================
    // Control Flow (0x00 - 0x0F)
    // ========================================================================
    /// No operation. Used for padding/alignment.
    Nop = 0x00,

    /// Unconditional jump to target.
    /// `jump target:u32`
    Jump = 0x01,

    /// Conditional jump if register is truthy.
    /// `jump_if cond:reg target:u32`
    JumpIf = 0x02,

    /// Conditional jump if register is falsy.
    /// `jump_unless cond:reg target:u32`
    JumpUnless = 0x03,

    /// Call function with arguments.
    /// `call dst:reg func:u32 argc:u16 [args:reg...]`
    Call = 0x04,

    /// Return from function with value.
    /// `ret src:reg`
    Ret = 0x05,

    /// Return from function with no value (void).
    /// `ret_void`
    RetVoid = 0x06,

    /// Halt execution (end of program).
    /// `halt`
    Halt = 0x07,

    // ========================================================================
    // Load/Store (0x10 - 0x1F)
    // ========================================================================
    /// Load constant from pool into register.
    /// `load_const dst:reg idx:u32`
    LoadConst = 0x10,

    /// Copy register to register.
    /// `mov dst:reg src:reg`
    Mov = 0x11,

    /// Load global variable.
    /// `load_global dst:reg idx:u32`
    LoadGlobal = 0x12,

    /// Store to global variable.
    /// `store_global idx:u32 src:reg`
    StoreGlobal = 0x13,

    /// Load from array at index.
    /// `load_index dst:reg arr:reg idx:reg`
    LoadIndex = 0x14,

    /// Store to array at index.
    /// `store_index arr:reg idx:reg val:reg`
    StoreIndex = 0x15,

    /// Load field from struct/agent.
    /// `load_field dst:reg obj:reg field:u16`
    LoadField = 0x16,

    /// Store field to struct/agent.
    /// `store_field obj:reg field:u16 val:reg`
    StoreField = 0x17,

    /// Load immediate small integer (-128 to 127).
    /// `load_imm dst:reg val:i8`
    LoadImm = 0x18,

    /// Load zero into register.
    /// `load_zero dst:reg`
    LoadZero = 0x19,

    /// Load boolean true.
    /// `load_true dst:reg`
    LoadTrue = 0x1A,

    /// Load boolean false.
    /// `load_false dst:reg`
    LoadFalse = 0x1B,

    // ========================================================================
    // Integer Arithmetic (0x20 - 0x2F)
    // ========================================================================
    /// Integer addition: dst = a + b
    /// `add_i dst:reg a:reg b:reg`
    AddI = 0x20,

    /// Integer subtraction: dst = a - b
    /// `sub_i dst:reg a:reg b:reg`
    SubI = 0x21,

    /// Integer multiplication: dst = a * b
    /// `mul_i dst:reg a:reg b:reg`
    MulI = 0x22,

    /// Integer division: dst = a / b
    /// `div_i dst:reg a:reg b:reg`
    DivI = 0x23,

    /// Integer modulo: dst = a % b
    /// `mod_i dst:reg a:reg b:reg`
    ModI = 0x24,

    /// Integer negation: dst = -a
    /// `neg_i dst:reg src:reg`
    NegI = 0x25,

    /// Increment register: dst = src + 1
    /// `inc dst:reg src:reg`
    Inc = 0x26,

    /// Decrement register: dst = src - 1
    /// `dec dst:reg src:reg`
    Dec = 0x27,

    // ========================================================================
    // Float Arithmetic (0x30 - 0x3F)
    // ========================================================================
    /// Float addition: dst = a + b
    /// `add_f dst:reg a:reg b:reg`
    AddF = 0x30,

    /// Float subtraction: dst = a - b
    /// `sub_f dst:reg a:reg b:reg`
    SubF = 0x31,

    /// Float multiplication: dst = a * b
    /// `mul_f dst:reg a:reg b:reg`
    MulF = 0x32,

    /// Float division: dst = a / b
    /// `div_f dst:reg a:reg b:reg`
    DivF = 0x33,

    /// Float modulo: dst = a % b
    /// `mod_f dst:reg a:reg b:reg`
    ModF = 0x34,

    /// Float negation: dst = -a
    /// `neg_f dst:reg src:reg`
    NegF = 0x35,

    // ========================================================================
    // Comparison (0x40 - 0x4F)
    // ========================================================================
    /// Integer equality: dst = (a == b)
    /// `eq_i dst:reg a:reg b:reg`
    EqI = 0x40,

    /// Integer inequality: dst = (a != b)
    /// `ne_i dst:reg a:reg b:reg`
    NeI = 0x41,

    /// Integer less than: dst = (a < b)
    /// `lt_i dst:reg a:reg b:reg`
    LtI = 0x42,

    /// Integer less than or equal: dst = (a <= b)
    /// `le_i dst:reg a:reg b:reg`
    LeI = 0x43,

    /// Integer greater than: dst = (a > b)
    /// `gt_i dst:reg a:reg b:reg`
    GtI = 0x44,

    /// Integer greater than or equal: dst = (a >= b)
    /// `ge_i dst:reg a:reg b:reg`
    GeI = 0x45,

    /// Float equality: dst = (a == b)
    /// `eq_f dst:reg a:reg b:reg`
    EqF = 0x46,

    /// Float inequality: dst = (a != b)
    /// `ne_f dst:reg a:reg b:reg`
    NeF = 0x47,

    /// Float less than: dst = (a < b)
    /// `lt_f dst:reg a:reg b:reg`
    LtF = 0x48,

    /// Float less than or equal: dst = (a <= b)
    /// `le_f dst:reg a:reg b:reg`
    LeF = 0x49,

    /// Float greater than: dst = (a > b)
    /// `gt_f dst:reg a:reg b:reg`
    GtF = 0x4A,

    /// Float greater than or equal: dst = (a >= b)
    /// `ge_f dst:reg a:reg b:reg`
    GeF = 0x4B,

    // ========================================================================
    // Bitwise Operations (0x50 - 0x5F)
    // ========================================================================
    /// Bitwise AND: dst = a & b
    /// `and dst:reg a:reg b:reg`
    And = 0x50,

    /// Bitwise OR: dst = a | b
    /// `or dst:reg a:reg b:reg`
    Or = 0x51,

    /// Bitwise XOR: dst = a ^ b
    /// `xor dst:reg a:reg b:reg`
    Xor = 0x52,

    /// Bitwise NOT: dst = ~a
    /// `not dst:reg src:reg`
    Not = 0x53,

    /// Shift left: dst = a << b
    /// `shl dst:reg a:reg b:reg`
    Shl = 0x54,

    /// Shift right (arithmetic): dst = a >> b
    /// `shr dst:reg a:reg b:reg`
    Shr = 0x55,

    /// Shift right (logical): dst = a >>> b
    /// `ushr dst:reg a:reg b:reg`
    UShr = 0x56,

    // ========================================================================
    // Logical Operations (0x58 - 0x5F)
    // ========================================================================
    /// Logical AND (short-circuit in HIR, but explicit here)
    /// `land dst:reg a:reg b:reg`
    LAnd = 0x58,

    /// Logical OR
    /// `lor dst:reg a:reg b:reg`
    LOr = 0x59,

    /// Logical NOT: dst = !a
    /// `lnot dst:reg src:reg`
    LNot = 0x5A,

    // ========================================================================
    // Type Conversions (0x60 - 0x6F)
    // ========================================================================
    /// Convert i32 to i64: dst = (i64)src
    /// `i32_to_i64 dst:reg src:reg`
    I32ToI64 = 0x60,

    /// Convert i64 to i32 (truncate): dst = (i32)src
    /// `i64_to_i32 dst:reg src:reg`
    I64ToI32 = 0x61,

    /// Convert i32 to f32: dst = (f32)src
    /// `i32_to_f32 dst:reg src:reg`
    I32ToF32 = 0x62,

    /// Convert i64 to f64: dst = (f64)src
    /// `i64_to_f64 dst:reg src:reg`
    I64ToF64 = 0x63,

    /// Convert f32 to i32: dst = (i32)src
    /// `f32_to_i32 dst:reg src:reg`
    F32ToI32 = 0x64,

    /// Convert f64 to i64: dst = (i64)src
    /// `f64_to_i64 dst:reg src:reg`
    F64ToI64 = 0x65,

    /// Convert f32 to f64: dst = (f64)src
    /// `f32_to_f64 dst:reg src:reg`
    F32ToF64 = 0x66,

    /// Convert f64 to f32: dst = (f32)src
    /// `f64_to_f32 dst:reg src:reg`
    F64ToF32 = 0x67,

    /// Convert int to bool: dst = (bool)src
    /// `to_bool dst:reg src:reg`
    ToBool = 0x68,

    // ========================================================================
    // Array/Collection Operations (0x70 - 0x7F)
    // ========================================================================
    /// Create new array with given capacity.
    /// `new_array dst:reg capacity:u16`
    NewArray = 0x70,

    /// Get array length.
    /// `array_len dst:reg arr:reg`
    ArrayLen = 0x71,

    /// Push element to array.
    /// `array_push arr:reg val:reg`
    ArrayPush = 0x72,

    /// Pop element from array.
    /// `array_pop dst:reg arr:reg`
    ArrayPop = 0x73,

    /// Create tuple from registers.
    /// `new_tuple dst:reg count:u16 [elements:reg...]`
    NewTuple = 0x74,

    /// Get tuple element.
    /// `tuple_get dst:reg tuple:reg idx:u16`
    TupleGet = 0x75,

    // ========================================================================
    // X3 Context Operations (0x80 - 0x8F)
    // ========================================================================
    /// Load context.sender into register.
    /// `ctx_sender dst:reg`
    CtxSender = 0x80,

    /// Load context.block_height into register.
    /// `ctx_block_height dst:reg`
    CtxBlockHeight = 0x81,

    /// Load context.timestamp into register.
    /// `ctx_timestamp dst:reg`
    CtxTimestamp = 0x82,

    /// Load context.value into register.
    /// `ctx_value dst:reg`
    CtxValue = 0x83,

    /// Load context.gas_remaining into register.
    /// `ctx_gas dst:reg`
    CtxGas = 0x84,

    /// Load context.chain_id into register.
    /// `ctx_chain_id dst:reg`
    CtxChainId = 0x85,

    // ========================================================================
    // X3 Atomic Operations (0x90 - 0x9F)
    // ========================================================================
    /// Begin atomic transaction block.
    /// `atomic_begin id:u16`
    AtomicBegin = 0x90,

    /// Commit atomic transaction block.
    /// `atomic_commit id:u16`
    AtomicCommit = 0x91,

    /// Rollback atomic transaction block.
    /// `atomic_rollback id:u16`
    AtomicRollback = 0x92,

    /// Check if inside atomic block.
    /// `atomic_check dst:reg`
    AtomicCheck = 0x93,

    // ========================================================================
    // X3 Agent Operations (0xA0 - 0xAF)
    // ========================================================================
    /// Load agent self reference.
    /// `agent_self dst:reg`
    AgentSelf = 0xA0,

    /// Initialize agent with field values.
    /// `agent_init agent:reg field_count:u16 [field_idx:u16 val:reg...]`
    AgentInit = 0xA1,

    /// Emit event.
    /// `emit event_id:u32 argc:u16 [args:reg...]`
    Emit = 0xA2,

    // ========================================================================
    // VM Intrinsics (0xB0 - 0xCF)
    // ========================================================================
    // EVM Intrinsics (0xB0 - 0xBF)
    /// EVM call to external contract.
    /// `evm_call dst:reg gas:reg addr:reg value:reg data:reg`
    EvmCall = 0xB0,

    /// EVM static call (read-only).
    /// `evm_staticcall dst:reg gas:reg addr:reg data:reg`
    EvmStaticCall = 0xB1,

    /// EVM delegate call.
    /// `evm_delegatecall dst:reg gas:reg addr:reg data:reg`
    EvmDelegateCall = 0xB2,

    /// EVM storage load.
    /// `evm_sload dst:reg slot:reg`
    EvmSload = 0xB3,

    /// EVM storage store.
    /// `evm_sstore slot:reg val:reg`
    EvmSstore = 0xB4,

    /// EVM create contract.
    /// `evm_create dst:reg value:reg code:reg`
    EvmCreate = 0xB5,

    /// EVM create2 contract.
    /// `evm_create2 dst:reg value:reg code:reg salt:reg`
    EvmCreate2 = 0xB6,

    /// EVM log (event emission).
    /// `evm_log topic_count:u8 [topics:reg...] data:reg`
    EvmLog = 0xB7,

    /// EVM get balance.
    /// `evm_balance dst:reg addr:reg`
    EvmBalance = 0xB8,

    /// EVM get code size.
    /// `evm_codesize dst:reg addr:reg`
    EvmCodeSize = 0xB9,

    // SVM Intrinsics (0xC0 - 0xCF)
    /// SVM cross-program invocation.
    /// `svm_invoke dst:reg program:reg accounts:reg data:reg`
    SvmInvoke = 0xC0,

    /// SVM signed invocation.
    /// `svm_invoke_signed dst:reg program:reg accounts:reg data:reg seeds:reg`
    SvmInvokeSigned = 0xC1,

    /// SVM create account.
    /// `svm_create_account dst:reg lamports:reg space:reg owner:reg`
    SvmCreateAccount = 0xC2,

    /// SVM transfer lamports.
    /// `svm_transfer from:reg to:reg lamports:reg`
    SvmTransfer = 0xC3,

    /// SVM get account data.
    /// `svm_get_data dst:reg account:reg`
    SvmGetData = 0xC4,

    /// SVM set account data.
    /// `svm_set_data account:reg data:reg`
    SvmSetData = 0xC5,

    /// SVM get rent.
    /// `svm_get_rent dst:reg`
    SvmGetRent = 0xC6,

    /// SVM get clock.
    /// `svm_get_clock dst:reg`
    SvmGetClock = 0xC7,

    // ========================================================================
    // GPU Compute Intrinsics (0xD0 - 0xDF)
    // ========================================================================
    /// GPU SHA-256 batch hash: dst = gpu_sha256_batch(inputs, count)
    /// `gpu_sha256_batch dst:reg inputs:reg count:reg`
    GpuSha256Batch = 0xD0,

    /// GPU Ed25519 batch verify: dst = gpu_ed25519_verify(sigs, count)
    /// `gpu_ed25519_verify dst:reg sigs:reg count:reg`
    GpuEd25519Verify = 0xD1,

    /// GPU PoH chain: dst = gpu_poh_chain(seeds, count, chain_length)
    /// `gpu_poh_chain dst:reg seeds:reg count:reg chain_len:reg`
    GpuPohChain = 0xD2,

    /// GPU SHA-256 streamed batch (with CUDA stream pipelining).
    /// `gpu_sha256_streamed dst:reg inputs:reg count:reg streams:reg`
    GpuSha256Streamed = 0xD3,

    /// GPU device info query: dst = gpu_device_count()
    /// `gpu_device_count dst:reg`
    GpuDeviceCount = 0xD4,

    /// GPU pipeline benchmark: dst = gpu_benchmark(count, streams)
    /// `gpu_benchmark dst:reg count:reg streams:reg`
    GpuBenchmark = 0xD5,

    /// GPU Keccak-256 batch hash: dst = gpu_keccak256_batch(inputs, count)
    /// `gpu_keccak256_batch dst:reg inputs:reg count:reg`
    GpuKeccak256Batch = 0xD6,

    /// GPU secp256k1 ECDSA batch verify: dst = gpu_secp256k1_verify(sigs, count)
    /// `gpu_secp256k1_verify dst:reg sigs:reg count:reg`
    GpuSecp256k1Verify = 0xD7,

    // ========================================================================
    // Trading extension (0xE0 - 0xE9), format 1.1
    // ========================================================================
    // Each operation operand is a `u16` index into the module's trading section
    // (`crate::trading::TradingSection::ops`); the verifier checks the index is in range and that
    // the indexed operation is the kind the opcode names. Every trading opcode must run inside an
    // atomic window, whose snapshot covers the trading journal (balances, reserves, debts, costs,
    // events, intents, transcript).
    /// Open the trade: chain, deadline, private-submission and capability checks.
    /// `trade_begin`
    TradeBegin = 0xE0,

    /// Flash-borrow from a provider. `trade_borrow op:u16`
    TradeBorrow = 0xE1,

    /// Constant-product swap at a venue. `trade_swap op:u16`
    TradeSwap = 0xE2,

    /// Repay a flash debt with its fee. `trade_repay op:u16`
    TradeRepay = 0xE3,

    /// Commit every cost (gas through the end of the function, settlement, protocol, relay,
    /// oracle, solver, cross-domain). `trade_charge_costs op:u16`
    TradeChargeCosts = 0xE4,

    /// Net-profit floor in an asset, after every cost. `trade_assert_profit op:u16`
    TradeAssertProfit = 0xE5,

    /// Every debt is closed. `trade_assert_debts_closed op:u16`
    TradeAssertDebtsClosed = 0xE6,

    /// Every asset's net delta is non-negative. `trade_assert_solvent op:u16`
    TradeAssertSolvent = 0xE7,

    /// Close the trade: open debts, uncharged costs or a missing profit assertion trap.
    /// `trade_commit`
    TradeCommit = 0xE8,

    /// Realized net profit of the profit asset, as a checked `i64`. `trade_load_profit dst:reg`
    TradeLoadProfit = 0xE9,

    // ========================================================================
    // Debug/Meta (0xF0 - 0xFF)
    // ========================================================================
    /// Debug print register value.
    /// `debug_print src:reg`
    DebugPrint = 0xF0,

    /// Breakpoint for debugger.
    /// `breakpoint`
    Breakpoint = 0xF1,

    /// Assert condition (trap if false).
    /// `assert cond:reg msg_idx:u32`
    Assert = 0xF2,

    /// Panic with message.
    /// `panic msg_idx:u32`
    Panic = 0xF3,
}

impl Opcode {
    /// Get opcode from byte value.
    pub fn from_byte(byte: u8) -> Option<Opcode> {
        // Use a match for safety (repr(u8) guarantees values)
        Some(match byte {
            0x00 => Opcode::Nop,
            0x01 => Opcode::Jump,
            0x02 => Opcode::JumpIf,
            0x03 => Opcode::JumpUnless,
            0x04 => Opcode::Call,
            0x05 => Opcode::Ret,
            0x06 => Opcode::RetVoid,
            0x07 => Opcode::Halt,

            0x10 => Opcode::LoadConst,
            0x11 => Opcode::Mov,
            0x12 => Opcode::LoadGlobal,
            0x13 => Opcode::StoreGlobal,
            0x14 => Opcode::LoadIndex,
            0x15 => Opcode::StoreIndex,
            0x16 => Opcode::LoadField,
            0x17 => Opcode::StoreField,
            0x18 => Opcode::LoadImm,
            0x19 => Opcode::LoadZero,
            0x1A => Opcode::LoadTrue,
            0x1B => Opcode::LoadFalse,

            0x20 => Opcode::AddI,
            0x21 => Opcode::SubI,
            0x22 => Opcode::MulI,
            0x23 => Opcode::DivI,
            0x24 => Opcode::ModI,
            0x25 => Opcode::NegI,
            0x26 => Opcode::Inc,
            0x27 => Opcode::Dec,

            0x30 => Opcode::AddF,
            0x31 => Opcode::SubF,
            0x32 => Opcode::MulF,
            0x33 => Opcode::DivF,
            0x34 => Opcode::ModF,
            0x35 => Opcode::NegF,

            0x40 => Opcode::EqI,
            0x41 => Opcode::NeI,
            0x42 => Opcode::LtI,
            0x43 => Opcode::LeI,
            0x44 => Opcode::GtI,
            0x45 => Opcode::GeI,
            0x46 => Opcode::EqF,
            0x47 => Opcode::NeF,
            0x48 => Opcode::LtF,
            0x49 => Opcode::LeF,
            0x4A => Opcode::GtF,
            0x4B => Opcode::GeF,

            0x50 => Opcode::And,
            0x51 => Opcode::Or,
            0x52 => Opcode::Xor,
            0x53 => Opcode::Not,
            0x54 => Opcode::Shl,
            0x55 => Opcode::Shr,
            0x56 => Opcode::UShr,
            0x58 => Opcode::LAnd,
            0x59 => Opcode::LOr,
            0x5A => Opcode::LNot,

            0x60 => Opcode::I32ToI64,
            0x61 => Opcode::I64ToI32,
            0x62 => Opcode::I32ToF32,
            0x63 => Opcode::I64ToF64,
            0x64 => Opcode::F32ToI32,
            0x65 => Opcode::F64ToI64,
            0x66 => Opcode::F32ToF64,
            0x67 => Opcode::F64ToF32,
            0x68 => Opcode::ToBool,

            0x70 => Opcode::NewArray,
            0x71 => Opcode::ArrayLen,
            0x72 => Opcode::ArrayPush,
            0x73 => Opcode::ArrayPop,
            0x74 => Opcode::NewTuple,
            0x75 => Opcode::TupleGet,

            0x80 => Opcode::CtxSender,
            0x81 => Opcode::CtxBlockHeight,
            0x82 => Opcode::CtxTimestamp,
            0x83 => Opcode::CtxValue,
            0x84 => Opcode::CtxGas,
            0x85 => Opcode::CtxChainId,

            0x90 => Opcode::AtomicBegin,
            0x91 => Opcode::AtomicCommit,
            0x92 => Opcode::AtomicRollback,
            0x93 => Opcode::AtomicCheck,

            0xA0 => Opcode::AgentSelf,
            0xA1 => Opcode::AgentInit,
            0xA2 => Opcode::Emit,

            0xB0 => Opcode::EvmCall,
            0xB1 => Opcode::EvmStaticCall,
            0xB2 => Opcode::EvmDelegateCall,
            0xB3 => Opcode::EvmSload,
            0xB4 => Opcode::EvmSstore,
            0xB5 => Opcode::EvmCreate,
            0xB6 => Opcode::EvmCreate2,
            0xB7 => Opcode::EvmLog,
            0xB8 => Opcode::EvmBalance,
            0xB9 => Opcode::EvmCodeSize,

            0xC0 => Opcode::SvmInvoke,
            0xC1 => Opcode::SvmInvokeSigned,
            0xC2 => Opcode::SvmCreateAccount,
            0xC3 => Opcode::SvmTransfer,
            0xC4 => Opcode::SvmGetData,
            0xC5 => Opcode::SvmSetData,
            0xC6 => Opcode::SvmGetRent,
            0xC7 => Opcode::SvmGetClock,

            0xD0 => Opcode::GpuSha256Batch,
            0xD1 => Opcode::GpuEd25519Verify,
            0xD2 => Opcode::GpuPohChain,
            0xD3 => Opcode::GpuSha256Streamed,
            0xD4 => Opcode::GpuDeviceCount,
            0xD5 => Opcode::GpuBenchmark,
            0xD6 => Opcode::GpuKeccak256Batch,
            0xD7 => Opcode::GpuSecp256k1Verify,

            0xE0 => Opcode::TradeBegin,
            0xE1 => Opcode::TradeBorrow,
            0xE2 => Opcode::TradeSwap,
            0xE3 => Opcode::TradeRepay,
            0xE4 => Opcode::TradeChargeCosts,
            0xE5 => Opcode::TradeAssertProfit,
            0xE6 => Opcode::TradeAssertDebtsClosed,
            0xE7 => Opcode::TradeAssertSolvent,
            0xE8 => Opcode::TradeCommit,
            0xE9 => Opcode::TradeLoadProfit,

            0xF0 => Opcode::DebugPrint,
            0xF1 => Opcode::Breakpoint,
            0xF2 => Opcode::Assert,
            0xF3 => Opcode::Panic,

            _ => return None,
        })
    }

    /// Convert opcode to its byte value.
    pub fn to_byte(self) -> u8 {
        self as u8
    }

    /// Get human-readable name for opcode.
    pub fn name(self) -> &'static str {
        match self {
            Opcode::Nop => "nop",
            Opcode::Jump => "jump",
            Opcode::JumpIf => "jump_if",
            Opcode::JumpUnless => "jump_unless",
            Opcode::Call => "call",
            Opcode::Ret => "ret",
            Opcode::RetVoid => "ret_void",
            Opcode::Halt => "halt",

            Opcode::LoadConst => "load_const",
            Opcode::Mov => "mov",
            Opcode::LoadGlobal => "load_global",
            Opcode::StoreGlobal => "store_global",
            Opcode::LoadIndex => "load_index",
            Opcode::StoreIndex => "store_index",
            Opcode::LoadField => "load_field",
            Opcode::StoreField => "store_field",
            Opcode::LoadImm => "load_imm",
            Opcode::LoadZero => "load_zero",
            Opcode::LoadTrue => "load_true",
            Opcode::LoadFalse => "load_false",

            Opcode::AddI => "add_i",
            Opcode::SubI => "sub_i",
            Opcode::MulI => "mul_i",
            Opcode::DivI => "div_i",
            Opcode::ModI => "mod_i",
            Opcode::NegI => "neg_i",
            Opcode::Inc => "inc",
            Opcode::Dec => "dec",

            Opcode::AddF => "add_f",
            Opcode::SubF => "sub_f",
            Opcode::MulF => "mul_f",
            Opcode::DivF => "div_f",
            Opcode::ModF => "mod_f",
            Opcode::NegF => "neg_f",

            Opcode::EqI => "eq_i",
            Opcode::NeI => "ne_i",
            Opcode::LtI => "lt_i",
            Opcode::LeI => "le_i",
            Opcode::GtI => "gt_i",
            Opcode::GeI => "ge_i",
            Opcode::EqF => "eq_f",
            Opcode::NeF => "ne_f",
            Opcode::LtF => "lt_f",
            Opcode::LeF => "le_f",
            Opcode::GtF => "gt_f",
            Opcode::GeF => "ge_f",

            Opcode::And => "and",
            Opcode::Or => "or",
            Opcode::Xor => "xor",
            Opcode::Not => "not",
            Opcode::Shl => "shl",
            Opcode::Shr => "shr",
            Opcode::UShr => "ushr",
            Opcode::LAnd => "land",
            Opcode::LOr => "lor",
            Opcode::LNot => "lnot",

            Opcode::I32ToI64 => "i32_to_i64",
            Opcode::I64ToI32 => "i64_to_i32",
            Opcode::I32ToF32 => "i32_to_f32",
            Opcode::I64ToF64 => "i64_to_f64",
            Opcode::F32ToI32 => "f32_to_i32",
            Opcode::F64ToI64 => "f64_to_i64",
            Opcode::F32ToF64 => "f32_to_f64",
            Opcode::F64ToF32 => "f64_to_f32",
            Opcode::ToBool => "to_bool",

            Opcode::NewArray => "new_array",
            Opcode::ArrayLen => "array_len",
            Opcode::ArrayPush => "array_push",
            Opcode::ArrayPop => "array_pop",
            Opcode::NewTuple => "new_tuple",
            Opcode::TupleGet => "tuple_get",

            Opcode::CtxSender => "ctx_sender",
            Opcode::CtxBlockHeight => "ctx_block_height",
            Opcode::CtxTimestamp => "ctx_timestamp",
            Opcode::CtxValue => "ctx_value",
            Opcode::CtxGas => "ctx_gas",
            Opcode::CtxChainId => "ctx_chain_id",

            Opcode::AtomicBegin => "atomic_begin",
            Opcode::AtomicCommit => "atomic_commit",
            Opcode::AtomicRollback => "atomic_rollback",
            Opcode::AtomicCheck => "atomic_check",

            Opcode::AgentSelf => "agent_self",
            Opcode::AgentInit => "agent_init",
            Opcode::Emit => "emit",

            Opcode::EvmCall => "evm_call",
            Opcode::EvmStaticCall => "evm_staticcall",
            Opcode::EvmDelegateCall => "evm_delegatecall",
            Opcode::EvmSload => "evm_sload",
            Opcode::EvmSstore => "evm_sstore",
            Opcode::EvmCreate => "evm_create",
            Opcode::EvmCreate2 => "evm_create2",
            Opcode::EvmLog => "evm_log",
            Opcode::EvmBalance => "evm_balance",
            Opcode::EvmCodeSize => "evm_codesize",

            Opcode::SvmInvoke => "svm_invoke",
            Opcode::SvmInvokeSigned => "svm_invoke_signed",
            Opcode::SvmCreateAccount => "svm_create_account",
            Opcode::SvmTransfer => "svm_transfer",
            Opcode::SvmGetData => "svm_get_data",
            Opcode::SvmSetData => "svm_set_data",
            Opcode::SvmGetRent => "svm_get_rent",
            Opcode::SvmGetClock => "svm_get_clock",

            Opcode::GpuSha256Batch => "gpu_sha256_batch",
            Opcode::GpuEd25519Verify => "gpu_ed25519_verify",
            Opcode::GpuPohChain => "gpu_poh_chain",
            Opcode::GpuSha256Streamed => "gpu_sha256_streamed",
            Opcode::GpuDeviceCount => "gpu_device_count",
            Opcode::GpuBenchmark => "gpu_benchmark",
            Opcode::GpuKeccak256Batch => "gpu_keccak256_batch",
            Opcode::GpuSecp256k1Verify => "gpu_secp256k1_verify",

            Opcode::TradeBegin => "trade_begin",
            Opcode::TradeBorrow => "trade_borrow",
            Opcode::TradeSwap => "trade_swap",
            Opcode::TradeRepay => "trade_repay",
            Opcode::TradeChargeCosts => "trade_charge_costs",
            Opcode::TradeAssertProfit => "trade_assert_profit",
            Opcode::TradeAssertDebtsClosed => "trade_assert_debts_closed",
            Opcode::TradeAssertSolvent => "trade_assert_solvent",
            Opcode::TradeCommit => "trade_commit",
            Opcode::TradeLoadProfit => "trade_load_profit",

            Opcode::DebugPrint => "debug_print",
            Opcode::Breakpoint => "breakpoint",
            Opcode::Assert => "assert",
            Opcode::Panic => "panic",
        }
    }

    /// Check if this opcode is a control flow instruction.
    pub fn is_control_flow(self) -> bool {
        matches!(
            self,
            Opcode::Jump
                | Opcode::JumpIf
                | Opcode::JumpUnless
                | Opcode::Call
                | Opcode::Ret
                | Opcode::RetVoid
                | Opcode::Halt
        )
    }

    /// Check if this opcode is a terminator (ends a basic block).
    pub fn is_terminator(self) -> bool {
        matches!(
            self,
            Opcode::Jump | Opcode::Ret | Opcode::RetVoid | Opcode::Halt | Opcode::Panic
        )
    }

    /// Check if this opcode has side effects.
    pub fn has_side_effects(self) -> bool {
        matches!(
            self,
            Opcode::Call
                | Opcode::StoreGlobal
                | Opcode::StoreIndex
                | Opcode::StoreField
                | Opcode::ArrayPush
                | Opcode::ArrayPop
                | Opcode::AtomicBegin
                | Opcode::AtomicCommit
                | Opcode::AtomicRollback
                | Opcode::Emit
                | Opcode::AgentInit
                | Opcode::EvmCall
                | Opcode::EvmStaticCall
                | Opcode::EvmDelegateCall
                | Opcode::EvmSstore
                | Opcode::EvmCreate
                | Opcode::EvmCreate2
                | Opcode::EvmLog
                | Opcode::SvmInvoke
                | Opcode::SvmInvokeSigned
                | Opcode::SvmCreateAccount
                | Opcode::SvmTransfer
                | Opcode::SvmSetData
                | Opcode::GpuSha256Batch
                | Opcode::GpuEd25519Verify
                | Opcode::GpuPohChain
                | Opcode::GpuSha256Streamed
                | Opcode::GpuBenchmark
                | Opcode::GpuKeccak256Batch
                | Opcode::GpuSecp256k1Verify
                | Opcode::DebugPrint
                | Opcode::Panic
                | Opcode::TradeBegin
                | Opcode::TradeBorrow
                | Opcode::TradeSwap
                | Opcode::TradeRepay
                | Opcode::TradeChargeCosts
                | Opcode::TradeCommit
        )
    }

    /// Check if this opcode belongs to the trading extension (format 1.1).
    pub fn is_trading_op(self) -> bool {
        (0xE0..=0xE9).contains(&(self as u8))
    }

    /// Does this trading opcode carry a trading-section operation index?
    pub fn trading_op_index_operand(self) -> bool {
        matches!(
            self,
            Opcode::TradeBorrow
                | Opcode::TradeSwap
                | Opcode::TradeRepay
                | Opcode::TradeChargeCosts
                | Opcode::TradeAssertProfit
                | Opcode::TradeAssertDebtsClosed
                | Opcode::TradeAssertSolvent
        )
    }

    /// Check if this opcode accesses EVM state
    pub fn is_evm_intrinsic(self) -> bool {
        matches!(
            self,
            Opcode::EvmCall
                | Opcode::EvmStaticCall
                | Opcode::EvmDelegateCall
                | Opcode::EvmSload
                | Opcode::EvmSstore
                | Opcode::EvmCreate
                | Opcode::EvmCreate2
                | Opcode::EvmLog
                | Opcode::EvmBalance
                | Opcode::EvmCodeSize
        )
    }

    /// Check if this opcode accesses SVM state
    pub fn is_svm_intrinsic(self) -> bool {
        matches!(
            self,
            Opcode::SvmInvoke
                | Opcode::SvmInvokeSigned
                | Opcode::SvmCreateAccount
                | Opcode::SvmTransfer
                | Opcode::SvmGetData
                | Opcode::SvmSetData
                | Opcode::SvmGetRent
                | Opcode::SvmGetClock
        )
    }

    /// Check if this opcode crosses VM boundaries (reads/writes other VM state)
    pub fn crosses_vm_boundary(self) -> bool {
        self.is_evm_intrinsic() || self.is_svm_intrinsic() || self.is_gpu_intrinsic()
    }

    /// Check if this opcode dispatches GPU compute
    pub fn is_gpu_intrinsic(self) -> bool {
        matches!(
            self,
            Opcode::GpuSha256Batch
                | Opcode::GpuEd25519Verify
                | Opcode::GpuPohChain
                | Opcode::GpuSha256Streamed
                | Opcode::GpuDeviceCount
                | Opcode::GpuBenchmark
                | Opcode::GpuKeccak256Batch
                | Opcode::GpuSecp256k1Verify
        )
    }

    /// Check if this opcode is part of atomic transaction semantics
    pub fn is_atomic_op(self) -> bool {
        matches!(
            self,
            Opcode::AtomicBegin
                | Opcode::AtomicCommit
                | Opcode::AtomicRollback
                | Opcode::AtomicCheck
        )
    }

    /// Get gas cost category hint for optimizer (for gas-aware optimizations)
    pub fn gas_cost_category(self) -> &'static str {
        match self {
            // Cheap: 1-3 gas
            Opcode::Nop
            | Opcode::LoadImm
            | Opcode::LoadZero
            | Opcode::LoadTrue
            | Opcode::LoadFalse
            | Opcode::Mov
            | Opcode::Inc
            | Opcode::Dec => "cheap",

            // Medium: 3-10 gas
            Opcode::AddI
            | Opcode::SubI
            | Opcode::MulI
            | Opcode::And
            | Opcode::Or
            | Opcode::Xor
            | Opcode::Not
            | Opcode::Shl
            | Opcode::Shr
            | Opcode::UShr
            | Opcode::LoadConst
            | Opcode::CtxSender
            | Opcode::CtxBlockHeight
            | Opcode::CtxTimestamp => "medium",

            // Expensive: 10-50 gas
            Opcode::DivI
            | Opcode::ModI
            | Opcode::AddF
            | Opcode::SubF
            | Opcode::MulF
            | Opcode::DivF
            | Opcode::ModF
            | Opcode::NewArray
            | Opcode::LoadIndex
            | Opcode::StoreIndex => "expensive",

            // Very expensive: 100+ gas (cross-VM, storage, atomics)
            Opcode::Call
            | Opcode::EvmCall
            | Opcode::EvmStaticCall
            | Opcode::EvmDelegateCall
            | Opcode::EvmSload
            | Opcode::EvmSstore
            | Opcode::EvmCreate
            | Opcode::EvmCreate2
            | Opcode::EvmLog
            | Opcode::SvmInvoke
            | Opcode::SvmInvokeSigned
            | Opcode::SvmCreateAccount
            | Opcode::SvmTransfer
            | Opcode::AtomicBegin
            | Opcode::AtomicCommit
            | Opcode::AtomicRollback
            | Opcode::Emit
            | Opcode::GpuSha256Batch
            | Opcode::GpuEd25519Verify
            | Opcode::GpuPohChain
            | Opcode::GpuSha256Streamed
            | Opcode::GpuBenchmark => "very_expensive",

            // Unknown/special
            _ => "unknown",
        }
    }
}

impl fmt::Display for Opcode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// A decoded instruction with its operands.
#[derive(Clone, Debug, PartialEq)]
pub enum Instruction {
    Nop,
    Jump(JumpTarget),
    JumpIf(Register, JumpTarget),
    JumpUnless(Register, JumpTarget),
    Call {
        dst: Register,
        func: FuncIdx,
        args: Vec<Register>,
    },
    Ret(Register),
    RetVoid,
    Halt,

    LoadConst(Register, ConstIdx),
    Mov(Register, Register),
    LoadGlobal(Register, u32),
    StoreGlobal(u32, Register),
    LoadIndex(Register, Register, Register),
    StoreIndex(Register, Register, Register),
    LoadField(Register, Register, u16),
    StoreField(Register, u16, Register),
    LoadImm(Register, i8),
    LoadZero(Register),
    LoadTrue(Register),
    LoadFalse(Register),

    AddI(Register, Register, Register),
    SubI(Register, Register, Register),
    MulI(Register, Register, Register),
    DivI(Register, Register, Register),
    ModI(Register, Register, Register),
    NegI(Register, Register),
    Inc(Register, Register),
    Dec(Register, Register),

    AddF(Register, Register, Register),
    SubF(Register, Register, Register),
    MulF(Register, Register, Register),
    DivF(Register, Register, Register),
    ModF(Register, Register, Register),
    NegF(Register, Register),

    EqI(Register, Register, Register),
    NeI(Register, Register, Register),
    LtI(Register, Register, Register),
    LeI(Register, Register, Register),
    GtI(Register, Register, Register),
    GeI(Register, Register, Register),
    EqF(Register, Register, Register),
    NeF(Register, Register, Register),
    LtF(Register, Register, Register),
    LeF(Register, Register, Register),
    GtF(Register, Register, Register),
    GeF(Register, Register, Register),

    And(Register, Register, Register),
    Or(Register, Register, Register),
    Xor(Register, Register, Register),
    Not(Register, Register),
    Shl(Register, Register, Register),
    Shr(Register, Register, Register),
    UShr(Register, Register, Register),
    LAnd(Register, Register, Register),
    LOr(Register, Register, Register),
    LNot(Register, Register),

    I32ToI64(Register, Register),
    I64ToI32(Register, Register),
    I32ToF32(Register, Register),
    I64ToF64(Register, Register),
    F32ToI32(Register, Register),
    F64ToI64(Register, Register),
    F32ToF64(Register, Register),
    F64ToF32(Register, Register),
    ToBool(Register, Register),

    NewArray(Register, u16),
    ArrayLen(Register, Register),
    ArrayPush(Register, Register),
    ArrayPop(Register, Register),
    NewTuple {
        dst: Register,
        elements: Vec<Register>,
    },
    TupleGet(Register, Register, u16),

    CtxSender(Register),
    CtxBlockHeight(Register),
    CtxTimestamp(Register),
    CtxValue(Register),
    CtxGas(Register),
    CtxChainId(Register),

    AtomicBegin(AtomicId),
    AtomicCommit(AtomicId),
    AtomicRollback(AtomicId),
    AtomicCheck(Register),

    AgentSelf(Register),
    AgentInit {
        agent: Register,
        fields: Vec<(u16, Register)>,
    },
    Emit {
        event_id: u32,
        args: Vec<Register>,
    },

    // VM intrinsics (simplified representation)
    EvmCall {
        dst: Register,
        gas: Register,
        addr: Register,
        value: Register,
        data: Register,
    },
    EvmStaticCall {
        dst: Register,
        gas: Register,
        addr: Register,
        data: Register,
    },
    EvmSload(Register, Register),
    EvmSstore(Register, Register),

    SvmInvoke {
        dst: Register,
        program: Register,
        accounts: Register,
        data: Register,
    },
    SvmTransfer(Register, Register, Register),

    // GPU Compute intrinsics
    GpuSha256Batch {
        dst: Register,
        inputs: Register,
        count: Register,
    },
    GpuEd25519Verify {
        dst: Register,
        sigs: Register,
        count: Register,
    },
    GpuPohChain {
        dst: Register,
        seeds: Register,
        count: Register,
        chain_len: Register,
    },
    GpuSha256Streamed {
        dst: Register,
        inputs: Register,
        count: Register,
        streams: Register,
    },
    GpuDeviceCount(Register),
    GpuBenchmark {
        dst: Register,
        count: Register,
        streams: Register,
    },

    DebugPrint(Register),
    Breakpoint,
    Assert(Register, ConstIdx),
    Panic(ConstIdx),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcode_roundtrip() {
        // Test a sampling of opcodes for byte roundtrip
        let opcodes = [
            Opcode::Nop,
            Opcode::Jump,
            Opcode::Call,
            Opcode::AddI,
            Opcode::EqI,
            Opcode::AtomicBegin,
            Opcode::EvmCall,
            Opcode::SvmInvoke,
            Opcode::Panic,
        ];

        for op in opcodes {
            let byte = op.to_byte();
            let decoded = Opcode::from_byte(byte);
            assert_eq!(decoded, Some(op), "Roundtrip failed for {:?}", op);
        }
    }

    #[test]
    fn opcode_names() {
        assert_eq!(Opcode::Nop.name(), "nop");
        assert_eq!(Opcode::AddI.name(), "add_i");
        assert_eq!(Opcode::EvmCall.name(), "evm_call");
        assert_eq!(Opcode::AtomicBegin.name(), "atomic_begin");
    }

    #[test]
    fn opcode_classifications() {
        assert!(Opcode::Jump.is_control_flow());
        assert!(Opcode::Call.is_control_flow());
        assert!(!Opcode::AddI.is_control_flow());

        assert!(Opcode::Jump.is_terminator());
        assert!(Opcode::Ret.is_terminator());
        assert!(!Opcode::JumpIf.is_terminator());

        assert!(Opcode::Call.has_side_effects());
        assert!(Opcode::EvmSstore.has_side_effects());
        assert!(!Opcode::AddI.has_side_effects());
    }

    #[test]
    fn register_display() {
        assert_eq!(format!("{}", Register(0)), "r0");
        assert_eq!(format!("{}", Register(42)), "r42");
        assert_eq!(format!("{}", Register::RET), "r0");
        assert_eq!(format!("{}", Register::ARG0), "r1");
    }

    #[test]
    fn opcode_vm_hints() {
        // VM-aware optimization hints for optimizer passes
        assert!(Opcode::AtomicBegin.has_side_effects()); // Atomic boundaries = side effects (not control flow)
        assert!(Opcode::EvmCall.has_side_effects()); // Cross-VM calls = side effects
        assert!(!Opcode::AddI.has_side_effects()); // Pure arithmetic
        assert!(Opcode::SvmInvoke.has_side_effects()); // SVM invoke = side effects
    }

    #[test]
    fn opcode_dual_vm_coverage() {
        // Verify complete coverage of EVM and SVM intrinsics
        let evm_opcodes = [
            Opcode::EvmCall,
            Opcode::EvmStaticCall,
            Opcode::EvmDelegateCall,
            Opcode::EvmSload,
            Opcode::EvmSstore,
            Opcode::EvmCreate,
            Opcode::EvmCreate2,
            Opcode::EvmLog,
            Opcode::EvmBalance,
            Opcode::EvmCodeSize,
        ];
        for op in evm_opcodes {
            assert!(op.to_byte() >= 0xB0 && op.to_byte() <= 0xB9, "{:?}", op);
        }

        let svm_opcodes = [
            Opcode::SvmInvoke,
            Opcode::SvmInvokeSigned,
            Opcode::SvmCreateAccount,
            Opcode::SvmTransfer,
            Opcode::SvmGetData,
            Opcode::SvmSetData,
            Opcode::SvmGetRent,
            Opcode::SvmGetClock,
        ];
        for op in svm_opcodes {
            assert!(op.to_byte() >= 0xC0 && op.to_byte() <= 0xC7, "{:?}", op);
        }
    }
}
