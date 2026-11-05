#!/usr/bin/env bash
# One-command local verification for the offline X3Lang three-track judge demo.
#
# It runs the existing evidence recorder (Python, browser, scenario freshness and x3-guard),
# prints the five-panel walkthrough, and emits a machine-readable readiness report.
# It does not access the network, sign a transaction, deploy a contract or move funds.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
REPORT="${1:-/tmp/x3lang-hackathon-readiness.json}"

cd "$ROOT"
python3 x3-lang/near/demo/judge/collect_evidence.py --with-contract
python3 scripts/hackathon/demo.py
python3 scripts/hackathon/readiness.py --out "$REPORT"

printf '\nReadiness report: %s\n' "$REPORT"
