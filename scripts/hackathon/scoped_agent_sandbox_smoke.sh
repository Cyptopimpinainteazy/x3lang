#!/usr/bin/env bash
# Real, local NEAR Sandbox: prove a disposable function-call-only key cannot
# bypass NEAR access-key permissions using raw transactions. No public chain.
set -euo pipefail
umask 077
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
GUARD_MANIFEST="$ROOT/x3-lang/near/x3-guard/Cargo.toml"
HARNESS_MANIFEST="$ROOT/x3-lang/near/guard-sandbox-tests/Cargo.toml"
GUARD_WASM="$ROOT/x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm"
HARNESS="$ROOT/x3-lang/near/guard-sandbox-tests/target/debug/scoped_agent_boundary"
if [[ -n "${NEAR_NETWORK_ID:-}" && "${NEAR_NETWORK_ID}" != sandbox ]]; then
  echo "Refusing non-sandbox NEAR network configuration" >&2
  exit 2
fi
cargo build --locked --release --target wasm32-unknown-unknown --manifest-path "$GUARD_MANIFEST"
cargo build --locked --manifest-path "$HARNESS_MANIFEST" --bin scoped_agent_boundary
test -f "$GUARD_WASM" && test -x "$HARNESS"
echo "X3_SCOPED_BOUNDARY_WASM_SHA256 $(sha256sum "$GUARD_WASM" | cut -d' ' -f1)"
LIBC_VERSION="$(getconf GNU_LIBC_VERSION 2>/dev/null || true)"
LIBC_MINOR="${LIBC_VERSION##*.}"
if [[ "$LIBC_VERSION" == glibc\ 2.* && "$LIBC_MINOR" =~ ^[0-9]+$ && "$LIBC_MINOR" -lt 39 ]]; then
  command -v docker >/dev/null || { echo "Docker required on glibc under 2.39" >&2; exit 2; }
  docker image inspect ubuntu:24.04 >/dev/null || { echo "Cached Ubuntu 24.04 image missing" >&2; exit 2; }
  CACHED_BIN="${X3_SANDBOX_BIN_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/x3lang/near-sandbox-2.14.0/near-sandbox}"
  PINNED_SHA256=797b9d7a9a23907048d68a427e86a55c94f1de232dbfb8b8a1eec09ecef3bf13
  [[ -f "$CACHED_BIN" && -x "$CACHED_BIN" ]] || { echo "Pinned sandbox binary missing" >&2; exit 2; }
  [[ "$(sha256sum "$CACHED_BIN" | cut -d' ' -f1)" == "$PINNED_SHA256" ]] || { echo "Sandbox executable hash mismatch" >&2; exit 2; }
  docker run --rm --network none --user "$(id -u):$(id -g)" \
    --tmpfs /tmp:rw,nosuid,size=512m \
    -e HOME=/tmp -e NEAR_SANDBOX_BIN_PATH=/opt/x3-near-sandbox \
    -v "$ROOT:$ROOT:ro" -v "$CACHED_BIN:/opt/x3-near-sandbox:ro" -w "$ROOT" \
    ubuntu:24.04 "$HARNESS" "$GUARD_WASM"
else
  "$HARNESS" "$GUARD_WASM"
fi
