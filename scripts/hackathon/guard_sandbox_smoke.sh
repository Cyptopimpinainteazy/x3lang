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
# Build from the Guard directory so its rust-toolchain.toml selects stable.
# Running cargo from the repository root would use the host default toolchain
# (1.90 here), which is too old for near-sdk 5.30.
(cd "$ROOT/x3-lang/near/x3-guard" && cargo build --locked --release --target wasm32-unknown-unknown --manifest-path "$MANIFEST")
cargo build --locked --manifest-path "$TEST_MANIFEST"
sha256sum "$WASM"
# The sandbox must exercise an on-chain-mode build: refusal reasons need env.panic_utf8.
python3 "$ROOT/scripts/hackathon/check_guard_wasm_host_panics.py" --expect present "$WASM"
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
  # `near-sandbox` puts its default download directory in Cargo OUT_DIR,
  # which lives under the read-only source mount. Feed it a reviewed cached
  # local binary through the library's supported override instead.
  CACHED_BIN="${X3_SANDBOX_BIN_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/x3lang/near-sandbox-2.14.0/near-sandbox}"
  PINNED_SHA256='797b9d7a9a23907048d68a427e86a55c94f1de232dbfb8b8a1eec09ecef3bf13'
  if [[ ! -f "$CACHED_BIN" || ! -x "$CACHED_BIN" ]]; then
    echo "Pinned NEAR Sandbox 2.14.0 binary absent: $CACHED_BIN; provision it from a trusted release before running the offline container." >&2
    exit 2
  fi
  if [[ "$(sha256sum "$CACHED_BIN" | awk '{print $1}')" != "$PINNED_SHA256" ]]; then
    echo 'Refusing NEAR Sandbox cache whose SHA256 differs from the audited x3star1 binary' >&2
    exit 2
  fi
  echo 'Starting OFFLINE Ubuntu 24.04 container with pinned cached NEAR Sandbox binary (no host GLIBC upgrade)' >&2
  docker run --rm --network none --user "$(id -u):$(id -g)" \
    --tmpfs /tmp:rw,nosuid,size=512m \
    -e HOME=/tmp -e NEAR_SANDBOX_BIN_PATH=/opt/x3-near-sandbox \
    -v "$ROOT:$ROOT:ro" -v "$CACHED_BIN:/opt/x3-near-sandbox:ro" -w "$ROOT" \
    ubuntu:24.04 "$HARNESS" "$WASM" "$FIXTURE"
else
  "$HARNESS" "$WASM" "$FIXTURE"
fi
