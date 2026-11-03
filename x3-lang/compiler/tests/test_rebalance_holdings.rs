//! A rebalance can state what it holds, so the artifact carries both ends of the move
//! (TICKET-070).
//!
//! PHASE 11's last sentence is that the compiler should "eventually" generate the transaction
//! graph that reaches the target. It could not, and the reason was not effort: every trade to
//! the target depends on where the portfolio *starts*, and a compiler has no state. The
//! declaration may state it now — `holds { chain.ASSET = <n>; }` — and it travels beside the
//! target, so a host that prices both can compute the trades. These tests are about the
//! carrying: that the clause parses, that the amounts reach the artifact in order, that
//! "stated none" is a different fact from "holds nothing", and that neither a reformat nor a
//! stored AST loses it.

use x3_lang_compiler::ir::Operation;
use x3_lang_compiler::semantic::CompilationMode;

/// A portfolio that adds up, with or without the holdings clause.
fn portfolio(holds: &str) -> String {
    format!(
        r#"rebalance portfolio {{
{holds}    BTC = 60%;
    ETH = 40%;
    minimize {{ fees; }}
    atomic;
}}
"#
    )
}

const HOLDS: &str = "    holds {\n        ethereum.BTC = 5;\n        ethereum.ETH = 40;\n    }\n";

fn lowered(source: &str) -> Vec<Operation> {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("it must lower");
    assert!(outcome.errors.is_empty(), "it must check: {:?}", outcome.errors);
    ir.operations
}

fn holdings(source: &str) -> Vec<(String, u128)> {
    lowered(source)
        .into_iter()
        .find_map(|operation| match operation {
            Operation::Rebalance { holdings, .. } => Some(holdings),
            _ => None,
        })
        .expect("a rebalance must reach the IR")
}

#[test]
fn the_stated_holdings_reach_the_ir_in_the_order_written() {
    assert_eq!(
        holdings(&portfolio(HOLDS)),
        vec![("ethereum.BTC".to_string(), 5), ("ethereum.ETH".to_string(), 40),],
        "both ends of the move travel, so a host can price them and compute the trades"
    );
}

/// `holds` absent is not `holds {}`. An empty list is a portfolio that holds nothing, which
/// is a portfolio at the start of its life; a missing clause is a program that did not say.
/// The artifact carries the empty list either way, and the *program* is what differs — which
/// is why this asserts both readings rather than only that the field exists.
#[test]
fn a_rebalance_that_states_no_holdings_carries_none_rather_than_zeroes() {
    assert!(
        holdings(&portfolio("")).is_empty(),
        "a program that said nothing carries nothing, not an invented zero"
    );
    assert_eq!(
        holdings(&portfolio("    holds {\n        ethereum.BTC = 0;\n    }\n")),
        vec![("ethereum.BTC".to_string(), 0)],
        "and a stated zero is a holding, because the program wrote it"
    );
}

/// The clause has to survive a reformat, or `x3c fmt` would delete the input the trades need
/// — the defect TICKET-090 fixed for a percent literal, in the other direction.
#[test]
fn the_clause_round_trips_through_the_formatter() {
    let source = portfolio(HOLDS);
    let program = x3_lang_compiler::parser::parse_source(&source).expect("it must parse");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(
        formatted.contains("holds {") && formatted.contains("ethereum.BTC = 5;"),
        "the holdings must survive the formatter: {formatted}"
    );

    let again = x3_lang_compiler::parser::parse_source(&formatted).expect("the output must parse");
    let twice = x3_lang_compiler::formatter::X3Formatter::new().format_program(&again);
    assert_eq!(formatted, twice, "and formatting must be idempotent");
    assert_eq!(
        holdings(&formatted),
        holdings(&source),
        "the reformatted program holds what the original did"
    );

    // And a program with no clause does not acquire an empty one.
    let without = portfolio("");
    let program = x3_lang_compiler::parser::parse_source(&without).expect("it must parse");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(
        !formatted.contains("holds"),
        "the formatter must not invent a clause the program did not write: {formatted}"
    );
}

/// A holding named twice is the defect a weight named twice is: the second amount would
/// replace the first, so one of them would not be in force.
#[test]
fn a_holding_stated_twice_is_refused_with_the_asset_named() {
    let source = portfolio("    holds {\n        ethereum.BTC = 5;\n        ethereum.BTC = 6;\n    }\n");
    let (_, _, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev).expect("it must lower");
    let messages: Vec<String> = outcome.errors.iter().map(|error| error.to_string()).collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("ethereum.BTC") && message.contains("twice")),
        "the refusal must name the asset that is doubly held: {messages:?}"
    );
}

/// A holding is an amount in the asset's own units. A name is not an amount, and a fractional
/// amount is not representable — the units are the asset's, and the compiler has no decimals
/// to scale by.
#[test]
fn a_holding_that_is_not_an_amount_is_refused() {
    for bad in [
        "    holds {\n        ethereum.BTC = eth;\n    }\n",
        "    holds {\n        ethereum.BTC = 1.5;\n    }\n",
    ] {
        let source = portfolio(bad);
        let outcome = x3_lang_compiler::parser::parse_source(&source)
            .err()
            .map(|error| error.to_string())
            .or_else(|| {
                x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev)
                    .ok()
                    .filter(|(_, _, outcome)| !outcome.errors.is_empty())
                    .map(|(_, _, outcome)| format!("{:?}", outcome.errors))
            });
        assert!(
            outcome.is_some(),
            "`{bad}` is not an amount and must be refused rather than read as one"
        );
    }
}

/// An AST stored before the clause existed carries no holdings, and `#[serde(default)]` is
/// what makes it load rather than fail — the TICKET-067 precedent.
#[test]
fn an_ast_stored_before_the_clause_still_loads() {
    let json = r#"{
        "name": "portfolio",
        "weights": [
            [{"chain": "ethereum", "name": "BTC"}, 60],
            [{"chain": "ethereum", "name": "ETH"}, 40]
        ],
        "minimize": ["MinimizeFees"]
    }"#;
    let decl: x3_lang_ast::ast::RebalanceDecl = serde_json::from_str(json).expect("an older AST must still load");
    assert!(
        decl.holdings.is_empty(),
        "a document written before the clause carries no holdings, which is the honest answer"
    );
}
