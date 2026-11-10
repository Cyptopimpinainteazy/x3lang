# Why X3 Shield stands out

**An agent can propose a payment. The owner's policy determines whether Guard authorizes the proposed terms.**

X3 Shield brings together an owner-readable `.x3` mandate, deterministic policy checks, and a NEAR contract that checks signed quote terms. The fastest way to judge it is to follow one claim through to its evidence.

## The proof trail

| What to look at | Evidence | What it establishes |
| --- | --- | --- |
| An owner sets the rules | [Example mandate](../../x3-lang/examples/near_mandate_agent.x3) and [denied request](../../x3-lang/examples/near_intents_agent_denied.x3) | The checked-in policy describes permitted recipients, assets and limits. |
| A malicious recipient is rejected | [Offline demo](JUDGE_GUIDE.md#1-run-the-offline-policy-demo) | A local fixture targeting `attacker.near` returns `mandate_recipient`. This fixture does not show a live model being attacked. |
| A real local model was red-teamed | [Local model run](../../evidence/agent-redteam-local-2026-11-06/README.md) | Eight cases: the model refused seven; in the eighth it produced the permitted recipient and an effect check identified the attack as neutralized. This was not a Guard refusal after model compromise. |
| Guard has an on-chain allow/refuse control | [Testnet decisions](JUDGE_GUIDE.md#2-inspect-the-testnet-guard-decisions) | One agent intent was refused and a separate permitted intent registered. The archived view gives the reason; the transaction explorer may show a generic trap. |
| A signature is necessary, but policy still matters | [Signed-quote evidence](GUARD_TESTNET_EVIDENCE.md) | An official signed dry quote with a recipient mismatch was refused; a separate matching live quote was accepted and stopped before deposit. The dry quote also fails dry-quote checks. |
| There is a real cross-chain testnet evidence trail | [Tidepool walkthrough](FIRST_SWAP_JUDGE_WALKTHROUGH.md) and [certificate](FIRST_FINALIZED_SWAP_EVIDENCE.md) | Historical records show 1.000000 test USDC sent on Base Sepolia and 0.997400 received on NEAR. Their linkage depends on the trusted Tidepool testnet relay; this is not proof of atomic cross-chain causality. |
| Judges can inspect consensus work | [Offline validator verification](../../README.md#10-verify-near-validator-signatures-offline) | Archived testnet validator approvals can be checked from a pinned checkpoint. This verifies headers, not an independently authenticated Guard receipt or settlement. |

## Try it yourself

From the repository root, with the [project Python dependencies](../../x3-lang/requirements.txt) installed:

```bash
python3 scripts/hackathon/demo.py
```

Find `prompt-injection-recipient`: the expected local result is `denied_by_policy` with `mandate_recipient`. For setup, the full one-command judge gate, and the evidence limitations, use the [judge guide](JUDGE_GUIDE.md).

Replay the archived Tidepool evidence separately:

```bash
PYTHONPATH=x3-lang:scripts/hackathon python3 scripts/hackathon/verify_first_swap_archive.py
```

The offline replay checks archived signed evidence; the trusted-relay boundary remains. Neither command signs transactions or proves a new trade.

## Why this is more than an agent prompt

A valid signature authenticates quote terms; it does not mean the owner intended to pay that recipient. Guard checks signed terms against a stored policy before authorizing a deposit address. NEAR function-call keys and policy-bound wallets provide other valuable controls; X3's focus is the separate, inspectable transaction-policy and signed-quote check. See the [comparison and its scope](JUDGE_GUIDE.md#why-x3-shield-is-different-from-the-near-agent-api).

## Boundaries worth showing judges

- **No wallet custody in Guard.** Someone holding an unrestricted wallet key can transfer outside Guard. Scoped vault work has separate evidence and is not part of the historical Tidepool transfer.
- **No claim of an official live forbidden quote refused by the agent mandate.** The official forbidden example is a dry quote; the permitted live quote stopped before funding.
- **No claim of a live-model compromise in the offline fixture.** The [judge guide](JUDGE_GUIDE.md#public-evidence-matrix) separately identifies local model red-team evidence and its outcome.
- **No claim of mainnet readiness, independent audit, or cryptographic atomic settlement.** The [threat model](../security/X3_SHIELD_THREAT_MODEL.md) describes risks and controls.
- **Prior work is disclosed.** The compiler and VM predate the hackathon; see [provenance](../../PRIOR_WORK.md).

For the complete three-minute walkthrough and submission checklist, go to the [judge guide](JUDGE_GUIDE.md).
