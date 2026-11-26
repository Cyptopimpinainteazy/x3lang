"""Offline command-line entry point for .x3 coin authoring and treasury reservations."""
import argparse
import json
import sqlite3
import sys
from pathlib import Path

from . import MAX_SOURCE_BYTES, CoinError, author_coin, compile_coin
from .sale import plan_sale


def unique_fields(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise CoinError("duplicate JSON field")
        result[key] = value
    return result


def read_input(path):
    with path.open("rb") as handle:
        raw = handle.read(MAX_SOURCE_BYTES + 1)
    if len(raw) > MAX_SOURCE_BYTES:
        raise CoinError("input exceeds the size limit")
    return raw.decode("ascii")


def read_spec(path):
    return json.loads(read_input(path), object_pairs_hook=unique_fields)


def run_book(args):
    from .reservations import ReservationBook

    if args.command == "inventory-init":
        config = read_spec(args.input)
        return ReservationBook(args.db).initialize(config)
    if not args.db.is_file():
        raise CoinError("reservation database does not exist; use inventory-init first")
    book = ReservationBook(args.db)
    if args.command == "reserve":
        return book.reserve(read_spec(args.input))
    if args.command == "inventory":
        return book.inventory(args.asset_id)
    if args.command == "reservation":
        return book.get(args.asset_id, args.order_id)
    return book.release(args.asset_id, args.order_id)


def main(argv=None):
    parser = argparse.ArgumentParser(prog="python -m coin", description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("author", help="write .x3 source from an issuer specification").add_argument("input", type=Path)
    commands.add_parser("build", help="compile .x3 to an unsigned dual-VM bundle").add_argument("input", type=Path)
    commands.add_parser("sale", help="quote equal participation by treasury inventories").add_argument("input", type=Path)
    for name, help_text in (
        ("inventory-init", "initialize opening balances from an inventory JSON file"),
        ("reserve", "place a local hold from an order JSON file"),
        ("inventory", "show available quota and delivery balance on each network"),
        ("reservation", "look up an existing local hold"),
        ("release", "release a local hold"),
    ):
        command = commands.add_parser(name, help=help_text)
        command.add_argument("--db", required=True, type=Path, help="local reservation database path")
        if name in ("inventory-init", "reserve"):
            command.add_argument("input", type=Path)
        else:
            command.add_argument("asset_id")
            if name in ("reservation", "release"):
                command.add_argument("order_id")
    args = parser.parse_args(argv)
    try:
        if args.command == "author":
            sys.stdout.write(author_coin(read_spec(args.input)))
            return 0
        if args.command == "build":
            result = compile_coin(read_input(args.input))
        elif args.command == "sale":
            result = plan_sale(read_spec(args.input))
        else:
            result = run_book(args)
        print(json.dumps(result, indent=2))
        return 0
    except (ValueError, OSError, RecursionError, sqlite3.Error) as exc:
        print(f"coin: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
