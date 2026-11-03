# X3Lang

**Programmable financial policies and deterministic transaction validation for X3 and cross-chain applications.**

> **Development software.** No claim of production custody, deployed liquidity pools, atomic cross-chain borrowing, fully verified solver settlement, or MEV immunity. Do not send funds based only on local tests, unsigned provider status, or a displayed balance change.

This standalone repository contains the X3Lang **source extracted from the existing [X3 monorepo](https://github.com/Cyptopimpinainteazy/xxxstar)**, not a newly written language. The migration is preserved as a **draft PR for review**, and its original development history remains in the source monorepo. See [SOURCE_PROVENANCE.md](SOURCE_PROVENANCE.md).

## Layout

- [`x3-lang/`](x3-lang/) — Python intent DSL, Rust compiler and VM, tests, NEAR Intents adapter and Guard/browser integration.
- [`crates/x3bc-core/`](crates/x3bc-core/) — shared canonical X3BC bytecode definitions required by the compiler.
- [`x3-lang/vm/src/liquidity.rs`](x3-lang/vm/src/liquidity.rs) — deterministic LP share and flash-loan **shadow accounting**, without token custody.
- [`x3-lang/vm/src/agent_firewall.rs`](x3-lang/vm/src/agent_firewall.rs) — Ed25519-signed local authorization preflight; **not** a wallet or persistent spending authority.
- [`docs/hackathons/`](docs/hackathons/) — risks and integration gates for X3 Shield.
- [`docs/x3-lang/fixtures/`](docs/x3-lang/fixtures/) and [`scripts/proof/`](scripts/proof/) — archived verification fixtures and their generator, required for reproducible VM tests.

The `x3-lang/` subdirectory is intentional: the source/test paths and Cargo path dependencies originally targeted this structure. Changing them without a verified migration could break bytecode and cross-VM tests.

## Verify locally

Requirements: Rust 1.90+, Python 3.11+ with `pytest`, and Node.js 22+ for the browser-demo tests.

```bash
python3 -m pip install -r x3-lang/requirements.txt pytest
PYTHONPATH=x3-lang python3 -m pytest x3-lang/tests -q
(cd x3-lang && cargo test -p x3-lang-vm --locked && cargo fmt --all -- --check && cargo clippy -p x3-lang-vm --all-targets --locked -- -D warnings)
(cd x3-lang/near/x3-guard && cargo test --locked)
node --test x3-lang/near/demo/test/*.test.mjs
```

The [X3 Guard public demo](https://x3-guard.pages.dev/) checks **real signed dry quotes** against source rules; it is not a verified live settlement. Independent cross-chain attribution remains an explicit release blocker.

## Scope and release policy

Use this repository for the shared X3Lang compiler, policy VM and risk primitives. A hackathon-specific submission should import or pin **only the necessary X3Lang components**, with prior work disclosed separately from hackathon-window changes. Changes to wallet custody, token transfers, external providers, or mainnet trading require independent review and verified execution evidence.
