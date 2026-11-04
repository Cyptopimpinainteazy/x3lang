"""Three track stories must share the same existing X3 mandate semantics."""
import json
from datetime import datetime, timezone
from pathlib import Path

import track_control as tc

ROOT = Path(__file__).resolve().parents[1]
NOW = datetime(2026, 11, 3, 3, 0, 0, tzinfo=timezone.utc)
MANDATE = ROOT / "examples" / "near_mandate_agent.x3"
ALLOWED = ROOT / "examples" / "near_intents_agent_payment.x3"
DENIED = ROOT / "examples" / "near_intents_agent_denied.x3"
TOKENS = ROOT / "tests" / "fixtures" / "near_intents_tokens.json"


def evaluate(path):
    return tc.evaluate(str(MANDATE), str(path), json.loads(TOKENS.read_text()), now=NOW)


def test_one_x3_policy_passes_three_track_preflight_without_authorization():
    result = evaluate(ALLOWED)
    assert result["decision"] == "preflight_passed"
    assert result["denied_guards"] == []
    assert result["approvals_needed"] == 0
    assert result["tracks"] == list(tc.TRACKS)
    assert result["authorization"] == "not_verified"
    assert result["wallet_enforced"] is False
    assert result["quote_verified"] is False
    assert result["settlement_proven"] is False
    assert len(result["checks"]) == 10


def test_forbidden_recipient_is_denied_in_every_track():
    result = evaluate(DENIED)
    assert result["decision"] == "denied_by_policy"
    assert result["denied_guards"] == ["mandate_recipient"]
    assert result["authorization"] == "not_verified"


def test_owner_mismatch_fails_closed(tmp_path):
    changed = tmp_path / "wrong_owner.x3"
    changed.write_text(ALLOWED.read_text().replace("under mandate x3-owner.near", "under mandate other.near"))
    assert evaluate(changed)["denied_guards"] == ["mandate_owner"]


def test_over_threshold_requires_separate_approval(tmp_path):
    changed = tmp_path / "approval.x3"
    changed.write_text(ALLOWED.read_text().replace("amount 25000000", "amount 35000000")
                       .replace("min_receive 24800000", "min_receive 34800000"))
    result = evaluate(changed)
    assert result["decision"] == "approval_required"
    assert result["denied_guards"] == []
    assert result["approvals_needed"] == 1
    assert result["wallet_enforced"] is False


def test_cli_never_promotes_local_preview_to_live_proof(capsys):
    args = ["--mandate", str(MANDATE), "--intent", str(ALLOWED), "--tokens", str(TOKENS),
            "--at", "2026-11-03T03:00:00Z"]
    assert tc.main(args) == 0
    assert json.loads(capsys.readouterr().out)["evidence_level"] == "local_policy_preview_only"
    args[3] = str(DENIED)
    assert tc.main(args) == 6
    assert json.loads(capsys.readouterr().out)["decision"] == "denied_by_policy"


def test_naive_time_is_refused():
    import pytest
    with pytest.raises(ValueError, match="timezone-aware"):
        tc.evaluate(str(MANDATE), str(ALLOWED), [], now=datetime(2026, 11, 3))


def test_assumed_revocation_and_spends_reach_the_same_mandate_checks():
    tokens = json.loads(TOKENS.read_text())
    revoked = tc.evaluate(str(MANDATE), str(ALLOWED), tokens, now=NOW, assumed_state={"revoked": True})
    assert revoked["denied_guards"] == ["mandate_active"]
    assert revoked["assumed_guard_state"] == {"revoked": True, "spends": []}
    assert revoked["authorization"] == "not_verified"
    hour_ago = int(NOW.timestamp() * 1000) - 3_600_000
    spent = tc.evaluate(str(MANDATE), str(ALLOWED), tokens, now=NOW,
                        assumed_state={"spends": [{"amount": "180000000", "at_ms": hour_ago}]})
    assert spent["denied_guards"] == ["mandate_daily"]


def test_no_assumed_state_keeps_the_original_report():
    assert "assumed_guard_state" not in evaluate(ALLOWED)


def test_assumed_state_rejects_anything_but_revoked_and_spends():
    import pytest
    tokens = json.loads(TOKENS.read_text())
    for bad in ({"version": 3}, {"revoked": "yes"}, {"spends": {}}):
        with pytest.raises(ValueError, match="assumed_state"):
            tc.evaluate(str(MANDATE), str(ALLOWED), tokens, now=NOW, assumed_state=bad)


def test_judge_console_scenarios_are_current():
    import subprocess
    import sys
    script = ROOT / "near" / "demo" / "judge" / "build_scenarios.py"
    proc = subprocess.run([sys.executable, str(script), "--check"], capture_output=True, text=True)
    assert proc.returncode == 0, proc.stderr
