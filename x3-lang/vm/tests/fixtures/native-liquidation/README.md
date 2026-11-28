# Local native liquidation fixture

This fixture deploys **real Morpho Blue v1.0.0** and the public Profit Monkey
`MonkeyLiquidatorSwap` on an owned, loopback-only Anvil chain (31337). Tokens,
oracle and DEX are explicit test mocks. A successful swap moves actual local ERC20
balances; the genuine Morpho callback pulls repayment before the owner receives
the remaining loan tokens. This is local contract execution, not a public-chain
deployment, real-market quote, profitability finding or X3 runtime proof by itself.

## Reproduce

Requires Node 20+ and Anvil on PATH (or `X3_ANVIL_BIN`). No key files, external RPC,
external funds or signing credentials are used. Anvil runs with one thread.

```sh
cd x3-lang/vm/tests/fixtures/native-liquidation
npm ci --ignore-scripts
npm run build
npm test
node fixture.mjs --self-test --out proof.local.json
```

`source-pins.json` records every vendored source/license SHA-256 and original
repository, path and full commit. Morpho is pinned to
`55d2d99304fb3fb930c688462ae2ccabb1d533ad`; the executor and mocks are unchanged
from public Profit Monkey commit `1a8ead1ff4b67899b48d9cadc979373710e57592`.
Original SPDX headers and Morpho licenses are preserved. The npm lock pins
`solc` 0.8.19 and `viem` 2.58.8. Build settings are optimizer 999999 / Paris.
The build checks source hashes and the exact compiler version before compiling.
The runtime checks those source hashes and the artifact source-manifest hash;
run the build first whenever preparing a fresh checkout.

## Seeded market

- Mock loan token: FUSD, 6 decimals. Mock collateral: FBTC, 8 decimals.
- Borrower posts 1 FBTC, borrows 80,000 FUSD at 86% LLTV with a zero-rate IRM.
- Oracle starts at 100,000 and falls to 92,000 FUSD/FBTC, making the loan unhealthy.
- Mock DEX initially pays 50 bps below the oracle. The owner receives roughly
  3,089 FUSD after actual Morpho repayment. The executor enforces both a 0.5%
  owner floor and the order's 1,000-FUSD minimum.
- A direct Morpho `eth_call` produces `repay_assets`; a real executor `eth_call`
  produces `gross_profit`. These are quotes for this deterministic local state.

The proof mines a success, a completed DEX swap rejected by callback settlement
(`SwapShort`), and a DEX call that reverts during the callback (`SwapFailed`).
Both failures restore token balances, allowances, borrower position and market
accounting. Sender nonce and gas payment still advance, as normal on EVM. Gas is
reported in native wei separately from loan-token profit; no FX assumption is made.
Compact call traces show which nested call failed; a successful nested DEX call
inside a reverted outer transaction does not commit.

## Native runtime test seam

Start the server directly so stdout contains JSON lines only:

```sh
node x3-lang/vm/tests/fixtures/native-liquidation/fixture.mjs --serve
```

The first line is a manifest with lowercase addresses and these fields:

| Field | Meaning |
| --- | --- |
| `rpc_url`, `chain_id` | Owned loopback node, chain 31337 |
| `keeper`, `owner` | Separate local accounts; only owner receives loan-token profit |
| `morpho`, `executor`, `executor_code_hash`, `router` | Deployed addresses and observed executor runtime Keccak |
| `market` | `loan_token`, `collateral_token`, `oracle`, `irm`, decimal-string `lltv` |
| `market_id`, `borrower` | Morpho market identity and seeded borrower |
| `seized_assets`, `repaid_shares`, `min_profit`, `repay_assets`, `gross_profit` | Decimal strings in raw token units |
| `swap_data`, `calldata`, `gas_limit` | Router calldata, complete liquidation calldata, local tx gas cap |
| `block_number`, `block_hash`, `timestamp` | Current local state anchor after preparation |
| `source_manifest_sha256` | SHA-256 of the committed source pin manifest |

Write one JSON command per stdin line; each produces one JSON reply:

- `{"op":"setKeeper","address":"0x..."}`: local owner mines `setKeeper`, funds
  that address with 1000 synthetic ETH, and returns `{ok, receipt, manifest}`.
  The address is **not unlocked or impersonated**; a native client must sign its
  own raw transaction. Call before liquidation, or reset first. This refreshes
  the reset baseline so subsequent resets retain the configured keeper.
- `{"op":"state"}`: `{ok, ...state}` with owner/pool/router/executor token
  balances, allowances, borrower/market accounting and keeper ETH/nonce.
- `{"op":"reset"}`: restores the seeded snapshot and returns `{ok, manifest, state}`.
- `{"op":"swapFail"}`: changes only the local DEX to 5% below the oracle and
  returns `{ok, receipt, state}`. Previously quoted liquidation terms are now stale.
- `{"op":"stop"}`: `{ok:true}`, then terminates the owned Anvil child. Closing
  stdin also cleans up. Always stop/close stdin and wait for exit before killing
  a server process; SIGKILL cannot run cleanup handlers.

Receipt fields are `hash`, `status` (`success`/`reverted`), `block_number`,
`block_hash`, `gas_used`, and `effective_gas_price_wei`. Invalid commands return
`{ok:false,error}`. Neither stdout nor evidence contains private keys or mnemonics.

The native Rust test should consume this server, configure an ephemeral signer,
compile/execute the actual `.x3` path, and independently check its transaction and
the returned state. The `--self-test` intentionally proves contracts only; it
does not stand in for that compiler/VM/transport integration.
