//! Every statement the parser can produce is lowered or refused, and nothing is dropped
//! (TICKET-109).
//!
//! `lower_statement` ended in `_ => ir.push(Operation::Nop)` under the comment "Other statement
//! types (return, break, etc.)". That arm is why a program of `let` bindings, `return`s and calls
//! to a stdlib this language does not have **checked clean**: each statement became a `Nop`, which
//! the emitter writes as four zero bytes — bytes the compiler's own walker skips as padding and the
//! VM's verifier breaks on as the end of the stream. The artifact of `let a = 1; let c = a + b;`
//! was byte-identical to the artifact of an empty program, and `tests/test_arithmetic.x3` reported
//! `3 ops, no semantic errors`.
//!
//! Each construct is refused by name here. The *pair* matters: an arm that refuses everything is as
//! useless as one that drops everything, so the last test in this file lowers a program built from
//! the statements this compiler does support and asserts it reaches the IR.

use x3_lang_compiler::ir::{Condition, Operation};
use x3_lang_compiler::semantic::CompilationMode;

/// A strategy module whose `execute` body is `body`.
fn module(body: &str) -> String {
    format!(
        r#"strategy Statements {{
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
        require profit >= 5
{body}
        on_fail refund ethereum.USDC to sender
    }}
}}
"#
    )
}

/// The refusals a program produced, as rendered messages.
///
/// Two stages refuse here and they arrive differently: a statement the *lowerer* cannot place is an
/// `Err` from the check entry point, while a record the *verifier* will not let through lands in the
/// outcome's `errors`. A program's author sees one failing check either way, so the test reads both.
fn refusals(body: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(&module(body), CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![error.to_string()],
    }
}

#[test]
fn a_let_binding_is_refused_by_name_and_not_dropped() {
    // The measured defect: this used to check with `3 ops, no semantic errors`, and those three
    // operations were three `Nop`s.
    let messages = refusals("        let a = 1;\n        let b = 2;");
    assert_eq!(messages.len(), 1, "one statement, one refusal: {messages:?}");
    assert!(
        messages[0].contains("`let a = …`"),
        "the refusal must name the binding the program wrote: {}",
        messages[0]
    );
    assert!(
        messages[0].contains("no register holds a source-level binding"),
        "and say what is missing rather than only that it is unsupported: {}",
        messages[0]
    );
}

#[test]
fn a_return_a_break_and_a_continue_are_refused_by_name() {
    for (body, named) in [
        ("        return 1;", "`return`"),
        ("        break;", "`break`"),
        ("        continue;", "`continue`"),
    ] {
        let messages = refusals(body);
        assert_eq!(messages.len(), 1, "`{body}`: {messages:?}");
        assert!(
            messages[0].contains(named) && messages[0].contains("would be dropped"),
            "`{body}` must be refused as the construct it is, and as a *drop*: {}",
            messages[0]
        );
    }
}

#[test]
fn a_for_loop_is_refused_by_the_iterable_it_names() {
    let messages = refusals("        for i in steps {\n            require profit >= 5\n        }");
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert!(
        messages[0].contains("`for … in steps`"),
        "the refusal must name the iterable the program wrote: {}",
        messages[0]
    );
}

#[test]
fn a_bare_loop_lowers_and_is_refused_where_while_true_is() {
    // `loop { … }` is the unbounded loop `while true` spells, so it takes the same path rather than
    // a second refusal: it reaches the IR with a decided-true condition and the one place that words
    // this refusal — the IR verifier and the emitter — names it.
    let source = module("        loop {\n            require profit >= 5\n        }");
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev).expect("`loop` must lower");
    let looped = ir
        .operations
        .iter()
        .find_map(|operation| match operation {
            Operation::Loop { condition, body, .. } => Some((condition.clone(), body.len())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("a bare `loop` must reach the IR as a loop: {:?}", ir.operations));
    assert!(
        matches!(looped.0, Condition::True),
        "an unbounded loop is the condition `true`: {:?}",
        looped.0
    );
    assert_eq!(looped.1, 1, "and its body stays in the IR beside it");
    assert!(
        outcome
            .errors
            .iter()
            .any(|error| error.to_string().contains("`loop` over `true`")),
        "refused by the same message `while true` gets: {:?}",
        outcome.errors
    );
}

#[test]
fn the_statements_this_compiler_does_support_still_reach_the_ir() {
    // The pair that stops "refuse everything" from passing the tests above: the same module with
    // only supported statements lowers and checks.
    let source = module("        mempool_scan(max_results=10);");
    let (_, ir, outcome) = x3_lang_compiler::check_source_diagnostics_with_mode(&source, CompilationMode::Dev)
        .expect("a supported statement must lower");
    assert!(outcome.errors.is_empty(), "{:?}", outcome.errors);
    assert!(
        ir.operations
            .iter()
            .any(|operation| matches!(operation, Operation::MempoolScan { .. })),
        "the statement the program wrote is in the IR: {:?}",
        ir.operations
    );
    assert!(
        !ir.operations
            .iter()
            .any(|operation| matches!(operation, Operation::Nop)),
        "and nothing lowered to a `Nop`: {:?}",
        ir.operations
    );
}

/// Nothing in `lowering.rs` constructs an operation whose record no reader can see.
///
/// `Operation::Nop` is written as four zero bytes, which the compiler's own instruction walker skips
/// as padding and the VM's verifier stops at as the end of the stream — so a `Nop` in the
/// *lowerer* is a statement or an annotation that left no trace, which is what TICKET-109
/// (`_ => push(Nop)`) and TICKET-110 (`@gas_adaptive`'s placeholder bodies) both were. `Operation::Nop`
/// is still a record the IR may carry — a hand-built IR, or a future construct with nothing to say —
/// and this file asserts the *lowerer* is not where one comes from.
///
/// A source scan rather than a compile-time check because the property is about the source: a `Nop`
/// construction is legal Rust and compiles, and its failure is a program that checks clean and runs
/// as if a statement had not been written.
#[test]
fn the_lowerer_constructs_no_invisible_operation() {
    const LOWERING: &str = include_str!("../src/lowering.rs");
    let code_lines = LOWERING.lines().filter(|line| !line.trim_start().starts_with("//"));
    let invisible: Vec<&str> = code_lines
        .filter(|line| line.contains("Operation::Nop") || line.contains("Operation::GasAdaptive {"))
        .collect();
    assert!(
        invisible.is_empty(),
        "these lines construct an operation whose record no reader can see: {invisible:?}"
    );
}
