"""Keep the vault's complete-path evidence honest and tied to the contract source.

evidence/agent-vault-local/complete-path-sandbox-20261113.json records one run of
scripts/hackathon/vault_sandbox_smoke.sh against an ephemeral local NEAR Sandbox. These checks
fail if the committed harness log no longer matches its hash, if a condition or refusal recorded
in the JSON is missing from that log, if a vault refusal reason no longer exists in the contract,
or if the evidence stops stating its limits.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = ROOT / "evidence" / "agent-vault-local" / "complete-path-sandbox-20261113.json"
CONTRACT = ROOT / "x3-lang" / "near" / "x3-agent-vault" / "src" / "lib.rs"
DOC = ROOT / "docs" / "security" / "X3_AGENT_CONSTITUTION_V1.md"
CONDITIONS = (
    "permitted_action_executes",
    "forbidden_action_cannot_bypass",
    "revocation_stops_later_actions",
    "evidence_preserved",
)


def _evidence() -> dict:
    return json.loads(EVIDENCE.read_text(encoding="utf-8"))


def _log_bytes(evidence: dict) -> bytes:
    return (ROOT / evidence["sandbox_log"]).read_bytes()


def test_the_log_matches_its_hash_and_ends_with_the_completion_marker():
    evidence = _evidence()
    raw = _log_bytes(evidence)
    assert hashlib.sha256(raw).hexdigest() == evidence["sandbox_log_sha256"]
    lines = raw.decode("utf-8").splitlines()
    assert lines, "empty harness log"
    assert all(line.startswith("X3_VAULT_SANDBOX_") for line in lines), "only harness markers belong in the log"
    assert lines[-1] == "X3_VAULT_SANDBOX_COMPLETE agent_key_restricted=true funds_vault_custodied=true public_chain=false"
    assert "/home/" not in raw.decode("utf-8")
    assert evidence["execution"]["sandbox_exit_code"] == 0


def test_each_condition_points_at_markers_that_are_in_the_log():
    evidence = _evidence()
    log = _log_bytes(evidence).decode("utf-8")
    path = evidence["complete_path"]
    assert set(path) == set(CONDITIONS)
    for condition in CONDITIONS[:3]:
        markers = path[condition]
        assert markers, condition
        for marker in markers:
            assert marker in log, f"{condition}: {marker}"
    assert path["evidence_preserved"] == evidence["sandbox_log_sha256"]


def test_recorded_refusals_are_in_the_log_and_vault_reasons_are_in_the_contract():
    evidence = _evidence()
    log = _log_bytes(evidence).decode("utf-8")
    source = CONTRACT.read_text(encoding="utf-8")
    refusals = evidence["verified_refusals"]
    assert len(refusals) >= 9
    for refusal in refusals:
        reason = refusal["typed_rejection"]
        assert reason in log, reason
        if reason.startswith("X3_VAULT_DENIED: "):
            assert reason in source, f"{reason} is not a refusal reason in the vault contract"


def test_the_evidence_states_its_limits():
    evidence = _evidence()
    assert evidence["classification"].endswith("NOT_PUBLIC_CHAIN")
    limits = " ".join(evidence["limitations"])
    for phrase in ("not public testnet or mainnet", "not independently authenticated consensus proof", "FullAccess", "invoice_06"):
        assert phrase in limits, phrase


def test_the_constitution_doc_cites_this_run():
    doc = DOC.read_text(encoding="utf-8")
    assert "evidence/agent-vault-local/complete-path-sandbox-20261113.json" in doc
    assert f"commit `{_evidence()['source_base_commit'][:7]}`" in doc
