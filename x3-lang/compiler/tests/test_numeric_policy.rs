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

/// RFC t5-6 as amended 2026-09-26: an unsuffixed literal takes its parameter's integer type.
#[test]
fn bare_literal_takes_a_signed_parameter_type() {
    let codes = numeric_codes("fn takes_i64(x: i64) { } fn main() { takes_i64(1); }");
    assert!(codes.is_empty(), "a bare literal must satisfy i64: {codes:?}");
}

#[test]
fn unary_negative_literal_is_not_implicitly_coerced_to_unsigned_argument() {
    let codes = numeric_codes("fn takes_u64(x: u64) { } fn main() { takes_u64(-1); }");
    assert_eq!(codes, vec![DiagnosticCode::ArgumentTypeMismatch]);
}

#[test]
fn bare_literal_takes_a_narrower_parameter_type_when_it_fits() {
    let codes = numeric_codes("fn takes_u32(x: u32) { } fn main() { takes_u32(1); }");
    assert!(codes.is_empty(), "a bare literal that fits must satisfy u32: {codes:?}");
}

#[test]
fn bare_literal_out_of_range_for_its_parameter_is_refused() {
    let codes = numeric_codes("fn takes_u8(x: u8) { } fn main() { takes_u8(300); }");
    assert_eq!(codes, vec![DiagnosticCode::ArgumentTypeMismatch]);
    let codes = numeric_codes("fn takes_i8(x: i8) { } fn main() { takes_i8(-129); }");
    assert_eq!(codes, vec![DiagnosticCode::ArgumentTypeMismatch]);
}

#[test]
fn negative_literal_is_refused_for_every_unsigned_width() {
    for ty in ["u8", "u16", "u32", "u64", "u128"] {
        let source = format!("fn takes(x: {ty}) {{ }} fn main() {{ takes(-1); }}");
        assert_eq!(
            numeric_codes(&source),
            vec![DiagnosticCode::ArgumentTypeMismatch],
            "-1 must not satisfy {ty}"
        );
    }
}
