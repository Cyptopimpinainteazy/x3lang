//! Errors from reading or building an X3BC container.
//!
//! `x3-backend` converts these into its `BackendError` (same kinds, same messages), so code that
//! handled a format error before the move handles it unchanged.

use alloc::string::String;
use core::fmt;

use crate::codec::CodecError;

/// Why an X3BC container could not be read or built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FormatError {
    pub kind: FormatErrorKind,
}

impl FormatError {
    pub fn without_span(kind: FormatErrorKind) -> Self {
        Self { kind }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FormatErrorKind {
    UnexpectedEof,
    InvalidMagic,
    UnsupportedVersion(u32),
    ChecksumMismatch {
        expected: u32,
        found: u32,
    },
    CorruptedBytecode {
        offset: usize,
    },
    ConstPoolOverflow {
        max: u32,
    },
    StringTooLong {
        len: usize,
        max: usize,
    },
    /// The trading section is malformed, missing where the header promises it, or present where
    /// the header does not.
    TradingSection {
        offset: usize,
        reason: String,
    },
}

pub type FormatResult<T> = Result<T, FormatError>;

impl fmt::Display for FormatErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatErrorKind::UnexpectedEof => write!(f, "unexpected end of bytecode"),
            FormatErrorKind::InvalidMagic => write!(f, "invalid bytecode magic"),
            FormatErrorKind::UnsupportedVersion(v) => write!(f, "unsupported bytecode version {v}"),
            FormatErrorKind::ChecksumMismatch { expected, found } => {
                write!(
                    f,
                    "checksum mismatch: expected {expected:#x}, found {found:#x}"
                )
            }
            FormatErrorKind::CorruptedBytecode { offset } => {
                write!(f, "corrupted bytecode at offset {offset}")
            }
            FormatErrorKind::ConstPoolOverflow { max } => {
                write!(f, "constant pool overflow: maximum {max} entries")
            }
            FormatErrorKind::StringTooLong { len, max } => {
                write!(f, "string constant too long: {len} bytes (max {max})")
            }
            FormatErrorKind::TradingSection { offset, reason } => {
                write!(f, "invalid trading section at offset {offset}: {reason}")
            }
        }
    }
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl core::error::Error for FormatError {}

impl From<CodecError> for FormatErrorKind {
    fn from(error: CodecError) -> Self {
        FormatErrorKind::TradingSection {
            offset: 0,
            reason: alloc::format!("{error:?}"),
        }
    }
}
