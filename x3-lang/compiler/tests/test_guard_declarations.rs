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
    // This check is about *unknown* kinds. `audit_gate` is a known kind whose
    // evaluation is still missing (TICKET-049: nothing declares an audit for it to
    // be about), and refusing it *here* would be a different change than the one
    // this test is about — the ledger says so, and this test keeps the two from
    // being confused. `route_score` and then `bridge_liquidity` were this test's
    // examples until they gained checkers; the kind used has to be one that has
    // none.
    let source = program("", "    require audit_gate iso_27001");
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

#[test]
fn a_route_score_guard_needs_a_declared_score() {
    // Six corpus programs required `route_score >= 85..90` with nothing anywhere
    // declaring a score, so the guard was a claim no configuration backed. The
    // declaration is `risk_policy { min_route_score <n> }`, and the check reads it
    // as a floor.
    let declared = program(
        "risk_policy {\n    min_route_score 90\n}\n\n",
        "    require route_score >= 90",
    );
    assert_eq!(errors(&declared), Vec::<String>::new(), "90 is what it declares");

    let lower = program(
        "risk_policy {\n    min_route_score 80\n}\n\n",
        "    require route_score >= 90",
    );
    let lower_errors = errors(&lower);
    assert!(
        lower_errors
            .iter()
            .any(|error| error.contains("requires a route score of 90") && error.contains("80")),
        "the message must name both figures: {lower_errors:?}"
    );

    let undeclared = program("", "    require route_score >= 90");
    let undeclared_errors = errors(&undeclared);
    assert!(
        undeclared_errors
            .iter()
            .any(|error| error.contains("no `risk_policy { min_route_score")),
        "{undeclared_errors:?}"
    );

    let ceiling = program(
        "risk_policy {\n    min_route_score 90\n}\n\n",
        "    require route_score <= 90",
    );
    let ceiling_errors = errors(&ceiling);
    assert!(
        ceiling_errors
            .iter()
            .any(|error| error.contains("without a `>=` bound")),
        "{ceiling_errors:?}"
    );
}

#[test]
fn an_unknown_risk_policy_field_is_refused_by_name() {
    // The parser used to end its match with "skip unknown config fields" while
    // its own doc comment advertised `max_route_risk`, `max_fee` and
    // `min_liquidity` — none of which it read. A declared bound that is dropped
    // without a word is the same defect as a timeout unit that is ignored.
    let source = program(
        "risk_policy {\n    max_route_risk 3\n}\n\n",
        "    require route_score >= 90",
    );
    let errors = errors(&source);
    assert!(
        errors
            .iter()
            .any(|error| error.contains("unknown risk_policy field 'max_route_risk'")
                && error.contains("min_route_score")),
        "the message must name the field and the fields that exist: {errors:?}"
    );
}

#[test]
fn every_example_that_requires_a_route_score_declares_one() {
    // The corpus invariant this change restored: a guard's claim must be backed
    // by the program that writes it, in every example the repo ships.
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../examples");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("the examples must be readable") {
        let path = entry.expect("a readable entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("x3") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("a readable example");
        if !source.contains("require route_score") {
            continue;
        }
        checked += 1;
        assert!(
            source.contains("min_route_score"),
            "{} requires a route score without declaring one",
            path.display()
        );
    }
    assert_eq!(checked, 6, "six examples require a route score; found {checked}");
}

#[test]
fn a_vm_guard_needs_a_declaration_that_uses_that_vm() {
    // `require vm_supported <vm>` is a claim about the artifact's own adapters, so
    // the compiler decides it: the program's `vm`, `target` and `venue`
    // declarations say which families it uses, and the guard is compared through
    // the family map the parser already uses for chain prefixes — `solana` and
    // `svm` are one family, so a guard is not refused over a spelling.
    fn errors(source: &str) -> Vec<String> {
        match x3_lang_compiler::check_source_diagnostics(source) {
            Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
            Err(error) => vec![format!("{error}")],
        }
    }
    let with_vm = |declaration: &str, guard: &str| format!("{declaration}\n{}", program("", guard));

    // Declared, and declared under another spelling of the same family.
    assert_eq!(
        errors(&with_vm(
            "vm {\n    chain arbitrum\n    adapter evm\n}",
            "    require vm_supported evm"
        )),
        Vec::<String>::new()
    );
    assert_eq!(
        errors(&with_vm(
            "target svm {\n    adapter svm_adapter\n}",
            "    require vm_supported solana"
        )),
        Vec::<String>::new(),
        "`solana` and `svm` are the same family"
    );

    // Not declared.
    let refused = errors(&with_vm(
        "vm {\n    chain arbitrum\n    adapter evm\n}",
        "    require vm_supported movevm",
    ));
    assert!(
        refused
            .iter()
            .any(|error| error.contains("requires the VM 'movevm'") && error.contains("declares {evm}")),
        "the message must name the VM and what is declared: {refused:?}"
    );

    // Nothing declared at all, and a guard that names nothing.
    let undeclared = errors(&with_vm("", "    require vm_supported evm"));
    assert!(
        undeclared.iter().any(|error| error.contains("declares no `vm`")),
        "{undeclared:?}"
    );
    let nameless = errors(&with_vm(
        "vm {\n    chain arbitrum\n    adapter evm\n}",
        "    require vm_supported",
    ));
    assert!(
        nameless.iter().any(|error| error.contains("without naming the VM")),
        "{nameless:?}"
    );
}

#[test]
fn an_invariant_guard_needs_a_declared_invariant() {
    // An invariant is checked because the program declares it, so a guard naming
    // one it does not declare asks for a check nothing provides.
    fn errors(source: &str) -> Vec<String> {
        match x3_lang_compiler::check_source_diagnostics(source) {
            Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
            Err(error) => vec![format!("{error}")],
        }
    }
    let with_declaration = |declaration: &str, guard: &str| format!("{declaration}\n{}", program("", guard));

    assert_eq!(
        errors(&with_declaration(
            "invariant no_double_claim",
            "    require invariant no_double_claim"
        )),
        Vec::<String>::new(),
        "the invariant is declared"
    );

    let refused = errors(&with_declaration(
        "invariant no_double_claim",
        "    require invariant no_double_refund",
    ));
    assert!(
        refused
            .iter()
            .any(|error| error.contains("'no_double_refund'") && error.contains("no_double_claim")),
        "the message must name the invariant and what is declared: {refused:?}"
    );

    let none = errors(&with_declaration("", "    require invariant total_shares"));
    assert!(
        none.iter().any(|error| error.contains("declares no `invariant`")),
        "{none:?}"
    );

    let nameless = errors(&with_declaration("invariant a", "    require invariant"));
    assert!(
        nameless.iter().any(|error| error.contains("without naming it")),
        "{nameless:?}"
    );
}

#[test]
fn a_bridge_liquidity_guard_needs_bridges_that_deep() {
    // The guard asserts the bridges this program uses can absorb N, and a
    // `venue { kind bridge … liquidity … }` declaration is where a program states a
    // bridge's depth. Every declared bridge rather than any one of them: a route may
    // take whichever the planner finds.
    fn errors(source: &str) -> Vec<String> {
        match x3_lang_compiler::check_source_diagnostics(source) {
            Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
            Err(error) => vec![format!("{error}")],
        }
    }
    let venue = |name: &str, liquidity: u128| {
        format!(
            "venue {name} {{\n    kind bridge\n    chain ethereum\n    domain evm\n    asset_in ethereum.USDC\n    asset_out solana.SOL\n    fee_bps 2\n    liquidity {liquidity}\n    slippage_bps 5\n    latency_ms 900\n    finality_blocks 32\n    risk 4\n    proof destination_fill_proof\n}}\n"
        )
    };
    let with_venues = |venues: &str, guard: &str| format!("{venues}\n{}", program("", guard));

    // Deep enough.
    assert_eq!(
        errors(&with_venues(
            &venue("x3_bridge", 500_000),
            "    require bridge_liquidity >= 100_000"
        )),
        Vec::<String>::new()
    );

    // Too thin, and the message says which bridge and by how much.
    let thin = errors(&with_venues(
        &venue("x3_bridge", 5_000),
        "    require bridge_liquidity >= 100_000",
    ));
    assert!(
        thin.iter()
            .any(|error| error.contains("'x3_bridge'") && error.contains("declares 5000")),
        "the message must name the bridge and its depth: {thin:?}"
    );

    // Every declared bridge: the deep one does not cover for the thin one.
    let mixed = errors(&with_venues(
        &format!("{}{}", venue("deep_bridge", 500_000), venue("thin_bridge", 1_000)),
        "    require bridge_liquidity >= 100_000",
    ));
    assert_eq!(mixed.len(), 1, "one bridge is too thin: {mixed:?}");
    assert!(mixed[0].contains("thin_bridge"), "{mixed:?}");

    // Nothing to be about, and a ceiling instead of a floor.
    let none = errors(&with_venues("", "    require bridge_liquidity >= 100_000"));
    assert!(
        none.iter()
            .any(|error| error.contains("declares no `venue { kind bridge")),
        "{none:?}"
    );
    let ceiling = errors(&with_venues(
        &venue("x3_bridge", 500_000),
        "    require bridge_liquidity <= 100_000",
    ));
    assert!(
        ceiling.iter().any(|error| error.contains("without a `>=` bound")),
        "{ceiling:?}"
    );
}
