//! A `while` states what it tests, and a `while` the compiler can decide is decided (TICKET-098).
//!
//! `Statement::While` used to lower with `let _cond_ir = expression_to_condition(cond)?;` — the
//! condition was parsed, converted and thrown away, so the IR carried a loop with a fixed
//! iteration cap and **nothing saying what it looped on**. `x3c lower` printed that loop, and the
//! refusal that the verifier and the emitter both raise could not name the guard it was refusing:
//! with several `while`s in a program, its reader had to guess which one was the problem.
//!
//! Two facts are held here. The condition travels, so a reader can see the guard the program
//! wrote; and the class the compiler can decide is decided rather than dropped — `while <false>`
//! is a body that never runs, written as nothing, which is the same treatment an `if` decided
//! false with no `else` already gets. What is *not* decided is refused by name.

use x3_lang_compiler::emitter::emit_x3ir;
use x3_lang_compiler::ir::{Condition, Operation};
use x3_lang_compiler::semantic::CompilationMode;
use x3_lang_compiler::verify::verify_ir;

/// A module whose `execute` body holds `statement` after a swap whose slippage guard is the
/// program's one unconditional requirement.
fn module(statement: &str) -> String {
    format!(
        r#"strategy Loops {{
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
        {statement}
        on_fail refund ethereum.USDC to sender
    }}
}}
"#
    )
}

/// Lower a program far enough to look at the loop it produced.
fn lowered(source: &str) -> (x3_lang_compiler::ir::X3IR, x3_lang_compiler::semantic::VerifyOutcome) {
    let (_, ir, outcome) = x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev)
        .unwrap_or_else(|error| panic!("the program must lower: {error}"));
    (ir, outcome)
}

/// The loop in a lowered program, as (condition, body length).
fn loop_of(ir: &x3_lang_compiler::ir::X3IR) -> (Condition, usize) {
    ir.operations
        .iter()
        .find_map(|operation| match operation {
            Operation::Loop { condition, body, .. } => Some((condition.clone(), body.len())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the program must reach the IR as a loop: {:?}", ir.operations))
}

#[test]
fn a_loop_carries_the_condition_the_program_wrote() {
    // The defect: this returned `Loop { max_iterations: 1000 }` and the condition was gone, so
    // `x3c lower` — the surface the ledger names as where a reader looks — could not show that
    // the loop tested anything at all.
    let (ir, _) = lowered(&module("while steps < 10 { require profit >= 5 }"));
    let (condition, body_len) = loop_of(&ir);
    match &condition {
        Condition::Expression { expr } => {
            assert_eq!(expr, "steps < 10", "the guard, in the program's own spelling")
        }
        other => panic!("the loop must carry the guard the program wrote, and carried {other:?}"),
    }
    assert_eq!(body_len, 1, "and the body stays in the IR beside it");
}

#[test]
fn a_loop_the_compiler_decides_false_is_written_as_nothing() {
    // `while 1 > 2 { … }` never runs its body: that is the language's meaning, and the folder may
    // decide it because it only decides literals, arithmetic on them and logical combinations —
    // nothing in the decision is a call with a side effect.
    //
    // Both programs discharge the module's `min_profit` guarantee, so the only difference between
    // them is the loop. A comparison against a program that does not compile would pass vacuously
    // — the empty artifact on the right of this comparison is the trap, which is why the
    // without-loop artifact is asserted non-empty before the two are compared.
    let (ir, outcome) = lowered(&module(
        "require profit >= 5\n        while 1 > 2 { require profit >= 9 }",
    ));
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    let (condition, body_len) = loop_of(&ir);
    assert!(
        matches!(condition, Condition::False),
        "the compiler must decide it, and decided {condition:?}"
    );
    assert_eq!(body_len, 1, "and keep the body it decided not to run");

    // Nothing is refused, because there is nothing to execute and nothing to jump back to. Other
    // diagnostics are not this test's subject, so the loop is what is asserted absent.
    if let Err(diagnostics) = verify_ir(&ir) {
        assert!(
            !diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("`loop`")),
            "a loop decided false is not a construct the VM has to execute: {diagnostics:?}"
        );
    }

    // And the artifact is byte-for-byte what the same program without the loop produces: the body
    // contributes nothing, which is the claim. A difference here would mean the body was written
    // after all, or that the compiler wrote a record for a loop that never runs.
    let with_loop = emit_x3ir(&ir).expect("a loop decided false is emittable");
    let (without_loop_ir, without_loop_outcome) = lowered(&module("require profit >= 5"));
    assert!(
        without_loop_outcome.errors.is_empty(),
        "the program this is compared against has to be valid, or the comparison proves nothing: \
         {:?}",
        without_loop_outcome.errors
    );
    let without_loop = emit_x3ir(&without_loop_ir).expect("the same program without the loop must also emit");
    assert!(
        !without_loop.is_empty(),
        "and it must not be an empty artifact, which would make the comparison vacuous"
    );
    assert_eq!(
        with_loop, without_loop,
        "a loop that never runs must not reach the artifact"
    );
}

#[test]
fn a_condition_the_compiler_cannot_decide_is_refused_by_name() {
    // The second half of the ticket: a refusal must name the guard it could not evaluate, not
    // only the construct. Both refusals are checked because `emit_x3ir` is public — a caller that
    // assembles an IR by hand gets the same answer as the compiler's own `check`.
    let (ir, outcome) = lowered(&module("while steps < 10 { require profit >= 5 }"));
    let named = "`loop` over `steps < 10`";
    assert!(
        outcome.errors.iter().any(|error| error.to_string().contains(named)),
        "the verifier's refusal must name the guard: {:?}",
        outcome.errors
    );

    let error = emit_x3ir(&ir).expect_err("an undecidable loop must not be emitted");
    assert!(error.to_string().contains(named), "and so must the emitter's: {error}");
}

#[test]
fn a_loop_the_compiler_decides_true_is_refused_rather_than_dropped() {
    // The dangerous half of folding: a `while true` is not "not decidable", so a folder that
    // treated every decision as a licence to drop the loop would silently delete a program's
    // body. It is refused, and the refusal says which guard it is.
    let (ir, outcome) = lowered(&module("while true { require profit >= 5 }"));
    let (condition, _) = loop_of(&ir);
    assert!(
        matches!(condition, Condition::True),
        "the condition is decided, and it is true: {condition:?}"
    );
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.to_string().contains("`loop` over `true`")),
        "a loop that would run for ever must be refused by name: {:?}",
        outcome.errors
    );
    let error = emit_x3ir(&ir).expect_err("a loop decided true must not be emitted");
    assert!(error.to_string().contains("`loop` over `true`"), "{error}");
}
