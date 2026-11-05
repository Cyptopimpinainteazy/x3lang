use serde::{Deserialize, Serialize};

use crate::model::{validate_public_token, AuthorityId, AuthorityRole, ContentHash, EvidenceId};
use crate::ScrapyardError;

/// Validated agent lifecycle state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    /// Normal operation.
    Active,
    /// A violation is recorded, but new execution remains permitted.
    Warning,
    /// Restricted operation pending demonstrated remediation.
    Probation,
    /// Evidence is preserved and new execution authority is denied.
    Quarantined,
    /// All execution is suspended pending governance resolution.
    Suspended,
    /// Irreversible logical termination.
    Terminated,
    /// Terminal forensic archive.
    Archived,
}

impl LifecycleState {
    /// Whether this state can transition to `next`.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use LifecycleState::{
            Active, Archived, Probation, Quarantined, Suspended, Terminated, Warning,
        };
        matches!(
            (self, next),
            (Active, Warning | Quarantined | Suspended | Terminated)
                | (
                    Warning,
                    Active | Probation | Quarantined | Suspended | Terminated
                )
                | (Probation, Active | Quarantined | Suspended | Terminated)
                | (Quarantined, Probation | Suspended | Terminated)
                | (Suspended, Probation | Quarantined | Terminated)
                | (Terminated, Archived)
        )
    }

    /// States that deny new execution authority.
    #[must_use]
    pub const fn denies_execution(self) -> bool {
        matches!(
            self,
            Self::Quarantined | Self::Suspended | Self::Terminated | Self::Archived
        )
    }

    /// Irreversible or final states.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Terminated | Self::Archived)
    }
}

/// One validated lifecycle transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleTransition {
    /// Previous state.
    pub from: LifecycleState,
    /// New state.
    pub to: LifecycleState,
    /// Trusted timestamp.
    pub at: u64,
    /// Stable reason code, never model-authored guilt text.
    pub reason_code: String,
    /// Evidence supporting the transition.
    pub evidence: Vec<EvidenceId>,
    /// Authenticated authority, when the transition requires one.
    pub authority_id: Option<AuthorityId>,
    /// Role exercised by the authority.
    pub authority_role: Option<AuthorityRole>,
    /// Digest of the transition record.
    pub transition_hash: ContentHash,
}

/// Lifecycle history for one identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLifecycle {
    state: LifecycleState,
    history: Vec<LifecycleTransition>,
}

impl AgentLifecycle {
    /// Create an active lifecycle with its genesis entry.
    #[must_use]
    pub fn active(agent_id_text: &str, at: u64, genesis_evidence: EvidenceId) -> Self {
        let mut lifecycle = Self {
            state: LifecycleState::Active,
            history: Vec::new(),
        };
        let mut transition = LifecycleTransition {
            from: LifecycleState::Active,
            to: LifecycleState::Active,
            at,
            reason_code: "genesis_admitted".into(),
            evidence: vec![genesis_evidence],
            authority_id: None,
            authority_role: None,
            transition_hash: ContentHash::ZERO,
        };
        transition.transition_hash = hash_transition(agent_id_text, &transition);
        lifecycle.history.push(transition);
        lifecycle
    }

    /// Current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> LifecycleState {
        self.state
    }

    /// Complete append-only transition history.
    #[must_use]
    pub fn history(&self) -> &[LifecycleTransition] {
        &self.history
    }

    /// Validate and append a transition.
    pub fn transition(
        &mut self,
        agent_id_text: &str,
        next: LifecycleState,
        at: u64,
        reason_code: impl Into<String>,
        evidence: Vec<EvidenceId>,
        authority_id: Option<AuthorityId>,
        authority_role: Option<AuthorityRole>,
    ) -> Result<LifecycleTransition, ScrapyardError> {
        if !self.state.can_transition_to(next) {
            return Err(ScrapyardError::InvalidLifecycleTransition {
                from: self.state,
                to: next,
            });
        }
        if evidence.is_empty() {
            return Err(ScrapyardError::EvidenceRequired);
        }
        let reason_code = reason_code.into();
        validate_public_token("lifecycle.reason_code", &reason_code, 96)?;

        let mut transition = LifecycleTransition {
            from: self.state,
            to: next,
            at,
            reason_code,
            evidence,
            authority_id,
            authority_role,
            transition_hash: ContentHash::ZERO,
        };
        transition.transition_hash = hash_transition(agent_id_text, &transition);
        self.state = next;
        self.history.push(transition.clone());
        Ok(transition)
    }
}

fn hash_transition(agent_id_text: &str, transition: &LifecycleTransition) -> ContentHash {
    let evidence = transition
        .evidence
        .iter()
        .map(EvidenceId::as_str)
        .collect::<Vec<_>>()
        .join(",");
    let canonical = format!(
        "x3.scrapyard.lifecycle.v1\0{}\0{:?}\0{:?}\0{}\0{}\0{}\0{}\0{}",
        agent_id_text,
        transition.from,
        transition.to,
        transition.at,
        transition.reason_code,
        evidence,
        transition
            .authority_id
            .as_ref()
            .map_or("", AuthorityId::as_str),
        transition
            .authority_role
            .map_or_else(|| "none".to_owned(), |role| format!("{role:?}"))
    );
    ContentHash::sha256(canonical.as_bytes())
}
