//! Stable compiler-facing diagnostics for X3Lang.
//!
//! Human-readable wording may evolve, but these codes are part of the
//! X3Lang 1.x tooling contract and should remain semantically stable.

use x3_lang_common::Span;

/// Stable machine-readable diagnostic identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticCode {
    UnexpectedToken,
    UndefinedSymbol,
    IncompatibleTypes,
    ArgumentTypeMismatch,
    InvalidNumericCoercion,
    InvalidCrossChainRoute,
    UnsafeIr,
}

impl DiagnosticCode {
    /// Return the stable X3Lang diagnostic identifier.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnexpectedToken => "X3E0001",
            Self::UndefinedSymbol => "X3E0101",
            Self::IncompatibleTypes => "X3E0201",
            Self::ArgumentTypeMismatch => "X3E0202",
            Self::InvalidNumericCoercion => "X3E0301",
            Self::InvalidCrossChainRoute => "X3E0401",
            Self::UnsafeIr => "X3E0501",
        }
    }
}

/// Severity for a compiler diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Note,
}

/// Stable compiler-facing diagnostic representation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilerDiagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub primary_span: Span,
    pub secondary_spans: Vec<Span>,
    pub help: Option<String>,
}

impl CompilerDiagnostic {
    /// Construct an error diagnostic with a required primary source span.
    pub fn error(code: DiagnosticCode, message: impl Into<String>, primary_span: Span) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Error,
            message: message.into(),
            primary_span,
            secondary_spans: Vec::new(),
            help: None,
        }
    }

    /// Attach an additional source span that provides context for the error.
    pub fn with_secondary_span(mut self, span: Span) -> Self {
        self.secondary_spans.push(span);
        self
    }

    /// Attach optional remediation text.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
}
