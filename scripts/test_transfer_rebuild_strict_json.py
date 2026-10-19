"""Regression sources for strict external JSON parsing."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent / "transfer_rebuild"))
from strict_json import StrictJsonError, loads  # noqa: E402


class StrictJsonTests(unittest.TestCase):
    def test_rejects_duplicate_members_at_any_depth(self) -> None:
        for raw in (b'{"a":1,"a":2}', b'{"outer":{"x":1,"x":2}}'):
            with self.subTest(raw=raw), self.assertRaises(StrictJsonError):
                loads(raw, label="artifact")

    def test_rejects_nonstandard_and_overflow_numbers(self) -> None:
        for raw in (
            b'{"value":NaN}',
            b'{"value":Infinity}',
            b'{"value":1e999}',
            b'{"value":' + (b"9" * 5000) + b"}",
        ):
            with self.subTest(raw=raw), self.assertRaises(StrictJsonError):
                loads(raw, label="observation")

    def test_preserves_finite_numeric_types(self) -> None:
        self.assertEqual(loads(b'{"count":3,"rate":1.25}'), {"count": 3, "rate": 1.25})


if __name__ == "__main__":
    unittest.main()
