"""Read-only RHEA (formerly Ref Finance) pool discovery from agreed, fresh NEAR chain state.

Contract and methods, from official documentation (read 2026-11-13):

- `v2.ref-finance.near` is the "Exchange Contract (AMM)" in
  https://guide.ref.finance/developers/contracts, and the DEX contract used by
  https://docs.near.org/primitives/dex.md, which describes "Rhea Finance (formerly Ref
  Finance)".
- `get_pools(from_index: u64, limit: u64) -> Vec<PoolInfo>` and
  `get_return(pool_id: u64, token_in, amount_in: U128, token_out) -> U128` are documented at
  https://ref-finance.gitbook.io/ref-finance/smart-contracts/ref-exchange; argument shapes
  (`amount_in` as a string) at https://guide.ref.finance/developers-1/cli-trading.
- `PoolInfo` fields `pool_kind`, `token_account_ids`, `amounts`, `total_fee`,
  `shares_total_supply`, `amp` are from the docs.near.org example response.
- Token decimals come from each token's NEP-148 `ft_metadata`
  (https://github.com/near/NEPs/blob/master/neps/nep-0148.md).

Every number here is read at one pinned final block that all configured RPC endpoints
agree on (`chain.AgreedNearView`). There are **no default or synthetic reserves**: an
empty, malformed, truncated or stale answer is a refusal. A RHEA result is an observation
of shared AMM state, not a reservation or a signed commitment, so it is always
`executable=False`. The expected output is the contract's own `get_return`, so the price
and the pool fee inside it trace to chain state rather than to local AMM math.

The unit of `total_fee` is not stated in the pages above; it is recorded verbatim as
`provider_value` and never used in arithmetic.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Any, List, Tuple

from .chain import AgreedNearView, BlockRef, valid_account_id
from .quote import (
    SCHEMA_VERSION, U128_MAX, AssetRef, Commitment, FeeCharge, FeeComponent, LiquidityError, ProviderQuote,
    Provenance, canonical_json, parse_uint, quote_terms_sha256, sha256_hex, unsigned_status,
)

PROVIDER_ID = "rhea-v2"
RHEA_EXCHANGE_CONTRACT = "v2.ref-finance.near"
NETWORK = "near"
MAX_PAGE = 500


@dataclass(frozen=True)
class RheaPool:
    pool_id: int
    pool_kind: str
    token_account_ids: Tuple[str, ...]
    amounts: Tuple[int, ...]
    total_fee: int
    shares_total_supply: int
    amp: int

    def reserve_of(self, token: str) -> int:
        return self.amounts[self.token_account_ids.index(token)]


def parse_pool(raw: Any, pool_id: int) -> RheaPool:
    """Strictly validate one `PoolInfo`. Unknown shapes are refused, not defaulted."""
    if not isinstance(raw, dict):
        raise LiquidityError("X3_LIQ_MALFORMED", f"pool {pool_id} is not an object")
    required = {"pool_kind", "token_account_ids", "amounts", "total_fee", "shares_total_supply", "amp"}
    missing = sorted(required - raw.keys())
    if missing:
        raise LiquidityError("X3_LIQ_MALFORMED", f"pool {pool_id} lacks {', '.join(missing)}")
    kind, tokens, amounts = raw["pool_kind"], raw["token_account_ids"], raw["amounts"]
    if not isinstance(kind, str) or not kind or not kind.replace("_", "").isalnum():
        raise LiquidityError("X3_LIQ_MALFORMED", f"pool {pool_id} has a malformed pool_kind")
    if not isinstance(tokens, list) or len(tokens) < 2 or not all(valid_account_id(t) for t in tokens):
        raise LiquidityError("X3_LIQ_MALFORMED", f"pool {pool_id} token_account_ids are not NEAR account IDs")
    if len(set(tokens)) != len(tokens):
        raise LiquidityError("X3_LIQ_DUPLICATE_INVENTORY", f"pool {pool_id} lists a token twice")
    if not isinstance(amounts, list) or len(amounts) != len(tokens):
        raise LiquidityError("X3_LIQ_MALFORMED", f"pool {pool_id} amounts do not match its tokens")
    for amount in amounts:
        if not isinstance(amount, str):
            # U128 is serialized as a JSON string; a bare number may already have lost precision.
            raise LiquidityError("X3_LIQ_MALFORMED", f"pool {pool_id} reserve is not a U128 string")
    reserves = tuple(parse_uint(a, f"pool {pool_id} amount", maximum=U128_MAX) for a in amounts)
    for name in ("total_fee", "amp"):
        if type(raw[name]) is not int or raw[name] < 0:
            raise LiquidityError("X3_LIQ_MALFORMED", f"pool {pool_id} {name} is not a non-negative integer")
    shares = raw["shares_total_supply"]
    if not isinstance(shares, str):
        raise LiquidityError("X3_LIQ_MALFORMED", f"pool {pool_id} shares_total_supply is not a U128 string")
    return RheaPool(pool_id=pool_id, pool_kind=kind, token_account_ids=tuple(tokens), amounts=reserves,
                    total_fee=raw["total_fee"], shares_total_supply=parse_uint(shares, "shares", maximum=U128_MAX),
                    amp=raw["amp"])


@dataclass(frozen=True)
class PoolSnapshot:
    block: BlockRef
    pools: Tuple[RheaPool, ...]
    pages_sha256: Tuple[str, ...]
    endpoints: str


class RheaPoolDiscovery:
    def __init__(self, view: AgreedNearView, *, contract: str = RHEA_EXCHANGE_CONTRACT, page_size: int = 100,
                 max_pools: int = 10_000):
        if not isinstance(view, AgreedNearView):
            raise LiquidityError("X3_LIQ_RPC_CONFIG", "RHEA discovery needs an AgreedNearView")
        if not valid_account_id(contract):
            raise LiquidityError("X3_LIQ_RPC_CONFIG", "contract must be a NEAR account ID")
        if type(page_size) is not int or not 0 < page_size <= MAX_PAGE or type(max_pools) is not int or max_pools <= 0:
            raise LiquidityError("X3_LIQ_RPC_CONFIG", "invalid page_size or max_pools")
        self.view = view
        self.contract = contract
        self.page_size = page_size
        self.max_pools = max_pools

    def discover(self, *, now: int) -> PoolSnapshot:
        """Every pool at one fresh, agreed final block. A pool count above `max_pools` is a
        refusal, never a silent truncation."""
        block = self.view.final_block(now=now)
        pools: List[RheaPool] = []
        digests: List[str] = []
        index = 0
        while True:
            page = self.view.view(block, self.contract, "get_pools", {"from_index": index, "limit": self.page_size})
            if not isinstance(page.value, list) or len(page.value) > self.page_size:
                raise LiquidityError("X3_LIQ_MALFORMED", "get_pools did not return a bounded list")
            digests.append(page.raw_sha256)
            pools.extend(parse_pool(raw, index + offset) for offset, raw in enumerate(page.value))
            if len(pools) > self.max_pools:
                raise LiquidityError("X3_LIQ_TOO_MANY_POOLS", f"more than {self.max_pools} pools; refusing to truncate")
            if len(page.value) < self.page_size:
                break
            index += self.page_size
        return PoolSnapshot(block=block, pools=tuple(pools), pages_sha256=tuple(digests), endpoints=self.view.endpoint_label)

    def _decimals(self, block: BlockRef, token: str) -> Tuple[int, str]:
        meta = self.view.view(block, token, "ft_metadata", {})
        decimals = meta.value.get("decimals") if isinstance(meta.value, dict) else None
        if type(decimals) is not int or not 0 <= decimals <= 255:
            raise LiquidityError("X3_LIQ_DECIMALS", f"{token} ft_metadata has no valid decimals")
        return decimals, meta.raw_sha256

    def observe_quote(self, snapshot: PoolSnapshot, pool_id: int, token_in: str, token_out: str, amount_in: int, *,
                      now: int) -> ProviderQuote:
        """An observation-only quote from one pool at the snapshot's block.

        Uses the contract's own `get_return` at that block. Refuses a stale snapshot, a pool
        that does not hold both tokens, zero reserves, a zero result, and a result that
        would drain the pool's output reserve.
        """
        age = now - snapshot.block.timestamp_s
        if age > self.view.max_block_age_s:
            raise LiquidityError("X3_LIQ_STALE", f"pool snapshot is {age}s old (max {self.view.max_block_age_s}s)")
        matches = [p for p in snapshot.pools if p.pool_id == pool_id]
        if len(matches) != 1:
            raise LiquidityError("X3_LIQ_NO_LIQUIDITY", f"pool {pool_id} is not in the snapshot")
        pool = matches[0]
        if token_in == token_out or token_in not in pool.token_account_ids or token_out not in pool.token_account_ids:
            raise LiquidityError("X3_LIQ_ASSET_MISMATCH", f"pool {pool_id} does not trade {token_in} -> {token_out}")
        amount_in = parse_uint(amount_in, "amount_in", maximum=U128_MAX)
        if amount_in == 0:
            raise LiquidityError("X3_LIQ_MALFORMED", "amount_in must be positive")
        reserve_in, reserve_out = pool.reserve_of(token_in), pool.reserve_of(token_out)
        if reserve_in == 0 or reserve_out == 0:
            raise LiquidityError("X3_LIQ_NO_LIQUIDITY", f"pool {pool_id} has an empty reserve")

        args = {"pool_id": pool_id, "token_in": token_in, "amount_in": str(amount_in), "token_out": token_out}
        returned = self.view.view(snapshot.block, self.contract, "get_return", args)
        if not isinstance(returned.value, str):
            raise LiquidityError("X3_LIQ_MALFORMED", "get_return did not return a U128 string")
        amount_out = parse_uint(returned.value, "get_return", maximum=U128_MAX)
        if amount_out == 0:
            raise LiquidityError("X3_LIQ_NO_LIQUIDITY", f"pool {pool_id} returns nothing for {amount_in}")
        if amount_out >= reserve_out:
            raise LiquidityError("X3_LIQ_NO_LIQUIDITY", f"pool {pool_id} cannot pay {amount_out} from reserve {reserve_out}")

        decimals_in, meta_in = self._decimals(snapshot.block, token_in)
        decimals_out, meta_out = self._decimals(snapshot.block, token_out)
        asset_in = AssetRef(NETWORK, f"nep141:{token_in}", decimals_in)
        asset_out = AssetRef(NETWORK, f"nep141:{token_out}", decimals_out)

        observed = sha256_hex(canonical_json({"pages": list(snapshot.pages_sha256), "get_return": returned.raw_sha256,
                                              "ft_metadata": [meta_in, meta_out]}))
        provenance = Provenance(source="chain_state", endpoint=snapshot.endpoints, response_sha256=observed,
                                observed_at=now, reference=f"{self.contract}#pool{pool_id}@{snapshot.block.hash}",
                                block_height=snapshot.block.height)
        request_hash = sha256_hex(canonical_json({"contract": self.contract, "method": "get_return", "args": args,
                                                  "block_hash": snapshot.block.hash}))
        fees = (FeeComponent(kind="pool", charge=FeeCharge.INCLUDED_IN_QUOTE, asset=None, amount=None,
                             provider_value=str(pool.total_fee), source_field=f"get_pools[{pool_id}].total_fee",
                             authenticated=True),)
        issued_at = snapshot.block.timestamp_s
        expires_at = issued_at + self.view.max_block_age_s
        expiry_source = f"policy:max_block_age_s={self.view.max_block_age_s}"
        terms = quote_terms_sha256(provider_id=PROVIDER_ID, asset_in=asset_in, asset_out=asset_out, amount_in=amount_in,
                                   amount_out=amount_out, min_amount_out=0, issued_at=issued_at, expires_at=expires_at,
                                   expiry_source=expiry_source, commitment=Commitment.CHAIN_OBSERVATION, fees=fees,
                                   provenance=provenance, request_hash=request_hash)
        status = unsigned_status(PROVIDER_ID, observed, terms,
                                 f"chain state agreed by {len(self.view.endpoints)} RPC endpoints at block "
                                 f"{snapshot.block.hash}; not a light-client proof and not a provider signature")
        return ProviderQuote(
            schema_version=SCHEMA_VERSION, provider_id=PROVIDER_ID, asset_in=asset_in, asset_out=asset_out,
            amount_in=amount_in, amount_out=amount_out, min_amount_out=0, issued_at=issued_at, expires_at=expires_at,
            expiry_source=expiry_source, commitment=Commitment.CHAIN_OBSERVATION, fees=fees, provenance=provenance,
            request_hash=request_hash, signature=status, executable=False,
            non_executable_reason="observation of shared AMM reserves: no reservation, no signed commitment, "
                                  "no guaranteed output; executing would need a separately approved signed swap",
        )


__all__ = ["PROVIDER_ID", "RHEA_EXCHANGE_CONTRACT", "NETWORK", "RheaPool", "PoolSnapshot", "RheaPoolDiscovery",
           "parse_pool"]
