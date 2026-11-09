"""Tests for the read-only agent-vault testnet evidence.

A fake NEAR testnet RPC on localhost plays one full run (deploy, lock, fund,
scope, attack, revoke, recover). The happy path must collect and re-verify;
every way the chain could disagree with the run must fail closed.
"""
from __future__ import annotations

import base64
import copy
import hashlib
import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

import pytest

from testnet_guard_evidence import BASE58_ALPHABET, EXPECTED_TESTNET_GENESIS, EvidenceError
import vault_testnet_evidence as v

OWNER, VAULT, AGENT = "x3lang.testnet", "vault-1.x3lang.testnet", "agent-1.x3lang.testnet"
VENDOR, STRANGER = "vendor-1.x3lang.testnet", "stranger-1.x3lang.testnet"
CAP, BUDGET, FUND = "200000000000000000000000", "400000000000000000000000", "1000000000000000000000000"
PAY_1, PAY_2, PAY_3, OVER = (str(n * 10**23) for n in (1, 2, 2, 3))
SCOPED_PK = "ed25519:" + "A" * 44
FULL_PK = "ed25519:" + "B" * 44
FINAL = {"hash": "F" * 44, "height": 5000}
WASM = b"\x00asm\x01\x00\x00\x00vault-test-code"
TRUE = base64.b64encode(b"true").decode()


def b58(raw: bytes) -> str:
    n, out = int.from_bytes(raw, "big"), ""
    while n:
        n, r = divmod(n, 58)
        out = BASE58_ALPHABET[r] + out
    return "1" * (len(raw) - len(raw.lstrip(b"\0"))) + out


def tx_hash(i: int) -> str:
    return b58(hashlib.sha256(f"tx{i}".encode()).digest())


def fn_action(method: str, args: dict) -> dict:
    return {"FunctionCall": {"method_name": method, "args": base64.b64encode(json.dumps(args).encode()).decode(),
                             "gas": 100_000_000_000_000, "deposit": "0"}}


def denied(reason: str) -> dict:
    return {"Failure": {"ActionError": {"index": 0, "kind": {"FunctionCallError": {
        "ExecutionError": f"Smart contract panicked: X3_VAULT_DENIED: {reason}"}}}}}


class Chain:
    """Fake chain state plus a localhost JSON-RPC server for it."""

    def __init__(self):
        self.code_hash = b58(hashlib.sha256(WASM).digest())
        self.balances = {VENDOR: str(10**23), STRANGER: str(10**23), VAULT: str(5 * 10**24),
                         OWNER: str(50 * 10**24), AGENT: str(5 * 10**23)}
        self.keys = {
            AGENT: [{"public_key": SCOPED_PK, "access_key": {"nonce": 9, "permission": {"FunctionCall": {
                "allowance": "240000000000000000000000", "receiver_id": VAULT, "method_names": ["request_payment"]}}}}],
            VAULT: [],
        }
        self.views = {"spent": str(3 * 10**23), "reserved": "0", "is_revoked": True, "available": "0"}
        self.payments = {"pay-1": {"amount": PAY_1, "recipient": VENDOR, "state": "Paid"},
                         "pay-2": {"amount": PAY_2, "recipient": VENDOR, "state": "Paid"}}
        self.txs: dict[str, dict] = {}

    def add_tx(self, i, signer, receiver, actions, status):
        h = tx_hash(i)
        failure = "Failure" in status
        self.txs[h] = {
            "final_execution_status": "FINAL", "status": status,
            "transaction": {"signer_id": signer, "receiver_id": receiver, "actions": actions, "hash": h},
            "transaction_outcome": {"id": h, "block_hash": FINAL["hash"], "outcome": {}},
            "receipts_outcome": [{"id": "R" * 43, "outcome": {"status": status if failure else {"SuccessValue": ""}}}],
        }
        return h

    def handle(self, method, params):
        if method == "EXPERIMENTAL_chain":
            return None, {"code": -32601, "message": "Method not found", "cause": {"name": "METHOD_NOT_FOUND"}}
        if method == "status":
            return {"chain_id": "testnet", "genesis_hash": EXPECTED_TESTNET_GENESIS}, None
        if method == "block":
            return {"header": dict(FINAL)}, None
        if method == "tx":
            found = self.txs.get(params[0])
            return (found, None) if found else (None, {"message": "UNKNOWN_TRANSACTION"})
        if method == "query":
            at = {"block_hash": FINAL["hash"], "block_height": FINAL["height"]}
            kind, account = params["request_type"], params["account_id"]
            if kind == "view_account":
                return {**at, "amount": self.balances[account], "code_hash": self.code_hash if account == VAULT else "1" * 32}, None
            if kind == "view_access_key_list":
                return {**at, "keys": copy.deepcopy(self.keys.get(account, []))}, None
            if kind == "call_function":
                name = params["method_name"]
                args = json.loads(base64.b64decode(params["args_base64"]))
                value = self.payments.get(args["nonce"]) if name == "payment" else self.views[name]
                return {**at, "result": list(json.dumps(value).encode()), "logs": []}, None
        return None, {"message": f"unhandled {method}"}

    def __enter__(self):
        chain = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *a):
                pass

            def do_POST(self):
                req = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                result, error = chain.handle(req["method"], req["params"])
                body = {"jsonrpc": "2.0", "id": req["id"], **({"error": error} if error else {"result": result})}
                raw = json.dumps(body).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(raw)))
                self.end_headers()
                self.wfile.write(raw)

        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        return self

    def __exit__(self, *exc):
        self.server.shutdown()


def pay_args(nonce, recipient, amount):
    return {"nonce": nonce, "recipient": recipient, "amount": amount}


# label, signer, receiver, actions, status, expect, payment
RUN_STEPS = [
    ("create_vault", OWNER, VAULT, ["CreateAccount", {"Transfer": {"deposit": "1"}}], {"SuccessValue": ""}, "success", None),
    ("vault_deployed", VAULT, VAULT, [{"DeployContract": {"code": "x"}}, fn_action("new", {"owner": OWNER})], {"SuccessValue": ""}, "success", None),
    ("vault_locked", VAULT, VAULT, [{"DeleteKey": {"public_key": FULL_PK}}], {"SuccessValue": ""}, "success", None),
    ("owner_funds", OWNER, VAULT, [fn_action("fund", {})], {"SuccessValue": ""}, "success", None),
    ("agent_pays_vendor", AGENT, VAULT, None, {"SuccessValue": TRUE}, "returns_true", ("pay-1", VENDOR, PAY_1)),
    ("agent_over_cap", AGENT, VAULT, None, denied("per-payment cap exceeded"), "denied:per-payment cap exceeded", ("over-cap", VENDOR, OVER)),
    ("agent_pays_stranger", AGENT, VAULT, None, denied("recipient not allowlisted"), "denied:recipient not allowlisted", ("to-stranger", STRANGER, PAY_1)),
    ("agent_replays_nonce", AGENT, VAULT, None, denied("nonce already used"), "denied:nonce already used", ("pay-1", VENDOR, PAY_1)),
    ("agent_pays_vendor_again", AGENT, VAULT, None, {"SuccessValue": TRUE}, "returns_true", ("pay-2", VENDOR, PAY_2)),
    ("agent_over_budget", AGENT, VAULT, None, denied("cumulative budget exceeded"), "denied:cumulative budget exceeded", ("pay-3", VENDOR, PAY_3)),
    ("owner_revokes", OWNER, VAULT, [fn_action("revoke", {})], {"SuccessValue": ""}, "success", None),
    ("owner_recovers", OWNER, VAULT, [fn_action("recover_available", {"amount": "1"})], {"SuccessValue": TRUE}, "returns_true", None),
]


def play(chain: Chain, tmp: Path, steps=RUN_STEPS, *, locked=True) -> tuple[Path, Path]:
    run = tmp / "run.json"
    v.init_run(run, roles={"owner": OWNER, "vault": VAULT, "agent": AGENT, "vendor": VENDOR, "stranger": STRANGER},
               cap=CAP, budget=BUDGET, fund=FUND, deadline_ms=4_000_000_000_000, locked=locked)
    vendor_before = int(chain.balances[VENDOR])
    v.add_snapshot(run, chain.url, "before_attacks", [VENDOR, STRANGER])
    for i, (label, signer, receiver, actions, status, expect, payment) in enumerate(steps):
        if payment:
            actions = [fn_action("request_payment", pay_args(*payment))]
        h = chain.add_tx(i, signer, receiver, actions, status)
        extra = dict(zip(("nonce", "recipient", "amount"), payment)) if payment else {}
        v.add_step(run, label=label, expect=expect, signer=signer, tx_hash=h, **extra)
    v.add_step(run, label="agent_raw_transfer", expect="rpc_refused:requires_full_access", signer=AGENT, tx_hash=None,
               cli_exit=1, log_text="\x1b[31mError: Transaction requires a full permission access key.\x1b[0m\n")
    chain.balances[VENDOR] = str(vendor_before + int(PAY_1) + int(PAY_2))
    wasm = tmp / "vault.wasm"
    wasm.write_bytes(WASM)
    return run, wasm


def test_full_run_collects_and_reverifies(tmp_path):
    with Chain() as chain:
        run, wasm = play(chain, tmp_path)
        evidence = v.collect(run, wasm, chain.url)
        out = tmp_path / "evidence.json"
        v._write_json(out, evidence)
        result = v.verify(out, chain.url, wasm)
    assert result["verified"] is True and result["transactions"] == len(RUN_STEPS)
    verdicts = {x["label"]: x["verdict"] for x in evidence["verdicts"]}
    assert verdicts["agent_over_cap"] == "refused: X3_VAULT_DENIED: per-payment cap exceeded"
    assert verdicts["agent_pays_vendor"] == "succeeded_and_vault_returned_true"
    assert verdicts["agent_raw_transfer"] == ("never included on chain: Error: Transaction requires a full permission "
                                              "access key. (CLI exit 1)")
    assert evidence["keys"]["vault_has_no_keys"] is True
    assert evidence["balances"]["stranger_received"] == "0"
    assert evidence["balances"]["vendor_received"] == str(int(PAY_1) + int(PAY_2))
    assert out.stat().st_mode & 0o777 == 0o600


def _mutate_and_expect(tmp_path, mutate, message):
    with Chain() as chain:
        run, wasm = play(chain, tmp_path)
        mutate(chain)
        with pytest.raises(EvidenceError, match=message):
            v.collect(run, wasm, chain.url)


def test_vault_that_accepted_an_over_cap_payment_fails(tmp_path):
    def accept(chain):
        h = tx_hash(5)
        chain.txs[h]["status"] = {"SuccessValue": TRUE}
        chain.txs[h]["receipts_outcome"][0]["outcome"]["status"] = {"SuccessValue": ""}
    _mutate_and_expect(tmp_path, accept, "agent_over_cap: expected refusal")


def test_refusal_for_the_wrong_reason_fails(tmp_path):
    def wrong_reason(chain):
        chain.txs[tx_hash(6)]["status"] = denied("per-payment cap exceeded")
    _mutate_and_expect(tmp_path, wrong_reason, "recipient not allowlisted")


def test_agent_with_a_full_access_key_fails(tmp_path):
    def add_full(chain):
        chain.keys[AGENT].append({"public_key": FULL_PK, "access_key": {"nonce": 1, "permission": "FullAccess"}})
    _mutate_and_expect(tmp_path, add_full, "not exactly one")


def test_agent_key_scoped_to_more_methods_fails(tmp_path):
    def widen(chain):
        chain.keys[AGENT][0]["access_key"]["permission"]["FunctionCall"]["method_names"] = ["request_payment", "revoke"]
    _mutate_and_expect(tmp_path, widen, "not exactly one")


def test_locked_vault_that_still_has_a_key_fails(tmp_path):
    def keep_key(chain):
        chain.keys[VAULT] = [{"public_key": FULL_PK, "access_key": {"nonce": 1, "permission": "FullAccess"}}]
    _mutate_and_expect(tmp_path, keep_key, "still has keys")


def test_stranger_receiving_anything_fails(tmp_path):
    def leak(chain):
        chain.balances[STRANGER] = str(int(chain.balances[STRANGER]) + 1)
    _mutate_and_expect(tmp_path, leak, "stranger balance moved")


def test_vendor_paid_more_than_the_run_fails(tmp_path):
    def overpay(chain):
        chain.balances[VENDOR] = str(int(chain.balances[VENDOR]) + 1)
    _mutate_and_expect(tmp_path, overpay, "vendor balance moved")


def test_ledger_that_disagrees_with_payments_fails(tmp_path):
    def drift(chain):
        chain.views["spent"] = str(4 * 10**23)
    _mutate_and_expect(tmp_path, drift, "vault spent")


def test_refused_nonce_that_left_a_record_fails(tmp_path):
    def record(chain):
        chain.payments["over-cap"] = {"amount": OVER, "recipient": VENDOR, "state": "Failed"}
    _mutate_and_expect(tmp_path, record, "refused nonce over-cap")


def test_unsettled_reservation_fails(tmp_path):
    def pending(chain):
        chain.views["reserved"] = "1"
    _mutate_and_expect(tmp_path, pending, "still reserves")


def test_deployed_code_that_is_not_the_wasm_fails(tmp_path):
    def swap(chain):
        chain.code_hash = b58(hashlib.sha256(b"other").digest())
    _mutate_and_expect(tmp_path, swap, "code_hash does not match")


def test_payment_with_different_arguments_fails(tmp_path):
    def redirect(chain):
        chain.txs[tx_hash(4)]["transaction"]["actions"] = [fn_action("request_payment", pay_args("pay-1", STRANGER, PAY_1))]
    _mutate_and_expect(tmp_path, redirect, "on-chain arguments differ")


def test_wrong_signer_fails(tmp_path):
    def resign(chain):
        chain.txs[tx_hash(3)]["transaction"]["signer_id"] = AGENT
    with Chain() as chain:
        run, wasm = play(chain, tmp_path)
        resign(chain)
        with pytest.raises(EvidenceError):
            v.collect(run, wasm, chain.url)


def test_tampered_evidence_fails_verification(tmp_path):
    with Chain() as chain:
        run, wasm = play(chain, tmp_path)
        evidence = v.collect(run, wasm, chain.url)
        evidence["balances"]["stranger_received"] = "1"
        out = tmp_path / "evidence.json"
        v._write_json(out, evidence)
        with pytest.raises(EvidenceError, match="record_hash mismatch"):
            v.verify(out, chain.url, wasm)


def test_chain_changing_after_collection_fails_verification(tmp_path):
    with Chain() as chain:
        run, wasm = play(chain, tmp_path)
        out = tmp_path / "evidence.json"
        v._write_json(out, v.collect(run, wasm, chain.url))
        chain.keys[VAULT] = [{"public_key": FULL_PK, "access_key": {"nonce": 1, "permission": "FullAccess"}}]
        with pytest.raises(EvidenceError, match="still has keys"):
            v.verify(out, chain.url, wasm)


def test_incomplete_run_is_refused(tmp_path):
    with Chain() as chain:
        run, wasm = play(chain, tmp_path, steps=RUN_STEPS[:2], locked=False)
        with pytest.raises(EvidenceError, match="no agent_pays_vendor"):
            v.collect(run, wasm, chain.url)


# ---- run-file bookkeeping ---------------------------------------------------------------


def _fresh_run(tmp_path):
    run = tmp_path / "run.json"
    v.init_run(run, roles={"owner": OWNER, "vault": VAULT, "agent": AGENT, "vendor": VENDOR, "stranger": STRANGER},
               cap=CAP, budget=BUDGET, fund=FUND, deadline_ms=1, locked=True)
    return run


def test_step_rules(tmp_path):
    run = _fresh_run(tmp_path)
    refused = "Error: Transaction requires a full permission access key."
    raw = dict(label="agent_raw_transfer", expect="rpc_refused:requires_full_access", signer=AGENT)
    with pytest.raises(EvidenceError, match="reached the chain"):
        v.add_step(run, **raw, tx_hash=tx_hash(1), cli_exit=1, log_text=refused)
    with pytest.raises(EvidenceError, match="non-zero exit"):
        v.add_step(run, **raw, tx_hash=None, cli_exit=0, log_text=refused)
    with pytest.raises(EvidenceError, match="names a transaction"):
        v.add_step(run, **raw, tx_hash=None, cli_exit=1,
                   log_text=f"Could not confirm whether the transaction was included.\nTransaction ID: {tx_hash(9)}")
    with pytest.raises(EvidenceError, match="does not contain NEAR's refusal"):
        v.add_step(run, **raw, tx_hash=None, cli_exit=1, log_text="Transport error transaction")
    with pytest.raises(EvidenceError, match="does not contain NEAR's refusal"):
        v.add_step(run, **raw, tx_hash=None, cli_exit=1,
                   log_text="Error: Transaction method name <revoke> isn't allowed by the access key.")
    with pytest.raises(EvidenceError, match="unknown RPC refusal"):
        v.add_step(run, label="agent_raw_transfer", expect="rpc_refused:whatever", signer=AGENT, tx_hash=None, cli_exit=1)
    step = v.add_step(run, **raw, tx_hash=None, cli_exit=1, log_text=refused)
    assert step["cli_message"] == refused and step["reason"] == "requires_full_access"
    with pytest.raises(EvidenceError, match="hash is required"):
        v.add_step(run, label="agent_over_cap", expect="denied:per-payment cap exceeded", signer=AGENT, tx_hash=None)
    with pytest.raises(EvidenceError, match="unknown expectation"):
        v.add_step(run, label="x_step", expect="maybe", signer=AGENT, tx_hash=tx_hash(1))
    v.add_step(run, label="owner_funds", expect="success", signer=OWNER, tx_hash=tx_hash(1))
    with pytest.raises(EvidenceError, match="already recorded"):
        v.add_step(run, label="owner_funds", expect="success", signer=OWNER, tx_hash=tx_hash(2))
    with pytest.raises(EvidenceError, match="testnet account"):
        v.add_step(run, label="mainnet_step", expect="success", signer="x3lang.near", tx_hash=tx_hash(3))


def test_init_rules(tmp_path):
    roles = {"owner": OWNER, "vault": VAULT, "agent": AGENT, "vendor": VENDOR, "stranger": STRANGER}
    with pytest.raises(EvidenceError, match="distinct"):
        v.init_run(tmp_path / "a.json", roles={**roles, "agent": OWNER}, cap=CAP, budget=BUDGET, fund=FUND, deadline_ms=1, locked=True)
    with pytest.raises(EvidenceError, match="cap <= budget"):
        v.init_run(tmp_path / "b.json", roles=roles, cap=BUDGET, budget=CAP, fund=FUND, deadline_ms=1, locked=True)
    _fresh_run(tmp_path)
    with pytest.raises(EvidenceError, match="already exists"):
        _fresh_run(tmp_path)


def test_tx_hash_extraction():
    h = tx_hash(7)
    log = f"\x1b[32mGas used: 1\x1b[0m\nTransaction ID: {h}\nopen:\nhttps://explorer.testnet.near.org/transactions/{h}\n"
    assert v.extract_tx_hash(log) == h
    assert v.extract_tx_hash(f"https://testnet.nearblocks.io/txns/{h}") == h
    assert v.extract_tx_hash("Error: InvalidTxError RequiresFullAccess") is None
    with pytest.raises(EvidenceError, match="several transactions"):
        v.extract_tx_hash(f"Transaction ID: {h}\nTransaction ID: {tx_hash(8)}")


def test_find_key_file_matches_public_key_only(tmp_path):
    account_dir = tmp_path / "testnet" / AGENT
    account_dir.mkdir(parents=True)
    (tmp_path / "testnet" / f"{AGENT}.json").write_text(json.dumps({"public_key": FULL_PK, "private_key": "ed25519:secret"}))
    scoped = account_dir / f"{SCOPED_PK.replace(':', '_')}.json"
    scoped.write_text(json.dumps({"public_key": SCOPED_PK, "private_key": "ed25519:secret2"}))
    assert v.find_key_file(tmp_path, "testnet", AGENT, SCOPED_PK) == scoped
    assert v.find_key_file(tmp_path, "testnet", AGENT, FULL_PK) == tmp_path / "testnet" / f"{AGENT}.json"
    assert v.key_file_public_key(scoped) == SCOPED_PK
    with pytest.raises(EvidenceError, match="no key file"):
        v.find_key_file(tmp_path, "testnet", AGENT, "ed25519:" + "C" * 44)


def test_onchain_key_refuses_ambiguity(tmp_path):
    with Chain() as chain:
        assert v.onchain_key(chain.url, AGENT, "function-call", VAULT, "request_payment") == SCOPED_PK
        with pytest.raises(EvidenceError, match="0 full keys"):
            v.onchain_key(chain.url, AGENT, "full")
        chain.keys[AGENT].append(copy.deepcopy(chain.keys[AGENT][0]))
        chain.keys[AGENT][1]["public_key"] = "ed25519:" + "D" * 44
        with pytest.raises(EvidenceError, match="2 function-call keys"):
            v.onchain_key(chain.url, AGENT, "function-call", VAULT, "request_payment")


def test_cli_rejects_mainnet_accounts(capsys):
    assert v.main(["--rpc-url", "http://127.0.0.1:9", "public-key", "--account", "x3lang.near", "--kind", "full"]) == 2
    assert "testnet account" in capsys.readouterr().err
