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
            if raw is None and isinstance(result.get("error"), str):
                raise ChainEvidenceError("X3_EVIDENCE_VIEW_FAILED", f"{contract}.{method} failed: {result['error']}",
                                         {"error": result["error"]})
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
