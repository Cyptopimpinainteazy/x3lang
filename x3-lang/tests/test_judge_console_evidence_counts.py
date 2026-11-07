"""Judge evidence collection must report real Node test counts, not invented zeroes."""
from importlib.util import module_from_spec, spec_from_file_location
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "near" / "demo" / "judge" / "collect_evidence.py"
spec = spec_from_file_location("x3_judge_console_collect_evidence", SCRIPT)
collector = module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(collector)


def test_node_hash_prefixed_tap_summary_reports_actual_pass_count():
    output = "# tests 42\n# suites 0\n# pass 42\n# fail 0\n# cancelled 0\n# skipped 1\n# todo 0\n"
    assert collector.counts_node(output) == {"passed": 42, "failed": 0, "skipped": 1}


def test_node_legacy_info_symbol_summary_remains_supported():
    output = "ℹ tests 41\nℹ pass 40\nℹ fail 1\nℹ cancelled 0\nℹ skipped 0\nℹ todo 0\n"
    assert collector.counts_node(output) == {"passed": 40, "failed": 1, "skipped": 0}


def test_node_cancelled_or_todo_is_not_counted_as_success():
    output = "# tests 42\n# pass 40\n# fail 0\n# cancelled 1\n# skipped 0\n# todo 1\n"
    assert collector.counts_node(output) == {"passed": 40, "failed": 1, "skipped": 1}
