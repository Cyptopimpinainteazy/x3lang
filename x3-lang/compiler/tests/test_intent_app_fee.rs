//! `fee <rate> to <account>` is adapter metadata, like `under mandate`.
//!
//! The parser keeps the rate (whole bps) and the recipient; the semantic verifier
//! refuses a fee the NEAR adapter would refuse (zero, above 1Click's 500 bps
//! ceiling, without a `require fees` cap, or above it); the formatter writes it
//! back; and lowering hands it to the intent resolver as a constraint rather than
//! dropping it or pretending the VM paid it.

use x3_lang_ast::ast::Item;
use x3_lang_compiler::intent_emit::from_intent_decl;
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::{check_source, compile_to_ir, Operation};

const SOURCE: &str = r#"
intent app_fee_payment {
    from Solana.USDC amount 25000000 receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    to Near.USDC receiver x3-demo.near
    route {
        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24700000;
    }
    fee 25 bps to fees.x3-demo.near
    require slippage <= 0.5%
    require fees <= 100 bps
    require nonce unused app_fee_001
    timeout 10m refund Solana.USDC
    on_fail rollback
}
"#;

fn intent(source: &str) -> x3_lang_ast::ast::IntentDecl {
    let program = parse_source(source).expect("the intent must parse");
    program
        .items
        .into_iter()
        .find_map(|item| match item.node {
            Item::IntentDecl(intent) => Some(intent),
            _ => None,
        })
        .expect("the source declares one intent")
}

fn semantic_errors(source: &str) -> Vec<String> {
    match check_source(source) {
        Ok((_, _, errors)) => errors.iter().map(|e| e.to_string()).collect(),
        Err(error) => vec![error.to_string()],
    }
}

#[test]
fn the_fee_is_parsed_into_the_intent() {
    let fee = intent(SOURCE).app_fee.expect("the fee clause is kept");
    assert_eq!(fee.bps, 25);
    assert_eq!(fee.recipient.as_str(), "fees.x3-demo.near");
}

#[test]
fn a_percent_rate_is_whole_bps() {
    let source = SOURCE.replace("fee 25 bps", "fee 0.25%");
    assert_eq!(intent(&source).app_fee.map(|fee| fee.bps), Some(25));
    let source = SOURCE.replace("fee 25 bps", "fee 1%");
    assert_eq!(intent(&source).app_fee.map(|fee| fee.bps), Some(100));
}

#[test]
fn a_quoted_implicit_recipient_is_accepted() {
    let implicit = "5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd";
    let source = SOURCE.replace("fees.x3-demo.near", &format!("\"{implicit}\""));
    assert_eq!(
        intent(&source).app_fee.map(|fee| fee.recipient.as_str().to_string()),
        Some(implicit.to_string())
    );
}

#[test]
fn a_malformed_fee_is_a_parse_error() {
    for (bad, why) in [
        ("fee 25 to fees.x3-demo.near", "a rate without a unit"),
        ("fee 25 bps fees.x3-demo.near", "no `to`"),
        ("fee 0.125% to fees.x3-demo.near", "a fraction of a basis point"),
        ("fee 25 bps to Fees.Near", "an account that is not lowercase"),
    ] {
        let source = SOURCE.replace("fee 25 bps to fees.x3-demo.near", bad);
        assert!(parse_source(&source).is_err(), "{why} must be refused: {bad}");
    }
    let twice = SOURCE.replace(
        "    fee 25 bps to fees.x3-demo.near\n",
        "    fee 25 bps to fees.x3-demo.near\n    fee 5 bps to other.near\n",
    );
    let error = parse_source(&twice).expect_err("one fee per intent");
    assert!(error.to_string().contains("one `fee` clause"), "{error}");
}

#[test]
fn the_verifier_accepts_a_fee_inside_its_cap() {
    assert!(semantic_errors(SOURCE).is_empty(), "{:?}", semantic_errors(SOURCE));
    let at_cap = SOURCE.replace("fee 25 bps", "fee 100 bps");
    assert!(semantic_errors(&at_cap).is_empty(), "{:?}", semantic_errors(&at_cap));
}

#[test]
fn the_verifier_refuses_a_fee_the_adapter_would_refuse() {
    let cases = [
        (SOURCE.replace("fee 25 bps", "fee 0 bps"), "charges nothing"),
        (
            SOURCE.replace("fee 25 bps", "fee 101 bps"),
            "above its own `require fees` cap of 100 bps",
        ),
        (
            SOURCE
                .replace("fee 25 bps", "fee 501 bps")
                .replace("require fees <= 100 bps", "require fees <= 10%"),
            "the most 1Click quotes",
        ),
        (
            SOURCE.replace("    require fees <= 100 bps\n", ""),
            "no `require fees <= <bound>`",
        ),
        (
            SOURCE.replace("require fees <= 100 bps", "require fees < 25 bps"),
            "above its own `require fees` cap of 24 bps",
        ),
    ];
    for (source, needle) in cases {
        let errors = semantic_errors(&source);
        assert!(
            errors.iter().any(|e| e.contains(needle)),
            "expected an error containing {needle:?}, got {errors:?}"
        );
    }
}

#[test]
fn formatter_round_trips_the_fee() {
    let program = parse_source(SOURCE).expect("the source must parse");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(
        formatted.contains("    fee 25 bps to fees.x3-demo.near\n"),
        "the formatter must keep the fee:\n{formatted}"
    );
    assert_eq!(intent(&formatted).app_fee, intent(SOURCE).app_fee);

    let implicit = "5880ad2b362620fadf759cbceb1cd5737ce8c6ed7fb8e9942881e6731f9247dd";
    let source = SOURCE.replace("fees.x3-demo.near", &format!("\"{implicit}\""));
    let program = parse_source(&source).expect("the quoted recipient must parse");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert_eq!(intent(&formatted).app_fee, intent(&source).app_fee, "{formatted}");
}

#[test]
fn lowering_hands_the_fee_to_the_intent_resolver() {
    let program = parse_source(SOURCE).expect("the source must parse");
    let ir = compile_to_ir(&program).expect("lowering keeps adapter metadata");
    assert!(
        ir.operations.iter().any(|operation| matches!(
            operation,
            Operation::IntentResolve { constraints, .. }
                if constraints.iter().any(|value| value == "fee 25 bps to fees.x3-demo.near")
        )),
        "the fee must reach the adapter boundary: {ir:?}"
    );
}

#[test]
fn the_adapter_draft_carries_the_fee() {
    let draft = from_intent_decl(&intent(SOURCE));
    assert!(
        draft
            .constraints
            .iter()
            .any(|constraint| constraint.kind == "app_fee" && constraint.arg == "25 bps to fees.x3-demo.near"),
        "{draft:?}"
    );
}

#[test]
fn the_shipped_example_checks_clean() {
    let source = include_str!("../../examples/near_intents_app_fee.x3");
    let errors = semantic_errors(source);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(intent(source).app_fee.map(|fee| fee.bps), Some(25));
}
