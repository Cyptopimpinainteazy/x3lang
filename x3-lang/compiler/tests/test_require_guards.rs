//! `require` guards — the six shapes, and the one that has no reading.
//!
//! `require proof verified` used to be unparseable: the parser read the second
//! word as the guard's subject and then demanded a value the program never
//! wrote, because a guard's value was not optional (TICKET-045). Making it
//! optional is only half the work — the other half is that every *other* shape
//! still parses to the same guard it always did, and that a bound with no bound
//! in it is still refused.

use x3_lang_ast::ast::{ComparisonOp, Expression, RequireGuard, RequireKind, Statement};

/// The guards of an intent, in order.
fn guards(body: &str) -> Vec<RequireGuard> {
    let source = format!(
        "intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n{body}\n    on_fail rollback\n}}\n"
    );
    let program = x3_lang_compiler::parser::parse_source(&source).expect("the probe must parse");
    let mut found = Vec::new();
    for item in &program.items {
        if let x3_lang_ast::ast::Item::IntentDecl(intent) = &item.node {
            for statement in &intent.body.stmts {
                if let Statement::Require(guard) = statement {
                    found.push(guard.clone());
                }
            }
        }
    }
    found
}

fn guard(body: &str) -> RequireGuard {
    let mut found = guards(body);
    assert_eq!(found.len(), 1, "expected exactly one guard in {body:?}");
    found.remove(0)
}

#[test]
fn a_guard_that_names_a_property_states_no_value() {
    // The shape that could not be written: a name, and nothing after it.
    let canonical = guard("    require canonical_supply USDC");
    assert!(matches!(canonical.kind, RequireKind::CanonicalSupply));
    assert_eq!(
        canonical.subject.map(|s| s.as_str().to_string()),
        Some("USDC".to_string())
    );
    assert!(canonical.value.is_none(), "a property guard has no right-hand side");
    assert!(canonical.comparison.is_none());

    let proof = guard("    require proof verified");
    assert!(
        matches!(&proof.kind, RequireKind::Custom(name) if name.as_str() == "proof"),
        "a kind the vocabulary does not know is carried by name: {:?}",
        proof.kind
    );
    assert_eq!(
        proof.subject.map(|s| s.as_str().to_string()),
        Some("verified".to_string())
    );
    assert!(proof.value.is_none());
}

#[test]
fn a_property_guard_can_name_nothing_at_all() {
    // `require mainnet_safe` asserts a property of the program, which is a whole
    // guard with no subject and no value.
    let bare = guard("    require mainnet_safe");
    assert!(matches!(bare.kind, RequireKind::MainnetSafe));
    assert!(bare.subject.is_none());
    assert!(bare.value.is_none());
}

#[test]
fn the_shapes_that_existed_still_parse_the_same() {
    let slippage = guard("    require slippage <= 50");
    assert!(matches!(slippage.kind, RequireKind::Slippage));
    assert_eq!(slippage.comparison, Some(ComparisonOp::LessOrEqual));
    assert!(slippage.subject.is_none());
    assert!(
        matches!(
            slippage.value,
            Some(Expression::Literal(x3_lang_ast::ast::LiteralExpr::Int {
                value: 50,
                ..
            }))
        ),
        "the threshold is the value: {:?}",
        slippage.value
    );

    // A threshold about a named subject, written with a space.
    let finality = guard("    require finality Ethereum >= 64");
    assert!(matches!(finality.kind, RequireKind::Finality));
    assert_eq!(
        finality.subject.map(|s| s.as_str().to_string()),
        Some("Ethereum".to_string())
    );
    assert_eq!(finality.comparison, Some(ComparisonOp::GreaterOrEqual));
    assert!(finality.value.is_some());

    // The same, with the subject written explicitly.
    let dotted = guard("    require finality.sol == finalized");
    assert_eq!(dotted.subject.map(|s| s.as_str().to_string()), Some("sol".to_string()));
    assert_eq!(dotted.comparison, Some(ComparisonOp::Equal));
    assert!(dotted.value.is_some());

    // A subject and a value: the one kind whose subject is a status.
    let nonce = guard("    require nonce unused simple_swap_001");
    assert!(matches!(nonce.kind, RequireKind::Nonce));
    assert_eq!(
        nonce.subject.map(|s| s.as_str().to_string()),
        Some("unused".to_string())
    );
    assert!(nonce.value.is_some(), "the nonce identifier is the value");
}

#[test]
fn a_bound_with_no_bound_in_it_is_refused() {
    let source = "intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    require \
                  slippage\n    on_fail rollback\n}\n";
    let error = x3_lang_compiler::parser::parse_source(source)
        .expect_err("`require slippage` states no bound")
        .to_string();
    assert!(error.contains("states no bound"), "got: {error}");
}

#[test]
fn a_nonce_guard_without_an_identifier_is_refused_before_lowering() {
    // The replay-protection check reads the identifier, and a guard with none
    // would leave that check looking for nothing.
    let source = "intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    require \
                  nonce unused\n    on_fail rollback\n}\n";
    let error = x3_lang_compiler::parser::parse_source(source)
        .expect_err("a nonce guard must name the nonce")
        .to_string();
    assert!(error.contains("is a bound and has no value"), "got: {error}");
}

#[test]
fn a_property_guard_reaches_the_ir_with_what_it_named() {
    // A guard that parsed and lowered to nothing would be the shape this
    // repository keeps finding: a check that is not on the path.
    let source = "intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    require \
                  canonical_supply USDC\n    on_fail refund ethereum.USDC to sender\n}\n";
    let bytecode = x3_lang_compiler::compile_source(source).expect("it must compile");
    assert!(!bytecode.is_empty());

    let program = x3_lang_compiler::parser::parse_source(source).expect("parse");
    let ir = x3_lang_compiler::compile_to_ir(&program).expect("lower");
    let conditions: Vec<String> = ir
        .operations
        .iter()
        .filter_map(|operation| match operation {
            x3_lang_compiler::ir::Operation::Require {
                kind: x3_lang_compiler::ir::RequireKind::CanonicalSupply,
                condition,
                ..
            } => match condition {
                x3_lang_compiler::ir::Condition::Expression { expr } => Some(expr.clone()),
                other => Some(format!("{other:?}")),
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        conditions,
        vec!["USDC".to_string()],
        "the guard must lower with the name it asserted, not to an unconditional requirement"
    );
}

#[test]
fn a_clause_after_a_valueless_guard_is_a_clause() {
    // The shape that made the first cut of this fix wrong: `require
    // proof_complete` followed by `amount 500` parsed as a guard *about* the
    // word `amount`, leaving `500` behind as a statement. The program still
    // parsed, still compiled, and had no amount. Reading the guard and reading
    // the clause have to agree, in either order.
    let guard_first = "atomic swap eth.USDC -> sol.SOL {\n    require proof_complete\n    amount \
                       500\n    receiver sol.wallet.owner\n    hashlock sha256(secret)\n    timeout \
                       source 40m\n    timeout destination 20m\n    require finality.eth >= 12\n}\n";
    let clause_first = "atomic swap eth.USDC -> sol.SOL {\n    amount 500\n    receiver \
                        sol.wallet.owner\n    hashlock sha256(secret)\n    timeout source 40m\n    timeout \
                        destination 20m\n    require proof_complete\n    require finality.eth >= 12\n}\n";

    let first = x3_lang_compiler::compile_to_ir(
        &x3_lang_compiler::parser::parse_source(guard_first).expect("the guard-first order must parse"),
    )
    .expect("and lower");
    let second = x3_lang_compiler::compile_to_ir(
        &x3_lang_compiler::parser::parse_source(clause_first).expect("the clause-first order must parse"),
    )
    .expect("and lower");
    assert_eq!(
        first.operations.len(),
        second.operations.len(),
        "the two orders describe the same swap and must lower to the same operations"
    );
}

#[test]
fn a_guard_before_a_timeout_leaves_the_timeout_alone() {
    // The second clause word that bit this fix: `timeout` reaches the parser as
    // an identifier, so a guard that reads "whatever comes next" swallows it and
    // leaves `30s` behind. One guard, one timeout, in either order.
    let guard_first = "intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    require \
                       canonical_supply USDC\n    timeout 30s refund ethereum.USDC to sender\n    \
                       on_fail rollback\n}\n";
    let timeout_first = "intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    timeout \
                         30s refund ethereum.USDC to sender\n    require canonical_supply USDC\n    \
                         on_fail rollback\n}\n";
    let first = x3_lang_compiler::compile_to_ir(
        &x3_lang_compiler::parser::parse_source(guard_first).expect("guard first must parse"),
    )
    .expect("and lower");
    let second = x3_lang_compiler::compile_to_ir(
        &x3_lang_compiler::parser::parse_source(timeout_first).expect("timeout first must parse"),
    )
    .expect("and lower");
    assert_eq!(
        first.operations.len(),
        second.operations.len(),
        "the guard must not have taken the timeout clause with it"
    );
}

/// The `REQUIRE` flags byte carries two things: the comparison mode in bits 0-1 and the
/// guard's own operator in bits 2-4. Reading either one by masking by hand is a reader
/// that can forget to mask — the VM, the disassembler and the simulator's floor reader
/// each did, and the simulator's compared the raw byte, so it found **no** measured
/// guard in an artifact that carried two (PHASE 54, `1a9274900`'s follow-up).
///
/// This pins the writer and the reader as inverses over every combination, so a new
/// mode or a new operator cannot be added to one side only.
#[test]
fn the_require_flags_writer_and_readers_are_inverses() {
    use x3_lang_compiler::spec::opcodes::{
        require_comparison, require_flags, require_guard_operator, GUARD_OP_EQ, GUARD_OP_GE, GUARD_OP_GT, GUARD_OP_LE,
        GUARD_OP_LT, GUARD_OP_NE, REQUIRE_COMPARE_GE, REQUIRE_COMPARE_MEASURED_PROFIT,
        REQUIRE_COMPARE_MEASURED_SLIPPAGE, REQUIRE_COMPARE_STATIC,
    };

    let modes = [
        REQUIRE_COMPARE_STATIC,
        REQUIRE_COMPARE_GE,
        REQUIRE_COMPARE_MEASURED_PROFIT,
        REQUIRE_COMPARE_MEASURED_SLIPPAGE,
    ];
    let operators = [
        GUARD_OP_LT,
        GUARD_OP_LE,
        GUARD_OP_GT,
        GUARD_OP_GE,
        GUARD_OP_EQ,
        GUARD_OP_NE,
    ];

    for mode in modes {
        for operator in operators {
            let flags = require_flags(mode, operator);
            assert_eq!(
                require_comparison(flags),
                mode,
                "mode {mode} was lost with operator {operator} (flags {flags:#04x})"
            );
            assert_eq!(
                require_guard_operator(flags),
                operator,
                "operator {operator} was lost with mode {mode} (flags {flags:#04x})"
            );
        }
    }

    // The measured modes are 2 and 3, so their flags bytes differ from the mode alone —
    // which is exactly what a reader that skips the mask gets wrong, and what makes this
    // test meaningful rather than a tautology.
    assert_ne!(
        require_flags(REQUIRE_COMPARE_MEASURED_PROFIT, GUARD_OP_GE),
        REQUIRE_COMPARE_MEASURED_PROFIT
    );
    assert_ne!(
        require_flags(REQUIRE_COMPARE_MEASURED_SLIPPAGE, GUARD_OP_LE),
        REQUIRE_COMPARE_MEASURED_SLIPPAGE
    );
}

/// TICKET-089: **a whole-number percent guard does not eat the clause after it.**
///
/// The literal reader handled `Int.Dot.Int.Percent` and left `Int.Percent` to fall
/// through to the integer arm, so `1%` kept its `%` and the expression parser took it for
/// the **modulo** operator, consuming the next token as its right-hand operand. In a guard
/// that token is the next clause:
///
/// ```text
/// require slippage <= 1%      FAIL  unexpected clause in intent body: Ident("45s")
/// require slippage <= 0.5%    OK
/// ```
///
/// The refusal named a line that was correct, and the shape survived because a percent
/// guard written *last* in a body has no following clause to swallow — which is where
/// every passing example and fixture happened to put its own. `1%` is the most natural
/// way to write one percent.
#[test]
fn a_whole_number_percent_guard_does_not_eat_the_clause_after_it() {
    let whole = guard("    require slippage <= 1%\n    timeout 30s refund ethereum.USDC to sender");
    assert!(matches!(whole.kind, RequireKind::Slippage));
    assert_eq!(whole.comparison, Some(ComparisonOp::LessOrEqual));
    assert!(
        matches!(
            whole.value,
            Some(Expression::Literal(x3_lang_ast::ast::LiteralExpr::Percentage { .. }))
        ),
        "`1%` is a percentage literal, not an integer followed by a modulo: {:?}",
        whole.value
    );

    // And the clause after it is still a clause. The percentage and the basis-point
    // spelling must lower to the same program, which is the shape the existing
    // `a_guard_before_a_timeout_leaves_the_timeout_alone` test uses for a different
    // guard.
    let percent = "intent probe {\n    from ethereum.USDC amount 1\n    to solana.SOL\n    route {\n        \
                   swap uniswap ethereum.USDC -> solana.SOL amount 1 min_output 1\n    }\n    \
                   require slippage <= 1%\n    timeout 30s refund ethereum.USDC to sender\n    \
                   on_fail rollback\n}\n";
    let basis_points = percent.replace("require slippage <= 1%", "require slippage <= 100");
    let from_percent = x3_lang_compiler::compile_to_ir(
        &x3_lang_compiler::parser::parse_source(percent).expect("a whole-number percent guard must parse"),
    )
    .expect("and lower");
    let from_points = x3_lang_compiler::compile_to_ir(
        &x3_lang_compiler::parser::parse_source(&basis_points).expect("and the bps spelling must parse"),
    )
    .expect("and lower");
    assert_eq!(
        from_percent.operations.len(),
        from_points.operations.len(),
        "1% and 100 basis points are one bound, so the two programs are the same program"
    );
}

/// Where `%` is a percent and where it is still a modulo.
///
/// The percent fix is at the guard's bound, not in the literal reader, and this is the
/// boundary it draws. Three cases, all pinned:
///
/// 1. **Between non-literals** (`totals % slots`) — a modulo, untouched, because the fix
///    never touched the operator table.
/// 2. **Between integer literals outside a guard** (`let x = 1 + 2 * 3 - 4 / 5 % 6;`) — a
///    modulo, and this is the case that decided where the fix belongs: claiming every
///    `Int` followed by `%` as a percentage in the literal reader would have taken that
///    expression away, and `compiler/tests/test_parser_coverage.rs` exercises exactly it.
/// 3. **Inside a guard's bound** — `1%` is a percentage, which is the point of the fix, so
///    a modulo between literals *there* needs parentheses. That narrowing is deliberate
///    and recorded here rather than left to be discovered: the spelling it replaces
///    produced a guard whose value was the text `5 Percent 6`, which is a bound nobody
///    wrote.
#[test]
fn a_percent_in_a_guard_bound_is_a_percentage_and_a_modulo_everywhere_else() {
    fn guard_value(source: &str) -> Option<Expression> {
        let program = x3_lang_compiler::parser::parse_source(source).expect("the probe must parse");
        for item in &program.items {
            if let x3_lang_ast::ast::Item::IntentDecl(intent) = &item.node {
                for statement in &intent.body.stmts {
                    if let Statement::Require(guard) = statement {
                        return guard.value.clone();
                    }
                }
            }
        }
        None
    }

    let probe = |bound: &str| {
        format!(
            "intent probe {{\n    from ethereum.USDC amount 1\n    to solana.SOL\n    \
             require route_score matched == {bound}\n    on_fail rollback\n}}\n"
        )
    };

    // 1. Non-literals: still a modulo.
    assert!(
        matches!(guard_value(&probe("totals % slots")), Some(Expression::Binary { .. })),
        "`totals % slots` must still parse as a modulo"
    );

    // 2. Integer literals outside a guard: still a modulo. The parser-coverage shape.
    x3_lang_compiler::parser::parse_source("fn f() { let x = 1 + 2 * 3 - 4 / 5 % 6; }")
        .expect("`5 % 6` in a `let` must still parse as a modulo");

    // 3. Inside a guard's bound: `Int %` is the percent spelling.
    assert!(
        matches!(
            guard_value(&probe("1%")),
            Some(Expression::Literal(x3_lang_ast::ast::LiteralExpr::Percentage { .. }))
        ),
        "`1%` in a guard's bound is a percentage"
    );
    // And a modulo between literals there needs the parentheses, which is the narrowing
    // this fix makes on purpose.
    assert!(
        matches!(guard_value(&probe("(5 % 6)")), Some(Expression::Binary { .. })),
        "parenthesised, `5 % 6` is still a modulo inside a guard's bound"
    );
}
