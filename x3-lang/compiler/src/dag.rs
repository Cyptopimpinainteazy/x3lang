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
            to_asset,
            ..
        } => (vec![key(from_chain, from_asset)], vec![key(from_chain, to_asset)]),
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
pub fn leg_from_operations(name: &str, operations: &[Operation]) -> Leg {
    let mut reads = BTreeSet::new();
    let mut writes = BTreeSet::new();
    for operation in operations {
        let (operation_reads, operation_writes) = touched(operation);
        reads.extend(operation_reads);
        writes.extend(operation_writes);
    }
    // An asset a leg both produces and consumes is its own business: the read is
    // satisfied by the write in the same leg, so it is not a dependency on
    // another leg.
    for produced in &writes {
        reads.remove(produced);
    }
    Leg {
        name: name.to_string(),
        reads,
        writes,
    }
}

/// Build the DAG and its waves, or say why it cannot be built.
///
/// `BTreeSet`/`BTreeMap` throughout: the ordering of the output must not depend
/// on a hash seed, and PHASE 42 names unordered maps as the first thing a
/// consensus-affecting decision may not use.
pub fn plan(legs: &[Leg]) -> Result<ParallelPlan, RaceError> {
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

    // A race is two legs producing the same asset. There is no edge that
    // resolves it: whichever ran first, the other's write is against a state the
    // program never described.
    let mut producers: BTreeMap<&str, &str> = BTreeMap::new();
    for leg in legs {
        for asset in &leg.writes {
            if let Some(first) = producers.get(asset.as_str()) {
                return Err(RaceError::WriteWrite {
                    asset: asset.clone(),
                    first: (*first).to_string(),
                    second: leg.name.clone(),
                });
            }
            producers.insert(asset.as_str(), leg.name.as_str());
        }
    }

    // A producer must run before its consumers.
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

    let waves = waves(legs, &edges)?;
    Ok(ParallelPlan {
        edges: edges.into_iter().collect(),
        waves,
    })
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
