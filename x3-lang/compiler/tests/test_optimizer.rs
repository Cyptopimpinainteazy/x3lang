//! The deterministic route optimizer — spec build-order item 17.
//!
//! PHASE 42 forbids non-deterministic optimizer output and prescribes bounded
//! search, stable ordering and canonical tie-breaking. These tests are those
//! three requirements plus the one that is easiest to get quietly wrong: a tie
//! must be *reported* rather than resolved in silence, and an exhausted budget
//! must not be reported as "no route".

use x3_lang_compiler::opportunity::{OpportunityConstraints, OpportunityGraph};
use x3_lang_compiler::optimizer::{optimize, optimize_with_budget, NoRoute, Objective};

/// A venue declaration with every field present.
fn venue(name: &str, fee_bps: u32, slippage_bps: u32, latency_ms: u32) -> String {
    format!(
        r#"venue {name} {{
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps {fee_bps}
    liquidity 1_000_000
    slippage_bps {slippage_bps}
    latency_ms {latency_ms}
    finality_blocks 12
    risk 2
}}
"#
    )
}

fn bridge(name: &str) -> String {
    format!(
        r#"venue {name} {{
    kind bridge
    chain ethereum
    domain evm
    asset_in ethereum.ETH
    asset_out solana.SOL
    fee_bps 2
    liquidity 1_000_000
    slippage_bps 5
    latency_ms 900
    finality_blocks 32
    risk 4
}}
"#
    )
}

fn graph_from(source: &str) -> OpportunityGraph {
    let program = x3_lang_compiler::parser::parse_source(source).expect("venues must parse");
    OpportunityGraph::from_program(&program)
}

fn constraints() -> OpportunityConstraints {
    OpportunityConstraints {
        max_hops: 4,
        ..Default::default()
    }
}

/// Two routes to ETH, then one bridge. `cheap` is cheap and slippy, `tight` is
/// dear and tight, so the fee and slippage objectives disagree.
fn disagreement() -> String {
    format!(
        "{}{}{}",
        venue("cheap", 4, 21, 14),
        venue("tight", 7, 8, 12),
        bridge("to_solana")
    )
}

#[test]
fn the_objective_decides_which_route_wins() {
    let graph = graph_from(&disagreement());
    let by_fee = optimize(
        &graph,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeFees,
        &constraints(),
    );
    let by_slippage = optimize(
        &graph,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeSlippage,
        &constraints(),
    );
    assert_eq!(
        by_fee.chosen.as_ref().expect("a route").venues,
        vec!["cheap", "to_solana"],
        "cheapest fee wins on fees"
    );
    assert_eq!(
        by_slippage.chosen.as_ref().expect("a route").venues,
        vec!["tight", "to_solana"],
        "and the other route wins on slippage — otherwise the objective is decorative"
    );
    assert_eq!(by_fee.objective_value, Some(6));
    assert_eq!(by_slippage.objective_value, Some(8));
}

#[test]
fn optimizing_twice_gives_the_same_answer() {
    let graph = graph_from(&disagreement());
    let first = optimize(
        &graph,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeFees,
        &constraints(),
    );
    let second = optimize(
        &graph,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeFees,
        &constraints(),
    );
    assert_eq!(first, second, "the optimizer must be a function of its inputs");
}

#[test]
fn a_tie_on_the_objective_is_reported_rather_than_hidden() {
    // Two venues with identical attributes: the objective cannot separate them,
    // so the canonical tie-break does. A caller that is not told has been misled
    // about whether the objective actually decided.
    let source = format!(
        "{}{}{}",
        venue("alpha", 5, 10, 12),
        venue("beta", 5, 10, 12),
        bridge("to_solana")
    );
    let graph = graph_from(&source);
    let report = optimize(
        &graph,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeFees,
        &constraints(),
    );
    assert!(!report.decided_by_objective(), "the objective did not decide this");
    assert_eq!(report.tied.len(), 1, "the tying route must be surfaced: {report:?}");
    assert_eq!(report.tied[0].venues, vec!["beta", "to_solana"]);
}

#[test]
fn the_tie_break_is_canonical_not_declaration_order() {
    // The same two venues, declared in the opposite order. A tie-break that
    // depended on the search's iteration order would pick `alpha` here and
    // `beta` there, which makes the optimizer's output a property of the file
    // layout rather than of the graph.
    let forward = graph_from(&format!(
        "{}{}{}",
        venue("alpha", 5, 10, 12),
        venue("beta", 5, 10, 12),
        bridge("to_solana")
    ));
    let reversed = graph_from(&format!(
        "{}{}{}",
        venue("beta", 5, 10, 12),
        venue("alpha", 5, 10, 12),
        bridge("to_solana")
    ));
    let first = optimize(
        &forward,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeFees,
        &constraints(),
    );
    let second = optimize(
        &reversed,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeFees,
        &constraints(),
    );
    assert_eq!(
        first.chosen.as_ref().expect("a route").venues,
        second.chosen.as_ref().expect("a route").venues,
        "the winner must not depend on the order the venues were declared in"
    );
}

#[test]
fn an_infeasible_request_names_the_blocking_constraint() {
    let graph = graph_from(&disagreement());
    let tight = OpportunityConstraints {
        max_hops: 4,
        max_slippage_bps: Some(3),
        ..Default::default()
    };
    let report = optimize(&graph, "ethereum.USDC", "solana.SOL", Objective::MinimizeFees, &tight);
    assert!(report.chosen.is_none(), "nothing should be chosen: {report:?}");
    match report.no_route.expect("a reason") {
        NoRoute::AllRefused { refused } => {
            assert_eq!(
                refused.len(),
                3,
                "every venue is refused, and each is named: {refused:?}"
            );
            assert!(refused
                .iter()
                .all(|(_, reason)| *reason == x3_lang_compiler::opportunity::RejectionReason::SlippageAboveBound));
        }
        other => panic!("expected the refusals to be reported, got {other:?}"),
    }
}

#[test]
fn an_unreachable_route_is_distinguished_from_a_refused_one() {
    // Non-vacuous: with no venues at all there is nothing to refuse, and the
    // report must say "none" rather than an empty refusal list.
    let graph = graph_from(&venue("lonely", 5, 10, 12));
    let report = optimize(
        &graph,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeFees,
        &constraints(),
    );
    assert_eq!(report.no_route, Some(NoRoute::None));
}

#[test]
fn exhausting_the_budget_is_not_reported_as_no_route() {
    let graph = graph_from(&disagreement());
    let report = optimize_with_budget(
        &graph,
        "ethereum.USDC",
        "solana.SOL",
        Objective::MinimizeFees,
        &constraints(),
        1,
    );
    assert!(report.chosen.is_none());
    match report.no_route.expect("a reason") {
        NoRoute::BudgetExhausted { examined, budget } => {
            assert_eq!(budget, 1);
            assert!(
                examined > budget,
                "the budget must be reported as exceeded, got {examined}"
            );
        }
        other => panic!("an unfinished search must not look like an empty one, got {other:?}"),
    }
}

#[test]
fn the_objective_set_is_closed_and_round_trips() {
    for objective in Objective::ALL {
        assert_eq!(Objective::parse(objective.as_str()), Some(*objective));
    }
    assert_eq!(Objective::parse("maximize_vibes"), None);
}
