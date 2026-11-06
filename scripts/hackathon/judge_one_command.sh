#!/usr/bin/env bash
# X3 Shield judge preflight: local tests only. NEVER signs, deploys or sends funds.
set -euo pipefail
umask 077
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPORT_DIR="${1:-/tmp/x3-shield-judge}"
cd "$ROOT"

for dependency in node cargo; do
  if ! command -v "$dependency" >/dev/null 2>&1; then
    echo "Missing $dependency: install the development prerequisite, then rerun." >&2
    exit 2
  fi
done

if [[ -n "${PYTHON:-}" ]]; then
  PYTHON_BIN="$PYTHON"
else
  PYTHON_BIN=""
  for candidate in python3.14 python3.13 python3.12 python3.11 python3; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
      PYTHON_BIN="$(command -v "$candidate")"
      break
    fi
  done
  if [[ -z "$PYTHON_BIN" ]]; then
    echo 'Python 3.11+ is required by NEAR evidence parsing; install Python 3.11+ (Python 3.10 cannot run this judge gate).' >&2
    exit 2
  fi
fi

if ! "$PYTHON_BIN" -c 'import sys; import pytest, cryptography; sys.exit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
  if [[ -n "${PYTHON:-}" ]]; then
    echo "Specified PYTHON=$PYTHON_BIN lacks Python 3.11+, pytest or cryptography. Install requirements and pytest==9.2.0 into that environment." >&2
    exit 2
  fi
  VENV_DIR="${X3_JUDGE_VENV:-${XDG_CACHE_HOME:-$HOME/.cache}/x3lang/judge-quick-venv}"
  if [[ -x "$VENV_DIR/bin/python" ]] && ! "$VENV_DIR/bin/python" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
    echo 'Existing judge environment is too old; set X3_JUDGE_VENV to a different directory.' >&2
    exit 2
  fi
  if [[ ! -x "$VENV_DIR/bin/python" ]] || ! "$VENV_DIR/bin/python" -c 'import pytest, cryptography' >/dev/null 2>&1; then
    echo "Provisioning isolated judge environment at $VENV_DIR (package downloads may require internet)." >&2
    "$PYTHON_BIN" -m venv "$VENV_DIR"
    "$VENV_DIR/bin/python" -m pip install --disable-pip-version-check -r x3-lang/requirements.txt 'pytest==9.2.0'
  fi
  PYTHON_BIN="$VENV_DIR/bin/python"
fi

"$PYTHON_BIN" scripts/judge_evidence.py --mode quick --report-dir "$REPORT_DIR"
