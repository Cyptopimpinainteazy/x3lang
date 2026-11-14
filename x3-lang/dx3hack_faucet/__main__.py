"""python3 -m dx3hack_faucet serve --network mainnet --key ~/.x3/dx3hack/faucet-key.mainnet.json --db faucet.db

Reads TURNSTILE_SECRET and TURNSTILE_SITE_KEY from the environment (required on mainnet).
Binds loopback by default; publish it through a Cloudflare tunnel (e.g. faucet.dotx3.net)
and pass --trust-cf-ip so per-client limits use CF-Connecting-IP.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import sys

from .near_tx import NearError
from .runner import Config, Faucet, FaucetServer


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(prog="python3 -m dx3hack_faucet")
    sub = parser.add_subparsers(dest="cmd", required=True)
    for name in ("serve", "preflight"):
        p = sub.add_parser(name)
        p.add_argument("--network", choices=["mainnet", "testnet"], required=True)
        p.add_argument("--key", required=True, help="the scoped faucet key file setup.sh wrote")
        p.add_argument("--db", required=True)
        p.add_argument("--rpc", action="append", help="NEAR RPC URL (repeatable; default fastnear)")
        p.add_argument("--allow-origin", action="append", help="CORS origin (default https://dotx3.net)")
        p.add_argument("--host", default="127.0.0.1")
        p.add_argument("--port", type=int, default=8791)
        p.add_argument("--trust-cf-ip", action="store_true", help="use CF-Connecting-IP (only behind Cloudflare)")
        p.add_argument("--holders-per-client", type=int, default=3)
        p.add_argument("--forbidden-per-day", type=int, default=40)
    args = parser.parse_args(argv)
    logging.basicConfig(level=logging.INFO, format="%(asctime)s %(levelname)s %(name)s: %(message)s")
    config = Config(network=args.network, key_file=args.key, db_path=args.db, rpc_urls=args.rpc or [],
                    turnstile_secret=os.environ.get("TURNSTILE_SECRET") or None,
                    turnstile_site_key=os.environ.get("TURNSTILE_SITE_KEY") or None,
                    trust_cf_connecting_ip=args.trust_cf_ip, holders_per_client_per_day=args.holders_per_client,
                    forbidden_demos_per_day=args.forbidden_per_day)
    if args.allow_origin:
        config.allowed_origins = args.allow_origin
    try:
        faucet = Faucet(config)
        report = faucet.preflight()
    except (ValueError, NearError, OSError) as exc:
        print(f"refusing to start: {exc}", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2), file=sys.stderr)
    if args.cmd == "preflight":
        return 0
    server = FaucetServer((args.host, args.port), faucet)
    print(f"DX3HACK faucet on http://{args.host}:{server.server_address[1]} ({args.network})", file=sys.stderr)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
