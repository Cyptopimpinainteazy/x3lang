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
    ///
    /// **This rendering is for `Error`-severity diagnostics only.** `X3Error` has no
    /// severity and no secondary spans, so a warning put through here arrives at the
    /// accumulator as an error *message* and its caller then chooses the vector by
    /// hand — which is exactly the drift severity-as-a-field exists to prevent, and
    /// the reason this was a trap rather than a defect while there was no
    /// `CompilerDiagnostic::warning` to build (TICKET-104). A diagnostic that has a
    /// severity to carry goes through
    /// `Diagnostic::from(CompilerDiagnostic)` instead, which keeps the level, the code
    /// and every span; the `debug_assert` below is what makes the difference loud in
    /// a test build the moment a non-error reaches this path.
    pub fn into_error(self) -> x3_lang_common::X3Error {
        debug_assert_eq!(
            self.severity,
            DiagnosticSeverity::Error,
            "`into_error` cannot carry the severity of a `{:?}` diagnostic: use \
             `Diagnostic::from(diagnostic)`, which keeps the level and the secondary spans",
            self.severity
        );
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

    /// Construct a warning diagnostic — a coded finding that does not stop the build.
    ///
    /// It exists so that the three severities the accumulator and `DiagnosticLevel` already
    /// distinguish can be *built*, not only *rendered*: before this, every coded diagnostic the
    /// trading and objective paths produced came from `error`, so the lossy `into_error` had no
    /// caller that would suffer from it (TICKET-104). A warning is produced only where the
    /// program is correct and the finding is about something else — a construct nothing reads,
    /// a declaration that is wider than the work — and its consumer is
    /// `Diagnostic::from(diagnostic)`, never `into_error`.
    pub fn warning(code: DiagnosticCode, message: impl Into<String>, primary_span: Span) -> Self {
        Self {
            code,
            severity: DiagnosticSeverity::Warning,
            message: message.into(),
            primary_span,
            secondary_spans: Vec::new(),
            help: None,
        }
    }

    /// This diagnostic as the entry the accumulator's `warnings` vector carries.
    ///
    /// The accumulator's two vectors *are* the severity channel — `VerifyOutcome.errors` and
    /// `VerifyOutcome.warnings` are both `Vec<X3Error>`, which has no level — so a warning filed
    /// here arrives as a warning, and this renderer asserts the severity it is for rather than
    /// trusting its caller: filing an error as a warning would make a rejected program read as a
    /// clean one, which is the more dangerous direction of the same mistake `into_error` guards
    /// against (TICKET-104).
    pub fn into_warning(self) -> x3_lang_common::X3Error {
        debug_assert_ne!(
            self.severity,
            DiagnosticSeverity::Error,
            "`into_warning` is the warnings vector's renderer and this diagnostic is an error: file \
             it with `into_error`"
        );
        x3_lang_common::X3Error::SemanticError {
            message: format!("{}: {}", self.code.as_str(), self.message),
            span: self.primary_span,
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

/// A coded diagnostic as the richer diagnostic the accumulators and renderers take.
///
/// `X3Error` — what [`CompilerDiagnostic::into_error`] produces — has a message and one span. This
/// conversion keeps everything the diagnostic carries instead of two of its six fields: the
/// **severity**, so a caller does not re-decide which vector a finding belongs in, and the
/// **secondary spans**, as secondary labels, so the context a diagnostic was built with reaches
/// whoever has to act on it. `X3Error` cannot carry either, which is why the two conversions exist
/// side by side rather than one replacing the other: a caller that only has an `X3Error` to return
/// uses `into_error`, and a caller that is reporting a diagnostic uses this (TICKET-104).
///
/// The code is a string here rather than [`DiagnosticCode`] because `Diagnostic::code` is
/// `Option<String>` and the catalogue's rendering (`X3E0501`) is what tooling keys on; the
/// catalogue's own spelling is the one place that decides it.
impl From<CompilerDiagnostic> for x3_lang_common::diagnostic::Diagnostic {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        use x3_lang_common::diagnostic::{Diagnostic, DiagnosticLabel, DiagnosticLevel};

        let level = match diagnostic.severity {
            DiagnosticSeverity::Error => DiagnosticLevel::Error,
            DiagnosticSeverity::Warning => DiagnosticLevel::Warning,
            DiagnosticSeverity::Note => DiagnosticLevel::Note,
        };
        let mut labels = vec![DiagnosticLabel::primary_no_message(diagnostic.primary_span)];
        labels.extend(diagnostic.secondary_spans.iter().map(|span| DiagnosticLabel {
            span: *span,
            message: None,
            is_primary: false,
        }));
        Diagnostic {
            level,
            code: Some(diagnostic.code.as_str().to_string()),
            message: diagnostic.message,
            labels,
            notes: Vec::new(),
            help: diagnostic.help.into_iter().collect(),
        }
    }
}
