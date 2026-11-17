"""Compare 1Click's live chain list with what X3 Route routes, so the map cannot silently go stale.

    python3 x3-lang/tools/oneclick_chain_sync.py                 # fetch GET /v0/tokens (no key needed)
    python3 x3-lang/tools/oneclick_chain_sync.py --tokens t.json # an archived /v0/tokens list

Every `blockchain` code 1Click lists must be either routed (`near_intents.CHAIN_TO_BLOCKCHAIN`) or
classified with a reason (`near_intents.ONECLICK_UNROUTED`). Exit 1 when:

- 1Click lists a code that is neither (a new chain to decide on), or
- a code X3 Route routes is no longer listed (a route that can no longer be quoted).

A classified code that 1Click stopped listing is reported as `stale_unrouted` but does not fail:
nothing routes it. The report is JSON on stdout. Nothing here requests a quote or moves funds.
"""
from __future__ import annotations

import argparse
import importlib.util
import json
import sys
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional

ROOT = Path(__file__).resolve().parents[1]


def _near_intents():
    sys.path.insert(0, str(ROOT))
    spec = importlib.util.spec_from_file_location("near_intents_sync", ROOT / "near_intents.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def compare(live_codes: Iterable[str], routed: Dict[str, str], unrouted: Dict[str, str]) -> Dict[str, Any]:
    """The verdict for one snapshot of 1Click's chain codes. Pure: no network."""
    live = set(live_codes)
    routed_codes = set(routed.values())
    report = {
        "live_codes": len(live),
        "routed": sorted(routed_codes & live),
        "missing_routed": sorted(routed_codes - live),
        "unclassified": sorted(live - routed_codes - set(unrouted)),
        "stale_unrouted": sorted(set(unrouted) - live),
        "overlap": sorted(routed_codes & set(unrouted)),
    }
    report["ok"] = not (report["missing_routed"] or report["unclassified"] or report["overlap"])
    return report


def live_codes(tokens: List[Dict[str, Any]]) -> List[str]:
    codes = {t.get("blockchain") for t in tokens if isinstance(t, dict)}
    if None in codes or not codes:
        raise ValueError("a /v0/tokens entry has no blockchain code, or the list is empty")
    return sorted(codes)


def main(argv: Optional[List[str]] = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--tokens", help="a saved GET /v0/tokens JSON list (default: fetch it live)")
    args = parser.parse_args(argv)
    ni = _near_intents()
    if args.tokens:
        tokens = json.loads(Path(args.tokens).read_text())
    else:
        tokens = ni.OneClickClient(jwt="").tokens()
    report = compare(live_codes(tokens), ni.CHAIN_TO_BLOCKCHAIN, ni.ONECLICK_UNROUTED)
    print(json.dumps(report, indent=2, sort_keys=True))
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
