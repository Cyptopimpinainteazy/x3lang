# X3Lang Master Feature Matrix

Status date: 2026-11-03

This is an evidence index, not a release score. A feature is only marked
implemented and verified when the cited code and a reproducible test or live
evidence exist. The historical 644-entry inventory was not present in either
current checkout, so this matrix starts from the components and interfaces that
are verifiable here. Missing inventory entries must be added only when their
source is recovered.

## Status classes

- **A** — implemented and verified by reproducible local tests.
- **B** — implemented but not independently verified.
- **C** — partially implemented or not wired through the full production path.
- **D** — duplicated authority requiring consolidation.
- **E** — broken or contradicted by current evidence.
- **F** — planned.
- **G** — research.

Risk is **P0** (correctness, security, funds, consensus), **P1** (blocks a
required integration), **P2** (hardening or developer experience), or **P3**
(research/later roadmap).

## Compiler and bytecode

| ID | Feature | Authoritative component | Status | Evidence | Risk | Missing / completion criterion |
|---|---|---|---|---|---|---|
| CMP-001 | Lexer and source spans | `x3-lang/crates/x3-lexer` | A | `cargo test -p x3-lang-vm`; lexer tests | P1 | Stable grammar/version document in master spec |
| CMP-002 | Parser and diagnostics | `x3-lang/compiler/src/parser.rs` | A | Parser, syntax, conformance and CLI tests | P1 | Historical 644-feature inventory reconciliation |
| CMP-003 | Typed AST and visitors | `x3-lang/crates/x3-ast` | A | AST and trading AST tests | P1 | None for current grammar |
| CMP-004 | Semantic/type/capability checks | `compiler/src/semantic.rs`, `verify.rs` | A | Compiler verifier and adversarial fixture tests | P0 | Maintain fail-closed coverage for every new capability |
| CMP-005 | Canonical IR lowering | `compiler/src/lowering.rs`, `ir.rs` | A | Determinism, control-flow and trading E2E tests | P0 | Runtime parity at a distributed validator level |
| CMP-006 | X3BC emission | `compiler/src/x3bc.rs`, `emitter.rs` | A | Bytecode/version/lowering tests | P0 | Reproducible cross-platform artifact comparison |
| BC-001 | Versioned X3BC envelope | `crates/x3bc-core/src/bytecode.rs` | A | Version bound and checksum tests | P0 | Release-signed artifact provenance |
| BC-002 | Canonical codec | `crates/x3bc-core/src/codec.rs` | A | `no_std` core tests through VM gates | P0 | Fuzz corpus retained as CI artifact |
| BC-003 | Bytecode verifier | `x3-lang/vm/src/verifier.rs` | A | 358 VM tests; PR #17 rejects jumps into padding | P0 | Register/stack-depth verification remains a separate hardening item |

## VM and execution proof

| ID | Feature | Authoritative component | Status | Evidence | Risk | Missing / completion criterion |
|---|---|---|---|---|---|---|
| VM-001 | Verified instruction dispatch | `x3-lang/vm/src/executor.rs` | A | VM integration and executor tests | P0 | Runtime/native parity evidence |
| VM-002 | Deterministic gas accounting | `executor.rs`, `economic.rs` | B | Economic and gas tests | P0 | Differential gas evidence on each supported execution environment |
| VM-003 | Transactional rollback | `x3-lang/vm/src/trading.rs`, `x3vm/executor` | B | Trading execution and receipt tests | P0 | Live runtime failure/rollback evidence |
| REC-001 | Canonical trading receipts | `crates/x3bc-core/src/receipt.rs` | A | Receipt and trading-receipt tests | P0 | Independent external verifier implementation |
| PRF-001 | Deterministic replay | `x3-lang/vm/tests/trading_receipts.rs` | B | Replay and tamper tests | P0 | Cross-platform replay and chain inclusion evidence |
| PRF-002 | Chain inclusion/finality | VM/bridge evidence modules | C | Archived Ethereum/Solana fixtures and verifier code | P0 | Live finalized inclusion and reorg tests |

## Economic and cross-VM safety

| ID | Feature | Authoritative component | Status | Evidence | Risk | Missing / completion criterion |
|---|---|---|---|---|---|---|
| ECO-001 | Balance/supply invariants | VM trading and economic modules | B | Property and execution tests | P0 | Ledger-backed runtime evidence |
| ECO-002 | Flash-loan repayment | VM trading execution | B | Atomic rollback tests | P0 | Independent same-extrinsic runtime proof |
| ECO-003 | Realized profit enforcement | VM profit/trading verifier | B | Profit-path and adversarial tests | P0 | Actual settled-cost integration evidence |
| XVM-001 | Shared cross-VM interfaces | VM bridge and adapters | C | Adapter and proof fixtures | P0 | Canonical route/failure protocol across all domains |
| XVM-002 | Live cross-VM settlement | EVM/SVM adapters | C | No live settlement receipt in current repo | P0 | Finalized source and destination evidence with causality caveat |

## NEAR and AI agent

| ID | Feature | Authoritative component | Status | Evidence | Risk | Missing / completion criterion |
|---|---|---|---|---|---|---|
| NEAR-001 | Signed 1Click quotes | `x3-lang/near_intents.py` | A | Real signed fixtures and tamper tests | P0 | Live quote receipt |
| NEAR-002 | X3 Guard intent enforcement | `x3-lang/near/x3-guard` | A | 51 contract tests and exact-head CI | P0 | Deployed contract and live transaction evidence |
| NEAR-003 | Agent mandates | `near_mandate.py`, Guard mandate module | A | Python/Rust mandate tests | P0 | Deployed mandate lifecycle evidence |
| NEAR-004 | Wallet authorization boundary | Local `feat/wallet-bound-quote-authorization` work; no verified PR yet | C | Active isolated worktree | P0 | Wallet or smart-account enforcement independently proven |
| NEAR-005 | Authorized live settlement | NEAR adapter and Guard | C | No public live settlement in repo | P0 | Real transaction evidence and independent verification |
| NEAR-006 | Authenticated light-client head | `near_authenticated.py`, `near_merkle.py`, approval core | B | Real p27→Gb approvals under a test-only checkpoint authority; adversarial tests | P0 | Independently approved checkpoint plus authenticated escrow storage/code evidence |
| AI-001 | Model-generated X3 | `x3-lang/near_agent.py` | B | Provider wire tests and local run report | P1 | Anthropic evidence with provider/model/response IDs |
| AI-002 | Adversarial model tests | `near_agent.py redteam` | B | Deterministic tests; concurrent local 8/8 report | P1 | Committed JSON evidence for a real model |
| AI-003 | Benign accuracy suite | `near_agent.py` prompt pipeline | C | No committed 10-request evidence | P1 | At least 9/10 with full transcripts |

## Tooling, CI, synchronization, and infrastructure

| ID | Feature | Authoritative component | Status | Evidence | Risk | Missing / completion criterion |
|---|---|---|---|---|---|---|
| TOOL-001 | Compiler CLI | `x3-lang/crates/x3-tools/src/bin/x3c.rs` | A | CLI integration tests | P1 | Judge-facing end-to-end command |
| TOOL-002 | Formatter/linter | compiler formatter and linter | A | Formatter and linter suites | P2 | LSP integration |
| TOOL-003 | Studio/LSP/debugger | No canonical implementation | F | None | P2 | Versioned design and implementation |
| SEC-001 | Dependency advisories | Guard and x3-lang workspaces | A | No open Dependabot alerts; `wee_alloc` removed | P0 | Continuous RustSec policy |
| SEC-002 | Secret containment | CI plus exact-head review | B | GitGuardian checks | P0 | Historical-history scanning on every release candidate |
| SYNC-001 | Historical source provenance | `PRIOR_WORK.md`, `baseline` | A | 389 filtered commits and SHA map | P1 | Automated divergence monitoring |
| SYNC-002 | Two-repository mirror | Proof workflow in PR #16 | B | Active exact-head mirror check | P1 | Independent validation after merge |
| CI-001 | Nested workspace checks | `.github/workflows/x3lang-history-ci.yml` | A | Python/browser, VM, fmt, clippy, Guard | P0 | Root/nested matrix documented and enforced |
| CI-002 | Exact-head evidence | GitHub PR checks | A | PR heads with complete required jobs | P0 | Remove external status ambiguity from release gate |
| NET-001 | Seven-validator network | Infrastructure | C | No current seven-validator evidence | P0 | Seven physical validators and 72-hour endurance evidence |

## Immediate dependency order

1. Land and verify Guard schema migration and wallet-bound authorization.
2. Keep bytecode verifier hardening ahead of runtime integration.
3. Merge the execution-proof and repository-mirror gates only after independent
   exact-head validation.
4. Produce a real NEAR authorization/refusal evidence bundle.
5. Only then expand to live cross-VM settlement and validator-network claims.

