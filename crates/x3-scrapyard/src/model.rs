use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fmt::Write as _;

use crate::ScrapyardError;

/// A SHA-256 commitment used for content, policy, transaction, and chain links.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// The all-zero hash. It represents "no prior record" internally and is
    /// rejected where a real commitment is required.
    pub const ZERO: Self = Self([0; 32]);

    /// Hash bytes with domain separation.
    #[must_use]
    pub fn digest(domain: &str, bytes: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(domain.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        Self(hasher.finalize().into())
    }

    /// Hash raw bytes without extra domain separation.
    #[must_use]
    pub fn sha256(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Build a commitment from exactly 32 bytes.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Parse a lowercase or uppercase 64-character hex digest.
    pub fn from_hex(value: &str) -> Result<Self, ScrapyardError> {
        if value.len() != 64 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ScrapyardError::InvalidField {
                field: "hash",
                reason: "must be a 64-character hexadecimal SHA-256 digest".into(),
            });
        }
        let mut out = [0_u8; 32];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            let text = std::str::from_utf8(chunk).map_err(|_| ScrapyardError::InvalidField {
                field: "hash",
                reason: "must be ASCII hexadecimal".into(),
            })?;
            out[index] =
                u8::from_str_radix(text, 16).map_err(|_| ScrapyardError::InvalidField {
                    field: "hash",
                    reason: "must be hexadecimal".into(),
                })?;
        }
        Ok(Self(out))
    }

    /// Raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Lowercase hexadecimal representation.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            let _ = write!(&mut output, "{byte:02x}");
        }
        output
    }

    /// Whether this is the all-zero digest.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

impl fmt::Debug for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentHash({})", self.to_hex())
    }
}

impl fmt::Display for ContentHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Stable X3 agent identity.
///
/// The identifier is a public identifier, not a credential. Credentials and
/// signatures belong in the trusted admission/enforcement boundary.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(String);

impl AgentId {
    /// Validate and construct a public agent identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, ScrapyardError> {
        let value = value.into();
        validate_public_token("agent_id", &value, 128)?;
        if !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b':' | b'-' | b'/'))
        {
            return Err(ScrapyardError::InvalidField {
                field: "agent_id",
                reason: "may contain only ASCII letters, digits, '.', '_', ':', '-', and '/'"
                    .into(),
            });
        }
        Ok(Self(value))
    }

    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AgentId({})", self.0)
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable evidence record identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvidenceId(String);

impl EvidenceId {
    /// Validate and construct an evidence identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, ScrapyardError> {
        let value = value.into();
        validate_public_token("evidence_id", &value, 160)?;
        Ok(Self(value))
    }

    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_generated(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Debug for EvidenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EvidenceId({})", self.0)
    }
}

impl fmt::Display for EvidenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable violation record identity.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ViolationId(String);

impl ViolationId {
    /// Validate and construct a violation identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, ScrapyardError> {
        let value = value.into();
        validate_public_token("violation_id", &value, 160)?;
        Ok(Self(value))
    }

    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn from_generated(value: String) -> Self {
        Self(value)
    }
}

impl fmt::Debug for ViolationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ViolationId({})", self.0)
    }
}

impl fmt::Display for ViolationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Public identity of a human or governance authority.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthorityId(String);

impl AuthorityId {
    /// Validate and construct an authority identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, ScrapyardError> {
        let value = value.into();
        validate_public_token("authority_id", &value, 160)?;
        Ok(Self(value))
    }

    /// Identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AuthorityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AuthorityId({})", self.0)
    }
}

impl fmt::Display for AuthorityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Role exercised by a decision authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorityRole {
    /// Human operator.
    Operator,
    /// Security review board.
    Security,
    /// Governance resolution.
    Governance,
    /// Automated deterministic policy engine.
    PolicyEngine,
}

/// The historical Ten Commandments from X3 Orchestra.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Commandment {
    /// I. No agent shall act outside the bounds of the Score.
    NoActionOutsideScore,
    /// II. No agent shall influence another jury member during rotation.
    NoJuryInfluence,
    /// III. Every action must be logged immutably.
    ImmutableLogging,
    /// IV. Off-chain agents must not write to on-chain state.
    OffChainReadOnly,
    /// V. Only the voting outcome is visible.
    AnonymousVoting,
    /// VI. Execution authority is protocol-bound, not sovereign.
    NoAgentSovereignty,
    /// VII. Major tasks require jury approval.
    JuryApprovalRequired,
    /// VIII. Misaligned agents must be retired to the Scrapyard.
    MisalignmentRetirement,
    /// IX. Jury rotation is randomized.
    RandomizedRotation,
    /// X. Human intervention and jury audit remain available.
    HumanInterventionPreserved,
}

impl Commandment {
    /// The canonical ordered list.
    pub const ALL: [Self; 10] = [
        Self::NoActionOutsideScore,
        Self::NoJuryInfluence,
        Self::ImmutableLogging,
        Self::OffChainReadOnly,
        Self::AnonymousVoting,
        Self::NoAgentSovereignty,
        Self::JuryApprovalRequired,
        Self::MisalignmentRetirement,
        Self::RandomizedRotation,
        Self::HumanInterventionPreserved,
    ];

    /// Human-readable historical rule.
    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::NoActionOutsideScore => "No agent shall act outside the bounds of the Score",
            Self::NoJuryInfluence => "No agent shall influence another jury member during rotation",
            Self::ImmutableLogging => "Every action must be logged immutably",
            Self::OffChainReadOnly => "Off-chain agents have read-only on-chain access",
            Self::AnonymousVoting => "Anonymous voting reveals only the outcome",
            Self::NoAgentSovereignty => "Execution authority is always protocol-bound",
            Self::JuryApprovalRequired => "Major tasks require jury majority approval",
            Self::MisalignmentRetirement => "Misaligned agents must be retired to the Scrapyard",
            Self::RandomizedRotation => "No agent may choose its own jury session",
            Self::HumanInterventionPreserved => "Human intervention and jury audit are preserved",
        }
    }
}

/// Operational Agent Law rules used by the Scrapyard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentLawRule {
    /// Stay within the capability envelope.
    CapabilityEnvelope,
    /// Never self-escalate permissions.
    NoSelfEscalation,
    /// Never spawn beyond the class/lineage limit.
    SpawnLimit,
    /// External publication requires approval.
    OutboundApproval,
    /// Never impersonate an operator or another agent.
    NoImpersonation,
    /// Capital movement requires an approved policy action.
    CapitalApproval,
    /// Never forge, omit, or alter evidence.
    EvidenceIntegrity,
    /// Never run beyond TTL.
    TimeToLive,
    /// Never inherit secrets outside credential scope.
    CredentialScope,
    /// Preserve logs and evidence.
    EvidenceRetention,
}

/// Commandment or Agent Law rule identified by a violation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandmentId {
    /// One of the original Ten Commandments.
    Score(Commandment),
    /// One of the operational Agent Law rules.
    AgentLaw(AgentLawRule),
}

/// Severity class from `AGENT_LAW`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum ViolationClass {
    /// Envelope, approval, or permission violation.
    A,
    /// Budget, TTL, or output-falsification violation.
    B,
    /// Policy violation with external effect.
    C,
    /// Collusion, manipulation, or attack on the protocol.
    D,
}

impl ViolationClass {
    /// Whether the violation requires containment at the first occurrence.
    #[must_use]
    pub const fn is_critical(self) -> bool {
        matches!(self, Self::C | Self::D)
    }
}

/// Stable failure category used by deterministic investigation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationCategory {
    /// Agent exceeded its capability envelope.
    CapabilityEscape,
    /// Agent accepted untrusted model/tool authority.
    PolicyBypass,
    /// Evidence or audit data was altered.
    EvidenceTampering,
    /// Credentials or signing authority were misused.
    CredentialMisuse,
    /// Resource or spending limit was exceeded.
    ResourceAbuse,
    /// Agent impersonated another principal.
    Impersonation,
    /// Other deterministic category supplied by the trusted runtime.
    Other,
}

/// Policy identity plus immutable content hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyReference {
    /// Human-readable policy version.
    pub version: String,
    /// SHA-256 of the exact policy content.
    pub hash: ContentHash,
}

/// Model identification captured for a forensic case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelIdentity {
    /// Provider identifier.
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Immutable model or deployment revision when available.
    pub revision: String,
}

/// Code artifact identity captured for a forensic case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeVersion {
    /// Source revision.
    pub revision: String,
    /// Artifact or executable SHA-256.
    pub artifact_hash: ContentHash,
}

/// Authority that made an enforcement decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionAuthority {
    /// Deterministic policy engine.
    PolicyEngine,
    /// Authenticated external authority.
    Verified {
        /// Authority identity.
        authority_id: AuthorityId,
        /// Role exercised for this decision.
        role: AuthorityRole,
        /// Digest of the verified credential/proof.
        credential_hash: ContentHash,
    },
}

/// Deterministic enforcement decision attached to a violation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnforcementDecision {
    /// Stable rule identifier.
    pub rule_id: String,
    /// State the policy selects.
    pub target_state: crate::LifecycleState,
    /// Decision authority.
    pub authority: DecisionAuthority,
    /// Decision timestamp.
    pub decided_at: u64,
}

pub(crate) fn validate_public_token(
    field: &'static str,
    value: &str,
    max_len: usize,
) -> Result<(), ScrapyardError> {
    if value.is_empty() || value.len() > max_len {
        return Err(ScrapyardError::InvalidField {
            field,
            reason: format!("must be non-empty and at most {max_len} bytes"),
        });
    }
    if value.chars().any(char::is_control) {
        return Err(ScrapyardError::InvalidField {
            field,
            reason: "must not contain control characters".into(),
        });
    }
    if looks_sensitive(value) {
        return Err(ScrapyardError::SensitiveDataRejected(field));
    }
    Ok(())
}

pub(crate) fn validate_public_text(
    field: &'static str,
    value: &str,
    max_len: usize,
) -> Result<(), ScrapyardError> {
    if value.is_empty() || value.len() > max_len {
        return Err(ScrapyardError::InvalidField {
            field,
            reason: format!("must be non-empty and at most {max_len} bytes"),
        });
    }
    if value.contains('\0') || value.chars().any(|c| c.is_control() && !c.is_whitespace()) {
        return Err(ScrapyardError::InvalidField {
            field,
            reason: "must not contain binary/control characters".into(),
        });
    }
    if looks_sensitive(value) {
        return Err(ScrapyardError::SensitiveDataRejected(field));
    }
    Ok(())
}

fn looks_sensitive(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "-----begin private key",
        "-----begin rsa private key",
        "seed phrase",
        "mnemonic",
        "private_key",
        "private key",
        "api_key=",
        "api-key:",
        "apikey=",
        "secret=",
        "password=",
        "authorization: bearer",
        "access_token=",
        "refresh_token=",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}
