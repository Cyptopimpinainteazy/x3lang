"""Operator signing boundary tests use disposable synthetic credentials only.

RPC sends are intercepted. One integration test runs the installed CLI offline
with a synthetic key; it never broadcasts or accesses user credentials.
"""
import json

import pytest

import near_agent_key as keys

PUBLIC = "ed25519:AKnL4NNf3DGWZJS6cPknBuEGnVsV4A4m5tgebLHaRSZ9"
METHODS = ["agent_register_intent", "agent_accept_quote", "agent_record_outcome"]


def final_response(request):
    import base64, hashlib, ed25519
    unsigned = base64.b64decode(request["params"]["signed_tx_base64"])[:-65]
    return {"result": {"final_execution_status": "FINAL",
        "status": {"SuccessValue": ""}, "transaction": {"hash": ed25519.b58encode(hashlib.sha256(unsigned).digest()),
        "signer_id": "agent.near", "receiver_id": "guard.near"}}}


class Rpc:
    def __init__(self, permission):
        self.permission = permission
        self.requests = []

    def __call__(self, method, url, headers, body, timeout):
        request = json.loads(body)
        self.requests.append(request)
        if request["method"] == "send_tx":
            return 200, json.dumps(final_response(request)).encode()
        result = {"chain_id": "mainnet"} if request["method"] == "status" else {
            "block_height": 123, "block_hash": "11111111111111111111111111111111", "nonce": 1,
            "permission": self.permission,
        }
        return 200, json.dumps({"jsonrpc": "2.0", "id": request["id"], "result": result}).encode()


def restricted(**changes):
    return {"FunctionCall": {"receiver_id": "guard.near", "method_names": METHODS,
                             "allowance": "100000000000000000000000", **changes}}


def check(rpc):
    return keys.check_access_key(account="agent.near", public_key=PUBLIC, contract="guard.near",
                                 method="agent_register_intent", max_allowance_yocto="100000000000000000000000",
                                 network="mainnet", transport=rpc)


def test_exact_pinned_key_is_queried_at_finality_without_account_wide_fullaccess_ban():
    rpc = Rpc(restricted())
    result = check(rpc)
    assert result["public_key"] == PUBLIC
    assert result["permission"] == "FunctionCall"
    assert result["account_other_keys"] == "not_checked; owner recovery keys may remain"
    assert rpc.requests[0]["method"] == "status"
    assert rpc.requests[1]["params"] == {
        "request_type": "view_access_key", "finality": "final", "account_id": "agent.near", "public_key": PUBLIC,
    }


def test_pinned_fullaccess_key_is_refused():
    with pytest.raises(keys.AgentKeyError, match="X3_AGENT_KEY_PERMISSION"):
        check(Rpc("FullAccess"))


def credential(tmp_path, seed_byte=7):
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    import ed25519
    seed = bytes([seed_byte]) * 32
    public = Ed25519PrivateKey.from_private_bytes(seed).public_key().public_bytes_raw()
    value = {"account_id": "agent.near", "public_key": "ed25519:" + ed25519.b58encode(public),
             "private_key": "ed25519:" + ed25519.b58encode(seed + public)}
    path = tmp_path / "synthetic-key.json"
    path.write_text(json.dumps(value))
    path.chmod(0o600)
    return path, value


def signed_output(command, value):
    """Synthetic CLI substitute; production uses only the actual near signer."""
    import base64, hashlib, ed25519
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
    def argument(flag): return command[command.index(flag) + 1]
    def string(text):
        data = text.encode(); return len(data).to_bytes(4, "little") + data
    receiver, method = command[command.index("as-transaction")+1:command.index("as-transaction")+3]
    public = ed25519.b58decode(value["public_key"][8:])
    unsigned = (string(argument("sign-as")) + bytes([0]) + public
                + int(argument("--nonce")).to_bytes(8, "little") + string(receiver)
                + ed25519.b58decode(argument("--block-hash")) + bytes([1,0,0,0,2])
                + string(method) + string(base64.b64decode(argument("base64-args")).decode())
                + (int(argument("prepaid-gas").split()[0]) * 10**12).to_bytes(8, "little")
                + (int(argument("attached-deposit").split()[0]) * 10**24).to_bytes(16, "little"))
    private = Ed25519PrivateKey.from_private_bytes(ed25519.b58decode(value["private_key"][8:])[:32])
    raw = unsigned + bytes([0]) + private.sign(hashlib.sha256(unsigned).digest())
    return "Signed transaction (serialized as base64):\n" + base64.b64encode(raw).decode() + "\n"


def send(path, value, rpc, runner, **changes):
    kwargs = dict(account="agent.near", public_key=value["public_key"], contract="guard.near",
                  method="agent_register_intent", max_allowance_yocto="100000000000000000000000",
                  network="mainnet", transport=rpc, runner=runner, key_file=str(path), args={"nonce": "fixture"})
    return keys.execute(**(kwargs | changes))


def test_send_uses_only_validated_credential_snapshot_then_removes_it(tmp_path):
    from pathlib import Path
    from subprocess import CompletedProcess
    path, value = credential(tmp_path)
    rpc = Rpc(restricted())
    calls = []
    snapshots = []

    def transport(*args):
        # A replaced original file must never select a different signing key.
        path.write_text('{"private_key":"replacement"}')
        return rpc(*args)

    def runner(command, **kwargs):
        calls.append(command)
        snapshot = Path(command[command.index("sign-with-access-key-file") + 1])
        snapshots.append(snapshot)
        assert snapshot != path and snapshot.stat().st_mode & 0o777 == 0o600
        assert json.loads(snapshot.read_text()) == value
        assert command[command.index("attached-deposit") + 1] == "0 NEAR"
        return CompletedProcess(command, 0, signed_output(command, value), "")

    assert send(path, value, transport, runner) == 0
    assert len(calls) == 1 and len(rpc.requests) == 3
    assert not snapshots[0].exists()


@pytest.mark.parametrize("permission", ["FullAccess", restricted(receiver_id="other.near"),
    restricted(method_names=[]), restricted(method_names=["withdraw_agent_storage"]),
    restricted(method_names=["agent_accept_quote"]), restricted(allowance=None),
    restricted(allowance="0"), restricted(allowance="100000000000000000000001")])
def test_send_never_invokes_cli_when_permission_or_budget_invalid(tmp_path, permission):
    path, value = credential(tmp_path)
    def forbidden(*args, **kwargs):
        pytest.fail("near CLI must not be invoked")
    with pytest.raises(keys.AgentKeyError):
        send(path, value, Rpc(permission), forbidden)


def test_send_never_invokes_cli_when_credential_pin_mismatches(tmp_path):
    path, value = credential(tmp_path)
    def forbidden(*args, **kwargs):
        pytest.fail("RPC and near CLI must not be invoked")
    with pytest.raises(keys.AgentKeyError, match="X3_AGENT_KEY_CREDENTIAL"):
        send(path, value, forbidden, forbidden, public_key=PUBLIC)


def test_prepaid_commands_require_pin_and_route_through_send_boundary():
    import near_intents as ni
    common = dict(contract="guard.near", signer="agent.near", key_file="synthetic-key.json",
                  method="agent_register_intent", prepaid_agent_storage=True)
    with pytest.raises(ni.NearIntentsError) as error:
        ni.guard_command("register", {}, **common)
    assert error.value.code == "X3_AGENT_KEY_PIN_REQUIRED"
    command = ni.guard_command("register", {}, **common, agent_public_key=PUBLIC,
                               agent_max_allowance_yocto="100000000000000000000000")
    assert command[1].endswith("near_agent_key.py") and command[2] == "send"
    assert command[command.index("--public-key") + 1] == PUBLIC
    assert command[command.index("--method") + 1] == "agent_register_intent"
    assert "sign-with-access-key-file" not in command


@pytest.mark.parametrize("failure", ["network", "missing_network", "http", "rpc_error", "nested_rpc_error", "json", "timeout",
                                     "nonce", "block_hash", "block_height"])
def test_rpc_failures_never_invoke_cli(tmp_path, failure):
    path, value = credential(tmp_path)
    rpc = Rpc(restricted())
    def transport(*args):
        if failure == "timeout":
            raise TimeoutError("untrusted RPC text")
        status, raw = rpc(*args)
        data = json.loads(raw)
        if failure == "http":
            return 503, raw
        if failure == "json":
            return 200, b"not JSON"
        if failure == "rpc_error":
            return 200, b'{"error":{"message":"untrusted RPC text"}}'
        if failure == "nested_rpc_error" and "chain_id" not in data["result"]:
            return 200, b'{"result":{"error":"missing access key"}}'
        if "chain_id" in data["result"]:
            if failure == "network":
                data["result"]["chain_id"] = "testnet"
            if failure == "missing_network":
                data["result"].pop("chain_id")
        elif failure in ("nonce", "block_hash", "block_height"):
            data["result"][failure] = None
        return status, json.dumps(data).encode()
    def forbidden(*args, **kwargs):
        pytest.fail("near CLI must not be invoked")
    with pytest.raises(keys.AgentKeyError) as error:
        send(path, value, transport, forbidden)
    assert "untrusted RPC text" not in str(error.value)
    if failure == "nested_rpc_error":
        assert str(error.value) == "X3_AGENT_KEY_RPC"


def test_stale_verification_never_invokes_cli(tmp_path):
    path, value = credential(tmp_path)
    def forbidden(*args, **kwargs):
        pytest.fail("near CLI must not be invoked")
    ticks = iter((10, 71))
    with pytest.raises(keys.AgentKeyError, match="X3_AGENT_KEY_STALE"):
        send(path, value, Rpc(restricted()), forbidden, clock=lambda: next(ticks))


@pytest.mark.parametrize("change", ["account", "private_key", "public_suffix", "permissions", "symlink", "json"])
def test_credential_failures_prevent_rpc_and_send(tmp_path, change):
    import ed25519
    path, value = credential(tmp_path)
    changed = dict(value)
    if change == "account":
        changed["account_id"] = "other.near"
    elif change == "private_key":
        changed["private_key"] = "ed25519:" + ed25519.b58encode(bytes([8]) * 64)
    elif change == "public_suffix":
        raw = ed25519.b58decode(changed["private_key"][8:])
        changed["private_key"] = "ed25519:" + ed25519.b58encode(raw[:32] + bytes(32))
    path.write_text(json.dumps(changed))
    if change == "permissions":
        path.chmod(0o644)
    elif change == "symlink":
        target = path.with_name("target.json")
        path.rename(target)
        path.symlink_to(target)
    elif change == "json":
        path.write_text("invalid JSON")
    def forbidden(*args, **kwargs):
        pytest.fail("neither RPC nor near CLI may run")
    with pytest.raises(keys.AgentKeyError, match="X3_AGENT_KEY_CREDENTIAL"):
        send(path, value, forbidden, forbidden)


def test_cli_failure_cleans_snapshot_and_does_not_retry_or_expose_secret(tmp_path):
    from pathlib import Path
    path, value = credential(tmp_path)
    snapshots = []
    def runner(command, **kwargs):
        snapshots.append(Path(command[command.index("sign-with-access-key-file") + 1]))
        raise OSError(value["private_key"])
    with pytest.raises(keys.AgentKeyError, match="X3_AGENT_KEY_CLI_SIGNING_FAILED") as error:
        send(path, value, Rpc(restricted()), runner)
    assert len(snapshots) == 1 and not snapshots[0].exists()
    assert value["private_key"] not in str(error.value)


def test_cli_output_emits_only_safe_metadata_and_binds_finalized_nonce_and_block(tmp_path, capsys):
    from subprocess import CompletedProcess
    path, value = credential(tmp_path)
    payload = {"signed_payload": '{"depositAddress":"PRIVATE-RECEIPT-MARKER"}',
               "signature": "PRIVATE-SIGNATURE-MARKER"}
    def runner(command, **kwargs):
        assert command[:2] == ["near", "--offline"] and command[-1] == "display"
        assert command[command.index("--nonce") + 1] == "2"
        assert command[command.index("--block-hash") + 1] == "11111111111111111111111111111111"
        assert command[command.index("--block-height") + 1] == "123"
        stdout = (json.dumps(command) + "\n" + json.dumps(json.dumps(payload)) + "\n"
                  + value["private_key"] + "\n" + signed_output(command, value))
        return CompletedProcess(command, 0, stdout, value["private_key"][8:])
    rpc = Rpc(restricted())
    def transport(*args):
        if json.loads(args[3])["method"] == "send_tx":
            return 200, json.dumps({"error": {"message": "X3_MANDATE_DENIED " + json.dumps(payload)}}).encode()
        return rpc(*args)
    assert send(path, value, transport, runner, args=payload) == 1
    output = capsys.readouterr()
    assert output.err == ""
    ready, metadata = [json.loads(line) for line in output.out.splitlines()]
    assert ready == {"stage": "signed", "transaction_hash_candidate": metadata["transaction_hash_candidate"],
                     "verified": False, "broadcasted": False}
    import ed25519
    assert len(ed25519.b58decode(metadata.pop("transaction_hash_candidate"))) == 32
    assert metadata == {"cli_exit_code": 0, "error_code_candidates": ["X3_MANDATE_DENIED"],
                        "verified": False, "broadcasted": "unknown", "send_status": "outcome_unknown"}
    for secret in (value["private_key"], value["private_key"][8:], "PRIVATE-RECEIPT-MARKER",
                   "PRIVATE-SIGNATURE-MARKER", "--args-json", "json-args"):
        assert secret not in output.out


@pytest.mark.parametrize("changes", [{"attached_deposit": "1 NEAR"}, {"args": []}, {"public_key": None},
                                      {"max_allowance_yocto": None}, {"method": "withdraw_agent_storage"},
                                      {"network": "unknown"}])
def test_invalid_call_configuration_never_invokes_cli(tmp_path, changes):
    path, value = credential(tmp_path)
    def forbidden(*args, **kwargs):
        pytest.fail("near CLI must not be invoked")
    with pytest.raises(keys.AgentKeyError):
        send(path, value, Rpc(restricted()), forbidden, **changes)


@pytest.mark.parametrize("arguments", [{"nonce": "fixture"},
    {"nonce": "fixture", "signed_payload": '{"memo":"synthetic quote"}', "text": "Unicode λ", "nested": [True, 1]}])
def test_real_cli_offline_display_binds_exact_reviewed_transaction(tmp_path, capsys, arguments):
    """Installed CLI with synthetic key only; RPC send is intercepted, never broadcast."""
    import shutil
    import subprocess
    if shutil.which("near") is None:
        pytest.skip("installed near CLI required for offline integration")
    path, value = credential(tmp_path)
    calls = []
    rpc = Rpc(restricted())
    def offline_only(command, **kwargs):
        assert command[:2] == ["near", "--offline"]
        assert command[-1] == "display"
        calls.append(command)
        return subprocess.run(command, **kwargs)
    def transport(method, url, headers, body, timeout):
        request = json.loads(body)
        if request["method"] != "send_tx":
            return rpc(method, url, headers, body, timeout)
        assert url == keys.RPC_URLS["mainnet"]
        assert request["params"]["wait_until"] == "FINAL"
        # Independent decoding of the real CLI artifact, not expected serialization.
        import base64, hashlib, ed25519
        from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PublicKey
        raw = base64.b64decode(request["params"]["signed_tx_base64"], validate=True)
        offset = 0
        def take(n):
            nonlocal offset
            part = raw[offset:offset+n]; offset += n; return part
        def number(n): return int.from_bytes(take(n), "little")
        def string(): return take(number(4))
        assert string() == b"agent.near"
        assert number(1) == 0
        public = take(32)
        assert "ed25519:" + ed25519.b58encode(public) == value["public_key"]
        assert number(8) == 2
        assert string() == b"guard.near"
        assert take(32) == bytes(32)
        assert number(4) == 1 and number(1) == 2
        assert string() == b"agent_register_intent"
        assert json.loads(string()) == arguments
        assert number(8) == 30_000_000_000_000 and number(16) == 0
        unsigned = raw[:offset]
        assert number(1) == 0
        Ed25519PublicKey.from_public_bytes(public).verify(take(64), hashlib.sha256(unsigned).digest())
        assert offset == len(raw)
        return 200, json.dumps(final_response(request)).encode()
    assert send(path, value, transport, offline_only, args=arguments) == 0
    assert len(calls) == 1
    assert value["private_key"] not in capsys.readouterr().out


@pytest.mark.parametrize("field", ["--nonce", "--block-hash", "sign-as", "receiver", "method", "base64-args",
                                   "prepaid-gas", "attached-deposit", "public_key", "signature", "trailing", "missing", "duplicate"])
def test_altered_signed_artifact_never_reaches_broadcast(tmp_path, field):
    from subprocess import CompletedProcess
    import base64
    path, value = credential(tmp_path)
    rpc = Rpc(restricted())
    def runner(command, **kwargs):
        command = list(command)
        signing_value = value
        replacements = {"--nonce":"3", "--block-hash":"11111111111111111111111111111112",
                        "sign-as":"other.near", "base64-args":base64.b64encode(b'{"nonce":"substituted"}').decode(),
                        "prepaid-gas":"31 Tgas", "attached-deposit":"1 NEAR"}
        if field in replacements: command[command.index(field)+1] = replacements[field]
        if field == "receiver": command[command.index("as-transaction")+1] = "other.near"
        if field == "method": command[command.index("as-transaction")+2] = "agent_accept_quote"
        if field == "public_key": _, signing_value = credential(tmp_path, seed_byte=8)
        output = signed_output(command, signing_value)
        if field in ("signature", "trailing"):
            raw = bytearray(base64.b64decode(output.splitlines()[1]))
            if field == "signature": raw[-1] ^= 1
            else: raw += b"extra"
            output = output.splitlines()[0] + "\n" + base64.b64encode(raw).decode() + "\n"
        if field == "missing": output = "not an artifact"
        if field == "duplicate": output += output
        return CompletedProcess(command, 0, output, "")
    with pytest.raises(keys.AgentKeyError, match="X3_AGENT_KEY_SIGNED_TRANSACTION_MISMATCH"):
        send(path, value, rpc, runner)
    assert len(rpc.requests) == 2


def test_stale_after_signing_never_broadcasts(tmp_path):
    from subprocess import CompletedProcess
    path, value = credential(tmp_path)
    ticks = iter((10, 11, 71))
    rpc = Rpc(restricted())
    def runner(command, **kwargs): return CompletedProcess(command, 0, signed_output(command, value), "")
    with pytest.raises(keys.AgentKeyError, match="X3_AGENT_KEY_STALE"):
        send(path, value, rpc, runner, clock=lambda: next(ticks))
    assert len(rpc.requests) == 2


@pytest.mark.parametrize("failure", ["timeout", "http", "malformed", "rpc_error", "execution_failure", "missing_outcome", "missing_status", "unknown_status", "wrong_hash", "wrong_signer", "wrong_receiver"])
def test_broadcast_failure_keeps_hash_and_never_retries(tmp_path, capsys, failure):
    from subprocess import CompletedProcess
    path, value = credential(tmp_path)
    rpc = Rpc(restricted()); sends = []
    def runner(command, **kwargs): return CompletedProcess(command, 0, signed_output(command, value), "")
    def transport(*args):
        request = json.loads(args[3])
        if request["method"] != "send_tx": return rpc(*args)
        sends.append(request)
        ready = json.loads(capsys.readouterr().out)
        assert ready["stage"] == "signed" and ready["broadcasted"] is False
        if failure == "timeout": raise TimeoutError(value["private_key"])
        if failure == "http": return 503, b'{}'
        if failure == "malformed": return 200, b'not JSON'
        if failure == "rpc_error": return 200, b'{"error":{"message":"private"}}'
        response = final_response(request)
        outcome = response["result"]
        if failure == "execution_failure": outcome["status"] = {"Failure": {}}
        if failure == "missing_outcome": response["result"] = {"final_execution_status": "FINAL"}
        if failure == "missing_status": outcome.pop("status")
        if failure == "unknown_status": outcome["status"] = {"Unknown": None}
        if failure == "wrong_hash": outcome["transaction"]["hash"] = "11111111111111111111111111111111"
        if failure == "wrong_signer": outcome["transaction"]["signer_id"] = "other.near"
        if failure == "wrong_receiver": outcome["transaction"]["receiver_id"] = "other.near"
        return 200, json.dumps(response).encode()
    assert send(path, value, transport, runner) == 1
    assert len(sends) == 1
    result = json.loads(capsys.readouterr().out)
    assert result["transaction_hash_candidate"] and result["send_status"] == "outcome_unknown"


@pytest.mark.parametrize("message,code", [
    ("Access key file not found!", "X3_AGENT_KEY_CLI_CREDENTIAL"),
    ("unexpected argument", "X3_AGENT_KEY_CLI_ARGUMENT"),
    ("unrecognized subcommand", "X3_AGENT_KEY_CLI_ARGUMENT"),
    ("Cannot send transaction in offline mode", "X3_AGENT_KEY_CLI_OFFLINE_SEND"),
    ("unrecognized private diagnostic", "X3_AGENT_KEY_CLI_SIGNING_FAILED"),
])
def test_local_cli_failure_is_sanitized_and_cannot_broadcast(tmp_path, message, code):
    from subprocess import CompletedProcess
    path, value = credential(tmp_path); rpc = Rpc(restricted())
    def runner(command, **kwargs): return CompletedProcess(command, 1, value["private_key"], message + value["private_key"])
    with pytest.raises(keys.AgentKeyError) as error:
        send(path, value, rpc, runner)
    assert str(error.value) == code
    assert len(rpc.requests) == 2
