"""Strict numeric parsing shared by the Python intent pipeline."""
from decimal import Decimal, InvalidOperation
from math import isfinite
from typing import Any


class NumericParseError(ValueError):
    """Raised when an intent numeric value is missing, malformed, or non-finite."""


def parse_decimal(value: Any, *, allow_unit_suffix: bool = False) -> Decimal:
    if isinstance(value, bool) or value is None:
        raise NumericParseError("value must be numeric")
    text = str(value).strip()
    if allow_unit_suffix:
        text = text.split(maxsplit=1)[0] if text else text
        text = text.removesuffix("%")
    try:
        result = Decimal(text)
    except (InvalidOperation, ValueError) as exc:
        raise NumericParseError("value must be numeric") from exc
    if not result.is_finite():
        raise NumericParseError("value must be finite")
    # `is_finite` above is `Decimal`'s notion of finite, and it is wider than the one
    # every caller can honour: all nine of them narrow the result immediately
    # (`float(parse_decimal(...))`), and `float()` does **not** raise on a `Decimal`
    # outside its range — it returns `inf`. So `1e400` was finite here and infinite one
    # line later. That `inf` reached the planner's estimates as `expected_profit_usd:
    # NaN` and `estimated_slippage_usd: Infinity`, and `json.dumps` writes both, so the
    # runner's own output document was not valid JSON (RFC 8259 defines neither) and
    # nothing in the pipeline said so. An amount the pipeline cannot represent is not a
    # large amount, it is an unrepresentable one, and this is the single place every
    # caller passes through.
    try:
        narrowed = float(result)
    except OverflowError as exc:
        raise NumericParseError("value must be finite") from exc
    if not isfinite(narrowed):
        raise NumericParseError("value must be finite")
    return result


def parse_positive_decimal(value: Any, *, allow_unit_suffix: bool = False) -> Decimal:
    result = parse_decimal(value, allow_unit_suffix=allow_unit_suffix)
    if result <= 0:
        raise NumericParseError("value must be positive")
    return result
