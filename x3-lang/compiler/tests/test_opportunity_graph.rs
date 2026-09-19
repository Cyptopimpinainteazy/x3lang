//! The opportunity graph and its planner — spec build-order item 16.
//!
//! The tests below are the ones that make a graph search trustworthy rather
//! than plausible: it terminates on a cyclic graph, it never exceeds its hop
//! budget, its ranking is a function of the graph and not of iteration order,
//! and a constraint removes a path instead of merely ranking it lower.

use x3_lang_compiler::opportunity::{
    search, Opportunity, OpportunityConstraints, OpportunityGraph, RejectionReason, SearchOutcome,
};

/// Unwrap a finished search. A test that silently accepted an exhausted budget
/// would be testing the wrong outcome entirely.
fn found(outcome: SearchOutcome) -> Vec<Opportunity> {
    match outcome {
        SearchOutcome::Found(found) => found,
        SearchOutcome::BudgetExhausted { examined, budget } => {
            panic!("the search budget was exhausted ({examined} of {budget}); the test graph is tiny, so this is a bug")
        }
    }
}

fn graph_from(source: &str) -> OpportunityGraph {
    let program = x3_lang_compiler::parser::parse_source(source).expect("venues must parse");
    OpportunityGraph::from_program(&program)
}

/// Two ways from USDC to ETH, then one bridge to SOL.
const TWO_WAYS: &str = r#"
venue cheap_but_thin {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps 4
    liquidity 100_000
    slippage_bps 21
    latency_ms 14
    finality_blocks 12
    risk 3
    proof source_lock_proof
}

venue dear_but_deep {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps 7
    liquidity 4_000_000
    slippage_bps 8
    latency_ms 12
    finality_blocks 12
    risk 2
    proof source_lock_proof
}

venue to_solana {
    kind bridge
    chain ethereum
    domain evm
    asset_in ethereum.ETH
    asset_out solana.SOL
    fee_bps 2
    liquidity 500_000
    slippage_bps 5
    latency_ms 900
    finality_blocks 32
    risk 4
    proof destination_fill_proof
}
"#;

#[test]
fn the_graph_is_built_from_the_declarations() {
    let graph = graph_from(TWO_WAYS);
    assert_eq!(graph.edges.len(), 3, "one edge per venue");
    assert_eq!(graph.edges[0].from, "ethereum.USDC");
    assert_eq!(graph.edges[0].to, "ethereum.ETH");
    assert_eq!(graph.edges[2].to, "solana.SOL");
}

#[test]
fn both_ways_are_found_and_ranked_by_fee() {
    let graph = graph_from(TWO_WAYS);
    let constraints = OpportunityConstraints {
        max_hops: 4,
        ..Default::default()
    };
    let found = found(search(&graph, "ethereum.USDC", "solana.SOL", &constraints));
    assert_eq!(found.len(), 2, "both two-hop routes must be found: {found:?}");
    assert_eq!(
        found[0].venues,
        vec!["cheap_but_thin", "to_solana"],
        "the cheaper route ranks first on fee"
    );
    assert_eq!(found[0].fee_bps, 6);
    assert_eq!(found[1].fee_bps, 9);
}

#[test]
fn a_slippage_bound_removes_a_route_rather_than_ranking_it_lower() {
    let graph = graph_from(TWO_WAYS);
    let constraints = OpportunityConstraints {
        max_hops: 4,
        max_slippage_bps: Some(10),
        ..Default::default()
    };
    let found = found(search(&graph, "ethereum.USDC", "solana.SOL", &constraints));
    assert_eq!(found.len(), 1, "a bound is a filter, not a preference: {found:?}");
    assert_eq!(found[0].venues, vec!["dear_but_deep", "to_solana"]);
}

#[test]
fn a_liquidity_floor_removes_venues_that_cannot_absorb_the_size() {
    let graph = graph_from(TWO_WAYS);
    let constraints = OpportunityConstraints {
        max_hops: 4,
        min_liquidity: Some(1_000_000),
        ..Default::default()
    };
    let found = found(search(&graph, "ethereum.USDC", "solana.SOL", &constraints));
    // The bridge declares 500_000, so no path can absorb the size.
    assert!(found.is_empty(), "no route should survive: {found:?}");
}

#[test]
fn a_path_reports_the_worst_attribute_it_contains() {
    let graph = graph_from(TWO_WAYS);
    let constraints = OpportunityConstraints {
        max_hops: 4,
        ..Default::default()
    };
    let found = found(search(&graph, "ethereum.USDC", "solana.SOL", &constraints));
    let route = &found[0];
    // Slippage and finality are worst-case, not sums: a route is no better than
    // its weakest leg, and it is not settled until its slowest leg is.
    assert_eq!(route.slippage_bps, 21, "the worst leg's slippage, not the sum");
    assert_eq!(route.finality_blocks, 32);
    // Fee and latency are sums: both are actually paid along the way.
    assert_eq!(route.fee_bps, 6);
    assert_eq!(route.latency_ms, 914);
    assert_eq!(route.min_liquidity, 100_000, "the thinnest venue bounds the size");
}

#[test]
fn the_ranking_is_a_function_of_the_graph() {
    // Two graphs built from the same source, searched twice: the planner must
    // not depend on hash iteration order or on anything else that varies
    // between runs. A ranking that does is not reproducible, and an
    // unreproducible planner cannot be reviewed.
    let first = found(search(
        &graph_from(TWO_WAYS),
        "ethereum.USDC",
        "solana.SOL",
        &OpportunityConstraints {
            max_hops: 4,
            ..Default::default()
        },
    ));
    let second = found(search(
        &graph_from(TWO_WAYS),
        "ethereum.USDC",
        "solana.SOL",
        &OpportunityConstraints {
            max_hops: 4,
            ..Default::default()
        },
    ));
    assert_eq!(first, second, "the same graph must rank the same way");
}

#[test]
fn a_cyclic_graph_terminates() {
    // USDC -> ETH -> USDC -> ETH ... forever, if the search let it. A path that
    // may revisit an asset is not a finite set of opportunities.
    let cyclic = r#"
venue out {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps 5
    liquidity 1000
    slippage_bps 5
    latency_ms 10
    finality_blocks 12
    risk 1
}

venue back {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.ETH
    asset_out ethereum.USDC
    fee_bps 5
    liquidity 1000
    slippage_bps 5
    latency_ms 10
    finality_blocks 12
    risk 1
}
"#;
    let graph = graph_from(cyclic);
    let constraints = OpportunityConstraints {
        max_hops: 8,
        ..Default::default()
    };
    let found = found(search(&graph, "ethereum.USDC", "ethereum.ETH", &constraints));
    assert_eq!(
        found.len(),
        1,
        "a cycle round the same pair is not a second opportunity: {found:?}"
    );
    assert_eq!(found[0].venues, vec!["out"]);
}

#[test]
fn the_hop_budget_is_respected() {
    let graph = graph_from(TWO_WAYS);
    let one_hop = found(search(
        &graph,
        "ethereum.USDC",
        "solana.SOL",
        &OpportunityConstraints {
            max_hops: 1,
            ..Default::default()
        },
    ));
    assert!(one_hop.is_empty(), "the route needs two hops: {one_hop:?}");
}

#[test]
fn a_required_proof_filters_out_venues_that_declare_none() {
    let graph = graph_from(TWO_WAYS);
    let constraints = OpportunityConstraints {
        max_hops: 4,
        require_proof: true,
        ..Default::default()
    };
    let found = found(search(&graph, "ethereum.USDC", "solana.SOL", &constraints));
    assert_eq!(
        found.len(),
        2,
        "every venue in this graph declares a proof, so both routes survive: {found:?}"
    );
}

#[test]
fn a_chain_set_admits_paths_by_name_not_by_count() {
    // `chains = [x3, ethereum, solana]` is a set, not "at most three chains". The two
    // readings agree on a path's size and differ on which chains it may use, so a bound
    // that only counted would let a path through a chain nobody allowed.
    let graph = graph_from(TWO_WAYS);
    let ethereum_only = OpportunityConstraints {
        max_hops: 4,
        allowed_chains: Some(vec!["ethereum".to_string()]),
        ..Default::default()
    };
    // The route to solana.SOL crosses into solana, so the set refuses it...
    let refused = found(search(&graph, "ethereum.USDC", "solana.SOL", &ethereum_only));
    assert!(refused.is_empty(), "solana is not in the set: {refused:?}");
    // ...while the routes inside ethereum survive it.
    let inside = found(search(&graph, "ethereum.USDC", "ethereum.ETH", &ethereum_only));
    assert_eq!(inside.len(), 2, "both pools are on ethereum: {inside:?}");

    // Chain names are compared case-insensitively, the way every other chain reference in
    // this compiler is, so a declaration's spelling is not a second constraint.
    let capitalised = OpportunityConstraints {
        max_hops: 4,
        allowed_chains: Some(vec!["Ethereum".to_string()]),
        ..Default::default()
    };
    assert_eq!(
        found(search(&graph, "ethereum.USDC", "ethereum.ETH", &capitalised)).len(),
        2
    );

    // The refusal is the same judgement the search made, not a second opinion computed
    // beside it: the CLI prints these reasons.
    assert_eq!(
        x3_lang_compiler::opportunity::path_reject_reason(
            &["to_solana".to_string()],
            &["ethereum.ETH".to_string(), "solana.SOL".to_string()],
            &graph,
            &ethereum_only,
        ),
        Some(RejectionReason::ChainNotAllowed)
    );

    // An empty set admits nothing, which is the honest reading of an empty declaration
    // rather than a licence to search everything.
    let no_chains = OpportunityConstraints {
        max_hops: 4,
        allowed_chains: Some(Vec::new()),
        ..Default::default()
    };
    assert!(found(search(&graph, "ethereum.USDC", "ethereum.ETH", &no_chains)).is_empty());
}

#[test]
fn the_rejection_reason_names_the_bound_that_refused_the_edge() {
    // The CLI prints these, so they have to be the same judgement the search
    // made rather than a second opinion computed beside it.
    let graph = graph_from(TWO_WAYS);
    let constraints = OpportunityConstraints {
        max_slippage_bps: Some(10),
        ..Default::default()
    };
    let thin = graph.venue("cheap_but_thin").expect("declared");
    assert_eq!(
        x3_lang_compiler::opportunity::reject_reason(thin, &constraints),
        Some(RejectionReason::SlippageAboveBound)
    );
    let deep = graph.venue("dear_but_deep").expect("declared");
    assert_eq!(x3_lang_compiler::opportunity::reject_reason(deep, &constraints), None);
}
