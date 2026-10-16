# X3-lang Product & Technical Specification

**Status: 🚧 ACTIVE DEVELOPMENT — Per-feature readiness derived from `FEATURE_REGISTRY.toml`**

> Readiness scores are the canonical source. Run `scripts/check-readiness-consistency.sh` to validate.
> See `CURRENT_MAINNET_STATUS.md` for the full system scoreboard.

## Quick Navigation

- **Unique Capabilities**: [x3-unique-capabilities.md](x3-unique-capabilities.md)
- **Profitable Primitives**: [x3-profit-primitives.md](x3-profit-primitives.md)

## What is X3-lang?

X3-lang is a deterministic finance language for the X3 ecosystem. Today it compiles to its own intent bytecode and runs in `x3-lang/vm` off chain; targeting X3BC (the chain's bytecode, executed by `crates/x3-vm`) is the next step. EVM/SVM calls from X3 programs are not implemented yet.

## Instruction sets — what exists

This section used to list opcodes (`CONST_U128`, `HOST_CALL_EVM`, `GPU_BATCH_SIM`, ...) as "ALL
EXECUTING". None of those names exist in either instruction set in this repository.

- **x3-lang intent bytecode:** [`opcodes.yaml`](opcodes.yaml) is the source of truth (84 entries).
  It executes in `x3-lang/vm`, which is not on chain.
- **X3BC, the chain's bytecode:** `crates/x3-backend/src/opcode.rs` (114 opcodes). Which of them the
  chain's engine runs is `x3_vm::vm::engine_executes`, an exhaustive table checked against the
  interpreter by a test; the on-chain verifier refuses the rest at intake.

## Compiler Pipeline

```
X3 Source → Lexer → Parser → AST → Type Checker → HIR → LIR → Bytecode Emitter → Bytecode Verifier → X3VM
```

## Trading Core v1

Trading Core v1 adds `asset`, `risk policy`, and `atomic trade` declarations to
the Rust compiler pipeline:

```
asset/risk policy/atomic trade → trading semantic types → debt/risk verifier
→ trading IR → bytecode → capability-controlled atomic execution → receipt
```

Guarantees:

- assets are distinct by VM family, chain, canonical identifier, and decimals;
- every successful borrowed-capital path repays its linear debt exactly once;
- minimum net profit and all-debts guards are required before commit;
- host output below `min_out`, fee-ceiling violations, deadline expiry,
  state-commitment mismatches, and low net profit roll back atomically;
- success and failure receipts are canonical, hash-verified, and tamper-evident.

Boundary: v1 provides fixture-host execution only. It does not ship live venue
adapters, route discovery, oracle consensus, or mainnet execution evidence.

### `atomic swap` and minimum output

`atomic swap` declares an escrow/payout, not a venue `swap`, so it deliberately has no
`min_output` clause. A program that states `min_output` inside an `atomic swap` is refused
by name and directed to put the floor on a route `swap` instead. The refusal records the
language decision rather than silently dropping a price floor (TICKET-122).

## Production Readiness

> **Note**: All 26 opcodes execute in the VM. ON_FAIL now wires real failure-handler dispatch — a trapped opcode (REQUIRE, OOG, timeout, etc.) transfers control to the most recent handler target instead of immediately returning `Err`. The atomic_router feature is at **85%** readiness per `FEATURE_REGISTRY.toml`. Remaining gaps are production hardening — multi-validator testing, external bridge integration, and CI gate wiring — not missing VM functionality.

- **26 opcodes**: all executing in VM
- **10+ E2E control-flow tests**: IF, LOOP, REQUIRE, ON_FAIL, ON_TIMEOUT, ATOMIC_BEGIN/END/ROLLBACK
- **Bridge backend**: 4 verifier families (evm-light-client, svm-light-client, evm-rpc, svm-rpc)
- **CI gates**: 22 gates all passing
- **Registry**: 23 features tracked in `FEATURE_REGISTRY.toml` (average readiness ~36%)

**X3-lang status: All opcodes execute. Production hardening in progress. 🚧**

*Last updated: June 2026*
