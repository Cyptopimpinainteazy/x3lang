//! Intent fusion — spec build-order item 21, PHASE 21.
//!
//! Several intents whose flows close a ring can settle against each other
//! instead of each taking external liquidity: Alice gives ETH and wants SOL, Bob
//! gives SOL and wants USDC, Charlie gives USDC and wants ETH, and nobody has to
//! trade outside the ring.
//!
//! What this module will and will not claim is the whole design.
//!
//! **It reads the lowered operations, not the source**, so what a participant
//! gives and wants is what their intent actually does. A source-level reading
//! would be a second, weaker opinion about the same question.
//!
//! **It refuses to be sure where the language is silent.** Whether a ring
//! satisfies everyone's minimum needs each participant's minimum and each
//! supplier's amount. When either is missing — a bridge-only intent states what
//! it bridges, never what it delivers on the far side — the check reports
//! `Unverifiable` with the reason. Reporting "satisfied" there would be the
//! worst answer available: a ring that looks checked and was not.
//!
//! **Nothing is fused partially.** If one participant's minimum is not met, the
//! ring is not fusable at all. Netting that satisfies some participants and
//! shorts others is exactly the fairness failure the spec names, and it is not
//! something a compiler should decide by itself.
//!
//! PHASE 21's other requirements are here too: every participant must have opted
//! in (`allow intent_fusion`), so an intent that did not consent is never
//! internalized; assets must match exactly at every hop; and the ring reports the
//! earliest deadline among its participants, because a ring is not settled until
//! its most urgent member is out of time.

use serde::{Deserialize, Serialize};

use x3_lang_ast::ast::{Item, Program};
use x3_lang_common::Spanned;

use crate::ir::{Operation, X3IR};

/// Longest ring the analysis will look for.
///
/// Bounded for the same reason the route search is: an unbounded cycle hunt is
/// not an analysis, it is a hang.
pub const MAX_RING_LENGTH: usize = 6;

/// What one intent offers and asks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentFlow {
    pub name: String,
    /// The asset it gives and how much, from its lock or swap input.
    pub gives: Option<(String, u128)>,
    /// The asset it wants, and the minimum it declared for that asset.
    pub wants: Option<(String, Option<u128>)>,
    /// The expiry it declared, in blocks.
    pub deadline_blocks: Option<u32>,
    /// Whether it wrote `allow intent_fusion`.
    pub opted_in: bool,
}

/// The outcome of one preservation check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Check {
    Satisfied,
    /// The check could not be performed, and this is why.
    Unverifiable(String),
    /// The check was performed and failed, and this is what failed.
    Failed(String),
}

impl Check {
    pub fn is_satisfied(&self) -> bool {
        matches!(self, Check::Satisfied)
    }
}

/// One candidate ring of intents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FusionRing {
    /// Participants in ring order: each one's want is supplied by the next.
    pub participants: Vec<String>,
    /// The asset handed over at each hop, in the same order.
    pub assets: Vec<String>,
    /// The earliest deadline among the participants.
    pub earliest_deadline: Option<u32>,
    pub authorization: Check,
    pub asset_correctness: Check,
    pub minimum_output: Check,
    pub deadline: Check,
    pub fairness: Check,
}

impl FusionRing {
    /// Whether every check passed, so the ring may be netted.
    pub fn is_fusable(&self) -> bool {
        self.authorization.is_satisfied()
            && self.asset_correctness.is_satisfied()
            && self.minimum_output.is_satisfied()
            && self.deadline.is_satisfied()
            && self.fairness.is_satisfied()
    }
}

/// Read one intent's flow from its lowered operations.
pub fn flow_of(name: &str, ir: &X3IR) -> IntentFlow {
    let mut gives = None;
    let mut wants_asset: Option<String> = None;
    let mut wants_min: Option<u128> = None;
    let mut deadline_blocks = None;
    let mut opted_in = false;

    for operation in &ir.operations {
        match operation {
            Operation::Lock {
                chain, asset, amount, ..
            } => {
                if gives.is_none() && *amount > 0 {
                    gives = Some((key(chain, asset), *amount));
                }
            }
            Operation::Release { chain, asset, .. } => {
                if wants_asset.is_none() {
                    wants_asset = Some(key(chain, asset));
                }
            }
            Operation::Swap {
                to_chain,
                to_asset,
                min_output,
                ..
            } => {
                // The minimum, when the swap delivers the asset this intent says
                // it wants. A swap delivering something else states a minimum for
                // a different asset, which says nothing about what the
                // participant receives.
                if let Some(want) = &wants_asset {
                    if want == &key(to_chain, to_asset) && *min_output > 0 && wants_min.is_none() {
                        wants_min = Some(*min_output);
                    }
                }
            }
            Operation::Bridge {
                to_chain,
                to_asset,
                min_output,
                ..
            } => {
                // A bridge names the asset it delivers, and now also carries the
                // minimum the source stated for that delivery (TICKET-038).
                if wants_asset.is_none() {
                    wants_asset = Some(key(to_chain, to_asset));
                }
                if wants_asset.as_deref() == Some(key(to_chain, to_asset).as_str())
                    && *min_output > 0
                    && wants_min.is_none()
                {
                    wants_min = Some(*min_output);
                }
            }
            Operation::OnTimeout {
                duration_blocks: blocks,
                ..
            } => {
                if deadline_blocks.is_none() {
                    deadline_blocks = Some(*blocks);
                }
            }
            Operation::FeatureAllow { feature, .. } => {
                if *feature == crate::spec::opcodes::FEATURE_INTENT_FUSION {
                    opted_in = true;
                }
            }
            _ => {}
        }
    }

    IntentFlow {
        name: name.to_string(),
        gives,
        wants: wants_asset.map(|asset| (asset, wants_min)),
        deadline_blocks,
        opted_in,
    }
}

/// Read every intent in a program, lowering each one on its own.
pub fn flows(program: &Program) -> Vec<IntentFlow> {
    let mut flows = Vec::new();
    for item in &program.items {
        let Item::IntentDecl(intent) = &item.node else {
            continue;
        };
        let single = Program {
            items: vec![Spanned::dummy(item.node.clone())],
        };
        match crate::lowering::lower_program(&single, crate::lowering::LowerCtx::new()) {
            Ok(ir) => flows.push(flow_of(intent.name.as_str(), &ir)),
            // An intent that does not lower has no flow to net. It is not
            // silently skipped: it appears with nothing in it, so the report can
            // say why it took no part.
            Err(_) => flows.push(IntentFlow {
                name: intent.name.as_str().to_string(),
                gives: None,
                wants: None,
                deadline_blocks: None,
                opted_in: false,
            }),
        }
    }
    flows
}

/// Find every candidate ring, deterministically.
///
/// Only opted-in intents with a known give and want take part: an intent that did
/// not consent is never internalized, and one whose flows cannot be read cannot
/// be checked.
pub fn rings(flows: &[IntentFlow]) -> Vec<FusionRing> {
    let candidates: Vec<&IntentFlow> = flows
        .iter()
        .filter(|flow| flow.opted_in && flow.gives.is_some() && flow.wants.is_some())
        .collect();

    let mut found: Vec<FusionRing> = Vec::new();
    for start in &candidates {
        let mut path: Vec<&IntentFlow> = vec![start];
        walk(&candidates, start, start, &mut path, &mut found);
    }
    // Canonical presentation: each ring starts at its lexicographically smallest
    // participant, and the list is sorted, so the report does not depend on
    // declaration order.
    for ring in &mut found {
        canonicalise(ring);
    }
    found.sort_by(|left, right| left.participants.cmp(&right.participants));
    found.dedup();
    found
}

fn walk<'a>(
    candidates: &[&'a IntentFlow],
    start: &IntentFlow,
    current: &'a IntentFlow,
    path: &mut Vec<&'a IntentFlow>,
    found: &mut Vec<FusionRing>,
) {
    if path.len() > MAX_RING_LENGTH {
        return;
    }
    let Some((want_asset, _)) = &current.wants else {
        return;
    };
    for next in candidates {
        if path.iter().any(|member| member.name == next.name) {
            continue;
        }
        let Some((give_asset, _)) = &next.gives else {
            continue;
        };
        // The next participant must supply exactly what this one wants, and a
        // participant that gives and wants the same asset is not a hop — it is a
        // transfer, and netting it against anything would be modelling a trade
        // that does not exist.
        let gives_and_wants_the_same = next.wants.as_ref().is_some_and(|(want, _)| want == give_asset);
        if give_asset != want_asset || gives_and_wants_the_same {
            continue;
        }
        path.push(next);
        if let Some((next_want, _)) = &next.wants {
            if let Some((start_gives, _)) = &start.gives {
                if next_want == start_gives && path.len() >= 2 {
                    found.push(build(path));
                }
            }
        }
        walk(candidates, start, next, path, found);
        path.pop();
    }
}

fn build(path: &[&IntentFlow]) -> FusionRing {
    let participants: Vec<String> = path.iter().map(|flow| flow.name.clone()).collect();
    let assets: Vec<String> = path
        .iter()
        .map(|flow| flow.gives.as_ref().map(|(asset, _)| asset.clone()).unwrap_or_default())
        .collect();
    let earliest_deadline = path.iter().filter_map(|flow| flow.deadline_blocks).min();

    // Authorization: by construction every member opted in. Recorded rather than
    // assumed, because the ring's claim includes it.
    let authorization = if path.iter().all(|flow| flow.opted_in) {
        Check::Satisfied
    } else {
        Check::Failed("a participant did not allow intent_fusion".to_string())
    };

    // Asset correctness: by construction each hop hands over what the next one
    // wants. The check exists so the report states it rather than implying it.
    let asset_correctness = if path.iter().all(|flow| flow.gives.is_some() && flow.wants.is_some()) {
        Check::Satisfied
    } else {
        Check::Unverifiable("a participant's give or want could not be read".to_string())
    };

    // Minimum output: every participant's declared minimum must be met by what
    // the next participant hands over.
    let mut minimum_output = Check::Satisfied;
    for (index, flow) in path.iter().enumerate() {
        let next = path[(index + 1) % path.len()];
        let Some((_, want_min)) = &flow.wants else {
            continue;
        };
        let Some((give_asset, give_amount)) = &next.gives else {
            continue;
        };
        match want_min {
            None => {
                if minimum_output.is_satisfied() {
                    minimum_output = Check::Unverifiable(format!(
                        "'{}' declares no minimum for {give_asset}, so whether what '{}' hands over \
                         is enough cannot be checked",
                        flow.name, next.name
                    ));
                }
            }
            Some(minimum) if *give_amount < *minimum => {
                minimum_output = Check::Failed(format!(
                    "'{}' requires at least {minimum} of {give_asset} but '{}' hands over {give_amount}",
                    flow.name, next.name
                ));
            }
            Some(_) => {}
        }
    }

    // Deadline: a ring is not settled until its most urgent member is out of
    // time, so the check reports the earliest and refuses a ring where any
    // participant's expiry is unknown.
    let deadline = if path.iter().all(|flow| flow.deadline_blocks.is_some()) {
        Check::Satisfied
    } else {
        Check::Unverifiable(
            "a participant declares no expiry, so the ring has no deadline to settle before".to_string(),
        )
    };

    // Fairness: all or nothing. If any participant is short, the ring is not
    // fusable — netting that satisfies some and shorts others is the failure the
    // spec names.
    let fairness = match &minimum_output {
        Check::Failed(reason) => Check::Failed(format!("no partial netting is possible: {reason}")),
        Check::Unverifiable(reason) => Check::Unverifiable(reason.clone()),
        Check::Satisfied => Check::Satisfied,
    };

    FusionRing {
        participants,
        assets,
        earliest_deadline,
        authorization,
        asset_correctness,
        minimum_output,
        deadline,
        fairness,
    }
}

/// Rotate a ring so it starts at its smallest participant: the same ring written
/// from a different starting point is the same ring, and the report should say so
/// once.
fn canonicalise(ring: &mut FusionRing) {
    let Some(smallest) = ring.participants.iter().min().cloned() else {
        return;
    };
    let Some(position) = ring.participants.iter().position(|name| *name == smallest) else {
        return;
    };
    ring.participants.rotate_left(position);
    ring.assets.rotate_left(position);
}

fn key(chain: &str, asset: &str) -> String {
    format!("{chain}.{asset}")
}
