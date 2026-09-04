use x3_lang_compiler::diagnostic::DiagnosticCode;
use x3_lang_compiler::numeric::verify_numeric_policy;
use x3_lang_compiler::parser::parse_source;

fn numeric_codes(source: &str) -> Vec<DiagnosticCode> {
    let program = parse_source(source).expect("numeric-policy fixture must parse");
    verify_numeric_policy(&program).into_iter().map(|d| d.code).collect()
}

#[test]
fn bare_integer_literal_is_u64_for_direct_arguments() {
    let codes = numeric_codes("fn takes_u64(x: u64) { } fn main() { takes_u64(1); }");
    assert!(codes.is_empty(), "bare integer literal must satisfy u64: {codes:?}");
}

#[test]
fn unary_negation_produces_signed_numeric_argument() {
    let codes = numeric_codes("fn takes_i64(x: i64) { } fn main() { takes_i64(-1); }");
    assert!(codes.is_empty(), "unary-negated integer must satisfy i64: {codes:?}");
}

#[test]
fn bare_unsigned_literal_is_not_implicitly_coerced_to_signed_argument() {
    let codes = numeric_codes("fn takes_i64(x: i64) { } fn main() { takes_i64(1); }");
    assert_eq!(codes, vec![DiagnosticCode::ArgumentTypeMismatch]);
}

#[test]
fn unary_negative_literal_is_not_implicitly_coerced_to_unsigned_argument() {
    let codes = numeric_codes("fn takes_u64(x: u64) { } fn main() { takes_u64(-1); }");
    assert_eq!(codes, vec![DiagnosticCode::ArgumentTypeMismatch]);
}

#[test]
fn bare_u64_literal_is_not_implicitly_narrowed_to_u32() {
    let codes = numeric_codes("fn takes_u32(x: u32) { } fn main() { takes_u32(1); }");
    assert_eq!(codes, vec![DiagnosticCode::ArgumentTypeMismatch]);
}
