//! TICKET-120: one failure-action slot must not silently keep the first of
//! two contradictory `on_fail` / `on_timeout` actions.

use x3_lang_compiler::formatter::X3Formatter;
use x3_lang_compiler::parser::parse_source;

#[test]
fn bridge_with_two_different_failure_actions_is_refused_and_names_both() {
    let source = r#"
bridge b ethereum.USDC to solana.USDC {
    on_fail halt
    on_timeout 30s refund solana.USDC to bob
}
"#;
    let error = parse_source(source).expect_err("contradictory failure actions must be refused");
    let message = error.to_string();
    assert!(
        message.contains("on_fail"),
        "message should name the clauses: {message}"
    );
    assert!(
        message.contains("on_timeout"),
        "message should name the clauses: {message}"
    );
    assert!(
        message.contains("halt"),
        "message should name the first action: {message}"
    );
    assert!(
        message.contains("refund"),
        "message should name the second action: {message}"
    );
}

#[test]
fn atomic_swap_with_two_different_failure_actions_is_refused() {
    let source = r#"
atomic_swap Probe {
    on_fail halt
    on_timeout 30s refund solana.SOL to bob
}
"#;
    let error = parse_source(source).expect_err("contradictory failure actions must be refused");
    assert!(error.to_string().contains("two different actions"));
}

#[test]
fn bridge_with_the_same_action_stated_twice_parses_and_round_trips() {
    let source = r#"
bridge b ethereum.USDC to solana.USDC {
    on_fail rollback
    on_timeout 30s rollback
}
"#;
    let program = parse_source(source).expect("the same action stated twice is not a contradiction");
    let formatted = X3Formatter::new().format_program(&program);
    let reparsed = parse_source(&formatted).expect("the formatter's spelling must parse");
    let first = serde_json::to_value(&program.items[0].node).unwrap();
    let second = serde_json::to_value(&reparsed.items[0].node).unwrap();
    assert_eq!(first, second);
}

#[test]
fn atomic_swap_with_the_same_action_stated_twice_parses() {
    let source = r#"
atomic swap ethereum.USDC -> solana.SOL {
    on_fail rollback
    on_timeout 30s rollback
}
"#;
    let program = parse_source(source).expect("the same action stated twice must be accepted");
    assert_eq!(program.items.len(), 1);
}
