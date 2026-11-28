"""Tests for the draft sequential / inventory / economic-predicate mandate extensions.

These clauses are accepted by the compiler today so a mandate source hash can
bind them. On-chain Guard enforcement of the new fields is follow-on work.
"""
import pytest

from test_near_mandate import BASE, mandate_file, nm, tokens, NOW


def test_sequential_inventory_and_economic_predicates_parse(tmp_path):
    lines = BASE + [
        "max_incomplete_cycle_notional 150000000",
        "require min_net_bps >= 15",
        "require min_profit_bps >= 10",
        "halt_on_partial_or_failed_leg",
        "require_re_quote_after_settlement",
    ]
    parsed = nm.parse_mandate(mandate_file(tmp_path, *lines))
    assert parsed["max_incomplete_cycle_notional"] == "150000000"
    kinds = {r["kind"] for r in parsed["requires"]}
    assert "min_net_bps" in kinds
    assert "min_profit_bps" in kinds
    assert parsed["halt_on_partial_or_failed_leg"] is True
    assert parsed["require_re_quote_after_settlement"] is True


def test_unknown_economic_predicate_is_still_refused(tmp_path):
    lines = BASE + ["require totally_made_up >= 1"]
    with pytest.raises(Exception) as err:
        nm.parse_mandate(mandate_file(tmp_path, *lines))
    assert "malformed require" in str(err.value) or "not yet supported" in str(err.value)


def test_duplicate_inventory_cap_is_refused(tmp_path):
    lines = BASE + [
        "max_incomplete_cycle_notional 100000000",
        "max_incomplete_cycle_notional 200000000",
    ]
    with pytest.raises(Exception) as err:
        nm.parse_mandate(mandate_file(tmp_path, *lines))
    assert "appears twice" in str(err.value)
