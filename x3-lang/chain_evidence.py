"""Independent chain evidence for a NEAR Intents swap: the deposit, the delivery, the refund.

1Click's status endpoint is not signed, so it cannot be trusted to say a swap settled. This
module asks the chains directly:

- **Solana deposit / refund**: `getTransaction` at `finalized` commitment, read as
  `jsonParsed`. The transfer is proven from the transaction's own token balances
  (`meta.preTokenBalances` / `meta.postTokenBalances`): the owner's balance of the mint must
  rise by at least the required amount in a transaction that succeeded (`meta.err` is null)
  and landed before the deadline (`blockTime`). This reads what the chain recorded, not the
  instructions, so it does not depend on which program or route made the transfer.
- **NEAR delivery**: the recipient's NEP-141 balance (`ft_balance_of`, a view call) is read
  when the quote is accepted and again after settlement. The rise must be at least the
  quote's *signed* `minAmountOut`.

Every verdict is fail-closed. A missing transaction, an RPC error, an unreadable response or
endpoints that disagree all produce `verified: false`, never a guess. Each piece of evidence
records the RPC endpoints, the slot or block, and a SHA-256 of each raw result, so anyone can
fetch the same data and compare.

**What a balance proof does and does not show.** A rise in the recipient's balance between the
two reads shows that at least that much of the token arrived. It does not say who sent it, so
an unrelated transfer to the same account in that window would also count. Use a recipient
account that receives nothing else while the swap is in flight.
"""
import base64
import hashlib
import json
import os
import re
from typing import Any, Callable, Dict, List, Optional, Sequence, Tuple

Transport = Callable[[str, str, Dict[str, str], Optional[bytes], float], Tuple[int, bytes]]

DEFAULT_SOLANA_RPC_URLS = ("https://api.mainnet-beta.solana.com",)
DEFAULT_BSC_RPC_URLS = ("https://bsc-dataseed.binance.org", "https://bsc-rpc.publicnode.com")
BSC_CHAIN_ID = 56
DEFAULT_NEAR_RPC_URLS = ("https://free.rpc.fastnear.com",)
EVIDENCE_VERSION = 1


class ChainEvidenceError(Exception):
    def __init__(self, code: str, message: str, detail: Any = None):
        super().__init__(message)
        self.code = code
        self.message = message
        self.detail = detail

    def to_dict(self) -> Dict[str, Any]:
        out: Dict[str, Any] = {"code": self.code, "message": self.message}
        if self.detail is not None:
            out["detail"] = self.detail
        return out


def sha256_json(value: Any) -> str:
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")).hexdigest()


def rpc_urls(env_name: str, defaults: Sequence[str], override: Optional[Sequence[str]] = None) -> List[str]:
    """Endpoints from an explicit list, else a comma-separated environment variable, else defaults."""
    if override:
        urls = list(override)
    else:
        raw = os.environ.get(env_name, "")
        urls = [u.strip() for u in raw.split(",") if u.strip()] or list(defaults)
    if not urls:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"no RPC endpoints configured ({env_name})")
    return urls


def json_rpc(transport: Transport, url: str, method: str, params: Any, timeout: float = 20.0) -> Any:
    """One JSON-RPC 2.0 call. Returns `result` (which may be null); raises on any error."""
    body = json.dumps({"jsonrpc": "2.0", "id": "x3-evidence", "method": method, "params": params}).encode("utf-8")
    status, raw = transport("POST", url, {
        "Content-Type": "application/json",
        "Accept": "application/json",
        "User-Agent": "x3-lang-evidence/1",
    }, body, timeout)
    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: HTTP {status} with no JSON body")
    if not isinstance(data, dict):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: response is not a JSON object")
    if data.get("error") is not None:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: {data['error']}", {"error": data["error"]})
    if status >= 400:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: HTTP {status}")
    if "result" not in data:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: response has no result")
    return data["result"]


# --------------------------------------------------------------------------------------------
# Solana
# --------------------------------------------------------------------------------------------

def _int_amount(entry: Dict[str, Any]) -> int:
    amount = (entry.get("uiTokenAmount") or {}).get("amount")
    if not isinstance(amount, str) or not amount.isdigit():
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"token balance entry has no integer amount: {entry!r}")
    return int(amount)


def spl_balance_delta(tx: Dict[str, Any], *, mint: str, owner: str) -> int:
    """The owner's net change in `mint` across this transaction, in base units."""
    meta = tx.get("meta")
    if not isinstance(meta, dict):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "transaction has no meta")
    pre, post = meta.get("preTokenBalances"), meta.get("postTokenBalances")
    if not isinstance(pre, list) or not isinstance(post, list):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "transaction meta lacks pre/postTokenBalances")

    def total(entries: List[Dict[str, Any]]) -> int:
        return sum(_int_amount(e) for e in entries if e.get("mint") == mint and e.get("owner") == owner)

    return total(post) - total(pre)


def solana_transfer_facts(tx: Optional[Dict[str, Any]], *, signature: str, mint: str, owner: str) -> Dict[str, Any]:
    """What one finalized `getTransaction` result says about a transfer of `mint` to `owner`."""
    if tx is None:
        return {"found": False}
    if not isinstance(tx, dict):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "getTransaction result is not an object")
    signatures = ((tx.get("transaction") or {}).get("signatures")) or []
    meta = tx.get("meta") or {}
    return {
        "found": True,
        "signature_matches": bool(signatures) and signatures[0] == signature,
        "succeeded": isinstance(tx.get("meta"), dict) and meta.get("err") is None,
        "slot": tx.get("slot"),
        "block_time": tx.get("blockTime"),
        "received": spl_balance_delta(tx, mint=mint, owner=owner),
    }


def judge_solana_transfer(facts_by_url: List[Tuple[str, Dict[str, Any], Optional[str]]], *, signature: str,
                          mint: str, owner: str, min_amount: int, not_after_ms: Optional[int],
                          purpose: str, after_slot: Optional[int] = None) -> Dict[str, Any]:
    """Evidence from every endpoint's facts. Verified only if each check passes on every endpoint
    and the endpoints agree exactly."""
    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: bool, **detail: Any) -> None:
        checks.append({"check": name, "ok": bool(ok), **detail})

    first = facts_by_url[0][1]
    check("finalized", all(f.get("found") for _, f, _ in facts_by_url),
          reason=None if all(f.get("found") for _, f, _ in facts_by_url) else
          "not found at finalized commitment (unknown, failed to land, or not yet finalized)")
    if all(f.get("found") for _, f, _ in facts_by_url):
        comparable = [{k: f[k] for k in ("signature_matches", "succeeded", "slot", "block_time", "received")} for _, f, _ in facts_by_url]
        check("endpoints_agree", all(c == comparable[0] for c in comparable), endpoints=len(facts_by_url))
        check("signature_matches", first["signature_matches"])
        check("succeeded", first["succeeded"])
        check("amount", first["received"] >= min_amount, received=str(first["received"]), required=str(min_amount))
        block_time = first["block_time"]
        if not_after_ms is not None:
            check("before_deadline", isinstance(block_time, int) and block_time * 1000 <= not_after_ms,
                  block_time=block_time, not_after_ms=not_after_ms)
        if after_slot is not None:
            check("after_deposit", isinstance(first["slot"], int) and first["slot"] > after_slot,
                  slot=first["slot"], deposit_slot=after_slot)
    return {
        "evidence_version": EVIDENCE_VERSION,
        "kind": "solana_spl_transfer",
        "purpose": purpose,
        "verified": all(c["ok"] for c in checks),
        "signature": signature,
        "mint": mint,
        "owner": owner,
        "required": str(min_amount),
        "observed": str(first.get("received")) if first.get("found") else None,
        "slot": first.get("slot"),
        "block_time": first.get("block_time"),
        "commitment": "finalized",
        "rpcs": [{"url": url, "result_sha256": digest} for url, _, digest in facts_by_url],
        "checks": checks,
    }


class SolanaRpc:
    def __init__(self, transport: Transport, urls: Optional[Sequence[str]] = None, timeout: float = 20.0):
        self.urls = rpc_urls("SOLANA_RPC_URLS", DEFAULT_SOLANA_RPC_URLS, urls)
        self.transport = transport
        self.timeout = timeout

    def get_transaction(self, url: str, signature: str) -> Optional[Dict[str, Any]]:
        return json_rpc(self.transport, url, "getTransaction", [
            signature,
            {"encoding": "jsonParsed", "commitment": "finalized", "maxSupportedTransactionVersion": 0},
        ], self.timeout)

    def verify_transfer(self, *, signature: str, mint: str, owner: str, min_amount: int,
                        not_after_ms: Optional[int], purpose: str, after_slot: Optional[int] = None) -> Dict[str, Any]:
        facts = []
        for url in self.urls:
            tx = self.get_transaction(url, signature)
            facts.append((url, solana_transfer_facts(tx, signature=signature, mint=mint, owner=owner),
                          sha256_json(tx) if tx is not None else None))
        return judge_solana_transfer(facts, signature=signature, mint=mint, owner=owner, min_amount=min_amount,
                                     not_after_ms=not_after_ms, purpose=purpose, after_slot=after_slot)


# --------------------------------------------------------------------------------------------
# BNB Smart Chain
# --------------------------------------------------------------------------------------------

def _hex_int(value: Any, field: str) -> int:
    if not isinstance(value, str) or not value.startswith("0x"):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{field} is not a hex quantity: {value!r}")
    try:
        return int(value, 16)
    except ValueError:
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{field} is not a hex quantity: {value!r}")


def bsc_native_transfer_facts(*, tx: Optional[Dict[str, Any]], receipt: Optional[Dict[str, Any]],
                              block: Optional[Dict[str, Any]], finalized: Optional[Dict[str, Any]],
                              chain_id: Any, tx_hash: str) -> Dict[str, Any]:
    """Facts from one BSC endpoint for a native-BNB transaction and finality boundary."""
    if tx is None or receipt is None:
        return {"found": False}
    if not isinstance(tx, dict) or not isinstance(receipt, dict):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "BSC transaction or receipt is not an object")
    tx_block = _hex_int(tx.get("blockNumber"), "transaction.blockNumber")
    receipt_block = _hex_int(receipt.get("blockNumber"), "receipt.blockNumber")
    if tx_block != receipt_block:
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "BSC transaction and receipt disagree on blockNumber")
    if not isinstance(block, dict):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "BSC transaction block is unavailable")
    block_number = _hex_int(block.get("number"), "block.number")
    if block_number != tx_block:
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "BSC block number does not match the transaction")
    finalized_number = (
        _hex_int(finalized.get("number"), "finalized.number")
        if isinstance(finalized, dict) and finalized.get("number") is not None
        else None
    )
    return {
        "found": True,
        "chainId": _hex_int(chain_id, "eth_chainId"),
        "hash": tx.get("hash"),
        "transaction_hash_matches": tx.get("hash") == tx_hash == receipt.get("transactionHash"),
        "succeeded": receipt.get("status") == "0x1",
        "from": str(tx.get("from") or "").lower(),
        "to": str(tx.get("to") or "").lower(),
        "value": _hex_int(tx.get("value"), "transaction.value"),
        "blockNumber": tx_block,
        "blockHash": tx.get("blockHash"),
        "receiptBlockHash": receipt.get("blockHash"),
        "blockObjectHash": block.get("hash"),
        "block_hash_matches": tx.get("blockHash") == receipt.get("blockHash") == block.get("hash"),
        "blockTimestamp": _hex_int(block.get("timestamp"), "block.timestamp"),
        "finalizedBlock": finalized_number,
    }


def judge_bsc_native_transfer(facts_by_url: List[Tuple[str, Dict[str, Any], Optional[str]]], *,
                              tx_hash: str, recipient: str, min_amount: int, not_after_ms: Optional[int],
                              purpose: str, after_block: Optional[int] = None) -> Dict[str, Any]:
    """Verify a native BNB transfer only when every endpoint agrees it is finalized."""
    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: bool, **detail: Any) -> None:
        checks.append({"check": name, "ok": bool(ok), **detail})

    found = all(f.get("found") for _, f, _ in facts_by_url)
    check("found", found)
    first = facts_by_url[0][1]
    if found:
        comparable_keys = (
            "chainId", "hash", "transaction_hash_matches", "succeeded", "from", "to", "value",
            "blockNumber", "blockHash", "receiptBlockHash", "blockObjectHash",
            "block_hash_matches", "blockTimestamp",
        )
        comparable = [{k: f.get(k) for k in comparable_keys} for _, f, _ in facts_by_url]
        check("endpoints_agree", all(value == comparable[0] for value in comparable[1:]),
              endpoints=len(facts_by_url))
        check("transaction_hash_matches", first["transaction_hash_matches"] is True)
        check("chain_id", all(f.get("chainId") == BSC_CHAIN_ID for _, f, _ in facts_by_url),
              expected=BSC_CHAIN_ID, observed=[f.get("chainId") for _, f, _ in facts_by_url])
        check("block_hash_matches", first["block_hash_matches"] is True,
              transactionHash=first["blockHash"], receiptHash=first["receiptBlockHash"],
              blockHash=first["blockObjectHash"])
        check("succeeded", first["succeeded"] is True)
        check("recipient", first["to"] == recipient.lower(), expected=recipient.lower(), observed=first["to"])
        check("amount", first["value"] >= min_amount, observed=str(first["value"]), required=str(min_amount))
        finalized = [f.get("finalizedBlock") for _, f, _ in facts_by_url]
        check("finalized", all(isinstance(n, int) and n >= first["blockNumber"] for n in finalized),
              finalizedBlocks=finalized, transactionBlock=first["blockNumber"])
        if not_after_ms is not None:
            check("before_deadline", first["blockTimestamp"] * 1000 <= not_after_ms,
                  blockTimestamp=first["blockTimestamp"], notAfterMs=not_after_ms)
        if after_block is not None:
            check("after_deposit", first["blockNumber"] > after_block,
                  blockNumber=first["blockNumber"], depositBlock=after_block)
    return {
        "evidence_version": EVIDENCE_VERSION,
        "kind": "bsc_native_transfer",
        "purpose": purpose,
        "verified": all(c["ok"] for c in checks),
        "transaction_hash": tx_hash,
        "chainId": first.get("chainId"),
        "recipient": recipient.lower(),
        "required": str(min_amount),
        "observed": str(first.get("value")) if first.get("found") else None,
        "blockNumber": first.get("blockNumber"),
        "finalizedBlock": first.get("finalizedBlock"),
        "blockTimestamp": first.get("blockTimestamp"),
        "commitment": "finalized",
        "rpcs": [{"url": url, "result_sha256": digest} for url, _, digest in facts_by_url],
        "checks": checks,
    }


class BscRpc:
    def __init__(self, transport: Transport, urls: Optional[Sequence[str]] = None, timeout: float = 20.0):
        self.urls = rpc_urls("BSC_RPC_URLS", DEFAULT_BSC_RPC_URLS, urls)
        self.transport = transport
        self.timeout = timeout

    def _facts(self, url: str, tx_hash: str) -> Tuple[Dict[str, Any], str]:
        chain_id = json_rpc(self.transport, url, "eth_chainId", [], self.timeout)
        tx = json_rpc(self.transport, url, "eth_getTransactionByHash", [tx_hash], self.timeout)
        receipt = json_rpc(self.transport, url, "eth_getTransactionReceipt", [tx_hash], self.timeout)
        block = None
        finalized = None
        if isinstance(tx, dict) and tx.get("blockNumber"):
            block = json_rpc(self.transport, url, "eth_getBlockByNumber", [tx["blockNumber"], False], self.timeout)
            finalized = json_rpc(self.transport, url, "eth_getBlockByNumber", ["finalized", False], self.timeout)
        raw = {"chainId": chain_id, "transaction": tx, "receipt": receipt, "block": block, "finalized": finalized}
        return bsc_native_transfer_facts(
            tx=tx,
            receipt=receipt,
            block=block,
            finalized=finalized,
            chain_id=chain_id,
            tx_hash=tx_hash,
        ), sha256_json(raw)

    def verify_native_transfer(self, *, tx_hash: str, recipient: str, min_amount: int,
                               not_after_ms: Optional[int], purpose: str,
                               after_block: Optional[int] = None) -> Dict[str, Any]:
        facts = []
        for url in self.urls:
            facts_by_url, digest = self._facts(url, tx_hash)
            facts.append((url, facts_by_url, digest))
        return judge_bsc_native_transfer(
            facts,
            tx_hash=tx_hash,
            recipient=recipient,
            min_amount=min_amount,
            not_after_ms=not_after_ms,
            purpose=purpose,
            after_block=after_block,
        )


# --------------------------------------------------------------------------------------------
# EVM chains (native and ERC-20 transfers)
# --------------------------------------------------------------------------------------------
#
# The same proof as BSC above, for every EVM chain the 1Click adapter routes, plus ERC-20
# transfers: the receipt's `Transfer(address,address,uint256)` logs emitted by the token
# contract itself. Every endpoint must agree, the block must be at or below the chain's
# `finalized` block, and the chain ID must be the chain's. No endpoint is assumed: each chain's
# RPCs come from `X3_EVM_RPC_URLS_<CHAIN>` (comma-separated), so an unconfigured chain fails
# closed instead of trusting a default nobody reviewed. BSC keeps its reviewed defaults.

EVM_CHAIN_IDS = {
    "ethereum": 1,
    "optimism": 10,
    "bsc": 56,
    "gnosis": 100,
    "polygon": 137,
    "monad": 143,
    "base": 8453,
    "arbitrum": 42161,
    "avalanche": 43114,
    "berachain": 80094,
    "scroll": 534352,
    "xlayer": 196,
    "abstract": 2741,
    "plasma": 9745,
}
EVM_DEFAULT_RPC_URLS = {"bsc": DEFAULT_BSC_RPC_URLS}
#: keccak256("Transfer(address,address,uint256)")
ERC20_TRANSFER_TOPIC = "0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef"


def _topic_address(topic: Any, field: str) -> str:
    if not isinstance(topic, str) or len(topic) != 66 or not topic.startswith("0x"):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{field} is not a 32-byte topic: {topic!r}")
    if topic[2:26].strip("0"):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{field} is not a left-padded address: {topic!r}")
    return "0x" + topic[26:].lower()


def erc20_transfers(receipt: Dict[str, Any], token: str) -> List[Dict[str, Any]]:
    """Every `Transfer` the token contract itself logged in this receipt, in log order."""
    token = token.lower()
    out = []
    logs = receipt.get("logs")
    if not isinstance(logs, list):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "receipt has no logs list")
    for i, log in enumerate(logs):
        if not isinstance(log, dict):
            raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"receipt.logs[{i}] is not an object")
        if str(log.get("address", "")).lower() != token or log.get("removed") is True:
            continue
        topics = log.get("topics") or []
        if not topics or str(topics[0]).lower() != ERC20_TRANSFER_TOPIC:
            continue
        if len(topics) != 3:
            raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"receipt.logs[{i}] Transfer has {len(topics)} topics")
        data = log.get("data")
        if not isinstance(data, str) or not data.startswith("0x") or len(data) != 66:
            raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"receipt.logs[{i}] Transfer data is not one uint256")
        out.append({"from": _topic_address(topics[1], f"logs[{i}].topics[1]"),
                    "to": _topic_address(topics[2], f"logs[{i}].topics[2]"),
                    "value": _hex_int(data, f"logs[{i}].data")})
    return out


def judge_evm_transfer(facts_by_url: List[Tuple[str, Dict[str, Any], Optional[str]]], *, chain: str,
                       tx_hash: str, recipient: str, min_amount: int, not_after_ms: Optional[int],
                       purpose: str, token: Optional[str] = None,
                       after_block: Optional[int] = None) -> Dict[str, Any]:
    """Verify a native (token=None) or ERC-20 transfer only when every endpoint agrees it is final."""
    expected_chain_id = EVM_CHAIN_IDS[chain]
    recipient = recipient.lower()
    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: bool, **detail: Any) -> None:
        checks.append({"check": name, "ok": bool(ok), **detail})

    if not facts_by_url:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"no {chain} RPC endpoints answered")
    found = all(f.get("found") for _, f, _ in facts_by_url)
    check("found", found)
    first = facts_by_url[0][1]
    observed = None
    if found:
        comparable_keys = (
            "chainId", "hash", "transaction_hash_matches", "succeeded", "from", "to", "value",
            "blockNumber", "blockHash", "receiptBlockHash", "blockObjectHash", "block_hash_matches",
            "blockTimestamp", "token_transfers",
        )
        comparable = [{k: f.get(k) for k in comparable_keys} for _, f, _ in facts_by_url]
        check("endpoints_agree", all(value == comparable[0] for value in comparable[1:]),
              endpoints=len(facts_by_url))
        check("transaction_hash_matches", first["transaction_hash_matches"] is True)
        check("chain_id", all(f.get("chainId") == expected_chain_id for _, f, _ in facts_by_url),
              expected=expected_chain_id, observed=[f.get("chainId") for _, f, _ in facts_by_url])
        check("block_hash_matches", first["block_hash_matches"] is True,
              transactionHash=first["blockHash"], receiptHash=first["receiptBlockHash"],
              blockHash=first["blockObjectHash"])
        check("succeeded", first["succeeded"] is True)
        if token is None:
            observed = first["value"]
            check("recipient", first["to"] == recipient, expected=recipient, observed=first["to"])
        else:
            # The transaction may call a router; what counts is what the token contract logged.
            observed = sum(t["value"] for t in first["token_transfers"] if t["to"] == recipient)
            check("token_contract", True, token=token.lower())
            check("recipient", observed > 0, expected=recipient,
                  transfers_to_recipient=sum(1 for t in first["token_transfers"] if t["to"] == recipient))
        check("amount", observed >= min_amount, observed=str(observed), required=str(min_amount))
        finalized = [f.get("finalizedBlock") for _, f, _ in facts_by_url]
        check("finalized", all(isinstance(n, int) and n >= first["blockNumber"] for n in finalized),
              finalizedBlocks=finalized, transactionBlock=first["blockNumber"])
        if not_after_ms is not None:
            check("before_deadline", first["blockTimestamp"] * 1000 <= not_after_ms,
                  blockTimestamp=first["blockTimestamp"], notAfterMs=not_after_ms)
        if after_block is not None:
            check("after_deposit", first["blockNumber"] > after_block,
                  blockNumber=first["blockNumber"], depositBlock=after_block)
    return {
        "evidence_version": EVIDENCE_VERSION,
        "kind": "evm_native_transfer" if token is None else "evm_token_transfer",
        "chain": chain,
        "purpose": purpose,
        "verified": all(c["ok"] for c in checks),
        "transaction_hash": tx_hash,
        "chainId": first.get("chainId"),
        "token": token.lower() if token else None,
        "recipient": recipient,
        "required": str(min_amount),
        "observed": str(observed) if observed is not None else None,
        "blockNumber": first.get("blockNumber"),
        "finalizedBlock": first.get("finalizedBlock"),
        "blockTimestamp": first.get("blockTimestamp"),
        "commitment": "finalized",
        "rpcs": [{"url": url, "result_sha256": digest} for url, _, digest in facts_by_url],
        "checks": checks,
    }


class EvmRpc:
    """Finalized transfer evidence on one EVM chain, from every configured endpoint."""

    def __init__(self, chain: str, transport: Transport, urls: Optional[Sequence[str]] = None, timeout: float = 20.0):
        if chain not in EVM_CHAIN_IDS:
            raise ChainEvidenceError("X3_EVIDENCE_UNSUPPORTED", f"no EVM evidence for chain {chain!r}")
        self.chain = chain
        self.urls = rpc_urls(f"X3_EVM_RPC_URLS_{chain.upper()}", EVM_DEFAULT_RPC_URLS.get(chain, ()), urls)
        self.transport = transport
        self.timeout = timeout

    def _facts(self, url: str, tx_hash: str, token: Optional[str]) -> Tuple[Dict[str, Any], str]:
        chain_id = json_rpc(self.transport, url, "eth_chainId", [], self.timeout)
        tx = json_rpc(self.transport, url, "eth_getTransactionByHash", [tx_hash], self.timeout)
        receipt = json_rpc(self.transport, url, "eth_getTransactionReceipt", [tx_hash], self.timeout)
        block = finalized = None
        if isinstance(tx, dict) and tx.get("blockNumber"):
            block = json_rpc(self.transport, url, "eth_getBlockByNumber", [tx["blockNumber"], False], self.timeout)
            finalized = json_rpc(self.transport, url, "eth_getBlockByNumber", ["finalized", False], self.timeout)
        raw = {"chainId": chain_id, "transaction": tx, "receipt": receipt, "block": block, "finalized": finalized}
        facts = bsc_native_transfer_facts(tx=tx, receipt=receipt, block=block, finalized=finalized,
                                          chain_id=chain_id, tx_hash=tx_hash)
        if facts.get("found"):
            facts["token_transfers"] = erc20_transfers(receipt, token) if token else None
        return facts, sha256_json(raw)

    def _verify(self, *, tx_hash: str, recipient: str, min_amount: int, not_after_ms: Optional[int],
                purpose: str, token: Optional[str], after_block: Optional[int]) -> Dict[str, Any]:
        facts = [(url, *self._facts(url, tx_hash, token)) for url in self.urls]
        return judge_evm_transfer(facts, chain=self.chain, tx_hash=tx_hash, recipient=recipient,
                                  min_amount=min_amount, not_after_ms=not_after_ms, purpose=purpose,
                                  token=token, after_block=after_block)

    def verify_native_transfer(self, *, tx_hash: str, recipient: str, min_amount: int,
                               not_after_ms: Optional[int], purpose: str,
                               after_block: Optional[int] = None) -> Dict[str, Any]:
        return self._verify(tx_hash=tx_hash, recipient=recipient, min_amount=min_amount, not_after_ms=not_after_ms,
                            purpose=purpose, token=None, after_block=after_block)

    def verify_token_transfer(self, *, tx_hash: str, token: str, recipient: str, min_amount: int,
                              not_after_ms: Optional[int], purpose: str,
                              after_block: Optional[int] = None) -> Dict[str, Any]:
        return self._verify(tx_hash=tx_hash, recipient=recipient, min_amount=min_amount, not_after_ms=not_after_ms,
                            purpose=purpose, token=token, after_block=after_block)


# --------------------------------------------------------------------------------------------
# Tron (TRX and TRC-20)
# --------------------------------------------------------------------------------------------
#
# Read from a full node's **solidity** API (`/walletsolidity/...`), which answers only for
# solidified, irreversible blocks: an unconfirmed transaction is "not found", never a guess.
# A TRC-20 transfer is proven from the token contract's own `Transfer` log in the transaction
# info; native TRX from the `TransferContract` itself. Endpoints come from `X3_TRON_RPC_URLS`
# (no default) and must all agree.

TRANSFER_TOPIC_HEX = ERC20_TRANSFER_TOPIC[2:]


def _address_registry():
    import importlib.util
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "registry.py")
    spec = importlib.util.spec_from_file_location("registry_for_evidence", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def tron_post(transport: Transport, url: str, path: str, body: Dict[str, Any], timeout: float = 20.0) -> Any:
    status, raw = transport("POST", url.rstrip("/") + path,
                            {"Content-Type": "application/json", "Accept": "application/json",
                             "User-Agent": "x3-lang-evidence/1"},
                            json.dumps(body).encode(), timeout)
    try:
        data = json.loads(raw.decode() or "{}")
    except (UnicodeDecodeError, ValueError):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url}{path}: HTTP {status} with no JSON body")
    if status != 200 or not isinstance(data, dict):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url}{path}: HTTP {status}")
    if data.get("Error"):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url}{path}: {data['Error']}")
    return data


def tron_transfer_facts(tx: Dict[str, Any], info: Dict[str, Any], *, tx_id: str,
                        token: Optional[str]) -> Dict[str, Any]:
    """Facts from one Tron endpoint. `token` is the TRC-20 contract (base58), None for TRX."""
    reg = _address_registry()
    if not tx.get("txID") or not info.get("id"):
        return {"found": False}
    contracts = ((tx.get("raw_data") or {}).get("contract")) or []
    ret = (tx.get("ret") or [{}])[0] or {}
    receipt = info.get("receipt") or {}
    succeeded = (ret.get("contractRet") == "SUCCESS" and info.get("result") != "FAILED"
                 and receipt.get("result", "SUCCESS") == "SUCCESS")
    facts: Dict[str, Any] = {
        "found": True,
        "txID": tx.get("txID"),
        "transaction_hash_matches": tx.get("txID") == info.get("id") == tx_id,
        "succeeded": succeeded,
        "blockNumber": info.get("blockNumber"),
        "blockTimeStampMs": info.get("blockTimeStamp"),
        "contract_types": [c.get("type") for c in contracts if isinstance(c, dict)],
    }
    if not isinstance(facts["blockNumber"], int) or not isinstance(facts["blockTimeStampMs"], int):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "Tron transaction info has no integer block number/time")
    if token is None:
        transfers = []
        for c in contracts:
            if isinstance(c, dict) and c.get("type") == "TransferContract":
                value = ((c.get("parameter") or {}).get("value")) or {}
                to_hex, amount = value.get("to_address"), value.get("amount")
                if not isinstance(to_hex, str) or type(amount) is not int:
                    raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "TransferContract has no to_address/amount")
                transfers.append({"to": reg.tron_address_from_hex(to_hex), "value": amount})
        facts["transfers"] = transfers
    else:
        token_hex = reg.tron_address_bytes(token).hex()
        transfers = []
        for i, log in enumerate(info.get("log") or []):
            if not isinstance(log, dict) or "41" + str(log.get("address", "")).lower() != token_hex:
                continue
            topics = [str(t).lower() for t in log.get("topics") or []]
            if not topics or topics[0] != TRANSFER_TOPIC_HEX:
                continue
            data = str(log.get("data", ""))
            if len(topics) != 3 or len(topics[2]) != 64 or len(data) != 64:
                raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"Tron log[{i}] is not a TRC-20 Transfer")
            transfers.append({"to": reg.tron_address_from_hex("41" + topics[2][24:]), "value": int(data, 16)})
        facts["transfers"] = transfers
    return facts


def judge_tron_transfer(facts_by_url: List[Tuple[str, Dict[str, Any], Optional[str]]], *, tx_id: str,
                        recipient: str, min_amount: int, not_after_ms: Optional[int], purpose: str,
                        token: Optional[str] = None, after_block: Optional[int] = None) -> Dict[str, Any]:
    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: bool, **detail: Any) -> None:
        checks.append({"check": name, "ok": bool(ok), **detail})

    if not facts_by_url:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", "no Tron endpoints answered")
    found = all(f.get("found") for _, f, _ in facts_by_url)
    # The solidity API only knows solidified blocks, so "found" there is finality.
    check("solidified", found)
    first = facts_by_url[0][1]
    observed = None
    if found:
        keys = ("txID", "transaction_hash_matches", "succeeded", "blockNumber", "blockTimeStampMs", "transfers")
        comparable = [{k: f.get(k) for k in keys} for _, f, _ in facts_by_url]
        check("endpoints_agree", all(c == comparable[0] for c in comparable[1:]), endpoints=len(facts_by_url))
        check("transaction_hash_matches", first["transaction_hash_matches"] is True)
        check("succeeded", first["succeeded"] is True)
        observed = sum(t["value"] for t in first["transfers"] if t["to"] == recipient)
        check("recipient", observed > 0, expected=recipient,
              transfers_to_recipient=sum(1 for t in first["transfers"] if t["to"] == recipient))
        check("amount", observed >= min_amount, observed=str(observed), required=str(min_amount))
        if not_after_ms is not None:
            check("before_deadline", first["blockTimeStampMs"] <= not_after_ms,
                  blockTimeStampMs=first["blockTimeStampMs"], notAfterMs=not_after_ms)
        if after_block is not None:
            check("after_deposit", first["blockNumber"] > after_block,
                  blockNumber=first["blockNumber"], depositBlock=after_block)
    return {
        "evidence_version": EVIDENCE_VERSION,
        "kind": "tron_native_transfer" if token is None else "tron_token_transfer",
        "chain": "tron",
        "purpose": purpose,
        "verified": all(c["ok"] for c in checks),
        "transaction_hash": tx_id,
        "token": token,
        "recipient": recipient,
        "required": str(min_amount),
        "observed": str(observed) if observed is not None else None,
        "blockNumber": first.get("blockNumber"),
        "blockTimeStampMs": first.get("blockTimeStampMs"),
        "commitment": "solidified",
        "rpcs": [{"url": url, "result_sha256": digest} for url, _, digest in facts_by_url],
        "checks": checks,
    }


class TronRpc:
    """Solidified TRX and TRC-20 transfer evidence from every configured Tron endpoint."""

    def __init__(self, transport: Transport, urls: Optional[Sequence[str]] = None, timeout: float = 20.0):
        self.urls = rpc_urls("X3_TRON_RPC_URLS", (), urls)
        self.transport = transport
        self.timeout = timeout

    def _verify(self, *, tx_hash: str, recipient: str, min_amount: int, not_after_ms: Optional[int],
                purpose: str, token: Optional[str], after_block: Optional[int]) -> Dict[str, Any]:
        reg = _address_registry()
        if not reg.is_valid_tron_address(recipient) or (token and not reg.is_valid_tron_address(token)):
            raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "Tron recipient or token is not a checksummed address")
        facts = []
        for url in self.urls:
            tx = tron_post(self.transport, url, "/walletsolidity/gettransactionbyid", {"value": tx_hash}, self.timeout)
            info = tron_post(self.transport, url, "/walletsolidity/gettransactioninfobyid", {"value": tx_hash},
                             self.timeout)
            facts.append((url, tron_transfer_facts(tx, info, tx_id=tx_hash, token=token),
                          sha256_json({"transaction": tx, "info": info})))
        return judge_tron_transfer(facts, tx_id=tx_hash, recipient=recipient, min_amount=min_amount,
                                   not_after_ms=not_after_ms, purpose=purpose, token=token, after_block=after_block)

    def verify_native_transfer(self, *, tx_hash: str, recipient: str, min_amount: int,
                               not_after_ms: Optional[int], purpose: str,
                               after_block: Optional[int] = None) -> Dict[str, Any]:
        return self._verify(tx_hash=tx_hash, recipient=recipient, min_amount=min_amount, not_after_ms=not_after_ms,
                            purpose=purpose, token=None, after_block=after_block)

    def verify_token_transfer(self, *, tx_hash: str, token: str, recipient: str, min_amount: int,
                              not_after_ms: Optional[int], purpose: str,
                              after_block: Optional[int] = None) -> Dict[str, Any]:
        return self._verify(tx_hash=tx_hash, recipient=recipient, min_amount=min_amount, not_after_ms=not_after_ms,
                            purpose=purpose, token=token, after_block=after_block)


# --------------------------------------------------------------------------------------------
# UTXO chains (Bitcoin, Litecoin, Dogecoin, Bitcoin Cash, Zcash, Dash)
# --------------------------------------------------------------------------------------------
#
# Read from each chain's node JSON-RPC (bitcoind and its forks: `getblockchaininfo`,
# `getrawtransaction <txid> 1`, `getblockheader`). The node needs `txindex=1` (or a provider
# that has it). A payment is the sum of the transaction's outputs to the address; the block
# must be at least `UTXO_MIN_CONFIRMATIONS[chain]` deep on every endpoint, every endpoint must
# report `chain == "main"`, and all must agree on the transaction and its block. Coin values
# arrive as JSON numbers, so they are parsed as Decimal and must be whole base units: a float
# would round them. Endpoints come from `X3_UTXO_RPC_URLS_<CHAIN>`; there are no defaults.

from decimal import Decimal as _Decimal

#: Evidence depth per chain: deep enough that a reorg past it is not a practical risk at the
#: chain's block time (Bitcoin ~1h, Litecoin ~30m, Dogecoin ~40m, BCH ~1h, Zcash ~30m, Dash ~15m).
UTXO_MIN_CONFIRMATIONS = {"bitcoin": 6, "litecoin": 12, "dogecoin": 40, "bitcoincash": 6, "zcash": 24, "dash": 6}
#: The address kind (`registry.UTXO_ADDRESS_RULES`) for each chain.
UTXO_ADDRESS_KIND = {"bitcoin": "btc", "litecoin": "ltc", "dogecoin": "doge", "bitcoincash": "bch",
                     "zcash": "zec", "dash": "dash"}
UTXO_DECIMALS = 8


def utxo_rpc(transport: Transport, url: str, method: str, params: Any, timeout: float = 20.0) -> Any:
    """One bitcoind-style JSON-RPC call, numbers parsed as Decimal."""
    body = json.dumps({"jsonrpc": "1.0", "id": "x3-evidence", "method": method, "params": params}).encode()
    status, raw = transport("POST", url, {"Content-Type": "application/json", "Accept": "application/json",
                                          "User-Agent": "x3-lang-evidence/1"}, body, timeout)
    try:
        data = json.loads(raw.decode("utf-8"), parse_float=_Decimal)
    except (UnicodeDecodeError, ValueError):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: HTTP {status} with no JSON body")
    if not isinstance(data, dict):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: response is not a JSON object")
    if data.get("error") is not None:
        error = data["error"]
        # -5: no such transaction (not seen, or the node has no txindex): "not found", not an outage.
        if isinstance(error, dict) and error.get("code") == -5 and method == "getrawtransaction":
            return None
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: {error}")
    if status >= 400 or "result" not in data:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: HTTP {status}")
    return data["result"]


def _coin_units(value: Any, field: str) -> int:
    if isinstance(value, bool) or not isinstance(value, (int, _Decimal)):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{field} is not a number: {value!r}")
    units = _Decimal(value) * (10 ** UTXO_DECIMALS)
    if units != units.to_integral_value() or units < 0:
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{field} {value} is not a whole number of base units")
    return int(units)


def utxo_transfer_facts(chain: str, info: Any, tx: Any, header: Any, *, tx_id: str) -> Dict[str, Any]:
    reg = _address_registry()
    kind = UTXO_ADDRESS_KIND[chain]
    if not isinstance(info, dict):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "getblockchaininfo did not return an object")
    facts: Dict[str, Any] = {"network": info.get("chain")}
    if tx is None:
        return {**facts, "found": False}
    if not isinstance(tx, dict) or not isinstance(tx.get("vout"), list):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "getrawtransaction did not return a decoded transaction")
    outputs = []
    for i, out in enumerate(tx["vout"]):
        spk = (out or {}).get("scriptPubKey") or {}
        addresses = [spk["address"]] if spk.get("address") else list(spk.get("addresses") or [])
        if len(addresses) != 1:
            continue  # bare multisig, OP_RETURN and the like pay no single address
        canonical = reg.canonical_utxo_address(kind, addresses[0])
        if canonical is None:
            continue
        outputs.append({"address": canonical, "value": _coin_units(out.get("value"), f"vout[{i}].value")})
    in_block = bool(tx.get("blockhash")) and isinstance(header, dict)
    return {
        **facts,
        "found": True,
        "txid": tx.get("txid"),
        "transaction_hash_matches": tx.get("txid") == tx_id,
        "outputs": outputs,
        "blockhash": tx.get("blockhash"),
        "in_block": in_block,
        "block_hash_matches": in_block and header.get("hash") == tx.get("blockhash"),
        "height": header.get("height") if in_block else None,
        "blocktime": header.get("time") if in_block else None,
        "confirmations": header.get("confirmations") if in_block else 0,
    }


def judge_utxo_transfer(facts_by_url: List[Tuple[str, Dict[str, Any], Optional[str]]], *, chain: str, tx_id: str,
                        recipient: str, min_amount: int, not_after_ms: Optional[int], purpose: str,
                        after_block: Optional[int] = None) -> Dict[str, Any]:
    reg = _address_registry()
    recipient = reg.canonical_utxo_address(UTXO_ADDRESS_KIND[chain], recipient)
    need = UTXO_MIN_CONFIRMATIONS[chain]
    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: bool, **detail: Any) -> None:
        checks.append({"check": name, "ok": bool(ok), **detail})

    if not facts_by_url:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"no {chain} endpoints answered")
    check("mainnet", all(f.get("network") == "main" for _, f, _ in facts_by_url),
          networks=[f.get("network") for _, f, _ in facts_by_url])
    found = all(f.get("found") for _, f, _ in facts_by_url)
    check("found", found)
    first = facts_by_url[0][1]
    observed = None
    if found:
        keys = ("txid", "transaction_hash_matches", "outputs", "blockhash", "in_block", "block_hash_matches",
                "height", "blocktime")
        comparable = [{k: f.get(k) for k in keys} for _, f, _ in facts_by_url]
        check("endpoints_agree", all(c == comparable[0] for c in comparable[1:]), endpoints=len(facts_by_url))
        check("transaction_hash_matches", first["transaction_hash_matches"] is True)
        check("block_hash_matches", first["block_hash_matches"] is True)
        depths = [f.get("confirmations") or 0 for _, f, _ in facts_by_url]
        check("confirmed", all(isinstance(d, int) and d >= need for d in depths), required=need, observed=depths)
        observed = sum(o["value"] for o in first["outputs"] if o["address"] == recipient)
        check("recipient", observed > 0, expected=recipient,
              outputs_to_recipient=sum(1 for o in first["outputs"] if o["address"] == recipient))
        check("amount", observed >= min_amount, observed=str(observed), required=str(min_amount))
        if not_after_ms is not None:
            check("before_deadline", isinstance(first["blocktime"], int) and first["blocktime"] * 1000 <= not_after_ms,
                  blocktime=first["blocktime"], notAfterMs=not_after_ms)
        if after_block is not None:
            check("after_deposit", isinstance(first["height"], int) and first["height"] > after_block,
                  blockNumber=first["height"], depositBlock=after_block)
    return {
        "evidence_version": EVIDENCE_VERSION,
        "kind": "utxo_native_transfer",
        "chain": chain,
        "purpose": purpose,
        "verified": all(c["ok"] for c in checks),
        "transaction_hash": tx_id,
        "recipient": recipient,
        "required": str(min_amount),
        "observed": str(observed) if observed is not None else None,
        "blockNumber": first.get("height"),
        "blockTime": first.get("blocktime"),
        "commitment": f"{need} confirmations",
        "rpcs": [{"url": url, "result_sha256": digest} for url, _, digest in facts_by_url],
        "checks": checks,
    }


class UtxoRpc:
    """Confirmed native-coin payment evidence on one UTXO chain, from every configured endpoint."""

    def __init__(self, chain: str, transport: Transport, urls: Optional[Sequence[str]] = None, timeout: float = 20.0):
        if chain not in UTXO_MIN_CONFIRMATIONS:
            raise ChainEvidenceError("X3_EVIDENCE_UNSUPPORTED", f"no UTXO evidence for chain {chain!r}")
        self.chain = chain
        self.urls = rpc_urls(f"X3_UTXO_RPC_URLS_{chain.upper()}", (), urls)
        self.transport = transport
        self.timeout = timeout

    def verify_native_transfer(self, *, tx_hash: str, recipient: str, min_amount: int, not_after_ms: Optional[int],
                               purpose: str, after_block: Optional[int] = None) -> Dict[str, Any]:
        reg = _address_registry()
        if reg.utxo_address_kind(UTXO_ADDRESS_KIND[self.chain], recipient) is None:
            raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{recipient!r} is not a valid {self.chain} address")
        facts = []
        for url in self.urls:
            info = utxo_rpc(self.transport, url, "getblockchaininfo", [], self.timeout)
            tx = utxo_rpc(self.transport, url, "getrawtransaction", [tx_hash, 1], self.timeout)
            header = None
            if isinstance(tx, dict) and tx.get("blockhash"):
                header = utxo_rpc(self.transport, url, "getblockheader", [tx["blockhash"]], self.timeout)
            raw = {"info": info, "transaction": tx, "header": header}
            facts.append((url, utxo_transfer_facts(self.chain, info, tx, header, tx_id=tx_hash),
                          hashlib.sha256(json.dumps(raw, sort_keys=True, default=str).encode()).hexdigest()))
        return judge_utxo_transfer(facts, chain=self.chain, tx_id=tx_hash, recipient=recipient, min_amount=min_amount,
                                   not_after_ms=not_after_ms, purpose=purpose, after_block=after_block)

    def verify_token_transfer(self, **kw: Any) -> Dict[str, Any]:
        raise ChainEvidenceError("X3_EVIDENCE_UNSUPPORTED", f"{self.chain} has no tokens this adapter routes")


# --------------------------------------------------------------------------------------------
# XRP Ledger (native XRP)
# --------------------------------------------------------------------------------------------
#
# rippled JSON-RPC `tx`. A payment counts only when the ledger that holds it is **validated**,
# the result is `tesSUCCESS`, it is a `Payment` to the address, and — read from
# `meta.delivered_amount`, never `Amount` — at least the required drops actually arrived: a
# partial payment's `Amount` overstates what was delivered. When the signed quote carries a
# deposit memo, the payment's `DestinationTag` must equal it. `server_info` must not report a
# test network ID. Endpoints come from `X3_XRP_RPC_URLS`; there are no defaults.

RIPPLE_EPOCH = 946684800  # 2000-01-01T00:00:00Z


def xrp_rpc(transport: Transport, url: str, method: str, params: Dict[str, Any], timeout: float = 20.0) -> Dict[str, Any]:
    body = json.dumps({"method": method, "params": [params]}).encode()
    status, raw = transport("POST", url, {"Content-Type": "application/json", "Accept": "application/json",
                                          "User-Agent": "x3-lang-evidence/1"}, body, timeout)
    try:
        data = json.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, ValueError):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: HTTP {status} with no JSON body")
    result = data.get("result") if isinstance(data, dict) else None
    if status >= 400 or not isinstance(result, dict):
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: HTTP {status}")
    if result.get("status") == "error" and result.get("error") != "txnNotFound":
        raise ChainEvidenceError("X3_EVIDENCE_RPC", f"{url} {method}: {result.get('error')}")
    return result


def xrp_payment_facts(info: Dict[str, Any], tx: Dict[str, Any], *, tx_id: str) -> Dict[str, Any]:
    network_id = (info.get("info") or {}).get("network_id")
    facts: Dict[str, Any] = {"network_id": network_id}
    if tx.get("error") == "txnNotFound" or not tx.get("hash"):
        return {**facts, "found": False}
    meta = tx.get("meta") or {}
    delivered = meta.get("delivered_amount")
    # Native XRP is a string of drops; an issued currency is an object and is not XRP.
    drops = int(delivered) if isinstance(delivered, str) and delivered.isdigit() else 0
    tag = tx.get("DestinationTag")
    return {
        **facts,
        "found": True,
        "hash": str(tx.get("hash")).upper(),
        "transaction_hash_matches": str(tx.get("hash")).upper() == tx_id.upper(),
        "validated": tx.get("validated") is True,
        "succeeded": meta.get("TransactionResult") == "tesSUCCESS",
        "is_payment": tx.get("TransactionType") == "Payment",
        "destination": tx.get("Destination"),
        "destination_tag": tag if type(tag) is int else None,
        "delivered_drops": drops,
        "ledger_index": tx.get("ledger_index"),
        "close_time_ms": (int(tx["date"]) + RIPPLE_EPOCH) * 1000 if type(tx.get("date")) is int else None,
    }


def judge_xrp_payment(facts_by_url: List[Tuple[str, Dict[str, Any], Optional[str]]], *, tx_id: str, recipient: str,
                      min_amount: int, not_after_ms: Optional[int], purpose: str,
                      destination_tag: Optional[int] = None, after_block: Optional[int] = None) -> Dict[str, Any]:
    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: bool, **detail: Any) -> None:
        checks.append({"check": name, "ok": bool(ok), **detail})

    if not facts_by_url:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", "no XRP Ledger endpoints answered")
    check("mainnet", all(f.get("network_id") in (None, 0) for _, f, _ in facts_by_url),
          network_ids=[f.get("network_id") for _, f, _ in facts_by_url])
    found = all(f.get("found") for _, f, _ in facts_by_url)
    check("found", found)
    first = facts_by_url[0][1]
    observed = None
    if found:
        keys = ("hash", "transaction_hash_matches", "validated", "succeeded", "is_payment", "destination",
                "destination_tag", "delivered_drops", "ledger_index", "close_time_ms")
        comparable = [{k: f.get(k) for k in keys} for _, f, _ in facts_by_url]
        check("endpoints_agree", all(c == comparable[0] for c in comparable[1:]), endpoints=len(facts_by_url))
        check("transaction_hash_matches", first["transaction_hash_matches"] is True)
        check("validated", first["validated"] is True)
        check("succeeded", first["succeeded"] is True)
        check("is_payment", first["is_payment"] is True)
        check("recipient", first["destination"] == recipient, expected=recipient, observed=first["destination"])
        if destination_tag is not None:
            check("destination_tag", first["destination_tag"] == destination_tag, expected=destination_tag,
                  observed=first["destination_tag"])
        observed = first["delivered_drops"]
        check("amount", observed >= min_amount, observed=str(observed), required=str(min_amount),
              read_from="meta.delivered_amount")
        if not_after_ms is not None:
            check("before_deadline", isinstance(first["close_time_ms"], int) and first["close_time_ms"] <= not_after_ms,
                  closeTimeMs=first["close_time_ms"], notAfterMs=not_after_ms)
        if after_block is not None:
            check("after_deposit", isinstance(first["ledger_index"], int) and first["ledger_index"] > after_block,
                  blockNumber=first["ledger_index"], depositBlock=after_block)
    return {
        "evidence_version": EVIDENCE_VERSION, "kind": "xrp_payment", "chain": "xrp", "purpose": purpose,
        "verified": all(c["ok"] for c in checks), "transaction_hash": tx_id, "recipient": recipient,
        "destination_tag": destination_tag, "required": str(min_amount),
        "observed": str(observed) if observed is not None else None,
        "blockNumber": first.get("ledger_index"), "commitment": "validated",
        "rpcs": [{"url": url, "result_sha256": digest} for url, _, digest in facts_by_url], "checks": checks,
    }


class XrpRpc:
    def __init__(self, transport: Transport, urls: Optional[Sequence[str]] = None, timeout: float = 20.0):
        self.urls = rpc_urls("X3_XRP_RPC_URLS", (), urls)
        self.transport = transport
        self.timeout = timeout

    def verify_native_transfer(self, *, tx_hash: str, recipient: str, min_amount: int, not_after_ms: Optional[int],
                               purpose: str, after_block: Optional[int] = None,
                               destination_tag: Optional[int] = None) -> Dict[str, Any]:
        facts = []
        for url in self.urls:
            info = xrp_rpc(self.transport, url, "server_info", {}, self.timeout)
            tx = xrp_rpc(self.transport, url, "tx", {"transaction": tx_hash, "binary": False}, self.timeout)
            facts.append((url, xrp_payment_facts(info, tx, tx_id=tx_hash), sha256_json({"info": info, "tx": tx})))
        return judge_xrp_payment(facts, tx_id=tx_hash, recipient=recipient, min_amount=min_amount,
                                 not_after_ms=not_after_ms, purpose=purpose, destination_tag=destination_tag,
                                 after_block=after_block)

    def verify_token_transfer(self, **kw: Any) -> Dict[str, Any]:
        raise ChainEvidenceError("X3_EVIDENCE_UNSUPPORTED", "XRP Ledger issued currencies are not routed here")


# --------------------------------------------------------------------------------------------
# Sui (SUI and coins)
# --------------------------------------------------------------------------------------------
#
# `sui_getTransactionBlock` with effects and balance changes. Sui transactions are final once
# they are in a checkpoint, so a transaction with no checkpoint is not yet evidence. The payment
# is the recipient's positive balance change in the coin type; the endpoint must be Sui mainnet
# (`sui_getChainIdentifier` == "35834a8a"). Endpoints come from `X3_SUI_RPC_URLS`; no defaults.

SUI_MAINNET_CHAIN_ID = "35834a8a"
SUI_NATIVE_COIN = "0x2::sui::SUI"


def _sui_address(value: Any) -> Optional[str]:
    text = str(value or "").lower()
    if not re.fullmatch(r"0x[0-9a-f]{1,64}", text):
        return None
    return "0x" + text[2:].rjust(64, "0")


def sui_coin_type(value: Any) -> Optional[str]:
    """`<address>::<module>::<name>` with the address in full 64-hex form, for comparison."""
    parts = str(value or "").split("::")
    if len(parts) != 3 or _sui_address(parts[0]) is None:
        return None
    return "::".join([_sui_address(parts[0]), parts[1], parts[2]])


def sui_transfer_facts(chain_id: Any, tx: Any, *, tx_id: str, coin_type: str) -> Dict[str, Any]:
    facts: Dict[str, Any] = {"chain_id": chain_id}
    if not isinstance(tx, dict) or not tx.get("digest"):
        return {**facts, "found": False}
    want = sui_coin_type(coin_type)
    credits: Dict[str, int] = {}
    for change in tx.get("balanceChanges") or []:
        owner = _sui_address(((change or {}).get("owner") or {}).get("AddressOwner"))
        if owner is None or sui_coin_type(change.get("coinType")) != want:
            continue
        amount = str(change.get("amount", ""))
        if not re.fullmatch(r"-?\d+", amount):
            raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"Sui balance change amount is not an integer: {amount!r}")
        credits[owner] = credits.get(owner, 0) + int(amount)
    checkpoint = tx.get("checkpoint")
    return {
        **facts,
        "found": True,
        "digest": tx.get("digest"),
        "transaction_hash_matches": tx.get("digest") == tx_id,
        "succeeded": (((tx.get("effects") or {}).get("status")) or {}).get("status") == "success",
        "checkpoint": int(checkpoint) if isinstance(checkpoint, str) and checkpoint.isdigit() else None,
        "timestamp_ms": int(tx["timestampMs"]) if str(tx.get("timestampMs", "")).isdigit() else None,
        "credits": sorted(credits.items()),
    }


def judge_sui_transfer(facts_by_url: List[Tuple[str, Dict[str, Any], Optional[str]]], *, tx_id: str, recipient: str,
                       coin_type: str, min_amount: int, not_after_ms: Optional[int], purpose: str,
                       after_block: Optional[int] = None) -> Dict[str, Any]:
    recipient = _sui_address(recipient) or recipient
    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: bool, **detail: Any) -> None:
        checks.append({"check": name, "ok": bool(ok), **detail})

    if not facts_by_url:
        raise ChainEvidenceError("X3_EVIDENCE_RPC", "no Sui endpoints answered")
    check("mainnet", all(f.get("chain_id") == SUI_MAINNET_CHAIN_ID for _, f, _ in facts_by_url),
          chain_ids=[f.get("chain_id") for _, f, _ in facts_by_url])
    found = all(f.get("found") for _, f, _ in facts_by_url)
    check("found", found)
    first = facts_by_url[0][1]
    observed = None
    if found:
        keys = ("digest", "transaction_hash_matches", "succeeded", "checkpoint", "timestamp_ms", "credits")
        comparable = [{k: f.get(k) for k in keys} for _, f, _ in facts_by_url]
        check("endpoints_agree", all(c == comparable[0] for c in comparable[1:]), endpoints=len(facts_by_url))
        check("transaction_hash_matches", first["transaction_hash_matches"] is True)
        check("finalized", isinstance(first["checkpoint"], int), checkpoint=first["checkpoint"])
        check("succeeded", first["succeeded"] is True)
        observed = dict(first["credits"]).get(recipient, 0)
        check("recipient", observed > 0, expected=recipient, coin_type=sui_coin_type(coin_type))
        check("amount", observed >= min_amount, observed=str(observed), required=str(min_amount))
        if not_after_ms is not None:
            check("before_deadline", isinstance(first["timestamp_ms"], int) and first["timestamp_ms"] <= not_after_ms,
                  timestampMs=first["timestamp_ms"], notAfterMs=not_after_ms)
        if after_block is not None:
            check("after_deposit", isinstance(first["checkpoint"], int) and first["checkpoint"] > after_block,
                  blockNumber=first["checkpoint"], depositBlock=after_block)
    return {
        "evidence_version": EVIDENCE_VERSION,
        "kind": "sui_native_transfer" if sui_coin_type(coin_type) == sui_coin_type(SUI_NATIVE_COIN) else "sui_coin_transfer",
        "chain": "sui", "purpose": purpose, "verified": all(c["ok"] for c in checks), "transaction_hash": tx_id,
        "token": None if sui_coin_type(coin_type) == sui_coin_type(SUI_NATIVE_COIN) else coin_type,
        "recipient": recipient, "required": str(min_amount),
        "observed": str(observed) if observed is not None else None,
        "blockNumber": first.get("checkpoint"), "commitment": "checkpointed",
        "rpcs": [{"url": url, "result_sha256": digest} for url, _, digest in facts_by_url], "checks": checks,
    }


class SuiRpc:
    def __init__(self, transport: Transport, urls: Optional[Sequence[str]] = None, timeout: float = 20.0):
        self.urls = rpc_urls("X3_SUI_RPC_URLS", (), urls)
        self.transport = transport
        self.timeout = timeout

    def _verify(self, *, tx_hash: str, recipient: str, coin_type: str, min_amount: int, not_after_ms: Optional[int],
                purpose: str, after_block: Optional[int]) -> Dict[str, Any]:
        facts = []
        for url in self.urls:
            chain_id = json_rpc(self.transport, url, "sui_getChainIdentifier", [], self.timeout)
            tx = json_rpc(self.transport, url, "sui_getTransactionBlock",
                          [tx_hash, {"showEffects": True, "showBalanceChanges": True}], self.timeout)
            facts.append((url, sui_transfer_facts(chain_id, tx, tx_id=tx_hash, coin_type=coin_type),
                          sha256_json({"chain_id": chain_id, "tx": tx})))
        return judge_sui_transfer(facts, tx_id=tx_hash, recipient=recipient, coin_type=coin_type,
                                  min_amount=min_amount, not_after_ms=not_after_ms, purpose=purpose,
                                  after_block=after_block)

    def verify_native_transfer(self, *, tx_hash: str, recipient: str, min_amount: int, not_after_ms: Optional[int],
                               purpose: str, after_block: Optional[int] = None) -> Dict[str, Any]:
        return self._verify(tx_hash=tx_hash, recipient=recipient, coin_type=SUI_NATIVE_COIN, min_amount=min_amount,
                            not_after_ms=not_after_ms, purpose=purpose, after_block=after_block)

    def verify_token_transfer(self, *, tx_hash: str, token: str, recipient: str, min_amount: int,
                              not_after_ms: Optional[int], purpose: str,
                              after_block: Optional[int] = None) -> Dict[str, Any]:
        return self._verify(tx_hash=tx_hash, recipient=recipient, coin_type=token, min_amount=min_amount,
                            not_after_ms=not_after_ms, purpose=purpose, after_block=after_block)


# --------------------------------------------------------------------------------------------
# NEAR
# --------------------------------------------------------------------------------------------

def parse_ft_balance(result: Any) -> int:
    """`query` / `call_function` result for `ft_balance_of`: bytes of a JSON string of digits."""
    if not isinstance(result, dict) or not isinstance(result.get("result"), list):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"call_function result has no byte array: {result!r}")
    try:
        value = json.loads(bytes(result["result"]).decode("utf-8"))
    except (ValueError, UnicodeDecodeError, TypeError):
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", "ft_balance_of did not return JSON")
    if not isinstance(value, str) or not value.isdigit():
        raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"ft_balance_of returned {value!r}, not a U128 string")
    return int(value)


class NearViews:
    def __init__(self, transport: Transport, urls: Optional[Sequence[str]] = None, timeout: float = 20.0):
        if urls is None and not os.environ.get("NEAR_RPC_URLS") and os.environ.get("NEAR_RPC_URL"):
            urls = [os.environ["NEAR_RPC_URL"]]
        self.urls = rpc_urls("NEAR_RPC_URLS", DEFAULT_NEAR_RPC_URLS, urls)
        self.transport = transport
        self.timeout = timeout

    def _call(self, url: str, contract: str, method: str, args: Dict[str, Any], block: Dict[str, Any]) -> Dict[str, Any]:
        params = {
            "request_type": "call_function",
            "account_id": contract,
            "method_name": method,
            "args_base64": base64.b64encode(json.dumps(args).encode("utf-8")).decode("ascii"),
            **block,
        }
        result = json_rpc(self.transport, url, "query", params, self.timeout)
        if not isinstance(result, dict) or not isinstance(result.get("block_height"), int):
            raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{url} query result has no block_height")
        return result

    def view_json(self, contract: str, method: str, args: Dict[str, Any]) -> Dict[str, Any]:
        """A contract view's JSON result at one final block, identical on every endpoint."""
        first = self._call(self.urls[0], contract, method, args, {"finality": "final"})
        height = first["block_height"]
        reads = [(self.urls[0], first)]
        for url in self.urls[1:]:
            reads.append((url, self._call(url, contract, method, args, {"block_id": height})))

        def decode(result: Dict[str, Any]) -> Any:
            raw = result.get("result")
            if not isinstance(raw, list) or not all(isinstance(b, int) and 0 <= b < 256 for b in raw):
                raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{method} returned no result bytes")
            try:
                return json.loads(bytes(raw).decode("utf-8"))
            except (ValueError, UnicodeDecodeError):
                raise ChainEvidenceError("X3_EVIDENCE_FORMAT", f"{method} did not return JSON")

        values = [decode(r) for _, r in reads]
        if any(v != values[0] for v in values[1:]):
            raise ChainEvidenceError("X3_EVIDENCE_RPC_DISAGREE", f"endpoints disagree on {contract}.{method} at block {height}")
        return {"value": values[0], "block_height": height, "block_hash": first.get("block_hash"),
                "rpcs": [{"url": url, "result_sha256": sha256_json(r)} for url, r in reads]}

    def ft_balance(self, token: str, account: str) -> Dict[str, Any]:
        """Balance at one final block, read from every endpoint at that same block height."""
        first = self._call(self.urls[0], token, "ft_balance_of", {"account_id": account}, {"finality": "final"})
        height = first["block_height"]
        reads = [(self.urls[0], first)]
        for url in self.urls[1:]:
            reads.append((url, self._call(url, token, "ft_balance_of", {"account_id": account}, {"block_id": height})))
        balances = {parse_ft_balance(r) for _, r in reads}
        if len(balances) != 1:
            raise ChainEvidenceError("X3_EVIDENCE_RPC_DISAGREE", f"endpoints disagree on {account}'s {token} balance at block {height}",
                                     {"balances": sorted(str(b) for b in balances)})
        return {
            "token": token,
            "account": account,
            "balance": str(balances.pop()),
            "block_height": height,
            "block_hash": first.get("block_hash"),
            "rpcs": [{"url": url, "result_sha256": sha256_json(r)} for url, r in reads],
        }


def judge_near_delivery(baseline: Dict[str, Any], observed: Dict[str, Any], *, min_amount: int) -> Dict[str, Any]:
    checks: List[Dict[str, Any]] = []

    def check(name: str, ok: bool, **detail: Any) -> None:
        checks.append({"check": name, "ok": bool(ok), **detail})

    same_target = baseline["token"] == observed["token"] and baseline["account"] == observed["account"]
    check("same_account_and_token", same_target)
    check("observed_after_baseline", observed["block_height"] > baseline["block_height"],
          baseline_block=baseline["block_height"], observed_block=observed["block_height"])
    delivered = int(observed["balance"]) - int(baseline["balance"])
    check("amount", delivered >= min_amount, delivered=str(delivered), required=str(min_amount))
    return {
        "evidence_version": EVIDENCE_VERSION,
        "kind": "near_ft_balance_delta",
        "purpose": "settlement",
        "verified": all(c["ok"] for c in checks),
        "token": observed["token"],
        "account": observed["account"],
        "required": str(min_amount),
        "delivered": str(delivered),
        "baseline": baseline,
        "observed": observed,
        "checks": checks,
        "caveat": "a balance rise shows the token arrived, not who sent it",
    }
