# X3 Force Field — Judge Evidence Index (post-submission addendum)

> **Evidence status:** This document is a post-submission navigation aid. It is **not** part of the frozen `hackathon-submission` tag, does not alter the original organizer submission, and must not be described as evidence submitted before the deadline. It does not itself constitute an audit or fresh chain consensus check.

## Immutable reference points

| Record | Pinned revision | Status | What it demonstrates |
| --- | --- | --- | --- |
| [Frozen hackathon submission](https://github.com/Cyptopimpinainteazy/x3lang/tree/73f3f17dfc3529cda59bb7077a31dac6a536833a) | `73f3f17dfc3529cda59bb7077a31dac6a536833a` | Frozen submission snapshot | Original NEAR Guard + invoice_06 evidence, signed quote policy checks, source-recovery and tests. |
| [invoice_06 archive](https://github.com/Cyptopimpinainteazy/x3lang/tree/73f3f17dfc3529cda59bb7077a31dac6a536833a/evidence/near-guard-mainnet-invoice06-2026-11-12) | `73f3f17` | Archived mainnet records; offline verification | Owner approval, restricted agent authorization, 25 USDC Solana deposit, 24.9952 USDC payout on NEAR, recorded outcome. The owner signed the Solana deposit. |
| [Source-vault pilot archive](https://github.com/Cyptopimpinainteazy/x3lang/tree/9ec3b48000bcada0ceb20df85d0d7e5eeed744bb/evidence/solana-vault-mainnet-2026-11-14) | `9ec3b48000bcada0ceb20df85d0d7e5eeed744bb` | **Separately published later evidence, not included in frozen tag** | Owner approved an exact quote; agent signed the execution packet for a 5-USDC payment from PDA-controlled Solana funds; archived payout is 4.999746 USDC on NEAR. |
| [Public judge overview](https://dotx3.net/judge) / [proof](https://dotx3.net/proof) | **Deployed SHA not independently pinned here** | Website; dynamic content | Convenience navigation, **not** authoritative evidence of the frozen tag or immutable release provenance. |

## Two different security boundaries — do not merge claims

**invoice_06 (Guard authorization)** — NEAR `x3-guard` enforced stored mandate checks against a signed 1Click quote prior to the agent workflow accepting it. A restricted NEAR agent key did not change the mandate. The **owner's Solana wallet signed the source deposit**; Guard did not custody or spend those Solana funds. Guard records a caller-supplied outcome; it does **not** authenticate outcome hashes. Archived receipts support the observed token delivery, but do not prove cryptographic cross-chain atomicity.

**November 14 vault pilot (source spending)** — A Solana policy PDA controlled the source tokens. The owner approved exact quote terms and the agent signed the subsequent execution packet. The source packet / instruction / CPI and NEAR destination observations are checked by the [specialized archived verifier](https://github.com/Cyptopimpinainteazy/x3lang/blob/9ec3b48000bcada0ceb20df85d0d7e5eeed744bb/evidence/solana-vault-mainnet-2026-11-14/verify_archived.py). **No new NEAR Guard authorization occurred in that pilot.** The owner retained program upgrade authority, and deployed Solana SBF byte-for-byte build provenance has not been established.

## Reproduce checks without moving funds

- Frozen invoice evidence: `python3 scripts/hackathon/invoice06_chain.py` at `73f3f17`. The archive comparison checks the source/compiled/Guard/payment trail; offline checks are **not** fresh chain state.
- Vault pilot: follow the [pinned archive README](https://github.com/Cyptopimpinainteazy/x3lang/blob/9ec3b48000bcada0ceb20df85d0d7e5eeed744bb/evidence/solana-vault-mainnet-2026-11-14/README.md) and run `verify_archived.py` with its declared dependencies. It verifies archived objects, not deployed-program build identity or consensus quorum.

  The [maintained offline verifier](../../scripts/hackathon/verify_solana_pilot_archive.py) accepts a local copy of that pinned archive. In a project virtual environment with `solders==0.26.0` installed, run `python3 scripts/hackathon/verify_solana_pilot_archive.py --evidence /path/to/solana-vault-mainnet-2026-11-14`. The verifier pins the archive manifest, checks the signed packets and reports the source-quorum and solver-causality limits. Run `python3 scripts/hackathon/check_solana_pilot_archive.py --evidence /path/to/solana-vault-mainnet-2026-11-14` for the tamper tests.
- **Known incompatibility**: generic `near_intents._signed_quote` correctly **rejects** this pilot's receipt: the compiled `leg.deadline` is `2026-11-17T13:31:48.804Z`, whereas the signed 1Click `quoteRequest.deadline` is `2026-11-14T12:01:49.430Z`. The provider's signed quote deposit deadline is separately `2026-11-17T12:01:49.430Z`. Do **not** weaken the generic equality check or report this verifier as passing. The archive's distinct verifier passed its own checks. A future compatible schema must bind the effective authorization deadline *before* owner approval and must not retroactively bless the old mismatched receipt.

## Evidence classifications

- **Archived mainnet**: recorded transaction and RPC observations; not fresh independent consensus attestation.
- **Local tested**: reproducible scripts and mutation tests, not paid mainnet denial transactions.
- **Simulated / Sandbox / testnet**: explicitly labelled as such; never relabel as mainnet adversarial execution.
- **Not demonstrated**: cryptographic cross-chain atomicity, verified deployed SBF build identity, independent custody audit, sustained autonomous business/revenue, and an owner-free autonomous end-to-end swap.

## Publication reconciliation checklist

Before claiming a production website is current, independently verify and record its deployed commit SHA, fix stale PR-status text and pending media placeholders, and correct any statement that Guard validates **every** cross-chain payment. Preserve the frozen tag; publish corrections as a dated addendum and show the two archived workflows separately.

**Change policy:** No signed keys, funding, trade execution, deployment, publication, or alteration of frozen evidence is needed to reproduce this index.
