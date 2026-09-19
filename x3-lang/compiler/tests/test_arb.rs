//! The arbitrage scope and its risk policy — spec build-order item 37, PHASE 37.
//!
//! Two things are tested here and they are different in kind. The first is the
//! declaration's own arithmetic: a scope with a chain list, a hop bound, a
//! liquidity floor, a capital ceiling and three risk bounds, each refused with the
//! figures when it is not a bound. The second is the phase's *pipeline* claim —
//! that six of its seven stages already exist in this compiler and two do not —
//! which is only worth anything if the modules it names are really there, so a
//! test walks the mapping and checks the files exist.

use std::path::Path;

use x3_lang_compiler::arb::{self, Enforcement};

/// Two venues the sound scope can actually search.
///
/// The declaration is judged against the graph as well as against itself, so a
/// scope with no venues to search is refused — which means every test below needs
/// a graph for its declaration to be about.
const VENUES: &str = "\
venue uniswap_v3 {\n    \
    kind pool\n    chain ethereum\n    domain evm\n    \
    asset_in ethereum.USDC\n    asset_out solana.USDC\n    \
    fee_bps 5\n    liquidity 1_000_000\n    slippage_bps 8\n    \
    latency_ms 12\n    finality_blocks 12\n    risk 2\n\
}\n\
venue x3_pool {\n    \
    kind pool\n    chain x3\n    domain x3vm\n    \
    asset_in x3.USDC\n    asset_out x3.ETH\n    \
    fee_bps 3\n    liquidity 2_000_000\n    slippage_bps 4\n    \
    latency_ms 5\n    finality_blocks 1\n    risk 1\n\
}\n";

/// The guarded trade the scope bounds, so the declared floor has something
/// enforcing it.
const TRADE: &str = "\
intent spread_trade {\n    \
    from ethereum.USDC amount 1_000_000 receiver 0xA1\n    \
    to solana.USDC receiver 0xA2\n    \
    route {\n        \
        swap uniswap ethereum.USDC -> solana.USDC amount 1_000_000 min_output 1_001_000\n    \
    }\n    \
    require profit >= 20\n    \
    require slippage <= 50\n    \
    timeout 30s refund ethereum.USDC to sender\n    \
    on_fail rollback\n\
}\n";

/// The `arb` block, with each of its four parts supplied by the caller, over a
/// program that declares the venues it will be judged against.
fn arb_source(discover: &str, capital: &str, execution: &str, risk: &str) -> String {
    format!(
        "{TRADE}\n{VENUES}\narb spread {{\n    \
             discover {{ {discover} }}\n    \
             capital {{ {capital} }}\n    \
             execution {{ {execution} }}\n    \
             risk {{ {risk} }}\n\
         }}\n"
    )
}

/// The same scope with the venue declarations replaced, so a test can say what
/// the search has to work with.
fn program_with_venues(venues: &str) -> String {
    format!(
        "{TRADE}\n{venues}\narb spread {{\n    \
             discover {{ chains = [x3, ethereum, solana]; max_hops = 4; liquidity_min = 500_000 \
             ethereum.USDC; }}\n    \
             capital {{ flash = disabled; max = 50_000_000 ethereum.USDC; }}\n    \
             execution {{ atomic = true; parallel = true; private = false; }}\n    \
             risk {{ min_profit = 20bps; max_slippage = 8bps; max_total_fee = 6bps; deadline = \
             220ms; }}\n\
         }}\n"
    )
}

/// A declaration every part of which is a bound, so only the property under test
/// can make it fail.
fn sound() -> String {
    compose(
        "chains = [x3, ethereum, solana]; max_hops = 4; liquidity_min = 500_000 ethereum.USDC;",
        "flash = disabled; max = 50_000_000 ethereum.USDC;",
        "atomic = true; parallel = true; private = false;",
        "min_profit = 20bps; max_slippage = 8bps; max_total_fee = 6bps; deadline = 220ms;",
    )
}

fn compose(discover: &str, capital: &str, execution: &str, risk: &str) -> String {
    arb_source(discover, capital, execution, risk)
}

/// Replace one clause inside one block of the sound program.
fn clause(block: &str, from: &str, to: &str) -> String {
    let sound = sound();
    let (start, end) = match block {
        "discover" => ("discover { ", " }"),
        "capital" => ("capital { ", " }"),
        "execution" => ("execution { ", " }"),
        "risk" => ("risk { ", " }"),
        other => panic!("no block named '{other}'"),
    };
    let open = sound.find(start).expect("the block must be in the sound program") + start.len();
    let close = sound[open..].find(end).expect("the block must close") + open;
    let body = &sound[open..close];
    assert!(body.contains(from), "the clause '{from}' must be in {block}: {body}");
    let replaced = body.replacen(from, to, 1);
    format!("{}{}{}", &sound[..open], replaced, &sound[close..])
}

fn parse(source: &str) -> x3_lang_ast::ast::Program {
    x3_lang_compiler::parser::parse_source(source).expect("an arb must parse")
}

/// The refusal `arb::verify` reports, as one string.
fn refusal(source: &str) -> String {
    let program = parse(source);
    let mut acc = x3_lang_common::ErrorAccumulator::new();
    arb::verify(&program, &mut acc);
    assert!(acc.has_errors(), "the declaration must be refused: {source}");
    acc.errors()
        .iter()
        .map(|error| format!("{error}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The declaration's decided policy, when it has one.
fn decided(source: &str) -> arb::ArbPolicy {
    let program = parse(source);
    let mut acc = x3_lang_common::ErrorAccumulator::new();
    arb::verify(&program, &mut acc);
    assert!(!acc.has_errors(), "the declaration must be sound: {:?}", acc.errors());
    let item = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::Arb(decl) => Some(decl),
            _ => None,
        })
        .expect("the program declares an arb");
    arb::policy(item).expect("a sound declaration decides")
}

/// Which guard, if any, enforces `source`'s declared profit floor.
fn enforcement_of(source: &str) -> Enforcement {
    let program = parse(source);
    let policy = decided(source);
    arb::enforcement(&program, policy.min_profit_bps)
}

#[test]
fn a_sound_scope_is_decided_and_its_floor_is_enforced_by_the_guard() {
    let policy = decided(&sound());
    assert_eq!(policy.name, "spread");
    assert_eq!(policy.chains, vec!["x3", "ethereum", "solana"]);
    assert_eq!(policy.max_hops, 4);
    assert_eq!(policy.liquidity_min, (500_000, "ethereum.USDC".to_string()));
    assert_eq!(policy.capital_max, (50_000_000, "ethereum.USDC".to_string()));
    assert!(policy.parallel, "parallel execution is declared and recorded");
    assert_eq!(policy.min_profit_bps, 20);
    assert_eq!(policy.max_slippage_bps, 8);
    assert_eq!(policy.max_total_fee_bps, 6);
    assert_eq!(
        enforcement_of(&sound()),
        Enforcement::Guarded {
            owner: "spread_trade".to_string(),
            bound_bps: 20,
        },
        "`require profit >= 20` is exactly the declared floor"
    );
}

#[test]
fn the_deadline_is_read_by_the_same_reader_every_other_duration_uses() {
    // 220ms is less than a block at the language's block time, and the reader
    // rounds up rather than down: a window shorter than the program asked for is
    // the dangerous direction.
    assert_eq!(decided(&sound()).deadline_blocks, 1);
    let one_minute = clause("risk", "deadline = 220ms;", "deadline = 60s;");
    let blocks = decided(&one_minute).deadline_blocks;
    assert_eq!(
        blocks,
        (60 / x3_lang_compiler::lowering::SECONDS_PER_BLOCK) as u32,
        "60s is ten blocks at {}s/block",
        x3_lang_compiler::lowering::SECONDS_PER_BLOCK
    );
}

#[test]
fn the_pipeline_stage_map_names_modules_that_are_really_there() {
    // The phase's pipeline is the claim that this declaration can be wired onto
    // machinery that already exists. A mapping that named modules nobody wrote
    // would be the same fake as a verifier that verifies nothing.
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the compiler lives one level below the workspace root")
        .to_path_buf();
    for (stage, implementation) in arb::STAGES {
        let Some(implementation) = implementation else {
            continue;
        };
        let path = implementation
            .split_whitespace()
            .next()
            .expect("a stage names a path first");
        assert!(
            root.join(path).exists(),
            "stage '{stage}' names '{path}', which does not exist in {}",
            root.display()
        );
    }
    assert_eq!(
        arb::missing_stages(),
        vec!["Execution Plan", "Atomic Settlement"],
        "the stages with no implementation are named, and there are two of them"
    );
}

#[test]
fn an_empty_chain_list_is_refused() {
    let source = clause("discover", "chains = [x3, ethereum, solana];", "chains = [];");
    let message = refusal(&source);
    assert!(
        message.contains("`chains = []`") && message.contains("nothing to search"),
        "the refusal must say the scope is empty: {message}"
    );
}

#[test]
fn a_chain_named_twice_is_refused() {
    let source = clause("discover", "solana", "x3");
    let message = refusal(&source);
    assert!(
        message.contains("names the chain 'x3' twice"),
        "the refusal must name the repeated chain: {message}"
    );
}

#[test]
fn a_hop_bound_of_zero_is_refused() {
    let source = clause("discover", "max_hops = 4;", "max_hops = 0;");
    let message = refusal(&source);
    assert!(
        message.contains("`max_hops = 0`") && message.contains("cannot leave the asset"),
        "the refusal must say a zero-hop search goes nowhere: {message}"
    );
}

#[test]
fn a_hop_bound_beyond_the_route_searchs_own_bound_is_refused_with_both_figures() {
    let bound = x3_lang_compiler::semantic::DEFAULT_MAX_ROUTE_HOPS;
    let source = clause("discover", "max_hops = 4;", &format!("max_hops = {};", bound + 1));
    let message = refusal(&source);
    assert!(
        message.contains(&format!("`max_hops = {}`", bound + 1)) && message.contains(&bound.to_string()),
        "the refusal must give the declared bound and the search's: {message}"
    );
}

#[test]
fn a_liquidity_floor_outside_the_scope_is_refused_with_the_scope_named() {
    let source = clause(
        "discover",
        "liquidity_min = 500_000 ethereum.USDC;",
        "liquidity_min = 500_000 base.USDC;",
    );
    let message = refusal(&source);
    assert!(
        message.contains("'base'") && message.contains("x3, ethereum, solana"),
        "the refusal must name the chain and the scope: {message}"
    );
}

#[test]
fn an_asset_written_without_its_chain_is_refused_and_says_to_qualify_it() {
    // The phase's own example writes `liquidity_min = 500_000 USDC`. The scope
    // check needs to know which ledger's liquidity is meant, so the author is
    // asked for the one word that makes the question answerable.
    let source = clause(
        "discover",
        "liquidity_min = 500_000 ethereum.USDC;",
        "liquidity_min = 500_000 USDC;",
    );
    let message = refusal(&source);
    assert!(
        message.contains("no chain on it") && message.contains("<chain>.<ASSET>"),
        "the refusal must say what to write instead: {message}"
    );
}

#[test]
fn flash_capital_is_refused_with_the_reason_the_phase_gives() {
    let source = clause("capital", "flash = disabled;", "flash = enabled;");
    let message = refusal(&source);
    assert!(
        message.contains("flash = enabled") && message.contains("PHASE 20"),
        "the refusal must cite the phase that forbids it: {message}"
    );
}

#[test]
fn private_submission_is_refused_because_no_private_path_exists() {
    let source = clause("execution", "private = false;", "private = true;");
    let message = refusal(&source);
    assert!(
        message.contains("private = true") && message.contains("no private submission path"),
        "the refusal must say the claim would be false: {message}"
    );
}

#[test]
fn a_non_atomic_arbitrage_is_refused() {
    let source = clause("execution", "atomic = true;", "atomic = false;");
    let message = refusal(&source);
    assert!(
        message.contains("atomic = false") && message.contains("set of positions"),
        "the refusal must say what a non-atomic arbitrage is: {message}"
    );
}

#[test]
fn a_capital_ceiling_that_is_missing_or_zero_is_refused() {
    let missing = refusal(&clause("capital", "max = 50_000_000 ethereum.USDC;", ""));
    assert!(
        missing.contains("no `capital.max`") && missing.contains("no ceiling"),
        "an unbounded strategy must be refused: {missing}"
    );
    let zero = refusal(&clause(
        "capital",
        "max = 50_000_000 ethereum.USDC;",
        "max = 0 ethereum.USDC;",
    ));
    assert!(zero.contains("`max = 0`"), "a ceiling of zero permits no trade: {zero}");
}

#[test]
fn a_liquidity_floor_that_is_missing_or_zero_is_refused() {
    let missing = refusal(&clause("discover", "liquidity_min = 500_000 ethereum.USDC;", ""));
    assert!(
        missing.contains("no `liquidity_min`"),
        "a search with no floor must be refused: {missing}"
    );
    let zero = refusal(&clause(
        "discover",
        "liquidity_min = 500_000 ethereum.USDC;",
        "liquidity_min = 0 ethereum.USDC;",
    ));
    assert!(
        zero.contains("`liquidity_min = 0`"),
        "a floor of zero admits empty pools: {zero}"
    );
}

#[test]
fn a_profit_floor_of_zero_is_refused() {
    let source = clause("risk", "min_profit = 20bps;", "min_profit = 0bps;");
    let message = refusal(&source);
    assert!(
        message.contains("`min_profit = 0bps`") && message.contains("same as stating no floor"),
        "the refusal must say a zero floor is not a floor: {message}"
    );
}

#[test]
fn the_risk_bounds_may_not_add_up_to_the_whole_trade() {
    // 9_995 + 3 + 3 is more than the entire trade, so the three bounds together
    // permit the trade to keep nothing and still be called profitable.
    let source = compose(
        "chains = [x3, ethereum, solana]; max_hops = 4; liquidity_min = 500_000 ethereum.USDC;",
        "flash = disabled; max = 50_000_000 ethereum.USDC;",
        "atomic = true; parallel = true; private = false;",
        "min_profit = 9_995bps; max_slippage = 3bps; max_total_fee = 3bps; deadline = 220ms;",
    );
    let message = refusal(&source);
    assert!(
        message.contains("10001bps"),
        "the refusal must show the sum of the three bounds: {message}"
    );
    assert!(
        message.contains("the whole trade"),
        "the refusal must say why the sum matters: {message}"
    );
}

#[test]
fn a_slippage_or_fee_ceiling_that_is_missing_is_refused() {
    let no_slippage = refusal(&clause("risk", "max_slippage = 8bps;", ""));
    assert!(
        no_slippage.contains("no `risk.max_slippage`"),
        "a trade can be filled at any price without a ceiling: {no_slippage}"
    );
    let no_fee = refusal(&clause("risk", "max_total_fee = 6bps;", ""));
    assert!(
        no_fee.contains("no `risk.max_total_fee`"),
        "a trade can pay out its margin in fees: {no_fee}"
    );
}

#[test]
fn a_profit_floor_no_guard_enforces_is_refused_as_a_label() {
    // The declaration is well formed; what makes it a defect is that nothing in
    // the program acts on the floor.
    let unguarded = sound().replace("    require profit >= 20\n", "");
    let message = refusal(&unguarded);
    assert!(
        message.contains("no `require profit >= …` guard enforces it") && message.contains("a label nothing acts on"),
        "the refusal must say the floor is unenforced: {message}"
    );
}

#[test]
fn a_guard_that_permits_less_profit_than_the_declaration_demands_is_refused() {
    let source = sound().replace("require profit >= 20", "require profit >= 5");
    let message = refusal(&source);
    assert!(
        message.contains("permits 5bps") && message.contains("allows what the declaration forbids"),
        "the refusal must give both figures and the direction: {message}"
    );
}

#[test]
fn a_guard_whose_bound_is_not_in_basis_points_cannot_be_decided() {
    // `require profit >= 1_000 USDC` is a claim about profit, but not in the unit
    // the declaration's floor is in. Accepting it would compare two quantities the
    // program never related to each other.
    // A bare integer in a profit guard is basis points by this language's rule
    // (`semantic::bound_bps_from_expr`), so the case that cannot be decided is a
    // *fractional* bound: half a basis point is not a bound the VM can compare.
    let source = sound().replace("require profit >= 20", "require profit >= 20.5");
    let message = refusal(&source);
    assert!(
        message.contains("not a basis-point figure") && message.contains("write the guard's bound in bps"),
        "the refusal must say why the guard cannot be read and what to write: {message}"
    );
}

#[test]
fn a_guard_demanding_more_than_the_declared_floor_still_enforces_it() {
    let source = sound().replace("require profit >= 20", "require profit >= 35");
    assert_eq!(
        enforcement_of(&source),
        Enforcement::Guarded {
            owner: "spread_trade".to_string(),
            bound_bps: 35,
        },
        "a stricter guard satisfies the floor it was declared with"
    );
}

#[test]
fn a_repeated_clause_or_block_is_refused_rather_than_picked_between() {
    let repeated = clause("discover", "max_hops = 4;", "max_hops = 4; max_hops = 5;");
    let error = x3_lang_compiler::parser::parse_source(&repeated).expect_err("two answers");
    assert!(
        format!("{error}").contains("duplicate 'max_hops'"),
        "the parser must refuse two answers to one clause: {error}"
    );

    let two_blocks = sound().replace(
        "    capital { flash = disabled; max = 50_000_000 ethereum.USDC; }\n",
        "    capital { flash = disabled; max = 50_000_000 ethereum.USDC; }\n    \
         capital { flash = disabled; max = 1 ethereum.USDC; }\n",
    );
    let error = x3_lang_compiler::parser::parse_source(&two_blocks).expect_err("two capital blocks");
    assert!(
        format!("{error}").contains("duplicate `capital` block"),
        "the parser must refuse a repeated block: {error}"
    );
}

#[test]
fn a_bps_bound_without_its_unit_is_refused() {
    // `min_profit = 20` could be twenty basis points or twenty USDC, and this
    // block's clause names do not say which.
    let source = clause("risk", "min_profit = 20bps;", "min_profit = 20;");
    let error = x3_lang_compiler::parser::parse_source(&source).expect_err("an ambiguous bound");
    assert!(
        format!("{error}").contains("bound in basis points"),
        "the parser must ask for the unit: {error}"
    );
}

#[test]
fn an_unknown_block_or_clause_is_refused() {
    let unknown_block = sound().replace("    risk {", "    exposure {");
    let error = x3_lang_compiler::parser::parse_source(&unknown_block).expect_err("unknown block");
    assert!(
        format!("{error}").contains("unknown arb block 'exposure'"),
        "the parser must name the block it does not know: {error}"
    );

    let unknown_clause = clause("risk", "max_slippage = 8bps;", "max_mev = 8bps;");
    let error = x3_lang_compiler::parser::parse_source(&unknown_clause).expect_err("unknown clause");
    assert!(
        format!("{error}").contains("unknown `risk` clause 'max_mev'"),
        "the parser must name the clause it does not know: {error}"
    );
}

#[test]
fn a_missing_block_is_refused_with_its_name() {
    let start = sound().find("    capital {").expect("the block is there");
    let end = sound()[start..].find("}\n").expect("the block closes") + start + 2;
    let without_capital = format!("{}{}", &sound()[..start], &sound()[end..]);
    let error = x3_lang_compiler::parser::parse_source(&without_capital).expect_err("no capital");
    assert!(
        format!("{error}").contains("has no `capital` block"),
        "the parser must name the missing block: {error}"
    );
}

#[test]
fn an_arb_with_no_guarded_trade_at_all_is_refused_for_the_same_reason() {
    // The scope is well formed and there is no trade for it to bound, which is
    // the same defect as a floor no guard enforces.
    let only_the_arb = match sound().find("arb spread {") {
        Some(position) => sound()[position..].to_string(),
        None => panic!("the sound program declares an arb"),
    };
    let message = refusal(&only_the_arb);
    assert!(
        message.contains("no `require profit >= …` guard enforces it"),
        "a scope with nothing to bound must be refused: {message}"
    );
}

/// The declaration's decided policy without running the program-level checks, so a
/// test can ask what the standings are for a scope whose venues were all removed.
fn policy_only(source: &str) -> arb::ArbPolicy {
    let program = parse(source);
    let item = program
        .items
        .iter()
        .find_map(|item| match &item.node {
            x3_lang_ast::ast::Item::Arb(decl) => Some(decl),
            _ => None,
        })
        .expect("the program declares an arb");
    arb::policy(item).expect("the declaration decides on its own terms")
}

/// One venue declaration, so a test can place it on a chain or give it a depth.
fn venue(name: &str, chain: &str, fee_bps: u32, liquidity: u128, slippage_bps: u32) -> String {
    format!(
        "venue {name} {{\n    kind pool\n    chain {chain}\n    domain evm\n    asset_in \
         {chain}.USDC\n    asset_out {chain}.ETH\n    fee_bps {fee_bps}\n    liquidity \
         {liquidity}\n    slippage_bps {slippage_bps}\n    latency_ms 12\n    finality_blocks \
         12\n    risk 2\n}}\n"
    )
}

#[test]
fn a_program_with_no_venue_declaration_has_no_graph_to_search() {
    // The scope says which of a program's venues may be used. With none declared,
    // every path is empty by construction, and that is the refusal's reason.
    let message = refusal(&program_with_venues(""));
    assert!(
        message.contains("declares no `venue`") && message.contains("no graph to search"),
        "the refusal must say there is no graph: {message}"
    );
}

#[test]
fn a_scope_no_declared_venue_survives_is_refused_with_every_reason() {
    // Both venues charge more than the fee ceiling, so neither can be on a path
    // the declaration describes. The declaration is internally consistent and
    // describes a search that returns nothing.
    let venues = format!(
        "{}{}",
        venue("pricey_a", "ethereum", 50, 1_000_000, 4),
        venue("pricey_b", "x3", 90, 1_000_000, 4),
    );
    let message = refusal(&program_with_venues(&venues));
    assert!(
        message.contains("no declared venue survives") && message.contains("every search under it returns nothing"),
        "the refusal must say the scope is unsatisfiable: {message}"
    );
    assert!(
        message.contains("'pricey_a'") && message.contains("'pricey_b'"),
        "the refusal must name every venue it judged: {message}"
    );
    assert!(
        message.contains("charges 50bps") && message.contains("`max_total_fee` is 6bps"),
        "the refusal must give the figure and the bound that removed it: {message}"
    );
}

#[test]
fn a_venue_off_the_declared_chains_is_removed_by_the_scope() {
    // The scope is the only thing that says where the search may look, so a venue
    // on a chain nobody declared is invisible to it.
    let venues = venue("base_pool", "base", 5, 1_000_000, 4);
    let message = refusal(&program_with_venues(&venues));
    assert!(
        message.contains("'base_pool'") && message.contains("settles on 'base'"),
        "the refusal must name the venue and the chain it settles on: {message}"
    );
    assert!(
        message.contains("none of those is in `chains`"),
        "the refusal must say the scope excluded it: {message}"
    );
}

#[test]
fn a_liquidity_floor_no_venue_can_absorb_is_refused_with_both_figures() {
    let venues = venue("shallow", "ethereum", 5, 100, 4);
    let message = refusal(&program_with_venues(&venues));
    assert!(
        message.contains("declares 100 of liquidity") && message.contains("`liquidity_min` is 500000 ethereum.USDC"),
        "the refusal must give the depth and the floor: {message}"
    );
}

#[test]
fn one_surviving_venue_is_enough() {
    // The weakest claim the graph can support: one venue survives every declared
    // bound as a one-hop candidate. The other venue being removed is not a defect.
    let venues = format!(
        "{}{}",
        venue("too_pricey", "ethereum", 50, 1_000_000, 4),
        venue("fine", "ethereum", 5, 1_000_000, 8),
    );
    let source = program_with_venues(&venues);
    let program = parse(&source);
    let policy = policy_only(&source);
    assert_eq!(
        arb::graph_grounding(&program, &policy),
        Ok(1),
        "exactly one venue survives, and that is enough for the scope to be a scope"
    );
    // And the whole checker agrees, so the declaration compiles past this layer.
    let mut acc = x3_lang_common::ErrorAccumulator::new();
    arb::verify(&program, &mut acc);
    assert!(
        !acc.has_errors(),
        "a scope with one viable venue must not be refused: {:?}",
        acc.errors()
    );
}

#[test]
fn the_standings_name_the_bound_that_removed_each_venue() {
    let venues = format!(
        "{}{}{}",
        venue("off_scope", "base", 5, 1_000_000, 4),
        venue("shallow", "ethereum", 5, 10, 4),
        venue("slippy", "ethereum", 5, 1_000_000, 400),
    );
    let source = program_with_venues(&venues);
    let program = parse(&source);
    let policy = policy_only(&source);
    let standings = arb::venue_standings(&program, &policy);
    let reasons: Vec<String> = standings
        .iter()
        .map(|(name, standing)| match standing {
            arb::VenueStanding::Survives => format!("{name}: survives"),
            arb::VenueStanding::Removed(reason) => format!("{name}: {reason}"),
        })
        .collect();
    assert_eq!(standings.len(), 3, "all three were judged: {reasons:?}");
    assert!(
        reasons.iter().all(|reason| reason.contains(": ")),
        "every venue gets a reason or a pass: {reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("off_scope") && reason.contains("`chains`")),
        "the off-scope venue's reason is the scope: {reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("shallow") && reason.contains("liquidity_min")),
        "the shallow venue's reason is the floor: {reasons:?}"
    );
    assert!(
        reasons
            .iter()
            .any(|reason| reason.contains("slippy") && reason.contains("max_slippage")),
        "the slippy venue's reason is the ceiling: {reasons:?}"
    );
    assert!(
        !standings
            .iter()
            .any(|(_, standing)| matches!(standing, arb::VenueStanding::Survives)),
        "none of the three survives its own bound: {reasons:?}"
    );
}
