use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

use crate::lifecycle::{LifecycleState, LifecycleTransition};
use crate::model::{
    validate_public_text, validate_public_token, AgentId, AuthorityId, CodeVersion, ContentHash,
    EvidenceId, ModelIdentity, PolicyReference, ViolationCategory, ViolationId,
};
use crate::scrapyard::ViolationRecord;
use crate::ScrapyardError;

/// Stable forensic case identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CaseId(String);

impl CaseId {
    /// Validate and construct a case id.
    pub fn new(value: impl Into<String>) -> Result<Self, ScrapyardError> {
        let value = value.into();
        validate_public_token("case_id", &value, 160)?;
        Ok(Self(value))
    }

    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for CaseId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CaseId({})", self.0)
    }
}

/// Root-cause evidence status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootCauseStatus {
    /// No defensible root cause yet.
    Unknown,
    /// Supported hypothesis, not proven.
    Hypothesis,
    /// Independently reviewed and confirmed.
    Confirmed,
}

/// Deterministic root-cause category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootCauseCategory {
    /// Model produced unsafe or manipulated output.
    ModelFailure,
    /// Policy did not cover the observed path.
    PolicyGap,
    /// Operator or governance process failed.
    OperatorError,
    /// Outside actor influenced the agent.
    ExternalActor,
    /// Tool or runtime adapter failed.
    ToolFailure,
    /// Implementation defect.
    CodeDefect,
    /// Category cannot yet be established.
    Unknown,
}

/// Root-cause finding and its review state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootCauseFinding {
    /// Evidence status.
    pub status: RootCauseStatus,
    /// Category.
    pub category: RootCauseCategory,
    /// Stable explanation code; not a guilt determination.
    pub rationale_code: String,
    /// Evidence supporting the finding.
    pub evidence: Vec<EvidenceId>,
    /// Reviewer required to confirm a finding.
    pub reviewer: Option<AuthorityId>,
}

impl RootCauseFinding {
    /// Build an explicitly unknown finding.
    #[must_use]
    pub fn unknown() -> Self {
        Self {
            status: RootCauseStatus::Unknown,
            category: RootCauseCategory::Unknown,
            rationale_code: "insufficient_root_cause_evidence".into(),
            evidence: Vec::new(),
            reviewer: None,
        }
    }
}

/// State of a proposed repair or lesson.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewStatus {
    /// Awaiting an authorized reviewer.
    Pending,
    /// Reviewed and approved as knowledge. Nothing is deployed by this state.
    Approved,
    /// Rejected by an authorized reviewer.
    Rejected,
}

/// Verification state of a case or repair recommendation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationStatus {
    /// No verification has been performed.
    NotStarted,
    /// Verification is queued or in progress.
    Pending,
    /// Regression evidence passed.
    Verified,
    /// Verification failed or was rejected.
    Failed,
}

/// A failed deterministic policy check captured for investigation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailedPolicyCheck {
    /// Policy identity.
    pub policy: PolicyReference,
    /// Stable check identifier.
    pub check_id: String,
    /// Evidence proving the check failed.
    pub evidence: EvidenceId,
}

/// A non-executable proposed correction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedCorrection {
    /// Stable correction code.
    pub code: String,
    /// Evidence motivating the correction.
    pub evidence: Vec<EvidenceId>,
    /// Review state.
    pub review_status: ReviewStatus,
}

/// Linked incident from a prior or concurrent case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedIncident {
    /// Incident or case id.
    pub id: String,
    /// Related agent.
    pub agent_id: AgentId,
    /// Stable relationship code.
    pub relationship: String,
}

/// Input fields supplied by the trusted runtime when opening a case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForensicCaseInput {
    /// Execution trace evidence.
    pub execution_traces: Vec<EvidenceId>,
    /// Failed policy checks.
    pub failed_policy_checks: Vec<FailedPolicyCheck>,
    /// Historical incidents.
    pub related_incidents: Vec<RelatedIncident>,
    /// Root-cause finding.
    pub root_cause: RootCauseFinding,
    /// Proposed corrections. These are records only, never executable actions.
    pub proposed_corrections: Vec<ProposedCorrection>,
}

/// Structured forensic record. Fields are immutable after insertion into the
/// Scrapyard.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForensicCase {
    case_id: CaseId,
    agent_id: AgentId,
    agent_config_hash: ContentHash,
    model: ModelIdentity,
    code_version: CodeVersion,
    tool_permissions: BTreeSet<String>,
    execution_trace_refs: Vec<EvidenceId>,
    failed_policy_checks: Vec<FailedPolicyCheck>,
    violation_ids: Vec<ViolationId>,
    related_incidents: Vec<RelatedIncident>,
    evidence_hashes: BTreeMap<EvidenceId, ContentHash>,
    root_cause: RootCauseFinding,
    proposed_corrections: Vec<ProposedCorrection>,
    verification_status: VerificationStatus,
    opened_at: u64,
}

impl ForensicCase {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        case_id: CaseId,
        agent_id: AgentId,
        agent_config_hash: ContentHash,
        model: ModelIdentity,
        code_version: CodeVersion,
        tool_permissions: BTreeSet<String>,
        violation_ids: Vec<ViolationId>,
        input: ForensicCaseInput,
        evidence_hashes: BTreeMap<EvidenceId, ContentHash>,
        opened_at: u64,
    ) -> Self {
        Self {
            case_id,
            agent_id,
            agent_config_hash,
            model,
            code_version,
            tool_permissions,
            execution_trace_refs: input.execution_traces,
            failed_policy_checks: input.failed_policy_checks,
            violation_ids,
            related_incidents: input.related_incidents,
            evidence_hashes,
            root_cause: input.root_cause,
            proposed_corrections: input.proposed_corrections,
            verification_status: VerificationStatus::NotStarted,
            opened_at,
        }
    }

    /// Case identifier.
    #[must_use]
    pub fn case_id(&self) -> &CaseId {
        &self.case_id
    }

    /// Original agent identity. Archiving never removes this link.
    #[must_use]
    pub fn agent_id(&self) -> &AgentId {
        &self.agent_id
    }

    /// Agent configuration digest.
    #[must_use]
    pub const fn agent_config_hash(&self) -> ContentHash {
        self.agent_config_hash
    }

    /// Model identity.
    #[must_use]
    pub fn model(&self) -> &ModelIdentity {
        &self.model
    }

    /// Code version.
    #[must_use]
    pub fn code_version(&self) -> &CodeVersion {
        &self.code_version
    }

    /// Tool and API permissions observed for the case.
    #[must_use]
    pub fn tool_permissions(&self) -> &BTreeSet<String> {
        &self.tool_permissions
    }

    /// Execution trace references.
    #[must_use]
    pub fn execution_trace_refs(&self) -> &[EvidenceId] {
        &self.execution_trace_refs
    }

    /// Failed policy checks.
    #[must_use]
    pub fn failed_policy_checks(&self) -> &[FailedPolicyCheck] {
        &self.failed_policy_checks
    }

    /// Violations associated with the case.
    #[must_use]
    pub fn violation_ids(&self) -> &[ViolationId] {
        &self.violation_ids
    }

    /// Historical incidents linked to this case.
    #[must_use]
    pub fn related_incidents(&self) -> &[RelatedIncident] {
        &self.related_incidents
    }

    /// Exact evidence hashes captured when the case was opened.
    #[must_use]
    pub fn evidence_hashes(&self) -> &BTreeMap<EvidenceId, ContentHash> {
        &self.evidence_hashes
    }

    /// Root-cause finding.
    #[must_use]
    pub fn root_cause(&self) -> &RootCauseFinding {
        &self.root_cause
    }

    /// Proposed corrections.
    #[must_use]
    pub fn proposed_corrections(&self) -> &[ProposedCorrection] {
        &self.proposed_corrections
    }

    /// Verification state.
    #[must_use]
    pub const fn verification_status(&self) -> VerificationStatus {
        self.verification_status
    }

    /// Case creation time.
    #[must_use]
    pub const fn opened_at(&self) -> u64 {
        self.opened_at
    }

    pub(crate) fn set_confirmed_root_cause(&mut self, finding: RootCauseFinding) {
        self.root_cause = finding;
    }
}

/// One normalized item in the reconstructed case timeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineEntry {
    /// Trusted timestamp.
    pub at: u64,
    /// Event category.
    pub event_type: String,
    /// Stable record reference.
    pub reference: String,
}

/// Regression test proposed by deterministic analysis.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegressionProposal {
    /// Stable proposal id.
    pub id: String,
    /// Required test scenario code.
    pub scenario: String,
    /// Evidence the test must cover.
    pub evidence: Vec<EvidenceId>,
    /// Review is required before a repair can be promoted.
    pub review_status: ReviewStatus,
    /// Verification state.
    pub verification_status: VerificationStatus,
}

/// Read-only investigation output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvestigationReport {
    /// Original immutable case.
    pub case: ForensicCase,
    /// Reconstructed available timeline.
    pub timeline: Vec<TimelineEntry>,
    /// Possible categories. `Confirmed` appears only when the case itself has
    /// an explicitly reviewed confirmed finding.
    pub root_cause_status: RootCauseStatus,
    /// Possible root-cause categories.
    pub possible_categories: Vec<RootCauseCategory>,
    /// Linked historical incidents.
    pub related_incidents: Vec<RelatedIncident>,
    /// Proposed regression tests.
    pub regression_tests: Vec<RegressionProposal>,
    /// Promotion state for any repair.
    pub repair_review_status: ReviewStatus,
}

/// Stable lesson identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LessonId(String);

impl LessonId {
    /// Validate and construct a lesson id.
    pub fn new(value: impl Into<String>) -> Result<Self, ScrapyardError> {
        let value = value.into();
        validate_public_token("lesson_id", &value, 160)?;
        Ok(Self(value))
    }

    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for LessonId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LessonId({})", self.0)
    }
}

/// Verified knowledge extracted from a failed agent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lesson {
    /// Lesson identity.
    pub lesson_id: LessonId,
    /// Original case.
    pub original_incident: CaseId,
    /// Confirmed root cause.
    pub root_cause: RootCauseFinding,
    /// Evidence references.
    pub evidence: Vec<EvidenceId>,
    /// Applicable policy.
    pub policy: PolicyReference,
    /// Applicable code version.
    pub code_version: CodeVersion,
    /// Regression test proposal.
    pub regression_test: RegressionProposal,
    /// Proposed remediation code.
    pub remediation: ProposedCorrection,
    /// Review state. Approval is knowledge review, not deployment.
    pub review_status: ReviewStatus,
}

/// Registry of reviewed and pending lessons.
#[derive(Debug, Default)]
pub struct KnowledgeRegistry {
    lessons: BTreeMap<LessonId, Lesson>,
}

impl KnowledgeRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(&mut self, lesson: Lesson) -> &Lesson {
        self.lessons
            .entry(lesson.lesson_id.clone())
            .or_insert(lesson)
    }

    pub(crate) fn set_review(
        &mut self,
        id: &LessonId,
        status: ReviewStatus,
    ) -> Result<&Lesson, ScrapyardError> {
        let lesson = self
            .lessons
            .get_mut(id)
            .ok_or_else(|| ScrapyardError::UnknownLesson(id.clone()))?;
        if lesson.review_status == ReviewStatus::Approved && status != ReviewStatus::Approved {
            return Err(ScrapyardError::TerminalRecord);
        }
        lesson.review_status = status;
        Ok(lesson)
    }

    /// Retrieve a lesson.
    #[must_use]
    pub fn get(&self, id: &LessonId) -> Option<&Lesson> {
        self.lessons.get(id)
    }

    /// All lessons in deterministic order.
    pub fn lessons(&self) -> impl Iterator<Item = &Lesson> {
        self.lessons.values()
    }
}

pub(crate) fn build_case(
    agent_id: AgentId,
    agent_config_hash: ContentHash,
    model: ModelIdentity,
    code_version: CodeVersion,
    tool_permissions: BTreeSet<String>,
    violations: &[ViolationRecord],
    input: ForensicCaseInput,
    evidence_hashes: BTreeMap<EvidenceId, ContentHash>,
    opened_at: u64,
) -> Result<ForensicCase, ScrapyardError> {
    validate_forensic_input(&input)?;
    let violation_ids = violations
        .iter()
        .map(|violation| violation.id().clone())
        .collect::<Vec<_>>();
    let mut canonical = Vec::new();
    canonical.extend_from_slice(agent_id.as_str().as_bytes());
    canonical.push(0);
    canonical.extend_from_slice(agent_config_hash.as_bytes());
    canonical.extend_from_slice(&opened_at.to_be_bytes());
    for violation in violations {
        canonical.extend_from_slice(violation.id().as_str().as_bytes());
        canonical.push(0);
    }
    canonical.extend_from_slice(
        ContentHash::sha256(
            serde_json::to_string(&input)
                .map_err(|error| ScrapyardError::Serialization(error.to_string()))?
                .as_bytes(),
        )
        .to_hex()
        .as_bytes(),
    );
    let digest = ContentHash::sha256(&canonical);
    let case_id = CaseId::new(format!("case:{}", digest.to_hex()))?;
    Ok(ForensicCase::new(
        case_id,
        agent_id,
        agent_config_hash,
        model,
        code_version,
        tool_permissions,
        violation_ids,
        input,
        evidence_hashes,
        opened_at,
    ))
}

pub(crate) fn investigate_case(
    case: ForensicCase,
    lifecycle_state: LifecycleState,
    lifecycle_history: &[LifecycleTransition],
    violations: &[ViolationRecord],
) -> InvestigationReport {
    let mut timeline = lifecycle_history
        .iter()
        .map(|transition| TimelineEntry {
            at: transition.at,
            event_type: format!("lifecycle:{:?}", transition.to),
            reference: transition.transition_hash.to_hex(),
        })
        .collect::<Vec<_>>();
    timeline.extend(violations.iter().map(|violation| TimelineEntry {
        at: violation.occurred_at(),
        event_type: format!("violation:{:?}", violation.class()),
        reference: violation.id().as_str().to_owned(),
    }));
    timeline.sort_by(|left, right| {
        left.at
            .cmp(&right.at)
            .then_with(|| left.reference.cmp(&right.reference))
    });

    let possible_categories = violations
        .iter()
        .map(ViolationRecord::category)
        .map(category_to_root_cause)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let root_cause_status = case.root_cause.status;
    let evidence = if case.execution_trace_refs.is_empty() {
        case.evidence_hashes.keys().cloned().collect()
    } else {
        case.execution_trace_refs.clone()
    };
    let scenario = match lifecycle_state {
        LifecycleState::Terminated | LifecycleState::Archived => {
            "terminated_agent_cannot_regain_authority"
        }
        LifecycleState::Quarantined | LifecycleState::Suspended => {
            "quarantined_agent_cannot_receive_new_work"
        }
        _ => "reproduce_failed_policy_check",
    }
    .to_owned();
    let proposal_id = format!(
        "regression:{}",
        ContentHash::digest(
            "x3.scrapyard.regression.v1",
            format!("{}\0{scenario}", case.case_id.as_str()).as_bytes()
        )
        .to_hex()
    );
    let regression = RegressionProposal {
        id: proposal_id,
        scenario,
        evidence,
        review_status: ReviewStatus::Pending,
        verification_status: VerificationStatus::NotStarted,
    };
    InvestigationReport {
        related_incidents: case.related_incidents.clone(),
        case,
        timeline,
        root_cause_status,
        possible_categories,
        regression_tests: vec![regression],
        repair_review_status: ReviewStatus::Pending,
    }
}

fn category_to_root_cause(category: ViolationCategory) -> RootCauseCategory {
    match category {
        ViolationCategory::CapabilityEscape | ViolationCategory::PolicyBypass => {
            RootCauseCategory::PolicyGap
        }
        ViolationCategory::EvidenceTampering => RootCauseCategory::ToolFailure,
        ViolationCategory::CredentialMisuse => RootCauseCategory::ExternalActor,
        ViolationCategory::ResourceAbuse | ViolationCategory::Impersonation => {
            RootCauseCategory::OperatorError
        }
        ViolationCategory::Other => RootCauseCategory::Unknown,
    }
}

fn validate_forensic_input(input: &ForensicCaseInput) -> Result<(), ScrapyardError> {
    for check in &input.failed_policy_checks {
        validate_public_token("failed_policy_check.check_id", &check.check_id, 128)?;
        validate_public_token(
            "failed_policy_check.policy.version",
            &check.policy.version,
            96,
        )?;
    }
    validate_public_token(
        "root_cause.rationale_code",
        &input.root_cause.rationale_code,
        160,
    )?;
    if input.root_cause.status == RootCauseStatus::Confirmed
        && (input.root_cause.evidence.is_empty() || input.root_cause.reviewer.is_none())
    {
        return Err(ScrapyardError::EvidenceRequired);
    }
    for incident in &input.related_incidents {
        validate_public_text("related_incident.id", &incident.id, 160)?;
        validate_public_token("related_incident.relationship", &incident.relationship, 96)?;
    }
    for correction in &input.proposed_corrections {
        validate_public_token("proposed_correction.code", &correction.code, 160)?;
    }
    Ok(())
}
