#!/usr/bin/env bash
# Offline, isolated real-NEAR-node persistence test. No existing service touched.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
[[ "${NEAR_NETWORK_ID:-sandbox}" == "sandbox" ]] || { echo 'Non-sandbox network refused' >&2; exit 2; }
[[ "$(id -u)" -ne 0 ]] || { echo 'Run unprivileged' >&2; exit 2; }
command -v docker >/dev/null
docker image inspect ubuntu:24.04 >/dev/null
free_kib="$(df -Pk "$ROOT" | awk 'NR==2{print $4}')"
min_gib="${X3_RESTART_MIN_FREE_GIB:-25}"
[[ "$min_gib" =~ ^[0-9]+$ ]] && (( min_gib >= 8 && min_gib <= 100 )) || {
  echo 'Invalid test disk floor: must be 8..100 GiB' >&2
  exit 2
}
min_kib="$((min_gib * 1024 * 1024))"
if (( free_kib < min_kib )); then
  echo "FAIL CLOSED: free space under ${min_gib}GiB: ${free_kib} KiB" >&2
  exit 2
fi
CACHED_BIN="${X3_SANDBOX_BIN_CACHE:-${HOME}/.cache/x3lang/near-sandbox-2.14.0/near-sandbox}"
PINNED_SHA256="797b9d7a9a23907048d68a427e86a55c94f1de232dbfb8b8a1eec09ecef3bf13"
[[ -x "$CACHED_BIN" && "$(sha256sum "$CACHED_BIN" | awk '{print $1}')" == "$PINNED_SHA256" ]] || {
  echo 'Pinned NEAR Sandbox 2.14.0 binary missing/mismatched' >&2; exit 2;
}
TEST_MANIFEST="$ROOT/x3-lang/near/guard-sandbox-tests/Cargo.toml"
export CARGO_TARGET_DIR="${X3_RESTART_CARGO_TARGET:-$ROOT/x3-lang/near/guard-sandbox-tests/target}"
cargo build --offline --locked --manifest-path "$TEST_MANIFEST" --bin persisted_restart
HARNESS="$CARGO_TARGET_DIR/debug/persisted_restart"
[[ -x "$HARNESS" ]] || exit 2
WASM="${X3_RESTART_GUARD_WASM:-$ROOT/x3-lang/near/x3-guard/target/wasm32-unknown-unknown/release/x3_guard.wasm}"
if [[ ! -f "$WASM" ]]; then
  echo 'Compiled production Guard WASM absent; build the exact branch Guard before running' >&2
  exit 2
fi
WASM_HASH="$(sha256sum "$WASM" | awk '{print $1}')"
if [[ -n "${X3_RESTART_GUARD_SHA256:-}" && "$WASM_HASH" != "$X3_RESTART_GUARD_SHA256" ]]; then
  echo 'Compiled Guard WASM hash differs from approved local artifact' >&2; exit 2
fi
RUN_ROOT="$(mktemp -d /tmp/x3-guard-node-restart.XXXXXXXX)"
cleanup() {
  code="$?"
  if [[ "$RUN_ROOT" == /tmp/x3-guard-node-restart.* && -d "$RUN_ROOT" ]]; then
    if [[ "$code" -ne 0 ]]; then
      for f in "$RUN_ROOT"/*.log; do [[ -f "$f" ]] && tail -15 "$f" >&2 || :; done
    fi
    rm -rf -- "$RUN_ROOT"
  fi
}
trap cleanup EXIT
echo "X3_RESTART_TEST_PRECHECK pinned_neard_sha256=$PINNED_SHA256 guard_wasm_sha256=$WASM_HASH offline=true"
docker run --rm --network none --read-only --user "$(id -u):$(id -g)" \
  --memory 6g --cpus 2 --pids-limit 192 \
  --tmpfs /tmp:rw,nosuid,size=128m \
  -e HOME=/tmp -e NEAR_SANDBOX_BIN_PATH=/opt/x3-near-sandbox \
  -v "$CACHED_BIN:/opt/x3-near-sandbox:ro" \
  -v "$HARNESS:/opt/x3-restart-harness:ro" \
  -v "$WASM:/opt/x3-guard.wasm:ro" \
  -v "$RUN_ROOT:/x3-persist:rw" \
  ubuntu:24.04 /opt/x3-restart-harness /opt/x3-guard.wasm /x3-persist/guard-node-home
