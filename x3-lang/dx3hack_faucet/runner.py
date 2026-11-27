"""DX3HACK faucet runner: the backend behind dotx3.net/faucet.

It holds the faucet operator's **scoped** key (FunctionCall on the token, methods
``add_holder`` and ``faucet_claim`` only) and refuses to start with anything broader. For
each judge it allocates a holder name ``j<n>.<holder parent>``, sends ``add_holder`` and
``faucet_claim`` on mainnet, and returns the real transaction hashes. It can also send the
three requests the contract must refuse (a DEX, a lookalike parent, a nested subaccount),
so a judge can watch a refusal land on chain.

Every rule that matters is enforced by the contract, not here: the cooldown, the daily cap,
max supply, max holders, the subaccount rule and the burn date. The runner only adds abuse
limits (Turnstile, holders per client per day, a daily cap on refusal demos) so the 0.5 NEAR
gas allowance on the faucet key isn't wasted.

Holder accounts are names, not on-chain accounts: the contract registers their storage
itself and nobody holds a key for them, so judges can claim and inspect but not spend. That
is the point of a hackathon-only demo credit.
"""

from __future__ import annotations

import hashlib
import json
import logging
import secrets
import sqlite3
import threading
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any, Dict, List, Optional, Tuple
from urllib.parse import urlparse

from .near_tx import NearError, NearRpc, SigningKey, classify, encode_function_call, json_args, valid_account

log = logging.getLogger("dx3hack.faucet")

FAUCET_METHODS = {"add_holder", "faucet_claim"}
GAS = 30 * 10 ** 12
MAX_BODY = 8 * 1024
TURNSTILE_URL = "https://challenges.cloudflare.com/turnstile/v0/siteverify"
NETWORKS = {
    "mainnet": {"rpc": ["https://rpc.mainnet.fastnear.com", "https://free.rpc.fastnear.com"],
                "explorer": "https://nearblocks.io", "suffix": "near", "dex": "v2.ref-finance.near"},
    "testnet": {"rpc": ["https://rpc.testnet.fastnear.com", "https://rpc.testnet.near.org"],
                "explorer": "https://testnet.nearblocks.io", "suffix": "testnet", "dex": "v2.ref-finance.testnet"},
}
FORBIDDEN_CASES = ("dex", "lookalike", "nested")


class FaucetError(Exception):
    def __init__(self, message: str, status: int = 400, code: str = "bad_request"):
        super().__init__(message)
        self.status = status
        self.code = code


@dataclass
class Config:
    network: str
    key_file: str
    db_path: str
    token: str = ""
    holder_parent: str = ""
    rpc_urls: List[str] = field(default_factory=list)
    allowed_origins: List[str] = field(default_factory=lambda: ["https://dotx3.net", "https://www.dotx3.net"])
    turnstile_secret: Optional[str] = None
    turnstile_site_key: Optional[str] = None
    trust_cf_connecting_ip: bool = False
    holders_per_client_per_day: int = 3
    forbidden_demos_per_day: int = 40
    min_allowance_yocto: int = 20 * 10 ** 21  # stop at 0.02 NEAR left on the scoped key

    def __post_init__(self) -> None:
        if self.network not in NETWORKS:
            raise ValueError("network must be mainnet or testnet")
        net = NETWORKS[self.network]
        self.token = self.token or f"dx3hack.x3lang.{net['suffix']}"
        self.holder_parent = self.holder_parent or f"judges.x3lang.{net['suffix']}"
        self.rpc_urls = self.rpc_urls or list(net["rpc"])

    @property
    def explorer(self) -> str:
        return NETWORKS[self.network]["explorer"]


class Store:
    SCHEMA = """
    CREATE TABLE IF NOT EXISTS holders (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        account TEXT UNIQUE,
        client_hash TEXT NOT NULL,
        created_at INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS txs (
        seq INTEGER PRIMARY KEY AUTOINCREMENT,
        at INTEGER NOT NULL,
        purpose TEXT NOT NULL,
        method TEXT NOT NULL,
        args_json TEXT NOT NULL,
        tx_hash TEXT NOT NULL,
        outcome TEXT NOT NULL,
        reason TEXT,
        client_hash TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
    """

    def __init__(self, path: str):
        self._lock = threading.RLock()
        self._db = sqlite3.connect(path, check_same_thread=False, isolation_level=None, timeout=30)
        self._db.row_factory = sqlite3.Row
        self._db.execute("PRAGMA journal_mode=WAL")
        self._db.executescript(self.SCHEMA)
        row = self._db.execute("SELECT value FROM meta WHERE key = 'client_salt'").fetchone()
        if row is None:
            self._db.execute("INSERT INTO meta (key, value) VALUES ('client_salt', ?)", (secrets.token_hex(32),))
            row = self._db.execute("SELECT value FROM meta WHERE key = 'client_salt'").fetchone()
        self._salt = row["value"]

    def client_hash(self, client: str) -> str:
        """Clients are stored only as a salted hash of their IP address."""
        return hashlib.sha256(f"{self._salt}:{client}".encode()).hexdigest()

    def allocate_holder(self, parent: str, client_hash: str, per_day: int) -> str:
        with self._lock:
            self._db.execute("BEGIN IMMEDIATE")
            try:
                since = int(time.time()) - 86400
                count = self._db.execute("SELECT COUNT(*) AS n FROM holders WHERE client_hash = ? AND created_at >= ?",
                                         (client_hash, since)).fetchone()["n"]
                if count >= per_day:
                    raise FaucetError(f"this network already got {count} holder accounts today; try again tomorrow",
                                      429, "client_limit")
                cur = self._db.execute("INSERT INTO holders (client_hash, created_at) VALUES (?, ?)",
                                       (client_hash, int(time.time())))
                account = f"j{cur.lastrowid}.{parent}"
                self._db.execute("UPDATE holders SET account = ? WHERE seq = ?", (account, cur.lastrowid))
                self._db.execute("COMMIT")
                return account
            except BaseException:
                self._db.execute("ROLLBACK")
                raise

    def is_ours(self, account: str) -> bool:
        with self._lock:
            return self._db.execute("SELECT 1 FROM holders WHERE account = ?", (account,)).fetchone() is not None

    def forbidden_today(self) -> int:
        with self._lock:
            return self._db.execute("SELECT COUNT(*) AS n FROM txs WHERE purpose = 'forbidden' AND at >= ?",
                                    (int(time.time()) - 86400,)).fetchone()["n"]

    def record_tx(self, purpose: str, method: str, args: Dict[str, Any], tx_hash: str, outcome: str,
                  reason: Optional[str], client_hash: str) -> None:
        with self._lock:
            self._db.execute("INSERT INTO txs (at, purpose, method, args_json, tx_hash, outcome, reason, client_hash) "
                             "VALUES (?, ?, ?, ?, ?, ?, ?, ?)", (int(time.time()), purpose, method,
                                                                 json.dumps(args, sort_keys=True), tx_hash, outcome,
                                                                 reason, client_hash))

    def feed(self, limit: int = 50) -> List[Dict[str, Any]]:
        with self._lock:
            rows = self._db.execute("SELECT seq, at, purpose, method, args_json, tx_hash, outcome, reason FROM txs "
                                    "ORDER BY seq DESC LIMIT ?", (limit,)).fetchall()
        return [{"seq": r["seq"], "at": r["at"], "purpose": r["purpose"], "method": r["method"],
                 "args": json.loads(r["args_json"]), "tx_hash": r["tx_hash"], "outcome": r["outcome"],
                 "reason": r["reason"]} for r in rows]


class Faucet:
    def __init__(self, config: Config, *, rpc: Optional[NearRpc] = None, key: Optional[SigningKey] = None,
                 http_post=None):
        self.config = config
        self.key = key or SigningKey.from_file(config.key_file)
        self.rpc = rpc or NearRpc(config.rpc_urls)
        self.store = Store(config.db_path)
        self._http_post = http_post or _form_post
        self._send_lock = threading.Lock()
        self._config_cache: Tuple[float, Optional[Dict[str, Any]]] = (0.0, None)
        self.paused_reason: Optional[str] = None
        if config.network == "mainnet" and not config.turnstile_secret:
            raise ValueError("mainnet needs a Turnstile secret (TURNSTILE_SECRET): the faucet is public")

    # ---------------------------------------------------------------- checks

    def preflight(self) -> Dict[str, Any]:
        """Refuse to run unless the key is exactly the contract's scoped faucet key."""
        cfg = self.hackathon_config(force=True)
        if cfg["faucet_operator_id"] != self.key.account_id:
            raise ValueError(f"the contract's faucet operator is {cfg['faucet_operator_id']}, "
                             f"but the key file is for {self.key.account_id}")
        if cfg["holder_parent_id"] != self.config.holder_parent:
            raise ValueError(f"the contract's holder parent is {cfg['holder_parent_id']}, "
                             f"not {self.config.holder_parent}")
        access = self.rpc.view_access_key(self.key.account_id, self.key.public_key)
        permission = access.get("permission")
        call = permission.get("FunctionCall") if isinstance(permission, dict) else None
        if call is None:
            raise ValueError("the key file holds a FullAccess key; the runner only runs with the scoped faucet key "
                             "setup.sh created (add_holder and faucet_claim on the token)")
        if call.get("receiver_id") != self.config.token or set(call.get("method_names") or []) != FAUCET_METHODS:
            raise ValueError(f"the key is scoped to {call.get('receiver_id')} {call.get('method_names')}; expected "
                             f"{self.config.token} {sorted(FAUCET_METHODS)}")
        allowance = int(call.get("allowance") or 0)
        self._check_allowance(allowance)
        return {"operator": self.key.account_id, "public_key": self.key.public_key, "token": self.config.token,
                "allowance_yocto": str(allowance), "active": cfg["active"], "paused": cfg["paused"]}

    def _check_allowance(self, allowance: int) -> None:
        if allowance and allowance < self.config.min_allowance_yocto:
            self.paused_reason = "the faucet key's gas allowance is nearly spent"
            raise FaucetError(self.paused_reason, 503, "low_gas")

    def hackathon_config(self, force: bool = False) -> Dict[str, Any]:
        at, cached = self._config_cache
        if not force and cached is not None and time.time() - at < 20:
            return cached
        cfg = self.rpc.view(self.config.token, "hackathon_config", {})
        if not isinstance(cfg, dict):
            raise NearError("hackathon_config returned nothing", "BAD_RESPONSE")
        self._config_cache = (time.time(), cfg)
        return cfg

    def _require_live(self) -> Dict[str, Any]:
        if self.paused_reason:
            raise FaucetError(self.paused_reason, 503, "paused")
        cfg = self.hackathon_config()
        if not cfg["active"]:
            raise FaucetError("the hackathon is over and DX3HACK has burned", 410, "burned")
        if cfg["paused"]:
            raise FaucetError("DX3HACK is paused by the owner", 503, "paused")
        return cfg

    def verify_turnstile(self, token: Any, client_ip: str) -> None:
        if not self.config.turnstile_secret:
            return
        if not isinstance(token, str) or not 0 < len(token) <= 2048:
            raise FaucetError("complete the human check first", 403, "turnstile_failed")
        try:
            data = self._http_post(TURNSTILE_URL, {"secret": self.config.turnstile_secret, "response": token,
                                                   "remoteip": client_ip})
        except OSError as exc:
            raise FaucetError("the human check could not be verified right now", 503, "turnstile_unavailable") from exc
        if not isinstance(data, dict) or data.get("success") is not True:
            raise FaucetError("the human check failed; reload and try again", 403, "turnstile_failed")

    # ---------------------------------------------------------------- transactions

    def _send(self, method: str, args: Dict[str, Any], purpose: str, client_hash: str) -> Dict[str, Any]:
        with self._send_lock:
            for attempt in range(2):
                access = self.rpc.view_access_key(self.key.account_id, self.key.public_key)
                call = (access.get("permission") or {}).get("FunctionCall") if isinstance(access.get("permission"),
                                                                                          dict) else None
                if call is None:
                    raise FaucetError("the faucet key is no longer a scoped FunctionCall key; stopping", 503,
                                      "key_changed")
                self._check_allowance(int(call.get("allowance") or 0))
                tx = encode_function_call(signer_id=self.key.account_id, public_key=self.key.public_key,
                                          nonce=int(access["nonce"]) + 1, receiver_id=self.config.token,
                                          block_hash=access["block_hash"], method_name=method, args=json_args(args),
                                          gas=GAS, deposit=0)
                signed, tx_hash = self.key.sign_transaction(tx)
                try:
                    result = self.rpc.send_tx(signed)
                except NearError as exc:
                    if exc.name == "INVALID_TRANSACTION" and "Nonce" in str(exc) and attempt == 0:
                        continue
                    if exc.name in ("TIMEOUT_ERROR", "UNREACHABLE"):
                        # It may still land. Record it so the hash can be checked later; never resend blindly.
                        self.store.record_tx(purpose, method, args, tx_hash, "unknown", str(exc)[:300], client_hash)
                        raise FaucetError(f"NEAR didn't confirm in time; check {self.config.explorer}/txns/{tx_hash}",
                                          504, "unconfirmed") from exc
                    raise FaucetError(f"NEAR refused the transaction before execution: {exc}", 502, "rpc") from exc
                outcome, reason, value = classify(result)
                self.store.record_tx(purpose, method, args, tx_hash, outcome, reason, client_hash)
                return {"method": method, "args": args, "tx_hash": tx_hash, "outcome": outcome, "reason": reason,
                        "value": value, "url": f"{self.config.explorer}/txns/{tx_hash}"}
        raise FaucetError("could not get a fresh nonce", 502, "rpc")

    # ---------------------------------------------------------------- API

    def status(self) -> Dict[str, Any]:
        cfg = self.hackathon_config()
        return {"network": self.config.network, "token": self.config.token, "operator": self.key.account_id,
                "holder_parent": self.config.holder_parent, "explorer": self.config.explorer,
                "turnstile_site_key": self.config.turnstile_site_key, "paused_reason": self.paused_reason,
                "contract": cfg}

    def holder(self, account: str) -> Dict[str, Any]:
        if not valid_account(account) or not account.endswith("." + self.config.holder_parent):
            raise FaucetError("not a faucet holder account", 404, "not_found")
        balance = self.rpc.view(self.config.token, "ft_balance_of", {"account_id": account})
        available = int(self.rpc.view(self.config.token, "claim_available_at", {"account_id": account}) or 0)
        return {"account": account,
                "is_holder": self.rpc.view(self.config.token, "is_holder", {"account_id": account}),
                "balance": balance,
                "claim_available_at_ns": str(available),
                "claim_wait_sec": self._wait_sec(available),
                "transfers_left_today": self.rpc.view(self.config.token, "transfers_left_today",
                                                      {"account_id": account}),
                "url": f"{self.config.explorer}/address/{account}"}

    def _wait_sec(self, available_ns: int) -> int:
        """Seconds until ``available_ns``, measured on the chain's clock (the contract's ``now``),
        so the runner host's clock never matters."""
        if not available_ns:
            return 0
        now_ns = int(self.hackathon_config(force=True).get("now") or time.time_ns())
        return max(0, -(-(available_ns - now_ns) // 10 ** 9))

    def claim(self, body: Dict[str, Any], client_ip: str) -> Dict[str, Any]:
        self._require_live()
        self.verify_turnstile(body.get("turnstileToken"), client_ip)
        client = self.store.client_hash(client_ip)
        requested = body.get("holder")
        txs = []
        if requested is not None:
            if not (isinstance(requested, str) and self.store.is_ours(requested)):
                raise FaucetError("unknown holder; claim without one to get a new account", 404, "not_found")
            account = requested
            available = int(self.rpc.view(self.config.token, "claim_available_at", {"account_id": account}) or 0)
            wait = self._wait_sec(available)
            if wait:
                raise FaucetError(f"{account} claimed recently; it can claim again in {-(-wait // 60)} minutes",
                                  429, "cooldown")
        else:
            account = self.store.allocate_holder(self.config.holder_parent, client,
                                                 self.config.holders_per_client_per_day)
            added = self._send("add_holder", {"account_id": account}, "add_holder", client)
            txs.append(added)
            if added["outcome"] != "success":
                raise FaucetError(f"add_holder was refused on chain: {added['reason']}", 502, "add_holder_refused")
        claimed = self._send("faucet_claim", {"receiver_id": account}, "claim", client)
        txs.append(claimed)
        result = {"holder": account, "txs": txs, "claimed": claimed["outcome"] == "success",
                  "reason": claimed["reason"]}
        try:
            result["state"] = self.holder(account)
        except NearError:
            result["state"] = None
        return result

    def forbidden(self, body: Dict[str, Any], client_ip: str) -> Dict[str, Any]:
        """Send a request the contract must refuse, and return the on-chain refusal."""
        self._require_live()
        case = body.get("case")
        if case not in FORBIDDEN_CASES:
            raise FaucetError(f"case must be one of {', '.join(FORBIDDEN_CASES)}")
        self.verify_turnstile(body.get("turnstileToken"), client_ip)
        if self.store.forbidden_today() >= self.config.forbidden_demos_per_day:
            raise FaucetError("today's refusal demos are used up; see the feed for earlier refusals", 429,
                              "demo_limit")
        parent = self.config.holder_parent
        target = {"dex": NETWORKS[self.config.network]["dex"],
                  "lookalike": "evil-" + parent,
                  "nested": f"pool.j1.{parent}"}[case]
        sent = self._send("add_holder", {"account_id": target}, "forbidden", self.store.client_hash(client_ip))
        sent["case"] = case
        sent["expected"] = "refused"
        sent["as_expected"] = sent["outcome"] == "refused"
        if not sent["as_expected"]:
            log.error("SECURITY: forbidden add_holder %s was not refused (%s)", target, sent["tx_hash"])
        return sent


def _form_post(url: str, fields: Dict[str, str]) -> Any:
    data = urllib.parse.urlencode(fields).encode()
    req = urllib.request.Request(url, data=data, method="POST")
    with urllib.request.urlopen(req, timeout=10) as resp:  # noqa: S310 - fixed Cloudflare URL
        return json.loads(resp.read(64 * 1024).decode())


# --------------------------------------------------------------------------------------------
# HTTP
# --------------------------------------------------------------------------------------------

class FaucetServer(ThreadingHTTPServer):
    daemon_threads = True
    allow_reuse_address = True

    def __init__(self, address: Tuple[str, int], faucet: Faucet):
        super().__init__(address, FaucetHandler)
        self.faucet = faucet


class FaucetHandler(BaseHTTPRequestHandler):
    server: FaucetServer
    server_version = "dx3hack-faucet"
    sys_version = ""
    protocol_version = "HTTP/1.1"
    timeout = 30

    def log_message(self, fmt: str, *args: Any) -> None:
        log.info("%s", fmt % args)

    def _client_ip(self) -> str:
        if self.server.faucet.config.trust_cf_connecting_ip:
            forwarded = self.headers.get("CF-Connecting-IP")
            if forwarded:
                return forwarded.strip()[:64]
        return self.client_address[0]

    def _cors(self) -> None:
        origin = self.headers.get("Origin")
        if origin and origin in self.server.faucet.config.allowed_origins:
            self.send_header("Access-Control-Allow-Origin", origin)
            self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
            self.send_header("Access-Control-Allow-Headers", "Content-Type")
            self.send_header("Access-Control-Max-Age", "600")
        self.send_header("Vary", "Origin")

    def _json(self, status: int, payload: Any) -> None:
        body = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.send_header("X-Content-Type-Options", "nosniff")
        self._cors()
        self.end_headers()
        self.wfile.write(body)

    def _error(self, exc: FaucetError) -> None:
        self.close_connection = True
        self._json(exc.status, {"error": {"code": exc.code, "message": str(exc)}})

    def do_OPTIONS(self) -> None:  # noqa: N802
        self.send_response(204)
        self._cors()
        self.send_header("Content-Length", "0")
        self.end_headers()

    def do_GET(self) -> None:  # noqa: N802
        path = urlparse(self.path).path
        faucet = self.server.faucet
        try:
            if path == "/v1/faucet/status":
                self._json(200, faucet.status())
            elif path == "/v1/faucet/feed":
                self._json(200, {"txs": faucet.store.feed()})
            elif path.startswith("/v1/faucet/holders/"):
                self._json(200, faucet.holder(urllib.parse.unquote(path[len("/v1/faucet/holders/"):])))
            elif path == "/healthz":
                self._json(200, {"ok": True})
            else:
                raise FaucetError("not found", 404, "not_found")
        except FaucetError as exc:
            self._error(exc)
        except NearError as exc:
            self._error(FaucetError(f"NEAR RPC is unavailable: {exc}", 502, "rpc"))

    def do_POST(self) -> None:  # noqa: N802
        path = urlparse(self.path).path
        faucet = self.server.faucet
        try:
            length = int(self.headers.get("Content-Length") or -1)
            if not 0 <= length <= MAX_BODY:
                raise FaucetError("body missing or too large", 413, "too_large")
            try:
                body = json.loads(self.rfile.read(length) or b"{}")
            except json.JSONDecodeError:
                raise FaucetError("body is not JSON") from None
            if not isinstance(body, dict):
                raise FaucetError("body must be a JSON object")
            if path == "/v1/faucet/claim":
                self._json(200, faucet.claim(body, self._client_ip()))
            elif path == "/v1/faucet/forbidden":
                self._json(200, faucet.forbidden(body, self._client_ip()))
            else:
                raise FaucetError("not found", 404, "not_found")
        except FaucetError as exc:
            self._error(exc)
        except NearError as exc:
            self._error(FaucetError(f"NEAR RPC is unavailable: {exc}", 502, "rpc"))
        except Exception:  # pragma: no cover - defensive
            log.exception("POST %s failed", path)
            self._error(FaucetError("internal error", 500, "internal"))
