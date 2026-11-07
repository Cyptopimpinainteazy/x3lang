#!/usr/bin/env bash
# EXACT compiled production WASM in an ephemeral, local NEAR Sandbox node.
# No public Testnet/Mainnet RPC use, external wallet keys, or real funds.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
MANIFEST="$ROOT/x3-lang/near/x3-guard/Cargo.toml"
TEST_MANIFEST="$ROOT/x3-lang/near/guard-sandbox-tests/Cargo.toml"
WASM="$ROOT/x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm"
HARNESS="$ROOT/x3-lang/near/guard-sandbox-tests/target/debug/x3-guard-near-sandbox-tests"
FIXTURE="$ROOT/x3-lang/tests/fixtures/x3_guard_args.json"
if [[ -n "${NEAR_NETWORK_ID:-}" ]] && [[ "${NEAR_NETWORK_ID}" != "sandbox" ]]; then
  echo 'Refusing non-sandbox network configuration' >&2
  exit 2
fi
cargo build --locked --release --target wasm32-unknown-unknown --manifest-path "$MANIFEST"
cargo build --locked --manifest-path "$TEST_MANIFEST"
sha256sum "$WASM"
# Ubuntu 22.04's glibc 2.35 cannot execute recent prebuilt NEAR Sandbox, which
# needs glibc >=2.39. Use only cached Ubuntu 24.04 Docker image and an
# explicitly DISCONNECTED network to avoid any public blockchain interaction.
LIBC_VERSION="$(getconf GNU_LIBC_VERSION 2>/dev/null || true)"
LIBC_MAJOR_MINOR="${LIBC_VERSION##* }"
if [[ "${LIBC_MAJOR_MINOR%.*}" == '2' && "${LIBC_MAJOR_MINOR#*.}" =~ ^[0-9]+$ && "${LIBC_MAJOR_MINOR#*.}" -lt 39 ]]; then
  if ! command -v docker >/dev/null || ! docker image inspect ubuntu:24.04 >/dev/null 2>&1; then
    echo 'Host GLIBC is too old for recent NEAR Sandbox. Install/copy a trusted Ubuntu 24.04 Docker image or run on a newer Linux machine.' >&2
    exit 2
  fi
  echo 'Starting OFFLINE Ubuntu 24.04 container for local NEAR Sandbox (no host GLIBC upgrade)' >&2
  docker run --rm --network none --user "$(id -u):$(id -g)" \
    --tmpfs /tmp:rw,nosuid,size=512m \
    -e HOME=/tmp -v "$ROOT:$ROOT:ro" -w "$ROOT" \
    ubuntu:24.04 "$HARNESS" "$WASM" "$FIXTURE"
else
  "$HARNESS" "$WASM" "$FIXTURE"
fi
