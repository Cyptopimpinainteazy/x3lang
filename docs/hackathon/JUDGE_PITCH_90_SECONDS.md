# X3 Force Field — the 90-second judge pitch

> **An AI agent should be able to propose a payment without being able to change the rules that authorize it.**

## Read this aloud (approximately 90 seconds)

"AI agents can already hold wallets and request payments. But when an agent is prompt-injected, its model output must never become unrestricted spending authority.

We built **X3 Force Field**: an owner-controlled, typed `.x3` spending policy enforced independently of the AI. The owner specifies who can be paid, what assets are allowed, maximum amounts, daily limits, refund destinations, slippage, expiry, and approvals. The compiler rejects unsafe payment programs. On NEAR, our deployed Guard checks an official 1Click signed quote against the stored mandate *before* it authorizes a deposit address.

We tested the hostile-agent case locally: our Grok-inspired replay makes the simulated model follow malicious instructions, yet **all three unauthorized payment programs are refused**, while the legitimate payment passes.

And this is not just an offline prototype. Our archived mainnet demonstration shows a restricted agent FunctionCall key accepting a quote after **21 passing checks**. The owner then funded **25 USDC on Solana**; **24.9952 USDC arrived on NEAR**. We provide the source hashes, five transaction records, a reproducible verifier, and the limits of what each step proves.

The limit is important: the owner signed the deposit; Guard does not hold those tokens, prevent an unrestricted funding key from bypassing it, or prove atomic cross-chain settlement. We are not claiming otherwise.

**The advance is verifiable delegation: a model can ask, but deterministic policy must authorize.**"

## Judge: see it, don't just hear it

| What to inspect | Direct evidence | What it establishes |
| --- | --- | --- |
| **1. A malicious model request is refused; a valid control passes** | Run `python3 scripts/hackathon/replay_grok_wallet_drain.py` from the configured project environment ([source](../../scripts/hackathon/replay_grok_wallet_drain.py)) | Three **local**, scripted unauthorized payment refusals; not mainnet hostile-model execution |
| **2. One real bounded payment reaches the destination** | [Mainnet proof card](../../PROOF_CARD.md): owner approval → agent quote acceptance → Solana deposit → NEAR payout → agent-recorded outcome | Archived mainnet Guard authorization and on-chain transfer receipts; owner manually funded |
| **3. The security boundary is honest** | [Seven limits](../../PROOF_CARD.md#seven-limits) and [incident matrix](INCIDENT_GUARD_TRUST_BOUNDARIES.md) | Guard constrains Guard-mediated proposals; external custody, owner keys, false outcome hashes and bridge finality remain independent risks |

## 30-second opening for a live judge Q&A

"Let me show you a useful distinction. The AI can be fooled. The authorization layer should not be. In our offline replay, the model follows malicious instructions, but out-of-policy payment programs are rejected and an authorized control is accepted. On NEAR mainnet, we separately show a restricted agent authorizing a real signed 1Click quote under the owner's spending rules, followed by an owner-funded Solana-to-NEAR payment. The transaction links and replay code are public. We also document exactly where Guard's protection stops."

## Answer the hard questions before they are asked

**Did the agent autonomously spend funds?** No. The agent's restricted FunctionCall key accepted a quote on Guard. The owner signed the separate Solana deposit.

**Does Guard prevent the owner or an unrestricted wallet key from stealing/spending funds?** No. The demonstrated Guard is noncustodial and the owner FullAccess keys remain trusted.

**Did you prove atomic cross-chain settlement?** No. The deposit and payout are observed on their respective chains; their association comes from the provider's 1Click status, not a cryptographic atomicity proof.

**Are the three malicious attack refusals on mainnet?** No. They are deterministic **local** replays; historical testnet refusals and mainnet allowed acceptance are distinct evidence.

**Is this a new language built entirely at the hackathon?** No. The compiler/VM predate the event, and the NEAR integrations and Guard evidence are the hackathon contribution. See [prior work](../../PRIOR_WORK.md).

**Does the Mt. Gox-inspired test mean Guard protects Bitcoin exchanges?** No. It is an [offline synthetic accounting reference model](MTGOX_INSPIRED_ACCOUNTING_REPLAY.md), not deployed Guard code.

## Track fit (claims bounded to observed proof)

- **AI Security & Defense — primary:** compiler checks and Guard-mediated limits constrain malicious payment proposals. A compromised owner/funding key remains outside this protection.
- **Agent Economy:** a restricted agent key authorizes a payment under an owner mandate. No autonomous agent custody or agent-driven funding is shown.
- **Unstoppable DeFi:** official 1Click signed-quote authorization precedes a real mainnet Solana-to-NEAR payment. X3 is not an atomic bridge or DEX.
- **Autonomous Business:** illustrates owner-set spending policies for possible business agents. The sole mainnet payment was owner-funded to the owner's own account; no operating treasury or revenue system has been demonstrated.

## Public links to provide judges

- [Start here: JUDGES.md](../../JUDGES.md)
- [One-page mainnet proof card](../../PROOF_CARD.md)
- [Incident-to-guard matrix](INCIDENT_GUARD_TRUST_BOUNDARIES.md)
- [Exact reproduction instructions](../../JUDGES.md#reproduce-the-same-payments-checks--offline-after-setup)

**Submission note:** A prepared video, a PR, or a README link is **not** proof of a public native X demo post or organizer receipt. Confirm the actual posted URL and official submission separately.
