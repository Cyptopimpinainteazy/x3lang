use serde::{Deserialize, Serialize};

use crate::model::{validate_public_text, ContentHash, EvidenceId};
use crate::ScrapyardError;

/// Maximum evidence object size accepted for one digest operation.
pub const MAX_EVIDENCE_BYTES: usize = 16 * 1024 * 1024;

/// Kind of evidence admitted to the append-only ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    /// Policy evaluation output.
    PolicyDecision,
    /// Execution trace.
    ExecutionTrace,
    /// Lifecycle or enforcement event.
    LifecycleEvent,
    /// Agent configuration snapshot.
    Configuration,
    /// Model prompt/response metadata. Raw secrets are never admitted.
    ModelTrace,
    /// Tool invocation record.
    ToolInvocation,
    /// Operator or governance decision.
    AuthorityDecision,
    /// Regression test result.
    RegressionTest,
    /// Other trusted evidence type.
    Other,
}

/// Input whose bytes are hashed but never retained.
pub struct EvidenceInput {
    /// Evidence kind.
    pub kind: EvidenceKind,
    /// Stable external reference (path, URI, receipt id).
    pub reference: String,
    /// Collector or trusted subsystem identity.
    pub supplied_by: String,
    /// Evidence collection time, supplied by the trusted runtime.
    pub collected_at: u64,
    /// Complete content bytes. These are consumed only to compute a digest.
    pub content: Vec<u8>,
}

impl std::fmt::Debug for EvidenceInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EvidenceInput")
            .field("kind", &self.kind)
            .field("reference", &self.reference)
            .field("supplied_by", &self.supplied_by)
            .field("collected_at", &self.collected_at)
            .field("content_len", &self.content.len())
            .finish()
    }
}

/// Immutable evidence metadata kept by the Scrapyard.
///
/// This record intentionally does not contain the source bytes. Integrity can
/// be checked by presenting the bytes again to
/// [`EvidenceLedger::verify_content`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct EvidenceRecord {
    id: EvidenceId,
    kind: EvidenceKind,
    reference: String,
    supplied_by: String,
    collected_at: u64,
    content_hash: ContentHash,
    previous_record_hash: ContentHash,
    record_hash: ContentHash,
}

impl EvidenceRecord {
    /// Deterministic evidence identifier.
    #[must_use]
    pub fn id(&self) -> &EvidenceId {
        &self.id
    }

    /// Evidence kind.
    #[must_use]
    pub const fn kind(&self) -> EvidenceKind {
        self.kind
    }

    /// External evidence reference.
    #[must_use]
    pub fn reference(&self) -> &str {
        &self.reference
    }

    /// Trusted collector identity.
    #[must_use]
    pub fn supplied_by(&self) -> &str {
        &self.supplied_by
    }

    /// Collection timestamp.
    #[must_use]
    pub const fn collected_at(&self) -> u64 {
        self.collected_at
    }

    /// SHA-256 of the original content.
    #[must_use]
    pub const fn content_hash(&self) -> ContentHash {
        self.content_hash
    }

    /// Hash of the preceding ledger record, or zero for the first record.
    #[must_use]
    pub const fn previous_record_hash(&self) -> ContentHash {
        self.previous_record_hash
    }

    /// Hash of this complete record, including its predecessor link.
    #[must_use]
    pub const fn record_hash(&self) -> ContentHash {
        self.record_hash
    }
}

/// Append-only, integrity-linked evidence ledger.
///
/// The ledger deliberately has no update or delete operation.
#[derive(Debug, Default)]
pub struct EvidenceLedger {
    records: Vec<EvidenceRecord>,
}

impl EvidenceLedger {
    /// Create an empty ledger.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Admit evidence. Replaying exactly the same event is idempotent.
    pub fn append(&mut self, input: EvidenceInput) -> Result<EvidenceRecord, ScrapyardError> {
        validate_public_text("evidence.reference", &input.reference, 512)?;
        validate_public_text("evidence.supplied_by", &input.supplied_by, 160)?;
        if input.content.is_empty() {
            return Err(ScrapyardError::InvalidField {
                field: "evidence.content",
                reason: "must not be empty".into(),
            });
        }
        if input.content.len() > MAX_EVIDENCE_BYTES {
            return Err(ScrapyardError::InvalidField {
                field: "evidence.content",
                reason: format!("must be at most {MAX_EVIDENCE_BYTES} bytes"),
            });
        }
        if !self.verify_chain() {
            return Err(ScrapyardError::EvidenceChainInvalid);
        }

        let content_hash = ContentHash::digest("x3.scrapyard.evidence.content.v1", &input.content);
        let id = derive_evidence_id(
            input.kind,
            &input.reference,
            &input.supplied_by,
            input.collected_at,
            content_hash,
        );

        if let Some(existing) = self.records.iter().find(|record| record.id == id) {
            return Ok(existing.clone());
        }

        let previous_record_hash = self
            .records
            .last()
            .map_or(ContentHash::ZERO, EvidenceRecord::record_hash);
        let mut record = EvidenceRecord {
            id,
            kind: input.kind,
            reference: input.reference,
            supplied_by: input.supplied_by,
            collected_at: input.collected_at,
            content_hash,
            previous_record_hash,
            record_hash: ContentHash::ZERO,
        };
        record.record_hash = compute_record_hash(&record);
        self.records.push(record.clone());
        Ok(record)
    }

    /// Retrieve a record by identifier.
    #[must_use]
    pub fn get(&self, id: &EvidenceId) -> Option<&EvidenceRecord> {
        self.records.iter().find(|record| record.id == *id)
    }

    /// All records in append order.
    #[must_use]
    pub fn records(&self) -> &[EvidenceRecord] {
        &self.records
    }

    /// Number of admitted evidence records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the ledger is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Verify the bytes against the admitted digest.
    #[must_use]
    pub fn verify_content(&self, id: &EvidenceId, content: &[u8]) -> bool {
        self.get(id).is_some_and(|record| {
            ContentHash::digest("x3.scrapyard.evidence.content.v1", content) == record.content_hash
        })
    }

    /// Verify the complete predecessor-linked ledger.
    #[must_use]
    pub fn verify_chain(&self) -> bool {
        let mut previous = ContentHash::ZERO;
        for record in &self.records {
            if record.previous_record_hash != previous
                || record.record_hash != compute_record_hash(record)
            {
                return false;
            }
            previous = record.record_hash;
        }
        true
    }
}

fn derive_evidence_id(
    kind: EvidenceKind,
    reference: &str,
    supplied_by: &str,
    collected_at: u64,
    content_hash: ContentHash,
) -> EvidenceId {
    let canonical = format!(
        "x3.scrapyard.evidence.id.v1\0{kind:?}\0{reference}\0{supplied_by}\0{collected_at}\0{content_hash}"
    );
    let digest = ContentHash::sha256(canonical.as_bytes());
    EvidenceId::from_generated(format!("ev:{}", digest.to_hex()))
}

fn compute_record_hash(record: &EvidenceRecord) -> ContentHash {
    let canonical = format!(
        "x3.scrapyard.evidence.record.v1\0{}\0{:?}\0{}\0{}\0{}\0{}\0{}",
        record.id,
        record.kind,
        record.reference,
        record.supplied_by,
        record.collected_at,
        record.content_hash,
        record.previous_record_hash
    );
    ContentHash::sha256(canonical.as_bytes())
}
