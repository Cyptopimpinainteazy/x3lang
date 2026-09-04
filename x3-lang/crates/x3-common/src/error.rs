//! Error types for the X3 compiler.

use crate::span::Span;
use thiserror::Error;

/// The main error type for X3 compilation.
#[derive(Error, Debug, Clone)]
pub enum X3Error {
    #[error("Lexer error: {message}")]
    LexerError { message: String, span: Span },

    #[error("Parser error: {message}")]
    ParseError {
        message: String,
        span: Span,
        expected: Vec<String>,
        found: String,
    },

    #[error("Type error: {message}")]
    TypeError { message: String, span: Span },

    #[error("Name resolution error: {message}")]
    NameError {
        message: String,
        span: Span,
        suggestions: Vec<String>,
    },

    #[error("Semantic error: {message}")]
    SemanticError { message: String, span: Span },

    #[error("Codegen error: {message}")]
    CodegenError { message: String, span: Option<Span> },

    #[error("IO error: {message}")]
    IoError { message: String, path: Option<String> },

    #[error("Internal compiler error: {message}")]
    InternalError { message: String },

    // X3-specific errors
    #[error("Agent error: {message}")]
    AgentError {
        message: String,
        span: Span,
        agent_name: String,
    },

    #[error("Atomic block error: {message}")]
    AtomicError { message: String, span: Span },

    #[error("Cross-chain error: {message}")]
    CrossChainError {
        message: String,
        span: Span,
        source_chain: String,
        target_chain: String,
    },

    #[error("MEV operation error: {message}")]
    MevError {
        message: String,
        span: Span,
        operation: String,
    },
}

impl X3Error {
    pub fn span(&self) -> Option<Span> {
        match self {
            X3Error::LexerError { span, .. } => Some(*span),
            X3Error::ParseError { span, .. } => Some(*span),
            X3Error::TypeError { span, .. } => Some(*span),
            X3Error::NameError { span, .. } => Some(*span),
            X3Error::SemanticError { span, .. } => Some(*span),
            X3Error::CodegenError { span, .. } => *span,
            X3Error::IoError { .. } => None,
            X3Error::InternalError { .. } => None,
            X3Error::AgentError { span, .. } => Some(*span),
            X3Error::AtomicError { span, .. } => Some(*span),
            X3Error::CrossChainError { span, .. } => Some(*span),
            X3Error::MevError { span, .. } => Some(*span),
        }
    }

    pub fn with_span(self, new_span: Span) -> Self {
        match self {
            X3Error::LexerError { message, .. } => X3Error::LexerError {
                message,
                span: new_span,
            },
            X3Error::ParseError {
                message,
                expected,
                found,
                ..
            } => X3Error::ParseError {
                message,
                span: new_span,
                expected,
                found,
            },
            X3Error::TypeError { message, .. } => X3Error::TypeError {
                message,
                span: new_span,
            },
            other => other,
        }
    }
}

/// Result type using X3Error.
pub type X3Result<T> = Result<T, X3Error>;

impl From<std::io::Error> for X3Error {
    fn from(value: std::io::Error) -> Self {
        X3Error::IoError {
            message: value.to_string(),
            path: None,
        }
    }
}

/// A collection of errors that can be accumulated during compilation.
#[derive(Debug, Default)]
pub struct ErrorAccumulator {
    errors: Vec<X3Error>,
    warnings: Vec<X3Error>,
    max_errors: usize,
}

impl ErrorAccumulator {
    pub fn new() -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            max_errors: 100,
        }
    }

    pub fn with_max_errors(max: usize) -> Self {
        Self {
            errors: Vec::new(),
            warnings: Vec::new(),
            max_errors: max,
        }
    }

    pub fn add_error(&mut self, error: X3Error) {
        if self.errors.len() < self.max_errors {
            self.errors.push(error);
        }
    }

    pub fn add_warning(&mut self, warning: X3Error) {
        self.warnings.push(warning);
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn error_count(&self) -> usize {
        self.errors.len()
    }

    pub fn warning_count(&self) -> usize {
        self.warnings.len()
    }

    pub fn errors(&self) -> &[X3Error] {
        &self.errors
    }

    pub fn warnings(&self) -> &[X3Error] {
        &self.warnings
    }

    pub fn take_errors(self) -> Vec<X3Error> {
        self.errors
    }

    pub fn into_result<T>(self, value: T) -> X3Result<T> {
        if self.has_errors() {
            Err(self
                .errors
                .into_iter()
                .next()
                .expect("has_errors was true so errors is non-empty"))
        } else {
            Ok(value)
        }
    }

    pub fn merge(&mut self, other: ErrorAccumulator) {
        self.errors.extend(other.errors);
        self.warnings.extend(other.warnings);
    }
}

/// A trait for types that can accumulate errors.
pub trait ErrorReporter {
    fn report_error(&mut self, error: X3Error);
    fn report_warning(&mut self, warning: X3Error);
}

impl ErrorReporter for ErrorAccumulator {
    fn report_error(&mut self, error: X3Error) {
        self.add_error(error);
    }

    fn report_warning(&mut self, warning: X3Error) {
        self.add_warning(warning);
    }
}
