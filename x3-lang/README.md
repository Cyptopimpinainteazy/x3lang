# x3-lang v1.0 — Production Language & VM Workspace

**Status: 🚧 ACTIVE DEVELOPMENT — Per-feature readiness below (derived from `FEATURE_REGISTRY.toml`)**

> Readiness scores are the canonical source. Run `scripts/check-readiness-consistency.sh` to validate.
> See `CURRENT_MAINNET_STATUS.md` for the full system scoreboard.

This folder contains the Python MVP surface and the production Rust language workspace.

## Source-of-truth rules

- Python files: authoritative for the shipping intent DSL and MVP pipeline.
- `x3-lang/Cargo.toml`, `compiler/`, `vm/`, `crates/x3-ast`, `crates/x3-common`: authoritative for Rust compiler, IR, opcode, and VM bytecode semantics.

## Python MVP files
- `cli.py`, `registry.py`, `typechecker.py`, `planner.py`, `simulator.py`, `emitter/`, `schema.json`, `runner.py`, `tests/`, `examples/arb_solana_eth.x3`

## Rust workspace files
- `Cargo.toml`, `compiler/`, `vm/`, `spec/`, `crates/x3-ast`, `crates/x3-common`, `crates/x3-lexer`, `crates/x3-tools`

## VM Opcode Surface — Per-Feature Readiness

The atomic_router feature owns VM control-flow execution. Its readiness score is **85%** (derived from `FEATURE_REGISTRY.toml`).

| Opcode Group | Status | Detail |
|---|---|---|
| Arithmetic (ADD, SUB, POW) | ✅ EXECUTING | Register-to-register with gas metering |
| Memory (LOAD, STORE) | ✅ EXECUTING | 16-byte aligned loads/stores |
| Asset ops (LOCK, MINT, BURN, RELEASE, SWAP) | ✅ EXECUTING | Compiler-stream payload required |
| BRIDGE transfers | ✅ EXECUTING | Cross-chain transfer with proof verification |
| EMIT (EVM call) | ✅ EXECUTING | Via bridge adapter |
| CALL_HOST (SVM call) | ✅ EXECUTING | Via bridge adapter |
| Capability dispatch (GPU/SIMULATE/etc.) | ✅ EXECUTING | 13+ host capabilities |
| CALL / RET | ✅ EXECUTING | Call stack with return address |
| NOP, HALT | ✅ EXECUTING | |
| IF, LOOP | ✅ EXECUTING | Register-conditioned skip/jump with bounded loops |
| REQUIRE | ✅ EXECUTING | Panics on zero condition, caught by ON_FAIL handler |
| ON_FAIL, ON_TIMEOUT | ✅ EXECUTING | Handler push → dispatch on trap, + deadline enforcement |
| ATOMIC_BEGIN, ATOMIC_END, ATOMIC_ROLLBACK | ✅ EXECUTING | Snapshot/commit/rollback with full state restore |

> **Note**: All opcodes execute in the VM. ON_FAIL now wires real failure-handler dispatch — a trapped opcode transfers control to the most recent handler target instead of immediately returning `Err`. The 85% readiness score reflects remaining production hardening: multi-validator atomic execution tests, external bridge integration, and CI gate wiring — not missing VM functionality.

## Bridge Production Backend ✅

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

**x3-lang status: All 26 opcodes execute. 10+ E2E control-flow tests pass. Bridge backend wired. Ready for production hardening. 🚧**
