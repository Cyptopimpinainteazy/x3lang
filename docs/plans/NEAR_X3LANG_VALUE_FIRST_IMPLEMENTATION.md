# NEAR / X3Lang — value-first monetization integration gates

This plan is the handoff for the agent building X3 Shield's NEAR monetization and
mandated spending limits. **Only the preview arithmetic is implemented** in
`x3-lang/value_fee.py`. Never represent plans as deployed capabilities.

## Principles

1. The user receives a measurable **positive net improvement** after all fees.
   Without a trustworthy obtainable baseline, unknown costs, or authenticated
   final execution evidence, no success fee may be charged.
2. All fees are **opt-in**, visibly itemized before approval, capped in absolute
   amount and basis points, bound to a chain+asset+receiver+intent+nonce+expiry,
   and limited to the specific value-creating service.
3. Source code is already public and has a preserved historical baseline:
   don't claim an embedded license check prevents forks. Revenue must come
   from valuable **managed services**, real provider integrations, or a user-
   authorized on-chain fee. Prior contributor/license rights need auditing.
4. The language's original VM/compiler history and `PRIOR_WORK.md` cannot be
   rewritten or passed off as new NEAR hackathon work.
5. Fail closed. No test mock or dry quote may be advertised as an executed
   swap, proof of independent chain settlement, or paid integration.

## A concrete mathematical contract

Let `R` be a comparable *obtainable* baseline route's guaranteed destination
base-units net of all charges not already deducted in its quote. Let `C` be
the candidate route's corresponding amount. Then `S=max(C-R,0)`. An authorized
operator fee may be `F <= floor(S * user_approved_share_bps/10000)`, `F <=
user_approved_absolute_cap`, and `C-F >= R+min_positive_user_gain`. If `S=0`,
no fee. Never double-count costs already included in a signed quote; only
subtract additional independently measured costs. The preview is **not**
permission to charge, and no user approval is inferred from `opted_in=true`
in an input JSON payload.

## PR phases (complete in the stated order)

**Phase 1 (implemented; tests verified locally):** quote economics primitive,
nonbillable CLI, bounds, required comparability and stale-quote refusals,
18 TDD tests, docs. Keep it separate from trading authorization.

**Phase 2 (not implemented):** connect to signed and authenticated NEAR 1Click
quotes; source/destination asset ID/decimal verification; two independent
obtainable same-time routes, no cherry-picked references; compare net receiver
amount after existing protocol/solver fees, bridge/gas/refund charges and
slippage. If available quote infrastructure only provides one route, return
`reference_unavailable`, don't invent savings. Persist evidence/hashes.

**Phase 3 (fee linkage not implemented):** The independent NEAR agent mandates
work was merged on `x3lang/main` in **PR #3** (`78f72a9`) after this branch
started. Its `mandate.rs` and `near_mandate.py` implement Guard-side agent
registration/acceptance restrictions, rolling limits, owner revocation and
approval checks. **Do not claim mandates themselves authorize a wallet spend**:
Guard holds no funds and an external wallet could bypass it unless separately
restricted. The missing monetization integration requires a versioned X3Lang
fee-policy AST + typed, signed artifact, explicit owner consent and Guard-side
fee cap/recipient checks. Reuse existing mandate keys and spending counters
rather than creating duplicate or inconsistent authorization state.

**Phase 4 (not implemented):** add 1Click `appFees` only if the provider's
signed quote **cryptographically covers the authorized fee** (an echoed but unsigned `appFees` field is insufficient), gives a strictly better net result
than the reference, and refunds/failure receipts are reconciled. An app fee is
expressed as bps of the trade, NOT bps of savings; compute a safe integer bps
floor (round down), re-quote including the app fee, reject rounding that
would violate the user's absolute cap or minimum benefit, and prove provider
paid exactly one authorized recipient. No silent `appFees`, referral, markup,
retry billing, or fee on failures/refunds. Respect 1Click partner fee terms.

**Phase 5 (not implemented):** independent source deposit+destination receipt
attribution and proof of realized gain. On-chain or audited service state
reconciles gas, final amounts, refund, quote expiry, duplicate receipts and
actual net outcome. User can independently replay all receipts. Bill only on
cryptographically attributable settled value with pre-authorized wallet/
contract enforcement; no unsigned provider status alone.

**Phase 6 (not implemented):** premium API service with authenticated tenant
accounts, rate limits, per-service SLAs and billing for verified user benefits;
read-only solver/MEV/oracle/LP analytics, agent-commerce/escrow and autonomous
business extensions with service-specific consent. Don't enable raw flash
loans or asset custody without audited contracts and real finality evidence.

## Prioritized feature integration from the 50-idea catalogue

| Feature | Shared X3Lang policy | When user value / revenue is allowed |
|---|---|---|
| #4 Sandwich/MEV defense | Oracle, price deviation, execution floor | Only for measurably better net routes; no claim of immunity |
| #32 Cross-chain payouts | Allowed assets/recipients, finality and refunds | Better net payout plus attributable delivery; otherwise $0 |
| #50 Security firewall | Mandate, calldata, permission and replay policy | Opt-in managed value, not mandatory language royalty |
| #5 AI portfolio | AI proposals require deterministic guard approval | Savings/operational-value subscriptions only after value demonstrated |
| #39 Solver order-flow | Authenticated comparable independent quotes | Transparent share of realized routing gains |
| #41 Liquidity routing | Quote ceilings, exact integer amounts | Lower full-route cost for equivalent execution |
| #38 Agent data commerce | User-approved per-service amount and invoice | Paid service delivered and receipted |
| #17 Streaming payments | Per-merchant budgets and periods | Opt-in automation/service fees with real value |
| #9 Oracle, #23 arbitrage, #26 LP, #29 lending | Verified sources, no imaginary collateral/atomicity | Premium analytics only until provider adapters audited |
| #11 gas sponsorship, #47 escrow | Budget and authorization contracts | Users receive net gas savings or verified escrow release |
| #19 and #6 flash loans | Same-chain atomic repayment and real lender proof | No live integration until provider/custody security audit |

## Mandatory quality gates

- Reproduce Rust compiler+VM+Guard and Python+browser tests on **exact PR HEAD**.
- First red, then green, for real behavior tests; include adversarial fee
  tampering, forgery, double charge, duplicates, race, zero savings, future /
  stale reference, wrong asset decimals, unauthorized recipient, alternate
  signer, contract upgrade, refund, slippage, route switching and expired quote.
- Property/fuzz tests for fee arithmetic, irreversible transfers, time windows,
  token decimals and cap invariant. Controlled on-chain sandbox if available;
  mainnet only with separately authorized minimum funds, never private keys
  in logs. No production mocks, no disabled checks, no unsubstantiated claims.
- Provide explicit evidence table: compiler, adapter, wallet, Guard, provider,
  source chain, NEAR chain, receipt attribution, billing authorization.
- Require third-party security review, licensing audit, and final human
  approval for any value transfer, release, fee activation or mainnet deploy.

**Success criterion:** users can opt into a service whose independently
verifiable *realized* net benefit exceeds its clearly approved X3 fee. The
service works even if an AI prompt is malicious, while the open X3Lang source
remains usable without secret monetization switches.

**Cost reconciliation guard:** the preview requires `costs_complete: true` for
both offers; this is a caller assertion, **not independently verified proof**.
It remains non-billable (`collectible: false`) and is invalid if either quote
is more than 120 seconds old, even when a provider sets a distant expiry.
