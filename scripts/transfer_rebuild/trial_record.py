#!/usr/bin/env python3
"""Bind paired-plan, client, fixture, timing, and observer data into a trial.

This module constructs `transfer-trial-v1` records but does not launch clients
or claim that an unmeasured run completed. The linked manifest remains the
authority for artifact digests and referenced-file verification.
"""

from __future__ import annotations

import datetime as dt
import hashlib
import json
import os
import platform
import re
import stat
import subprocess
import sys
from pathlib import Path
from typing import Any

from jsonschema import Draft202012Validator, FormatChecker, SchemaError


TRIAL_SCHEMA_VERSION = "transfer-trial-v1"
SCHEMA_PATH = Path(__file__).resolve().parents[2] / "certification/transfer-rebuild/trial.schema.json"
SHA256_RE = re.compile(r"^[a-f0-9]{64}$")
GIT_SHA_RE = re.compile(r"^[a-f0-9]{40}$")
LOCKSET_FORMAT = "transfer-harness-lockset-v1"
LOCKSET_PATHS = ("Cargo.lock", "scripts/transfer_rebuild/requirements.txt")
RESOURCE_FIELDS = {
    "rss_peak_bytes": "rss_bytes",
    "pss_peak_bytes": "pss_bytes",
    "cgroup_anon_peak_bytes": "cgroup_anon_bytes",
    "cgroup_file_peak_bytes": "cgroup_file_bytes",
    "cgroup_reclaimable_cache_peak_bytes": "cgroup_reclaimable_cache_bytes",
    "cgroup_current_peak_bytes": "cgroup_current_bytes",
    "cpu_time_ns": "cpu_time_ns",
    "threads_peak": "threads",
    "fds_peak": "fds",
    "block_read_bytes": "block_read_bytes",
    "block_write_bytes": "block_write_bytes",
}
CLIENT_OBSERVATION_SCHEMA_PATH = (
    Path(__file__).resolve().parents[2]
    / "certification/transfer-rebuild/client-observation.schema.json"
)
RESOURCE_OBSERVATION_SCHEMA_PATH = (
    Path(__file__).resolve().parents[2]
    / "certification/transfer-rebuild/resource-observation.schema.json"
)
DURATION_FIELDS = ("barrier_wait", "transfer", "flush", "validation", "shutdown")
PAYLOAD_FIELDS = (
    "useful_bytes",
    "received_bytes",
    "written_bytes",
    "read_bytes",
    "uploaded_bytes",
    "duplicate_bytes",
)
REQUIRED_PAYLOAD_FIELDS = {"received_bytes", "written_bytes", "read_bytes", "duplicate_bytes"}
OPTIONAL_PAYLOAD_FIELDS = {
    "useful_bytes",
    "received_bytes",
    "written_bytes",
    "read_bytes",
    "uploaded_bytes",
    "duplicate_bytes",
}


class TrialRecordError(ValueError):
    """A trial record is missing or mixes incompatible evidence."""


def _strict_nonnegative_integer(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise TrialRecordError(f"{label} must be a nonnegative integer")
    return value


def _valid_sha256(value: Any, label: str) -> str:
    if not isinstance(value, str) or SHA256_RE.fullmatch(value) is None:
        raise TrialRecordError(f"{label} must be a lowercase SHA-256 digest")
    return value


def _read_regular_nofollow(path: Path, label: str, *, limit_bytes: int) -> tuple[bytes, os.stat_result]:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise TrialRecordError(f"cannot open {label} without following links") from error
    content = bytearray()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit_bytes:
            raise TrialRecordError(f"{label} must be a bounded regular file")
        while len(content) <= limit_bytes:
            chunk = os.read(descriptor, min(1024 * 1024, limit_bytes + 1 - len(content)))
            if not chunk:
                break
            content.extend(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)

    before_identity = (
        before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns
    )
    after_identity = (
        after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns
    )
    if len(content) > limit_bytes or len(content) != before.st_size or before_identity != after_identity:
        raise TrialRecordError(f"{label} changed while it was read")
    return bytes(content), before


def _hash_regular_nofollow(path: Path, label: str) -> tuple[str, os.stat_result]:
    """Hash one stable regular file without buffering its contents."""
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise TrialRecordError(f"cannot open {label} without following links") from error
    digest = hashlib.sha256()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise TrialRecordError(f"{label} must be a regular file")
        byte_count = 0
        while chunk := os.read(descriptor, 1024 * 1024):
            digest.update(chunk)
            byte_count += len(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)

    before_identity = (
        before.st_dev,
        before.st_ino,
        before.st_size,
        before.st_mtime_ns,
        before.st_ctime_ns,
    )
    after_identity = (
        after.st_dev,
        after.st_ino,
        after.st_size,
        after.st_mtime_ns,
        after.st_ctime_ns,
    )
    if byte_count != before.st_size or before_identity != after_identity:
        raise TrialRecordError(f"{label} changed while it was hashed")
    return digest.hexdigest(), before


def _git_output(repo: Path, *arguments: str) -> str:
    try:
        result = subprocess.run(
            ["git", "-C", str(repo), *arguments],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise TrialRecordError("cannot capture the measurement harness Git identity") from error
    return result.stdout.strip()


def capture_harness_identity(repo_root: Path) -> dict[str, Any]:
    """Capture the runner executable, source revision, toolchain, and lockset."""
    repo = Path(repo_root).resolve(strict=True)
    top = Path(_git_output(repo, "rev-parse", "--show-toplevel")).resolve(strict=True)
    if top != repo:
        raise TrialRecordError("repo_root must name the TorrentNG worktree root")
    source_commit = _git_output(repo, "rev-parse", "--verify", "HEAD^{commit}")
    if GIT_SHA_RE.fullmatch(source_commit) is None:
        raise TrialRecordError("Git returned an invalid 40-character source commit")
    dirty = bool(_git_output(repo, "status", "--porcelain=v1", "--untracked-files=all"))

    lockset_files: dict[str, str] = {}
    for relative in LOCKSET_PATHS:
        contents, _metadata = _read_regular_nofollow(
            repo / relative,
            f"harness lockset file {relative}",
            limit_bytes=64 * 1024 * 1024,
        )
        lockset_files[relative] = hashlib.sha256(contents).hexdigest()
    lockset = {
        "format": LOCKSET_FORMAT,
        "files": lockset_files,
    }
    lockfile_sha256 = hashlib.sha256(
        json.dumps(lockset, sort_keys=True, separators=(",", ":")).encode("utf-8")
    ).hexdigest()

    executable = Path(sys.executable).resolve(strict=True)
    executable_sha256, executable_metadata = _hash_regular_nofollow(
        executable, "measurement Python interpreter"
    )
    return {
        "source_commit": source_commit,
        "dirty_worktree": dirty,
        "lockfile_sha256": lockfile_sha256,
        "binary_sha256": executable_sha256,
        "binary_size_bytes": executable_metadata.st_size,
        "binary_path": str(executable),
        "image_digest": None,
        "toolchain": f"{platform.python_implementation()} {platform.python_version()}",
    }


def _validate_timestamp(value: Any, label: str) -> str:
    if not isinstance(value, str) or not value:
        raise TrialRecordError(f"{label} must be an ISO-8601 timestamp")
    try:
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
    except ValueError as error:
        raise TrialRecordError(f"{label} must be an ISO-8601 timestamp") from error
    if parsed.tzinfo is None or parsed.utcoffset() is None:
        raise TrialRecordError(f"{label} must include a timezone")
    return value


def _client_fields(
    planned_trial: dict[str, Any],
    outcome_status: str,
    observation: dict[str, Any] | None,
    observation_sha256: str | None,
) -> tuple[dict[str, Any] | None, dict[str, Any]]:
    if observation is None:
        if outcome_status != "unmeasured":
            raise TrialRecordError("a trial without a client observation must be unmeasured")
        if observation_sha256 is not None:
            raise TrialRecordError("client observation digest was supplied without an observation")
        return None, {"captured": False, "values": {}, "artifact_sha256": None}

    if outcome_status == "unmeasured":
        raise TrialRecordError("an unmeasured trial cannot contain a client process observation")
    _validate_document_schema(
        observation,
        CLIENT_OBSERVATION_SCHEMA_PATH,
        "client observation",
    )
    if observation.get("client_id") != planned_trial.get("client_id"):
        raise TrialRecordError("client observation does not match the scheduled client")
    digest = _valid_sha256(observation_sha256, "client observation artifact digest")
    runtime = observation.get("runtime_identity")
    settings = observation.get("settings_readback")
    if not isinstance(runtime, dict) or not isinstance(settings, dict):
        raise TrialRecordError("client observation is missing runtime identity or settings")
    if settings.get("captured") is not True or not isinstance(settings.get("values"), dict):
        raise TrialRecordError("client settings observation is not captured")
    process_id = runtime.get("process_id")
    start_ticks = runtime.get("process_start_ticks")
    if isinstance(process_id, bool) or not isinstance(process_id, int) or process_id <= 0:
        raise TrialRecordError("client observation process ID is invalid")
    _strict_nonnegative_integer(start_ticks, "client process start time")
    settings_sha256 = _valid_sha256(settings.get("sha256"), "settings digest")
    result = {
        "client_id": observation["client_id"],
        "version": observation["version"],
        "runtime_kind": runtime.get("kind"),
        "process_id": process_id,
        "process_start_ticks": start_ticks,
        "observation_sha256": digest,
        "settings_sha256": settings_sha256,
    }
    settings_result = {
        "captured": True,
        "values": settings["values"],
        "artifact_sha256": digest,
    }
    return result, settings_result


def _resource_fields(
    client_identity: dict[str, Any] | None,
    observation: dict[str, Any] | None,
    observation_sha256: str | None,
) -> dict[str, Any]:
    if observation is None:
        if observation_sha256 is not None:
            raise TrialRecordError("resource observation digest was supplied without an observation")
        return {
            "observer_id": "not_run",
            "sample_interval_ms": None,
            **{field: None for field in RESOURCE_FIELDS},
            "network_rx_bytes": None,
            "network_tx_bytes": None,
            "energy_joules": None,
            "raw_artifact_sha256": None,
        }

    digest = _valid_sha256(observation_sha256, "resource observation artifact digest")
    _validate_document_schema(
        observation,
        RESOURCE_OBSERVATION_SCHEMA_PATH,
        "resource observation",
    )
    if client_identity is None:
        raise TrialRecordError("resource observation requires a client process identity")
    if observation.get("root_pid") != client_identity["process_id"]:
        raise TrialRecordError("resource observer root PID differs from the client")
    if observation.get("root_process_start_ticks") != client_identity["process_start_ticks"]:
        raise TrialRecordError("resource observer start time differs from the client")
    _strict_nonnegative_integer(observation.get("sample_count"), "resource sample count")
    if observation["sample_count"] < 1:
        raise TrialRecordError("resource observation must contain at least one sample")
    interval = _strict_nonnegative_integer(
        observation.get("sample_interval_ms"), "resource sample interval"
    )
    if interval < 1:
        raise TrialRecordError("resource sample interval must be positive")
    peaks = observation.get("peaks")
    if not isinstance(peaks, dict):
        raise TrialRecordError("resource observation is missing its peak counters")

    result = {
        "observer_id": observation.get("observer_id"),
        "sample_interval_ms": interval,
        **{
            trial_field: peaks.get(observation_field)
            for trial_field, observation_field in RESOURCE_FIELDS.items()
        },
        "network_rx_bytes": observation.get("network_rx_bytes"),
        "network_tx_bytes": observation.get("network_tx_bytes"),
        "energy_joules": None,
        "raw_artifact_sha256": digest,
    }
    if not isinstance(result["observer_id"], str) or not result["observer_id"]:
        raise TrialRecordError("resource observer identity is invalid")
    return result


def _validate_trial_schema(trial: dict[str, Any]) -> None:
    _validate_document_schema(trial, SCHEMA_PATH, "transfer trial")


def _validate_trial_record(trial: dict[str, Any]) -> None:
    """Validate the schema and outcome-specific measurement invariants."""
    _validate_trial_schema(trial)
    integrity = trial["integrity"]
    if integrity["status"] == "passed":
        expected = _valid_sha256(integrity["expected_sha256"], "expected payload digest")
        observed = _valid_sha256(integrity["observed_sha256"], "observed payload digest")
        if expected != observed:
            raise TrialRecordError("passed integrity evidence must have matching payload digests")
        if integrity["hash_failures"] != 0 or integrity["false_completions"] != 0:
            raise TrialRecordError("passed integrity evidence cannot record a hash failure or false completion")

    if trial["outcome"]["status"] == "complete":
        expected_bytes = _strict_nonnegative_integer(
            trial["workload"]["expected_payload_bytes"], "workload expected payload bytes"
        )
        useful_bytes = _strict_nonnegative_integer(
            trial["payload"]["useful_bytes"], "payload useful bytes"
        )
        if useful_bytes != expected_bytes:
            raise TrialRecordError("a complete trial must account for the full useful payload")
        if trial["durations_ns"]["transfer"] < 1:
            raise TrialRecordError("a complete transfer must have a positive transfer duration")
        if trial["resources"].get("raw_artifact_sha256") is None:
            raise TrialRecordError("a complete trial requires linked independent resource observation")
        if expected_bytes > 0 and trial["integrity"]["status"] != "passed":
            raise TrialRecordError("a complete payload transfer requires passed integrity evidence")


def _validate_document_schema(value: Any, schema_path: Path, label: str) -> None:
    try:
        schema = json.loads(schema_path.read_bytes())
        Draft202012Validator.check_schema(schema)
    except (OSError, json.JSONDecodeError, UnicodeDecodeError, SchemaError) as error:
        raise TrialRecordError(f"cannot load {label} schema: {error}") from error
    errors = sorted(
        Draft202012Validator(schema, format_checker=FormatChecker()).iter_errors(value),
        key=lambda error: (list(map(str, error.absolute_path)), error.message),
    )
    if errors:
        message = "; ".join(error.message for error in errors[:10])
        raise TrialRecordError(f"{label} does not match its schema: {message}")


def _open_directory_tree_nofollow(path: Path) -> int:
    """Open an absolute directory path one component at a time without links."""
    if not path.is_absolute() or ".." in path.parts:
        raise TrialRecordError("trial output parent must be absolute and normalized")
    if (
        not hasattr(os, "O_NOFOLLOW")
        or not hasattr(os, "O_DIRECTORY")
        or os.open not in os.supports_dir_fd
    ):
        raise TrialRecordError("this platform lacks no-follow directory-relative file opens")
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
    try:
        current = os.open(path.anchor, flags)
        for component in path.parts[1:]:
            next_fd = os.open(component, flags, dir_fd=current)
            metadata = os.fstat(next_fd)
            if not stat.S_ISDIR(metadata.st_mode):
                os.close(next_fd)
                raise TrialRecordError("trial output path contains a non-directory component")
            os.close(current)
            current = next_fd
        return current
    except BaseException:
        if "current" in locals():
            try:
                os.close(current)
            except OSError:
                pass
        raise


def build_trial_record(
    *,
    planned_trial: dict[str, Any],
    harness_identity: dict[str, Any],
    client_observation: dict[str, Any] | None,
    client_observation_sha256: str | None,
    workload: dict[str, Any],
    environment: dict[str, Any],
    durations_ns: dict[str, int],
    payload: dict[str, Any],
    resource_observation: dict[str, Any] | None,
    resource_observation_sha256: str | None,
    integrity: dict[str, Any],
    outcome: dict[str, Any],
    started_at: str,
    finished_at: str,
) -> dict[str, Any]:
    """Assemble one trial and reject mismatched process or evidence identities."""
    required_plan_fields = {"trial_id", "pair_id", "attempt", "case_id", "client_id"}
    if not isinstance(planned_trial, dict) or not required_plan_fields <= set(planned_trial):
        raise TrialRecordError("planned trial is missing its scheduled identity")
    for field in ("trial_id", "pair_id", "case_id", "client_id"):
        if not isinstance(planned_trial[field], str) or not planned_trial[field]:
            raise TrialRecordError(f"planned trial {field} must be a nonempty string")
    attempt = planned_trial["attempt"]
    if isinstance(attempt, bool) or not isinstance(attempt, int) or attempt < 1:
        raise TrialRecordError("trial attempt must be a positive integer")
    if not isinstance(outcome, dict):
        raise TrialRecordError("trial outcome must be an object")
    outcome_status = outcome.get("status")
    if not isinstance(outcome_status, str) or outcome_status not in {
        "complete",
        "incomplete",
        "invalid",
        "failed",
        "unmeasured",
    }:
        raise TrialRecordError("trial outcome has an unknown status")
    if not isinstance(harness_identity, dict):
        raise TrialRecordError("measurement harness identity is required")
    if not isinstance(workload, dict) or workload.get("case_id") != planned_trial["case_id"]:
        raise TrialRecordError("workload case does not match the planned trial")
    if not isinstance(environment, dict):
        raise TrialRecordError("trial environment must be an object")
    if not isinstance(integrity, dict):
        raise TrialRecordError("trial integrity evidence must be an object")

    start = _validate_timestamp(started_at, "started_at")
    finish = _validate_timestamp(finished_at, "finished_at")
    if dt.datetime.fromisoformat(finish.replace("Z", "+00:00")) < dt.datetime.fromisoformat(
        start.replace("Z", "+00:00")
    ):
        raise TrialRecordError("finished_at precedes started_at")

    if not isinstance(durations_ns, dict) or set(durations_ns) != set(DURATION_FIELDS):
        raise TrialRecordError("trial durations must contain exactly the five contracted phases")
    duration_values = {
        name: _strict_nonnegative_integer(durations_ns[name], f"duration {name}")
        for name in DURATION_FIELDS
    }
    if outcome_status == "complete" and duration_values["transfer"] == 0:
        raise TrialRecordError("a complete transfer must have a positive transfer duration")

    if not isinstance(payload, dict) or not REQUIRED_PAYLOAD_FIELDS <= set(payload):
        raise TrialRecordError("payload accounting is missing required byte counters")
    payload_result: dict[str, Any] = {}
    for name in PAYLOAD_FIELDS:
        value = payload.get(name)
        if name in OPTIONAL_PAYLOAD_FIELDS and value is None:
            payload_result[name] = None
        else:
            payload_result[name] = _strict_nonnegative_integer(value, f"payload {name}")
    expected_payload_bytes = _strict_nonnegative_integer(
        workload.get("expected_payload_bytes"), "workload expected payload bytes"
    )
    if (
        payload_result["useful_bytes"] is not None
        and payload_result["useful_bytes"] > expected_payload_bytes
    ):
        raise TrialRecordError("useful payload bytes exceed the workload payload")
    if (
        payload_result["received_bytes"] is not None
        and payload_result["useful_bytes"] is not None
        and payload_result["received_bytes"] < payload_result["useful_bytes"]
    ):
        raise TrialRecordError("received bytes cannot be less than useful payload bytes")
    if (
        payload_result["duplicate_bytes"] is not None
        and payload_result["received_bytes"] is not None
        and payload_result["duplicate_bytes"] > payload_result["received_bytes"]
    ):
        raise TrialRecordError("duplicate bytes cannot exceed received bytes")
    if outcome_status == "complete":
        useful_bytes = _strict_nonnegative_integer(
            payload_result["useful_bytes"], "payload useful bytes"
        )
        if useful_bytes != expected_payload_bytes:
            raise TrialRecordError("a complete trial must account for the full useful payload")
    if "copy_accounting" in payload:
        payload_result["copy_accounting"] = payload["copy_accounting"]

    client_identity, settings_readback = _client_fields(
        planned_trial,
        outcome_status,
        client_observation,
        client_observation_sha256,
    )
    resources = _resource_fields(
        client_identity, resource_observation, resource_observation_sha256
    )
    integrity_status = integrity.get("status")
    if not isinstance(integrity_status, str) or integrity_status not in {
        "passed",
        "failed",
        "not_applicable",
        "not_run",
    }:
        raise TrialRecordError("trial integrity evidence has an unknown status")
    if outcome_status == "complete" and resources["raw_artifact_sha256"] is None:
        raise TrialRecordError("a complete trial requires linked independent resource observation")
    if outcome_status == "complete" and integrity_status not in {"passed", "not_applicable"}:
        raise TrialRecordError("a complete trial requires passing applicable integrity evidence")
    if outcome_status == "complete" and expected_payload_bytes > 0 and integrity_status != "passed":
        raise TrialRecordError("a complete payload transfer requires passed integrity evidence")
    if integrity_status == "passed":
        expected_digest = _valid_sha256(integrity.get("expected_sha256"), "expected payload digest")
        observed_digest = _valid_sha256(integrity.get("observed_sha256"), "observed payload digest")
        if expected_digest != observed_digest:
            raise TrialRecordError("passed integrity evidence must have matching payload digests")
        if integrity.get("hash_failures") != 0 or integrity.get("false_completions") != 0:
            raise TrialRecordError("passed integrity evidence cannot record a hash failure or false completion")

    trial = {
        "schema_version": TRIAL_SCHEMA_VERSION,
        "trial_id": planned_trial["trial_id"],
        "pair_id": planned_trial["pair_id"],
        "attempt": attempt,
        "started_at": start,
        "finished_at": finish,
        "identity": harness_identity,
        "client_identity": client_identity,
        "workload": workload,
        "environment": environment,
        "settings_readback": settings_readback,
        "durations_ns": duration_values,
        "payload": payload_result,
        "resources": resources,
        "integrity": integrity,
        "outcome": outcome,
    }
    _validate_trial_record(trial)
    return trial


def write_trial(path: Path, trial: dict[str, Any]) -> str:
    """Validate and publish one new mode-0600 trial artifact without overwrite."""
    _validate_trial_record(trial)
    path = Path(path)
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise TrialRecordError("trial output path must be absolute and normalized")
    repo_root = Path(__file__).resolve().parents[2]
    try:
        path.relative_to(repo_root)
    except ValueError:
        pass
    else:
        raise TrialRecordError("trial artifacts must be written outside the source worktree")
    try:
        encoded = (
            json.dumps(
                trial,
                sort_keys=True,
                indent=2,
                ensure_ascii=False,
                allow_nan=False,
            )
            + "\n"
        ).encode("utf-8")
    except (TypeError, ValueError, UnicodeEncodeError) as error:
        raise TrialRecordError(f"trial cannot be encoded as strict JSON: {error}") from error

    try:
        parent_fd = _open_directory_tree_nofollow(path.parent)
    except OSError as error:
        raise TrialRecordError("trial output parent must be an existing no-symlink directory") from error
    file_fd: int | None = None
    created_identity: tuple[int, int] | None = None
    try:
        try:
            flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW
            file_fd = os.open(path.name, flags, 0o600, dir_fd=parent_fd)
            os.fchmod(file_fd, 0o600)
            created = os.fstat(file_fd)
            created_identity = (created.st_dev, created.st_ino)
            if not stat.S_ISREG(created.st_mode):
                raise TrialRecordError("trial output must be a regular file")
            view = memoryview(encoded)
            while view:
                written = os.write(file_fd, view)
                if written <= 0:
                    raise TrialRecordError("short write while publishing trial artifact")
                view = view[written:]
            os.fsync(file_fd)
            os.close(file_fd)
            file_fd = None
        except BaseException as error:
            if file_fd is not None:
                try:
                    os.close(file_fd)
                except OSError:
                    pass
                finally:
                    file_fd = None
            if created_identity is not None:
                try:
                    current = os.stat(path.name, dir_fd=parent_fd, follow_symlinks=False)
                    if (current.st_dev, current.st_ino) == created_identity:
                        os.unlink(path.name, dir_fd=parent_fd)
                except OSError:
                    pass
            if isinstance(error, OSError):
                raise TrialRecordError(f"cannot create a new trial artifact: {error}") from error
            raise
        os.fsync(parent_fd)
    finally:
        if file_fd is not None:
            try:
                os.close(file_fd)
            except OSError:
                pass
        os.close(parent_fd)
    return hashlib.sha256(encoded).hexdigest()
