//! Each built-in invariant rule has a case that fails **only** for it (TICKET-002b).
//!
//! The six rules were scoped to atomic route bodies because they fired on every well-formed intent —
//! which made them warnings that could not be errors, and a warning nobody can promote is a rule that
//! protects nothing. Scoping fixed the false positives; what this file holds is the other half of
//! "these are rules": for every rule there is an input that violates *that* rule and leaves the other
//! five silent. A rule that fires on another rule's negative case is noise wearing a name, and the
//! matrix below is what makes that visible — it is also what the promotion to errors rests on.

use x3_lang_compiler::ir::{Condition, FailureAction, Operation, ProgramMetadata, ReleaseAct, X3IR};
use x3_lang_compiler::semantic::get_builtin_invariants;

fn ir_with(operations: Vec<Operation>) -> X3IR {
    X3IR {
        operations,
        metadata: ProgramMetadata {
            nonce: Some("invariant-cases".to_owned()),
            chain_id: Some(1),
            timeout_blocks: Some(10),
        },
    }
}

fn lock(asset: &str) -> Operation {
    Operation::Lock {
        chain: "ethereum".to_owned(),
        asset: asset.to_owned(),
        amount: 100,
        from: "sender".to_owned(),
    }
}

fn claim(asset: &str, index: u32) -> Operation {
    Operation::Release {
        chain: "ethereum".to_owned(),
        asset: asset.to_owned(),
        to: "receiver".to_owned(),
        act: ReleaseAct::Claims(index),
    }
}

fn refund(asset: &str) -> Operation {
    Operation::OnFail {
        action: FailureAction::Refund {
            chain: "ethereum".to_owned(),
            asset: asset.to_owned(),
            to: "sender".to_owned(),
        },
    }
}

fn bridge() -> Operation {
    Operation::Bridge {
        via: "wormhole".to_owned(),
        from_chain: "ethereum".to_owned(),
        from_asset: "USDC".to_owned(),
        to_chain: "solana".to_owned(),
        to_asset: "SOL".to_owned(),
        amount: 100,
        receiver: "0xB1".to_owned(),
        source_finality_proof: Vec::new(),
        transfer_proof: Vec::new(),
    }
}

fn route_score() -> Operation {
    Operation::RouteScore {
        strategy: "scored".to_owned(),
        weights: [("uniswap".to_owned(), 100u32)].into_iter().collect(),
    }
}

/// One case per rule: the rule's name and an IR that violates it.
fn cases() -> Vec<(&'static str, X3IR)> {
    vec![
        // One lock, claimed twice inside one route.
        (
            "no_double_claim",
            ir_with(vec![
                Operation::AtomicBegin,
                lock("USDC"),
                claim("USDC", 0),
                claim("USDC", 0),
                Operation::AtomicEnd,
            ]),
        ),
        // One lock refunded by two handlers. The handlers are `OnFail`/`OnTimeout` rather than
        // `Release`, so the claim-ordering rules have nothing to read.
        ("no_double_refund", ir_with(vec![refund("USDC"), refund("USDC")])),
        // A refund, then a claim, in one route.
        (
            "no_claim_after_refund",
            ir_with(vec![
                Operation::AtomicBegin,
                refund("USDC"),
                claim("USDC", 0),
                Operation::AtomicEnd,
            ]),
        ),
        // A claim, then a refund of the same escrow, in one route.
        (
            "no_refund_after_claim",
            ir_with(vec![
                Operation::AtomicBegin,
                lock("USDC"),
                claim("USDC", 0),
                refund("USDC"),
                Operation::AtomicEnd,
            ]),
        ),
        // A claim before the bridge that fills the destination. A bridge must be present: the rule
        // returns early for a route that does not bridge, which is its scoping.
        (
            "destination_fill_before_source_claim",
            ir_with(vec![
                Operation::AtomicBegin,
                claim("USDC", 0),
                bridge(),
                Operation::AtomicEnd,
            ]),
        ),
        // The route is scored after the capital is locked, which can only mean the route moved.
        (
            "no_route_mutation_after_lock",
            ir_with(vec![lock("USDC"), route_score()]),
        ),
    ]
}

#[test]
fn every_rule_refuses_its_own_case_and_no_other_rule_does() {
    let rules = get_builtin_invariants();
    assert_eq!(rules.len(), 6, "the built-in set is six rules");
    let cases = cases();
    assert_eq!(cases.len(), rules.len(), "one case per rule");

    for (expected, ir) in &cases {
        let mut failed: Vec<&str> = rules
            .iter()
            .filter(|rule| (rule.check_fn)(ir).is_err())
            .map(|rule| rule.name.as_str())
            .collect();
        failed.sort_unstable();
        assert_eq!(
            failed,
            vec![*expected],
            "the case for `{expected}` must fail that rule and no other: {:?}",
            ir.operations
        );
    }
}

#[test]
fn the_set_of_cases_names_every_rule_exactly_once() {
    // The other direction: a rule with no case would make the matrix above pass by being absent from
    // it, which is the shape that lets a rule rot unnoticed.
    let rules = get_builtin_invariants();
    let mut rule_names: Vec<&str> = rules.iter().map(|rule| rule.name.as_str()).collect();
    rule_names.sort_unstable();
    let mut case_names: Vec<&str> = cases().iter().map(|(name, _)| *name).collect();
    case_names.sort_unstable();
    assert_eq!(rule_names, case_names);
}

#[test]
fn a_well_formed_route_trips_none_of_them() {
    // The scoping's own case, and the reason the rules can be errors: a route that locks an escrow,
    // bridges, fills the destination and claims the source is exactly what they are about, and none
    // of them has anything to say about it.
    let ir = ir_with(vec![
        Operation::AtomicBegin,
        lock("USDC"),
        bridge(),
        claim("USDC", 0),
        Operation::AtomicEnd,
        refund("USDC"),
    ]);
    let rules = get_builtin_invariants();
    let failed: Vec<&str> = rules
        .iter()
        .filter(|rule| (rule.check_fn)(&ir).is_err())
        .map(|rule| rule.name.as_str())
        .collect();
    assert!(
        failed.is_empty(),
        "a well-formed bridging route must trip none: {failed:?}"
    );
    let _ = Condition::True;
}
