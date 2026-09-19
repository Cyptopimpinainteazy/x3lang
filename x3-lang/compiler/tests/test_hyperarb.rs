//! Hyperarb — spec build-order item 38, PHASE 38.
//!
//! Every clause of the phase's own example either maps onto machinery that exists
//! or is refused with a reason, and the tests are those two halves. The property
//! that matters most is that a leg is *resolved*: `evaluate(EVM_PATH)` in the
//! phase's sketch names something, and a target naming nothing the program declares
//! is a route to nowhere rather than a leg.

use x3_lang_compiler::hyperarb;

/// Two venues on two different domains, so `settle_across_domains` has something
/// to be true about.
const VENUES: &str = "venue uniswap_v3 {\n    kind pool\n    chain ethereum\n    domain evm\n    \
                      asset_in ethereum.USDC\n    asset_out solana.USDC\n    fee_bps 5\n    \
                      liquidity 1_000_000\n    slippage_bps 8\n    latency_ms 12\n    \
                      finality_blocks 12\n    risk 2\n}\n\
                      venue x3_pool {\n    kind pool\n    chain x3\n    domain x3vm\n    \
                      asset_in x3.USDC\n    asset_out x3.ETH\n    fee_bps 3\n    liquidity \
                      2_000_000\n    slippage_bps 4\n    latency_ms 5\n    finality_blocks \
                      1\n    risk 1\n}\n";

/// The hedge PHASE 9 provides, so `hedge volatility` has a bound to point at.
const HEDGE: &str = "atomic_hedge {\n    buy 1_000 ethereum.ETH spot;\n    short equivalent ethereum.ETH perp;\n    \
     require delta <= 0.01%;\n}\n";

const HYPERARB: &str = "hyperarb triangular {\n    capital = 25_000_000 ethereum.USDC;\n    \
                        parallel {\n        route_a = evaluate(uniswap_v3);\n        route_b = \
                        evaluate(x3_pool);\n        route_c = evaluate(ethereum);\n    }\n    \
                        choose highest_net_output;\n    hedge volatility;\n    \
                        settle_across_domains;\n    require net_profit >= 35bps;\n}\n";

/// A whole program: the venues, the hedge and the declaration.
fn program(hyperarb_source: &str) -> String {
    format!("{VENUES}{HEDGE}{hyperarb_source}")
}

fn parse(source: &str) -> x3_lang_ast::ast::Program {
    x3_lang_compiler::parser::parse_source(source).expect("a hyperarb must parse")
}

/// The refusal `hyperarb::verify` reports, as one string.
fn refusal(source: &str) -> String {
    let parsed = parse(source);
    let mut acc = x3_lang_common::ErrorAccumulator::new();
    hyperarb::verify(&parsed, &mut acc);
    assert!(acc.has_errors(), "the declaration must be refused: {source}");
    acc.errors()
        .iter()
        .map(|error| format!("{error}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The decided plan, when the declaration is sound.
fn decided(source: &str) -> hyperarb::HyperarbPlan {
    let parsed = parse(source);
    let mut acc = x3_lang_common::ErrorAccumulator::new();
    hyperarb::verify(&parsed, &mut acc);
    assert!(!acc.has_errors(), "the declaration must be sound: {:?}", acc.errors());
    let decl = parsed
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::Hyperarb(decl) => Some(decl),
            _ => None,
        })
        .expect("the program declares a hyperarb");
    hyperarb::analyse(&parsed, decl).expect("a sound declaration decides")
}

/// Replace one clause of the sound declaration.
fn clause(from: &str, to: &str) -> String {
    assert!(HYPERARB.contains(from), "the clause '{from}' must be in the fixture");
    program(&HYPERARB.replacen(from, to, 1))
}

#[test]
fn a_sound_hyperarb_resolves_every_leg_and_records_the_domains() {
    let plan = decided(&program(HYPERARB));
    assert_eq!(plan.name, "triangular");
    assert_eq!(plan.capital, (25_000_000, "ethereum.USDC".to_string()));
    assert_eq!(
        plan.legs,
        vec![
            ("route_a".to_string(), "the venue 'uniswap_v3'".to_string()),
            ("route_b".to_string(), "the venue 'x3_pool'".to_string()),
            ("route_c".to_string(), "the chain 'ethereum'".to_string()),
        ],
        "each leg says what it resolved to, so a reader is not left to re-resolve it"
    );
    assert_eq!(plan.choose, "highest_net_output");
    assert!(plan.hedge_volatility && plan.settle_across_domains);
    assert_eq!(plan.net_profit_bps, 35);
    assert_eq!(
        plan.domains,
        vec!["evm".to_string(), "x3vm".to_string()],
        "the legs cover two domains, which is what `settle_across_domains` claims"
    );
}

#[test]
fn flash_capital_is_refused_with_the_phase_that_forbids_it_and_the_amount() {
    let source = clause(
        "capital = 25_000_000 ethereum.USDC;",
        "capital = flash(25_000_000 ethereum.USDC);",
    );
    let message = refusal(&source);
    assert!(
        message.contains("flash(25000000 ethereum.USDC)"),
        "the refusal must quote what the author wrote: {message}"
    );
    assert!(
        message.contains("PHASE 20") && message.contains("formal safety proof"),
        "the refusal must cite the phase that forbids it: {message}"
    );
}

#[test]
fn zero_capital_is_refused() {
    let message = refusal(&clause(
        "capital = 25_000_000 ethereum.USDC;",
        "capital = 0 ethereum.USDC;",
    ));
    assert!(
        message.contains("commits zero capital"),
        "a trade with no capital has no position to hedge: {message}"
    );
}

#[test]
fn a_single_leg_is_not_a_choice() {
    let source = clause(
        "        route_b = evaluate(x3_pool);\n        route_c = evaluate(ethereum);\n",
        "",
    );
    let message = refusal(&source);
    assert!(
        message.contains("declares 1 leg(s)") && message.contains("nothing to choose between"),
        "the refusal must say why one leg is not a choice: {message}"
    );
}

#[test]
fn a_leg_named_twice_is_refused() {
    let source = clause("route_b = evaluate(x3_pool);", "route_a = evaluate(x3_pool);");
    let message = refusal(&source);
    assert!(
        message.contains("names the leg 'route_a' twice"),
        "two legs with one name cannot be told apart in the ranking: {message}"
    );
}

#[test]
fn a_leg_that_names_nothing_the_program_declares_is_refused_with_what_it_does_declare() {
    // The phase's sketch writes `evaluate(EVM_PATH)`. In this language a leg names
    // a venue, a chain or a domain the program declares, so an invented name is a
    // route to nowhere — and the refusal lists the real ones, because a typo is the
    // likely cause.
    let source = clause("evaluate(uniswap_v3)", "evaluate(EVM_PATH)");
    let message = refusal(&source);
    assert!(
        message.contains("`evaluate(EVM_PATH)` names nothing the program declares"),
        "the refusal must name the target: {message}"
    );
    assert!(
        message.contains("uniswap_v3") && message.contains("x3_pool") && message.contains("evm"),
        "the refusal must list the venues, chains and domains it does declare: {message}"
    );
}

#[test]
fn a_leg_may_name_a_venue_a_chain_or_a_domain() {
    let by_venue = decided(&clause("evaluate(ethereum)", "evaluate(uniswap_v3)"));
    assert!(
        by_venue
            .legs
            .iter()
            .any(|(_, target)| target == "the venue 'uniswap_v3'"),
        "a venue name resolves: {:?}",
        by_venue.legs
    );
    let by_chain = decided(&clause("evaluate(x3_pool)", "evaluate(x3)"));
    assert!(
        by_chain.legs.iter().any(|(_, target)| target == "the chain 'x3'"),
        "a chain a venue settles on resolves: {:?}",
        by_chain.legs
    );
    let by_domain = decided(&clause("evaluate(uniswap_v3)", "evaluate(evm)"));
    assert!(
        by_domain.legs.iter().any(|(_, target)| target == "the domain 'evm'"),
        "a domain resolves: {:?}",
        by_domain.legs
    );
}

#[test]
fn hedge_volatility_with_no_hedge_in_the_program_is_refused() {
    let message = refusal(&format!("{VENUES}{HYPERARB}"));
    assert!(
        message.contains("declares no `atomic_hedge`") && message.contains("no bound to net to"),
        "a hedge with no bound is an intention, not a constraint: {message}"
    );
}

#[test]
fn hedge_volatility_pointing_at_a_declared_hedge_is_accepted() {
    // The hedge exists and PHASE 9 decides its exposure. Its *execution* is refused
    // by the IR layer (a perp leg needs a venue adapter, TICKET-068), which is a
    // different layer's answer and not this one's.
    let plan = decided(&program(HYPERARB));
    assert!(plan.hedge_volatility);
}

#[test]
fn settle_across_domains_over_one_domain_is_refused() {
    // Both the surviving legs are on `evm`, so the claim is empty.
    let source = clause(
        "        route_a = evaluate(uniswap_v3);\n        route_b = evaluate(x3_pool);\n",
        "        route_a = evaluate(uniswap_v3);\n        route_b = evaluate(ethereum);\n",
    );
    let message = refusal(&source);
    assert!(
        message.contains("says `settle_across_domains` and its legs resolve onto 1 domain(s)"),
        "the refusal must give the domain count: {message}"
    );
    assert!(
        message.contains("over one it is empty"),
        "the refusal must say why the claim is empty: {message}"
    );
}

#[test]
fn a_settlement_claim_is_accepted_when_the_legs_really_cross_domains() {
    let plan = decided(&program(HYPERARB));
    assert_eq!(plan.domains.len(), 2);
    assert!(plan.settle_across_domains);
}

#[test]
fn a_net_profit_floor_of_zero_is_refused() {
    let message = refusal(&clause("require net_profit >= 35bps;", "require net_profit >= 0bps;"));
    assert!(
        message.contains("`require net_profit >= 0bps`"),
        "a floor of zero is not a floor: {message}"
    );
}

#[test]
fn a_program_with_no_venue_has_no_path_for_a_leg_to_name() {
    let message = refusal(&format!("{HEDGE}{HYPERARB}"));
    assert!(
        message.contains("program declares no `venue`") && message.contains("no path for a leg"),
        "the refusal must say the graph is empty: {message}"
    );
}

#[test]
fn an_unknown_choice_criterion_is_refused_with_the_ones_that_exist() {
    let error = x3_lang_compiler::parser::parse_source(&clause("choose highest_net_output;", "choose most_profit;"))
        .expect_err("an invented criterion must not parse");
    let message = format!("{error}");
    assert!(
        message.contains("unknown choice criterion 'most_profit'"),
        "the refusal must name the word: {message}"
    );
    assert!(
        message.contains("highest_net_output") && message.contains("fewest_hops"),
        "the refusal must list the criteria that exist: {message}"
    );
}

#[test]
fn a_repeated_or_unknown_clause_is_refused_rather_than_picked_between() {
    let repeated = clause(
        "require net_profit >= 35bps;",
        "require net_profit >= 35bps;\n    require net_profit >= 20bps;",
    );
    let error = x3_lang_compiler::parser::parse_source(&repeated).expect_err("two floors");
    assert!(
        format!("{error}").contains("declares `require net_profit` twice"),
        "the parser must refuse two answers to one clause: {error}"
    );

    let unknown = clause("hedge volatility;", "hedge the downside;");
    let error = x3_lang_compiler::parser::parse_source(&unknown).expect_err("unknown hedge");
    let message = format!("{error}");
    assert!(
        message.contains("can `hedge volatility`") && message.contains("found `hedge the`"),
        "the parser must say what it read and what it accepts: {message}"
    );
}

#[test]
fn the_formatter_round_trips_a_hyperarb() {
    let source = program(HYPERARB);
    let parsed = parse(&source);
    let formatted = x3_lang_compiler::formatter::X3Formatter::new().format_program(&parsed);
    assert!(
        formatted.contains("hyperarb triangular {")
            && formatted.contains("route_a = evaluate(uniswap_v3);")
            && formatted.contains("choose highest_net_output;")
            && formatted.contains("hedge volatility;")
            && formatted.contains("settle_across_domains;")
            && formatted.contains("require net_profit >= 35bps;"),
        "every clause must survive a reformat: {formatted}"
    );
    let again = parse(&formatted);
    let round_tripped = x3_lang_compiler::formatter::X3Formatter::new().format_program(&again);
    assert_eq!(formatted, round_tripped, "formatting must be idempotent");
    assert_eq!(
        decided(&formatted),
        decided(&source),
        "a reformat must not change what the declaration decided"
    );
}
