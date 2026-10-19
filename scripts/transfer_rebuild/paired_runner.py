#!/usr/bin/env python3
"""Execute pinned PERF-110 download and seed trials from a paired plan.

The runner creates one private task network per workload, starts a fresh
candidate and helper process, and retains trial, identity, topology, tracker,
cache, network-control, and independent resource evidence. It deliberately
leaves unmeasured counters null.
"""

from __future__ import annotations

import argparse
import copy
import datetime as dt
import hashlib
import json
import os
import platform
import re
import shutil
import signal
import stat
import subprocess
import sys
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path, PurePosixPath
from typing import Any, Mapping

from adapters import AdapterError, load_pins_document, probe_client, write_new_json
from cache_control import prepare_cache_state, write_record as write_cache_record
from client_control import ClientControlError, ClientController, TorrentState
from fixtures import FixtureFile, generate_fixture, verify_fixture
from managed_client import (
    ManagedClient,
    ManagedClientError,
    bind_client_topology,
    launch_client,
    launch_container_unattached,
)
from manifest import create_manifest
from measurement import FileStartBarrier
from metainfo import retarget_fixture_torrent
from network_control import PROFILES as NETWORK_PROFILES
from network_control import apply_profile, derive_pair_seed as _network_seed, reset_profile
from paired_plan import paired_plan_errors
from process_supervisor import ManagedProcess, ProcessSupervisorError
from task_network import TaskNetworkError, cleanup_topology_record, create_topology
from strict_json import StrictJsonError, loads as strict_json_loads
from task_tracker import TaskTracker
from trial_record import (
    TrialRecordError,
    build_trial_record,
    capture_harness_identity,
    write_trial,
)
from validate_artifact import validate as validate_artifact


ROOT = Path(__file__).resolve().parents[2]
DEFAULT_PROFILE_PATH = Path(__file__).with_name("fixture_profiles.json")
RUNNER_SCHEMA_VERSION = "transfer-paired-runner-v1"
DEFAULT_TIMEOUT_SECONDS = 1800
POLL_SECONDS = 0.1
MAX_JSON_BYTES = 16 * 1024 * 1024
SCHEDULED_TRIAL_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_./:-]{0,180}$")
SHA256_RE = re.compile(r"^[a-f0-9]{64}$")
INFO_HASH_RE = re.compile(r"^(?:[a-f0-9]{40}|[a-f0-9]{64})$")
ENVIRONMENT_FIELDS = {
    "host_profile",
    "os",
    "kernel",
    "cpu_model",
    "cgroup_version",
    "memory_limit_bytes",
    "storage",
    "network",
    "independent_ceiling_sha256",
    "cache_state",
    "cache_state_observation_sha256",
    "durability_mode",
}


class PairedRunnerError(RuntimeError):
    """A paired workload cannot be run or its evidence is incomplete."""


@dataclass(frozen=True)
class RuntimePaths:
    root: Path
    data: Path
    state: Path
    log: Path
    pid: Path
    env: Path
    tracker_url: str = ""
    peer_address: str = ""

    def as_dict(self) -> dict[str, Any]:
        return {
            "root": self.root,
            "data": self.data,
            "state": self.state,
            "log": self.log,
            "pid": self.pid,
            "env": self.env,
            "tracker_url": self.tracker_url,
            "peer_address": self.peer_address,
        }


@dataclass
class Endpoint:
    role: str
    client_id: str
    profile: dict[str, Any]
    paths: RuntimePaths
    managed: ManagedClient | None = None
    direct_process: ManagedProcess | None = None
    controller: ClientController | None = None
    observation: dict[str, Any] | None = None
    observation_path: Path | None = None
    observation_sha256: str | None = None
    direct_profile: dict[str, Any] | None = None
    direct_result: dict[str, Any] | None = None

    @property
    def pid(self) -> int:
        if self.managed is not None:
            return self.managed.pid
        if self.direct_process is not None:
            return self.direct_process.pid
        raise PairedRunnerError(f"{self.role} process has not started")


@dataclass
class RunArtifacts:
    trials: list[Path] = field(default_factory=list)
    evidence: dict[tuple[str, Path], None] = field(default_factory=dict)

    def add(self, kind: str, path: Path) -> None:
        self.evidence[(kind, path)] = None

    def manifest_artifacts(self) -> list[tuple[str, Path]]:
        return sorted(
            self.evidence,
            key=lambda item: (item[0], str(item[1])),
        )


def _read_regular(path: Path, label: str, *, limit: int = MAX_JSON_BYTES) -> bytes:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise PairedRunnerError(f"cannot open {label} without following links") from error
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit:
            raise PairedRunnerError(f"{label} must be a bounded regular file")
        content = bytearray()
        while len(content) <= limit:
            chunk = os.read(descriptor, min(1024 * 1024, limit + 1 - len(content)))
            if not chunk:
                break
            content.extend(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    identity = lambda info: (
        info.st_dev,
        info.st_ino,
        info.st_size,
        info.st_mtime_ns,
        info.st_ctime_ns,
    )
    if len(content) != before.st_size or len(content) > limit or identity(before) != identity(after):
        raise PairedRunnerError(f"{label} changed while it was read")
    return bytes(content)


def _scheduled_trial_key(trial_id: str, mode: str) -> str:
    """Hash one schedule identity into a filesystem-safe key.

    Catalog profile IDs include slashes and repetition IDs include hyphens.
    The scheduled identity is metadata only; hashing it keeps it out of paths.
    """
    trial_suffix = f"{trial_id}:{mode}"
    if SCHEDULED_TRIAL_ID_RE.fullmatch(trial_suffix) is None:
        raise PairedRunnerError("scheduled trial identity contains unsafe characters")
    return hashlib.sha256(trial_suffix.encode("utf-8")).hexdigest()[:20]


def _load_json(path: Path, label: str) -> tuple[Any, bytes]:
    raw = _read_regular(path, label)
    try:
        value = strict_json_loads(raw, label=label)
    except StrictJsonError as error:
        raise PairedRunnerError(f"{label} is not strict UTF-8 JSON: {error}") from error
    return value, raw


def _sha256_file(path: Path, label: str) -> str:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise PairedRunnerError(f"cannot open {label} without following links") from error
    digest = hashlib.sha256()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise PairedRunnerError(f"{label} must be a regular file")
        byte_count = 0
        while chunk := os.read(descriptor, 1024 * 1024):
            digest.update(chunk)
            byte_count += len(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    identity = lambda info: (
        info.st_dev,
        info.st_ino,
        info.st_size,
        info.st_mtime_ns,
        info.st_ctime_ns,
    )
    if byte_count != before.st_size or identity(before) != identity(after):
        raise PairedRunnerError(f"{label} changed while it was hashed")
    return digest.hexdigest()


def _write_private_json(path: Path, value: dict[str, Any]) -> str:
    if path.exists() or path.is_symlink():
        raise PairedRunnerError(f"refusing to overwrite evidence file: {path}")
    encoded = (
        json.dumps(value, sort_keys=True, indent=2, ensure_ascii=False, allow_nan=False) + "\n"
    ).encode("utf-8")
    descriptor = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
        0o600,
    )
    try:
        os.fchmod(descriptor, 0o600)
        view = memoryview(encoded)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise PairedRunnerError("short write while publishing runner evidence")
            view = view[written:]
        os.fsync(descriptor)
    except BaseException:
        os.close(descriptor)
        try:
            path.unlink()
        except OSError:
            pass
        raise
    else:
        os.close(descriptor)
    parent_fd = os.open(path.parent, os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(parent_fd)
    finally:
        os.close(parent_fd)
    return hashlib.sha256(encoded).hexdigest()


def _start_task_tracker_worker(
    tracker: TaskTracker,
    topology_record: dict[str, Any],
    worker_role: str,
    reserved_ports: set[int],
    run_dir: Path,
    snapshot_path: Path,
) -> tuple[subprocess.Popen[bytes], str]:
    ip = shutil.which("ip")
    if ip is None:
        raise PairedRunnerError("iproute2 is required to place the task tracker inside its namespace")
    endpoint = topology_record.get(worker_role)
    namespace = topology_record.get(f"{worker_role}_namespace")
    if (
        worker_role not in {"target", "seed"}
        or not isinstance(endpoint, dict)
        or not isinstance(endpoint.get("address"), str)
        or not isinstance(namespace, str)
    ):
        raise PairedRunnerError("task topology has no valid endpoint for the tracker")
    bind_address = endpoint["address"]
    ready_path = run_dir / "tracker-ready.json"
    log_path = run_dir / "tracker-worker.log"
    command = [
        ip,
        "netns",
        "exec",
        namespace,
        sys.executable,
        str(Path(__file__).with_name("task_tracker.py").resolve()),
        "--worker",
        "--gateway",
        str(tracker.gateway),
        "--target-address",
        str(tracker.target_address),
        "--seed-address",
        str(tracker.seed_address),
        "--dialer-role",
        tracker.dialer_role,
        "--bind-address",
        bind_address,
        "--port",
        "0",
        "--seed-port",
        str(tracker.seed_port or 0),
        "--topology-sha256",
        tracker.topology_sha256,
        "--ready-path",
        str(ready_path),
        "--snapshot-path",
        str(snapshot_path),
    ]
    for info_hash in sorted(tracker.info_hashes):
        command.extend(("--info-hash", info_hash.hex()))
    for port in sorted(reserved_ports):
        command.extend(("--reserved-port", str(port)))
    log_descriptor = os.open(
        log_path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
        0o600,
    )
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=log_descriptor,
            stderr=log_descriptor,
            close_fds=True,
            start_new_session=True,
        )
    except OSError as error:
        os.close(log_descriptor)
        raise PairedRunnerError(f"could not start isolated task tracker: {error}") from error
    else:
        os.close(log_descriptor)

    deadline = time.monotonic() + 10.0
    try:
        while time.monotonic() < deadline:
            if ready_path.exists():
                ready, _raw = _load_json(ready_path, "task tracker readiness")
                if (
                    not isinstance(ready, dict)
                    or ready.get("schema_version") != "transfer-task-tracker-ready-v1"
                    or ready.get("address") != bind_address
                    or isinstance(ready.get("port"), bool)
                    or not isinstance(ready.get("port"), int)
                    or not 1 <= ready["port"] <= 65535
                ):
                    raise PairedRunnerError("task tracker published invalid readiness data")
                return process, f"http://{bind_address}:{ready['port']}/announce"
            if process.poll() is not None:
                raise PairedRunnerError(
                    f"isolated task tracker exited before readiness; inspect {log_path.name}"
                )
            time.sleep(0.05)
        raise PairedRunnerError("isolated task tracker did not become ready within 10 seconds")
    except BaseException:
        try:
            _stop_task_tracker_worker(process, snapshot_path)
        except Exception:
            pass
        raise


def _stop_task_tracker_worker(
    process: subprocess.Popen[bytes], snapshot_path: Path
) -> dict[str, Any] | None:
    if process.poll() is None:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
    try:
        process.wait(timeout=10)
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        process.wait(timeout=5)
    if process.returncode != 0:
        raise PairedRunnerError(
            f"isolated task tracker exited with status {process.returncode}; inspect tracker-worker.log"
        )
    if not snapshot_path.exists():
        raise PairedRunnerError("isolated task tracker exited without publishing its snapshot")
    snapshot, _raw = _load_json(snapshot_path, "task tracker snapshot")
    if not isinstance(snapshot, dict) or snapshot.get("schema_version") != "transfer-task-tracker-v1":
        raise PairedRunnerError("isolated task tracker published an invalid snapshot")
    return snapshot


def _profile_listener_ports(profile: dict[str, Any]) -> set[int]:
    ports: set[int] = set()
    launch = profile.get("paired_launch")
    if isinstance(launch, dict):
        for key in ("api_port", "peer_port"):
            value = launch.get(key)
            if isinstance(value, int) and not isinstance(value, bool) and 1 <= value <= 65535:
                ports.add(value)
    expected = profile.get("expected_settings")
    settings = expected.get("settings") if isinstance(expected, dict) else None
    session = settings.get("session") if isinstance(settings, dict) else None
    listen_port = session.get("listen_port") if isinstance(session, dict) else None
    if isinstance(listen_port, int) and not isinstance(listen_port, bool) and 1 <= listen_port <= 65535:
        ports.add(listen_port)
    return ports


def _mkdir_private(path: Path) -> None:
    if path.exists() or path.is_symlink():
        raise PairedRunnerError(f"runner output directory already exists: {path}")
    path.mkdir(mode=0o700)
    os.chmod(path, 0o700)


def _ensure_private_root(path: Path) -> Path:
    path = Path(os.path.abspath(path))
    if path.is_symlink() or not path.is_dir() or path.resolve(strict=True) != path:
        raise PairedRunnerError("scratch root must be an existing non-symlink directory")
    try:
        path.relative_to(ROOT)
    except ValueError:
        pass
    else:
        raise PairedRunnerError("benchmark evidence must be outside the measured source worktree")
    os.chmod(path, 0o700)
    return path


def _stage_input(root: Path, source: Path, raw: bytes, name: str) -> Path:
    inputs = root / "inputs"
    if not inputs.exists():
        _mkdir_private(inputs)
    else:
        if inputs.is_symlink() or not inputs.is_dir() or inputs.resolve(strict=True) != inputs:
            raise PairedRunnerError("staged input directory must be a real task-local directory")
        directory_mode = inputs.stat(follow_symlinks=False).st_mode
        if not stat.S_ISDIR(directory_mode) or stat.S_IMODE(directory_mode) != 0o700:
            raise PairedRunnerError("staged input directory must have mode 0700")
    destination = inputs / name
    digest = hashlib.sha256(raw).hexdigest()
    if destination.exists() or destination.is_symlink():
        metadata = destination.lstat()
        if (
            not stat.S_ISREG(metadata.st_mode)
            or stat.S_IMODE(metadata.st_mode) != 0o600
            or metadata.st_nlink != 1
        ):
            raise PairedRunnerError("existing staged input must be a private, singly linked regular file")
        if _sha256_file(destination, f"staged {name}") != digest:
            raise PairedRunnerError(f"staged input differs from its current source: {name}")
        return destination
    descriptor = os.open(
        destination,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
        0o600,
    )
    try:
        os.fchmod(descriptor, 0o600)
        view = memoryview(raw)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise PairedRunnerError(f"short write staging {name}")
            view = view[written:]
        os.fsync(descriptor)
    except BaseException:
        os.close(descriptor)
        try:
            destination.unlink()
        except OSError:
            pass
        raise
    else:
        os.close(descriptor)
    parent_fd = os.open(inputs, os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW)
    try:
        os.fsync(parent_fd)
    finally:
        os.close(parent_fd)
    return destination


def _load_inputs(
    plan_path: Path,
    pins_path: Path,
    preflight_path: Path,
    environment_path: Path,
    profiles_path: Path,
    scratch_root: Path,
) -> tuple[dict[str, Any], dict[str, dict[str, Any]], dict[str, Any], dict[str, Any], list[tuple[str, Path]]]:
    plan_value, plan_raw = _load_json(plan_path, "paired plan")
    pins_value, pins_raw = _load_json(pins_path, "client pins")
    preflight_value, preflight_raw = _load_json(preflight_path, "compatibility preflight")
    environment_value, environment_raw = _load_json(environment_path, "environment template")
    _profiles_value, profiles_raw = _load_json(profiles_path, "fixture profiles")
    errors = paired_plan_errors(plan_value)
    if errors:
        raise PairedRunnerError("paired plan is invalid: " + "; ".join(errors))
    if hashlib.sha256(profiles_raw).hexdigest() != plan_value["catalog"]["profiles_sha256"]:
        raise PairedRunnerError("fixture profiles digest does not match the paired plan")
    for kind, path in (
        ("paired-plan", plan_path),
        ("compatibility-preflight", preflight_path),
    ):
        problems = validate_artifact(kind, path)
        if problems:
            raise PairedRunnerError(f"{kind} artifact is invalid: " + "; ".join(problems[:8]))
        source_bytes = plan_raw if kind == "paired-plan" else preflight_raw
        if _sha256_file(path, kind) != hashlib.sha256(source_bytes).hexdigest():
            raise PairedRunnerError(f"{kind} changed while its schema was being checked")
    if not isinstance(preflight_value, dict) or preflight_value.get("ready") is not True:
        raise PairedRunnerError("compatibility preflight is not ready")
    plan_sha = hashlib.sha256(plan_raw).hexdigest()
    pins_sha = hashlib.sha256(pins_raw).hexdigest()
    if (
        plan_value.get("plan_id") != preflight_value.get("plan_id")
        or plan_sha != preflight_value.get("plan_sha256")
        or pins_sha != plan_value.get("pins_sha256")
        or pins_sha != preflight_value.get("pins_sha256")
        or plan_value.get("catalog_sha256") != preflight_value.get("catalog_sha256")
    ):
        raise PairedRunnerError("plan, pins, and compatibility preflight identities do not match")
    if not isinstance(environment_value, dict) or set(environment_value) != ENVIRONMENT_FIELDS:
        raise PairedRunnerError("environment template has missing or unknown fields")
    profiles = load_pins_document(pins_value)
    if not isinstance(profiles, dict):
        raise PairedRunnerError("client pins did not produce a profile map")
    staged = [
        ("transfer-paired-plan-v1", _stage_input(scratch_root, plan_path, plan_raw, "paired-plan.json")),
        ("transfer-client-pins-v1", _stage_input(scratch_root, pins_path, pins_raw, "client-pins.json")),
        (
            "transfer-compatibility-preflight-v1",
            _stage_input(scratch_root, preflight_path, preflight_raw, "compatibility-preflight.json"),
        ),
        (
            "transfer-benchmark-environment-v1",
            _stage_input(scratch_root, environment_path, environment_raw, "environment-template.json"),
        ),
        (
            "transfer-fixture-profiles-v1",
            _stage_input(scratch_root, profiles_path, profiles_raw, "fixture-profiles.json"),
        ),
    ]
    return plan_value, profiles, preflight_value, environment_value, staged


def _check_eligible(
    plan: dict[str, Any], preflight: dict[str, Any], client_ids: set[str], profile_id: str
) -> None:
    if not client_ids <= set(plan.get("pinned_client_ids", [])):
        raise PairedRunnerError("candidate and helper must both be pinned in the plan")
    eligible = set(preflight.get("eligible_case_ids", []))
    results = {
        item.get("case_id"): item
        for item in preflight.get("results", [])
        if isinstance(item, dict)
    }
    for client_id in client_ids:
        case_id = f"{client_id}:{profile_id}"
        result = results.get(case_id)
        if case_id not in eligible or not isinstance(result, dict) or result.get("status") != "passed":
            raise PairedRunnerError(f"compatibility preflight did not pass {case_id}")


def _profile_port(profile: dict[str, Any], client_id: str) -> int:
    launch = profile.get("paired_launch")
    if client_id == "urtorrent" or not isinstance(launch, dict):
        raise PairedRunnerError(f"{client_id} cannot act as the API-controlled helper")
    port = launch.get("peer_port")
    if isinstance(port, bool) or not isinstance(port, int) or not 1 <= port <= 65535:
        raise PairedRunnerError(f"{client_id} has no fixed pinned peer listener port")
    return port


def _urtorrent_profile_for_trial(
    profile: dict[str, Any], listen_v4: str, peer_address: str, tracker_url: str, mode: str
) -> dict[str, Any]:
    """Bind address- and mode-dependent settings to this isolated trial."""
    if mode not in {"download", "seed"}:
        raise PairedRunnerError("urtorrent trial mode must be download or seed")
    result = copy.deepcopy(profile)
    expected = result.get("expected_settings")
    settings = expected.get("settings") if isinstance(expected, dict) else None
    if not isinstance(settings, dict):
        raise PairedRunnerError("urtorrent pin has no expected settings object")
    session = settings.get("session")
    control = settings.get("control")
    runner_inputs = settings.get("runner_inputs")
    if not all(isinstance(value, dict) for value in (session, control, runner_inputs)):
        raise PairedRunnerError("urtorrent pin omits a trial-bound settings section")
    session["listen_v4"] = listen_v4
    control["pinned_peer"] = peer_address
    control["tracker_url"] = tracker_url
    control["trackers_after_isolation"] = 1
    runner_inputs["mode"] = mode
    return result


def _safe_relative(name: str) -> PurePosixPath:
    if not isinstance(name, str):
        raise PairedRunnerError("fixture file name must be a string")
    path = PurePosixPath(name)
    if path.is_absolute() or not path.parts or any(part in {"", ".", ".."} for part in path.parts):
        raise PairedRunnerError("fixture file name must remain below its torrent root")
    return path


def _seed_payload(
    fixture: Path,
    manifest: dict[str, Any],
    data_root: Path,
    torrent_name: str,
    expected_identities: dict[str, tuple[int, int, int, int, int]],
) -> None:
    root_flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
    data_fd: int | None = None
    fixture_fd: int | None = None
    try:
        data_fd = os.open(data_root, root_flags)
        fixture_fd = os.open(fixture, root_flags)
    except OSError as error:
        if fixture_fd is not None:
            os.close(fixture_fd)
        if data_fd is not None:
            os.close(data_fd)
        raise PairedRunnerError("seed fixture and data roots must be real directories") from error
    try:
        if data_fd is None or fixture_fd is None:
            raise PairedRunnerError("seed roots did not remain open during validation")
        torrent_parts = _safe_relative(torrent_name).parts
        for item in manifest["files"]:
            relative = _safe_relative(item["name"])
            source_parent = _open_directory_fd_beneath(fixture_fd, relative.parts[:-1])
            try:
                try:
                    source_fd = os.open(
                        relative.parts[-1],
                        os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
                        dir_fd=source_parent,
                    )
                except OSError as error:
                    raise PairedRunnerError("seed fixture payload is missing or linked") from error
                try:
                    source_info = os.fstat(source_fd)
                    if not stat.S_ISREG(source_info.st_mode) or source_info.st_size != item["size_bytes"]:
                        raise PairedRunnerError("seed fixture payload differs from its verified size")
                    observed_identity = (
                        source_info.st_dev,
                        source_info.st_ino,
                        source_info.st_size,
                        source_info.st_mtime_ns,
                        source_info.st_ctime_ns,
                    )
                    if expected_identities.get(item["name"]) != observed_identity:
                        raise PairedRunnerError("seed fixture payload changed before it was linked")
                finally:
                    os.close(source_fd)

                destination_parent = _open_or_create_directory_beneath(
                    data_fd, torrent_parts + relative.parts[:-1]
                )
                try:
                    os.link(
                        relative.parts[-1],
                        relative.parts[-1],
                        src_dir_fd=source_parent,
                        dst_dir_fd=destination_parent,
                        follow_symlinks=False,
                    )
                    destination_info = os.stat(
                        relative.parts[-1], dir_fd=destination_parent, follow_symlinks=False
                    )
                    if (
                        not stat.S_ISREG(destination_info.st_mode)
                        or (destination_info.st_dev, destination_info.st_ino)
                        != (source_info.st_dev, source_info.st_ino)
                    ):
                        os.unlink(relative.parts[-1], dir_fd=destination_parent)
                        raise PairedRunnerError("seed destination does not share the verified source inode")
                    os.fsync(destination_parent)
                except FileExistsError as error:
                    raise PairedRunnerError("seed destination already contains a payload file") from error
                except OSError as error:
                    raise PairedRunnerError(f"cannot link verified seed payload: {error}") from error
                finally:
                    os.close(destination_parent)
            finally:
                os.close(source_parent)
    finally:
        if fixture_fd is not None:
            os.close(fixture_fd)
        if data_fd is not None:
            os.close(data_fd)


def _fixture_file_identities(
    fixture: Path, manifest: dict[str, Any]
) -> dict[str, tuple[int, int, int, int, int]]:
    identities: dict[str, tuple[int, int, int, int, int]] = {}
    for entry in manifest["files"]:
        relative = _safe_relative(entry["name"])
        parent = fixture
        for component in relative.parts[:-1]:
            parent = parent / component
            metadata = parent.lstat()
            if not stat.S_ISDIR(metadata.st_mode):
                raise PairedRunnerError("fixture payload path contains a linked or non-directory parent")
        path = parent / relative.parts[-1]
        metadata = path.lstat()
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != entry["size_bytes"]:
            raise PairedRunnerError("fixture payload identity differs from its verified manifest")
        identities[entry["name"]] = (
            metadata.st_dev,
            metadata.st_ino,
            metadata.st_size,
            metadata.st_mtime_ns,
            metadata.st_ctime_ns,
        )
    return identities


def _fixture_matches_identities(
    fixture: Path,
    manifest: dict[str, Any],
    identities: dict[str, tuple[int, int, int, int, int]],
) -> bool:
    try:
        return _fixture_file_identities(fixture, manifest) == identities
    except (OSError, PairedRunnerError):
        return False


def _open_directory_beneath(root: Path, components: tuple[str, ...]) -> int:
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
    try:
        current = os.open(root, flags)
    except OSError as error:
        raise PairedRunnerError("payload root is not an accessible real directory") from error
    try:
        return _open_directory_fd_beneath(current, components)
    finally:
        os.close(current)


def _open_directory_fd_beneath(root_fd: int, components: tuple[str, ...]) -> int:
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
    current = os.dup(root_fd)
    try:
        for component in components:
            try:
                following = os.open(component, flags, dir_fd=current)
            except OSError as error:
                raise PairedRunnerError("payload path contains a missing or linked directory") from error
            os.close(current)
            current = following
        return current
    except BaseException:
        os.close(current)
        raise


def _open_or_create_directory_beneath(root_fd: int, components: tuple[str, ...]) -> int:
    flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
    current = os.dup(root_fd)
    try:
        for component in components:
            try:
                os.mkdir(component, mode=0o700, dir_fd=current)
            except FileExistsError:
                pass
            following = os.open(component, flags, dir_fd=current)
            metadata = os.fstat(following)
            if not stat.S_ISDIR(metadata.st_mode):
                os.close(following)
                raise PairedRunnerError("seed destination contains a non-directory component")
            os.close(current)
            current = following
        return current
    except BaseException:
        os.close(current)
        raise


def _open_payload_file(data_root: Path, torrent_name: str, relative: PurePosixPath) -> int:
    torrent_parts = _safe_relative(torrent_name).parts
    parent_parts = torrent_parts + relative.parts[:-1]
    directory = _open_directory_beneath(data_root, parent_parts)
    try:
        try:
            return os.open(
                relative.parts[-1],
                os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
                dir_fd=directory,
            )
        except OSError as error:
            raise PairedRunnerError("payload file is missing or is a symbolic link") from error
    finally:
        os.close(directory)


def _hash_payload_file(
    data_root: Path, torrent_name: str, relative: PurePosixPath
) -> tuple[int, str]:
    descriptor = _open_payload_file(data_root, torrent_name, relative)
    digest = hashlib.sha256()
    size = 0
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode):
            raise PairedRunnerError("payload validation encountered a non-regular file")
        while chunk := os.read(descriptor, 1024 * 1024):
            digest.update(chunk)
            size += len(chunk)
        after = os.fstat(descriptor)
        if (
            metadata.st_dev,
            metadata.st_ino,
            metadata.st_size,
            metadata.st_mtime_ns,
            metadata.st_ctime_ns,
        ) != (
            after.st_dev,
            after.st_ino,
            after.st_size,
            after.st_mtime_ns,
            after.st_ctime_ns,
        ):
            raise PairedRunnerError("payload changed while its digest was captured")
    finally:
        os.close(descriptor)
    return size, digest.hexdigest()


def _aggregate_payload_digest(files: list[tuple[str, int, str]]) -> str:
    digest = hashlib.sha256()
    for name, size, file_digest in sorted(files):
        name_bytes = name.encode("ascii")
        digest.update(len(name_bytes).to_bytes(4, "big"))
        digest.update(name_bytes)
        digest.update(size.to_bytes(8, "big"))
        digest.update(bytes.fromhex(file_digest))
    return digest.hexdigest()


def _validate_payload(
    data_root: Path, manifest: dict[str, Any], torrent_name: str
) -> tuple[int, str, str, int]:
    expected_files = []
    observed_files = []
    failures = 0
    for entry in manifest["files"]:
        expected_files.append((entry["name"], entry["size_bytes"], entry["sha256"]))
    for entry in manifest["files"]:
        try:
            size, digest = _hash_payload_file(
                data_root, torrent_name, _safe_relative(entry["name"])
            )
        except (OSError, PairedRunnerError):
            failures += 1
            continue
        observed_files.append((entry["name"], size, digest))
        if size != entry["size_bytes"] or digest != entry["sha256"]:
            failures += 1
    expected_digest = _aggregate_payload_digest(expected_files)
    observed_digest = _aggregate_payload_digest(observed_files)
    if len(observed_files) != len(expected_files) or observed_digest != expected_digest:
        failures = max(1, failures)
    return (
        sum(item["size_bytes"] for item in manifest["files"]),
        expected_digest,
        observed_digest,
        failures,
    )


def _fsync_payload(data_root: Path, manifest: dict[str, Any], torrent_name: str) -> None:
    torrent_parts = _safe_relative(torrent_name).parts
    directory_components: set[tuple[str, ...]] = {(), torrent_parts}
    for entry in manifest["files"]:
        relative = _safe_relative(entry["name"])
        descriptor = _open_payload_file(data_root, torrent_name, relative)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
        for depth in range(1, len(relative.parts)):
            directory_components.add(torrent_parts + relative.parts[:depth])
    for components in sorted(directory_components, key=len, reverse=True):
        descriptor = _open_directory_beneath(data_root, components)
        try:
            os.fsync(descriptor)
        finally:
            os.close(descriptor)


def _runtime_paths(root: Path, *, tracker_url: str = "", peer_address: str = "") -> RuntimePaths:
    _mkdir_private(root)
    data = root / "data"
    state = root / "state"
    _mkdir_private(data)
    _mkdir_private(state)
    return RuntimePaths(
        root=root,
        data=data,
        state=state,
        log=root / "client.log",
        pid=state / "launcher.pid",
        env=state / "client.env",
        tracker_url=tracker_url,
        peer_address=peer_address,
    )


def _start_urtorrent(
    endpoint: Endpoint,
    topology: dict[str, Any],
    metainfo: Path,
    peer_address: str,
    mode: str,
    barrier_dir: Path,
    timeout_seconds: int,
) -> None:
    profile = endpoint.profile
    binary_path = Path(str(profile.get("binary_path", ""))).expanduser()
    digest = profile.get("expected_binary_sha256")
    expected = profile.get("expected_settings")
    settings = expected.get("settings") if isinstance(expected, dict) else None
    if not binary_path.is_absolute() or not isinstance(digest, str) or not isinstance(settings, dict):
        raise PairedRunnerError("urtorrent pin is missing its runner binary or settings contract")
    session = settings.get("session")
    runner_inputs = settings.get("runner_inputs")
    if not isinstance(session, dict) or not isinstance(runner_inputs, dict):
        raise PairedRunnerError("urtorrent pin is missing session or runner inputs")
    task = topology.get(endpoint.role)
    namespace = topology.get(f"{endpoint.role}_namespace")
    if not isinstance(task, dict) or not isinstance(namespace, str):
        raise PairedRunnerError("urtorrent task network endpoint is invalid")
    args = [
        str(binary_path),
        "--mode", mode,
        "--namespace", namespace,
        "--torrent", str(metainfo),
        "--save-path", str(endpoint.paths.data),
        "--pid-file", str(endpoint.paths.state / "urtorrent.pid"),
        "--settings-file", str(endpoint.paths.state / "settings.json"),
        "--start-gate", str(barrier_dir / "released"),
        "--ready-file", str(barrier_dir / "candidate.ready"),
        "--stop-file", str(barrier_dir / "stop-runner"),
        "--listen-v4", str(task["address"]),
        "--peer", peer_address,
        "--tracker-url", endpoint.paths.tracker_url,
        "--max-connections", str(session.get("max_connections", 500)),
        "--max-peers", str(session.get("max_peers_per_torrent", 50)),
        "--unchoke-slots", str(session.get("unchoke_slots", 8)),
        "--hash-threads", str(runner_inputs.get("hash_threads", 2)),
        "--upload-limit", str(session.get("upload_limit", 0)),
        "--download-limit", str(session.get("download_limit", 0)),
        "--timeout-seconds", str(timeout_seconds),
        "--seed-seconds", str(timeout_seconds),
    ]
    endpoint.direct_profile = _urtorrent_profile_for_trial(
        profile, str(task["address"]), peer_address, endpoint.paths.tracker_url, mode
    )
    endpoint.direct_profile["pid_file"] = str(endpoint.paths.state / "urtorrent.pid")
    endpoint.direct_profile["settings_file"] = str(endpoint.paths.state / "settings.json")
    endpoint.direct_profile["_task_local_api"] = None
    try:
        endpoint.direct_process = ManagedProcess.start(
            args,
            binary_path=binary_path,
            expected_binary_sha256=digest,
            log_path=endpoint.paths.log,
            environment={"LANG": "C", "PATH": os.environ.get("PATH", "/usr/bin:/bin")},
            pid_file_path=endpoint.paths.pid,
            cwd=endpoint.paths.state,
            startup_timeout_seconds=min(30.0, max(5.0, float(profile.get("preflight_timeout_seconds", 30)))),
            network_namespace=namespace,
            network_namespace_identity=task["namespace_identity"],
        )
    except ProcessSupervisorError as error:
        raise PairedRunnerError(f"could not start pinned urtorrent runner: {error}") from error


def _probe_endpoint(endpoint: Endpoint, timeout_seconds: float = 30.0) -> dict[str, Any]:
    deadline = time.monotonic() + timeout_seconds
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            if endpoint.managed is not None:
                return endpoint.managed.probe()
            if endpoint.direct_profile is not None and endpoint.direct_process is not None:
                observation = probe_client("urtorrent", endpoint.direct_profile)
                identity = observation.get("runtime_identity")
                if not isinstance(identity, dict) or identity.get("process_id") != endpoint.direct_process.pid:
                    raise PairedRunnerError("urtorrent settings readback belongs to another process")
                return observation
            raise PairedRunnerError("client endpoint has not started")
        except (AdapterError, ManagedClientError, OSError, ValueError) as error:
            last_error = error
            time.sleep(0.1)
    raise PairedRunnerError(f"client did not become ready for settings readback: {last_error}")


def _start_endpoint(
    endpoint: Endpoint,
    topology: dict[str, Any],
    role_paths: RuntimePaths,
    prelaunched: ManagedClient | None,
) -> None:
    if endpoint.client_id == "urtorrent":
        return
    if prelaunched is not None:
        endpoint.managed = prelaunched
        if endpoint.profile.get("runtime") == "container":
            bind_client_topology(prelaunched, topology, role_paths.as_dict())
        else:
            raise PairedRunnerError("native endpoint was launched before its topology")
    elif endpoint.profile.get("runtime") == "container":
        raise PairedRunnerError("container endpoint was not launched before topology creation")
    else:
        endpoint.managed = launch_client(
            endpoint.client_id,
            endpoint.profile,
            endpoint.role,
            topology,
            role_paths.as_dict(),
        )
    endpoint.observation = _probe_endpoint(endpoint)
    endpoint.controller = ClientController(
        endpoint.client_id,
        endpoint.managed.api_profile,
        endpoint.observation,
    )


def _start_observer(endpoint: Endpoint, run_dir: Path, interval_ms: int) -> tuple[subprocess.Popen[bytes], Path, Path, Path]:
    stop_path = run_dir / "observer.stop"
    csv_path = run_dir / "resources.csv"
    summary_path = run_dir / "resources.json"
    command = [
        sys.executable,
        str(Path(__file__).with_name("observe.py")),
        "--pid", str(endpoint.pid),
        "--interval-ms", str(interval_ms),
        "--network-interface", "tngveth0",
        "--stop-file", str(stop_path),
        "--csv", str(csv_path),
        "--summary", str(summary_path),
    ]
    try:
        process = subprocess.Popen(
            command,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
            close_fds=True,
            start_new_session=True,
            env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C"},
        )
    except OSError as error:
        raise PairedRunnerError(f"cannot start independent resource observer: {error}") from error
    return process, stop_path, csv_path, summary_path


def _stop_observer(process: subprocess.Popen[bytes], stop_path: Path) -> int:
    if not stop_path.exists() and not stop_path.is_symlink():
        descriptor = os.open(
            stop_path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
            0o600,
        )
        try:
            os.write(descriptor, b"stop\n")
            os.fsync(descriptor)
        finally:
            os.close(descriptor)
    try:
        return process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        process.send_signal(signal.SIGTERM)
        try:
            return process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            return process.wait(timeout=5)


def _wait_for_observer_sample(
    process: subprocess.Popen[bytes], csv_path: Path, timeout_seconds: float = 30.0
) -> None:
    """Ensure the observer has sampled the live client before releasing peers."""
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        return_code = process.poll()
        if return_code is not None:
            raise PairedRunnerError(
                f"independent observer exited before its first sample (status {return_code})"
            )
        try:
            descriptor = os.open(csv_path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
        except FileNotFoundError:
            time.sleep(0.01)
            continue
        try:
            sample = os.read(descriptor, 256 * 1024)
        finally:
            os.close(descriptor)
        if sample.count(b"\n") >= 2:
            return
        time.sleep(0.01)
    raise PairedRunnerError("independent observer did not record a live-client sample")


def _wait_seed_complete(controller: ClientController, info_hash: str, timeout_seconds: float) -> TorrentState:
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        state = controller.status(info_hash)
        if state is not None and state.complete:
            return state
        time.sleep(POLL_SECONDS)
    raise PairedRunnerError("preloaded seed was not verified before the barrier timeout")


def _status(controller: ClientController | None, info_hash: str) -> TorrentState | None:
    if controller is None:
        return None
    return controller.status(info_hash)


def _trial_status(
    controller: ClientController | None, info_hash: str
) -> tuple[TorrentState | None, str | None]:
    try:
        return _status(controller, info_hash), None
    except ClientControlError as error:
        if "HTTP 503 for " not in str(error):
            raise
        return None, str(error)


def _runner_result(endpoint: Endpoint) -> dict[str, Any] | None:
    if endpoint.direct_process is None:
        return None
    try:
        raw = _read_regular(endpoint.paths.log, "urtorrent runner log", limit=8 * 1024 * 1024)
    except PairedRunnerError:
        return None
    for line in reversed(raw.decode("utf-8", "replace").splitlines()):
        try:
            value = strict_json_loads(line, label="urtorrent runner result")
        except StrictJsonError:
            continue
        if isinstance(value, dict) and value.get("format") == "transfer-urtorrent-run-result-v1":
            return value
    return None


def _wait_for_transfer(
    *,
    target_controller: ClientController | None,
    helper_controller: ClientController,
    target_role: str,
    target_endpoint: Endpoint,
    candidate_endpoint: Endpoint,
    helper_endpoint: Endpoint,
    info_hash: str,
    mode: str,
    timeout_seconds: int,
    counter_baseline: TorrentState | None,
) -> tuple[int | None, int | None, int | None]:
    """Wait for the receiver and sample useful-transfer bounds via peer counters."""
    target_api = target_controller
    counter_controller = target_controller
    counter_field = "received_bytes"
    if target_role == "target" and target_endpoint.client_id == "urtorrent":
        counter_controller = helper_controller
        counter_field = "uploaded_bytes"
    prior = getattr(counter_baseline, counter_field) if counter_baseline is not None else 0
    first_ns: int | None = None
    last_ns: int | None = None
    candidate_bytes: int | None = None
    last_transient_status_error: str | None = None
    deadline = time.monotonic() + timeout_seconds
    while time.monotonic() < deadline:
        observed, transient_error = _trial_status(counter_controller, info_hash)
        last_transient_status_error = transient_error or last_transient_status_error
        if observed is not None:
            current = getattr(observed, counter_field)
            if current < prior:
                raise PairedRunnerError("receiver transfer counter regressed during the trial")
            if current > prior:
                now = time.perf_counter_ns()
                if first_ns is None:
                    first_ns = now
                last_ns = now
            prior = current
        if target_api is not None:
            state, transient_error = _trial_status(target_api, info_hash)
            last_transient_status_error = transient_error or last_transient_status_error
            if state is not None:
                if state.state.lower() in {"error", "stopped-on-error"}:
                    raise PairedRunnerError("receiver reported a transfer error")
                if state.complete:
                    break
            if candidate_endpoint.client_id == "urtorrent" and mode == "seed":
                process = candidate_endpoint.direct_process
                if process is None:
                    raise PairedRunnerError("direct seeder process is missing")
                try:
                    process.wait(timeout_seconds=0.01)
                except subprocess.TimeoutExpired:
                    pass
                else:
                    raise PairedRunnerError("direct seeder exited before the helper verified the payload")
        else:
            process = target_endpoint.direct_process
            if process is None:
                raise PairedRunnerError("direct receiver process is missing")
            try:
                exit_code = process.wait(timeout_seconds=0.05)
            except subprocess.TimeoutExpired:
                exit_code = None
            if exit_code is not None:
                if exit_code != 0:
                    raise PairedRunnerError(f"urtorrent receiver exited with status {exit_code}")
                result = _runner_result(target_endpoint)
                if result is None or result.get("complete") is not True:
                    raise PairedRunnerError("urtorrent receiver exited without a complete result record")
                target_endpoint.direct_result = result
                break
        time.sleep(POLL_SECONDS)
    else:
        reason = "paired payload transfer exceeded its bounded timeout"
        if last_transient_status_error is not None:
            reason += f"; last transient status response: {last_transient_status_error}"
        raise PairedRunnerError(reason)

    if mode == "download":
        candidate_state, transient_error = _trial_status(candidate_endpoint.controller, info_hash)
        last_transient_status_error = transient_error or last_transient_status_error
        if candidate_state is not None:
            candidate_bytes = candidate_state.received_bytes
        elif candidate_endpoint.direct_process is not None:
            candidate_endpoint.direct_result = _runner_result(candidate_endpoint)
            if candidate_endpoint.direct_result is not None:
                raw_count = candidate_endpoint.direct_result.get("downloaded")
                candidate_bytes = raw_count if isinstance(raw_count, int) and raw_count >= 0 else None
    elif candidate_endpoint.controller is not None:
        candidate_state, transient_error = _trial_status(candidate_endpoint.controller, info_hash)
        last_transient_status_error = transient_error or last_transient_status_error
        if candidate_state is not None:
            candidate_bytes = candidate_state.uploaded_bytes
    return first_ns, last_ns, candidate_bytes


def _create_marker(path: Path, payload: bytes) -> None:
    descriptor = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
        0o600,
    )
    try:
        view = memoryview(payload)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise PairedRunnerError("short write creating a paired-run marker")
            view = view[written:]
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat(timespec="microseconds").replace("+00:00", "Z")


def _environment_for_trial(
    template: dict[str, Any],
    topology_sha256: str | None,
    cache_record: dict[str, Any] | None,
    cache_sha256: str | None,
    network_profile: str | None,
    network_digests: list[str],
    network_seeds: list[int] | None = None,
) -> dict[str, Any]:
    environment = copy.deepcopy(template)
    network = environment["network"]
    if not isinstance(network, dict):
        raise PairedRunnerError("environment network settings must be an object")
    network["task_topology_sha256"] = topology_sha256
    network["control_profile_id"] = network_profile
    network["network_profile_id"] = network_profile
    network["control_observation_sha256s"] = network_digests
    network["control_prng_seeds"] = (
        list(network_seeds or []) if network_profile is not None else None
    )
    if network_profile is not None:
        settings = NETWORK_PROFILES[network_profile]
        network["link_limit_bps"] = settings["rate_bps"]
        network["rtt_ms"] = None
        network["one_way_delay_ms"] = settings["delay_ms"]
        network["jitter_ms"] = settings["jitter_ms"]
        network["loss_percent"] = settings["loss_ppm"] / 10_000
    else:
        network["link_limit_bps"] = None
        network["rtt_ms"] = None
        network["one_way_delay_ms"] = None
        network["jitter_ms"] = None
        network["loss_percent"] = None
    if cache_record is not None and cache_record.get("status") == "achieved":
        environment["cache_state"] = cache_record["requested_state"]
    else:
        environment["cache_state"] = "unknown"
    environment["cache_state_observation_sha256"] = cache_sha256
    return environment


def _resource_observation(path: Path) -> dict[str, Any]:
    value, _raw = _load_json(path, "resource observation")
    problems = validate_artifact("resource-observation", path)
    if problems:
        raise PairedRunnerError("independent observer emitted invalid evidence: " + "; ".join(problems[:8]))
    return value


def _candidate_info_hash(torrent_record: dict[str, Any], client_id: str) -> str:
    hashes = torrent_record.get("info_hashes")
    if not isinstance(hashes, dict):
        raise PairedRunnerError("fixture torrent record has no info hashes")
    value = hashes.get("sha1") or hashes.get("sha256")
    if client_id == "urtorrent":
        value = hashes.get("sha1")
    if not isinstance(value, str) or INFO_HASH_RE.fullmatch(value) is None:
        raise PairedRunnerError("fixture has no info hash supported by the selected client")
    return value


def _start_container_if_needed(
    client_id: str, profile: dict[str, Any], role: str, paths: RuntimePaths
) -> ManagedClient | None:
    if profile.get("runtime") != "container":
        return None
    try:
        return launch_container_unattached(client_id, profile, role, paths.as_dict())
    except ManagedClientError as error:
        raise PairedRunnerError(f"could not start isolated {client_id} container: {error}") from error


def _run_trial(
    *,
    planned: dict[str, Any],
    mode: str,
    candidate_id: str,
    helper_id: str,
    profiles: dict[str, dict[str, Any]],
    fixture_dir: Path,
    fixture_manifest: dict[str, Any],
    fixture_identities: dict[str, tuple[int, int, int, int, int]],
    scratch_root: Path,
    environment_template: dict[str, Any],
    harness_identity: dict[str, Any],
    network_profile: str | None,
    cache_state: str,
    timeout_seconds: int,
    observer_interval_ms: int,
    artifacts: RunArtifacts,
) -> Path:
    if mode not in {"download", "seed"}:
        raise PairedRunnerError("trial workload must be download or seed")
    candidate_role = "target" if mode == "download" else "seed"
    helper_role = "seed" if mode == "download" else "target"
    target_role = candidate_role
    if mode == "seed":
        target_role = helper_role
    candidate_profile = profiles[candidate_id]
    helper_profile = profiles[helper_id]
    trial_suffix = f"{planned['trial_id']}:{mode}"
    trial_key = _scheduled_trial_key(planned["trial_id"], mode)
    run_dir = scratch_root / "runs" / trial_key
    _mkdir_private(run_dir)
    barriers_dir = run_dir / "barrier"
    _mkdir_private(barriers_dir)

    candidate_paths = _runtime_paths(
        scratch_root / "endpoints" / f"{trial_key}-{candidate_role}"
    )
    helper_paths = _runtime_paths(
        scratch_root / "endpoints" / f"{trial_key}-{helper_role}"
    )
    endpoints = {
        candidate_role: Endpoint(candidate_role, candidate_id, candidate_profile, candidate_paths),
        helper_role: Endpoint(helper_role, helper_id, helper_profile, helper_paths),
    }
    if candidate_role == helper_role:
        raise PairedRunnerError("candidate and helper roles must be opposite endpoints")
    candidate_endpoint = endpoints[candidate_role]
    helper_endpoint = endpoints[helper_role]
    endpoint_by_role = {candidate_role: candidate_endpoint, helper_role: helper_endpoint}
    target_endpoint = endpoint_by_role[target_role]
    seed_endpoint = endpoint_by_role["seed"]

    _seed_payload(
        fixture_dir,
        fixture_manifest,
        seed_endpoint.paths.data,
        fixture_manifest["profile"]["torrent_name"],
        fixture_identities,
    )
    # Creating these private seed hard links changes inode ctime even though
    # payload bytes stay unchanged. Take the mutation baseline after linking.
    fixture_identities.clear()
    fixture_identities.update(_fixture_file_identities(fixture_dir, fixture_manifest))

    prelaunched: dict[str, ManagedClient | None] = {}

    topology = None
    topology_record: dict[str, Any] | None = None
    topology_path = run_dir / "task-network.json"
    tracker: TaskTracker | None = None
    tracker_process: subprocess.Popen[bytes] | None = None
    network_applied: list[tuple[Path, Path, dict[str, Any]]] = []
    observer_process: subprocess.Popen[bytes] | None = None
    observer_stop: Path | None = None
    observer_csv: Path | None = None
    observer_summary: Path | None = None
    started_at = _utc_now()
    candidate_observation: dict[str, Any] | None = None
    candidate_observation_path: Path | None = None
    candidate_observation_sha: str | None = None
    resource_value: dict[str, Any] | None = None
    resource_sha: str | None = None
    cache_value: dict[str, Any] | None = None
    cache_sha: str | None = None
    retargeted_path = run_dir / "fixture.torrent"
    tracker_path = run_dir / "tracker.json"
    topology_digest: str | None = None
    network_digests: list[str] = []
    network_seeds: list[int] = []
    measured_received: int | None = None
    measured_uploaded: int | None = None
    first_payload_ns: int | None = None
    last_payload_ns: int | None = None
    durations = {name: 0 for name in ("barrier_wait", "transfer", "flush", "validation", "shutdown")}
    integrity = {
        "status": "not_run",
        "expected_sha256": None,
        "observed_sha256": None,
        "hash_failures": 0,
        "false_completions": 0,
    }
    payload = {
        "useful_bytes": None,
        "received_bytes": None,
        "written_bytes": None,
        "read_bytes": None,
        "uploaded_bytes": None,
        "duplicate_bytes": None,
    }
    status = "unmeasured"
    reason: str | None = None
    exit_code: int | None = None
    transfer_started_ns: int | None = None
    transfer_finished_ns: int | None = None
    candidate_baseline: TorrentState | None = None
    helper_baseline: TorrentState | None = None
    info_hash = ""
    shutdown_started_ns: int | None = None
    try:
        if not _fixture_matches_identities(fixture_dir, fixture_manifest, fixture_identities):
            raise PairedRunnerError("fixture payload changed since its full content verification")
        if network_profile is not None and network_profile not in NETWORK_PROFILES:
            raise PairedRunnerError("network profile is not in the pinned control catalog")
        for endpoint in (candidate_endpoint, helper_endpoint):
            prelaunched[endpoint.role] = _start_container_if_needed(
                endpoint.client_id, endpoint.profile, endpoint.role, endpoint.paths
            )
        topology = create_topology(
            target_pid=(prelaunched["target"].pid if prelaunched.get("target") else None),
            seed_pid=(prelaunched["seed"].pid if prelaunched.get("seed") else None),
        )
        topology_record = topology.as_record()
        _write_private_json(topology_path, topology_record)
        topology_digest = _sha256_file(topology_path, "task topology")
        artifacts.add("transfer-task-network-v1", topology_path)

        for endpoint in (candidate_endpoint, helper_endpoint):
            other_role = "seed" if endpoint.role == "target" else "target"
            other_id = endpoint_by_role[other_role].client_id
            other_profile = endpoint_by_role[other_role].profile
            if endpoint.client_id == "urtorrent":
                if other_id == "urtorrent":
                    raise PairedRunnerError("the direct peer runner cannot be both trial endpoints")
                peer_port = _profile_port(other_profile, other_id)
            elif other_id == "urtorrent":
                # API clients do not consume this value; the direct runner
                # initiates the peer connection to the API client's listener.
                peer_port = _profile_port(endpoint.profile, endpoint.client_id)
            else:
                peer_port = _profile_port(other_profile, other_id)
            peer_address = f"{topology_record[other_role]['address']}:{peer_port}"
            endpoint.paths = RuntimePaths(
                root=endpoint.paths.root,
                data=endpoint.paths.data,
                state=endpoint.paths.state,
                log=endpoint.paths.log,
                pid=endpoint.paths.pid,
                env=endpoint.paths.env,
                tracker_url="",
                peer_address=peer_address,
            )

        if network_profile is not None:
            for role in ("target", "seed"):
                apply_path = run_dir / f"network-{role}-apply.json"
                reset_path = run_dir / f"network-{role}-reset.json"
                seed = _network_seed(planned["pair_id"], network_profile, role)
                record = apply_profile(
                    topology_record[f"{role}_namespace"],
                    topology_record[role]["interface"],
                    network_profile,
                    apply_path,
                    seed=seed,
                )
                network_applied.append((apply_path, reset_path, record))
                network_seeds.append(seed)
                digest = _sha256_file(apply_path, f"{role} network-control record")
                network_digests.append(digest)
                artifacts.add("transfer-network-control-v1", apply_path)

        info_record, _info_record_raw = _load_json(fixture_dir / "torrent.json", "fixture torrent record")
        hashes = info_record.get("info_hashes")
        if not isinstance(hashes, dict):
            raise PairedRunnerError("fixture torrent record has no info hashes")
        info_hash_bytes = {
            bytes.fromhex(value)
            for value in hashes.values()
            if isinstance(value, str) and INFO_HASH_RE.fullmatch(value)
        }
        if not info_hash_bytes:
            raise PairedRunnerError("fixture torrent record has no valid v1/v2 info hashes")
        seed_port = None if seed_endpoint.client_id == "urtorrent" else _profile_port(seed_endpoint.profile, seed_endpoint.client_id)
        native_roles = [
            endpoint.role
            for endpoint in (candidate_endpoint, helper_endpoint)
            if endpoint.client_id == "torrentngd"
        ]
        dialer_role = native_roles[0] if len(native_roles) == 1 else "target"
        tracker = TaskTracker(
            gateway=topology_record["gateway"],
            target_address=topology_record["target"]["address"],
            seed_address=topology_record["seed"]["address"],
            seed_port=seed_port,
            seed_peer_id=None,
            info_hashes=info_hash_bytes,
            topology_sha256=str(topology_record["artifact_sha256"]),
            dialer_role=dialer_role,
        )
        tracker_process, tracker_url = _start_task_tracker_worker(
            tracker,
            topology_record,
            helper_endpoint.role,
            _profile_listener_ports(candidate_endpoint.profile)
            | _profile_listener_ports(helper_endpoint.profile),
            run_dir,
            tracker_path,
        )
        retarget_record = retarget_fixture_torrent(fixture_dir, retargeted_path, tracker_url)
        artifacts.add("transfer-retargeted-metainfo-v1", retargeted_path)
        retarget_record_path = run_dir / "retargeted-metainfo.json"
        _write_private_json(retarget_record_path, retarget_record)
        artifacts.add("transfer-retargeted-metainfo-record-v1", retarget_record_path)

        for endpoint in (candidate_endpoint, helper_endpoint):
            endpoint.paths = RuntimePaths(
                root=endpoint.paths.root,
                data=endpoint.paths.data,
                state=endpoint.paths.state,
                log=endpoint.paths.log,
                pid=endpoint.paths.pid,
                env=endpoint.paths.env,
                tracker_url=tracker_url,
                peer_address=endpoint.paths.peer_address,
            )
            _start_endpoint(endpoint, topology_record, endpoint.paths, prelaunched.get(endpoint.role))

        if candidate_endpoint.observation is not None:
            candidate_observation = candidate_endpoint.observation
            status = "incomplete"

        info_hash = _candidate_info_hash(retarget_record, candidate_id)
        if candidate_endpoint.controller is not None:
            candidate_endpoint.controller.add_paused(
                retargeted_path,
                str(candidate_endpoint.paths.data) if candidate_endpoint.profile.get("runtime") != "container" else "/data",
                info_hash,
            )
        else:
            _start_urtorrent(
                candidate_endpoint,
                topology_record,
                retargeted_path,
                candidate_endpoint.paths.peer_address,
                mode,
                barriers_dir,
                timeout_seconds,
            )

        if helper_endpoint.controller is not None:
            helper_endpoint.controller.add_paused(
                retargeted_path,
                str(helper_endpoint.paths.data) if helper_endpoint.profile.get("runtime") != "container" else "/data",
                info_hash,
            )
        else:
            raise PairedRunnerError("the trial helper must use an API-controlled client")

        # The direct runner adds its own torrent and reports readiness before the
        # shared gate; API-controlled endpoints must be pinned and paused here.
        if candidate_endpoint.client_id == "urtorrent":
            direct_deadline = time.monotonic() + 30
            while not (barriers_dir / "candidate.ready").is_file():
                if candidate_endpoint.direct_process is not None:
                    try:
                        candidate_endpoint.direct_process.wait(timeout_seconds=0.02)
                    except subprocess.TimeoutExpired:
                        pass
                    else:
                        raise PairedRunnerError("urtorrent exited before reaching the start barrier")
                if time.monotonic() >= direct_deadline:
                    raise PairedRunnerError("urtorrent did not reach the start barrier")
                time.sleep(0.05)
            candidate_endpoint.observation = _probe_endpoint(candidate_endpoint)

        if candidate_endpoint.observation is not None:
            candidate_observation = candidate_endpoint.observation
            status = "incomplete"

        if seed_endpoint.controller is not None:
            # Starting only the preloaded seed lets it verify the fixture while
            # the measured receiver remains paused. The seed has no peer until
            # the receiver is released, so its setup bytes cannot enter the
            # measured interval; its counters are sampled after verification.
            seed_endpoint.controller.resume(info_hash)
            _wait_seed_complete(seed_endpoint.controller, info_hash, timeout_seconds=min(timeout_seconds, 300))

        for endpoint in (candidate_endpoint, helper_endpoint):
            if endpoint.observation is None:
                endpoint.observation = _probe_endpoint(endpoint)
            observation_path = run_dir / f"client-{endpoint.role}.json"
            endpoint.observation_sha256 = write_new_json(observation_path, endpoint.observation)
            endpoint.observation_path = observation_path
            artifacts.add("transfer-client-observation-v1", observation_path)
            if endpoint is candidate_endpoint:
                candidate_observation = endpoint.observation
                candidate_observation_path = observation_path
                candidate_observation_sha = endpoint.observation_sha256
        candidate_observation = candidate_endpoint.observation
        candidate_observation_path = candidate_endpoint.observation_path
        candidate_observation_sha = candidate_endpoint.observation_sha256

        if candidate_endpoint.controller is not None:
            candidate_baseline = candidate_endpoint.controller.status(info_hash)
        if helper_endpoint.controller is not None:
            helper_baseline = helper_endpoint.controller.status(info_hash)
        if helper_baseline is None:
            raise PairedRunnerError("helper did not report a baseline state for the exact torrent")
        if candidate_endpoint.controller is not None and candidate_baseline is None:
            raise PairedRunnerError("candidate did not report a baseline state for the exact torrent")

        # The seed's hash check reads the full fixture. Apply cache controls
        # after that setup work and immediately before release so a cold trial
        # does not silently start with the seed payload resident again.
        requested_cache = prepare_cache_state(
            fixture_dir,
            cache_state,
            verified_manifest=fixture_manifest,
        )
        cache_path = run_dir / "cache-state.json"
        write_cache_record(cache_path, requested_cache)
        cache_value = requested_cache
        cache_sha = _sha256_file(cache_path, "cache-state record")
        artifacts.add("transfer-cache-state-v1", cache_path)
        if not _fixture_matches_identities(fixture_dir, fixture_manifest, fixture_identities):
            raise PairedRunnerError("fixture payload changed during client setup")
        observer_process, observer_stop, observer_csv, observer_summary = _start_observer(
            candidate_endpoint, run_dir, observer_interval_ms
        )
        if observer_csv is None:
            raise PairedRunnerError("independent observer did not provide a CSV path")
        _wait_for_observer_sample(observer_process, observer_csv)

        if candidate_endpoint.client_id == "urtorrent":
            candidate_ready = barriers_dir / "candidate.ready"
        else:
            candidate_ready = None
        barrier = FileStartBarrier(barriers_dir, ["candidate", "helper"])
        barrier_errors: list[BaseException] = []
        barrier_results: list[Any] = []

        def release_endpoint(name: str, endpoint: Endpoint) -> None:
            try:
                result = barrier.wait(name, timeout_seconds=min(timeout_seconds, 60))
                barrier_results.append(result)
                if endpoint.controller is not None and endpoint is not seed_endpoint:
                    endpoint.controller.resume(info_hash)
            except BaseException as error:
                barrier_errors.append(error)

        barrier_started_ns = time.perf_counter_ns()
        threads = []
        for name, endpoint in (("candidate", candidate_endpoint), ("helper", helper_endpoint)):
            if name == "candidate" and candidate_ready is not None:
                continue
            thread = threading.Thread(
                target=release_endpoint,
                args=(name, endpoint),
                name=f"barrier-{trial_key}-{name}",
                daemon=True,
            )
            threads.append(thread)
            thread.start()
        for thread in threads:
            thread.join(timeout=min(timeout_seconds, 60) + 5)
        if any(thread.is_alive() for thread in threads):
            raise PairedRunnerError("a barrier participant did not return after release")
        if barrier_errors:
            raise PairedRunnerError(f"start barrier participant failed: {barrier_errors[0]}")
        if not (barriers_dir / "released").is_file():
            raise PairedRunnerError("common start barrier was not released")
        barrier_finished_ns = time.perf_counter_ns()
        durations["barrier_wait"] = max(
            [result.wait_ns for result in barrier_results]
            + [barrier_finished_ns - barrier_started_ns]
        )

        # Poll the receiver's exact-hash counters so the transfer window is
        # bounded by observed useful progress instead of the whole process life.
        target_controller = target_endpoint.controller
        if target_controller is None and candidate_endpoint.client_id != "urtorrent":
            raise PairedRunnerError("API-controlled receiver has no transfer controller")
        helper_controller = helper_endpoint.controller
        if helper_controller is None:
            raise PairedRunnerError("helper transfer controller is missing")
        timeout = timeout_seconds
        direct_stop = barriers_dir / "stop-runner"
        if not barrier_results:
            raise PairedRunnerError("no API-controlled participant observed the released barrier")
        # BarrierResult timestamps the release marker observation. Starting at
        # this shared event avoids dropping API-resume latency from tiny trials.
        transfer_started_ns = min(result.released_observed_ns for result in barrier_results)
        receiver_baseline = (
            helper_baseline
            if target_endpoint.controller is None or target_endpoint is helper_endpoint
            else candidate_baseline
        )
        first_payload_ns, last_payload_ns, candidate_bytes = _wait_for_transfer(
            target_controller=target_controller,
            helper_controller=helper_controller,
            target_role=target_role,
            target_endpoint=target_endpoint,
            candidate_endpoint=candidate_endpoint,
            helper_endpoint=helper_endpoint,
            info_hash=info_hash,
            mode=mode,
            timeout_seconds=timeout,
            counter_baseline=receiver_baseline,
        )
        transfer_finished_ns = time.perf_counter_ns()
        if first_payload_ns is not None and last_payload_ns is not None:
            if last_payload_ns > first_payload_ns:
                durations["transfer"] = last_payload_ns - first_payload_ns
            else:
                # A small fixture can finish between adjacent samples. The
                # barrier-to-completion interval is the conservative measured
                # bound; reporting one nanosecond would manufacture throughput.
                durations["transfer"] = max(1, transfer_finished_ns - transfer_started_ns)
        elif transfer_finished_ns > transfer_started_ns:
            raise PairedRunnerError("receiver completed but no useful-byte progress interval was observed")
        if mode == "download":
            baseline_received = (
                candidate_baseline.received_bytes if candidate_baseline is not None else 0
            )
            if candidate_bytes is not None and candidate_bytes < baseline_received:
                raise PairedRunnerError("candidate received-byte counter regressed during the trial")
            measured_received = (
                candidate_bytes - baseline_received
                if candidate_bytes is not None
                else None
            )
        else:
            baseline_uploaded = (
                candidate_baseline.uploaded_bytes if candidate_baseline is not None else 0
            )
            if candidate_bytes is not None and candidate_bytes < baseline_uploaded:
                raise PairedRunnerError("candidate uploaded-byte counter regressed during the trial")
            measured_uploaded = (
                candidate_bytes - baseline_uploaded
                if candidate_bytes is not None
                else None
            )

        # Stop independent sampling at the transfer boundary, before client
        # shutdown and verification cleanup add teardown-only samples.
        if observer_process is not None and observer_stop is not None:
            observer_code = _stop_observer(observer_process, observer_stop)
            if observer_code != 0 or observer_summary is None or not observer_summary.is_file():
                raise PairedRunnerError(f"independent observer exited with status {observer_code}")
            resource_value = _resource_observation(observer_summary)
            resource_sha = _sha256_file(observer_summary, "resource observation")
            artifacts.add("transfer-resource-observation-v1", observer_summary)
            if observer_csv is not None:
                artifacts.add("transfer-resource-samples-v1", observer_csv)
            observer_process = None

        if candidate_endpoint.client_id == "urtorrent" and mode == "seed":
            if not direct_stop.exists():
                _create_marker(direct_stop, b"stop\n")
            if candidate_endpoint.direct_process is not None:
                exit_code = candidate_endpoint.direct_process.wait(timeout_seconds=min(timeout_seconds, 300))
                if exit_code != 0:
                    raise PairedRunnerError(f"urtorrent seeder exited with status {exit_code}")
                candidate_endpoint.direct_result = _runner_result(candidate_endpoint)
                if candidate_endpoint.direct_result is None:
                    raise PairedRunnerError("urtorrent seeder omitted its final result counters")
                raw_count = candidate_endpoint.direct_result.get("uploaded")
                measured_uploaded = raw_count if isinstance(raw_count, int) and raw_count >= 0 else None

        receiver_data = target_endpoint.paths.data
        durations["flush"] = 0
        flush_started = time.perf_counter_ns()
        _fsync_payload(receiver_data, fixture_manifest, fixture_manifest["profile"]["torrent_name"])
        flush_finished = time.perf_counter_ns()
        durations["flush"] = flush_finished - flush_started

        validation_started = time.perf_counter_ns()
        expected_bytes, expected_sha, observed_digest, hash_failures = _validate_payload(
            receiver_data,
            fixture_manifest,
            fixture_manifest["profile"]["torrent_name"],
        )
        validation_finished = time.perf_counter_ns()
        durations["validation"] = validation_finished - validation_started
        fixture_unchanged = _fixture_matches_identities(
            fixture_dir, fixture_manifest, fixture_identities
        )
        integrity = {
            "status": "passed" if hash_failures == 0 else "failed",
            "expected_sha256": expected_sha,
            "observed_sha256": observed_digest,
            "hash_failures": hash_failures,
            "false_completions": 0 if hash_failures == 0 else 1,
        }
        if not fixture_unchanged:
            status = "invalid"
            reason = "fixture payload changed while the trial was running"
        elif hash_failures:
            status = "invalid"
            reason = "receiver payload failed the independent per-file digest check"
        else:
            status = "complete"
            payload["useful_bytes"] = expected_bytes
        if measured_received is not None:
            payload["received_bytes"] = measured_received
        if measured_uploaded is not None:
            payload["uploaded_bytes"] = measured_uploaded

        # Retire only the exact trial torrent, then stop the pinned processes.
        shutdown_started_ns = time.perf_counter_ns()
        for endpoint in (candidate_endpoint, helper_endpoint):
            if endpoint.controller is not None:
                try:
                    endpoint.controller.remove(info_hash, delete_data=False)
                except ClientControlError as error:
                    if status == "complete":
                        status = "failed"
                    reason = f"client refused exact-hash torrent cleanup: {error}"
        for endpoint in (candidate_endpoint, helper_endpoint):
            if endpoint.managed is not None:
                endpoint.managed.stop()
            if endpoint.direct_process is not None:
                try:
                    endpoint.direct_process.wait(timeout_seconds=0.01)
                except subprocess.TimeoutExpired:
                    endpoint.direct_process.stop(grace_seconds=10)
                finally:
                    endpoint.direct_process.close()
        if tracker_process is not None:
            _stop_task_tracker_worker(tracker_process, tracker_path)
            tracker_process = None
        for apply_path, reset_path, _record in network_applied:
            reset_record = reset_profile(apply_path, reset_path)
            artifacts.add("transfer-network-control-v1", reset_path)
            if reset_record.get("action") != "reset":
                raise PairedRunnerError("network control did not confirm qdisc reset")
        if topology_record is not None:
            cleanup_topology_record(topology_record)
        durations["shutdown"] = time.perf_counter_ns() - shutdown_started_ns

        if tracker is not None:
            if not tracker_path.exists():
                _write_private_json(tracker_path, tracker.snapshot())
            artifacts.add("transfer-task-tracker-v1", tracker_path)

        if status == "complete" and (resource_value is None or candidate_observation is None):
            status = "failed"
            reason = "complete transfer lacks client identity or independent resource evidence"
        if status == "complete" and not durations["transfer"]:
            status = "incomplete"
            reason = "complete payload had no measurable useful-transfer interval"
    except BaseException as error:
        reason = str(error)
        if candidate_observation is None:
            status = "unmeasured"
        elif status != "invalid":
            status = "incomplete"
        if observer_process is not None and observer_stop is not None:
            try:
                observer_code = _stop_observer(observer_process, observer_stop)
                if observer_code == 0 and observer_summary is not None and observer_summary.is_file():
                    resource_value = _resource_observation(observer_summary)
                    resource_sha = _sha256_file(observer_summary, "resource observation")
                    artifacts.add("transfer-resource-observation-v1", observer_summary)
                    if observer_csv is not None:
                        artifacts.add("transfer-resource-samples-v1", observer_csv)
                observer_process = None
            except Exception as cleanup_error:
                reason += f"; observer error: {cleanup_error}"
        # Cleanup is best-effort but identity guarded. Keep the task artifacts
        # when cleanup itself fails so its exact state remains inspectable.
        if shutdown_started_ns is None:
            shutdown_started_ns = time.perf_counter_ns()
        for managed in prelaunched.values():
            if (
                managed is not None
                and managed is not candidate_endpoint.managed
                and managed is not helper_endpoint.managed
            ):
                try:
                    managed.stop()
                except Exception as cleanup_error:
                    reason += f"; unattached container cleanup error: {cleanup_error}"
        for endpoint in (candidate_endpoint, helper_endpoint):
            try:
                if endpoint.managed is not None:
                    endpoint.managed.stop()
                if endpoint.direct_process is not None:
                    try:
                        endpoint.direct_process.wait(timeout_seconds=0.01)
                    except subprocess.TimeoutExpired:
                        endpoint.direct_process.stop(grace_seconds=10)
                    finally:
                        endpoint.direct_process.close()
            except Exception as cleanup_error:
                reason += f"; cleanup error: {cleanup_error}"
        if tracker_process is not None:
            try:
                _stop_task_tracker_worker(tracker_process, tracker_path)
                tracker_process = None
            except Exception as cleanup_error:
                reason += f"; tracker cleanup error: {cleanup_error}"
        for apply_path, reset_path, _record in network_applied:
            try:
                reset_profile(apply_path, reset_path)
                artifacts.add("transfer-network-control-v1", reset_path)
            except Exception as cleanup_error:
                reason += f"; network reset error: {cleanup_error}"
        if topology_record is not None:
            try:
                cleanup_topology_record(topology_record)
            except Exception as cleanup_error:
                reason += f"; topology cleanup error: {cleanup_error}"
        if tracker is not None and not tracker_path.exists():
            try:
                _write_private_json(tracker_path, tracker.snapshot())
                artifacts.add("transfer-task-tracker-v1", tracker_path)
            except Exception:
                pass
        elif tracker is not None:
            artifacts.add("transfer-task-tracker-v1", tracker_path)
        if durations["shutdown"] == 0:
            durations["shutdown"] = max(1, time.perf_counter_ns() - shutdown_started_ns)

    finished_at = _utc_now()
    if candidate_observation_path is not None and candidate_observation_sha is not None:
        candidate_observation_sha = _sha256_file(candidate_observation_path, "candidate observation")
    elif candidate_observation is not None:
        candidate_observation_path = run_dir / "client-candidate.json"
        candidate_observation_sha = write_new_json(candidate_observation_path, candidate_observation)
        artifacts.add("transfer-client-observation-v1", candidate_observation_path)

    if status == "unmeasured":
        candidate_observation = None
        candidate_observation_sha = None
        resource_value = None
        resource_sha = None
        integrity = {
            "status": "not_run",
            "expected_sha256": None,
            "observed_sha256": None,
            "hash_failures": 0,
            "false_completions": 0,
        }
    planned_record = {
        "trial_id": trial_suffix,
        "pair_id": f"{planned['pair_id']}:{mode}",
        "attempt": planned["attempt"],
        "case_id": planned["case_id"],
        "client_id": candidate_id,
    }
    environment = _environment_for_trial(
        environment_template,
        topology_digest,
        cache_value,
        cache_sha,
        network_profile if network_applied else None,
        network_digests,
        network_seeds,
    )
    trial_workload = {
        "case_id": planned["case_id"],
        "kind": mode,
        "protocol": fixture_manifest["profile"]["torrent_kind"],
        "fixture_sha256": fixture_manifest["fixture_sha256"],
        "expected_payload_bytes": sum(item["size_bytes"] for item in fixture_manifest["files"]),
    }
    if status == "complete" and integrity["status"] == "passed":
        payload["useful_bytes"] = trial_workload["expected_payload_bytes"]
    trial = build_trial_record(
        planned_trial=planned_record,
        harness_identity=harness_identity,
        client_observation=candidate_observation,
        client_observation_sha256=candidate_observation_sha,
        workload=trial_workload,
        environment=environment,
        durations_ns=durations,
        payload=payload,
        resource_observation=resource_value,
        resource_observation_sha256=resource_sha,
        integrity=integrity,
        outcome={"status": status, "reason": reason, "exit_code": exit_code},
        started_at=started_at,
        finished_at=finished_at,
    )
    trial_path = run_dir / "trial.json"
    write_trial(trial_path, trial)
    artifacts.trials.append(trial_path)
    if retargeted_path.is_file():
        artifacts.add("transfer-retargeted-metainfo-v1", retargeted_path)
    return trial_path


def _fixture_for_plan(
    root: Path, plan: dict[str, Any], planned: dict[str, Any], profiles_path: Path
) -> tuple[Path, dict[str, Any]]:
    pair_key = hashlib.sha256(planned["pair_id"].encode("utf-8")).hexdigest()[:20]
    fixture_dir = root / "fixtures" / pair_key
    if fixture_dir.exists() or fixture_dir.is_symlink():
        manifest = verify_fixture(fixture_dir)
        profile = manifest.get("profile")
        if not isinstance(profile, dict) or profile.get("id") != planned["profile_id"]:
            raise PairedRunnerError("existing fixture directory belongs to a different plan profile")
        return fixture_dir, manifest
    from fixtures import load_fixture_profile

    profile = load_fixture_profile(profiles_path, planned["profile_id"])
    seed = "perf110-" + hashlib.sha256(
        f"{plan['plan_id']}\0{planned['profile_id']}\0{planned['repetition']}".encode("utf-8")
    ).hexdigest()
    generate_fixture(
        fixture_dir,
        seed,
        profile["files"],
        announce_url="http://127.0.0.1:9/announce",
        torrent_name=profile["torrent_name"],
        piece_length=profile["piece_length"],
        torrent_kind=profile["torrent_kind"],
        profile_id=planned["profile_id"],
    )
    manifest = verify_fixture(fixture_dir)
    return fixture_dir, manifest


def run_plan(
    *,
    plan_path: Path,
    pins_path: Path,
    preflight_path: Path,
    environment_path: Path,
    scratch_root: Path,
    helper_id: str,
    network_profile: str | None,
    cache_state: str,
    timeout_seconds: int,
    observer_interval_ms: int,
    manifest_path: Path | None,
    trial_id_filter: set[str] | None = None,
    profiles_path: Path = DEFAULT_PROFILE_PATH,
) -> list[Path]:
    scratch_root = _ensure_private_root(scratch_root)
    for directory in (scratch_root / "runs", scratch_root / "endpoints", scratch_root / "fixtures"):
        if not directory.exists():
            _mkdir_private(directory)
        elif directory.is_symlink() or not directory.is_dir():
            raise PairedRunnerError(f"scratch subdirectory is not a real directory: {directory}")
    plan, profiles, preflight, environment, staged_inputs = _load_inputs(
        plan_path, pins_path, preflight_path, environment_path, profiles_path, scratch_root
    )
    staged_profiles_path = next(
        (path for kind, path in staged_inputs if kind == "transfer-fixture-profiles-v1"),
        None,
    )
    if staged_profiles_path is None:
        raise PairedRunnerError("fixture profiles were not staged with the source-bound inputs")
    if helper_id == "urtorrent" or helper_id not in profiles:
        raise PairedRunnerError("helper client must be a pinned API-controlled client")
    if timeout_seconds <= 0 or timeout_seconds > 24 * 60 * 60:
        raise PairedRunnerError("trial timeout must be within 1..86400 seconds")
    if observer_interval_ms < 10 or observer_interval_ms > 60_000:
        raise PairedRunnerError("observer interval must be within 10..60000 ms")
    if cache_state not in {"warm", "cold"}:
        raise PairedRunnerError("cache state must be warm or cold")
    harness_identity = capture_harness_identity(ROOT)
    if manifest_path is not None and harness_identity.get("dirty_worktree") is not False:
        raise PairedRunnerError(
            "commit the exact harness source and use a clean worktree before requesting a manifest"
        )
    if manifest_path is not None and manifest_path.resolve(strict=False).parent != scratch_root:
        raise PairedRunnerError("manifest output must be directly in the task scratch root")
    artifacts = RunArtifacts()
    for kind, path in staged_inputs:
        artifacts.add(kind, path)

    selected = [
        item for item in plan["schedule"]
        if trial_id_filter is None or item["trial_id"] in trial_id_filter
    ]
    if trial_id_filter is not None:
        known_trial_ids = {item["trial_id"] for item in plan["schedule"]}
        unknown_trial_ids = trial_id_filter - known_trial_ids
        if unknown_trial_ids:
            raise PairedRunnerError(
                "trial filter contains unknown IDs: " + ", ".join(sorted(unknown_trial_ids))
            )
    if not selected:
        raise PairedRunnerError("plan contains no scheduled trials selected for execution")
    fixture_cache: dict[
        str,
        tuple[Path, dict[str, Any], dict[str, tuple[int, int, int, int, int]]],
    ] = {}
    for planned in selected:
        candidate_id = planned["client_id"]
        if candidate_id == helper_id and len(plan["pinned_client_ids"]) < 2:
            raise PairedRunnerError("candidate/helper comparison needs two independently pinned clients")
        _check_eligible(plan, preflight, {candidate_id, helper_id}, planned["profile_id"])
        fixture_key = planned["pair_id"]
        if fixture_key not in fixture_cache:
            fixture_dir, fixture_manifest = _fixture_for_plan(
                scratch_root, plan, planned, staged_profiles_path
            )
            fixture_cache[fixture_key] = (
                fixture_dir,
                fixture_manifest,
                _fixture_file_identities(fixture_dir, fixture_manifest),
            )
        fixture_dir, fixture_manifest, fixture_identities = fixture_cache[fixture_key]
        fixture_manifest_path = fixture_dir / "fixture.json"
        torrent_manifest_path = fixture_dir / "torrent.json"
        artifacts.add("transfer-fixture-manifest-v1", fixture_manifest_path)
        artifacts.add("transfer-fixture-torrent-record-v1", torrent_manifest_path)
        for mode in ("download", "seed"):
            _run_trial(
                planned=planned,
                mode=mode,
                candidate_id=candidate_id,
                helper_id=helper_id,
                profiles=profiles,
                fixture_dir=fixture_dir,
                fixture_manifest=fixture_manifest,
                fixture_identities=fixture_identities,
                scratch_root=scratch_root,
                environment_template=environment,
                harness_identity=harness_identity,
                network_profile=network_profile,
                cache_state=cache_state,
                timeout_seconds=timeout_seconds,
                observer_interval_ms=observer_interval_ms,
                artifacts=artifacts,
            )
    if manifest_path is not None:
        create_manifest(
            manifest_path,
            artifacts.trials,
            artifacts.manifest_artifacts(),
            repo=ROOT,
        )
    return artifacts.trials


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--pins", type=Path, required=True)
    parser.add_argument("--preflight", type=Path, required=True)
    parser.add_argument("--environment", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, default=DEFAULT_PROFILE_PATH)
    parser.add_argument("--scratch-root", type=Path, required=True)
    parser.add_argument("--helper-client", required=True)
    parser.add_argument("--network-profile", choices=sorted(NETWORK_PROFILES))
    parser.add_argument("--cache-state", choices=("warm", "cold"), default="cold")
    parser.add_argument("--timeout-seconds", type=int, default=DEFAULT_TIMEOUT_SECONDS)
    parser.add_argument("--observer-interval-ms", type=int, default=250)
    parser.add_argument("--manifest", type=Path)
    parser.add_argument("--trial-id", action="append", dest="trial_ids")
    args = parser.parse_args(argv)
    try:
        if args.manifest is not None:
            manifest_path = args.manifest.resolve(strict=False)
            scratch_path = args.scratch_root.resolve(strict=True)
            if manifest_path.parent != scratch_path:
                raise PairedRunnerError("manifest output must be directly in the task scratch root")
        trials = run_plan(
            plan_path=args.plan,
            pins_path=args.pins,
            preflight_path=args.preflight,
            environment_path=args.environment,
            profiles_path=args.profiles,
            scratch_root=args.scratch_root,
            helper_id=args.helper_client,
            network_profile=args.network_profile,
            cache_state=args.cache_state,
            timeout_seconds=args.timeout_seconds,
            observer_interval_ms=args.observer_interval_ms,
            manifest_path=args.manifest,
            trial_id_filter=set(args.trial_ids) if args.trial_ids else None,
        )
    except (
        AdapterError,
        ClientControlError,
        ManagedClientError,
        PairedRunnerError,
        ProcessSupervisorError,
        TaskNetworkError,
        TrialRecordError,
        OSError,
        ValueError,
        subprocess.SubprocessError,
    ) as error:
        print(f"paired runner: {error}", file=sys.stderr)
        return 2
    print(json.dumps({"schema_version": RUNNER_SCHEMA_VERSION, "trials": [str(path) for path in trials]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
