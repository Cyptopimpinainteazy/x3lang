"""Check that a devnet refusal replays the previously finalized Execute instruction."""
import json
import sys
from pathlib import Path


def check_replay(execution, replay, *, revoked):
    decoder = json.JSONDecoder()
    records = []
    while execution.strip():
        execution = execution.lstrip()
        record, end = decoder.raw_decode(execution)
        records.append(record)
        execution = execution[end:]
    original, signed, final = records
    if not (signed["signature"] and signed["signature"] == final["signature"]
            and final["sent"] is True and final["finalized"] is True and final["error"] is None
            and original["instruction"] == "execute" and original["expect_error"] is None
            and original["simulation"]["ok"] is True):
        raise ValueError("the original Execute has no successful finalized receipt")
    for field in ("instruction", "cluster", "genesis_hash", "program", "signer_and_fee_payer",
                  "args", "data_hex", "accounts"):
        if replay[field] != original[field]:
            raise ValueError(f"replay differs from the original Execute in {field}")
    state = replay["checks"]
    if not (replay["cluster"] == "devnet" and replay["sent"] is False
            and replay["expect_error"] == "IncorrectTokenAccount (custom 10)"
            and replay["simulation"]["as_expected"] is True and replay["simulation"]["ok"] is False
            and replay["simulation"]["error"] == replay["expect_error"]
            and state["policy_last_nonce"] == replay["args"]["nonce"]
            and state["policy_pending"] is False and state["policy_revoked"] is revoked
            and state["pending_destination"] == "11111111111111111111111111111111"):
        raise ValueError("replay simulation or cleared policy state differs from expectations")


if __name__ == "__main__":
    original, replay, phase = sys.argv[1:]
    if phase not in ("active", "revoked"):
        raise SystemExit("unknown replay phase")
    check_replay(Path(original).read_text(), json.loads(Path(replay).read_text()), revoked=phase == "revoked")
