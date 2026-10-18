#!/usr/bin/env python3
"""Assemble exact, immutable manifests for transfer-rebuild trial evidence."""

from __future__ import annotations

import argparse
import datetime as dt
import hashlib
import json
import math
import os
import re
import stat
import subprocess
import sys
import uuid
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator, FormatChecker


ROOT = Path(__file__).resolve().parents[2]
SCHEMA_PATH = ROOT / "certification/transfer-rebuild/manifest.schema.json"
TRIAL_SCHEMA_PATH = ROOT / "certification/transfer-rebuild/trial.schema.json"
MANIFEST_VERSION = "transfer-manifest-v1"
TRIAL_VERSION = "transfer-trial-v1"
ARTIFACT_KIND_RE = re.compile(r"^[a-z0-9][a-z0-9-]{0,95}$")
MAX_JSON_BYTES = 16 * 1024 * 1024
READ_CHUNK_BYTES = 1024 * 1024


class ManifestError(ValueError):
    """A trial input or immutable evidence manifest is invalid."""


def _strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise ManifestError(f"duplicate JSON field: {key}")
        result[key] = value
    return result


def _reject_constant(value: str) -> None:
    raise ManifestError(f"non-standard JSON number is not allowed: {value}")


def _strict_float(value: str) -> float:
    result = float(value)
    if not math.isfinite(result):
        raise ManifestError(f"JSON number is outside the finite range: {value}")
    return result


def _relative_path(root: Path, path: Path) -> str:
    path = Path(path)
    if ".." in path.parts:
        raise ManifestError(f"evidence path cannot contain '..': {path}")
    candidate = path if path.is_absolute() else Path.cwd() / path
    candidate = Path(os.path.abspath(candidate))
    try:
        relative = candidate.relative_to(root)
    except ValueError as error:
        raise ManifestError(
            f"evidence must live below the manifest directory: {path}"
        ) from error
    if not relative.parts or any(part in {"", ".", ".."} for part in relative.parts):
        raise ManifestError(f"evidence path is not normalized: {path}")
    return relative.as_posix()


def _require_external_evidence(root: Path, relative: str, repo: Path) -> None:
    """Keep generated evidence outside the source tree it describes."""
    candidate = root.joinpath(*Path(relative).parts)
    try:
        candidate.relative_to(repo)
    except ValueError:
        return
    raise ManifestError(f"evidence must be outside the measured source worktree: {relative}")


def _open_relative_regular(root: Path, relative: str) -> int:
    """Open a manifest-relative regular file without following any symlink."""
    if not hasattr(os, "O_NOFOLLOW") or os.open not in os.supports_dir_fd:
        raise ManifestError("this platform lacks no-follow directory-relative file opens")
    parts = Path(relative).parts
    if not parts or any(part in {"", ".", ".."} for part in parts):
        raise ManifestError("evidence path must be a normalized relative path")

    directory_flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW
    directory_flags |= getattr(os, "O_DIRECTORY", 0)
    current = os.open(root, directory_flags)
    try:
        for component in parts[:-1]:
            next_fd = os.open(component, directory_flags, dir_fd=current)
            metadata = os.fstat(next_fd)
            if not stat.S_ISDIR(metadata.st_mode):
                os.close(next_fd)
                raise ManifestError("evidence path contains a non-directory component")
            os.close(current)
            current = next_fd
        descriptor = os.open(
            parts[-1], os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW, dir_fd=current
        )
    except OSError as error:
        raise ManifestError(
            f"cannot open evidence without following links: {relative}: {error}"
        ) from error
    finally:
        os.close(current)

    metadata = os.fstat(descriptor)
    if not stat.S_ISREG(metadata.st_mode):
        os.close(descriptor)
        raise ManifestError(f"evidence must be a regular file: {relative}")
    return descriptor


def _file_identity(info: os.stat_result) -> tuple[int, int, int, int, int]:
    return (info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns)


def _read_and_hash(
    root: Path, relative: str, *, max_bytes: int | None = None
) -> tuple[bytes | None, str]:
    descriptor = _open_relative_regular(root, relative)
    digest = hashlib.sha256()
    content = bytearray() if max_bytes is not None else None
    try:
        before = os.fstat(descriptor)
        if max_bytes is not None and before.st_size > max_bytes:
            raise ManifestError(
                f"JSON evidence exceeds the {max_bytes}-byte size limit: {relative}"
            )
        while chunk := os.read(descriptor, READ_CHUNK_BYTES):
            digest.update(chunk)
            if content is not None:
                content.extend(chunk)
                if len(content) > max_bytes:
                    raise ManifestError(
                        f"JSON evidence exceeds the {max_bytes}-byte size limit: {relative}"
                    )
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    if _file_identity(before) != _file_identity(after):
        raise ManifestError(f"evidence changed while it was read: {relative}")
    return (bytes(content) if content is not None else None, digest.hexdigest())


def _load_json(data: bytes, label: str) -> Any:
    try:
        return json.loads(
            data,
            object_pairs_hook=_strict_object,
            parse_constant=_reject_constant,
            parse_float=_strict_float,
        )
    except (json.JSONDecodeError, UnicodeDecodeError, ManifestError) as error:
        raise ManifestError(f"{label} is not strict JSON: {error}") from error


def _git_identity(repo: Path) -> tuple[str, bool]:
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
    except subprocess.CalledProcessError as error:
        detail = error.stderr.strip() if error.stderr else str(error)
        raise ManifestError(f"cannot capture source identity: {detail}") from error
    except (OSError, subprocess.SubprocessError) as error:
        raise ManifestError(f"cannot capture source identity: {error}") from error

    if Path(top).resolve(strict=True) != repo:
        raise ManifestError(f"--repo must name the worktree root, not a subdirectory: {repo}")
    if re.fullmatch(r"[a-f0-9]{40}", commit) is None:
        raise ManifestError("Git returned an invalid source commit")
    return commit, bool(status)


def _validate_schema(value: Any, schema_path: Path, label: str) -> None:
    try:
        schema = json.loads(schema_path.read_bytes())
        Draft202012Validator.check_schema(schema)
    except (OSError, json.JSONDecodeError, UnicodeDecodeError) as error:
        raise ManifestError(f"cannot load {label} schema: {error}") from error
    errors = sorted(
        Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(value),
        key=lambda error: (list(map(str, error.absolute_path)), error.message),
    )
    if errors:
        messages = "; ".join(error.message for error in errors[:10])
        raise ManifestError(f"invalid {label}: {messages}")


def _trial_record(
    root: Path, path: Path, commit: str, dirty: bool, repo: Path
) -> tuple[dict[str, Any], str]:
    relative = _relative_path(root, path)
    _require_external_evidence(root, relative, repo)
    data, digest = _read_and_hash(root, relative, max_bytes=MAX_JSON_BYTES)
    if data is None:
        raise ManifestError(f"trial JSON was not read: {relative}")
    trial = _load_json(data, f"trial {relative}")
    _validate_schema(trial, TRIAL_SCHEMA_PATH, "transfer trial")
    if trial.get("schema_version") != TRIAL_VERSION:
        raise ManifestError(f"unsupported trial schema version in {relative}")
    if trial["identity"]["source_commit"] != commit:
        raise ManifestError(f"trial {relative} belongs to a different source commit")
    if trial["identity"]["dirty_worktree"] is not dirty:
        raise ManifestError(f"trial {relative} has a different dirty-worktree state")
    record = {
        "path": relative,
        "sha256": digest,
        "trial_id": trial["trial_id"],
        "pair_id": trial["pair_id"],
        "outcome": trial["outcome"]["status"],
    }
    return record, relative


def _artifact_record(root: Path, kind: str, path: Path, repo: Path) -> dict[str, str]:
    if not isinstance(kind, str) or ARTIFACT_KIND_RE.fullmatch(kind) is None:
        raise ManifestError(f"artifact kind is invalid: {kind!r}")
    relative = _relative_path(root, path)
    _require_external_evidence(root, relative, repo)
    _, digest = _read_and_hash(root, relative)
    return {"path": relative, "sha256": digest, "kind": kind}


def _fsync_directory(directory: Path) -> None:
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW
    flags |= getattr(os, "O_DIRECTORY", 0)
    descriptor = os.open(directory, flags)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _write_candidate(path: Path, data: bytes) -> None:
    descriptor = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
        0o600,
    )
    try:
        with os.fdopen(descriptor, "wb") as output:
            os.fchmod(output.fileno(), 0o600)
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        try:
            path.unlink()
        except FileNotFoundError:
            pass
        raise


def create_manifest(
    output: Path,
    trial_paths: list[Path],
    artifacts: list[tuple[str, Path]],
    *,
    repo: Path = ROOT,
) -> dict[str, Any]:
    """Create and validate a mode-0600 manifest without overwriting evidence."""
    if not trial_paths:
        raise ManifestError("at least one trial file is required")
    repo = Path(repo).resolve(strict=True)
    output = Path(output)
    if output.is_symlink():
        raise ManifestError("manifest output cannot be a symbolic link")
    parent = output.parent.resolve(strict=True)
    if not parent.is_dir():
        raise ManifestError("manifest output parent must be an existing directory")
    if parent == repo or repo in parent.parents:
        raise ManifestError("manifest output must be outside the measured source worktree")
    output = parent / output.name
    if output.exists() or output.is_symlink():
        raise ManifestError(f"refusing to overwrite manifest: {output}")

    source_commit, worktree_dirty = _git_identity(repo)
    if worktree_dirty:
        raise ManifestError(
            "source worktree is dirty; commit the measured source before creating a manifest"
        )
    trial_records: list[dict[str, Any]] = []
    seen_trial_ids: set[str] = set()
    input_digests: dict[str, str] = {}
    for trial_path in trial_paths:
        record, relative = _trial_record(
            parent, trial_path, source_commit, worktree_dirty, repo
        )
        if record["trial_id"] in seen_trial_ids:
            raise ManifestError(f"duplicate trial id: {record['trial_id']}")
        seen_trial_ids.add(record["trial_id"])
        input_digests[relative] = record["sha256"]
        trial_records.append(record)

    artifact_records: list[dict[str, str]] = []
    seen_paths = set(input_digests)
    for kind, artifact_path in artifacts:
        record = _artifact_record(parent, kind, artifact_path, repo)
        if record["path"] in seen_paths:
            raise ManifestError(f"evidence path was supplied more than once: {record['path']}")
        seen_paths.add(record["path"])
        input_digests[record["path"]] = record["sha256"]
        artifact_records.append(record)

    trial_records.sort(
        key=lambda item: (item["pair_id"], item["trial_id"], item["path"])
    )
    artifact_records.sort(key=lambda item: (item["kind"], item["path"]))
    created_at = (
        dt.datetime.now(dt.timezone.utc)
        .isoformat(timespec="microseconds")
        .replace("+00:00", "Z")
    )
    manifest = {
        "schema_version": MANIFEST_VERSION,
        "manifest_id": uuid.uuid4().hex,
        "created_at": created_at,
        "source_commit": source_commit,
        "worktree_dirty": worktree_dirty,
        "trial_files": trial_records,
        "artifacts": artifact_records,
    }
    _validate_schema(manifest, SCHEMA_PATH, "transfer manifest")

    candidate = parent / f".manifest-{uuid.uuid4().hex}.candidate"
    candidate_created = False
    try:
        serialized = json.dumps(
            manifest, sort_keys=True, indent=2, ensure_ascii=False, allow_nan=False
        ).encode("utf-8") + b"\n"
        _write_candidate(candidate, serialized)
        candidate_created = True
        transfer_tools = str(Path(__file__).resolve().parent)
        if transfer_tools not in sys.path:
            sys.path.insert(0, transfer_tools)
        from validate_artifact import validate

        validation_errors = validate("manifest", candidate, verify_referenced_files=True)
        if validation_errors:
            raise ManifestError(
                "manifest verification failed: " + "; ".join(validation_errors[:20])
            )

        for relative, expected_digest in input_digests.items():
            _, observed_digest = _read_and_hash(parent, relative)
            if observed_digest != expected_digest:
                raise ManifestError(f"evidence changed before manifest publication: {relative}")
        if _git_identity(repo) != (source_commit, worktree_dirty):
            raise ManifestError("source worktree changed while the manifest was being created")

        try:
            os.link(candidate, output, follow_symlinks=False)
        except OSError as error:
            raise ManifestError(
                f"cannot publish manifest exclusively at {output}: {error}"
            ) from error
        _fsync_directory(parent)
    finally:
        if candidate_created:
            try:
                candidate.unlink()
            except FileNotFoundError:
                pass

    return manifest


def _parse_artifact(value: str) -> tuple[str, Path]:
    kind, separator, raw_path = value.partition("=")
    if not separator or not raw_path:
        raise argparse.ArgumentTypeError("artifact must use KIND=PATH")
    if ARTIFACT_KIND_RE.fullmatch(kind) is None:
        raise argparse.ArgumentTypeError(
            "artifact KIND must use lowercase letters, digits, and hyphens"
        )
    return kind, Path(raw_path)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", type=Path, default=ROOT, help="exact measured Git worktree root")
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="new manifest path outside the source worktree",
    )
    parser.add_argument(
        "--trial",
        action="append",
        type=Path,
        required=True,
        help="trial JSON file; repeat for every outcome",
    )
    parser.add_argument(
        "--artifact",
        action="append",
        type=_parse_artifact,
        default=[],
        help="additional evidence as KIND=PATH",
    )
    args = parser.parse_args(argv)
    try:
        create_manifest(args.output, args.trial, args.artifact, repo=args.repo)
    except (ManifestError, OSError, ValueError) as error:
        print(f"transfer manifest: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
