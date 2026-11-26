"""Terms, unsigned wallet calls and durable accounting for an EVM treasury sale."""
import hashlib
import json
import re

from emitter._keccak import keccak_256
from . import CoinError, _address, _fields, _uint
from .sale import network_id


SEPOLIA_CHAIN_ID = 11155111
SEPOLIA_GENESIS = "0x25a5cc106eea7138acab33231d7160d69cb777ee0c2c553fcddf5138993e6dd9"

# Legacy reservation readers count 'reserved' rows. Keep this storage value
# even after delivery so deployed older clients cannot release spent inventory.
# The database trigger also applies to older clients' expiry sweeps and release.
SCHEMA = (
    """CREATE TABLE IF NOT EXISTS sale_delivery_routes (
        destination TEXT PRIMARY KEY NOT NULL, binding TEXT NOT NULL)""",
    """CREATE TABLE IF NOT EXISTS sale_deliveries (
        order_id TEXT PRIMARY KEY NOT NULL REFERENCES sale_holds(order_id),
        terms TEXT NOT NULL, order_key TEXT UNIQUE NOT NULL, evidence TEXT,
        tx_hash TEXT UNIQUE)""",
    """CREATE TABLE IF NOT EXISTS sale_obligations (
        order_id TEXT NOT NULL REFERENCES sale_deliveries(order_id),
        debtor TEXT NOT NULL, creditor TEXT NOT NULL, amount TEXT NOT NULL,
        PRIMARY KEY(order_id, debtor))""",
    """CREATE TRIGGER IF NOT EXISTS sale_keep_delivery_hold
       BEFORE UPDATE OF status ON sale_holds
       WHEN NEW.status != 'reserved' AND EXISTS (
           SELECT 1 FROM sale_deliveries WHERE order_id=OLD.order_id)
       BEGIN SELECT RAISE(IGNORE); END""",
)


def _json(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"))


def _route(policy):
    _fields(policy, ("chain_id", "genesis_hash", "sale_contract", "sale_code_hash",
                     "token_contract", "token_code_hash", "treasury", "from_block",
                     "destination", "payment_wei"), "sale policy")
    chain = policy["chain_id"]
    if type(chain) is not int or chain not in (31337, SEPOLIA_CHAIN_ID):
        raise CoinError("delivery supports a local chain (31337) or Ethereum Sepolia")
    route = {"chain_id": chain, "destination": network_id(policy["destination"])}
    if route["destination"] != "ethereum":
        raise CoinError("this delivery adapter serves only the ethereum treasury")
    for field in ("genesis_hash", "sale_code_hash", "token_code_hash"):
        value = policy[field]
        if not isinstance(value, str) or not re.fullmatch(r"0x[0-9a-fA-F]{64}", value) or int(value, 16) == 0:
            raise CoinError(f"{field} requires a nonzero 32-byte hex hash")
        route[field] = value.lower()
    if chain == SEPOLIA_CHAIN_ID and route["genesis_hash"] != SEPOLIA_GENESIS:
        raise CoinError("wrong genesis hash for Ethereum Sepolia")
    for field in ("sale_contract", "token_contract", "treasury"):
        route[field] = _address(policy[field], field)
    if len({route["sale_contract"], route["token_contract"], route["treasury"]}) != 3:
        raise CoinError("the sale contract, token and treasury must be distinct")
    if type(policy["from_block"]) is not int or not 0 <= policy["from_block"] < 2**63:
        raise CoinError("from_block requires a nonnegative deployment block number")
    route["from_block"] = policy["from_block"]
    _uint(policy["payment_wei"], "payment_wei", 2**256 - 1, 1)
    return route


def _selector(signature):
    return keccak_256(signature.encode("ascii"))[:4].hex()


def wallet_calls(terms):
    key = terms["order_key"][2:]
    buyer = terms["buyer"][2:].rjust(64, "0")
    args = "".join(f"{int(terms[field]):064x}" for field in ("amount", "payment_wei", "expires_at"))
    wallet = {"to": terms["sale_contract"], "chainId": hex(terms["chain_id"])}
    return {
        "authorize": {**wallet, "value": "0x0",
                      "data": "0x" + _selector("authorize(bytes32,address,uint64,uint256,uint64)") + key + buyer + args},
        "buy": {**wallet, "from": terms["buyer"], "value": hex(int(terms["payment_wei"])),
                "data": "0x" + _selector("buy(bytes32)") + key},
    }


def read_delivery(db, order_id):
    row = db.execute("SELECT terms,evidence FROM sale_deliveries WHERE order_id=?", (order_id,)).fetchone()
    if row is None:
        raise CoinError("order has no prepared sale delivery")
    terms = json.loads(row["terms"])
    evidence = json.loads(row["evidence"]) if row["evidence"] else None
    obligations = db.execute(
        "SELECT debtor,creditor,amount FROM sale_obligations WHERE order_id=? ORDER BY debtor", (order_id,)
    )
    return {"status": "delivered" if evidence else "awaiting_confirmation", "terms": terms,
            "wallet_calls": wallet_calls(terms), "evidence": evidence, "obligations": [dict(n) for n in obligations]}


def prepare(db, receipt, decimals, policy):
    route = _route(policy)
    if receipt["destination"] != route["destination"]:
        raise CoinError("delivery route differs from the reserved destination")
    buyer = _address(receipt["buyer"], "reserved buyer")
    if buyer in (route["sale_contract"], route["token_contract"], route["treasury"]):
        raise CoinError("buyer must differ from the sale contract, token and treasury")
    terms = {**route, "decimals": decimals, "asset_id": receipt["asset_id"], "order_id": receipt["order_id"], "buyer": buyer,
             "amount": receipt["amount"], "payment_wei": policy["payment_wei"], "expires_at": receipt["expires_at"]}
    digest = hashlib.sha256(b"x3.coin.sale.order.v1\0" + _json(terms).encode("ascii")).hexdigest()
    terms["order_key"] = "0x" + digest
    previous = db.execute("SELECT terms FROM sale_deliveries WHERE order_id=?", (terms["order_id"],)).fetchone()
    if previous is not None:
        if previous["terms"] != _json(terms):
            raise CoinError("prepared order has different immutable sale terms")
        return read_delivery(db, terms["order_id"])
    if receipt["status"] != "reserved":
        raise CoinError("only an active reservation can prepare a payment")
    binding = _json(route)
    db.execute("INSERT OR IGNORE INTO sale_delivery_routes VALUES(?,?)", (route["destination"], binding))
    stored = db.execute("SELECT binding FROM sale_delivery_routes WHERE destination=?", (route["destination"],)).fetchone()
    if stored["binding"] != binding:
        raise CoinError("route differs from this book's pinned network, contracts, treasury or deployment block")
    db.execute("INSERT INTO sale_deliveries(order_id, terms, order_key) VALUES(?,?,?)",
               (terms["order_id"], _json(terms), terms["order_key"]))
    return read_delivery(db, terms["order_id"])


def record_confirmation(db, config, order_id, evidence):
    current = read_delivery(db, order_id)
    if current["evidence"] is not None:
        if current["evidence"] != evidence:
            raise CoinError("confirmation differs from the recorded delivery")
        return current
    tx_hash = evidence["tx_hash"]
    if db.execute("SELECT 1 FROM sale_deliveries WHERE tx_hash=?", (tx_hash,)).fetchone():
        raise CoinError("purchase transaction already accounts for another order")
    terms = current["terms"]
    hold = db.execute("SELECT charge FROM sale_holds WHERE order_id=?", (order_id,)).fetchone()
    db.execute("UPDATE sale_deliveries SET evidence=?,tx_hash=? WHERE order_id=?", (_json(evidence), tx_hash, order_id))
    db.executemany("INSERT INTO sale_obligations(order_id,debtor,creditor,amount) VALUES(?,?,?,?)",
                   [(order_id, n["id"], terms["destination"], hold["charge"]) for n in config["networks"]
                    if n["id"] != terms["destination"]])
    return read_delivery(db, order_id)
