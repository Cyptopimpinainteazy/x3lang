# X3-lang Product & Technical Specification

Status: active development. Use the [language README](../README.md) for supported
execution paths and limitations. Test results apply to the commit and command
that produced them.

## Quick Navigation

- [Capability proposals](x3-unique-capabilities.md)
- [Trading primitive proposals](x3-profit-primitives.md)

## What is X3-lang?

X3-lang is a deterministic finance language for the X3 ecosystem. Today it compiles to its own intent bytecode and runs in `x3-lang/vm` off chain; targeting X3BC (the chain's bytecode, executed by `crates/x3-vm`) is the next step. EVM/SVM calls from X3 programs are not implemented yet.

## Instruction sets — what exists

Opcode definitions and executor dispatch determine which instructions are supported.

- **x3-lang intent bytecode:** [`opcodes.yaml`](opcodes.yaml) is the source of truth (84 entries).
  It executes in `x3-lang/vm`, which is not on chain.
- **Canonical X3BC core:** [`crates/x3bc-core`](../../crates/x3bc-core/) defines
  bytecode headers, opcodes, verification and receipts shared with the parent chain.
  Chain runtime integration is maintained in `xxxstar`; these shared definitions
  alone do not establish on-chain execution.

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

`ON_FAIL` registers failure-handler targets in the local VM. The implementation
and regression cases are in [`vm/src/executor.rs`](../vm/src/executor.rs).
A local control-flow test does not establish live adapter execution or network readiness.

The [VM execution table](../README.md#x3-lang-vm--what-executes-and-where)
distinguishes dispatched instructions, reserved opcodes and adapter requirements.
Trading Core v1 uses fixture hosts; live venue integration and on-chain execution
need separate evidence.

For a CI or release claim, record the tested commit, commands, results and
workflow links. Track live adapter execution, deployment and independent review
separately from local VM tests.
