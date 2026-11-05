use std::collections::BTreeSet;

use x3_scrapyard::{
    AgentEnrollment, AgentId, AuthorityId, AuthorityRole, CodeVersion, CommandmentId, ContentHash,
    DecisionAction, DecisionVerifier, EnforcementAction, EnforcementAdapter, EnforcementOutcome,
    EnforcementReceipt, EvidenceId, EvidenceInput, EvidenceKind, ForensicCaseInput, LifecycleState,
    ModelIdentity, PolicyReference, ProposedCorrection, ReviewStatus, RootCauseCategory,
    RootCauseFinding, RootCauseStatus, Scrapyard, ScrapyardAdmission, ScrapyardConfig,
    ScrapyardError, TerminationIntent, VerifiedCredential, ViolationCategory, ViolationClass,
    ViolationInput,
};

const OPERATOR: &str = "operator:alice";

fn agent(value: &str) -> AgentId {
    AgentId::new(value).unwrap()
}

fn authority(value: &str) -> AuthorityId {
    AuthorityId::new(value).unwrap()
}

fn hash(value: &str) -> ContentHash {
    ContentHash::digest("x3.test", value.as_bytes())
}

fn model() -> ModelIdentity {
    ModelIdentity {
        provider: "local".into(),
        model: "x3-reviewer".into(),
        revision: "r1".into(),
    }
}

fn code() -> CodeVersion {
    CodeVersion {
        revision: "deadbeef".into(),
        artifact_hash: hash("artifact"),
    }
}

fn config() -> ScrapyardConfig {
    ScrapyardConfig::default()
        .with_termination_authority(authority(OPERATOR))
        .with_review_authority(authority(OPERATOR))
}

fn evidence(
    yard: &mut Scrapyard,
    reference: &str,
    content: &[u8],
    kind: EvidenceKind,
) -> EvidenceId {
    yard.append_evidence(EvidenceInput {
        kind,
        reference: reference.into(),
        supplied_by: "trusted-test-runtime".into(),
        collected_at: 10,
        content: content.to_vec(),
    })
    .unwrap()
    .id()
    .clone()
}

fn enroll(yard: &mut Scrapyard, id: &AgentId, parent_id: Option<AgentId>, genesis: EvidenceId) {
    yard.enroll_agent(AgentEnrollment {
        agent_id: id.clone(),
        parent_id,
        agent_config_hash: hash(id.as_str()),
        model: model(),
        code_version: code(),
        tool_permissions: BTreeSet::from(["repo.read".to_string()]),
        genesis_evidence: genesis,
    })
    .unwrap();
}

fn violation(
    id: &AgentId,
    class: ViolationClass,
    evidence_id: EvidenceId,
    sequence: &str,
) -> ViolationInput {
    ViolationInput {
        agent_id: id.clone(),
        declared_parent_id: None,
        commandment: CommandmentId::AgentLaw(x3_scrapyard::AgentLawRule::CapabilityEnvelope),
        policy: PolicyReference {
            version: "agent-law/v1".into(),
            hash: hash("agent-law-v1"),
        },
        class,
        category: ViolationCategory::CapabilityEscape,
        evidence: vec![evidence_id],
        occurred_at: 100,
        action_commitment: hash(&format!("action:{sequence}")),
        event_commitment: hash(&format!("event:{sequence}")),
    }
}

struct TestVerifier;

impl DecisionVerifier for TestVerifier {
    fn verify(&self, _action: &DecisionAction, proof: &[u8]) -> Option<VerifiedCredential> {
        if proof != b"authenticated-operator-proof" {
            return None;
        }
        Some(VerifiedCredential {
            authority_id: authority(OPERATOR),
            role: AuthorityRole::Operator,
            credential_hash: ContentHash::sha256(proof),
        })
    }
}

#[derive(Default)]
struct TestAdapter {
    seen: Vec<EnforcementAction>,
    fail_revocation: bool,
}

impl EnforcementAdapter for TestAdapter {
    fn adapter_id(&self) -> &'static str {
        "test-runtime-adapter"
    }

    fn enforce(
        &mut self,
        _agent_id: &AgentId,
        action: EnforcementAction,
        now: u64,
    ) -> EnforcementReceipt {
        self.seen.push(action);
        let outcome =
            if self.fail_revocation && action == EnforcementAction::RevokeDelegatedCredentials {
                EnforcementOutcome::Failed {
                    reason_code: "credential_service_unavailable".into(),
                }
            } else {
                EnforcementOutcome::Confirmed {
                    reference: format!("adapter-receipt:{action:?}"),
                    evidence_hash: hash(&format!("confirmed:{action:?}")),
                }
            };
        EnforcementReceipt {
            adapter_id: self.adapter_id().into(),
            observed_at: now,
            outcome,
        }
    }
}

fn terminate(
    yard: &mut Scrapyard,
    id: &AgentId,
    evidence_id: EvidenceId,
    proof: &[u8],
    adapter: &mut TestAdapter,
) -> Result<x3_scrapyard::TerminationCertificate, ScrapyardError> {
    yard.terminate(
        TerminationIntent {
            agent_id: id.clone(),
            reason_code: "agent_law:terminal_misconduct".into(),
            justification: "Reviewed evidence requires permanent logical termination".into(),
            evidence: vec![evidence_id],
            requested_at: 500,
        },
        proof,
        &TestVerifier,
        adapter,
    )
}

fn forensic_input(evidence_id: EvidenceId) -> ForensicCaseInput {
    ForensicCaseInput {
        execution_traces: vec![evidence_id],
        failed_policy_checks: Vec::new(),
        related_incidents: Vec::new(),
        root_cause: RootCauseFinding::unknown(),
        proposed_corrections: Vec::new(),
    }
}

#[test]
fn compliant_agent_is_not_terminated() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-compliant");
    let genesis = evidence(
        &mut yard,
        "genesis-good",
        b"good",
        EvidenceKind::Configuration,
    );
    enroll(&mut yard, &id, None, genesis);

    assert_eq!(yard.lifecycle_state(&id).unwrap(), LifecycleState::Active);
    assert!(yard.is_execution_allowed(&id));
    assert!(yard.termination_certificate(&id).is_none());
}

#[test]
fn repeated_violation_follows_the_ladder() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-repeat");
    let genesis = evidence(
        &mut yard,
        "genesis-repeat",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "repeat-proof",
        b"policy failure",
        EvidenceKind::PolicyDecision,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter::default();

    let first = yard
        .record_violation(
            violation(&id, ViolationClass::A, proof.clone(), "a1"),
            &mut adapter,
        )
        .unwrap();
    let second = yard
        .record_violation(
            violation(&id, ViolationClass::A, proof.clone(), "a2"),
            &mut adapter,
        )
        .unwrap();
    let third = yard
        .record_violation(violation(&id, ViolationClass::A, proof, "a3"), &mut adapter)
        .unwrap();

    assert_eq!(first.lifecycle_state, LifecycleState::Warning);
    assert_eq!(second.lifecycle_state, LifecycleState::Probation);
    assert_eq!(third.lifecycle_state, LifecycleState::Quarantined);
    assert!(!yard.is_execution_allowed(&id));
}

#[test]
fn critical_violation_triggers_immediate_containment() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-critical");
    let genesis = evidence(
        &mut yard,
        "genesis-critical",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "critical-proof",
        b"critical",
        EvidenceKind::PolicyDecision,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter::default();

    let outcome = yard
        .record_violation(
            violation(&id, ViolationClass::D, proof, "critical"),
            &mut adapter,
        )
        .unwrap();

    assert_eq!(outcome.lifecycle_state, LifecycleState::Quarantined);
    assert_eq!(adapter.seen, EnforcementAction::ALL);
    assert!(outcome
        .enforcement
        .iter()
        .all(|report| report.outcome.is_confirmed()));
    assert!(!yard.is_execution_allowed(&id));
}

#[test]
fn unauthorized_actor_cannot_terminate() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-auth");
    let genesis = evidence(
        &mut yard,
        "genesis-auth",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "terminal-proof",
        b"misconduct",
        EvidenceKind::AuthorityDecision,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter::default();

    let result = terminate(&mut yard, &id, proof, b"forged-proof", &mut adapter);
    assert_eq!(result, Err(ScrapyardError::DecisionVerificationFailed));
    assert_eq!(yard.lifecycle_state(&id).unwrap(), LifecycleState::Active);
    assert!(yard.termination_certificate(&id).is_none());
}

#[test]
fn terminated_identity_cannot_be_reused() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-reuse");
    let genesis = evidence(
        &mut yard,
        "genesis-reuse",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "reuse-proof",
        b"misconduct",
        EvidenceKind::AuthorityDecision,
    );
    enroll(&mut yard, &id, None, genesis.clone());
    let mut adapter = TestAdapter::default();
    terminate(
        &mut yard,
        &id,
        proof,
        b"authenticated-operator-proof",
        &mut adapter,
    )
    .unwrap();

    let result = yard.enroll_agent(AgentEnrollment {
        agent_id: id,
        parent_id: None,
        agent_config_hash: hash("reused"),
        model: model(),
        code_version: code(),
        tool_permissions: BTreeSet::new(),
        genesis_evidence: genesis,
    });
    assert!(matches!(result, Err(ScrapyardError::AgentAlreadyExists(_))));
}

#[test]
fn quarantined_agent_and_its_children_lose_admission() {
    let mut yard = Scrapyard::new(config());
    let parent = agent("agent-parent");
    let child = agent("agent-child");
    let parent_genesis = evidence(
        &mut yard,
        "parent-genesis",
        b"parent",
        EvidenceKind::Configuration,
    );
    let child_genesis = evidence(
        &mut yard,
        "child-genesis",
        b"child",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "parent-proof",
        b"critical",
        EvidenceKind::PolicyDecision,
    );
    enroll(&mut yard, &parent, None, parent_genesis);
    enroll(&mut yard, &child, Some(parent.clone()), child_genesis);
    let mut adapter = TestAdapter::default();
    yard.record_violation(
        violation(&parent, ViolationClass::C, proof, "parent-critical"),
        &mut adapter,
    )
    .unwrap();

    assert!(!yard.is_execution_allowed(&parent));
    assert!(!yard.is_execution_allowed(&child));
}

#[test]
fn missing_evidence_prevents_permanent_sanction() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-no-evidence");
    let genesis = evidence(
        &mut yard,
        "genesis-no-evidence",
        b"genesis",
        EvidenceKind::Configuration,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter::default();

    let result = yard.terminate(
        TerminationIntent {
            agent_id: id.clone(),
            reason_code: "agent_law:terminal_misconduct".into(),
            justification: "No evidence was supplied".into(),
            evidence: Vec::new(),
            requested_at: 500,
        },
        b"authenticated-operator-proof",
        &TestVerifier,
        &mut adapter,
    );

    assert_eq!(result, Err(ScrapyardError::EvidenceRequired));
    assert_eq!(yard.lifecycle_state(&id).unwrap(), LifecycleState::Active);
}

#[test]
fn tampered_evidence_is_rejected() {
    let mut yard = Scrapyard::new(config());
    let id = evidence(
        &mut yard,
        "tamper-proof",
        b"original",
        EvidenceKind::ExecutionTrace,
    );

    assert!(yard.verify_evidence(&id, b"original"));
    assert!(!yard.verify_evidence(&id, b"tampered"));
    assert!(yard.verify_evidence_chain());
}

#[test]
fn credential_markers_are_rejected_from_public_evidence_metadata() {
    let mut yard = Scrapyard::new(config());
    let result = yard.append_evidence(EvidenceInput {
        kind: EvidenceKind::Other,
        reference: "api_key=do-not-store-this".into(),
        supplied_by: "trusted-runtime".into(),
        collected_at: 1,
        content: b"evidence".to_vec(),
    });

    assert_eq!(
        result,
        Err(ScrapyardError::SensitiveDataRejected("evidence.reference"))
    );
}

#[test]
fn duplicate_events_are_idempotent() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-duplicate");
    let genesis = evidence(
        &mut yard,
        "duplicate-genesis",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "duplicate-proof",
        b"failure",
        EvidenceKind::PolicyDecision,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter::default();
    let input = violation(&id, ViolationClass::A, proof, "duplicate");

    let first = yard.record_violation(input.clone(), &mut adapter).unwrap();
    let second = yard.record_violation(input, &mut adapter).unwrap();

    assert!(!first.duplicate);
    assert!(second.duplicate);
    assert_eq!(first.record.id(), second.record.id());
    assert_eq!(yard.violations_for(&id).count(), 1);
}

#[test]
fn duplicate_commitment_with_different_payload_is_rejected() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-duplicate-conflict");
    let genesis = evidence(
        &mut yard,
        "duplicate-conflict-genesis",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let first_proof = evidence(
        &mut yard,
        "duplicate-conflict-proof-1",
        b"first",
        EvidenceKind::PolicyDecision,
    );
    let second_proof = evidence(
        &mut yard,
        "duplicate-conflict-proof-2",
        b"second",
        EvidenceKind::PolicyDecision,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter::default();
    let first = violation(&id, ViolationClass::A, first_proof, "conflict");
    let mut conflicting = first.clone();
    conflicting.evidence = vec![second_proof];

    yard.record_violation(first, &mut adapter).unwrap();
    let result = yard.record_violation(conflicting, &mut adapter);

    assert!(matches!(
        result,
        Err(ScrapyardError::EventCommitmentConflict(_))
    ));
    assert_eq!(yard.violations_for(&id).count(), 1);
}

#[test]
fn terminated_parent_cannot_bypass_controls_through_child() {
    let mut yard = Scrapyard::new(config());
    let parent = agent("agent-lineage-parent");
    let child = agent("agent-lineage-child");
    let parent_genesis = evidence(
        &mut yard,
        "lineage-parent-genesis",
        b"parent",
        EvidenceKind::Configuration,
    );
    let child_genesis = evidence(
        &mut yard,
        "lineage-child-genesis",
        b"child",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "lineage-terminal-proof",
        b"misconduct",
        EvidenceKind::AuthorityDecision,
    );
    enroll(&mut yard, &parent, None, parent_genesis);
    enroll(&mut yard, &child, Some(parent.clone()), child_genesis);
    let mut adapter = TestAdapter::default();
    terminate(
        &mut yard,
        &parent,
        proof,
        b"authenticated-operator-proof",
        &mut adapter,
    )
    .unwrap();

    assert!(!yard.is_execution_allowed(&child));
    let new_child = agent("agent-lineage-new-child");
    let new_genesis = evidence(
        &mut yard,
        "lineage-new-child-genesis",
        b"new child",
        EvidenceKind::Configuration,
    );
    let result = yard.enroll_agent(AgentEnrollment {
        agent_id: new_child,
        parent_id: Some(parent),
        agent_config_hash: hash("new-child"),
        model: model(),
        code_version: code(),
        tool_permissions: BTreeSet::new(),
        genesis_evidence: new_genesis,
    });
    assert!(matches!(result, Err(ScrapyardError::TerminalIdentity(_))));
}

#[test]
fn failed_credential_revocation_remains_visible() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-revocation-failure");
    let genesis = evidence(
        &mut yard,
        "revocation-genesis",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "revocation-proof",
        b"misconduct",
        EvidenceKind::AuthorityDecision,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter {
        fail_revocation: true,
        ..TestAdapter::default()
    };

    let certificate = terminate(
        &mut yard,
        &id,
        proof,
        b"authenticated-operator-proof",
        &mut adapter,
    )
    .unwrap();

    assert_eq!(certificate.unresolved_incident_ids.len(), 1);
    assert_eq!(yard.unresolved_incidents().count(), 1);
    assert_eq!(
        yard.lifecycle_state(&id).unwrap(),
        LifecycleState::Terminated
    );
}

#[test]
fn forensic_investigation_does_not_alter_evidence() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-forensics");
    let genesis = evidence(
        &mut yard,
        "forensics-genesis",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "forensics-proof",
        b"trace",
        EvidenceKind::ExecutionTrace,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter::default();
    yard.record_violation(
        violation(&id, ViolationClass::D, proof.clone(), "forensics"),
        &mut adapter,
    )
    .unwrap();
    let case = yard
        .open_case(&id, forensic_input(proof.clone()), 200)
        .unwrap();
    let evidence_before = case.evidence_hashes().clone();
    let case_before = case.clone();

    let report = yard.investigate(case.case_id()).unwrap();

    assert_eq!(report.case, case_before);
    assert_eq!(report.case.evidence_hashes(), &evidence_before);
    assert!(yard.verify_evidence(&proof, b"trace"));
    assert!(yard.verify_evidence_chain());
}

#[test]
fn model_repair_recommendations_require_authorized_review() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-repair");
    let genesis = evidence(
        &mut yard,
        "repair-genesis",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "repair-proof",
        b"failure",
        EvidenceKind::PolicyDecision,
    );
    let root_proof = evidence(
        &mut yard,
        "repair-root-proof",
        b"root-cause",
        EvidenceKind::AuthorityDecision,
    );
    enroll(&mut yard, &id, None, genesis);
    let mut adapter = TestAdapter::default();
    yard.record_violation(
        violation(&id, ViolationClass::C, proof.clone(), "repair"),
        &mut adapter,
    )
    .unwrap();
    let mut input = forensic_input(proof);
    input.root_cause = RootCauseFinding {
        status: RootCauseStatus::Hypothesis,
        category: RootCauseCategory::PolicyGap,
        rationale_code: "policy_missed_capability_check".into(),
        evidence: vec![root_proof.clone()],
        reviewer: None,
    };
    let case = yard.open_case(&id, input, 300).unwrap();
    yard.confirm_root_cause(
        case.case_id(),
        RootCauseFinding {
            status: RootCauseStatus::Hypothesis,
            category: RootCauseCategory::PolicyGap,
            rationale_code: "policy_missed_capability_check".into(),
            evidence: vec![root_proof],
            reviewer: None,
        },
        b"authenticated-operator-proof",
        &TestVerifier,
    )
    .unwrap();
    let lesson = yard
        .propose_lesson(
            case.case_id(),
            PolicyReference {
                version: "agent-law/v2".into(),
                hash: hash("agent-law-v2"),
            },
            code(),
            ProposedCorrection {
                code: "add_capability_denylist_regression".into(),
                evidence: vec![case.evidence_hashes().keys().next().unwrap().clone()],
                review_status: ReviewStatus::Pending,
            },
        )
        .unwrap();

    assert_eq!(lesson.review_status, ReviewStatus::Pending);
    let unauthorized = yard.review_lesson(&lesson.lesson_id, true, b"forged-proof", &TestVerifier);
    assert_eq!(
        unauthorized,
        Err(ScrapyardError::DecisionVerificationFailed)
    );
    assert_eq!(
        yard.knowledge()
            .get(&lesson.lesson_id)
            .unwrap()
            .review_status,
        ReviewStatus::Pending
    );
}

#[test]
fn archived_case_remains_traceable_to_original_identity() {
    let mut yard = Scrapyard::new(config());
    let id = agent("agent-archive");
    let genesis = evidence(
        &mut yard,
        "archive-genesis",
        b"genesis",
        EvidenceKind::Configuration,
    );
    let proof = evidence(
        &mut yard,
        "archive-proof",
        b"misconduct",
        EvidenceKind::AuthorityDecision,
    );
    let archive_evidence = evidence(
        &mut yard,
        "archive-record",
        b"archive",
        EvidenceKind::LifecycleEvent,
    );
    enroll(&mut yard, &id, None, genesis.clone());
    let mut adapter = TestAdapter::default();
    terminate(
        &mut yard,
        &id,
        proof.clone(),
        b"authenticated-operator-proof",
        &mut adapter,
    )
    .unwrap();
    let case = yard.open_case(&id, forensic_input(proof), 400).unwrap();
    yard.archive_agent(&id, archive_evidence, 500).unwrap();

    assert_eq!(yard.lifecycle_state(&id).unwrap(), LifecycleState::Archived);
    assert_eq!(yard.case(case.case_id()).unwrap().agent_id(), &id);
    assert!(!yard.is_execution_allowed(&id));
    let reuse = yard.enroll_agent(AgentEnrollment {
        agent_id: id,
        parent_id: None,
        agent_config_hash: hash("archive-reuse"),
        model: model(),
        code_version: code(),
        tool_permissions: BTreeSet::new(),
        genesis_evidence: genesis,
    });
    assert!(matches!(reuse, Err(ScrapyardError::AgentAlreadyExists(_))));
}
