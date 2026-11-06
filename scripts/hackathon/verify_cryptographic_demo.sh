#!/usr/bin/env bash
# One command: real Rust x3c -> dry-run VM -> archived genuine NEAR evidence.
# Package downloads may occur for first-time setup only; NO chain RPC, signing,
# wallet access, contract deployment or funds movement.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPORT_DIR="${1:-/tmp/x3lang-judge-crypto-demo}"
PYTHON_BIN="${PYTHON:-python3}"
cd "$ROOT"
if ! "$PYTHON_BIN" -c 'import cryptography' >/dev/null 2>&1; then
  if [[ -n "${PYTHON:-}" ]]; then
    echo "Selected PYTHON=$PYTHON_BIN lacks required cryptography; install x3-lang/requirements.txt into it" >&2
    exit 1
  fi
  VENV_DIR="${X3_DEMO_VENV:-${XDG_CACHE_HOME:-$HOME/.cache}/x3lang/judge-crypto-venv}"
  if ! "$VENV_DIR/bin/python" -c 'import cryptography' >/dev/null 2>&1; then
    echo "Provisioning isolated Python dependencies in $VENV_DIR (package downloads may require internet)" >&2
    python3 -m venv "$VENV_DIR"
    "$VENV_DIR/bin/python" -m pip install --disable-pip-version-check -r x3-lang/requirements.txt
  fi
  PYTHON_BIN="$VENV_DIR/bin/python"
fi
if ! cargo build --locked --offline --manifest-path x3-lang/Cargo.toml -p x3-tools --bin x3c --quiet; then
  echo 'Cargo cache not ready; fetching lockfile-pinned Rust packages (may require internet)' >&2
  cargo fetch --locked --manifest-path x3-lang/Cargo.toml
  cargo build --locked --offline --manifest-path x3-lang/Cargo.toml -p x3-tools --bin x3c --quiet
fi
"$PYTHON_BIN" scripts/hackathon/cryptographic_demo.py --x3c "$ROOT/x3-lang/target/debug/x3c" --output-dir "$REPORT_DIR"
