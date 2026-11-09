# X3 Shield: evidence-first hackathon execution plan

Status: approved direction; implementation and release gates remain open.
Recorded: 2026-11-08. Inspection baseline: `038e82d73a522642ae755af48eb6acb838902329`.
Owner for the first increment: ChatGPT, branch `fix/shield-harness-evidence-20261108`.
Tracking: [compromised-agent boundary #119](https://github.com/Cyptopimpinainteazy/x3lang/issues/119).

## Approved design and product claim

Build one complete, understandable NEAR security demonstration, not a new trading
product or the entire X3 blockchain. Owners express a supported payment policy in
X3Lang; compilation produces reviewed configuration; a scoped agent can exercise
only the demonstrated authority; the owner can revoke that authority and recover
unused escrow. Evidence must connect each of those steps.

The desired final claim is a target, not a statement that every step already works:

> The owner writes the rule. X3Lang produces the permitted configuration. A
> compromised agent cannot exceed the demonstrated authority. The owner can stop
> it and recover unused funds. The result is independently reproducible.

Primary submission: X3 Shield, powered by X3Lang. Focus on AI Security & Defense
and Agent Economy. No claim of exclusive invention, a winning probability, an
independent custody audit, production readiness, or universal attack prevention.

The earlier competitor discussion is strategy context, not a security audit of
other projects. Do not copy unverified competitor claims into the README.

## Scope and permissions

- Work only on an isolated feature branch in `Cyptopimpinainteazy/x3lang`.
- Keep `xxxstar` read-only. Do not bypass an applicable cross-repository mirror gate.
- Preserve history, the baseline tag, provenance, and immutable historical evidence.
- Local disposable Sandbox execution is separate from public-chain execution.
- No real wallet secrets, public deployment, token funding, mainnet activity,
  repository visibility change, public post, or final submission is authorized by
  this plan. Those require separate explicit owner approval.
- Open a focused draft PR. Do not merge security changes without independent review,
  exact-head gates, and owner approval.
- Never classify unavailable infrastructure, timeout, malformed transactions, or
  unknown SDK errors as successful security refusals.
- Do not weaken tests, pins, required checks, evidence labels, or release conditions.

## Baseline: what is actually present

At the inspection SHA, the native-NEAR custody prototype and owner recovery are in
`x3-lang/near/x3-agent-vault/src/lib.rs`; the Sandbox harness is in
`x3-lang/near/agent-vault-sandbox-tests/src/main.rs`.

The harness currently has two ambiguous permission-denial paths: a direct-transfer
SDK error becomes `sdk_or_rpc_permission_refusal`, and an agent recovery SDK error
matches `Err(_) => true`. Included contract failures are also accepted without
checking the intended failure reason. These observations motivate Task 1; they do
not establish that funds can be stolen.

The vault constructor takes `owner`, `agent`, `recipient`, `cap`, `budget`, and
`deadline_ms`. The inspected harness supplies those values directly rather than
compiling a `.x3` mandate. Its budget is cumulative, not rolling-24-hour. Do not
claim language-to-custody semantic parity until Task 2 establishes it.

The current vault workflow runs vault unit tests, lint, formatting and a WASM build;
it does not by itself establish a new Sandbox run. Existing historical results do
not certify future commits.

Known limits to retain: 64 lifetime payment records, provisional recovery reserve,
missing-callback pending recovery, unaudited state migration, trusted owner keys,
and no established public deployment of this custody prototype. Never upgrade an
existing funded vault in place using the changed state layout without a reviewed
migration design.

## Implementation order and pass/fail gates

| Task | Deliverable | Acceptance gate | Status |
| --- | --- | --- | --- |
| 1 | Correct refusal evidence | Expected permission/policy errors pass; unrelated failures never pass; valid controls succeed | Core lifecycle locally verified; expiry/deposit cases and exact-head CI pending |
| 2 | Bind `.x3` semantics to custody configuration | Generated values match deployed values; changing a supported rule changes enforcement without hand-editing parameters | Not started by this plan |
| 3 | Integrated payment lifecycle | Same pinned deployment demonstrates allowed payment, adversarial requests, replay, fresh post-revocation request and recovery | Not verified by this plan |
| 4 | Judge demo and evidence page | One understandable sequence, evidence labels and limitations; no splicing unrelated runs into a single transaction | Pending |
| 5 | README and release reconciliation | Claims match code, execution environment and actual deployment; anonymous access verified after publication approval | Pending |

### Task 1: refusal evidence, not generic failure detection

Files to inspect/change:
- `x3-lang/near/agent-vault-sandbox-tests/src/main.rs`
- Focused regression tests alongside that harness.
- `.github/workflows/x3-agent-vault-testnet.yml`, only to add real checks, never to remove gates.

Test-first requirements:
1. Demonstrate the current false-positive classification with a regression before
   modifying the implementation. Synthetic error inputs are unit tests, never chain evidence.
2. Separate pre-inclusion permission rejection from an included contract failure.
3. Match the expected structured access-key reason for the attempted action.
   Generic `InvalidTransaction` or an error string alone is insufficient when the
   underlying subtype is unavailable. Fail with an inconclusive diagnostic instead.
4. Propagate transport, timeout, malformed transaction, nonce, gas and unknown errors
   as nonzero failures. Do not print successful-refusal markers for those cases.
5. For contract-level negative cases, require the intended policy failure and verify
   relevant state/balances. Pair each restriction with an otherwise-valid control.
6. Test direct transfer, unauthorized method, wrong receiver and attached deposit
   separately from recipient, amount, nonce, expiry, budget and revocation rules.
7. Test a fresh nonce after revocation so replay protection cannot mask a broken
   revocation check. Preserve terminal payment state after later rejected requests.
8. Run formatter, strict lints, unit regressions, the real isolated Sandbox harness
   and applicable exact-head CI. Report each separately; do not infer Sandbox success
   from a successful WASM build.

If the pinned SDK loses the necessary reason, stop the claim at inconclusive and
implement a reviewed raw-response path. Do not broaden the matcher to regain green.

### Task 2: language-to-enforcement binding

Implement a narrow native-NEAR custody backend rather than pretending the full
Guard mandate language is supported. Inspect the existing compiler interfaces before
choosing a new one; avoid a competing parser or second policy evaluator.

The compiled manifest must bind source and compiler identity, policy version,
network, token/units, principals, recipient, cap, cumulative budget, expiry and
expected contract code identity. Hashes identify artifacts; tests establish semantics.

Acceptance:
- One supported `.x3` rule change changes admission without editing Rust or manually
  replacing constructor arguments.
- Deployment configuration is generated and owner-reviewed, then checked against
  actual contract configuration.
- Rolling daily budgets, unsupported assets, refund semantics, approval thresholds,
  slippage or other unsupported constructs fail closed. Never silently weaken them.
- Cross-owner, wrong-network, stale-policy and altered-manifest inputs are rejected.
- No deployment or signing occurs as a side effect of compilation or verification.

### Task 3: complete custody lifecycle

Use one pinned source/WASM/configuration set and explicit network label. Record an
allowed payment exactly once; wrong recipient; excessive amount; pending-budget
concurrency; replay; expiry; revocation followed by a fresh request; owner-only
recovery; and failure/callback accounting. Distinguish unit coverage from actual
Sandbox receipts. Do not claim revocation reverses an executed or scheduled transfer.

Before any owner-approved public testnet run, review initialization, key separation,
funding limit, gas/storage reserve, recovery, deployment identity and teardown.
Local owner keys retained by a test harness do not demonstrate isolated production
key custody. Never disclose even disposable secret keys in the repository or chat.

### Task 4: honest three-minute demonstration

Use an assumed-compromised agent for repeatable execution-boundary testing. Label it
as deliberately adversarial. A separate live-model observation reports what that
model actually did; do not weaken a model and market it as a normal injection result.

Sequence: owner policy -> allowed payment -> unauthorized request -> direct bypass
attempt -> revoke -> fresh request refused -> recover unused escrow -> verify.
Show local/Sandbox/public-testnet/archived evidence labels throughout. Pre-inclusion
rejections have no included receipt; never invent an explorer link for them.

Keep historical Tidepool delivery separate. It was a trusted-relayer transfer, not
proof that this newer custody vault protected that payment. The advisory Guard,
scoped-key permissions, vault rules and settlement evidence are distinct layers.

### Task 5: presentation and release

Refresh stale draft/merged descriptions without upgrading deployment claims. Put the
user problem, one runnable path with its prerequisites, one result and the trust
boundary first. Keep deeper light-client/cross-chain material accessible separately.
Preserve prior-work disclosure and identify the NEAR-specific contribution.

Per `AGENTS.md`, freeze unrelated features by November 12 and target submission by
November 14, 2026, 23:59 UTC (16:59 Denver). Recheck live organizer rules before the
owner-approved final submission. Public repo, public demo post, visibility changes,
payout details and submission confirmation remain owner-controlled.

## Work deliberately deferred

No new multi-chain custody promise, additional VM adapter, mainnet trade requirement,
validator launch, broad DEX work, or X3 OS redesign. A future integration with other
agent frameworks is not a completed feature. One defensible enforcement path wins
priority over several unverified ones.

## Verification ledger and rollback

Every increment records its base/head SHA, changed files, commands, exact results,
known blockers, review status and deployment status. Keep implemented, tested,
CI-passing, independently reviewed, approved and deployed as separate states.

Rollback for this first increment: close the draft PR or revert its focused commit
through normal review. No deployed contract or funds should need rollback because
this increment changes documentation/testing only. Do not restore ambiguous errors
as successful-refusal evidence to make a red check green.

## First-increment execution environment

GitHub read/write access is available. Desktop Commander is paused by its monthly
usage limit; SentinelX reports no enrolled host. The conversation container has no
Rust toolchain and cannot currently resolve external download hosts. These are
verification constraints, not successful test results. Do not claim a local Cargo
or Sandbox run unless an actual execution path becomes available and the command
is run. A draft PR may document a precise blocker; it must not waive the gate.
