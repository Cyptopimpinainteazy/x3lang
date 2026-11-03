//! X3 Lexer - Lexical analysis for the X3 programming language
//!
//! This module provides tokenization of X3 source code, converting raw text into
//! a stream of tokens that can be consumed by the parser. The lexer is implemented
//! using the `logos` crate for high-performance lexing.
//!
//! # Token Categories
//!
//! - **Keywords**: `fn`, `let`, `agent`, `atomic`, `bundle`, etc.
//! - **Identifiers**: Variable and function names
//! - **Literals**: Numbers, strings, addresses, hashes
//! - **Operators**: Arithmetic, logical, comparison
//! - **Delimiters**: Brackets, braces, punctuation
//! - **Comments**: Line and block comments
//!
//! # Example
//!
//! ```
//! use x3_lang_lexer::Lexer;
//!
//! let source = "fn main() { let x = 42; }";
//! let mut lexer = Lexer::new(source, 0);
//!
//! for token in lexer {
//!     println!("{:?}", token);
//! }
//! ```

pub mod cursor;
#[cfg(feature = "logos")]
pub mod lexer;
pub mod token;

pub use cursor::Cursor;
#[cfg(feature = "logos")]
pub use lexer::Lexer;

#[cfg(not(feature = "logos"))]
#[derive(Debug, Clone, Copy)]
pub struct Lexer<'a> {
    _src: &'a str,
}

#[cfg(not(feature = "logos"))]
impl<'a> Lexer<'a> {
    pub fn new(source: &'a str, _file_id: u32) -> Self {
        Self { _src: source }
    }
}

#[cfg(not(feature = "logos"))]
impl<'a> Iterator for Lexer<'a> {
    type Item = ();
    fn next(&mut self) -> Option<Self::Item> {
        None
    }
}

pub use token::{BinOp, Delimiter, Keyword, Literal, Token, TokenKind, UnOp};
