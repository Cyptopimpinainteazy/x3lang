//! GPU acceleration — spec PHASE 51.
//!
//! The phase names six computations worth accelerating and attaches one condition to
//! all of them: "Do NOT move consensus-critical behavior to GPU unless determinism is
//! guaranteed. CPU and GPU results must match bit-for-bit where consensus matters."
//!
//! The GPU path in this workspace is a **host call** — `BridgeAdapter::gpu_dispatch`,
//! reached by the `GpuDispatch` instruction — and no adapter here implements it: the
//! default refuses with `X3_FEATURE_NOT_AVAILABLE` and the production adapter with
//! `X3_BACKEND_REQUIRED`. So the honest state of PHASE 51 is that the *classification*
//! and the *equality requirement* are the work, and the accelerator is not. This
//! module is those two things, and nothing else pretends to be a kernel.
//!
//! ## The classification is in code, not in a comment
//!
//! [`classify`] answers, for each of the phase's six candidates, whether the
//! computation is consensus-critical and why. The test is whether its *output* enters
//! a block: a route score decides which route a signed transaction takes, a signature
//! check decides whether a transaction is valid at all, a state-root hash becomes part
//! of the block. A simulation result settles nothing and an off-chain discovery filter
//! decides only what a searcher looks at. Six classifications, each with a reason a
//! reader can disagree with — which is the point of writing them down.
//!
//! ## Equality is *probed*, not asserted
//!
//! [`run_equality`] computes the CPU backend's answers for real and then looks for a
//! second backend to compare them against by calling the host path
//! ([`gpu_backend_probe`]). With none, the verdict is
//! [`Equality::Unproven`] carrying the probe's own message, and it can never be
//! `Proven` on the strength of one backend: a CPU-vs-CPU comparison called equality is
//! the silent fallback the phase forbids, and the type makes it unrepresentable rather
//! than merely discouraged.
//!
//! The CPU side is genuinely computed and encoded, so the harness is not vacuous and a
//! later GPU backend has something concrete to match byte for byte.
//!
//! ## What may be dispatched where
//!
//! [`select`] names the backend a candidate may run on. Today that is always the CPU,
//! and its reason is the probe's: the CPU is not a fallback here, it is the only
//! implementation. [`gpu_path_permitted`] is the other half — the refusal a caller who
//! wants the GPU path gets, worded per candidate so a consensus-critical computation
//! is refused for the reason the phase gives rather than for the missing hardware
//! alone.

use crate::bridge::{BridgeAdapter, UnconfiguredBridge};

/// The computations PHASE 51 names as candidates for acceleration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize)]
pub enum GpuCandidate {
    RouteCandidateScoring,
    SignatureVerification,
    HashBatches,
    GraphScoring,
    SimulationBatches,
    OpportunityFiltering,
}

impl GpuCandidate {
    /// Every candidate, in the order the phase lists them.
    pub const ALL: [GpuCandidate; 6] = [
        GpuCandidate::RouteCandidateScoring,
        GpuCandidate::SignatureVerification,
        GpuCandidate::HashBatches,
        GpuCandidate::GraphScoring,
        GpuCandidate::SimulationBatches,
        GpuCandidate::OpportunityFiltering,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            GpuCandidate::RouteCandidateScoring => "route candidate scoring",
            GpuCandidate::SignatureVerification => "signature verification",
            GpuCandidate::HashBatches => "hash batches",
            GpuCandidate::GraphScoring => "graph scoring",
            GpuCandidate::SimulationBatches => "simulation batches",
            GpuCandidate::OpportunityFiltering => "opportunity filtering",
        }
    }
}

/// Whether a computation's output enters a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consensus {
    /// The output decides what a block contains, so two implementations disagreeing
    /// is a fork rather than a bug.
    Critical,
    /// The output never settles: it decides what somebody looks at or tries.
    NotCritical,
}

/// One candidate's classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    pub candidate: GpuCandidate,
    pub consensus: Consensus,
    pub reason: &'static str,
}

/// Classify one candidate.
pub fn classify(candidate: GpuCandidate) -> Classification {
    let (consensus, reason) = match candidate {
        GpuCandidate::RouteCandidateScoring => (
            Consensus::Critical,
            "the score decides which route a signed transaction takes, and the route is \
             in the block: two scorers disagreeing by one unit is a different trade",
        ),
        GpuCandidate::SignatureVerification => (
            Consensus::Critical,
            "the check decides whether a transaction is valid at all, which is the most \
             consensus-critical question there is",
        ),
        GpuCandidate::HashBatches => (
            Consensus::Critical,
            "when the hashes are commitments — receipt hashes, state roots, canonical \
             supply — the digest is in the block, so the batch and the single hash must \
             agree bit for bit",
        ),
        GpuCandidate::GraphScoring => (
            Consensus::Critical,
            "the graph's scores are what the search ranks by, and the ranking selects \
             the route, so a different score is a different block",
        ),
        GpuCandidate::SimulationBatches => (
            Consensus::NotCritical,
            "a simulation is a dry run: its output is read by the caller and settles \
             nothing, which is why it is the one candidate here that could move first",
        ),
        GpuCandidate::OpportunityFiltering => (
            Consensus::NotCritical,
            "the filter decides which opportunities a *searcher* looks at, off the \
             settlement path; what is settled is still decided by the route's own \
             guards",
        ),
    };
    Classification {
        candidate,
        consensus,
        reason,
    }
}

/// Every candidate, classified.
pub fn classifications() -> Vec<Classification> {
    GpuCandidate::ALL.into_iter().map(classify).collect()
}

/// A backend a computation may run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Cpu,
    Gpu,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Cpu => "cpu",
            Backend::Gpu => "gpu",
        }
    }
}

/// Ask the host path whether a GPU backend answers.
///
/// This is a *probe*, not a constant: the GPU path is `BridgeAdapter::gpu_dispatch`,
/// so the question "is there a GPU backend?" is answered by calling it and reading the
/// answer. Today the adapter this workspace provides for an unconfigured host refuses,
/// and the refusal's own message is what [`run_equality`] and [`select`] report — so a
/// host that one day implements the path changes this module's answers without anyone
/// editing a list here.
pub fn gpu_backend_probe() -> Result<(), String> {
    match UnconfiguredBridge.gpu_dispatch("__probe__", &[]) {
        Ok(_) => Ok(()),
        // The probe reports what the host said rather than a message of this module's
        // own, so a reader of a refusal sees the host's words and can look them up.
        Err(error) => Err(error.to_string()),
    }
}

/// The backends this runtime can actually run a batched computation on.
pub fn available_backends() -> Vec<Backend> {
    let mut backends = vec![Backend::Cpu];
    if gpu_backend_probe().is_ok() {
        backends.push(Backend::Gpu);
    }
    backends
}

/// The CPU backend's batch kernel: a route score for each candidate.
///
/// Integer arithmetic only. That is not an implementation detail: it is what makes
/// "CPU and GPU results match bit-for-bit" a statement that can be checked at all, so
/// the compared path contains no floating point to disagree about.
pub fn cpu_route_score_batch(candidates: &[(i128, i128, i128)]) -> Vec<i128> {
    candidates
        .iter()
        .map(|(fee_bps, slippage_bps, liquidity_fraction_millionths)| {
            // Lower is better: the fee plus the slippage, penaltied by how thin the
            // pool is. Every term is an integer, and the subtraction cannot overflow
            // because the penalty is bounded by the multiplier it is divided by.
            let total = fee_bps + slippage_bps;
            total.saturating_mul(1_000_000) / (*liquidity_fraction_millionths).max(1)
        })
        .collect()
}

/// Little-endian bytes of a batch's answers, which is the form the comparison is over.
fn encode(scores: &[i128]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(scores.len() * 16);
    for score in scores {
        bytes.extend_from_slice(&score.to_le_bytes());
    }
    bytes
}

/// What a CPU-versus-accelerator comparison concluded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Equality {
    /// Two implementations produced identical bytes over the same inputs.
    Proven { samples: usize },
    /// The comparison could not be made, and this is why. It is a distinct answer from
    /// `Proven` so that a caller cannot read "we only ran the CPU" as agreement.
    Unproven(String),
}

/// One equality run: the CPU's answers, and what could be concluded from them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EqualityRun {
    pub candidate: GpuCandidate,
    /// The CPU backend's encoded answers, computed for real.
    pub cpu_bytes: Vec<u8>,
    pub verdict: Equality,
    pub samples: usize,
}

/// Compute the CPU backend's batch answers and try to compare them with a second
/// backend.
///
/// The CPU side is always computed. The comparison is only made when
/// [`gpu_backend_probe`] finds a backend to compare against — and when it does not,
/// the verdict is [`Equality::Unproven`] carrying the probe's message. There is no
/// path through this function that returns `Proven` on the strength of one backend.
pub fn run_equality(candidate: GpuCandidate, inputs: &[(i128, i128, i128)]) -> EqualityRun {
    let cpu_bytes = encode(&cpu_route_score_batch(inputs));
    let verdict = match gpu_backend_probe() {
        Err(reason) => Equality::Unproven(format!(
            "no GPU backend to compare against — the GPU path is the host call \
             `BridgeAdapter::gpu_dispatch` and it answered {reason}, so the CPU's bytes \
             are unverified against anything"
        )),
        Ok(()) => Equality::Unproven(
            "a GPU backend answered the probe and no kernel was dispatched through it, so \
             nothing was compared: answering a probe is not the same as agreeing"
                .to_string(),
        ),
    };
    EqualityRun {
        candidate,
        samples: inputs.len(),
        cpu_bytes,
        verdict,
    }
}

/// Which backend may run a candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dispatch {
    Allowed { backend: Backend, reason: String },
    Refused(String),
}

/// Decide which backend a candidate may run on.
///
/// Today every candidate is `Allowed { backend: Cpu }`, and the reason says why that
/// is not a fallback: the CPU is the only implementation there is, and the probe is
/// what says so.
pub fn select(candidate: GpuCandidate) -> Dispatch {
    match gpu_backend_probe() {
        Err(reason) => Dispatch::Allowed {
            backend: Backend::Cpu,
            reason: format!(
                "the CPU is the only backend available, not a fallback: {} answered {reason}",
                "BridgeAdapter::gpu_dispatch"
            ),
        },
        Ok(()) => match gpu_path_permitted(candidate) {
            Ok(()) => Dispatch::Allowed {
                backend: Backend::Gpu,
                reason: "a GPU backend answered the probe and this computation's equality \
                         requirement is satisfied"
                    .to_string(),
            },
            // A backend exists and this computation still may not use it: the refusal is
            // about the computation, not the hardware.
            Err(refusal) => Dispatch::Allowed {
                backend: Backend::Cpu,
                reason: format!("the GPU path is refused for this computation — {refusal}"),
            },
        },
    }
}

/// Whether a candidate may be dispatched to the GPU path.
///
/// The two conditions are the phase's own. A consensus-critical computation needs
/// bit-for-bit equality with the CPU — [`run_equality`] is how that is established, and
/// it reports `Unproven` while there is no second backend. A non-critical computation
/// still needs a backend to dispatch to.
pub fn gpu_path_permitted(candidate: GpuCandidate) -> Result<(), String> {
    let classification = classify(candidate);
    if let Err(reason) = gpu_backend_probe() {
        return Err(match classification.consensus {
            Consensus::Critical => format!(
                "{} is consensus-critical ({}) and must match the CPU bit for bit before the \
                 GPU path may be taken; with no GPU backend to compare against the equality \
                 cannot be established, and the host path answered {reason}",
                candidate.as_str(),
                classification.reason
            ),
            Consensus::NotCritical => format!(
                "{} is not consensus-critical ({}) so it could move first, but there is no GPU \
                 backend to move it to: the host path answered {reason}",
                candidate.as_str(),
                classification.reason
            ),
        });
    }
    // A backend answered. A consensus-critical computation still needs the equality
    // proof, which is established by `run_equality` over a real input set rather than
    // asserted here — so this function refuses until a caller has one.
    Err(format!(
        "{} answered the GPU probe and no equality proof has been recorded for it; run \
         `run_equality` and attach the result before taking the GPU path",
        candidate.as_str()
    ))
}
