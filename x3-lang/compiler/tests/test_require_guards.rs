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
