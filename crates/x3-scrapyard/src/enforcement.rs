use serde::{Deserialize, Serialize};

use crate::model::{validate_public_token, AgentId, ContentHash};
use crate::ScrapyardError;

/// Actions the consuming runtime must support for containment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementAction {
    /// Stop admission of new tasks.
    StopNewAdmission,
    /// Cancel queued tasks.
    CancelQueuedTasks,
    /// Suspend running tasks when the runtime supports it.
    SuspendRunningTasks,
    /// Revoke delegated credentials.
    RevokeDelegatedCredentials,
    /// Deny further signing authority.
    DenySigningAuthority,
}

impl EnforcementAction {
    /// Every containment action requested for quarantine and termination.
    pub const ALL: [Self; 5] = [
        Self::StopNewAdmission,
        Self::CancelQueuedTasks,
        Self::SuspendRunningTasks,
        Self::RevokeDelegatedCredentials,
        Self::DenySigningAuthority,
    ];
}

/// Adapter-reported result for one enforcement action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforcementOutcome {
    /// The trusted adapter observed the requested effect.
    Confirmed {
        /// Adapter-owned receipt or audit reference.
        reference: String,
        /// Digest of the adapter evidence.
        evidence_hash: ContentHash,
    },
    /// The runtime cannot implement this action. This is never treated as
    /// success.
    Unsupported {
        /// Stable reason code.
        reason_code: String,
    },
    /// The request was attempted and failed. This is never treated as success.
    Failed {
        /// Stable reason code.
        reason_code: String,
    },
}

impl EnforcementOutcome {
    /// Whether a trusted adapter reported a confirmed effect.
    #[must_use]
    pub const fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed { .. })
    }
}

/// Single adapter response. The Scrapyard supplies the requested action and
/// identity; the adapter supplies its own identity and observed outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnforcementReceipt {
    /// Adapter identity.
    pub adapter_id: String,
    /// Observation time.
    pub observed_at: u64,
    /// Actual adapter-reported result.
    pub outcome: EnforcementOutcome,
}

/// One enforcement request and its reported outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnforcementReport {
    /// Affected agent.
    pub agent_id: AgentId,
    /// Requested action.
    pub action: EnforcementAction,
    /// Adapter identity.
    pub adapter_id: String,
    /// Adapter response.
    pub outcome: EnforcementOutcome,
    /// Adapter observation time.
    pub observed_at: u64,
}

/// Visible security incident for enforcement that did not complete.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedIncident {
    /// Deterministic incident id.
    pub id: String,
    /// Affected agent.
    pub agent_id: AgentId,
    /// Action that was not confirmed.
    pub action: EnforcementAction,
    /// Stable reason code.
    pub reason_code: String,
    /// When the failure was recorded.
    pub recorded_at: u64,
}

/// Trusted execution boundary for containment effects.
///
/// An implementation must call the real scheduler, credential service, and
/// signing boundary. Returning [`EnforcementOutcome::Confirmed`] is an
/// assertion by this trusted adapter that the effect was observed. The
/// Scrapyard records that assertion but cannot independently prove the effect.
pub trait EnforcementAdapter {
    /// Stable adapter identity.
    fn adapter_id(&self) -> &str;

    /// Request one containment action.
    fn enforce(
        &mut self,
        agent_id: &AgentId,
        action: EnforcementAction,
        now: u64,
    ) -> EnforcementReceipt;
}

pub(crate) fn run_required_enforcement(
    agent_id: &AgentId,
    now: u64,
    adapter: &mut dyn EnforcementAdapter,
) -> Result<Vec<EnforcementReport>, ScrapyardError> {
    let adapter_id = adapter.adapter_id().to_owned();
    validate_public_token("enforcement.adapter_id", &adapter_id, 160)?;

    let mut reports = Vec::with_capacity(EnforcementAction::ALL.len());
    for action in EnforcementAction::ALL {
        let receipt = adapter.enforce(agent_id, action, now);
        let outcome = normalize_receipt(&adapter_id, &receipt);
        reports.push(EnforcementReport {
            agent_id: agent_id.clone(),
            action,
            adapter_id: adapter_id.clone(),
            outcome,
            observed_at: receipt.observed_at,
        });
    }
    Ok(reports)
}

pub(crate) fn validate_enforcement_adapter(
    adapter: &dyn EnforcementAdapter,
) -> Result<(), ScrapyardError> {
    validate_public_token("enforcement.adapter_id", adapter.adapter_id(), 160)
}

fn normalize_receipt(expected_adapter: &str, receipt: &EnforcementReceipt) -> EnforcementOutcome {
    if receipt.adapter_id != expected_adapter
        || validate_public_token("enforcement.adapter_id", &receipt.adapter_id, 160).is_err()
    {
        return EnforcementOutcome::Failed {
            reason_code: "adapter_protocol_error".into(),
        };
    }
    match &receipt.outcome {
        EnforcementOutcome::Confirmed {
            reference,
            evidence_hash,
        } if !evidence_hash.is_zero()
            && validate_public_token("enforcement.reference", reference, 512).is_ok() =>
        {
            receipt.outcome.clone()
        }
        EnforcementOutcome::Confirmed { .. } => EnforcementOutcome::Failed {
            reason_code: "adapter_protocol_error".into(),
        },
        EnforcementOutcome::Unsupported { reason_code }
        | EnforcementOutcome::Failed { reason_code }
            if validate_public_token("enforcement.reason_code", reason_code, 96).is_ok() =>
        {
            receipt.outcome.clone()
        }
        EnforcementOutcome::Unsupported { .. } | EnforcementOutcome::Failed { .. } => {
            EnforcementOutcome::Failed {
                reason_code: "adapter_protocol_error".into(),
            }
        }
    }
}

impl UnresolvedIncident {
    pub(crate) fn from_report(report: &EnforcementReport, recorded_at: u64) -> Self {
        let reason_code = match &report.outcome {
            EnforcementOutcome::Unsupported { reason_code }
            | EnforcementOutcome::Failed { reason_code } => reason_code.clone(),
            EnforcementOutcome::Confirmed { .. } => "confirmed".into(),
        };
        let canonical = format!(
            "x3.scrapyard.unresolved.v1\0{}\0{:?}\0{}\0{}\0{}",
            report.agent_id, report.action, report.adapter_id, reason_code, recorded_at
        );
        let digest = ContentHash::sha256(canonical.as_bytes());
        Self {
            id: format!("incident:{}", digest.to_hex()),
            agent_id: report.agent_id.clone(),
            action: report.action,
            reason_code,
            recorded_at,
        }
    }
}
