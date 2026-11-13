# X3 Force Field — three-minute judge walkthrough

**Five observable stages:** written `.x3` language → compiler refusal → deterministic NEAR Guard enforcement → real mainnet payment → recorded outcome.

This is a recording plan, **not** evidence of a published video or a new transaction. X3 Shield was the historical name; contract identifiers, source files and transactions retain their original identities. Use the [one-page invoice_06 proof card](../../PROOF_CARD.md) for all mainnet claims and limitations.

## Before recording

From a clean checkout of the exact commit you intend to submit, show `git rev-parse HEAD`. Install the documented Python dependencies in a local environment, never reveal wallet keys or environment files. Every terminal run below is **LOCAL** and uses no wallet. Do not imply that the same payment traversed the local compiler demonstration and an on-chain X3VM—the mainnet sources are hash-matched to Guard state, but Guard does not execute the native VM.

| Time | What judges see | Evidence level | Narration |
| --- | --- | --- | --- |
| 00:00–00:20 | Title **X3 Force Field**, tagline and a human-controlled spending mandate | EXPLANATION | “An AI agent can propose a payment. Only the owner's written rules can authorize its terms.” |
| 00:20–00:45 | [Original `mandate-proposed.x3`](../../evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/mandate-proposed.x3) beside [`payment-proposed.x3`](../../evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/payment-proposed.x3). Highlight recipient, asset, limits, refund, approvals, slippage and nonce. | MAINNET SOURCE MATCH (not on-chain compilation) | “These are typed, reviewable source programs—not editable prompt instructions. Their exact SHA-256 hashes match the mandate v4 and intent stored by NEAR Guard.” |
| 00:45–01:10 | Run the compiler's nine unsafe-source mutations; show a missing bound and lookalike-recipient rejection. Optionally show three local Grok-pattern replay refusals. | LOCAL, OFF-CHAIN | “The checker refuses invalid source before quote submission. Model compliance does not override deterministic restrictions. These are local tests, not on-chain transactions.” |
| 01:10–01:40 | Show the [owner approval](https://nearblocks.io/txns/GFdQLrzT5tnPPdZwKLtmwpNvSwrxVHDvVg2C3WX5fsK6), then the [restricted-agent quote acceptance](https://nearblocks.io/txns/Eg75uehwU4YJ86ZZPEPKiBqnjkr6PxwPmV23HFAgu153): 21 signed-quote checks passed, authenticated minimum 24.770243 USDC versus policy floor 24.75. | NEAR MAINNET GUARD | “The deployed Guard on `x3lang.near` verified the signed 1Click terms before authorizing a deposit address. Acceptance alone is not settlement.” |
| 01:40–02:10 | [25 USDC Solana deposit](https://solscan.io/tx/4zhGYyWNabYQ5gixTUzoVHVbgFKWAsQssSpCv3B1J1LjCRisEJW6wm3zuS2GVuDotAT93S8wm1qezHwsKX2RkXo5), then [24.9952 USDC NEAR payout](https://nearblocks.io/txns/8jfbACLSxrzi3fnajfAVGc4Fgo6XLCWA1mpiJWXPw43P), about 84 seconds later. | TWO FINALIZED MAINNET LEGS | “The owner manually funded from Solflare. 1Click status associates the legs; it is not a cryptographic atomic cross-chain proof.” |
| 02:10–02:30 | [Restricted-agent `agent_record_outcome`](https://nearblocks.io/txns/9sTn9pJ1ZbCVazBm9Wz9oEDh6aWw4FUVMoAJEKCCzuqE) and [finalized state](../../evidence/near-guard-mainnet-invoice06-2026-11-12/outcome-recording/README.md). Show `Settled`, `24995200` and `floor_met: true`. | NEAR MAINNET OUTCOME | “The agent recorded its settlement report. Guard stored the hashes and evaluated the floor, but did not verify those hashes against the chains.” |
| 02:30–02:48 | Seven limits on the [proof card](../../PROOF_CARD.md#seven-limits). | TRUST BOUNDARIES | “One owner-assisted ~$25 run, noncustodial Guard, owner's FullAccess recovery key on agent account, unsigned provider association, no atomicity and no native X3VM on-chain.” |
| 02:48–03:00 | Public repository, `git rev-parse HEAD`, [README reproduction steps](../../README.md#try-it-do-x-see-y), and proof-card link | REPRODUCIBILITY | “Inspect the exact source, independently rerun compiler refusals and follow every transaction.” |

## Commands for the LOCAL panel

Run from the public repository root after setting up the documented environment:

```bash
git rev-parse HEAD
sha256sum evidence/near-guard-mainnet-invoice06-2026-11-12/source-recovery/{mandate-proposed,payment-proposed}.x3
python3 scripts/hackathon/x3_wont_compile.py
python3 scripts/hackathon/replay_grok_wallet_drain.py
```

Expected results as documented in the [README](../../README.md#3-unsafe-payments-do-not-compile): **9/9 unsafe edits refused** and **3/3 local attack programs refused**, with the authorized control allowed. Check actual terminal output while recording and do not paste a fabricated pass result.

## Five evidence boundaries — display them explicitly

1. **`.x3` source + exact SHA-256:** reproduces source identity; Guard stores hashes, but does not compile source on-chain.
2. **Compiler/type checks:** offline program rejection; this is not proof of on-chain refusal.
3. **Signed quote + policy:** actual Guard acceptance and 21 `ok` checks; acceptance is not a transfer.
4. **Deposit and payout:** two finalized on-chain transfers; funding wallet was manually operated by the owner, not Guard-controlled. 1Click associates the legs without atomicity.
5. **Recorded outcome:** agent-reported `Settled`, amount and transaction hashes persisted in Guard; Guard does not authenticate the supplied hashes.

## Publish only after verification

Show **LOCAL** vs **NEAR MAINNET** vs **SOLANA MAINNET** labels on screen. Do not call this a repeatable or autonomous production trading system. Never expose signing material, `.env` contents, private logs or credentials. Confirm that the actual published video and X post work while signed out; add those public URLs to [submission copy](../../submission/submission-copy.md) only after they exist. This document itself is not a video, post or submission receipt.
