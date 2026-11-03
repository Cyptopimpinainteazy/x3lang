# X3Lang security-audit rollout

This document describes the **candidate** workflow at
`.github/workflows/x3lang-security-audit.yml`. The existing
`x3lang-history-ci.yml` remains unchanged. This is NOT a security audit
certificate or authorization to deploy a contract or process funds.

## Enforced in the candidate workflow

- NEAR Guard formatting and existing contract regression tests (including
  mandate, replay, quote signature, storage migration, and authorization).
- Existing allocator check ensuring the abandoned `wee_alloc` dependency
  is not included.

The new workflow exercises existing tests; it does not claim that new
authorization properties were formally verified.

## Advisory checks, not release approvals

- `cargo clippy --all-targets --locked -- -D warnings` for Guard runs
  non-blocking initially, until warnings and tool behavior are reconciled.
- RustSec `cargo-audit` scans the compiler/VM and NEAR Guard lockfiles,
  records both exit statuses and raw JSON artifacts. Findings and errors
  require triage. This reporting job does not make advisories mandatory.
- The manual `workflow_dispatch` job attempts two WASM builds with the
  same toolchain but independent target directories and compares hashes.
  A green result proves only those two outputs match in that runner.
  It does NOT independently verify a deployed contract, NEP-330 build
  metadata, or artifact equivalence across different machines.
  A failed compilation is evidence that the build path needs repair.

## Next phases (NOT YET IMPLEMENTED)

1. Triage RustSec findings, check current transitive dependencies and
   fix actionable vulnerabilities. Promote the audited dependencies gate
   from advisory to blocking only after an explicit baseline is reviewed.
2. Add property-based tests and a bounded fuzz corpus for parser inputs,
   signatures, mandate approvals, nonces, state migration, amounts, and
   cross-chain evidence. Run them against the exact PR head.
3. Produce versioned NEP-330 metadata with immutable Git revision,
   pinned build image digest, locked dependency graph, and exact output
   path. Run a separate `near-verify-rs` check against that metadata.
4. Compare the reproducible artifact hash with the actual deployed
   on-chain NEAR contract code hash. This requires a deployed address
   and independent public-chain evidence; no deployment is presumed.
5. Review `near-sdk-contract-tools` and `near-plugins` as potential
   patterns **without** swapping authorization mechanisms implicitly.
   Evaluate established audit reports as research, not as proof that
   this application is secure.
6. Add TypeScript-specific static checks if a TypeScript smart contract
   workspace is introduced. X3 Guard is presently a Rust contract.
7. Consider an optional LLM reviewer as a supplemental report only.
   Do not forward private keys, API tokens, or wallet material to it.

## Merge and production boundaries

- Keep `x3lang-history-ci.yml` and existing required status checks.
- Evaluate false positives and runner reliability on this PR before
  configuring branch protection for the new checks.
- Do not force-merge security findings or treat passing CI as a
  professional contract audit.
- The current X3Lang settlement-evidence gate intentionally fails closed
  when independent chain binding is unavailable. No fee collection,
  deployment, or chain transaction is enabled by this workflow.
