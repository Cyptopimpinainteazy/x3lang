"""Keep the README's compromised-agent table tied to the source and evidence it cites.

The table under "Trust boundaries" names Guard checks, Guard tests, agent methods, the
mainnet mandate's approval rule, transactions and links into the README and the proof card.
Each test below fails when one of those stops existing, so the table cannot drift.
"""

from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
README = ROOT / "README.md"
PROOF_CARD = ROOT / "PROOF_CARD.md"
GUARD_SRC = ROOT / "x3-lang" / "near" / "x3-guard" / "src"
MAINNET_MANDATE = (
    ROOT / "evidence" / "near-guard-mainnet-invoice06-2026-11-12" / "source-recovery" / "mandate-proposed.x3"
)
HEADING = "### If the agent is compromised"


def _table_section() -> str:
    text = README.read_text(encoding="utf-8")
    start = text.index(HEADING)
    body = text[start + len(HEADING):]
    end = re.search(r"\n#{2,3} ", body)
    assert end, "the table section must end at the next heading"
    return HEADING + body[: end.start()]


def _anchors(markdown: str) -> set[str]:
    """GitHub heading anchors plus explicit <a id> anchors."""
    anchors = set(re.findall(r'<a id="([^"]+)"></a>', markdown))
    for heading in re.findall(r"^#{1,6} (.+)$", markdown, flags=re.M):
        anchors.add(re.sub(r"[^\w\- ]", "", heading.strip().lower()).replace(" ", "-"))
    return anchors


def _guard_source() -> str:
    return "\n".join(path.read_text(encoding="utf-8") for path in sorted(GUARD_SRC.glob("*.rs")))


def test_every_link_in_the_table_resolves():
    section = _table_section()
    readme_anchors = _anchors(README.read_text(encoding="utf-8"))
    card_anchors = _anchors(PROOF_CARD.read_text(encoding="utf-8"))
    links = re.findall(r"\]\(([^)\s]+)\)", section)
    assert len(links) >= 10
    for link in links:
        if link.startswith("https://"):
            continue
        path, _, anchor = link.partition("#")
        if not path:
            assert anchor in readme_anchors, link
        elif path == "PROOF_CARD.md":
            assert anchor in card_anchors, link
        else:
            assert (ROOT / path).is_file(), link


def test_the_proof_card_links_to_the_table():
    assert "README.md#if-the-agent-is-compromised" in PROOF_CARD.read_text(encoding="utf-8")
    assert "if-the-agent-is-compromised" in _anchors(README.read_text(encoding="utf-8"))


def test_transactions_in_the_table_are_the_recorded_ones():
    section = _table_section()
    elsewhere = README.read_text(encoding="utf-8").replace(section, "") + PROOF_CARD.read_text(encoding="utf-8")
    transactions = re.findall(r"https://(?:testnet\.)?nearblocks\.io/txns/([1-9A-HJ-NP-Za-km-z]{43,44})", section)
    assert len(transactions) == 4
    for tx in transactions:
        assert tx in elsewhere, f"{tx} is not in the README's testnet table or the proof card"


def test_named_guard_checks_tests_and_agent_methods_exist():
    section = _table_section()
    source = _guard_source()
    checks = set(re.findall(r"`(mandate_[a-z0-9_]+)`", section))
    assert {"mandate_recipient", "mandate_per_payment", "mandate_daily", "mandate_approvals"} <= checks
    for check in checks:
        assert f'"{check}"' in source, check
    tests = set(re.findall(r"`([a-z][a-z0-9_]{30,})`", section))
    assert len(tests) == 2
    for test in tests:
        assert re.search(rf"\bfn {test}\(", source), test
    for method in ("agent_register_intent", "agent_accept_quote", "agent_record_outcome",
                   "create_mandate", "revoke_mandate"):
        assert f"`{method}`" in section, method
        assert re.search(rf"\bpub fn {method}\(", source), method


def test_the_mainnet_approval_rule_is_the_committed_mandate():
    rule = re.search(r"`(approve above \d+ by \d+ of [a-z0-9.\-]+)`", _table_section())
    assert rule, "the approval row must quote the mandate's approval rule"
    assert rule.group(1) in MAINNET_MANDATE.read_text(encoding="utf-8")
