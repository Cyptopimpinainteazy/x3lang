"""Read-only NEAR contract views that several independent RPC operators must agree on.

One RPC answer is one operator's claim. This module pins one finalized block, asks every
configured endpoint for that same block and the same view at that block, and fails closed
unless all of them return the same block hash, height, timestamp and byte-identical result.
It also refuses a block older than the configured age: stale chain state is not current
liquidity.

Agreement among RPC operators is **not** a light-client proof; it narrows, but does not
remove, trust in the operators. Every endpoint must be HTTPS on a distinct host, and
redirects are refused (`near_intents.urllib_transport`). Optional per-endpoint request
headers (e.g. an operator API key) are sent but never recorded in evidence.

Request shapes follow the NEAR RPC docs: `block` with `finality: "final"` or `block_id`
(https://docs.near.org/api/rpc/block-chunk.md) and `query` with
`request_type: "call_function"`, `account_id`, `method_name`, `args_base64`, whose result is
`{result: [bytes], logs, block_height, block_hash}`
(https://docs.near.org/api/rpc/contracts.md,
https://docs.near.org/api-reference/call-view-function.md).
"""
from __future__ import annotations

import base64
import json
import re
import urllib.parse
from dataclasses import dataclass
from typing import Any, Dict, List, Mapping, Sequence, Tuple

import near_intents as ni

from .quote import LiquidityError, canonical_json, sha256_hex

_ACCOUNT_RE = re.compile(r"(?=.{2,64}$)(([a-z\d]+[-_])*[a-z\d]+\.)*([a-z\d]+[-_])*[a-z\d]+")
_B58_HASH_RE = re.compile(r"[1-9A-HJ-NP-Za-km-z]{32,44}")


def valid_account_id(value: Any) -> bool:
    return isinstance(value, str) and bool(_ACCOUNT_RE.fullmatch(value))


@dataclass(frozen=True)
class RpcEndpoint:
    url: str
    headers: Tuple[Tuple[str, str], ...] = ()

    def __post_init__(self) -> None:
        parsed = urllib.parse.urlsplit(self.url) if isinstance(self.url, str) else None
        if parsed is None or parsed.scheme != "https" or not parsed.hostname or parsed.username or parsed.password:
            raise LiquidityError("X3_LIQ_RPC_CONFIG", f"RPC endpoint must be an https URL without credentials: {self.url!r}")

    @property
    def host(self) -> str:
        return urllib.parse.urlsplit(self.url).hostname or ""


@dataclass(frozen=True)
class BlockRef:
    height: int
    hash: str
    timestamp_ns: int

    @property
    def timestamp_s(self) -> int:
        return self.timestamp_ns // 1_000_000_000


@dataclass(frozen=True)
class ViewResult:
    value: Any
    raw_sha256: str
    block: BlockRef
    endpoints: Tuple[str, ...]


class AgreedNearView:
    def __init__(self, endpoints: Sequence[RpcEndpoint], *, transport: "ni.Transport" = ni.urllib_transport,
                 min_endpoints: int = 2, max_block_age_s: int = 30, max_clock_skew_s: int = 5,
                 timeout: float = 15.0):
        endpoints = tuple(endpoints)
        if type(min_endpoints) is not int or min_endpoints < 2:
            raise LiquidityError("X3_LIQ_RPC_CONFIG", "agreement needs at least two endpoints")
        if len(endpoints) < min_endpoints:
            raise LiquidityError("X3_LIQ_RPC_CONFIG", f"{len(endpoints)} endpoints configured; {min_endpoints} required")
        if not all(isinstance(e, RpcEndpoint) for e in endpoints):
            raise LiquidityError("X3_LIQ_RPC_CONFIG", "endpoints must be RpcEndpoint")
        hosts = [e.host for e in endpoints]
        if len(set(hosts)) != len(hosts):
            raise LiquidityError("X3_LIQ_RPC_CONFIG", "endpoints must be on distinct hosts to count as independent")
        if type(max_block_age_s) is not int or max_block_age_s <= 0:
            raise LiquidityError("X3_LIQ_RPC_CONFIG", "max_block_age_s must be a positive int")
        self.endpoints = endpoints
        self.transport = transport
        self.max_block_age_s = max_block_age_s
        self.max_clock_skew_s = max_clock_skew_s
        self.timeout = timeout

    @property
    def endpoint_label(self) -> str:
        return ",".join(e.url for e in self.endpoints)

    def _rpc(self, endpoint: RpcEndpoint, method: str, params: Dict[str, Any]) -> Dict[str, Any]:
        body = json.dumps({"jsonrpc": "2.0", "id": "x3-liquidity", "method": method, "params": params}).encode()
        headers = {"Content-Type": "application/json", "Accept": "application/json"}
        headers.update(dict(endpoint.headers))
        try:
            status, raw = self.transport("POST", endpoint.url, headers, body, self.timeout)
        except ni.NearIntentsError as exc:
            raise LiquidityError("X3_LIQ_RPC_UNAVAILABLE", f"{endpoint.host}: {exc.code}") from None
        if status == 429:
            raise LiquidityError("X3_LIQ_RATE_LIMITED", f"{endpoint.host} rate-limited the request", {"host": endpoint.host})
        try:
            data = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            raise LiquidityError("X3_LIQ_RPC_UNAVAILABLE", f"{endpoint.host} returned HTTP {status} without JSON") from None
        if not isinstance(data, dict):
            raise LiquidityError("X3_LIQ_RPC_UNAVAILABLE", f"{endpoint.host} returned a non-object")
        if "error" in data:
            error = data["error"]
            cause = error.get("cause", {}).get("name") if isinstance(error, dict) and isinstance(error.get("cause"), dict) else None
            raise LiquidityError("X3_LIQ_RPC_ERROR", f"{endpoint.host}: {cause or 'error'}", {"host": endpoint.host, "cause": cause})
        result = data.get("result")
        if status != 200 or not isinstance(result, dict):
            raise LiquidityError("X3_LIQ_RPC_UNAVAILABLE", f"{endpoint.host} returned HTTP {status} without a result")
        return result

    @staticmethod
    def _block_ref(result: Mapping[str, Any], host: str) -> BlockRef:
        header = result.get("header")
        if not isinstance(header, dict):
            raise LiquidityError("X3_LIQ_RPC_MALFORMED", f"{host}: block has no header")
        height, block_hash, timestamp = header.get("height"), header.get("hash"), header.get("timestamp")
        if type(height) is not int or height <= 0 or type(timestamp) is not int or timestamp <= 0 \
                or not isinstance(block_hash, str) or not _B58_HASH_RE.fullmatch(block_hash):
            raise LiquidityError("X3_LIQ_RPC_MALFORMED", f"{host}: block header height/hash/timestamp malformed")
        return BlockRef(height=height, hash=block_hash, timestamp_ns=timestamp)

    def final_block(self, *, now: int) -> BlockRef:
        """The first endpoint's final block, confirmed by every other endpoint, and fresh."""
        first = self._block_ref(self._rpc(self.endpoints[0], "block", {"finality": "final"}), self.endpoints[0].host)
        for endpoint in self.endpoints[1:]:
            other = self._block_ref(self._rpc(endpoint, "block", {"block_id": first.hash}), endpoint.host)
            if other != first:
                raise LiquidityError("X3_LIQ_RPC_DISAGREEMENT", f"{endpoint.host} disagrees about block {first.hash}",
                                     {"host": endpoint.host})
        age = now - first.timestamp_s
        if age > self.max_block_age_s:
            raise LiquidityError("X3_LIQ_STALE", f"final block {first.height} is {age}s old (max {self.max_block_age_s}s)",
                                 {"age_s": age})
        if age < -self.max_clock_skew_s:
            raise LiquidityError("X3_LIQ_STALE", f"final block {first.height} is dated {-age}s in the future")
        return first

    def view(self, block: BlockRef, contract: str, method: str, args: Mapping[str, Any]) -> ViewResult:
        """Call `contract.method(args)` at `block` on every endpoint; all bytes must match."""
        if not valid_account_id(contract):
            raise LiquidityError("X3_LIQ_MALFORMED", f"contract {contract!r} is not a NEAR account ID")
        params = {"request_type": "call_function", "block_id": block.hash, "account_id": contract,
                  "method_name": method, "args_base64": base64.b64encode(canonical_json(dict(args))).decode()}
        raw_values: List[bytes] = []
        for endpoint in self.endpoints:
            result = self._rpc(endpoint, "query", params)
            if result.get("block_hash") != block.hash or result.get("block_height") != block.height:
                raise LiquidityError("X3_LIQ_RPC_DISAGREEMENT", f"{endpoint.host} answered at a different block",
                                     {"host": endpoint.host})
            data = result.get("result")
            if not isinstance(data, list) or not all(type(b) is int and 0 <= b <= 255 for b in data):
                raise LiquidityError("X3_LIQ_RPC_MALFORMED", f"{endpoint.host}: call_function result is not bytes")
            raw_values.append(bytes(data))
        if any(raw != raw_values[0] for raw in raw_values[1:]):
            raise LiquidityError("X3_LIQ_RPC_DISAGREEMENT", f"endpoints returned different {method} results at block {block.height}")
        try:
            value = json.loads(raw_values[0].decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            raise LiquidityError("X3_LIQ_RPC_MALFORMED", f"{method} result is not JSON") from None
        return ViewResult(value=value, raw_sha256=sha256_hex(raw_values[0]), block=block,
                          endpoints=tuple(e.url for e in self.endpoints))


__all__ = ["RpcEndpoint", "BlockRef", "ViewResult", "AgreedNearView", "valid_account_id"]
