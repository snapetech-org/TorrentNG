"""Strict JSON decoding for external artifacts and runtime readbacks."""

from __future__ import annotations

import json
import math
from typing import Any


class StrictJsonError(ValueError):
    """JSON is malformed, ambiguous, non-standard, or outside finite range."""


def _object_without_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise StrictJsonError(f"duplicate JSON member: {key}")
        result[key] = value
    return result


def _reject_constant(value: str) -> None:
    raise StrictJsonError(f"non-standard JSON number: {value}")


def _finite_float(value: str) -> float:
    parsed = float(value)
    if not math.isfinite(parsed):
        raise StrictJsonError(f"JSON number is outside the finite range: {value}")
    return parsed


def loads(data: str | bytes | bytearray, *, label: str = "JSON") -> Any:
    """Decode JSON while rejecting duplicate keys and non-finite numbers."""
    try:
        return json.loads(
            data,
            object_pairs_hook=_object_without_duplicates,
            parse_constant=_reject_constant,
            parse_float=_finite_float,
        )
    except (UnicodeDecodeError, ValueError, RecursionError) as error:
        raise StrictJsonError(f"{label} is not strict JSON: {error}") from error
