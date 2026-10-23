//! The canonical X3BC semantic core.
//!
//! One definition of the bytecode the X3 chain executes, shared by everything that reads or writes
//! it: the X3BC header and version rules ([`bytecode`]), the opcode table ([`opcode`]) and its
//! exhaustive description — operand widths, gas, on-chain admission, traps, differential coverage
//! ([`spec`]), the container format ([`format`]), the trading extension ([`trading`]), the
//! constant-product pricing venues settle with ([`amm`]), receipt-visible traps ([`trap`]) and the
//! trading receipt ([`receipt`]).
//!
//! `no_std`, and free of Substrate on purpose: the chain's engine (`crates/x3-vm`) and the
//! canonical compiler (`x3-lang/compiler`, a separate workspace) both depend on it, and neither
//! may carry its own copy (docs/roadmap/X3LANG_PRODUCTION_UNIFICATION.md).

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub(crate) mod prelude {
    pub use alloc::{
        format,
        string::{String, ToString},
        vec,
        vec::Vec,
    };
}

pub mod amm;
pub mod bytecode;
pub mod codec;
pub mod economics;
pub mod error;
pub mod format;
pub mod format_helpers;
pub mod hash;
pub mod opcode;
pub mod receipt;
pub mod spec;
pub mod state;
pub mod trading;
pub mod trap;

pub use error::{FormatError, FormatErrorKind, FormatResult};
pub use format::BytecodeModule;
pub use opcode::Opcode;
