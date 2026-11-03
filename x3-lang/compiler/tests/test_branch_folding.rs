//! A branch the compiler can decide is folded, and the branch that runs is the one written
//! (TICKET-058).
//!
//! `if`/`loop` were refused outright because neither half of the pipeline could walk a branch
//! record: the VM branches on a register and skips whole four-byte instructions, while a
//! compiler stream frames instructions with a width that varies and pads them to absolute
//! boundaries. A condition the compiler can decide removes the problem instead of solving it —
//! there is nothing to test at run time and nothing to jump over, so the taken branch's
//! instructions go into the stream where the writer pads them and every reader can walk them.
//!
//! What these tests hold is the boundary of that: which conditions are decided, that a decided
//! one is decided *correctly*, that a condition the compiler cannot decide is still refused, and
//! that neither an overflow nor a division by zero is ever rounded into a decision.

use x3_lang_compiler::ir::{Condition, Operation};
use x3_lang_compiler::semantic::CompilationMode;

/// A module whose `execute` body holds a branch over `condition`, with branches of different
/// lengths so that which one was taken is visible in the instruction count.
fn module(condition: &str) -> String {
    format!(
        r#"strategy Folded {{
    input ethereum.USDC amount 25_000_000 max 50_000_000
    output ethereum.ETH
    effects [swap]
    guarantees [min_profit]
    domains [ethereum]
    risk {{ max_slippage_bps 50 max_total_fee_bps 8 }}
    bounds {{ max_steps 10 max_gas 200_000 }}
    execute {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1
        require slippage <= 50
        if {condition} {{
            require profit >= 5
        }} else {{
            require profit >= 5
            require profit >= 99
        }}
        on_fail refund ethereum.USDC to sender
    }}
}}
"#
    )
}

fn lowered(condition: &str) -> Vec<Operation> {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(&module(condition), CompilationMode::Dev)
            .unwrap_or_else(|error| panic!("`if {condition}` must lower: {error}"));
    assert!(
        outcome.errors.is_empty(),
        "`if {condition}` must check: {:?}",
        outcome.errors
    );
    ir.operations
}

/// The condition the branch carries, and the number of operations in each body.
fn branch(condition: &str) -> (Condition, usize, usize) {
    lowered(condition)
        .into_iter()
        .find_map(|operation| match operation {
            Operation::If {
                condition,
                then_ops,
                else_ops,
            } => Some((condition, then_ops.len(), else_ops.map_or(0, |body| body.len()))),
            _ => None,
        })
        .unwrap_or_else(|| panic!("`if {condition}` must reach the IR as a branch"))
}

#[test]
fn a_condition_the_program_states_is_folded_to_the_branch_that_runs() {
    // `true` on the left of `||` decides it whatever the right side is, and `false` on the left
    // of `&&` decides it too — those are the language's short-circuit meanings, and a folder that
    // needed both sides would refuse them.
    let true_cases = [
        "true",
        "1 > 0",
        "1 == 1",
        "1 <= 1",
        "2 >= 2",
        "1 != 2",
        "!(1 > 2)",
        "true || steps > 0",
        "1 > 0 && 2 > 1",
    ];
    for case in true_cases {
        let (condition, then_len, else_len) = branch(case);
        assert!(
            matches!(condition, Condition::True),
            "`if {case}` is decidable and true, so the IR must say so rather than carry an \
             expression a reader would have to evaluate"
        );
        assert_eq!((then_len, else_len), (1, 2), "and both bodies stay in the IR");
    }

    let false_cases = ["false", "1 > 2", "1 == 2", "2 <= 1", "!(1 < 2)", "false && steps > 0"];
    for case in false_cases {
        let (condition, then_len, else_len) = branch(case);
        assert!(
            matches!(condition, Condition::False),
            "`if {case}` is decidable and false, so the IR must say so"
        );
        assert_eq!((then_len, else_len), (1, 2), "and both bodies stay in the IR");
    }
}

/// The branch that was *not* taken stays in the IR. Dropping it would leave an artifact whose
/// reader cannot tell a folded branch from straight-line code — and `x3c lower` is where a
/// reader looks.
#[test]
fn the_branch_that_was_not_taken_stays_in_the_ir() {
    let (_, then_len, else_len) = branch("1 > 0");
    assert_eq!(
        (then_len, else_len),
        (1, 2),
        "the body that did not run is still the compiler's record of what the program said"
    );
}

#[test]
fn a_condition_the_compiler_cannot_decide_is_refused_not_guessed() {
    let source = module("steps > 0");
    let outcome = x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev);
    let (_, ir, outcome) = outcome.expect("it must lower far enough to be refused");
    assert!(
        !outcome.errors.is_empty(),
        "an undecidable condition must not check: this VM cannot branch on it"
    );
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.to_string().contains("not decidable at compile time")),
        "and the refusal must name the reason: {:?}",
        outcome.errors
    );
    assert!(
        matches!(branch_of(&ir.operations), Some(Condition::Expression { .. })),
        "while the IR still says what the condition was"
    );

    // And the emitter refuses it too, because `emit_x3ir` is public: a caller that assembles an
    // IR by hand must not be able to write a branch record no reader can follow.
    let error = x3_lang_compiler::emitter::emit_x3ir(&ir).expect_err("an undecidable branch must not be emitted");
    assert!(
        error.to_string().contains("not decidable at compile time"),
        "and it must say why: {error}"
    );
}

/// Neither an overflow nor a division by zero is ever rounded into a decision.
///
/// This is the failing edge the folder exists to have: a branch decided from a wrapped number is
/// decided *wrongly*, which is worse than one that refuses, because nothing downstream can tell.
#[test]
fn arithmetic_that_does_not_have_a_value_is_not_decided() {
    let u128_max = u128::MAX;
    let cases = [
        // A division by zero has no value, so the comparison has none either.
        "1 / 0 > 0".to_string(),
        "1 % 0 > 0".to_string(),
        // `u128::MAX * 2` does not fit, and neither does `u128::MAX + 1`.
        format!("{u128_max} * 2 > 0"),
        format!("{u128_max} + 1 > 0"),
        // Subtraction below zero wraps in a `u128`, which would decide the branch wrongly.
        "1 - 2 > 0".to_string(),
    ];
    for case in cases {
        let source = module(&case);
        let (_, ir, outcome) = x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev)
            .unwrap_or_else(|error| panic!("`if {case}` must at least parse: {error}"));
        assert!(
            !outcome.errors.is_empty(),
            "`if {case}` has no decidable value, so it must be refused rather than decided"
        );
        assert!(
            matches!(branch_of(&ir.operations), Some(Condition::Expression { .. })),
            "`if {case}` must stay an expression"
        );
    }
}

fn branch_of(ops: &[Operation]) -> Option<Condition> {
    ops.iter().find_map(|operation| match operation {
        Operation::If { condition, .. } => Some(condition.clone()),
        _ => None,
    })
}
