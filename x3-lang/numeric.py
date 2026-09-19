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
    # Every caller narrows this to float immediately; a Decimal outside
    # float range is finite by Decimal's own definition but silently
    # becomes `inf`/`-inf` on that narrowing (Python's float() does not
    # raise), which is exactly the silent-corruption class this parser
    # exists to reject.
    try:
        if not isfinite(float(result)):
            raise NumericParseError("value must be finite")
    except OverflowError as exc:
        raise NumericParseError("value must be finite") from exc
    return result


def parse_positive_decimal(value: Any, *, allow_unit_suffix: bool = False) -> Decimal:
    result = parse_decimal(value, allow_unit_suffix=allow_unit_suffix)
    if result <= 0:
        raise NumericParseError("value must be positive")
    return result
