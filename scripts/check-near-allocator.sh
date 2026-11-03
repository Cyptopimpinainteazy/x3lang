#!/usr/bin/env bash
# Ensure NEAR Guard never depends on the abandoned wasm32 wee_alloc backend.
set -euo pipefail
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo/x3-lang/near/x3-guard"
if cargo tree --target wasm32-unknown-unknown --locked --prefix none | grep -Eq '^wee_alloc v'; then
  echo 'ERROR: NEAR Guard still includes unmaintained wee_alloc on wasm32' >&2
  exit 1
fi
if grep -Eq '^name = "wee_alloc"$' Cargo.lock; then
  echo 'ERROR: NEAR Guard Cargo.lock still resolves wee_alloc' >&2
  exit 1
fi
echo 'OK: no wee_alloc in wasm32 dependency graph or locked dependency set'
