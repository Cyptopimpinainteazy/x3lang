# X3Lang Grand-Prize Submission Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a public, exact-head-green X3Lang submission that demonstrates an owner mandate refusing a prompt-injected payment locally and on NEAR testnet.

**Architecture:** Build a focused release branch from `main`, integrate only the reviewed mandate/Guard path, add a deterministic prompt-injection scenario to the existing cross-runtime judge console, then deploy the reviewed Guard to a dedicated funded testnet account. Keep the offline proof as the guaranteed path; add testnet evidence and an independently re-queryable evidence package without claiming settlement or custody.

**Tech Stack:** Python 3.12, Node.js 22, Rust 1.90, NEAR SDK 5.30, `wasm32-unknown-unknown`, `near` CLI 0.31.1, GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-11-06-x3lang-grand-prize-design.md`

## Global Constraints

- Canonical base is `origin/main`; the release branch is `release/x3lang-submission-20261106`.
- Deadline is **2026-11-14 23:59 UTC**; feature freeze is **2026-11-12**.
- Never deploy to mainnet, authorize custody, sign wallet transfers, or claim cross-chain settlement/causality.
- Testnet deployment and chain signing are explicitly owner-authorized for this demo only.
- Use a dedicated testnet account and newly generated testnet-only key stored outside the repository with owner-only permissions.
- Never print, commit, or copy private key material into logs or evidence.
- Do not weaken the peer-repository mirror gate. Create matching mirrored content or stop.
- Exact-head GitHub Actions must be green for the final public commit.
- One task, one owner, one isolated worktree; the lead alone integrates the release branch.

## Review Focus

- A fresh clone with no private environment must run the judge command without
  application network access after one documented dependency-provisioning step.
- A cryptographically valid quote to an unauthorized recipient must fail on `mandate_recipient`, not on signature parsing.
- Testnet evidence must fail when a transaction hash, receipt status, block height, chain identity, or deployed code hash cannot be independently re-queried.
- The evidence collector and demo must fail before reading or printing a key file, `.env` file, token, or private receipt.
- A dirty or unpushed final SHA must never pass readiness, exact-head CI, or submission checks.

---

### Task 1: Establish the focused release candidate

**Files:**
- Modify by cherry-pick: `x3-lang/ed25519.py`
- Modify by cherry-pick: `x3-lang/near_intents.py`
- Modify by cherry-pick: `x3-lang/compiler/src/{intent_emit.rs,lib.rs,lowering.rs,parser.rs,semantic.rs}`
- Modify by cherry-pick: `x3-lang/crates/x3-ast/src/ast.rs`
- Modify by cherry-pick: `x3-lang/near/x3-guard/src/mandate_tests.rs`
- Modify by cherry-pick: `x3-lang/near/demo/judge.html`
- Modify by cherry-pick: `.github/workflows/x3-shield-judge-readonly-quick.yml`
- Create if required: `Cyptopimpinainteazy/xxxstar` branch `release/x3lang-submission-20261106`
- Test: existing focused suites from the integrated branches

**Interfaces:**
- Consumes: exact PR heads `#66=3e131a2f05c5a7d62fb7423ea031240a971776d5`, `#68=4d169dee4a0bb57c09213c322991d9bd6ca80a4`, `#69=4c93da2f409b0d5847c839cc584c60809180d4b4`, `#75=5bb92bbcaa914892dda9ec237c3659a468528ea4`, and `#76=c990c4d6b4defe7565a49d45613e2484b7dd90a4`.
- Produces: a release branch containing the strict Ed25519, mandate metadata/compiler, signed-quote refusal, deployment preflight, and release-doc path, with no bridge/HTLC/liquidity integration.

- [ ] **Step 1: Verify the release worktree**

Run:

```bash
git rev-parse --show-toplevel
git branch --show-current
git rev-parse HEAD
git status --short
```

Expected: `/home/lojak/Desktop/x3lang-ship-20261106`, `release/x3lang-submission-20261106`, base `136aa8804f0c150ba5c813c013f740db6a2773e7`, with only the intentional ShipFrame/wiki/design artifacts untracked.

- [ ] **Step 2: Fetch the five exact PR heads and record their commits**

Run:

```bash
git fetch origin pull/66/head:refs/remotes/origin/pr66 pull/68/head:refs/remotes/origin/pr68 pull/69/head:refs/remotes/origin/pr69 pull/75/head:refs/remotes/origin/pr75 pull/76/head:refs/remotes/origin/pr76
for n in 66 68 69 75 76; do git rev-parse "origin/pr$n"; done
```

Expected: the five SHAs in the Interfaces block.

- [ ] **Step 3: Cherry-pick the minimum release path in dependency order**

Run:

```bash
git cherry-pick 3e131a2f05c5a7d62fb7423ea031240a971776d5
git cherry-pick 4d169dee4a0bb57c09213c322991d9bd6ca80a4
git cherry-pick 4c93da2f409b0d5847c839cc584c60809180d4b4
git cherry-pick 5bb92bbcaa914892dda9ec237c3659a468528ea4
git cherry-pick c990c4d6b4defe7565a49d45613e2484b7dd90a4
```

Expected: five clean commits. Resolve conflicts narrowly; do not import bridge, HTLC, Chain Signatures, liquidity, MPC, or value-fee code.

- [ ] **Step 4: Run focused integration gates**

Run:

```bash
PYTHONPATH=x3-lang .venv/bin/python -m pytest x3-lang/tests/test_ed25519_strict.py x3-lang/tests/test_near_intents.py -q
(cd x3-lang && cargo test -p x3-lang-compiler --locked)
(cd x3-lang && cargo test -p x3-lang-vm --locked --quiet)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
node --test x3-lang/near/demo/test/*.test.mjs
```

Expected: every command exits `0`.

- [ ] **Step 5: Resolve the mirror gate without weakening it**

Run:

```bash
python3 scripts/proof/check_x3lang_mirror.py --help
git ls-remote --heads https://github.com/Cyptopimpinainteazy/xxxstar.git release/x3lang-submission-20261106
```

Expected: the help command succeeds and the peer branch query is understood. Create the matching peer branch only if the mirror script identifies exact files that must exist there; otherwise stop and report the mirror policy blocker without changing CI policy.

- [ ] **Step 6: Commit release integration**

Run:

```bash
git status --short
git diff --check
git add x3-lang .github scripts docs
git commit -m "feat(release): integrate X3 Shield refusal path"
```

Expected: one commit containing only the selected core path and its tests.

---

### Task 2: Add the prompt-injection mandate scenario

**Files:**
- Modify: `x3-lang/near/demo/judge/build_scenarios.py`
- Modify: `x3-lang/near/demo/judge/scenarios.json`
- Modify: `scripts/hackathon/demo.py`
- Modify: `x3-lang/tests/test_hackathon_support.py`
- Modify: `x3-lang/near/demo/test/judge.test.mjs`
- Test: `x3-lang/near/x3-guard/src/mandate_tests.rs`

**Interfaces:**
- Consumes: `track_control.evaluate(mandate_path, intent_path, tokens, now, assumed_state)` and `scripts.hackathon.demo.build_report() -> dict`.
- Produces: scenario id `prompt-injection-recipient`, decision `denied_by_policy`, denied guard list `["mandate_recipient"]`, and a terminal/browser story that explicitly describes model prompt injection.

- [ ] **Step 1: Write the failing Python expectations**

Add to `x3-lang/tests/test_hackathon_support.py`:

```python
def test_prompt_injection_is_refused_by_the_recipient_guard():
    demo = load_script("scripts/hackathon/demo.py")
    by_id = {scenario["id"]: scenario for scenario in demo.build_report()["scenarios"]}
    scenario = by_id["prompt-injection-recipient"]
    assert scenario["decision"] == "denied_by_policy"
    assert scenario["denied_guards"] == ["mandate_recipient"]
    assert "prompt" in scenario["story"].lower()
```

- [ ] **Step 2: Run the new test to verify it fails**

Run:

```bash
PYTHONPATH=x3-lang .venv/bin/python -m pytest x3-lang/tests/test_hackathon_support.py::test_prompt_injection_is_refused_by_the_recipient_guard -q
```

Expected: FAIL with `KeyError: 'prompt-injection-recipient'`.

- [ ] **Step 3: Add the scenario definition**

Add a group-B scenario to `SCENARIOS` using `DENIED` as its source:

```python
{"id": "prompt-injection-recipient", "group": "B",
 "title": "Prompt-injected payment",
 "story": "A malicious instruction convinces the AI agent to pay attacker.near, but the owner never authorized that recipient.",
 "takeaway": "The model can generate any request; the compiled mandate stops recipient substitution.",
 "source": DENIED, "edits": []}
```

Add `"prompt-injection-recipient"` to `DEMO_SCENARIOS` in `scripts/hackathon/demo.py`. Replace the fixed three-row renderer with iteration over `report["scenarios"]` so all selected decisions render by scenario title.

- [ ] **Step 4: Regenerate and verify browser parity**

Run:

```bash
PYTHONPATH=x3-lang .venv/bin/python x3-lang/near/demo/judge/build_scenarios.py
PYTHONPATH=x3-lang .venv/bin/python -m pytest x3-lang/tests/test_hackathon_support.py -q
node --test x3-lang/near/demo/test/judge.test.mjs
```

Expected: PASS, including browser/Python agreement for the new scenario.

- [ ] **Step 5: Reuse the integrated valid-signature Guard test**

PR #76 adds
`valid_test_signature_with_forbidden_recipient_never_authorizes_a_deposit`.
Use that existing test rather than duplicating its cryptography. It must prove:

- the prohibited quote is signed after changing the recipient;
- `quote_signature` passes;
- `quote_request_echo` fails;
- `agent_accept_quote` refuses without recording accepted deposit state or spend;
- a separately signed allowed quote still succeeds.

- [ ] **Step 6: Run the contract test and full focused suite**

Run:

```bash
(cd x3-lang/near/x3-guard && cargo test --locked valid_test_signature_with_forbidden_recipient_never_authorizes_a_deposit -- --nocapture)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add x3-lang/near/demo x3-lang/tests/test_hackathon_support.py scripts/hackathon/demo.py
git commit -m "feat(demo): show mandate refusal of prompt injection"
```

---

### Task 3: Make the judge path one command and three-minute clear

**Files:**
- Modify: `scripts/hackathon/judge_one_command.sh`
- Modify: `scripts/hackathon/verify.sh`
- Modify: `x3-lang/near/demo/judge.html`
- Modify: `x3-lang/near/demo/judge/collect_evidence.py`
- Modify: `x3-lang/near/demo/test/judge.test.mjs`
- Modify: `README.md`
- Create: `docs/hackathon/DEMO_VIDEO_SCRIPT.md`
- Test: existing `x3-lang/tests/test_hackathon_support.py`

**Interfaces:**
- Consumes: scenarios from Task 2 and the existing five-panel report.
- Produces: `bash scripts/hackathon/judge_one_command.sh` as the screener path; it exits nonzero on stale scenarios, policy disagreement, failed tests, or a missing prompt-injection case.

- [ ] **Step 1: Add a failing one-command content test**

Add to `x3-lang/tests/test_hackathon_support.py`:

```python
def test_one_command_runs_prompt_injection_and_parity():
    source = (ROOT / "scripts/hackathon/judge_one_command.sh").read_text()
    assert "demo.py" in source
    assert "build_scenarios.py --check" in source
    assert "node --test x3-lang/near/demo/test/*.test.mjs" in source
```

- [ ] **Step 2: Run the test to verify it fails, then implement the command**

Run:

```bash
PYTHONPATH=x3-lang .venv/bin/python -m pytest x3-lang/tests/test_hackathon_support.py::test_one_command_runs_prompt_injection_and_parity -q
```

Expected: FAIL until `judge_one_command.sh` exposes the prompt-injection demo, scenario freshness, Python tests, and browser tests.

- [ ] **Step 3: Implement the one-command gate**

`judge_one_command.sh` must run, in order:

```bash
python3 x3-lang/near/demo/judge/build_scenarios.py --check
python3 scripts/hackathon/demo.py
node --test x3-lang/near/demo/test/judge.test.mjs
python3 scripts/hackathon/judge_evidence.py --mode quick
```

Use `${PYTHON:-python3}` and `${NODE:-node}` so fresh environments can override binaries without editing the script.

- [ ] **Step 4: Rewrite the README first screen**

Put these headings above architecture detail:

```text
WORKS TODAY
DELIBERATELY BLOCKED
60-SECOND JUDGE PATH
ON-CHAIN EVIDENCE
SECURITY MODEL
```

State the prompt-injection scenario, exact one-command path, current evidence level, and explicit absence of settlement/causality claims. Link the testnet evidence section only after Task 4 creates it.

- [ ] **Step 5: Add the three-minute demo script**

`docs/hackathon/DEMO_VIDEO_SCRIPT.md` must define these shots in order: owner mandate, malicious injected request, local refusal, Guard signed-quote refusal, testnet transaction, evidence-limit slide.

- [ ] **Step 6: Run the complete local judge path**

Run:

```bash
bash scripts/hackathon/judge_one_command.sh
python3 scripts/hackathon/demo.py --json | jq -e '.scenarios[] | select(.id == "prompt-injection-recipient") | .denied_guards == ["mandate_recipient"]'
```

Expected: both commands exit `0`.

- [ ] **Step 7: Commit**

```bash
git add README.md docs/hackathon/DEMO_VIDEO_SCRIPT.md scripts/hackathon x3-lang/near/demo x3-lang/tests/test_hackathon_support.py
git commit -m "docs(demo): make the grand-prize proof runnable"
```

---

### Task 4: Deploy the reviewed Guard and collect testnet refusal evidence

**Files:**
- Modify: `README.md`
- Create: `scripts/hackathon/testnet_guard_evidence.py`
- Create: `scripts/hackathon/test_testnet_guard_evidence.py`
- Modify: `scripts/judge_evidence.py`
- Modify: `.github/workflows/x3-shield-judge-readonly-quick.yml`
- Create: `docs/hackathon/GUARD_TESTNET_EVIDENCE.md`
- Create after execution: `evidence/near-guard-testnet/manifest.json`
- Create after execution: `evidence/near-guard-testnet/transactions.json`

**Interfaces:**
- Consumes: `near` CLI 0.31.1, the release Guard Wasm, and a dedicated account/key outside the repository.
- Produces: `record_manifest(account_id: str, wasm: Path, rpc_url: str) -> dict`, `record_transaction(tx_hash: str, rpc_url: str) -> dict`, and `verify_evidence(path: Path, rpc_url: str) -> dict`; malformed or mismatched evidence raises `EvidenceError`.

- [ ] **Step 1: Write failing manifest and tamper tests**

In `scripts/hackathon/test_testnet_guard_evidence.py`, use injected fake RPC responses to assert:

```python
def test_manifest_rejects_wrong_chain_id(fake_rpc):
    with pytest.raises(EvidenceError, match="chain"):
        verify_evidence(fixture("wrong-chain.json"), rpc_url=fake_rpc.url)

def test_transaction_rejects_missing_final_execution_status(fake_rpc):
    with pytest.raises(EvidenceError, match="final"):
        record_transaction("missing-final", rpc_url=fake_rpc.url)
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
PYTHONPATH=x3-lang .venv/bin/python -m pytest scripts/hackathon/test_testnet_guard_evidence.py -q
```

Expected: FAIL because the module does not exist.

- [ ] **Step 3: Implement the evidence collector**

The collector must:

- accept only HTTPS RPC URLs for non-local execution;
- query `EXPERIMENTAL_chain` and require chain id `testnet`;
- canonicalize JSON before hashing;
- record contract account, code hash, Wasm SHA-256, size, block height, transaction hash, receipt status, method names, and sanitized arguments;
- reject transactions not included in a final block;
- reject evidence containing private-key fields or environment data;
- expose a `--verify` mode that fails on any RPC mismatch.

- [ ] **Step 4: Build the exact Guard twice and compare**

Run:

```bash
(cd x3-lang/near/x3-guard && cargo build --release --locked --target wasm32-unknown-unknown)
sha256sum x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm
```

Build twice from a clean target directory and require equal SHA-256 values. Record both digests and byte size.

- [ ] **Step 5: Create and fund the dedicated testnet account**

Run only after backup and with output redacted:

```bash
npx --yes near-cli-rs@0.31.1 account create-account sponsor-by-faucet-service <dedicated-account>.testnet autogenerate-new-keypair
npx --yes near-cli-rs@0.31.1 account view-account-summary <dedicated-account>.testnet network-config testnet now
```

Expected: account exists, chain is testnet, balance exceeds the deployment plus transaction reserve, and the key is outside the repository with mode `0600`.

- [ ] **Step 6: Deploy and record code identity**

Run the reviewed near-cli deployment command with `new` initialization and `0 NEAR` attached. Then query the account and require its final `code_hash` to match the locally recorded Wasm digest according to NEAR's documented code-hash semantics.

- [ ] **Step 7: Execute allowed, prohibited, invalid-signature, and revoked paths**

Use the exact sanitized calls produced by `near_intents.py mandate args`,
`guard-args`, and `mandate revoke`. The prohibited path must use a valid
signature from the contract's pinned **test-only** manager key unless official
testnet signer provenance has been independently established, and it must
violate the registered policy at `quote_request_echo` or `mandate_recipient`.
Label this as test-signer evidence, not as an official 1Click production
signature. Record separate transaction hashes and receipt errors.

- [ ] **Step 8: Independently verify every record**

Run:

```bash
PYTHONPATH=x3-lang .venv/bin/python scripts/hackathon/testnet_guard_evidence.py --verify evidence/near-guard-testnet/manifest.json
PYTHONPATH=x3-lang .venv/bin/python scripts/hackathon/testnet_guard_evidence.py --verify evidence/near-guard-testnet/transactions.json
```

Expected: both exit `0`; changing any hash, chain id, account, block height, or status causes failure.

- [ ] **Step 9: Commit only sanitized evidence**

```bash
git diff --check
git add scripts/hackathon/testnet_guard_evidence.py scripts/hackathon/test_testnet_guard_evidence.py scripts/judge_evidence.py .github/workflows/x3-shield-judge-readonly-quick.yml docs/hackathon/GUARD_TESTNET_EVIDENCE.md evidence/near-guard-testnet
git commit -m "feat(evidence): record authenticated testnet guard refusal"
```

---

### Task 5: Exact-head review, public branch, and submission package

**Files:**
- Modify: `README.md`
- Modify: `docs/hackathon/JUDGE_GUIDE.md`
- Modify: `docs/hackathon/SUBMISSION_CHECKLIST.md`
- Modify: `docs/hackathon/DEMO_VIDEO_SCRIPT.md`
- Create: `submission/x-post-draft.md`
- Create: `submission/submission-copy.md`
- Test: all repository gates and GitHub Actions

**Interfaces:**
- Consumes: the exact release SHA and Task 4 evidence.
- Produces: a public release branch, peer mirror branch, draft PR, exact-head CI result, demo post draft, and submission description that only names proven evidence.

- [ ] **Step 1: Run the full local release matrix**

Run:

```bash
bash scripts/hackathon/judge_one_command.sh
python3 scripts/judge_evidence.py --mode full
PYTHONPATH=x3-lang .venv/bin/python -m pytest x3-lang/tests -q
node --test x3-lang/near/demo/test/*.test.mjs
(cd x3-lang && cargo test -p x3-lang-vm --locked --quiet)
(cd x3-lang && cargo fmt --all -- --check)
(cd x3-lang && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
(cd x3-lang/near/x3-guard && cargo test --locked --quiet)
(cd x3-lang/near/x3-guard && cargo check --target wasm32-unknown-unknown --locked)
bash scripts/check-near-allocator.sh
```

Expected: all commands exit `0`.

- [ ] **Step 2: Run a fresh-clone screener simulation**

Use a temporary clone of the final commit with no inherited environment or credentials:

```bash
git clone --no-local . /tmp/x3lang-fresh-clone
cd /tmp/x3lang-fresh-clone
python3 -m venv .venv
.venv/bin/python -m pip install -r x3-lang/requirements.txt pytest
bash scripts/hackathon/judge_one_command.sh
```

Expected: PASS offline; no private environment is required.

- [ ] **Step 3: Push the release branch and matching peer mirror**

```bash
git push -u origin release/x3lang-submission-20261106
```

Push only the exact files required by the mirror policy to the same-named `xxxstar` branch. Confirm exact equality with `scripts/proof/check_x3lang_mirror.py`.

- [ ] **Step 4: Open a draft PR**

Create one draft PR titled:

```text
feat(x3lang): prompt-injection mandate refusal with NEAR testnet proof
```

The PR body must link the testnet transactions, exact Wasm/code hashes, evidence manifest, exact test counts, known limitations, and rollback path.

- [ ] **Step 5: Obtain exact-head CI and independent review**

Wait for every applicable GitHub Actions workflow on the final head. Prompt `x3review` to review the exact head and evidence without editing. Do not claim green until every required run is `success`.

- [ ] **Step 6: Audit the submission copy**

Run `shipframe:evidence-audit` against `submission/submission-copy.md` and the final README. Replace every unproven claim with its exact evidence level. The draft X post must say it was built at NEAR Hackathon and tag `@NEARProtocol` and `#NEARHackathon`.

- [ ] **Step 7: Commit final submission assets**

```bash
git add README.md docs/hackathon submission
git commit -m "docs(submission): finalize evidence-backed grand-prize package"
git push
```

Expected: the pushed head is the exact head with green CI and completed review; no merge or final submission is performed without owner approval.

