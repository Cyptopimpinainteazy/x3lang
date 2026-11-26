"""Test driver: new processes reconcile a book against signed EthereumJS execution."""
from contextlib import contextmanager
import json
import os
from pathlib import Path
import sys

LANGUAGE = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(LANGUAGE))
from coin.reservations import ReservationBook

ORDER = "ethereum-first-purchase"


class VMTranscript:
    """Read-only JSON-RPC projection of the VM snapshot supplied by the integration test."""

    def __init__(self, sample):
        self.sample = sample
        self.heights = {b["number"]: b for b in sample["blocks"]}
        self.hashes = {b["hash"]: b for b in sample["blocks"]}

    def __call__(self, method, params):
        data = self.sample
        if method == "eth_chainId":
            return hex(data["chain_id"])
        if method == "eth_getBlockByNumber":
            tag, full = params
            assert full is False
            return self.hashes[data["finalized_hash"]] if tag == "finalized" else self.heights[tag]
        if method == "eth_getLogs":
            query, = params
            start, end = int(query["fromBlock"], 16), int(query["toBlock"], 16)
            return [log for log in data["logs"] if
                    start <= int(log["blockNumber"], 16) <= end and
                    log["address"] == query["address"] and log["topics"][:len(query["topics"])] == query["topics"]]
        if method == "eth_getTransactionByHash":
            return next((t for t in data["transactions"] if t["hash"] == params[0]), None)
        if method == "eth_getTransactionReceipt":
            return next((r for r in data["receipts"] if r["transactionHash"] == params[0]), None)
        if method in ("eth_getCode", "eth_call"):
            # This snapshot captures state after its last mined block only.
            request, anchor = params
            assert anchor == {"blockHash": data["finalized_hash"], "requireCanonical": True}
            if method == "eth_getCode":
                return data["code"][request]
            return data["calls"][request["to"] + ":" + request["data"]]
        raise AssertionError(f"unexpected RPC method: {method}")


class InterruptedBook(ReservationBook):
    """Stop a disposable process mid-write, with the confirmation transaction open."""
    @contextmanager
    def _transaction(self, **kwargs):
        with super()._transaction(**kwargs) as (db, now):
            db.create_function("terminate_worker", 0, lambda: os._exit(75))
            db.execute("""CREATE TEMP TRIGGER interrupt_after_posting AFTER INSERT ON main.sale_obligations
                          BEGIN SELECT terminate_worker(); END""")
            yield db, now


def main(mode, db_path, payload):
    asset = payload["asset_id"]
    book_class = InterruptedBook if mode == "interrupt" else ReservationBook
    book = book_class(db_path, clock=lambda: payload["now"])
    if mode == "prepare":
        opening = json.loads((LANGUAGE / "coin/examples/ten_chain_inventory.json").read_text())
        book.initialize({**opening, "asset_id": asset})
        book.reserve({"asset_id": asset, "order_id": ORDER, "destination": "ethereum",
                      "amount": "100000000", "buyer": payload["buyer"], "expires_at": payload["expires_at"]})
        result = book.prepare_delivery(asset, ORDER, payload["policy"])
        assert result["status"] == "awaiting_confirmation"
        return result

    assert mode in ("before", "interrupt", "recover")
    # Two views of one simulated chain exercise quorum code; they do not provide
    # independent public-node or consensus-finality evidence.
    nodes = [(f"https://local-{name}.invalid", VMTranscript(payload["chain"])) for name in ("a", "b")]
    result = book.reconcile_delivery(asset, ORDER, nodes)
    if mode == "interrupt":
        raise AssertionError("worker never interrupted its first obligation write")
    if mode == "before":
        assert result["status"] == "awaiting_confirmation"
        assert result["evidence"] is None and result["obligations"] == []
    else:
        assert result["status"] == "delivered" and result["evidence"]["tx_hash"]
        debtors = {n["id"] for n in book.inventory(asset)["networks"]} - {"ethereum"}
        assert result["obligations"] == [
            {"debtor": n, "creditor": "ethereum", "amount": "10000000"} for n in sorted(debtors)
        ]
        assert len(result["obligations"]) == 9
        balances = book.inventory(asset)["networks"]
        assert all(n["quota_available"] == "99990000000" for n in balances)
        assert next(n for n in balances if n["id"] == "ethereum")["float_available"] == "99900000000"
    return result


if __name__ == "__main__":
    print(json.dumps(main(sys.argv[1], sys.argv[2], json.load(sys.stdin))))
