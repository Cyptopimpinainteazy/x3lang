"""One complete agent-law path: permit, block, revoke, and preserve evidence."""

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))

import agent_law  # noqa: E402


def test_the_invoice_path_permits_blocks_revokes_and_preserves():
    report = agent_law.demonstrate()
    assert report["ok"] is True
    assert report["permitted"] is True
    assert report["forbidden_reason"] == "mandate_recipient"
    assert report["outside_reason"] == "route"
    assert report["revoked_reason"] == "revoked"
    assert report["court_receipts"] == 1
    assert report["self_judgment"] == "agent_cannot_judge_itself"
    assert report["jury"] == "irreversible_payment"
    assert report["commandments_sha256"] == hashlib.sha256(agent_law.MANDATE.read_bytes()).hexdigest()


def test_birth_rejects_the_agent_and_a_solana_capability():
    try:
        agent_law.birth(
            role="agent",
            agent_id="x3-agent.x3lang.near",
            mandate_source="mandate {}",
            capabilities=["x3.quote.accept"],
            now_ms=0,
            compiled_at=agent_law.datetime(2026, 11, 12, tzinfo=agent_law.timezone.utc),
            tokens=[],
        )
    except agent_law.LawError as exc:
        assert exc.code == "X3_LAW_ROLE"
    else:
        raise AssertionError("the agent birthed itself")
    try:
        agent_law.birth(
            role="operator",
            agent_id="x3-agent.x3lang.near",
            mandate_source="mandate {}",
            capabilities=["solana.transfer"],
            now_ms=0,
            compiled_at=agent_law.datetime(2026, 11, 12, tzinfo=agent_law.timezone.utc),
            tokens=[],
        )
    except agent_law.LawError as exc:
        assert exc.code == "X3_LAW_CAPABILITY"
    else:
        raise AssertionError("birth granted a Solana signer")


def test_authorize_is_the_only_receipt_door():
    source = Path(agent_law.__file__).read_text(encoding="utf-8")
    assert source.count("\ndef authorize(") == 1
    assert "def execute_unrestricted" not in source
    assert "def owner_execute" not in source


def test_the_editor_source_passes_the_same_door_and_a_stuffed_privilege_does_not():
    mandate = (ROOT / "x3-lang/examples/near_mandate_agent.x3").read_text(encoding="utf-8")
    program = (ROOT / "x3-lang/examples/near_intents_agent_payment.x3").read_text(encoding="utf-8")
    door = agent_law.Door()
    born = door.act({"act": "birth", "mandate": mandate, "program": program, "privileged": ["ab"]})
    assert born["ok"] is True
    assert born["reason"] == "born"
    assert born["agent_id"] == "x3-agent.near"
    assert born["wallet_authorized"] is False
    tested = door.act({"act": "test", "mandate": mandate, "program": program})
    assert tested["ok"] is True
    assert tested["reason"] == "tested"
    assert tested["court_receipts"] == 0
    permitted = door.act({"act": "authorize", "mandate": mandate, "program": program})
    assert permitted["ok"] is True
    assert permitted["reason"] == "executed"
    assert permitted["broadcast"] is False
    assert permitted["settlement"] is False
    assert permitted["receipt"]["broadcast"] is False
    assert permitted["receipt"]["recipient"] == "x3-demo.near"
    assert permitted["receipt"]["amount"] == 25_000_000
    assert permitted["spent"] == 25_000_000
    attacker = program.replace(
        "to Near.USDC receiver x3-demo.near",
        "to Near.USDC receiver attacker.near",
        1,
    )
    stuffed = hashlib.sha256(attacker.encode("utf-8")).hexdigest()
    blocked = door.act({
        "act": "authorize",
        "mandate": mandate,
        "program": attacker,
        "privileged": [stuffed],
        "role": "operator",
    })
    assert blocked["ok"] is False
    assert blocked["reason"] == "not_tested"
    assert blocked["spent"] == 25_000_000
    assert blocked["court_receipts"] == 1
    refused = door.act({"act": "test", "mandate": mandate, "program": attacker})
    assert refused["ok"] is False
    assert refused["reason"] == "mandate_recipient"
    replay = door.act({"act": "authorize", "mandate": mandate, "program": program})
    assert replay["ok"] is False
    assert replay["reason"] == "nonce"
    assert replay["spent"] == 25_000_000
    revoked = door.act({"act": "revoke", "mandate": mandate, "program": program})
    assert revoked["ok"] is True
    assert revoked["status"] == "revoked"
    later = program.replace("agent_payment_001", "agent_payment_002", 1)
    door.act({"act": "test", "mandate": mandate, "program": later})
    stopped = door.act({"act": "authorize", "mandate": mandate, "program": later})
    assert stopped["ok"] is False
    assert stopped["reason"] == "revoked"
    assert stopped["court_receipts"] == 1
    absent = agent_law.Door().act({"act": "authorize", "mandate": mandate, "program": program})
    assert absent["reason"] == "no_birth"


def test_render_names_the_four_results():
    text = agent_law.render(agent_law.demonstrate())
    assert "permitted X3 call: True" in text
    assert "forbidden bypass: mandate_recipient" in text
    assert "after revocation: revoked" in text
    assert "court receipts preserved: 1" in text


def test_exhibit_page_matches_the_run_and_the_apps_show_it():
    report = agent_law.demonstrate()
    view = agent_law.public_view(report)
    assert view["wallet_authorized"] is False
    assert view["settlement_proven"] is False
    assert view["settlement"] is False
    assert view["broadcast"] is False
    assert set(view["stages_seen"]) == set(agent_law.ARCHITECTURE)
    page = (ROOT / "docs/hackathon/agent-law-proof.html").read_text(encoding="utf-8")
    assert page == agent_law.proof_page(report)
    embedded = page.split('<script type="application/json" id="report">', 1)[1].split("</script>", 1)[0]
    assert json.loads(embedded) == view
    assert "does not settle the payment" in page
    assert "Settled" not in page
    lab = (ROOT / "docs/hackathon/attack-lab.html").read_text(encoding="utf-8")
    for fact in ("mandate_recipient", "revoked", "agent-law-proof.html", "does not settle invoice_06"):
        assert fact in lab
    workbench = (ROOT / "x3-lang/near/dotx3/index.html").read_text(encoding="utf-8")
    assert "PENDING FOR THIS INTENT" in workbench
    assert "This screen does not establish that proof." in workbench
    assert 'id="law-status"' in workbench
    assert "LOCAL PATH" in workbench
    assert "does not settle this intent" in workbench
    assert 'id="door-status"' in workbench
    assert 'id="law-authorize"' in workbench
    assert "does not replace the archived invoice_06 run" in workbench
