# All-chain 1Click discovery adapter

`x3-lang/oneclick_catalog.py` discovers every chain/asset returned by the current
`GET /v0/tokens` catalog. It reuses the existing HTTP client; it does not modify
compiler registries, wallets, Guard, or evidence verifiers. No symbols are used
as asset identity: callers select the exact provider assetId.

## Run

From the repository root, after installing the README Python dependencies:

```sh
python3 x3-lang/oneclick_catalog.py
python3 x3-lang/oneclick_catalog.py --request /path/to/dry-request.json
```

The request must explicitly contain dry=true, swapType (EXACT_INPUT or
EXACT_OUTPUT), originAsset, destinationAsset, amount (positive base-unit integer
string), depositType, recipientType, refundType, recipient, refundTo, deadline,
and slippageTolerance (0..500 bps). Unknown fields are rejected. Addresses must
be supplied by the caller; this discovery layer does not validate chain-specific
address formats or prove account existence. The provider can refuse them.

## Compare provider and compiler chain codes

```sh
python3 x3-lang/oneclick_catalog.py --chain-coverage
python3 x3-lang/oneclick_catalog.py --chain-coverage /path/to/saved-v0-tokens.json
```

With a file, the comparison runs offline. Without a file, it reads the public
token list anonymously. The report groups distinct provider codes into three
sorted lists. `listed_mapped_codes` occur in both the token list and the current
compiled adapter map. `catalog_only_codes` appear only in the provider list.
`missing_mapped_codes` are configured in the adapter and absent from the supplied
list. Use a complete provider snapshot to check for missing mappings. A filtered
list can report omissions caused by the filter.

The command exits 0 when every mapped code appears, 1 when any are missing,
and 2 on invalid input or a fetch error. Newly listed codes remain available
for discovery. The report makes no quote and grants no payment authorization.

## Limits

The advisory snapshot supplied by the owner on 2026-11-11 says:

- Only ORIGIN_CHAIN -> DESTINATION_CHAIN is subject to these chain minimums.
- BNB (`bsc`) on either side: USD 1,000.
- Tron (`tron`) as origin: USD 100.
- Other routes: no documented chain minimum, NOT guaranteed liquidity or zero
  bridge minimum. These rules do not apply to deposits/withdrawals via INTENTS.
- EXACT_INPUT values the origin amount; EXACT_OUTPUT values the destination
  amount, using that asset's decimals and catalog price. Equality meets the floor.

Source: https://docs.near-intents.org/resources/swap-limits
Token catalog: https://docs.near-intents.org/api-reference/oneclick/get-supported-tokens
Quote API: https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote

Prices and the rule snapshot may be stale. Advisory values never authorize or
block a request; the actual provider response is authoritative for availability.
Provider minimum and bridge errors are preserved. No auto-increase, route switch,
fee activation, deposit submission, signing or nonce consumption occurs.

## dotx3 integration contract

Use the catalog for chain/token selectors, showing contract address and assetId.
The CLI emits JSON; this PR does not install a web API or modify dotx3.
Never render a discovery response as "Guard approved" or "ready to deposit":
`eligible_for_wallet_authorization`, `policy_checked`, and `settlement_verified`
are false. Returned quote data is explicitly unverified. Dry quote addresses,
if present in a provider response, are not payment instructions.

Funding must continue through the existing compiled .x3 mandate, signed live
quote validation, appropriate custody/signing boundary and independent chain
verification. Those capabilities are NOT enabled for every discovered chain.
PR #188 separately probes candidate Base/Solana/NEAR routes; this module does not
replace its signed-quote checks. Integration must retain them before live use.
