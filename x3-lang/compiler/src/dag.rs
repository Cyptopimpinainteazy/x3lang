//! The parallel dependency DAG — spec build-order item 18, PHASE 16.
//!
//! A `parallel` block declares legs that are *allowed* to run concurrently. The
//! compiler's job is to decide which of them actually can, and PHASE 16 is
//! specific about the two things that decision has to get right:
//!
//! - **parallelism must be deterministic** — the plan is a function of the
//!   program. Waves are produced by Kahn's algorithm with a lexicographic
//!   tie-break on leg names, so the same legs always produce the same plan, and
//!   nothing depends on the order the legs were declared in or on a hash.
//! - **race conditions must be rejected or resolved by explicit semantics** —
//!   the only resolution this module accepts is a *data dependency*, which is
//!   explicit in the program: if one leg produces an asset another consumes,
//!   there is an edge, and the plan orders them. Two legs that both *produce*
//!   the same asset have no such resolution — nothing in the program says which
//!   one wins — so that is rejected rather than ordered arbitrarily.
//!
//! The dependency is derived from the lowered IR, not from the source text: a
//! leg's asset reads and writes are whatever its operations actually do. A
//! source-level guess would be a second, weaker opinion about the same question.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::ir::Operation;

/// Maximum legs a `parallel` block may declare.
///
/// A plan is a thing a reader has to be able to check; past a point the bound is
/// what keeps "parallel execution" from meaning "an arbitrary graph".
pub const MAX_PARALLEL_LEGS: usize = 8;

/// One leg of a `parallel` block, with the assets its operations touch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Leg {
    pub name: String,
    /// Assets the leg consumes.
    pub reads: BTreeSet<String>,
    /// Assets the leg produces. Two legs producing the same asset is a race.
    pub writes: BTreeSet<String>,
    /// Chains the leg's operations touch.
    pub chains: BTreeSet<String>,
    /// Cross-chain steps the leg contains, as `(from_chain, to_chain)`.
    pub bridges: BTreeSet<(String, String)>,
    /// Proof inputs this leg's bridges need and do not carry.
    ///
    /// A `Bridge` operation takes a source-finality proof and a transfer proof
    /// as inputs; when a program writes the bridge without them, the obligation
    /// is outstanding. It is recorded rather than assumed settled, because a
    /// coordinator cannot treat a wave as final on a proof nobody produced.
    pub outstanding_proofs: BTreeSet<String>,
}

/// Why a `parallel` block cannot be scheduled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RaceError {
    /// Two legs both produce the same asset.
    WriteWrite {
        asset: String,
        first: String,
        second: String,
    },
    /// A leg depends on itself, directly or through the graph.
    Cycle { legs: Vec<String> },
    /// Fewer than two legs: a "parallel" block of one is not parallel, and
    /// accepting it would make the construct's name a lie in the artifact.
    TooFewLegs { legs: usize },
    /// Above the production bound.
    TooManyLegs { legs: usize, bound: usize },
    /// Two legs with the same name.
    DuplicateLeg { name: String },
    /// A leg touches more chains than it has cross-chain steps for.
    ImplicitCrossChain {
        leg: String,
        chains: Vec<String>,
        bridges: usize,
    },
}

/// A deterministic execution plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParallelPlan {
    /// Legs that must run in order; an edge `a -> b` means `a` produces an asset
    /// `b` consumes.
    pub edges: Vec<(String, String)>,
    /// Groups of legs that can run concurrently. Within a wave the legs are
    /// sorted, and every leg in wave `n` depends only on legs in waves `< n`.
    pub waves: Vec<Vec<String>>,
    /// What a coordinator owes before each wave may be treated as settled.
    pub settlement: Vec<WaveSettlement>,
    /// The execution domain of each leg, in wave order.
    ///
    /// A domain is the VM family a chain runs on, taken from the program's own
    /// declarations. A chain nothing declares is its own domain: the compiler
    /// can only honestly say "this is chain X" when the program never said two
    /// chains share a VM. The set of these values is the answer to "is this plan
    /// multi-VM", which is what makes the plan a *multi-VM* plan rather than a
    /// list of legs.
    pub domains: BTreeMap<String, BTreeSet<String>>,
}

impl ParallelPlan {
    /// The legs that run concurrently with `leg`, for a reader that wants to
    /// know what "parallel" bought.
    pub fn concurrent_with(&self, leg: &str) -> Vec<String> {
        self.waves
            .iter()
            .find(|wave| wave.iter().any(|candidate| candidate == leg))
            .map(|wave| {
                wave.iter()
                    .filter(|candidate| candidate.as_str() != leg)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A coordinator's view of one wave.
///
/// The waves say what may run concurrently; this says what has to be *true*
/// afterwards, which is the part a coordinator acts on. Two facts are derivable
/// and both matter:
///
/// - **`outstanding_proofs`** — the proof inputs the wave's bridges need and do
///   not carry. A coordinator cannot treat a cross-domain effect as final on a
///   proof nobody produced.
/// - **`locally_recoverable`** — whether the VM alone can undo the wave. A wave
///   confined to one domain can be rolled back by the local atomic scope; a wave
///   spanning domains cannot, because its effects on the other domain are
///   already out of this VM's hands. Saying so is the difference between a plan
///   and a hope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaveSettlement {
    /// Wave index, matching `ParallelPlan::waves`.
    pub wave: usize,
    /// VM families the wave touches.
    pub domains: BTreeSet<String>,
    /// Proof inputs the wave's bridges need and do not carry.
    pub outstanding_proofs: BTreeSet<String>,
    /// Whether the VM's own rollback can undo the wave without a counterparty.
    pub locally_recoverable: bool,
}

/// The assets an operation consumes and produces.
///
/// A `Swap` reads its input asset and writes its output asset; a `Lock`,
/// `Mint`, `Burn` or `Release` writes the asset it names. Treating a lock as a
/// write is deliberate: it changes who holds the asset, which is exactly the
/// state two concurrent legs must not both be changing.
fn touched(operation: &Operation) -> (Vec<String>, Vec<String>) {
    match operation {
        Operation::Lock { chain, asset, .. }
        | Operation::Mint { chain, asset, .. }
        | Operation::Burn { chain, asset, .. }
        | Operation::Release { chain, asset, .. } => (Vec::new(), vec![key(chain, asset)]),
        Operation::Swap {
            from_chain,
            from_asset,
            to_chain,
            to_asset,
            ..
        } => (vec![key(from_chain, from_asset)], vec![key(to_chain, to_asset)]),
        Operation::Bridge {
            from_chain,
            from_asset,
            to_chain,
            to_asset,
            ..
        } => (vec![key(from_chain, from_asset)], vec![key(to_chain, to_asset)]),
        _ => (Vec::new(), Vec::new()),
    }
}

fn key(chain: &str, asset: &str) -> String {
    format!("{chain}.{asset}")
}

/// Build one leg from its lowered operations.
///
/// Refuses a leg that touches more chains than it has cross-chain steps for.
/// Moving value between N chains takes at least N-1 steps that say so; a leg
/// that names several chains and fewer bridges is moving value across a domain
/// boundary the program never drew, which is exactly what "multi-VM planning"
/// has to refuse rather than infer.
pub fn leg_from_operations(name: &str, operations: &[Operation]) -> Result<Leg, RaceError> {
    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();
    let mut chains = BTreeSet::new();
    let mut bridges = BTreeSet::new();
    let mut outstanding = BTreeSet::new();
    for operation in operations {
        let (operation_reads, operation_writes) = touched(operation);
        reads.extend(operation_reads);
        writes.extend(operation_writes);
        for chain in chains_of(operation) {
            chains.insert(chain);
        }
        if let Operation::Bridge {
            from_chain,
            to_chain,
            source_finality_proof,
            transfer_proof,
            ..
        } = operation
        {
            bridges.insert((from_chain.clone(), to_chain.clone()));
            // Name the obligation with the chain it is owed on, so a coordinator
            // can act on it without re-deriving which bridge asked.
            if source_finality_proof.is_empty() {
                outstanding.insert(format!("{from_chain}:source_finality_proof"));
            }
            if transfer_proof.is_empty() {
                outstanding.insert(format!("{to_chain}:transfer_proof"));
            }
        }
    }
    // Note on why there is no "an asset this leg both reads and writes is
    // internal, so drop the read" step here. There was one, and it was wrong in
    // the direction that matters. A leg that bridges another leg's output also
    // carries a refund path on that asset, and the refund path lowers to a
    // `Release` on it — a write. Pruning the read against that write erased the
    // leg's real dependency on its producer, so the two legs looked unordered
    // and an otherwise correct plan was refused as a race.
    //
    // Keeping both sets means a leg that writes what another produces gets
    // *both* the dependency edge and the write-write overlap; the overlap check
    // then sees the ordering and stays quiet. The cost is the other direction: a
    // leg that locks an asset it also consumes now depends on whoever produces
    // it, which serialises two legs that might have run together. Ordering too
    // much is a lost opportunity; ordering too little is a race.

    if chains.len() > 1 && bridges.len() < chains.len() - 1 {
        return Err(RaceError::ImplicitCrossChain {
            leg: name.to_string(),
            chains: chains.iter().cloned().collect(),
            bridges: bridges.len(),
        });
    }

    Ok(Leg {
        name: name.to_string(),
        reads,
        writes,
        chains,
        bridges,
        outstanding_proofs: outstanding,
    })
}

/// The chains an operation names.
fn chains_of(operation: &Operation) -> Vec<String> {
    match operation {
        Operation::Lock { chain, .. }
        | Operation::Mint { chain, .. }
        | Operation::Burn { chain, .. }
        | Operation::Release { chain, .. } => vec![chain.clone()],
        Operation::Swap {
            from_chain, to_chain, ..
        } => vec![from_chain.clone(), to_chain.clone()],
        Operation::Bridge {
            from_chain, to_chain, ..
        } => vec![from_chain.clone(), to_chain.clone()],
        _ => Vec::new(),
    }
}

/// Build the DAG and its waves, or say why it cannot be built.
///
/// `BTreeSet`/`BTreeMap` throughout: the ordering of the output must not depend
/// on a hash seed, and PHASE 42 names unordered maps as the first thing a
/// consensus-affecting decision may not use.
pub fn plan(legs: &[Leg], chain_domains: &BTreeMap<String, String>) -> Result<ParallelPlan, RaceError> {
    if legs.len() < 2 {
        return Err(RaceError::TooFewLegs { legs: legs.len() });
    }
    if legs.len() > MAX_PARALLEL_LEGS {
        return Err(RaceError::TooManyLegs {
            legs: legs.len(),
            bound: MAX_PARALLEL_LEGS,
        });
    }
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for leg in legs {
        if !seen.insert(&leg.name) {
            return Err(RaceError::DuplicateLeg { name: leg.name.clone() });
        }
    }

    // A producer must run before its consumers. This is the "explicit
    // semantics" half of the race requirement: the program said one leg feeds
    // another, so the order is derived rather than guessed.
    let mut edges: BTreeSet<(String, String)> = BTreeSet::new();
    for producer in legs {
        for consumer in legs {
            if producer.name == consumer.name {
                continue;
            }
            if producer.writes.intersection(&consumer.reads).next().is_some() {
                edges.insert((producer.name.clone(), consumer.name.clone()));
            }
        }
    }

    // A race is two legs writing the same asset *with nothing ordering them*.
    // The ordering has to be checked before the conflict is called a race:
    // `alpha` producing ETH and a later leg locking that ETH both write it, and
    // the dependency between them is exactly what makes that safe. An earlier
    // version of this function compared writes across the whole block and
    // refused that program, which is the same mistake as sequencing two legs
    // silently — it ignored the ordering the program had already expressed.
    for (index, first) in legs.iter().enumerate() {
        for second in legs.iter().skip(index + 1) {
            let shared: Vec<&String> = first.writes.intersection(&second.writes).collect();
            if shared.is_empty() {
                continue;
            }
            if ordered(&edges, &first.name, &second.name) || ordered(&edges, &second.name, &first.name) {
                continue;
            }
            let asset = shared[0].clone();
            return Err(RaceError::WriteWrite {
                asset,
                first: first.name.clone(),
                second: second.name.clone(),
            });
        }
    }

    let waves = waves(legs, &edges)?;
    let mut settlement: Vec<WaveSettlement> = Vec::new();
    for (index, wave) in waves.iter().enumerate() {
        let mut wave_domains: BTreeSet<String> = BTreeSet::new();
        let mut wave_proofs: BTreeSet<String> = BTreeSet::new();
        for name in wave {
            if let Some(leg) = legs.iter().find(|leg| &leg.name == name) {
                for chain in &leg.chains {
                    wave_domains.insert(chain_domains.get(chain).cloned().unwrap_or_else(|| chain.clone()));
                }
                wave_proofs.extend(leg.outstanding_proofs.iter().cloned());
            }
        }
        settlement.push(WaveSettlement {
            wave: index,
            domains: wave_domains.clone(),
            outstanding_proofs: wave_proofs,
            // One domain: the VM's atomic scope is the whole story. More than
            // one: part of the wave is beyond this VM's reach.
            locally_recoverable: wave_domains.len() <= 1,
        });
    }

    let mut domains: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for leg in legs {
        let leg_domains: BTreeSet<String> = leg
            .chains
            .iter()
            .map(|chain| chain_domains.get(chain).cloned().unwrap_or_else(|| chain.clone()))
            .collect();
        domains.insert(leg.name.clone(), leg_domains);
    }
    Ok(ParallelPlan {
        edges: edges.into_iter().collect(),
        waves,
        settlement,
        domains,
    })
}

/// The domains a plan spans, sorted. One entry means the plan is single-VM.
pub fn domains_spanned(plan: &ParallelPlan) -> BTreeSet<String> {
    plan.domains.values().flatten().cloned().collect()
}

/// Whether `from` reaches `to` along the dependency edges, so the two legs have
/// an order the program expressed.
fn ordered(edges: &BTreeSet<(String, String)>, from: &str, to: &str) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut frontier: Vec<&str> = vec![from];
    while let Some(current) = frontier.pop() {
        if current == to {
            return true;
        }
        if !seen.insert(current) {
            continue;
        }
        for (edge_from, edge_to) in edges {
            if edge_from == current {
                frontier.push(edge_to);
            }
        }
    }
    false
}

/// Kahn's algorithm with a lexicographic tie-break, so the wave assignment is
/// unique for a given graph.
fn waves(legs: &[Leg], edges: &BTreeSet<(String, String)>) -> Result<Vec<Vec<String>>, RaceError> {
    let mut remaining: BTreeSet<String> = legs.iter().map(|leg| leg.name.clone()).collect();
    let mut waves: Vec<Vec<String>> = Vec::new();
    let mut scheduled: BTreeSet<String> = BTreeSet::new();

    while !remaining.is_empty() {
        let ready: Vec<String> = remaining
            .iter()
            .filter(|candidate| {
                !edges
                    .iter()
                    .any(|(from, to)| to == *candidate && !scheduled.contains(from))
            })
            .cloned()
            .collect();
        if ready.is_empty() {
            // Every remaining leg waits on another remaining leg: a cycle. There
            // is no order to execute in, and guessing one would be the silent
            // wrong answer.
            return Err(RaceError::Cycle {
                legs: remaining.into_iter().collect(),
            });
        }
        for leg in &ready {
            remaining.remove(leg);
        }
        scheduled.extend(ready.iter().cloned());
        waves.push(ready);
    }
    Ok(waves)
}
