# Automatic .x3-mandated round trips

`Trader` runs the same durable two-leg coordinator with either `PaperExchange` or
`LiveExchange`. The CLI and dashboard do not arm live funding. Development and verification
used no mainnet/testnet signing, Guard writes, live deposit quotes or transfers.

## Run a complete paper cycle

From the repository root:

```bash
PYTHONPATH=x3-lang python3 -m arbitrage trade --demo --key first-paper-cycle \
  --database /tmp/x3-automatic-paper.sqlite
PYTHONPATH=x3-lang python3 -m arbitrage trade-status --key first-paper-cycle \
  --database /tmp/x3-automatic-paper.sqlite
```

The constructed example starts with 100 USDC plus a 0.5-USDC external-cost reserve, spends
100 USDC, simulates receipt of 101 USDC on NEAR, obtains a fresh return quote for that
confirmed inventory, and simulates receipt of 102.5 USDC on Solana. Final simulated profit
is **2 USDC**, with zero intermediate inventory. These are simulated wallet transactions,
not just the earlier quote projection, but they are not public-chain transactions or market fills.

The dashboard's route inspector has **Run automatic paper cycle** for constructed demo
routes. It displays both compiled `.x3` programs and a separate automatic-cycle journal.
That section filters out live histories. HTTP cannot select a live backend or arm custody.

For a restart test, first run `trade --demo ... --steps 2`, then use:

```bash
PYTHONPATH=x3-lang python3 -m arbitrage trade-continue --key first-paper-cycle \
  --database /tmp/x3-automatic-paper.sqlite
```

A cycle key binds an immutable strategy; it must not be reused for a newly generated demo
or different strategy. `trade-continue` loads the existing snapshot rather than regenerating it.
`trade-resume` resets only a bounded observation budget for a `RESOLUTION_REQUIRED` operation;
it preserves its operation ID and does not unconditionally resubmit a transfer.

`trade --strategy /absolute/path/strategy.json` accepts the explicit `Strategy.document()`
shape: `route`, `tokens`, two `.x3` mandate source strings in `mandates`, two `addresses`,
`minimum_profit_units`, `max_external_cost_units`, and absolute Unix `expires_at`.
Amounts and cost/profit limits use origin base units. Existing compiler/catalog checks refuse
ambiguous symbols or changed exact token identities. `--mode live` cannot arm funding from
this CLI; a non-demo strategy stops at `DISARMED` before network or wallet operations.

## Enforcement and persistence

- Each direction has its own actual `.x3` mandate. The existing grammar pins one spending
  asset per mandate, so opposite directions use distinct owner/agent mandate pairs.
- Mandates are compiled once into registered absolute-expiry policies. Restarting a runner
  does not renew their lifetime. Each `.x3` leg includes the mandate owner, one-use nonce,
  fee/slippage bounds, delivery floor, deadline and refund path.
- Both registered local rolling budgets are checked and reserved atomically before outbound
  execution. Return capacity is checked again for the actual settled amount, including overfills.
  Local reservations do not reserve public-chain budget or liquidity against other actors.
- The return quote must cover initial input + the full external-cost budget + minimum profit.
  An inadequate fresh quote stops with held intermediate inventory; it never invents a rollback
  of the already settled outbound leg. Refunds also leave prior settled legs committed.
- `SUBMITTING` is persisted before invoking a wallet. Stable operation IDs and the backend's
  durable operation lookup reconcile crashes and lost responses. An uncertain operation is
  never blindly resubmitted. Only an authoritative `not_submitted` answer permits another attempt.
- SQLite uses WAL, FULL synchronization, prepared statements, an indexed spend ledger, and
  atomic budget/inventory/operation updates. Confirmed live usage is also persisted locally;
  it cannot disappear when reservations are released. Terminal state and reservation cleanup
  commit together. No SQLite transaction spans a network call.
- A process-wide-per-database file lock serializes runners. Live pending observations yield
  until the next five-second poll and stop after 60 attempts; paper outage fixtures use four.
  Gateway I/O has a total deadline. Neither failure mode causes an infinite retry loop.

Paper scenarios: `success`, `submit_timeout`, `outbound_overfill`, `return_loss`,
`outbound_refund`, `return_refund`, `rpc_failure`, and `timeout`.

```bash
python3 scripts/proof/arbitrage_roundtrip.py --output /tmp/x3-paper-proof.json
```

The proof runs eight isolated SQLite wallets through the real compiler and mandate checks,
asserts their expected terminal/holding states, checks the worked example's simulated P&L,
and records zero network calls and real transactions.

## Disarmed live adapter and custody boundary

`LiveExchange` implements fresh signed dry quoting, both-mandate on-chain preflight,
policy-checked live quote preparation, Guard registration/acceptance arguments, wallet
submission, settlement observation and inventory reconciliation. It revalidates cached receipts
against immutable compiled source, exact IDs/decimals, nonce, mandate owner, all quote guards,
and fee-adjusted output floors. Mere receipt flags and unsigned provider status cannot authorize
funding.

A live backend requires **all** of: an exact approved cycle identity, `funding_enabled=True`
in trusted host configuration, Guard contract, private artifact directory, provider client,
and a compatible external custody gateway with its response public key pinned. No such gateway
or approval was configured in this task. The repository supplies the gateway client/protocol,
not a production custody/signing service. The host must never accept an agent-written boolean
or an HTTP request as authority to arm a wallet. Real funding needs fresh owner approval outside
this paper-only session.

The gateway must durably enforce operation idempotency, recheck its owner authority and
on-chain mandate/revocation/approvals, confirm both Guard operations before funding, bind
exact deposit asset/address/amount/memo, bound gas and external cost exposure, and independently
observe source and destination finality. It must refuse a missing or unverifiable condition.
The coordinator supplies the complete signed quote, source, mandate, Guard arguments and
cost/profit envelope; keys stay in the separate trusted gateway.

### Gateway wire contract: `x3.arbitrage.wallet.v1`

A private Unix socket owned by the current user accepts one newline-delimited JSON request.
Peer UID is checked. Maximum request/response sizes are bounded below 1 MB; the default total
I/O deadline is five seconds. Requests include a random challenge, schema, method and payload.
Methods are `authorize_and_fund`, `observe`, and `inventory`.

Responses are `{ "body": { "request_sha256": "...", "result": {...} }, "signature": "BASE64" }`.
The Ed25519 signature covers `near_liquidity.quote.canonical_json(body)` and the request digest
covers the exact canonical request bytes without its newline. A mismatched challenge/digest,
key, signature, peer, operation or quote binding is refused.

- Funding results bind `operation`, `quote_hash`, `status: submitted`, `funding_tx`, and
  `funded_at_ms` within the signed quote's window.
- Final observations bind `operation`, `quote_hash`, `program_sha256`, `recipient`, `refund_to`,
  `amount_in`, exact `asset_out`, positive `amount_out`, `funding_tx`, `payout_tx`,
  `funded_at_ms`, `finality: finalized`, and `status: settled|refunded`.
- Pending/unknown/not-submitted observations retain their stable operation binding.
  `not_submitted` is an authoritative custody-journal assertion, not a missing RPC result.
- Inventory results return a `balances` map of exact asset keys to base-unit strings.

A signed gateway response is a **trusted-host attestation**, not a public-chain light-client
proof, wallet isolation, solver causality proof, or cryptographic cross-chain atomicity. A
production integration must be reviewed and exercised with the actual gateway before claiming
real-chain readiness. The development tests exercise real private sockets and Ed25519 locally,
and simulate the live backend contract only to test coordinator accounting without funding.
