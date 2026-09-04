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
