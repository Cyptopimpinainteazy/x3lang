//! Hyperarb — the multi-leg, multi-domain arbitrage primitive. Spec PHASE 38.
//!
//! ```text
//! hyperarb triangular {
//!     capital = 25_000_000 ethereum.USDC;
//!     parallel { route_a = evaluate(uniswap_v3); route_b = evaluate(x3_pool); }
//!     choose highest_net_output;
//!     hedge volatility;
//!     settle_across_domains;
//!     require net_profit >= 35bps;
//! }
//! ```
//!
//! "Every part must eventually map to explicit deterministic operations", so every
//! clause is mapped onto machinery that already exists, and the mapping is the
//! design rather than a comment:
//!
//! | Clause | What it maps to |
//! |---|---|
//! | `parallel { … }` | `Operation::ParallelPlan` (PHASE 16) |
//! | `choose …` | `Operation::AtomicChoice` and its `CHOICE_CRITERION_*` vocabulary |
//! | `hedge volatility` | the `atomic_hedge` of PHASE 9, which decides the exposure |
//! | `settle_across_domains` | the chain list the resolved legs cover |
//! | `require net_profit >= …` | the declaration's own floor, stated inline |
//!
//! Nothing here generates legs: the pipeline stages PHASE 37 named and
//! `compiler/src/arb.rs` mapped are still missing two of their seven, so
//! `Operation::Hyperarb` is refused by the IR verifier and by the emitter, and
//! `check` and `build` agree that a `hyperarb` cannot run yet (TICKET-073).
//!
//! ## What is refused, and why
//!
//! - **`capital = flash(…)`.** The phase's own example writes it. Spec PHASE 20 says
//!   flash collateral must not ship before a formal safety proof, so the flash form
//!   is refused with that reason and the amount it named, and the honest spelling is
//!   a committed amount. Refusing here rather than ignoring the word is what keeps
//!   the declaration's meaning equal to the declaration's text.
//! - **A leg whose target resolves to nothing.** `evaluate(<target>)` names
//!   something the program declares — a venue, a chain or a domain — and a target
//!   that names none of them is a route to nowhere. The refusal lists what the
//!   program does declare, because a typo is the likely cause.
//! - **`parallel` with fewer than two legs.** One leg is not a choice, and the
//!   criterion would have nothing to rank.
//! - **`hedge volatility` with no hedge in the program.** A hedge with no bound is a
//!   hedge with nothing to check, so the clause has to point at the `atomic_hedge`
//!   whose `require delta <= …` states what the position has to net to. Recording
//!   the word would be recording an intention as if it were a constraint.
//! - **`settle_across_domains` over one domain.** Settlement across domains is a
//!   claim about how many ledgers the legs touch; over one domain it is empty, and
//!   saying it anyway would describe a cross-domain trade that is not one.
//! - **A net-profit floor of zero.** A floor of zero accepts every trade.

use std::collections::{BTreeMap, BTreeSet};

use x3_lang_ast::ast::{HyperarbDecl, Item, Program, VenueDecl};
use x3_lang_common::{ErrorAccumulator, Span, X3Error};

/// A `hyperarb` whose clauses all resolve to something real.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyperarbPlan {
    pub name: String,
    /// The committed capital: its amount, and `chain.ASSET`.
    pub capital: (u128, String),
    /// `(leg name, what it resolved to)` in declaration order.
    pub legs: Vec<(String, String)>,
    /// The criterion the choice is ranked by.
    pub choose: String,
    pub hedge_volatility: bool,
    pub settle_across_domains: bool,
    pub net_profit_bps: u16,
    /// Every domain the resolved legs cover, sorted.
    pub domains: Vec<String>,
}

/// What a leg's target names, once resolution has succeeded.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolution {
    Venue(String),
    Chain(String),
    Domain(String),
}

impl Resolution {
    fn describe(&self) -> String {
        match self {
            Resolution::Venue(name) => format!("the venue '{name}'"),
            Resolution::Chain(chain) => format!("the chain '{chain}'"),
            Resolution::Domain(domain) => format!("the domain '{domain}'"),
        }
    }
}

/// Decide a `hyperarb` declaration against the program it sits in.
pub fn analyse(program: &Program, decl: &HyperarbDecl) -> Result<HyperarbPlan, String> {
    let name = decl.name.as_str();
    let venues = declared_venues(program);

    // --- capital ----------------------------------------------------------
    let Some((amount, asset)) = &decl.capital else {
        return Err(format!(
            "the hyperarb '{name}' declares no `capital`; the trade it describes has nothing to \
             commit"
        ));
    };
    let asset_key = format!("{}.{}", asset.chain.as_str(), asset.name.as_str());
    if decl.flash {
        return Err(format!(
            "the hyperarb '{name}' declares `capital = flash({amount} {asset_key})`, and flash \
             collateral must not ship before a formal safety proof (spec PHASE 20); commit the \
             capital instead, or the declaration states a borrowing the runtime cannot honour"
        ));
    }
    if *amount == 0 {
        return Err(format!(
            "the hyperarb '{name}' commits zero capital: a trade with no capital has no position to \
             hedge and no profit to floor"
        ));
    }
    if venues.is_empty() {
        return Err(format!(
            "the hyperarb '{name}' declares legs to evaluate and the program declares no `venue`, \
             so there is no path for a leg to name: a `hyperarb` routes over the venues its program \
             declares"
        ));
    }

    // --- legs -------------------------------------------------------------
    if decl.legs.len() < 2 {
        return Err(format!(
            "the hyperarb '{name}' declares {} leg(s); `choose {}` ranks the paths it is given, and \
             with fewer than two there is nothing to choose between",
            decl.legs.len(),
            decl.choose.as_str()
        ));
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut legs: Vec<(String, String)> = Vec::with_capacity(decl.legs.len());
    let mut domains: BTreeSet<String> = BTreeSet::new();
    for leg in &decl.legs {
        let leg_name = leg.name.as_str();
        if !seen.insert(leg_name) {
            return Err(format!(
                "the hyperarb '{name}' names the leg '{leg_name}' twice; two legs with one name \
                 cannot be told apart in the choice it ranks them with"
            ));
        }
        let target = leg.target.as_str();
        let resolution = resolve(&venues, target)?;
        for venue in match &resolution {
            Resolution::Venue(wanted) => venues
                .values()
                .copied()
                .filter(|venue| venue.name.as_str() == wanted)
                .collect::<Vec<_>>(),
            Resolution::Chain(chain) => venues
                .values()
                .copied()
                .filter(|venue| venue.chain.as_str() == chain)
                .collect::<Vec<_>>(),
            Resolution::Domain(domain) => venues
                .values()
                .copied()
                .filter(|venue| venue.domain.as_str() == domain)
                .collect::<Vec<_>>(),
        } {
            domains.insert(venue.domain.as_str().to_string());
        }
        legs.push((leg_name.to_string(), resolution.describe()));
    }

    // --- hedge ------------------------------------------------------------
    if decl.hedge_volatility {
        // An `atomic_hedge` carries no name of its own — it is identified by the
        // exposure it decides (`delta_bound_bps`) — so the check is existence, and
        // the bound it carries is what the refusal quotes when there is none.
        let hedges: Vec<u32> = program
            .items
            .iter()
            .filter_map(|item| match &item.node {
                Item::AtomicHedge(hedge) => hedge.delta_bound_bps,
                _ => None,
            })
            .collect();
        if hedges.is_empty() {
            return Err(format!(
                "the hyperarb '{name}' says `hedge volatility` and the program declares no \
                 `atomic_hedge`, so the hedge has no bound to net to: write the `atomic_hedge` whose \
                 `require delta <= …` states what the position has to net within (PHASE 9), because \
                 a hedge with no bound is an intention recorded as a constraint"
            ));
        }
    }

    // --- settlement -------------------------------------------------------
    // Checked in `plan`, against the route that actually settles: the legs are
    // alternative candidate routes and `choose` selects one, so "across domains" is a
    // claim about the selected leg's path rather than about the union of everything the
    // declarations mention. Checking the union here would accept a hyperarb whose
    // selected route stays on one chain and refuse one whose alternatives happen to
    // share a domain.

    if decl.net_profit_bps == 0 {
        return Err(format!(
            "the hyperarb '{name}' declares `require net_profit >= 0bps`: a floor of zero accepts \
             every trade, which is the same as stating no floor at all"
        ));
    }

    Ok(HyperarbPlan {
        name: name.to_string(),
        capital: (*amount, asset_key),
        legs,
        choose: decl.choose.as_str().to_string(),
        hedge_volatility: decl.hedge_volatility,
        settle_across_domains: decl.settle_across_domains,
        net_profit_bps: decl.net_profit_bps,
        domains: domains.into_iter().collect(),
    })
}

/// Every venue the program declares, by name.
fn declared_venues(program: &Program) -> BTreeMap<String, &VenueDecl> {
    program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::VenueDecl(venue) => Some((venue.name.as_str().to_string(), venue)),
            _ => None,
        })
        .collect()
}

/// Resolve a leg's target to something the program actually declares.
///
/// A venue name, a chain or a domain all resolve, because "evaluate the paths on
/// ethereum" is as meaningful as "evaluate this venue". Anything else is a route to
/// nowhere, and the refusal lists what the program does declare.
fn resolve(venues: &BTreeMap<String, &VenueDecl>, target: &str) -> Result<Resolution, String> {
    if venues.contains_key(target) {
        return Ok(Resolution::Venue(target.to_string()));
    }
    let chains: BTreeSet<String> = venues
        .values()
        .flat_map(|venue| {
            [
                venue.chain.as_str().to_string(),
                venue.asset_in.chain.as_str().to_string(),
                venue.asset_out.chain.as_str().to_string(),
            ]
        })
        .collect();
    if chains.contains(target) {
        return Ok(Resolution::Chain(target.to_string()));
    }
    let domains: BTreeSet<String> = venues.values().map(|venue| venue.domain.as_str().to_string()).collect();
    if domains.contains(target) {
        return Ok(Resolution::Domain(target.to_string()));
    }
    Err(format!(
        "`evaluate({target})` names nothing the program declares, so the leg is a route to nowhere: \
         the venues are [{}], the chains are [{}] and the domains are [{}]",
        venues.keys().cloned().collect::<Vec<_>>().join(", "),
        chains.into_iter().collect::<Vec<_>>().join(", "),
        domains.into_iter().collect::<Vec<_>>().join(", ")
    ))
}

/// Check every `hyperarb` in a program, in the same layer as the hedge,
/// liquidation, rebalance, netting and arb checks — before anything is lowered.
pub fn verify(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::Hyperarb(decl) = &item.node else {
            continue;
        };
        if let Err(reason) = analyse(program, decl) {
            acc.add_error(X3Error::SemanticError {
                message: reason,
                span: Span::DUMMY,
            });
        }
    }
}

/// One leg's route: the asset path it takes and the venues approved to serve it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegRoute {
    pub name: String,
    /// `chain.ASSET` along the leg, starting at the venue's input asset.
    pub path: Vec<String>,
    /// The venues that may serve this leg, cheapest declared fee first. The order is
    /// the compiler's preference and it is carried rather than implied.
    pub approved: Vec<String>,
    /// The declared fee of the leg's cheapest venue, in basis points.
    pub declared_fee_bps: u32,
    /// Whether the leg's path moves value between chains.
    pub crosses_chains: bool,
}

/// The plan a `hyperarb` lowers to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutePlan {
    pub name: String,
    /// The committed capital: its amount and `chain.ASSET`.
    pub capital: (u128, String),
    /// Every leg, in declaration order.
    pub legs: Vec<LegRoute>,
    /// The leg `choose` selected, as an index into `legs`.
    pub selected: usize,
    /// The criterion the selection was made by.
    pub criterion: x3_lang_ast::ast::ChoiceCriterion,
    pub net_profit_bps: u16,
}

/// Turn a decided `hyperarb` into the route its `choose` clause selects.
///
/// ## The reading this takes, and why
///
/// `parallel { route_a = evaluate(…); … }` is read as **candidate routes**, not as legs
/// that all run. The phase's own description of its lowerings is "Opportunity Graph →
/// Candidate Routes → Filter → Dependency DAG → Risk Verification → Execution Plan →
/// Atomic Settlement": candidates are *filtered*, which is a selection among
/// alternatives. And `choose` only means anything under that reading — you cannot
/// choose one of three things you have already run. So the plan is an `AtomicChoice`
/// over the legs followed by the selected leg's operations, which is the machinery
/// `atomic_choice` already has.
///
/// ## What cannot be ranked, and is refused
///
/// `choose highest_net_output` ranks the legs by their outputs. `opportunity.rs` opens
/// by naming what it does not model — "What it deliberately does *not* do: model price
/// impact as a function" — and a venue edge carries fee, slippage, liquidity, latency,
/// finality and risk and no price. So that criterion needs a number the compiler does
/// not have, which is already why the language refuses `maximize profit` for an
/// objective. `fewest_hops` and `lowest_declared_fee` *are* computable, and the phase's
/// own example is the one that is not.
///
/// ## The exchange, stated plainly
///
/// Every leg is one venue's asset pair, because that is what a leg's target names and
/// what the graph can be sure of: a leg whose venues disagree about which assets they
/// move is refused rather than resolved to one of their pairs. So the legs are
/// alternatives between venues, and `choose` is a choice among them — not a search for
/// the most profitable path through all of them, which needs prices.
pub fn plan(program: &Program, decl: &HyperarbDecl) -> Result<RoutePlan, String> {
    let decided = analyse(program, decl)?;
    let venues = declared_venues(program);

    let mut legs: Vec<LegRoute> = Vec::with_capacity(decl.legs.len());
    for leg in &decl.legs {
        let target = leg.target.as_str();
        let reached: Vec<&VenueDecl> = match resolve(&venues, target)? {
            Resolution::Venue(wanted) => venues
                .values()
                .copied()
                .filter(|venue| venue.name.as_str() == wanted)
                .collect(),
            Resolution::Chain(chain) => venues
                .values()
                .copied()
                .filter(|venue| venue.chain.as_str() == chain)
                .collect(),
            Resolution::Domain(domain) => venues
                .values()
                .copied()
                .filter(|venue| venue.domain.as_str() == domain)
                .collect(),
        };
        if reached.is_empty() {
            return Err(format!(
                "the hyperarb '{}' leg '{}' resolves to '{target}' and no venue serves it, so the \
                 leg has no route to take",
                decided.name,
                leg.name.as_str()
            ));
        }

        // The venues a leg resolves to have to agree about what the leg moves. A chain
        // or a domain can hold several venues and they need not pair the same assets;
        // resolving to one of their pairs would be choosing a route nobody wrote.
        let first = &(reached[0].asset_in.clone(), reached[0].asset_out.clone());
        let disagreement: Vec<String> = reached
            .iter()
            .filter(|venue| &(venue.asset_in.clone(), venue.asset_out.clone()) != first)
            .map(|venue| {
                format!(
                    "{} ({} -> {})",
                    venue.name.as_str(),
                    crate::opportunity::asset_label(&venue.asset_in),
                    crate::opportunity::asset_label(&venue.asset_out)
                )
            })
            .collect();
        if !disagreement.is_empty() {
            return Err(format!(
                "the hyperarb '{}' leg '{}' resolves to venues that do not agree on an asset pair: \
                 {} takes {} -> {}, while {} take other pairs. A leg whose venues move different \
                 assets has no single route, so name the venue whose route the leg means",
                decided.name,
                leg.name.as_str(),
                reached[0].name.as_str(),
                crate::opportunity::asset_label(&reached[0].asset_in),
                crate::opportunity::asset_label(&reached[0].asset_out),
                disagreement.join(", ")
            ));
        }

        // Cheapest declared fee first, then by name: the preference is a function of the
        // declarations rather than of the order they were written in.
        let mut ordered: Vec<&VenueDecl> = reached.clone();
        ordered.sort_by(|left, right| {
            left.fee_bps
                .cmp(&right.fee_bps)
                .then(left.name.as_str().cmp(right.name.as_str()))
        });

        // Every leg is an alternative route for the *same* trade from the *same*
        // capital, so a leg whose input asset is not the capital's is a route for a
        // different trade: the amount the plan carries is denominated in the capital,
        // and spending it on a leg that takes another asset would be spending it twice
        // in two units. Refused with both assets rather than converted, because a
        // conversion is a trade nobody wrote.
        let leg_input = crate::opportunity::asset_label(&ordered[0].asset_in);
        if leg_input != decided.capital.1 {
            return Err(format!(
                "the hyperarb '{}' leg '{}' takes {leg_input} while the capital is committed in \
                 {}; the legs of one hyperarb are alternative routes for one trade, so a leg that \
                 takes another asset is a route for a different trade",
                decided.name,
                leg.name.as_str(),
                decided.capital.1
            ));
        }

        legs.push(LegRoute {
            name: leg.name.as_str().to_string(),
            path: vec![
                crate::opportunity::asset_label(&ordered[0].asset_in),
                crate::opportunity::asset_label(&ordered[0].asset_out),
            ],
            approved: ordered.iter().map(|venue| venue.name.as_str().to_string()).collect(),
            declared_fee_bps: ordered[0].fee_bps,
            crosses_chains: ordered[0].asset_in.chain.as_str() != ordered[0].asset_out.chain.as_str(),
        });
    }

    let selected = select_leg(decl, &legs, &decided.name)?;

    // The plan contains a swap, and `semantic::verify_slippage_explicit` refuses a swap
    // leg in an artifact with no explicit slippage bound. The hyperarb has no slippage
    // clause of its own — the phase's example states only a profit floor — so the bound
    // has to come from the program, and a program that states none is refused here with
    // the rule named rather than emitted into an artifact the next layer rejects.
    let bounded = crate::semantic::require_guards(program)
        .into_iter()
        .any(|(_, guard)| guard.kind == x3_lang_ast::ast::RequireKind::Slippage);
    if !bounded {
        return Err(format!(
            "the hyperarb '{}' plans a swap route and this program declares no `require slippage <= \
             <n>` bound; a swap without an explicit ceiling is a leg whose price is unconstrained, \
             so write the bound where the trade is bounded",
            decided.name
        ));
    }

    // `settle_across_domains` is a claim about the route that settles, which under this
    // reading is the selected leg. A leg that stays on one chain settles on one ledger.
    if decided.settle_across_domains && !legs[selected].crosses_chains {
        return Err(format!(
            "the hyperarb '{}' says `settle_across_domains` and the leg it selects ('{}') moves {} \
             within one chain; a settlement across domains is one whose route crosses chains, and \
             over one chain the claim is empty",
            decided.name,
            legs[selected].name,
            legs[selected].path.join(" -> ")
        ));
    }

    Ok(RoutePlan {
        name: decided.name,
        capital: decided.capital,
        legs,
        selected,
        criterion: decl.choose,
        net_profit_bps: decided.net_profit_bps,
    })
}

/// Which leg `choose` selects.
///
/// Ties go to the earliest declared leg, which is `atomic_choice`'s own convention and
/// what makes the selection a function of the program text.
fn select_leg(decl: &HyperarbDecl, legs: &[LegRoute], name: &str) -> Result<usize, String> {
    use x3_lang_ast::ast::ChoiceCriterion;

    match decl.choose {
        ChoiceCriterion::LowestDeclaredFee => Ok(legs
            .iter()
            .enumerate()
            .min_by_key(|(index, leg)| (leg.declared_fee_bps, *index))
            .map(|(index, _)| index)
            .unwrap_or(0)),
        // Every leg is one venue's pair, so every leg is one hop and the criterion
        // cannot separate them: the earliest declared leg is the selection, which the
        // convention above makes a decision rather than an accident.
        ChoiceCriterion::FewestHops => Ok(0),
        ChoiceCriterion::HighestNetOutput => Err(format!(
            "the hyperarb '{name}' chooses by `highest_net_output`, which ranks the legs by their \
             outputs; the opportunity graph holds venue attributes and no price, so the compiler \
             has no output to rank by and will not invent one. Choose `lowest_declared_fee` or \
             `fewest_hops`, which are computable from what the program declares"
        )),
    }
}
