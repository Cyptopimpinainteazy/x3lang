"""Provider isolation, rate limits, like-for-like aggregation and net-output ranking.

Aggregation compares only quotes for the identical input asset, output asset (network,
asset ID and decimals), input quantity and freshness window. Everything else is rejected
with a reason, never coerced: there is no decimal rescaling between different assets, no
route or asset substitution and no partial fill across quotes.

Ranking is by **guaranteed** net output after every fee, and only quotes whose terms a
verified provider signature binds to a deposit can be ranked. Observation-only quotes (chain
reserves, dry or indicative quotes) are returned separately, labelled, ordered by their
expected net output, and can never be selected. `select_executable` re-checks expiry at
the moment of selection and claims a quote once, so a quote that expired after ranking, or
that was already used for another order, is refused.
"""
from __future__ import annotations

import threading
from dataclasses import dataclass
from typing import Any, Callable, Dict, Iterable, List, Mapping, Optional, Sequence, Tuple, Union

import near_intents as ni

from .quote import AssetRef, LiquidityError, ProviderQuote, parse_uint


@dataclass(frozen=True)
class Rejection:
    provider_id: str
    code: str
    reason: str
    reference: Optional[str] = None

    def to_json(self) -> Dict[str, Any]:
        return {"provider_id": self.provider_id, "code": self.code, "reason": self.reason, "reference": self.reference}


# ---------------------------------------------------------------------------------------
# Rate limiting and provider isolation
# ---------------------------------------------------------------------------------------

class ProviderRateLimiter:
    """A local per-provider token bucket. It keeps this client inside a provider's
    published (or operator-chosen) request budget; it is not the provider's own limit, which
    is still observed through HTTP 429."""

    def __init__(self, limits: Mapping[str, Tuple[int, int]], *, clock: Callable[[], float]):
        self._limits: Dict[str, Tuple[int, int]] = {}
        for provider, (capacity, per_seconds) in limits.items():
            if type(capacity) is not int or capacity <= 0 or type(per_seconds) is not int or per_seconds <= 0:
                raise LiquidityError("X3_LIQ_CONFIG", f"invalid rate limit for {provider}")
            self._limits[provider] = (capacity, per_seconds)
        self._clock = clock
        self._state: Dict[str, Tuple[float, float]] = {}
        self._lock = threading.Lock()

    def acquire(self, provider_id: str) -> None:
        if provider_id not in self._limits:
            raise LiquidityError("X3_LIQ_RATE_LIMITED", f"no request budget is configured for {provider_id}")
        capacity, per_seconds = self._limits[provider_id]
        with self._lock:
            now = self._clock()
            tokens, last = self._state.get(provider_id, (float(capacity), now))
            tokens = min(float(capacity), tokens + (now - last) * capacity / per_seconds)
            if tokens < 1.0:
                self._state[provider_id] = (tokens, now)
                raise LiquidityError("X3_LIQ_RATE_LIMITED",
                                     f"local budget for {provider_id} ({capacity}/{per_seconds}s) is exhausted")
            self._state[provider_id] = (tokens - 1.0, now)


QuoteSource = Callable[[], Union[ProviderQuote, Sequence[ProviderQuote]]]


def _classify(exc: Exception) -> Tuple[str, str]:
    if isinstance(exc, LiquidityError):
        return exc.code, exc.message
    if isinstance(exc, ni.NearIntentsError):
        status = exc.detail.get("status") if isinstance(exc.detail, dict) else None
        if exc.code == "X3_NEAR_PROVIDER_MINIMUM":
            return "X3_LIQ_PROVIDER_MINIMUM", exc.message
        if status == 429:
            return "X3_LIQ_RATE_LIMITED", exc.message
        return "X3_LIQ_PROVIDER_ERROR", f"{exc.code}: {exc.message}"
    return "X3_LIQ_PROVIDER_ERROR", f"{type(exc).__name__} from provider adapter"


def collect_quotes(sources: Mapping[str, QuoteSource], *, limiter: ProviderRateLimiter
                   ) -> Tuple[List[ProviderQuote], List[Rejection]]:
    """Call each provider source under its rate budget. One provider's failure (including a
    rate limit or provider minimum) is recorded for that provider only; it never stops the
    others and is never replaced by another provider's answer."""
    quotes: List[ProviderQuote] = []
    rejections: List[Rejection] = []
    for provider_id, source in sources.items():
        try:
            limiter.acquire(provider_id)
            produced = source()
        except Exception as exc:  # isolate every provider-specific failure
            code, reason = _classify(exc)
            rejections.append(Rejection(provider_id, code, reason))
            continue
        items = [produced] if isinstance(produced, ProviderQuote) else list(produced)
        for item in items:
            if not isinstance(item, ProviderQuote):
                rejections.append(Rejection(provider_id, "X3_LIQ_PROVIDER_ERROR", "source returned a non-quote"))
            elif item.provider_id != provider_id:
                rejections.append(Rejection(provider_id, "X3_LIQ_PROVIDER_MISMATCH",
                                            f"source for {provider_id} returned a {item.provider_id} quote",
                                            item.provenance.reference))
            else:
                quotes.append(item)
    return quotes, rejections


# ---------------------------------------------------------------------------------------
# Aggregation
# ---------------------------------------------------------------------------------------

@dataclass(frozen=True)
class AggregationRequest:
    asset_in: AssetRef
    asset_out: AssetRef
    amount_in: int
    now: int
    max_age_s: int
    #: Minimum remaining life a quote must have to be ranked: time to act before it expires.
    min_remaining_s: int
    max_clock_skew_s: int = 5

    def __post_init__(self) -> None:
        if not isinstance(self.asset_in, AssetRef) or not isinstance(self.asset_out, AssetRef):
            raise LiquidityError("X3_LIQ_MALFORMED", "request assets must be AssetRef")
        if parse_uint(self.amount_in, "amount_in") == 0:
            raise LiquidityError("X3_LIQ_MALFORMED", "amount_in must be positive")
        for name in ("now", "max_age_s", "min_remaining_s", "max_clock_skew_s"):
            if type(getattr(self, name)) is not int or getattr(self, name) < 0:
                raise LiquidityError("X3_LIQ_MALFORMED", f"{name} must be a non-negative int")
        if self.max_age_s == 0:
            raise LiquidityError("X3_LIQ_MALFORMED", "max_age_s must be positive")


@dataclass(frozen=True)
class RankedQuote:
    quote: ProviderQuote
    guaranteed_net_output: int


@dataclass(frozen=True)
class Observation:
    quote: ProviderQuote
    expected_net_output: Optional[int]
    label: str = "observation-only; not executable"


@dataclass(frozen=True)
class AggregationResult:
    request: AggregationRequest
    ranked: Tuple[RankedQuote, ...]
    observations: Tuple[Observation, ...]
    rejected: Tuple[Rejection, ...]

    @property
    def best_executable(self) -> Optional[RankedQuote]:
        return self.ranked[0] if self.ranked else None

    def to_json(self) -> Dict[str, Any]:
        return {
            "ranked": [{"guaranteed_net_output": str(r.guaranteed_net_output), "quote": r.quote.to_json()} for r in self.ranked],
            "observations": [{"label": o.label, "expected_net_output": None if o.expected_net_output is None
                              else str(o.expected_net_output), "quote": o.quote.to_json()} for o in self.observations],
            "rejected": [r.to_json() for r in self.rejected],
        }


def _mismatch(request: AggregationRequest, quote: ProviderQuote) -> Optional[Tuple[str, str]]:
    for side, want, got in (("input", request.asset_in, quote.asset_in), ("output", request.asset_out, quote.asset_out)):
        if got.network != want.network:
            return "X3_LIQ_NETWORK_MISMATCH", f"{side} network {got.network} != {want.network}"
        if got.asset_id != want.asset_id:
            return "X3_LIQ_ASSET_MISMATCH", f"{side} asset {got.asset_id} != {want.asset_id}"
        if got.decimals != want.decimals:
            return "X3_LIQ_DECIMALS_MISMATCH", f"{side} decimals {got.decimals} != {want.decimals} for the same asset"
    if quote.amount_in != request.amount_in:
        return "X3_LIQ_AMOUNT_MISMATCH", f"amount_in {quote.amount_in} != {request.amount_in}"
    return None


def _freshness(request: AggregationRequest, quote: ProviderQuote) -> Optional[Tuple[str, str]]:
    age = request.now - quote.issued_at
    if age > request.max_age_s:
        return "X3_LIQ_STALE", f"issued {age}s ago (max {request.max_age_s}s)"
    if age < -request.max_clock_skew_s:
        return "X3_LIQ_STALE", f"issued {-age}s in the future"
    remaining = quote.expires_at - request.now
    if remaining <= 0:
        return "X3_LIQ_EXPIRED", f"expired {-remaining}s ago"
    if remaining < request.min_remaining_s:
        return "X3_LIQ_EXPIRY_RACE", f"only {remaining}s left; {request.min_remaining_s}s needed to act"
    return None


def aggregate(request: AggregationRequest, quotes: Iterable[ProviderQuote],
              prior_rejections: Iterable[Rejection] = ()) -> AggregationResult:
    quotes = list(quotes)
    rejected: List[Rejection] = list(prior_rejections)

    # The same inventory reported twice (same provider and same reference) is ambiguous:
    # counting it once would still trust an answer that contradicts itself. Reject every copy.
    seen: Dict[Tuple[str, str], int] = {}
    for q in quotes:
        if not isinstance(q, ProviderQuote):
            raise LiquidityError("X3_LIQ_MALFORMED", "aggregate accepts ProviderQuote values only")
        key = (q.provider_id, q.provenance.reference)
        seen[key] = seen.get(key, 0) + 1

    ranked: List[RankedQuote] = []
    observations: List[Observation] = []
    for q in quotes:
        ref = q.provenance.reference
        if seen[(q.provider_id, ref)] > 1:
            rejected.append(Rejection(q.provider_id, "X3_LIQ_DUPLICATE_INVENTORY", "same inventory reported more than once", ref))
            continue
        problem = _mismatch(request, q) or _freshness(request, q)
        if problem:
            rejected.append(Rejection(q.provider_id, problem[0], problem[1], ref))
            continue
        if not q.executable:
            try:
                expected = q.expected_net_output()
            except LiquidityError as exc:
                rejected.append(Rejection(q.provider_id, exc.code, exc.message, ref))
                continue
            observations.append(Observation(q, expected))
            continue
        try:
            net = q.guaranteed_net_output()
        except LiquidityError as exc:
            rejected.append(Rejection(q.provider_id, exc.code, exc.message, ref))
            continue
        if net is None or net <= 0:
            rejected.append(Rejection(q.provider_id, "X3_LIQ_NOT_EXECUTABLE", "no positive guaranteed net output", ref))
            continue
        ranked.append(RankedQuote(q, net))

    ranked.sort(key=lambda r: (-r.guaranteed_net_output, r.quote.provider_id, r.quote.terms_sha256))
    observations.sort(key=lambda o: (o.expected_net_output is None, -(o.expected_net_output or 0),
                                     o.quote.provider_id, o.quote.terms_sha256))
    return AggregationResult(request, tuple(ranked), tuple(observations), tuple(rejected))


class QuoteLedger:
    """Single-use claims: one signed quote backs at most one order in this process."""

    def __init__(self) -> None:
        self._claimed: set = set()
        self._lock = threading.Lock()

    def claim(self, quote: ProviderQuote) -> None:
        key = (quote.provider_id, quote.terms_sha256)
        with self._lock:
            if key in self._claimed:
                raise LiquidityError("X3_LIQ_OVERBOOKED", f"{quote.provider_id} quote already claimed by another order")
            self._claimed.add(key)


def select_executable(result: AggregationResult, *, now: int, ledger: QuoteLedger) -> ProviderQuote:
    """The best ranked quote, re-checked at selection time. Never an observation."""
    if not result.ranked:
        raise LiquidityError("X3_LIQ_NO_LIQUIDITY", "no verified executable liquidity for this exact request",
                             {"rejected": [r.to_json() for r in result.rejected],
                              "observations": len(result.observations)})
    best = result.ranked[0].quote
    if not best.executable:
        raise LiquidityError("X3_LIQ_NOT_EXECUTABLE", "ranked quote is not executable")
    remaining = best.expires_at - now
    if remaining < result.request.min_remaining_s:
        raise LiquidityError("X3_LIQ_EXPIRY_RACE",
                             f"best quote has {remaining}s left at selection; {result.request.min_remaining_s}s needed")
    ledger.claim(best)
    return best


__all__ = ["Rejection", "ProviderRateLimiter", "collect_quotes", "AggregationRequest", "RankedQuote", "Observation",
           "AggregationResult", "aggregate", "QuoteLedger", "select_executable"]
