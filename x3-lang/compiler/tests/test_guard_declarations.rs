//! Two guard kinds that now have a checker behind them (TICKET-027/049).
//!
//! `require proof_complete <name>` is compared against the program's own
//! `proofs required { … }` declaration, and a guard whose kind is not a word the
//! compiler knows is refused rather than recorded — a guard nothing can check is
//! a comment about the artifact, and the language's rule everywhere else is that
//! an unrecognised construct is one it cannot check.

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics(source) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

/// An intent with `declarations`, a route, and the guards under test.
fn program(declarations: &str, guards: &str) -> String {
    format!(
        "{declarations}intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {{\n        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }}\n    require slippage <= 50\n{guards}\n    on_fail refund ethereum.USDC to sender\n}}\n"
    )
}

const PROOFS: &str = "proofs required {\n    source_lock_proof\n    destination_fill_proof\n}\n\n";

#[test]
fn a_declared_proof_may_be_required_to_be_complete() {
    let source = program(PROOFS, "    require proof_complete source_lock_proof");
    assert_eq!(errors(&source), Vec::<String>::new(), "the proof is declared");
}

#[test]
fn requiring_a_proof_the_program_never_declares_is_refused() {
    let source = program(PROOFS, "    require proof_complete solver_signature");
    let errors = errors(&source);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("solver_signature") && error.contains("source_lock_proof")),
        "the message must name the proof and what the program declares: {errors:?}"
    );
}

#[test]
fn requiring_a_proof_with_no_declaration_at_all_is_refused() {
    // The guard asserts a proof type the artifact never mentions, so there is
    // nothing for its claim to be backed by.
    let source = program("", "    require proof_complete source_lock_proof");
    let errors = errors(&source);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("declares no `proofs required")),
        "{errors:?}"
    );
}

#[test]
fn a_proof_guard_that_names_nothing_is_refused() {
    let source = program(PROOFS, "    require proof_complete");
    let errors = errors(&source);
    assert!(
        errors.iter().any(|error| error.contains("without naming the proof")),
        "{errors:?}"
    );
}

#[test]
fn a_guard_kind_the_compiler_does_not_know_is_refused() {
    // `require proof verified` — the corpus's one instance — used to parse into a
    // `Custom` kind and lower to a `REQUIRE` the executor treats as true.
    let source = program(PROOFS, "    require proof verified");
    let errors = errors(&source);
    assert_eq!(errors.len(), 1, "one unknown kind, one error: {errors:?}");
    assert!(errors[0].contains("not a guard kind this compiler knows"), "{errors:?}");
    assert!(
        errors[0].contains("proof_complete") && errors[0].contains("canonical_supply"),
        "the message has to list the kinds it does know: {errors:?}"
    );
}

#[test]
fn a_known_kind_with_no_checker_yet_is_not_refused_here() {
    // This check is about *unknown* kinds. `route_score` is a known kind whose
    // evaluation is still missing (TICKET-049), and refusing it here would be a
    // different change than the one this test is about — the ledger says so, and
    // this test keeps the two from being confused.
    let source = program("", "    require route_score >= 90");
    let errors = errors(&source);
    assert!(
        !errors.iter().any(|error| error.contains("not a guard kind")),
        "a known kind is not an unknown kind: {errors:?}"
    );
}

#[test]
fn every_listed_kind_name_is_one_the_parser_knows() {
    // The list is a second statement of the parser's match, and the direction
    // that matters is this one: a name in the list that the parser does *not*
    // know would tell a user to write a guard that gets refused.
    for name in x3_lang_compiler::parser::REQUIRE_KIND_NAMES {
        let source = program(PROOFS, &format!("    require {name} <= 1"));
        let errors = errors(&source);
        assert!(
            !errors
                .iter()
                .any(|error| error.contains("not a guard kind this compiler knows")),
            "`require {name}` is listed as a kind the compiler knows but is refused as unknown: \
             {errors:?}"
        );
    }
}

#[test]
fn a_refund_path_guard_needs_a_refund_path() {
    // The guard is a claim about the program's own shape, so it is decided
    // against the program: with a refund action it holds, without one it does
    // not. This is the second kind closed by evaluation rather than by a
    // declaration (TICKET-027).
    let with_path = "intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    require slippage <= 50\n    require refund_path sender\n    on_fail refund ethereum.USDC to sender\n}\n";
    assert_eq!(errors(with_path), Vec::<String>::new(), "the program has one");

    let without_path = "intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    require slippage <= 50\n    require refund_path sender\n    on_fail rollback\n}\n";
    let errors = errors(without_path);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("require refund_path") && error.contains("has none")),
        "the guard demands what the program does not have: {errors:?}"
    );
}
