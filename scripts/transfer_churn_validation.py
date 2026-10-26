#!/usr/bin/env python3
"""Fail-closed transfer evidence checks shared by the churn script and tests."""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from typing import Any


_SHA256_RE = re.compile(r"^[0-9a-fA-F]{64}$")
_TORRENT_HASH_RE = re.compile(r"^(?:[0-9a-fA-F]{40}|[0-9a-fA-F]{64})$")


def _count(value: Any) -> int | None:
    if isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value if value >= 0 else None
    if isinstance(value, float) and math.isfinite(value) and value.is_integer() and value >= 0:
        return int(value)
    return None


def _progress(value: Any) -> float | None:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        return None
    result = float(value)
    if not math.isfinite(result) or not 0.0 <= result <= 1.0:
        return None
    return result


def is_complete_transfer(
    row: Any,
    *,
    expected_bytes: int,
    disk_bytes: int,
    expected_sha256: str,
    actual_sha256: str,
) -> bool:
    """Require API completion, byte counts, exact file size, and payload identity."""
    expected_count = _count(expected_bytes)
    disk_count = _count(disk_bytes)
    if not isinstance(row, dict) or expected_count is None or expected_count == 0 or disk_count is None:
        return False
    progress = _progress(row.get("progress"))
    downloaded = _count(row.get("downloaded"))
    if progress is None or downloaded is None:
        return False
    if progress < 1.0 or downloaded < expected_count or disk_count != expected_count:
        return False
    if not isinstance(expected_sha256, str) or not isinstance(actual_sha256, str):
        return False
    if not _SHA256_RE.fullmatch(expected_sha256) or not _SHA256_RE.fullmatch(actual_sha256):
        return False
    return expected_sha256.lower() == actual_sha256.lower()


def has_public_transfer(row: Any, minimum_downloaded_bytes: int) -> bool:
    """Require visible progress and a nonzero configured amount of payload data."""
    minimum_count = _count(minimum_downloaded_bytes)
    if not isinstance(row, dict) or minimum_count is None or minimum_count == 0:
        return False
    progress = _progress(row.get("progress"))
    downloaded = _count(row.get("downloaded"))
    return (
        progress is not None
        and progress > 0.0
        and downloaded is not None
        and downloaded >= minimum_count
    )


def category_hashes(rows: Any) -> list[str] | None:
    """Return all valid hashes from a torrent-info array, or None if malformed."""
    if not isinstance(rows, list):
        return None
    hashes: list[str] = []
    for row in rows:
        if not isinstance(row, dict):
            return None
        torrent_hash = row.get("hash")
        if not isinstance(torrent_hash, str) or not _TORRENT_HASH_RE.fullmatch(torrent_hash):
            return None
        hashes.append(torrent_hash)
    return hashes


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="mode", required=True)

    complete = subparsers.add_parser("complete")
    complete.add_argument("--expected-bytes", type=int, required=True)
    complete.add_argument("--disk-bytes", type=int, required=True)
    complete.add_argument("--expected-sha256", required=True)
    complete.add_argument("--actual-sha256", required=True)

    public = subparsers.add_parser("public")
    public.add_argument("--minimum-downloaded-bytes", type=int, required=True)

    subparsers.add_parser("category-hashes")

    args = parser.parse_args(argv)
    try:
        row = json.load(sys.stdin)
    except (json.JSONDecodeError, UnicodeDecodeError):
        return 2

    if args.mode == "category-hashes":
        hashes = category_hashes(row)
        if hashes is None:
            return 1
        print("|".join(hashes) if hashes else "EMPTY")
        return 0
    if args.mode == "complete":
        passed = is_complete_transfer(
            row,
            expected_bytes=args.expected_bytes,
            disk_bytes=args.disk_bytes,
            expected_sha256=args.expected_sha256,
            actual_sha256=args.actual_sha256,
        )
    else:
        passed = has_public_transfer(row, args.minimum_downloaded_bytes)
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
