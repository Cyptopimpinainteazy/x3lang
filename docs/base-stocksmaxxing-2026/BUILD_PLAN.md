# Base Stocks Maxxing 2026 — engineering plan and acceptance gates

**Private planning only.** The Base stock-specific path described here is not yet implemented. Keep NEAR competition changes and original repositories isolated; do not merge this branch before review.

## Scope

Deliver the smallest **real** new Base-specific feature that demonstrates a stock-linked agent acting under typed X3 Shield policy, without U.S. stock trading or premature mainnet claims. Reuse X3Lang's compiler/policy semantics and historical Base Sepolia testnet evidence. Do not silently couple to the incomplete x3DEX execution layer.

### Milestone A — scope, instrument provenance and regulatory boundary

- [ ] Verify the latest Base contest eligibility, US-stock-trading exclusion, rules, application disclosures and potential token-history disqualification; owner selects track and partner preferences.
- [ ] Choose an actual Base-supported tokenized-stock instrument and a reliable first-party source for its chain ID, contract, decimals, supply/rights and metadata; if no eligible asset is available, fail closed and demonstrate *read-only* intent evaluation, clearly labeled.
- [ ] Record what the instrument represents and **does not** represent (no invented voting rights or underlying-share ownership).
- [ ] Record historical work vs newly delivered work at exact commit SHAs with diff and dates.

### Milestone B — safe, minimal Base integration

- [ ] Version a Base instrument-registry adapter with strict chain ID 8453 / 84532 differentiation; verify contract and asset addresses with chain context.
- [ ] Validate source freshness, chain/contract/decimals, owner, recipient, cap, permitted action, slippage/deadline, allowance ceiling, revocation, nonce and replay; default deny on missing or inconsistent data.
- [ ] Bind the source proposal and typed `.x3` mandate to a canonical digest; expose a deterministic `ALLOW_FOR_REVIEW` / `DENY` decision and machine-readable evidence record.
- [ ] Keep wallet signing, market orders, stock transfers, clearing and settlement **OFF** for this US-targeted demo. Do not present local decisions as on-chain authorization.
- [ ] Wire a minimal real Base testnet read or contract interaction only if the exact asset/chain claims can be independently verified, with fail-closed RPC handling. Do not conflate reading with settlement.

### Milestone C — adversarial verification

- [ ] Exercise known-good proposal and genuine negative tests: wrong recipient, wrong chain ID, unsupported contract/asset, changed payload after signature, expired/revoked mandate, nonce replay, malicious prompt instruction, cap/allowance overflow, stale oracle/metadata.
- [ ] Add fuzz/property tests, mutation tests for critical check removal, duplicate/concurrent nonce operations, and parse/canonicalization edge cases.
- [ ] Verify all changed packages on **exact head**, record commands and environment; separate local results, testnet read proof, testnet transaction proof and independently authenticated finality.
- [ ] Ensure full failure does not authorize a partial trade, create phantom fills, or silently change balances.

### Milestone D — reproducible demo and launch

- [ ] Add a one-command Base judge path that fails nonzero if metadata, assets, contracts, source commit or tests are missing. No sample fake proof labeled real.
- [ ] Capture real screen recording according to `VIDEO_STORYBOARD.md`.
- [ ] Prepare a public-safe summary and sanitized reproducibility artifact; do **not** reveal NEAR-repo secrets or unreviewed proprietary code.
- [ ] Review legal requirements and all contest terms (including arbitration) personally. Do not submit automatically.
- [ ] Owner approves public video posting and submission. Tag `@buildonbase` with real demo. Submit official form manually by the posted deadline and retain confirmation.

## Non-goals for hackathon prototype

- No mainnet brokerage, securities/derivatives platform, legal-compliance certification, live U.S. stock trading, production custody, permissionless bridging, native trustless atomic settlement, guaranteed returns, simulated real executions, or fake on-chain proofs.

## Cross-project baseline references

- `README.md` and `PRIOR_WORK.md` — existing X3Lang source/provenance and current NEAR challenge boundary.
- `docs/hackathon/FIRST_SWAP_JUDGE_WALKTHROUGH.md` — historical Base Sepolia → NEAR verified testnet settlement via Tidepool. Valid **only** within its stated trust boundary; not stock trading.
- `Cyptopimpinainteazy/x3DEX` README — frontend/read-only market-data foundation; executable routing remains disabled.
- `docs/base-stocksmaxxing-2026/APPLICATION_DRAFT.md` — application fields, unresolved eligibility and submission gates.
- `docs/base-stocksmaxxing-2026/VIDEO_STORYBOARD.md` — planned demo shots; each requires empirical evidence before publication.

## External references

- https://www.base.org/stocksmaxxing
- https://www.base.org/stocksmaxxing/rules
- https://docs.google.com/forms/d/e/1FAIpQLSeS8uKavZzuH9-vB6bq3oaOsnxTDsrcLtekLBipzW-9x2A3HQ/viewform
- https://docs.base.org/get-started/builder-stack

**Status:** All checkboxes are open. This branch documents a plan; it does not ship any new Base stock-linked feature, register the team, or constitute a contest submission.
