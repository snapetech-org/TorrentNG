#!/usr/bin/env python3
"""Assemble an immutable, source-bound qualification wrapper from evidence digests."""

from __future__ import annotations

import argparse
import json
import os
import stat
import sys
from pathlib import Path
from typing import Any

try:
    from jsonschema import Draft202012Validator, FormatChecker
    from rollout_gate import REQUIRED_CHECKS
    from trial_record import _open_directory_tree_nofollow
    from strict_json import StrictJsonError, loads as strict_json_loads
    from validate_artifact import SCHEMAS
except ImportError as error:  # pragma: no cover - exercised by the CLI environment
    raise SystemExit("qualification_record.py must run from its scripts directory") from error


MAX_INPUT_BYTES = 4 * 1024 * 1024
GIT_SHA_LENGTH = 40


class QualificationRecordError(ValueError):
    """Qualification inputs cannot be represented as a trusted record."""


def _strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise QualificationRecordError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def _read_json(path: Path) -> Any:
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise QualificationRecordError(f"cannot open qualification input: {error}") from error
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > MAX_INPUT_BYTES:
            raise QualificationRecordError("qualification input must be a bounded regular file")
        content = bytearray()
        while len(content) <= MAX_INPUT_BYTES:
            chunk = os.read(descriptor, min(1024 * 1024, MAX_INPUT_BYTES + 1 - len(content)))
            if not chunk:
                break
            content.extend(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    identity = lambda value: (
        value.st_dev,
        value.st_ino,
        value.st_size,
        value.st_mtime_ns,
        value.st_ctime_ns,
    )
    if (
        len(content) > MAX_INPUT_BYTES
        or len(content) != metadata.st_size
        or identity(metadata) != identity(after)
    ):
        raise QualificationRecordError("qualification input changed while it was read")
    try:
        return strict_json_loads(bytes(content), label="qualification input")
    except (json.JSONDecodeError, UnicodeDecodeError, QualificationRecordError, StrictJsonError) as error:
        raise QualificationRecordError(f"qualification input is not strict JSON: {error}") from error


def build_record(kind: str, source_commit: str, checks: Any, notes: str = "") -> dict[str, Any]:
    if kind not in REQUIRED_CHECKS:
        raise QualificationRecordError(f"unknown qualification kind: {kind}")
    if (
        not isinstance(source_commit, str)
        or len(source_commit) != GIT_SHA_LENGTH
        or any(character not in "0123456789abcdef" for character in source_commit)
    ):
        raise QualificationRecordError("source commit must be a lowercase 40-character Git SHA")
    if not isinstance(checks, list) or not checks:
        raise QualificationRecordError("checks must be a nonempty array")
    if not isinstance(notes, str) or len(notes) > 8192:
        raise QualificationRecordError("notes must be a string no longer than 8192 characters")
    ids: set[str] = set()
    for check in checks:
        if not isinstance(check, dict) or set(check) != {"id", "status", "evidence_sha256s"}:
            raise QualificationRecordError("each check must contain id, status, and evidence_sha256s")
        check_id = check["id"]
        if not isinstance(check_id, str) or not check_id or check_id in ids:
            raise QualificationRecordError("check IDs must be nonempty and unique")
        ids.add(check_id)
        status = check["status"]
        if not isinstance(status, str) or status not in {"passed", "failed", "not_run"}:
            raise QualificationRecordError(f"check {check_id} has an invalid status")
        digests = check["evidence_sha256s"]
        if not isinstance(digests, list) or not digests:
            raise QualificationRecordError(f"check {check_id} must link to evidence digests")
        if any(
            not isinstance(digest, str)
            or len(digest) != 64
            or any(character not in "0123456789abcdef" for character in digest)
            for digest in digests
        ) or len(set(digests)) != len(digests):
            raise QualificationRecordError(f"check {check_id} has duplicate or invalid evidence digests")
    missing = REQUIRED_CHECKS[kind] - ids
    status = "passed" if not missing and all(check["status"] == "passed" for check in checks) else "blocked"
    record = {
        "schema_version": "transfer-qualification-v1",
        "kind": kind,
        "source_commit": source_commit,
        "status": status,
        "checks": checks,
        "notes": notes,
    }
    try:
        schema = json.loads(SCHEMAS["qualification"].read_bytes())
    except (OSError, json.JSONDecodeError) as error:
        raise QualificationRecordError(f"cannot read qualification schema: {error}") from error
    validation_errors = sorted(
        Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(record),
        key=lambda error: (list(map(str, error.absolute_path)), error.message),
    )
    if validation_errors:
        raise QualificationRecordError("qualification record is invalid: " + "; ".join(
            error.message for error in validation_errors[:10]
        ))
    return record


def write_record(path: Path, record: dict[str, Any]) -> None:
    data = (json.dumps(record, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()
    path = Path(path)
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise QualificationRecordError("qualification output path must be absolute and normalized")
    repo_root = Path(__file__).resolve().parents[2]
    try:
        path.relative_to(repo_root)
    except ValueError:
        pass
    else:
        raise QualificationRecordError("qualification output must be outside the source worktree")
    try:
        parent_fd = _open_directory_tree_nofollow(path.parent)
    except OSError as error:
        raise QualificationRecordError("qualification output parent must be an existing no-symlink directory") from error
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW
    descriptor: int | None = None
    created_identity: tuple[int, int] | None = None
    try:
        descriptor = os.open(path.name, flags, 0o600, dir_fd=parent_fd)
        created = os.fstat(descriptor)
        created_identity = (created.st_dev, created.st_ino)
        os.fchmod(descriptor, 0o600)
        view = memoryview(data)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise QualificationRecordError("short write while publishing qualification record")
            view = view[written:]
        os.fsync(descriptor)
    except BaseException as error:
        try:
            current = os.stat(path.name, dir_fd=parent_fd, follow_symlinks=False)
            if created_identity == (current.st_dev, current.st_ino):
                os.unlink(path.name, dir_fd=parent_fd)
        except OSError:
            pass
        os.close(parent_fd)
        if isinstance(error, OSError):
            raise QualificationRecordError(f"cannot publish qualification record: {error}") from error
        raise
    finally:
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError:
                pass
    try:
        os.fsync(parent_fd)
    finally:
        os.close(parent_fd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kind", required=True, choices=sorted(REQUIRED_CHECKS))
    parser.add_argument("--source-commit", required=True)
    parser.add_argument("--checks", required=True, type=Path)
    parser.add_argument("--notes", default="")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        record = build_record(args.kind, args.source_commit, _read_json(args.checks), args.notes)
        write_record(args.output, record)
    except (OSError, ValueError, TypeError) as error:
        print(f"qualification_record.py: {error}", file=sys.stderr)
        return 2
    print(f"{record['status']}: {args.output}")
    return 0 if record["status"] == "passed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
