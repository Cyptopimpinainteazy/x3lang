//! GPU acceleration — spec build-order item 51, PHASE 51.
//!
//! The phase's constraint is the testable part: "Do NOT move consensus-critical
//! behavior to GPU unless determinism is guaranteed. CPU and GPU results must match
//! bit-for-bit where consensus matters." These tests hold the module to it — in
//! particular they check that equality comes back **unproven** rather than passing on
//! the strength of one backend, which is the silent fallback the phase forbids.

use x3_lang_vm::gpu::{self, Backend, Consensus, Dispatch, Equality, GpuCandidate};

#[test]
fn every_candidate_the_phase_names_is_classified_with_a_reason() {
    let classified = gpu::classifications();
    assert_eq!(
        classified.len(),
        GpuCandidate::ALL.len(),
        "every candidate must be classified, and none twice"
    );
    for classification in &classified {
        assert!(
            classification.reason.len() > 20,
            "'{}' must have a reason a reader can disagree with: {}",
            classification.candidate.as_str(),
            classification.reason
        );
    }
    // The four that decide what a block contains, and the two that do not. Written
    // out rather than counted, so a change to any one of them is a visible decision.
    let critical: Vec<&str> = classified
        .iter()
        .filter(|classification| classification.consensus == Consensus::Critical)
        .map(|classification| classification.candidate.as_str())
        .collect();
    assert_eq!(
        critical,
        vec![
            "route candidate scoring",
            "signature verification",
            "hash batches",
            "graph scoring",
        ],
        "the computations whose output enters a block are the consensus-critical ones"
    );
    let not_critical: Vec<&str> = classified
        .iter()
        .filter(|classification| classification.consensus == Consensus::NotCritical)
        .map(|classification| classification.candidate.as_str())
        .collect();
    assert_eq!(
        not_critical,
        vec!["simulation batches", "opportunity filtering"],
        "a dry run and an off-chain filter settle nothing"
    );
}

#[test]
fn the_probe_reports_what_the_host_path_answers_rather_than_a_constant() {
    // The GPU path is a host call. This asserts the module's answers come from calling
    // it: the refusal names the host's own error code, so a host that implements the
    // path changes these answers without anyone editing a list here.
    let probe = gpu::gpu_backend_probe();
    assert!(
        probe.is_err(),
        "no adapter in this workspace implements the GPU path: {probe:?}"
    );
    let reason = probe.expect_err("checked above");
    assert!(
        reason.contains("X3_BACKEND_REQUIRED") || reason.contains("X3_FEATURE_NOT_AVAILABLE"),
        "the refusal must be the host's own: {reason}"
    );
}

#[test]
fn the_only_available_backend_is_the_cpu() {
    assert_eq!(
        gpu::available_backends(),
        vec![Backend::Cpu],
        "with no GPU backend, the CPU is the only implementation"
    );
}

#[test]
fn equality_is_unproven_and_never_claimed_on_one_backend() {
    // The heart of it. A CPU-versus-CPU comparison called "equal" is the silent
    // fallback the phase forbids, so the verdict must be a distinct answer carrying a
    // reason — and the CPU side must have been computed for real, or the harness would
    // be vacuous.
    let inputs: Vec<(i128, i128, i128)> = (1..=32).map(|i| (i, i * 2, 900_000 + i * 1_000)).collect();
    let run = gpu::run_equality(GpuCandidate::RouteCandidateScoring, &inputs);
    assert_eq!(run.samples, 32);
    assert!(
        !run.cpu_bytes.is_empty(),
        "the CPU backend's answers must be computed even when nothing can be compared"
    );
    match &run.verdict {
        Equality::Unproven(reason) => {
            assert!(
                reason.contains("no GPU backend to compare against"),
                "the verdict must say what was missing: {reason}"
            );
            assert!(
                reason.contains("X3_BACKEND_REQUIRED") || reason.contains("X3_FEATURE_NOT_AVAILABLE"),
                "the verdict must carry the host's answer: {reason}"
            );
        }
        Equality::Proven { .. } => panic!("equality cannot be proven with one implementation: {:?}", run.verdict),
    }
}

#[test]
fn the_cpu_batch_is_deterministic_over_the_same_inputs() {
    let inputs: Vec<(i128, i128, i128)> = (1..=64).map(|i| (i, 3 * i, 500_000 + i)).collect();
    let first = gpu::run_equality(GpuCandidate::GraphScoring, &inputs);
    let second = gpu::run_equality(GpuCandidate::GraphScoring, &inputs);
    assert_eq!(
        first.cpu_bytes, second.cpu_bytes,
        "the backend the comparison is against must itself be reproducible"
    );
}

#[test]
fn the_compared_path_contains_no_floating_point() {
    // `cpu_route_score_batch` returns integers, which is what makes "bit for bit" a
    // statement that can be checked rather than an aspiration. This test is the
    // declaration of that property: if the kernel ever grew a float, this is where the
    // change would have to be argued.
    let scores: Vec<i128> = gpu::cpu_route_score_batch(&[(5, 8, 1_000_000)]);
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0], 13, "5 + 8 bps over a full-depth pool is 13");
    // An exact-value check over several inputs, because "it is integer" is only worth
    // something if the integers are right.
    let exact = gpu::cpu_route_score_batch(&[(0, 0, 1), (5, 5, 1), (1, 1, 2)]);
    assert_eq!(exact, vec![0, 10_000_000, 1_000_000]);
}

#[test]
fn a_consensus_critical_computation_is_refused_the_gpu_path_for_the_phases_own_reason() {
    let refusal = gpu::gpu_path_permitted(GpuCandidate::SignatureVerification)
        .expect_err("a signature check must not move to an unproven backend");
    assert!(
        refusal.contains("signature verification") && refusal.contains("consensus-critical"),
        "the refusal must name the computation and its class: {refusal}"
    );
    assert!(
        refusal.contains("bit for bit") && refusal.contains("cannot be established"),
        "the refusal must state the requirement that is unmet: {refusal}"
    );
}

#[test]
fn a_non_critical_computation_is_refused_the_gpu_path_for_the_missing_backend() {
    let refusal =
        gpu::gpu_path_permitted(GpuCandidate::SimulationBatches).expect_err("there is no backend to move it to");
    assert!(
        refusal.contains("simulation batches") && refusal.contains("not consensus-critical"),
        "the refusal must say the computation could move first: {refusal}"
    );
    assert!(
        refusal.contains("no GPU backend to move it to"),
        "and that the reason it does not is the hardware: {refusal}"
    );
}

#[test]
fn every_candidate_runs_on_the_cpu_and_the_reason_says_it_is_not_a_fallback() {
    for candidate in GpuCandidate::ALL {
        match gpu::select(candidate) {
            Dispatch::Allowed { backend, reason } => {
                assert_eq!(backend, Backend::Cpu, "{}", candidate.as_str());
                assert!(
                    reason.contains("only backend available, not a fallback"),
                    "the selection must say the CPU is the only implementation: {reason}"
                );
            }
            Dispatch::Refused(reason) => panic!("{} must be runnable on the CPU: {reason}", candidate.as_str()),
        }
    }
}
