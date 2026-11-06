"""TDD tests for the testnet Guard evidence collector.

Seams under test (fixed by the Task 4 brief):
- ``record_manifest(account_id, wasm, rpc_url)``
- ``record_transaction(tx_hash, rpc_url)``
- ``verify_evidence(path, rpc_url)``
All three raise ``EvidenceError`` on malformed or mismatched evidence.
Fake RPC responses are injected over localhost HTTP; the collector must
accept localhost HTTP for tests but require HTTPS for non-local RPC.
"""
from __future__ import annotations

import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import threading

import pytest

from testnet_guard_evidence import EvidenceError, record_manifest, record_transaction, verify_evidence


# ---------------------------------------------------------------------------
# Injected fake NEAR RPC (localhost only)
# ---------------------------------------------------------------------------

class _FakeRpc:
    def __init__(self, responses):
        self.responses = responses
        self.url = None
        self._server = None
        self._thread = None

    def __enter__(self):
        # Bind responses for handler via closure attribute on instance.
        outer = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, fmt, *args):  # silence
                pass

            def do_POST(self):
                length = int(self.headers.get("Content-Length", 0))
                body = self.rfile.read(length) if length else b"{}"
                try:
                    req = json.loads(body)
                except Exception:
                    req = {}
                method = req.get("method", "")
                rid = req.get("id", "fake")
                result = outer.responses.get(method)
                if callable(result):
                    payload = result(req.get("params"))
                else:
                    payload = result
                if payload is None:
                    resp = {"jsonrpc": "2.0", "id": rid, "error": {"code": -32601, "message": "Method not found"}}
                elif isinstance(payload, dict) and "error" in payload:
                    resp = {"jsonrpc": "2.0", "id": rid, **payload}
                elif isinstance(payload, dict) and "result" in payload:
                    resp = {"jsonrpc": "2.0", "id": rid, **payload}
                else:
                    resp = {"jsonrpc": "2.0", "id": rid, "result": payload}
                data = json.dumps(resp).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.url = f"http://127.0.0.1:{self._server.server_address[1]}"
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()
        return self

    def __exit__(self, *exc):
        self._server.shutdown()
        self._server.server_close()
        return False


@pytest.fixture
def fake_rpc():
    # Default: correct testnet chain, plenty of helpers set per-test via factory.
    responses = {
        "EXPERIMENTAL_chain": {"chain_id": "testnet"},
        "status": {"chain_id": "testnet", "genesis_hash": "FWJ9kR6KFWoyMoNjpLXXGHeuiy7tEY6GmoFeCA5yuc6b",
                   "sync_info": {"latest_block_height": 100, "latest_block_hash": "11111111111111111111111111111111"}},
        "query": {"amount": "10000000000000000000000000", "storage_usage": 182, "code_hash": "11111111111111111111111111111111", "block_height": 100, "block_hash": "11111111111111111111111111111111"},
        "block": {"header": {"height": 100, "hash": "11111111111111111111111111111111", "prev_hash": "22222222222222222222222222222222"}},
        "tx": {"final_execution_status": "FINAL", "status": {"SuccessValue": ""},
               "transaction_outcome": {"id": "11111111111111111111111111111111", "block_hash": "11111111111111111111111111111111", "outcome": {"status": {"SuccessReceiptId": "22222222222222222222222222222222"}}},
               "receipts_outcome": [{"id": "22222222222222222222222222222222", "outcome": {"status": {"SuccessValue": ""}, "logs": []}}],
               "transaction": {"actions": [{"FunctionCall": {"method_name": "new", "args": "e30="}}]}},
    }
    rpc = _FakeRpc(responses)
    with rpc as started:
        # Expose mutator so tests can change responses mid-fixture.
        started.responses = responses
        yield started


def fixture(name, payload, tmp_path):
    path = tmp_path / name
    path.write_text(json.dumps(payload, sort_keys=True), encoding="utf-8")
    return path


def test_manifest_rejects_wrong_chain_id(fake_rpc, tmp_path):
    fake_rpc.responses["EXPERIMENTAL_chain"] = {"chain_id": "mainnet"}
    with pytest.raises(EvidenceError, match="chain"):
        verify_evidence(fixture("wrong-chain.json", {"schema": "x3.guard.testnet.manifest.v1", "account_id": "x.testnet"}, tmp_path), rpc_url=fake_rpc.url)


def test_transaction_rejects_missing_final_execution_status(fake_rpc):
    def _tx(params):
        return {"final_execution_status": "EXECUTED", "status": {"SuccessValue": ""}, "transaction_outcome": {"id": "missing-final", "block_hash": "11111111111111111111111111111111", "outcome": {"status": {"SuccessReceiptId": "22222222222222222222222222222222"}}}, "receipts_outcome": [], "transaction": {"actions": []}}
    fake_rpc.responses["tx"] = _tx
    fake_rpc.responses["EXPERIMENTAL_chain"] = {"chain_id": "testnet"}
    with pytest.raises(EvidenceError, match="final"):
        record_transaction("missing-final", rpc_url=fake_rpc.url)


def test_verify_rejects_private_key_evidence(fake_rpc, tmp_path):
    with pytest.raises(EvidenceError, match="private"):
        verify_evidence(fixture("bad-key.json", {"schema": "x3.guard.testnet.manifest.v1", "private_key": "ed25519:fake"}, tmp_path), rpc_url=fake_rpc.url)


def test_record_manifest_requires_https_for_non_local():
    with pytest.raises(EvidenceError, match="https|HTTPS"):
        record_manifest("x3lang-guard-20261106.testnet", Path("/tmp/nope.wasm"), "http://rpc.example.com")


def test_manifest_records_code_identity(fake_rpc, tmp_path):
    wasm = tmp_path / "x.wasm"
    wasm.write_bytes(b"\x00asm" + b"\x00" * 32)
    rec = record_manifest("x3lang-guard-20261106.testnet", wasm, rpc_url=fake_rpc.url)
    assert rec["chain_id"] == "testnet"
    assert rec["account_id"] == "x3lang-guard-20261106.testnet"
    assert rec["wasm_bytes"] == 36
    assert len(rec["wasm_sha256"]) == 64
    assert rec["code_hash"] == "11111111111111111111111111111111"
    assert rec["block_height"] == 100
    # Canonical hash must verify.
    assert "record_hash" in rec
