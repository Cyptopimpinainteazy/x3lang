#!/usr/bin/env bash
# Reproduce exactly the deployed Guard bytecode from review-branch source.
# No keys, wallet, testnet transaction, or external blockchain RPC calls.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

if [[ -n "${RUSTFLAGS:-}" || -n "${CARGO_ENCODED_RUSTFLAGS:-}" || -n "${RUSTC_WRAPPER:-}" ]]; then
  echo 'Refusing ambient RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS/RUSTC_WRAPPER: exact-byte build must control compiler flags' >&2
  exit 2
fi
version="$(rustc -vV | sed -n 's/^release: //p')"
commit="$(rustc -vV | sed -n 's/^commit-hash: //p')"
if [[ "$version" != '1.99.1' || "$commit" != '58cf8f9c35b355199727627d07ae34a4e2846d67' ]]; then
  echo "Pinned Rust 1.99.1 (58cf8f9c35...) required, got release=$version commit=$commit" >&2
  exit 2
fi

sysroot="$(rustc --print sysroot)"
export RUSTFLAGS="--remap-path-prefix=$sysroot/lib/rustlib/src/rust=/rustc/$commit"
export CARGO_TARGET_DIR="${X3_GUARD_REPRO_TARGET:-${XDG_CACHE_HOME:-${HOME:-/tmp}/.cache}/x3lang/guard-wasm-repro-1.99.1}"

cargo build --locked --release --target wasm32-unknown-unknown \
  --manifest-path "$ROOT/x3-lang/near/x3-guard/Cargo.toml"
python3 "$ROOT/scripts/hackathon/verify_guard_wasm_bytes.py" \
  --manifest "$ROOT/evidence/near-guard-testnet/manifest.json" \
  --wasm "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/x3_guard.wasm"
