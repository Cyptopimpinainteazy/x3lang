//! A fractional amount is refused where it is written (TICKET-047).
//!
//! `expression_to_u128` has refused a fractional literal since PHASE 43 — an amount
//! is converted only when the conversion is exact, and the exact path goes through
//! an asset's declared decimals. But the swap lowering called the *non-erroring*
//! wrapper and wrote `0` for anything it could not convert, so `min_output 0.09`
//! reached the verifier as "min_output must be greater than zero": a report about a
//! zero, three passes after the literal that caused it, naming neither the literal
//! nor the reason.

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics(source) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

fn intent_with_min_output(min_output: &str) -> String {
    format!(
        "intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {{\n        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output {min_output}\n    }}\n    require slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}}\n"
    )
}

#[test]
fn a_fractional_min_output_is_refused_by_its_own_literal() {
    let errors = errors(&intent_with_min_output("0.09"));
    assert_eq!(errors.len(), 1, "one literal, one refusal: {errors:?}");
    assert!(
        errors[0].contains("`0.09`"),
        "the message must name the literal: {errors:?}"
    );
    assert!(
        errors[0].contains("fractional literal") && errors[0].contains("exact"),
        "and say why, and where the exact path is: {errors:?}"
    );
    assert!(
        !errors[0].contains("must be greater than zero"),
        "the verifier's report about a zero is what this replaces: {errors:?}"
    );
}

#[test]
fn an_integer_amount_is_untouched() {
    assert_eq!(errors(&intent_with_min_output("900")), Vec::<String>::new());
}

#[test]
fn a_written_zero_is_refused_for_being_zero_and_says_so() {
    // The distinction the fix draws: a literal that cannot be *read* as an amount is
    // refused with the literal named, and a literal that reads as zero is refused for
    // being zero — two different mistakes, two different messages. A swap with no
    // floor at all takes the same path as an explicit zero, because a missing amount
    // is 0 and a floor of 0 accepts any output.
    let zero = errors(&intent_with_min_output("0"));
    assert_eq!(zero.len(), 1, "{zero:?}");
    assert!(
        zero[0].contains("must be greater than zero"),
        "a stated zero is refused for being zero: {zero:?}"
    );
    assert!(
        !zero[0].contains("fractional"),
        "and not for being unreadable: {zero:?}"
    );

    let missing = errors("intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        swap uniswap ethereum.USDC -> solana.SOL amount 1\n    }\n    require slippage <= 50\n    on_fail refund ethereum.USDC to sender\n}\n");
    assert!(
        missing.iter().any(|error| error.contains("must be greater than zero")),
        "a swap with no floor is the same shape as a zero one: {missing:?}"
    );
}

#[test]
fn the_same_refusal_covers_the_swap_amount() {
    // The other half of the pair: the same conversion, the same rule.
    let source = intent_with_min_output("900").replace("amount 1 min_output", "amount 0.5 min_output");
    let errors = errors(&source);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("`0.5`") && error.contains("fractional")),
        "{errors:?}"
    );
}
