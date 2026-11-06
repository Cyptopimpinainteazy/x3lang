# Judge start here: X3Lang / X3 Shield

**NEAR HACKATHON 2026 | Track IDs:** `security` (primary), `agents`, `defi`  
**Repository:** https://github.com/Cyptopimpinainteazy/x3lang  
**Submission deadline:** 2026-11-14 23:59 UTC (verify against the [organizer's current rules](https://nearhackathon.com/HACKATHON.md)).

## The 20-second pitch

AI agents can propose payments. They must not get to rewrite their owner's authority. **X3Lang is a financial-intent language and policy toolchain:** an owner writes a bounded `.x3` mandate; an agent supplies an `.x3` payment intent; the checker distinguishes **locally eligible**, **human approval required**, and **denied**. The NEAR Intents adapter and the NEAR Guard contract are the proposed enforcement and routing path. Local eligibility by itself does **not** authorize a wallet transfer.

**What differentiates this from a basic wallet spend limit:** the inspectable program and owner mandate bind *recipient, source/destination asset, per-payment and rolling caps, approval threshold, slippage, fees, expiry, nonce and refund destination* as one financial instruction. The demo traces the exact failed guard and the corresponding NEAR Intents route. These individual features are not unique; the value is their combination in a purpose-built language with testable decision boundaries.

## Do X, see Y (read-only; no keys or funding)

Clone the public repository and enter its root. Use Python **3.11+**, Node.js **22+** and Rust/Cargo for the full evidence run. Use an isolated virtual environment for dependencies:

```bash
python3 -m venv .venv
. .venv/bin/activate
python -m pip install -r x3-lang/requirements.txt pytest
python scripts/hackathon/demo.py
```

**Expected result:** Five labeled panels and three decisions from checked-in X3 programs:

| Case | Expected policy decision | Meaning |
| --- | --- | --- |
| 25 USDC to approved recipient | `preflight_passed` | Locally eligible; **not** sent |
| 40 USDC, above approval threshold | `approval_required` | Owner must approve; **not** approved |
| Payment to `attacker.near` | `denied_by_policy` with `mandate_recipient` | Locally refused; **not** an on-chain rejection |

Machine-readable version, with the same scenarios and explicit proof limitations:

```bash
python scripts/hackathon/demo.py --json
```

**Expected JSON:** `schema` is `x3.hackathon.demo.v1`; `evidence_level` is `local_policy_preview_only`. The object contains mandate/program hashes, individual guards, route terms and `evidence_boundaries`. A passing process exit code does not imply funds moved.

Run the broader local evidence collector and readiness check (requires Node and Rust installed):

```bash
bash scripts/hackathon/verify.sh
```

**Expected behavior:** exercises the Python/browser/Guard evidence runs, prints the demo, and writes `/tmp/x3lang-hackathon-readiness.json`. It must fail or report blockers when required tests/evidence are missing. **Do not equate a local pass with live settlement, green hosted CI or an authorized deployment.** Inspect the actual exit status, evidence timestamp, HEAD, and current GitHub Actions run.

See the browser scenario explorer (localhost is needed for browser crypto APIs):

```bash
python3 -m http.server 8000 --directory x3-lang/near/demo
# Open http://localhost:8000/judge.html
```

**Expected result:** nine policy/adversarial scenarios with per-guard parity between Python and browser, including refusal, refund redirection, approval and rolling-spend cases. The browser runs offline checks, not transactions.

## Architecture and exact trust boundary

```text
Owner .x3 mandate --------+
                           +--> parser/local policy checker --> allow / approval / deny
Agent .x3 intent ----------+                    |
                                                v
                                     NEAR Intents route plan
                                                |
                                          NEAR Guard methods
                                                |
                                  [wallet / deposit / settlement]
```

The last line is **not exercised by the offline demonstration**. Guard methods do not custody a wallet or restrict a keyholder who bypasses the Guard. A signed 1Click dry quote proves signed terms, not that a payment occurred. Observed transactions on two chains are not necessarily cryptographic proof that the same solver caused both.

## Evidence ledger (do not upgrade these claims without artifacts)

| Claim | Status in the current main-branch demo | Evidence pointer |
| --- | --- | --- |
| Owner mandate and agent intent as `.x3` source | Inspectable | `x3-lang/examples/near_mandate_agent.x3` and `near_intents_agent_payment.x3` |
| Local pass / approval / refusal with exact guards | Demonstrated by offline commands | `scripts/hackathon/demo.py` and `x3-lang/tests/test_track_control.py` |
| Python/browser parity and NEAR Guard unit tests | Testable locally; CI status must be checked for exact HEAD | `scripts/hackathon/verify.sh` and `x3-lang/near/x3-guard/src/mandate_tests.rs` |
| Real AI model proposes an `.x3` program | Model-backed feature exists; offline demo uses checked-in examples | `x3-lang/`, merged [PR #6](https://github.com/Cyptopimpinainteazy/x3lang/pull/6) |
| Signed-quote refusal at real Guard method | **Draft local-test enhancement**, not live proof | [PR #76](https://github.com/Cyptopimpinainteazy/x3lang/pull/76) |
| Deployed Guard, owner-controlled authorization, live refusal or real swap | **Not established** | [Issue #73](https://github.com/Cyptopimpinainteazy/x3lang/issues/73) |
| Independently authenticated settlement and causal cross-chain proof | **Not established** | [Issue #56](https://github.com/Cyptopimpinainteazy/x3lang/issues/56) |

**The decisive next demonstration:** verify a correct signature on a policy-forbidden quote and show that a *deployed*, authorized Guard rejects it before deposit, with public transaction evidence and a trusted chain checkpoint. The draft local contract test is not a substitute.

## Why NEAR, why these tracks?

- **AI Security & Defense (primary):** a malicious or mistaken agent cannot self-approve a forbidden recipient in the checked-in local policy; the on-chain enforcement boundary is separately testable.
- **Agent Economy:** agents may propose valid payments while owners retain programmable limits and approval thresholds.
- **Unstoppable DeFi:** X3Lang lowers a bounded intent toward a Solana USDC → NEAR USDC route through NEAR Intents; a completed public settlement is *not claimed*.

## Pre-existing work vs hackathon work

The X3Lang language, compiler, bytecode core and trading VM **predate** this event. The relevant hackathon contribution is its NEAR-specific adapter/Guard/agent mandate/verification integration and judge demonstration, not the invention of the historical language. Review [PRIOR_WORK.md](../../PRIOR_WORK.md), the [baseline-to-main diff](https://github.com/Cyptopimpinainteazy/x3lang/compare/baseline...main), and the preserved Git history.

## Files to review next

1. [README](../../README.md) — overview and supported workflows.
2. [Judge Guide](JUDGE_GUIDE.md) — tests, limitations and proof instructions.
3. [Judge Package](JUDGE_PACKAGE.md) — scenarios, threat cases and commands.
4. [Submission Checklist](SUBMISSION_CHECKLIST.md) — exact submission requirements.
5. [Video Walkthrough](VIDEO_WALKTHROUGH.md) — three-minute reproducible presentation.

**Integrity rule:** Never claim an AI judge score, wallet custody, safety audit, independent consensus checkpoint, cross-chain causality, testnet deployment or mainnet execution without public, independently checkable evidence. Honest fail-closed results are a feature of this project, not something to hide.
