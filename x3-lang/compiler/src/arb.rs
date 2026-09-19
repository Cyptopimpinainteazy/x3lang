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

use std::collections::BTreeSet;

use x3_lang_ast::ast::{ArbDecl, Expression, Item, Program, RequireKind};
use x3_lang_common::{ErrorAccumulator, Span, X3Error};

use crate::lowering;
use crate::opportunity::{path_reject_reason, reject_reason, OpportunityConstraints, OpportunityGraph};
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
    ("Execution Plan", None),
    ("Atomic Settlement", None),
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
    // above the whole trade is not a bound.
    let margin = u32::from(min_profit_bps) + u32::from(max_slippage_bps) + u32::from(max_total_fee_bps);
    if margin >= 10_000 {
        return Err(format!(
            "the arb '{name}' declares min_profit {min_profit_bps}bps, max_slippage \
             {max_slippage_bps}bps and max_total_fee {max_total_fee_bps}bps, which is {margin}bps — \
             the whole trade — so the three bounds together permit the trade to keep nothing and \
             still be called profitable"
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

/// The constraints a routing stage would search under, built from the scope.
///
/// This is the mapping PHASE 37 asks for: the declaration is the source-level spelling
/// of what the opportunity search takes, so the compiler can *run* the filter the plan
/// would run rather than trusting that the declaration's numbers are compatible with
/// the graph. The units line up by construction — slippage and the fee ceiling are both
/// in basis points, the liquidity floor is an amount — except for the deadline:
///
/// `max_latency_ms` is left unset, and that is a statement rather than an omission. The
/// policy records the deadline in *blocks*, a venue declares its latency in
/// *milliseconds*, and `expression_to_blocks` rounds up to whole blocks — the safe
/// direction for a window, the permissive one for a filter. Deriving one figure from the
/// other would let a venue through that the declared budget excludes, so that comparison
/// belongs to the plan generator (TICKET-073) rather than to a loose reading here.
/// `max_finality_blocks`, `max_risk` and `require_proof` are unset because the scope
/// states nothing about them: they are the objective's and the venue's vocabulary, and
/// inventing a value would be a bound nobody wrote.
pub fn search_constraints(policy: &ArbPolicy) -> OpportunityConstraints {
    OpportunityConstraints {
        max_hops: policy.max_hops as usize,
        // The set, not a count: `chains = [a, b]` says which chains, and the count is a
        // consequence of that statement rather than the statement itself.
        max_chains: None,
        allowed_chains: Some(policy.chains.clone()),
        max_fee_bps: Some(u32::from(policy.max_total_fee_bps)),
        max_slippage_bps: Some(u32::from(policy.max_slippage_bps)),
        min_liquidity: Some(policy.liquidity_min.0),
        max_latency_ms: None,
        max_finality_blocks: None,
        max_risk: None,
        require_proof: false,
    }
}

/// The declared venues the scope's own bounds admit, and why each of the rest is out.
///
/// A venue is admitted when it survives both functions the search itself uses: the
/// per-venue bounds ([`reject_reason`]) and the path bounds as they apply to the shortest
/// path through it ([`path_reject_reason`] on a one-hop path). The second call is what
/// gives the fee ceiling and the chain set teeth, because those are properties of a path
/// rather than of any single edge. One hop is the weakest claim the graph can support,
/// and it is exactly the claim this check makes: *something* is there to rank.
///
/// The wording of each refusal is the reason enum's `Debug`, which is the rendering
/// `x3c optimize` already ships for the same refusals: one presentation rather than two
/// that drift.
pub fn admitted_venues(program: &Program, policy: &ArbPolicy) -> (Vec<String>, Vec<(String, String)>) {
    let graph = OpportunityGraph::from_program(program);
    let constraints = search_constraints(policy);
    let mut admitted: Vec<String> = Vec::new();
    let mut refused: Vec<(String, String)> = Vec::new();
    for edge in &graph.edges {
        let reason = reject_reason(edge, &constraints)
            .map(|reason| format!("{reason:?}"))
            .or_else(|| {
                let venues = vec![edge.venue.clone()];
                let assets = vec![edge.from.clone(), edge.to.clone()];
                path_reject_reason(&venues, &assets, &graph, &constraints).map(|reason| format!("{reason:?}"))
            });
        match reason {
            Some(reason) => refused.push((edge.venue.clone(), reason)),
            None => admitted.push(edge.venue.clone()),
        }
    }
    (admitted, refused)
}

/// Refuse a scope whose own bounds leave the routing stages nothing to rank.
///
/// A declaration states a search space; this runs the search's own filter over the
/// program's declared venues, so a scope this compiler accepts cannot be one the search
/// would refuse. A scope that admits nothing says "no opportunity exists" while looking
/// like a strategy, and the refusal names every declared venue and the bound that removed
/// it rather than reporting an empty result the author has to guess at.
///
/// The three answers that are not "there is something to rank" are separated, because
/// they need different fixes: no venue is declared at all (write one), no venue is on a
/// chain the scope names (widen the scope or move the venue), and venues exist on those
/// chains but every one breaks a bound (loosen the bound or deepen the venue).
fn scope_admits_a_venue(program: &Program, policy: &ArbPolicy) -> Result<Vec<String>, String> {
    let admitted = admitted_venues(program, policy);
    if !admitted.0.is_empty() {
        return Ok(admitted.0);
    }
    let graph = OpportunityGraph::from_program(program);
    let detail = if graph.edges.is_empty() {
        "no `venue` is declared, so the opportunity graph is empty and the routing stages \
         would search nothing — the phase's own pipeline starts at that graph"
            .to_string()
    } else {
        let named: Vec<String> = admitted
            .1
            .iter()
            .map(|(venue, reason)| format!("{venue} (refused: {reason})"))
            .collect();
        format!(
            "the {} declared venue(s) are refused by the scope's own bounds — {}",
            named.len(),
            named.join(", ")
        )
    };
    Err(format!(
        "the arb '{}' declares a scope whose bounds admit no venue to rank, so the search \
         would report an empty plan as a strategy: over chains [{}], with a liquidity floor \
         of {} {}, a slippage ceiling of {}bps and a fee ceiling of {}bps, {detail}",
        policy.name,
        policy.chains.join(", "),
        policy.liquidity_min.0,
        policy.liquidity_min.1,
        policy.max_slippage_bps,
        policy.max_total_fee_bps
    ))
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

/// Check every `arb` declaration in a program, in the same layer as the hedge,
/// liquidation, rebalance and netting checks — before anything is lowered.
pub fn verify(program: &Program, acc: &mut ErrorAccumulator) {
    for item in &program.items {
        let Item::Arb(decl) = &item.node else {
            continue;
        };
        match policy(decl) {
            Err(reason) => acc.add_error(semantic_error(reason)),
            Ok(decided) => {
                // Both questions are asked even when the first fails: an author fixing the
                // scope should not have to rediscover the floor question afterwards, and
                // an empty search space and an unenforced floor are different mistakes.
                if let Err(reason) = scope_admits_a_venue(program, &decided) {
                    acc.add_error(semantic_error(reason));
                }
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
        }
    }
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
