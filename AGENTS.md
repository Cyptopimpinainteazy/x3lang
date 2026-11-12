# X3Lang coding-agent instructions

These instructions apply to all assistants working in `Cyptopimpinainteazy/x3lang`.
Read `README.md` and `PRIOR_WORK.md` before editing. The original X3Lang
compiler/VM predates the NEAR hackathon; preserve real commit history,
the `baseline` tag, and provenance.

## Scope and release objective

Deliver a reproducible **X3 Shield** NEAR hackathon submission by
**2026-11-14 23:59 UTC**. Freeze unrelated features by November 12.
Prioritize real AI-generated X3Lang, deterministic policy checks,
NEAR Guard, independently verified transaction evidence, and the judge demo.

- Write only to this repository's isolated feature branches/worktrees.
- `Cyptopimpinainteazy/xxxstar` is **read-only context** for this sprint.
  No experimental cross-VM, liquidity, or agent-firewall code imports.
- Mission Control and orchestration live in separate local repositories.
- Unrelated projects are out of scope.

Before changing code, check current GitHub issues, PRs, checks, branch ownership,
and dependencies. Another agent may already be handling your proposed task.

## Git and agent safety

1. Verify the exact project checkout with `git rev-parse --show-toplevel`,
   `git remote -v`, `git status --short`, and the active branch.
2. **Stop if Git resolves to the user's home folder or another repository.**
   Never run `git add -A` from an unverified directory.
3. Use one task, one dedicated worktree, one branch, and one owner.
4. Never write directly to `master`, force-push, rewrite history, delete
   another worker's checkout, or run destructive global cleanup.
5. Changes require a focused draft PR, tests and independent review.
   Never merge financial or security changes without owner approval.

## Financial security and evidence

- Treat AI output, invoices, retrieved documents, and model instructions as
  untrusted. The compiler and deterministic mandate decide admissibility.
- Validate chain, asset, recipient, amount, nonce, expiry, approval requirements
  and refund destination through a trusted execution/signing boundary.
- **NEAR Guard does not custody swap assets.** Native NEAR storage credit pays
  for contract storage and is separate from swapped tokens. Guard-level rejection is not
  protection against an independent wallet keyholder transferring directly.
- A signed NEAR 1Click quote proves signed terms, **not** completion of a trade.
  Local test fixtures, RPC status strings, receipts with self-reported outcome,
  and balance deltas are not proof of solver-level cross-chain causality.
- Never present mocks or local simulations as on-chain transactions.
- Never print, commit or transmit seed phrases, private keys, wallet files,
  JWTs, API tokens, private receipts or `.env` files.
- No unattended wallet signing, contract deployment, token transfer,
  financial-fee activation or final hackathon submission.

## Test commands (from repository root)

Use a project-specific Python virtual environment, not global/root pip.
Run focused regression tests first, then the full applicable matrix:

```bash
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests -q
node --test x3-lang/near/demo/test/*.test.mjs
(cd x3-lang && cargo test -p x3-lang-vm --locked --quiet)
(cd x3-lang && cargo fmt --all -- --check)
(cd x3-lang && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
(cd x3-lang/near/x3-guard && cargo check --target wasm32-unknown-unknown --locked)
bash scripts/check-near-allocator.sh
```

NEAR Guard requires the WebAssembly contract target; an unsupported native
`cargo check` is not a substitute. If a dependency is missing, report the
specific blocker instead of calling the gate green.

When the judge evidence tool is integrated, run its full mode on the exact
proposed commit. Do not assume unmerged PR tools exist on `master`.

## Definition of done

- Acceptance criteria for the issue demonstrably pass.
- New behavior has success and failure regression tests.
- Local commands and GitHub CI ran on the exact proposed head SHA.
- Security review covers signing boundaries, replay, limits and evidence.
- The PR records tests, security limitations and precise changed files.
- Distinguish **implemented**, **tested**, **CI green**, **reviewed**,
  **approved to merge**, and **deployed/verified on-chain**.
- Stop after bounded failed repair attempts; escalate a precise blocker.
