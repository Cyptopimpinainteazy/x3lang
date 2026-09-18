//! Opportunity graph: deterministic search over venues, domains, and
//! settlement paths (super-prompt phase 14).
//!
//! The graph models the economic topology a strategy can traverse: assets,
//! venues, pools, order books, chains, VMs, bridge adapters, lending markets,
//! perpetual markets, collateral relations, flash-liquidity sources, and
//! settlement paths. Edges carry the economics of moving value from one node
//! to another.
//!
//! # Determinism contract
//!
//! Consensus-affecting decisions must never depend on container iteration
//! order, wall-clock time, randomness, or floating point. This module:
//!
//! - stores nodes and edges in [`BTreeMap`]/[`BTreeSet`] keyed by ordered ids;
//! - expands candidates in ascending [`EdgeId`] order;
//! - breaks ties with an explicit, documented comparator
//!   ([`RouteRanking`]) instead of relying on discovery order;
//! - uses integer arithmetic only, with checked accumulation that reports
//!   [`GraphError::ArithmeticOverflow`] instead of wrapping.
//!
//! # v1 scope
//!
//! `search` finds a *single best linear, acyclic route* between two nodes. It
//! does not perform multi-asset netting (see the intent-fusion module) and it
//! does not solve multi-objective programs. `expected_output` on an edge is the
//! amount of output asset a venue returns for exactly `input_ref` of the input
//! asset, **already net of that venue's fees**; `fee` is the reported fee in
//! output-asset units, carried for accounting and receipts.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable, ordered identifier for a graph node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u32);

impl fmt::Display for NodeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "node#{}", self.0)
    }
}

/// Stable, ordered identifier for a graph edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EdgeId(pub u32);

impl fmt::Display for EdgeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "edge#{}", self.0)
    }
}

/// What an edge node represents economically.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum NodeKind {
    Asset,
    Venue,
    Pool,
    OrderBook,
    Chain,
    Vm,
    BridgeAdapter,
    LendingMarket,
    PerpMarket,
    CollateralRelation,
    FlashLiquiditySource,
    SettlementPath,
}

/// How an edge moves value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EdgeKind {
    Swap,
    Bridge,
    Lend,
    Borrow,
    Flash,
    Transfer,
    Settle,
}

/// Settlement strength. Ordered from weakest to strongest so that a
/// `min_finality` constraint is a simple `>=` comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum FinalityClass {
    /// Probabilistic confirmation only.
    Probabilistic,
    /// Confirmed with an explicit confirmation depth.
    Confirmed,
    /// Deterministic finality (GRANDPA-class, BFT-final, or equivalent).
    Deterministic,
}

/// What must be proven before an edge may settle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ProofRequirement {
    /// Same-domain state transition, no cross-domain proof needed.
    None,
    /// A host-supplied state commitment must be echoed by the runtime.
    HostCommitment,
    /// A light-client proof over the foreign chain's headers/roots.
    LightClient,
    /// A validity proof (Groth16/PLONK-class) over the transition.
    ValidityProof,
}

/// A graph node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    /// Economic domain, e.g. `x3`, `ethereum`, `solana`, `bitcoin`.
    pub domain: String,
    pub label: String,
}

/// The economics of traversing an edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EdgeSpec {
    pub kind: EdgeKind,
    /// Reference input amount the quote was taken at. Must be non-zero.
    pub input_ref: u128,
    /// Expected output for exactly `input_ref` input, net of venue fees.
    pub expected_output: u128,
    /// Reported fee paid to the venue, in output-asset units at `input_ref`.
    pub fee: u128,
    /// Maximum input this edge can absorb before the quote is invalid.
    pub liquidity: u128,
    /// Worst-case slippage applied as an additional haircut, in parts per
    /// million of the gross output.
    pub slippage_ppm: u32,
    pub latency_ms: u64,
    pub finality: FinalityClass,
    /// Edge-local risk in basis points.
    pub risk_bps: u16,
    pub proof_requirement: ProofRequirement,
    pub domain: String,
}

/// A stored edge with its identity and endpoints resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub from: NodeId,
    pub to: NodeId,
    pub spec: EdgeSpec,
}

/// Hard bounds a route must satisfy to be considered valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteConstraints {
    /// Maximum number of edges in the route. Must be non-zero.
    pub max_hops: usize,
    /// Every traversed edge must absorb at least this much.
    pub min_liquidity: u128,
    /// Ceiling on the route's total reported fee, converted to final-output
    /// units.
    pub max_fee: u128,
    /// Ceiling on summed edge latency.
    pub max_latency_ms: u64,
    /// Floor on every traversed edge's finality class.
    pub min_finality: FinalityClass,
    /// Ceiling on each traversed edge's risk.
    pub max_risk_bps: u16,
    /// Ceiling on the summed slippage haircut across the route.
    pub max_slippage_ppm: u32,
}

impl RouteConstraints {
    /// Permissive bounds, used when a caller only wants reachability.
    pub fn unbounded() -> Self {
        Self {
            max_hops: u16::MAX as usize,
            min_liquidity: 0,
            max_fee: u128::MAX,
            max_latency_ms: u64::MAX,
            min_finality: FinalityClass::Probabilistic,
            max_risk_bps: u16::MAX,
            max_slippage_ppm: u32::MAX,
        }
    }
}

/// One traversal step in a resolved route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteLeg {
    pub edge: EdgeId,
    pub from: NodeId,
    pub to: NodeId,
    pub kind: EdgeKind,
    pub domain: String,
    /// Amount entering this leg.
    pub input: u128,
    /// Amount leaving this leg, after the worst-case slippage haircut.
    pub output: u128,
    /// Reported venue fee for this leg, in the leg's output asset.
    pub fee: u128,
    pub latency_ms: u64,
    pub finality: FinalityClass,
    pub risk_bps: u16,
    pub proof_requirement: ProofRequirement,
}

/// A fully resolved route.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub from: NodeId,
    pub to: NodeId,
    pub legs: Vec<RouteLeg>,
    /// Amount that enters the first leg.
    pub input: u128,
    /// Amount that leaves the last leg.
    pub output: u128,
    /// Worst-case total fee expressed in final-output units.
    pub total_fee_in_output: u128,
    pub total_latency_ms: u64,
    /// Weakest finality class along the route.
    pub min_finality: FinalityClass,
    /// Highest edge-local risk along the route.
    pub max_risk_bps: u16,
    /// Sum of per-edge slippage haircuts.
    pub total_slippage_ppm: u32,
    /// Canonical, insertion-order-independent ordering key: one entry per leg
    /// built from the leg's kind, domain, and endpoint labels. Used for
    /// deterministic target tie-breaking and as a stable route identity for
    /// receipts and de-duplication.
    pub canonical_key: Vec<String>,
}

/// Ordered preference applied when several valid routes are found.
///
/// The comparator is applied in order, and the first difference decides:
///
/// 1. higher [`Route::output`];
/// 2. lower [`Route::total_fee_in_output`];
/// 3. lower [`Route::total_latency_ms`];
/// 4. fewer legs;
/// 5. lexicographically smaller [`Route::canonical_key`].
///
/// Step 5 is content-based, so two graphs that differ only in insertion order
/// still select the same winner. A final edge-id comparison breaks ties between
/// routes whose canonical keys are byte-identical, i.e. routes that are
/// economically indistinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteRanking {
    MaximizeOutputThenCheapest,
}

impl RouteRanking {
    /// Returns `Ordering::Greater` when `left` is the better route.
    pub fn compare(&self, left: &Route, right: &Route) -> std::cmp::Ordering {
        match self {
            RouteRanking::MaximizeOutputThenCheapest => left
                .output
                .cmp(&right.output)
                .then_with(|| right.total_fee_in_output.cmp(&left.total_fee_in_output))
                .then_with(|| right.total_latency_ms.cmp(&left.total_latency_ms))
                .then_with(|| right.legs.len().cmp(&left.legs.len()))
                .then_with(|| right.canonical_key.cmp(&left.canonical_key))
                .then_with(|| edge_path(right).cmp(&edge_path(left))),
        }
    }
}

impl Default for RouteRanking {
    fn default() -> Self {
        RouteRanking::MaximizeOutputThenCheapest
    }
}

fn edge_path(route: &Route) -> Vec<EdgeId> {
    route.legs.iter().map(|leg| leg.edge).collect()
}

/// Failures surfaced by graph construction and search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// An endpoint id is not present in the graph.
    UnknownNode(NodeId),
    /// A quote used a zero reference input.
    ZeroReferenceInput(EdgeId),
    /// No route satisfies the constraints.
    NoRoute { from: NodeId, to: NodeId },
    /// `max_hops` is zero, so no route can exist.
    EmptyRouteBudget,
    /// Checked arithmetic overflowed while evaluating a route.
    ArithmeticOverflow,
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownNode(id) => write!(formatter, "unknown graph node {id}"),
            Self::ZeroReferenceInput(id) => {
                write!(formatter, "edge {id} declares a zero reference input")
            }
            Self::NoRoute { from, to } => write!(formatter, "no valid route from {from} to {to}"),
            Self::EmptyRouteBudget => formatter.write_str("max_hops is zero, so no route can be constructed"),
            Self::ArithmeticOverflow => formatter.write_str("checked arithmetic overflowed"),
        }
    }
}

impl std::error::Error for GraphError {}

/// Deterministic economic topology.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpportunityGraph {
    nodes: BTreeMap<NodeId, Node>,
    edges: BTreeMap<EdgeId, Edge>,
    outgoing: BTreeMap<NodeId, BTreeSet<EdgeId>>,
    incoming: BTreeMap<NodeId, BTreeSet<EdgeId>>,
    next_node: u32,
    next_edge: u32,
}

impl OpportunityGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a node and return its generated id.
    pub fn insert_node(&mut self, kind: NodeKind, domain: impl Into<String>, label: impl Into<String>) -> NodeId {
        let id = NodeId(self.next_node);
        self.next_node += 1;
        let node = Node {
            id,
            kind,
            domain: domain.into(),
            label: label.into(),
        };
        self.nodes.insert(id, node);
        self.outgoing.entry(id).or_default();
        self.incoming.entry(id).or_default();
        id
    }

    /// Insert an edge between two existing nodes.
    pub fn insert_edge(&mut self, from: NodeId, to: NodeId, spec: EdgeSpec) -> Result<EdgeId, GraphError> {
        if !self.nodes.contains_key(&from) {
            return Err(GraphError::UnknownNode(from));
        }
        if !self.nodes.contains_key(&to) {
            return Err(GraphError::UnknownNode(to));
        }
        let id = EdgeId(self.next_edge);
        if spec.input_ref == 0 {
            return Err(GraphError::ZeroReferenceInput(id));
        }
        self.next_edge += 1;
        self.edges.insert(id, Edge { id, from, to, spec });
        self.outgoing.entry(from).or_default().insert(id);
        self.incoming.entry(to).or_default().insert(id);
        Ok(id)
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn edge(&self, id: EdgeId) -> Option<&Edge> {
        self.edges.get(&id)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn edge_count(&self) -> usize {
        self.edges.len()
    }

    /// Edges leaving `node`, in ascending id order.
    pub fn edges_from(&self, node: NodeId) -> Vec<EdgeId> {
        self.outgoing
            .get(&node)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Edges arriving at `node`, in ascending id order.
    pub fn edges_to(&self, node: NodeId) -> Vec<EdgeId> {
        self.incoming
            .get(&node)
            .map(|ids| ids.iter().copied().collect())
            .unwrap_or_default()
    }

    /// Search for the best valid route under `constraints`.
    ///
    /// Returns `Ok(None)` when no route satisfies the constraints. Malformed
    /// inputs (unknown nodes, zero hop budget) return a [`GraphError`].
    pub fn search(
        &self,
        from: NodeId,
        to: NodeId,
        amount: u128,
        constraints: &RouteConstraints,
        ranking: RouteRanking,
    ) -> Result<Option<Route>, GraphError> {
        if !self.nodes.contains_key(&from) {
            return Err(GraphError::UnknownNode(from));
        }
        if !self.nodes.contains_key(&to) {
            return Err(GraphError::UnknownNode(to));
        }
        if constraints.max_hops == 0 {
            return Err(GraphError::EmptyRouteBudget);
        }

        let mut candidates: Vec<Route> = Vec::new();
        let mut path: Vec<RouteLeg> = Vec::new();
        let mut visited: BTreeSet<NodeId> = BTreeSet::new();
        visited.insert(from);
        self.descend(from, to, amount, constraints, &mut path, &mut visited, &mut candidates)?;

        let mut best: Option<Route> = None;
        for candidate in candidates {
            best = match best {
                None => Some(candidate),
                Some(current) => {
                    if ranking.compare(&candidate, &current) == std::cmp::Ordering::Greater {
                        Some(candidate)
                    } else {
                        Some(current)
                    }
                }
            };
        }
        Ok(best)
    }

    #[allow(clippy::too_many_arguments)]
    fn descend(
        &self,
        cursor: NodeId,
        target: NodeId,
        amount: u128,
        constraints: &RouteConstraints,
        path: &mut Vec<RouteLeg>,
        visited: &mut BTreeSet<NodeId>,
        candidates: &mut Vec<Route>,
    ) -> Result<(), GraphError> {
        if cursor == target && !path.is_empty() {
            if let Some(route) = build_route(self, path, constraints)? {
                candidates.push(route);
            }
            return Ok(());
        }
        if path.len() >= constraints.max_hops {
            return Ok(());
        }

        for edge_id in self.edges_from(cursor) {
            let edge = &self.edges[&edge_id];

            if visited.contains(&edge.to) {
                continue;
            }
            if edge.spec.liquidity < constraints.min_liquidity || edge.spec.liquidity < amount || amount == 0 {
                continue;
            }
            if edge.spec.finality < constraints.min_finality {
                continue;
            }
            if edge.spec.risk_bps > constraints.max_risk_bps {
                continue;
            }
            let leg = traverse(edge, amount)?;
            if leg.output == 0 {
                continue;
            }

            // Read the leg's output before moving it into `path`, so the
            // recursive call can carry the amount without cloning the leg.
            let leg_output = leg.output;
            path.push(leg);
            visited.insert(edge.to);
            self.descend(edge.to, target, leg_output, constraints, path, visited, candidates)?;
            visited.remove(&edge.to);
            path.pop();
        }
        Ok(())
    }
}

/// Scale an edge's quote to `amount` and apply the worst-case slippage haircut.
fn traverse(edge: &Edge, amount: u128) -> Result<RouteLeg, GraphError> {
    let gross = mul_div_floor(amount, edge.spec.expected_output, edge.spec.input_ref)?;
    let fee = mul_div_floor(amount, edge.spec.fee, edge.spec.input_ref)?;
    let net_of_fee = gross.saturating_sub(fee);
    let haircut = mul_div_floor(net_of_fee, edge.spec.slippage_ppm as u128, PPM_DENOMINATOR)?;
    let output = net_of_fee.saturating_sub(haircut);
    Ok(RouteLeg {
        edge: edge.id,
        from: edge.from,
        to: edge.to,
        kind: edge.spec.kind,
        domain: edge.spec.domain.clone(),
        input: amount,
        output,
        fee,
        latency_ms: edge.spec.latency_ms,
        finality: edge.spec.finality,
        risk_bps: edge.spec.risk_bps,
        proof_requirement: edge.spec.proof_requirement,
    })
}

const PPM_DENOMINATOR: u128 = 1_000_000;

/// Floor-rounded `value * numerator / denominator` with checked widening.
fn mul_div_floor(value: u128, numerator: u128, denominator: u128) -> Result<u128, GraphError> {
    if denominator == 0 {
        return Err(GraphError::ArithmeticOverflow);
    }
    let product = value.checked_mul(numerator).ok_or(GraphError::ArithmeticOverflow)?;
    Ok(product / denominator)
}

/// Assemble the route and evaluate route-level budgets.
///
/// Returns `Ok(None)` when the path is a valid sequence of edges but violates a
/// route-level budget (total fee, latency, or slippage). Such a path is simply
/// not a candidate; it must not abort the wider search.
fn build_route(
    graph: &OpportunityGraph,
    path: &[RouteLeg],
    constraints: &RouteConstraints,
) -> Result<Option<Route>, GraphError> {
    let input = path[0].input;
    let output = path[path.len() - 1].output;

    let mut total_fee_in_output: u128 = 0;
    let mut total_latency_ms: u64 = 0;
    let mut min_finality = FinalityClass::Deterministic;
    let mut max_risk_bps: u16 = 0;
    let mut total_slippage_ppm: u32 = 0;

    for (index, leg) in path.iter().enumerate() {
        // Convert this leg's fee into final-output units by walking the
        // remaining legs' realized conversion ratios, floor-rounded at every
        // step so the figure is a deterministic worst case.
        let mut converted = leg.fee;
        for downstream in &path[index + 1..] {
            if downstream.input == 0 {
                return Err(GraphError::ArithmeticOverflow);
            }
            converted = mul_div_floor(converted, downstream.output, downstream.input)?;
        }
        total_fee_in_output = total_fee_in_output
            .checked_add(converted)
            .ok_or(GraphError::ArithmeticOverflow)?;

        total_latency_ms = total_latency_ms
            .checked_add(leg.latency_ms)
            .ok_or(GraphError::ArithmeticOverflow)?;

        let edge = graph.edge(leg.edge).ok_or(GraphError::UnknownNode(leg.from))?;
        total_slippage_ppm = total_slippage_ppm
            .checked_add(edge.spec.slippage_ppm)
            .ok_or(GraphError::ArithmeticOverflow)?;

        min_finality = min_finality.min(leg.finality);
        max_risk_bps = max_risk_bps.max(leg.risk_bps);
    }

    if total_fee_in_output > constraints.max_fee
        || total_latency_ms > constraints.max_latency_ms
        || total_slippage_ppm > constraints.max_slippage_ppm
    {
        return Ok(None);
    }

    Ok(Some(Route {
        from: path[0].from,
        to: path[path.len() - 1].to,
        legs: path.to_vec(),
        input,
        output,
        total_fee_in_output,
        total_latency_ms,
        min_finality,
        max_risk_bps,
        total_slippage_ppm,
        canonical_key: canonical_key(graph, path),
    }))
}

fn canonical_key(graph: &OpportunityGraph, path: &[RouteLeg]) -> Vec<String> {
    let mut key = Vec::with_capacity(path.len());
    for leg in path {
        let from = graph.node(leg.from).map(|node| node.label.as_str()).unwrap_or("?");
        let to = graph.node(leg.to).map(|node| node.label.as_str()).unwrap_or("?");
        key.push(format!("{:?}:{}:{from}->{to}", leg.kind, leg.domain));
    }
    key
}
