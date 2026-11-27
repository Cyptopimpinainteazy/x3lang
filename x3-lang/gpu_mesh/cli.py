"""Command line for the X3 GPU mesh. Run from the repository with ``PYTHONPATH=x3-lang``:

    python3 -m gpu_mesh coordinator serve --db mesh.db --key coordinator.key
    python3 -m gpu_mesh worker run --key worker.key --coordinator https://gpu.x3star.net --label x3gpu1
    python3 -m gpu_mesh admin list --db mesh.db

Admin commands write straight to the coordinator's SQLite database, so they run on the
coordinator host and take effect at once. There is no admin HTTP endpoint to attack.
"""

from __future__ import annotations

import argparse
import json
import logging
import os
import sys
from datetime import datetime, timezone
from typing import List, Optional

from . import __version__, ledger
from .coordinator import Config, Coordinator, serve
from .protocol import Identity, ProtocolError
from .store import Store, parse_time_ms
from .worker import OllamaClient, Worker


def _print(data, as_json: bool) -> None:
    print(json.dumps(data, indent=2, sort_keys=True) if as_json else data)


def _when(ts: Optional[int]) -> str:
    if not ts:
        return "never"
    return datetime.fromtimestamp(ts, tz=timezone.utc).strftime("%Y-%m-%d %H:%M:%SZ")


def cmd_keygen(args) -> int:
    if os.path.exists(args.out):
        print(f"{args.out} already exists; refusing to overwrite a key", file=sys.stderr)
        return 1
    identity = Identity.generate()
    identity.save(args.out)
    print(identity.public_hex)
    return 0


def cmd_pubkey(args) -> int:
    print(Identity.load(args.key).public_hex)
    return 0


def cmd_serve(args) -> int:
    identity, created = Identity.load_or_create(args.key)
    if created:
        print(f"created coordinator key {args.key}", file=sys.stderr)
    store = Store(args.db)
    config = Config(job_timeout_s=args.job_timeout, canary_interval_s=args.canary_interval,
                    heartbeat_ttl_s=args.heartbeat_ttl)
    server = serve(Coordinator(store, identity, config), args.host, args.port)
    print(f"x3 gpu mesh coordinator {identity.public_hex} on http://{args.host}:{server.server_address[1]}",
          file=sys.stderr)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()
        store.close()
    return 0


def cmd_worker(args) -> int:
    identity, created = Identity.load_or_create(args.key)
    if created:
        print(f"created contributor key {args.key}", file=sys.stderr)
    print(f"contributor public key: {identity.public_hex}", file=sys.stderr)
    models = [m.strip() for m in args.models.split(",") if m.strip()] if args.models else None
    worker = Worker(identity, args.coordinator, OllamaClient(args.ollama, allow_http=True), label=args.label,
                    payout_account=args.payout_account, models=models, max_concurrency=args.max_concurrency,
                    allow_http=args.allow_http)
    worker.run_forever()
    return 0


def cmd_list(args) -> int:
    store = Store(args.db)
    rows = store.workers()
    if args.json:
        _print(rows, True)
        return 0
    if not rows:
        print("no contributors yet")
    for w in rows:
        gpus = ", ".join(f"{g['name']} {g['vram_mb'] // 1024}GB" for g in w["gpus"]) or "no GPU reported"
        models = ", ".join(m["name"] for m in w["models"])
        print(f"{w['pubkey']}  {w['label']}\n  {w['tier']:<6} {w['status']:<9} {w['status_reason']}\n"
              f"  gpus: {gpus}\n  models: {models}\n  jobs ok {w['jobs_ok']} failed {w['jobs_failed']}, "
              f"canaries {w['canary_pass']}/{w['canary_pass'] + w['canary_fail']}, "
              f"last heartbeat {_when(w['last_heartbeat'])}, payout {w['payout_account'] or '-'}")
    return 0


def _status(args, status: str) -> int:
    store = Store(args.db)
    info = store.set_status(args.pubkey, status, args.reason or f"set {status} by owner")
    print(f"{info['pubkey']} is now {info['status']} ({info['tier']})")
    return 0


def cmd_enroll(args) -> int:
    Store(args.db).enroll_fleet(args.pubkey, args.label)
    print(f"{args.pubkey} enrolled as fleet ({args.label}); it is approved when it registers")
    return 0


def cmd_tier(args) -> int:
    info = Store(args.db).set_tier(args.pubkey, args.tier)
    print(f"{info['pubkey']} tier is now {info['tier']}")
    return 0


def cmd_app_add(args) -> int:
    key = Store(args.db).add_app_key(args.name, allow_public=args.allow_public, requests_per_minute=args.rpm,
                                     daily_token_budget=args.daily_tokens, max_tokens_cap=args.max_tokens)
    print(key)
    print(f"app key for {args.name!r} (shown once; store it server-side only)", file=sys.stderr)
    return 0


def cmd_app_revoke(args) -> int:
    Store(args.db).revoke_app_key(args.name)
    print(f"revoked the key for {args.name!r}")
    return 0


def cmd_app_list(args) -> int:
    store = Store(args.db)
    rows = store.apps()
    for row in rows:
        row["tokens_used_today"] = store.tokens_used_today(row["name"])
    if args.json:
        _print(rows, True)
        return 0
    for row in rows:
        state = "revoked" if row["revoked_at"] else "active"
        print(f"{row['name']:<16} {state:<8} public={'yes' if row['allow_public'] else 'no ':<4}"
              f"{row['requests_per_minute']}/min, {row['tokens_used_today']}/{row['daily_token_budget']} tokens "
              f"today, max_tokens {row['max_tokens_cap']}")
    return 0


def cmd_verify(args) -> int:
    store = Store(args.db)
    pubkey = store.meta_get("coordinator_pubkey")
    if args.key:
        pubkey = Identity.load(args.key).public_hex
    if not pubkey:
        print("no coordinator public key recorded; pass --key", file=sys.stderr)
        return 1
    result = store.verify_receipts(pubkey)
    _print(result, True)
    return 0 if result["ok"] else 2


def cmd_ledger(args) -> int:
    store = Store(args.db)
    identity = Identity.load(args.key)
    document = ledger.build(store, identity, since_ms=parse_time_ms(args.since) or 0,
                            until_ms=parse_time_ms(args.until))
    text = json.dumps(document, indent=2, sort_keys=True)
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(text + "\n")
        print(f"wrote {args.out} ({len(document['ledger']['contributors'])} contributors)", file=sys.stderr)
    else:
        print(text)
    return 0


def cmd_events(args) -> int:
    for event in reversed(Store(args.db).events(args.limit)):
        print(f"{_when(event['ts'])}  {event['kind']:<22} {event['subject']}  {json.dumps(event['detail'])}")
    return 0


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(prog="python3 -m gpu_mesh", description="X3 GPU contributor mesh")
    parser.add_argument("--version", action="version", version=f"x3-gpu-mesh {__version__}")
    parser.add_argument("-v", "--verbose", action="store_true")
    sub = parser.add_subparsers(dest="group", required=True)

    p = sub.add_parser("keygen", help="create an Ed25519 key file (mode 0600)")
    p.add_argument("--out", required=True)
    p.set_defaults(func=cmd_keygen)
    p = sub.add_parser("pubkey", help="print the public key of a key file")
    p.add_argument("--key", required=True)
    p.set_defaults(func=cmd_pubkey)

    coord = sub.add_parser("coordinator").add_subparsers(dest="cmd", required=True)
    p = coord.add_parser("serve", help="run the coordinator (bind loopback; put TLS in front)")
    p.add_argument("--db", required=True)
    p.add_argument("--key", required=True)
    p.add_argument("--host", default="127.0.0.1")
    p.add_argument("--port", type=int, default=8790)
    p.add_argument("--job-timeout", type=int, default=110)
    p.add_argument("--canary-interval", type=int, default=900)
    p.add_argument("--heartbeat-ttl", type=int, default=90)
    p.set_defaults(func=cmd_serve)

    work = sub.add_parser("worker").add_subparsers(dest="cmd", required=True)
    p = work.add_parser("run", help="lend this machine's GPU through the local Ollama")
    p.add_argument("--key", required=True)
    p.add_argument("--coordinator", required=True)
    p.add_argument("--ollama", default="http://127.0.0.1:11434")
    p.add_argument("--label", required=True)
    p.add_argument("--payout-account", help="NEAR account for contribution payouts")
    p.add_argument("--models", help="comma-separated allowlist of Ollama models to serve (default: all pulled)")
    p.add_argument("--max-concurrency", type=int, default=1)
    p.add_argument("--allow-http", action="store_true", help="allow a plain-http coordinator on a private network")
    p.set_defaults(func=cmd_worker)

    admin = sub.add_parser("admin").add_subparsers(dest="cmd", required=True)

    def with_db(name: str, help_text: str):
        q = admin.add_parser(name, help=help_text)
        q.add_argument("--db", required=True)
        return q

    q = with_db("list", "list contributors")
    q.add_argument("--json", action="store_true")
    q.set_defaults(func=cmd_list)
    q = with_db("enroll-fleet", "pre-approve one of your own machines by public key")
    q.add_argument("pubkey")
    q.add_argument("--label", required=True)
    q.set_defaults(func=cmd_enroll)
    for name, status in (("approve", "approved"), ("suspend", "suspended"), ("revoke", "revoked")):
        q = with_db(name, f"mark a contributor {status}")
        q.add_argument("pubkey")
        q.add_argument("--reason")
        q.set_defaults(func=lambda a, s=status: _status(a, s))
    q = with_db("set-tier", "move a contributor between fleet and public")
    q.add_argument("pubkey")
    q.add_argument("tier", choices=["fleet", "public"])
    q.set_defaults(func=cmd_tier)
    q = with_db("app-add", "create an app key (agents, ide, site, ...)")
    q.add_argument("name")
    q.add_argument("--allow-public", action="store_true",
                   help="let public contributors run this app's prompts (they can read them)")
    q.add_argument("--rpm", type=int, default=60, help="requests per minute")
    q.add_argument("--daily-tokens", type=int, default=2_000_000, help="prompt+completion tokens per UTC day")
    q.add_argument("--max-tokens", type=int, default=4096, help="cap on max_tokens per request")
    q.set_defaults(func=cmd_app_add)
    q = with_db("app-revoke", "revoke an app key")
    q.add_argument("name")
    q.set_defaults(func=cmd_app_revoke)
    q = with_db("app-list", "list app keys and today's usage")
    q.add_argument("--json", action="store_true")
    q.set_defaults(func=cmd_app_list)
    q = with_db("verify-receipts", "recompute the receipt chain and all signatures")
    q.add_argument("--key", help="coordinator key file (default: the public key recorded in the db)")
    q.set_defaults(func=cmd_verify)
    q = with_db("ledger", "export the signed contribution ledger for a period")
    q.add_argument("--key", required=True, help="coordinator key file")
    q.add_argument("--since", help="YYYY-MM-DD or ISO-8601 (UTC); default: all time")
    q.add_argument("--until", help="YYYY-MM-DD or ISO-8601 (UTC); default: now")
    q.add_argument("--out")
    q.set_defaults(func=cmd_ledger)
    q = with_db("events", "show the audit log")
    q.add_argument("--limit", type=int, default=50)
    q.set_defaults(func=cmd_events)
    return parser


def main(argv: Optional[List[str]] = None) -> int:
    args = build_parser().parse_args(argv)
    logging.basicConfig(level=logging.DEBUG if args.verbose else logging.INFO,
                        format="%(asctime)s %(levelname)s %(name)s: %(message)s")
    try:
        return args.func(args)
    except ProtocolError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
