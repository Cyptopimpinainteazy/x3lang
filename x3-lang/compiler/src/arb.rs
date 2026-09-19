//! The arbitrage scope and its risk policy — spec PHASE 37.
//!
//! ```text
//! arb spread {
//!     discover  { chains = [x3, ethereum, solana]; max_hops = 4; liquidity_min = 500_000 USDC; }
//!     capital   { flash = disabled; max = 50_000_000 USDC; }
//!     execution { atomic = true; parallel = true; private = false; }
//!     risk      { min_profit = 20bps; max_slippage = 8bps; max_total_fee = 6bps; deadline = 220ms; }
//! }
//! ```
//!
//! An `arb` declares *what may be searched* and *what the trade must satisfy*. It
//! does not execute anything, and this module is explicit about why: the phase's
//! own lowering pipeline has seven stages, six of which already exist in this
//! compiler for other constructs. Naming which module implements which — and
//! which stage has no implementation at all — is the difference between wiring
//! this phase onto the existing machinery and building a second one. See
//! [`STAGES`] and [`missing_stages`].
//!
//! ## What is decided
//!
//! Everything above is checked against the declaration's own numbers:
//!
//! - **the scope is a scope**: at least one chain, no chain named twice, a hop
//!   bound of at least one and at most the route search's own bound, and a
//!   liquidity floor whose asset lives on a chain the scope can reach. An asset
//!   outside the declared chains is refused rather than silently widening the
//!   search, and an empty chain list is refused because a search over nothing is
//!   not a search.
//! - **the capital is bounded**: `max` is required and non-zero. A strategy with
//!   no capital ceiling has no ceiling, which is the same argument PHASE 41 makes
//!   for resource caps.
//! - **the risk bounds are bounds**: a profit floor of zero is not a floor, the
//!   three bps figures must not add up to more than the whole trade, and the
//!   deadline must be a duration the block-time reader can convert.
//! - **the declared floor is enforced by a guard, or it is a label.** This is the
//!   check that matters most, and it is the one PHASE 15's own comment names as a
//!   defect: a value nothing acts on is not a policy. A floor with no `require
//!   profit` guard, a guard that permits less profit than the declaration demands,
//!   or a guard whose bound cannot be read as basis points at all, are each
//!   refused with the figures.
//!
//! ## What is refused because it would be a false claim
//!
//! - **`capital { flash = enabled }`.** Spec PHASE 20 says flash collateral must
//!   not ship before a formal safety proof, and this repository holds to that. A
//!   declaration that says `enabled` would be a claim the runtime cannot honour,
//!   so the answer is no with the phase's own reason rather than a silent ignore.
//! - **`execution { private = true }`.** No private submission path exists in this
//!   compiler or this VM. The artifact would be public, so a declaration that says
//!   otherwise is false; it is refused rather than recorded.
//! - **`execution { atomic = false }`.** An arbitrage whose legs may settle
//!   separately is not one trade, it is a set of positions — and a half-settled
//!   cross-domain arbitrage is the failure the atomicity exists to prevent.
//!
//! ## The declaration is judged against the graph, not only against itself
//!
//! The checks above are arithmetic. They are necessary and not sufficient: an
//! `arb` whose `chains` name nothing, or whose `liquidity_min` no declared venue
//! can absorb, or whose fee and slippage ceilings every declared venue exceeds, is
//! a perfectly consistent declaration that can never find anything. So every
//! declared venue is judged against the declaration's own bounds
//! ([`venue_standings`]) using the same numbers the opportunity graph carries, and
//! a declaration no venue survives is refused with **every venue and the bound
//! that removed it**.
//!
//! The weakest claim the graph can support is that one declared venue survives the
//! declared bounds as a one-hop candidate. Refusing below that is what makes this
//! check worth having: a scope with no candidates returns nothing at run time, and
//! a compiler that reported it as an empty search would be reporting a typo as a
//! result.
//!
//! ## The execution plan, and the number the compiler does not have
//!
//! [`plan`] turns a decided scope into the operations that would run: the asset cycle
//! the search found, the venues it may use, and the floors as runtime guards.
//!
//! It reaches "Execution Plan" and "Atomic Settlement" — the two stages this module
//! used to report as missing — **without prices**, and that is worth stating plainly
//! because a profitable cycle cannot be computed from this graph. The graph holds venue
//! *attributes* (fee, slippage, liquidity, latency, finality, risk) and no prices, which
//! is why the language already refuses `maximize profit` for an objective rather than
//! guessing. So the generator does not pick the most profitable cycle; it cannot. It
//! picks the cycle with the lowest **declared** fee among those the declaration's own
//! bounds admit, which is the strongest ranking computable from what a program
//! declares, and it names that criterion `lowest_declared_fee` rather than borrowing
//! `highest_net_output` for a ranking that never looked at an output.
//!
//! The profit floor is therefore a **runtime** guard, not a compile-time choice:
//! `require profit >= <n>bps` travels in the artifact and the runtime refuses to settle
//! below it. That is the honest division of labour — the compiler bounds the trade, the
//! runtime measures it — and it is why the plan carries guards rather than a projected
//! profit.
//!
//! The amounts follow the same rule. [`MultiHopSwap`](crate::ir::Operation::MultiHopSwap)
//! takes the *input* amount, which the declaration states, and leaves the per-hop
//! outputs to the host: no intermediate amount is invented, because every intermediate
//! amount depends on a price the compiler does not have.
//!
//! ## On the second implementation of this phase
//!
//! A second `arb` implementation exists (`compiler/src/arbitrage.rs`, preserved
//! unrebased on `wip/x3lang-preserve-packets-and-arbitrage-20260919`). It is not
//! merged, and this module is the surviving surface (TICKET-076). Its
//! graph-grounded validation is carried over here as [`venue_standings`], because
//! judging the clauses against `opportunity.rs` rather than beside it is the better
//! design. Its treatment of `capital { flash = enabled }` is **not** carried over:
//! it allows flash when a declared flash venue covers the ceiling, and spec PHASE 20
//! says flash collateral must not ship before a formal safety proof, so allowing it
//! would be permitting a claim the phase forbids.

use std::collections::BTreeSet;

use x3_lang_ast::ast::{ArbDecl, Expression, Item, Program, RequireKind, VenueDecl};
use x3_lang_common::{Bps, ErrorAccumulator, Span, X3Error};

use crate::lowering;
use crate::semantic;

/// The pipeline PHASE 37 names, and where each stage lives today.
///
/// A stage whose second element is `None` has no implementation, which is what
/// [`missing_stages`] reports and what the IR verifier and the emitter refuse the
/// operation over. The list is the phase's own, in its own order: reading it
/// against the code is the whole point, so do not reorder or merge entries.
pub const STAGES: [(&str, Option<&str>); 7] = [
    ("Opportunity Graph", Some("compiler/src/opportunity.rs (PHASE 14)")),
    ("Candidate Routes", Some("compiler/src/optimizer.rs (PHASE 15)")),
    ("Filter", Some("compiler/src/objective.rs (PHASE 15)")),
    (
        "Dependency DAG",
        Some("compiler/src/dag.rs and Operation::ParallelPlan (PHASE 16)"),
    ),
    (
        "Risk Verification",
        Some("compiler/src/profitability.rs and the semantic guard checks (PHASE 36)"),
    ),
    (
        "Execution Plan",
        Some("compiler/src/arb.rs (the generator below) and Operation::MultiHopSwap"),
    ),
    (
        "Atomic Settlement",
        Some("compiler/src/lowering.rs (the atomic block each plan is wrapped in) and Operation::Require"),
    ),
];

/// The stages of the phase's pipeline that no module implements.
pub fn missing_stages() -> Vec<&'static str> {
    STAGES
        .iter()
        .filter(|(_, implementation)| implementation.is_none())
        .map(|(stage, _)| *stage)
        .collect()
}

/// Whether the declared profit floor is actually enforced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Enforcement {
    /// A guard demands at least the declared floor, in basis points.
    Guarded { owner: String, bound_bps: u32 },
    /// A guard mentions profit but permits less than the declaration demands.
    Contradicted { owner: String, bound_bps: u32 },
    /// A guard mentions profit, and its bound is not a basis-point figure, so
    /// whether it enforces the floor cannot be decided from it.
    Unverifiable { owner: String, bound: String },
    /// No guard mentions profit at all.
    Unguarded,
}

/// An arbitrage scope and the bounds its trade must satisfy.
///
/// There is deliberately no `enforcement` field here. Whether the declared profit
/// floor is enforced is a property of the *program* — it depends on guards that
/// live in other declarations — so it is answered by [`enforcement`], which is
/// handed the program, rather than by a field this type would have to leave
/// wrong until somebody filled it in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArbPolicy {
    pub name: String,
    /// The chains the search may look at, in the order written.
    pub chains: Vec<String>,
    pub max_hops: u32,
    /// `(amount, "chain.ASSET")` for the liquidity floor.
    pub liquidity_min: (u128, String),
    /// `(amount, "chain.ASSET")` for the capital ceiling.
    pub capital_max: (u128, String),
    pub parallel: bool,
    pub min_profit_bps: u16,
    pub max_slippage_bps: u16,
    pub max_total_fee_bps: u16,
    pub deadline_blocks: u32,
}

/// Decide an `arb` declaration from its own numbers.
pub fn policy(decl: &ArbDecl) -> Result<ArbPolicy, String> {
    let name = decl.name.as_str();

    // --- discover ---------------------------------------------------------
    if decl.discover.chains.is_empty() {
        return Err(format!(
            "the arb '{name}' declares `chains = []`: a discovery scope with no chain in it has \
             nothing to search, so the routing stages would be asked for a route over an empty graph"
        ));
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for chain in &decl.discover.chains {
        if !seen.insert(chain.as_str()) {
            return Err(format!(
                "the arb '{name}' names the chain '{}' twice in `chains`; a chain listed twice makes \
                 the scope's size depend on how it was written",
                chain.as_str()
            ));
        }
    }
    let chains: Vec<String> = decl
        .discover
        .chains
        .iter()
        .map(|chain| chain.as_str().to_string())
        .collect();

    if decl.discover.max_hops == 0 {
        return Err(format!(
            "the arb '{name}' declares `max_hops = 0`: a search that may take no hop cannot leave \
             the asset it starts from"
        ));
    }
    if decl.discover.max_hops > crate::semantic::DEFAULT_MAX_ROUTE_HOPS {
        return Err(format!(
            "the arb '{name}' declares `max_hops = {}` and the route search's bound is {}; a longer \
             path is not one this compiler will search, so the declaration would promise more than \
             the search delivers",
            decl.discover.max_hops,
            crate::semantic::DEFAULT_MAX_ROUTE_HOPS
        ));
    }

    let Some((liquidity_min, liquidity_asset)) = &decl.discover.liquidity_min else {
        return Err(format!(
            "the arb '{name}' declares no `liquidity_min`: without a floor the search will route \
             through pools that cannot fill the trade, and the phase's own `discover` block states \
             one"
        ));
    };
    if *liquidity_min == 0 {
        return Err(format!(
            "the arb '{name}' declares `liquidity_min = 0`: a floor of zero admits every pool, \
             including empty ones"
        ));
    }
    let liquidity_key = asset_key(liquidity_asset);
    require_chain_in_scope(name, &chains, liquidity_asset.chain.as_str(), "liquidity_min")?;

    // --- capital ----------------------------------------------------------
    let Some((capital_max, capital_asset)) = &decl.capital.max else {
        return Err(format!(
            "the arb '{name}' declares no `capital.max`: a strategy with no capital ceiling has no \
             ceiling, and the ceiling is what bounds the loss a failed routing can commit"
        ));
    };
    if *capital_max == 0 {
        return Err(format!(
            "the arb '{name}' declares `max = 0`: a ceiling of zero permits no trade, which is a \
             disabled strategy written as a bound"
        ));
    }
    let capital_key = asset_key(capital_asset);
    require_chain_in_scope(name, &chains, capital_asset.chain.as_str(), "max")?;

    if decl.capital.flash {
        return Err(format!(
            "the arb '{name}' declares `flash = enabled`, and flash collateral must not ship before \
             a formal safety proof (spec PHASE 20); this compiler will not lower a claim the runtime \
             cannot honour, so the declaration is refused rather than ignored"
        ));
    }

    // --- execution --------------------------------------------------------
    if !decl.execution.atomic {
        return Err(format!(
            "the arb '{name}' declares `atomic = false`: an arbitrage whose legs may settle \
             separately is a set of positions rather than one trade, and a half-settled \
             cross-domain arbitrage is the state atomicity exists to prevent"
        ));
    }
    if decl.execution.private {
        return Err(format!(
            "the arb '{name}' declares `private = true`, and there is no private submission path in \
             this compiler or this VM: the artifact, its legs and its state reads would all be \
             public, so the declaration states something that would not be true"
        ));
    }

    // --- risk -------------------------------------------------------------
    let Some(min_profit_bps) = decl.risk.min_profit_bps else {
        return Err(format!(
            "the arb '{name}' declares no `risk.min_profit`: a risk block with no profit floor is a \
             strategy with no floor, and the floor is the guard the whole trade is decided against"
        ));
    };
    if min_profit_bps == 0 {
        return Err(format!(
            "the arb '{name}' declares `min_profit = 0bps`: a floor of zero accepts every trade, \
             which is the same as stating no floor at all"
        ));
    }
    let Some(max_slippage_bps) = decl.risk.max_slippage_bps else {
        return Err(format!(
            "the arb '{name}' declares no `risk.max_slippage`: without a slippage ceiling the trade \
             can be filled at any price and still be called profitable"
        ));
    };
    let Some(max_total_fee_bps) = decl.risk.max_total_fee_bps else {
        return Err(format!(
            "the arb '{name}' declares no `risk.max_total_fee`: without a fee ceiling the trade can \
             pay out its whole margin in fees and still satisfy the profit floor as stated"
        ));
    };

    // The three figures are all in the same unit, so they are added rather than
    // converted: a trade that has to gain `min_profit`, may lose `max_slippage`
    // and may pay `max_total_fee` has a margin of the sum, and a margin at or
    // above the whole trade is not a bound. The unit is named (`Bps::WHOLE`)
    // rather than spelled as a literal, because the same ten thousand is the
    // ceiling in half a dozen other files (PHASE 43).
    let margin = Bps::from_raw(u32::from(min_profit_bps) + u32::from(max_slippage_bps) + u32::from(max_total_fee_bps));
    if margin.raw() >= Bps::WHOLE.raw() {
        return Err(format!(
            "the arb '{name}' declares min_profit {min_profit_bps}bps, max_slippage \
             {max_slippage_bps}bps and max_total_fee {max_total_fee_bps}bps, which is {}bps — the \
             whole trade — so the three bounds together permit the trade to keep nothing and still \
             be called profitable",
            margin.raw()
        ));
    }

    let Some(deadline) = &decl.risk.deadline else {
        return Err(format!(
            "the arb '{name}' declares no `risk.deadline`: a cross-domain trade with no deadline \
             holds its capital until it settles, and the phase's own example states one"
        ));
    };
    let deadline_blocks = lowering::expression_to_blocks(deadline).map_err(|error| {
        format!("the arb '{name}' declares a deadline this compiler cannot read as a duration: {error}")
    })?;
    if deadline_blocks == 0 {
        return Err(format!(
            "the arb '{name}' declares a deadline of zero blocks: the trade would be out of time \
             before it could be submitted"
        ));
    }

    Ok(ArbPolicy {
        name: name.to_string(),
        chains,
        max_hops: decl.discover.max_hops,
        liquidity_min: (*liquidity_min, liquidity_key),
        capital_max: (*capital_max, capital_key),
        parallel: decl.execution.parallel,
        min_profit_bps,
        max_slippage_bps,
        max_total_fee_bps,
        deadline_blocks,
    })
}

/// Decide which guard, if any, enforces a declared profit floor.
///
/// A floor with nothing enforcing it is a label, so the two answers that are not a
/// pass — no guard, or a guard that cannot be read in the declared unit — are both
/// refusals. Accepting them would make `min_profit = 20bps` a comment.
pub fn enforcement(program: &Program, floor_bps: u16) -> Enforcement {
    let mut unreadable: Option<Enforcement> = None;
    for (owner, guard) in semantic::require_guards(program) {
        if guard.kind != RequireKind::Profit {
            continue;
        }
        // A `require profit <= N` is an upper bound on profit, which is a claim
        // about profit that is not a floor. It cannot enforce this declaration.
        if !guard.comparison.is_some_and(|op| op.is_lower_bound()) {
            continue;
        }
        let Some(bound_bps) = guard.value.as_ref().and_then(semantic::bound_bps_from_expr) else {
            if unreadable.is_none() {
                unreadable = Some(Enforcement::Unverifiable {
                    owner: owner.to_string(),
                    bound: guard
                        .value
                        .as_ref()
                        .map(render_expression)
                        .unwrap_or_else(|| "nothing".to_string()),
                });
            }
            continue;
        };
        if bound_bps < u32::from(floor_bps) {
            return Enforcement::Contradicted {
                owner: owner.to_string(),
                bound_bps,
            };
        }
        return Enforcement::Guarded {
            owner: owner.to_string(),
            bound_bps,
        };
    }
    unreadable.unwrap_or(Enforcement::Unguarded)
}

/// Where one declared venue stands against an `arb` declaration's own bounds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VenueStanding {
    /// The venue satisfies every bound the declaration states, so it is a
    /// one-hop candidate for the search.
    Survives,
    /// The venue cannot lie on any path the declaration describes, and this is
    /// the bound that removed it.
    Removed(String),
}

/// Judge every declared venue against the declaration's own bounds.
///
/// The numbers are the ones the opportunity graph carries (`fee_bps`, `liquidity`,
/// `slippage_bps`, and the chain the venue settles on), so a venue this says
/// survives cannot be one the search would refuse for the same bound. The reasons
/// are worded here rather than in the graph because the graph's filters answer a
/// different question — "may this edge be used at all" — and this one is "does the
/// scope the author wrote leave anything to rank".
pub fn venue_standings(program: &Program, policy: &ArbPolicy) -> Vec<(String, VenueStanding)> {
    let mut standings = Vec::new();
    for item in &program.items {
        let Item::VenueDecl(venue) = &item.node else {
            continue;
        };
        let name = venue.name.as_str().to_string();

        // A venue is in the scope when the chain it settles on, or a chain either
        // of its assets lives on, is declared. A cross-domain venue touches more
        // than one chain and the author should not have to list all of them for
        // the venue to be visible.
        if !venue_in_scope(policy, venue) {
            standings.push((
                name,
                VenueStanding::Removed(format!(
                    "it settles on '{}' and its assets live on '{}' and '{}', and none of those is \
                     in `chains` ({})",
                    venue.chain.as_str(),
                    venue.asset_in.chain.as_str(),
                    venue.asset_out.chain.as_str(),
                    policy.chains.join(", ")
                )),
            ));
            continue;
        }

        let (floor, floor_asset) = &policy.liquidity_min;
        if venue.liquidity < *floor {
            standings.push((
                name,
                VenueStanding::Removed(format!(
                    "it declares {} of liquidity and `liquidity_min` is {floor} {floor_asset}",
                    venue.liquidity
                )),
            ));
            continue;
        }
        if venue.fee_bps > u32::from(policy.max_total_fee_bps) {
            standings.push((
                name,
                VenueStanding::Removed(format!(
                    "it charges {}bps and `max_total_fee` is {}bps, so one hop already spends the \
                     whole fee ceiling",
                    venue.fee_bps, policy.max_total_fee_bps
                )),
            ));
            continue;
        }
        if venue.slippage_bps > u32::from(policy.max_slippage_bps) {
            standings.push((
                name,
                VenueStanding::Removed(format!(
                    "it declares {}bps of slippage and `max_slippage` is {}bps",
                    venue.slippage_bps, policy.max_slippage_bps
                )),
            ));
            continue;
        }
        standings.push((name, VenueStanding::Survives));
    }
    standings
}

/// Check every `arb` declaration in a program, in the same layer as the hedge,
/// liquidation, rebalance and netting checks — before anything is lowered.
pub fn verify(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::Arb(decl) = &item.node else {
            continue;
        };
        let decided = match policy(decl) {
            Err(reason) => {
                acc.add_error(semantic_error(reason));
                continue;
            }
            Ok(decided) => decided,
        };
        {
            let decided = &decided;
            match enforcement(program, decided.min_profit_bps) {
                Enforcement::Guarded { .. } => {}
                Enforcement::Contradicted { owner, bound_bps } => acc.add_error(semantic_error(format!(
                    "the arb '{}' declares `min_profit = {}bps` and the guard in '{owner}' permits \
                     {bound_bps}bps; the guard allows what the declaration forbids",
                    decided.name, decided.min_profit_bps
                ))),
                Enforcement::Unverifiable { owner, bound } => acc.add_error(semantic_error(format!(
                    "the arb '{}' declares `min_profit = {}bps` and the guard in '{owner}' bounds \
                     profit at '{bound}', which is not a basis-point figure, so whether it enforces \
                     the declared floor cannot be decided; write the guard's bound in bps",
                    decided.name, decided.min_profit_bps
                ))),
                Enforcement::Unguarded => acc.add_error(semantic_error(format!(
                    "the arb '{}' declares `min_profit = {}bps` and no `require profit >= …` guard \
                     enforces it, so the floor is a label nothing acts on; a declared bound with \
                     nothing enforcing it is the defect PHASE 15's own comment names",
                    decided.name, decided.min_profit_bps
                ))),
            }
        }
        if let Err(reason) = graph_grounding(program, &decided) {
            acc.add_error(semantic_error(reason));
        }
    }
}

/// Whether the declared scope leaves anything for the search to rank.
///
/// The declaration is consistent on its own terms and can still describe a search
/// with no candidates: `chains` naming nowhere a venue lives, a floor deeper than
/// every pool, ceilings every venue exceeds. The weakest claim the graph can support
/// is that one declared venue survives the declared bounds as a one-hop candidate,
/// and below that the refusal names every venue and the bound that removed it, so
/// the author can see which line to change.
pub fn graph_grounding(program: &Program, policy: &ArbPolicy) -> Result<usize, String> {
    let standings = venue_standings(program, policy);
    if standings.is_empty() {
        return Err(format!(
            "the arb '{}' declares a discovery scope and the program declares no `venue`, so there \
             is no graph to search: a scope describes which of a program's venues may be used, and \
             with none declared every path is empty by construction",
            policy.name
        ));
    }
    let surviving = standings
        .iter()
        .filter(|(_, standing)| matches!(standing, VenueStanding::Survives))
        .count();
    if surviving > 0 {
        return Ok(surviving);
    }
    let mut reasons: Vec<String> = standings
        .iter()
        .map(|(name, standing)| match standing {
            VenueStanding::Survives => format!("'{name}' survives"),
            VenueStanding::Removed(reason) => format!("'{name}' — {reason}"),
        })
        .collect();
    reasons.sort();
    Err(format!(
        "the arb '{}' declares bounds no declared venue survives, so every search under it returns \
         nothing: {}",
        policy.name,
        reasons.join("; ")
    ))
}

fn require_chain_in_scope(arb: &str, chains: &[String], chain: &str, field: &str) -> Result<(), String> {
    if chains.iter().any(|declared| declared == chain) {
        return Ok(());
    }
    // A bare asset (`500_000 USDC`) arrives with no chain on it. The scope check
    // needs to know which ledger's liquidity is meant, and the registry that would
    // answer "does every chain in scope carry USDC?" does not exist — so the
    // author is asked for the one word that makes the question answerable rather
    // than the check guessing on their behalf.
    if chain == "unknown" {
        return Err(format!(
            "the arb '{arb}' declares `{field}` as an asset with no chain on it; write \
             `<chain>.<ASSET>` so the scope check can tell which ledger's liquidity is meant \
             (the scope here is {})",
            chains.join(", ")
        ));
    }
    Err(format!(
        "the arb '{arb}' declares `{field}` on the chain '{chain}', which is not in `chains` \
         ({}) — the search may only look where the scope says it may, so an asset outside the \
         scope is refused rather than silently widening it",
        chains.join(", ")
    ))
}

fn asset_key(asset: &x3_lang_ast::ast::AssetRef) -> String {
    format!("{}.{}", asset.chain.as_str(), asset.name.as_str())
}

fn render_expression(expression: &Expression) -> String {
    match expression {
        Expression::Literal(literal) => match literal {
            x3_lang_ast::ast::LiteralExpr::Int { value, .. } => value.to_string(),
            x3_lang_ast::ast::LiteralExpr::Float { raw, .. } => raw.as_str().to_string(),
            x3_lang_ast::ast::LiteralExpr::Percentage { value } => value.as_str().to_string(),
            other => format!("{other:?}"),
        },
        other => format!("{other:?}"),
    }
}

fn semantic_error(message: impl Into<String>) -> X3Error {
    X3Error::SemanticError {
        message: message.into(),
        span: Span::DUMMY,
    }
}

/// How many search nodes the cycle hunt may expand.
///
/// Bounded for the same reason the route search is bounded: an unbounded cycle hunt on
/// a graph a program controls is not an analysis, it is a hang. Exceeding it is a
/// refusal that names the bound rather than a truncated answer presented as a complete
/// one.
pub const MAX_ARB_EXPANSIONS: usize = 4096;

/// One cycle the search found: the venues that serve it and the assets it passes
/// through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cycle {
    /// Venues in hop order.
    pub venues: Vec<String>,
    /// `chain.ASSET` along the way, starting and ending at the same asset.
    pub assets: Vec<String>,
    /// The sum of the venues' declared fees, in basis points.
    pub declared_fee_bps: u32,
}

impl Cycle {
    /// The asset the cycle starts and ends in.
    pub fn base_asset(&self) -> &str {
        self.assets.first().map(String::as_str).unwrap_or("")
    }

    /// Hops taken: one less than the assets passed through.
    pub fn hops(&self) -> usize {
        self.assets.len().saturating_sub(1)
    }
}

/// A floor the plan carries into the artifact as a runtime guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Guard {
    pub kind: x3_lang_ast::ast::RequireKind,
    pub comparison: x3_lang_ast::ast::ComparisonOp,
    pub bps: u16,
}

/// The execution plan an `arb` declaration lowers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArbPlan {
    pub name: String,
    /// The winning cycle.
    pub cycle: Cycle,
    /// How many cycles the search found and ranked. One means no choice was made, and
    /// the artifact says so by carrying no choice operation.
    pub candidates: usize,
    /// The criterion the winner was ranked by.
    pub criterion: x3_lang_ast::ast::ChoiceCriterion,
    /// The capital committed: its amount and `chain.ASSET`.
    pub capital: (u128, String),
    /// The floors the runtime enforces.
    pub guards: Vec<Guard>,
}

/// Turn a decided `arb` declaration into the operations that would run.
///
/// The search is the graph's own: every edge is admitted by
/// [`opportunity::reject_reason`](crate::opportunity::reject_reason) and every candidate
/// by [`path_reject_reason`](crate::opportunity::path_reject_reason), so a cycle this
/// returns cannot be one the route search would refuse for the same bound. The venue
/// fee sum is `opportunity::path_fee_bps`, for the same reason: one place adds fees.
pub fn plan(program: &Program, decl: &ArbDecl) -> Result<ArbPlan, String> {
    let policy = policy(decl)?;
    graph_grounding(program, &policy)?;

    let graph = crate::opportunity::OpportunityGraph::from_program(program);
    let constraints = crate::opportunity::OpportunityConstraints {
        max_hops: policy.max_hops as usize,
        max_chains: None,
        max_fee_bps: Some(u32::from(policy.max_total_fee_bps)),
        max_slippage_bps: Some(u32::from(policy.max_slippage_bps)),
        min_liquidity: Some(policy.liquidity_min.0),
        max_latency_ms: None,
        max_finality_blocks: None,
        max_risk: None,
        require_proof: false,
    };

    // The trade commits capital in one asset and returns to it, so the cycle starts
    // where the capital is.
    let base = policy.capital_max.1.clone();
    let mut candidates = enumerate_cycles(program, &graph, &policy, &constraints, &base)?;
    if candidates.is_empty() {
        return Err(no_cycle_reason(&graph, &policy, &constraints, &base));
    }

    // Rank: lowest declared fee, then fewest hops, then the venue names themselves, so
    // the winner is a function of the graph rather than of the search's order.
    candidates.sort_by(|left, right| {
        left.declared_fee_bps
            .cmp(&right.declared_fee_bps)
            .then(left.hops().cmp(&right.hops()))
            .then(left.venues.cmp(&right.venues))
    });
    let found = candidates.len();
    if found as u32 > crate::semantic::MAX_ATOMIC_CHOICE_PATHS {
        return Err(format!(
            "the arb '{}' found {found} candidate cycles and the artifact's choice operation carries \
             at most {}; a plan that recorded more would be refused by the verifier, so the scope is \
             refused here with the count rather than emitting a plan that cannot be built",
            policy.name,
            crate::semantic::MAX_ATOMIC_CHOICE_PATHS
        ));
    }
    let winner = candidates.swap_remove(0);

    Ok(ArbPlan {
        name: policy.name.clone(),
        cycle: winner,
        candidates: found,
        criterion: x3_lang_ast::ast::ChoiceCriterion::LowestDeclaredFee,
        capital: policy.capital_max.clone(),
        guards: vec![
            Guard {
                kind: x3_lang_ast::ast::RequireKind::Profit,
                comparison: x3_lang_ast::ast::ComparisonOp::GreaterOrEqual,
                bps: policy.min_profit_bps,
            },
            Guard {
                kind: x3_lang_ast::ast::RequireKind::Slippage,
                comparison: x3_lang_ast::ast::ComparisonOp::LessOrEqual,
                bps: policy.max_slippage_bps,
            },
        ],
    })
}

/// Every cycle the declaration's bounds admit, from `base` back to `base`.
fn enumerate_cycles(
    program: &Program,
    graph: &crate::opportunity::OpportunityGraph,
    policy: &ArbPolicy,
    constraints: &crate::opportunity::OpportunityConstraints,
    base: &str,
) -> Result<Vec<Cycle>, String> {
    // The admitted venues, by name, from the same scope rule `venue_standings` applies,
    // so a venue the standings called out of scope cannot be one the search walks
    // through.
    let admitted: std::collections::BTreeSet<String> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::VenueDecl(venue) => Some(venue),
            _ => None,
        })
        .filter(|venue| venue_in_scope(policy, venue))
        .map(|venue| venue.name.as_str().to_string())
        .collect();

    let mut found: Vec<Cycle> = Vec::new();
    let mut expansions = 0usize;
    let mut venues: Vec<String> = Vec::new();
    let mut assets: Vec<String> = vec![base.to_string()];
    walk_cycles(
        graph,
        policy,
        constraints,
        &admitted,
        base,
        &mut venues,
        &mut assets,
        &mut found,
        &mut expansions,
    )?;
    Ok(found)
}

#[allow(clippy::too_many_arguments)]
fn walk_cycles(
    graph: &crate::opportunity::OpportunityGraph,
    policy: &ArbPolicy,
    constraints: &crate::opportunity::OpportunityConstraints,
    admitted: &std::collections::BTreeSet<String>,
    base: &str,
    venues: &mut Vec<String>,
    assets: &mut Vec<String>,
    found: &mut Vec<Cycle>,
    expansions: &mut usize,
) -> Result<(), String> {
    if venues.len() >= policy.max_hops as usize {
        return Ok(());
    }
    let Some(current) = assets.last().cloned() else {
        return Ok(());
    };

    for edge in graph.edges_from(&current) {
        *expansions += 1;
        if *expansions > MAX_ARB_EXPANSIONS {
            return Err(format!(
                "the cycle search from '{base}' expanded more than {MAX_ARB_EXPANSIONS} nodes without finishing; the scope's venues admit too many paths to rank, so the declaration is refused rather than answered from a truncated search"
            ));
        }
        // The scope says where the search may look — by venue, because the edge carries
        // the venue's domain rather than its chain — and the graph's own rejection
        // decides whether the hop is admissible at all.
        if !admitted.contains(&edge.venue) {
            continue;
        }
        if crate::opportunity::reject_reason(edge, constraints).is_some() {
            continue;
        }
        let target = edge.to.clone();
        // Repeating an asset would make the "cycle" a path with a loop in it, and the hop
        // count would no longer describe the route.
        if assets.contains(&target) && target != base {
            continue;
        }

        venues.push(edge.venue.clone());
        assets.push(target.clone());

        if target == base && venues.len() >= 2 {
            // The whole-path check, so a cycle is judged by the same function that judges
            // any other route.
            if crate::opportunity::path_reject_reason(venues, assets, graph, constraints).is_none() {
                found.push(Cycle {
                    declared_fee_bps: crate::opportunity::path_fee_bps(venues, graph),
                    venues: venues.clone(),
                    assets: assets.clone(),
                });
            }
        } else {
            walk_cycles(
                graph,
                policy,
                constraints,
                admitted,
                base,
                venues,
                assets,
                found,
                expansions,
            )?;
        }

        venues.pop();
        assets.pop();
    }
    Ok(())
}

/// Whether a venue is inside the declared scope.
///
/// The rule is stated once and used twice — by `venue_standings` when it judges each
/// declared venue, and by the cycle search when it filters an edge.
fn venue_in_scope(policy: &ArbPolicy, venue: &VenueDecl) -> bool {
    [
        venue.chain.as_str(),
        venue.asset_in.chain.as_str(),
        venue.asset_out.chain.as_str(),
    ]
    .iter()
    .any(|chain| policy.chains.iter().any(|declared| declared == chain))
}

/// Why the search found nothing, naming what it looked at.
fn no_cycle_reason(
    graph: &crate::opportunity::OpportunityGraph,
    policy: &ArbPolicy,
    constraints: &crate::opportunity::OpportunityConstraints,
    base: &str,
) -> String {
    let leaving: Vec<String> = graph
        .edges_from(base)
        .into_iter()
        .map(|edge| {
            let why = crate::opportunity::reject_reason(edge, constraints)
                .map(|reason| format!("{reason:?}"))
                .unwrap_or_else(|| "admitted".to_string());
            format!("{} ({} -> {}, {why})", edge.venue, edge.from, edge.to)
        })
        .collect();

    if leaving.is_empty() {
        return format!(
            "the arb '{}' commits {} and no venue in the declared scope takes it as an input, so no cycle can start: the scope is {}",
            policy.name,
            base,
            policy.chains.join(", ")
        );
    }
    format!(
        "the arb '{}' found no cycle from '{base}' back to '{base}' in at most {} hop(s) that satisfies its own bounds (min_profit {}bps, max_slippage {}bps, max_total_fee {}bps, liquidity_min {}). The venues leaving '{base}' are: {}",
        policy.name,
        policy.max_hops,
        policy.min_profit_bps,
        policy.max_slippage_bps,
        policy.max_total_fee_bps,
        policy.liquidity_min.0,
        leaving.join("; ")
    )
}
