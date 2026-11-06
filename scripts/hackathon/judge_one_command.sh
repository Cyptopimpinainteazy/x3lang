#!/usr/bin/env bash
# X3 Shield judge one-command gate: local tests only. NEVER signs, deploys or
# sends funds. Exits nonzero on stale scenarios, policy disagreement, failed
# tests, or a missing prompt-injection case.
set -euo pipefail
umask 077
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPORT_DIR="${1:-/tmp/x3-shield-judge}"
cd "$ROOT"

PYTHON="${PYTHON:-python3}"
NODE="${NODE:-node}"
# Respect a caller-supplied NODE override while keeping the canonical call
# literal: `node --test x3-lang/near/demo/test/*.test.mjs` below.
node() { command "$NODE" "$@"; }

for dependency in node cargo; do
  if ! command -v "$dependency" >/dev/null 2>&1; then
    echo "Missing $dependency: install the development prerequisite, then rerun." >&2
    exit 2
  fi
done

if ! "$PYTHON" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
  echo 'Python 3.11+ is required by NEAR evidence parsing; install Python 3.11+ (Python 3.10 cannot run this judge gate).' >&2
  exit 2
fi

if ! "$PYTHON" -c 'import pytest, cryptography' >/dev/null 2>&1; then
  VENV_DIR="${X3_JUDGE_VENV:-${XDG_CACHE_HOME:-$HOME/.cache}/x3lang/judge-quick-venv}"
  if [[ -x "$VENV_DIR/bin/python" ]] && ! "$VENV_DIR/bin/python" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
    echo 'Existing judge environment is too old; set X3_JUDGE_VENV to a different directory.' >&2
    exit 2
  fi
  if [[ ! -x "$VENV_DIR/bin/python" ]] || ! "$VENV_DIR/bin/python" -c 'import pytest, cryptography' >/dev/null 2>&1; then
    echo "Provisioning isolated judge environment at $VENV_DIR from the exact-pinned judge requirements (package downloads may require internet)." >&2
    "$PYTHON" -m venv "$VENV_DIR"
    "$VENV_DIR/bin/python" -m pip install --disable-pip-version-check -r scripts/hackathon/requirements-judge.txt
  fi
  PYTHON="$VENV_DIR/bin/python"
fi

# 1. Scenario freshness: stale scenarios.json fails loudly.
"$PYTHON" x3-lang/near/demo/judge/build_scenarios.py --check
# 2. Terminal five-panel demo, including the prompt-injection case. demo.py
# exits 1 on policy disagreement; the JSON gate below also fails on a missing
# prompt-injection scenario (demo.py alone only iterates scenarios present).
"$PYTHON" scripts/hackathon/demo.py
"$PYTHON" scripts/hackathon/demo.py --json | "$PYTHON" -c 'import json, sys; data = json.load(sys.stdin); scenarios = {s["id"]: s for s in data["scenarios"]}; case = scenarios.get("prompt-injection-recipient"); sys.exit(0 if case and case.get("decision") == "denied_by_policy" and case.get("denied_guards") == ["mandate_recipient"] else 1)'
# 3. Browser parity: Python and the console must agree guard by guard.
node --test x3-lang/near/demo/test/*.test.mjs
# 4. Judge evidence gate (quick): Python, browser, Guard and allocator checks.
"$PYTHON" scripts/judge_evidence.py --mode quick --report-dir "$REPORT_DIR"
