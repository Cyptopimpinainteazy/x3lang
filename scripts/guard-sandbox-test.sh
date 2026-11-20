#!/usr/bin/env bash
# Local NEAR Sandbox integration test for the deployed x3-guard WASM.
# No public testnet/mainnet, no wallet keys, no funds. Sandbox hashes are local.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
(
  cd "$ROOT/x3-lang/near/x3-guard"
  cargo build --locked --release --target wasm32-unknown-unknown
)
cd "$ROOT/x3-lang/near/x3-guard/sandbox-test"
exec cargo test -- --nocapture
