# X3 Scrapyard and Agent Forensics

`x3-scrapyard` is a standalone Rust policy and evidence component for agent
containment. It reuses the concepts, not the dependency graph, of the original
X3 Orchestra Score, X3 Agent Law, and GPU-swarm Scrapyard.

## Implemented

- Seven-state, validated agent lifecycle with terminal-state protection.
- Fail-closed admission that denies quarantined, suspended, terminated, and
  archived agents, including descendants of a blocked ancestor.
- Append-only SHA-256 evidence ledger with predecessor-linked integrity.
- Deterministic `AGENT_LAW` violation ladder with immediate containment for
  Class C/D violations.
- Evidence-backed quarantine and termination workflows.
- Trusted `DecisionVerifier` and `EnforcementAdapter` boundaries.
- Explicit adapter outcomes. Unsupported or failed containment actions become
  unresolved incidents and are never presented as success.
- Irreversible identity termination, termination certificates, and archive
  traceability.
- Forensic case records, read-only investigation reports, root-cause review,
  regression proposals, and a knowledge registry.
- Adversarial integration tests and lifecycle property tests.

## Trust boundaries

`VerifiedCredential` is only meaningful when returned by a real
`DecisionVerifier`. Production implementations must verify a signature,
authenticated session, or governance proof bound to the exact
`DecisionAction`; body fields or model-authored booleans are not credentials.

`EnforcementOutcome::Confirmed` is an assertion by the injected trusted runtime
adapter that it observed the requested scheduler, credential, or signing
effect. The Scrapyard stores that assertion but cannot independently prove the
external effect. There is no default adapter and no host process kill, wallet
signing, secret access, or deployment path in this crate.

The evidence ledger stores metadata and content digests only. Raw evidence
bytes are consumed for hashing and are not retained. Public metadata is scanned
for common private-key, seed-phrase, API-key, password, and token markers, but
callers must still redact evidence before allowing it into trusted references.

## Admission-PR boundary

Draft admission PR #29 performs deterministic checks on an untrusted candidate
manifest but explicitly does **not** authenticate signatures, reserve budget,
persist the identity registry, or enforce runtime permission. A successful
admission check is not an admission certificate.

The trusted runtime should complete the atomic admission transaction first,
then call `Scrapyard::enroll_agent` with the immutable manifest hash and genesis
evidence. This crate rejects duplicate identities, detached parents, and
children of blocked ancestors. It does not yet verify a signed admission
manifest itself.

## Verification

From this directory:

```bash
cargo test --locked
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
```

The GitHub workflow adds the same commands as a dedicated `scrapyard` job.

## Not production-ready yet

- The store is in memory; durable transactional persistence is not included.
- The Commandments compiler and runtime admission adapter are not wired.
- The real scheduler, credential service, and signing-authority adapters do not
  yet exist in this repository.
- Cryptographic approval verification is an injected integration contract, not
  a shipped production verifier.
- No on-chain, NEAR Guard, wallet, deployment, or funds behavior is changed.
