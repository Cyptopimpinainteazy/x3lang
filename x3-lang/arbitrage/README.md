# X3 Arbitrage

A local scanner for cross-chain round trips, conservative route economics, compiled `.x3`
leg exports, persistent paper projections and explicit policy-checked live quote preparation.
It extends `near_liquidity` and `near_intents`; it is separate from the frozen hackathon submission.

## Run

From the repository root, with the existing Python runtime dependencies installed:

```bash
PYTHONPATH=x3-lang python3 -m arbitrage serve --mode demo
```

Open <http://127.0.0.1:8787>. Demo mode uses **constructed examples**, not market prices.
The dashboard includes route filters, cost/floor inspection, paper scenarios, a persistent
journal, and `.x3` downloads that are parsed, typechecked and lowered before returning.
The default SQLite database is `~/.x3/arbitrage/paper.sqlite`. Override with `--database PATH`.
The service binds only `127.0.0.1`; this is a single-user local application, not a public server.

## Live market discovery

Copy `x3-lang/arbitrage/routes.example.json` outside the repository and replace its address
placeholders with your own addresses. Confirm exact token IDs against the current 1Click
catalog (`PYTHONPATH=x3-lang python3 x3-lang/oneclick_catalog.py`). Token IDs in the example
were observed on 2026-11-16 America/Denver; catalog presence does not establish executable liquidity.
No keys or credentials belong in this configuration.

```bash
PYTHONPATH=x3-lang python3 -m arbitrage serve --mode live --routes /absolute/path/routes.json
# Or one read-only cross-chain pass:
PYTHONPATH=x3-lang python3 -m arbitrage scan --routes /absolute/path/routes.json
```

- Cross-chain: 1Click dry quotes on the configured exact assets. Each outward quote is
  followed by a return quote using its fee-adjusted floor as the return input. Existing
  pinned-key signature and request-echo checks are used. No deposit address is requested.
- NEAR, Solana and Ethereum were exercised with real signed dry responses. Other provider
  chains may be scanned, but `.x3` export only supports the chains and unambiguous symbols
  the existing compiler adapter can lower. No new VM/bridge support is implied.
- CEX: Coinbase and Kraken public BTC/USD order books, walking up to 100 levels for a
  $1,000 inventory spread. A conservative **assumed 100bps taker fee per side** is included;
  account-specific fees, inventory availability and rebalancing are unknown. These rows
  remain observation-only; no CEX trading or withdrawal adapter is included.
- At most four configured round trips (eight sequential quote calls plus a catalog request)
  per scan. HTTP timeout is eight seconds per request. Refresh is manual and limited to
  once per 15 seconds; provider failures are isolated and never replaced by demo data.

## Economics and evidence

The input and final output must be the **same chain-qualified asset and decimals**. Each
intermediate input must exactly equal the prior fee-adjusted floor; underfunded legs and
unvalued residual inventory are refused. Profit uses the final floor minus initial input
minus all external costs, in integer base units of the origin asset. No USD peg assumption
or symbol-based cross-chain token equivalence is used.

`external_cost_units` is an explicit **operator estimate**, not a signed or automatically
converted gas quote. Include all gas, approval/return costs and any costs not already
inside provider output. `null` means unknown, not zero, and blocks paper eligibility and
export. `cost_source` records the basis of that estimate. Ambiguous withdrawal fees are
conservatively deducted using `near_liquidity` semantics, which may double-count them.

Quotes older than 30 seconds, future quotes, quotes with less than 10 seconds remaining,
unknown costs, non-closed cycles, and non-positive projected profits cannot be paper eligible.
Independently requested sequential quotes do **not** reserve liquidity, guarantee a combined
minimum, or prove that a profitable cycle can actually execute.

The paper journal records projections, simulated timeout/refund/RPC-failure scenarios, or
refusals. It never claims real fills or realized P&L. Its idempotency keys bind route and
scenario, survive refresh/restart, and are serialized with real SQLite transactions, WAL,
FULL synchronous mode and a five-second lock timeout. This is a projection journal rather
than a historical market replay or a funded paper exchange.

## `.x3` export and live preparation

Inspect a current cross-chain route and supply a source/refund address, destination receiver,
and (for live preparation) mandate owner. Download one `.x3` program **per leg**. It includes
amount, minimum delivery, slippage/fee bounds, nonce, timeout and refund asset. Embedded
metadata binds exact token IDs/decimals, quote expiry and a SHA-256 of the program body.
This is a local consistency check, not an authenticated market attestation or owner signature.

A separate adapter obtains a live deposit quote through the existing guarded pipeline:

```bash
PYTHONPATH=x3-lang python3 -m arbitrage prepare \
  --program /absolute/path/leg.x3 \
  --mandate /absolute/path/owner-mandate.x3 \
  --guard-contract YOUR_GUARD.near \
  --receipt /absolute/path/live-quote-receipt.json \
  --explicit-live-quote
```

Run this only after authorizing that specific preparation. It creates a real deposit quote
and may consume the existing nonce; it does not sign, fund, authorize a Guard write, or
submit a deposit. It refuses demo data, expired exports, changed source or token identity,
and uses an immutable program snapshot plus one catalog snapshot throughout the existing mandate/account/signature/fee checks.
Obtain fresh quotes if the scan has expired; `.x3` files are intentionally short-lived.

**Automatic coordination is implemented** in `trader.py`, with paper and live adapters.
Live funding remains disarmed in the CLI/dashboard; see [automatic trading](AUTOMATIC_TRADING.md)
for the tested state machine and external custody-gateway boundary. A leg's `on_fail rollback` does not
undo a previously settled leg. Reassess the remaining route after every real settlement;
each exported leg by itself does not enforce global round-trip profit or cryptographic cross-chain atomicity.
Live preparation was tested for local refusals; no live deposit quote or wallet trade was
requested while developing this feature.

## Local API

`GET /api/state` returns `x3.arbitrage.v1`: assessed routes, token symbols, provider errors,
latest 100 paper orders and a per-process CSRF capability. All monetary quantities are
base-unit strings. Net profit can be signed; unavailable costs/profit are `null`.

POSTs require `Content-Type: application/json` and `X-X3-CSRF` from state. Host must be
`localhost:PORT` or `127.0.0.1:PORT`; a supplied Origin must match a local origin.
Bodies are limited to 8KiB. No arbitrary filesystem paths, provider URLs, credentials,
wallet calls or signing endpoints are accepted by the HTTP API.

| Path | JSON request | Result |
|---|---|---|
| `/api/refresh` | `{}` | Bounded background refresh |
| `/api/paper` | `route_id`, `key`, optional `scenario` | Durable projection or idempotent replay |
| `/api/export` | `route_id`, `leg`, `refund_to`, `recipient`, optional `mandate_owner` | Compiled source and filename |

Route IDs bind the entire observed route, including timestamps and cost estimate. Refresh
changes IDs. Replaying an existing paper key uses its stored route even after refresh;
a new order needs a current route. This additive API changes no existing compiler/contract
or quote API. Unknown external consumers have not been surveyed.

## Verification

```bash
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests/test_arbitrage*.py \
  x3-lang/tests/test_near_liquidity_quotes.py x3-lang/tests/test_near_liquidity_rhea.py -q
python3 -m compileall -q x3-lang/arbitrage
node --test x3-lang/arbitrage/web/format.test.mjs
node --check x3-lang/arbitrage/web/app.mjs
```

The focused CI workflow runs these offline gates; it never calls market APIs or trades.
Browser verification uses an already-installed Playwright and Chromium against local demo
data. Artifacts under `audit-artifacts/arbitrage-20261116/` distinguish screenshots,
real read-only provider observations, local tests and untested live execution.

Provider references: [1Click quote API](https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote),
[Coinbase order book](https://docs.cdp.coinbase.com/api-reference/exchange-api/rest-api/products/get-product-book),
[Kraken order book](https://docs.kraken.com/api-reference/market-data/get-order-book).

Optional browser gate (reuse an existing installation; no package installation needed):

```bash
X3_PLAYWRIGHT_MODULE=/absolute/path/to/node_modules/playwright \
X3_CHROMIUM=/absolute/path/to/chrome \
X3_ARTIFACT_DIR=/absolute/path/to/screenshots \
node x3-lang/arbitrage/browser-smoke.mjs
```

This starts and cleans up its own loopback demo server and temporary SQLite database.
It checks desktop/mobile layout, filters, route inspection, a paper projection, compiled
program download, and browser console/page errors.

For the complete paper round trip, run `python3 scripts/proof/arbitrage_roundtrip.py --output /tmp/x3-paper-proof.json`.

## More tokens and route building

Use **Load provider tokens** in the dashboard, or:

```bash
PYTHONPATH=x3-lang python3 -m arbitrage tokens
PYTHONPATH=x3-lang python3 -m arbitrage tokens --chain sol --search SOL
```

The public, anonymous 1Click catalogue snapshot on November 17, 2026 lists 231 exact assets,
including 120 across NEAR (36), Solana (23), Ethereum (29), Arbitrum (7), Polygon (9), and
BSC (16). Eleven of those six-chain entries are marked deprecated. Listings are discovery
coverage, not vetted contracts, spend approval, executable liquidity or custody.

The browser keeps exact provider IDs, contract metadata and decimals, flags deprecated
listings, and distinguishes current `.x3` export candidates from discovery-only chains or
unexpressible/ambiguous symbols. Candidate status does not substitute for actual compilation.
Symbols such as `$WIF` are never silently renamed to fit the compiler.

The route builder selects a start token and an intermediate token on a different chain,
accepts human-readable amounts, and converts them using the exact start token's decimals.
For example, BSC USDC uses 18 decimals, whereas NEAR/Solana/Ethereum USDC use 6. A changed
chain/decimal identity is refused **before** requesting a quote for a UI-configured route.
The current bounded session accepts at most four configured round trips; restart with a
reviewed configuration to replace them. Input size limits and the existing 15-second refresh
budget still apply. No amount is automatically increased to bypass provider minimums.

Supply addresses on the two chains only for the dry quote requests. No wallet connection,
keys, deposit addresses or funding calls are involved. Blank external cost remains unknown
and blocks eligibility. If demo rows and provider dry routes share the workspace, the header
and each demo row label that distinction. Catalogue selection and quote/export snapshots
remain separate, so reloading listings cannot swap the token set behind a displayed quote.

`POST /api/catalogue` loads/caches the public catalogue for 30 seconds; GET state returns it
as `catalogue`. `POST /api/routes` accepts exactly `origin_asset`, `destination_asset`,
`amount`, `origin_address`, `destination_address`, `external_cost`, and `cost_source`.
It adds a bounded **dry-discovery** configuration and starts a background scan. These new
POSTs use the same local Host/Origin/CSRF boundary; neither can choose an execution backend.
This is additive to `x3.arbitrage.v1` and leaves existing compiler/contract APIs unchanged.

The optional `X3_CATALOGUE_SMOKE=1` browser gate exercises a real anonymous public catalogue
load and its chain/search filters, including 18-decimal BSC USDC. It does not request quotes
or fund anything. Offline CI remains independent of provider availability.

## Click tokens → edit .x3 → run paper

Each token card has **Start here** and **Scan via** actions. These select exact provider
asset IDs, populate the scanner pair and quote configuration, and lead to the route builder.
Same-chain pairs are flagged, deprecated entries remain disabled, and selection alone does
not request quotes or move assets. Supply amount, read-only quote addresses and costs before
adding a provider scan. The selected pair is visible on the opportunity board.

Each cross-chain opportunity has an **.x3 policy** button, also available from its inspector.
The workbench opens the actual outbound and return `.x3` mandate sources for editing, plus
the simulated wallet addresses and human-readable minimum-profit/external-cost ceilings.
Mandate amounts inside `.x3` source remain integer base units, as in the existing language.

**Compile & validate .x3** uses the existing parser, type checker, lowerer and `Trader.create`
policy preflight in an isolated real SQLite preview. It returns both generated `.x3` intent
programs. It does not consume the running wallet's budgets, run transfers or authorize a
wallet. Registered spending/revocation state is checked again when a paper cycle starts.
Changing source or limits invalidates the preview in the browser; the run API independently
compiles/checks submitted input even if a client bypasses that UI control.

**Run validated paper cycle** persists and executes the exact draft through the existing
coordinator. Original route/token snapshots are owned by the server, not accepted from the
browser. Idempotency keys bind mandate source, addresses, limits, scenario and expiry; replay
survives route refresh/server restart, while changed draft inputs cannot reuse a key.
The older built-in paper demo uses its default fixture mandates and is labeled separately.

The added paper-only endpoints share existing Host/Origin/CSRF/body limits:

| Path | Input | Result |
|---|---|---|
| `/api/strategy/template` | `route_id` | Editable draft `.x3` mandate sources and bounded limits |
| `/api/strategy/compile` | `draft` | Compiled programs and declared policy validation; never wallet authority |
| `/api/strategy/run` | `draft`, `key`, `scenario` | Exact-draft durable paper execution |

Drafts contain only `route_id`, two `mandates`, two `addresses`, `minimum_profit`,
`max_external_cost`, and absolute `expires_at`. They cannot supply a backend, live mode,
provider URL, filesystem path, token substitution or arbitrary route data. Unsupported
chains/symbols, unknown costs, stale quotes and failed `.x3` policy checks remain refusals.
