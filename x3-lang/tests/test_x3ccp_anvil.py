"""X3CCP parity: x3ccp.predict() against real mined transactions on a local anvil EVM.

Deploys the compiled X3ControlledTreasury to a fresh anvil chain (31337), compiles a `control`
policy for that exact address, installs it in a transaction, then mines a seeded randomized
corpus of calls. Every mined call must match the predictor: success or failure, the exact
revert data (`X3Refused(clause, code)`, or empty when Solidity's ABI decoder refuses first),
and the ETH actually paid out.

This is a LOCAL EVM run: real bytecode and real transactions, but not a public chain and not
evidence of any Base Sepolia deployment. It is skipped when anvil or forge is unavailable.
"""

from __future__ import annotations

import json
import os
import random
import shutil
import socket
import subprocess
import sys
import time
import urllib.request

import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

import x3ccp  # noqa: E402

EVM = os.path.join(ROOT, "x3ccp-evm")
DEPLOYER = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266"  # anvil's default unlocked account 0
CALLERS = ["0x1111111111111111111111111111111111111111", "0x3333333333333333333333333333333333333333"]
OUTSIDER = "0x6666666666666666666666666666666666666666"
RECIPIENTS = ["0x2222222222222222222222222222222222222222", "0x4444444444444444444444444444444444444444"]
T0 = 1_900_000_000


def _tool(name):
    for candidate in (os.environ.get(f"X3CCP_{name.upper()}"), shutil.which(name), f"/opt/foundry/{name}",
                      os.path.expanduser(f"~/.foundry/bin/{name}")):
        if candidate and os.path.exists(candidate):
            return candidate
    return None


ANVIL, FORGE = _tool("anvil"), _tool("forge")
pytestmark = pytest.mark.skipif(not (ANVIL and FORGE), reason="anvil/forge not installed (local EVM parity skipped)")


class Rpc:
    def __init__(self, url):
        self.url = url
        self.n = 0

    def __call__(self, method, *params):
        self.n += 1
        body = json.dumps({"jsonrpc": "2.0", "id": self.n, "method": method, "params": list(params)}).encode()
        req = urllib.request.Request(self.url, body, {"Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=30) as resp:
            out = json.loads(resp.read())
        if "error" in out:
            raise RuntimeError(f"{method}: {out['error']}")
        return out["result"]


@pytest.fixture(scope="module")
def chain():
    subprocess.run([FORGE, "build", "--offline"], cwd=EVM, check=True, capture_output=True)
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    proc = subprocess.Popen([ANVIL, "--port", str(port), "--chain-id", "31337", "--auto-impersonate",
                             "--timestamp", str(T0 - 1000), "--silent"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    rpc = Rpc(f"http://127.0.0.1:{port}")
    for _ in range(100):
        try:
            rpc("eth_chainId")
            break
        except Exception:
            time.sleep(0.1)
    else:
        proc.kill()
        pytest.skip("anvil did not start")
    yield rpc
    proc.kill()
    proc.wait()


def _send(rpc, sender, to, data, ts, value=0):
    rpc("evm_setNextBlockTimestamp", hex(ts))
    tx = {"from": sender, "data": data, "gas": hex(3_000_000), "value": hex(value)}
    if to:
        tx["to"] = to
    h = rpc("eth_sendTransaction", tx)
    receipt = None
    for _ in range(200):
        receipt = rpc("eth_getTransactionReceipt", h)
        if receipt:
            break
        time.sleep(0.02)
    assert receipt, f"transaction {h} was not mined"
    trace = rpc("debug_traceTransaction", h, {"tracer": "callTracer"})
    return receipt, trace


def _balance(rpc, who):
    return int(rpc("eth_getBalance", who, "latest"), 16)


def test_predictor_matches_mined_transactions(chain):
    rpc = chain
    with open(os.path.join(EVM, "out", "X3ControlledTreasury.sol", "X3ControlledTreasury.json")) as fh:
        bytecode = json.load(fh)["bytecode"]["object"]
    ts = T0
    receipt, _ = _send(rpc, DEPLOYER, None, bytecode, ts)
    assert receipt["status"] == "0x1"
    target = x3ccp.to_checksum(receipt["contractAddress"])
    for who in CALLERS + [OUTSIDER]:
        rpc("anvil_setBalance", who, hex(10**21))
    ts += 1
    _send(rpc, DEPLOYER, target, "0x", ts, value=10**18)  # fund the treasury through receive()

    source = f"""control anvil_parity {{
    target local {target}
    owner {DEPLOYER}
    allow callers {CALLERS[0]}, {CALLERS[1]}
    allow fn withdraw(address to, uint256 amount)
    allow fn setMemo(bytes32 memo, bool urgent)
    limit withdraw.amount <= 400_000
    limit withdraw.amount >= 1_000
    limit withdraw.to in {RECIPIENTS[0]}, {RECIPIENTS[1]}
    rate withdraw.amount per 1h <= 1_000_000
    rate withdraw calls per 10m <= 4
    limit setMemo.urgent == false
    rate setMemo calls per 1m <= 2
    expires at 2031-01-01T00:00:00Z
}}
"""
    compiled = x3ccp.compile_source(source)
    ts += 1
    withdraw_probe = x3ccp.encode_call(compiled, "withdraw", [RECIPIENTS[0], 5_000])
    receipt, trace = _send(rpc, CALLERS[0], target, "0x" + withdraw_probe.hex(), ts)
    assert receipt["status"] == "0x0", "no policy installed yet: fail closed"
    assert x3ccp.decode_refusal(bytes.fromhex(trace["output"][2:]))["reason"] == "NO_POLICY"
    install = x3ccp.keccak256(b"installPolicy(bytes)")[:4]
    policy = bytes.fromhex(compiled["policy_hex"][2:])
    padded = policy + bytes((32 - len(policy) % 32) % 32)
    calldata = install + (32).to_bytes(32, "big") + len(policy).to_bytes(32, "big") + padded
    ts += 1
    receipt, _ = _send(rpc, DEPLOYER, target, "0x" + calldata.hex(), ts)
    assert receipt["status"] == "0x1", "policy install transaction must succeed"
    on_chain_hash = rpc("eth_call", {"to": target, "data": "0x" + x3ccp.keccak256(b"x3PolicyHash()")[:4].hex()},
                        "latest")
    assert on_chain_hash == compiled["policy_hash"]

    rng = random.Random(20261126)
    state = x3ccp.EnforcerState()
    stats = {"allowed": 0, "refused": 0, "decoder": 0}
    reasons = set()
    withdraw = x3ccp.keccak256(b"withdraw(address,uint256)")[:4]
    for step in range(320):
        # Alternate bursts (to hit call-rate limits) with gaps (to roll windows over).
        ts += rng.choice([1, 1, 1, 2, 5]) if (step // 20) % 2 == 0 else rng.choice([1, 30, 61, 300, 700, 3600])
        sender = rng.choice(CALLERS * 4 + [OUTSIDER])
        roll = rng.random()
        if roll < 0.62:
            amount = rng.choice([rng.randint(1_000, 400_000), rng.randint(0, 999), rng.randint(400_001, 900_000),
                                 1_000, 400_000])
            to = rng.choice(RECIPIENTS * 5 + [OUTSIDER])
            data = x3ccp.encode_call(compiled, "withdraw", [to, amount])
        elif roll < 0.80:
            data = x3ccp.encode_call(compiled, "setMemo", ["0x" + rng.randbytes(32).hex(), rng.random() < 0.3])
        elif roll < 0.86:
            data = x3ccp.keccak256(b"sweep(address)")[:4] + bytes(12) + bytes.fromhex(RECIPIENTS[0][2:])
        elif roll < 0.92:
            data = x3ccp.encode_call(compiled, "withdraw", [RECIPIENTS[0], 5_000]) + bytes(32)
        else:
            dirty = bytearray(withdraw + bytes(12) + bytes.fromhex(RECIPIENTS[1][2:]) + (5_000).to_bytes(32, "big"))
            dirty[4 + rng.randint(0, 11)] = rng.randint(1, 255)
            data = bytes(dirty)
        expected = x3ccp.predict(compiled, state, sender=sender, calldata=data, timestamp=ts, chain_id=31337)
        paid_to = paid = None
        if expected["allowed"] and data[:4] == withdraw:
            paid_to = x3ccp.to_checksum("0x" + data[16:36].hex())
            paid = int.from_bytes(data[36:68], "big")
            before = _balance(rpc, paid_to)
        receipt, trace = _send(rpc, sender, target, "0x" + data.hex(), ts)
        block = rpc("eth_getBlockByNumber", receipt["blockNumber"], False)
        assert int(block["timestamp"], 16) == ts
        if expected["allowed"]:
            assert receipt["status"] == "0x1", (expected, trace)
            stats["allowed"] += 1
            if paid_to:
                assert _balance(rpc, paid_to) - before == paid
        else:
            assert receipt["status"] == "0x0", (expected, data.hex())
            got = bytes.fromhex(trace.get("output", "0x")[2:])
            if expected.get("abi_decoder_first"):
                assert got == b"", (expected, got.hex())
                stats["decoder"] += 1
            else:
                assert x3ccp.decode_refusal(got) == {"clause": expected["clause"], "code": expected["code"],
                                                     "reason": expected["reason"]}, (expected, got.hex())
            stats["refused"] += 1
            reasons.add(expected["reason"])
    print(f"x3ccp anvil parity: {stats}, reasons={sorted(reasons)}")
    # The corpus must actually exercise the enforcer, not just one path.
    assert stats["allowed"] >= 25 and stats["refused"] >= 40 and stats["decoder"] >= 3, stats
    assert {"CALLER_NOT_ALLOWED", "FUNCTION_NOT_ALLOWED", "BAD_CALLDATA", "ABOVE_MAX", "BELOW_MIN", "NOT_IN_SET",
            "NOT_EQUAL", "RATE_SUM_EXCEEDED", "RATE_CALLS_EXCEEDED"} <= reasons, reasons
