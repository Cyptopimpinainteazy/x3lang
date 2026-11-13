#!/usr/bin/env bash
# X3 Shield judge one-command gate: local tests only. NEVER signs, deploys or
# sends funds. Exits nonzero on stale scenarios, policy disagreement, failed
# tests, or a missing prompt-injection case.
#
# Platform lock: this gate requires Linux x86_64 and Python 3.12. The
# hash-locked judge requirements in scripts/hackathon/requirements-judge.txt
# were validated only on Linux x86_64 + Python 3.12. The general X3Lang
# bridge/evidence code retains its Python 3.11+ minimum, but the one-command
# judge gate fails clearly on any other platform or Python version.
#
# Default behavior always creates or reuses the isolated pinned venv at
# $X3_JUDGE_VENV (or the XDG cache default) installed from the hash-locked
# requirements, and verifies the full 9-package freeze on every run. System
# pytest/cryptography are never sufficient on their own. A caller-managed
# interpreter (PYTHON set) requires explicit opt-in via
# X3_JUDGE_ALLOW_SYSTEM_PYTHON=1 plus the same full-freeze validation,
# otherwise the script refuses (exit 2).
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

if ! command -v "$NODE" >/dev/null 2>&1; then
  echo "Missing Node executable '$NODE': install Node 22+ or set NODE to the executable path." >&2
  exit 2
fi
if ! command -v cargo >/dev/null 2>&1; then
  echo "Missing cargo: install the Rust development prerequisite, then rerun." >&2
  exit 2
fi

# The hash lock is only valid on Linux x86_64. Fail clearly elsewhere.
HOST_OS="$(uname -s 2>/dev/null || echo unknown)"
HOST_MACHINE="$(uname -m 2>/dev/null || echo unknown)"
if [[ "$HOST_OS" != "Linux" || "$HOST_MACHINE" != "x86_64" ]]; then
  echo "Judge gate requires Linux x86_64 (the one-command lock was validated only there); detected '$HOST_OS $HOST_MACHINE'. General code may use Python 3.11+, but this gate needs the 3.12 judge lock on Linux x86_64." >&2
  exit 2
fi

# A judge python must be Python 3.12 on Linux x86_64 with the exact 9-package freeze.
validate_judge_python() {
  local candidate="$1"
  "$candidate" - <<'PY'
from importlib.metadata import PackageNotFoundError, version
import platform, sys
expected = {
    "cffi": "2.1.1",
    "cryptography": "50.0.2",
    "iniconfig": "2.3.1",
    "packaging": "26.3",
    "pluggy": "1.7.0",
    "pycparser": "3.12",
    "pycryptodome": "4.3.0",
    "Pygments": "2.21.0",
    "pytest": "9.2.0",
}
problems = []
if sys.platform != "linux" or platform.machine() != "x86_64":
    problems.append(f"platform {sys.platform}/{platform.machine()} is not Linux x86_64")
if sys.version_info[:2] != (3, 12):
    problems.append(f"Python {sys.version_info.major}.{sys.version_info.minor} is not 3.12")
installed = {}
for dist, want in expected.items():
    try:
        got = version(dist)
    except PackageNotFoundError:
        got = "<missing>"
    installed[dist] = got
    if got != want:
        problems.append(f"{dist} is {got}, requires {want}")
if problems:
    print(f"'{sys.executable}' does not match the judge lock: " + "; ".join(problems), file=sys.stderr)
    sys.exit(1)
PY
}

if [[ -n "$CALLER_PYTHON" ]]; then
  # Caller-managed interpreter: explicit opt-in plus exact validation, else refuse.
  if [[ "${X3_JUDGE_ALLOW_SYSTEM_PYTHON:-}" != "1" ]]; then
    echo "PYTHON=$CALLER_PYTHON is caller-managed. To use it, set X3_JUDGE_ALLOW_SYSTEM_PYTHON=1 (it must still have Python 3.12, Linux x86_64, and the full 9-package pin). Otherwise unset PYTHON so the isolated pinned judge venv is provisioned." >&2
    exit 2
  fi
  if ! validate_judge_python "$CALLER_PYTHON"; then
    echo "Refusing caller-managed python that does not match the judge lock. Unset PYTHON to provision the isolated venv, or install the full pinned freeze." >&2
    exit 2
  fi
  PYTHON="$CALLER_PYTHON"
else
  # Default: always create or reuse the isolated pinned venv.
  BASE_PYTHON=""
  for candidate in python3.12 python3; do
    if command -v "$candidate" >/dev/null 2>&1 && "$candidate" -c 'import sys; sys.exit(0 if sys.version_info[:2] == (3, 12) and sys.platform == "linux" else 1)' >/dev/null 2>&1; then
      BASE_PYTHON="$(command -v "$candidate")"
      break
    fi
  done
  # uv-managed interpreters may not expose python3.12 on PATH. Probe
  # existing local installations only; never download Python implicitly or
  # relax the hash-locked Linux x86_64 / Python 3.12 package requirements.
  if [[ -z "$BASE_PYTHON" ]] && command -v uv >/dev/null 2>&1; then
    uv_python="$(uv python find --offline --no-python-downloads 3.12 2>/dev/null || true)"
    if [[ -n "$uv_python" && -x "$uv_python" ]] &&
       "$uv_python" -c 'import platform,sys; sys.exit(0 if sys.platform == "linux" and platform.machine() == "x86_64" and sys.version_info[:2] == (3, 12) else 1)' >/dev/null 2>&1; then
      BASE_PYTHON="$uv_python"
    fi
  fi
  if [[ -z "$BASE_PYTHON" ]]; then
    echo "Judge gate requires Linux x86_64 and Python 3.12 exactly (hash-locked). Install Python 3.12 on PATH or install a local uv-managed interpreter with: uv python install 3.12. Nonmatching platforms/interpreters remain blocked." >&2
    exit 2
  fi
  VENV_DIR="${X3_JUDGE_VENV:-${XDG_CACHE_HOME:-$HOME/.cache}/x3lang/judge-quick-venv}"
  if [[ -x "$VENV_DIR/bin/python" ]]; then
    if ! validate_judge_python "$VENV_DIR/bin/python"; then
      echo "Existing judge environment at $VENV_DIR does not match the full 9-package freeze. Set X3_JUDGE_VENV to a different directory; refusing rather than reusing a wrong env." >&2
      exit 2
    fi
  else
    if [[ -e "$VENV_DIR" ]]; then
      echo "Partial judge environment exists at $VENV_DIR without a usable python; set X3_JUDGE_VENV to a clean directory." >&2
      exit 2
    fi
    echo "Provisioning isolated judge environment at $VENV_DIR from the hash-locked judge requirements (first-run dependency installation may use the network)." >&2
    VENV_PARENT="$(dirname "$VENV_DIR")"
    mkdir -p "$VENV_PARENT"
    TMP_VENV_DIR="$(mktemp -d "$VENV_PARENT/.x3-judge-venv.provisioning.XXXXXX")"
    # Build into the temp dir; only promote it to $VENV_DIR after the hash
    # install and the full-freeze validation succeed. A failed build removes
    # the temp dir only, so no partial $VENV_DIR poisons the next run.
    if ! "$BASE_PYTHON" -m venv "$TMP_VENV_DIR" 2>/dev/null; then
      rm -rf "$TMP_VENV_DIR"
      echo "Provisioning failed while creating the temporary venv; $VENV_DIR was not created." >&2
      exit 2
    fi
    if ! "$TMP_VENV_DIR/bin/python" -m pip install --disable-pip-version-check --require-hashes -r scripts/hackathon/requirements-judge.txt; then
      rm -rf "$TMP_VENV_DIR"
      echo "Provisioning failed during hash-locked install; $VENV_DIR was not created and no partial venv remains." >&2
      exit 2
    fi
    if ! validate_judge_python "$TMP_VENV_DIR/bin/python"; then
      rm -rf "$TMP_VENV_DIR"
      echo "Freshly provisioned judge environment failed full-freeze validation; $VENV_DIR was not created." >&2
      exit 2
    fi
    # CPython venv entry-point scripts and activate scripts embed the venv's
    # absolute path; a move would otherwise leave dangling shebangs. Rewrite
    # the old temp prefix to the final prefix inside the moved files, then
    # re-validate from the final path before reusing it.
    mv "$TMP_VENV_DIR" "$VENV_DIR"
    if [[ -n "$(grep -rlI "$TMP_VENV_DIR" "$VENV_DIR" 2>/dev/null)" ]]; then
      grep -rlI "$TMP_VENV_DIR" "$VENV_DIR" | xargs sed -i "s|$TMP_VENV_DIR|$VENV_DIR|g" || true
    fi
    # Activate prompts embed the venv basename too; normalize that cosmetic name.
    grep -rlI "$(basename "$TMP_VENV_DIR")" "$VENV_DIR/bin" 2>/dev/null | xargs sed -i "s|$(basename "$TMP_VENV_DIR")|$(basename "$VENV_DIR")|g" 2>/dev/null || true
    if ! "$VENV_DIR/bin/pip" --version >/dev/null 2>&1 || ! validate_judge_python "$VENV_DIR/bin/python"; then
      rm -rf "$VENV_DIR"
      echo "Promoted judge environment at $VENV_DIR failed post-move entry-point/full-freeze validation; removing it." >&2
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
# 3b. Mainnet invoice_06, link by link, from committed evidence: the .x3 files Guard
# hashed, compiled to exactly the policy and mandate Guard stores; edited copies refused.
# Exits 1 if any of the five links breaks.
"$PYTHON" scripts/hackathon/invoice06_chain.py
# 4. Judge evidence gate (quick): Python, browser, Guard and allocator checks.
"$PYTHON" scripts/judge_evidence.py --mode quick --report-dir "$REPORT_DIR"
