//! Arbitrage plan contract — spec PHASE 37.
//!
//! `arb { discover { … } capital { … } execution { … } risk { … } }` is the
//! declaration-side spelling of the constraints an opportunity search runs under, and
//! this module is the one place that decides whether what was written *is* one.
//!
//! ## The phase's lowering chain, and which link is missing
//!
//! PHASE 37 asks for `Opportunity Graph → Candidate Routes → Filter → Dependency DAG
//! → Risk Verification → Execution Plan → Atomic Settlement`. The graph, the search,
//! the ranking, the dependency DAG and the risk score all exist as real, tested
//! artefacts (`opportunity.rs`, `optimizer.rs`, `dag.rs`, `risk.rs`). What does not
//! exist is the *generator* that takes an `arb` declaration and drives them in that
//! order, so the declaration is decided and no artifact is emitted for it
//! (TICKET-071).
//!
//! Deciding it *against* the real machinery rather than beside it is the point: the
//! clauses lower to [`OpportunityConstraints`], and the checks below call the same
//! functions the search calls — `reject_reason` for one venue, `path_reject_reason`
//! for a whole path — so a declaration this module accepts cannot be one the search
//! would refuse, and the reason a venue was refused is worded once, in
//! `opportunity.rs`.
//!
//! ## What is decided
//!
//! - **Every section and every bound is stated.** A plan built on a bound nobody
//!   wrote is a plan nobody constrained, so an absent `discover`, `capital`,
//!   `execution` or `risk`, or an absent bound inside one, is refused. Nothing is
//!   defaulted: the difference between "the author set no ceiling" and "the compiler
//!   chose one" is the difference between a refusal and an invented bound.
//! - **The declared chains are chains the graph hosts**, and the set becomes the
//!   search's own `allowed_chains` filter — not a count of chains, which is a
//!   different statement.
//! - **The declared bounds leave something to rank.** At least one declared venue
//!   has to survive them as a one-hop candidate, which is the weakest claim the graph
//!   can support; when none does, the refusal names every venue and the bound that
//!   removed it.
//! - **Flash capital is a ceiling the graph can supply.** `flash = enabled` needs a
//!   declared flash venue on the declared chains whose depth covers the ceiling in
//!   the ceiling's own asset. `flash = disabled` refuses a `max` instead of recording
//!   one, because this language declares no balance to check a ceiling on the
//!   program's own funds against.
//! - **Execution is atomic or it is not a plan.** This VM has no non-atomic
//!   settlement path, so `atomic = false` is refused rather than carried; `private =
//!   true` needs the privacy declaration that would hide the route, or the flag is a
//!   promise nothing backs.

use x3_lang_ast::ast::{ArbDecl, AssetRef, Item, Program, VenueDecl, VenueKind};
use x3_lang_common::{ErrorAccumulator, Span, X3Error};

use crate::opportunity::{path_reject_reason, reject_reason, GraphNode, OpportunityConstraints, OpportunityGraph};

/// The decided `arb` block: the constraint set its clauses lowered to, the chain set
/// those constraints allow, and the venues they admit.
#[derive(Debug, Clone)]
pub struct ArbContract {
    /// The constraints the opportunity search runs under, built from the clauses.
    pub constraints: OpportunityConstraints,
    /// The declared chains, lowercased: chain names are compared case-insensitively
    /// everywhere else in this compiler, and a contract that compared them
    /// byte-for-byte would refuse `Ethereum` for `ethereum`.
    pub chains: Vec<String>,
    pub max_hops: u32,
    /// The depth floor a venue must declare, with the asset as written.
    pub liquidity_min: (String, u128),
    /// The depth the search actually required: the tighter of [`Self::liquidity_min`] and
    /// the flash ceiling, because both are floors and the larger one is the bound. This
    /// is the figure the constraints carry and the artifact states.
    pub committed_depth: (String, u128),
    /// `capital.max`, when flash is enabled: the ceiling the plan may borrow, with the
    /// asset as written.
    pub flash_max: Option<(String, u128)>,
    pub parallel: bool,
    pub private: bool,
    /// The declared profit floor, in basis points. Carried rather than checked: a
    /// profit needs amounts and prices, which the graph does not model.
    pub min_profit_bps: u32,
    pub max_slippage_bps: u32,
    pub max_total_fee_bps: u32,
    pub deadline_ms: u32,
    /// Venues the declared bounds admit, in declaration order. Never empty: a block
    /// whose bounds admit nothing is refused, so a contract that reaches the artifact
    /// always names somewhere to search.
    pub admitted: Vec<String>,
}

/// Decide an `arb` declaration against the program's own venue declarations.
pub fn contract(program: &Program, decl: &ArbDecl) -> Result<ArbContract, String> {
    let discover = decl.discover.as_ref().ok_or_else(|| {
        "the `arb` block states no `discover` section; a plan needs the universe it \
         searches, and an absent section is not an empty one \
         (`discover { chains = [..]; max_hops = <n>; liquidity_min = <n> <ASSET>; }`)"
            .to_string()
    })?;
    let capital = decl.capital.as_ref().ok_or_else(|| {
        "the `arb` block states no `capital` section; the size a plan may commit is a bound \
         like any other, and nothing else in the language states it \
         (`capital { flash = enabled|disabled; max = <n> <ASSET>; }`)"
            .to_string()
    })?;
    let execution = decl.execution.as_ref().ok_or_else(|| {
        "the `arb` block states no `execution` section; whether the plan is atomic, parallel \
         and private is what the settlement path is chosen for \
         (`execution { atomic = true; parallel = true; private = true; }`)"
            .to_string()
    })?;
    let risk = decl.risk.as_ref().ok_or_else(|| {
        "the `arb` block states no `risk` section; a plan with no profit floor and no bounds is \
         not a plan anyone agreed to \
         (`risk { min_profit = <n>bps; max_slippage = <n>bps; max_total_fee = <n>bps; deadline = \
         <n>ms; }`)"
            .to_string()
    })?;

    // ---- discover ---------------------------------------------------------
    if discover.chains.is_empty() {
        return Err(
            "`discover` names no chains; discovery over nothing cannot produce a candidate, and \
             an empty list is not a default — the chains to search have to be written"
                .to_string(),
        );
    }
    let mut chains: Vec<String> = Vec::new();
    for chain in &discover.chains {
        let name = chain.as_str().to_ascii_lowercase();
        if chains.contains(&name) {
            return Err(format!(
                "`discover` names the chain '{name}' twice; discovery over a chain listed twice is \
                 the same discovery, and a duplicate hides which set was meant"
            ));
        }
        chains.push(name);
    }
    let max_hops = discover.max_hops.ok_or_else(|| {
        "`discover` states no `max_hops`; an unbounded search is not a bound anyone can price, and \
         this compiler will not pick a hop ceiling on the author's behalf"
            .to_string()
    })?;
    if max_hops == 0 {
        return Err(
            "`discover` states `max_hops = 0`; a route with no hops is not a route, and a bound of \
             zero can only ever be refused at run time"
                .to_string(),
        );
    }
    let (liquidity_asset, liquidity_min) = discover.liquidity_min.as_ref().ok_or_else(|| {
        "`discover` states no `liquidity_min`; a venue that cannot absorb the size is not a route \
         for it, and the size it has to absorb is a declaration rather than a guess"
            .to_string()
    })?;
    if *liquidity_min == 0 {
        return Err(
            "`discover` states `liquidity_min = 0`; a floor of zero admits every venue, so the \
             clause would be a filter that filters nothing — write the depth that makes a venue \
             usable, or omit the section rather than state a bound of nothing"
                .to_string(),
        );
    }

    let graph = OpportunityGraph::from_program(program);
    for chain in &chains {
        if !hosted_chains(&graph)
            .iter()
            .any(|hosted| hosted.eq_ignore_ascii_case(chain))
        {
            let hosted = hosted_chains(&graph);
            return Err(format!(
                "`discover` names the chain '{chain}', which hosts no declared venue; the graph's \
                 chains are [{}]. A search over a chain with nothing on it finds nothing, so the \
                 declaration and the graph disagree about where the opportunity is",
                if hosted.is_empty() {
                    "none".to_string()
                } else {
                    hosted.join(", ")
                }
            ));
        }
    }

    // ---- capital ----------------------------------------------------------
    let flash = capital.flash.ok_or_else(|| {
        "`capital` does not say whether the plan may borrow (`flash = enabled` or `disabled`); \
         borrowed capital has to be returned inside the transaction, so leaving it unstated is \
         leaving the funding of the plan unstated"
            .to_string()
    })?;
    let flash_max = if flash {
        let (asset, ceiling) = capital.max.as_ref().ok_or_else(|| {
            "`capital` enables flash borrowing and states no `max`; a borrow with no ceiling is \
             not a bound, and this compiler will not invent one"
                .to_string()
        })?;
        if *ceiling == 0 {
            return Err(
                "`capital` states `max = 0`; a ceiling of nothing is not capital, it is a plan \
                 that cannot buy anything"
                    .to_string(),
            );
        }
        Some((asset_label(asset), *ceiling))
    } else {
        if capital.max.is_some() {
            return Err(
                "`capital` states `flash = disabled` and a `max`; the ceiling would bound the \
                 program's own funds, and this language declares no balance to compare it \
                 against — enable flash, whose capacity a venue declares, or omit `max` rather \
                 than state a ceiling nothing checks"
                    .to_string(),
            );
        }
        None
    };

    // ---- execution --------------------------------------------------------
    let atomic = execution.atomic.ok_or_else(|| {
        "`execution` does not state `atomic`; this settlement layer either commits every leg or \
         none of them, and a plan that left it unstated would be asking for the partial execution \
         the rest of this language exists to prevent"
            .to_string()
    })?;
    if !atomic {
        return Err(
            "`execution` states `atomic = false`; this VM has no non-atomic settlement path for a \
             multi-leg plan, so the declaration asks for an execution mode that does not exist \
             rather than one that is merely unchecked"
                .to_string(),
        );
    }
    let parallel = execution.parallel.unwrap_or(false);
    let private = execution.private.unwrap_or(false);
    if private && !has_privacy_declaration(program) {
        return Err(
            "`execution` states `private = true` and the program declares no `privacy` block; a \
             private plan hides its route until a commitment, and without that declaration the \
             flag is a promise nothing backs"
                .to_string(),
        );
    }

    // ---- risk -------------------------------------------------------------
    let min_profit_bps = rate(risk.min_profit_bps, "min_profit")?;
    let max_slippage_bps = rate(risk.max_slippage_bps, "max_slippage")?;
    let max_total_fee_bps = rate(risk.max_total_fee_bps, "max_total_fee")?;
    let deadline_ms = risk.deadline_ms.ok_or_else(|| {
        "`risk` states no `deadline`; a plan with no window can be settled whenever it happens to \
         finish, which is how a route that was profitable at discovery settles at a loss"
            .to_string()
    })?;
    if deadline_ms == 0 {
        return Err(
            "`risk` states `deadline = 0`; a window of nothing is not a deadline, it is a plan \
             that can never settle"
                .to_string(),
        );
    }

    // The committed size is the larger of the two floors the block declares: the
    // depth discovery asks for and the capital the plan may borrow. Both are floors,
    // so the tighter one is the bound, and a venue that cannot absorb the capital is
    // not a route for it.
    let min_liquidity = match &flash_max {
        Some((_, ceiling)) => (*liquidity_min).max(*ceiling),
        None => *liquidity_min,
    };
    let constraints = OpportunityConstraints {
        max_hops: max_hops as usize,
        // The set, not a count: `chains = [a, b]` says which chains, and the count is
        // a consequence of that statement rather than the statement itself.
        max_chains: None,
        allowed_chains: Some(chains.clone()),
        max_fee_bps: Some(max_total_fee_bps),
        max_slippage_bps: Some(max_slippage_bps),
        min_liquidity: Some(min_liquidity),
        max_latency_ms: Some(deadline_ms),
        max_finality_blocks: None,
        max_risk: None,
        require_proof: false,
    };

    // The flash ceiling is checked before the filter below, because it is the more
    // specific refusal: when the ceiling is what made the depth floor unreachable, "the
    // plan would borrow more than the graph can lend" is the sentence the author has to
    // act on, not "no venue survived the bounds".
    if let Some((asset, ceiling)) = &flash_max {
        check_flash_capacity(program, &chains, asset, *ceiling)?;
    }

    // ---- the declared bounds have to leave something to rank --------------
    let (admitted, refused) = admitted_venues(&graph, &constraints);
    if admitted.is_empty() {
        let detail = if refused.is_empty() {
            "no venue is declared at all on the declared chains".to_string()
        } else {
            refused
                .iter()
                .map(|(venue, reason)| format!("{venue} (refused: {reason})"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        return Err(format!(
            "the `arb` block's own bounds admit no venue, so the search would have nothing to rank: \
             {detail}. A filter that removes every candidate is a declaration that says no \
             opportunity exists, and discovery over it would report success with an empty plan"
        ));
    }

    Ok(ArbContract {
        constraints,
        chains,
        max_hops,
        liquidity_min: (asset_label(liquidity_asset), *liquidity_min),
        committed_depth: (asset_label(liquidity_asset), min_liquidity),
        flash_max,
        parallel,
        private,
        min_profit_bps,
        max_slippage_bps,
        max_total_fee_bps,
        deadline_ms,
        admitted,
    })
}

/// Verify every `arb` block in a program.
pub fn verify(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::Arb(decl) = &item.node else {
            continue;
        };
        if let Err(reason) = contract(program, decl) {
            acc.add_error(err(reason));
        }
    }
}

/// The venues a constraint set admits, and why the rest were refused.
///
/// A venue is admitted when it survives both functions the search itself uses: the
/// per-venue bounds (`reject_reason`) and the path bounds as they apply to the
/// shortest path through it (`path_reject_reason` on a one-hop path). The second call
/// is what gives the fee ceiling and the chain set teeth, because those are
/// properties of a path rather than of a single edge.
///
/// One hop is the weakest claim the graph can support, and the claim made here is
/// exactly that: *something* survives. A longer route that also survives is a
/// question for the search, which needs endpoints this declaration does not name.
fn admitted_venues(
    graph: &OpportunityGraph,
    constraints: &OpportunityConstraints,
) -> (Vec<String>, Vec<(String, String)>) {
    let mut admitted: Vec<String> = Vec::new();
    let mut refused: Vec<(String, String)> = Vec::new();
    for edge in &graph.edges {
        let reason = reject_reason(edge, constraints)
            .map(|reason| format!("{reason:?}"))
            .or_else(|| {
                let venues = vec![edge.venue.clone()];
                let assets = vec![edge.from.clone(), edge.to.clone()];
                path_reject_reason(&venues, &assets, graph, constraints).map(|reason| format!("{reason:?}"))
            });
        match reason {
            // The wording is the enum's `Debug`, which is the rendering `x3c optimize`
            // ships for the same refusals: one presentation rather than two that drift.
            Some(reason) => refused.push((edge.venue.clone(), reason)),
            None => admitted.push(edge.venue.clone()),
        }
    }
    (admitted, refused)
}

/// Check that a flash ceiling can actually be borrowed on the declared chains.
///
/// Only a flash venue lends flash capital, so the venue has to exist, be on a chain
/// the block discovers over, take in the asset the ceiling names, and hold at least
/// the ceiling. The figures are in the message because the author has to change one
/// of them.
fn check_flash_capacity(program: &Program, chains: &[String], asset: &str, ceiling: u128) -> Result<(), String> {
    let flash_venues: Vec<&VenueDecl> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::VenueDecl(venue) if venue.kind == VenueKind::Flash => Some(venue),
            _ => None,
        })
        .filter(|venue| {
            chains
                .iter()
                .any(|chain| chain.eq_ignore_ascii_case(venue.chain.as_str()))
        })
        .collect();
    if flash_venues.is_empty() {
        return Err(format!(
            "`capital` enables flash borrowing and no venue of kind `flash` is declared on the \
             declared chains [{}]; flash capital is a venue with a depth, not a facility the \
             compiler may assume",
            chains.join(", ")
        ));
    }
    let matching: Vec<&&VenueDecl> = flash_venues
        .iter()
        .filter(|venue| asset_matches(asset, &venue.asset_in))
        .collect();
    if matching.is_empty() {
        let declared: Vec<String> = flash_venues.iter().map(|venue| asset_label(&venue.asset_in)).collect();
        return Err(format!(
            "`capital` states a flash ceiling in '{asset}' and the declared flash venues lend \
             [{}]; a ceiling is an amount of an asset, and this compiler will not compare two \
             different ones",
            declared.join(", ")
        ));
    }
    let deepest = matching.iter().map(|venue| venue.liquidity).max().unwrap_or(0);
    if deepest < ceiling {
        let names: Vec<String> = matching
            .iter()
            .map(|venue| format!("{} ({} {})", venue.name.as_str(), venue.liquidity, asset))
            .collect();
        return Err(format!(
            "`capital` states a flash ceiling of {ceiling} {asset}, and the deepest declared flash \
             venue on the declared chains supplies {deepest}: {}. The plan would borrow more than \
             the graph can lend",
            names.join(", ")
        ));
    }
    Ok(())
}

/// A required rate: stated, above zero, and below the whole amount.
///
/// 10 000 bps is 100%, which is not a bound — the same reading `semantic.rs` applies to
/// a venue's declared fee.
fn rate(declared: Option<u32>, clause: &str) -> Result<u32, String> {
    let value = declared.ok_or_else(|| {
        format!(
            "`risk` states no `{clause}`; a bound the plan has to respect is a declaration, and \
             this compiler will not choose one on the author's behalf"
        )
    })?;
    if value == 0 {
        return Err(format!(
            "`risk` states `{clause} = 0bps`; a bound of nothing admits nothing, so the block \
             would be a plan guaranteed to fail rather than a plan with a bound"
        ));
    }
    if value >= 10_000 {
        return Err(format!(
            "`risk` states `{clause} = {value}bps`; 10 000 bps is the whole amount, so a bound at \
             or above it is not a bound"
        ));
    }
    Ok(value)
}

/// The chains the graph hosts, in declaration order, deduplicated case-insensitively.
fn hosted_chains(graph: &OpportunityGraph) -> Vec<String> {
    let mut chains: Vec<String> = Vec::new();
    for node in &graph.nodes {
        let GraphNode::Venue { chain, .. } = node else {
            continue;
        };
        if !chains.iter().any(|known| known.eq_ignore_ascii_case(chain)) {
            chains.push(chain.clone());
        }
    }
    chains
}

/// Whether a program declares a privacy block that could hide a route.
fn has_privacy_declaration(program: &Program) -> bool {
    program
        .items
        .iter()
        .any(|item| matches!(&item.node, Item::PrivacyBlock(_)))
}

/// Whether an asset written in a declaration names the asset a venue takes in.
///
/// A bare name (`USDC`, whose chain is `unknown`) matches by symbol, because a bare
/// name does not claim a domain. A chained name (`ethereum.USDC`) matches the pair,
/// because it does.
fn asset_matches(written: &str, venue_asset: &AssetRef) -> bool {
    let venue = asset_label(venue_asset);
    if written.eq_ignore_ascii_case(&venue) {
        return true;
    }
    if !written.contains('.') {
        return venue
            .rsplit('.')
            .next()
            .is_some_and(|symbol| symbol.eq_ignore_ascii_case(written));
    }
    false
}

/// `chain.ASSET`, or the bare symbol when the chain was not written.
fn asset_label(asset: &AssetRef) -> String {
    let name = asset.name.as_str();
    if asset.chain.as_str().eq_ignore_ascii_case("unknown") {
        name.to_string()
    } else {
        format!("{}.{}", asset.chain.as_str(), name)
    }
}

fn err(message: impl Into<String>) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: Span::DUMMY,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use x3_lang_ast::ast::ArbDecl as Decl;

    /// Two chains, three venues: a USDC pool on ethereum, a flash venue that lends
    /// USDC on ethereum, and a pool on base.
    const PROGRAM: &str = r#"
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
}

venue flash_lender {
    kind flash
    chain ethereum
    domain evm
    asset_in ethereum.USDC
    asset_out ethereum.USDC
    fee_bps 9
    liquidity 50_000_000
    slippage_bps 1
    latency_ms 12
    finality_blocks 12
    risk 1
}

venue base_pool {
    kind pool
    chain base
    domain evm
    asset_in base.USDC
    asset_out base.ETH
    fee_bps 5
    liquidity 2_000_000
    slippage_bps 9
    latency_ms 11
    finality_blocks 20
    risk 3
}
"#;

    const DISCOVER: &str = "    discover {\n        chains = [ethereum, base];\n        max_hops = 4;\n        liquidity_min = 500_000 USDC;\n    }";
    const CAPITAL: &str = "    capital {\n        flash = enabled;\n        max = 2_000_000 USDC;\n    }";
    const EXECUTION: &str = "    execution {\n        atomic = true;\n        parallel = true;\n    }";
    const RISK: &str = "    risk {\n        min_profit = 20bps;\n        max_slippage = 10bps;\n        max_total_fee = 40bps;\n        deadline = 220ms;\n    }";

    /// An `arb` block holding exactly the sections it is handed, so an absent section
    /// in a test is an absent section in the source rather than a string edit that
    /// might have missed.
    fn arb_block(sections: &[&str]) -> String {
        format!("arb {{\n{}\n}}\n", sections.join("\n"))
    }

    fn sound() -> String {
        arb_block(&[DISCOVER, CAPITAL, EXECUTION, RISK])
    }

    fn parsed(arb: &str) -> (Program, Decl) {
        let text = format!("{PROGRAM}\n{arb}");
        let program = crate::parser::parse_source(&text).expect("the fixture must parse");
        let decl = program
            .items
            .iter()
            .find_map(|item| match &item.node {
                Item::Arb(decl) => Some(decl.clone()),
                _ => None,
            })
            .expect("the fixture holds an arb block");
        (program, decl)
    }

    fn decided(arb: &str) -> Result<ArbContract, String> {
        let (program, decl) = parsed(arb);
        contract(&program, &decl)
    }

    #[test]
    fn a_sound_block_lowers_to_the_search_constraints_and_admits_venues() {
        let contract = decided(&sound()).expect("the fixture is a plan contract");
        assert_eq!(contract.chains, vec!["ethereum", "base"]);
        assert_eq!(contract.constraints.max_hops, 4);
        assert_eq!(
            contract.constraints.allowed_chains,
            Some(vec!["ethereum".to_string(), "base".to_string()])
        );
        assert_eq!(contract.constraints.max_slippage_bps, Some(10));
        assert_eq!(contract.constraints.max_fee_bps, Some(40));
        // The committed size is the larger of the two floors the block states: the
        // flash ceiling, 2_000_000, not the discovery floor of 500_000.
        assert_eq!(contract.constraints.min_liquidity, Some(2_000_000));
        assert_eq!(contract.liquidity_min, ("USDC".to_string(), 500_000));
        assert_eq!(contract.committed_depth, ("USDC".to_string(), 2_000_000));
        assert_eq!(contract.constraints.max_latency_ms, Some(220));
        assert_eq!(contract.flash_max, Some(("USDC".to_string(), 2_000_000)));
        assert!(contract.parallel);
        assert!(!contract.private);
        assert_eq!(contract.admitted, vec!["deep_pool", "flash_lender", "base_pool"]);
    }

    #[test]
    fn a_bound_that_removes_every_venue_is_refused_with_the_figures() {
        // One basis point of slippage leaves the flash venue, which declares exactly
        // that, and removes both pools.
        let only_flash = sound().replace("max_slippage = 10bps", "max_slippage = 1bps");
        let contract = decided(&only_flash).expect("the flash venue declares 1 bp of slippage");
        assert_eq!(contract.admitted, vec!["flash_lender"]);

        // A fee bound the flash venue breaks as well removes the last candidate.
        let nothing = only_flash.replace("max_total_fee = 40bps", "max_total_fee = 1bps");
        let reason = decided(&nothing).expect_err("no venue survives 1 bp of fee and slippage");
        assert!(reason.contains("admit no venue"), "{reason}");
        assert!(
            reason.contains("flash_lender") && reason.contains("refused"),
            "{reason}"
        );
    }

    #[test]
    fn a_chain_with_no_venue_is_refused_against_the_graph_s_chains() {
        let reason =
            decided(&sound().replace("[ethereum, base]", "[ethereum, solana]")).expect_err("solana hosts no venue");
        assert!(
            reason.contains("solana") && reason.contains("hosts no declared venue"),
            "{reason}"
        );
        assert!(
            reason.contains("ethereum, base"),
            "the graph's chains are named: {reason}"
        );
    }

    #[test]
    fn flash_capital_is_a_ceiling_the_graph_can_supply() {
        let too_much = sound().replace("max = 2_000_000 USDC", "max = 90_000_000 USDC");
        let reason = decided(&too_much).expect_err("the lender holds 50_000_000");
        assert!(reason.contains("90000000") && reason.contains("50000000"), "{reason}");
        assert!(reason.contains("lend"), "{reason}");

        let wrong_asset = sound().replace("max = 2_000_000 USDC", "max = 1_000 ethereum.WBTC");
        let reason = decided(&wrong_asset).expect_err("no flash venue lends WBTC");
        assert!(reason.contains("ethereum.WBTC") && reason.contains("USDC"), "{reason}");
    }

    #[test]
    fn a_missing_section_or_bound_is_refused_rather_than_defaulted() {
        let reason = decided(&arb_block(&[DISCOVER, CAPITAL, EXECUTION])).expect_err("no risk section");
        assert!(reason.contains("no `risk` section"), "{reason}");

        let reason = decided(&arb_block(&[CAPITAL, EXECUTION, RISK])).expect_err("no discover section");
        assert!(reason.contains("no `discover` section"), "{reason}");

        let no_floor = sound().replace("        min_profit = 20bps;\n", "");
        let reason = decided(&no_floor).expect_err("no profit floor");
        assert!(reason.contains("no `min_profit`"), "{reason}");

        let zero_floor = sound().replace("min_profit = 20bps", "min_profit = 0bps");
        let reason = decided(&zero_floor).expect_err("a floor of nothing");
        assert!(reason.contains("bound of nothing"), "{reason}");
    }

    #[test]
    fn a_non_atomic_or_unbacked_private_plan_is_refused() {
        let non_atomic = sound().replace("atomic = true", "atomic = false");
        let reason = decided(&non_atomic).expect_err("there is no non-atomic settlement path");
        assert!(reason.contains("no non-atomic settlement path"), "{reason}");

        let private = sound().replace("atomic = true;", "atomic = true;\n        private = true;");
        let reason = decided(&private).expect_err("no privacy block is declared");
        assert!(reason.contains("no `privacy` block"), "{reason}");
    }

    #[test]
    fn a_flash_ceiling_under_a_disabled_flag_is_refused_because_nothing_checks_it() {
        let disabled = sound().replace("flash = enabled", "flash = disabled");
        let reason = decided(&disabled).expect_err("a ceiling on the program's own funds");
        assert!(reason.contains("declares no balance"), "{reason}");
    }
}
