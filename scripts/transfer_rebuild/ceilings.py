#!/usr/bin/env python3
"""Derive diagnostic transfer ceilings from independent calibration artifacts."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import stat
import sys
from pathlib import Path
from typing import Any

try:
    from jsonschema import Draft202012Validator, FormatChecker
except ImportError as error:  # pragma: no cover - exercised by CLI environment
    raise SystemExit(
        "missing dependency: install scripts/transfer_rebuild/requirements.txt"
    ) from error


INPUT_FORMAT = "transfer-ceiling-input-v1"
OUTPUT_FORMAT = "transfer-ceiling-v1"
MAX_INPUT_BYTES = 1024 * 1024
INPUT_SCHEMA = (
    Path(__file__).resolve().parents[2]
    / "certification/transfer-rebuild/ceiling-input.schema.json"
)


class CeilingError(ValueError):
    """An independent ceiling input is incomplete or inconsistent."""


def _pairs_without_duplicates(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise CeilingError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def read_calibration_input(path: Path) -> tuple[dict[str, Any], str]:
    descriptor = -1
    if path.is_symlink():
        raise CeilingError("input path must not be a symlink")
    try:
        descriptor = os.open(
            path,
            os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0),
        )
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_INPUT_BYTES:
            raise CeilingError("input must be a regular file no larger than 1 MiB")
        source = os.fdopen(descriptor, "rb")
        descriptor = -1
        with source:
            raw = source.read(MAX_INPUT_BYTES + 1)
            after = os.fstat(source.fileno())
        if len(raw) > MAX_INPUT_BYTES:
            raise CeilingError("input must be a regular file no larger than 1 MiB")
        if (metadata.st_dev, metadata.st_ino, metadata.st_size, metadata.st_mtime_ns) != (
            after.st_dev,
            after.st_ino,
            after.st_size,
            after.st_mtime_ns,
        ):
            raise CeilingError("calibration input changed while it was being read")
    except CeilingError:
        raise
    except OSError as error:
        raise CeilingError(f"cannot read calibration input: {error}") from error
    finally:
        if descriptor >= 0:
            os.close(descriptor)

    def reject_constant(value: str) -> None:
        raise CeilingError(f"non-standard JSON number is not allowed: {value}")

    try:
        value = json.loads(
            raw,
            object_pairs_hook=_pairs_without_duplicates,
            parse_constant=reject_constant,
        )
    except (json.JSONDecodeError, UnicodeDecodeError) as error:
        raise CeilingError(f"invalid calibration JSON: {error}") from error
    if not isinstance(value, dict):
        raise CeilingError("calibration input must be a JSON object")
    schema = json.loads(INPUT_SCHEMA.read_text(encoding="utf-8"))
    errors = sorted(
        Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(value),
        key=lambda error: (list(map(str, error.absolute_path)), error.message),
    )
    if errors:
        first = errors[0]
        path_text = ".".join(str(part) for part in first.absolute_path) or "$"
        raise CeilingError(f"invalid calibration input at {path_text}: {first.message}")
    return value, hashlib.sha256(raw).hexdigest()


def _positive_rate(value: Any, field: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise CeilingError(f"{field} must be a positive finite number")
    try:
        rate = float(value)
    except OverflowError as error:
        raise CeilingError(f"{field} must be a positive finite number") from error
    if not math.isfinite(rate) or rate <= 0:
        raise CeilingError(f"{field} must be a positive finite number")
    return rate


def _pipeline_rate(pipeline: dict[str, Any]) -> float:
    fields = ("peer_count", "requests_per_peer", "block_bytes", "rtt_ns")
    numbers: list[int] = []
    for name in fields:
        value = pipeline.get(name)
        if isinstance(value, bool) or not isinstance(value, int) or value <= 0:
            raise CeilingError(f"pipeline.{name} must be a positive integer")
        numbers.append(value)
    peer_count, requests_per_peer, block_bytes, rtt_ns = numbers
    try:
        rate = peer_count * requests_per_peer * block_bytes * 1_000_000_000 / rtt_ns
    except OverflowError as error:
        raise CeilingError("pipeline capacity is outside the supported numeric range") from error
    if not math.isfinite(rate) or rate <= 0:
        raise CeilingError("pipeline capacity is outside the supported numeric range")
    return rate


def _ceiling(candidates: dict[str, float]) -> dict[str, Any]:
    value = min(candidates.values())
    limiting = sorted(
        name
        for name, rate in candidates.items()
        if rate == value
    )
    return {"bytes_per_second": value, "limiting_components": limiting}


def calculate_report(calibration: dict[str, Any], input_sha256: str) -> dict[str, Any]:
    if calibration.get("schema_version") != INPUT_FORMAT:
        raise CeilingError(f"schema_version must be {INPUT_FORMAT}")
    protocol = calibration.get("protocol")
    if protocol not in {"v1", "v2", "hybrid"}:
        raise CeilingError("protocol must be v1, v2, or hybrid")
    network = calibration.get("network")
    disk = calibration.get("disk")
    hashes = calibration.get("hash")
    pipeline = calibration.get("pipeline")
    environment = calibration.get("environment")
    if not all(
        isinstance(item, dict)
        for item in (network, disk, hashes, pipeline, environment)
    ):
        raise CeilingError(
            "environment and all four independent calibration groups are required"
        )

    network_down = _positive_rate(
        network.get("download_bytes_per_second"), "network.download_bytes_per_second"
    )
    network_up = _positive_rate(
        network.get("upload_bytes_per_second"), "network.upload_bytes_per_second"
    )
    disk_write = _positive_rate(disk.get("write_bytes_per_second"), "disk.write_bytes_per_second")
    disk_read = _positive_rate(disk.get("read_bytes_per_second"), "disk.read_bytes_per_second")
    if protocol == "v1":
        hash_rate = _positive_rate(hashes.get("sha1_bytes_per_second"), "hash.sha1_bytes_per_second")
    elif protocol == "v2":
        hash_rate = _positive_rate(hashes.get("sha256_bytes_per_second"), "hash.sha256_bytes_per_second")
    else:
        hybrid = hashes.get("hybrid_bytes_per_second")
        hash_rate = _positive_rate(hybrid, "hash.hybrid_bytes_per_second")
    pipeline_rate = _pipeline_rate(pipeline)

    download_candidates = {
        "network": network_down,
        "disk": disk_write,
        "hash": hash_rate,
        "pipeline": pipeline_rate,
    }
    upload_candidates = {
        "network": network_up,
        "disk": disk_read,
        "pipeline": pipeline_rate,
    }
    return {
        "schema_version": OUTPUT_FORMAT,
        "calibration_id": calibration.get("calibration_id"),
        "input_sha256": input_sha256,
        "protocol": protocol,
        "environment": environment,
        "components": {
            "network_download": network_down,
            "network_upload": network_up,
            "disk_write": disk_write,
            "disk_read": disk_read,
            "hash": hash_rate,
            "pipeline": pipeline_rate,
        },
        "download": _ceiling(download_candidates),
        "upload": _ceiling(upload_candidates),
        "interpretation": "diagnostic upper-bound model; not a client measurement or performance claim",
    }


def _write_exclusive(path: Path, value: dict[str, Any]) -> None:
    parent = path.parent
    if parent.is_symlink() or not parent.is_dir():
        raise CeilingError("output parent must be an existing, non-symlink directory")
    encoded = (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode("utf-8")
    identity: tuple[int, int] | None = None
    try:
        descriptor = os.open(
            path,
            os.O_WRONLY
            | os.O_CREAT
            | os.O_EXCL
            | getattr(os, "O_CLOEXEC", 0)
            | getattr(os, "O_NOFOLLOW", 0),
            0o600,
        )
        info = os.fstat(descriptor)
        identity = (info.st_dev, info.st_ino)
    except FileExistsError as error:
        raise CeilingError("output already exists; keep calibration reports immutable") from error
    except OSError as error:
        raise CeilingError(f"cannot create report: {error}") from error
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
    except OSError:
        if identity is not None:
            try:
                current = path.stat(follow_symlinks=False)
                if (current.st_dev, current.st_ino) == identity:
                    path.unlink()
            except OSError:
                pass
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path, help="independent transfer-ceiling-input-v1 JSON")
    parser.add_argument("--output", required=True, type=Path, help="new report path; existing files are never replaced")
    arguments = parser.parse_args(argv)
    try:
        calibration, digest = read_calibration_input(arguments.input)
        _write_exclusive(arguments.output, calculate_report(calibration, digest))
    except (CeilingError, OSError) as error:
        print(f"ceiling report failed: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
