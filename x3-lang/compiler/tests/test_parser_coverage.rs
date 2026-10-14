//! Broad parser/expression coverage tests.
//!
//! The production-intent surface is fully exercised by the e2e
//! fixtures. This file fills the gap on the secondary parsers
//! (function, agent, struct, enum, expression operators,
//! postfix calls, closures, etc.) so the line coverage of
//! `parser.rs` / `lowering.rs` stays high.
//!
//! The contract target is "Coverage >80% for x3-lang scope";
//! hitting every dispatch arm in the parser is the cheapest way
//! to get there without rewriting the inline tokenizer.

use x3_lang_compiler::parser::parse_source;

fn must_parse(src: &str) {
    parse_source(src).unwrap_or_else(|e| panic!("parse failed for {src:?}: {e}"));
}

fn must_reject(src: &str) {
    assert!(parse_source(src).is_err(), "expected parser to reject {src:?}");
}

// ---------------- top-level items ----------------

#[test]
fn function_with_args_and_return() {
    must_parse("fn add(a: u64, b: u64) -> u64 { return a + b; }");
}

#[test]
fn function_with_let_binding() {
    must_parse("fn f() { let x = 1; let mut y = 2; }");
}

#[test]
fn agent_declaration() {
    must_parse("agent Counter { state: u64 }");
}

#[test]
fn struct_declaration() {
    must_parse("struct Point { x: u64, y: u64 }");
}

#[test]
fn enum_declaration() {
    must_parse("enum Color { Red, Green, Blue }");
}

#[test]
fn const_declaration() {
    must_parse("const MAX: u64 = 100;");
}

#[test]
fn import_declaration() {
    must_parse("import std::io;");
}

#[test]
fn async_function() {
    must_parse("async fn fetch() -> u64 { return 1; }");
}

// ---------------- expressions ----------------

#[test]
fn expression_arithmetic() {
    must_parse("fn f() { let x = 1 + 2 * 3 - 4 / 5 % 6; }");
}

#[test]
fn expression_comparisons() {
    must_parse("fn f() { let b = 1 == 2 && 3 != 4 || 5 < 6; }");
}

#[test]
fn expression_negation_and_not() {
    must_parse("fn f() { let b = !true; let n = -x; }");
}

#[test]
fn expression_loop_break_continue() {
    must_parse("fn f() { loop { break; continue; } }");
}

#[test]
fn expression_call_chain() {
    must_parse("fn f() { let r = a.b(c).d(e); }");
}

#[test]
fn expression_paren_grouping() {
    must_parse("fn f() { let r = (1 + 2) * 3; }");
}

#[test]
fn expression_block() {
    must_parse("fn f() { let r = { let x = 1; x + 1 }; }");
}

// ---------------- atomic / atomic_swap items ----------------

#[test]
fn subscription_top_level() {
    must_parse("subscription s1: 100, 5 { }");
}

#[test]
fn simulate_top_level() {
    must_parse("simulate dry_run { }");
}

#[test]
fn proposal_top_level() {
    must_parse("proposal p1 { }");
}

#[test]
fn strategy_top_level() {
    must_parse("strategy low_risk { }");
}

// ---------------- negative cases ----------------

#[test]
fn rejects_garbage_at_top_level() {
    must_reject("@@@@@");
}

#[test]
fn rejects_unterminated_string() {
    must_reject("let s = \"unterminated");
}

#[test]
fn rejects_unclosed_brace() {
    must_reject("fn f() { let x = 1; ");
}

#[test]
fn rejects_empty_top_level_intent() {
    // intent requires a name and body
    must_reject("intent");
}

#[test]
fn rejects_unknown_top_level() {
    must_reject("blah x { }");
}

/// Anything unrecognised inside an intent body used to be parsed as a generic
/// expression statement and then silently ignored, so a declaration the author
/// believed was in force was not. Only the documented clause set is accepted
/// now.
#[test]
fn rejects_an_unrecognised_clause_in_an_intent_body() {
    must_reject(
        r#"intent t {
    from ethereum.USDC amount 1
    widgetize the route
}"#,
    );
}

#[test]
fn rejects_proofs_required_nested_inside_an_intent_body() {
    // It parses as a file-scope item, and a nested copy was accepted and never
    // applied — the diagnostic has to say so rather than silently dropping it.
    let err = parse_source(
        r#"intent t {
    from ethereum.USDC amount 1
    proofs required { source_lock_proof }
}"#,
    )
    .expect_err("a nested proofs declaration must be rejected");
    assert!(
        format!("{err}").contains("file scope"),
        "the diagnostic must point at file scope, got: {err}"
    );
}

#[test]
fn accepts_proofs_required_at_file_scope() {
    must_parse(
        r#"proofs required { source_lock_proof destination_fill_proof }

intent t {
    from ethereum.USDC amount 1
}"#,
    );
}
#[test]
fn unknown_characters_are_rejected_at_their_exact_source_span() {
    for bad in ["?", "$", "#", "`", "\\", "💥"] {
        let prefix = "fn main() { ";
        let source = format!("{prefix}{bad} }}");
        let error = parse_source(&source).expect_err("unknown characters must never disappear");
        let x3_lang_common::X3Error::LexerError { message, span } = error else {
            panic!("expected a lexical refusal, got {error}");
        };
        assert!(message.contains(bad), "diagnostic must name {bad}: {message}");
        assert_eq!(span.start.as_usize(), prefix.len());
        assert_eq!(span.end.as_usize(), prefix.len() + bad.len());
        assert!(x3_lang_compiler::compile_source(&source).is_err());
    }
    assert!(parse_source("? fn main() {} ").is_err());
    assert!(parse_source("fn main() {} ?").is_err());
    assert!(parse_source("fn main() { /* unterminated").is_err());
}

#[test]
fn comments_and_string_contents_are_not_unknown_syntax() {
    assert!(parse_source("// ? $ # 💥\nfn main() { /* ? $ # 💥 */ let text = \"? $ # 💥\"; }").is_ok());
}
#[test]
fn escaped_strings_keep_their_value_after_formatting() {
    use x3_lang_ast::ast::{Expression, Item, LiteralExpr, Statement};
    let source = r#"fn main() { let text = "quote: \" slash: \\ line: \n tab: \t null: \0 💥"; }"#;
    let program = parse_source(source).expect("escaped string must parse");
    let Item::Function(function) = &program.items[0].node else {
        panic!("function expected")
    };
    let Statement::Let {
        expr: Some(Expression::Literal(LiteralExpr::String(value))),
        ..
    } = &function.body.stmts[0]
    else {
        panic!("string expected")
    };
    assert_eq!(value.as_str(), "quote: \" slash: \\ line: \n tab: \t null: \0 💥");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    let reparsed = parse_source(&formatted).expect("formatted string must parse");
    assert_eq!(
        serde_json::to_value(&function.body).unwrap(),
        match &reparsed.items[0].node {
            Item::Function(f) => serde_json::to_value(&f.body).unwrap(),
            _ => panic!("function expected"),
        }
    );
}

#[test]
fn malformed_strings_fail_lexically_without_panicking() {
    for source in [
        "fn main() { let text = \"💥",
        "fn main() { let text = \"bad\\q\"; }",
        "fn main() { let text = \"trailing\\",
    ] {
        assert!(
            matches!(parse_source(source), Err(x3_lang_common::X3Error::LexerError { .. })),
            "must reject {source:?}"
        );
    }
}

#[test]
fn decoded_strings_still_obey_asset_address_safety() {
    let source = r#"fn main() { atomic {
        lock ethereum.USDC amount 1 from "wallet\\name";
        release ethereum.USDC to "wallet\\name";
    } }"#;
    let error = x3_lang_compiler::compile_source(source).expect_err("unsafe address must be refused");
    assert!(error.to_string().contains("unsafe characters"));
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&parse_source(source).unwrap());
    let error = x3_lang_compiler::compile_source(&formatted).expect_err("formatting must not bypass address safety");
    assert!(error.to_string().contains("unsafe characters"));
}
#[test]
fn unicode_escape_survives_compilation_formatting_and_vm_execution() {
    let source = r#"fn main() { atomic {
        lock ethereum.USDC amount 1 from "wallet\u{7a}";
        release ethereum.USDC to "wallet\u{7a}";
    } }"#;
    let bytecode = x3_lang_compiler::compile_source(source).unwrap();
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&parse_source(source).unwrap());
    assert_eq!(bytecode, x3_lang_compiler::compile_source(&formatted).unwrap());
    let mut vm = x3_lang_vm::VM::new(bytecode, x3_lang_vm::VMConfig::default(), 100_000);
    vm.execute().unwrap();
    assert!(vm.state.asset_ops.iter().any(|op| matches!(op,
        x3_lang_common::AssetOpPayload::Lock { from, .. } if from == "walletz"
    )));
    for literal in [r#""\u{}""#, r#""\u{d800}""#, r#""\u{110000}""#] {
        assert!(matches!(
            parse_source(&format!("fn main() {{ let text = {literal}; }}")),
            Err(x3_lang_common::X3Error::LexerError { .. })
        ));
    }
}
#[test]
fn common_parser_errors_point_at_the_offending_token() {
    for (source, token) in [
        ("fn main() { let value = 1__0u8; }", "1__0u8"),
        ("// 💥\nfn main() { let value = 1__0u8; }", "1__0u8"),
        (
            "fn main() { let value = 340282366920938463463374607431768211456u128; }",
            "340282366920938463463374607431768211456u128",
        ),
        ("fn main() { let value = ; }", ";"),
        ("fn main() { let value = 1 }", "}"),
        ("fn 123() {}", "123"),
    ] {
        let x3_lang_common::X3Error::ParseError { span, .. } = parse_source(source).unwrap_err() else {
            panic!("parser error expected")
        };
        let start = source.find(token).unwrap();
        assert_eq!(span.start.as_usize(), start, "{source}");
        assert_eq!(span.end.as_usize(), start + token.len(), "{source}");
    }
    let source = "fn main() { let value = 1";
    let x3_lang_common::X3Error::ParseError { span, .. } = parse_source(source).unwrap_err() else {
        panic!("parser error expected")
    };
    assert_eq!(span.start.as_usize(), source.len());
    assert_eq!(span.end.as_usize(), source.len());
}
#[test]
fn annotation_errors_have_spans_and_integer_arguments_do_not_wrap() {
    for (source, offending) in [
        ("\n@no_recursion(4294967296) fn main() {}", "4294967296"),
        ("\n@multisig(2, \"bad\") fn main() {}", "\"bad\""),
        ("\n@role() fn main() {}", "role()"),
        ("\n@unknown fn main() {}", "unknown"),
        ("\n@subscription fn main() {}", "subscription"),
        (
            "\n@scheduled(18446744073709551616) fn main() {}",
            "18446744073709551616",
        ),
        ("\n@scheduled(\"bad\") fn main() {}", "\"bad\""),
        ("risk_policy { min_route_score 4294967296 }", "4294967296"),
        ("risk_policy { min_route_score \"bad\" }", "\"bad\""),
        (
            "risk_policy { max_slippage 18446744073709551616 }",
            "18446744073709551616",
        ),
        ("risk_policy { max_position \"bad\" }", "\"bad\""),
    ] {
        let x3_lang_common::X3Error::ParseError { span, .. } =
            parse_source(source).expect_err("invalid annotation must fail")
        else {
            panic!("parser error expected")
        };
        let start = source.find(offending).unwrap();
        assert_eq!(span.start.as_usize(), start, "{source}");
        assert_eq!(span.end.as_usize(), start + offending.len(), "{source}");
    }
    assert!(parse_source("@no_recursion(4294967295) fn main() {}").is_ok());
    let program = parse_source("@scheduled(18446744073709551615) fn main() {}").unwrap();
    let x3_lang_ast::ast::Item::Function(function) = &program.items[0].node else {
        panic!("function expected")
    };
    assert!(function.annotations.iter().any(
        |annotation| matches!(annotation, x3_lang_ast::ast::Annotation::Scheduled(period) if *period == u64::MAX)
    ));
}
