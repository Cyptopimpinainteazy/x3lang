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
fn an_audit_gate_guard_is_refused_because_nothing_can_back_it() {
    // This kind was "known but unchecked" until the last of TICKET-049: the name
    // parsed, the guard lowered, and the artifact recorded a condition nothing
    // read. It is refused by name now. An audit is evidence about the delivery
    // process rather than a property of the artifact, so no clause in a program
    // can state one and no pass can read one — recording the guard would make the
    // artifact assert something that is true because nothing looked.
    let found = errors(&program("", "    require audit_gate iso_27001"));
    assert_eq!(found.len(), 1, "{found:?}");
    assert!(
        found[0].contains("cannot back") && found[0].contains("nothing looked"),
        "the diagnostic must say why the kind has no checker and what that would mean: {found:?}"
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

// ---------------------------------------------------------------------------
// `require finality.<chain> >= N` — TICKET-049's last kind with corpus weight.
//
// Nine corpus programs wrote twelve finality guards against a quantity nothing
// declared. `finality_policy` is where a program states what it requires of a
// chain; these tests pin both directions of the check and the invariant that the
// corpus declares what its guards require.
// ---------------------------------------------------------------------------

/// A policy that requires 32 blocks of `solana`.
const SOLANA_32: &str = "finality_policy strict {\n    chain solana\n    requirement finalized\n    blocks 32\n}\n\n";

fn finality(declarations: &str, guard: &str) -> Vec<String> {
    errors(&program(declarations, &format!("    {guard}")))
}

#[test]
fn a_guard_at_the_declared_depth_is_accepted() {
    assert_eq!(
        finality(SOLANA_32, "require finality.solana >= 32"),
        Vec::<String>::new(),
        "the guard states exactly the policy's requirement"
    );
}

#[test]
fn a_guard_stricter_than_the_declaration_is_accepted() {
    // Demanding a deeper state than the policy's floor is strictly more
    // conservative: the program waits longer than it promised to.
    assert_eq!(
        finality(SOLANA_32, "require finality.solana >= 64"),
        Vec::<String>::new()
    );
}

#[test]
fn a_guard_below_the_declared_depth_is_refused_with_both_numbers() {
    // The guard would pass at 12 blocks while the program's own policy says the
    // chain is not final until 32 — the exact blur between confirmation depths
    // the declaration exists to prevent.
    let errors = finality(SOLANA_32, "require finality.solana >= 12");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("12") && errors[0].contains("32") && errors[0].contains("not final"),
        "the message must carry both depths and say which way the guard is wrong: {errors:?}"
    );
}

#[test]
fn a_depth_guard_with_no_declaration_names_the_clause_to_add() {
    let errors = finality("", "require finality.solana >= 32");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("finality_policy") && errors[0].contains("blocks"),
        "the diagnostic must name the declaration that would back the guard: {errors:?}"
    );
}

#[test]
fn a_policy_without_a_depth_cannot_back_a_depth_guard() {
    // A mode is not a depth. `requirement finalized` says what the chain must
    // reach, not how far behind the tip it must be.
    let declarations = "finality_policy strict {\n    chain solana\n    requirement finalized\n}\n\n";
    let errors = finality(declarations, "require finality.solana >= 32");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("no `blocks`") && errors[0].contains("blocks 32"),
        "the diagnostic must say which clause is missing: {errors:?}"
    );
}

#[test]
fn a_mode_guard_is_decided_against_the_declared_requirement() {
    let declarations = "finality_policy strict {\n    chain solana\n    requirement finalized\n    blocks 32\n}\n\n";
    assert_eq!(
        finality(declarations, "require finality.solana == finalized"),
        Vec::<String>::new(),
        "the guard states the mode the policy requires"
    );
    let mismatched = finality(declarations, "require finality.solana == safe");
    assert_eq!(mismatched.len(), 1, "{mismatched:?}");
    assert!(
        mismatched[0].contains("safe") && mismatched[0].contains("finalized"),
        "the diagnostic must name both modes: {mismatched:?}"
    );
}

#[test]
fn a_finality_depth_written_as_a_ceiling_is_refused() {
    let errors = finality(SOLANA_32, "require finality.solana <= 32");
    assert!(
        errors.iter().any(|error| error.contains("without a `>=` bound")),
        "{errors:?}"
    );
}

#[test]
fn chain_names_compare_without_case() {
    // Eight of the corpus's twelve guards spell their chain differently from the
    // way the declaration does (`finality Ethereum`), and a chain name is not an
    // identifier whose case is part of its meaning.
    let declarations = "finality_policy strict {\n    chain Ethereum\n    requirement finalized\n    blocks 64\n}\n\n";
    assert_eq!(
        finality(declarations, "require finality.ethereum >= 64"),
        Vec::<String>::new()
    );
}

#[test]
fn two_policies_for_one_chain_are_refused_where_a_guard_depends_on_them() {
    let declarations = format!(
        "{SOLANA_32}finality_policy loose {{\n    chain solana\n    requirement finalized\n    blocks 12\n}}\n\n"
    );
    let errors = finality(&declarations, "require finality.solana >= 32");
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].contains("2 `finality_policy` declarations"),
        "a guard read against the first of two depths is read against whichever was written first: {errors:?}"
    );
}

#[test]
fn no_example_gates_on_finality_without_declaring_what_it_requires() {
    // The corpus is where this defect lived, so the corpus is where the
    // invariant is asserted: every example that writes a finality guard must
    // declare the depth (or mode) it requires, and an example that gains one
    // without the other fails here rather than passing a check that reads it as
    // true. The six examples the Rust parser does not accept are included — their
    // other errors are ignored, but a finality guard with nothing behind it is
    // not.
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("examples");
    let mut gating_examples = 0usize;
    for entry in std::fs::read_dir(&directory).expect("the examples directory must be readable") {
        let path = entry.expect("a readable directory entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("x3") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("an example must be readable");
        if !source.contains("require finality") {
            continue;
        }
        gating_examples += 1;
        let unbacked: Vec<String> = errors(&source)
            .into_iter()
            .filter(|error| {
                error.contains("no `finality_policy` names that chain")
                    || error.contains("blocks of finality")
                    || error.contains("finality mode")
                    || error.contains("`finality_policy` declarations name chain")
            })
            .collect();
        assert!(
            unbacked.is_empty(),
            "{}: a finality guard with nothing behind it: {unbacked:?}",
            path.display()
        );
    }
    assert!(
        gating_examples >= 9,
        "the corpus must still contain the examples this check is about: {gating_examples}"
    );
}

#[test]
fn a_declared_depth_is_written_back_by_the_formatter() {
    // The round trip matters because the formatter is what `x3c fmt` writes and
    // what the corpus is regenerated through: a field the parser reads and the
    // formatter drops is a declaration that disappears on reformatting.
    let source = format!("{SOLANA_32}{}", program("", "    require finality.solana >= 32"));
    let program = x3_lang_compiler::parser::parse_source(&source).expect("the program should parse");
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&program);
    assert!(
        formatted.contains("blocks 32"),
        "the depth must survive the formatter: {formatted}"
    );
    let reparsed = x3_lang_compiler::parser::parse_source(&formatted).expect("the formatted program must parse");
    assert_eq!(
        x3_lang_compiler::compile_to_ir(&reparsed)
            .expect("it must lower")
            .operations
            .len(),
        x3_lang_compiler::compile_to_ir(&program)
            .expect("it must lower")
            .operations
            .len(),
        "formatting must not change the program"
    );
}

// ---------------------------------------------------------------------------
// The last three kinds of TICKET-049: `risk`, `mainnet_safe` and `audit_gate`.
// ---------------------------------------------------------------------------

/// The IR a program lowers to, whether or not the semantic checks refused it.
fn ir_of(source: &str) -> x3_lang_compiler::X3IR {
    let (_program, ir, _outcome) =
        x3_lang_compiler::check_source_diagnostics(source).expect("the program must parse and lower");
    ir
}

fn errors_in_mode(source: &str, mode: x3_lang_compiler::CompilationMode) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, mode) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

#[test]
fn a_risk_guard_is_decided_against_the_program_s_computed_score() {
    // `require risk <= N` names a quantity no clause declares: the score is
    // computed from the program's own operations, the same shape as
    // `canonical_supply`. The test computes the score rather than hardcoding one,
    // so it cannot drift away from the scorer — a number in the test would still
    // pass while the guard was being checked against something else.
    let base = program("", "");
    let score = x3_lang_compiler::semantic::compute_risk_score(&ir_of(&base)).total;
    assert!(
        score > 0,
        "the fixture must have some risk for a ceiling to be about: {score}"
    );

    let at_the_score = program("", &format!("    require risk <= {score}"));
    assert_eq!(
        errors(&at_the_score),
        Vec::<String>::new(),
        "a ceiling equal to the computed score is satisfiable"
    );

    let below_the_score = errors(&program("", &format!("    require risk <= {}", score - 1)));
    assert_eq!(below_the_score.len(), 1, "{below_the_score:?}");
    assert!(
        below_the_score[0].contains(&format!("at most {}", score - 1))
            && below_the_score[0].contains(&format!("computed score is {score}")),
        "the diagnostic must carry the claimed ceiling and the computed score: {below_the_score:?}"
    );
}

#[test]
fn a_risk_guard_written_as_a_floor_is_refused() {
    // The score is a risk, so its bound is a ceiling. A floor compared against a
    // score below it is a guard that cannot fail while reading as a constraint.
    let found = errors(&program("", "    require risk >= 10"));
    assert!(
        found.iter().any(|error| error.contains("without a ceiling")),
        "{found:?}"
    );
}

#[test]
fn a_risk_guard_whose_bound_is_not_a_number_is_refused() {
    let found = errors(&program("", "    require risk <= threshold"));
    assert!(
        found.iter().any(|error| error.contains("cannot read as a number")),
        "{found:?}"
    );
}

#[test]
fn a_mainnet_safe_guard_runs_the_mainnet_checks_in_any_mode() {
    // The guard is a request for the mainnet checks rather than a claim about a
    // mode: honouring it means running them, which is what makes the guard's claim
    // true instead of recorded. This fixture breaks three of those rules, so in
    // dev mode the three errors are the proof the checks ran.
    let found = errors_in_mode(
        &program("", "    require mainnet_safe"),
        x3_lang_compiler::CompilationMode::Dev,
    );
    assert!(
        found
            .iter()
            .any(|error| error.contains("mainnet: no RPC consensus declared")),
        "the RPC rule must have run: {found:?}"
    );
    assert!(
        found
            .iter()
            .any(|error| error.contains("mainnet: missing solver bond declaration")),
        "the solver-bond rule must have run: {found:?}"
    );
    assert!(
        // A configuration the mainnet gates refuse has its own class: the same program is acceptable
        // on a testnet, so it is not a malformed program and not `UnsafeIr` (TICKET-021).
        found.iter().all(|error| error.contains("X3E4028")),
        "every mainnet gate refusal must carry the class, not only the wording: {found:?}"
    );
}

#[test]
fn a_guard_whose_declaration_is_missing_carries_its_own_class() {
    // The recurring shape of this language and the reason the class exists: the guard kind is one the
    // compiler understands, and what is missing is the *declaration* it asserts — so it is not an
    // undefined symbol and not a malformed program (TICKET-021).
    let source = program("", "    require solver_bond >= 5");
    let found = errors_in_mode(&source, x3_lang_compiler::CompilationMode::Dev);
    assert!(
        found
            .iter()
            .any(|error| error.contains("X3E4027") && error.contains("solver bond")),
        "a guard nothing backs must be refused by name and by code: {found:?}"
    );
}

#[test]
fn the_mainnet_checks_do_not_run_without_the_guard_or_the_mode() {
    // The pair to the test above: without the guard, and compiling for dev, the
    // same program reports none of those errors — so it is the guard, and not
    // something else in the pipeline, that put the checks on the path.
    let found = errors(&program("", ""));
    assert!(
        !found.iter().any(|error| error.contains("mainnet:")),
        "no mainnet rule should run here: {found:?}"
    );
}

#[test]
fn the_mainnet_safe_example_declares_a_property_the_compiler_confirms() {
    // End to end on a real corpus file: `examples/mainnet_safe_swap.x3` is named
    // for the property, writes `require mainnet_safe`, and passes every mainnet
    // rule — in dev mode, where the guard is what asked for them. A guard that was
    // only honoured under `--mode mainnet` would have left this file's own name
    // unchecked.
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("examples");
    let source = std::fs::read_to_string(directory.join("mainnet_safe_swap.x3")).expect("the example must be readable");
    assert!(
        source.contains("require mainnet_safe"),
        "the example states the claim this test is about"
    );
    let found = errors_in_mode(&source, x3_lang_compiler::CompilationMode::Dev);
    assert_eq!(found, Vec::<String>::new(), "{found:?}");
}

// ---------------------------------------------------------------------------
// The declared depth reaches the artifact (TICKET-059).
// ---------------------------------------------------------------------------

#[test]
fn the_artifact_carries_the_declared_finality_depth() {
    // A replayer that reads only the artifact has to be able to re-check what the
    // compiler decided: the guard says `>= 32`, and the policy's `blocks 32` is the
    // number that decision was made against. It travels in the `REQUIRE` operand
    // (a `FinalityExplicit` record), so `x3c explain` shows it.
    let with_depth = format!(
        "{}{}",
        "finality_policy strict {\n    chain solana\n    requirement finalized\n    blocks 32\n}\n\n",
        program("", "    require finality.solana >= 32")
    );
    let bytecode = x3_lang_compiler::compile_source(&with_depth).expect("the program must compile");
    let trace = x3_lang_compiler::emitter::disassemble(&bytecode).expect("it must disassemble");
    assert!(
        trace.contains("REQUIRE static 32"),
        "the declaration's depth must be in the artifact: {trace}"
    );

    // A policy that states no depth carries zero there, which is why the parser
    // refuses `blocks 0`: a missing depth and a zero one cannot be the same number.
    let without_depth = format!(
        "{}{}",
        "finality_policy strict {\n    chain solana\n    requirement finalized\n}\n\n",
        program("", "    require finality.solana == finalized")
    );
    let bytecode = x3_lang_compiler::compile_source(&without_depth).expect("the program must compile");
    let trace = x3_lang_compiler::emitter::disassemble(&bytecode).expect("it must disassemble");
    assert!(
        trace.contains("REQUIRE static 0"),
        "a declaration that states no depth carries zero: {trace}"
    );
}

#[test]
fn a_finality_depth_of_zero_or_beyond_the_operand_is_refused() {
    // `blocks 0` states nothing, and the artifact's operand cannot hold a depth
    // above `u16::MAX`; both are refusals rather than a silently reduced number.
    let zero = errors(&format!(
        "{}{}",
        "finality_policy strict {\n    chain solana\n    requirement finalized\n    blocks 0\n}\n\n",
        program("", "    require finality.solana >= 1")
    ));
    assert!(zero.iter().any(|error| error.contains("states no depth")), "{zero:?}");

    let too_deep = errors(&format!(
        "{}{}",
        "finality_policy strict {\n    chain solana\n    requirement finalized\n    blocks 70000\n}\n\n",
        program("", "    require finality.solana >= 32")
    ));
    assert!(
        too_deep
            .iter()
            .any(|error| error.contains("largest depth the artifact can carry")),
        "{too_deep:?}"
    );
}

/// A guard is checked wherever it is written, including one block down.
///
/// Every pass that reads a program's guards does it through `semantic::require_guards`, and that
/// walk read the **top level** of an intent's body only. A guard inside a `fallback` block, a
/// `leg`, an `atomic` block or an `if` branch was therefore invisible to all thirteen checks built
/// on it — while reading, in the source, exactly like the ones that were checked. Measured: a
/// `fallback` block whose `require slippage <= 99` sat under `risk_policy { max_slippage 50 }`
/// compiled, and the same guard at the top level of the intent was refused.
mod a_guard_is_checked_wherever_it_is_written {
    use super::errors;

    /// The corpus's own shape: a route with a fallback, bounded by guards.
    fn with_fallback(policy_max_slippage: u32, fallback_bound: u32) -> String {
        format!(
            "risk_policy {{\n    max_slippage {policy_max_slippage}\n}}\n\n\
             intent probe {{\n\
             \x20   from ethereum.USDC amount 1_000 receiver 0x1111111111111111111111111111111111111111\n\
             \x20   to ethereum.ETH receiver 0x1111111111111111111111111111111111111111\n\
             \x20   route {{\n\
             \x20       swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1\n\
             \x20       fallback {{\n\
             \x20           replace with curve\n\
             \x20           require slippage <= {fallback_bound}\n\
             \x20       }}\n\
             \x20   }}\n\
             \x20   require slippage <= 50\n\
             \x20   on_fail refund ethereum.USDC to sender\n\
             }}\n"
        )
    }

    #[test]
    fn a_fallback_guard_above_the_policy_is_refused() {
        // 99 > 50: the block allows what the policy forbids, which is what the check exists for.
        let errors = errors(&with_fallback(50, 99));
        assert!(
            errors.iter().any(|error| error.contains("99") && error.contains("50")),
            "the refusal must name both numbers: {errors:?}"
        );
    }

    #[test]
    fn a_fallback_guard_within_the_policy_is_accepted() {
        // Non-vacuous: the same program with a bound the policy permits.
        let errors = errors(&with_fallback(50, 7));
        assert!(errors.is_empty(), "7 is within 50: {errors:?}");
    }

    #[test]
    fn a_guard_inside_a_leg_is_checked() {
        // A `parallel` leg's guard names a chain no `finality_policy` declares, which is the
        // refusal the fixtures in `test_parallel_dag.rs` met once this walk was fixed.
        let source = r#"parallel cross {
    leg a {
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1 min_output 1
        on_fail refund ethereum.USDC to sender
    }
    leg b {
        bridge x3 ethereum.ETH -> solana.SOL amount 1 receiver 0x1
        require finality.ethereum >= 12
        timeout 30s refund ethereum.ETH to sender
        on_fail refund ethereum.ETH to sender
    }
}
"#;
        let errors = errors(source);
        assert!(
            errors
                .iter()
                .any(|error| error.contains("finality.ethereum") && error.contains("finality_policy")),
            "the guard must be decided against a declaration, and the message must say so: {errors:?}"
        );
    }
}
