"""Read-only NEAR liquidity discovery for X3Lang (issue #40).

Nothing in this package signs, deposits, broadcasts, reserves on-chain liquidity or
activates fees. It produces versioned `ProviderQuote` records whose every number traces to a
provider response or to agreed chain state, compares them only like for like, and returns a
quote as executable only when a pinned provider signature binds its terms to a deposit.
See `README.md` in this directory for the boundary and the provider support record.
"""
from .quote import (  # noqa: F401
    SCHEMA_VERSION, AssetRef, Commitment, FeeCharge, FeeComponent, LiquidityError, ProviderQuote, Provenance,
    SignatureStatus, SignatureVerification, checked_add, checked_sub, formatted_to_units, parse_uint, rescale,
    trusted_signers, unsigned_status,
)
from .ranking import (  # noqa: F401
    AggregationRequest, AggregationResult, Observation, ProviderRateLimiter, QuoteLedger, RankedQuote, Rejection,
    aggregate, collect_quotes, select_executable,
)
from .providers import PROVIDERS, ProviderSupport, require_adapter  # noqa: F401
