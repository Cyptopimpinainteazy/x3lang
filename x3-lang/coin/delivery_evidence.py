"""Read-only evidence for a one-time native-wei purchase of a fixed ERC-20.

Each configured node is trusted to serve canonical history and consensus's
finalized tag. Historical state reads bind to the inclusion hash. Agreement of
independent nodes is an RPC observation, not a consensus or receipt-trie proof.
"""
import re
from urllib.parse import urlsplit

from emitter._keccak import keccak_256
from . import CoinError


def _check(ok, why):
    if not ok:
        raise CoinError("delivery evidence: " + why)


def _hex(value, n=None):
    _check(isinstance(value, str) and re.fullmatch(r"0x(?:[a-fA-F0-9]{2})*", value)
           and (n is None or len(value) == 2 + 2*n), "malformed hexadecimal data")
    return value.lower()


def _quantity(value):
    _check(isinstance(value, str) and re.fullmatch(r"0x(?:0|[1-9a-fA-F][a-fA-F0-9]{0,63})", value),
           "malformed RPC quantity")
    return int(value, 16)


def _object(value):
    _check(isinstance(value, dict), "missing RPC object")
    return value


def _signature(text):
    return "0x" + keccak_256(text.encode("ascii")).hex()


FILLED = _signature("SaleFilled(bytes32,address,address,uint256,uint256)")
TRANSFER = _signature("Transfer(address,address,uint256)")
GETTERS = {s: _signature(s)[:10] for s in (
    "coin()", "treasury()", "declarationHash()", "isFilled(bytes32)", "orders(bytes32)", "decimals()", "buy(bytes32)",
)}
LOG_PAGE_BLOCKS = 2000


def _word(value):
    return "0x" + value[2:].rjust(64, "0")


def _block(rpc, tag):
    row = _object(rpc("eth_getBlockByNumber", [tag, False]))
    height = _quantity(row.get("number"))
    digest = _hex(row.get("hash"), 32)
    if tag != "finalized":
        _check(height == _quantity(tag), "node returned wrong block height")
    return height, digest, row


def _find(rpc, sale, key, start, end):
    match = None
    for first in range(start, end + 1, LOG_PAGE_BLOCKS):
        last = min(end, first + LOG_PAGE_BLOCKS - 1)
        logs = rpc("eth_getLogs", [{
            "address": sale, "topics": [FILLED, key], "fromBlock": hex(first), "toBlock": hex(last),
        }])
        _check(isinstance(logs, list), "malformed historical log response")
        if logs:
            _check(match is None and len(logs) == 1, "multiple fills for order")
            match = _object(logs[0])
            _check(first <= _quantity(match.get("blockNumber")) <= last, "log outside query range")
    return match


def _log(log, tx_hash, block_hash, height, tx_index):
    row = _object(log)
    _check(_hex(row.get("transactionHash"), 32) == tx_hash and _hex(row.get("blockHash"), 32) == block_hash
           and _quantity(row.get("blockNumber")) == height and _quantity(row.get("transactionIndex")) == tx_index
           and row.get("removed") is False, "log outside canonical transaction")
    topics = row.get("topics")
    _check(isinstance(topics, list) and len(topics) <= 4, "invalid log topics")
    return (_quantity(row.get("logIndex")), _hex(row.get("address"), 20),
            [_hex(t, 32) for t in topics], _hex(row.get("data")))


def _verify_state(rpc, t, sale, token, key, anchor):
    # EIP-1898 rejects both non-canonical blocks and ambiguous numeric lookups
    # when requesting historical contract code and state.
    for address, name in ((sale, "sale_code_hash"), (token, "token_code_hash")):
        code = bytes.fromhex(_hex(rpc("eth_getCode", [address, anchor]))[2:])
        _check(code and "0x" + keccak_256(code).hex() == _hex(t[name], 32), "runtime code hash mismatch")
    declaration = _hex("0x" + t["asset_id"], 32)
    getters = (
        (sale, GETTERS["coin()"], _word(token)),
        (sale, GETTERS["treasury()"], _word(_hex(t["treasury"], 20))),
        (sale, GETTERS["declarationHash()"], declaration),
        (token, GETTERS["declarationHash()"], declaration),
        (token, GETTERS["decimals()"], "0x" + f"{t['decimals']:064x}"),
        (sale, GETTERS["orders(bytes32)"] + key[2:],
         _word(_hex(t["buyer"], 20)) + f"{int(t['amount']):064x}{t['expires_at']:064x}{int(t['payment_wei']):064x}{1:064x}"),
        (sale, GETTERS["isFilled(bytes32)"] + key[2:], "0x" + f"{1:064x}"),
    )
    for address, calldata, expected in getters:
        actual = rpc("eth_call", [{"to": address, "data": calldata}, anchor])
        _check(_hex(actual) == expected, "historical contract state mismatch")


def _observe(terms, rpc):
    t = terms
    chain, genesis = t["chain_id"], _hex(t["genesis_hash"], 32)
    sale, token = _hex(t["sale_contract"], 20), _hex(t["token_contract"], 20)
    key, buyer = _hex(t["order_key"], 32), _hex(t["buyer"], 20)
    _check(type(t["from_block"]) is int and t["from_block"] >= 0, "invalid discovery start")
    _check(_quantity(rpc("eth_chainId", [])) == chain and _block(rpc, "0x0")[1] == genesis,
           "network identity mismatch")
    finalized, anchor_hash, _ = _block(rpc, "finalized")
    event = _find(rpc, sale, key, t["from_block"], finalized)
    if event is None:
        return None, finalized

    tx_hash = _hex(event.get("transactionHash"), 32)
    height, block_hash = _quantity(event.get("blockNumber")), _hex(event.get("blockHash"), 32)
    tx_index = _quantity(event.get("transactionIndex"))
    tx = _object(rpc("eth_getTransactionByHash", [tx_hash]))
    receipt = _object(rpc("eth_getTransactionReceipt", [tx_hash]))
    _check(_hex(tx.get("hash"), 32) == tx_hash and _hex(receipt.get("transactionHash"), 32) == tx_hash,
           "transaction hash mismatch")
    for item in (tx, receipt):
        _check(_hex(item.get("blockHash"), 32) == block_hash and _quantity(item.get("blockNumber")) == height
               and _quantity(item.get("transactionIndex")) == tx_index
               and _hex(item.get("from"), 20) == buyer and _hex(item.get("to"), 20) == sale,
               "transaction inclusion or parties mismatch")
    if tx.get("chainId") is not None:
        _check(_quantity(tx["chainId"]) == chain, "transaction chain mismatch")
    amount, payment = int(t["amount"]), int(t["payment_wei"])
    _check(_quantity(receipt.get("status")) == 1 and _quantity(tx.get("value")) == payment
           and _hex(tx.get("input")) == GETTERS["buy(bytes32)"] + key[2:],
           "transaction failed or purchase terms mismatch")

    _, canonical, header = _block(rpc, hex(height))
    transactions = header.get("transactions")
    _check(canonical == block_hash and _quantity(header.get("timestamp")) < t["expires_at"]
           and isinstance(transactions, list) and tx_index < len(transactions)
           and _hex(transactions[tx_index], 32) == tx_hash, "wrong canonical inclusion or deadline")
    _verify_state(rpc, t, sale, token, key, {"blockHash": block_hash, "requireCanonical": True})

    # Match discovery to the actual receipt, including its exact log index. Reject
    # competing transfers from the same token or additional fills by this sale.
    evidence = _log(event, tx_hash, block_hash, height, tx_index)
    _check(evidence[1:] == (sale, [FILLED, key, _word(buyer), _word(token)], "0x" + f"{amount:064x}{payment:064x}"),
           "sale event terms mismatch")
    receipt_logs = receipt.get("logs")
    _check(isinstance(receipt_logs, list), "missing receipt logs")
    records = [_log(log, tx_hash, block_hash, height, tx_index) for log in receipt_logs]
    _check(len({r[0] for r in records}) == len(records), "duplicate log index")
    _check([r for r in records if r[1] == sale and r[2][:1] == [FILLED]] == [evidence],
           "fill absent or ambiguous in receipt")
    transfers = [r[1:] for r in records if r[1] == token and r[2][:1] == [TRANSFER]]
    _check(transfers == [(token, [TRANSFER, _word(sale), _word(buyer)], "0x" + f"{amount:064x}")],
           "exact ERC-20 buyer transfer missing")

    # The trusted node asserts canonical ancestry via eth_getBlockByNumber.
    # Recheck the finalized anchor and inclusion after historical state reads.
    _check(_block(rpc, hex(finalized))[1] == anchor_hash and _block(rpc, hex(height))[1] == block_hash,
           "canonical history changed during observation")
    return {"tx_hash": tx_hash, "block_hash": block_hash, "block_number": height}, finalized


def observe_fill(terms, rpc):
    """Inspect one operator-controlled node. Missing finalized evidence returns None."""
    return _observe(terms, rpc)[0]


def confirmed_fill(terms, rpcs):
    """Require all configured independent HTTPS hosts to agree on the same fill."""
    sources = list(rpcs)
    hosts = set()
    for url, _ in sources:
        endpoint = urlsplit(url)
        host = (endpoint.hostname or "").lower().rstrip(".")
        _check(endpoint.scheme == "https" and host and endpoint.username is None and endpoint.password is None
               and not endpoint.query and not endpoint.fragment and host not in hosts,
               "distinct HTTPS provider hosts required")
        hosts.add(host)
    _check(len(hosts) >= 2, "two independent provider hosts required")
    observations = [_observe(terms, rpc) for _, rpc in sources]
    present = [result for result, _ in observations if result is not None]
    _check(not present or all(result == present[0] for result in present), "providers disagree on fill")
    if len(present) < len(observations):
        return None
    common = min(finalized for _, finalized in observations)
    checkpoints = {_block(rpc, hex(common))[1] for _, rpc in sources}
    _check(len(checkpoints) == 1, "providers disagree on finalized checkpoint")
    return present[0]
