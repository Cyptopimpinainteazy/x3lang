//! X3 AST definitions
//!
//! This crate defines the full AST for the X3 language. AST nodes are fully deterministic
//! and use `x3-common::Span` for precise source locations. The AST is intentionally
//! immutable by design and suitable for serialisation, analysis, and lowering.

pub mod ast;
pub mod visitor;

pub use ast::*;
pub use visitor::{AstVisitor, WalkResult};
