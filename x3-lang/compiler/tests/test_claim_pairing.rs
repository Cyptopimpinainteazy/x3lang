//! A `Release` names the lock it claims, so a book settles as one unit (TICKET-080).
//!
//! PHASE 22's point is that several obligations can be discharged by the transfers that remain
//! after they are offset — and netting is only valid if the whole residual set settles or none
//! of it does. If some transfers settle and others do not, the positions that result are not the
//! positions the offsetting preserved.
//!
//! That needed the whole book in one atomic route, and the route was impossible: a `Release`
//! named its claim by asset alone, so two transfers of one asset were two claims no reader could
//! tell apart. A book nets *within* one asset, so that was the normal case rather than a corner,
//! and `no_double_claim` refused it. The claim now names a lock by its position among the
//! route's, which is a pairing a replayer can check rather than infer.

use x3_lang_compiler::ir::{Operation, ProgramMetadata, X3IR};
use x3_lang_compiler::semantic::CompilationMode;

/// A book whose residual is **two transfers of one asset** — the case that needed one route per
/// transfer. Both pay `bob`; they differ in who funds them.
const BOOK: &str = "netting book_a {\n    consent alice;\n    consent bob;\n    consent carol;\n    \
                    account alice = 0xA1;\n    account bob = 0xB1;\n    account carol = 0xC1;\n    \
                    alice owes 500 ethereum.USDC to bob;\n    bob owes 300 ethereum.USDC to \
                    alice;\n    carol owes 120 ethereum.USDC to alice;\n    alice owes 40 \
                    ethereum.USDC to bob;\n}\n";

fn lowered(source: &str) -> X3IR {
    let (_, ir, outcome) =
        x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev).expect("it must lower");
    assert!(outcome.errors.is_empty(), "it must check: {:?}", outcome.errors);
    ir
}

fn ir_with(operations: Vec<Operation>) -> X3IR {
    X3IR {
        operations,
        metadata: ProgramMetadata {
            nonce: Some("nonce-1".to_owned()),
            chain_id: Some(1),
            timeout_blocks: Some(10),
        },
    }
}

fn lock_to_release(ir: &X3IR) -> Vec<(usize, u32)> {
    // `(locks written so far, the claim index)` for each release, in order — which is exactly
    // the pairing a replayer does from the artifact.
    let mut locks = 0usize;
    let mut pairs = Vec::new();
    for op in &ir.operations {
        match op {
            Operation::Lock { .. } => locks += 1,
            Operation::Release { claims, .. } => pairs.push((locks, *claims)),
            _ => {}
        }
    }
    pairs
}

/// The whole residual set is one route, and each release names its own lock. Two claims of one
/// asset are distinguishable here, which is what the route could not express before.
#[test]
fn a_book_with_two_same_asset_transfers_settles_as_one_route() {
    let ir = lowered(BOOK);

    let begins = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::AtomicBegin))
        .count();
    assert_eq!(
        begins, 1,
        "the book must be one atomic route: all-or-nothing across the residual set is what makes \
         the offsetting valid, and one route per transfer settles them independently"
    );
    let locks = ir
        .operations
        .iter()
        .filter(|op| matches!(op, Operation::Lock { .. }))
        .count();
    assert_eq!(locks, 2, "the residual is two transfers, so two locks");

    let pairs = lock_to_release(&ir);
    assert_eq!(
        pairs,
        vec![(1, 0), (2, 1)],
        "each release names the lock it claims: the first the route's first lock, the second the \
         second. Two claims of one asset, told apart — which is what the field is for"
    );
    // And the two claims are of the *same asset*, so the distinction cannot be coming from the
    // asset: this is the case that used to collide.
    let assets: Vec<&str> = ir
        .operations
        .iter()
        .filter_map(|op| match op {
            Operation::Release { asset, .. } => Some(asset.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(
        assets,
        vec!["USDC", "USDC"],
        "two transfers of one asset, and they are still distinguishable: {assets:?}"
    );
}

/// The pairing survives into the artifact, so a replayer reading it can check the pairing rather
/// than infer it — the ticket's own validation.
#[test]
fn the_pairing_reaches_the_artifact() {
    let ir = lowered(BOOK);
    let bytecode = x3_lang_compiler::emitter::emit_x3ir(&ir).expect("the book must emit");
    let trace = x3_lang_compiler::emitter::disassemble(&bytecode).expect("its own artifact must disassemble");

    assert!(
        trace.contains("claims: 0") && trace.contains("claims: 1"),
        "both claims must be readable out of the artifact:\n{trace}"
    );
}

/// The rule that made one route per transfer necessary is still a rule: a route may not claim one
/// lock twice. The claim index is what it counts now, so the refusal names the lock rather than
/// the asset.
#[test]
fn a_route_claiming_one_lock_twice_is_refused() {
    let ir = ir_with(vec![
        Operation::AtomicBegin,
        Operation::Lock {
            chain: "ethereum".into(),
            asset: "USDC".into(),
            amount: 10,
            from: "0xA1".into(),
        },
        Operation::Release {
            chain: "ethereum".into(),
            asset: "USDC".into(),
            to: "0xB1".into(),
            claims: 0,
        },
        Operation::Release {
            chain: "ethereum".into(),
            asset: "USDC".into(),
            to: "0xC1".into(),
            claims: 0,
        },
        Operation::AtomicEnd,
    ]);
    // The rule is an *invariant*, not a structural check: it reasons about execution order
    // within a route, which is what `atomic_scoped_operations` is for.
    let rules = x3_lang_compiler::semantic::get_builtin_invariants();
    let rule = rules
        .iter()
        .find(|rule| rule.name == "no_double_claim")
        .expect("the rule is built in");
    let refusal = (rule.check_fn)(&ir).expect_err("one lock claimed twice must be refused");
    assert!(
        refusal.contains("same lock") && refusal.contains("#0"),
        "the refusal must name the lock it is about: {refusal}"
    );

    // And the same route with two locks and two claims is accepted, so the rule refuses the
    // double claim rather than the shape.
    let ok = ir_with(vec![
        Operation::AtomicBegin,
        Operation::Lock {
            chain: "ethereum".into(),
            asset: "USDC".into(),
            amount: 10,
            from: "0xA1".into(),
        },
        Operation::Release {
            chain: "ethereum".into(),
            asset: "USDC".into(),
            to: "0xB1".into(),
            claims: 0,
        },
        Operation::Lock {
            chain: "ethereum".into(),
            asset: "USDC".into(),
            amount: 4,
            from: "0xC1".into(),
        },
        Operation::Release {
            chain: "ethereum".into(),
            asset: "USDC".into(),
            to: "0xB1".into(),
            claims: 1,
        },
        Operation::AtomicEnd,
    ]);
    (rule.check_fn)(&ok).expect("two locks claimed once each is the case this exists for");
}
