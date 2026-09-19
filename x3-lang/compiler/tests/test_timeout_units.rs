//! Durations in timeouts — spec PHASE 42's "no ambiguous units" rule, applied to
//! the one place it was not (TICKET-033).
//!
//! A timeout is the window in which a claim can be made, so the two things that
//! matter are that the unit a program writes is the unit that is enforced, and
//! that the conversion errs towards the *longer* window: an HTLC whose source
//! deadline is shorter than the program asked for is the dangerous direction,
//! because the time it needed is the time it lost.

use x3_lang_compiler::ir::Operation;
use x3_lang_compiler::lowering::SECONDS_PER_BLOCK;

/// The blocks a timeout in the given source denotes, or the error it raises.
fn offset_blocks(timeout: &str, action: &str) -> Result<u32, String> {
    let source =
        format!("intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n    {timeout} {action}\n}}\n");
    let program = x3_lang_compiler::parser::parse_source(&source).map_err(|error| format!("{error}"))?;
    let ir = x3_lang_compiler::compile_to_ir(&program).map_err(|error| format!("{error}"))?;
    ir.operations
        .iter()
        .find_map(|operation| match operation {
            Operation::OnTimeout { duration_blocks, .. } => Some(*duration_blocks),
            _ => None,
        })
        .ok_or_else(|| "no OnTimeout operation was lowered".to_string())
}

fn blocks(timeout: &str) -> u32 {
    offset_blocks(timeout, "refund ethereum.USDC to sender")
        .unwrap_or_else(|error| panic!("`{timeout}` must lower: {error}"))
}

#[test]
fn a_timeout_written_in_time_is_the_time_it_says() {
    assert_eq!(SECONDS_PER_BLOCK, 6, "the block time is a documented fact, not a guess");
    // Seconds.
    assert_eq!(blocks("timeout 180s"), 30);
    assert_eq!(blocks("timeout 3600s"), 600);
    // Minutes and hours: the units the corpus writes and the units that were
    // silently dropped. `40m` meant forty *blocks*.
    assert_eq!(blocks("timeout 40m"), 400);
    assert_eq!(blocks("timeout 20m"), 200);
    assert_eq!(blocks("timeout 2h"), 1_200);
    assert_eq!(blocks("timeout 1d"), 14_400);
}

#[test]
fn a_bare_number_is_still_a_count_of_blocks() {
    // The other half of the contract, and the form every program that predates
    // units is written in.
    assert_eq!(blocks("timeout 40"), 40);
    assert_eq!(blocks("timeout 40 blocks"), 40);
}

#[test]
fn a_duration_that_is_not_a_whole_number_of_blocks_gets_the_extra_one() {
    // 45s is 7.5 blocks. Rounding down would give a window of 42 seconds to a
    // program that asked for 45; rounding up gives 48. Only one of those can
    // strand a claim.
    assert_eq!(blocks("timeout 45s"), 8);
    assert_eq!(blocks("timeout 41s"), 7);
    assert_eq!(blocks("timeout 500ms"), 1, "a sub-block window is one block, not none");
    assert_eq!(blocks("timeout 3_000ms"), 1);
    assert_eq!(blocks("timeout 6_000ms"), 1);
    assert_eq!(blocks("timeout 7_000ms"), 2);
}

#[test]
fn a_unit_the_language_does_not_define_is_refused() {
    // The defect was that the suffix was *decorative*: `40x` became 40 blocks.
    // A program that says something about time and nothing about blocks has to
    // be told so, next to the word it wrote.
    let error = offset_blocks("timeout 40x", "refund ethereum.USDC to sender")
        .expect_err("an undefined unit cannot be silently dropped");
    assert!(error.contains("does not define"), "got: {error}");
    assert!(error.contains("40x"), "the message must name what was written: {error}");
}

#[test]
fn a_fractional_duration_is_refused() {
    let error = offset_blocks("timeout 40.5s", "refund ethereum.USDC to sender")
        .expect_err("a fractional duration is not a block count");
    assert!(error.contains("fractional"), "got: {error}");
}

#[test]
fn the_ceiling_is_derived_from_the_block_time() {
    // 24 hours of blocks, computed from the same constant the conversion uses,
    // so the two cannot disagree about what a day is.
    assert_eq!(x3_lang_compiler::lowering::MAX_TIMEOUT_BLOCKS, 14_400);
    assert_eq!(blocks("timeout 1d"), x3_lang_compiler::lowering::MAX_TIMEOUT_BLOCKS);
    let error = offset_blocks("timeout 2d", "refund ethereum.USDC to sender")
        .expect_err("two days of blocks is over the ceiling");
    assert!(error.contains("exceeds maximum"), "got: {error}");
}

#[test]
fn the_atomic_swap_ordering_invariant_compares_what_it_enforces() {
    // The invariant is "the source window outlasts the destination window", and
    // it used to read the same expression as *seconds* while lowering read it as
    // blocks — and skip an expression it could not read (any unit at all).
    let ordered = "atomic swap eth.USDC -> sol.SOL {\n    amount 500\n    receiver sol.wallet.owner\n    \
                   hashlock sha256(secret)\n    timeout source 40m\n    timeout destination 20m\n    \
                   require finality.eth >= 12\n}\n";
    let program = x3_lang_compiler::parser::parse_source(ordered).expect("ordered timeouts must parse");
    x3_lang_compiler::compile_to_ir(&program).expect("400 blocks outlasts 200");

    let inverted = ordered
        .replace("timeout source 40m", "timeout source 20m")
        .replace("timeout destination 20m", "timeout destination 40m");
    // Through the checking entry point: the invariant is an AST-level pass, so a
    // test that only lowers would pass while the pass it is about never ran.
    let errors = x3_lang_compiler::check_source_diagnostics(&inverted)
        .err()
        .map(|error| error.to_string())
        .unwrap_or_else(|| {
            let (_, _, outcome) =
                x3_lang_compiler::check_source_diagnostics(&inverted).expect("the inverted program parses");
            outcome
                .errors
                .iter()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
                .join("; ")
        });
    assert!(
        errors.contains("must be greater than destination"),
        "a destination that outlasts its source strands the claim: {errors}"
    );
}
