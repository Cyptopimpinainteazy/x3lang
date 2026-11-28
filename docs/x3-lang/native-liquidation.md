# Native liquidation execution

`native_liquidation` compiles through the Rust X3 parser, AST, IR and binary
emitter to bytecode version 3, opcode `0xAC`. The verified VM filters a bounded
runtime candidate batch, ranks by conservative **net loan-token profit**, and
constructs the exact `MonkeyLiquidatorSwap.liquidate` call. Its in-process Rust
host re-simulates the complete call, signs with libsecp256k1, and encodes the
transaction with Alloy before sending on a local Anvil chain. No TypeScript or
Python bot is invoked by the effect.

This is an off-chain VM host effect. The bytecode is not deployed to an EVM or
the X3 chain. Existing `atomic_liquidation`, generic SVM host calls and PIR's
Base Sepolia/NoAsset restrictions retain their separate semantics.

## Authority and accounting

The compiled policy pins chain, keeper, owner, Morpho address, executor runtime
code hash, loan token, armed Morpho market IDs and routers. It sets repay/profit,
gas, fee, freshness, expiry and candidate-count limits. Every field is mandatory;
identities use lowercase, full-width hexadecimal. Loan amounts use **raw token
units** (`1_000_000_000` is 1,000 units of a six-decimal token); native fees use wei.
The initial adapter supports seize-asset swap orders, with `repaidShares = 0`.

The embedding host pins an independently approved artifact digest. A scanner
cannot substitute a more permissive source file and reuse that approval. The
VM chooses highest `gross_profit - cost.loan_units`, breaking ties by candidate
ID. Unknown fees, losses, out-of-policy routes and stale batches are refused.
The encoded on-chain gross-profit floor includes the policy's net floor plus
the entire conservative loan-token fee estimate.

`cost.native_wei` must cover `gas_limit * max_fee_per_gas` plus explicit extra
native fees and stay within the policy ceiling. `extra_native_wei: null` is
unknown and is refused. Its conversion to `loan_units` is trusted host evidence;
the VM does not prove an exchange rate or Base L1/operator fee estimate. DEX
fees and impact are reflected in the full transaction's measured proceeds.
Simulation gas at the pinned state can differ from mined gas in the next block
(for example, Morpho accrual writes depend on elapsed time). The fee guard uses
the full signed gas limit and fee cap; it does not price execution from trace
gas alone. The local proof records both measurements separately.

Before signing, the native host checks chain, canonical snapshot hash, pending
nonce, executor code and owner/keeper/Morpho/router configuration. It executes
the exact calldata with `eth_call` and `debug_traceCall` using `callTracer`, then
checks the genuine Morpho call's seized/repaid return values and the executor's
profit. A missing trace or changed head is a refusal. This requires an own node
or provider exposing debug RPC; Base's best-effort public endpoints omit it.

Only a standalone native instruction followed by `HALT` is allowed: a VM cannot
send and subsequently claim it rolled back the transaction. Financial atomicity
comes from the single EVM executor transaction. The kill switch is checked
before signing and posting. Once a POST is attempted, the host reserves its
nonce even on a timeout or mismatched response. No automatic resend occurs.
`Submitted` means a matching hash was acknowledged, not mined or finalized.

The Rust host, provider evidence and process remain trusted. A process controlling
the key can bypass this VM. This feature supplies no OS isolation, on-chain
mandate enforcement, cryptographic proof of RPC honesty, or durable production
nonce journal. Policy expiry is checked before submission; the current executor
has no inclusion deadline and a submitted transaction can outlive that check.
Repay bounds are checked against the pinned simulation. The existing seize-asset
executor has no on-chain maximum-repay field, so changed state before inclusion
can change repayment. It enforces the encoded gross-profit floor on chain.

## Run an unsigned preflight

From `x3-lang/`:

```bash
cargo run -p x3-tools --bin x3c -- build examples/native_liquidation.x3 --out /tmp/monkey.x3b
export X3_LIQUIDATION_RPC_URL='http://127.0.0.1:8545'
cargo run -p x3-tools --bin x3_liquidation -- \
  --artifact /tmp/monkey.x3b \
  --approved-artifact-hash 0xYOUR_INDEPENDENTLY_APPROVED_KECCAK_DIGEST \
  --batch /path/to/batch.json
```

The example contains placeholder identities, not a deployed configuration.
The digest is mandatory and must come from reviewing the compiled artifact.
The CLI has no key or broadcast option. It emits the exact authorization and
unsigned request hash after native preflight, or exits nonzero. Provider URLs
are read from the environment and are not included in its receipt or errors.
`Batch`, `Candidate`, `Snapshot` and `ExecutionCost` in `vm/src/liquidation.rs`
define the JSON input schema. Use exact JSON integer values for `u128` fields;
avoid passing these through JavaScript `Number` when generating a batch.

## Reproduce the local proof

The fixture under `x3-lang/vm/tests/fixtures/native-liquidation/` pins genuine
Morpho Blue and the public Profit Monkey executor. It deploys test ERC-20s, a
test oracle and a mock exchange on an owned loopback Anvil chain. It proves
callback repayment and all-or-nothing rollback with an actual EVM; it does not
demonstrate live DEX liquidity or mainnet profitability.

From the repository root:

```bash
cd x3-lang/vm/tests/fixtures/native-liquidation
npm ci
npm run build
npm test
cd ../../../..
cargo test -p x3-lang-compiler --test native_liquidation --locked
cargo test -p x3-lang-vm --test native_liquidation --locked
cargo test -p x3-lang-vm --test native_evm_transport --locked
cargo test -p x3-lang-vm --test native_evm_anvil --locked -- --ignored --nocapture --test-threads=1
```

The native local broadcast mode requires loopback read/submit endpoints,
chain ID 31337 and an ephemeral in-memory signer. It cannot broadcast to public
chains. Public-chain operation is unsigned preflight only until independent
fee evidence, durable nonce recovery and explicit activation are implemented.
The scanner/node state feed is supplied through `LiquidationHost`; this change
does not provide a reth ExEx or a continuously running oracle watcher.

The dependency audit still reports `RUSTSEC-2026-0387` for an optional
`crypto-bigint` entry in Cargo.lock. `cargo tree -i crypto-bigint --locked`
reports no active dependency path in this workspace configuration. The local
signer uses libsecp256k1 with an explicitly supplied in-memory host key and no
file/string key-loading API; the previous default
RustCrypto signer dependency was removed. This is not a clean lockfile audit.
The audit JSON also retains informational warnings for review. The URL parser
was updated to 2.5.9 for `RUSTSEC-2026-0592`.

The optional VM benchmark excludes network, simulation and signing:

```bash
cargo test -p x3-lang-vm --test native_liquidation \
  benchmark_compiled_vm_selection_64_candidates -- --ignored --nocapture
```

## Current sequencing assumptions

As checked on November 28, 2026, Base's Denim upgrade replaced Flashblocks with
complete canonical 200 ms blocks on November 5; the old streams closed on
November 19. The builder snapshots eligible transactions at block start and
orders that pool by priority fee. A reaction to a published block cannot enter
that already built block. Arbitrum One replaced Timeboost with priority gas
auctions on September 23. These rules inform experiments; native Rust or X3
execution alone proves no deadline, inclusion advantage, win rate or profit.

Sources: [Base migration](https://docs.base.org/upgrades/denim/migrate-from-flashblocks),
[Base ordering](https://docs.base.org/specifications/transactions/transaction-ordering),
[Base RPC](https://docs.base.org/base-chain/api-reference/rpc-overview),
[Arbitrum PGA notice](https://developer.arbitrum.io/how-arbitrum-works/priority-gas-auction).
