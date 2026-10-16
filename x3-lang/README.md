# x3-lang — the X3 finance language

**Status: development. Not on chain yet.** Read this before the rest of the file.

## Where X3 code runs today (measured 2026-10-16)

There are two X3 instruction sets in this repository, and only one of them is on chain:

| | X3BC (chain bytecode) | x3-lang intent bytecode (this folder) |
|---|---|---|
| Source language | `fn` / `let` / loops / `atomic {}` (`crates/x3-parser`, `crates/x3-compiler`) | `atomic trade`, `risk policy`, `intent`, `asset` (`x3-lang/compiler`) |
| Instruction set | 114 opcodes, `crates/x3-backend/src/opcode.rs` | 84 opcodes, `x3-lang/spec/opcodes.yaml` |
| Engine | `crates/x3-vm` | `x3-lang/vm` |
| Runs on chain? | **Yes.** `pallet-x3-kernel` executes it through `X3Executor::execute_on_chain`, `x3-vm` in every build (#612) | **No.** No pallet loads it; it runs in this workspace's tests and `x3c` |

So a program written in x3-lang's trading syntax is compiled, verified, simulated and replayed here,
but it cannot be submitted to the chain. Compiling x3-lang to X3BC, with the trading, receipt and
oracle-firewall logic as host modules of the one engine, is the next unification step (PR 4 of the
X3 Lang mainnet roadmap).

## Source-of-truth rules

- Python files: authoritative for the shipping intent DSL and MVP pipeline.
- `x3-lang/Cargo.toml`, `compiler/`, `vm/`, `crates/x3-ast`, `crates/x3-common`: authoritative for Rust compiler, IR, opcode, and VM bytecode semantics.

## Python MVP files
- `cli.py`, `registry.py`, `typechecker.py`, `planner.py`, `simulator.py`, `emitter/`, `schema.json`, `runner.py`, `tests/`, `examples/arb_solana_eth.x3`

## Rust workspace files
- `Cargo.toml`, `compiler/`, `vm/`, `spec/`, `crates/x3-ast`, `crates/x3-common`, `crates/x3-lexer`, `crates/x3-tools`

## Trading Core v1

Trading Core v1 adds chain-qualified assets, linear debts, explicit swap
routes, risk-policy guards, debt-closure and net-profit verification,
capability-controlled atomic VM execution, and deterministic receipts.

Canonical example: `examples/trading_core_v1.x3`.

Supported today:

- parse, type-check, verify, lower, encode, decode, and disassemble trading declarations;
- checked decimal-to-base-unit accounting with explicit rounding controls;
- borrow/swap/repay/net-profit/all-debts/commit verification;
- fixture-host execution with atomic rollback and receipt verification;
- `x3c receipt inspect <receipt.json>` and `x3c receipt verify <receipt.json>`.

Explicit v1 boundaries:

- no live Aave, Uniswap, SushiSwap, bridge, or private-relay adapters;
- no route discovery or oracle aggregation;
- fixture capabilities are rejected in production/mainnet execution mode;
- no live-DEX or mainnet claim is made without real adapter evidence.

## x3-lang VM — what executes, and where

`x3-lang/vm` executes the intent instruction set below **in this workspace only** (tests, `x3c run`,
`x3c simulate`). None of it runs on a block.

| Opcode group | In `x3-lang/vm` |
|---|---|
| Arithmetic, memory, asset ops, control flow, guards, `ATOMIC_*`, capability dispatch | Executes |
| `BRIDGE` | Executes against the configured backend; the production backend fails closed without a live verifier |
| Trading core (`TRADING_*`) | Executes through `TradingHost`; **no live venue adapters** (see Trading Core v1) |
| Reserved `0x70`–`0x7B`, B-52 feature-lock payloads | Not dispatched |

What remains for mainnet is listed in the roadmap: one engine (x3-lang compiling to X3BC), live
adapters, multi-validator execution, an external audit.

## Bridge backend (x3-lang VM)

`init_production_backend()` in [`vm/src/bridge.rs`](vm/src/bridge.rs:2990) supports 4 verifier families:
- `evm-light-client` (EthereumLightClientVerifier)
- `svm-light-client` (SolanaLightClientVerifier)
- `evm-rpc` (EthereumRpcFinalityVerifier)
- `svm-rpc` (SolanaRpcFinalityVerifier)

All configured via environment variables. Never silently falls back to dry-run.

## Quick start

```bash
python3 x3-lang/cli.py x3-lang/examples/arb_solana_eth.x3
cargo test --manifest-path x3-lang/Cargo.toml
```

## Rust Production Surface

```bash
cargo run -p x3-tools --bin x3c -- parse/check/lower/build/simulate/run/explain <FILE.x3>
```

**x3-lang status: compiles, verifies and simulates trading programs in this workspace; not executable on chain until it targets X3BC.**
