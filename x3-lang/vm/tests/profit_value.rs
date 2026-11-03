//! Profit as a first-class value — spec PHASE 5.
//!
//! "Profit must not be represented as an arbitrary integer." These tests are the
//! decomposition itself: that it adds up, that no cost kind can vanish from it,
//! that a loss is a loss, and that the two quantities the language cannot know
//! are absent with a reason rather than present as zeros.

use x3_lang_compiler::ir::CostKind;
use x3_lang_vm::profit::{describe, LedgerCost, Profit};

fn cost(kind: CostKind, amount: u128) -> LedgerCost {
    LedgerCost {
        amount,
        kind: kind.as_str().to_string(),
    }
}

#[test]
fn the_decomposition_adds_up() {
    let costs = vec![
        cost(CostKind::Gas, 30),
        cost(CostKind::LiquidityFee, 1_000),
        cost(CostKind::FlashLiquidityFee, 200),
        cost(CostKind::ProofFee, 50),
    ];
    let profit = Profit::from_ledger(20_000, 5_000, 100, &costs).expect("a known ledger");
    assert_eq!(profit.total_costs(), 1_280);
    // 20_000 - 5_000 - 1_280 - 100
    assert_eq!(profit.net, 13_620);
    assert_eq!(profit.gross, 20_000);
    assert_eq!(profit.principal, 5_000);
    assert_eq!(profit.safety_buffer, 100);
}

#[test]
fn every_cost_kind_has_somewhere_to_go() {
    // The behavioural form of the exhaustiveness the field list enforces: give
    // the ledger one cost of each kind and require the total to see all of them.
    // A kind that fell through the mapping would make `net` too large, which is
    // the worst direction for an error in a profit figure.
    let costs: Vec<LedgerCost> = CostKind::ALL.iter().map(|kind| cost(*kind, 10)).collect();
    let profit = Profit::from_ledger(1_000, 0, 0, &costs).expect("every kind is known");
    assert_eq!(
        profit.total_costs(),
        10 * CostKind::ALL.len() as u128,
        "every kind must reach the total"
    );
    assert_eq!(profit.net, 1_000 - profit.total_costs() as i128);
}

#[test]
fn a_cost_kind_this_version_does_not_know_is_refused() {
    // Omitting it would inflate the profit; guessing where it belongs would
    // invent an answer. Neither is acceptable, so the assembly fails.
    let costs = vec![LedgerCost {
        amount: 5,
        kind: "invented_fee".to_string(),
    }];
    let refused = Profit::from_ledger(100, 0, 0, &costs).expect_err("an unknown kind is refused");
    assert!(
        refused.to_string().contains("invented_fee"),
        "the refusal must name the kind: {refused}"
    );
}

#[test]
fn a_loss_is_a_loss_and_not_zero_profit() {
    // The accessor this replaced clamped the net delta at zero, so a losing trade
    // read as "profit 0" and satisfied a floor of zero.
    let costs = vec![cost(CostKind::Gas, 500)];
    let profit = Profit::from_ledger(1_000, 900, 0, &costs).expect("a known ledger");
    assert_eq!(profit.net, -400, "1_000 - 900 - 500");
    assert!(
        profit.margin_bps < 0,
        "a loss has a negative margin: {}",
        profit.margin_bps
    );
    assert_eq!(profit.realized(), -400);
}

#[test]
fn the_margin_is_basis_points_of_gross_without_a_float() {
    // 1_000 net on 100_000 gross is 100 bps, exactly.
    let profit = Profit::from_ledger(100_000, 0, 0, &[]).expect("an empty ledger");
    assert_eq!(profit.net, 100_000);
    assert_eq!(profit.margin_bps, 10_000);

    // 25 bps, and the integer division truncates toward zero rather than
    // rounding into a different tier.
    let profit = Profit::from_ledger(100_000, 99_750, 0, &[]).expect("an empty ledger");
    assert_eq!(profit.net, 250);
    assert_eq!(profit.margin_bps, 25);
}

#[test]
fn a_margin_against_no_proceeds_is_zero_rather_than_a_division() {
    let profit = Profit::from_ledger(0, 0, 0, &[]).expect("an empty ledger");
    assert_eq!(profit.margin_bps, 0, "a share of nothing is not a number to invent");
}

#[test]
fn the_specs_grouping_names_are_derived_from_the_kinds() {
    let costs = vec![
        cost(CostKind::LiquidityFee, 100),
        cost(CostKind::FlashLiquidityFee, 10),
        cost(CostKind::SolverInfrastructureFee, 5),
        cost(CostKind::ProofFee, 3),
        cost(CostKind::CrossDomainFee, 2),
        cost(CostKind::Slippage, 7),
        cost(CostKind::PriceImpact, 1),
        cost(CostKind::MevLeakage, 1),
    ];
    let profit = Profit::from_ledger(10_000, 0, 0, &costs).expect("a known ledger");
    assert_eq!(profit.dex_fees(), 100);
    assert_eq!(profit.flash_fees(), 10);
    assert_eq!(profit.protocol_fees(), 10, "solver, proof and cross-domain together");
    assert_eq!(profit.execution_costs(), 9, "slippage, price impact and MEV leakage");
    assert_eq!(
        profit.gas + profit.dex_fees() + profit.flash_fees() + profit.protocol_fees() + profit.execution_costs(),
        profit.total_costs(),
        "the grouping must account for every kind exactly once"
    );
}

#[test]
fn the_two_quantities_the_language_cannot_know_are_absent_with_a_reason() {
    let profit = Profit::from_ledger(1_000, 0, 0, &[]).expect("an empty ledger");
    assert_eq!(
        profit.hedging_costs(),
        None,
        "no hedging is modelled, so a zero here would claim a hedge cost nothing"
    );
    assert_eq!(
        profit.unrealized(),
        None,
        "unrealised profit needs a price, which this path has none of"
    );
    assert_eq!(profit.realized(), profit.net, "everything in the ledger has settled");
}

#[test]
fn the_description_reads_as_the_decomposition() {
    let costs = vec![cost(CostKind::LiquidityFee, 1_000)];
    let profit = Profit::from_ledger(20_000, 5_000, 100, &costs).expect("a known ledger");
    let text = describe(&profit);
    assert!(text.contains("gross 20000"), "{text}");
    assert!(text.contains("principal 5000"), "{text}");
    assert!(text.contains("= net 13900"), "{text}");
}
