# X3 Shield on Base — demo storyboard (private working draft)

> **Planned demonstration, not a recording and not a claim of shipped features.** The script must be rewritten to match actual outputs, real transaction identifiers and the final test gate. Never splice in NEAR footage and label it Base stock execution. Target 2:30–3:00.

## What the judges should understand in 10 seconds

**An agent proposes a financial action. X3 Shield independently checks the owner's typed mandate and the exact proposed Base stock-linked instrument; forged instructions and replayed proposals fail closed.**

This is **not a stock brokerage**. The competition feature is a risk-evaluation and authorization-evidence demonstration, not U.S. stock trading. Any live asset trade, lending, derivatives, wallet-signing or token custody must stay disabled unless separately lawful, authorized and independently verified.

## Frames and narration — contingent on actually building the stock-specific feature

| Time | Visual to capture from genuine running software | Suggested narration and evidence label |
| --- | --- | --- |
| 00:00–00:15 | Title and visible Base Sepolia network ID; source commit SHA on screen | “This is a **Base-specific tokenized-stock intent review**, protected by X3 Shield. Demo uses testnet or read-only intent data. No regulated stock trading is offered.” |
| 00:15–00:40 | Owner's exact `.x3` mandate; asset registry identifies actual Base network, contract address, decimals, issuer/rights metadata and data freshness | “The owner, not the AI, sets the instrument whitelist, amount, recipient, chain ID, slippage, expiry and one-time nonce.” |
| 00:40–01:05 | Show an agent-proposed stock-linked intent with hash; deterministic verifier produces `ALLOW_FOR_REVIEW` (not `TRADE_EXECUTED`) | “A valid intent passes explicit bounds; this is policy approval only, not proof of on-chain settlement or ownership rights.” |
| 01:05–01:40 | Replace recipient, change asset address/chain ID, replay nonce, change signed payload, exceed cap and show separate `DENY` codes | “The same risk controls refuse wrong recipients, instrument substitutions, stale or revoked mandates, and replay—even if an agent asks confidently.” |
| 01:40–02:05 | A genuine Base testnet contract read or anchored evidence, if built; show explorer URL and head/finality notation | “This is the Base portion we actually verified. A remote RPC response alone is not independent finality.” |
| 02:05–02:25 | Exact test command with real green output and negative tests; evidence JSON scoped to actual feature | “These are reproducible tests on commit [ACTUAL SHA], with both allowed and rejected paths.” |
| 02:25–02:45 | Claim boundary showing pre-existing X3 components vs new competition code | “The `.x3` language and prior Base–NEAR transfer existed earlier. The stock-specific registry and guarded agent flow shown here were built for this season.” |
| 02:45–03:00 | X @x3starchain, Base name, honest release status, optional demo URL | “X3 Shield: agents can propose; verified policy decides.” |

## Production instructions

- Capture screen from real terminal/browser sessions. Do not fabricate logs, event IDs, asset balances, approvals or contracts.
- Use code paths that exist on the reviewed commit. A storyboard frame is a **TODO** until a recording proves it.
- Avoid displaying tokens, secrets, RPC credentials, private wallet data or personal identifiers.
- Show labels: `LOCAL_POLICY`, `BASE_TESTNET_READ`, `BASE_TESTNET_TX`, `FINALIZED` only when warranted; otherwise use `UNVERIFIED`.
- Preserve the existing testnet Base Sepolia → NEAR Tidepool example as optional *historical context*, clearly marked **trusted relayer and non-stock asset**. Do not imply the new feature actually trades stocks.
- Include a short exact-head test log capture; avoid marketing animations replacing technical proof.
- Do not show a “successful swap” unless it really occurred and the constraints are actually enforced.
- Verify any real tokenized-stock project restrictions in the U.S. before displaying trading UX or connecting a wallet.

## Public communication — do not execute yet

**Draft X post (not to publish until new feature and video are real):**

> X3 Shield on Base: an autonomous agent can propose a stock-linked action, but it cannot bypass its owner's `.x3` mandate. Our new Base prototype shows exact asset/chain identity checks, cap controls, recipient protection, nonce replay denial and independently reviewable evidence. Demo: [VIDEO LINK]. Testnet / read-only policy evaluation; not a live stock brokerage. @buildonbase #Base

Owner should verify track-specific tags on the live page on publication day. The official entry requires a public X demo post and manual submission form; no submissions have been sent.

## Evidence before filming

- [ ] Executable stock-specific path works at exact repository head, independently built from clean checkout.
- [ ] Genuine Base token/instrument metadata sources and contract IDs displayed and reviewed; no fictitious stock symbols or unsupported asset rights.
- [ ] Allowed and at least five denied scenarios run and remain deny-by-default.
- [ ] No trading enabled for U.S. users.
- [ ] Measured results, CI tests, mutation/adversarial coverage attached with proper scope.
- [ ] No sensitive keys, passwords, private data or unpublished repo contents accidentally shown before owner-approved release.
- [ ] Owner signs off on final script, public X video and posted link.
