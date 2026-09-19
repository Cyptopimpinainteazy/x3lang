//! One slippage literal, one unit (TICKET-054).
//!
//! A bare number is **basis points** and a percent literal is a **percentage**,
//! and the two agree where they name the same bound: `<= 50` and `<= 0.5%` are
//! both fifty basis points. Three readers used to disagree — the mainnet ceiling
//! read a bare number as bps, the risk scorer multiplied it by 100 as though it
//! were a percent, and the strategy check compared it against a
//! `max_slippage_bps` field — so the corpus's most common bound,
//! `require slippage <= 50`, was 0.5% to one reader and 50% to another.

use x3_lang_ast::ast::{Expression, LiteralExpr};
use x3_lang_common::Symbol;
use x3_lang_compiler::risk::RiskScorer;
use x3_lang_compiler::semantic::{bound_bps_from_expr, slippage_bps_from_text, CompilationMode};

#[test]
fn a_bare_number_is_basis_points_and_a_percent_is_a_percentage() {
    // The two spellings, and the place they meet.
    assert_eq!(slippage_bps_from_text("50"), Some(50));
    assert_eq!(slippage_bps_from_text("0.5%"), Some(50), "0.5% is fifty basis points");
    assert_eq!(slippage_bps_from_text("0.05%"), Some(5));
    assert_eq!(slippage_bps_from_text("5%"), Some(500));
    assert_eq!(slippage_bps_from_text("500"), Some(500), "5% and 500 bps are one bound");
}

#[test]
fn a_fraction_of_a_basis_point_is_refused_rather_than_rounded() {
    // `<= 50.5` is 50 and a half basis points: not a number this language can
    // compare, so the reader gets nothing and a check that needs one says so.
    assert_eq!(slippage_bps_from_text("50.5"), None);
    assert_eq!(slippage_bps_from_text("0.005%"), None);
    // A fractional part of zero is not a fraction.
    assert_eq!(slippage_bps_from_text("50.0"), Some(50));
    assert_eq!(slippage_bps_from_text("0.50%"), Some(50));
}

#[test]
fn the_expression_form_reads_the_same_way() {
    let int = Expression::Literal(LiteralExpr::Int {
        value: 50,
        base: x3_lang_common::IntBase::Decimal,
        suffix: None,
    });
    let percent = Expression::Literal(LiteralExpr::Percentage {
        value: Symbol::new("0.5%"),
    });
    assert_eq!(bound_bps_from_expr(&int), Some(50));
    assert_eq!(bound_bps_from_expr(&percent), Some(50));
    assert_eq!(
        bound_bps_from_expr(&int),
        bound_bps_from_expr(&percent),
        "one bound, two spellings"
    );
}

fn intent(guard: &str) -> String {
    format!(
        "intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {{\n        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }}\n    {guard}\n    on_fail refund ethereum.USDC to sender\n}}\n"
    )
}

#[test]
fn the_risk_scorer_does_not_call_half_a_percent_high_slippage() {
    // The reproduction: `require slippage <= 50` was scored as
    // `high slippage (5000bps / 50.00%)` and given the worst slippage score.
    let program = x3_lang_compiler::parser::parse_source(&intent("require slippage <= 50")).expect("it must parse");
    let report = RiskScorer::new().score_program(&program);
    assert_eq!(
        report.categories.get("slippage_risk"),
        Some(&5),
        "fifty basis points is a tight bound: {report:?}"
    );
    assert!(
        !report.details.iter().any(|detail| detail.contains("high slippage")),
        "and it is not reported as high slippage: {report:?}"
    );
}

#[test]
fn the_mainnet_ceiling_reads_the_same_unit_as_the_guards() {
    // 5% is the ceiling, so 500 bps passes and 501 does not — and the percent
    // spelling of the same bound behaves the same way.
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples/mainnet_safe_swap.x3");
    let flagship = std::fs::read_to_string(&path).expect("the example must be readable");

    // The policy is raised with the guard in each case, so the ceiling is the
    // check under test rather than the policy/guard consistency check above it.
    // The check refuses only what is *above* the ceiling (`bound > policy`), so a
    // bound equal to it is inside. Two rows here used to disagree with that and
    // passed anyway: `501` against a `501` ceiling is equal and so is accepted, and
    // `5%` against a `500` ceiling is 500 bps against 500 and is accepted too — but
    // the second was "refused" only because a whole-number percent guard did not
    // parse at all (TICKET-089), so the assertion was satisfied by a parse failure
    // rather than by the comparison it names. Fixing that parse is what exposed the
    // table. Each row below is now a claim about the ceiling, and the two percent
    // rows pin that `5%` and `0.5%` are 500 and 50 basis points.
    for (guard, policy, accepted) in [
        ("require slippage <= 500", "500", true),
        ("require slippage <= 0.5%", "50", true),
        ("require slippage <= 5%", "500", true),
        ("require slippage <= 501", "500", false),
        ("require slippage <= 5%", "499", false),
    ] {
        let source = flagship
            .replace("max_slippage 5", &format!("max_slippage {policy}"))
            .replace("require slippage <= 5", guard);
        let outcome = x3_lang_compiler::compile_with_mode(&source, CompilationMode::Mainnet);
        assert_eq!(
            outcome.is_ok(),
            accepted,
            "`{guard}` must {} be inside the ceiling: {:?}",
            if accepted { "" } else { "not" },
            outcome.err().map(|error| error.to_string())
        );
    }
}

#[test]
fn a_guard_and_the_policy_above_it_are_compared_in_one_unit() {
    fn errors(source: &str) -> Vec<String> {
        match x3_lang_compiler::check_source_diagnostics(source) {
            Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
            Err(error) => vec![format!("{error}")],
        }
    }
    let with_policy =
        |policy: &str, guard: &str| format!("risk_policy {{\n    max_slippage {policy}\n}}\n\n{}", intent(guard));

    // The same bound written two ways is the same bound.
    assert_eq!(
        errors(&with_policy("50", "require slippage <= 0.5%")),
        Vec::<String>::new()
    );
    assert_eq!(
        errors(&with_policy("50", "require slippage <= 50")),
        Vec::<String>::new()
    );
    // And a guard looser than the policy is still refused.
    let refused = errors(&with_policy("5", "require slippage <= 0.5%"));
    assert!(
        refused.iter().any(|error| error.contains("permits a slippage of 50")),
        "0.5% is fifty basis points, above a five-basis-point policy: {refused:?}"
    );
}
