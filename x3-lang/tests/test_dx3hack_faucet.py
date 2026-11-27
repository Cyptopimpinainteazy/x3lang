"""DX3HACK faucet runner.

* The transaction encoder is checked byte for byte against vectors produced by the official
  ``@near-js/transactions`` library (``fixtures/dx3hack_near_js_vectors.js`` regenerates them).
* The runner is driven over real HTTP against a test-only NEAR RPC stand-in that decodes every
  signed transaction, verifies its Ed25519 signature and nonce independently, and applies the
  DX3HACK contract's rules (subaccount check, cooldown, daily cap). It is not the contract;
  the contract's own 39 Rust tests cover the contract.
"""

from __future__ import annotations

import base64
import hashlib
import json
import os
import struct
import threading
import urllib.error
import urllib.request

import pytest
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey

from dx3hack_faucet.near_tx import (NearError, NearRpc, SigningKey, classify, encode_function_call, json_args)
from dx3hack_faucet.runner import Config, Faucet, FaucetError, FaucetServer
from ed25519 import b58decode, b58encode

HERE = os.path.dirname(os.path.abspath(__file__))
VECTORS = json.load(open(os.path.join(HERE, "fixtures", "dx3hack_near_js_vectors.json")))
SEED = bytes.fromhex(VECTORS["seed_hex"])
TOKEN, PARENT, OPERATOR = "dx3hack.x3lang.testnet", "judges.x3lang.testnet", "faucet.x3lang.testnet"


def test_encoder_matches_near_js_byte_for_byte():
    key = SigningKey("faucet.x3lang.near", SEED)
    for case in VECTORS["cases"]:
        assert key.public_key == case["public_key"]
        tx = encode_function_call(signer_id=case["signer"], public_key=key.public_key, nonce=int(case["nonce"]),
                                  receiver_id=case["receiver"], block_hash=case["block_hash"],
                                  method_name=case["method"], args=json_args(case["args"]), gas=int(case["gas"]),
                                  deposit=int(case["deposit"]))
        signed, tx_hash = key.sign_transaction(tx)
        assert tx_hash == case["tx_hash"]
        assert signed == case["signed_tx_base64"]


def test_key_file_must_be_self_consistent(tmp_path):
    key = SigningKey(OPERATOR, SEED)
    pub = b58decode(key.public_key[8:])
    good = tmp_path / "k.json"
    good.write_text(json.dumps({"account_id": OPERATOR, "public_key": key.public_key,
                                "private_key": "ed25519:" + b58encode(SEED + pub)}))
    assert SigningKey.from_file(str(good)).public_key == key.public_key
    bad = tmp_path / "bad.json"
    bad.write_text(json.dumps({"account_id": OPERATOR, "public_key": key.public_key,
                               "private_key": "ed25519:" + b58encode(SEED + b"\x01" * 32)}))
    with pytest.raises(ValueError):
        SigningKey.from_file(str(bad))


def test_classify_reads_the_contract_panic_from_receipts():
    refused = {"status": {"Failure": {"ActionError": {"index": 0, "kind": {"FunctionCallError": {
        "ExecutionError": "Smart contract panicked: This holder claimed recently. Try again later."}}}}},
        "transaction": {"hash": "x"}, "receipts_outcome": []}
    assert classify(refused) == ("refused", "This holder claimed recently. Try again later.", None)
    ok = {"status": {"SuccessValue": base64.b64encode(b'"100000000"').decode()}, "receipts_outcome": [
        {"outcome": {"status": {"SuccessValue": ""}}}]}
    assert classify(ok) == ("success", None, "100000000")
    hidden = {"status": {"SuccessValue": ""}, "receipts_outcome": [{"outcome": {"status": {"Failure": {
        "ActionError": {"kind": {"FunctionCallError": {"ExecutionError": "Smart contract panicked: boom"}}}}}}}]}
    assert classify(hidden)[:2] == ("refused", "boom")
    assert classify({"status": {}})[0] == "unknown"


# --------------------------------------------------------------------------------------------
# Test-only NEAR RPC stand-in
# --------------------------------------------------------------------------------------------

def _decode_tx(raw: bytes):
    pos = 0

    def take(n):
        nonlocal pos
        out = raw[pos:pos + n]
        pos += n
        return out

    def string():
        (n,) = struct.unpack("<I", take(4))
        return take(n).decode()

    signer = string()
    assert take(1) == b"\x00"
    pub = take(32)
    (nonce,) = struct.unpack("<Q", take(8))
    receiver = string()
    take(32)
    (count,) = struct.unpack("<I", take(4))
    assert count == 1 and take(1) == b"\x02"
    method = string()
    (alen,) = struct.unpack("<I", take(4))
    args = json.loads(take(alen))
    (gas,) = struct.unpack("<Q", take(8))
    deposit = int.from_bytes(take(16), "little")
    body = raw[:pos]
    assert take(1) == b"\x00"
    sig = take(64)
    assert pos == len(raw)
    return dict(signer=signer, pub=pub, nonce=nonce, receiver=receiver, method=method, args=args, gas=gas,
                deposit=deposit, body=body, sig=sig)


class FakeChain:
    """Enough of NEAR RPC and the DX3HACK contract to drive the runner. Test-only."""

    def __init__(self, key: SigningKey, permission=None):
        self.key = key
        self.nonce = 1000
        self.permission = permission if permission is not None else {"FunctionCall": {
            "allowance": str(5 * 10 ** 23), "receiver_id": TOKEN, "method_names": ["add_holder", "faucet_claim"]}}
        self.holders, self.balances, self.last_claim = set(), {}, {}
        self.now_ns = 1_800_000_000 * 10 ** 9
        self.sent = []
        self.active, self.paused = True, False

    def __call__(self, url, body, timeout):
        req = json.loads(body)
        method, params = req["method"], req["params"]
        if method == "query" and params["request_type"] == "view_access_key":
            assert params["public_key"] == self.key.public_key
            return 200, json.dumps({"result": {"nonce": self.nonce, "block_hash": b58encode(b"\x05" * 32),
                                               "permission": self.permission}}).encode()
        if method == "query" and params["request_type"] == "call_function":
            args = json.loads(base64.b64decode(params["args_base64"]))
            value = self.view(params["method_name"], args)
            return 200, json.dumps({"result": {"result": list(json.dumps(value).encode())}}).encode()
        if method == "send_tx":
            return 200, json.dumps(self.send(base64.b64decode(params["signed_tx_base64"]))).encode()
        return 200, json.dumps({"error": {"name": "UNKNOWN_METHOD"}}).encode()

    def view(self, name, args):
        if name == "hackathon_config":
            return {"owner_id": "x3lang.testnet", "faucet_operator_id": OPERATOR, "holder_parent_id": PARENT,
                    "now": str(self.now_ns),
                    "active": self.active, "paused": self.paused, "holders": len(self.holders), "max_holders": 500,
                    "claim_amount": "100000000", "claim_cooldown_sec": 3600}
        if name == "ft_balance_of":
            return str(self.balances.get(args["account_id"], 0))
        if name == "is_holder":
            return args["account_id"] in self.holders
        if name == "claim_available_at":
            last = self.last_claim.get(args["account_id"])
            return str(last + 3600 * 10 ** 9) if last and self.now_ns < last + 3600 * 10 ** 9 else "0"
        if name == "transfers_left_today":
            return 20
        raise AssertionError(name)

    def send(self, raw):
        tx = _decode_tx(raw)
        digest = hashlib.sha256(tx["body"]).digest()
        Ed25519PublicKey.from_public_bytes(tx["pub"]).verify(tx["sig"], digest)  # raises if forged
        assert "ed25519:" + b58encode(tx["pub"]) == self.key.public_key and tx["signer"] == OPERATOR
        if tx["nonce"] <= self.nonce:
            return {"error": {"name": "INVALID_TRANSACTION", "cause": {"name": "INVALID_TRANSACTION",
                                                                        "info": "InvalidNonce"}}}
        self.nonce = tx["nonce"]
        assert tx["receiver"] == TOKEN and tx["deposit"] == 0
        self.sent.append(tx)
        panic = None
        account = tx["args"].get("account_id") or tx["args"].get("receiver_id")
        if tx["method"] == "add_holder":
            label = account[:-len("." + PARENT)] if account.endswith("." + PARENT) else ""
            if not label or "." in label:
                panic = "The faucet operator can only add direct subaccounts of the holder parent"
            else:
                self.holders.add(account)
        elif tx["method"] == "faucet_claim":
            last = self.last_claim.get(account)
            if last and self.now_ns < last + 3600 * 10 ** 9:
                panic = "This holder claimed recently. Try again later."
            else:
                self.last_claim[account] = self.now_ns
                self.balances[account] = self.balances.get(account, 0) + 100_000_000
        tx_hash = b58encode(digest)
        if panic:
            status = {"Failure": {"ActionError": {"index": 0, "kind": {"FunctionCallError": {
                "ExecutionError": f"Smart contract panicked: {panic}"}}}}}
        else:
            status = {"SuccessValue": base64.b64encode(b'"100000000"' if tx["method"] == "faucet_claim"
                                                       else b"").decode()}
        return {"result": {"status": status, "transaction": {"hash": tx_hash}, "receipts_outcome": []}}


@pytest.fixture
def faucet(tmp_path):
    key = SigningKey(OPERATOR, SEED)
    chain = FakeChain(key)
    config = Config(network="testnet", key_file="unused", db_path=str(tmp_path / "faucet.db"),
                    allowed_origins=["https://dotx3.net"], holders_per_client_per_day=2, forbidden_demos_per_day=3)
    f = Faucet(config, rpc=NearRpc(["https://rpc.test"], transport=chain), key=key)
    f.chain = chain
    server = FaucetServer(("127.0.0.1", 0), f)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    f.url = f"http://127.0.0.1:{server.server_address[1]}"
    yield f
    server.shutdown()
    server.server_close()


def call(f, method, path, body=None, origin="https://dotx3.net"):
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Content-Type": "application/json", "Origin": origin}
    req = urllib.request.Request(f.url + path, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=10) as resp:
            return resp.status, json.loads(resp.read() or b"null"), resp.headers
    except urllib.error.HTTPError as exc:
        return exc.code, json.loads(exc.read() or b"null"), exc.headers


def test_preflight_accepts_only_the_scoped_faucet_key(faucet, tmp_path):
    assert faucet.preflight()["operator"] == OPERATOR
    faucet.chain.permission = "FullAccess"
    with pytest.raises(ValueError, match="FullAccess"):
        faucet.preflight()
    faucet.chain.permission = {"FunctionCall": {"allowance": None, "receiver_id": TOKEN,
                                                "method_names": ["add_holder", "faucet_claim", "mint"]}}
    with pytest.raises(ValueError, match="scoped to"):
        faucet.preflight()


def test_mainnet_refuses_to_start_without_turnstile(tmp_path):
    with pytest.raises(ValueError, match="Turnstile"):
        Faucet(Config(network="mainnet", key_file="x", db_path=str(tmp_path / "m.db")),
               rpc=NearRpc(["https://x"]), key=SigningKey("faucet.x3lang.near", SEED))


def test_claim_creates_a_holder_and_mints_with_real_signed_transactions(faucet):
    status, data, headers = call(faucet, "POST", "/v1/faucet/claim", {})
    assert status == 200, data
    assert headers["Access-Control-Allow-Origin"] == "https://dotx3.net"
    assert data["holder"] == f"j1.{PARENT}" and data["claimed"] is True
    assert [t["method"] for t in data["txs"]] == ["add_holder", "faucet_claim"]
    assert all(t["outcome"] == "success" and t["url"].startswith("https://testnet.nearblocks.io/txns/")
               for t in data["txs"])
    assert data["state"]["balance"] == "100000000" and data["state"]["is_holder"] is True
    assert [tx["nonce"] for tx in faucet.chain.sent] == [1001, 1002]

    assert data["state"]["claim_wait_sec"] == 3600, "measured on the chain's clock, not the runner host's"
    status, data, _ = call(faucet, "POST", "/v1/faucet/claim", {"holder": f"j1.{PARENT}"})
    assert status == 429 and data["error"]["code"] == "cooldown", "the runner checks the cooldown before paying gas"
    assert "in 60 minutes" in data["error"]["message"]
    faucet.chain.now_ns += 3601 * 10 ** 9
    status, data, _ = call(faucet, "POST", "/v1/faucet/claim", {"holder": f"j1.{PARENT}"})
    assert status == 200 and [t["method"] for t in data["txs"]] == ["faucet_claim"]
    assert faucet.chain.balances[f"j1.{PARENT}"] == 200_000_000


def test_client_limit_and_unknown_holders(faucet):
    assert call(faucet, "POST", "/v1/faucet/claim", {})[0] == 200
    assert call(faucet, "POST", "/v1/faucet/claim", {})[0] == 200
    status, data, _ = call(faucet, "POST", "/v1/faucet/claim", {})
    assert status == 429 and data["error"]["code"] == "client_limit"
    status, data, _ = call(faucet, "POST", "/v1/faucet/claim", {"holder": "v2.ref-finance.testnet"})
    assert status == 404


def test_forbidden_requests_are_refused_on_chain_and_recorded(faucet):
    results = {}
    for case in ("dex", "lookalike", "nested"):
        status, data, _ = call(faucet, "POST", "/v1/faucet/forbidden", {"case": case})
        assert status == 200, data
        results[case] = data
    assert all(r["outcome"] == "refused" and r["as_expected"] for r in results.values())
    assert results["dex"]["args"] == {"account_id": "v2.ref-finance.testnet"}
    assert results["lookalike"]["args"] == {"account_id": f"evil-{PARENT}"}
    assert "only add direct subaccounts" in results["nested"]["reason"]
    status, data, _ = call(faucet, "POST", "/v1/faucet/forbidden", {"case": "dex"})
    assert status == 429 and data["error"]["code"] == "demo_limit"
    assert call(faucet, "POST", "/v1/faucet/forbidden", {"case": "mint"})[0] == 400
    status, feed, _ = call(faucet, "GET", "/v1/faucet/feed")
    assert [t["purpose"] for t in feed["txs"]] == ["forbidden"] * 3
    assert all(len(t["tx_hash"]) >= 43 for t in feed["txs"])


def test_status_holder_view_cors_and_paused_contract(faucet):
    status, data, _ = call(faucet, "GET", "/v1/faucet/status")
    assert status == 200 and data["contract"]["holder_parent_id"] == PARENT
    _, _, headers = call(faucet, "GET", "/v1/faucet/status", origin="https://evil.example")
    assert headers.get("Access-Control-Allow-Origin") is None
    assert call(faucet, "GET", "/v1/faucet/holders/v2.ref-finance.testnet")[0] == 404
    faucet.chain.paused = True
    faucet._config_cache = (0.0, None)
    status, data, _ = call(faucet, "POST", "/v1/faucet/claim", {})
    assert status == 503 and data["error"]["code"] == "paused"
    faucet.chain.active, faucet.chain.paused = False, False
    faucet._config_cache = (0.0, None)
    assert call(faucet, "POST", "/v1/faucet/claim", {})[0] == 410


def test_turnstile_is_enforced_when_configured(tmp_path):
    key = SigningKey(OPERATOR, SEED)
    seen = []

    def turnstile(url, fields):
        seen.append(fields)
        return {"success": fields["response"] == "good-token"}

    f = Faucet(Config(network="testnet", key_file="x", db_path=str(tmp_path / "t.db"), turnstile_secret="s3cret"),
               rpc=NearRpc(["https://rpc.test"], transport=FakeChain(key)), key=key, http_post=turnstile)
    with pytest.raises(FaucetError) as exc:
        f.claim({}, "203.0.113.9")
    assert exc.value.code == "turnstile_failed"
    with pytest.raises(FaucetError):
        f.claim({"turnstileToken": "bad"}, "203.0.113.9")
    assert f.claim({"turnstileToken": "good-token"}, "203.0.113.9")["claimed"] is True
    assert seen[-1] == {"secret": "s3cret", "response": "good-token", "remoteip": "203.0.113.9"}


def test_low_gas_allowance_pauses_the_faucet(faucet):
    faucet.chain.permission["FunctionCall"]["allowance"] = str(10 ** 21)
    status, data, _ = call(faucet, "POST", "/v1/faucet/claim", {})
    assert status == 503 and data["error"]["code"] == "low_gas"
    assert call(faucet, "POST", "/v1/faucet/claim", {})[1]["error"]["code"] == "paused"


def test_rpc_errors_surface_without_resending(faucet):
    def down(url, body, timeout):
        if json.loads(body)["method"] == "send_tx":
            return 200, json.dumps({"error": {"name": "HANDLER_ERROR", "cause": {"name": "TIMEOUT_ERROR",
                                                                                  "info": {}}}}).encode()
        return faucet.chain(url, body, timeout)

    faucet.rpc.transport = down
    status, data, _ = call(faucet, "POST", "/v1/faucet/claim", {})
    assert status == 504 and data["error"]["code"] == "unconfirmed"
    assert faucet.store.feed()[0]["outcome"] == "unknown"
    with pytest.raises(NearError):
        faucet.rpc.call("nope", {})
