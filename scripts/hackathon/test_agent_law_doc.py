"""Keep docs/hackathon/AGENT_LAW.md honest: links resolve, named code exists, evidence matches."""
from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
DOC = ROOT / "docs" / "hackathon" / "AGENT_LAW.md"
GUARD = ROOT / "x3-lang" / "near" / "x3-guard" / "src"
TEXT = DOC.read_text(encoding="utf-8")
GUARD_SOURCE = "\n".join(p.read_text(encoding="utf-8") for p in GUARD.glob("*.rs"))


def test_every_relative_link_resolves():
    links = [m for m in re.findall(r"\]\(([^)#]+)(?:#[^)]*)?\)", TEXT) if not m.startswith("http")]
    assert len(links) > 10
    for link in links:
        assert (DOC.parent / link).resolve().exists(), link


def test_every_guard_function_and_test_named_exists():
    for name in ["create_mandate", "revoke_mandate", "approve_intent", "check_mandate", "verify_signed_quote",
                 "evaluate", "agent_register_intent", "agent_accept_quote", "agent_record_outcome",
                 "revoking_stops_the_agent_at_once_and_replacing_voids_old_intents"]:
        assert f"`{name}`" in TEXT, name
        assert re.search(rf"\bfn {name}\b", GUARD_SOURCE), name
    for guard in ["quote_request_echo", "mandate_recipient", "mandate_refund_to"]:
        assert f'"{guard}"' in GUARD_SOURCE, guard
    assert "ONE_CLICK_MANAGER_PUBLIC_KEY" in GUARD_SOURCE


def test_the_quote_key_has_no_setter():
    deployed = "\n".join((GUARD / name).read_text(encoding="utf-8") for name in ("lib.rs", "mandate.rs"))
    assert not re.search(r"\.manager_public_key\s*=", deployed), "something assigns the manager key"
    values = [v.strip() for v in re.findall(r"manager_public_key\s*:\s*([^,\n;]+)", deployed)
              if not v.strip().startswith(("[", "&["))]
    assert values == ["decode_ed25519::<32>(ONE_CLICK_MANAGER_PUBLIC_KEY)", "current.manager_public_key"], values


def test_agent_methods_take_no_reason_argument():
    for method in ["agent_register_intent", "agent_accept_quote", "agent_record_outcome"]:
        signature = re.search(rf"pub fn {method}\s*\(([^)]*)\)", GUARD_SOURCE, re.S).group(1)
        assert not re.search(r"\b(reason|purpose|decision|memo|note)\w*\s*:", signature), (method, signature)


def test_the_testnet_revocation_is_in_the_committed_evidence():
    data = json.loads((ROOT / "evidence" / "near-guard-testnet" / "transactions.json").read_text(encoding="utf-8"))
    items = data if isinstance(data, list) else next(v for v in data.values() if isinstance(v, list))
    revoke = next(t for t in items if t.get("label") == "revoke_mandate")
    assert revoke["final_execution_status"] == "FINAL" and not revoke["receipt_errors"]
    assert revoke["tx_hash"] in TEXT and str(revoke["block_height"]) in TEXT


def test_every_lifecycle_row_has_a_status_label():
    table = TEXT.split("## The lifecycle", 1)[1].split("## Properties", 1)[0]
    rows = [r for r in table.splitlines() if r.startswith("| ") and not r.startswith("| Stage") and not r.startswith("|---")]
    assert len(rows) == 10
    for row in rows:
        assert re.search(r"\*\*(MAINNET|OFFLINE|CHECK ONLY|DESIGN|Gap)\*\*", row), row
