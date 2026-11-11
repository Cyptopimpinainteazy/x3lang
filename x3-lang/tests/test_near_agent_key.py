"""Operator signing boundary tests use disposable synthetic credentials only.

RPC and the near CLI process are intercepted. No real signatures or transactions.
"""
import json

import pytest

import near_agent_key as keys

PUBLIC = "ed25519:AKnL4NNf3DGWZJS6cPknBuEGnVsV4A4m5tgebLHaRSZ9"
METHODS = ["agent_register_intent", "agent_accept_quote", "agent_record_outcome"]


class Rpc:
    def __init__(self, permission):
        self.permission = permission
        self.requests = []

    def __call__(self, method, url, headers, body, timeout):
        request = json.loads(body)
        self.requests.append(request)
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
        return CompletedProcess(command, 0, "", "")

    assert send(path, value, transport, runner) == 0
    assert len(calls) == 1 and len(rpc.requests) == 2
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
    with pytest.raises(keys.AgentKeyError, match="X3_AGENT_KEY_CLI_FAILED_OUTCOME_UNKNOWN") as error:
        send(path, value, Rpc(restricted()), runner)
    assert len(snapshots) == 1 and not snapshots[0].exists()
    assert value["private_key"] not in str(error.value)


def test_cli_output_emits_only_safe_metadata_and_binds_finalized_nonce_and_block(tmp_path, capsys):
    from subprocess import CompletedProcess
    path, value = credential(tmp_path)
    payload = {"signed_payload": '{"depositAddress":"PRIVATE-RECEIPT-MARKER"}',
               "signature": "PRIVATE-SIGNATURE-MARKER"}
    transaction_hash = "11111111111111111111111111111111"
    def runner(command, **kwargs):
        assert command[command.index("--nonce") + 1] == "2"
        assert command[command.index("--block-hash") + 1] == transaction_hash
        assert command[command.index("--block-height") + 1] == "123"
        stdout = (json.dumps(command) + "\n" + json.dumps(json.dumps(payload)) + "\n"
                  + value["private_key"] + "\nTransaction ID: " + transaction_hash + "\n")
        return CompletedProcess(command, 1, stdout, "X3_MANDATE_DENIED " + value["private_key"][8:])
    assert send(path, value, Rpc(restricted()), runner, args=payload) == 1
    output = capsys.readouterr()
    assert output.err == ""
    metadata = json.loads(output.out)
    assert metadata == {"cli_exit_code": 1, "transaction_hash_candidate": transaction_hash,
                        "error_code_candidates": ["X3_MANDATE_DENIED"], "verified": False,
                        "broadcasted": "unknown"}
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
