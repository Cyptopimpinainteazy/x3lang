//! Token helper types shared across lexer, AST, parser, and codegen.
//!
//! These types encode operator and literal metadata that should be shared consistently
//! between components to ensure deterministic interpretations and binary compatibility.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Binary operators used across lexer and AST.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum BinOp {
    // Arithmetic
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Power,

    // Bitwise
    And,
    Or,
    Xor,
    Shl,
    Shr,

    // Logical
    AndAnd,
    OrOr,

    // Comparison
    EqEq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,

    // Special X3 operators
    Pipe,
    Compose,
    NullCoal,
}

impl fmt::Display for BinOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use BinOp::*;
        match self {
            Plus => write!(f, "+"),
            Minus => write!(f, "-"),
            Star => write!(f, "*"),
            Slash => write!(f, "/"),
            Percent => write!(f, "%"),
            Power => write!(f, "**"),
            And => write!(f, "&"),
            Or => write!(f, "|"),
            Xor => write!(f, "^"),
            Shl => write!(f, "<<"),
            Shr => write!(f, ">>"),
            AndAnd => write!(f, "&&"),
            OrOr => write!(f, "||"),
            EqEq => write!(f, "=="),
            Ne => write!(f, "!="),
            Lt => write!(f, "<"),
            Le => write!(f, "<="),
            Gt => write!(f, ">"),
            Ge => write!(f, ">="),
            Pipe => write!(f, "|>"),
            Compose => write!(f, ">>"),
            NullCoal => write!(f, "??"),
        }
    }
}

impl BinOp {
    pub fn precedence(&self) -> u8 {
        use BinOp::*;
        match self {
            OrOr => 1,
            AndAnd => 2,
            Or => 3,
            Xor => 4,
            And => 5,
            EqEq | Ne => 6,
            Lt | Le | Gt | Ge => 7,
            NullCoal => 8,
            Pipe | Compose => 9,
            Shl | Shr => 10,
            Plus | Minus => 11,
            Star | Slash | Percent => 12,
            Power => 13,
        }
    }

    pub fn is_right_assoc(&self) -> bool {
        matches!(self, BinOp::Power)
    }
}

/// Unary operators
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum UnOp {
    Not,
    Neg,
    Deref,
    Ref,
    RefMut,
}

impl fmt::Display for UnOp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UnOp::Not => write!(f, "!"),
            UnOp::Neg => write!(f, "-"),
            UnOp::Deref => write!(f, "*"),
            UnOp::Ref => write!(f, "&"),
            UnOp::RefMut => write!(f, "&mut"),
        }
    }
}

/// Integer literal base
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum IntBase {
    Decimal,
    Hex,
    Octal,
    Binary,
}

/// Integer suffix types
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum IntSuffix {
    U8,
    U16,
    U32,
    U64,
    U128,
    U256,
    Usize,
    I8,
    I16,
    I32,
    I64,
    I128,
    I256,
    Isize,
}

/// Float suffix
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum FloatSuffix {
    F32,
    F64,
}

impl fmt::Display for FloatSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FloatSuffix::F32 => write!(f, "f32"),
            FloatSuffix::F64 => write!(f, "f64"),
        }
    }
}

/// Duration unit
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum DurationUnit {
    Nanoseconds,
    Microseconds,
    Milliseconds,
    Seconds,
    Minutes,
    Hours,
    Days,
}

/// Size unit
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
pub enum SizeUnit {
    Bytes,
    Kilobytes,
    Megabytes,
    Gigabytes,
    Terabytes,
}
