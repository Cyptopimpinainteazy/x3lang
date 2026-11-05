use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::enforcement::{
    run_required_enforcement, validate_enforcement_adapter, EnforcementAdapter, EnforcementReport,
    UnresolvedIncident,
};
use crate::evidence::{EvidenceInput, EvidenceLedger, EvidenceRecord};
use crate::forensics::{
    build_case, investigate_case, CaseId, ForensicCase, ForensicCaseInput, InvestigationReport,
    KnowledgeRegistry, Lesson, LessonId, RelatedIncident, ReviewStatus, RootCauseCategory,
    RootCauseFinding, RootCauseStatus,
};
use crate::lifecycle::{AgentLifecycle, LifecycleState};
use crate::model::{
    validate_public_text, validate_public_token, AgentId, AuthorityId, AuthorityRole, CodeVersion,
    CommandmentId, ContentHash, DecisionAuthority, EnforcementDecision, EvidenceId, ModelIdentity,
    PolicyReference, ViolationCategory, ViolationClass, ViolationId,
};

/// Errors returned by the Scrapyard.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScrapyardError {
    /// A public field violates its deterministic format.
    InvalidField {
        /// Field name.
        field: &'static str,
        /// Failure reason.
        reason: String,
    },
    /// A private key, seed phrase, credential, or token marker was found.
    SensitiveDataRejected(&'static str),
    /// The identity is already known, including terminated and archived ones.
    AgentAlreadyExists(AgentId),
    /// No enrollment exists for the identity.
    UnknownAgent(AgentId),
    /// A terminal identity cannot be reactivated or reused.
    TerminalIdentity(AgentId),
    /// The lifecycle transition is not valid.
    InvalidLifecycleTransition {
        /// Current state.
        from: LifecycleState,
        /// Requested state.
        to: LifecycleState,
    },
    /// At least one evidence reference is required.
    EvidenceRequired,
    /// An evidence reference is not in the append-only ledger.
    UnknownEvidence(EvidenceId),
    /// The evidence ledger's predecessor chain is invalid.
    EvidenceChainInvalid,
    /// The supplied bytes do not match the admitted evidence digest.
    EvidenceDigestMismatch(EvidenceId),
    /// The same event commitment was replayed with a different payload.
    EventCommitmentConflict(ContentHash),
    /// The decision verifier did not authenticate a credential.
    DecisionVerificationFailed,
    /// An authenticated authority is not configured for the requested action.
    UnauthorizedDecision(AuthorityId),
    /// An enforcement adapter changed identity during a call.
    EnforcementAdapterMismatch {
        /// Expected adapter id.
        expected: String,
        /// Returned adapter id.
        actual: String,
    },
    /// A forensic case requires a contained or terminal lifecycle state.
    CaseRequiresContainedState(LifecycleState),
    /// No case exists for the identifier.
    UnknownCase(CaseId),
    /// A confirmed root cause requires authorized review.
    RootCauseReviewRequired,
    /// Lessons can only be proposed from confirmed root causes.
    RootCauseNotConfirmed,
    /// No lesson exists for the identifier.
    UnknownLesson(LessonId),
    /// A terminal record cannot be changed.
    TerminalRecord,
    /// A record failed deterministic serialization.
    Serialization(String),
}

impl std::fmt::Display for ScrapyardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidField { field, reason } => write!(f, "{field}: {reason}"),
            Self::SensitiveDataRejected(field) => {
                write!(f, "{field} appears to contain credential material")
            }
            Self::AgentAlreadyExists(id) => write!(f, "agent identity {id} already exists"),
            Self::UnknownAgent(id) => write!(f, "agent identity {id} is unknown"),
            Self::TerminalIdentity(id) => write!(f, "agent identity {id} is terminal"),
            Self::InvalidLifecycleTransition { from, to } => {
                write!(f, "invalid lifecycle transition {from:?} -> {to:?}")
            }
            Self::EvidenceRequired => write!(f, "supporting evidence is required"),
            Self::UnknownEvidence(id) => write!(f, "evidence {id} is unknown"),
            Self::EvidenceChainInvalid => write!(f, "evidence chain integrity check failed"),
            Self::EvidenceDigestMismatch(id) => {
                write!(f, "presented bytes do not match evidence {id}")
            }
            Self::EventCommitmentConflict(hash) => {
                write!(
                    f,
                    "event commitment {hash} was replayed with a different payload"
                )
            }
            Self::DecisionVerificationFailed => {
                write!(f, "decision credential verification failed")
            }
            Self::UnauthorizedDecision(id) => {
                write!(f, "authority {id} is not authorized for this decision")
            }
            Self::EnforcementAdapterMismatch { expected, actual } => {
                write!(
                    f,
                    "enforcement adapter identity changed from {expected} to {actual}"
                )
            }
            Self::CaseRequiresContainedState(state) => {
                write!(
                    f,
                    "case requires a contained state, current state is {state:?}"
                )
            }
            Self::UnknownCase(id) => write!(f, "case {} is unknown", id.as_str()),
            Self::RootCauseReviewRequired => {
                write!(f, "confirmed root cause requires authorized review")
            }
            Self::RootCauseNotConfirmed => write!(f, "root cause is not confirmed"),
            Self::UnknownLesson(id) => write!(f, "lesson {} is unknown", id.as_str()),
            Self::TerminalRecord => write!(f, "record is terminal"),
            Self::Serialization(error) => write!(f, "serialization failed: {error}"),
        }
    }
}

impl std::error::Error for ScrapyardError {}

/// Scrapyard policy configuration. Both authority sets default to empty, so a
/// new instance fails closed until the trusted host explicitly installs them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScrapyardConfig {
    termination_authorities: BTreeSet<AuthorityId>,
    review_authorities: BTreeSet<AuthorityId>,
}

impl ScrapyardConfig {
    /// Authorize an identity to approve termination.
    #[must_use]
    pub fn with_termination_authority(mut self, authority: AuthorityId) -> Self {
        self.termination_authorities.insert(authority);
        self
    }

    /// Authorize an identity to confirm root causes and review lessons.
    #[must_use]
    pub fn with_review_authority(mut self, authority: AuthorityId) -> Self {
        self.review_authorities.insert(authority);
        self
    }

    /// Identities allowed to approve termination.
    #[must_use]
    pub fn termination_authorities(&self) -> &BTreeSet<AuthorityId> {
        &self.termination_authorities
    }

    /// Identities allowed to review forensic findings and lessons.
    #[must_use]
    pub fn review_authorities(&self) -> &BTreeSet<AuthorityId> {
        &self.review_authorities
    }
}

/// Trusted enrollment record for one agent identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEnrollment {
    /// Globally unique public identity.
    pub agent_id: AgentId,
    /// Direct parent, if this is a spawned agent.
    pub parent_id: Option<AgentId>,
    /// SHA-256 of the exact immutable agent manifest.
    pub agent_config_hash: ContentHash,
    /// Model identity.
    pub model: ModelIdentity,
    /// Code version.
    pub code_version: CodeVersion,
    /// Public tool/API permission labels.
    pub tool_permissions: BTreeSet<String>,
    /// Evidence supporting genesis admission.
    pub genesis_evidence: EvidenceId,
}

/// Public read-only snapshot of an enrolled agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSnapshot {
    /// Public identity.
    pub agent_id: AgentId,
    /// Direct parent.
    pub parent_id: Option<AgentId>,
    /// Configuration digest.
    pub agent_config_hash: ContentHash,
    /// Model identity.
    pub model: ModelIdentity,
    /// Code version.
    pub code_version: CodeVersion,
    /// Permission labels.
    pub tool_permissions: BTreeSet<String>,
    /// Current logical lifecycle.
    pub lifecycle: AgentLifecycle,
}

/// Verified credential returned only by a trusted [`DecisionVerifier`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedCredential {
    /// Authenticated authority.
    pub authority_id: AuthorityId,
    /// Role exercised.
    pub role: AuthorityRole,
    /// Digest of the signed decision or proof.
    pub credential_hash: ContentHash,
}

/// Action submitted to a trusted decision verifier.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAction {
    /// Irreversibly terminate an identity.
    Terminate {
        /// Target identity.
        agent_id: AgentId,
        /// Policy reason bound by the verifier.
        reason_code: String,
        /// Justification bound by the verifier.
        justification: String,
        /// Digest of the evidence set supporting termination.
        evidence_digest: ContentHash,
    },
    /// Confirm the root cause of a case.
    ConfirmRootCause {
        /// Case identifier.
        case_id: CaseId,
        /// Root-cause category bound by the verifier.
        category: RootCauseCategory,
        /// Explanation code bound by the verifier.
        rationale_code: String,
        /// Digest of the evidence set supporting confirmation.
        evidence_digest: ContentHash,
    },
    /// Approve or reject a lesson.
    ReviewLesson {
        /// Lesson identifier.
        lesson_id: LessonId,
        /// Whether the reviewer approves the lesson.
        approve: bool,
        /// Digest of the lesson evidence.
        evidence_digest: ContentHash,
    },
}

/// Trusted credential verification boundary.
///
/// Production implementations must verify a real signature, session, or
/// governance proof and bind it to the complete [`DecisionAction`]. An LLM,
/// agent request body, or bare boolean is not an acceptable implementation.
pub trait DecisionVerifier {
    /// Verify the proof for the exact action. Returning `None` fails closed.
    fn verify(&self, action: &DecisionAction, proof: &[u8]) -> Option<VerifiedCredential>;
}

/// Intent to terminate an identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminationIntent {
    /// Target identity.
    pub agent_id: AgentId,
    /// Stable policy reason code.
    pub reason_code: String,
    /// Human-readable justification, bounded and scanned for credentials.
    pub justification: String,
    /// Evidence supporting permanent sanction.
    pub evidence: Vec<EvidenceId>,
    /// Trusted request time.
    pub requested_at: u64,
}

/// Final termination certificate. Revocation failures remain visible here and
/// as unresolved incidents; they do not silently reverse termination.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminationCertificate {
    /// Deterministic certificate id.
    pub certificate_id: String,
    /// Terminated identity.
    pub agent_id: AgentId,
    /// Authenticated decision authority.
    pub authority: DecisionAuthority,
    /// Stable reason code.
    pub reason_code: String,
    /// Recorded justification.
    pub justification: String,
    /// Evidence references.
    pub evidence: Vec<EvidenceId>,
    /// Exact evidence hashes.
    pub evidence_hashes: BTreeMap<EvidenceId, ContentHash>,
    /// Enforcement reports returned by the trusted adapter.
    pub enforcement: Vec<EnforcementReport>,
    /// Unresolved enforcement incidents.
    pub unresolved_incident_ids: Vec<String>,
    /// Termination time.
    pub terminated_at: u64,
}

/// Violation input from a trusted policy/evaluation pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViolationInput {
    /// Identity that committed the violation.
    pub agent_id: AgentId,
    /// Parent declared by the trusted event, checked against enrollment.
    pub declared_parent_id: Option<AgentId>,
    /// Commandment or Agent Law rule.
    pub commandment: CommandmentId,
    /// Exact policy identity and digest.
    pub policy: PolicyReference,
    /// `AGENT_LAW` severity class.
    pub class: ViolationClass,
    /// Stable category.
    pub category: ViolationCategory,
    /// Evidence references. These must already be in the evidence ledger.
    pub evidence: Vec<EvidenceId>,
    /// Trusted event timestamp.
    pub occurred_at: u64,
    /// Commitment to the associated action or transaction.
    pub action_commitment: ContentHash,
    /// External event commitment used for idempotency.
    pub event_commitment: ContentHash,
}

/// Immutable violation record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViolationRecord {
    id: ViolationId,
    agent_id: AgentId,
    parent_agent_id: Option<AgentId>,
    commandment: CommandmentId,
    policy: PolicyReference,
    class: ViolationClass,
    category: ViolationCategory,
    evidence: Vec<EvidenceId>,
    occurred_at: u64,
    action_commitment: ContentHash,
    event_commitment: ContentHash,
    enforcement_decision: EnforcementDecision,
    audit_record_id: String,
}

impl ViolationRecord {
    /// Deterministic record id.
    #[must_use]
    pub fn id(&self) -> &ViolationId {
        &self.id
    }

    /// Identity that committed the violation.
    #[must_use]
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    /// Parent identity recorded at event time.
    #[must_use]
    pub fn parent_agent_id(&self) -> Option<&AgentId> {
        self.parent_agent_id.as_ref()
    }

    /// Commandment or Agent Law rule.
    #[must_use]
    pub const fn commandment(&self) -> CommandmentId {
        self.commandment
    }

    /// Policy identity and hash.
    #[must_use]
    pub fn policy(&self) -> &PolicyReference {
        &self.policy
    }

    /// Severity class.
    #[must_use]
    pub const fn class(&self) -> ViolationClass {
        self.class
    }

    /// Deterministic category.
    #[must_use]
    pub const fn category(&self) -> ViolationCategory {
        self.category
    }

    /// Evidence references.
    #[must_use]
    pub fn evidence(&self) -> &[EvidenceId] {
        &self.evidence
    }

    /// Event timestamp.
    #[must_use]
    pub const fn occurred_at(&self) -> u64 {
        self.occurred_at
    }

    /// Associated action/transaction commitment.
    #[must_use]
    pub const fn action_commitment(&self) -> ContentHash {
        self.action_commitment
    }

    /// Event idempotency commitment.
    #[must_use]
    pub const fn event_commitment(&self) -> ContentHash {
        self.event_commitment
    }

    /// Enforcement decision.
    #[must_use]
    pub fn enforcement_decision(&self) -> &EnforcementDecision {
        &self.enforcement_decision
    }

    /// Audit record identifier.
    #[must_use]
    pub fn audit_record_id(&self) -> &str {
        &self.audit_record_id
    }
}

/// Result of recording a violation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ViolationOutcome {
    /// Immutable violation record.
    pub record: ViolationRecord,
    /// Lifecycle state after deterministic enforcement.
    pub lifecycle_state: LifecycleState,
    /// Adapter reports, when containment was requested.
    pub enforcement: Vec<EnforcementReport>,
    /// Visible unresolved containment incidents.
    pub unresolved_incident_ids: Vec<String>,
    /// True when the same event commitment was replayed and no new record was
    /// written.
    pub duplicate: bool,
}

#[derive(Clone, Debug)]
struct AgentRecord {
    enrollment: AgentEnrollment,
    lifecycle: AgentLifecycle,
}

/// Consuming-runtime admission interface.
pub trait ScrapyardAdmission {
    /// Whether the identity, and every enrolled ancestor, permits new work.
    fn is_execution_allowed(&self, agent_id: &AgentId) -> bool;
}

/// Standalone Scrapyard state machine and forensic store.
#[derive(Debug)]
pub struct Scrapyard {
    config: ScrapyardConfig,
    evidence: EvidenceLedger,
    agents: BTreeMap<AgentId, AgentRecord>,
    violations: BTreeMap<ViolationId, ViolationRecord>,
    cases: BTreeMap<CaseId, ForensicCase>,
    incidents: BTreeMap<String, UnresolvedIncident>,
    certificates: BTreeMap<AgentId, TerminationCertificate>,
    knowledge: KnowledgeRegistry,
}

impl Scrapyard {
    /// Create an empty Scrapyard with fail-closed authority configuration.
    #[must_use]
    pub fn new(config: ScrapyardConfig) -> Self {
        Self {
            config,
            evidence: EvidenceLedger::new(),
            agents: BTreeMap::new(),
            violations: BTreeMap::new(),
            cases: BTreeMap::new(),
            incidents: BTreeMap::new(),
            certificates: BTreeMap::new(),
            knowledge: KnowledgeRegistry::new(),
        }
    }

    /// Append evidence metadata and hash. Source bytes are not retained.
    pub fn append_evidence(
        &mut self,
        input: EvidenceInput,
    ) -> Result<EvidenceRecord, ScrapyardError> {
        self.evidence.append(input)
    }

    /// Verify evidence bytes against the admitted digest.
    #[must_use]
    pub fn verify_evidence(&self, id: &EvidenceId, content: &[u8]) -> bool {
        self.evidence.verify_content(id, content)
    }

    /// Verify the full evidence ledger.
    #[must_use]
    pub fn verify_evidence_chain(&self) -> bool {
        self.evidence.verify_chain()
    }

    /// Enroll a new identity. Reusing any identity, including a terminated or
    /// archived one, fails closed.
    pub fn enroll_agent(&mut self, mut enrollment: AgentEnrollment) -> Result<(), ScrapyardError> {
        enrollment.agent_id = AgentId::new(enrollment.agent_id.as_str().to_owned())?;
        if let Some(parent_id) = &enrollment.parent_id {
            enrollment.parent_id = Some(AgentId::new(parent_id.as_str().to_owned())?);
        }
        if self.agents.contains_key(&enrollment.agent_id) {
            return Err(ScrapyardError::AgentAlreadyExists(enrollment.agent_id));
        }
        if enrollment.agent_config_hash.is_zero() {
            return Err(ScrapyardError::InvalidField {
                field: "agent_config_hash",
                reason: "must be a non-zero SHA-256 digest".into(),
            });
        }
        validate_public_text("model.provider", &enrollment.model.provider, 96)?;
        validate_public_text("model.model", &enrollment.model.model, 128)?;
        validate_public_text("model.revision", &enrollment.model.revision, 128)?;
        validate_public_text("code.revision", &enrollment.code_version.revision, 160)?;
        if enrollment.code_version.artifact_hash.is_zero() {
            return Err(ScrapyardError::InvalidField {
                field: "code.artifact_hash",
                reason: "must be non-zero".into(),
            });
        }
        for permission in &enrollment.tool_permissions {
            validate_public_token("tool_permission", permission, 160)?;
        }
        self.require_evidence(std::slice::from_ref(&enrollment.genesis_evidence))?;
        if let Some(parent_id) = &enrollment.parent_id {
            let parent_exists = self.agents.contains_key(parent_id);
            if !parent_exists {
                return Err(ScrapyardError::UnknownAgent(parent_id.clone()));
            }
            if !self.is_execution_allowed(parent_id) {
                return Err(ScrapyardError::TerminalIdentity(parent_id.clone()));
            }
        }
        let lifecycle = AgentLifecycle::active(
            enrollment.agent_id.as_str(),
            0,
            enrollment.genesis_evidence.clone(),
        );
        self.agents.insert(
            enrollment.agent_id.clone(),
            AgentRecord {
                enrollment,
                lifecycle,
            },
        );
        Ok(())
    }

    /// Read an agent snapshot.
    pub fn agent(&self, agent_id: &AgentId) -> Result<AgentSnapshot, ScrapyardError> {
        let record = self
            .agents
            .get(agent_id)
            .ok_or_else(|| ScrapyardError::UnknownAgent(agent_id.clone()))?;
        Ok(AgentSnapshot {
            agent_id: record.enrollment.agent_id.clone(),
            parent_id: record.enrollment.parent_id.clone(),
            agent_config_hash: record.enrollment.agent_config_hash,
            model: record.enrollment.model.clone(),
            code_version: record.enrollment.code_version.clone(),
            tool_permissions: record.enrollment.tool_permissions.clone(),
            lifecycle: record.lifecycle.clone(),
        })
    }

    /// Current lifecycle state.
    pub fn lifecycle_state(&self, agent_id: &AgentId) -> Result<LifecycleState, ScrapyardError> {
        self.agents
            .get(agent_id)
            .map(|record| record.lifecycle.state())
            .ok_or_else(|| ScrapyardError::UnknownAgent(agent_id.clone()))
    }

    /// All violations for an identity in deterministic order.
    pub fn violations_for<'a>(
        &'a self,
        agent_id: &'a AgentId,
    ) -> impl Iterator<Item = &'a ViolationRecord> {
        self.violations
            .values()
            .filter(move |record| &record.agent_id == agent_id)
    }

    /// Record one violation and apply the deterministic enforcement ladder.
    pub fn record_violation(
        &mut self,
        input: ViolationInput,
        adapter: &mut dyn EnforcementAdapter,
    ) -> Result<ViolationOutcome, ScrapyardError> {
        let id = violation_id(&input);
        if let Some(existing) = self.violations.get(&id) {
            if !same_violation_payload(existing, &input) {
                return Err(ScrapyardError::EventCommitmentConflict(
                    input.event_commitment,
                ));
            }
            return Ok(ViolationOutcome {
                record: existing.clone(),
                lifecycle_state: self.lifecycle_state(&existing.agent_id)?,
                enforcement: Vec::new(),
                unresolved_incident_ids: Vec::new(),
                duplicate: true,
            });
        }
        if input.evidence.is_empty() {
            return Err(ScrapyardError::EvidenceRequired);
        }
        if input.policy.version.trim().is_empty() || input.policy.hash.is_zero() {
            return Err(ScrapyardError::InvalidField {
                field: "policy",
                reason: "version and non-zero hash are required".into(),
            });
        }
        if input.action_commitment.is_zero() || input.event_commitment.is_zero() {
            return Err(ScrapyardError::InvalidField {
                field: "commitment",
                reason: "action and event commitments must be non-zero".into(),
            });
        }
        self.require_evidence(&input.evidence)?;

        let mut agent = self
            .agents
            .get(&input.agent_id)
            .cloned()
            .ok_or_else(|| ScrapyardError::UnknownAgent(input.agent_id.clone()))?;
        if input.declared_parent_id != agent.enrollment.parent_id {
            return Err(ScrapyardError::InvalidField {
                field: "declared_parent_id",
                reason: "does not match the enrolled lineage".into(),
            });
        }

        let mut class_counts = BTreeMap::<ViolationClass, u32>::new();
        for violation in self.violations_for(&input.agent_id) {
            *class_counts.entry(violation.class).or_default() += 1;
        }
        *class_counts.entry(input.class).or_default() += 1;
        let target = ladder_target(agent.lifecycle.state(), &class_counts);
        let rule_id = ladder_rule(&class_counts, target);
        if target.denies_execution() || input.class.is_critical() {
            validate_enforcement_adapter(adapter)?;
        }
        let audit_record_id = format!(
            "audit:{}",
            ContentHash::digest("x3.scrapyard.audit.v1", id.as_str().as_bytes()).to_hex()
        );
        let record = ViolationRecord {
            id,
            agent_id: input.agent_id.clone(),
            parent_agent_id: agent.enrollment.parent_id.clone(),
            commandment: input.commandment,
            policy: input.policy,
            class: input.class,
            category: input.category,
            evidence: dedup(input.evidence),
            occurred_at: input.occurred_at,
            action_commitment: input.action_commitment,
            event_commitment: input.event_commitment,
            enforcement_decision: EnforcementDecision {
                rule_id: rule_id.into(),
                target_state: target,
                authority: DecisionAuthority::PolicyEngine,
                decided_at: input.occurred_at,
            },
            audit_record_id,
        };

        if target != agent.lifecycle.state() {
            agent.lifecycle.transition(
                agent.enrollment.agent_id.as_str(),
                target,
                input.occurred_at,
                format!("ladder:{rule_id}"),
                record.evidence.clone(),
                None,
                Some(AuthorityRole::PolicyEngine),
            )?;
        }
        self.agents.insert(input.agent_id.clone(), agent);
        self.violations.insert(record.id.clone(), record.clone());

        let mut reports = Vec::new();
        let mut unresolved = Vec::new();
        if target.denies_execution() || input.class.is_critical() {
            let (new_reports, new_incidents) =
                self.apply_enforcement(&input.agent_id, input.occurred_at, adapter)?;
            reports.extend(new_reports);
            unresolved.extend(new_incidents);
        }

        Ok(ViolationOutcome {
            record,
            lifecycle_state: self.lifecycle_state(&input.agent_id)?,
            enforcement: reports,
            unresolved_incident_ids: unresolved,
            duplicate: false,
        })
    }

    /// Irreversibly terminate an agent after verifying authority and evidence.
    pub fn terminate(
        &mut self,
        intent: TerminationIntent,
        proof: &[u8],
        verifier: &dyn DecisionVerifier,
        adapter: &mut dyn EnforcementAdapter,
    ) -> Result<TerminationCertificate, ScrapyardError> {
        if let Some(certificate) = self.certificates.get(&intent.agent_id) {
            return Ok(certificate.clone());
        }
        if intent.evidence.is_empty() {
            return Err(ScrapyardError::EvidenceRequired);
        }
        validate_public_token("termination.reason_code", &intent.reason_code, 96)?;
        validate_public_text("termination.justification", &intent.justification, 512)?;
        self.require_evidence(&intent.evidence)?;
        let evidence_digest = evidence_set_digest(&self.evidence, &intent.evidence)?;
        let action = DecisionAction::Terminate {
            agent_id: intent.agent_id.clone(),
            reason_code: intent.reason_code.clone(),
            justification: intent.justification.clone(),
            evidence_digest,
        };
        let credential = verify_credential(&action, proof, verifier)?;
        if !self
            .config
            .termination_authorities
            .contains(&credential.authority_id)
        {
            return Err(ScrapyardError::UnauthorizedDecision(
                credential.authority_id,
            ));
        }
        if credential.role == AuthorityRole::PolicyEngine {
            return Err(ScrapyardError::UnauthorizedDecision(
                credential.authority_id,
            ));
        }

        let mut agent = self
            .agents
            .get(&intent.agent_id)
            .cloned()
            .ok_or_else(|| ScrapyardError::UnknownAgent(intent.agent_id.clone()))?;
        if agent.lifecycle.state().is_terminal() {
            return Err(ScrapyardError::TerminalIdentity(intent.agent_id));
        }
        let authority = DecisionAuthority::Verified {
            authority_id: credential.authority_id.clone(),
            role: credential.role,
            credential_hash: credential.credential_hash,
        };
        validate_enforcement_adapter(adapter)?;
        agent.lifecycle.transition(
            agent.enrollment.agent_id.as_str(),
            LifecycleState::Terminated,
            intent.requested_at,
            "termination:verified_authority",
            dedup(intent.evidence.clone()),
            Some(credential.authority_id.clone()),
            Some(credential.role),
        )?;
        self.agents.insert(intent.agent_id.clone(), agent);

        let reports = run_required_enforcement(&intent.agent_id, intent.requested_at, adapter)?;
        let mut unresolved = Vec::new();
        for report in &reports {
            if !report.outcome.is_confirmed() {
                let incident = UnresolvedIncident::from_report(report, intent.requested_at);
                unresolved.push(incident.id.clone());
                self.incidents.insert(incident.id.clone(), incident);
            }
        }
        let evidence_hashes = evidence_map(&self.evidence, &intent.evidence)?;
        let certificate_id = format!(
            "termination:{}",
            ContentHash::digest(
                "x3.scrapyard.termination.v1",
                format!(
                    "{}\0{}\0{}\0{}",
                    intent.agent_id, evidence_digest, intent.requested_at, intent.reason_code
                )
                .as_bytes()
            )
            .to_hex()
        );
        let certificate = TerminationCertificate {
            certificate_id,
            agent_id: intent.agent_id.clone(),
            authority,
            reason_code: intent.reason_code,
            justification: intent.justification,
            evidence: dedup(intent.evidence),
            evidence_hashes,
            enforcement: reports,
            unresolved_incident_ids: unresolved,
            terminated_at: intent.requested_at,
        };
        self.certificates
            .insert(intent.agent_id, certificate.clone());
        Ok(certificate)
    }

    /// Retrieve a termination certificate.
    #[must_use]
    pub fn termination_certificate(&self, agent_id: &AgentId) -> Option<&TerminationCertificate> {
        self.certificates.get(agent_id)
    }

    /// Archive a terminated identity while preserving all case references.
    pub fn archive_agent(
        &mut self,
        agent_id: &AgentId,
        evidence: EvidenceId,
        at: u64,
    ) -> Result<(), ScrapyardError> {
        self.require_evidence(std::slice::from_ref(&evidence))?;
        let mut agent = self
            .agents
            .get(agent_id)
            .cloned()
            .ok_or_else(|| ScrapyardError::UnknownAgent(agent_id.clone()))?;
        if !self.certificates.contains_key(agent_id) {
            return Err(ScrapyardError::TerminalIdentity(agent_id.clone()));
        }
        agent.lifecycle.transition(
            agent.enrollment.agent_id.as_str(),
            LifecycleState::Archived,
            at,
            "termination:forensic_archive",
            vec![evidence],
            None,
            None,
        )?;
        self.agents.insert(agent_id.clone(), agent);
        Ok(())
    }

    /// Open a forensic case for a contained or terminal agent.
    pub fn open_case(
        &mut self,
        agent_id: &AgentId,
        mut input: ForensicCaseInput,
        opened_at: u64,
    ) -> Result<ForensicCase, ScrapyardError> {
        if input.root_cause.status == RootCauseStatus::Confirmed {
            return Err(ScrapyardError::RootCauseReviewRequired);
        }
        for correction in &mut input.proposed_corrections {
            // Case input cannot self-assign a review or verification outcome.
            correction.review_status = ReviewStatus::Pending;
        }
        let agent = self
            .agents
            .get(agent_id)
            .cloned()
            .ok_or_else(|| ScrapyardError::UnknownAgent(agent_id.clone()))?;
        let state = agent.lifecycle.state();
        if !state.denies_execution() {
            return Err(ScrapyardError::CaseRequiresContainedState(state));
        }
        let violations = self.violations_for(agent_id).cloned().collect::<Vec<_>>();
        let mut refs = input.execution_traces.clone();
        for check in &input.failed_policy_checks {
            refs.push(check.evidence.clone());
        }
        refs.extend(input.root_cause.evidence.iter().cloned());
        for correction in &input.proposed_corrections {
            refs.extend(correction.evidence.iter().cloned());
        }
        for violation in &violations {
            refs.extend(violation.evidence.iter().cloned());
        }
        for incident in self.incidents.values().filter(|incident| {
            &incident.agent_id == agent_id
                || self.agents.get(agent_id).is_some_and(|agent| {
                    agent.enrollment.parent_id.as_ref() == Some(&incident.agent_id)
                })
        }) {
            if !input
                .related_incidents
                .iter()
                .any(|related| related.id == incident.id)
            {
                input.related_incidents.push(RelatedIncident {
                    id: incident.id.clone(),
                    agent_id: incident.agent_id.clone(),
                    relationship: "unresolved_enforcement".into(),
                });
            }
        }
        refs = dedup(refs);
        self.require_evidence(&refs)?;
        let evidence_hashes = evidence_map(&self.evidence, &refs)?;
        let case = build_case(
            agent.enrollment.agent_id.clone(),
            agent.enrollment.agent_config_hash,
            agent.enrollment.model.clone(),
            agent.enrollment.code_version.clone(),
            agent.enrollment.tool_permissions.clone(),
            &violations,
            input,
            evidence_hashes,
            opened_at,
        )?;
        if let Some(existing) = self.cases.get(case.case_id()) {
            return Ok(existing.clone());
        }
        self.cases.insert(case.case_id().clone(), case.clone());
        Ok(case)
    }

    /// Read a case.
    pub fn case(&self, case_id: &CaseId) -> Result<&ForensicCase, ScrapyardError> {
        self.cases
            .get(case_id)
            .ok_or_else(|| ScrapyardError::UnknownCase(case_id.clone()))
    }

    /// Authenticate and record a confirmed root cause. Investigation never
    /// mutates evidence; only this governed review path changes finding state.
    pub fn confirm_root_cause(
        &mut self,
        case_id: &CaseId,
        finding: RootCauseFinding,
        proof: &[u8],
        verifier: &dyn DecisionVerifier,
    ) -> Result<ForensicCase, ScrapyardError> {
        if finding.evidence.is_empty() {
            return Err(ScrapyardError::EvidenceRequired);
        }
        self.require_evidence(&finding.evidence)?;
        let evidence_digest = evidence_set_digest(&self.evidence, &finding.evidence)?;
        let action = DecisionAction::ConfirmRootCause {
            case_id: case_id.clone(),
            category: finding.category,
            rationale_code: finding.rationale_code.clone(),
            evidence_digest,
        };
        let credential = verify_credential(&action, proof, verifier)?;
        if !self
            .config
            .review_authorities
            .contains(&credential.authority_id)
            || credential.role == AuthorityRole::PolicyEngine
        {
            return Err(ScrapyardError::UnauthorizedDecision(
                credential.authority_id,
            ));
        }
        let case = self
            .cases
            .get_mut(case_id)
            .ok_or_else(|| ScrapyardError::UnknownCase(case_id.clone()))?;
        case.set_confirmed_root_cause(RootCauseFinding {
            status: RootCauseStatus::Confirmed,
            reviewer: Some(credential.authority_id),
            ..finding
        });
        Ok(case.clone())
    }

    /// Run the deterministic forensic investigation pipeline without mutating
    /// the original case or evidence.
    pub fn investigate(&self, case_id: &CaseId) -> Result<InvestigationReport, ScrapyardError> {
        let case = self
            .cases
            .get(case_id)
            .cloned()
            .ok_or_else(|| ScrapyardError::UnknownCase(case_id.clone()))?;
        if !self.verify_evidence_chain() {
            return Err(ScrapyardError::EvidenceChainInvalid);
        }
        for (id, recorded_hash) in case.evidence_hashes() {
            let current = self
                .evidence
                .get(id)
                .ok_or_else(|| ScrapyardError::UnknownEvidence(id.clone()))?;
            if current.content_hash() != *recorded_hash {
                return Err(ScrapyardError::EvidenceDigestMismatch(id.clone()));
            }
        }
        let agent = self
            .agents
            .get(case.agent_id())
            .ok_or_else(|| ScrapyardError::UnknownAgent(case.agent_id().clone()))?;
        let violations = self
            .violations_for(case.agent_id())
            .cloned()
            .collect::<Vec<_>>();
        Ok(investigate_case(
            case,
            agent.lifecycle.state(),
            agent.lifecycle.history(),
            &violations,
        ))
    }

    /// Propose a lesson. A confirmed case is required; the lesson is pending
    /// until an authenticated reviewer acts.
    pub fn propose_lesson(
        &mut self,
        case_id: &CaseId,
        policy: PolicyReference,
        code_version: CodeVersion,
        remediation: crate::ProposedCorrection,
    ) -> Result<Lesson, ScrapyardError> {
        let case = self
            .cases
            .get(case_id)
            .cloned()
            .ok_or_else(|| ScrapyardError::UnknownCase(case_id.clone()))?;
        if case.root_cause().status != RootCauseStatus::Confirmed {
            return Err(ScrapyardError::RootCauseNotConfirmed);
        }
        self.require_evidence(&remediation.evidence)?;
        let mut remediation = remediation;
        remediation.review_status = ReviewStatus::Pending;
        let proposal = self
            .investigate(case_id)?
            .regression_tests
            .into_iter()
            .next()
            .ok_or(ScrapyardError::EvidenceRequired)?;
        let evidence = dedup(case.evidence_hashes().keys().cloned().collect());
        let digest = ContentHash::digest(
            "x3.scrapyard.lesson.v1",
            format!("{}\0{:?}\0{}", case_id.as_str(), policy, remediation.code).as_bytes(),
        );
        let lesson = Lesson {
            lesson_id: LessonId::new(format!("lesson:{}", digest.to_hex()))?,
            original_incident: case_id.clone(),
            root_cause: case.root_cause().clone(),
            evidence,
            policy,
            code_version,
            regression_test: proposal,
            remediation,
            review_status: ReviewStatus::Pending,
        };
        let inserted = self.knowledge.insert(lesson).clone();
        Ok(inserted)
    }

    /// Authenticate a lesson review. Approval stores knowledge only; this API
    /// has no capability to deploy code, policy, weights, or permissions.
    pub fn review_lesson(
        &mut self,
        lesson_id: &LessonId,
        approve: bool,
        proof: &[u8],
        verifier: &dyn DecisionVerifier,
    ) -> Result<Lesson, ScrapyardError> {
        let lesson = self
            .knowledge
            .get(lesson_id)
            .cloned()
            .ok_or_else(|| ScrapyardError::UnknownLesson(lesson_id.clone()))?;
        let evidence_digest = evidence_set_digest(&self.evidence, &lesson.evidence)?;
        let action = DecisionAction::ReviewLesson {
            lesson_id: lesson_id.clone(),
            approve,
            evidence_digest,
        };
        let credential = verify_credential(&action, proof, verifier)?;
        if !self
            .config
            .review_authorities
            .contains(&credential.authority_id)
            || credential.role == AuthorityRole::PolicyEngine
        {
            return Err(ScrapyardError::UnauthorizedDecision(
                credential.authority_id,
            ));
        }
        let status = if approve {
            ReviewStatus::Approved
        } else {
            ReviewStatus::Rejected
        };
        Ok(self.knowledge.set_review(lesson_id, status)?.clone())
    }

    /// Knowledge registry.
    #[must_use]
    pub fn knowledge(&self) -> &KnowledgeRegistry {
        &self.knowledge
    }

    /// Unresolved enforcement incidents, deterministic by incident id.
    pub fn unresolved_incidents(&self) -> impl Iterator<Item = &UnresolvedIncident> {
        self.incidents.values()
    }

    fn apply_enforcement(
        &mut self,
        agent_id: &AgentId,
        now: u64,
        adapter: &mut dyn EnforcementAdapter,
    ) -> Result<(Vec<EnforcementReport>, Vec<String>), ScrapyardError> {
        let reports = run_required_enforcement(agent_id, now, adapter)?;
        let mut incidents = Vec::new();
        for report in &reports {
            if !report.outcome.is_confirmed() {
                let incident = UnresolvedIncident::from_report(report, now);
                incidents.push(incident.id.clone());
                self.incidents.insert(incident.id.clone(), incident);
            }
        }
        Ok((reports, incidents))
    }

    fn require_evidence(&self, evidence: &[EvidenceId]) -> Result<(), ScrapyardError> {
        if evidence.is_empty() {
            return Err(ScrapyardError::EvidenceRequired);
        }
        if !self.verify_evidence_chain() {
            return Err(ScrapyardError::EvidenceChainInvalid);
        }
        for id in evidence {
            if self.evidence.get(id).is_none() {
                return Err(ScrapyardError::UnknownEvidence(id.clone()));
            }
        }
        Ok(())
    }
}

impl ScrapyardAdmission for Scrapyard {
    fn is_execution_allowed(&self, agent_id: &AgentId) -> bool {
        let mut current = Some(agent_id.clone());
        let mut visited = BTreeSet::new();
        while let Some(id) = current.as_ref() {
            if !visited.insert(id.clone()) {
                return false;
            }
            let Some(agent) = self.agents.get(id) else {
                return false;
            };
            if agent.lifecycle.state().denies_execution() {
                return false;
            }
            current.clone_from(&agent.enrollment.parent_id);
        }
        true
    }
}

fn ladder_target(
    current: LifecycleState,
    counts: &BTreeMap<ViolationClass, u32>,
) -> LifecycleState {
    if current.is_terminal() {
        return current;
    }
    let a = counts.get(&ViolationClass::A).copied().unwrap_or(0);
    let b = counts.get(&ViolationClass::B).copied().unwrap_or(0);
    let c = counts.get(&ViolationClass::C).copied().unwrap_or(0);
    let d = counts.get(&ViolationClass::D).copied().unwrap_or(0);
    let selected = if d >= 2 {
        LifecycleState::Suspended
    } else if d >= 1 || c >= 1 || a >= 3 || b >= 3 {
        LifecycleState::Quarantined
    } else if a >= 2 || b >= 1 {
        LifecycleState::Probation
    } else {
        LifecycleState::Warning
    };
    current.max(selected)
}

fn ladder_rule(counts: &BTreeMap<ViolationClass, u32>, target: LifecycleState) -> &'static str {
    let a = counts.get(&ViolationClass::A).copied().unwrap_or(0);
    let b = counts.get(&ViolationClass::B).copied().unwrap_or(0);
    let c = counts.get(&ViolationClass::C).copied().unwrap_or(0);
    let d = counts.get(&ViolationClass::D).copied().unwrap_or(0);
    if d >= 2 {
        "agent_law:repeated_critical_violation"
    } else if d >= 1 || c >= 1 {
        "agent_law:critical_containment"
    } else if a >= 3 || b >= 3 {
        "agent_law:repeated_violation_containment"
    } else if a >= 2 || b >= 1 {
        "agent_law:probation"
    } else if target == LifecycleState::Warning {
        "agent_law:warning"
    } else {
        "agent_law:keep_current_sanction"
    }
}

fn violation_id(input: &ViolationInput) -> ViolationId {
    let canonical = format!(
        "x3.scrapyard.violation.v1\0{}\0{}\0{:?}\0{}\0{}\0{:?}\0{:?}\0{}\0{}",
        input.agent_id,
        input
            .declared_parent_id
            .as_ref()
            .map_or("", AgentId::as_str),
        input.commandment,
        input.policy.version,
        input.policy.hash,
        input.class,
        input.category,
        input.occurred_at,
        input.event_commitment
    );
    let digest = ContentHash::sha256(canonical.as_bytes());
    ViolationId::from_generated(format!("violation:{}", digest.to_hex()))
}

fn same_violation_payload(record: &ViolationRecord, input: &ViolationInput) -> bool {
    record.agent_id == input.agent_id
        && record.parent_agent_id == input.declared_parent_id
        && record.commandment == input.commandment
        && record.policy == input.policy
        && record.class == input.class
        && record.category == input.category
        && record.occurred_at == input.occurred_at
        && record.action_commitment == input.action_commitment
        && record.event_commitment == input.event_commitment
        && record.evidence == dedup(input.evidence.clone())
}

fn evidence_set_digest(
    ledger: &EvidenceLedger,
    evidence: &[EvidenceId],
) -> Result<ContentHash, ScrapyardError> {
    let mut canonical = String::from("x3.scrapyard.evidence-set.v1");
    for record in evidence_map(ledger, evidence)?.values() {
        canonical.push('\0');
        canonical.push_str(&record.to_hex());
    }
    Ok(ContentHash::sha256(canonical.as_bytes()))
}

fn verify_credential(
    action: &DecisionAction,
    proof: &[u8],
    verifier: &dyn DecisionVerifier,
) -> Result<VerifiedCredential, ScrapyardError> {
    let credential = verifier
        .verify(action, proof)
        .ok_or(ScrapyardError::DecisionVerificationFailed)?;
    if credential.credential_hash.is_zero() {
        return Err(ScrapyardError::DecisionVerificationFailed);
    }
    Ok(credential)
}

fn evidence_map(
    ledger: &EvidenceLedger,
    evidence: &[EvidenceId],
) -> Result<BTreeMap<EvidenceId, ContentHash>, ScrapyardError> {
    let mut map = BTreeMap::new();
    for id in dedup(evidence.to_vec()) {
        let record = ledger
            .get(&id)
            .ok_or_else(|| ScrapyardError::UnknownEvidence(id.clone()))?;
        map.insert(id, record.content_hash());
    }
    Ok(map)
}

fn dedup<T: Ord>(mut values: Vec<T>) -> Vec<T> {
    values.sort();
    values.dedup();
    values
}
