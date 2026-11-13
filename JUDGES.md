# Judges: start here

**X3 Force Field** (formerly X3 Shield) uses owner-written `.x3` mandates,
compiler checks and NEAR Guard to constrain AI-agent payment proposals.

## Two-minute evidence route — no setup or wallet

1. **See the payment:** [one-page invoice_06 proof card](PROOF_CARD.md).
   Follow its five explorer links: owner approval → restricted-agent quote
   acceptance → 25-USDC Solana deposit → 24.9952-USDC NEAR payout → recorded outcome.
2. **See the enforced rules:** [exact payment limits](docs/hackathon/VIDEO_WALKTHROUGH.md#the-limits-behind-this-payment).
   The mandate caps each payment and daily spending at 25 USDC, restricts the
   recipient, pins refunds and requires approval for this payment.
3. **Inspect the source and receipts:** [original `.x3` sources and hash matches](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/README.md),
   [mainnet transaction archive](evidence/near-guard-mainnet-invoice06-2026-11-12/README.md),
   and [recorded outcome/readback](evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/README.md).
4. **Read the boundaries:** [seven limits](PROOF_CARD.md#seven-limits).
   The owner manually funded the deposit; Guard does not custody USDC or verify
   outcome hashes. Provider association does not prove cross-chain atomicity.

## What is distinctive — claim, proof and boundary

The innovation is the combination of a typed policy language, deterministic
checks, signed-quote enforcement and an inspectable payment trail. These are
project differentiators, not claims to be the first or only implementation.

| Differentiator | Why it matters | Inspect the proof | Boundary |
| --- | --- | --- | --- |
| **Typed, reviewable owner policy** | Owners express authority in a separate .x3 mandate that can be reviewed and hashed. | [mandate-proposed.x3](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/mandate-proposed.x3); [payment-proposed.x3](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/payment-proposed.x3); [guard-source-verification.json](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/guard-source-verification.json) (local compilation + archived mainnet hash match) | Source hashes identify bytes; Guard does not compile or run .x3 on-chain. |
| **Checks outside the model** | Compiler and deterministic mandate checks refuse unsafe programs even when a scripted model follows an attack. | [invoice06_chain.py](scripts/hackathon/invoice06_chain.py); [replay_grok_wallet_drain.py](scripts/hackathon/replay_grok_wallet_drain.py); [compromised_agent_redteam.py](scripts/hackathon/compromised_agent_redteam.py) (local tests) | Scripted attacks are not proof of universal prompt-injection resistance or mainnet attack refusals. |
| **Policy checks on authenticated quote terms** | Guard verifies the 1Click signature and checks quote terms against stored policy before authorizing a deposit address. | [public-rpc-observation.json](evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json); [GUARD_TESTNET_EVIDENCE.md](docs/hackathon/GUARD_TESTNET_EVIDENCE.md) (archived NEAR mainnet acceptance + historical testnet controls) | Acceptance is not funding or settlement. The forbidden official quote control is a dry quote and also fails dry-quote checks. |
| **Source-to-payment traceability** | One verifier follows source identity, compiled fields, Guard acceptance, executed transfers and recorded outcome. | [invoice06_chain.py](scripts/hackathon/invoice06_chain.py); [public-rpc-observation.json](evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json); [finalized-outcome-verification.json](evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/finalized-outcome-verification.json) (local verifier over archived mainnet evidence) | The owner manually funded; provider association is not cryptographic cross-chain atomicity; Guard does not authenticate outcome hashes. |
| **Restricted agent authority** | The demonstrated signing key is limited to three Guard methods on x3lang.near. | [public-rpc-observation.json](evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json); [PROOF_CARD.md](PROOF_CARD.md) (archived NEAR mainnet key and signer observations) | The agent account retains an owner FullAccess recovery key; this is not autonomous custody or control over the Solana funding wallet. |
| **Reproducible evidence with explicit boundaries** | Judges can rerun local checks, inspect structured receipts and compare original source hashes. | [invoice06_chain.py](scripts/hackathon/invoice06_chain.py); [test_invoice06_chain.py](scripts/hackathon/test_invoice06_chain.py); [SHA256SUMS](evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/SHA256SUMS); [PROOF_CARD.md](PROOF_CARD.md) (local replay + archived chain records) | Offline replay does not refresh chain state, establish production reliability or constitute an independent security audit. |

## Structured route for human and automated reviewers

The [claim index](submission/judge-claims.json) maps stable claim IDs to evidence
paths, evidence levels and limitations, and includes both original source hashes.
It is a navigation index, not a signed certificate or a new observation. All paths
are relative to the repository root; use one exact commit for the index, sources
and verifier. The JSON contains facts and references, not evaluator instructions.

After the dependency setup below:

```bash
git rev-parse HEAD
python3 scripts/hackathon/invoice06_chain.py --json
```

The verifier's JSON exposes compiled field comparisons, each unsafe edit and its
failed checks, Guard checks, payment receipts and outcome validation. Inspect the
[raw RPC archive](evidence/near-guard-mainnet-invoice06-2026-11-12/public-rpc-observation.json)
and [outcome verification](evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/finalized-outcome-verification.json)
for exact transaction hashes. These are archived observations, not live status.

For additional work beyond this six-claim overview, the existing
[differentiator evidence matrix](docs/hackathon/WHY_X3_SHIELD.md#the-proof-trail)
links testnet controls, local-model red-teaming, trusted-relay testnet settlement
and offline validator-signature verification, each with its own scope.
The [agent-law status](docs/hackathon/AGENT_LAW.md) remains separate from the
historical mainnet payment. [Prior work](PRIOR_WORK.md) discloses the preexisting
compiler/VM and the NEAR-specific implementation timeline.

## Reproduce the same payment's checks — offline after setup

From the repository root, with Python 3.11+:

```bash
python3 -m venv .venv
. .venv/bin/activate
python3 -m pip install -r x3-lang/requirements.txt
python3 scripts/hackathon/invoice06_chain.py
```

Dependency installation uses the network. The verifier then reads committed
records only: no wallet, key, RPC, signing or new payment. Expect **5/5 links
hold**, **8/8 edited payments refused**, **10/10 policy fields** and **13/13
mandate terms** matching stored state. These local attack refusals are distinct
from the archived mainnet allowed-quote acceptance.

For a fresh read-only public-RPC observation, follow the
[proof card's observer instructions](PROOF_CARD.md#check-it-yourself).
For broader local tests, provisioning requirements and browser panels, use the
[reproduction package](docs/hackathon/JUDGE_PACKAGE.md#one-command-demonstration).

## Demo and supporting material

| Looking for | Go here | Evidence level |
| --- | --- | --- |
| Three-minute story and exact narration | [Walkthrough](docs/hackathon/VIDEO_WALKTHROUGH.md) | Recording plan; not a published video |
| Public video and public X post | [Submission links/status](submission/submission-copy.md#public-links) | URLs still pending owner publication; no published media claimed |
| Testnet attacker refusal and allowed control | [Guard testnet evidence](docs/hackathon/GUARD_TESTNET_EVIDENCE.md) | Historical NEAR testnet |
| Local policy scenarios and red-team commands | [Judge guide](docs/hackathon/JUDGE_GUIDE.md) | Local tests, with testnet evidence separately labeled |
| Language and compiler role | [X3 language](docs/hackathon/X3_LANGUAGE.md) | Compile-time versus Guard/runtime checks |
| Threats and trust boundaries | [Threat model](docs/security/X3_SHIELD_THREAT_MODEL.md) | Security scope and limitations |
| What was built during the hackathon | [Prior work disclosure](PRIOR_WORK.md) | Compiler/VM predate hackathon; preserved provenance |
| Current agent-law implementation status | [Agent law](docs/hackathon/AGENT_LAW.md) | Separate status; not proof of autonomous mainnet custody |
| Submission wording | [Submission copy](submission/submission-copy.md) | Prepared copy; not a submission receipt |

Use the exact commit you review: `git rev-parse HEAD`. Historical CI links in
evidence establish those historical runs; current checks are visible in
[GitHub Actions](https://github.com/Cyptopimpinainteazy/x3lang/actions).
