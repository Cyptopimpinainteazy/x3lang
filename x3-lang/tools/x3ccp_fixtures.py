"""Regenerate the X3CCP policy fixtures that the Foundry suite installs on chain.

    python3 x3-lang/tools/x3ccp_fixtures.py          # write fixtures
    python3 x3-lang/tools/x3ccp_fixtures.py --check  # fail if any fixture is stale

The fixtures are the compiler's own output for fixed sources with absolute expiry, so the bytes
are reproducible. x3-lang/tests/test_x3ccp.py runs `--check`, which keeps the Solidity tests
pinned to what the compiler really emits.
"""

from __future__ import annotations

import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, ROOT)

import x3ccp  # noqa: E402

FIXTURE_DIR = os.path.join(ROOT, "x3ccp-evm", "test", "fixtures")

# A stricter replacement policy for the same treasury: used to prove that installing a new
# policy starts a new epoch, applies new limits at once and resets rate usage.
TIGHT_SOURCE = """control treasury_guard_tight {
    target local 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f
    owner 0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496
    allow callers 0x3333333333333333333333333333333333333333
    allow fn withdraw(address to, uint256 amount)
    limit withdraw.amount <= 10_000
    limit withdraw.to == 0x4444444444444444444444444444444444444444
    expires at 2030-01-01T00:00:00Z
}
"""


def sources() -> dict:
    with open(os.path.join(ROOT, "examples", "x3ccp_treasury_guard.x3"), encoding="utf-8") as fh:
        guard = fh.read()
    return {"treasury_guard": guard, "treasury_guard_tight": TIGHT_SOURCE}


def rendered() -> dict:
    out = {}
    for name, source in sources().items():
        compiled = x3ccp.compile_source(source)
        out[f"{name}.hex"] = compiled["policy_hex"] + "\n"
        out[f"{name}.hash"] = compiled["policy_hash"] + "\n"
    return out


def main(argv) -> int:
    check = "--check" in argv
    stale = []
    os.makedirs(FIXTURE_DIR, exist_ok=True)
    for name, text in rendered().items():
        path = os.path.join(FIXTURE_DIR, name)
        current = open(path, encoding="utf-8").read() if os.path.exists(path) else None
        if current != text:
            stale.append(name)
            if not check:
                with open(path, "w", encoding="utf-8") as fh:
                    fh.write(text)
    if check and stale:
        print("stale X3CCP fixtures: " + ", ".join(stale), file=sys.stderr)
        return 1
    print("fixtures " + ("current" if check else "written") + f" in {FIXTURE_DIR}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
