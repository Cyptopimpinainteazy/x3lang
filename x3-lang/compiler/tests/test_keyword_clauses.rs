//! Clauses whose words are **lexer keywords**, and the arms that read them.
//!
//! The parser matches tokens, and a word the lexer reserves never arrives as an identifier. Three
//! arms were written against the identifier form anyway, so the clauses they read were refused
//! while the messages and comments around them said the clauses were supported (TICKET-046):
//!
//! | clause | what happened |
//! |---|---|
//! | `use <target> <config>` in an intent body | `unexpected clause in intent body: KwUse`, from a
//!   match that lists `use` among the clauses it accepts |
//! | `finality_policy X { ethereum require finalized }` | `expected '}' after finality_policy body` |
//! | `rpc_quorum { source require 2_of_3 }` | `rpc_quorum source chain: expected identifier` |
//!
//! Each test below fails against the identifier arm: the source is a program no build could read.

use x3_lang_compiler::{compile_to_ir, parser::parse_source, Operation};

fn ir(source: &str) -> Vec<Operation> {
    let program = parse_source(source).unwrap_or_else(|error| panic!("must parse:\n{source}\n{error}"));
    compile_to_ir(&program)
        .unwrap_or_else(|error| panic!("must lower:\n{source}\n{error:?}"))
        .operations
}

#[test]
fn the_terse_finality_policy_form_parses() {
    // `<chain> require <mode>` — the form the parser's own comment documents, next to the long
    // `chain` / `requirement` / `blocks` form.
    let operations = ir("finality_policy strict {\n    ethereum require finalized\n    blocks 12\n}\n");
    assert!(
        operations.iter().any(|operation| matches!(
            operation,
            Operation::Require {
                condition: x3_lang_compiler::ir::Condition::FinalityPolicy { requirement, blocks, .. },
                ..
            } if requirement == "finalized" && *blocks == Some(12)
        )),
        "the terse form must state the requirement and the depth: {operations:?}"
    );
}

#[test]
fn the_long_finality_policy_form_still_parses() {
    let operations = ir("finality_policy strict {\n    chain ethereum\n    requirement finalized\n    blocks 12\n}\n");
    assert!(
        operations.iter().any(|operation| matches!(
            operation,
            Operation::Require {
                condition: x3_lang_compiler::ir::Condition::FinalityPolicy { requirement, blocks, .. },
                ..
            } if requirement == "finalized" && *blocks == Some(12)
        )),
        "the long form is unchanged: {operations:?}"
    );
}

#[test]
fn the_inline_rpc_quorum_form_parses() {
    // `source require N_of_M` on one line, which the parser's own comment calls the inline form.
    let operations = ir("rpc_quorum {\n    source require 2_of_3\n    relayers a b c\n}\n");
    assert!(
        operations
            .iter()
            .any(|operation| matches!(operation, Operation::RpcConsensus { require: (2, 3), .. })),
        "the inline form must state the quorum it names: {operations:?}"
    );
}

#[test]
fn the_use_clause_lowers_to_a_named_host_call() {
    // `use <target> <config>` is a host call: the language has no instruction for "use this
    // venue", so it asks the host by name — the same shape `on <event> <action>` lowers to.
    let operations = ir("intent probe {\n    from ethereum.USDC amount 1 receiver \
         0x1111111111111111111111111111111111111111\n    to solana.SOL receiver \
         4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD\n    use uniswap 1\n    on_fail \
         rollback\n}\n");
    assert!(
        operations.iter().any(|operation| matches!(
            operation,
            Operation::Call { function, args } if function == "use" && args == &["uniswap".to_string(), "1".to_string()]
        )),
        "`use uniswap 1` must reach the IR as a named call: {operations:?}"
    );
}
