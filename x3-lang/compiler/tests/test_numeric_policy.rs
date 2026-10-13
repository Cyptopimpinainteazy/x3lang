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

#[test]
fn full_width_unsuffixed_literals_use_parameter_bounds() {
    for ty in ["u128", "u256", "i256"] {
        let source = format!("fn takes(x: {ty}) {{ }} fn main() {{ takes({}); }}", u128::MAX);
        assert!(numeric_codes(&source).is_empty(), "u128::MAX must fit {ty}");
    }
    for ty in ["u8", "u64", "i128"] {
        let source = format!("fn takes(x: {ty}) {{ }} fn main() {{ takes({}); }}", u128::MAX);
        assert_eq!(numeric_codes(&source), vec![DiagnosticCode::ArgumentTypeMismatch]);
    }
}

#[test]
fn full_width_negative_literals_use_parameter_bounds() {
    let minimum_magnitude = 1u128 << 127;
    let source = format!("fn takes(x: i128) {{ }} fn main() {{ takes(-{minimum_magnitude}); }}");
    assert!(numeric_codes(&source).is_empty(), "i128::MIN must fit i128");
    for (ty, magnitude) in [("i128", minimum_magnitude + 1), ("u128", minimum_magnitude)] {
        let source = format!("fn takes(x: {ty}) {{ }} fn main() {{ takes(-{magnitude}); }}");
        assert_eq!(numeric_codes(&source), vec![DiagnosticCode::ArgumentTypeMismatch]);
    }
    let source = format!("fn takes(x: i256) {{ }} fn main() {{ takes(-{}); }}", u128::MAX);
    assert!(numeric_codes(&source).is_empty());
}

#[test]
fn build_rejects_full_width_literal_for_u64_parameter() {
    let source = format!("fn takes(x: u64) {{ }} fn main() {{ takes({}); }}", u128::MAX);
    let error = x3_lang_compiler::compile_source(&source).expect_err("out-of-range call must not build");
    assert!(
        error.to_string().contains("X3E0202"),
        "numeric policy must reject before lowering: {error}"
    );
}

#[test]
fn negated_zero_preserves_unsigned_compatibility() {
    for ty in ["u8", "u16", "u32", "u64", "u128", "u256", "usize"] {
        let source = format!("fn takes(x: {ty}) {{ }} fn main() {{ takes(-0); }}");
        assert!(numeric_codes(&source).is_empty(), "zero must fit {ty}");
    }
}

#[test]
fn suffixed_literals_must_fit_their_own_type_even_without_calls() {
    for literal in ["256u8", "128i8", "-129i8", "-1u8", "18446744073709551616u64"] {
        let source = format!("fn main() {{ let x = {literal}; }}");
        assert_eq!(
            numeric_codes(&source),
            vec![DiagnosticCode::InvalidNumericCoercion],
            "{literal}"
        );
    }
    for literal in ["255u8", "127i8", "-128i8", "0u8", "18446744073709551615u64"] {
        let source = format!("fn main() {{ let x = {literal}; }}");
        assert!(numeric_codes(&source).is_empty(), "{literal}");
    }
}

#[test]
fn typed_parameters_and_locals_require_exact_call_compatibility() {
    for source in [
        "fn takes(x: u64) { } fn main(value: u32) { takes(value); }",
        "fn takes(x: u64) { } fn main() { let value: u32 = 1; takes(value); }",
        "fn takes(x: u64) { } fn main() { let value = 1u32; takes(value); }",
        "fn takes(x: i64) { } fn main(value: u64) { takes(value); }",
    ] {
        assert_eq!(
            numeric_codes(source),
            vec![DiagnosticCode::ArgumentTypeMismatch],
            "{source}"
        );
    }
    assert!(numeric_codes("fn takes(x: u64) { } fn main(value: u64) { takes(value); }").is_empty());
}

#[test]
fn nested_bindings_do_not_leak_and_unknown_types_shadow_numeric_bindings() {
    let source =
        "fn takes(x: u64) { } fn main(value: u64) { if true { let value: u32 = 1; takes(value); } takes(value); }";
    assert_eq!(numeric_codes(source), vec![DiagnosticCode::ArgumentTypeMismatch]);
    let source =
        "fn takes(x: u64) { } fn main(value: u32) { if true { let value = false; takes(value); } takes(value); }";
    assert_eq!(numeric_codes(source), vec![DiagnosticCode::ArgumentTypeMismatch]);
}

#[test]
fn build_rejects_out_of_range_suffix_before_lowering() {
    let error = x3_lang_compiler::compile_source("fn main() { let value = 256u8; }")
        .expect_err("invalid suffix must not build");
    assert!(error.to_string().contains("X3E0301"), "{error}");
}

#[test]
fn formatting_preserves_explicit_integer_types() {
    let source = "fn takes(x: u64) { } fn main() { takes(1u32); }";
    let program = parse_source(source).expect("parse typed literal");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(formatted.contains("1u32"), "suffix lost: {formatted}");
    assert_eq!(numeric_codes(&formatted), vec![DiagnosticCode::ArgumentTypeMismatch]);
}

#[test]
fn explicit_integer_suffixes_and_bases_round_trip() {
    for literal in [
        "0u8", "0u16", "0u32", "0u64", "0u128", "0U256", "0usize", "0i8", "0i16", "0i32", "0i64", "0i128", "0I256",
        "0isize", "0xffu8", "0b111u16", "0o77u32",
    ] {
        let source = format!("fn main() {{ let value = {literal}; }}");
        let program = parse_source(&source).expect("parse suffixed integer");
        let x3_lang_ast::ast::Item::Function(function) = &program.items[0].node else {
            panic!("expected function");
        };
        assert!(
            matches!(
                &function.body.stmts[0],
                x3_lang_ast::ast::Statement::Let {
                    expr: Some(x3_lang_ast::ast::Expression::Literal(
                        x3_lang_ast::ast::LiteralExpr::Int { suffix: Some(_), .. }
                    )),
                    ..
                }
            ),
            "typed number became an identifier: {literal}"
        );
        let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
        assert!(formatted.contains(literal), "lost integer type or base: {formatted}");
        assert!(numeric_codes(&formatted).is_empty(), "{literal}");
    }
}

#[test]
fn annotated_initializers_refuse_numeric_coercion() {
    for source in [
        "fn main() { let value: u8 = 256; }",
        "fn main() { let value: u32 = 1u64; }",
        "fn main(value: u64) { let copy: u32 = value; }",
    ] {
        assert_eq!(
            numeric_codes(source),
            vec![DiagnosticCode::InvalidNumericCoercion],
            "{source}"
        );
    }
    assert!(numeric_codes("fn main() { let value: i8 = -128; }").is_empty());
}

#[test]
fn closure_parameters_shadow_outer_types_and_captured_values_keep_theirs() {
    use x3_lang_ast::ast::{Expression, Item, Parameter, Statement, TypeExpr};
    use x3_lang_common::Symbol;
    for (outer, params) in [
        (
            "u64",
            vec![Parameter {
                name: Some(Symbol::new("value")),
                ty: Some(TypeExpr::Primitive(Symbol::new("u32"))),
                is_mut: false,
            }],
        ),
        ("u32", vec![]),
    ] {
        let mut program = parse_source(&format!(
            "fn takes(x: u64) {{ }} fn main(value: {outer}) {{ takes(value); }}"
        ))
        .unwrap();
        let Item::Function(main) = &mut program.items[1].node else {
            panic!("expected function")
        };
        let Statement::Expr(body) = main.body.stmts.remove(0) else {
            panic!("expected call")
        };
        main.body.stmts.push(Statement::Expr(Expression::Closure {
            params,
            body: Box::new(body),
            is_async: false,
        }));
        assert_eq!(
            verify_numeric_policy(&program)
                .into_iter()
                .map(|d| d.code)
                .collect::<Vec<_>>(),
            vec![DiagnosticCode::ArgumentTypeMismatch]
        );
    }
}

#[test]
fn suffixed_signed_minimum_does_not_hide_intermediate_negation_overflow() {
    // -128i8 is a valid literal spelling; negating it twice more still passes through +128.
    assert_eq!(
        numeric_codes("fn main() { let value = -(-(-128i8)); }"),
        vec![DiagnosticCode::InvalidNumericCoercion]
    );
}

#[test]
fn call_expressions_in_callee_position_are_checked() {
    assert_eq!(
        numeric_codes("fn takes(x: u64) { } fn main() { factory(takes(1u32))(0); }"),
        vec![DiagnosticCode::ArgumentTypeMismatch]
    );
}

#[test]
fn malformed_or_unrepresentable_typed_integers_cannot_become_identifiers() {
    for literal in [
        "1__0u8",
        "1_u8",
        "0b2u8",
        "0x_u8",
        "340282366920938463463374607431768211456u128",
    ] {
        let source = format!("fn main() {{ let value = {literal}; }}");
        assert!(
            parse_source(&source).is_err(),
            "invalid integer parsed as a name: {literal}"
        );
    }
}
