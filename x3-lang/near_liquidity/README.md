# NEAR liquidity discovery (issue #40) — read-only, post-freeze draft

This package compares NEAR liquidity quotes. It **never** signs, deposits, broadcasts,
reserves on-chain liquidity, activates fees or trades. It is not part of the
2026-11-14 X3 Shield submission.

| Module | Role |
|---|---|
| `quote.py` | Versioned `ProviderQuote` (`x3.provider_quote.v1`), checked integer math, sealed signature status |
| `oneclick.py` | 1Click `/v0/quote` response → `ProviderQuote`, reusing `near_intents` signature and echo checks |
| `chain.py` | NEAR view calls pinned to one fresh final block that ≥2 independent HTTPS RPC hosts agree on |
| `rhea.py` | RHEA (`v2.ref-finance.near`) pool discovery and `get_return` observations |
| `ranking.py` | Provider isolation, rate budgets, like-for-like aggregation, net-output ranking, single-use selection |
| `providers.py` | Which providers have adapters and which are blocked, with the documentation checked |

## Executable means exactly this

A quote is `executable=True` only when all of these hold, and the record refuses to be
constructed otherwise:

1. a provider signature verified in this process under a key pinned in reviewed code
   (today only `near_intents.ONE_CLICK_MANAGER_PUBLIC_KEY`);
2. the signature binds the terms to a deposit (`SIGNED_DEPOSIT_QUOTE`: a live 1Click quote
   with a deposit address and deadline) — a dry quote is indicative only;
3. every fee is covered by that signature (an echoed, unsigned `appFees` is not);
4. the guaranteed net output — the signed floor less every fee not stated to be inside it —
   is computable in the output asset and positive.

"Executable" is a property of the evidence, not permission to act: nothing here executes.

RHEA results are observations of shared AMM state, agreed by several RPC operators. That is
not a light-client proof, not a reservation and not a guarantee; they are always
`executable=False` and are returned separately from ranked quotes.

## Provider status (checked 2026-11-13)

- **1Click:** adapter. Field meanings from
  <https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote>.
- **RHEA:** adapter. Contract from <https://guide.ref.finance/developers/contracts> and
  <https://docs.near.org/primitives/dex.md>; methods from
  <https://ref-finance.gitbook.io/ref-finance/smart-contracts/ref-exchange>.
- **LI.FI:** blocked. Its docs list NEAR Intents as a bridge it aggregates, not NEAR as a
  supported chain (<https://docs.li.fi/introduction/chains.md>,
  <https://docs.li.fi/changelog/backend.md>).
- **Rango:** blocked. Its docs say Near support is coming and list no NEAR blockchain or
  transaction type (<https://docs.rango.exchange/llms-full.txt>).

## Known limits

- The sealed signature status is a code-review boundary: in-process code can call the
  private `_issue_verified`, though it still runs the signature check under the pinned key.
- `withdrawFee` is deducted from the guarantee because the 1Click reference does not say
  whether `amountOut` is already net of it. This may understate a quote; it cannot overstate one.
- RPC agreement narrows trust in operators but does not remove it.
- RHEA `total_fee` is kept verbatim; its unit is not in the cited docs and it is not used in math.
- Freshness of a dry quote uses `near_intents.MAX_QUOTE_AGE_S`, recorded as policy.
- Pool IDs are taken as `from_index + offset` in `get_pools` pages, as the documented
  `from_index`/`limit` paging implies; `get_return` is then asked for that ID at the same block.
- An RPC host that has not yet seen the first host's final block makes discovery fail
  closed (`X3_LIQ_RPC_ERROR`); retrying is the caller's choice.
- No adapter output has been compared with live provider or chain answers: the shell that
  wrote this could not reach provider APIs or NEAR RPC. RHEA tests use a constructed fixture.

## Tests

```bash
PYTHONPATH=x3-lang python -m pytest x3-lang/tests/test_near_liquidity_quotes.py x3-lang/tests/test_near_liquidity_rhea.py -q
```
