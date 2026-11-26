"""Read-only observation of buyer payment, complete fill, canonical state and quorum."""
from copy import deepcopy

import pytest

from coin import CoinError
from coin.delivery_evidence import confirmed_fill, observe_fill
from emitter._keccak import keccak_256

FILL = "0x0ba56653b76f2bdff3311011729bf141abd67ac5d8e518aee249241b5596d55f"
TRANSFER = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"
BUYER, SALE, TOKEN, TREASURY = ["0x" + c * 40 for c in "4567"]
KEY, TX, BLOCK, GENESIS = ["0x" + c * 64 for c in "1234"]
DECLARATION = "a" * 64
AMOUNT, PAYMENT, EXPIRY = 100_000_000, 10**16, 1_000_000
HEIGHT = 3000


def word(value):
    digits = value[2:] if isinstance(value, str) else f"{value:x}"
    return "0x" + digits.rjust(64, "0")


def terms():
    return {
        "chain_id": 31337, "genesis_hash": GENESIS, "sale_contract": SALE, "token_contract": TOKEN,
        "sale_code_hash": "0x" + keccak_256(b"sale").hex(), "token_code_hash": "0x" + keccak_256(b"coin").hex(),
        "treasury": TREASURY, "asset_id": DECLARATION, "order_key": KEY, "buyer": BUYER,
        "amount": str(AMOUNT), "payment_wei": str(PAYMENT), "decimals": 6, "expires_at": EXPIRY, "from_block": 20,
    }


class Node:
    def __init__(self, finalized=HEIGHT):
        self.finalized = finalized
        self.windows = []
        self.code = {SALE: "0x" + b"sale".hex(), TOKEN: "0x" + b"coin".hex()}
        self.calls = {
            (SALE, "0x11df9995"): word(TOKEN), (SALE, "0x61d027b3"): word(TREASURY),
            (SALE, "0xc060e0eb"): "0x" + DECLARATION, (TOKEN, "0xc060e0eb"): "0x" + DECLARATION,
            (SALE, "0xb3372347" + KEY[2:]): word(1), (TOKEN, "0x313ce567"): word(6),
            (SALE, "0x9c3f1e90" + KEY[2:]):
                word(BUYER) + "".join(word(n)[2:] for n in (AMOUNT, EXPIRY, PAYMENT, 1)),
        }
        location = {"blockHash": BLOCK, "blockNumber": hex(HEIGHT), "transactionIndex": "0x0"}
        log = {**location, "transactionHash": TX, "removed": False}
        event = {**log, "logIndex": "0x1", "address": SALE,
                 "topics": [FILL, KEY, word(BUYER), word(TOKEN)], "data": word(AMOUNT) + word(PAYMENT)[2:]}
        transfer = {**log, "logIndex": "0x0", "address": TOKEN, "topics": [TRANSFER, word(SALE), word(BUYER)],
                    "data": word(AMOUNT)}
        self.events = [deepcopy(event)]
        self.tx = {**location, "hash": TX, "to": SALE, "from": BUYER, "value": hex(PAYMENT),
                   "input": "0x9c9a1061" + KEY[2:]}
        self.receipt = {**location, "transactionHash": TX, "to": SALE, "from": BUYER,
                        "status": "0x1", "logs": [transfer, event]}
        self.header = {"number": hex(HEIGHT), "hash": BLOCK, "timestamp": hex(EXPIRY-1), "transactions": [TX]}
        self.common_fork = None

    def __call__(self, method, args):
        if method == "eth_chainId":
            return hex(31337)
        if method == "eth_getBlockByNumber":
            assert args[1] is False
            h = self.finalized if args[0] == "finalized" else int(args[0], 16)
            if h == HEIGHT:
                return self.header
            return {"number": hex(h), "hash": GENESIS if h == 0 else self.common_fork or word(h)}
        if method == "eth_getLogs":
            q, = args
            assert q["topics"] == [FILL, KEY] and q["address"] == SALE
            bounds = int(q["fromBlock"], 16), int(q["toBlock"], 16)
            self.windows.append(bounds)
            return self.events if bounds[0] <= HEIGHT <= bounds[1] else []
        if method == "eth_getTransactionByHash":
            assert args == [TX]
            return self.tx
        if method == "eth_getTransactionReceipt":
            assert args == [TX]
            return self.receipt
        if method in ("eth_getCode", "eth_call"):
            assert args[1] == {"blockHash": BLOCK, "requireCanonical": True}
            if method == "eth_getCode":
                return self.code[args[0]]
            return self.calls[args[0]["to"], args[0]["data"]]
        raise AssertionError(method)


def quorum(a, b):
    return confirmed_fill(terms(), [("https://one.example", a), ("https://two.example", b)])


def test_exact_sale_in_block_after_first_log_page_and_finalized_quorum():
    a, b = Node(), Node(HEIGHT + 2)
    assert quorum(a, b) == {"tx_hash": TX, "block_hash": BLOCK, "block_number": HEIGHT}
    assert a.windows == [(20, 2019), (2020, HEIGHT)]
    assert b.windows == [(20, 2019), (2020, HEIGHT + 2)]


def test_missing_or_unfinalized_fill_never_confirms():
    missing = Node()
    missing.events = []
    assert observe_fill(terms(), missing) is None
    assert quorum(Node(), Node(HEIGHT - 1)) is None
    assert quorum(missing, Node()) is None


def test_common_finalized_hash_disagreement_and_provider_errors_fail_closed():
    a, b = Node(HEIGHT + 1), Node(HEIGHT + 2)
    b.common_fork = word(123)
    with pytest.raises(CoinError, match="finalized checkpoint"):
        quorum(a, b)

    def unavailable(_method, _params):
        raise OSError("unavailable")

    with pytest.raises(OSError):
        quorum(Node(), unavailable)


@pytest.mark.parametrize("other", ["https://ONE.example./other", "http://two.example", "https://user@two.example"])
def test_aliased_or_invalid_provider_hosts_rejected(other):
    with pytest.raises(CoinError):
        confirmed_fill(terms(), [("https://one.example", Node()), (other, Node())])


@pytest.mark.parametrize("action", [
    lambda n: n.tx.update(value="0x0"),
    lambda n: n.tx.update(input="0x"),
    lambda n: n.tx.update(chainId="0x1"),
    lambda n: n.tx.update(to=TOKEN),
    lambda n: n.receipt.update(status="0x0"),
    lambda n: n.receipt.update(transactionIndex="0x1"),
    lambda n: n.header.update(transactions=[]),
    lambda n: n.header.update(timestamp=hex(EXPIRY)),
    lambda n: n.header.update(hash=word(999)),
    lambda n: n.code.update({TOKEN: "0x01"}),
    lambda n: n.code.update({SALE: "0x"}),
    lambda n: n.events.append(deepcopy(n.events[0])),
    lambda n: n.events[0].update(removed=True),
    lambda n: n.events[0].update(logIndex="0x7"),
    lambda n: n.receipt["logs"][0].update(address=SALE),
    lambda n: n.receipt["logs"][0].update(data=word(AMOUNT-1)),
    lambda n: n.receipt["logs"][0].update(topics=[TRANSFER, word(SALE), word(TREASURY)]),
    lambda n: n.receipt["logs"][1].update(logIndex="0x0"),
    lambda n: n.receipt["logs"].append({**n.receipt["logs"][0], "logIndex": "0x9"}),
    lambda n: n.calls.update({(TOKEN, "0xc060e0eb"): word(123)}),
    lambda n: n.calls.update({(TOKEN, "0x313ce567"): word(18)}),
    lambda n: n.calls.update({(SALE, "0x61d027b3"): word(BUYER)}),
    lambda n: n.calls.update({(SALE, "0xb3372347" + KEY[2:]): word(0)}),
    lambda n: n.calls.update({(SALE, "0x9c3f1e90" + KEY[2:]):
                             word(BUYER) + "".join(word(i)[2:] for i in (AMOUNT, EXPIRY+1, PAYMENT, 1))}),
])
def test_changed_execution_or_state_is_refused(action):
    node = Node()
    action(node)
    with pytest.raises(CoinError):
        observe_fill(terms(), node)


@pytest.mark.parametrize("size,digest", [
    (134, "e5de5653994e2fa6729d329b65b5f332dee942a7ea54515e173824c232a4ff91"),
    (135, "34367dc248bbd832f4e3e69dfaac2f92638bd0bbd18f2912ba4ef454919cf446"),
    (136, "a6c4d403279fe3e0af03729caada8374b5ca54d8065329a3ebcaeb4b60aa386e"),
    (271, "132f47effd6c8b1b299efa53fe68aece77ec8ae4eb2e294f668eec94f76001e1"),
])
def test_runtime_keccak_block_boundary_matches_ethers_keccak256(size, digest):
    # Reference from ethers.keccak256(new Uint8Array(size).fill(0x61)).
    assert keccak_256(b"a" * size).hex() == digest
