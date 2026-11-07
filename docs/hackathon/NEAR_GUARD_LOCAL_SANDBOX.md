# X3 Guard: local real-WASM NEAR Sandbox integration smoke

**Scope:** entirely local NEAR Sandbox node, no public Testnet/Mainnet use, no real wallet keys, no funds. This is a higher-fidelity environment than `near-sdk` unit-test mocks, but *not* independently authenticated NEAR finality. A local sandbox transaction must never be described as a real on-chain Testnet deployment.

The official [NEAR Integration Testing guide](https://docs.near.org/smart-contracts/testing/integration-test) describes deploying production WASM to a local Sandbox node and checking actual transaction/receipt outcomes. This harness follows its `near-sandbox` + `near-api` Rust setup.

From the project root on Linux x86_64 with Rust >=1.85, `wasm32-unknown-unknown` target and network access to download pinned Rust crates/sandbox binaries on the first build:

```bash
bash scripts/hackathon/guard_sandbox_smoke.sh
```

On Ubuntu 22.04 (GLIBC 2.35), the downloaded NEAR Sandbox binary requires a newer libc. The wrapper *does not modify the OS*: when glibc is too old it uses **an already cached Ubuntu 24.04 Docker image** with `--network none`, a read-only repository bind mount and temporary in-memory `/tmp`. External networking is disabled for the entire sandbox run. The container also mounts a SHA-256-pinned, cached NEAR Sandbox 2.14.0 executable **read-only** via `NEAR_SANDBOX_BIN_PATH` rather than attempting downloads into the read-only Cargo build directory. The expected binary SHA-256 is `797b9d7a9a23907048d68a427e86a55c94f1de232dbfb8b8a1eec09ecef3bf13`; the default cache path is `~/.cache/x3lang/near-sandbox-2.14.0/near-sandbox` (override with `X3_SANDBOX_BIN_CACHE`). Missing/mismatched cache or Docker image fails closed, with no silent downloads inside the isolated container. Other sufficiently modern Linux systems run the harness natively.

What the program does:

1. Compiles `x3-guard` from its lockfile into the real release WASM and prints the local artifact hash.
2. Starts a disposable local NEAR Sandbox node and creates `guard.sandbox` with **synthetic local** 20 NEAR for storage; it does not query or fund any `.testnet`/`.near` address.
3. Deploys the **production** Guard WASM to that local account, invokes `new()`, then reads `get_manager_public_key` and `state_version` through sandbox RPC.
4. Requires the immutable official manager public key and schema version 2 to match the source; any mismatch fails the test.
5. Registers a policy on the local node, then checks **genuine archived 1Click manager-signed dry quote** bytes from `x3-lang/tests/fixtures/x3_guard_args.json` through the actual WASM. Asserts `quote_signature=true`, `quote_kind=false`; the state-changing `accept_quote` transaction fails, and a subsequent state read confirms `quote=null`.
6. Drops the local node when done. The temporary sandbox bootstrap signer is public test tooling, **never** a user's wallet or externally funded key.

**Important:** The quoted refusal is because a signed **dry** quote has no executable deposit authorization (and may also be expired), **not** a successful rejection of a genuine live manager-signed forbidden recipient. This integration smoke does not submit a validly signed prohibited 1Click **live** quote. The genuine archived official 1Click signed fixtures in the repository are *dry* quotes (not live deposit instructions); synthetic test signatures cannot be accepted under the actual production-pinned key on this production WASM. This distinction is exactly why the sandbox deployment smoke is separate from the existing [local mock-chain quote policy refusal test](X3_SHIELD_SIGNED_QUOTE_REFUSAL_DEMO.md).

The [dedicated exact-head CI workflow](../../.github/workflows/x3guard-real-wasm-sandbox.yml) runs this same local-node integration on an Ubuntu 24.04 runner; it has no wallet secrets and uses only ephemeral sandbox funding.

The public Testnet requirement in Issue #73 remains blocked: this local node cannot create a public explorer link or validate the independent trust anchor, and synthetic sandbox funding does not prove control of either deployed Testnet account. Both `x3lang.testnet` and `x3lang-guard-20261106.testnet` now have deployed Guard-compatible code; the original 2-NEAR/no-code read-only preflight is historical and superseded.
