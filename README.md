# X3 Shield (X3Lang) — verify what an AI agent is about to pay

**Not another policy-bound wallet.** NEAR's [AI Security & Defense track](https://near.army/)
already calls for spending limits, policy engines and sandboxed wallets; the
Agent API offers policy-bound agent wallets, and NEAR function-call access keys
restrict which contract methods a key can invoke. **X3 Shield instead explores
a programmable `.x3` transaction policy that checks the *signed terms* of a
NEAR Intents / 1Click quote against the owner's mandate before the
Guard-mediated deposit route is accepted.** It is complementary to wallet/key
restrictions, not a replacement for them.

**Judge the difference:** Can an AI agent present a *correctly signed* quote
that pays the wrong recipient or violates a cap, and will an independently
checked policy reject it **even though its signature is valid**? That is the
security problem this submission tackles—not the already-established idea
of giving agents spending limits.

**Current evidence level: `local_policy_preview_only` + `testnet_registration_refusal_and_live_acceptance_only`.** Below is the honest
three-minute screen. Architecture detail, value-fee economics and release
boundaries follow after it.

## WORKS TODAY

- One command (`60-SECOND JUDGE PATH` below) runs scenario freshness, the four-scenario
  terminal demo (`authorized`, `needs-approval`, `wrong-recipient`,
  `prompt-injection-recipient`), Python/browser guard-by-guard parity, and the judge
  evidence gate (`quick`), all local with no wallet, signing or deployment
  (first-run dependency installation may use the network). It exits nonzero on
  stale scenarios, policy disagreement, failed tests, or a missing
  prompt-injection case.
- The **prompt-injection** case shows an injected instruction steering the agent to pay
  `attacker.near`; the owner mandate refuses at `mandate_recipient`. It evaluates the
  **same checked-in DENIED payload** (`x3-lang/examples/near_intents_agent_denied.x3`)
  as `wrong-recipient` — only the story differs — so the refusal is the recipient guard
  deciding, not the model being trusted.
- Separately, a **mock-chain Guard proof** (`cargo test valid_test_signature_with_forbidden_recipient_never_authorizes_a_deposit`)
  shows a correctly **TEST-signed** quote paying a forbidden recipient is refused *after*
  signature validation, while an allowed control is accepted. Test-only key, local only.

| Claim | What exists | What is still missing |
| --- | --- | --- |
| `.x3` mandate | Python/browser policy-preview parity; Rust `x3c intent` preserves `under mandate` as adapter metadata (draft [#68](https://github.com/Cyptopimpinainteazy/x3lang/pull/68)) | An enforced mandate inside the general Rust VM; parity must be proved across actual production paths |
| Signed quote checks | `x3-guard` verifies the pinned 1Click manager's signature **before** evaluating stored quote/mandate terms in its contract code and tests; testnet deployment `x3lang-guard-20261106.testnet` shows allowed registration, invalid-signature refusal, revocation, official-dry valid-signature refusal, and official-live valid acceptance ([evidence](docs/hackathon/GUARD_TESTNET_EVIDENCE.md)) — **mixed: test-key invalid-signature path + official dry/live manager-signed quotes; live acceptance stopped before any deposit** | `mandate_recipient` on-chain acceptance refusal ([#73](https://github.com/Cyptopimpinainteazy/x3lang/issues/73) partially open) |
| Cryptographic proof | Real archived receipt/Merkle checks; real validator approvals (12 and 19 signatures) under an **untrusted RPC checkpoint** in draft [#60](https://github.com/Cyptopimpinainteazy/x3lang/pull/60); `.x3` → X3VM *read-only refusal* in draft [#72](https://github.com/Cyptopimpinainteazy/x3lang/pull/72) | Independently authenticated checkpoint, Guard's own receipt, storage proof and cross-chain causality |

## DELIBERATELY BLOCKED

- No `mandate_recipient` on-chain acceptance refusal and no mainnet deployment claim ([#73](https://github.com/Cyptopimpinainteazy/x3lang/issues/73) partially open; live acceptance authorizes a deposit address only, with no deposit/settlement attempted).
- No mainnet transaction links, no wallet custody, and no universal spending lock:
  an unrestricted wallet key can still bypass Guard.
- No settlement or cross-chain causality claim. A signed 1Click quote authenticates
  **quote terms**, not a completed swap; a valid Merkle path under an untrusted RPC
  root is **not trusted finality**; a `verified_outcome` flag in local JSON is not proof.

## 60-SECOND JUDGE PATH

```bash
bash scripts/hackathon/judge_one_command.sh /tmp/x3-shield-judge
```

**One-command judge gate: Linux x86_64 + Python 3.12 required.** The hash lock in
[`scripts/hackathon/requirements-judge.txt`](scripts/hackathon/requirements-judge.txt)
was validated only there (`--require-hashes`, full 9-package freeze); on Linux
x86_64, any other Python version fails clearly (exit 2). The general X3Lang
bridge/evidence code (`x3-lang/bridge/near_evidence.py`) retains its Python 3.11+
minimum. Rust/Cargo and Node.js are also required. By default the gate always
creates or reuses the isolated pinned venv and enforces the full 9-package freeze
(cryptography 50.0.2, pytest 9.2.0, pycryptodome 4.3.0 and six more); system
pytest/cryptography are never sufficient on their own. A caller-managed `PYTHON`
requires explicit opt-in via `X3_JUDGE_ALLOW_SYSTEM_PYTHON=1` plus the same exact
validation, or the script refuses (exit 2). Provisioning builds in a temporary
venv and only promotes it after a successful install plus validation, so a failed
run leaves no partial env behind. On first run, dependency installation may use the network.
This builds no blockchain transactions, runs
local policy/Guard tests, and writes evidence artifacts under the report directory.
It **does not** deploy a contract, perform a live payment, prove real-world prompt
injection resistance, or establish NEAR consensus finality. See the
[three-minute judge guide](docs/hackathon/JUDGE_GUIDE.md) and the
[demo video script](docs/hackathon/DEMO_VIDEO_SCRIPT.md) for exact claims.

## ON-CHAIN EVIDENCE

- **Claimed now:** deterministic local policy verdicts, local mock-chain contract
  behavior, plus testnet code identity, allowed registration, invalid-signature refusal,
  revocation, official-dry valid-signature refusal, and official-live valid acceptance
  (stopped before deposit). See
  [GUARD_TESTNET_EVIDENCE.md](docs/hackathon/GUARD_TESTNET_EVIDENCE.md) and
  [`evidence/near-guard-testnet/`](evidence/near-guard-testnet/manifest.json).
- **Exact testnet anchors:** contract
  `x3lang-guard-20261106.testnet`; WASM SHA-256
  `ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab`;
  deployment
  [`FJ5vo…`](https://explorer.testnet.near.org/transactions/FJ5voQaArx17QV4TzDLYDq93cuoAVLSRxjcxbswuyE8v);
  official-dry signature-valid recipient-mismatch failure
  [`3Q7q…`](https://explorer.testnet.near.org/transactions/3Q7qTZqiRBB6EuhN78Ra5Q5NBqPXDhfuCg2FKW8WoU2q);
  official-live allowed positive control
  [`6B6k…`](https://explorer.testnet.near.org/transactions/6B6k5kre4mFoBq7RuJNadQgrhEvmgb9WDLepX22CxzUZ).
  Every committed observation is re-queried by the evidence verifier; RPC
  agreement is not independent light-client finality.
- **Explicitly not claimed:** wallet transfer, deposit to the accepted address, observed
  settlement, finalized cross-chain settlement, a live forbidden-quote refusal,
  or proven cross-chain causality.
- Public mainnet links remain **PENDING — not fabricated**.

## SECURITY MODEL

- **AI output is untrusted.** The compiler and the deterministic mandate decide
  admissibility; retrieval, invoices and model text never authorize a payment.
- **NEAR Guard does not custody funds.** Guard can only constrain Guard-mediated
  actions; the mandate binds recipients, assets, caps, refunds, fees, slippage,
  deadlines, nonces and approvals for Guard methods, not a third-party wallet key.
- Every sensitive path names its evidence level in the console and in the judge
  report; unverified is said plainly instead of being upgraded to “proven.”

X3Lang is an existing experimental compiler, transaction-policy VM and
cross-chain verification toolkit developed as part of the X3 blockchain.
Its pre-hackathon history is [preserved and disclosed](PRIOR_WORK.md).

Unlike NEAR's policy-bound wallets or function-call keys, X3 mandates are a
typed language that also verifies NEAR Intents' signed quote terms before a
deposit is authorized.

**Works today** — a `.x3` mandate compiles and the same verdict is reproduced
in Python, the browser, and the NEAR guard contract; the guard's on-chain
Ed25519 check verifies a real 1Click quote's signed terms (locally tested);
refusal-first tests cover overspend, wrong recipient, replay, refund
redirection, and revocation.

**Deliberately blocked** — no wallet transfer, live settlement, or on-chain
authorization is claimed here. Cross-chain causality stays "unproven" until an
independently verifiable link exists.

**Who this protects.** A team runs an AI agent that pays for API inference and
data through NEAR Intents under one owner mandate
(`x3-lang/examples/near_mandate_agent.x3`); the agent holds a NEAR account, never
the wallet key. A request injected into its prompt to pay `attacker.near` is
compiled and checked like any other program, and the mandate's `mandate_recipient`
term refuses it. That "model falls for it, guard still refuses" case is pinned by
`x3-lang/tests/test_near_agent.py`, and a real local model held all 8 adversarial
requests in [`evidence/agent-redteam-local-2026-11-06`](evidence/agent-redteam-local-2026-11-06).

**Hackathon judges:** [Reproduce NEAR tests and inspect real-proof limitations](docs/hackathon/JUDGE_GUIDE.md).
The read-only “do X, see Y” gate is `python3 scripts/judge_evidence.py --mode quick`;
public mainnet transaction links remain pending and are **not** fabricated.

**Hackathon disclosure:** the original language and its trading engine were
created **before** the current NEAR hackathon. This repository preserves their
real Git history. Start with [PRIOR_WORK.md](PRIOR_WORK.md), including its
[`baseline...main` historical comparison](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...main).

The exact historical source is
[`xxxstar` / `feat/near-intents-adapter` @ `95587205`](https://github.com/Cyptopimpinainteazy/xxxstar/commit/95587205d5febc40c1047c845f379946c023228d).
The source was extracted with `git-filter-repo` from a **full Git clone**;
commit authorship and chronology were preserved, not condensed into a
single import commit. Review the SHA map in `docs/provenance/`.

## Try the judge console in 60 seconds

```bash
python3 -m http.server 8000 --directory x3-lang/near/demo
# Open http://localhost:8000/judge.html
```

The console shows one owner mandate deciding the allowed, approval-required
and forbidden actions shown by the checked-in X3 programs. The browser runs
the same mandate checks as the production adapter, and the page compares its
verdict with Python guard by guard.

For the terminal version:

```bash
python3 scripts/hackathon/demo.py
bash scripts/hackathon/verify.sh
```

### What a judge can inspect

| Question | Where to look |
|---|---|
| What exactly did the owner authorize? | Panel 1, the `.x3` mandate source and hash |
| What did the agent request? | Panel 2, the exact intent and changed lines |
| Was it allowed, denied, or held for approval? | Panel 3, verdict plus every guard |
| What cross-chain route would execute? | Panel 4, source/destination assets and bounds |
| What is actually proven? | Panel 5, evidence ladder and recorded test runs |
| How do I reproduce it? | `build_scenarios.py`, `collect_evidence.py`, `scripts/hackathon/demo.py` |

The scenario set covers spending caps, the approval threshold, unauthorized
recipients, refund redirection, fee and slippage limits, wrong mandate owners,
rolling spend, revocation and nonce replay. Each scenario names its judge
takeaway and whether its decisive evidence is local or contract-level.

### Evidence levels

| Level | What it means | Current demo |
|---|---|---|
| Local policy | The mandate and intent compile, and Python/browser checks agree | Proven |
| Signed quote | An official 1Click response verifies against the pinned manager key | Not contacted by the offline console; official dry/live artifacts verified separately |
| On-chain authorization | A deployed x3-guard registers and accepts the policy | Testnet dry-refusal and live-allowed paths recorded; no wallet transfer |
| Wallet transfer | The owner or authorized signer sends funds | Not claimed |
| Settlement | Source and destination chains are independently observed | Not claimed |
| Causality | One transaction is cryptographically proven to cause the other | Not proven |

The browser console is therefore a **policy-preview and refusal demo**, not a
payment executor. The terminal `verify.sh` emits a machine-readable readiness
report and fails when evidence is stale, dirty or missing.

## Contents

- **`x3-lang/`** — Python financial-intent DSL, Rust compiler, virtual machine,
  tests, examples, NEAR Intents adapter, x3-guard smart contract, and browser demo.
- **`crates/x3bc-core/`** — deterministic X3BC bytecode definitions required
  by the X3Lang compiler.
- **`docs/x3-lang/fixtures/` and `scripts/proof/`** — archived proof fixtures
  and verifier generator used by existing VM integration tests.

The Solana vault, BSC value-fee economics, and the NEAR Intents rollout/pilot
notes live in [docs/EXTENDED_SURFACES.md](docs/EXTENDED_SURFACES.md).

### Reproducible local checks

```bash
python3 -m pip install -r x3-lang/requirements.txt pytest
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests -q
(cd x3-lang && cargo test -p x3-lang-vm --locked --quiet)
(cd x3-lang && cargo fmt --all -- --check)
(cd x3-lang && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
node --test x3-lang/near/demo/test/*.test.mjs
```

### Public browser demonstration

[**X3 Guard — signed dry-quote validation**](https://x3-guard.pages.dev/)
verifies a real 1Click quote and checks risk rules. This is **not** proof of
an authorized wallet transaction or a completed cross-chain settlement.
The local `judge.html` console is the deterministic, offline companion: it
does not contact 1Click or NEAR and can be run entirely from the repository.

### Hackathon judge package

The offline three-track demonstration needs no wallet, network request or
funds:

```bash
python3 scripts/hackathon/demo.py
bash scripts/hackathon/verify.sh
```

It shows one owner mandate deciding an allowed payment, an approval-required
payment and a refused payment through the same X3Lang policy checker. See
[`docs/hackathon/JUDGE_PACKAGE.md`](docs/hackathon/JUDGE_PACKAGE.md), the
[three-minute walkthrough](docs/hackathon/VIDEO_WALKTHROUGH.md) and the
[submission checklist](docs/hackathon/SUBMISSION_CHECKLIST.md). Local policy
acceptance is not wallet authorization, a signed quote or settlement.

## Release boundaries

- Historical X3Lang source is not newly invented hackathon work.
- The NEAR Guard code is under review; do not authorize a wallet or deposit
  based only on dry-quote validation.
- This source **does not** include the experimental LP-pool or agent-firewall
  modules from `xxxstar` draft PR #727; those contain unresolved review issues.
- Cross-chain causality/settlement attribution and refund proofs remain
  release gates. There is no on-chain X3Lang trading execution claim here.
- The implemented controls and unresolved wallet, policy-binding, migration,
  storage, and settlement risks are tracked in the
  [X3 Shield threat model](docs/security/X3_SHIELD_THREAT_MODEL.md).
- Licensing, independent security review and exact-head GitHub Actions
  verification are required before a production release.

## What happens after the hackathon

- Release gates stay fail-closed: wallet-level enforcement, tuple-bound
  destination assets and recipients, bounded intent storage, versioned
  migration review, and explicit contract upgrade-authority lock-down.
- The next technical gate is an independent security review of the guard
  contract, the model pipeline, and the deployment configuration.
- The light client's offline checkpoint ceremony runs with real signers, then
  a live forbidden-quote refusal, a separately authorized deposit, and an
  independently verified receipt close the loop the offline demo deliberately
  leaves open.

## License

Dual-licensed under **MIT OR Apache-2.0**, matching the SPDX expressions in
the crate manifests. The MIT text is in [LICENSE](LICENSE); the Apache-2.0
text is the standard Apache License, Version 2.0.
