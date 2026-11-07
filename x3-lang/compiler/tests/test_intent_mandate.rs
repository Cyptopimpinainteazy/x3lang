//! `under mandate <owner>` is an adapter binding, not a generic VM guard.
//!
//! The parser must preserve the owner for intent hashing and host resolution,
//! while the canonical resolver path must refuse to pretend that a local VM
//! execution checked a mandate it has no store for.

use x3_lang_ast::ast::Item;
use x3_lang_compiler::intent_emit::from_intent_decl;
use x3_lang_compiler::parser::parse_source;
use x3_lang_compiler::{compile_to_ir, Operation};

const SOURCE: &str = r#"
intent agent_payment {
    from Solana.USDC amount 25000000 receiver 4Nd1mzi8Y1QYxJt9wZWBYZpG7S4pYkZs6YzD3Vt9aBcD
    to Near.USDC receiver x3-demo.near
    route {
        bridge NearIntents Solana.USDC -> Near.USDC min_receive 24800000;
    }
    under mandate x3-owner.near
    require slippage <= 0.5%
    require nonce unused agent_payment_001
    timeout 10m refund Solana.USDC
    on_fail rollback
}
"#;

fn intent(source: &str) -> x3_lang_ast::ast::IntentDecl {
    let program = parse_source(source).expect("the mandate-bound intent must parse");
    program
        .items
        .into_iter()
        .find_map(|item| match item.node {
            Item::IntentDecl(intent) => Some(intent),
            _ => None,
        })
        .expect("the source declares one intent")
}

#[test]
fn mandate_is_preserved_in_the_ast_and_adapter_draft() {
    let parsed = intent(SOURCE);
    assert_eq!(
        parsed.mandate.as_ref().map(|owner| owner.as_str()),
        Some("x3-owner.near")
    );

    let draft = from_intent_decl(&parsed);
    assert_eq!(draft.mandate_owner.as_deref(), Some("x3-owner.near"));
    assert!(
        draft
            .constraints
            .iter()
            .any(|constraint| constraint.kind == "slippage" && constraint.arg == "0.5%"),
        "the signed adapter payload must carry one percent sign: {draft:?}"
    );
    assert!(
        draft.constraints.iter().all(|constraint| constraint.arg != "0.5%%"),
        "a percent literal already contains its `%`: {draft:?}"
    );
}

#[test]
fn formatter_round_trips_the_mandate_binding() {
    let program = parse_source(SOURCE).expect("the source must parse");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(
        formatted.contains("    under mandate x3-owner.near\n"),
        "the formatter must preserve the authority binding:\n{formatted}"
    );
    let reparsed = intent(&formatted);
    assert_eq!(
        reparsed.mandate.as_ref().map(|owner| owner.as_str()),
        Some("x3-owner.near")
    );
}

#[test]
fn lowering_hands_the_mandate_to_the_intent_resolver() {
    let program = parse_source(SOURCE).expect("the source must parse");
    let ir = compile_to_ir(&program).expect("lowering preserves adapter metadata without deciding authority");
    assert!(
        ir.operations.iter().any(|operation| matches!(
            operation,
            Operation::IntentResolve { constraints, .. }
                if constraints.iter().any(|value| value == "under mandate x3-owner.near")
        )),
        "the binding must not be dropped before the adapter boundary: {ir:?}"
    );
}

#[test]
fn mandate_is_not_lowered_as_a_fake_vm_guard() {
    let program = parse_source(SOURCE).expect("the source must parse");
    let ir = compile_to_ir(&program).expect("lowering must preserve the adapter binding");
    assert!(
        !ir.operations.iter().any(|operation| matches!(
            operation,
            Operation::Require {
                kind: x3_lang_compiler::RequireKind::Custom(kind),
                ..
            } if kind == "mandate"
        )),
        "the generic VM has no mandate store; recording a made-up guard would claim unperformed enforcement: {ir:?}"
    );
}

#[test]
fn malformed_owner_is_a_parse_error() {
    let error =
        parse_source(&SOURCE.replace("x3-owner.near", "Owner.near")).expect_err("NEAR named accounts are lowercase");
    assert!(
        error.to_string().contains("not a valid NEAR named account"),
        "the refusal must name the account rule: {error}"
    );
}

#[test]
fn duplicate_mandate_is_rejected() {
    let duplicate = SOURCE.replace(
        "    under mandate x3-owner.near\n",
        "    under mandate x3-owner.near\n    under mandate other-owner.near\n",
    );
    let error = parse_source(&duplicate).expect_err("one intent cannot name two authorities");
    assert!(
        error.to_string().contains("more than once"),
        "the refusal must name duplicate authority: {error}"
    );
}

#[test]
fn mandate_outside_an_intent_is_rejected() {
    let misplaced = format!("under mandate x3-owner.near\n{SOURCE}");
    let error = parse_source(&misplaced).expect_err("authority cannot be declared outside an intent");
    assert!(
        error.to_string().contains("expected top-level item"),
        "the refusal must name the invalid placement: {error}"
    );
}

#[test]
fn mandate_without_a_nonce_is_refused_before_admission() {
    let without_nonce = SOURCE.replace("    require nonce unused agent_payment_001\n", "");
    let (_, _, outcome) =
        x3_lang_compiler::check_source_diagnostics(&without_nonce).expect("the AST-level diagnostics must be returned");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.to_string().contains("require nonce unused")),
        "mandate admission must fail closed without replay protection: {outcome:?}"
    );
}
