//! `fee <bps> bps to <account>` is an integrator fee the settling adapter pays (1Click
//! `appFees`). The compiler preserves it for the adapter and the formatter, and lowers it as a
//! constraint so a host that cannot pay it refuses the route instead of dropping the fee.

use x3_lang_ast::ast::Item;
use x3_lang_compiler::intent_emit::from_intent_decl;
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::{compile_to_ir, Operation};

const SOURCE: &str = r#"
intent x3_route_payment {
    from Base.USDC amount 25000000 receiver 0x000000000000000000000000000000000000dEaD
    to Near.USDC receiver x3-demo.near
    route {
        bridge NearIntents Base.USDC -> Near.USDC min_receive 24800000;
    }
    fee 25 bps to fees.yourapp.near
    require slippage <= 0.5%
    require fees <= 100 bps
    require nonce unused x3_route_payment_001
    timeout 10m refund Base.USDC
    on_fail rollback
}
"#;

fn intent(source: &str) -> x3_lang_ast::ast::IntentDecl {
    parse_source(source)
        .expect("the intent must parse")
        .items
        .into_iter()
        .find_map(|item| match item.node {
            Item::IntentDecl(intent) => Some(intent),
            _ => None,
        })
        .expect("the source declares one intent")
}

#[test]
fn fee_is_preserved_in_the_ast_and_the_adapter_draft() {
    let parsed = intent(SOURCE);
    assert_eq!(parsed.app_fees.len(), 1);
    assert_eq!(parsed.app_fees[0].bps, 25);
    assert_eq!(parsed.app_fees[0].recipient.as_str(), "fees.yourapp.near");
    let draft = from_intent_decl(&parsed);
    assert_eq!(draft.app_fees, vec![(25, "fees.yourapp.near".to_string())]);
}

#[test]
fn formatter_round_trips_the_fee() {
    let program = parse_source(SOURCE).expect("the source must parse");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(
        formatted.contains("    fee 25 bps to fees.yourapp.near\n"),
        "`x3c fmt` must never drop a fee:\n{formatted}"
    );
    let reparsed = intent(&formatted);
    assert_eq!(reparsed.app_fees[0].bps, 25);
}

#[test]
fn lowering_carries_the_fee_to_the_resolver_as_a_constraint() {
    let program = parse_source(SOURCE).expect("the source must parse");
    let ir = compile_to_ir(&program).expect("lowering preserves adapter metadata");
    assert!(
        ir.operations.iter().any(|operation| matches!(
            operation,
            Operation::IntentResolve { constraints, .. }
                if constraints.iter().any(|value| value == "app fee 25 bps to fees.yourapp.near")
        )),
        "a host that cannot pay the fee must see it and refuse: {ir:?}"
    );
}

#[test]
fn several_recipients_are_allowed_but_not_twice_the_same() {
    let two = SOURCE.replace(
        "    fee 25 bps to fees.yourapp.near\n",
        "    fee 20 bps to fees.yourapp.near\n    fee 5 bps to route.x3lang.near\n",
    );
    assert_eq!(intent(&two).app_fees.len(), 2);
    let dup = SOURCE.replace(
        "    fee 25 bps to fees.yourapp.near\n",
        "    fee 20 bps to fees.yourapp.near\n    fee 5 bps to fees.yourapp.near\n",
    );
    let error = parse_source(&dup).expect_err("one fee per recipient");
    assert!(error.to_string().contains("declared twice"), "{error}");
}

#[test]
fn malformed_fees_are_parse_errors() {
    for (bad, needle) in [
        ("fee 0 bps to fees.yourapp.near", "outside 1..=10000"),
        ("fee 10001 bps to fees.yourapp.near", "outside 1..=10000"),
        ("fee 25 pct to fees.yourapp.near", "stated in bps"),
        (
            "fee 25 bps for fees.yourapp.near",
            "expected `fee <n> bps to <account>`",
        ),
        ("fee 25 bps to Fees.near", "not a valid NEAR named account"),
        ("fee bps to fees.yourapp.near", "whole number of basis points"),
    ] {
        let source = SOURCE.replace("fee 25 bps to fees.yourapp.near", bad);
        let error = parse_source(&source).expect_err(bad);
        assert!(error.to_string().contains(needle), "{bad}: {error}");
    }
}

#[test]
fn fees_cannot_exceed_the_whole_input() {
    let source = SOURCE.replace(
        "    fee 25 bps to fees.yourapp.near\n",
        "    fee 6000 bps to a-fees.near\n    fee 5000 bps to b-fees.near\n",
    );
    let error = parse_source(&source).expect_err("fees above 100% are nonsense");
    assert!(error.to_string().contains("more than the whole input"), "{error}");
}

#[test]
fn the_new_chains_parse() {
    for chain in ["Gnosis", "Scroll", "Monad", "Berachain"] {
        let source = SOURCE.replace("Base.USDC", &format!("{chain}.USDC"));
        parse_source(&source).unwrap_or_else(|e| panic!("{chain}: {e}"));
    }
}
