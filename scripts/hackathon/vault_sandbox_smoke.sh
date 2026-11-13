#!/usr/bin/env bash
# Local real NEAR Sandbox only; no public testnet signer or funded wallet.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
test "${NEAR_NETWORK_ID:-sandbox}" = sandbox || { echo "sandbox only" >&2; exit 2; }
CONTRACT="$ROOT/x3-lang/near/x3-agent-vault"
HARNESS="$ROOT/x3-lang/near/agent-vault-sandbox-tests"
USE_CONTAINER=0
RUNTIME_IMAGE="${X3_SANDBOX_RUNTIME_IMAGE:-ubuntu:24.04}"
if [ "$(getconf GNU_LIBC_VERSION | cut -d. -f2)" -lt 39 ]; then
  USE_CONTAINER=1
  docker image inspect "$RUNTIME_IMAGE" >/dev/null 2>&1
  docker run --rm --network none --user "$(id -u):$(id -g)" --entrypoint python3 \
    "$RUNTIME_IMAGE" -I -c 'import sys; sys.exit(0 if sys.version_info >= (3, 12) else 2)' || {
      echo "offline Sandbox runtime needs Python 3.12+; pre-provision it and set X3_SANDBOX_RUNTIME_IMAGE" >&2
      exit 2
    }
else
  python3 -I -c 'import sys; sys.exit(0 if sys.version_info >= (3, 12) else 2)' || {
    echo "Sandbox raw-response verification requires Python 3.12+" >&2
    exit 2
  }
fi
(cd "$CONTRACT" && cargo build --locked --release --target wasm32-unknown-unknown)
(cd "$HARNESS" && cargo build --locked)
WASM="$CONTRACT/target/wasm32-unknown-unknown/release/x3_agent_vault_testnet.wasm"
EXEC="$HARNESS/target/debug/x3-agent-vault-sandbox-test"
sha256sum "$WASM"
PINNED="797b9d7a9a23907048d68a427e86a55c94f1de232dbfb8b8a1eec09ecef3bf13"
CACHED="${X3_SANDBOX_BIN_CACHE:-${HOME}/.cache/x3lang/near-sandbox-2.14.0/near-sandbox}"
test -x "$CACHED" && test "$(sha256sum "$CACHED" | awk '{print $1}')" = "$PINNED" || { echo "pinned offline sandbox missing" >&2; exit 2; }
if [ "$USE_CONTAINER" = 1 ]; then
  docker run --rm --network none --user "$(id -u):$(id -g)" --tmpfs /tmp:rw,nosuid,size=512m \
    -e HOME=/tmp -e NEAR_SANDBOX_BIN_PATH=/opt/x3-near-sandbox \
    -v "$ROOT:$ROOT:ro" -v "$CACHED:/opt/x3-near-sandbox:ro" -w "$ROOT" \
    --entrypoint "$EXEC" "$RUNTIME_IMAGE" "$WASM"
else
  NEAR_SANDBOX_BIN_PATH="$CACHED" "$EXEC" "$WASM"
fi
