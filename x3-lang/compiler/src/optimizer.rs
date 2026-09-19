//! The deterministic route optimizer — spec build-order item 17.
//!
//! Given the opportunity graph, an objective and a set of constraints, choose
//! one route. PHASE 42 is explicit about what "choose" may depend on: no
//! unordered maps, no wall clock, no randomness, no thread scheduling, no
//! floating-point ambiguity, no non-deterministic optimizer output. It
//! prescribes stable ordering, bounded search and canonical tie-breaking, and
//! this module is those three things.
//!
//! Three decisions worth stating, because each is where an optimizer usually
//! stops being reviewable:
//!
//! - **A tie is reported, not hidden.** Two routes can be equal on the
//!   objective and differ on everything else. The optimizer breaks the tie
//!   canonically and then *tells the caller it did* — a caller that believes
//!   the objective decided when a tie-break did has been misled about the
//!   quality of the decision, not just its provenance.
//! - **Infeasible names the blocking constraint.** "No route" is useless to a
//!   program author; "no route because every candidate has more slippage than
//!   `max_slippage_bps`" is actionable, and it comes from the same function the
//!   search used rather than from a second opinion computed beside it.
//! - **Exhausting the budget is not an answer.** It is its own outcome, because
//!   reporting an unreachable route for one that was never finished considering
//!   is the worst failure mode this module has.

use serde::{Deserialize, Serialize};

use crate::opportunity::{
    reject_reason, search_with_budget, Opportunity, OpportunityConstraints, OpportunityGraph, RejectionReason,
    SearchOutcome,
};

/// What a route is being optimized for.
///
/// A closed set, for the same reason `atomic_choice`'s criteria are closed: the
/// compiler can only optimize what it can rank, and every member here ranks on
/// an integer attribute a venue declares. "Maximize net profit" needs amounts
/// and prices, which the graph does not model yet, so it is not in the set
/// rather than being approximated into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Objective {
    /// Cheapest total fee.
    MinimizeFees,
    /// Tightest worst-case slippage.
    MinimizeSlippage,
    /// Lowest risk exposure.
    MinimizeRisk,
    /// Fastest total latency.
    MinimizeLatency,
    /// Least finality required to settle.
    MinimizeFinality,
}

impl Objective {
    pub fn as_str(self) -> &'static str {
        match self {
            Objective::MinimizeFees => "minimize_fees",
            Objective::MinimizeSlippage => "minimize_slippage",
            Objective::MinimizeRisk => "minimize_risk",
            Objective::MinimizeLatency => "minimize_latency",
            Objective::MinimizeFinality => "minimize_finality",
        }
    }

    /// The objectives the language accepts. The parser and the unknown-objective
    /// error message both read this, so they cannot list different sets.
    pub const ALL: &'static [Objective] = &[
        Objective::MinimizeFees,
        Objective::MinimizeSlippage,
        Objective::MinimizeRisk,
        Objective::MinimizeLatency,
        Objective::MinimizeFinality,
    ];

    pub fn parse(name: &str) -> Option<Objective> {
        Objective::ALL
            .iter()
            .copied()
            .find(|objective| objective.as_str() == name)
    }

    /// The value this objective ranks on, smaller being better.
    pub fn value(self, opportunity: &Opportunity) -> u32 {
        match self {
            Objective::MinimizeFees => opportunity.fee_bps,
            Objective::MinimizeSlippage => opportunity.slippage_bps,
            Objective::MinimizeRisk => opportunity.max_risk,
            Objective::MinimizeLatency => opportunity.latency_ms,
            Objective::MinimizeFinality => opportunity.finality_blocks,
        }
    }
}

/// Why no route could be chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoRoute {
    /// The search found no path at all within the constraints.
    None,
    /// Every candidate path was refused, and these are the refusals.
    AllRefused { refused: Vec<(String, RejectionReason)> },
    /// The search ran out of expansion budget before finishing.
    BudgetExhausted { examined: usize, budget: usize },
}

/// What the optimizer decided, and enough to review the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptimizationReport {
    /// The chosen route, if one was chosen.
    pub chosen: Option<Opportunity>,
    /// The objective's value on the chosen route.
    pub objective_value: Option<u32>,
    /// How many opportunities the search found.
    pub considered: usize,
    /// Routes that were equal to the winner *on the objective*. Non-empty means
    /// the objective did not decide; the canonical tie-break did, and the caller
    /// is being told so.
    pub tied: Vec<Opportunity>,
    /// Why nothing was chosen, when nothing was.
    pub no_route: Option<NoRoute>,
}

impl OptimizationReport {
    /// Whether the choice was made by the objective alone.
    pub fn decided_by_objective(&self) -> bool {
        self.chosen.is_some() && self.tied.is_empty()
    }
}

/// Choose one route, deterministically.
///
/// Ranking is lexicographic: the objective's value first, then the route's own
/// canonical form. The second term is what makes the result a function of the
/// graph rather than of the order the search happened to reach things in — two
/// routes that tie on the objective are ordered by their venue lists, compared
/// as sequences. That is a total order, so there is no case where the optimizer
/// must "just pick one".
pub fn optimize(
    graph: &OpportunityGraph,
    from: &str,
    to: &str,
    objective: Objective,
    constraints: &OpportunityConstraints,
) -> OptimizationReport {
    optimize_with_budget(
        graph,
        from,
        to,
        objective,
        constraints,
        crate::opportunity::DEFAULT_MAX_EXPANSIONS,
    )
}

/// The same optimization with an explicit expansion budget.
pub fn optimize_with_budget(
    graph: &OpportunityGraph,
    from: &str,
    to: &str,
    objective: Objective,
    constraints: &OpportunityConstraints,
    max_expansions: usize,
) -> OptimizationReport {
    let outcome = search_with_budget(graph, from, to, constraints, max_expansions);
    let found = match &outcome {
        SearchOutcome::Found(found) => found.clone(),
        SearchOutcome::BudgetExhausted { examined, budget } => {
            return OptimizationReport {
                chosen: None,
                objective_value: None,
                considered: 0,
                tied: Vec::new(),
                no_route: Some(NoRoute::BudgetExhausted {
                    examined: *examined,
                    budget: *budget,
                }),
            }
        }
    };

    if found.is_empty() {
        // Say which venues were refused and why. Only the edges leaving the
        // start are worth naming when nothing was reachable at all; when the
        // start had edges, the refusals on them are the whole story.
        let refused: Vec<(String, RejectionReason)> = graph
            .edges
            .iter()
            .filter_map(|edge| reject_reason(edge, constraints).map(|reason| (edge.venue.clone(), reason)))
            .collect();
        return OptimizationReport {
            chosen: None,
            objective_value: None,
            considered: 0,
            tied: Vec::new(),
            no_route: Some(if refused.is_empty() {
                NoRoute::None
            } else {
                NoRoute::AllRefused { refused }
            }),
        };
    }

    let mut ranked = found;
    ranked.sort_by(|left, right| {
        objective
            .value(left)
            .cmp(&objective.value(right))
            .then_with(|| left.venues.cmp(&right.venues))
    });

    let best_value = objective.value(&ranked[0]);
    let chosen = ranked[0].clone();
    // Everything that ties with the winner on the objective: the caller has to
    // know the objective did not separate them.
    let tied: Vec<Opportunity> = ranked
        .iter()
        .skip(1)
        .filter(|opportunity| objective.value(opportunity) == best_value)
        .cloned()
        .collect();

    OptimizationReport {
        chosen: Some(chosen),
        objective_value: Some(best_value),
        considered: ranked.len(),
        tied,
        no_route: None,
    }
}
