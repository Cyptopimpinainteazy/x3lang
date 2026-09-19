//! Execution lanes — spec PHASE 30.
//!
//! A *lane* is the runtime class a program belongs to: what kind of work it is, and
//! therefore what it needs from the executor to run at all. PHASE 30 names five:
//! standard transactions, trading, atomic cross-domain execution, liquidation and
//! settlement. This module decides which one a program is, and the order in which
//! the lanes are served.
//!
//! ## The lane is read from what the program does
//!
//! [`classify`] reads the *lowered operations*, never the source and never a flag.
//! A program is not "a trading program" because an author said so: it is one
//! because it swaps. Reading the source would be a second, weaker opinion about the
//! same question, and a flag would make the lane a preference rather than a
//! property.
//!
//! A program can be several things at once — a liquidation swaps *and* repays *and*
//! touches a lending protocol — so the classification is a **precedence**, stated
//! once in [`PRIORITY`] and applied in order. The rule the order encodes is that the
//! most constrained lane wins: a program that needs the liquidation executor needs
//! it whether or not it also swaps, and putting it in the trading lane would hide
//! the guarantee it depends on. The order is data, so a reader can audit it, and a
//! test asserts it is a total order over every lane.
//!
//! ## The scheduling policy, and what it deliberately does not do
//!
//! PHASE 30's constraint is "Do NOT implement unfair ordering mechanisms.
//! Scheduling policy should be deterministic, documented, and auditable." So the
//! policy is [`schedule`], it is three rules, and there is no fourth:
//!
//! 1. **Within a lane, arrival order is preserved.** The order `schedule` is given is
//!    the order it returns, restricted to each lane. Nothing about a program's
//!    contents, size or fee moves it relative to another program in its own lane.
//! 2. **Across lanes, the order is [`PRIORITY`]** — a fixed sequence, not a computed
//!    score. It is the same for every block and every program, and it is printed by
//!    `x3c lanes` so it can be audited rather than inferred.
//! 3. **Nothing a participant controls changes either** — not a fee, not a stake, not
//!    a gas limit, not a declared priority. There is no parameter for one, and there
//!    is deliberately no way to add one without changing this module: a market in
//!    lane position is the unfair mechanism the phase forbids, and the cheapest way
//!    to be sure there is not one is to have no input it could be expressed in.
//!
//! This is a *classification and ordering* policy. It does not attach itself to a
//! real queue: the VM has no scheduler object for a lane to be admitted into, and
//! saying otherwise would be the "wired up" claim this repository refuses. The
//! function it does provide is the whole of the policy, so an executor that gains a
//! queue can use it directly.

use std::collections::BTreeSet;

use x3_lang_ast::ast::Program;

use crate::ir::{Operation, X3IR};

/// The runtime classes PHASE 30 names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum Lane {
    /// A program that only moves value it already has: locks, releases, mints,
    /// burns.
    Settlement,
    /// A program that trades: swaps, routes, choices, hedges, and the arbitrage
    /// scopes built out of them.
    Trading,
    /// A program whose legs settle on more than one ledger and must be atomic
    /// across them.
    AtomicCrossDomain,
    /// A program that closes a position on a lending protocol.
    Liquidation,
    /// A program that does nothing at all — only guards, nonces and bookkeeping.
    Standard,
}

/// The order the lanes are served in, most constrained first.
///
/// This is the whole cross-lane policy: a fixed sequence, not a score. A reader can
/// check it without running anything, and `x3c lanes` prints it.
pub const PRIORITY: [Lane; 5] = [
    Lane::Liquidation,
    Lane::AtomicCrossDomain,
    Lane::Trading,
    Lane::Settlement,
    Lane::Standard,
];

impl Lane {
    pub const ALL: [Lane; 5] = [
        Lane::Liquidation,
        Lane::AtomicCrossDomain,
        Lane::Trading,
        Lane::Settlement,
        Lane::Standard,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Lane::Liquidation => "liquidation",
            Lane::Standard => "standard",
            Lane::Trading => "trading",
            Lane::AtomicCrossDomain => "atomic_cross_domain",
            Lane::Settlement => "settlement",
        }
    }

    /// Where the lane sits in [`PRIORITY`].
    pub fn rank(self) -> usize {
        PRIORITY
            .iter()
            .position(|lane| *lane == self)
            .expect("every lane is in PRIORITY")
    }
}

/// Decide which lane a program belongs to from its lowered operations.
///
/// The most constrained lane the program's operations imply wins; see `PRIORITY`
/// for the order and the reason it is that order.
pub fn classify(ir: &X3IR) -> Lane {
    let mut lanes: BTreeSet<Lane> = BTreeSet::new();
    let mut domains: BTreeSet<String> = BTreeSet::new();

    for operation in &ir.operations {
        match operation {
            Operation::Liquidation { .. } => {
                lanes.insert(Lane::Liquidation);
            }
            Operation::Hedge { .. }
            | Operation::Swap { .. }
            | Operation::AtomicChoice { .. }
            | Operation::RouteFallback { .. }
            | Operation::Rebalance { .. }
            | Operation::MultiHopSwap { .. }
            | Operation::Hyperarb { .. } => {
                lanes.insert(Lane::Trading);
            }
            Operation::Bridge {
                from_chain, to_chain, ..
            } => {
                lanes.insert(Lane::AtomicCrossDomain);
                domains.insert(from_chain.clone());
                domains.insert(to_chain.clone());
            }
            Operation::ParallelPlan { domains: legs, .. } => {
                // A plan whose legs settle on more than one ledger is the
                // cross-domain case; one whose legs share a domain is not, and
                // calling it cross-domain would put it in a lane it does not need.
                for (chain, families) in legs {
                    domains.insert(chain.clone());
                    domains.extend(families.iter().cloned());
                }
                lanes.insert(Lane::Trading);
            }
            Operation::Release { .. } | Operation::Mint { .. } | Operation::Burn { .. } | Operation::Lock { .. } => {
                lanes.insert(Lane::Settlement);
            }
            _ => {}
        }
    }

    // A swap that names two chains is cross-domain even without a `Bridge`
    // operation: `ethereum.DAI -> solana.SOL` moves value between ledgers, which is
    // why the operation carries both chains explicitly.
    for operation in &ir.operations {
        if let Operation::Swap {
            from_chain, to_chain, ..
        } = operation
        {
            if from_chain != to_chain {
                lanes.insert(Lane::AtomicCrossDomain);
            }
        }
    }
    if domains.len() > 1 {
        lanes.insert(Lane::AtomicCrossDomain);
    }

    // Most constrained wins.
    PRIORITY
        .iter()
        .copied()
        .find(|lane| lanes.contains(lane))
        .unwrap_or(Lane::Standard)
}

/// One unit of work waiting to be served: what it is, and what lane it is in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Queued {
    pub name: String,
    pub lane: Lane,
}

/// Order work by the documented policy: lane class first, arrival order within it.
///
/// The sort is stable and keyed on the lane's rank alone, so two items in one lane
/// keep the order they were given. There is no argument for a fee, a stake or a
/// declared priority, and that is the policy rather than an omission — see the
/// module documentation.
pub fn schedule(work: &[Queued]) -> Vec<Queued> {
    let mut ordered: Vec<Queued> = work.to_vec();
    ordered.sort_by_key(|item| item.lane.rank());
    ordered
}

/// Classify every declaration in a program, for `x3c lanes`.
///
/// A declaration that does not lower has no operations and therefore **no lane**,
/// and it is reported with the reason rather than given one. Handing it the standard
/// lane would be a quiet mislabel: "standard" would then mean both "does nothing that
/// needs a lane" and "could not be read", which are different facts about a program
/// and would make the report useless for the one thing it is for.
pub fn classify_program(program: &Program) -> Vec<(String, Result<Lane, String>)> {
    let mut found = Vec::new();
    for item in &program.items {
        let name = match &item.node {
            x3_lang_ast::ast::Item::Function(function) => function.name.as_str().to_string(),
            x3_lang_ast::ast::Item::IntentDecl(intent) => intent.name.as_str().to_string(),
            x3_lang_ast::ast::Item::AtomicSwap(swap) => swap.name.as_str().to_string(),
            x3_lang_ast::ast::Item::Strategy(strategy) => strategy.name.as_str().to_string(),
            x3_lang_ast::ast::Item::Proposal(proposal) => proposal.name.as_str().to_string(),
            x3_lang_ast::ast::Item::ParallelDecl(parallel) => parallel.name.as_str().to_string(),
            _ => continue,
        };
        let single = Program {
            items: vec![item.clone()],
        };
        let lane = match crate::lowering::lower_program(&single, crate::lowering::LowerCtx::new()) {
            Ok(ir) => Ok(classify(&ir)),
            Err(error) => Err(format!("{error}")),
        };
        found.push((name, lane));
    }
    found
}
