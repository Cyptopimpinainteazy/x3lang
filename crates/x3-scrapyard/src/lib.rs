//! X3 Scrapyard and agent forensics.
//!
//! This crate is a deterministic policy and record layer. It does not assume
//! that changing an in-memory state stops a process, revokes a key, or prevents
//! a wallet signature. Those effects are requested through
//! [`EnforcementAdapter`] and are reported separately from the Scrapyard's
//! logical lifecycle state.
//!
//! The design reuses the concepts of the historical X3 Orchestra Score, X3
//! Agent Law misconduct ladder, and GPU-swarm Scrapyard without importing their
//! large Substrate and GPU dependency graphs.

#![forbid(unsafe_code)]

mod enforcement;
mod evidence;
mod forensics;
mod lifecycle;
mod model;
mod scrapyard;

pub use enforcement::{
    EnforcementAction, EnforcementAdapter, EnforcementOutcome, EnforcementReceipt,
    EnforcementReport, UnresolvedIncident,
};
pub use evidence::{
    EvidenceInput, EvidenceKind, EvidenceLedger, EvidenceRecord, MAX_EVIDENCE_BYTES,
};
pub use forensics::{
    CaseId, FailedPolicyCheck, ForensicCase, ForensicCaseInput, InvestigationReport,
    KnowledgeRegistry, Lesson, LessonId, ProposedCorrection, RegressionProposal, RelatedIncident,
    ReviewStatus, RootCauseCategory, RootCauseFinding, RootCauseStatus, TimelineEntry,
    VerificationStatus,
};
pub use lifecycle::{AgentLifecycle, LifecycleState, LifecycleTransition};
pub use model::{
    AgentId, AgentLawRule, AuthorityId, AuthorityRole, CodeVersion, Commandment, CommandmentId,
    ContentHash, DecisionAuthority, EnforcementDecision, EvidenceId, ModelIdentity,
    PolicyReference, ViolationCategory, ViolationClass, ViolationId,
};
pub use scrapyard::{
    AgentEnrollment, AgentSnapshot, DecisionAction, DecisionVerifier, Scrapyard,
    ScrapyardAdmission, ScrapyardConfig, ScrapyardError, TerminationCertificate, TerminationIntent,
    VerifiedCredential, ViolationInput, ViolationOutcome, ViolationRecord,
};

/// The canonical schema version for records produced by this crate.
pub const RECORD_SCHEMA_VERSION: &str = "x3.scrapyard.v1";
