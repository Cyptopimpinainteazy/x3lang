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

/// The venues the scope searches.
///
/// Declared rather than left out, because a scope whose own bounds admit no venue is
/// refused (`arb::admitted_venues`): the phase's pipeline starts at the opportunity
/// graph, and a scope over a graph with nothing in it is a declaration that says no
/// opportunity exists while looking like a strategy. Both venues sit inside the sound
/// scope's bounds — ethereum and solana are in `chains`, and each declares enough depth,
/// little enough slippage and a low enough fee to survive it.
const VENUES: &str = r#"venue arb_spot {
    kind pool
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.ETH
    fee_bps 5
    liquidity 1_000_000
    slippage_bps 6
    latency_ms 120
    finality_blocks 12
    risk 2
}

venue solana_pool {
    kind pool
    chain solana
    domain svm
    asset_in solana.SOL
    asset_out solana.USDC
    fee_bps 6
    liquidity 900_000
    slippage_bps 8
    latency_ms 200
    finality_blocks 32
    risk 3
}

"#;

/// The `arb` block, with each of its four parts supplied by the caller.
fn arb_source(discover: &str, capital: &str, execution: &str, risk: &str) -> String {
    format!(
        "{VENUES}intent spread_trade {{\n    \
             from ethereum.USDC amount 1_000_000 receiver 0xA1\n    \
             to solana.USDC receiver 0xA2\n    \
             route {{\n        \
                 swap uniswap ethereum.USDC -> solana.USDC amount 1_000_000 min_output 1_001_000\n    \
             }}\n    \
             require profit >= 20\n    \
             require slippage <= 50\n    \
             timeout 30s refund ethereum.USDC to sender\n    \
             on_fail rollback\n\
         }}\n\n\
         arb spread {{\n    \
             discover {{ {discover} }}\n    \
             capital {{ {capital} }}\n    \
             execution {{ {execution} }}\n    \
             risk {{ {risk} }}\n\
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
fn the_scope_admits_exactly_the_venues_its_own_bounds_allow() {
    let program = parse(&sound());
    let policy = decided(&sound());
    let (admitted, refused) = arb::admitted_venues(&program, &policy);
    assert_eq!(admitted, vec!["arb_spot", "solana_pool"]);
    assert!(refused.is_empty(), "both venues survive: {refused:?}");

    // The fee ceiling is a property of a *path*, not of one venue, so it is the second
    // function the search uses that refuses over it: `reject_reason` cannot see a fee
    // bound at all, and a check that looked at one edge at a time would admit a pool the
    // scope's own ceiling excludes.
    let tight = clause("risk", "max_total_fee = 6bps;", "max_total_fee = 5bps;");
    let program = parse(&tight);
    let policy = decided(&tight);
    let (admitted, refused) = arb::admitted_venues(&program, &policy);
    assert_eq!(admitted, vec!["arb_spot"], "arb_spot charges exactly 5bps");
    assert_eq!(
        refused,
        vec![("solana_pool".to_string(), "FeeAboveBound".to_string())],
        "solana_pool charges 6bps"
    );
}

#[test]
fn a_scope_whose_bounds_admit_no_venue_is_refused_with_the_venues_and_the_bound() {
    // One basis point of slippage: neither pool declares that little, so the scope has
    // nothing to rank and the search would report an empty plan as a strategy.
    let reason = refusal(&clause("risk", "max_slippage = 8bps;", "max_slippage = 1bps;"));
    assert!(reason.contains("admit no venue to rank"), "{reason}");
    assert!(
        reason.contains("arb_spot") && reason.contains("solana_pool") && reason.contains("SlippageAboveBound"),
        "every venue and the bound that removed it are named: {reason}"
    );
    assert!(
        reason.contains("a slippage ceiling of 1bps"),
        "the figures are quoted: {reason}"
    );
}

#[test]
fn a_scope_over_chains_with_no_venue_is_refused_rather_than_searching_an_empty_graph() {
    // The scope moves to x3 alone, and the floor and ceiling with it; no venue is
    // declared on x3, so the scope's own chain *set* removes every candidate. The graph
    // has venues — just not there — which is why this is not the empty-graph case below.
    let moved = sound()
        .replace("chains = [x3, ethereum, solana]", "chains = [x3]")
        .replace("500_000 ethereum.USDC", "500_000 x3.USDC")
        .replace("50_000_000 ethereum.USDC", "50_000_000 x3.USDC");
    let reason = refusal(&moved);
    assert!(reason.contains("admit no venue to rank"), "{reason}");
    assert!(reason.contains("over chains [x3]"), "the scope is quoted: {reason}");
    assert!(
        reason.contains("ChainNotAllowed"),
        "the set, not a count of chains, is what refused them: {reason}"
    );

    // A program with no venue at all says that, rather than reporting an empty search:
    // the opportunity graph is where the phase's own pipeline starts.
    let venue_less = sound().replace(VENUES, "");
    let reason = refusal(&venue_less);
    assert!(reason.contains("no `venue` is declared"), "{reason}");
    assert!(reason.contains("opportunity graph is empty"), "{reason}");
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
