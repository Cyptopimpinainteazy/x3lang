"""Versioned `ProviderQuote` record: provenance-bound, integer-only, fail-closed.

A `ProviderQuote` is what every liquidity adapter in this package returns. It never moves
funds and nothing in this package signs, deposits or broadcasts. The record carries:

- the provider ID and chain-qualified input/output assets with their decimals;
- integer base-unit amounts: the quoted output and the provider-guaranteed floor;
- issue and expiry times, and where the expiry came from;
- a fee breakdown in which every component names the response field or chain view it came
  from and whether a signature or agreed chain state covers it;
- source provenance: the endpoint, a SHA-256 of what was observed and a provider reference
  (correlation ID, request ID or block hash);
- a SHA-256 of the exact request;
- a signature-verification status; and
- an explicit `executable` flag, which has no default.

**Signature status cannot be asserted by a caller.** `SignatureVerification` has no public
constructor. `UNSIGNED` (which grants nothing) is available to anyone; `VERIFIED` is issued
only by `_issue_verified`, which itself runs Ed25519 verification against a signer pinned
in reviewed code for that provider, and seals the result with a per-process HMAC over the
provider, the observed-response digest and the quote's terms digest. A `ProviderQuote`
rechecks that seal, the pinned signer and the terms digest on construction, so a verified
status cannot be transplanted onto other terms and `dataclasses.replace` cannot flip a quote
to executable. Python cannot stop in-process code from calling a private function; that is
a code-review boundary, stated here rather than implied away.

`executable=True` additionally requires a signed deposit-binding commitment, every fee to
be authenticated, and a guaranteed net output that can be computed in the output asset and
is positive. Observation-only data (chain reserves, indicative or dry quotes) is always
`executable=False` with a stated reason.
"""
from __future__ import annotations

import enum
import hashlib
import hmac
import json
import re
import secrets
from dataclasses import dataclass, field
from typing import Any, Dict, FrozenSet, Optional, Tuple

import near_intents as ni

SCHEMA_VERSION = "x3.provider_quote.v1"

#: Upper bound for any base-unit amount handled here. NEAR U128 values and 256-bit EVM
#: amounts both fit; anything larger is refused rather than carried as an unbounded int.
U256_MAX = (1 << 256) - 1
U128_MAX = (1 << 128) - 1
MAX_DECIMALS = 255


class LiquidityError(ValueError):
    """A refusal with a stable machine-readable code. Never a partial success."""

    def __init__(self, code: str, message: str, detail: Optional[Dict[str, Any]] = None):
        super().__init__(f"{code}: {message}")
        self.code = code
        self.message = message
        self.detail = dict(detail or {})


# ---------------------------------------------------------------------------------------
# Checked integer arithmetic
# ---------------------------------------------------------------------------------------

_UINT_RE = re.compile(r"0|[1-9][0-9]{0,77}")


def parse_uint(value: Any, field_name: str, *, maximum: int = U256_MAX) -> int:
    """A non-negative base-unit integer from an int or a canonical decimal string.

    Booleans, floats, signs, whitespace, leading zeros, exponents and separators are all
    refused: a provider amount that is not already an exact integer is not one.
    """
    if isinstance(value, bool):
        raise LiquidityError("X3_LIQ_MALFORMED", f"{field_name} is a boolean, not an integer")
    if isinstance(value, int):
        number = value
    elif isinstance(value, str) and _UINT_RE.fullmatch(value):
        number = int(value)
    else:
        raise LiquidityError("X3_LIQ_MALFORMED", f"{field_name} is not a canonical base-unit integer: {value!r}")
    if number < 0 or number > maximum:
        raise LiquidityError("X3_LIQ_OUT_OF_RANGE", f"{field_name} is outside 0..{maximum}")
    return number


def checked_add(a: int, b: int, what: str) -> int:
    total = a + b
    if a < 0 or b < 0 or total > U256_MAX:
        raise LiquidityError("X3_LIQ_OVERFLOW", f"{what} overflows the 256-bit bound")
    return total


def checked_sub(a: int, b: int, what: str) -> int:
    if a < 0 or b < 0 or b > a:
        raise LiquidityError("X3_LIQ_UNDERFLOW", f"{what} underflows: {a} - {b}", {"minuend": str(a), "subtrahend": str(b)})
    return a - b


def rescale(amount: int, from_decimals: int, to_decimals: int) -> int:
    """Move `amount` between decimal precisions exactly, or refuse.

    Scaling up is checked against the 256-bit bound. Scaling down must be exact: a
    remainder would be silently discarded value, so it is an error, never a rounding.
    """
    for name, d in (("from_decimals", from_decimals), ("to_decimals", to_decimals)):
        if type(d) is not int or not 0 <= d <= MAX_DECIMALS:
            raise LiquidityError("X3_LIQ_DECIMALS", f"{name} must be an int in 0..{MAX_DECIMALS}")
    amount = parse_uint(amount, "amount")
    if to_decimals >= from_decimals:
        scaled = amount * 10 ** (to_decimals - from_decimals)
        if scaled > U256_MAX:
            raise LiquidityError("X3_LIQ_OVERFLOW", "rescaled amount overflows the 256-bit bound")
        return scaled
    divisor = 10 ** (from_decimals - to_decimals)
    quotient, remainder = divmod(amount, divisor)
    if remainder:
        raise LiquidityError("X3_LIQ_PRECISION_LOSS",
                             f"{amount} at {from_decimals} decimals is not exact at {to_decimals} decimals")
    return quotient


_FORMATTED_RE = re.compile(r"(0|[1-9][0-9]*)(?:\.([0-9]+))?")


def formatted_to_units(text: Any, decimals: int, field_name: str) -> int:
    """`"24.997929"` at 6 decimals -> 24997929, exactly; more fractional digits than
    `decimals` (other than trailing zeros) means the decimals are wrong, so it refuses."""
    if not isinstance(text, str):
        raise LiquidityError("X3_LIQ_MALFORMED", f"{field_name} is not a string")
    match = _FORMATTED_RE.fullmatch(text)
    if not match:
        raise LiquidityError("X3_LIQ_MALFORMED", f"{field_name} is not a plain decimal: {text!r}")
    whole, frac = match.group(1), (match.group(2) or "")
    stripped = frac.rstrip("0")
    if len(stripped) > decimals:
        raise LiquidityError("X3_LIQ_DECIMALS",
                             f"{field_name}={text!r} has more precision than {decimals} decimals")
    units = int(whole) * 10 ** decimals + (int(stripped.ljust(decimals, "0")) if decimals else 0)
    if units > U256_MAX:
        raise LiquidityError("X3_LIQ_OVERFLOW", f"{field_name} overflows the 256-bit bound")
    return units


def canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True,
                      allow_nan=False).encode("ascii")


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


# ---------------------------------------------------------------------------------------
# Record parts
# ---------------------------------------------------------------------------------------

_NETWORK_RE = re.compile(r"[a-z0-9][a-z0-9-]{0,31}")
_ASSET_RE = re.compile(r"[\x21-\x7e]{1,256}")
_HEX64_RE = re.compile(r"[0-9a-f]{64}")
_PROVIDER_RE = re.compile(r"[a-z0-9][a-z0-9.-]{0,63}")


@dataclass(frozen=True)
class AssetRef:
    """A chain-qualified asset: the delivery network, the provider-native asset ID and its
    decimals. Two assets are identical only if all three match exactly."""

    network: str
    asset_id: str
    decimals: int

    def __post_init__(self) -> None:
        if not isinstance(self.network, str) or not _NETWORK_RE.fullmatch(self.network):
            raise LiquidityError("X3_LIQ_MALFORMED", f"network {self.network!r} is not a lowercase network name")
        if not isinstance(self.asset_id, str) or not _ASSET_RE.fullmatch(self.asset_id):
            raise LiquidityError("X3_LIQ_MALFORMED", f"asset ID {self.asset_id!r} is not printable ASCII")
        if type(self.decimals) is not int or not 0 <= self.decimals <= MAX_DECIMALS:
            raise LiquidityError("X3_LIQ_DECIMALS", f"decimals must be an int in 0..{MAX_DECIMALS}")

    @property
    def key(self) -> str:
        return f"{self.network}/{self.asset_id}"

    def to_json(self) -> Dict[str, Any]:
        return {"network": self.network, "asset_id": self.asset_id, "decimals": self.decimals}


class FeeCharge(str, enum.Enum):
    #: Already reflected in the quoted output and floor (e.g. computed by the pool itself).
    INCLUDED_IN_QUOTE = "included_in_quote"
    #: Paid on top of the quoted amounts.
    ADDITIONAL = "additional"
    #: Charged only if the swap refunds; does not reduce a successful delivery.
    REFUND_PATH_ONLY = "refund_path_only"
    #: The provider does not say whether its output is net of this fee. Treated as additional.
    UNKNOWN = "unknown"


_FEE_KINDS = frozenset({"pool", "provider", "protocol", "app", "withdraw", "refund", "network_gas", "bridge"})


@dataclass(frozen=True)
class FeeComponent:
    """One fee, exactly as a provider response or chain view stated it.

    `amount` is base units of `asset` when the source gives one; `provider_value` keeps a
    rate or parameter verbatim (e.g. a pool's `total_fee`) when its unit is not confirmed.
    `authenticated` is True only when a verified signature or agreed chain state covers it.
    """

    kind: str
    charge: FeeCharge
    asset: Optional[str]
    amount: Optional[int]
    provider_value: Optional[str]
    source_field: str
    authenticated: bool

    def __post_init__(self) -> None:
        if self.kind not in _FEE_KINDS:
            raise LiquidityError("X3_LIQ_MALFORMED", f"unknown fee kind {self.kind!r}")
        if not isinstance(self.charge, FeeCharge):
            raise LiquidityError("X3_LIQ_MALFORMED", "fee charge must be a FeeCharge")
        if self.asset is not None and (not isinstance(self.asset, str) or "/" not in self.asset):
            raise LiquidityError("X3_LIQ_MALFORMED", "fee asset must be a chain-qualified key or None")
        if self.amount is not None:
            parse_uint(self.amount, f"fee {self.kind} amount")
        if self.amount is None and self.provider_value is None:
            raise LiquidityError("X3_LIQ_MALFORMED", f"fee {self.kind} states neither an amount nor a provider value")
        if self.provider_value is not None and not isinstance(self.provider_value, str):
            raise LiquidityError("X3_LIQ_MALFORMED", "provider_value must be the verbatim string")
        if not isinstance(self.source_field, str) or not self.source_field:
            raise LiquidityError("X3_LIQ_MALFORMED", "every fee must name the field it came from")
        if type(self.authenticated) is not bool:
            raise LiquidityError("X3_LIQ_MALFORMED", "fee authenticated must be a bool")

    def to_json(self) -> Dict[str, Any]:
        return {"kind": self.kind, "charge": self.charge.value, "asset": self.asset,
                "amount": None if self.amount is None else str(self.amount),
                "provider_value": self.provider_value, "source_field": self.source_field,
                "authenticated": self.authenticated}


_SOURCES = frozenset({"provider_response", "chain_state"})


@dataclass(frozen=True)
class Provenance:
    """Where the numbers came from. `response_sha256` is over exactly what was observed."""

    source: str
    endpoint: str
    response_sha256: str
    observed_at: int
    reference: str
    block_height: Optional[int] = None

    def __post_init__(self) -> None:
        if self.source not in _SOURCES:
            raise LiquidityError("X3_LIQ_MALFORMED", f"provenance source must be one of {sorted(_SOURCES)}")
        if not isinstance(self.endpoint, str) or not self.endpoint.startswith("https://"):
            raise LiquidityError("X3_LIQ_MALFORMED", "provenance endpoint must be an https URL list")
        if not isinstance(self.response_sha256, str) or not _HEX64_RE.fullmatch(self.response_sha256):
            raise LiquidityError("X3_LIQ_MALFORMED", "response_sha256 must be 64 lowercase hex characters")
        if type(self.observed_at) is not int or self.observed_at <= 0:
            raise LiquidityError("X3_LIQ_MALFORMED", "observed_at must be a positive unix time")
        if not isinstance(self.reference, str) or not self.reference:
            raise LiquidityError("X3_LIQ_MALFORMED", "provenance must carry a provider reference")
        if self.source == "chain_state" and (type(self.block_height) is not int or self.block_height <= 0):
            raise LiquidityError("X3_LIQ_MALFORMED", "chain-state provenance must name its block height")

    def to_json(self) -> Dict[str, Any]:
        return {"source": self.source, "endpoint": self.endpoint, "response_sha256": self.response_sha256,
                "observed_at": self.observed_at, "reference": self.reference, "block_height": self.block_height}


# ---------------------------------------------------------------------------------------
# Signature verification status (sealed)
# ---------------------------------------------------------------------------------------

class SignatureStatus(str, enum.Enum):
    #: A pinned provider key signed these exact terms, and this process checked it.
    VERIFIED = "verified"
    #: The provider publishes no signature (or the data is chain state). Grants nothing.
    UNSIGNED = "unsigned"


def trusted_signers(provider_id: str) -> FrozenSet[str]:
    """Signers pinned in reviewed code. Read at call time so a key rotation is a code change
    to `near_intents.ONE_CLICK_MANAGER_PUBLIC_KEY`, never configuration."""
    if provider_id == "near-intents-1click":
        return frozenset({ni.ONE_CLICK_MANAGER_PUBLIC_KEY})
    return frozenset()


_SEAL_KEY = secrets.token_bytes(32)


class SignatureVerification:
    """The outcome of a signature check, sealed so callers cannot assert it."""

    __slots__ = ("status", "scheme", "signer", "provider_id", "response_sha256", "terms_sha256", "reason", "_mac")

    def __init__(self, *args: Any, **kwargs: Any) -> None:
        raise TypeError("SignatureVerification is issued by a verifier, not constructed")

    @staticmethod
    def _message(status: SignatureStatus, scheme: Optional[str], signer: Optional[str], provider_id: str,
                 response_sha256: str, terms_sha256: str, reason: str) -> bytes:
        return canonical_json(["x3.signature_status.v1", status.value, scheme, signer, provider_id,
                               response_sha256, terms_sha256, reason])

    @classmethod
    def _seal(cls, status: SignatureStatus, scheme: Optional[str], signer: Optional[str], provider_id: str,
              response_sha256: str, terms_sha256: str, reason: str) -> "SignatureVerification":
        self = object.__new__(cls)
        values = {"status": status, "scheme": scheme, "signer": signer, "provider_id": provider_id,
                  "response_sha256": response_sha256, "terms_sha256": terms_sha256, "reason": reason}
        for name, value in values.items():
            object.__setattr__(self, name, value)
        mac = hmac.new(_SEAL_KEY, cls._message(status, scheme, signer, provider_id, response_sha256,
                                               terms_sha256, reason), hashlib.sha256).digest()
        object.__setattr__(self, "_mac", mac)
        return self

    def __setattr__(self, name: str, value: Any) -> None:
        raise AttributeError("SignatureVerification is immutable")

    def __delattr__(self, name: str) -> None:
        raise AttributeError("SignatureVerification is immutable")

    def authentic(self) -> bool:
        try:
            expected = hmac.new(_SEAL_KEY, self._message(self.status, self.scheme, self.signer, self.provider_id,
                                                         self.response_sha256, self.terms_sha256, self.reason),
                                hashlib.sha256).digest()
            return hmac.compare_digest(self._mac, expected)
        except (AttributeError, TypeError):
            return False

    def to_json(self) -> Dict[str, Any]:
        return {"status": self.status.value, "scheme": self.scheme, "signer": self.signer,
                "reason": self.reason, "sealed_in_process": self.authentic()}

    def __repr__(self) -> str:
        return f"SignatureVerification({self.status.value}, signer={self.signer!r}, reason={self.reason!r})"


def unsigned_status(provider_id: str, response_sha256: str, terms_sha256: str, reason: str) -> SignatureVerification:
    """Anyone may record that data is unsigned; it can never make a quote executable."""
    if not isinstance(reason, str) or not reason:
        raise LiquidityError("X3_LIQ_MALFORMED", "an unsigned status must say why")
    return SignatureVerification._seal(SignatureStatus.UNSIGNED, None, None, provider_id,
                                       response_sha256, terms_sha256, reason)


def _issue_verified(provider_id: str, *, public_key: str, message: bytes, signature: str,
                    response_sha256: str, terms_sha256: str) -> SignatureVerification:
    """Run Ed25519 verification under a pinned signer and seal the result.

    Package-internal: provider adapters call this with the message they derived from the
    same response as `terms_sha256`. It refuses an unpinned key and a bad signature, so it
    cannot mint `VERIFIED` for anything that did not actually verify.
    """
    if public_key not in trusted_signers(provider_id):
        raise LiquidityError("X3_LIQ_UNTRUSTED_SIGNER", f"{public_key!r} is not pinned for {provider_id!r}")
    try:
        raw_key = ni._decode_ed25519(public_key, 32)
        raw_sig = ni._decode_ed25519(signature, 64)
    except (ni.QuoteSignatureFormatError, ValueError) as exc:
        raise LiquidityError("X3_LIQ_INVALID_SIGNATURE", f"signature or key is malformed: {exc}") from None
    if not ni.ed25519.verify(raw_key, bytes(message), raw_sig):
        raise LiquidityError("X3_LIQ_INVALID_SIGNATURE", f"signature does not verify under the pinned {provider_id} key")
    return SignatureVerification._seal(SignatureStatus.VERIFIED, "ed25519", public_key, provider_id,
                                       response_sha256, terms_sha256, "verified")


# ---------------------------------------------------------------------------------------
# The record
# ---------------------------------------------------------------------------------------

class Commitment(str, enum.Enum):
    #: Signed terms bound to a deposit address and deadline. The only executable kind.
    SIGNED_DEPOSIT_QUOTE = "signed_deposit_quote"
    #: Signed but indicative (e.g. a 1Click dry quote: no deposit address).
    SIGNED_INDICATIVE = "signed_indicative"
    #: Read from chain state; nothing reserves it for this order.
    CHAIN_OBSERVATION = "chain_observation"


def quote_terms_sha256(*, provider_id: str, asset_in: AssetRef, asset_out: AssetRef, amount_in: int,
                       amount_out: int, min_amount_out: int, issued_at: int, expires_at: int,
                       expiry_source: str, commitment: Commitment, fees: Tuple[FeeComponent, ...],
                       provenance: Provenance, request_hash: str) -> str:
    return sha256_hex(canonical_json({
        "schema_version": SCHEMA_VERSION, "provider_id": provider_id,
        "asset_in": asset_in.to_json(), "asset_out": asset_out.to_json(),
        "amount_in": str(amount_in), "amount_out": str(amount_out), "min_amount_out": str(min_amount_out),
        "issued_at": issued_at, "expires_at": expires_at, "expiry_source": expiry_source,
        "commitment": commitment.value, "fees": [f.to_json() for f in fees],
        "provenance": provenance.to_json(), "request_hash": request_hash,
    }))


@dataclass(frozen=True)
class ProviderQuote:
    schema_version: str
    provider_id: str
    asset_in: AssetRef
    asset_out: AssetRef
    amount_in: int
    amount_out: int
    min_amount_out: int
    issued_at: int
    expires_at: int
    expiry_source: str
    commitment: Commitment
    fees: Tuple[FeeComponent, ...]
    provenance: Provenance
    request_hash: str
    signature: SignatureVerification
    executable: bool
    non_executable_reason: Optional[str]
    terms_sha256: str = field(init=False)

    def __post_init__(self) -> None:
        if self.schema_version != SCHEMA_VERSION:
            raise LiquidityError("X3_LIQ_SCHEMA", f"unsupported schema {self.schema_version!r}")
        if not isinstance(self.provider_id, str) or not _PROVIDER_RE.fullmatch(self.provider_id):
            raise LiquidityError("X3_LIQ_MALFORMED", f"provider ID {self.provider_id!r} is malformed")
        if not isinstance(self.asset_in, AssetRef) or not isinstance(self.asset_out, AssetRef):
            raise LiquidityError("X3_LIQ_MALFORMED", "assets must be AssetRef")
        if self.asset_in == self.asset_out:
            raise LiquidityError("X3_LIQ_MALFORMED", "input and output asset are identical")
        for name in ("amount_in", "amount_out", "min_amount_out"):
            parse_uint(getattr(self, name), name)
        if self.amount_in == 0:
            raise LiquidityError("X3_LIQ_MALFORMED", "amount_in must be positive")
        if self.min_amount_out > self.amount_out:
            raise LiquidityError("X3_LIQ_MALFORMED", "min_amount_out exceeds amount_out")
        if type(self.issued_at) is not int or type(self.expires_at) is not int or not 0 < self.issued_at < self.expires_at:
            raise LiquidityError("X3_LIQ_MALFORMED", "issued_at must be positive and before expires_at")
        if not isinstance(self.expiry_source, str) or not self.expiry_source:
            raise LiquidityError("X3_LIQ_MALFORMED", "expiry_source must say where the expiry came from")
        if not isinstance(self.commitment, Commitment):
            raise LiquidityError("X3_LIQ_MALFORMED", "commitment must be a Commitment")
        if not isinstance(self.fees, tuple) or not all(isinstance(f, FeeComponent) for f in self.fees):
            raise LiquidityError("X3_LIQ_MALFORMED", "fees must be a tuple of FeeComponent")
        if not isinstance(self.provenance, Provenance):
            raise LiquidityError("X3_LIQ_MALFORMED", "provenance must be a Provenance")
        if not isinstance(self.request_hash, str) or not _HEX64_RE.fullmatch(self.request_hash):
            raise LiquidityError("X3_LIQ_MALFORMED", "request_hash must be 64 lowercase hex characters")
        if type(self.executable) is not bool:
            raise LiquidityError("X3_LIQ_MALFORMED", "executable must be stated explicitly as a bool")

        terms = quote_terms_sha256(
            provider_id=self.provider_id, asset_in=self.asset_in, asset_out=self.asset_out,
            amount_in=self.amount_in, amount_out=self.amount_out, min_amount_out=self.min_amount_out,
            issued_at=self.issued_at, expires_at=self.expires_at, expiry_source=self.expiry_source,
            commitment=self.commitment, fees=self.fees, provenance=self.provenance, request_hash=self.request_hash)
        object.__setattr__(self, "terms_sha256", terms)

        sig = self.signature
        if not isinstance(sig, SignatureVerification) or not sig.authentic():
            raise LiquidityError("X3_LIQ_SIGNATURE_STATUS_FORGED", "signature status was not issued by a verifier")
        if sig.provider_id != self.provider_id or sig.response_sha256 != self.provenance.response_sha256 \
                or sig.terms_sha256 != terms:
            raise LiquidityError("X3_LIQ_SIGNATURE_STATUS_MISMATCH",
                                 "signature status was issued for a different provider, response or terms")
        if sig.status is SignatureStatus.VERIFIED and sig.signer not in trusted_signers(self.provider_id):
            raise LiquidityError("X3_LIQ_UNTRUSTED_SIGNER", "verified signer is no longer pinned for this provider")

        if self.executable:
            refusal = self.executable_refusal()
            if refusal is not None:
                raise LiquidityError("X3_LIQ_NOT_EXECUTABLE", refusal)
            if self.non_executable_reason is not None:
                raise LiquidityError("X3_LIQ_MALFORMED", "an executable quote has no non-executable reason")
        elif not isinstance(self.non_executable_reason, str) or not self.non_executable_reason:
            raise LiquidityError("X3_LIQ_MALFORMED", "a non-executable quote must state why")

    # -- derived values ------------------------------------------------------------------

    def _net(self, base: int) -> Optional[int]:
        """`base` less every fee that is not already inside it. None if not computable."""
        deductions = 0
        for fee in self.fees:
            if fee.charge in (FeeCharge.INCLUDED_IN_QUOTE, FeeCharge.REFUND_PATH_ONLY):
                continue
            if fee.amount is None or fee.asset != self.asset_out.key:
                # A fee in another asset, or of unstated size, cannot be netted against the
                # output without a price. Refuse to produce a number rather than guess one.
                return None
            deductions = checked_add(deductions, fee.amount, "fee total")
        return checked_sub(base, deductions, f"{self.provider_id} output after fees")

    def guaranteed_net_output(self) -> Optional[int]:
        """The provider-guaranteed delivery after all fees, or None when nothing is guaranteed:
        only a verified, deposit-bound signed quote guarantees anything."""
        if self.signature.status is not SignatureStatus.VERIFIED or self.commitment is not Commitment.SIGNED_DEPOSIT_QUOTE:
            return None
        return self._net(self.min_amount_out)

    def expected_net_output(self) -> Optional[int]:
        """The quoted (not guaranteed) output after fees, for labelled observations only."""
        return self._net(self.amount_out)

    def executable_refusal(self) -> Optional[str]:
        if self.signature.status is not SignatureStatus.VERIFIED:
            return "terms are not covered by a verified provider signature"
        if self.commitment is not Commitment.SIGNED_DEPOSIT_QUOTE:
            return f"commitment is {self.commitment.value}, not a signed deposit-bound quote"
        unauthenticated = [f.kind for f in self.fees if not f.authenticated]
        if unauthenticated:
            return f"fees not covered by the signature: {', '.join(unauthenticated)}"
        try:
            net = self.guaranteed_net_output()
        except LiquidityError as exc:
            return exc.message
        if net is None:
            return "guaranteed net output cannot be computed in the output asset"
        if net <= 0:
            return "guaranteed net output is zero"
        return None

    def to_json(self) -> Dict[str, Any]:
        try:
            guaranteed = self.guaranteed_net_output()
        except LiquidityError:
            guaranteed = None
        try:
            expected = self.expected_net_output()
        except LiquidityError:
            expected = None
        return {
            "schema_version": self.schema_version, "provider_id": self.provider_id,
            "asset_in": self.asset_in.to_json(), "asset_out": self.asset_out.to_json(),
            "amount_in": str(self.amount_in), "amount_out": str(self.amount_out),
            "min_amount_out": str(self.min_amount_out),
            "guaranteed_net_output": None if guaranteed is None else str(guaranteed),
            "expected_net_output": None if expected is None else str(expected),
            "issued_at": self.issued_at, "expires_at": self.expires_at, "expiry_source": self.expiry_source,
            "commitment": self.commitment.value, "fees": [f.to_json() for f in self.fees],
            "provenance": self.provenance.to_json(), "request_hash": self.request_hash,
            "terms_sha256": self.terms_sha256, "signature": self.signature.to_json(),
            "executable": self.executable, "non_executable_reason": self.non_executable_reason,
        }


__all__ = [
    "SCHEMA_VERSION", "U128_MAX", "U256_MAX", "LiquidityError", "parse_uint", "checked_add", "checked_sub",
    "rescale", "formatted_to_units", "canonical_json", "sha256_hex", "AssetRef", "FeeCharge", "FeeComponent",
    "Provenance", "SignatureStatus", "SignatureVerification", "trusted_signers", "unsigned_status",
    "Commitment", "quote_terms_sha256", "ProviderQuote",
]
