"""Offline .x3 payment compilation and mandate preview for the website and loopback workbench."""
from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "x3-lang"))
import near_intents as ni
import track_control

TOKENS = ROOT / "x3-lang/tests/fixtures/near_intents_tokens.json"
MAX_BODY = 96 * 1024
MAX_SOURCE = 32 * 1024


def source_input(payload):
    if not isinstance(payload, dict):
        raise ValueError("Expected a JSON object")
    for field in ("program", "mandate"):
        text = payload.get(field)
        if not isinstance(text, str) or not text.strip() or len(text.encode()) > MAX_SOURCE:
            raise ValueError(f"{field} must contain 1–32768 bytes of .x3 source")
    return payload["program"], payload["mandate"]


def compile_sources(payload, *, now=None):
    """No model, network, wallet or chain state. Execute existing compiler/checker."""
    program, mandate = source_input(payload)
    now = now or datetime.now(timezone.utc)
    result = {
        "schema": "x3.dotx3.workbench.v1", "evaluated_at": now.isoformat(),
        "program_sha256": hashlib.sha256(program.encode()).hexdigest(),
        "mandate_sha256": hashlib.sha256(mandate.encode()).hexdigest(),
        "evidence_level": "local_compiler_and_policy_only",
        "token_registry": "checked_in_fixture_not_live_discovery",
        "model_provenance": "user_supplied_source_not_verified_model_output",
        "wallet_authorized": False, "onchain_authorization_verified": False,
        "settlement_proven": False,
        "assumed_state": {"revoked": False, "spends": []},
    }
    with tempfile.TemporaryDirectory(prefix="x3-workbench-") as folder:
        p, m = Path(folder) / "payment.x3", Path(folder) / "mandate.x3"
        p.write_text(program, encoding="utf-8")
        m.write_text(mandate, encoding="utf-8")
        try:
            _, leg = ni.compile_program(str(p), now=now)
            result.update(compiled=True, stage="mandate", leg=leg)
            tokens = json.loads(TOKENS.read_text())
            policy = track_control.evaluate(str(m), str(p), tokens, now=now)
            result.update(decision=policy["decision"], policy=policy)
        except ni.NearIntentsError as exc:
            result.update(decision="compile_error" if "compiled" not in result else "mandate_error",
                          compiled=result.get("compiled", False),
                          error={"code": exc.code, "message": exc.message, "detail": exc.detail})
    return result
