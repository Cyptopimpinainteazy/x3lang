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
    if decl.settle_across_domains && domains.len() < 2 {
        return Err(format!(
            "the hyperarb '{name}' says `settle_across_domains` and its legs resolve onto {} \
             domain(s) ({}); settlement across domains is a claim about how many ledgers the legs \
             touch, and over one it is empty",
            domains.len(),
            if domains.is_empty() {
                "none".to_string()
            } else {
                domains.iter().cloned().collect::<Vec<_>>().join(", ")
            }
        ));
    }

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
