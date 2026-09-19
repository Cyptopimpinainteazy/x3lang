//! `require canonical_supply <ASSET>` is decided, not recorded.
//!
//! The guard claims the canonical supply of an asset is preserved. That is a
//! claim about the program's own operations, so it has a compile-time answer: a
//! program that mints or burns the asset alone has changed its supply, and the
//! guard is a statement the artifact contradicts.
//!
//! This is the shape TICKET-027 asks for — "each guard kind either has a real
//! runtime quantity at its guard and emits a comparison, or is evaluated by the
//! compiler; none may be an unbacked `STATIC` assertion" — for the one kind whose
//! quantity is entirely inside the program.

/// Compile a program and report the errors it is refused for.
fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics(source) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

/// An intent whose route holds `steps`, guarding the supply of `asset`.
///
/// The slippage guard and the refund path are there because every route needs
/// them: this file is about one guard, and a probe that fails another check would
/// be testing that one instead.
fn intent(steps: &str, guard: &str) -> String {
    format!(
        "intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {{\n{steps}\n    }}\n    require slippage <= 50\n    {guard}\n    on_fail refund ethereum.USDC to sender\n}}\n"
    )
}

#[test]
fn a_program_that_does_not_move_the_supply_may_claim_it_does_not() {
    let source = intent(
        "        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1",
        "require canonical_supply USDC",
    );
    assert_eq!(errors(&source), Vec::<String>::new(), "the claim holds");
}

#[test]
fn minting_the_asset_alone_contradicts_the_guard() {
    let source = intent(
        "        mint solana.USDC amount 5 to wallet",
        "require canonical_supply USDC",
    );
    let errors = errors(&source);
    assert_eq!(errors.len(), 1, "one guard, one contradiction: {errors:?}");
    assert!(
        errors[0].contains("mints 5 and burns 0"),
        "the message has to say what the program does: {errors:?}"
    );
    assert!(errors[0].contains("USDC"), "and about which asset: {errors:?}");
}

#[test]
fn burning_the_asset_alone_contradicts_the_guard() {
    let source = intent(
        "        burn solana.USDC amount 7 from wallet",
        "require canonical_supply USDC",
    );
    let errors = errors(&source);
    assert!(
        errors.iter().any(|error| error.contains("mints 0 and burns 7")),
        "{errors:?}"
    );
}

#[test]
fn minting_and_burning_the_same_amount_preserves_the_supply() {
    // "Preserved" is net zero, not "never touched": a program that takes five
    // out and puts five back has not changed what is in circulation.
    let source = intent(
        "        mint solana.USDC amount 5 to wallet\n        burn solana.USDC amount 5 from wallet",
        "require canonical_supply USDC",
    );
    assert_eq!(errors(&source), Vec::<String>::new(), "net zero is preserved");
}

#[test]
fn minting_more_than_it_burns_is_the_contradiction() {
    let source = intent(
        "        mint solana.USDC amount 5 to wallet\n        burn solana.USDC amount 4 from wallet",
        "require canonical_supply USDC",
    );
    let errors = errors(&source);
    assert!(
        errors.iter().any(|error| error.contains("mints 5 and burns 4")),
        "{errors:?}"
    );
}

#[test]
fn the_guard_is_about_the_asset_it_names() {
    // Minting one asset says nothing about another's supply, and a check that
    // cannot tell them apart would refuse a correct program.
    let source = intent(
        "        mint solana.SOL amount 5 to wallet",
        "require canonical_supply USDC",
    );
    assert_eq!(errors(&source), Vec::<String>::new(), "SOL is not USDC");
}

#[test]
fn a_guard_that_names_no_asset_is_refused() {
    // `canonical_supply` with nothing to hold the supply of is a guard that
    // cannot be evaluated at all, which is worse than one that fails.
    let source = intent(
        "        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1",
        "require canonical_supply",
    );
    let errors = errors(&source);
    assert!(
        errors.iter().any(|error| error.contains("without naming the asset")),
        "{errors:?}"
    );
}

#[test]
fn a_program_that_claims_nothing_may_mint() {
    // The guard is what makes the supply a promise; without it, minting is
    // simply what the program does.
    // No filler guard: the helper already writes `require slippage <= 50`, and a
    // guard nothing backs (`route_score` without a `min_route_score`) would fail
    // this test for a reason that has nothing to do with supply.
    let source = intent("        mint solana.USDC amount 5 to wallet", "");
    assert_eq!(
        errors(&source),
        Vec::<String>::new(),
        "no canonical_supply guard, no claim to contradict"
    );
}
