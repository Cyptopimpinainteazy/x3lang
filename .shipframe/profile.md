# X3Lang Ship Profile

This profile is the source of truth for ShipFrame work in
`Cyptopimpinainteazy/x3lang`.

## Release objective

Ship a reproducible, evidence-backed X3Lang / X3 Shield submission by
**2026-11-14 23:59 UTC**. Freeze unrelated feature work on **2026-11-12**.

The release is not complete when a local test passes. Track these states
separately:

1. implemented
2. tested locally
3. exact-head GitHub Actions green
4. independently reviewed
5. approved to merge
6. merged to `main`
7. deployed and observable in the target environment
8. verified on-chain, where applicable

## Repository topology

- `x3-lang/` - Python intent DSL, Rust compiler and VM, browser demo.
- `x3-lang/near/x3-guard/` - standalone NEAR Rust/Wasm guard contract.
- `x3-lang/solana/x3-mandate-vault/` - standalone Solana SBF/SPL vault.
- `x3-lang/bridge/` - NEAR and BSC bridge prototypes.
- `crates/x3bc-core/` - shared deterministic bytecode definitions.
- `scripts/` - proof, judge, and release-evidence tooling.
- `docs/` - judge, security, provenance, deployment, and release material.
- `evidence/` - committed evidence fixtures; never fabricate or rewrite them.

## Branch and merge rules

- Canonical base: `origin/main`.
- Integration branch: `release/x3lang-submission-20261106`.
- One task gets one branch, one worktree, and one writing agent.
- Never write directly to `main`.
- Never force-push or rewrite history.
- Follow `docs/security/STANDALONE_CI_POLICY.md` for source admission and manual
  read-only peer parity. Automatic CI does not assert equality with `xxxstar`.
- Merging financial, contract, cryptographic, or CI-policy changes requires
  explicit owner approval.
- Build a PR dependency graph before selecting merge order.
- Rebase or update a PR only after preserving its previous exact head.

## Agent roles

| Agent | Role | Write policy | Stop condition |
|---|---|---|---|
| Codex `x3lead` | Release lead and integrator | Integration branch only | Stop before merge, deploy, wallet action, or submission |
| Claude `x3review` | Independent architecture and security reviewer | Read-only | Report findings; do not edit or merge |
| OpenCode `x3build` | Scoped implementation and local verification | One assigned task worktree | Stop when scoped task and evidence report are complete |

Only `x3lead` coordinates other agents. Workers must not prompt or control one
another. Chat output is not the shared source of truth; Git state, diffs,
test output, CI run IDs, and committed evidence are.

## Active ShipFrame skill stack

### Mandatory for every code change

- `shipframe:project-profile`
- `shipframe:project-memory-refresh`
- `shipframe:wiki-query`
- `shipframe:plan-expert`
- `shipframe:tdd` for features and fixes
- `shipframe:bug-diagnosis` for broken behavior
- `shipframe:implement-task`
- `shipframe:code-review`
- `shipframe:security-review`
- `shipframe:proof-runner`
- `shipframe:evidence-audit`
- `shipframe:create-pr`

### Mandatory when the change touches the matching surface

- `shipframe:api-contract-review` - compiler IR, manifest, RPC, or contract API
- `shipframe:dependency-upgrade` - Cargo, Python, npm, or toolchain changes
- `shipframe:live-docs` - NEAR SDK, Solana SDK, Rust, Python, or browser APIs
- `shipframe:e2e-verify` - judge flow, browser console, or cross-runtime path
- `shipframe:frontend-release` - browser demo or static-site changes
- `shipframe:backend-release` - NEAR, Solana, bridge, or service changes
- `shipframe:security-hardening` - signing, authorization, replay, storage,
  settlement, or deployment-boundary changes

### Mandatory for the submission

- `shipframe:release-checklist`
- `shipframe:project-release`
- `shipframe:deploy-evidence`
- `shipframe:evidence-audit`
- `shipframe:handoff`
- `shipframe:wiki-sync`

### Deferred unless the task requires them

Design-system, accessibility, copy-review, incident-response, task creation,
and README-generation skills are not on the critical shipping path. Use them
only when the task materially needs them.

## Local verification gates

Use a repository-local `.venv`; do not install into the system Python.

```bash
PYTHONPATH=x3-lang .venv/bin/python -m pytest x3-lang/tests -q
node --test x3-lang/near/demo/test/*.test.mjs
(cd x3-lang && cargo test -p x3-lang-vm --locked --quiet)
(cd x3-lang && cargo fmt --all -- --check)
(cd x3-lang && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
(cd x3-lang/near/x3-guard && cargo check --target wasm32-unknown-unknown --locked)
bash scripts/check-near-allocator.sh
```

Run focused tests first. Run every applicable repository workflow on the exact
proposed head SHA. A local pass cannot substitute for exact-head CI.

## Judge and release evidence

```bash
python3 -m pip install -r x3-lang/requirements.txt pytest
python3 scripts/judge_evidence.py --mode quick
python3 scripts/hackathon/demo.py
bash scripts/hackathon/verify.sh
```

The final release report must distinguish:

- local policy verdict
- signed dry quote verification
- on-chain contract authorization
- wallet transfer
- source-chain observation
- destination-chain observation
- cross-chain settlement and causality

Do not use a signed quote as proof of execution. Do not use a receipt or balance
delta as proof of cross-chain causality. Do not claim a deployment without a
target network, contract ID, transaction hash, and observable result.

## Required task handoff

Every worker report must include:

- task and exact base SHA
- branch and worktree
- files changed
- commands run and exit status
- local test results
- exact-head CI run URL or explicit `not run`
- known limitations and unverified claims
- recommended next gate

Use `.shipframe/templates/task-brief.md` when dispatching work.
