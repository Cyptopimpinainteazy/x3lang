# Reproduce the X3 Guard deployed on NEAR Testnet — exact WASM bytes

**Security scope: byte-for-byte build integrity only.** This check cannot grant signing authority, authenticate an independent NEAR consensus checkpoint, prove settlement, or move funds. It never loads keys, deploys a contract, or submits a transaction.

## Reproducible deployed contract identity

The committed read-only [`manifest.json`](../../evidence/near-guard-testnet/manifest.json) and [testnet receipts](GUARD_TESTNET_EVIDENCE.md) identify:

| Fact | Pinned value |
| --- | --- |
| Contract account | `x3lang-guard-20261106.testnet` |
| Deployed WASM bytes | `544944` |
| Deployed SHA-256 | `ac44fa53acf514d26c7adf883b31b49a125faf5a4c1074928d3e1fe94b36f8ab` |
| NEAR on-chain code hash | `CbU8SpgfTgG1egbw8ygMTK3deozvDnoFrF8DJ8qifSe6` |
| Reviewed release source baseline | `800a077eec0b36bc66d7115995cb6903fa9d4f0a` |
| Rust compiler | `rustc 1.99.1`, commit `58cf8f9c35b355199727627d07ae34a4e2846d67` |
| Build command | `cargo build --locked --release --target wasm32-unknown-unknown` with canonical standard-library source-path mapping |

### One-command offline build + exact hash gate

From the release branch or a descendant that has **not** changed the Guard's runtime source, manifest, or locked dependencies:

```bash
bash scripts/hackathon/verify_guard_wasm_repro.sh
```

Requirements: Rust 1.99.1 with the `rust-src` component and `wasm32-unknown-unknown`, Python 3, locked Cargo dependencies (first-run downloading dependencies may use the internet). No blockchain RPC requests or wallet interactions occur during verification. Existing `RUSTFLAGS`, `CARGO_ENCODED_RUSTFLAGS` or `RUSTC_WRAPPER` cause a hard failure rather than an unreviewed compiler configuration. The script builds into a separate user-cache target directory and verifies both the manifest's record hash and `base58(SHA256(WASM))` against the pinned deployed identity. Missing dependencies/toolchain or changed code fail nonzero.

### Why a plain rebuild originally produced 696 additional bytes

On x3star1 (Rust 1.99.1), a direct `cargo build --locked --release --target wasm32-unknown-unknown` produced **545640 bytes**, SHA-256 `b2b3c397d088c5d813d9efea1cd18e6250baeb743ad701d5f7d48166f0b9fae3`, despite identical source-level exported ABI. Its compiled data referenced absolute local standard-library paths under `/home/lojak/.rustup/toolchains/.../lib/rustlib/src/rust/`; the deployed WASM used canonical `/rustc/58cf8f9c35.../` source locations. These source-path strings also shifted bytecode data addresses, altering the code-section digest.

The reproducible gate pins the same Rust version and normalizes the standard-library source prefix:

```bash
RUSTFLAGS="--remap-path-prefix=$(rustc --print sysroot)/lib/rustlib/src/rust=/rustc/58cf8f9c35b355199727627d07ae34a4e2846d67"
```

The build also embeds absolute Cargo registry source paths. The deployed WASM
used `/home/lojak/.cargo/registry/src/` and a fresh GitHub-hosted runner uses
`/home/runner/.cargo/registry/src/`. The difference adds **24 bytes** to the
compiled data and changes internal code offsets unless normalized. The gate
therefore applies a second path-only normalization:

```bash
--remap-path-prefix="${CARGO_HOME:-$HOME/.cargo}/registry/src=/home/lojak/.cargo/registry/src"
```

Both remappings refer only to compiler source-location strings. **No
byte mismatch is tolerated** after the build. The unchanged hard-pinned
manifest, SHA-256 and base58 code-hash gate must still pass. With canonical
paths, the real release-source rebuild on x3star1 reproduced the on-chain
SHA-256 exactly, `ac44fa53...f8ab`, at **544944 bytes**. No source code, contract state, code hash, deployed contract, or wallet was changed to obtain this result. The script makes this check mandatory rather than silently declaring all builds equivalent.

### Two distinct verification gates

1. **Build reproducibility (offline):** this script checks that the deployed on-chain binary **can be produced** by the reviewed release source with the documented toolchain and flags.
2. **Public-chain evidence (read-only):** `scripts/hackathon/testnet_guard_evidence.py --verify` independently re-queries receipts and account code identity against NEAR Testnet RPCs. On November 6, 2026, FastNEAR verified all **13 archived transactions**, and the deprecated `rpc.testnet.near.org` service verified the manifest but rate-limited/deprecated bulk historical verification. An RPC provider's agreement is not independently authenticated light-client finality.

The successful real on-chain Guard transactions include a **manager-signed dry forbidden quote refusal** and a separate **manager-signed live allowed quote acceptance**. No deposit or settlement was established. A **fresh valid live quote that violates an active mandate and is refused as a finalized on-chain `agent_accept_quote` transaction** is still a distinct unproven milestone (Issue #73).

Recheck the relevant contract's public code hash and chain ID immediately before any proposed testnet interaction. An exact binary match is a source-provenance argument, not an independent audit or authorization to sign or transmit transactions.
