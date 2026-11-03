# X3Lang commercial architecture — user-first monetization

**Status:** November 3, 2026. A proposed business model with a **working,
non-billable off-chain savings-fee preview** (`x3-lang/value_fee.py`). No
payment collection, revenue contract, service license server, production
application fee, wallet signature or solver integration has been enabled.

## Rule: the user must win first

The commercial offer for managed routing is a **share of a strictly positive,
measurable net improvement** over a comparable baseline. The explicit policy:

- Compare the **same** source asset/amount, destination asset, recipient and
  quoted time window. Use destination-token **integer base units**, never
  displayed floating-point USD estimates.
- Deduct all known provider, network, gas and routing charges from **both**
  options before measuring improvement. Unknown cost or stale/incomparable
  inputs **refuse** comparison. Untrusted or unreasonably bad benchmark quotes
  cannot establish real savings.
- Proposed X3 service fee is **0 by default** unless the user opts in;
  when opted in, use the **lowest of** a published share-of-savings ceiling,
  an absolute maximum, and the limit that leaves the user a strictly positive
  benefit. Calculation rounds the *fee down* to token base units.
- No fee for no improvement. Quotes are **not finalized executions**. This
  preview is never a billable receipt; a fee needs real wallet/contract
  authorization, an authenticated independent reference, actual execution
  proof linked to the user's original intent, and a reconciliation of final
  costs, refunds and realized recipient output. If those requirements fail,
  **no platform success fee**.
- Disclose the fee recipient, route comparison and *total net amount to the
  user* before any transaction. Do not charge for routes the user could have
  taken for the same or better net price without the X3 service.

### Implemented example (fictional data, no wallets)

```bash
python3 x3-lang/value_fee.py --preview-stdin \
  < x3-lang/examples/fees/verified_savings_preview.json
```

The example's reference net is **24.880000 USDC**, candidate pre-fee net
**24.970000 USDC**, improvement **0.090000 USDC**, 25% proposed service share
**0.022500 USDC**, and user post-fee benefit **0.067500 USDC**. The output
will say `collectible: false`; sample quote IDs are deliberately marked
illustrative and are *not* presented as cryptographic evidence.

## Revenue lines worth developing

| Service | Charge condition | Why a customer benefits | Execution/protection boundary |
| --- | --- | --- | --- |
| Managed NEAR route optimization | Opt-in share of **realized** positive user savings, capped and fully disclosed | Pays only for a better net route | Requires authenticated competing quotes and linked finalized settlements |
| Managed agent treasury controls | Optional hosted subscription with opt-in, contractual ROI guarantee or free trial | Saves operational costs or avoids demonstrable mistakes | Wallet/NEAR contract, not a prompt, must enforce spending limits |
| Verified cross-chain settlement evidence API | Subscription or per **verified** receipt for customers who voluntarily buy it | Saves integration/operations cost | Must independently bind source and destination receipts, not unsigned status |
| Enterprise managed compiler/security service | Commercial service agreement for support, private SLAs, compliance evidence | Reliable operations, reduced maintenance cost | Must check pre-existing contributor IP rights and third-party licenses |
| Solver/channel partnership | Disclosed integrator revenue share only when net user outcome is no worse | Competitive routing, no hidden markups | Need provider commercial terms; don't assume automatic eligibility |
| Liquidity and risk analytics | Paid managed reports only where they demonstrably reduce risk/operating cost | Position safety and timely operational decisions | Read-only first; no flash lending or asset custody |

The open X3Lang policy math is **not** the revenue product. The differentiated
managed business is the live quote/solver infrastructure, independently
verified receipts, uptime, support, and authorized settlement services.

## NEAR Intents integration path

NEAR's published 1Click API supports an optional `appFees` quote field of
`[{"recipient": "yourfee.near", "fee": 100}]` (basis points) and a
`referral` identifier. See the [official NEAR integration guide](https://github.com/near/agent-skills/blob/main/skills/near-intents/rules/api-quote.md).
**This code does not add any appFees or referral to real transactions.**

Before using appFees, the quote request AND its authenticated signed echo,
fee/cost arithmetic and refund path must be reviewed together. 1Click may
charge its own minimum fee and share revenue with integration partners; don't
assume an app fee is additional profit or the cheapest route. A fixed bps
app fee is not equivalent to an opt-in percentage of savings: actual `appFees`
will require a provider-specific translation and new adversarial tests. If
adding it would make the user worse off, use **zero fee** or a different route.

## Forks, ownership and licensing — what is realistic

- **No local or embedded license key can make a public repository
  un-forkable.** In source code, forks can remove a royalty branch; this
  language library must not disguise it as an unbreakable payment system.
- The public repository currently has **no root LICENSE**. GitHub's Terms of
  Service allow viewing and forking through GitHub, but a public repository
  does not automatically grant unrestricted commercial redistribution.
- Do **not** silently relicense existing historical source: the preserved
  Git history has multiple contributors. Audit ownership, pre-existing
  third-party code/licenses, and contributor permissions with counsel
  **before** choosing source-available commercial terms, dual licensing,
  AGPL, or a proprietary redistribution license.
- A real commercial gate belongs on a **server-operated, authenticated
  service** or a specific, user-opted-in smart contract with immutable or
  governance-restricted payment caps. Users and forks that never use those
  services cannot technically be forced to pay for them.
- Don't put proprietary backend secrets, API keys, signer keys or revenue
  credentials in this publicly reviewed hackathon source. Any commercial
  service is a separate system and must not be required to reproduce the
  open judge demo.
- Keep the published prior-work disclosures and `baseline` tag intact.

## Open implementation gaps — do not market as done

1. Independent authenticated same-time route reference, no cherry-picking.
2. User wallet authorization binding the exact service, fee limit, asset,
   receiver, chain ID, deadline, nonce and refundable failure policy.
3. Source/destination chain **causal attribution** and verifiable net
   settlement against the reference, including all failed/refund costs.
4. Provider-supported `appFees` with signed-quote echo and fee-receipt proof;
   prohibit duplicate billing and incorrect decimals.
5. Reconciliation, anti-replay accounting, user dispute path, compliance/tax
   analysis and an external review before handling customer funds.
6. License/IP ownership review and selection — no license is changed here.
