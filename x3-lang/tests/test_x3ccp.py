"""X3CCP compiler, decoder, predictor and diagnostics."""

from __future__ import annotations

import json
import os
import subprocess
import sys
from datetime import datetime, timezone

import pytest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)

import x3ccp  # noqa: E402

NOW = datetime(2026, 11, 26, tzinfo=timezone.utc)
EXAMPLE = os.path.join(ROOT, "examples", "x3ccp_treasury_guard.x3")
AGENT = "0x1111111111111111111111111111111111111111"
OPS = "0x3333333333333333333333333333333333333333"
STORE = "0x2222222222222222222222222222222222222222"
BOB = "0x4444444444444444444444444444444444444444"
MALLORY = "0x6666666666666666666666666666666666666666"
T0 = 1_800_000_000


def _example():
    with open(EXAMPLE, encoding="utf-8") as fh:
        return x3ccp.compile_source(fh.read(), now=NOW)


def _src(*clauses: str, header: str = "control p {") -> str:
    base = [
        header,
        "target local 0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f",
        "owner 0x7FA9385bE102ac3EAc297483Dd6233D62b3e1496",
        f"allow callers {AGENT}",
        "allow fn withdraw(address to, uint256 amount)",
        *clauses,
        "expires 30d",
        "}",
    ]
    return "\n".join(base) + "\n"


def _err(source: str) -> x3ccp.X3CCPError:
    with pytest.raises(x3ccp.X3CCPError) as info:
        x3ccp.compile_source(source, now=NOW)
    return info.value


# ------------------------------------------------------------------------ compile


def test_keccak_and_selector_are_ethereum_values():
    assert x3ccp.keccak256(b"transfer(address,uint256)")[:4].hex() == "a9059cbb"
    c = _example()
    withdraw = next(f for f in c["functions"] if f["name"] == "withdraw")
    assert withdraw["selector"] == "0x" + x3ccp.keccak256(b"withdraw(address,uint256)")[:4].hex()
    assert x3ccp.REFUSED_SELECTOR == x3ccp.keccak256(b"X3Refused(uint16,uint8)")[:4]


def test_example_compiles_deterministically_and_round_trips():
    a, b = _example(), _example()
    assert a["policy_hex"] == b["policy_hex"]
    assert a["policy_hash"] == "0x" + x3ccp.keccak256(bytes.fromhex(a["policy_hex"][2:])).hex()
    decoded = x3ccp.decode_policy(bytes.fromhex(a["policy_hex"][2:]))
    assert decoded["chain_id"] == 31337 and decoded["callers"] == [AGENT, OPS]
    assert [f["line"] for f in decoded["functions"]] == [17, 18]
    assert [c["line"] for c in decoded["functions"][0]["checks"]] == [19, 20, 21, 22, 23]
    assert a["expires_at"] == 1_893_456_000


def test_fixtures_are_current():
    result = subprocess.run([sys.executable, os.path.join(ROOT, "tools", "x3ccp_fixtures.py"), "--check"],
                            capture_output=True, text=True)
    assert result.returncode == 0, result.stderr


def test_relative_expiry_uses_compile_time():
    c = x3ccp.compile_source(_src(), now=NOW)
    assert c["expires_at"] == int(NOW.timestamp()) + 30 * 86_400


@pytest.mark.parametrize("bad, code, line", [
    (_src(header="contract p {"), "X3CCP_PARSE", 1),
    (_src("limit withdraw.amount <= 5").replace("expires 30d\n", ""), "X3CCP_MISSING", None),
    (_src("limit withdraw.amt <= 5"), "X3CCP_UNKNOWN_PARAM", 6),
    (_src("limit pay.amount <= 5"), "X3CCP_UNKNOWN_FN", 6),
    (_src("limit withdraw.to <= 5"), "X3CCP_OPERATOR", 6),
    (_src("limit withdraw.amount >= 10", "limit withdraw.amount <= 5"), "X3CCP_CONFLICT", 6),
    (_src("rate withdraw.amount per 1h <= 0"), "X3CCP_CONFLICT", 6),
    (_src("rate withdraw.to per 1h <= 5"), "X3CCP_OPERATOR", 6),
    (_src("rate withdraw calls per 400d <= 5"), "X3CCP_DURATION", 6),
    (_src("limit withdraw.amount <= 5", "limit withdraw.amount <= 6"), "X3CCP_DUPLICATE", 7),
    (_src("limit withdraw.to in 0x2222222222222222222222222222222222222222, "
          "0x2222222222222222222222222222222222222222"), "X3CCP_DUPLICATE", 6),
    (_src("allow fn pay(string memo)"), "X3CCP_TYPE", 6),
    (_src("frobnicate everything"), "X3CCP_PARSE", 6),
    (_src().replace("target local", "target solana"), "X3CCP_CHAIN", 2),
    (_src().replace("0x5615dEB798BB3E4dFa0139dFa1b3D433Cc23b72f", "0x5615DEB798BB3E4dFa0139dFa1b3D433Cc23b72f"),
     "X3CCP_CHECKSUM", 2),
    (_src().replace("expires 30d", "expires at 2020-01-01T00:00:00Z"), "X3CCP_EXPIRY", 6),
    (_src("limit withdraw.amount <= 1_0x"), "X3CCP_NUMBER", 6),
])
def test_compile_errors_carry_code_line_and_hint(bad, code, line):
    err = _err(bad)
    assert err.code == code
    assert err.line == line
    assert err.to_dict()["code"] == code


def test_every_error_has_a_hint_for_models():
    for bad in (_src("limit withdraw.amt <= 5"), _src("rate withdraw.to per 1h <= 5"), _src("nope")):
        assert _err(bad).hint


def test_shadowed_cap_is_a_warning_not_an_error():
    c = x3ccp.compile_source(_src("limit withdraw.amount <= 100", "rate withdraw.amount per 1h <= 50"), now=NOW)
    assert [w["code"] for w in c["warnings"]] == ["X3CCP_CAP_SHADOWED"]


def test_uint_width_is_enforced_at_compile_time():
    src = _src().replace("uint256 amount", "uint8 amount")
    assert x3ccp.compile_source(src + "", now=NOW)
    err = _err(src.replace("expires 30d", "limit withdraw.amount <= 256\nexpires 30d"))
    assert err.code == "X3CCP_NUMBER"


def test_decoder_rejects_malformed_bytes():
    raw = bytes.fromhex(_example()["policy_hex"][2:])
    for mutated, msg in ((b"Y" + raw[1:], "bad magic"), (raw + b"\x00", "trailing"), (raw[:-1], "end early")):
        with pytest.raises(ValueError, match=msg):
            x3ccp.decode_policy(mutated)


# ------------------------------------------------------------------------ predict


def _call(c, state, sender, fn, args, ts=T0, chain=31337):
    return x3ccp.predict(c, state, sender=sender, calldata=x3ccp.encode_call(c, fn, args), timestamp=ts,
                         chain_id=chain)


def test_predict_each_clause():
    c = _example()
    s = x3ccp.EnforcerState()
    assert _call(c, s, AGENT, "withdraw", [STORE, 5_000])["allowed"]
    assert _call(c, s, MALLORY, "withdraw", [STORE, 5_000]) == {"allowed": False, "clause": 16, "code": 4,
                                                                 "reason": "CALLER_NOT_ALLOWED"}
    assert _call(c, s, AGENT, "withdraw", [STORE, 50_000_001])["clause"] == 19
    assert _call(c, s, AGENT, "withdraw", [STORE, 999])["clause"] == 20
    assert _call(c, s, AGENT, "withdraw", [MALLORY, 5_000])["reason"] == "NOT_IN_SET"
    assert _call(c, s, OPS, "setMemo", ["0x" + "00" * 32, True])["clause"] == 24
    assert _call(c, s, AGENT, "withdraw", [STORE, 5_000], ts=1_893_456_000)["reason"] == "EXPIRED"
    assert _call(c, s, AGENT, "withdraw", [STORE, 5_000], chain=84532)["reason"] == "WRONG_CHAIN"
    assert x3ccp.predict(c, x3ccp.EnforcerState(active=False), sender=AGENT,
                         calldata=x3ccp.encode_call(c, "withdraw", [STORE, 5_000]), timestamp=T0,
                         chain_id=31337)["reason"] == "NO_POLICY"


def test_predict_rate_windows_and_refusals_consume_nothing():
    c = _example()
    s = x3ccp.EnforcerState()
    assert _call(c, s, AGENT, "withdraw", [STORE, 50_000_000])["allowed"]
    assert _call(c, s, AGENT, "withdraw", [STORE, 50_000_000])["allowed"]
    assert _call(c, s, AGENT, "withdraw", [STORE, 30_000_000])["reason"] == "RATE_SUM_EXCEEDED"
    assert _call(c, s, AGENT, "withdraw", [STORE, 20_000_000])["allowed"]
    # The 24h budget is now exactly spent; line 22 (sum) is checked before line 23 (calls).
    assert _call(c, s, AGENT, "withdraw", [STORE, 1_000])["clause"] == 22
    next_hour = (T0 // 3600 + 1) * 3600
    assert _call(c, s, AGENT, "withdraw", [STORE, 1_000], ts=next_hour)["reason"] == "RATE_SUM_EXCEEDED"
    next_day = (T0 // 86_400 + 1) * 86_400
    assert _call(c, s, AGENT, "withdraw", [STORE, 1_000], ts=next_day)["allowed"]
    calls = x3ccp.EnforcerState()
    for _ in range(3):
        assert _call(c, calls, AGENT, "withdraw", [STORE, 1_000])["allowed"]
    assert _call(c, calls, AGENT, "withdraw", [MALLORY, 1_000])["reason"] == "NOT_IN_SET"
    assert _call(c, calls, AGENT, "withdraw", [STORE, 1_000])["clause"] == 23
    assert _call(c, calls, AGENT, "withdraw", [STORE, 1_000], ts=next_hour)["allowed"]


def test_predict_unlisted_function_and_bad_calldata():
    c = _example()
    s = x3ccp.EnforcerState()
    sweep = x3ccp.keccak256(b"sweep(address)")[:4] + bytes(12) + bytes.fromhex(STORE[2:])
    assert x3ccp.predict(c, s, sender=AGENT, calldata=sweep, timestamp=T0, chain_id=31337)["clause"] == 13
    good = x3ccp.encode_call(c, "withdraw", [STORE, 5_000])
    trailing = x3ccp.predict(c, s, sender=AGENT, calldata=good + bytes(32), timestamp=T0, chain_id=31337)
    assert trailing["reason"] == "BAD_CALLDATA" and not trailing.get("abi_decoder_first")
    dirty = bytearray(good)
    dirty[4] = 1
    out = x3ccp.predict(c, s, sender=AGENT, calldata=bytes(dirty), timestamp=T0, chain_id=31337)
    assert out["reason"] == "BAD_CALLDATA" and out["abi_decoder_first"]


def test_revert_data_round_trip():
    data = x3ccp.refusal_revert_data(21, 10)
    assert x3ccp.decode_refusal(data) == {"clause": 21, "code": 10, "reason": "NOT_IN_SET"}
    assert x3ccp.decode_refusal(b"\x00" * 68) is None


def test_explain_has_one_sentence_per_clause_in_source_terms():
    out = x3ccp.explain(_example())
    lines = [item["line"] for item in out]
    assert sorted(lines) == [14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25]
    text = " ".join(item["text"] for item in out)
    assert "at most 50,000,000" in text and "3 times in any fixed 1-hour window" in text
    assert "24-hour" in text and "2030-01-01 00:00 UTC" in text


def test_cli_compile_and_error_exit_code(tmp_path):
    ok = subprocess.run([sys.executable, os.path.join(ROOT, "x3ccp.py"), "compile", EXAMPLE], capture_output=True,
                        text=True)
    assert ok.returncode == 0 and json.loads(ok.stdout)["format"] == "x3ccp-v1"
    bad = tmp_path / "bad.x3"
    bad.write_text(_src("limit withdraw.amt <= 5"))
    out = subprocess.run([sys.executable, os.path.join(ROOT, "x3ccp.py"), "compile", str(bad)], capture_output=True,
                         text=True)
    assert out.returncode == 2 and json.loads(out.stderr)["error"]["code"] == "X3CCP_UNKNOWN_PARAM"


@pytest.mark.parametrize("chain,chain_id", [
    ("base", 8453), ("ethereum", 1), ("mainnet", 1), ("optimism", 10), ("arbitrum", 42161), ("bsc", 56),
])
def test_mainnet_targets_compile_with_an_unaudited_warning(chain, chain_id):
    compiled = x3ccp.compile_source(_src().replace("target local", f"target {chain}"), now=NOW)
    assert compiled["chain"] == chain
    assert compiled["chain_id"] == chain_id
    assert [w["code"] for w in compiled["warnings"]] == ["X3CCP_MAINNET_UNAUDITED"]
    assert compiled["warnings"][0]["line"] == 2
    decoded = x3ccp.decode_policy(bytes.fromhex(compiled["policy_hex"][2:]))
    assert decoded["chain_id"] == chain_id
    target_text = next(item["text"] for item in x3ccp.explain(compiled) if item["line"] == 2)
    assert "mainnet: real funds, unaudited enforcer" in target_text


def test_testnet_targets_carry_no_mainnet_warning():
    compiled = x3ccp.compile_source(_src(), now=NOW)
    assert all(w["code"] != "X3CCP_MAINNET_UNAUDITED" for w in compiled["warnings"])
    target_text = next(item["text"] for item in x3ccp.explain(compiled) if item["line"] == 2)
    assert "mainnet" not in target_text


def test_mainnet_policy_still_refuses_on_the_wrong_chain():
    compiled = x3ccp.compile_source(_src().replace("target local", "target base"), now=NOW)
    state = x3ccp.EnforcerState()
    call = x3ccp.encode_call(compiled, "withdraw", ["0x2222222222222222222222222222222222222222", 5])
    verdict = x3ccp.predict(compiled, state, sender=AGENT, calldata=call,
                            timestamp=int(NOW.timestamp()), chain_id=84532)
    assert verdict["allowed"] is False
    assert verdict["reason"] == "WRONG_CHAIN"
