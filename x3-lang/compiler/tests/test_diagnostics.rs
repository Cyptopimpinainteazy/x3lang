use x3_lang_common::Span;
use x3_lang_compiler::diagnostic::{CompilerDiagnostic, DiagnosticCode, DiagnosticSeverity};

#[test]
fn diagnostic_codes_are_stable_x3lang_identifiers() {
    assert_eq!(DiagnosticCode::UnexpectedToken.as_str(), "X3E0001");
    assert_eq!(DiagnosticCode::UndefinedSymbol.as_str(), "X3E0101");
    assert_eq!(DiagnosticCode::IncompatibleTypes.as_str(), "X3E0201");
    assert_eq!(DiagnosticCode::ArgumentTypeMismatch.as_str(), "X3E0202");
    assert_eq!(DiagnosticCode::InvalidNumericCoercion.as_str(), "X3E0301");
    assert_eq!(DiagnosticCode::InvalidCrossChainRoute.as_str(), "X3E0401");
    assert_eq!(DiagnosticCode::UnsafeIr.as_str(), "X3E0501");
}

#[test]
fn diagnostic_preserves_primary_span_and_severity() {
    let span = Span::from_range(7..14, 3);
    let diagnostic = CompilerDiagnostic::error(DiagnosticCode::UndefinedSymbol, "undefined symbol `missing`", span);

    assert_eq!(diagnostic.code, DiagnosticCode::UndefinedSymbol);
    assert_eq!(diagnostic.severity, DiagnosticSeverity::Error);
    assert_eq!(diagnostic.primary_span, span);
    assert_eq!(diagnostic.message, "undefined symbol `missing`");
    assert!(diagnostic.secondary_spans.is_empty());
    assert_eq!(diagnostic.help, None);
}

#[test]
fn diagnostic_builder_methods_preserve_secondary_context() {
    let primary = Span::from_range(1..4, 0);
    let secondary = Span::from_range(10..13, 0);
    let diagnostic = CompilerDiagnostic::error(
        DiagnosticCode::ArgumentTypeMismatch,
        "argument type does not match parameter",
        primary,
    )
    .with_secondary_span(secondary)
    .with_help("use an explicit conversion once conversion syntax is supported");

    assert_eq!(diagnostic.secondary_spans, vec![secondary]);
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("use an explicit conversion once conversion syntax is supported")
    );
}
