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


def test_no_example_makes_this_surface_raise_anything_but_a_parse_error():
    """Every example is either read or refused by name — nothing crashes.

    This is the part of the boundary that is not a scope decision. The surface reads one
    `intent` per file, nine guard kinds of the compiler's eighteen, and a stricter address
    shape than the compiler's, so it refuses a good many of the repository's examples — and
    each of those refusals has to be a `X3ParseError` with a code and a line. Five examples
    used to raise `IndexError: list index out of range` instead, from a file with no
    `intent` in it: a caller had nothing to catch and no line number to report.

    Scoped to the whole `examples/` directory rather than to the suite's fixtures, because
    the claim — nothing crashes — is true of all of them and does not depend on the scope
    question above.
    """
    root = Path(__file__).resolve().parents[1]
    examples = sorted(glob.glob(str(root / "examples" / "*.x3")))
    assert len(examples) > 10, (
        f"found {len(examples)} examples under {root / 'examples'}, which is too few to be "
        "the directory this test thinks it is reading"
    )

    crashes = []
    for path in examples:
        try:
            cli.parse_file(path)
        except cli.X3ParseError:
            pass  # refused by name, which is what this asserts
        except Exception as error:  # noqa: BLE001 - any other type is the defect
            crashes.append(f"{os.path.basename(path)}: {type(error).__name__}: {error}")

    assert not crashes, (
        "these examples failed with something other than X3ParseError, so a caller has "
        "nothing to catch:\n" + "\n".join(crashes)
    )


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


#: What this surface reads today, and what it refuses **and why**, one entry per file.
#:
#: The scope is stated in `cli.py`'s module doc — one `intent` per file, nine guard kinds of the
#: compiler's eighteen, a stricter address shape. This is that statement turned into a contract:
#: a file that starts being read, or one that stops, fails here rather than being discovered by
#: hand. The refusal *code* is part of the entry because a refusal for a different reason is a
#: different boundary, and the codes are what `cli.py` promises a caller.
#:
#: Not a claim that the boundary is right. Two entries here are the compiler accepting something
#: this surface refuses (`X3_PARSE_RECEIVER`, the address shape — TICKET-091) and three are guard
#: kinds outside `registry.REQUIRE_KINDS`. The assertion is that the boundary is *known*.
READS = {
    "arb_solana_eth.x3",
    "timeout_refund.x3",
    "timeout_refund_minimal.x3",
}

REFUSES = {
    "arb_scope.x3": "X3_PARSE_RECEIVER",
    "atomic_choice.x3": "X3_PARSE_NO_INTENT",
    "atomic_swap.x3": "X3_PARSE_NO_INTENT",
    "flagship_b52.x3": "X3_PARSE_NO_INTENT",
    "intent_fusion.x3": "X3_PARSE_RECEIVER",
    "mainnet_safe_swap.x3": "X3_PARSE_NO_INTENT",
    "multi_leg_route.x3": "X3_PARSE_REQUIRE",
    "objective_routing.x3": "X3_PARSE_NO_INTENT",
    "opportunity_graph.x3": "X3_PARSE_NO_INTENT",
    "parallel_dag.x3": "X3_PARSE_NO_INTENT",
    "route_fallback.x3": "X3_PARSE_OPERATION",
    "simple_swap.x3": "X3_PARSE_REQUIRE",
    "staking_intent.x3": "X3_PARSE_REQUIRE",
    "strategy_module.x3": "X3_PARSE_NO_INTENT",
    "trading_core_v1.x3": "X3_PARSE_NO_INTENT",
    "trading_effects.x3": "X3_PARSE_NO_INTENT",
}


def test_the_accept_refuse_set_is_what_this_surface_reads():
    """Every example is either read or refused **for the reason recorded here**."""
    root = Path(__file__).resolve().parents[1]
    examples = sorted(glob.glob(str(root / "examples" / "*.x3")))
    assert len(examples) > 10, (
        f"found {len(examples)} examples under {root / 'examples'}, which is too few to be "
        "the directory this test thinks it is reading"
    )

    pinned = set(READS) | set(REFUSES)
    assert not (set(READS) & set(REFUSES)), "a file cannot be read and refused"
    unclassified = sorted(os.path.basename(path) for path in examples if os.path.basename(path) not in pinned)
    assert not unclassified, (
        "these examples are not in the table below, so nothing says whether this surface is "
        "supposed to read them. Classify each as read, or as refused with the code it refuses "
        "with:\n  " + "\n  ".join(unclassified)
    )

    wrong = []
    for path in examples:
        name = os.path.basename(path)
        expected = "reads" if name in READS else REFUSES[name]
        try:
            cli.parse_file(path)
            outcome = "reads"
        except cli.X3ParseError as error:
            outcome = error.code
        if outcome != expected:
            wrong.append(f"{name}: expected {expected!r}, got {outcome!r}")

    assert not wrong, (
        "the boundary this surface draws moved. If that is intended, move the entry with it — "
        "the numbers are the point, because a file that *starts* being read is as much a change "
        "as one that stops:\n  " + "\n  ".join(wrong)
    )
