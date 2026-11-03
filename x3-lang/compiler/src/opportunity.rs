//! The opportunity graph — spec build-order item 16, PHASE 14.
//!
//! A graph over the venues a program declares: assets are nodes, and each venue
//! contributes an edge from the asset it takes in to the asset it gives out,
//! carrying the attributes the spec lists (expected output, fees, liquidity,
//! slippage, latency, finality, risk, domain, proof requirement). The planner
//! searches it for valid opportunities.
//!
//! Two properties this module is built around, because they are the ones that
//! make a graph search trustworthy rather than plausible:
//!
//! - **Deterministic.** Every ordering is explicit, ties are broken by a total
//!   order over the path itself, and nothing iterates a `HashMap`. Two runs over
//!   the same program produce the same ranking, and so does a run on a machine
//!   with a different hash seed.
//! - **Bounded.** The search is hop-bounded and chain-bounded, and never
//!   revisits an asset in a path, so a cyclic graph cannot produce an unbounded
//!   or infinite family of opportunities. An unbounded search is not a planner,
//!   it is a hang.
//!
//! What it deliberately does *not* do: model price impact as a function. A
//! venue declares a slippage bound at its declared liquidity, and the search
//! checks a candidate size against that bound. A function would be more
//! expressive and would also be a promise nothing in the language can check —
//! see `EdgeAttributes::slippage_bps`.

use serde::{Deserialize, Serialize};

use x3_lang_ast::ast::{AssetRef, Item, Program, VenueDecl, VenueKind};

/// Maximum hops a searched path may use.
pub const DEFAULT_MAX_PATH_HOPS: usize = 4;

/// Maximum edge examinations one search may perform.
///
/// PHASE 42 requires a *bounded* search, and the hop bound alone does not give
/// one: a graph with many edges per node still explodes. The budget makes the
/// cost of a search a property of the caller's declaration rather than of
/// whatever graph it is pointed at.
pub const DEFAULT_MAX_EXPANSIONS: usize = 4096;

/// The result of a search.
///
/// Exhausting the budget is a distinct outcome rather than an empty result.
/// Returning "no opportunities" for "I stopped looking" is the quiet wrong
/// answer this type exists to make impossible: a caller that cannot tell the
/// two apart will report an unreachable route for a route it never finished
/// considering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchOutcome {
    Found(Vec<Opportunity>),
    BudgetExhausted { examined: usize, budget: usize },
}

impl SearchOutcome {
    /// The opportunities found, if the search finished.
    pub fn found(&self) -> Option<&[Opportunity]> {
        match self {
            SearchOutcome::Found(found) => Some(found),
            SearchOutcome::BudgetExhausted { .. } => None,
        }
    }
}

/// A node in the graph. Assets and venues are both nodes; the edge carries the
/// relationship, which is why an asset's identity and a venue's identity cannot
/// be confused for one another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphNode {
    /// `chain.ASSET` — the unit that moves.
    Asset { chain: String, asset: String },
    /// A declared venue.
    Venue {
        name: String,
        kind: VenueKind,
        chain: String,
        domain: String,
    },
}

impl GraphNode {
    /// A stable label, used for ordering and for diagnostics.
    pub fn label(&self) -> String {
        match self {
            GraphNode::Asset { chain, asset } => format!("{chain}.{asset}"),
            GraphNode::Venue { name, .. } => name.clone(),
        }
    }
}

/// The attributes an edge carries, exactly the spec's list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeAttributes {
    /// Venue fee in basis points.
    pub fee_bps: u32,
    /// Declared depth, in units of the input asset.
    pub liquidity: u128,
    /// Slippage in basis points at the declared liquidity.
    pub slippage_bps: u32,
    /// Expected latency to settlement, in milliseconds.
    pub latency_ms: u32,
    /// Blocks of finality the settlement requires.
    pub finality_blocks: u32,
    /// Declared risk, 0 (safest) to 100.
    pub risk: u32,
    /// VM family hosting the venue.
    pub domain: String,
    /// Proof a claim against this venue must carry.
    pub proof: Option<String>,
}

/// A traversable edge: `from` asset → `to` asset through one venue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
    pub venue: String,
    pub attributes: EdgeAttributes,
}

/// The graph itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpportunityGraph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
}

impl OpportunityGraph {
    /// Build the graph from a program's `venue` declarations.
    ///
    /// Declaration order is preserved in `nodes`, and edges are emitted in
    /// declaration order, so a graph built from the same source is identical
    /// byte for byte.
    pub fn from_program(program: &Program) -> Self {
        let mut graph = OpportunityGraph::default();
        for item in &program.items {
            let Item::VenueDecl(venue) = &item.node else {
                continue;
            };
            graph.push_venue(venue);
        }
        graph
    }

    fn push_venue(&mut self, venue: &VenueDecl) {
        let from = asset_label(&venue.asset_in);
        let to = asset_label(&venue.asset_out);
        self.nodes.push(GraphNode::Asset {
            chain: venue.asset_in.chain.as_str().to_string(),
            asset: venue.asset_in.name.as_str().to_string(),
        });
        self.nodes.push(GraphNode::Asset {
            chain: venue.asset_out.chain.as_str().to_string(),
            asset: venue.asset_out.name.as_str().to_string(),
        });
        self.nodes.push(GraphNode::Venue {
            name: venue.name.as_str().to_string(),
            kind: venue.kind,
            chain: venue.chain.as_str().to_string(),
            domain: venue.domain.as_str().to_string(),
        });
        self.edges.push(GraphEdge {
            from,
            to,
            venue: venue.name.as_str().to_string(),
            attributes: EdgeAttributes {
                fee_bps: venue.fee_bps,
                liquidity: venue.liquidity,
                slippage_bps: venue.slippage_bps,
                latency_ms: venue.latency_ms,
                finality_blocks: venue.finality_blocks,
                risk: venue.risk,
                domain: venue.domain.as_str().to_string(),
                proof: venue.proof.as_ref().map(|proof| proof.as_str().to_string()),
            },
        });
    }

    /// Look up one declared venue by name.
    pub fn venue(&self, name: &str) -> Option<&GraphEdge> {
        self.edges.iter().find(|edge| edge.venue == name)
    }

    /// Every edge leaving an asset, in declaration order.
    pub fn edges_from(&self, asset: &str) -> Vec<&GraphEdge> {
        self.edges.iter().filter(|edge| edge.from == asset).collect()
    }
}

/// What a caller will accept from an opportunity.
///
/// Every field is a hard bound rather than a preference: a path that violates
/// one is not ranked lower, it is not an opportunity. Ranking happens only
/// among the paths that satisfy all of them.
#[derive(Debug, Clone, Default)]
pub struct OpportunityConstraints {
    /// Maximum hops.
    pub max_hops: usize,
    /// Largest number of distinct chains the path may touch.
    ///
    /// A path's chains are counted, not its assets: two venues on the same
    /// chain are one chain's worth of exposure no matter how many assets they
    /// move between.
    pub max_chains: Option<u32>,
    /// Largest fee sum the path may incur, in basis points.
    pub max_fee_bps: Option<u32>,
    /// Largest slippage any single venue on the path may declare.
    pub max_slippage_bps: Option<u32>,
    /// Smallest trade size the path must be able to absorb. Checked against
    /// each venue's declared liquidity, because a venue that cannot absorb the
    /// size is not a route for it.
    pub min_liquidity: Option<u128>,
    /// Largest total latency the path may incur, in milliseconds.
    pub max_latency_ms: Option<u32>,
    /// Largest finality the path may require, in blocks.
    pub max_finality_blocks: Option<u32>,
    /// Largest risk any single venue on the path may declare.
    pub max_risk: Option<u32>,
    /// Require every venue on the path to declare a proof.
    pub require_proof: bool,
}

/// One verified opportunity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Opportunity {
    /// Venues in the order they would be used.
    pub venues: Vec<String>,
    /// Assets along the way, starting at the input asset and ending at the
    /// output asset.
    pub assets: Vec<String>,
    /// Sum of the venues' fees, in basis points.
    pub fee_bps: u32,
    /// Worst (largest) slippage bound any venue on the path declares.
    pub slippage_bps: u32,
    /// Largest risk any venue on the path declares.
    pub max_risk: u32,
    /// Sum of the venues' latencies, in milliseconds.
    pub latency_ms: u32,
    /// Finality the path requires: the largest any venue on it needs, because a
    /// path is not settled until its slowest leg is.
    pub finality_blocks: u32,
    /// Smallest declared liquidity on the path, in input-asset units.
    pub min_liquidity: u128,
}

impl Opportunity {
    /// A total order for ranking, most attractive first.
    ///
    /// Fee, then slippage, then risk, then latency, then finality, then the
    /// venue list itself as the final tie-break. The last term matters: without
    /// it two paths that agree on every attribute would be ordered by whatever
    /// order the search happened to produce, which is the kind of thing that
    /// makes a planner's output unreproducible.
    pub fn rank_key(&self) -> (u32, u32, u32, u32, u32, &[String]) {
        (
            self.fee_bps,
            self.slippage_bps,
            self.max_risk,
            self.latency_ms,
            self.finality_blocks,
            &self.venues,
        )
    }
}

/// Why a path was refused, kept as data so a caller can report it rather than
/// guess from an empty result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectionReason {
    TooManyHops,
    TooManyChains,
    FeeAboveBound,
    SlippageAboveBound,
    LiquidityBelowBound,
    LatencyAboveBound,
    FinalityAboveBound,
    RiskAboveBound,
    MissingProof,
}

/// The hop ceiling a search uses: the declared one, or the default when the
/// declaration is zero (which is what "nothing declared" looks like in a
/// struct that is `Default`).
///
/// One function because two readers have to agree: the search prunes with it,
/// and [`path_reject_reason`] explains a path with it.
fn effective_max_hops(constraints: &OpportunityConstraints) -> usize {
    if constraints.max_hops == 0 {
        DEFAULT_MAX_PATH_HOPS
    } else {
        constraints.max_hops
    }
}

/// Search the graph for valid opportunities from one asset to another.
///
/// Results are ranked by [`Opportunity::rank_key`], most attractive first. The
/// search is depth-first with an explicit hop budget and a visited set, so it
/// terminates on any graph, including one with cycles.
pub fn search(graph: &OpportunityGraph, from: &str, to: &str, constraints: &OpportunityConstraints) -> SearchOutcome {
    search_with_budget(graph, from, to, constraints, DEFAULT_MAX_EXPANSIONS)
}

/// The same search with an explicit expansion budget.
pub fn search_with_budget(
    graph: &OpportunityGraph,
    from: &str,
    to: &str,
    constraints: &OpportunityConstraints,
    max_expansions: usize,
) -> SearchOutcome {
    let max_hops = effective_max_hops(constraints);

    let mut found: Vec<Opportunity> = Vec::new();
    let mut venues: Vec<String> = Vec::new();
    let mut assets: Vec<String> = vec![from.to_string()];
    let mut visited: Vec<String> = vec![from.to_string()];
    let mut examined = 0usize;
    let exhausted = walk(
        graph,
        from,
        to,
        constraints,
        max_hops,
        max_expansions,
        &mut examined,
        &mut venues,
        &mut assets,
        &mut visited,
        &mut found,
    );
    if exhausted {
        return SearchOutcome::BudgetExhausted {
            examined,
            budget: max_expansions,
        };
    }
    found.sort_by(|left, right| left.rank_key().cmp(&right.rank_key()));
    SearchOutcome::Found(found)
}

/// Depth-first walk. Returns `true` when the budget ran out.
#[allow(clippy::too_many_arguments)]
fn walk(
    graph: &OpportunityGraph,
    current: &str,
    target: &str,
    constraints: &OpportunityConstraints,
    max_hops: usize,
    budget: usize,
    examined: &mut usize,
    venues: &mut Vec<String>,
    assets: &mut Vec<String>,
    visited: &mut Vec<String>,
    found: &mut Vec<Opportunity>,
) -> bool {
    if venues.len() >= max_hops {
        return false;
    }
    for edge in graph.edges_from(current) {
        *examined += 1;
        if *examined > budget {
            return true;
        }
        // A path may not revisit an asset. Without this a cycle is an infinite
        // family of paths that differ only in how many times they go round.
        if visited.contains(&edge.to) {
            continue;
        }
        if !edge_is_within_bounds(edge, constraints) {
            continue;
        }
        venues.push(edge.venue.clone());
        assets.push(edge.to.clone());
        visited.push(edge.to.clone());

        // Constraints that only a whole path can answer are checked on the
        // extended path, so a path that is over its budget is never extended
        // further and never reported as an opportunity.
        if path_reject_reason(venues, assets, graph, constraints).is_none() {
            if edge.to == target {
                found.push(summarize(venues, assets, graph));
            } else if walk(
                graph,
                &edge.to,
                target,
                constraints,
                max_hops,
                budget,
                examined,
                venues,
                assets,
                visited,
                found,
            ) {
                venues.pop();
                assets.pop();
                visited.pop();
                return true;
            }
        }

        venues.pop();
        assets.pop();
        visited.pop();
    }
    false
}

/// Why an edge cannot be used under these constraints, or `None` if it can.
///
/// The search and any diagnostic a caller builds read this one function, so a
/// CLI cannot explain a path differently from the way the planner rejected it.
pub fn reject_reason(edge: &GraphEdge, constraints: &OpportunityConstraints) -> Option<RejectionReason> {
    let attributes = &edge.attributes;
    if let Some(bound) = constraints.max_slippage_bps {
        if attributes.slippage_bps > bound {
            return Some(RejectionReason::SlippageAboveBound);
        }
    }
    if let Some(bound) = constraints.min_liquidity {
        if attributes.liquidity < bound {
            return Some(RejectionReason::LiquidityBelowBound);
        }
    }
    if let Some(bound) = constraints.max_latency_ms {
        if attributes.latency_ms > bound {
            return Some(RejectionReason::LatencyAboveBound);
        }
    }
    if let Some(bound) = constraints.max_finality_blocks {
        if attributes.finality_blocks > bound {
            return Some(RejectionReason::FinalityAboveBound);
        }
    }
    if let Some(bound) = constraints.max_risk {
        if attributes.risk > bound {
            return Some(RejectionReason::RiskAboveBound);
        }
    }
    if constraints.require_proof && attributes.proof.is_none() {
        return Some(RejectionReason::MissingProof);
    }
    None
}

fn edge_is_within_bounds(edge: &GraphEdge, constraints: &OpportunityConstraints) -> bool {
    reject_reason(edge, constraints).is_none()
}

/// Why a path as a whole cannot be used, or `None` if it can.
///
/// [`reject_reason`] answers for a single edge. A constraint that is a property
/// of the path rather than of any edge on it cannot be answered there, so it is
/// answered here — and the search asks this of every path it extends, so the
/// budget that refuses a path and the diagnostic that explains the refusal are
/// the same code.
pub fn path_reject_reason(
    venues: &[String],
    assets: &[String],
    graph: &OpportunityGraph,
    constraints: &OpportunityConstraints,
) -> Option<RejectionReason> {
    // A path's first asset is where it starts, so its hop count is one less
    // than its asset count: every other asset is an edge that was taken.
    if assets.len().saturating_sub(1) > effective_max_hops(constraints) {
        return Some(RejectionReason::TooManyHops);
    }
    if let Some(bound) = constraints.max_fee_bps {
        if path_fee_bps(venues, graph) > bound {
            return Some(RejectionReason::FeeAboveBound);
        }
    }
    if let Some(bound) = constraints.max_chains {
        if distinct_chains(assets, graph).len() > bound as usize {
            return Some(RejectionReason::TooManyChains);
        }
    }
    None
}

/// What a path costs in fees: the sum of its venues', in basis points.
///
/// A total is a property of the path, not of any edge, which is why it is here
/// rather than in [`reject_reason`]: a bound on the total cannot be checked one
/// venue at a time.
pub(crate) fn path_fee_bps(venues: &[String], graph: &OpportunityGraph) -> u32 {
    venues
        .iter()
        .filter_map(|venue| graph.venue(venue))
        .fold(0u32, |sum, edge| sum.saturating_add(edge.attributes.fee_bps))
}

/// The distinct chains a path touches, in the order it first touches them.
fn distinct_chains(assets: &[String], graph: &OpportunityGraph) -> Vec<String> {
    let mut chains: Vec<String> = Vec::new();
    for asset in assets {
        let chain = chain_of(graph, asset);
        if !chain.is_empty() && !chains.contains(&chain) {
            chains.push(chain);
        }
    }
    chains
}

/// The chain an `chain.ASSET` node label belongs to.
///
/// Read from the graph's own asset nodes rather than by splitting the label, so
/// an asset whose name contains a dot cannot be read as a chain boundary. A
/// label with no node behind it — the caller's own `from`, when it names an
/// asset no venue touches — falls back to its prefix, which is all there is to
/// go on.
fn chain_of(graph: &OpportunityGraph, asset: &str) -> String {
    graph
        .nodes
        .iter()
        .find_map(|node| match node {
            GraphNode::Asset { chain, asset: name } if format!("{chain}.{name}") == *asset => Some(chain.clone()),
            _ => None,
        })
        .or_else(|| asset.split('.').next().map(str::to_string))
        .unwrap_or_default()
}

fn summarize(venues: &[String], assets: &[String], graph: &OpportunityGraph) -> Opportunity {
    let mut slippage_bps = 0u32;
    let mut max_risk = 0u32;
    let mut latency_ms = 0u32;
    let mut finality_blocks = 0u32;
    let mut min_liquidity = u128::MAX;
    for venue in venues {
        if let Some(edge) = graph.venue(venue) {
            slippage_bps = slippage_bps.max(edge.attributes.slippage_bps);
            max_risk = max_risk.max(edge.attributes.risk);
            latency_ms = latency_ms.saturating_add(edge.attributes.latency_ms);
            finality_blocks = finality_blocks.max(edge.attributes.finality_blocks);
            min_liquidity = min_liquidity.min(edge.attributes.liquidity);
        }
    }
    Opportunity {
        venues: venues.to_vec(),
        assets: assets.to_vec(),
        fee_bps: path_fee_bps(venues, graph),
        slippage_bps,
        max_risk,
        latency_ms,
        finality_blocks,
        min_liquidity: if min_liquidity == u128::MAX { 0 } else { min_liquidity },
    }
}

/// The `chain.ASSET` label an asset reference denotes.
pub fn asset_label(asset: &AssetRef) -> String {
    format!("{}.{}", asset.chain.as_str(), asset.name.as_str())
}
