#!/usr/bin/env bash
# X3 Shield judge one-command gate: local tests only. NEVER signs, deploys or
# sends funds. Exits nonzero on stale scenarios, policy disagreement, failed
# tests, or a missing prompt-injection case.
#
# Default behavior always creates or reuses the isolated pinned venv at
# $X3_JUDGE_VENV (or the XDG cache default) installed from the hash-locked
# scripts/hackathon/requirements-judge.txt, and verifies exact installed
# versions (cryptography, pytest and pycryptodome). System pytest/cryptography
# are never sufficient on their own. A caller-managed interpreter (PYTHON set)
# requires explicit opt-in via X3_JUDGE_ALLOW_SYSTEM_PYTHON=1 plus exact
# version validation, otherwise the script provisions or refuses (exit 2).
set -euo pipefail
umask 077
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPORT_DIR="${1:-/tmp/x3-shield-judge}"
cd "$ROOT"

CALLER_PYTHON="${PYTHON:-}"
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

validate_judge_python() {
  local candidate="$1"
  if ! "$candidate" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
    echo "Judge gate requires Python 3.11+; '$candidate' is not." >&2
    return 1
  fi
  if ! "$candidate" - <<'PY' >/dev/null 2>&1
from importlib.metadata import PackageNotFoundError, version
import sys
expected = {"cryptography": "50.0.2", "pytest": "9.2.0", "pycryptodome": "4.3.0"}
for dist, want in expected.items():
    try:
        got = version(dist)
    except PackageNotFoundError:
        sys.exit(1)
    if got != want:
        sys.exit(1)
PY
  then
    echo "Judge gate requires exact judge versions (cryptography==50.0.2, pytest==9.2.0, pycryptodome==4.3.0); '$candidate' does not match." >&2
    return 1
  fi
  return 0
}

if [[ -n "$CALLER_PYTHON" ]]; then
  # Caller-managed interpreter: explicit opt-in plus exact validation, else refuse.
  if [[ "${X3_JUDGE_ALLOW_SYSTEM_PYTHON:-}" != "1" ]]; then
    echo "PYTHON=$CALLER_PYTHON is caller-managed. To use it, set X3_JUDGE_ALLOW_SYSTEM_PYTHON=1 (it must still have the exact pinned judge versions). Otherwise unset PYTHON so the isolated pinned judge venv is provisioned." >&2
    exit 2
  fi
  if ! validate_judge_python "$CALLER_PYTHON"; then
    echo "Refusing caller-managed python that does not match the pinned judge versions. Unset PYTHON to provision the isolated venv, or install the pinned versions." >&2
    exit 2
  fi
  PYTHON="$CALLER_PYTHON"
else
  # Default: always create or reuse the isolated pinned venv.
  BASE_PYTHON=""
  for candidate in python3.14 python3.13 python3.12 python3.11 python3; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -c 'import sys; sys.exit(0 if sys.version_info >= (3, 11) else 1)' >/dev/null 2>&1; then
      BASE_PYTHON="$(command -v "$candidate")"
      break
    fi
  done
  if [[ -z "$BASE_PYTHON" ]]; then
    echo 'Python 3.11+ is required by NEAR evidence parsing; install Python 3.11+ (Python 3.10 cannot run this judge gate).' >&2
    exit 2
  fi
  VENV_DIR="${X3_JUDGE_VENV:-${XDG_CACHE_HOME:-$HOME/.cache}/x3lang/judge-quick-venv}"
  if [[ -x "$VENV_DIR/bin/python" ]]; then
    if ! validate_judge_python "$VENV_DIR/bin/python"; then
      echo "Existing judge environment at $VENV_DIR does not match the pinned versions. Set X3_JUDGE_VENV to a different directory; refusing rather than reusing a wrong env." >&2
      exit 2
    fi
  else
    echo "Provisioning isolated judge environment at $VENV_DIR from the hash-locked judge requirements (first-run dependency installation may use the network)." >&2
    if [[ -e "$VENV_DIR" ]]; then
      echo "Partial judge environment exists at $VENV_DIR without a usable python; set X3_JUDGE_VENV to a clean directory." >&2
      exit 2
    fi
    "$BASE_PYTHON" -m venv "$VENV_DIR"
    "$VENV_DIR/bin/python" -m pip install --disable-pip-version-check --require-hashes -r scripts/hackathon/requirements-judge.txt
    if ! validate_judge_python "$VENV_DIR/bin/python"; then
      echo "Freshly provisioned judge environment at $VENV_DIR failed exact version validation." >&2
      exit 2
    fi
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
