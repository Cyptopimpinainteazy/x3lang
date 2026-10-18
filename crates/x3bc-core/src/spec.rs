//! The canonical description of every X3BC opcode.
//!
//! One exhaustive `match` (no wildcard arm), so adding an opcode to [`Opcode`] without saying how
//! it is encoded, what it costs, whether the chain runs it and what it can trap with does not
//! compile. The verifier's decoder, its static gas bound, the interpreter's charge and the on-chain
//! admission rules all read this table; `crates/x3-vm/tests/opcode_spec_exhaustive.rs` holds the
//! engine to it.

use crate::opcode::Opcode;
use crate::trap::TrapId;

/// How an operand is encoded in the instruction stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperandKind {
    /// A register, one byte.
    Reg,
    /// A signed 8-bit immediate, sign-extended when decoded.
    Imm8,
    /// A one-byte index (`emit`'s event name).
    Index8,
    /// A `u16` index (field, capacity).
    Index16,
    /// A `u32` index (constant, global, function, message).
    Index32,
    /// A `u32` code offset.
    Target32,
    /// An atomic window id, `u16`.
    AtomicId,
    /// A trading-section operation index, `u16`.
    TradeOp,
    /// A `u16` count followed by that many registers.
    RegList16,
    /// A `u16` count followed by that many (field, register) byte pairs.
    PairList16,
    /// A `u8` count followed by that many registers.
    TopicList8,
}

impl OperandKind {
    /// Fixed width in bytes. For the list kinds, the width of the count prefix alone.
    pub const fn width(&self) -> usize {
        match self {
            OperandKind::Reg
            | OperandKind::Imm8
            | OperandKind::Index8
            | OperandKind::TopicList8 => 1,
            OperandKind::Index16
            | OperandKind::AtomicId
            | OperandKind::TradeOp
            | OperandKind::RegList16
            | OperandKind::PairList16 => 2,
            OperandKind::Index32 | OperandKind::Target32 => 4,
        }
    }

    /// Bytes each counted element takes, for the list kinds.
    pub const fn element_width(&self) -> usize {
        match self {
            OperandKind::RegList16 | OperandKind::TopicList8 => 1,
            OperandKind::PairList16 => 2,
            _ => 0,
        }
    }
}

/// Whether the interpreter has an implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Execution {
    Interpreted,
    /// Decoded and refused: the interpreter has no implementation (cross-VM and agent intrinsics,
    /// 32-bit conversions, `ctx_gas`).
    NotExecuted,
}

/// What the chain's verifier does with the opcode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnChain {
    Admitted,
    /// Refused on chain by rule (debug, GPU, 32-bit conversions, `ctx_gas`).
    Forbidden,
    /// Refused on chain because the engine does not execute it.
    NotExecutable,
    /// Refused where platform-dependent float arithmetic is denied (always on chain).
    FloatDenied,
    /// Admitted only where the trading extension is enabled.
    TradingOnly,
}

/// One opcode's canonical description.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OpSpec {
    pub opcode: Opcode,
    pub operands: &'static [OperandKind],
    /// Static gas, charged once per execution and summed by the verifier's bound.
    pub gas: u64,
    pub execution: Execution,
    pub on_chain: OnChain,
    /// Every trap the opcode can raise (an unimplemented one raises `EngineFault`).
    pub traps: &'static [TrapId],
    /// `path::test` holding the production engine to a reference for this opcode, for every opcode
    /// the chain admits.
    pub differential: Option<&'static str>,
}

impl OpSpec {
    /// Admitted by the chain's verifier without the trading extension.
    pub const fn on_chain_base(&self) -> bool {
        matches!(self.on_chain, OnChain::Admitted)
    }

    /// Admitted by the chain's verifier with the trading extension enabled.
    pub const fn on_chain_with_trading(&self) -> bool {
        matches!(self.on_chain, OnChain::Admitted | OnChain::TradingOnly)
    }

    /// Encoded length of the instruction when every list operand is empty.
    pub fn fixed_len(&self) -> usize {
        1 + self.operands.iter().map(OperandKind::width).sum::<usize>()
    }

    /// Encoded length of the instruction that starts at `at[0]`, reading each list operand's
    /// count from the bytes. `None` when the bytes end before the instruction does.
    pub fn encoded_len(&self, at: &[u8]) -> Option<usize> {
        let mut offset = 1usize;
        for kind in self.operands {
            let width = kind.width();
            let field = at.get(offset..offset.checked_add(width)?)?;
            offset += width;
            if kind.element_width() > 0 {
                let mut count = 0usize;
                for (i, byte) in field.iter().enumerate() {
                    count |= usize::from(*byte) << (8 * i);
                }
                offset = offset.checked_add(count.checked_mul(kind.element_width())?)?;
            }
        }
        (offset <= at.len()).then_some(offset)
    }
}

/// The canonical description of `opcode`.
pub const fn of(opcode: Opcode) -> OpSpec {
    match opcode {
        Opcode::Nop => OpSpec {
            opcode,
            operands: &[],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Jump => OpSpec {
            opcode,
            operands: &[OperandKind::Target32],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::JumpIf => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Target32],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::JumpUnless => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Target32],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Call => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Index32, OperandKind::RegList16],
            gas: 10,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Ret => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::RetVoid => OpSpec {
            opcode,
            operands: &[],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Halt => OpSpec {
            opcode,
            operands: &[],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LoadConst => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Index32],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Mov => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LoadGlobal => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Index32],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::StoreGlobal => OpSpec {
            opcode,
            operands: &[OperandKind::Index32, OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LoadIndex => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::StoreIndex => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LoadField => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Index16],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::StoreField => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Index16, OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LoadImm => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Imm8],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LoadZero => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LoadTrue => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LoadFalse => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::AddI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::SubI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::MulI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::DivI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::ModI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::NegI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Inc => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Dec => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::AddF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::FloatDenied,
            traps: &[TrapId::GasExhausted],
            differential: None,
        },
        Opcode::SubF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::FloatDenied,
            traps: &[TrapId::GasExhausted],
            differential: None,
        },
        Opcode::MulF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::FloatDenied,
            traps: &[TrapId::GasExhausted],
            differential: None,
        },
        Opcode::DivF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::FloatDenied,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::ModF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::FloatDenied,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::NegF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::EqI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::NeI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LtI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LeI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::GtI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::GeI => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::EqF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::NeF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LtF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LeF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::GtF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::GeF => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::And => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Or => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Xor => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Not => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Shl => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Shr => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::UShr => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LAnd => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LOr => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::LNot => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::I32ToI64 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::I64ToI32 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::I32ToF32 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::I64ToF64 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::F32ToI32 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::F64ToI64 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::F32ToF64 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::F64ToF32 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::ToBool => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::NewArray => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Index16],
            gas: 10,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::ArrayLen => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 2,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::ArrayPush => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::ArrayPop => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::NewTuple => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::RegList16],
            gas: 10,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::TupleGet => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Index16],
            gas: 3,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::CtxSender => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::CtxBlockHeight => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::CtxTimestamp => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::CtxValue => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::CtxGas => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::NotExecuted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::CtxChainId => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::AtomicBegin => OpSpec {
            opcode,
            operands: &[OperandKind::AtomicId],
            gas: 100,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::AtomicAborted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::AtomicCommit => OpSpec {
            opcode,
            operands: &[OperandKind::AtomicId],
            gas: 100,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::AtomicAborted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::AtomicRollback => OpSpec {
            opcode,
            operands: &[OperandKind::AtomicId],
            gas: 100,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::AtomicAborted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::AtomicCheck => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::AgentSelf => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::AgentInit => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::PairList16],
            gas: 50,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::Emit => OpSpec {
            opcode,
            operands: &[OperandKind::Index8, OperandKind::RegList16],
            gas: 20,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::EvmCall => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 2500,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::EvmStaticCall => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 700,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::EvmDelegateCall => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 700,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::EvmSload => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 200,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::EvmSstore => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 5000,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::EvmCreate => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 32000,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::EvmCreate2 => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 32000,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::EvmLog => OpSpec {
            opcode,
            operands: &[OperandKind::TopicList8, OperandKind::Reg],
            gas: 375,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::EvmBalance => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 400,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::EvmCodeSize => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 100,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::SvmInvoke => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 5000,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::SvmInvokeSigned => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 6000,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::SvmCreateAccount => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 10000,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::SvmTransfer => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 3000,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::SvmGetData => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 200,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::SvmSetData => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg],
            gas: 1000,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::SvmGetRent => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 100,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::SvmGetClock => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 100,
            execution: Execution::NotExecuted,
            on_chain: OnChain::NotExecutable,
            traps: &[TrapId::EngineFault],
            differential: None,
        },
        Opcode::GpuSha256Batch => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 500,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::GpuEd25519Verify => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 500,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::GpuPohChain => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 500,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::GpuSha256Streamed => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 750,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::GpuDeviceCount => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 10,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::GpuBenchmark => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 1000,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::GpuKeccak256Batch => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 500,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::GpuSecp256k1Verify => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Reg, OperandKind::Reg],
            gas: 600,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::TradeBegin => OpSpec {
            opcode,
            operands: &[],
            gas: 500,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::ChainMismatch, TrapId::DeadlineExceeded, TrapId::PrivateSubmissionUnavailable, TrapId::MissingCapability, TrapId::CapabilityVersion],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeBorrow => OpSpec {
            opcode,
            operands: &[OperandKind::TradeOp],
            gas: 2000,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::MissingCapability, TrapId::HostRefused, TrapId::FlashFeeExceeded, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeSwap => OpSpec {
            opcode,
            operands: &[OperandKind::TradeOp],
            gas: 3000,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::MissingCapability, TrapId::HostRefused, TrapId::InsufficientBalance, TrapId::OracleMissing, TrapId::OracleStale, TrapId::OracleDivergent, TrapId::OracleDeviation, TrapId::MinOutput, TrapId::Slippage, TrapId::PriceImpact, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeRepay => OpSpec {
            opcode,
            operands: &[OperandKind::TradeOp],
            gas: 2000,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::InsufficientBalance, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeChargeCosts => OpSpec {
            opcode,
            operands: &[OperandKind::TradeOp],
            gas: 500,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::CostKindForbidden, TrapId::CostCeiling, TrapId::InsufficientBalance, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeAssertProfit => OpSpec {
            opcode,
            operands: &[OperandKind::TradeOp],
            gas: 200,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::ProfitBelowFloor, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeAssertDebtsClosed => OpSpec {
            opcode,
            operands: &[OperandKind::TradeOp],
            gas: 100,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::DebtOutstanding],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeAssertSolvent => OpSpec {
            opcode,
            operands: &[OperandKind::TradeOp],
            gas: 200,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::Insolvent, TrapId::ArithmeticOverflow],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeCommit => OpSpec {
            opcode,
            operands: &[],
            gas: 500,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::DebtOutstanding],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::TradeLoadProfit => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::TradingOnly,
            traps: &[TrapId::GasExhausted, TrapId::SequenceViolation, TrapId::InvalidConversion],
            differential: Some("crates/x3-integration/tests/x3lang_trading_differential.rs::production_and_reference_agree_on_the_flagship_trade"),
        },
        Opcode::DebugPrint => OpSpec {
            opcode,
            operands: &[OperandKind::Reg],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::Breakpoint => OpSpec {
            opcode,
            operands: &[],
            gas: 1,
            execution: Execution::Interpreted,
            on_chain: OnChain::Forbidden,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: None,
        },
        Opcode::Assert => OpSpec {
            opcode,
            operands: &[OperandKind::Reg, OperandKind::Index32],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
        Opcode::Panic => OpSpec {
            opcode,
            operands: &[OperandKind::Index32],
            gas: 5,
            execution: Execution::Interpreted,
            on_chain: OnChain::Admitted,
            traps: &[TrapId::GasExhausted, TrapId::EngineFault],
            differential: Some("crates/x3-integration/tests/opcode_differential.rs::every_on_chain_opcode_agrees_with_the_reference_engine"),
        },
    }
}

/// Number of opcodes in the instruction set.
pub const COUNT: usize = 124;

/// Every opcode's description, in opcode order.
pub static ALL: [OpSpec; COUNT] = [
    of(Opcode::Nop),
    of(Opcode::Jump),
    of(Opcode::JumpIf),
    of(Opcode::JumpUnless),
    of(Opcode::Call),
    of(Opcode::Ret),
    of(Opcode::RetVoid),
    of(Opcode::Halt),
    of(Opcode::LoadConst),
    of(Opcode::Mov),
    of(Opcode::LoadGlobal),
    of(Opcode::StoreGlobal),
    of(Opcode::LoadIndex),
    of(Opcode::StoreIndex),
    of(Opcode::LoadField),
    of(Opcode::StoreField),
    of(Opcode::LoadImm),
    of(Opcode::LoadZero),
    of(Opcode::LoadTrue),
    of(Opcode::LoadFalse),
    of(Opcode::AddI),
    of(Opcode::SubI),
    of(Opcode::MulI),
    of(Opcode::DivI),
    of(Opcode::ModI),
    of(Opcode::NegI),
    of(Opcode::Inc),
    of(Opcode::Dec),
    of(Opcode::AddF),
    of(Opcode::SubF),
    of(Opcode::MulF),
    of(Opcode::DivF),
    of(Opcode::ModF),
    of(Opcode::NegF),
    of(Opcode::EqI),
    of(Opcode::NeI),
    of(Opcode::LtI),
    of(Opcode::LeI),
    of(Opcode::GtI),
    of(Opcode::GeI),
    of(Opcode::EqF),
    of(Opcode::NeF),
    of(Opcode::LtF),
    of(Opcode::LeF),
    of(Opcode::GtF),
    of(Opcode::GeF),
    of(Opcode::And),
    of(Opcode::Or),
    of(Opcode::Xor),
    of(Opcode::Not),
    of(Opcode::Shl),
    of(Opcode::Shr),
    of(Opcode::UShr),
    of(Opcode::LAnd),
    of(Opcode::LOr),
    of(Opcode::LNot),
    of(Opcode::I32ToI64),
    of(Opcode::I64ToI32),
    of(Opcode::I32ToF32),
    of(Opcode::I64ToF64),
    of(Opcode::F32ToI32),
    of(Opcode::F64ToI64),
    of(Opcode::F32ToF64),
    of(Opcode::F64ToF32),
    of(Opcode::ToBool),
    of(Opcode::NewArray),
    of(Opcode::ArrayLen),
    of(Opcode::ArrayPush),
    of(Opcode::ArrayPop),
    of(Opcode::NewTuple),
    of(Opcode::TupleGet),
    of(Opcode::CtxSender),
    of(Opcode::CtxBlockHeight),
    of(Opcode::CtxTimestamp),
    of(Opcode::CtxValue),
    of(Opcode::CtxGas),
    of(Opcode::CtxChainId),
    of(Opcode::AtomicBegin),
    of(Opcode::AtomicCommit),
    of(Opcode::AtomicRollback),
    of(Opcode::AtomicCheck),
    of(Opcode::AgentSelf),
    of(Opcode::AgentInit),
    of(Opcode::Emit),
    of(Opcode::EvmCall),
    of(Opcode::EvmStaticCall),
    of(Opcode::EvmDelegateCall),
    of(Opcode::EvmSload),
    of(Opcode::EvmSstore),
    of(Opcode::EvmCreate),
    of(Opcode::EvmCreate2),
    of(Opcode::EvmLog),
    of(Opcode::EvmBalance),
    of(Opcode::EvmCodeSize),
    of(Opcode::SvmInvoke),
    of(Opcode::SvmInvokeSigned),
    of(Opcode::SvmCreateAccount),
    of(Opcode::SvmTransfer),
    of(Opcode::SvmGetData),
    of(Opcode::SvmSetData),
    of(Opcode::SvmGetRent),
    of(Opcode::SvmGetClock),
    of(Opcode::GpuSha256Batch),
    of(Opcode::GpuEd25519Verify),
    of(Opcode::GpuPohChain),
    of(Opcode::GpuSha256Streamed),
    of(Opcode::GpuDeviceCount),
    of(Opcode::GpuBenchmark),
    of(Opcode::GpuKeccak256Batch),
    of(Opcode::GpuSecp256k1Verify),
    of(Opcode::TradeBegin),
    of(Opcode::TradeBorrow),
    of(Opcode::TradeSwap),
    of(Opcode::TradeRepay),
    of(Opcode::TradeChargeCosts),
    of(Opcode::TradeAssertProfit),
    of(Opcode::TradeAssertDebtsClosed),
    of(Opcode::TradeAssertSolvent),
    of(Opcode::TradeCommit),
    of(Opcode::TradeLoadProfit),
    of(Opcode::DebugPrint),
    of(Opcode::Breakpoint),
    of(Opcode::Assert),
    of(Opcode::Panic),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoded_len_reads_list_counts_and_refuses_truncation() {
        for entry in ALL.iter() {
            let mut bytes = alloc::vec![entry.opcode as u8];
            bytes.resize(entry.fixed_len(), 0);
            assert_eq!(
                entry.encoded_len(&bytes),
                Some(entry.fixed_len()),
                "{:?}",
                entry.opcode
            );
            if entry.fixed_len() > 1 {
                let truncated = &bytes[..bytes.len() - 1];
                assert_eq!(entry.encoded_len(truncated), None, "{:?}", entry.opcode);
            }
        }
        // `Call dst, func(u32), args(u16 count)` with three argument registers.
        let call = of(Opcode::Call);
        let mut bytes = alloc::vec![Opcode::Call as u8, 0, 1, 0, 0, 0, 3, 0, 4, 5, 6];
        assert_eq!(call.encoded_len(&bytes), Some(bytes.len()));
        bytes.pop();
        assert_eq!(
            call.encoded_len(&bytes),
            None,
            "a missing argument register"
        );
    }

    #[test]
    fn the_table_is_in_opcode_order_and_complete() {
        let mut previous = None;
        for entry in ALL.iter() {
            let byte = entry.opcode as u8;
            assert_eq!(Opcode::from_byte(byte), Some(entry.opcode));
            if let Some(previous) = previous {
                assert!(byte > previous, "ordered and unique");
            }
            previous = Some(byte);
        }
        let decodable = (0u8..=255)
            .filter(|byte| Opcode::from_byte(*byte).is_some())
            .count();
        assert_eq!(decodable, COUNT);
    }

    #[test]
    fn every_opcode_costs_gas_and_names_its_traps() {
        for entry in ALL.iter() {
            assert!(entry.gas > 0, "{:?}", entry.opcode);
            assert!(!entry.traps.is_empty(), "{:?}", entry.opcode);
        }
    }

    #[test]
    fn trading_opcodes_are_trading_only_and_the_rest_are_not() {
        for entry in ALL.iter() {
            assert_eq!(
                entry.opcode.is_trading_op(),
                entry.on_chain == OnChain::TradingOnly,
                "{:?}",
                entry.opcode
            );
            if entry.opcode.is_trading_op() {
                let has_index = entry.operands == [OperandKind::TradeOp];
                assert_eq!(has_index, entry.opcode.trading_op_index_operand());
            }
        }
    }

    #[test]
    fn every_admitted_opcode_names_differential_coverage() {
        for entry in ALL.iter() {
            assert_eq!(
                entry.differential.is_some(),
                entry.on_chain_with_trading(),
                "{:?}",
                entry.opcode
            );
        }
    }
}
