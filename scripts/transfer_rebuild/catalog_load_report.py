#!/usr/bin/env python3
"""Bind live-catalog mixed-load output to its clean source and host identity."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import socket
import stat
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

try:
    from jsonschema import Draft202012Validator, FormatChecker
    from strict_json import StrictJsonError, loads as strict_json_loads
    from trial_record import _open_directory_tree_nofollow
    from validate_artifact import SCHEMAS
except ImportError as error:  # pragma: no cover - exercised by the CLI environment
    raise SystemExit("catalog_load_report.py must run from its scripts directory") from error


REPORT_VERSION = "transfer-catalog-load-report-v1"
LOG_PREFIX = "scale_live_native_catalog_mixed_json="
MAX_LOG_BYTES = 16 * 1024 * 1024
REQUIRED_CATALOG_ROWS = (1_000, 100_000)
REQUIRED_ROUNDS = 16
REQUIRED_ROUTES = frozenset(
    {
        "/api/v1/torrents?limit=200&sort=name&reverse=true",
        "/api/v1/torrents?limit=200&offset=200",
        "/api/v1/transfer/info",
        "/metrics",
    }
)


class CatalogReportError(ValueError):
    """Catalog-load evidence is incomplete or cannot be safely published."""


def _read_log(path: Path) -> tuple[bytes, str]:
    path = Path(path)
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise CatalogReportError("test log path must be absolute and normalized")
    try:
        parent_fd = _open_directory_tree_nofollow(path.parent)
    except OSError as error:
        raise CatalogReportError("test log parent must be an existing no-symlink directory") from error
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW
    try:
        descriptor = os.open(path.name, flags, dir_fd=parent_fd)
    except OSError as error:
        os.close(parent_fd)
        raise CatalogReportError(f"cannot open test log without following links: {error}") from error
    digest = hashlib.sha256()
    content = bytearray()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_LOG_BYTES:
            raise CatalogReportError("test log must be a bounded regular file")
        while chunk := os.read(descriptor, 1024 * 1024):
            content.extend(chunk)
            digest.update(chunk)
            if len(content) > MAX_LOG_BYTES:
                raise CatalogReportError("test log exceeds the 16 MiB input limit")
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
        os.close(parent_fd)
    identity = lambda info: (
        info.st_dev,
        info.st_ino,
        info.st_size,
        info.st_mtime_ns,
        info.st_ctime_ns,
    )
    if identity(before) != identity(after) or len(content) != before.st_size:
        raise CatalogReportError("test log changed while it was read")
    return bytes(content), digest.hexdigest()


def _percentile(samples: list[int], numerator: int, denominator: int) -> int:
    ordered = sorted(samples)
    rank = max(1, (len(ordered) * numerator + denominator - 1) // denominator)
    return ordered[min(rank - 1, len(ordered) - 1)]


def _validate_observation(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict) or set(value) != {
        "schema_version",
        "catalog_rows",
        "catalog_state",
        "rounds",
        "concurrent_routes_per_round",
        "routes",
    }:
        raise CatalogReportError("mixed-load JSON record has an invalid top-level shape")
    if value["schema_version"] != "transfer-live-catalog-mixed-v1":
        raise CatalogReportError("unsupported mixed-load log record version")
    rows = value["catalog_rows"]
    if type(rows) is not int or rows not in REQUIRED_CATALOG_ROWS:
        raise CatalogReportError("mixed-load record must describe the 1k or 100k live catalog")
    if (
        value["catalog_state"] != "restored_paused"
        or value["rounds"] != REQUIRED_ROUNDS
        or value["concurrent_routes_per_round"] != len(REQUIRED_ROUTES)
    ):
        raise CatalogReportError("mixed-load catalog state or request schedule differs from the contract")
    routes = value["routes"]
    if not isinstance(routes, list) or len(routes) != len(REQUIRED_ROUTES):
        raise CatalogReportError("mixed-load record must include each required route exactly once")
    seen: set[str] = set()
    normalized: list[dict[str, Any]] = []
    for item in routes:
        if not isinstance(item, dict) or set(item) != {
            "route",
            "sample_count",
            "p50_ns",
            "p95_ns",
            "samples_ns",
        }:
            raise CatalogReportError("mixed-load route entry has an invalid shape")
        route = item["route"]
        samples = item["samples_ns"]
        if not isinstance(route, str) or route not in REQUIRED_ROUTES or route in seen:
            raise CatalogReportError("mixed-load route is unknown or duplicated")
        if type(item["sample_count"]) is not int or item["sample_count"] != REQUIRED_ROUNDS:
            raise CatalogReportError(f"{route} must contain exactly {REQUIRED_ROUNDS} latency samples")
        if (
            not isinstance(samples, list)
            or len(samples) != REQUIRED_ROUNDS
            or any(type(sample) is not int or sample <= 0 for sample in samples)
        ):
            raise CatalogReportError(f"{route} latency samples must be positive integer nanoseconds")
        p50 = _percentile(samples, 50, 100)
        p95 = _percentile(samples, 95, 100)
        if (
            type(item["p50_ns"]) is not int
            or type(item["p95_ns"]) is not int
            or item["p50_ns"] != p50
            or item["p95_ns"] != p95
        ):
            raise CatalogReportError(f"{route} percentiles do not match the raw samples")
        normalized.append(
            {
                "route": route,
                "sample_count": len(samples),
                "p50_ns": p50,
                "p95_ns": p95,
                "samples_ns": samples,
            }
        )
        seen.add(route)
    if seen != REQUIRED_ROUTES:
        raise CatalogReportError("mixed-load record omits one or more required routes")
    return {
        "catalog_rows": rows,
        "catalog_state": "restored_paused",
        "rounds": REQUIRED_ROUNDS,
        "concurrent_routes_per_round": len(REQUIRED_ROUTES),
        "routes": sorted(normalized, key=lambda item: item["route"]),
    }


def _extract_observations(log_bytes: bytes) -> list[dict[str, Any]]:
    try:
        text = log_bytes.decode("utf-8", "strict")
    except UnicodeDecodeError as error:
        raise CatalogReportError("test log is not UTF-8") from error
    found: dict[int, dict[str, Any]] = {}
    for line in text.splitlines():
        position = line.find(LOG_PREFIX)
        if position < 0:
            continue
        raw = line[position + len(LOG_PREFIX) :]
        try:
            value = strict_json_loads(raw, label="mixed-load log record")
        except StrictJsonError as error:
            raise CatalogReportError(f"mixed-load log record is invalid JSON: {error}") from error
        observation = _validate_observation(value)
        rows = observation["catalog_rows"]
        if rows in found:
            raise CatalogReportError(f"test log contains duplicate {rows}-row observations")
        found[rows] = observation
    if set(found) != set(REQUIRED_CATALOG_ROWS):
        missing = sorted(set(REQUIRED_CATALOG_ROWS) - set(found))
        raise CatalogReportError(
            "test log must include both 1k and 100k mixed-load observations; missing "
            + ", ".join(map(str, missing))
        )
    return [found[rows] for rows in REQUIRED_CATALOG_ROWS]


def _source_identity(repo: Path, expected_commit: str) -> str:
    try:
        top = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "--show-toplevel"],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip()
        commit = subprocess.run(
            ["git", "-C", str(repo), "rev-parse", "--verify", "HEAD^{commit}"],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip()
        status = subprocess.run(
            ["git", "-C", str(repo), "status", "--porcelain=v1", "--untracked-files=all"],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout
    except (OSError, subprocess.SubprocessError) as error:
        raise CatalogReportError(f"cannot capture exact source identity: {error}") from error
    if len(expected_commit) != 40 or any(char not in "0123456789abcdef" for char in expected_commit):
        raise CatalogReportError("source commit must be a lowercase 40-character Git SHA")
    if Path(top).resolve(strict=True) != repo:
        raise CatalogReportError("--repo must name the worktree root, not a subdirectory")
    if commit != expected_commit:
        raise CatalogReportError("requested source commit differs from the checked-out revision")
    if status:
        raise CatalogReportError("catalog report requires a clean committed source worktree")
    return commit


def _host_identity() -> dict[str, str]:
    hostname_digest = hashlib.sha256(socket.gethostname().encode("utf-8")).hexdigest()
    cgroup_root = Path("/sys/fs/cgroup")
    cgroup_version = (
        "v2"
        if (cgroup_root / "cgroup.controllers").is_file()
        else "v1"
        if cgroup_root.is_dir()
        else "unknown"
    )
    return {
        "hostname_sha256": hostname_digest,
        "platform": platform.system() or "unknown",
        "kernel_release": platform.release() or "unknown",
        "machine": platform.machine() or "unknown",
        "cpu_model": platform.processor() or "unknown",
        "cgroup_version": cgroup_version,
    }


def build_report(log_bytes: bytes, log_sha256: str, source_commit: str) -> dict[str, Any]:
    observations = _extract_observations(log_bytes)
    report = {
        "schema_version": REPORT_VERSION,
        "source_commit": source_commit,
        "worktree_dirty": False,
        "captured_at": datetime.now(timezone.utc).isoformat(timespec="seconds").replace("+00:00", "Z"),
        "host": _host_identity(),
        "raw_log": {"sha256": log_sha256, "size_bytes": len(log_bytes)},
        "observations": observations,
    }
    try:
        schema = strict_json_loads(
            SCHEMAS["catalog-load-report"].read_bytes(),
            label="catalog-load report schema",
        )
    except (OSError, StrictJsonError) as error:
        raise CatalogReportError(f"cannot read catalog-load report schema: {error}") from error
    errors = sorted(
        Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(report),
        key=lambda error: (list(map(str, error.absolute_path)), error.message),
    )
    if errors:
        raise CatalogReportError("catalog-load report is invalid: " + "; ".join(
            error.message for error in errors[:20]
        ))
    return report


def write_report(path: Path, report: dict[str, Any], repo: Path) -> None:
    path = Path(path)
    repo = Path(repo).resolve(strict=True)
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise CatalogReportError("report output path must be absolute and normalized")
    try:
        path.relative_to(repo)
    except ValueError:
        pass
    else:
        raise CatalogReportError("catalog report must be written outside the source worktree")
    try:
        parent_fd = _open_directory_tree_nofollow(path.parent)
    except OSError as error:
        raise CatalogReportError("report parent must be an existing no-symlink directory") from error
    data = (json.dumps(report, sort_keys=True, indent=2, allow_nan=False) + "\n").encode("utf-8")
    descriptor: int | None = None
    created_identity: tuple[int, int] | None = None
    try:
        descriptor = os.open(
            path.name,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
            0o600,
            dir_fd=parent_fd,
        )
        created = os.fstat(descriptor)
        created_identity = (created.st_dev, created.st_ino)
        os.fchmod(descriptor, 0o600)
        view = memoryview(data)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise CatalogReportError("short write while publishing catalog report")
            view = view[written:]
        os.fsync(descriptor)
        os.close(descriptor)
        descriptor = None
        os.fsync(parent_fd)
    except BaseException as error:
        try:
            current = os.stat(path.name, dir_fd=parent_fd, follow_symlinks=False)
            if created_identity == (current.st_dev, current.st_ino):
                os.unlink(path.name, dir_fd=parent_fd)
        except OSError:
            pass
        if isinstance(error, OSError):
            raise CatalogReportError(f"cannot publish catalog report: {error}") from error
        raise
    finally:
        if descriptor is not None:
            os.close(descriptor)
        os.close(parent_fd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, required=True)
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--log", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        repo = args.repo.resolve(strict=True)
        log_path = args.log.resolve(strict=True)
        try:
            log_path.relative_to(repo)
        except ValueError:
            pass
        else:
            raise CatalogReportError("test log must be retained outside the source worktree")
        source_commit = _source_identity(repo, args.source_commit)
        log_bytes, log_sha256 = _read_log(args.log.absolute())
        report = build_report(log_bytes, log_sha256, source_commit)
        write_report(args.output, report, repo)
    except (OSError, ValueError, TypeError) as error:
        print(f"catalog_load_report.py: {error}", file=sys.stderr)
        return 2
    print(f"created source-bound catalog report: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
