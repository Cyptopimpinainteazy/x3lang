//! `risk_policy` means what it says, in one unit (TICKET-052).
//!
//! `max_slippage` used to lower to a `RiskScore`, whose own range is 0..=100 —
//! so a program that bounded slippage at 120 (a percentage bound, which is what
//! the language's guards write) produced a risk score the VM refuses and the
//! program stopped running. It is a slippage ceiling: the same kind and unit as
//! `require slippage <= n`, which is also what the mainnet ceiling reads.

use x3_lang_compiler::ir::{ComparisonOp, Condition, Operation, RequireKind};
use x3_lang_compiler::semantic::CompilationMode;

fn intent_with(policy: &str, guard: &str) -> String {
    format!(
        "{policy}intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {{\n        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }}\n    {guard}\n    on_fail refund ethereum.USDC to sender\n}}\n"
    )
}

fn slippage_bounds(ir: &x3_lang_compiler::ir::X3IR) -> Vec<String> {
    ir.operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::Require {
                kind: RequireKind::SlippageTolerance,
                condition: Condition::Expression { expr },
                comparison,
                ..
            } => Some(format!("{expr} ({comparison:?})")),
            _ => None,
        })
        .collect()
}

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics(source) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

#[test]
fn a_slippage_policy_is_a_slippage_ceiling_and_not_a_risk_score() {
    let source = intent_with("risk_policy {\n    max_slippage 120\n}\n\n", "require slippage <= 120");
    let program = x3_lang_compiler::parser::parse_source(&source).expect("it must parse");
    let ir = x3_lang_compiler::compile_to_ir(&program).expect("it must lower");
    assert_eq!(
        slippage_bounds(&ir),
        vec![
            "120 (Some(LessOrEqual))".to_string(),
            "120 (Some(LessOrEqual))".to_string()
        ],
        "the policy and the guard are the same kind of claim in the same unit"
    );
    assert!(
        !ir.operations
            .iter()
            .any(|operation| matches!(operation, Operation::RiskScore { .. })),
        "a slippage bound is not a risk score: {:?}",
        ir.operations
    );
}

#[test]
fn an_unstated_slippage_policy_emits_no_ceiling() {
    // The field is not optional in the AST, so zero is the absence — and a
    // program that declares no slippage policy must not acquire a ceiling of 0.
    let source = intent_with("risk_policy {\n    min_route_score 90\n}\n\n", "require slippage <= 50");
    let program = x3_lang_compiler::parser::parse_source(&source).expect("it must parse");
    let ir = x3_lang_compiler::compile_to_ir(&program).expect("it must lower");
    assert_eq!(
        slippage_bounds(&ir),
        vec!["50 (Some(LessOrEqual))".to_string()],
        "only the guard states a bound"
    );
}

#[test]
fn the_policy_reaches_the_mainnet_ceiling() {
    // The safety half of the fix: as a `RiskScore` the policy was invisible to
    // the mainnet slippage check, which reads exactly this operation. 600 is over
    // the ceiling in the unit that check uses — basis points, so 600 is 6.00%.
    //
    // That the *linter* reads the same literal as a whole percent (and multiplies
    // by 100 to get basis points) is a separate disagreement between three
    // readers of one quantity; it is recorded as TICKET-054 rather than decided
    // here, and this test pins what the mainnet gate actually does.
    let source = intent_with("risk_policy {\n    max_slippage 600\n}\n\n", "require slippage <= 600");
    let error = x3_lang_compiler::compile_with_mode(&source, CompilationMode::Mainnet)
        .expect_err("a 600 bps policy is over the 5% mainnet ceiling")
        .to_string();
    assert!(
        error.contains("slippage tolerance 6.00% exceeds maximum 5%"),
        "got: {error}"
    );
}

#[test]
fn a_guard_looser_than_the_policy_is_refused() {
    // The policy says "no route slips more than 5"; a guard that permits 50
    // permits what the policy forbids, so the program has said two things about
    // one quantity.
    let source = intent_with("risk_policy {\n    max_slippage 5\n}\n\n", "require slippage <= 50");
    let errors = errors(&source);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("permits a slippage of 50") && error.contains("at most 5")),
        "{errors:?}"
    );
}

#[test]
fn a_guard_inside_the_policy_is_fine() {
    for guard in ["require slippage <= 5", "require slippage <= 3"] {
        let source = intent_with("risk_policy {\n    max_slippage 5\n}\n\n", guard);
        assert_eq!(errors(&source), Vec::<String>::new(), "{guard} is within the policy");
    }
}
