//! Objective declarations — spec PHASE 15.
//!
//! `objective { minimize fees; constraints { hops <= 4 } }` is what the planner
//! was already being told on a command line, declared in the program instead.
//!
//! Two of PHASE 15's requirements shape this module:
//!
//! - **"Multi-objective optimization must be deterministic."** One objective,
//!   one metric: a declaration that ranks two things has no defined answer, so it
//!   is refused rather than combined by some weighting nobody wrote down.
//! - **"Do not allow non-deterministic solver output to affect consensus."** The
//!   metric must be one the optimizer can rank from what the opportunity graph
//!   holds. PHASE 15 lists eight metrics and four of them — profit, output,
//!   external liquidity, capital efficiency — need quantities the graph does not
//!   model, so they are refused *with that reason*. Accepting them would leave
//!   the ranking to whatever a solver decided, which is exactly the thing the
//!   phase forbids.

use x3_lang_ast::ast::{Item, ObjectiveConstraints, ObjectiveMetric, Program, RiskBound};
use x3_lang_common::{ErrorAccumulator, X3Error};

use crate::optimizer::Objective;

fn err(message: String) -> X3Error {
    X3Error::SemanticError {
        message,
        span: x3_lang_common::Span::DUMMY,
    }
}

/// The optimizer's criterion for a declared metric, or why there is none.
pub fn criterion_for(metric: ObjectiveMetric) -> Result<Objective, &'static str> {
    match metric {
        ObjectiveMetric::MinimizeFees => Ok(Objective::MinimizeFees),
        ObjectiveMetric::MinimizeSlippage => Ok(Objective::MinimizeSlippage),
        ObjectiveMetric::MinimizeRisk => Ok(Objective::MinimizeRisk),
        ObjectiveMetric::MinimizeExecutionTime => Ok(Objective::MinimizeLatency),
        ObjectiveMetric::MinimizeFinality => Ok(Objective::MinimizeFinality),
        ObjectiveMetric::MaximizeProfit => Err(
            "the opportunity graph holds venue attributes, not amounts or prices, so the optimizer \
             cannot rank two routes by profit; ranking them would fall to a solver, which is what \
             PHASE 15 forbids",
        ),
        ObjectiveMetric::MaximizeOutput => Err(
            "the graph does not model the quantities a route outputs, so a ranking by output would \
             be a solver's opinion rather than the compiler's",
        ),
        ObjectiveMetric::MinimizeExternalLiquidity => Err(
            "the graph does not distinguish liquidity the ring supplies from liquidity it has to \
             borrow, so there is nothing to minimise",
        ),
        ObjectiveMetric::MaximizeCapitalEfficiency => Err(
            "the graph does not model capital or returns, so capital efficiency has no value to rank \
             on",
        ),
    }
}

/// The name an `objective { … }` declaration without one gets.
///
/// PHASE 15's example is anonymous, and a declaration still needs something for
/// a diagnostic to point at — so the parser names it, and the messages here
/// recognise that name rather than printing `objective 'objective'`.
pub const ANONYMOUS_OBJECTIVE_NAME: &str = "objective";

/// How a diagnostic refers to an objective.
fn label(name: &str) -> String {
    if name == ANONYMOUS_OBJECTIVE_NAME {
        "the objective".to_string()
    } else {
        format!("objective '{name}'")
    }
}

/// The program's objective declaration.
///
/// `None` when there is none and when there is more than one: two declarations
/// have no single answer, the verifier reports that, and a caller that asks
/// anyway gets nothing rather than whichever came first.
pub fn declaration_of(program: &Program) -> Option<&x3_lang_ast::ast::ObjectiveDecl> {
    let mut found = None;
    for item in &program.items {
        if let Item::ObjectiveDecl(objective) = &item.node {
            if found.is_some() {
                return None;
            }
            found = Some(objective);
        }
    }
    found
}

/// Verify every objective declaration is one the planner can actually follow.
pub fn verify_objective_decls(program: &Program, acc: &mut ErrorAccumulator) {
    let strategies: Vec<&x3_lang_ast::ast::CrossChainStrategy> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::Strategy(strategy) => Some(strategy),
            _ => None,
        })
        .collect();
    let objectives: Vec<&x3_lang_ast::ast::ObjectiveDecl> = program
        .items
        .iter()
        .filter_map(|item| match &item.node {
            Item::ObjectiveDecl(objective) => Some(objective),
            _ => None,
        })
        .collect();

    // One program, one objective. A second declaration would leave which one
    // the planner follows to whoever reads them, which is the same
    // non-determinism that refusing two metrics in one objective prevents.
    if objectives.len() > 1 {
        acc.add_error(err(format!(
            "the program declares {} objectives ({}); the compiler follows one, so the rest have \
             no effect",
            objectives.len(),
            objectives
                .iter()
                .map(|o| o.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    for (index, objective) in objectives.iter().enumerate() {
        let name = objective.name.as_str();
        // Two anonymous declarations share the name the parser gave them, and
        // the count above has already said what is wrong with that; reporting a
        // duplicate name as well would name the same mistake twice.
        if name != ANONYMOUS_OBJECTIVE_NAME && objectives[..index].iter().any(|seen| seen.name.as_str() == name) {
            acc.add_error(err(format!(
                "objective '{name}' is declared twice; two declarations with one name cannot be \
                 told apart in a diagnostic"
            )));
        }
    }
    if !objectives.is_empty() && !program.items.iter().any(|item| matches!(item.node, Item::VenueDecl(_))) {
        acc.add_error(err(format!(
            "objective '{}' ranks routes over a program that declares no venues; there is nothing \
             to rank",
            objectives[0].name.as_str()
        )));
    }

    for item in &program.items {
        let Item::ObjectiveDecl(objective) = &item.node else {
            continue;
        };
        let name = objective.name.as_str();

        if let Err(reason) = criterion_for(objective.metric) {
            acc.add_error(err(format!(
                "{} cannot rank '{}': {reason}",
                label(name),
                objective.metric.as_str()
            )));
        }

        let constraints = &objective.constraints;
        for (field, value) in [("hops", constraints.max_hops), ("chains", constraints.max_chains)] {
            if value == Some(0) {
                acc.add_error(err(format!(
                    "{} bounds {field} at zero; a route with no {field} is not a route",
                    label(name)
                )));
            }
        }
        for (field, value) in [
            ("fees", constraints.max_fees_bps),
            ("slippage", constraints.max_slippage_bps),
        ] {
            if value.is_some_and(|value| value > 10_000) {
                acc.add_error(err(format!(
                    "{} bounds {field} above 10,000 bps, which is the whole amount",
                    label(name)
                )));
            }
        }
        if let Some(capital) = &constraints.capital {
            match &capital.value {
                x3_lang_ast::ast::Expression::Literal(x3_lang_ast::ast::LiteralExpr::Int { value, .. })
                    if *value > 0 => {}
                _ => acc.add_error(err(format!(
                    "{} requires capital in {} but states no positive integer amount, so no size can \
                     be checked against it",
                    label(name),
                    capital.asset.as_str()
                ))),
            }
        }

        // `private` is a claim about how the submission travels, and the place
        // that says so is a strategy's submission policy. An objective asking for
        // privacy beside no such policy is asking for something the artifact does
        // not declare.
        if constraints.private
            && !strategies.iter().any(|strategy| {
                strategy
                    .submission
                    .is_some_and(|submission| submission.private != x3_lang_ast::ast::PrivateSubmissionMode::Allowed)
            })
        {
            acc.add_error(err(format!(
                "{} requires `private` execution and no strategy declares a submission policy that \
                 provides it; add `submission {{ private = required }}` to the module the objective \
                 applies to",
                label(name)
            )));
        }

        // `risk <= strategy.policy` reads the module's declared profile rather
        // than restating a number. With several modules it names none of them.
        if constraints.max_risk == Some(RiskBound::StrategyPolicy) {
            let declaring: Vec<&str> = strategies
                .iter()
                .filter(|strategy| strategy.risk.is_some())
                .map(|strategy| strategy.name.as_str())
                .collect();
            match declaring.len() {
                0 => acc.add_error(err(format!(
                    "{} bounds risk by `strategy.policy` and no strategy declares a risk profile; \
                     there is no policy to bound it by",
                    label(name)
                ))),
                1 => {}
                _ => acc.add_error(err(format!(
                    "{} bounds risk by `strategy.policy`, which is ambiguous: {} strategy modules \
                     declare risk profiles",
                    label(name),
                    declaring.len()
                ))),
            }
        }
    }
}

/// The constraints a declared objective imposes on the planner.
///
/// `risk <= strategy.policy` resolves here, against the one module that declares
/// a profile, because that is the only place the figures exist.
pub fn constraints_for(
    objective: &ObjectiveConstraints,
    program: &Program,
) -> crate::opportunity::OpportunityConstraints {
    // A profile states its bounds as ceilings on slippage and total fee. Both
    // apply alongside anything the objective states, and two ceilings on one
    // quantity are still both ceilings: the tighter one is the bound.
    let policy = match objective.max_risk {
        Some(RiskBound::StrategyPolicy) => program.items.iter().find_map(|item| match &item.node {
            Item::Strategy(strategy) => strategy.risk.as_ref(),
            _ => None,
        }),
        _ => None,
    };
    crate::opportunity::OpportunityConstraints {
        max_hops: objective.max_hops.map(|hops| hops as usize).unwrap_or(0),
        max_chains: objective.max_chains,
        // An objective constrains paths by *count*, not by name: `max_chains` says how
        // many chains a path may touch and nothing in its grammar says which ones. The
        // `arb` block is where a chain set is stated, so it is the caller that fills
        // this in (PHASE 37).
        allowed_chains: None,
        max_fee_bps: tighten(objective.max_fees_bps, policy.map(|policy| policy.max_total_fee_bps)),
        max_slippage_bps: tighten(objective.max_slippage_bps, policy.map(|policy| policy.max_slippage_bps)),
        // `capital <= N <ASSET>` is the size the program means to commit, and
        // the graph's liquidity figure is what a venue can absorb, so the
        // declaration becomes a floor on each venue's depth.
        min_liquidity: objective.capital.as_ref().and_then(|capital| match &capital.value {
            x3_lang_ast::ast::Expression::Literal(x3_lang_ast::ast::LiteralExpr::Int { value, .. }) => Some(*value),
            _ => None,
        }),
        max_latency_ms: objective.max_execution_time_ms,
        max_finality_blocks: objective.max_finality_blocks,
        max_risk: match objective.max_risk {
            Some(RiskBound::Score(score)) => Some(score),
            // A profile bounds slippage and total fee, not a risk score. There
            // is no number to compare a venue's declared risk against, so none
            // is invented from one of the other two.
            Some(RiskBound::StrategyPolicy) | None => None,
        },
        require_proof: false,
    }
}

/// The tighter of two ceilings, either of which may be unstated.
fn tighten(declared: Option<u32>, policy: Option<u32>) -> Option<u32> {
    match (declared, policy) {
        (Some(declared), Some(policy)) => Some(declared.min(policy)),
        (declared, policy) => declared.or(policy),
    }
}
