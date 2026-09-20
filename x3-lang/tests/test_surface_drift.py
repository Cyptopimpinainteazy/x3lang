"""The Python surface must read the examples its own suite relies on.

This repository has two parsers for one language: the Rust compiler, and the Python
surface in `cli.py` / `typechecker.py`. They drift, and one round found three drifts in
the same direction — every one of them a spelling the compiler or the *formatter* accepts
and this surface did not:

  - `proof_complete`, the compiler's name for what this surface called `proof`;
  - `finality.<chain>`, which is what `x3c fmt` writes for `finality <chain>`;
  - a refund whose receiver is omitted, which means `sender` — again the formatter's
    canonical output for `refund <asset> to sender`.

Each made this surface reject a file the compiler had just accepted, and each was found by
accident: the first while renaming a guard in an example, the other two only after
reformatting one. The examples are where the two implementations meet, and `x3c fmt` is the
canonical writer, so they are where the drifts surface.

**Why this test is scoped to the suite's own fixtures rather than to every example.** The
same gate written against all of `examples/*.x3` fails for **thirteen of nineteen** files,
and the reasons are the surface's scope rather than this round's drift: it accepts only a
file whose first item is an `intent` (seven examples start with `finality_policy` or
`risk_policy`), it validates address *shape* so a short placeholder like `0xA1` is refused,
and it knows nine guard kinds where the compiler knows eighteen. That boundary is a
decision for whoever owns this surface — either it is intentional and should be stated, or
it should be closed — and it is recorded as a ticket rather than asserted here as a
requirement the surface does not meet.

What is asserted here is true today and was not before: the example this suite runs must
be readable, so the three drifts above cannot come back unnoticed.
"""

import glob
import os
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

import cli  # noqa: E402  (the path insert has to come first)


def test_every_example_the_suite_uses_is_readable_by_this_surface():
    root = Path(__file__).resolve().parents[1]
    # Derived from the suite rather than listed, so a test that starts using a new example
    # brings it into the gate by doing so.
    used = set()
    for test_file in glob.glob(str(Path(__file__).resolve().parent / "*.py")):
        source = Path(test_file).read_text()
        for name in glob.glob("*", root_dir=root / "examples"):
            if f"examples' / '{name}" in source or f'examples" / "{name}' in source:
                used.add(name)
    examples = sorted(str(root / "examples" / name) for name in used)

    # A gate that found nothing would pass the assertion below. Same guard, and the same
    # reason, as the Rust gate over the same directory.
    assert len(examples) >= 1, "the suite names no example, so this gate would be vacuous"

    failures = []
    for path in examples:
        try:
            cli.parse_file(path)
        except Exception as error:  # noqa: BLE001 - the point is to report any refusal
            failures.append(f"{os.path.basename(path)}: {error}")

    assert not failures, "every example must be readable by this surface:\n" + "\n".join(failures)
