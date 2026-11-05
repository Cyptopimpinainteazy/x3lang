# X3 Scrapyard and Agent Forensics

Status: **implemented and locally tested as a standalone Rust component;
runtime adapters, durable storage, exact-head CI, and independent security
review remain pending.**

## Provenance

The historical X3 system combines two complementary constitutional layers:

- `xxxstar/crates/orchestra/src/score.rs`: the immutable Ten Commandments.
- `xxxstar/docs/swarm-governance/AGENT_LAW.md`: genesis, delegation,
  capability envelopes, the misconduct ladder, and termination duties.

The original `xxxstar` Scrapyard implementations added quarantine, disassembly,
knowledge recycling, and retirement concepts. This crate preserves those ideas
while removing the original Substrate/GPU dependency graph.

The source `xxxstar` checkout is read-only context. This work does not modify
the concurrently developed agent constitution files:

- `x3-lang/agent_constitution.py`
- `x3-lang/tests/test_agent_constitution.py`
- `docs/security/X3_AGENT_CONSTITUTION_V1.md`

It also does not modify NEAR Guard or financial enforcement paths.

## Enforcement sequence

Permanent sanction and containment follow this order:

1. Validate deterministic event fields and public record formats.
2. Verify the complete predecessor-linked evidence ledger.
3. Require all referenced evidence records to exist.
4. For termination, authenticate the exact action through `DecisionVerifier`
   and verify the authority is in the configured allowlist.
5. Commit the logical lifecycle state. Quarantined, suspended, terminated, and
   archived states immediately deny admission through `ScrapyardAdmission`.
6. Request stop-admission, queue cancellation, optional running-task suspension,
   delegated-credential revocation, and signing denial from the trusted
   `EnforcementAdapter`.
7. Record adapter confirmations verbatim. Convert every unsupported or failed
   action into a visible unresolved incident.
8. Emit the termination certificate without deleting violations, cases, or
   evidence references.

Step 5 deliberately precedes destructive cleanup. Step 6 does not execute host
processes directly and never uses `kill -9`.

## Decision and adapter trust

A `VerifiedCredential` is trusted only because a production `DecisionVerifier`
verified a real credential bound to the exact `DecisionAction`. The verifier
must not trust agent JSON, model output, or a caller-provided boolean.

An `EnforcementOutcome::Confirmed` is scoped to one adapter action and includes
the adapter identity, observation time, external receipt reference, and
evidence digest. This proves only what the configured trusted adapter asserts.
It is not independent proof that a scheduler, credential system, wallet, or
remote host changed state.

Both authority allowlists in `ScrapyardConfig` default to empty. A new instance
therefore cannot terminate or approve lessons until the trusted host installs
the appropriate authorities.

## Evidence model

The evidence ledger stores:

- deterministic evidence id;
- evidence kind and external reference;
- trusted collector identity;
- collection timestamp;
- SHA-256 of the original content;
- previous-record hash and this-record hash.

The ledger has no update or delete method. Duplicate admission is idempotent.
`investigate` verifies the ledger chain and compares every case evidence hash
against the live ledger before returning a report. It returns a cloned case and
does not mutate original evidence.

Raw content is not stored. Metadata is rejected when it contains markers for
private keys, seed phrases, passwords, API keys, bearer tokens, or refresh
tokens. This is a defense-in-depth check, not a substitute for redaction by the
collecting system.

## Lifecycle and enforcement ladder

The valid states are:

`ACTIVE`, `WARNING`, `PROBATION`, `QUARANTINED`, `SUSPENDED`, `TERMINATED`,
and `ARCHIVED`.

Terminal states cannot return to `ACTIVE`. Only `TERMINATED` can become
`ARCHIVED`.

The deterministic ladder is:

- First Class A: warning.
- Second Class A or first Class B: probation.
- Third Class A/B, or first Class C/D: quarantine.
- Second Class D: suspension.
- Termination remains a separately authenticated human/governance decision; the
  automated policy engine cannot terminate.

Every permanent identity remains reserved after termination and archival.
Admission of a child walks the complete enrolled parent chain and fails if any
ancestor denies execution.

## Forensic investigation

`ForensicCase` records configuration hash, model identity, code version, tool
permissions, execution trace references, failed policy checks, violations,
related incidents, evidence hashes, root-cause state, proposed corrections, and
verification state.

Opening a case requires a contained or terminal agent. Confirmed root causes
cannot be supplied directly by the untrusted case input; they require
`confirm_root_cause` with a verified review authority. A deterministic
investigation reconstructs the available lifecycle/violation timeline,
identifies possible categories, links incidents, and proposes a regression test.

Repair recommendations always start in `Pending` review. `review_lesson` can
approve or reject stored knowledge only. No API changes production policy,
model weights, executable code, credentials, or permissions.

## Integration requirements

Before this component can be described as live containment:

1. Integrate the cryptographically authenticated admission manifest and atomic
   identity/budget reservation from the admission track.
2. Implement a durable transactional `Scrapyard` store.
3. Implement real scheduler, credential, signing, and optional process adapters.
4. Bind the Ten Commandments compiler output and policy hashes to violations.
5. Add an independently reviewed production `DecisionVerifier`.
6. Run exact-head CI and an independent security review.
7. Test against real runtime adapters; in-memory tests are not live containment.

## Verification

From `crates/x3-scrapyard`:

```bash
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
```
