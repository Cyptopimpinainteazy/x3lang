"""Judge-facing support: offline demo, missing adversarial cases and readiness truthfulness."""
from __future__ import annotations

import importlib.util
import json
from datetime import datetime, timezone
from pathlib import Path

import pytest

import track_control as tc

ROOT = Path(__file__).resolve().parents[2]
X3 = ROOT / "x3-lang"
MANDATE = X3 / "examples" / "near_mandate_agent.x3"
PAYMENT = X3 / "examples" / "near_intents_agent_payment.x3"
TOKENS = X3 / "tests" / "fixtures" / "near_intents_tokens.json"
NOW = datetime(2026, 11, 3, 3, 0, 0, tzinfo=timezone.utc)


def load_script(relative: str):
    path = ROOT / relative
    spec = importlib.util.spec_from_file_location(path.stem, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


def evaluate_edit(tmp_path: Path, old: str, new: str, *, now: datetime = NOW, assumed=None):
    source = PAYMENT.read_text()
    assert source.count(old) == 1
    path = tmp_path / "edited.x3"
    path.write_text(source.replace(old, new))
    return tc.evaluate(
        str(MANDATE),
        str(path),
        json.loads(TOKENS.read_text()),
        now=now,
        assumed_state=assumed,
    )


def test_offline_demo_has_five_panels_and_three_expected_decisions(capsys):
    demo = load_script("scripts/hackathon/demo.py")
    assert demo.main([]) == 0
    text = capsys.readouterr().out
    for panel in range(1, 6):
        assert f"PANEL {panel}" in text
    assert "Passes the local policy check" in text
    assert "Needs approval" in text
    assert "Refused" in text
    assert "No wallet signature, on-chain authorization, signed quote or settlement is claimed" in text


def test_offline_demo_json_is_deterministic_and_embeds_the_actual_legs():
    demo = load_script("scripts/hackathon/demo.py")
    first = demo.build_report()
    second = demo.build_report()
    assert first == second
    assert first["schema"] == "x3.hackathon.demo.v1"
    assert first["evidence_level"] == "local_policy_preview_only"
    by_id = {scenario["id"]: scenario for scenario in first["scenarios"]}
    assert by_id["authorized"]["decision"] == "preflight_passed"
    assert by_id["needs-approval"]["decision"] == "approval_required"
    assert by_id["wrong-recipient"]["denied_guards"] == ["mandate_recipient"]
    assert by_id["authorized"]["cross_chain"]["origin_asset"] == "Solana.USDC"
    assert by_id["authorized"]["cross_chain"]["destination_asset"] == "Near.USDC"
    assert by_id["authorized"]["cross_chain"]["amount"] == 25_000_000


def test_oversized_payment_is_denied_by_the_shared_policy(tmp_path):
    result = evaluate_edit(
        tmp_path,
        "amount 25000000",
        "amount 60000000",
    )
    assert result["decision"] == "denied_by_policy"
    assert "mandate_per_payment" in result["denied_guards"]


def test_origin_asset_outside_the_mandate_is_denied(tmp_path):
    source = PAYMENT.read_text()
    source = source.replace("from Solana.USDC", "from Solana.SOL")
    source = source.replace("bridge NearIntents Solana.USDC ->", "bridge NearIntents Solana.SOL ->")
    source = source.replace("timeout 10m refund Solana.USDC", "timeout 10m refund Solana.SOL")
    path = tmp_path / "wrong_asset.x3"
    path.write_text(source)
    result = tc.evaluate(str(MANDATE), str(path), json.loads(TOKENS.read_text()), now=NOW)
    assert result["decision"] == "denied_by_policy"
    assert result["denied_guards"] == ["mandate_asset"]


@pytest.mark.parametrize(
    "old,new,guard",
    [
        ("require slippage <= 0.5%", "require slippage <= 1.5%", "mandate_slippage"),
        ("require fees <= 100 bps", "require fees <= 150 bps", "mandate_fees"),
    ],
)
def test_fee_and_slippage_violations_are_denied(tmp_path, old, new, guard):
    result = evaluate_edit(tmp_path, old, new)
    assert result["decision"] == "denied_by_policy"
    assert result["denied_guards"] == [guard]


def test_malformed_program_fails_closed(tmp_path):
    malformed = tmp_path / "malformed.x3"
    malformed.write_text("intent broken {\n")
    with pytest.raises(tc.ni.NearIntentsError):
        tc.evaluate(str(MANDATE), str(malformed), json.loads(TOKENS.read_text()), now=NOW)


def _evidence(commit: str, *, clean: bool = True):
    return {
        "commit": commit,
        "worktree_clean": clean,
        "runs": [
            {"name": "Policy checker, compiler and adapters (Python)", "exit_code": 0,
             "passed": 10, "failed": 0, "skipped": 1},
            {"name": "Browser checker and console (Node)", "exit_code": 0,
             "passed": 11, "failed": 0, "skipped": 0},
            {"name": "Console scenarios match the examples", "exit_code": 0,
             "passed": None, "failed": None, "skipped": None},
            {"name": "Offline five-panel judge demo", "exit_code": 0,
             "passed": None, "failed": None, "skipped": None},
            {"name": "x3-guard contract (Rust)", "exit_code": 0,
             "passed": 12, "failed": 0, "skipped": 0},
        ],
    }


def test_readiness_report_requires_current_clean_evidence():
    readiness = load_script("scripts/hackathon/readiness.py")
    commit = "a" * 40
    current = readiness.build_report(_evidence(commit), head=commit)
    assert current["local_verification_ready"] is True
    assert current["submission_ready"] is False
    assert current["checks"]["python"]["state"] == "passed"
    assert "onchain_authorization" in [item["id"] for item in current["unproven"]]
    assert current["existing_verified_near_integration"]["status"] == "contract_tests_passed"

    stale = readiness.build_report(_evidence("b" * 40), head=commit)
    assert stale["local_verification_ready"] is False
    assert "evidence_commit_does_not_match_head" in stale["blockers"]

    dirty = readiness.build_report(_evidence(commit, clean=False), head=commit)
    assert dirty["local_verification_ready"] is False
    assert "evidence_worktree_was_dirty" in dirty["blockers"]


def test_readiness_does_not_turn_a_skipped_contract_run_into_a_pass():
    readiness = load_script("scripts/hackathon/readiness.py")
    commit = "c" * 40
    evidence = _evidence(commit)
    evidence["runs"][-1].update(exit_code=None, passed=None, failed=None, skipped=None)
    report = readiness.build_report(evidence, head=commit)
    assert report["checks"]["contract"]["state"] == "not_run"
    assert report["local_verification_ready"] is False
    assert "contract_evidence_not_run" in report["blockers"]


def test_readiness_json_contains_no_completion_percentage():
    readiness = load_script("scripts/hackathon/readiness.py")
    report = readiness.build_report(_evidence("d" * 40), head="d" * 40)
    assert "completion" not in json.dumps(report).lower()
    assert "percent" not in json.dumps(report).lower()


def test_prompt_injection_is_refused_by_the_recipient_guard():
    demo = load_script("scripts/hackathon/demo.py")
    by_id = {scenario["id"]: scenario for scenario in demo.build_report()["scenarios"]}
    scenario = by_id["prompt-injection-recipient"]
    assert scenario["decision"] == "denied_by_policy"
    assert scenario["denied_guards"] == ["mandate_recipient"]
    assert "prompt" in scenario["story"].lower()


def test_one_command_runs_prompt_injection_and_parity():
    source = (ROOT / "scripts/hackathon/judge_one_command.sh").read_text()
    assert "demo.py" in source
    assert "build_scenarios.py --check" in source
    assert "node --test x3-lang/near/demo/test/*.test.mjs" in source
