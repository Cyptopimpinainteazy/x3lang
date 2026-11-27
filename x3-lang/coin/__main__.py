"""Offline command-line entry point for .x3 fixed-supply coin authoring."""
import argparse
import json
import sys
from pathlib import Path

from . import MAX_SOURCE_BYTES, CoinError, author_coin, compile_coin


def unique_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise CoinError("duplicate JSON field")
        result[key] = value
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(prog="python -m coin", description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("author", help="write .x3 source from an issuer specification").add_argument("input", type=Path)
    commands.add_parser("build", help="compile .x3 to an unsigned dual-VM bundle").add_argument("input", type=Path)
    commands.add_parser("sale", help="quote equal participation by treasury inventories").add_argument("input", type=Path)
    args = parser.parse_args(argv)
    try:
        with args.input.open("rb") as handle:
            raw = handle.read(MAX_SOURCE_BYTES + 1)
        if len(raw) > MAX_SOURCE_BYTES:
            raise CoinError("input exceeds the size limit")
        source = raw.decode("ascii")
        if args.command == "build":
            result = compile_coin(source)
        else:
            spec = json.loads(source, object_pairs_hook=unique_fields)
            if args.command == "author":
                sys.stdout.write(author_coin(spec))
                return 0
            from .sale import plan_sale
            result = plan_sale(spec)
        print(json.dumps(result, indent=2))
        return 0
    except (ValueError, OSError, RecursionError) as exc:
        print(f"coin: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
