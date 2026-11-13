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
