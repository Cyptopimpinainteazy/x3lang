//! A coded diagnostic reaches its consumer with the severity and the spans it was built with
//! (TICKET-104).
//!
//! `CompilerDiagnostic` carries six fields and `into_error` keeps two — the code in the message and
//! the primary span — because `X3Error` has no severity and no secondary spans. That was faithful
//! while every coded diagnostic came from `CompilerDiagnostic::error`, and it was a trap the moment
//! one did not: the rendering would say "error" for a warning and the caller would choose the
//! accumulator's vector by hand. There was no `warning` constructor to build, so nothing suffered —
//! which is exactly why the loss needed closing before something did.
//!
//! Two conversions exist now, side by side, and these tests hold the difference: `Diagnostic::from`
//! keeps the level, the code and every span; `into_error` is the error-only path and refuses a
//! severity it cannot carry, loudly, in a test build.

use x3_lang_common::diagnostic::DiagnosticLevel;
use x3_lang_common::span::{BytePos, Span};
use x3_lang_compiler::diagnostic::{CompilerDiagnostic, DiagnosticCode};
use x3_lang_compiler::semantic::VerifyOutcome;

/// A span that is distinguishable from another, so "the secondary span arrived" cannot pass by
/// accident when both are `0..0`.
fn span(start: u32, end: u32) -> Span {
    Span {
        start: BytePos::new(start),
        end: BytePos::new(end),
        file_id: 0,
    }
}

#[test]
fn a_warning_severity_diagnostic_reaches_its_consumer_as_a_warning() {
    let diagnostic = CompilerDiagnostic::warning(
        DiagnosticCode::UnsafeIr,
        "this declaration is wider than the work it guards",
        span(10, 20),
    )
    .with_secondary_span(span(30, 40));

    let rendered: x3_lang_common::diagnostic::Diagnostic = diagnostic.into();
    assert_eq!(
        rendered.level,
        DiagnosticLevel::Warning,
        "a warning must arrive as a warning: the level is the field the accumulator's two vectors \
         are keyed on"
    );
    assert_eq!(rendered.code.as_deref(), Some("X3E0501"), "and keep its catalogue code");
    assert_eq!(rendered.message, "this declaration is wider than the work it guards");
}

#[test]
fn an_error_severity_diagnostic_reaches_its_consumer_as_an_error() {
    let diagnostic = CompilerDiagnostic::error(DiagnosticCode::UnsafeIr, "no", span(0, 1));
    let rendered: x3_lang_common::diagnostic::Diagnostic = diagnostic.into();
    assert_eq!(rendered.level, DiagnosticLevel::Error);
    assert_eq!(rendered.code.as_deref(), Some("X3E0501"));
}

#[test]
fn the_secondary_spans_and_the_help_arrive_with_the_diagnostic() {
    // The other half of what `into_error` drops. Before, the accumulator could only ever see the
    // primary span, so a diagnostic built with context arrived without it.
    let diagnostic = CompilerDiagnostic::error(DiagnosticCode::UnsafeIr, "no", span(5, 7))
        .with_secondary_span(span(11, 13))
        .with_secondary_span(span(17, 19))
        .with_help("declare the guard where the work is");

    let rendered: x3_lang_common::diagnostic::Diagnostic = diagnostic.into();
    assert_eq!(rendered.labels.len(), 3, "one primary and two secondary labels");
    assert!(rendered.labels[0].is_primary, "the primary span comes first");
    assert_eq!(rendered.labels[0].span, span(5, 7));
    assert_eq!(
        rendered
            .labels
            .iter()
            .filter(|label| !label.is_primary)
            .map(|label| label.span)
            .collect::<Vec<_>>(),
        vec![span(11, 13), span(17, 19)],
        "and both secondary spans, in the order they were attached"
    );
    assert_eq!(rendered.help, vec!["declare the guard where the work is".to_string()]);
}

#[test]
fn into_error_still_carries_the_code_and_the_message_for_an_error() {
    // The path every existing caller uses is unchanged: this is the pair that stops "add the new
    // conversion" from being a replacement that quietly changes fifty call sites.
    let error = CompilerDiagnostic::error(DiagnosticCode::UnsafeIr, "the body is empty", span(3, 4)).into_error();
    let rendered = error.to_string();
    assert!(
        rendered.contains("X3E0501") && rendered.contains("the body is empty"),
        "code then message, as before: {rendered}"
    );
}

#[test]
#[should_panic(expected = "cannot carry the severity")]
fn into_error_refuses_a_severity_it_cannot_carry() {
    // The trap this ticket is about, made loud. `X3Error` cannot hold a severity, so rendering a
    // warning through it would produce an error *message* whose reader has no way to tell — and the
    // caller would then decide the vector by hand. The assertion fires in a test build, where a
    // developer meets it, rather than in a release binary where nobody does.
    let _ = CompilerDiagnostic::warning(DiagnosticCode::UnsafeIr, "a finding, not a failure", span(0, 1)).into_error();
}

#[test]
fn the_accumulator_files_a_diagnostic_by_its_own_severity() {
    // The "rather than the caller re-deciding it" half, in the accumulator's real shape: its two
    // vectors are the severity channel, so the vector has to come from the diagnostic's field.
    let mut outcome = VerifyOutcome {
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    outcome.push_diagnostic(CompilerDiagnostic::error(
        DiagnosticCode::UnsafeIr,
        "the body is empty",
        span(1, 2),
    ));
    outcome.push_diagnostic(CompilerDiagnostic::warning(
        DiagnosticCode::UnsafeIr,
        "a declaration wider than the work it guards",
        span(3, 4),
    ));

    assert_eq!(outcome.errors.len(), 1, "the error is a rejection");
    assert_eq!(
        outcome.warnings.len(),
        1,
        "and the warning is a finding, not a rejection"
    );
    assert!(outcome.errors[0].to_string().contains("the body is empty"));
    assert!(outcome.warnings[0]
        .to_string()
        .contains("a declaration wider than the work it guards"));
}

#[test]
#[should_panic(expected = "this diagnostic is an error")]
fn the_warnings_channel_refuses_an_error() {
    // The other direction, and the more dangerous one: a rejected program filed as a clean one.
    let _ = CompilerDiagnostic::error(DiagnosticCode::UnsafeIr, "rejected", span(0, 1)).into_warning();
}
