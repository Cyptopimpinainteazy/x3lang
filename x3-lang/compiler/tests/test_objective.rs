//! Objective declarations — spec PHASE 15.
//!
//! A declaration that says what to rank is worth nothing unless the ranking
//! follows it, so these tests are about two things: a metric the graph cannot
//! rank is refused *with its reason* rather than accepted and ignored, and the
//! ceilings a declaration states are the ones the search applies — including the
//! two that only a whole path can break, its fee total and its chain count.

use x3_lang_common::ErrorAccumulator;
use x3_lang_compiler::objective::{constraints_for, criterion_for, verify_objective_decls};
use x3_lang_compiler::opportunity::{
    path_reject_reason, search, OpportunityConstraints, OpportunityGraph, RejectionReason, SearchOutcome,
};
use x3_lang_compiler::semantic::CompilationMode;

fn errors(source: &str) -> Vec<String> {
    match x3_lang_compiler::check_source_diagnostics_with_mode(source, CompilationMode::Dev) {
        Ok((_, _, outcome)) => outcome.errors.iter().map(|error| error.to_string()).collect(),
        Err(error) => vec![format!("{error}")],
    }
}

fn parse(source: &str) -> x3_lang_ast::ast::Program {
    x3_lang_compiler::parser::parse_source(source).expect("the fixture must parse")
}

fn has(errors: &[String], needle: &str) -> bool {
    errors.iter().any(|error| error.contains(needle))
}

/// Two ways from USDC to ETH on one chain, then a bridge. Every ceiling in the
/// fixtures below is chosen to be loose enough that only the clause under test
/// can remove a route.
const VENUES: &str = r#"
venue deep_pool {
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

venue wide_pool {
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

venue x3_bridge {
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

/// A strategy module. `risk` and `submission` are whole lines, so a caller can
/// omit either — a module with no risk profile is itself an error, which is why
/// the case of "no profile to bound by" is written as "no module at all".
fn strategy(name: &str, risk: &str, submission: &str) -> String {
    format!(
        r#"strategy {name} {{
    input ethereum.USDC amount 25_000
    output ethereum.ETH
    effects [swap]
    guarantees [min_profit]
    domains [ethereum]
{risk}{submission}    bounds {{ max_steps 10 max_gas 200_000 }}
    execute {{
        swap uniswap ethereum.USDC -> ethereum.ETH amount 1_000 min_output 1
        require slippage <= 50
        require profit >= 5
        on_fail refund ethereum.USDC to sender
    }}
}}
"#
    )
}

fn risk_profile() -> &'static str {
    "    risk { max_slippage_bps 50 max_total_fee_bps 8 }\n"
}

fn objective(body: &str) -> String {
    format!("objective cheapest_route {{\n{body}\n}}\n")
}

fn program(objective_body: &str) -> String {
    format!(
        "{}{}{}",
        strategy("RoutePolicy", risk_profile(), ""),
        objective(objective_body),
        VENUES
    )
}

#[test]
fn an_objective_with_every_constraint_is_accepted() {
    let source = program(
        "    minimize fees\n    constraints {\n        hops <= 3\n        chains <= 2\n        \
         risk <= strategy.policy\n        execution_time <= 2_000 ms\n        fees <= 100 bps\n        \
         slippage <= 100 bps\n        finality <= 64 blocks\n        capital <= 25_000 USDC\n        \
         atomic\n    }",
    );
    assert_eq!(errors(&source), Vec::<String>::new(), "the declaration must verify");
}

#[test]
fn a_metric_the_graph_cannot_rank_is_refused_with_its_reason() {
    // The four the optimizer has no quantity for. Accepting one would leave the
    // ranking to whatever solver ran, which PHASE 15 forbids — so each is
    // refused, and refused by name.
    for metric in [
        "maximize profit",
        "maximize output",
        "minimize external_liquidity",
        "maximize capital_efficiency",
    ] {
        let source = program(&format!("    {metric}\n    constraints {{\n        hops <= 3\n    }}"));
        let errors = errors(&source);
        assert!(
            has(&errors, "cannot rank"),
            "'{metric}' must be refused with a reason, got: {errors:?}"
        );
    }
}

#[test]
fn a_metric_the_graph_can_rank_is_accepted() {
    for metric in [
        "minimize fees",
        "minimize slippage",
        "minimize risk",
        "minimize execution_time",
        "minimize finality",
    ] {
        let source = program(&format!("    {metric}\n    constraints {{\n        hops <= 3\n    }}"));
        assert_eq!(
            errors(&source),
            Vec::<String>::new(),
            "'{metric}' is one the optimizer ranks and must be accepted"
        );
    }
}

#[test]
fn an_objective_that_ranks_two_things_is_refused() {
    let source = program("    minimize fees\n    minimize risk\n    constraints {\n        hops <= 3\n    }");
    let errors = errors(&source);
    assert!(
        has(&errors, "metric twice"),
        "a declaration that ranks two things has no defined answer, got: {errors:?}"
    );
}

#[test]
fn a_direction_that_does_not_go_with_the_metric_is_refused() {
    let source = program("    maximize fees\n    constraints {\n        hops <= 3\n    }");
    let errors = errors(&source);
    assert!(
        has(&errors, "minimize") && has(&errors, "not to maximize"),
        "the message must name the direction the metric actually has, got: {errors:?}"
    );
}

#[test]
fn an_objective_with_no_metric_is_refused() {
    let source = program("    constraints {\n        hops <= 3\n    }");
    let errors = errors(&source);
    assert!(
        has(&errors, "states no metric"),
        "an objective that does not say what to rank is a heading, got: {errors:?}"
    );
}

#[test]
fn a_second_objective_is_refused() {
    let source = format!(
        "{}{}{}{}",
        strategy("RoutePolicy", risk_profile(), ""),
        objective("    minimize fees\n    constraints {\n        hops <= 3\n    }"),
        objective("    minimize risk\n    constraints {\n        hops <= 3\n    }"),
        VENUES
    );
    let errors = errors(&source);
    assert!(
        has(&errors, "declares 2 objectives"),
        "the compiler follows one objective, so a second has no effect, got: {errors:?}"
    );
}

#[test]
fn two_objectives_with_one_name_are_refused() {
    let source = format!(
        "{}{}{}",
        objective("    minimize fees\n    constraints {\n        hops <= 3\n    }"),
        objective("    minimize risk\n    constraints {\n        hops <= 3\n    }"),
        VENUES
    );
    let errors = errors(&source);
    assert!(
        has(&errors, "is declared twice"),
        "two declarations with one name cannot be told apart, got: {errors:?}"
    );
}

#[test]
fn an_objective_over_a_program_with_no_venues_is_refused() {
    let source = objective("    minimize fees\n    constraints {\n        hops <= 3\n    }");
    let errors = errors(&source);
    assert!(
        has(&errors, "no venues"),
        "there is nothing to rank, and a declaration that ranks nothing should say so, got: {errors:?}"
    );
}

#[test]
fn risk_by_strategy_policy_needs_a_profile_to_read() {
    let source = format!(
        "{}{}",
        objective("    minimize fees\n    constraints {\n        risk <= strategy.policy\n    }"),
        VENUES
    );
    let errors = errors(&source);
    assert!(has(&errors, "no strategy declares a risk profile"), "got: {errors:?}");
}

#[test]
fn risk_by_strategy_policy_is_refused_when_two_modules_declare_one() {
    let source = format!(
        "{}{}{}{}",
        strategy("First", risk_profile(), ""),
        strategy("Second", risk_profile(), ""),
        objective("    minimize fees\n    constraints {\n        risk <= strategy.policy\n    }"),
        VENUES
    );
    let errors = errors(&source);
    assert!(
        has(&errors, "ambiguous"),
        "two profiles means there is no answer, and the compiler must say so, got: {errors:?}"
    );
}

#[test]
fn a_policy_bound_is_the_tighter_of_the_two_ceilings() {
    // The profile says 50 bps slippage and 8 bps fee; the objective says 40 bps
    // slippage and nothing about fees. Two ceilings on one quantity are still
    // both ceilings, so slippage is 40 and the fee bound comes from the profile.
    let source = program(
        "    minimize fees\n    constraints {\n        risk <= strategy.policy\n        slippage <= 40 bps\n    }",
    );
    let program = parse(&source);
    let mut acc = ErrorAccumulator::new();
    verify_objective_decls(&program, &mut acc);
    assert!(!acc.has_errors(), "{:?}", acc.errors());

    let declaration = x3_lang_compiler::objective::declaration_of(&program).expect("one objective");
    let constraints = constraints_for(&declaration.constraints, &program);
    assert_eq!(constraints.max_slippage_bps, Some(40));
    assert_eq!(constraints.max_fee_bps, Some(8));
    // A profile bounds slippage and fee, not a risk *score*, so nothing is
    // invented for the venue-risk comparison.
    assert_eq!(constraints.max_risk, None);
}

#[test]
fn capital_becomes_a_floor_on_what_each_venue_can_absorb() {
    let source = program("    minimize fees\n    constraints {\n        capital <= 25_000 USDC\n    }");
    let program = parse(&source);
    let declaration = x3_lang_compiler::objective::declaration_of(&program).expect("one objective");
    let constraints = constraints_for(&declaration.constraints, &program);
    assert_eq!(constraints.min_liquidity, Some(25_000));
}

#[test]
fn an_attached_unit_is_read_as_the_unit_the_field_uses() {
    // `2000ms` is one word to the lexer, and PHASE 15 writes the unit that way,
    // so the suffix is read and checked rather than dropped: the value is in
    // milliseconds exactly as the field's name says.
    let source = program(
        "    minimize fees\n    constraints {\n        execution_time <= 2_000ms\n        fees <= 30bps\n        \
         finality <= 64blocks\n    }",
    );
    assert_eq!(
        errors(&source),
        Vec::<String>::new(),
        "the attached spelling must be understood"
    );
    let program = parse(&source);
    let declaration = x3_lang_compiler::objective::declaration_of(&program).expect("one objective");
    let constraints = constraints_for(&declaration.constraints, &program);
    assert_eq!(constraints.max_latency_ms, Some(2_000));
    assert_eq!(constraints.max_fee_bps, Some(30));
    assert_eq!(constraints.max_finality_blocks, Some(64));
}

#[test]
fn a_unit_that_does_not_go_with_the_field_is_refused() {
    // The suffix is the program saying what it thinks the number means. Reading
    // the digits and discarding the rest would store a number nobody wrote.
    let seconds = program("    minimize fees\n    constraints {\n        execution_time <= 2_000s\n    }");
    let second_errors = errors(&seconds);
    assert!(
        has(&second_errors, "measured in ms") && has(&second_errors, "write '2_000 ms'"),
        "got: {second_errors:?}"
    );

    let counted = program("    minimize fees\n    constraints {\n        hops <= 4x\n    }");
    let counted_errors = errors(&counted);
    assert!(
        has(&counted_errors, "counts, so it has no unit"),
        "got: {counted_errors:?}"
    );
}

#[test]
fn a_repeated_clause_is_refused() {
    // The second clause would replace the first, so one of the two ceilings the
    // program wrote would be in force nowhere.
    let source = program("    minimize fees\n    constraints {\n        hops <= 3\n        hops <= 5\n    }");
    let errors = errors(&source);
    assert!(has(&errors, "ceiling on 'hops' twice"), "got: {errors:?}");
}

#[test]
fn the_specs_own_example_is_understood_and_refused_for_its_reason() {
    // PHASE 15's example verbatim: anonymous objective, semicolon-separated
    // clauses, `capital <= 25_000_000 USDC`, `maximize net_profit`, and the
    // attached `200ms`. It has to *parse* — a program written to the spec that
    // failed with "unknown metric" would mean the surface and the spec had
    // drifted apart.
    let source = format!(
        "{}{}{}",
        strategy("RoutePolicy", risk_profile(), "    submission { private = required }\n"),
        r#"objective {
    maximize net_profit;

    constraints {
        capital <= 25_000_000 USDC;
        hops <= 10;
        chains <= 4;
        risk <= strategy.policy;
        execution_time <= 200ms;
        private;
        atomic;
    }
}
"#,
        VENUES
    );
    let errors = errors(&source);
    assert_eq!(
        errors.len(),
        1,
        "the spec's example verifies apart from its metric: {errors:?}"
    );
    assert!(
        has(&errors, "cannot rank 'maximize net_profit'") && has(&errors, "PHASE 15 forbids"),
        "the refusal has to be the phase's own determinism requirement: {errors:?}"
    );
    // An anonymous declaration is referred to as "the objective", not
    // `objective 'objective'`.
    assert!(has(&errors, "the objective cannot rank"), "got: {errors:?}");
}

#[test]
fn both_spellings_of_the_profit_metric_reach_the_same_refusal() {
    // The example writes `net_profit`; the phase's list of objectives writes
    // `profit`. One metric, one reason it cannot be ranked.
    for spelling in ["maximize net_profit", "maximize profit"] {
        let source = program(&format!(
            "    {spelling}\n    constraints {{\n        hops <= 3\n    }}"
        ));
        let errors = errors(&source);
        assert!(
            has(&errors, "cannot rank") && has(&errors, "ranking them would fall to a solver"),
            "'{spelling}' must reach the refusal, not an unknown word: {errors:?}"
        );
    }
}

#[test]
fn a_ceiling_of_zero_or_above_the_whole_amount_is_refused() {
    let zero = program("    minimize fees\n    constraints {\n        hops <= 0\n    }");
    assert!(
        has(&errors(&zero), "bounds hops at zero"),
        "a route with no hops is not a route"
    );

    let whole = program("    minimize fees\n    constraints {\n        fees <= 10_001 bps\n    }");
    assert!(
        has(&errors(&whole), "above 10,000 bps"),
        "10,000 bps is the whole amount, so a ceiling above it bounds nothing"
    );
}

#[test]
fn an_objective_demanding_private_execution_needs_a_policy_that_provides_it() {
    let bare = program("    minimize fees\n    constraints {\n        private\n    }");
    assert!(
        has(&errors(&bare), "no strategy declares a submission policy"),
        "an objective asking for privacy beside no such policy asks for what the artifact does not declare"
    );

    let declared = format!(
        "{}{}{}",
        strategy("RoutePolicy", risk_profile(), "    submission { private = required }\n"),
        objective("    minimize fees\n    constraints {\n        private\n    }"),
        VENUES
    );
    assert_eq!(errors(&declared), Vec::<String>::new());
}

#[test]
fn a_capital_with_no_positive_amount_is_refused() {
    let source = program("    minimize fees\n    constraints {\n        capital <= get_amount() USDC\n    }");
    let errors = errors(&source);
    assert!(has(&errors, "no positive integer amount"), "got: {errors:?}");
}

#[test]
fn every_objective_the_optimizer_ranks_is_declarable() {
    // The refusal set and the optimizer's set have to line up, or a metric the
    // planner can rank would be one no program can ask for — or two names would
    // mean one ranking, which a reader would never guess.
    use x3_lang_ast::ast::ObjectiveMetric;
    use x3_lang_compiler::optimizer::Objective;

    let mut mapped: Vec<Objective> = Vec::new();
    for metric in ObjectiveMetric::ALL {
        match criterion_for(metric) {
            Ok(objective) => {
                assert!(
                    !mapped.contains(&objective),
                    "'{}' and an earlier metric both mean {}; two names for one ranking",
                    metric.name(),
                    objective.as_str()
                );
                mapped.push(objective);
            }
            Err(reason) => assert!(
                !reason.is_empty(),
                "'{}' is refused, so the refusal has to say why",
                metric.name()
            ),
        }
    }
    for objective in Objective::ALL {
        assert!(
            mapped.contains(objective),
            "the optimizer ranks {}, and no declared metric asks for it",
            objective.as_str()
        );
    }
}

fn graph_from(source: &str) -> OpportunityGraph {
    OpportunityGraph::from_program(&parse(source))
}

fn found(outcome: SearchOutcome) -> Vec<x3_lang_compiler::opportunity::Opportunity> {
    match outcome {
        SearchOutcome::Found(found) => found,
        SearchOutcome::BudgetExhausted { examined, budget } => {
            panic!("budget exhausted ({examined} of {budget}) on a three-venue graph")
        }
    }
}

#[test]
fn a_fee_ceiling_removes_a_route_whose_total_exceeds_it() {
    // The fee bound is a sum over the whole path, so it cannot be checked one
    // venue at a time. Both routes here charge 6 and 9 bps; a bound of 8 must
    // remove the second rather than rank it lower.
    let graph = graph_from(VENUES);
    let constraints = OpportunityConstraints {
        max_hops: 4,
        max_fee_bps: Some(8),
        ..Default::default()
    };
    let found = found(search(&graph, "ethereum.USDC", "solana.SOL", &constraints));
    assert_eq!(found.len(), 1, "only the 6 bps route is within the bound: {found:?}");
    assert_eq!(found[0].venues, vec!["wide_pool", "x3_bridge"]);

    let refused = path_reject_reason(
        &["deep_pool".to_string(), "x3_bridge".to_string()],
        &[
            "ethereum.USDC".to_string(),
            "ethereum.ETH".to_string(),
            "solana.SOL".to_string(),
        ],
        &graph,
        &constraints,
    );
    assert_eq!(refused, Some(RejectionReason::FeeAboveBound));
}

#[test]
fn a_chain_ceiling_counts_chains_rather_than_assets() {
    // Two pools on ethereum move USDC → ETH → WBTC; that is three assets and one
    // chain. A ceiling of one chain has to leave the route alone, which it only
    // does if chains are counted and not assets.
    let source = format!(
        "{VENUES}{}",
        r#"
venue eth_hops {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.ETH
    asset_out ethereum.WBTC
    fee_bps 2
    liquidity 800_000
    slippage_bps 6
    latency_ms 10
    finality_blocks 12
    risk 3
    proof source_lock_proof
}
"#
    );
    let graph = graph_from(&source);
    let path = vec!["deep_pool".to_string(), "eth_hops".to_string()];
    let assets = vec![
        "ethereum.USDC".to_string(),
        "ethereum.ETH".to_string(),
        "ethereum.WBTC".to_string(),
    ];
    let one_chain = OpportunityConstraints {
        max_hops: 4,
        max_chains: Some(1),
        ..Default::default()
    };
    assert_eq!(
        path_reject_reason(&path, &assets, &graph, &one_chain),
        None,
        "three assets on one chain are one chain's worth of exposure"
    );

    let two_chains_worth = vec![
        "ethereum.USDC".to_string(),
        "ethereum.ETH".to_string(),
        "solana.SOL".to_string(),
    ];
    let through_the_bridge = vec!["deep_pool".to_string(), "x3_bridge".to_string()];
    assert_eq!(
        path_reject_reason(&through_the_bridge, &two_chains_worth, &graph, &one_chain),
        Some(RejectionReason::TooManyChains)
    );
    let two = OpportunityConstraints {
        max_chains: Some(2),
        ..one_chain.clone()
    };
    assert_eq!(
        path_reject_reason(&through_the_bridge, &two_chains_worth, &graph, &two),
        None
    );

    // And the search applies the same ceiling.
    let empty = found(search(&graph, "ethereum.USDC", "solana.SOL", &one_chain));
    assert!(empty.is_empty(), "no route to SOL stays on one chain: {empty:?}");
}

#[test]
fn a_declared_objective_reaches_the_search_through_the_cli_path() {
    // The whole chain the command line walks: parse, verify, turn the
    // declaration into constraints, search. A declaration that verified but did
    // not reach the search would be a check that is not on the path.
    let source = program(
        "    minimize fees\n    constraints {\n        hops <= 3\n        chains <= 2\n        \
         risk <= strategy.policy\n        capital <= 25_000 USDC\n    }",
    );
    let program = parse(&source);
    let mut acc = ErrorAccumulator::new();
    verify_objective_decls(&program, &mut acc);
    assert!(!acc.has_errors(), "{:?}", acc.errors());

    let declaration = x3_lang_compiler::objective::declaration_of(&program).expect("one objective");
    let constraints = constraints_for(&declaration.constraints, &program);
    let graph = OpportunityGraph::from_program(&program);
    let found = found(search(&graph, "ethereum.USDC", "solana.SOL", &constraints));
    assert_eq!(
        found.len(),
        1,
        "the profile's 8 bps fee ceiling leaves one route: {found:?}"
    );
    assert_eq!(found[0].venues, vec!["wide_pool", "x3_bridge"]);
}

#[test]
fn formatting_an_objective_keeps_the_metric_it_ranks() {
    use x3_lang_compiler::formatter::X3Formatter;

    // No strategy module here on purpose: `x3c fmt` still rewrites a strategy's
    // `execute` block into text that does not parse (TICKET-039), and this test
    // is about the objective block, not about that.
    let source = format!(
        "{}{}",
        objective(
            "    minimize slippage\n    constraints {\n        hops <= 3\n        chains <= 2\n        \
             execution_time <= 2_000 ms\n        fees <= 100 bps\n        slippage <= 100 bps\n        \
             finality <= 64 blocks\n        capital <= 25_000 USDC\n        atomic\n    }"
        ),
        VENUES
    );
    let formatted = X3Formatter::new().format_program(&parse(&source));
    assert!(
        formatted.contains("minimize slippage"),
        "the direction is part of the metric, so formatting must not turn one into the other:\n{formatted}"
    );
    let reparsed = parse(&formatted);
    let declaration = x3_lang_compiler::objective::declaration_of(&reparsed).expect("one objective");
    assert_eq!(declaration.metric, x3_lang_ast::ast::ObjectiveMetric::MinimizeSlippage);
    assert_eq!(
        X3Formatter::new().format_program(&reparsed),
        formatted,
        "formatting is idempotent"
    );
}

#[test]
fn a_hop_ceiling_is_reported_as_its_own_reason() {
    // The search prunes at the hop bound, so it never produces an over-long
    // path — but a caller holding one (a diagnostic, a replay) has to be able
    // to ask why it is not an opportunity, and "too many hops" is an answer
    // that has to exist rather than be inferred from a missing variant.
    let graph = graph_from(VENUES);
    let assets = vec![
        "ethereum.USDC".to_string(),
        "ethereum.ETH".to_string(),
        "solana.SOL".to_string(),
    ];
    let path = vec!["wide_pool".to_string(), "x3_bridge".to_string()];
    let one_hop = OpportunityConstraints {
        max_hops: 1,
        ..Default::default()
    };
    assert_eq!(
        path_reject_reason(&path, &assets, &graph, &one_hop),
        Some(RejectionReason::TooManyHops)
    );
    let two_hops = OpportunityConstraints {
        max_hops: 2,
        ..Default::default()
    };
    assert_eq!(path_reject_reason(&path, &assets, &graph, &two_hops), None);

    // Zero is "nothing declared", which the search reads as its own default —
    // and the explanation has to read it the same way, or a default search
    // would be explaining paths against a ceiling of none.
    let undeclared = OpportunityConstraints::default();
    assert_eq!(path_reject_reason(&path, &assets, &graph, &undeclared), None);
    let too_long = vec![
        "ethereum.USDC".to_string(),
        "ethereum.ETH".to_string(),
        "ethereum.WBTC".to_string(),
        "solana.SOL".to_string(),
        "solana.USDC".to_string(),
        "solana.ETH".to_string(),
    ];
    assert_eq!(
        path_reject_reason(&path, &too_long, &graph, &undeclared),
        Some(RejectionReason::TooManyHops)
    );
}
