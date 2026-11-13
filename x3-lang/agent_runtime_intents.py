"""Trusted local .x3 compiler adapter. Produces a plan; never signs or pays.

Host-configured policy is independent of the submitted source. This does not
execute arbitrary Python, shell commands or VM programs. It uses the existing
intent parser, type checker and NearIntents lowerer, then checks the lowered
payment fields against the host's envelope.
"""
from dataclasses import dataclass, field
from datetime import datetime, timezone
import hashlib
import json
import tempfile
from typing import Callable

import near_intents
from agent_runtime import PolicyViolation


@dataclass(frozen=True)
class IntentCompilerPolicy:
    recipient: str
    origin_chain: str
    destination_chain: str
    asset: str
    max_amount: int
    min_receive: int
    max_slippage_bps: int
    max_fee_bps: int
    refund_to: str
    mandate_owner: str
    clock: Callable[[], datetime] = field(default=lambda: datetime.now(timezone.utc), repr=False)

    def policy_id(self) -> str:
        fields = {k:v for k,v in self.__dict__.items() if k != "clock"}
        fields["compiler_adapter_version"] = 1
        return hashlib.sha256(json.dumps(fields, sort_keys=True).encode()).hexdigest()

    def compile(self, source: bytes) -> bytes:
        if not isinstance(source, bytes):
            raise TypeError("source must be bytes")
        if not source or len(source) > 65536:
            raise ValueError("source size")
        text = source.decode("utf-8", errors="strict")
        # A private temporary snapshot prevents path replacement between hash and parse.
        with tempfile.NamedTemporaryFile(mode="w+", encoding="utf-8", suffix=".x3") as f:
            f.write(text)
            f.flush()
            try:
                _, leg = near_intents.compile_program(f.name, now=self.clock())
            except near_intents.NearIntentsError as exc:
                raise PolicyViolation("compile rejected: " + exc.code) from exc
        exact = {
            "recipient": self.recipient, "refund_to": self.refund_to,
            "mandate_owner": self.mandate_owner,
        }
        for key, expected in exact.items():
            if leg.get(key) != expected:
                raise PolicyViolation("policy violation: " + key)
        if leg["origin"] != {"chain": self.origin_chain, "asset": self.asset}:
            raise PolicyViolation("policy violation: origin")
        if leg["destination"] != {"chain": self.destination_chain, "asset": self.asset}:
            raise PolicyViolation("policy violation: destination")
        if not (0 < leg["amount"] <= self.max_amount):
            raise PolicyViolation("policy violation: amount")
        if leg["min_receive"] < self.min_receive:
            raise PolicyViolation("policy violation: min_receive")
        if leg["slippage_bps"] > self.max_slippage_bps:
            raise PolicyViolation("policy violation: slippage")
        if leg["fee_cap_bps"] > self.max_fee_bps:
            raise PolicyViolation("policy violation: fees")
        return json.dumps({
            "kind": "local-compiled-plan-no-payment",
            "source_sha256": hashlib.sha256(source).hexdigest(), "leg": leg,
        }, sort_keys=True, separators=(",", ":"), allow_nan=False).encode("utf-8")
