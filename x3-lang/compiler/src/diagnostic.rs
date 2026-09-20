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
    /// An amount's asset is not the asset the operation requires.
    ///
    /// The super-prompt's `X3E-2107 ASSET_TYPE_MISMATCH`, in the four-digit
    /// spelling this catalogue already uses: economic code needs economic
    /// diagnostics, and a message alone cannot be keyed on by tooling (PHASE 52,
    /// TICKET-021).
    AssetTypeMismatch,
    /// A declared economic effect or guarantee that nothing in the body discharges.
    ///
    /// The super-prompt's `X3E-4021` "unresolved economic effect". This is the code
    /// a strategy module's effects/guarantees check reports under, so a build system
    /// can tell "you declared work you do not do" from every other semantic error.
    UnresolvedEconomicEffect,
    /// A debt's lifecycle broken: borrowed twice, repaid twice, repaid when nothing is open,
    /// or left open where the trade can succeed without repaying it.
    ///
    /// The trading path's `debt` class, which the ticket that asked for these codes named
    /// (PHASE 52, TICKET-021).
    DebtLifecycle,
    /// A trading operation in an order the program cannot execute: a statement on the source
    /// chain after its proceeds have bridged away, a second bridge, a bridge that does not
    /// move between two chains.
    ///
    /// The trading path's `sequence` class.
    TradingSequence,
    /// A declared risk policy whose own numbers are outside what the language can honour — a
    /// bound above the basis-point ceiling, a zero deadline, private submission with no
    /// capability attested to deliver it.
    RiskPolicyBound,
    /// A trading declaration that is incomplete or contradictory: a policy it does not
    /// declare, an invariant declared twice, a borrow with no `all_debts_repaid`, no receipt
    /// and no net-profit guard.
    TradeDeclaration,
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
            Self::AssetTypeMismatch => "X3E2107",
            Self::UnresolvedEconomicEffect => "X3E4021",
            Self::DebtLifecycle => "X3E4022",
            Self::TradingSequence => "X3E4023",
            Self::RiskPolicyBound => "X3E4024",
            Self::TradeDeclaration => "X3E4025",
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
    /// This diagnostic as the error the compiler's accumulators carry, with its
    /// stable code in front of the message.
    ///
    /// One renderer, used by the IR verifier's conversion and by every check that
    /// wants a code: two spellings of "code then message" is two things for tooling
    /// to parse, which is the defect the catalogue exists to prevent.
    pub fn into_error(self) -> x3_lang_common::X3Error {
        x3_lang_common::X3Error::SemanticError {
            message: format!("{}: {}", self.code.as_str(), self.message),
            span: self.primary_span,
        }
    }

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
