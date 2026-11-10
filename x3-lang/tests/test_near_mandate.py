"""Agent mandates: the X3 `mandate` declaration, `under mandate`, and the agent's x3-guard calls.

`near/x3-guard/src/mandate_tests.rs` reads `fixtures/x3_guard_mandate_args.json`, which this file
generates, so the terms and policies built here are the ones the contract is tested on, and the
contract's checks must give the verdicts recorded here.
"""
import base64
import json
import os
from pathlib import Path

import pytest

import near_agent as na
from test_near_intents import (  # noqa: F401  (_trust_the_test_key is an autouse fixture)
    NEAR_USDC, NOW, ROOT, SOL_ADDR, SOL_USDC, TOKENS, RecordingTransport, _status, _trust_the_test_key, api, near_rpc,
    ni,
)

MANDATE = ROOT / "examples" / "near_mandate_agent.x3"
AGENT_INTENT = ROOT / "examples" / "near_intents_agent_payment.x3"
REFUSED_INTENT = ROOT / "tests" / "fixtures" / "near_intents_agent_refused_payment.x3"
MANDATE_FIXTURE = ROOT / "tests" / "fixtures" / "x3_guard_mandate_args.json"
NOW_MS = int(NOW.timestamp() * 1000)
nm = ni.mandates()


def tokens():
    return json.loads(TOKENS.read_text())


def mandate_file(tmp_path, *lines, name="m.x3"):
    body = ["mandate m {", *[f"    {line}" for line in lines], "}"]
    path = tmp_path / name
    path.write_text("\n".join(body))
    return str(path)


BASE = [
    "owner x3-owner.near",
    "agent x3-agent.near",
    "spend Solana.USDC",
    "per_payment 50000000",
    "per_24h 200000000",
    "allow recipients x3-demo.near",
    "allow assets Near.USDC",
    f"refund_to {SOL_ADDR}",
    "require slippage <= 1%",
    "require fees <= 100 bps",
    "expires 30d",
]


def replaced(key, line):
    """BASE with the clause starting with `key` replaced by `line` (removed when None)."""
    out = [b for b in BASE if not b.startswith(key)]
    return out + ([line] if line else [])


def agent_intent(tmp_path, *, amount=25_000_000, receiver="x3-demo.near", owner="x3-owner.near", slippage="0.5%",
                 fees="require fees <= 100 bps", nonce="agent_001", timeout="10m", name="i.x3"):
    body = "\n".join(line for line in [
        "intent agent_payment {",
        f"    from Solana.USDC amount {amount} receiver {SOL_ADDR}",
        f"    to Near.USDC receiver {receiver}",
        "    route {",
        f"        bridge NearIntents Solana.USDC -> Near.USDC min_receive {amount * 99 // 100};",
        "    }",
        f"    under mandate {owner}" if owner else "",
        f"    require slippage <= {slippage}",
        f"    {fees}" if fees else "",
        f"    require nonce unused {nonce}" if nonce else "",
        f"    timeout {timeout} refund Solana.USDC",
        "    on_fail rollback",
        "}",
    ] if line)
    path = tmp_path / name
    path.write_text(body)
    return str(path)


# --- the declaration -------------------------------------------------------------------------

def test_the_example_mandate_compiles_to_create_mandate_terms():
    compiled = nm.compile_mandate(str(MANDATE), tokens(), now=NOW)
    assert (compiled["owner"], compiled["agent"]) == ("x3-owner.near", "x3-agent.near")
    t = compiled["terms"]
    assert t["asset"] == SOL_USDC and t["destination_assets"] == [NEAR_USDC]
    assert (t["per_payment_cap"], t["daily_cap"]) == ("50000000", "200000000")
    assert t["recipients"] == ["x3-demo.near"] and t["refund_to"] == SOL_ADDR
    assert (t["max_slippage_bps"], t["max_fee_bps"]) == (100, 100)
    assert (t["approval_above"], t["approvers"], t["approval_threshold"]) == ("30000000", ["x3-owner.near"], 1)
    assert t["expires_at_ms"] == NOW_MS + 30 * 86_400_000
    assert t["program_sha256"] == __import__("hashlib").sha256(MANDATE.read_bytes()).hexdigest()


@pytest.mark.parametrize("lines,code,fragment", [
    (BASE + ["spend_everything yes"], "X3_MANDATE_PARSE", "is not a mandate clause"),
    (BASE + ["per_payment 1"], "X3_MANDATE_PARSE", "appears twice"),
    (replaced("expires", None), "X3_MANDATE_PARSE", "must state `expires`"),
    (replaced("allow recipients", None), "X3_MANDATE_PARSE", "must state `allow recipients`"),
    (BASE + ["require nonce unused x"], "X3_MANDATE_PARSE", "belongs in the agent's intent"),
    (BASE + ["approve above 1 by two of a.near"], "X3_MANDATE_PARSE", "expected `approve above"),
])
def test_the_parser_refuses_what_it_does_not_know(tmp_path, lines, code, fragment):
    with pytest.raises(ni.NearIntentsError) as err:
        nm.compile_mandate(mandate_file(tmp_path, *lines), tokens(), now=NOW)
    assert err.value.code == code and fragment in err.value.message, err.value.message


def test_an_unclosed_mandate_or_trailing_text_is_refused(tmp_path):
    open_path = tmp_path / "open.x3"
    open_path.write_text("mandate m {\n    owner a.near\n")
    with pytest.raises(ni.NearIntentsError, match="never closed"):
        nm.parse_mandate(str(open_path))
    trailing = tmp_path / "trailing.x3"
    trailing.write_text("mandate m {\n" + "\n".join(BASE) + "\n}\nintent x {")
    with pytest.raises(ni.NearIntentsError, match="after the mandate's closing brace"):
        nm.parse_mandate(str(trailing))


@pytest.mark.parametrize("lines,code", [
    (replaced("agent", "agent x3-owner.near"), "X3_MANDATE_ACCOUNT"),
    (replaced("agent", "agent Not_An_Account"), "X3_MANDATE_ACCOUNT"),
    (replaced("per_24h", "per_24h 1000"), "X3_MANDATE_LIMIT"),
    (replaced("per_payment", "per_payment 0"), "X3_NEAR_AMOUNT"),
    (replaced("per_payment", "per_payment 1.5"), "X3_NEAR_AMOUNT"),
    (replaced("refund_to", "refund_to x3-owner.near"), "X3_MANDATE_REFUND_TO"),
    (replaced("allow recipients", "allow recipients x3-demo.near, x3-demo.near"), "X3_MANDATE_LIST"),
    (replaced("allow recipients", "allow recipients UPPER.near"), "X3_MANDATE_RECIPIENT"),
    (replaced("allow assets", "allow assets Near.NOPE"), "X3_NEAR_ASSET_UNSUPPORTED"),
    (replaced("require fees", None), "X3_MANDATE_BOUND"),
    (replaced("require slippage", "require slippage >= 1%"), "X3_NEAR_GUARD_OPERATOR"),
    (replaced("expires", "expires 10 blocks"), "X3_MANDATE_PARSE"),
    (replaced("expires", "expires 0d"), "X3_MANDATE_EXPIRES"),
    (BASE + ["approve above 50000000 by 1 of x3-owner.near"], "X3_MANDATE_APPROVAL"),
    (BASE + ["approve above 1 by 1 of x3-agent.near"], "X3_MANDATE_APPROVAL"),
    (BASE + ["approve above 1 by 3 of x3-owner.near, carol.near"], "X3_MANDATE_APPROVAL"),
    (BASE + ["approve above 1 by 1 of carol.near, carol.near"], "X3_MANDATE_APPROVAL"),
])
def test_terms_that_would_not_mean_what_they_say_are_refused(tmp_path, lines, code):
    with pytest.raises(ni.NearIntentsError) as err:
        nm.compile_mandate(mandate_file(tmp_path, *lines), tokens(), now=NOW)
    assert err.value.code == code, err.value.to_dict()


# --- `under mandate` ---------------------------------------------------------------------------

def test_under_mandate_lowers_to_an_owner_and_needs_a_nonce(tmp_path):
    _, leg = ni.compile_program(agent_intent(tmp_path), now=NOW)
    assert leg["mandate_owner"] == "x3-owner.near"
    _, plain = ni.compile_program(str(ROOT / "examples" / "near_intents_sol_to_near.x3"), now=NOW)
    assert "mandate_owner" not in plain, "programs without a mandate lower exactly as before"
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(agent_intent(tmp_path, nonce=None), now=NOW)
    assert err.value.code == "X3_NEAR_NONCE"
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(agent_intent(tmp_path, owner="NotAnAccount"), now=NOW)
    assert err.value.code == "X3_MANDATE_OWNER"
    bad = tmp_path / "bad.x3"
    source = Path(agent_intent(tmp_path)).read_text()
    bad.write_text(source.replace("under mandate x3-owner.near", "under owner x3-owner.near"))
    with pytest.raises(ni.NearIntentsError) as err:
        ni.compile_program(str(bad), now=NOW)
    assert err.value.code == "X3_PARSE_MANDATE"


def _policy(tmp_path, **kwargs):
    _, leg = ni.compile_program(agent_intent(tmp_path, **kwargs), now=NOW)
    request = ni.build_quote_request(leg, tokens(), dry=True)
    return ni.policy_for(leg, request, "0" * 64)


def _compiled():
    c = nm.compile_mandate(str(MANDATE), tokens(), now=NOW)
    return {"owner": c["owner"], "agent": c["agent"], "terms": c["terms"], "version": None, "revoked": False, "spends": []}


def _failed(results):
    return [r["guard"] for r in results if not r["ok"]]


POLICY_CASES = [
    ("allowed", {}, []),
    ("recipient", {"receiver": "attacker.near"}, ["mandate_recipient"]),
    ("per_payment", {"amount": 50_000_001}, ["mandate_per_payment"]),
    ("slippage", {"slippage": "1.5%"}, ["mandate_slippage"]),
    ("no_fee_cap", {"fees": None}, ["mandate_fees"]),
    ("fee_cap", {"fees": "require fees <= 150 bps"}, ["mandate_fees"]),
]


@pytest.mark.parametrize("name,kwargs,expected", POLICY_CASES)
def test_each_term_refuses_an_intent_that_breaks_it(tmp_path, name, kwargs, expected):
    assert _failed(nm.check_mandate(_compiled(), _policy(tmp_path, **kwargs), NOW_MS)) == expected


def test_refund_asset_and_deadline_terms(tmp_path):
    m = _compiled()
    p = _policy(tmp_path)
    assert _failed(nm.check_mandate(m, {**p, "refund_to": "AgentsOwnWallet1111111111111111"}, NOW_MS)) == ["mandate_refund_to"]
    assert _failed(nm.check_mandate(m, {**p, "origin_asset": "nep141:wrap.near"}, NOW_MS)) == ["mandate_asset"]
    assert _failed(nm.check_mandate(m, {**p, "destination_asset": "nep141:wrap.near"}, NOW_MS)) == ["mandate_destination_asset"]
    late = m["terms"]["expires_at_ms"] + 1
    assert _failed(nm.check_mandate(m, {**p, "deadline_ms": late}, NOW_MS)) == ["mandate_deadline"]
    assert _failed(nm.check_mandate(m, p, m["terms"]["expires_at_ms"])) == ["mandate_active"]
    assert _failed(nm.check_mandate({**m, "revoked": True}, p, NOW_MS)) == ["mandate_active"]


def test_the_daily_cap_counts_a_rolling_24_hours(tmp_path):
    m = _compiled()
    p = _policy(tmp_path)  # 25 USDC
    spends = [{"at_ms": NOW_MS - 86_400_000 + 1, "amount": "100000000", "nonce": "a"},
              {"at_ms": NOW_MS - 3_600_000, "amount": "80000000", "nonce": "b"}]
    assert _failed(nm.check_mandate({**m, "spends": spends}, p, NOW_MS)) == ["mandate_daily"]
    # One millisecond later the 100 USDC payment is 24 hours old and leaves the window.
    assert _failed(nm.check_mandate({**m, "spends": spends}, p, NOW_MS + 1)) == []
    many = [{"at_ms": NOW_MS - i, "amount": "1", "nonce": str(i)} for i in range(64)]
    assert _failed(nm.check_mandate({**m, "spends": many}, p, NOW_MS)) == ["mandate_daily"]
    assert nm.approvals_needed(m, 30_000_000) == 0 and nm.approvals_needed(m, 30_000_001) == 1


# --- quoting as the agent ------------------------------------------------------------------

class GuardViews:
    """NEAR `call_function` views: x3-guard's `get_mandate`, and `ft_balance_of` for the baseline."""

    def __init__(self, mandate):
        self.mandate = mandate
        self.height = 180_000_000

    def __call__(self, method, url, payload):
        params = payload["params"]
        args = json.loads(base64.b64decode(params["args_base64"]))
        if params["method_name"] == "get_mandate":
            value = self.mandate if self.mandate and (args["owner"], args["agent"]) == (
                self.mandate["owner"], self.mandate["agent"]) else None
        else:
            value = "0"
        self.height += 1
        return 200, {"jsonrpc": "2.0", "id": payload["id"], "result": {
            "result": list(json.dumps(value).encode()), "logs": [], "block_height": self.height, "block_hash": "h"}}


def views(mandate):
    return ni.chain_evidence.NearViews(transport=RecordingTransport(GuardViews(mandate)), urls=["https://rpc.invalid"])


def chain_mandate(**changes):
    c = nm.compile_mandate(str(MANDATE), tokens(), now=NOW)
    return {"owner": c["owner"], "agent": c["agent"], "terms": c["terms"], "version": 3, "revoked": False,
            "created_at_ms": NOW_MS, "spends": [], **changes}


def agent_quote(tmp_path, *, live, mandate_path=str(MANDATE), guard_contract=None, near_views=None, transport=None,
                intent=str(AGENT_INTENT)):
    return ni.quote(intent, client=ni.OneClickClient(jwt="k", transport=transport or api()), live=live,
                    nonce_store=ni.NonceStore(str(tmp_path / "n.json")), receipt_path=str(tmp_path / "r.json"),
                    rpc=near_rpc()[0], now=NOW, mandate_path=mandate_path, guard_contract=guard_contract,
                    near_views=near_views)


def test_a_live_agent_quote_checks_the_mandate_on_chain_first(tmp_path):
    result = agent_quote(tmp_path, live=True, guard_contract="x3-guard.near", near_views=views(chain_mandate()))
    names = [g["guard"] for g in result["guards"]]
    assert names[:10] == [
        "mandate_active", "mandate_asset", "mandate_destination_asset", "mandate_recipient", "mandate_refund_to",
        "mandate_per_payment", "mandate_daily", "mandate_slippage", "mandate_fees", "mandate_deadline"]
    assert "quote_signature" in names
    receipt = json.loads((tmp_path / "r.json").read_text())
    assert receipt["mandate"]["source"]["kind"] == "chain" and receipt["mandate"]["version"] == 3
    assert receipt["mandate"]["approvals_needed"] == 0


@pytest.mark.parametrize("changes,failed", [
    ({"revoked": True}, "mandate_active"),
    ({"spends": [{"at_ms": NOW_MS - 1_000, "amount": "190000000", "nonce": "x"}]}, "mandate_daily"),
])
def test_a_mandate_the_chain_refuses_never_reaches_1click(tmp_path, changes, failed):
    transport = api()
    with pytest.raises(ni.NearIntentsError) as err:
        agent_quote(tmp_path, live=True, guard_contract="x3-guard.near", near_views=views(chain_mandate(**changes)),
                    transport=transport)
    assert err.value.code == "X3_MANDATE_DENIED" and failed in err.value.message
    assert not any(c["url"].endswith("/v0/quote") for c in transport.calls)
    assert not (tmp_path / "n.json").exists() and not (tmp_path / "r.json").exists()


def test_no_mandate_on_chain_is_a_denial(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        agent_quote(tmp_path, live=True, guard_contract="x3-guard.near", near_views=views(None))
    assert err.value.code == "X3_MANDATE_DENIED" and "holds no mandate" in err.value.message


def test_a_live_agent_quote_needs_its_mandate(tmp_path):
    with pytest.raises(ni.NearIntentsError) as err:
        agent_quote(tmp_path, live=True, mandate_path=None)
    assert err.value.code == "X3_MANDATE_REQUIRED"
    # A dry quote may skip it: it authorizes nothing.
    assert agent_quote(tmp_path, live=False, mandate_path=None)["status"] == "quoted"


def test_a_live_agent_quote_requires_on_chain_mandate_state(tmp_path):
    transport = api()
    with pytest.raises(ni.NearIntentsError) as err:
        agent_quote(tmp_path, live=True, guard_contract=None, transport=transport)
    assert err.value.code == "X3_MANDATE_REQUIRED"
    assert "--guard-contract" in err.value.message
    assert not any(c["url"].endswith("/v0/quote") for c in transport.calls)
    assert not (tmp_path / "n.json").exists() and not (tmp_path / "r.json").exists()


def test_a_mandate_for_another_owner_is_refused(tmp_path):
    other = mandate_file(tmp_path, *replaced("owner", "owner someone-else.near"))
    with pytest.raises(ni.NearIntentsError) as err:
        agent_quote(tmp_path, live=False, mandate_path=other)
    assert err.value.code == "X3_MANDATE_OWNER"


# --- the agent's contract calls ----------------------------------------------------------------

def _agent_receipt(tmp_path):
    agent_quote(tmp_path, live=True, guard_contract="x3-guard.near", near_views=views(chain_mandate()))
    ni.track(ni.OneClickClient(jwt="k", transport=api(statuses=[_status("SUCCESS", amountOut="24950000")])),
             "9xQe", receipt_path=str(tmp_path / "r.json"))
    return json.loads((tmp_path / "r.json").read_text())


def test_agent_receipts_use_the_agent_methods_with_the_owner(tmp_path):
    receipt = _agent_receipt(tmp_path)
    for step, method in (("register", "agent_register_intent"), ("quote", "agent_accept_quote"),
                         ("approve", "approve_intent")):
        assert ni.guard_method(receipt, step) == method
        args = ni.guard_args(receipt, step)
        assert args["owner"] == "x3-owner.near" and args["nonce"] == "agent_payment_001"
    assert ni.guard_method(receipt, "outcome") == "agent_record_outcome"
    with pytest.raises(ni.NearIntentsError, match="payout transaction verifier"):
        ni.guard_args(receipt, "outcome")
    assert set(ni.guard_args(receipt, "approve")) == {"owner", "nonce"}
    plain = {**receipt, "leg": {k: v for k, v in receipt["leg"].items() if k != "mandate_owner"}}
    assert ni.guard_method(plain, "register") == "register_intent" and "owner" not in ni.guard_args(plain, "register")
    with pytest.raises(ni.NearIntentsError, match="for an intent under a mandate"):
        ni.guard_args(plain, "approve")


def test_cli_mandate_check_exits_6_on_a_denial(tmp_path, capsys):
    args = ["mandate", "check", str(MANDATE), agent_intent(tmp_path, receiver="attacker.near"), "--tokens", str(TOKENS)]
    assert ni.main(args, client=ni.OneClickClient(jwt="", transport=api())) == ni.EXIT_MANDATE_DENIED
    out = json.loads(capsys.readouterr().out)
    assert out["status"] == "denied"
    assert [c["guard"] for c in out["mandate"]["checks"] if not c["ok"]] == ["mandate_recipient"]
    ok_args = ["mandate", "check", str(MANDATE), str(AGENT_INTENT), "--tokens", str(TOKENS)]
    assert ni.main(ok_args, client=ni.OneClickClient(jwt="", transport=api())) == 0
    assert json.loads(capsys.readouterr().out)["status"] == "allowed"


def test_cli_mandate_args_and_revoke_print_the_owners_calls(tmp_path, capsys):
    common = ["--tokens", str(TOKENS), "--contract", "x3-guard.near", "--key-file", "owner.json"]
    assert ni.main(["mandate", "args", str(MANDATE), *common], client=ni.OneClickClient(jwt="", transport=api())) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["method"] == "create_mandate" and out["signer"] == "x3-owner.near"
    assert set(out["args"]) == {"agent", "terms"} and "create_mandate" in out["command"]
    assert "sign-as x3-owner.near" in out["command"]
    assert ni.main(["mandate", "revoke", str(MANDATE), *common], client=ni.OneClickClient(jwt="", transport=api())) == 0
    out = json.loads(capsys.readouterr().out)
    assert out["method"] == "revoke_mandate" and out["args"] == {"agent": "x3-agent.near"}


# --- the fixture the contract tests read -------------------------------------------------------

def _fixture(tmp_path):
    receipt = _agent_receipt(tmp_path)
    compiled = nm.compile_mandate(str(MANDATE), tokens(), now=NOW)
    cases = []
    for name, kwargs, expected in POLICY_CASES:
        sub = tmp_path / name
        sub.mkdir()
        policy = _policy(sub, **kwargs)
        cases.append({"name": name, "policy": policy,
                      "failed": _failed(nm.check_mandate(_compiled(), policy, NOW_MS))})
        assert cases[-1]["failed"] == expected
    return {
        "now_ms": NOW_MS,
        "owner": compiled["owner"],
        "agent": compiled["agent"],
        "create_mandate": {"agent": compiled["agent"], "terms": compiled["terms"]},
        "register": ni.guard_args(receipt, "register"),
        "refused_register": na.register_args(
            str(REFUSED_INTENT),
            str(MANDATE),
            client=ni.OneClickClient(jwt="", transport=api()),
            tokens_path=str(TOKENS),
            now=NOW,
        )["args"],
        "quote": ni.guard_args(receipt, "quote"),
        # Explicit historical contract-unit-test input, never generated as
        # settlement proof by the adapter.
        "outcome": {"nonce": "agent_payment_001", "owner": "x3-owner.near",
                    "status": "Settled", "amount_out": "24950000",
                    "refunded_amount": None, "tx_hashes": ["o1", "n1"]},
        "policy_cases": cases,
    }


def test_mandate_args_match_the_fixture_the_contract_tests_read(tmp_path):
    """Regenerate with `X3_REGEN_GUARD_FIXTURE=1` when the output changes on purpose, then run
    the contract tests (`cargo test` in near/x3-guard)."""
    fixture = _fixture(tmp_path)
    if os.environ.get("X3_REGEN_GUARD_FIXTURE") == "1":
        MANDATE_FIXTURE.write_text(json.dumps(fixture, indent=2, sort_keys=True) + "\n")
    assert json.loads(MANDATE_FIXTURE.read_text()) == fixture
