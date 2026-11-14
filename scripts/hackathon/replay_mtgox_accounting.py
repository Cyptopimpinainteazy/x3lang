#!/usr/bin/env python3
"""Mt. Gox-inspired OFFLINE accounting simulation. Not a historical theft reenactment.

Synthetic chain events demonstrate stable withdrawal intent identity despite
transaction-ID changes, and a fail-closed reserve reconciliation invariant.
These are NOT current X3 Guard checks, Bitcoin consensus, or mainnet evidence.
"""
import argparse
import json
from dataclasses import dataclass, field


@dataclass
class WithdrawalLedger:
    reserves_sats: int
    obligations_sats: int
    settled_intents: set[str] = field(default_factory=set)
    txid_aliases: dict[str, str] = field(default_factory=dict)
    frozen: bool = False

    def reconcile(self) -> bool:
        if self.reserves_sats < self.obligations_sats:
            self.frozen = True
        return not self.frozen

    def process(self, intent: str, txid: str, amount: int) -> str:
        if intent in self.settled_intents:
            self.txid_aliases[txid] = intent
            return "blocked_duplicate_intent"
        if not self.reconcile():
            return "blocked_missing_reserves"
        if amount <= 0 or amount > self.obligations_sats:
            return "invalid_amount"
        self.reserves_sats -= amount
        self.obligations_sats -= amount
        self.settled_intents.add(intent)
        self.txid_aliases[txid] = intent
        return "settled"


def evaluate() -> dict:
    ledger = WithdrawalLedger(2_000_000, 2_000_000)
    first = ledger.process("withdrawal-001", "txid-original", 400_000)
    mutated = ledger.process("withdrawal-001", "txid-altered", 400_000)
    underfunded = WithdrawalLedger(200_000, 900_000)
    shortage = underfunded.process("withdrawal-002", "txid-other", 100_000)
    control = WithdrawalLedger(1_000_000, 1_000_000)
    allowed = control.process("withdrawal-003", "txid-valid", 100_000)
    checks = {
        "original_payment_settles": first == "settled",
        "alternate_txid_cannot_trigger_second_withdrawal": mutated == "blocked_duplicate_intent",
        "exactly_one_economic_debit": ledger.reserves_sats == 1_600_000,
        "changed_txid_linked_to_same_intent": ledger.txid_aliases["txid-altered"] == "withdrawal-001",
        "missing_reserves_freeze": shortage == "blocked_missing_reserves" and underfunded.frozen,
        "authorized_control_allowed": allowed == "settled",
    }
    return {"scope": "synthetic offline accounting model; NOT deployed Guard enforcement", "checks": checks,
            "passed": sum(checks.values()), "total": len(checks), "ok": all(checks.values())}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true")
    args = parser.parse_args()
    report = evaluate()
    if args.json:
        print(json.dumps(report, sort_keys=True, indent=2))
    else:
        print(f"Mt. Gox-inspired offline replay: {report['passed']}/{report['total']} checks passed")
        for name, ok in report["checks"].items():
            print(("PASS" if ok else "FAIL") + " " + name)
        print(report["scope"])
    return 0 if report["ok"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
