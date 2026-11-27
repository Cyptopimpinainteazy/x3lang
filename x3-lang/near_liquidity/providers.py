"""Which liquidity providers have a NEAR adapter here, and why the others do not.

An adapter is added only after the provider's current NEAR support, authentication and
quote contract are confirmed in its official documentation. A provider that is not
confirmed has no adapter: `require_adapter` refuses it by name instead of guessing an API.
"""
from __future__ import annotations

from dataclasses import dataclass
from typing import Dict, Tuple

from .quote import LiquidityError


@dataclass(frozen=True)
class ProviderSupport:
    provider_id: str
    status: str  # "adapter" | "blocked"
    checked_on: str
    sources: Tuple[str, ...]
    summary: str


PROVIDERS: Dict[str, ProviderSupport] = {
    "near-intents-1click": ProviderSupport(
        "near-intents-1click", "adapter", "2026-11-13",
        ("https://docs.near-intents.org/api-reference/oneclick/request-a-swap-quote",
         "https://docs.near-intents.org/near-intents/integration/distribution-channels/1click-app-fees-calculation"),
        "Signed quotes; reuses near_intents signature and request-echo checks. Executable only for a "
        "verified live quote with a deposit address and fully signed fees.",
    ),
    "rhea-v2": ProviderSupport(
        "rhea-v2", "adapter", "2026-11-13",
        ("https://guide.ref.finance/developers/contracts", "https://docs.near.org/primitives/dex.md",
         "https://ref-finance.gitbook.io/ref-finance/smart-contracts/ref-exchange"),
        "Read-only pool discovery and get_return on v2.ref-finance.near at an agreed fresh block. "
        "Observation only; never executable.",
    ),
    "lifi": ProviderSupport(
        "lifi", "blocked", "2026-11-13",
        ("https://docs.li.fi/llms.txt", "https://docs.li.fi/introduction/chains.md", "https://docs.li.fi/changelog/backend.md"),
        "LI.FI's documentation lists NEAR Intents as an aggregated bridge for other chains but does not list "
        "NEAR Protocol as a supported source or destination chain, and no backend changelog entry adds it. "
        "No NEAR quote contract to implement against.",
    ),
    "rango": ProviderSupport(
        "rango", "blocked", "2026-11-13",
        ("https://docs.rango.exchange/llms-full.txt",
         "https://docs.rango.exchange/api-integration/basic-api-single-step/api-reference/get-quote",
         "https://docs.rango.exchange/api-integration/basic-api-single-step/api-reference/get-blockchains-and-tokens"),
        "Rango's documentation says it 'will soon' support Near; its blockchain tables and transaction "
        "types (EVM, TRANSFER, COSMOS, SOLANA, STARKNET, TRON, TON, SUI, XRPL, STELLAR) include no NEAR. "
        "No NEAR quote contract to implement against.",
    ),
}


def require_adapter(provider_id: str) -> ProviderSupport:
    support = PROVIDERS.get(provider_id)
    if support is None:
        raise LiquidityError("X3_LIQ_UNSUPPORTED_PROVIDER", f"{provider_id!r} has no reviewed adapter")
    if support.status != "adapter":
        raise LiquidityError("X3_LIQ_UNSUPPORTED_PROVIDER", f"{provider_id} is blocked: {support.summary}",
                             {"sources": list(support.sources), "checked_on": support.checked_on})
    return support


__all__ = ["ProviderSupport", "PROVIDERS", "require_adapter"]
