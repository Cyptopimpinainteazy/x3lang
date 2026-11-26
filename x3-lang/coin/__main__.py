"""Author coins, reserve inventory and reconcile testnet sale receipts."""
import argparse
from http.client import HTTPException
import json
import os
import sqlite3
import sys
from pathlib import Path
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

from . import MAX_SOURCE_BYTES, CoinError, author_coin, compile_coin
from .delivery import SEPOLIA_CHAIN_ID, SEPOLIA_GENESIS
from .sale import plan_sale


RPC_READ_METHODS = frozenset({
    "eth_chainId", "eth_getBlockByNumber", "eth_getLogs", "eth_getTransactionByHash",
    "eth_getTransactionReceipt", "eth_getCode", "eth_call",
})
MAX_RPC_BYTES = 2 * 1024 * 1024


class _NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise CoinError("RPC redirects are refused")


def _reader(url):
    opener = build_opener(_NoRedirect())

    def read(method, params):
        if method not in RPC_READ_METHODS:
            raise CoinError("unavailable RPC method for sale confirmation")
        payload = {"jsonrpc": "2.0", "id": "x3-sale-read", "method": method, "params": params}
        try:
            req = Request(url, data=json.dumps(payload).encode(), headers={"Content-Type": "application/json"})
            with opener.open(req, timeout=15) as response:
                if response.status != 200:
                    raise CoinError("RPC returned an unexpected HTTP status")
                raw = response.read(MAX_RPC_BYTES + 1)
            if len(raw) > MAX_RPC_BYTES:
                raise CoinError("RPC response exceeds size limit")
            reply = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_fields)
            if (not isinstance(reply, dict) or reply.get("jsonrpc") != "2.0"
                    or reply.get("id") != payload["id"] or "error" in reply or "result" not in reply):
                raise CoinError("RPC returned an invalid response")
            return reply["result"]
        except (OSError, ValueError, HTTPException, RecursionError):
            # Endpoints can carry API tokens in their path. RPC error messages
            # and urllib exceptions can include them, so expose only the method name.
            raise CoinError(f"RPC read failed for {method}") from None

    return read


def _sale_readers(urls):
    if not urls or len(urls) < 2:
        raise CoinError("sale confirmation requires at least two distinct HTTPS RPC hosts")
    hosts = set()
    readers = []
    for url in urls:
        try:
            if not isinstance(url, str) or not url.isascii() or any(ord(c) <= 32 or ord(c) == 127 for c in url):
                raise ValueError()
            parsed = urlsplit(url)
            host = (parsed.hostname or "").lower().rstrip(".")
            if (parsed.scheme != "https" or not host or parsed.username is not None
                    or parsed.password is not None or parsed.query or parsed.fragment or parsed.port == 0):
                raise ValueError()
        except ValueError:
            raise CoinError("RPC address must be an HTTPS URL without userinfo, query or fragment") from None
        if host in hosts:
            raise CoinError("use distinct hosts operated by independent RPC providers")
        hosts.add(host)
        readers.append((url, _reader(url)))
    return readers


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
    if args.command == "release":
        return book.release(args.asset_id, args.order_id)
    if args.command == "sale-prepare":
        return book.prepare_delivery(args.asset_id, args.order_id, read_spec(args.policy))
    delivery = book.delivery(args.asset_id, args.order_id)
    if args.command == "sale-status":
        return delivery
    terms = delivery["terms"]
    if terms["chain_id"] != SEPOLIA_CHAIN_ID or terms["genesis_hash"] != SEPOLIA_GENESIS:
        raise CoinError("remote sale confirmation requires Ethereum Sepolia chain ID and genesis hash")
    return book.reconcile_delivery(args.asset_id, args.order_id,
                                   _sale_readers([os.environ.get(name) for name in args.rpc_env]))


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
        ("sale-prepare", "pin the sale terms and return unsigned steward and buyer wallet calls"),
        ("sale-status", "read a prepared delivery and its stored evidence"),
        ("sale-confirm", "reconcile a prepared Ethereum Sepolia sale via read-only RPCs"),
    ):
        command = commands.add_parser(name, help=help_text)
        command.add_argument("--db", required=True, type=Path, help="local reservation database path")
        if name in ("inventory-init", "reserve"):
            command.add_argument("input", type=Path)
        else:
            command.add_argument("asset_id")
            if name != "inventory":
                command.add_argument("order_id")
            if name == "sale-prepare":
                command.add_argument("policy", type=Path, help="reviewed delivery policy JSON path")
            if name == "sale-confirm":
                command.add_argument("--rpc-env", action="append", required=True, metavar="NAME",
                                     help="environment variable containing an HTTPS RPC URL; repeat for each provider")
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
