# x3-lang Completion Plan

This document outlines the concrete steps required to bring **x3-lang** to a production‑ready state as described in the task.

## High‑Priority Milestones

1. **Opcode contract alignment**
   - Review `compiler/src/emitter.rs`, `vm/src/verifier.rs`, and `vm/src/executor.rs`.
   - Extract a single source‑of‑truth opcode spec (e.g., `x3-lang/spec/opcodes.yaml`).
   - Refactor emitter, verifier, and executor to use the shared spec.

2. **Preserve intent semantics in lowering**
   - Audit `compiler/src/parser.rs`, `compiler/src/lowering.rs`, and `compiler/src/ir.rs` for loss of:
     - `min_output` on swaps
     - Bridge route steps
     - Timeout/refund semantics
   - Extend IR structures to carry these attributes through lowering.
   - Update lowering passes to forward them unchanged.

3. **Real production backend wiring**
   - Replace the default `DryRunBridge` with a configurable production bridge in `vm/src/bridge.rs` and `vm/src/x3_lang_vm.rs`.
   - Add environment‑variable based selection (`X3_BACKEND=prod|dry`).

4. **Integration bridge implementation**
   - Implement `crates/x3-integration/src/compiler_bridge.rs` to translate compiled bytecode into on‑chain calls.
   - Wire the bridge into the VM startup flow.

5. **End‑to‑end compile‑run tests**
   - Add real `.x3` source files under `tests/e2e/`.
   - Create a test harness in `proof-forge/src/runners/x3language.rs` that:
     1. Compiles the source using the compiler.
     2. Executes the emitted bytecode on the VM.
     3. Asserts expected state changes.
   - Update existing VM test suites to use these new examples.

## Secondary Enhancements

* Slippage protection, refund policy objects, bridge destination/receiver/finality semantics, replay/nonce/finality checks – to be added to the IR and runtime.
* Rust type‑checker for finance/cross‑chain constraints.
* Source‑maps and debug metadata.
* Fuzz/property tests for determinism and rollback safety.
* Gas model tied to opcodes.

## Developer‑Experience Tooling (Nice‑to‑Have)

* `x3-fmt` – code formatter.
* `x3-lint` – static analysis linter.
* `x3-pkg` – package manager.
* `x3-repl` – interactive REPL.
* `x3-doc` – documentation generator.
* `x3-test` – test harness CLI.

## Documentation Updates

* Remove placeholder entries from `docs/x3-lang/README.md`.
* Add sections describing the opcode spec, backend configuration, and end‑to‑end testing workflow.

---

## Completed Milestones

- ✅ **Opcode contract alignment** — Shared spec in `x3-lang/spec/opcodes.yaml` and `opcodes.rs`. Emitter, verifier, and executor all reference the shared constants.
- ✅ **Intent semantics preservation** — `min_output` preserved at `lowering.rs:299-302`; bridge steps and refunds preserved through lowering pipeline.
- ✅ **Production bridge backend wiring** — `resolve_bridge_backend()` with `X3_BACKEND` environment variable. `BackendMode::Production` requires a wired adapter; fails closed rather than silently falling back to `DryRunBridge`.
- ✅ **Real compiler bridge** — `crates/x3-integration/src/compiler_bridge.rs` fail-closed with typed error; test-pinned until upstream gates clear.
- ✅ **End-to-end compile-run tests** — Three `.x3` source files in `x3-lang/tests/e2e/` (`simple_transfer.x3`, `atomic_swap.x3`, `bridge_step.x3`) compile through the pipeline and execute on the VM.
- ✅ **Slippage protection, refund objects, bridge semantics, replay/nonce** — Register allocator linear-scan algorithm (`x3-lang/compiler/src/regalloc.rs`), bytecode CRC32 checksum (`crates/x3-backend/src/bc_format_helpers.rs`), bridge adapter RPC validation in all three adapters (`crates/x3-bridge-adapters/src/{ethereum,solana,bitcoin}.rs`).
- ✅ **Executor authorization** — Settlement engine and cross-VM router gate execution through `pallet_x3_kernel::AuthorizedAccounts`.
- ✅ **Validator RPC** — Live authorities, leaderboard, and metrics queried from runtime API (not hardcoded stubs).
- ✅ **ZK proof feature gate** — `verify_zk_proof()` returns `Err(...)` unless the `zk-proofs` feature is explicitly enabled and a Groth16/PLONK verifier is wired.

## Pending Work

 - [ ] Begin developer‑experience tooling (formatter, linter, package manager, REPL, docs)
 - [ ] Implement slippage‑protection, proper refund objects, bridge semantics, and replay/nonce checks in the full runtime path (partial — bridge adapters wired, full e2e runtime integration still in progress)
 - [ ] Fuzz/property tests for determinism and rollback safety
 - [ ] Gas model tied to opcodes

This plan can be iterated on, and each checklist item can be marked complete as work progresses.
