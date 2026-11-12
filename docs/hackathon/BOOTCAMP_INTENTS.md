# Bootcamp: NEAR Intents integration

Original X3 implementation guidance, reviewed 2026-11-11. Educational summaries, not a substitute for provider terms or legal advice. Features described below are not claims of deployed execution.

## Execution and evidence

An intent is a desired outcome; a mandate is the owner’s authority. The compiler, Guard, provider infrastructure and chain verification have different jobs.

Read-only Explorer history is provider-reported, even when status says SUCCESS. A signed quote proves terms, not settlement. Guard acceptance is not funding. Keep Tidepool testnet evidence separate from official 1Click.

Source: https://docs.near-intents.org/integration/distribution-channels/1click-api/explorer/introduction

## Four swap types

EXACT_INPUT fixes origin units. EXACT_OUTPUT fixes destination units and applies slippage to the required input. FLEX_INPUT accepts variable deposits; the quote is not a spending cap.

ANY_INPUT is an ongoing collector: the supplied documentation specifies a $1,000 sweep threshold, no refunds, and partner authorization. This is not a universal minimum for ordinary swaps. Its creation deadline does not expire the collector. Do not enable ongoing collection under a one-payment mandate.

Source: https://docs.near-intents.org/integration/distribution-channels/1click-api/swap-types

## Costs and application fees

Basis points: 100 = 1%. The requested app fee may differ from the amount charged after provider minimums. The generic public policy is only an illustration; actual partner configuration and quote terms govern.

Distinguish protocol, provider, application, network and withdrawal costs. Do not automatically add another website’s distribution fee. Price improvements may be shared; failed swaps can still incur costs. Confidential swaps have a separate fee arrangement. No application fee is activated here.

Source: https://docs.near-intents.org/resources/fees

## Deadlines and signed terms

For local planning, dry quotes use the maximum window. Executable proposals use min(maximum, max(default, estimate)), bounded again by owner expiry. An estimate beyond the remaining window refuses execution.

The supplied narrative contradicted its 75-minute example. This lab explicitly adopts the capped example, not an asserted provider rule. Invalid estimates fail closed. Never mutate a signed quote: request a fresh quote, verify its signature, and rerun policy checks.

Source: https://docs.near-intents.org/integration/distribution-channels/1click-api/quickstart/making-a-request

## Bridges and refund paths

Omni, POA and HOT are distinct routing infrastructures with different trust assumptions. Bridge choice and supported assets must come from current route metadata. Treasury, refill and change addresses are reference labels, never substitute deposit destinations.

The BTC connector refund flow is specific to a deposit that never finalized. It requires a request, timelock, execution and MPC signing. Check actual state and current required storage deposit; never automate refunds from this lesson. BTC/ZEC withdrawals may use single-use inputs, so treasury-sender matching alone is insufficient.

Source: https://docs.near-intents.org/integration/bridging/overview

## Hyperliquid transfer boundaries

The supplied guide describes USDC credited to a perps balance, not spot or HyperEVM. That transfer does not itself place a derivative trade. Hyperliquid USDC uses 8 decimals; use integer quantities.

For deposits from Hyperliquid, the guide specifies 0.5 USDC minimum and 0.2 USDC fee. sendAsset/spotSend use quoted amountIn; usdSend additionally needs 1 USDC. Verify current quotes and supported methods. Direct bridge transfers to a quote address are not guaranteed. Derivatives access is not enabled here.

Source: https://docs.near-intents.org/integration/distribution-channels/1click-api/hyperliquid

## Earn and receipt tokens

Earn routes payment assets into third-party receipt tokens using the quote/deposit/status flow. A receipt represents a position, not a guarantee of redemption, principal or yield.

Display the actual provider, receipt asset, withdrawal conditions, lockups and costs before owner approval. Availability through the API is not an audit or endorsement. This site has no enabled Earn execution path.

Source: https://docs.near-intents.org/integration/distribution-channels/1click-api/earn

## Official SHIELD and X3 Shield

Official SHIELD communicates provider incidents; X3 Shield enforces owner policy. They are separate systems. Read incidents with a separate SHIELD credential. Incident submission needs additional provider permissions.

The incident API does not expose every surface security mode. Do not translate operational into Usual, or an incident into Under attack. Stale, malformed or unavailable data is unknown. Public views exclude internal descriptions, metadata and actor details. No incident submission is implemented.

Source: https://docs.near-intents.org/security-compliance/shield-incident-api

## Solver lifecycle and NO_LIQUIDITY

Solver activity is separate from ordinary 1Click integration. It requires onboarding, KYC/KYB, controlled liquidity, signing infrastructure and operational controls. NO_LIQUIDITY does not provide financing.

Use swap_hash for deduplication after selection; update delivery estimates until liquidity is delivered. Confirm deadline units and how the escrow flow interacts with firm-quote obligations before live work. Do not fill coordinated self-originated hackathon intents. No solver bus connection or signing is enabled.

Source: https://docs.near-intents.org/integration/market-makers/solver-terms-of-use

## Terms, records and disclosure

The API terms and solver terms govern different roles. Review API resale restrictions, user disclosures, credentials, compliance duties and incident reporting with appropriate advisers. A public app route must not become a standalone API resale gateway.

The API has a general 24-hour data-retention limit with a narrow history exception; solver obligations require seven-year records. Keep these stores and purposes distinct. Provider order flow must not feed unrelated model training. Do not infer endorsement or audit coverage for X3.

Source: https://docs.near-intents.org/security-compliance/terms-of-service

## Credentials and SDKs

Explorer, Swap and SHIELD access are separate capabilities. Keep credentials server-side. Use the official TypeScript, Go or Rust SDK matching the task; token metadata is available without authentication.

Preserve depositAddress and depositMemo together, use fresh deadlines, and distinguish dry quote simulation from mainnet execution. FAILED does not establish REFUNDED. Never paste credentials into lessons or commit them.

Source: https://docs.near-intents.org/integration/distribution-channels/1click-api/sdk

## Security and source references

Use published audit reports and the provider bounty program for their stated scopes. They do not prove that X3Lang or dotx3 has been audited. Provider screening coverage varies by flow.

Treasury references identify entities and address roles, not approved recipients or a proof of swap causality. Follow current official documentation and review source dates.

Source: https://docs.near-intents.org/security-compliance/security

## Additional references

- [Complete documentation index](https://docs.near-intents.org/llms.txt)
- [Solver onboarding](https://docs.near-intents.org/integration/market-makers/quickstart)
- [Confidential solver](https://docs.near-intents.org/integration/market-makers/confidential-example)
- [Treasury address directory](https://docs.near-intents.org/security-compliance/treasury-addresses)
- [BTC refund procedure](https://docs.near-intents.org/integration/bridging/btc-deposit-refund)
- [Fee configuration](https://docs.near-intents.org/integration/distribution-channels/1click-api/fee-config)
- [Provider audit reports](https://drive.google.com/drive/folders/1eNHI_GKsbmMSjeCENRklvtVh8imGSUvy)
- [Provider bounty program](https://hackenproof.com/programs/near-intents)
- [AML portal](https://aml.near-intents.org/)
