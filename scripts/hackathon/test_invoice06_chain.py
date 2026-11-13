"""Offline tests for scripts/hackathon/invoice06_chain.py (committed evidence only)."""
from __future__ import annotations

import importlib.util
import json
import sys
from pathlib import Path

import pytest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("invoice06_chain", HERE / "invoice06_chain.py")
chain = importlib.util.module_from_spec(spec)
sys.modules["invoice06_chain"] = chain
spec.loader.exec_module(chain)


@pytest.fixture(scope="module")
def report():
    return chain.evaluate()


def test_all_five_links_hold_on_the_committed_evidence(report):
    assert report["links"] == {"written": True, "compiler": True, "guard": True, "payment": True, "outcome": True}
    assert report["ok"] is True


def test_compiler_output_equals_what_guard_stores_on_mainnet(report):
    c = report["compiler"]
    assert (c["policy_fields_equal"], c["policy_fields"]) == (10, 10) and c["policy_diff"] == {}
    assert (c["mandate_terms_equal"], c["mandate_terms"]) == (13, 13) and c["mandate_terms_diff"] == {}
    assert c["nonce_equal"] and c["owner_agent_equal"] and c["file_mandate_checks_ok"]
    assert c["compiled_at_utc"] == "2026-11-12T01:49:21.152000Z"


def test_every_edit_of_the_mainnet_payment_is_refused_where_expected(report):
    rows = {e["case"]: e for e in report["compiler"]["edits"]}
    assert rows["recipient_attacker"]["failed"] == ["mandate_recipient"]
    assert rows["ten_times_amount"]["failed"] == ["mandate_daily", "mandate_per_payment"]
    assert rows["no_slippage_bound"]["code"] == "X3_NEAR_SLIPPAGE_REQUIRED"
    assert rows["second_leg"]["code"] == "X3_NEAR_ROUTE_SHAPE"
    assert all(e["pass"] for e in rows.values()) and len(rows) == len(chain.EDITS) == 8


def test_guard_payment_and_outcome_facts(report):
    g, p, o = report["guard"], report["payment"], report["outcome"]
    assert (g["checks"], g["ok"], g["failed"]) == (21, 21, []) and g["signed_by_function_call_key"]
    assert g["public_key"] == "ed25519:J3NiU8J63HAfz2MTDu1zQVhFRYS5Pio21h1kNtJdP11x"
    assert (p["payer_before"], p["payer_after"], p["deposit_address_received"]) == (31074779, 6074779, 25000000)
    assert p["payout_withdrawals"][0]["amount"] == "24995200" and p["payout_withdrawals"][0]["receiver_id"] == "x3lang.near"
    assert p["seconds_deposit_to_payout"] == 84
    assert (o["status"], o["amount_out"], o["floor_met"], o["cites_payout"]) == ("Settled", "24995200", True, True)
    assert o["cross_chain_causality_proven"] is False


def test_the_limits_are_part_of_the_report(report):
    text = " ".join(report["not_cryptographic"])
    assert "signed the Solana deposit by hand" in text
    assert "does not authenticate the hashes" in text
    assert "cross_chain_causality_proven: false" in text


def test_a_changed_payment_file_breaks_the_written_and_compiler_links(tmp_path, monkeypatch):
    changed = tmp_path / "payment-proposed.x3"
    changed.write_text(chain.PAYMENT.read_text(encoding="utf-8").replace("require fees <= 100 bps", "require fees <= 90 bps"))
    monkeypatch.setattr(chain, "PAYMENT", changed)
    r = chain.evaluate()
    assert r["links"]["written"] is False and r["links"]["compiler"] is False and r["ok"] is False
    assert set(r["compiler"]["policy_diff"]) == {"fee_cap_bps", "program_sha256"}


def test_an_edit_that_is_not_refused_fails_the_report(monkeypatch):
    harmless = {"case": "harmless", "tried": "rename the intent", "old": "intent pay_x3_demo_near",
                "new": "intent pay_x3_renamed", "stop": "compile", "expect": "X3_NEAR_ANYTHING"}
    monkeypatch.setattr(chain, "EDITS", chain.EDITS + [harmless])
    r = chain.evaluate()
    assert r["compiler"]["edits"][-1]["stopped_at"] is None and r["compiler"]["edits"][-1]["pass"] is False
    assert r["links"]["compiler"] is False and r["ok"] is False


def test_an_outcome_below_the_quote_fails_the_outcome_link(tmp_path, monkeypatch):
    data = json.loads(chain.OUTCOME.read_text(encoding="utf-8"))
    data["intent_after"]["value"]["outcome"]["amount_out"] = "24000000"
    bad = tmp_path / "outcome.json"
    bad.write_text(json.dumps(data))
    monkeypatch.setattr(chain, "OUTCOME", bad)
    assert chain.evaluate()["links"]["outcome"] is False


def test_a_drifted_edit_stops_the_run(monkeypatch):
    drifted = dict(chain.EDITS[0], old="receiver nobody.near")
    monkeypatch.setattr(chain, "EDITS", [drifted])
    with pytest.raises(SystemExit, match="recipient_attacker"):
        chain.evaluate()


def test_cli_exit_code_and_summary(capsys):
    assert chain.main([]) == 0
    out = capsys.readouterr().out
    assert "5/5 links hold on the committed evidence." in out and "21 ok, 0 failed" in out
    assert chain.main(["--json"]) == 0
    assert json.loads(capsys.readouterr().out)["schema"] == "x3.invoice06_chain.v1"
