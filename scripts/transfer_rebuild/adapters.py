#!/usr/bin/env python3
"""Read-only identity and settings probes for transfer benchmark clients.

Client profiles pin the executable or container image and declare the settings
that must match the trial contract. Credentials are named by environment
variable; this module never writes them into an observation artifact.
"""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import http.cookiejar
import ipaddress
import json
import math
import os
import re
import shutil
import socket
import stat
import subprocess
import sys
import urllib.parse
import urllib.error
import urllib.request
import xmlrpc.client
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from strict_json import StrictJsonError, loads as strict_json_loads


SCHEMA_VERSION = "transfer-client-observation-v1"
PINS_SCHEMA_VERSION = "transfer-client-pins-v1"
PAYLOAD_COPY_SCHEMA_VERSION = "transfer-payload-copy-observation-v1"
CLIENT_IDS = {
    "torrentngd", "qbittorrent", "transmission", "deluge", "rtorrent", "rqbit", "urtorrent"
}
SECRET_KEY_PARTS = (
    "password", "passwd", "secret", "token", "cookie", "api_key", "apikey",
    "authorization", "credential", "userpass",
)
IMAGE_REF_RE = re.compile(r"^.+@sha256:[a-f0-9]{64}$")
SHA256_RE = re.compile(r"^[a-f0-9]{64}$")
URL_USERINFO_RE = re.compile(r"(?i)(\b[a-z][a-z0-9+.-]*://)[^/\s@]+@")
URL_SECRET_QUERY_RE = re.compile(
    r"(?i)([?&](?:password|passwd|secret|token|api[_-]?key|apikey)=)[^&#\s]*"
)
PAYLOAD_COPY_METRICS = {
    "peer_v2_assembly": "torrentng_payload_copy_peer_v2_assembly_bytes_total",
    "peer_file_segment": "torrentng_payload_copy_peer_file_segment_bytes_total",
    "webseed_reassembly": "torrentng_payload_copy_webseed_reassembly_bytes_total",
}
AUTH_VALUE_RE = re.compile(r"(?i)\b(Bearer|Basic)\s+[^\s,;]+")
SECRET_OPTION_RE = re.compile(r"(?i)(?:password|passwd|secret|token|api[_-]?key|apikey)")


class AdapterError(RuntimeError):
    """A client could not be identified or its effective settings were unknown."""


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, file_pointer, code, message, headers, new_url):
        return None


def _canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def _sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            digest.update(chunk)
    return digest.hexdigest()


def _sha256_regular_file_nofollow(path: Path, label: str) -> tuple[str, os.stat_result]:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise AdapterError(f"cannot open pinned {label} without following links") from error
    digest = hashlib.sha256()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise AdapterError(f"pinned {label} must be a regular file")
        bytes_read = 0
        while chunk := os.read(descriptor, 1024 * 1024):
            digest.update(chunk)
            bytes_read += len(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    before_identity = (
        before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns
    )
    after_identity = (
        after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns
    )
    if before_identity != after_identity or bytes_read != before.st_size:
        raise AdapterError(f"pinned {label} changed while its digest was captured")
    return digest.hexdigest(), before


def _read_regular_file_nofollow(path: Path, label: str, max_bytes: int) -> bytes:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise AdapterError(f"cannot open pinned {label} without following links") from error
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size > max_bytes:
            raise AdapterError(f"pinned {label} must be a bounded regular file")
        content = bytearray()
        while len(content) <= max_bytes:
            chunk = os.read(descriptor, min(4096, max_bytes + 1 - len(content)))
            if not chunk:
                break
            content.extend(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    if (
        len(content) > max_bytes
        or (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns)
        != (after.st_dev, after.st_ino, after.st_size, after.st_mtime_ns, after.st_ctime_ns)
    ):
        raise AdapterError(f"pinned {label} changed while it was read")
    return bytes(content)


def _assert_native_process_identity(pid: int, identity: dict[str, Any]) -> None:
    if _process_start_ticks(pid) != identity.get("process_start_ticks"):
        raise AdapterError("native client process restarted after its identity was captured")
    try:
        executable = os.stat(f"/proc/{pid}/exe")
    except OSError as error:
        raise AdapterError("cannot recheck the native client executable identity") from error
    if (executable.st_dev, executable.st_ino) != (
        identity.get("process_exe_device"),
        identity.get("process_exe_inode"),
    ):
        raise AdapterError("native client process exec identity changed after it was pinned")


def _redact(value: Any) -> Any:
    if isinstance(value, dict):
        return {
            str(key): ("<redacted>" if any(part in str(key).lower() for part in SECRET_KEY_PARTS)
                       else _redact(child))
            for key, child in value.items()
        }
    if isinstance(value, list):
        return [_redact(child) for child in value]
    if isinstance(value, tuple):
        return [_redact(child) for child in value]
    if isinstance(value, str):
        value = URL_USERINFO_RE.sub(r"\1<redacted>@", value)
        value = URL_SECRET_QUERY_RE.sub(r"\1<redacted>", value)
        return AUTH_VALUE_RE.sub(r"\1 <redacted>", value)
    return value


def _redact_argv(arguments: list[str]) -> list[str]:
    safe: list[str] = []
    redact_next = False
    for argument in arguments:
        if redact_next:
            safe.append("<redacted>")
            redact_next = False
            continue
        option, separator, value = argument.partition("=")
        if SECRET_OPTION_RE.search(option):
            if separator:
                safe.append(f"{option}=<redacted>")
            else:
                safe.append(argument)
                redact_next = True
        else:
            safe.append(_redact(argument))
    if redact_next:
        safe.append("<missing>")
    return safe


def _process_start_ticks(pid: int) -> int:
    try:
        proc_stat = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    except (OSError, UnicodeDecodeError) as error:
        raise AdapterError("cannot read live process start time") from error
    stat_fields = proc_stat[proc_stat.rfind(")") + 2:].split()
    if len(stat_fields) <= 19:
        raise AdapterError("live process stat record is incomplete")
    try:
        return int(stat_fields[19])
    except ValueError as error:
        raise AdapterError("live process start time is malformed") from error


def _tcp_listener_addresses(
    table: str, port: int
) -> list[tuple[ipaddress.IPv4Address | ipaddress.IPv6Address, str]]:
    listeners: list[tuple[ipaddress.IPv4Address | ipaddress.IPv6Address, str]] = []
    for line in table.splitlines()[1:]:
        fields = line.split()
        if len(fields) < 12 or fields[3] != "0A":
            continue
        try:
            encoded_address, encoded_port = fields[1].rsplit(":", 1)
            local_port = int(encoded_port, 16)
            packed = bytes.fromhex(encoded_address)
            if len(packed) == 4:
                address = ipaddress.ip_address(packed[::-1])
            elif len(packed) == 16:
                address = ipaddress.ip_address(
                    b"".join(packed[index : index + 4][::-1] for index in range(0, 16, 4))
                )
            else:
                continue
        except (ValueError, IndexError):
            continue
        if local_port == port:
            # Recent Linux kernels combine tx/rx and timer columns into
            # colon-delimited fields, moving the socket inode from column 11
            # to column 9 and appending reference/pointer fields afterward.
            # Older procfs layouts expose those columns separately.
            inode_index = 9 if ":" in fields[4] and ":" in fields[5] else 11
            if len(fields) > inode_index and fields[inode_index].isdecimal():
                listeners.append((address, fields[inode_index]))
    return listeners


def _tcp_listener_inodes(table: str, port: int) -> set[str]:
    return {inode for _, inode in _tcp_listener_addresses(table, port)}


def _settings_subset(observed: Any, expected: Any, path: str = "settings") -> list[str]:
    """Return mismatching paths; compare JSON types as well as values."""
    if isinstance(expected, dict):
        if not isinstance(observed, dict):
            return [path]
        errors: list[str] = []
        for key, value in expected.items():
            if key not in observed:
                errors.append(f"{path}.{key}")
            else:
                errors.extend(_settings_subset(observed[key], value, f"{path}.{key}"))
        return errors
    if type(observed) is not type(expected) or observed != expected:
        return [path]
    return []


@dataclass
class HttpJson:
    base_url: str
    timeout: float = 10.0

    def __post_init__(self) -> None:
        self.base_url = self.base_url.rstrip("/")
        self.jar = http.cookiejar.CookieJar()
        self.opener = urllib.request.build_opener(
            _NoRedirect(), urllib.request.HTTPCookieProcessor(self.jar)
        )
        self.headers: dict[str, str] = {"Accept": "application/json"}
        self.transmission_session_id = ""

    def request(self, path: str, *, method: str = "GET", body: Any = None,
                form: dict[str, str] | None = None, raw_body: bytes | None = None,
                content_type: str | None = None,
                headers: dict[str, str] | None = None) -> bytes:
        request_headers = dict(self.headers)
        request_headers.update(headers or {})
        if sum(value is not None for value in (body, form, raw_body)) > 1:
            raise AdapterError("HTTP request accepts only one body encoding")
        data = None
        if raw_body is not None:
            data = raw_body
            if content_type:
                request_headers.setdefault("Content-Type", content_type)
        elif body is not None:
            data = _canonical_json(body)
            request_headers.setdefault("Content-Type", "application/json")
        elif form is not None:
            data = urllib.parse.urlencode(form).encode()
            request_headers.setdefault("Content-Type", "application/x-www-form-urlencoded")
        if self.transmission_session_id and path.rstrip("/").endswith("transmission/rpc"):
            request_headers["X-Transmission-Session-Id"] = self.transmission_session_id
        request = urllib.request.Request(
            self.base_url + path,
            data=data,
            headers=request_headers,
            method=method,
        )
        try:
            with self.opener.open(request, timeout=self.timeout) as response:
                raw = response.read(8 * 1024 * 1024 + 1)
                if len(raw) > 8 * 1024 * 1024:
                    raise AdapterError(f"response for {path} exceeds 8 MiB")
        except urllib.error.HTTPError as error:
            session_id = error.headers.get("X-Transmission-Session-Id")
            if error.code == 409 and session_id and not self.transmission_session_id:
                self.transmission_session_id = session_id
                return self.request(
                    path, method=method, body=body, form=form, raw_body=raw_body,
                    content_type=content_type, headers=headers,
                )
            raise AdapterError(f"HTTP {error.code} for {path}") from error
        return raw

    def json(self, path: str, *, method: str = "GET", body: Any = None,
             form: dict[str, str] | None = None,
             headers: dict[str, str] | None = None) -> Any:
        try:
            return strict_json_loads(
                self.request(path, method=method, body=body, form=form, headers=headers),
                label=f"{path} response",
            )
        except StrictJsonError as error:
            raise AdapterError(f"{path} did not return strict JSON") from error

    def text(self, path: str, *, method: str = "GET", body: Any = None,
             form: dict[str, str] | None = None,
             headers: dict[str, str] | None = None) -> str:
        try:
            return self.request(path, method=method, body=body, form=form, headers=headers).decode("utf-8")
        except UnicodeDecodeError as error:
            raise AdapterError(f"{path} did not return valid UTF-8 text") from error


def _env_value(profile: dict[str, Any], key: str, *, required: bool = False) -> str | None:
    env_name = profile.get(key)
    if env_name is None:
        if required:
            raise AdapterError(f"profile is missing {key}")
        return None
    if not isinstance(env_name, str) or not env_name:
        raise AdapterError(f"{key} must name an environment variable")
    value = os.environ.get(env_name)
    if value is None and required:
        raise AdapterError(f"required credential environment variable {env_name} is unset")
    return value


def _check_version(profile: dict[str, Any], version: str) -> None:
    expected = profile.get("expected_version")
    if not isinstance(expected, str) or not expected:
        raise AdapterError("each client profile must pin expected_version")
    if version != expected:
        raise AdapterError(f"client version mismatch: expected {expected!r}, observed {version!r}")


def _docker_launch_config(
    container: str,
) -> tuple[list[str], dict[str, str], int, int, dict[str, str]]:
    try:
        raw = subprocess.run(
            ["docker", "inspect", "--format", "{{json .}}", container],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip()
        inspected = strict_json_loads(raw, label="container inspect")
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError, StrictJsonError) as error:
        raise AdapterError(f"cannot read running launch settings for {container!r}") from error
    if not isinstance(inspected, dict) or not isinstance(inspected.get("State"), dict):
        raise AdapterError("docker inspect returned an unexpected container record")
    if inspected["State"].get("Running") is not True:
        raise AdapterError(f"container {container!r} is not running")
    command = inspected.get("Path")
    arguments = inspected.get("Args")
    config = inspected.get("Config")
    environment = config.get("Env") if isinstance(config, dict) else None
    pid = inspected["State"].get("Pid")
    mounts = inspected.get("Mounts")
    if arguments is None:
        arguments = []
    if environment is None:
        environment = []
    if mounts is None:
        mounts = []
    if (
        not isinstance(pid, int)
        or pid <= 0
        or not isinstance(command, str)
        or not command
        or not isinstance(arguments, list)
        or not all(isinstance(argument, str) for argument in arguments)
        or not isinstance(environment, list)
        or not all(isinstance(item, str) and "=" in item for item in environment)
        or not isinstance(mounts, list)
    ):
        raise AdapterError("docker inspect returned incomplete process command or environment")
    env = dict(item.split("=", 1) for item in environment)
    bind_mounts: dict[str, str] = {}
    for mount in mounts:
        if not isinstance(mount, dict):
            raise AdapterError("docker inspect returned a malformed mount record")
        source, destination = mount.get("Source"), mount.get("Destination")
        if mount.get("Type") == "bind" and isinstance(source, str) and isinstance(destination, str):
            bind_mounts[source] = destination
    return [command, *arguments], env, pid, _process_start_ticks(pid), bind_mounts


def _rqbit_launch_settings(profile: dict[str, Any], runtime: str,
                           runtime_identity: dict[str, Any]) -> dict[str, Any]:
    allowlist = profile.get("settings_env_allowlist")
    if not isinstance(allowlist, list) or not all(
        isinstance(name, str) and name for name in allowlist
    ):
        raise AdapterError("rqbit profile must pin settings_env_allowlist as an explicit list")
    if len(set(allowlist)) != len(allowlist):
        raise AdapterError("rqbit settings_env_allowlist cannot contain duplicate names")
    file_allowlist = profile.get("settings_file_allowlist")
    if not isinstance(file_allowlist, list) or not all(
        isinstance(path, str) and path for path in file_allowlist
    ):
        raise AdapterError("rqbit profile must pin settings_file_allowlist as an explicit list")
    if runtime == "container":
        container = profile.get("container")
        if not isinstance(container, str):
            raise AdapterError("rqbit container profile has no container name")
        argv, environment, pid, start_ticks, bind_mounts = _docker_launch_config(container)
        if pid != runtime_identity.get("process_id") or start_ticks != runtime_identity.get("process_start_ticks"):
            raise AdapterError("rqbit container restarted while its launch settings were being read")
    else:
        pid = runtime_identity.get("process_id")
        if not isinstance(pid, int):
            raise AdapterError("rqbit native identity has no process ID")
        try:
            raw_argv = Path(f"/proc/{pid}/cmdline").read_bytes().split(b"\0")
            argv = [argument.decode("utf-8") for argument in raw_argv if argument]
            raw_environment = Path(f"/proc/{pid}/environ").read_bytes().split(b"\0")
            environment = {}
            for item in raw_environment:
                if item:
                    key, separator, value = item.decode("utf-8").partition("=")
                    if separator:
                        environment[key] = value
        except (OSError, UnicodeDecodeError) as error:
            raise AdapterError("cannot read rqbit launch settings from its live process") from error
        if _process_start_ticks(pid) != runtime_identity.get("process_start_ticks"):
            raise AdapterError("rqbit process restarted while its launch settings were being read")
        bind_mounts = {}
    if not argv:
        raise AdapterError("rqbit process has no observable command line")
    missing = sorted(set(allowlist) - set(environment))
    if missing:
        raise AdapterError("rqbit process is missing allowlisted environment settings: " + ", ".join(missing))
    config_files: dict[str, str] = {}
    for raw_path in file_allowlist:
        path = Path(raw_path).expanduser()
        if not path.is_absolute() or path.is_symlink() or not path.is_file():
            raise AdapterError(f"rqbit settings file must be an existing absolute non-symlink file: {raw_path}")
        resolved_path = path.resolve(strict=True)
        if runtime == "container":
            destination = bind_mounts.get(str(resolved_path))
            if destination is None:
                raise AdapterError(f"rqbit settings file is not a bind mount of the pinned container: {raw_path}")
            config_files[destination] = _sha256_file(resolved_path)
        else:
            config_files[str(resolved_path)] = _sha256_file(resolved_path)
    return {
        "argv": _redact_argv(argv),
        "environment": _redact({name: environment[name] for name in allowlist}),
        "config_files_sha256": config_files,
    }


def _docker_identity(profile: dict[str, Any], *, capture_command: bool = False) -> dict[str, Any]:
    container = profile.get("container")
    image_ref = profile.get("image_ref")
    if not isinstance(container, str) or not container:
        raise AdapterError("container client profile must name a container")
    if not isinstance(image_ref, str) or not IMAGE_REF_RE.fullmatch(image_ref):
        raise AdapterError("container client image_ref must be repository@sha256:<64 hex>")
    try:
        inspected = subprocess.run(
            ["docker", "inspect", "--format", "{{.Id}} {{.Image}} {{.State.Running}} {{.State.Pid}}", container],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip().split()
        if len(inspected) != 4:
            raise AdapterError("docker inspect returned an unexpected identity record")
        container_id, image_id, running, raw_pid = inspected
        if running != "true":
            raise AdapterError(f"container {container!r} is not running")
        try:
            pid = int(raw_pid)
        except ValueError as error:
            raise AdapterError("docker inspect returned an invalid process ID") from error
        if pid <= 0:
            raise AdapterError(f"container {container!r} has no running process")
        start_ticks = _process_start_ticks(pid)
        container_id = container_id.removeprefix("sha256:")
        image_id = image_id.removeprefix("sha256:")
        if not SHA256_RE.fullmatch(container_id) or not SHA256_RE.fullmatch(image_id):
            raise AdapterError("docker inspect returned a malformed container or image ID")
        digests_raw = subprocess.run(
            ["docker", "image", "inspect", "--format", "{{json .RepoDigests}}", image_id],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip()
        repo_digests = strict_json_loads(digests_raw, label="container image inspect")
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError, StrictJsonError) as error:
        raise AdapterError(f"cannot inspect pinned container {container!r}") from error
    if not isinstance(repo_digests, list) or image_ref not in repo_digests:
        raise AdapterError(f"running image does not include pinned digest {image_ref}")
    identity: dict[str, Any] = {
        "kind": "container",
        "container_id": container_id,
        "image_id": image_id,
        "image_ref": image_ref,
        "image_digest": image_ref.rsplit("@", 1)[1],
        "process_id": pid,
        "process_start_ticks": start_ticks,
    }
    if capture_command:
        effective_argv, _, config_pid, config_start_ticks, _ = _docker_launch_config(container)
        if config_pid != pid or config_start_ticks != start_ticks:
            raise AdapterError(f"container {container!r} restarted during identity capture")
        safe_argv = _redact_argv(effective_argv)
        identity["effective_argv_sha256"] = _sha256_bytes(_canonical_json(safe_argv))
        identity["effective_argv_count"] = len(effective_argv)
    return identity


def _binary_identity(profile: dict[str, Any]) -> dict[str, Any]:
    binary = Path(str(profile.get("binary_path", ""))).expanduser()
    pid_file = Path(str(profile.get("pid_file", ""))).expanduser()
    expected_sha = profile.get("expected_binary_sha256")
    source_fields = {
        "source_root", "expected_source_commit", "expected_dirty_worktree",
        "expected_lockfile_sha256", "expected_toolchain",
    }
    supplied_source_fields = source_fields.intersection(profile)
    has_source_identity = bool(supplied_source_fields)
    expected_commit = profile.get("expected_source_commit")
    expected_dirty = profile.get("expected_dirty_worktree")
    expected_lockfile_sha = profile.get("expected_lockfile_sha256")
    if not binary.is_absolute():
        raise AdapterError("native client binary_path must be absolute")
    if not pid_file.is_absolute():
        raise AdapterError("native client pid_file must be absolute")
    if not isinstance(expected_sha, str) or not re.fullmatch(r"[a-f0-9]{64}", expected_sha):
        raise AdapterError("native client profile must pin expected_binary_sha256")
    if has_source_identity and supplied_source_fields != source_fields:
        raise AdapterError("native source identity requires source_root, commit, dirty state, and lockfile pins")
    binary_sha, binary_stat = _sha256_regular_file_nofollow(binary, "native client binary")
    if binary_sha != expected_sha:
        raise AdapterError(f"native client binary digest mismatch: expected {expected_sha}, observed {binary_sha}")
    try:
        pid_text = _read_regular_file_nofollow(pid_file, "native client pid file", 64).decode("ascii").strip()
        if not pid_text.isdecimal() or int(pid_text) <= 0:
            raise AdapterError("native client pid_file must contain one positive process ID")
        pid = int(pid_text)
        executable_stat = os.stat(f"/proc/{pid}/exe")
        if (executable_stat.st_dev, executable_stat.st_ino) != (binary_stat.st_dev, binary_stat.st_ino):
            raise AdapterError("native client process is not running the pinned binary file")
        process_start_ticks = _process_start_ticks(pid)
    except (OSError, UnicodeDecodeError, ValueError) as error:
        raise AdapterError("cannot bind native client identity to its live process") from error
    identity: dict[str, Any] = {
        "kind": "native_binary",
        "binary_path": str(binary),
        "binary_sha256": binary_sha,
        "binary_size_bytes": binary_stat.st_size,
        "process_id": pid,
        "process_start_ticks": process_start_ticks,
        "process_exe_device": executable_stat.st_dev,
        "process_exe_inode": executable_stat.st_ino,
    }
    if has_source_identity:
        source_root = Path(str(profile["source_root"])).expanduser()
        expected_commit = profile["expected_source_commit"]
        expected_dirty = profile["expected_dirty_worktree"]
        expected_lockfile_sha = profile["expected_lockfile_sha256"]
        expected_toolchain = profile["expected_toolchain"]
        if not source_root.is_absolute() or not source_root.is_dir() or source_root.is_symlink():
            raise AdapterError("native client source_root must be an existing absolute non-symlink directory")
        if not isinstance(expected_commit, str) or not re.fullmatch(r"[a-f0-9]{40}", expected_commit):
            raise AdapterError("native client profile must pin expected_source_commit")
        if not isinstance(expected_dirty, bool):
            raise AdapterError("native client profile must pin expected_dirty_worktree")
        if not isinstance(expected_lockfile_sha, str) or not SHA256_RE.fullmatch(expected_lockfile_sha):
            raise AdapterError("native client profile must pin expected_lockfile_sha256")
        if not isinstance(expected_toolchain, str) or not expected_toolchain:
            raise AdapterError("native client profile must pin expected_toolchain")
        try:
            commit = subprocess.run(
                ["git", "-C", str(source_root), "rev-parse", "HEAD"],
                check=True, capture_output=True, text=True, timeout=10,
            ).stdout.strip()
            dirty = bool(subprocess.run(
                ["git", "-C", str(source_root), "status", "--porcelain", "--untracked-files=all"],
                check=True, capture_output=True, text=True, timeout=10,
            ).stdout)
            toolchain = subprocess.run(
                ["rustc", "-Vv"], check=True, capture_output=True, text=True, timeout=10,
                cwd=source_root,
            ).stdout.strip()
        except (OSError, subprocess.SubprocessError) as error:
            raise AdapterError("cannot capture native source/toolchain identity") from error
        if toolchain != expected_toolchain:
            raise AdapterError("native Rust toolchain does not match the declared pin")
        if commit != expected_commit:
            raise AdapterError(f"native source commit mismatch: expected {expected_commit}, observed {commit}")
        if dirty != expected_dirty:
            raise AdapterError(f"native dirty-worktree mismatch: expected {expected_dirty}, observed {dirty}")
        lockfile = source_root / "Cargo.lock"
        if not lockfile.is_file() or lockfile.is_symlink():
            raise AdapterError("native source has no regular Cargo.lock")
        lockfile_sha = _sha256_file(lockfile)
        if lockfile_sha != expected_lockfile_sha:
            raise AdapterError(
                f"native lockfile digest mismatch: expected {expected_lockfile_sha}, observed {lockfile_sha}"
            )
        identity.update({
            "source_commit": commit,
            "dirty_worktree": dirty,
            "lockfile_sha256": lockfile_sha,
            "toolchain": toolchain,
        })
    _assert_native_process_identity(pid, identity)
    return identity


def _task_local_api_context(profile: dict[str, Any]) -> dict[str, Any] | None:
    context = profile.get("_task_local_api")
    if context is None:
        return None
    if not isinstance(context, dict) or set(context) != {
        "address", "namespace", "namespace_identity", "subnet", "interface", "ifindex",
        "topology_sha256",
    }:
        raise AdapterError("task-local API context is malformed")
    name = context.get("namespace")
    identity = context.get("namespace_identity")
    interface = context.get("interface")
    if (
        not isinstance(name, str)
        or re.fullmatch(r"tng[ts]-[a-f0-9]{8,12}", name) is None
        or not isinstance(identity, dict)
        or set(identity) != {"device", "inode"}
        or isinstance(identity.get("device"), bool)
        or not isinstance(identity.get("device"), int)
        or identity["device"] < 0
        or isinstance(identity.get("inode"), bool)
        or not isinstance(identity.get("inode"), int)
        or identity["inode"] <= 0
        or interface != "tngveth0"
        or isinstance(context.get("ifindex"), bool)
        or not isinstance(context.get("ifindex"), int)
        or context["ifindex"] <= 0
        or not isinstance(context.get("topology_sha256"), str)
        or SHA256_RE.fullmatch(context["topology_sha256"]) is None
    ):
        raise AdapterError("task-local API context has an invalid namespace or interface identity")
    try:
        subnet = ipaddress.ip_network(context.get("subnet"), strict=True)
        address = ipaddress.ip_address(context.get("address"))
    except (TypeError, ValueError) as error:
        raise AdapterError("task-local API context has an invalid address or subnet") from error
    host_offset = 3 if name.startswith("tngt-") else 2
    if (
        not isinstance(subnet, ipaddress.IPv4Network)
        or subnet.prefixlen != 24
        or not subnet.subnet_of(ipaddress.ip_network("198.18.0.0/15"))
        or not isinstance(address, ipaddress.IPv4Address)
        or address not in subnet
        or address != subnet.network_address + host_offset
    ):
        raise AdapterError("task-local API address does not match its isolated target subnet")
    return context


def _verify_task_local_namespace(pid: int, context: Mapping[str, Any]) -> None:
    namespace = context["namespace"]
    identity = context["namespace_identity"]
    try:
        namespace_fd = os.open(
            "/run/netns",
            os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
        )
        try:
            descriptor = os.open(
                namespace,
                os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
                dir_fd=namespace_fd,
            )
        finally:
            os.close(namespace_fd)
        try:
            named_identity = os.fstat(descriptor)
        finally:
            os.close(descriptor)
        process_namespace = os.stat(f"/proc/{pid}/ns/net")
    except OSError as error:
        raise AdapterError("cannot recheck the task-local network namespace identity") from error
    expected = (identity["device"], identity["inode"])
    if (named_identity.st_dev, named_identity.st_ino) != expected or (
        process_namespace.st_dev, process_namespace.st_ino
    ) != expected:
        raise AdapterError("client no longer runs in the recorded task network namespace")

    ip = shutil.which("ip")
    if ip is None:
        raise AdapterError("iproute2 is required to verify the task-local client interface")
    try:
        link_result = subprocess.run(
            [ip, "-n", namespace, "-j", "-d", "link", "show"],
            check=True, capture_output=True, text=True, timeout=10,
        )
        address_result = subprocess.run(
            [ip, "-n", namespace, "-j", "address", "show", "dev", context["interface"]],
            check=True, capture_output=True, text=True, timeout=10,
        )
        route_result = subprocess.run(
            [ip, "-n", namespace, "-j", "route", "show", "table", "main"],
            check=True, capture_output=True, text=True, timeout=10,
        )
        links = strict_json_loads(link_result.stdout, label="task namespace link output")
        addresses = strict_json_loads(address_result.stdout, label="task namespace address output")
        routes = strict_json_loads(route_result.stdout, label="task namespace route output")
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError, StrictJsonError) as error:
        raise AdapterError("cannot inspect the task-local client network topology") from error
    if (
        not isinstance(links, list)
        or len(links) != 2
        or {link.get("ifname") for link in links if isinstance(link, dict)} != {"lo", context["interface"]}
    ):
        raise AdapterError("task-local client namespace contains an unexpected interface")
    task_link = next(
        (link for link in links if isinstance(link, dict) and link.get("ifname") == context["interface"]),
        None,
    )
    if (
        not isinstance(task_link, dict)
        or task_link.get("ifindex") != context["ifindex"]
        or "UP" not in task_link.get("flags", [])
        or not isinstance(task_link.get("linkinfo"), dict)
        or task_link["linkinfo"].get("info_kind") != "veth"
    ):
        raise AdapterError("task-local client interface identity changed")
    endpoint = None
    if isinstance(addresses, list) and len(addresses) == 1 and isinstance(addresses[0], dict):
        endpoint = addresses[0].get("addr_info")
    if not isinstance(endpoint, list) or not any(
        isinstance(item, dict)
        and item.get("family") == "inet"
        and item.get("local") == context["address"]
        and item.get("prefixlen") == 24
        for item in endpoint
    ):
        raise AdapterError("task-local API address is no longer assigned to its recorded interface")
    if (
        not isinstance(routes, list)
        or len(routes) != 1
        or not isinstance(routes[0], dict)
        or routes[0].get("dst") != context["subnet"]
    ):
        raise AdapterError("task-local client namespace has an unexpected route")


def _listener_inodes(pid: int, port: int) -> list[tuple[ipaddress.IPv4Address | ipaddress.IPv6Address, str]]:
    listeners: list[tuple[ipaddress.IPv4Address | ipaddress.IPv6Address, str]] = []
    for table in (f"/proc/{pid}/net/tcp", f"/proc/{pid}/net/tcp6"):
        try:
            table_contents = Path(table).read_text(encoding="ascii")
        except OSError as error:
            raise AdapterError("cannot inspect native client network namespace") from error
        listeners.extend(_tcp_listener_addresses(table_contents, port))
    return listeners


def _assert_process_owns_listener(pid: int, inodes: set[str]) -> None:
    try:
        process_fds = Path(f"/proc/{pid}/fd").iterdir()
        owns_listener = False
        for fd in process_fds:
            try:
                target = os.readlink(fd)
            except OSError:
                continue
            match = re.fullmatch(r"socket:\[(\d+)\]", target)
            if match and match.group(1) in inodes:
                owns_listener = True
                break
    except OSError as error:
        raise AdapterError("cannot inspect client process sockets") from error
    if not owns_listener:
        raise AdapterError("pinned client process does not own its configured API listener")


def _verify_task_local_http_listener(pid: int, port: int, context: Mapping[str, Any]) -> None:
    _verify_task_local_namespace(pid, context)
    listeners = _listener_inodes(pid, port)
    expected_address = ipaddress.ip_address(context["address"])
    matching = [
        inode for address, inode in listeners
        if address == expected_address or address.is_unspecified
    ]
    if not matching or len(matching) != len(listeners):
        raise AdapterError("task-local client API is not bound only within its recorded namespace")
    _assert_process_owns_listener(pid, set(matching))


def _verify_native_http_listener(
    parsed: urllib.parse.SplitResult,
    runtime_identity: dict[str, Any] | None,
    task_local: Mapping[str, Any] | None = None,
) -> None:
    if runtime_identity is None or not isinstance(runtime_identity.get("process_id"), int):
        raise AdapterError("native HTTP adapter has no pinned process identity")
    host = parsed.hostname or ""
    if task_local is not None:
        if host != task_local["address"]:
            raise AdapterError("task-local API URL must use the recorded target interface address")
    elif host.lower() == "localhost":
        pass
    else:
        try:
            if not ipaddress.ip_address(host).is_loopback:
                raise AdapterError("native benchmark API must use a loopback address")
        except ValueError as error:
            raise AdapterError("native benchmark API host must be localhost or a loopback IP") from error
    try:
        port = parsed.port or (443 if parsed.scheme == "https" else 80)
    except ValueError as error:
        raise AdapterError("native benchmark API has an invalid port") from error
    pid = runtime_identity["process_id"]
    _assert_native_process_identity(pid, runtime_identity)
    if task_local is not None:
        _verify_task_local_http_listener(pid, port, task_local)
        return
    requested_address = None
    if host.lower() != "localhost":
        requested_address = ipaddress.ip_address(host)
    listening_inodes: set[str] = set()
    observed_listeners: list[tuple[ipaddress.IPv4Address | ipaddress.IPv6Address, str]] = []
    for table in (f"/proc/{pid}/net/tcp", f"/proc/{pid}/net/tcp6"):
        try:
            table_contents = Path(table).read_text(encoding="ascii")
        except OSError as error:
            raise AdapterError("cannot inspect native client network namespace") from error
        listeners = _tcp_listener_addresses(table_contents, port)
        observed_listeners.extend(listeners)
        for address, inode in listeners:
            if not address.is_loopback:
                continue
            if requested_address is not None and address.version != requested_address.version:
                continue
            listening_inodes.add(inode)
    if not observed_listeners or any(not address.is_loopback for address, _ in observed_listeners):
        raise AdapterError("native client API port has a wildcard or non-loopback listener")
    if not listening_inodes:
        raise AdapterError("native client process does not own a listening socket for its API port")
    _assert_process_owns_listener(pid, listening_inodes)


def _verify_native_unix_listener(socket_path: Path,
                                 runtime_identity: dict[str, Any]) -> None:
    pid = runtime_identity.get("process_id")
    if not isinstance(pid, int):
        raise AdapterError("native rTorrent process identity has no process ID")
    _assert_native_process_identity(pid, runtime_identity)
    if socket_path.is_symlink() or not stat.S_ISSOCK(socket_path.stat(follow_symlinks=False).st_mode):
        raise AdapterError("rTorrent SCGI path is not a regular Unix stream socket")
    try:
        rows = Path(f"/proc/{pid}/net/unix").read_text(encoding="ascii").splitlines()[1:]
        matching_inodes = {
            fields[6]
            for line in rows
            if len(fields := line.split(maxsplit=7)) >= 8 and fields[7] == str(socket_path)
        }
        fds = Path(f"/proc/{pid}/fd").iterdir()
        owns_socket = False
        for fd in fds:
            try:
                target = os.readlink(fd)
            except OSError:
                continue
            match = re.fullmatch(r"socket:\[(\d+)\]", target)
            if match and match.group(1) in matching_inodes:
                owns_socket = True
                break
    except OSError as error:
        raise AdapterError("cannot verify rTorrent SCGI socket ownership") from error
    if not owns_socket:
        raise AdapterError("pinned rTorrent process does not own the configured SCGI socket")


def _verify_container_unix_listener(profile: dict[str, Any],
                                    runtime_identity: dict[str, Any],
                                    host_socket_path: Path) -> None:
    container_path = profile.get("container_socket_path")
    pid = runtime_identity.get("process_id")
    if not isinstance(container_path, str) or not container_path.startswith("/"):
        raise AdapterError("rTorrent container profile must pin container_socket_path")
    if ".." in Path(container_path).parts or not isinstance(pid, int):
        raise AdapterError("rTorrent container socket path or process identity is invalid")
    if _process_start_ticks(pid) != runtime_identity.get("process_start_ticks"):
        raise AdapterError("rTorrent container restarted before SCGI readback")
    inside_path = Path(f"/proc/{pid}/root") / container_path.lstrip("/")
    try:
        host_stat = host_socket_path.stat(follow_symlinks=False)
        inside_stat = inside_path.stat(follow_symlinks=False)
        if not stat.S_ISSOCK(host_stat.st_mode) or not stat.S_ISSOCK(inside_stat.st_mode):
            raise AdapterError("rTorrent SCGI path is not a Unix socket in both mount views")
        if (host_stat.st_dev, host_stat.st_ino) != (inside_stat.st_dev, inside_stat.st_ino):
            raise AdapterError("rTorrent SCGI path is not the pinned container's mounted socket")
        rows = Path(f"/proc/{pid}/net/unix").read_text(encoding="ascii").splitlines()[1:]
    except OSError as error:
        raise AdapterError("cannot verify rTorrent container SCGI socket") from error
    if not any(
        len(fields := line.split(maxsplit=7)) >= 7
        and fields[4] == "0001"
        and fields[6] == str(host_stat.st_ino)
        for line in rows
    ):
        raise AdapterError("rTorrent socket is absent from the pinned container network namespace")


def _verify_container_http_endpoint(profile: dict[str, Any],
                                    runtime_identity: dict[str, Any],
                                    parsed: urllib.parse.SplitResult) -> None:
    container = profile.get("container")
    if not isinstance(container, str):
        raise AdapterError("container client profile has no container name")
    try:
        raw = subprocess.run(
            ["docker", "inspect", "--format", "{{json .}}", container],
            check=True,
            capture_output=True,
            text=True,
            timeout=10,
        ).stdout.strip()
        inspected = strict_json_loads(raw, label="container inspect")
    except (OSError, subprocess.SubprocessError, json.JSONDecodeError, StrictJsonError) as error:
        raise AdapterError("cannot bind HTTP endpoint to its pinned container") from error
    if not isinstance(inspected, dict):
        raise AdapterError("docker inspect returned an unexpected container record")
    container_id = str(inspected.get("Id", "")).removeprefix("sha256:")
    state = inspected.get("State")
    host_config = inspected.get("HostConfig")
    network_settings = inspected.get("NetworkSettings")
    if (
        container_id != runtime_identity.get("container_id")
        or not isinstance(state, dict)
        or state.get("Running") is not True
        or state.get("Pid") != runtime_identity.get("process_id")
        or _process_start_ticks(state["Pid"]) != runtime_identity.get("process_start_ticks")
    ):
        raise AdapterError("container identity changed before its HTTP endpoint was read")
    port = parsed.port or (443 if parsed.scheme == "https" else 80)
    if not isinstance(host_config, dict) or not isinstance(network_settings, dict):
        raise AdapterError("docker inspect omitted container network settings")
    network_mode = host_config.get("NetworkMode")
    task_local = _task_local_api_context(profile)
    if task_local is not None:
        if (parsed.hostname or "") != task_local["address"]:
            raise AdapterError("task-local API URL must use the recorded target interface address")
        if network_mode != "none":
            raise AdapterError("task-local comparator containers must start with Docker network mode none")
        _verify_task_local_http_listener(state["Pid"], port, task_local)
        return
    if network_mode == "host":
        if not _namespace_has_listener(state["Pid"], port, loopback_only=True):
            raise AdapterError("host-network client API must listen only on loopback")
        return
    else:
        port_bindings = network_settings.get("Ports") or {}
        if not isinstance(port_bindings, dict):
            raise AdapterError("docker inspect returned malformed published port bindings")
        mapped_port = None
        requested_address = None
        if (parsed.hostname or "").lower() != "localhost":
            try:
                requested_address = ipaddress.ip_address(parsed.hostname or "")
            except ValueError as error:
                raise AdapterError("benchmark API host must be localhost or a loopback IP") from error
        candidate_ports: set[int] = set()
        all_published: list[
            tuple[int, ipaddress.IPv4Address | ipaddress.IPv6Address | None, str]
        ] = []
        for container_port, bindings in port_bindings.items():
            if not isinstance(bindings, list):
                continue
            try:
                internal_port = int(container_port.rsplit("/", 1)[0])
            except (ValueError, IndexError):
                continue
            for binding in bindings:
                if not isinstance(binding, dict):
                    continue
                host_ip = str(binding.get("HostIp", ""))
                host_port = binding.get("HostPort")
                try:
                    published_address = ipaddress.ip_address(host_ip)
                except ValueError:
                    all_published.append((internal_port, None, str(host_port)))
                    continue
                all_published.append((internal_port, published_address, str(host_port)))
                if (
                    host_port == str(port)
                    and published_address.is_loopback
                    and (
                        requested_address is None
                        or requested_address.version == published_address.version
                    )
                ):
                    candidate_ports.add(internal_port)
        for candidate_port in candidate_ports:
            candidate_bindings = [
                address for internal_port, address, _ in all_published
                if internal_port == candidate_port
            ]
            if any(address is None or not address.is_loopback for address in candidate_bindings):
                raise AdapterError(
                    "container API port is also published on a wildcard or non-loopback address"
                )
            if any(
                internal_port == candidate_port
                and host_port == str(port)
                and (
                    requested_address is None
                    or address.version == requested_address.version
                )
                for internal_port, address, host_port in all_published
            ):
                mapped_port = candidate_port
                break
        if mapped_port is None:
            raise AdapterError("HTTP API port is not published by the pinned container")
    if not _namespace_has_listener(state["Pid"], mapped_port):
        raise AdapterError("pinned container network namespace has no API listener on the mapped port")


def _namespace_has_listener(pid: int, port: int, *, loopback_only: bool = False) -> bool:
    for table in (f"/proc/{pid}/net/tcp", f"/proc/{pid}/net/tcp6"):
        try:
            table_contents = Path(table).read_text(encoding="ascii")
        except OSError as error:
            raise AdapterError("cannot inspect client network namespace") from error
        listeners = _tcp_listener_addresses(table_contents, port)
        if listeners and (not loopback_only or all(address.is_loopback for address, _ in listeners)):
            return True
    return False


def _profile_http(profile: dict[str, Any], *, allow_basic_auth: bool = False,
                  runtime_identity: dict[str, Any] | None = None) -> HttpJson:
    base_url = profile.get("base_url")
    if not isinstance(base_url, str):
        raise AdapterError("HTTP client profile requires an http(s) base_url")
    parsed = urllib.parse.urlsplit(base_url)
    if (
        parsed.scheme not in ("http", "https")
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.query
        or parsed.fragment
    ):
        raise AdapterError("base_url must be http(s), have no embedded credentials, query, or fragment")
    try:
        parsed.port
    except ValueError as error:
        raise AdapterError("benchmark client API has an invalid port") from error
    host = parsed.hostname or ""
    task_local = _task_local_api_context(profile)
    if task_local is not None:
        if host != task_local["address"]:
            raise AdapterError("task-local API URL must use the recorded target interface address")
    elif host.lower() == "localhost":
        pass
    else:
        try:
            if not ipaddress.ip_address(host).is_loopback:
                raise AdapterError("benchmark client APIs must use a loopback address")
        except ValueError as error:
            raise AdapterError("benchmark client API host must be localhost or a loopback IP") from error
    if profile.get("runtime") == "native_binary":
        _verify_native_http_listener(parsed, runtime_identity, task_local)
    elif profile.get("runtime") == "container" and runtime_identity is not None:
        _verify_container_http_endpoint(profile, runtime_identity, parsed)
    client = HttpJson(base_url, float(profile.get("timeout_seconds", 10)))
    username = _env_value(profile, "username_env")
    password = _env_value(profile, "password_env")
    if allow_basic_auth and (username is not None or password is not None):
        if username is None or password is None:
            raise AdapterError("both username_env and password_env are required for login")
        token = base64.b64encode(f"{username}:{password}".encode()).decode()
        client.headers["Authorization"] = f"Basic {token}"
    # Other clients authenticate through their own API or a local socket.
    # Never send credentials to a generic HTTP auth challenge.
    return client


def _login_qbittorrent(client: HttpJson, profile: dict[str, Any]) -> None:
    username = _env_value(profile, "username_env")
    password = _env_value(profile, "password_env")
    if username is None and password is None:
        return
    if username is None or password is None:
        raise AdapterError("qBittorrent requires both username_env and password_env")
    response = client.text(
        "/api/v2/auth/login", method="POST",
        form={"username": username, "password": password},
    )
    if response != "Ok.":
        raise AdapterError("qBittorrent Web API login failed")


def _transmission_call(client: HttpJson, method: str, arguments: dict[str, Any] | None = None) -> Any:
    variants = [method]
    if "-" in method:
        variants.append(method.replace("-", "_"))
    elif "_" in method:
        variants.append(method.replace("_", "-"))
    last_error: Exception | None = None
    for variant in variants:
        try:
            response = client.json(
                "/transmission/rpc", method="POST",
                body={"method": variant, "arguments": arguments or {}},
            )
        except AdapterError as error:
            last_error = error
            continue
        if not isinstance(response, dict):
            last_error = AdapterError(f"Transmission RPC {variant} returned a non-object response")
            continue
        if response.get("result") == "success":
            return response.get("arguments", {})
        if "result" in response and isinstance(response["result"], dict):
            return response["result"]
        last_error = AdapterError(f"Transmission RPC {variant} failed: {response.get('result')}")
    raise AdapterError(str(last_error or "Transmission RPC failed"))


def _deluge_call(client: HttpJson, method: str, params: list[Any], request_id: int) -> Any:
    response = client.json(
        "/json", method="POST",
        body={"method": method, "params": params, "id": request_id},
    )
    if not isinstance(response, dict):
        raise AdapterError(f"Deluge RPC {method} returned a non-object response")
    if response.get("error"):
        raise AdapterError(f"Deluge RPC {method} failed: {response['error']}")
    return response.get("result")


def _scgi_call(socket_path: Path, method: str, arguments: list[Any]) -> Any:
    body = xmlrpc.client.dumps(tuple(arguments), methodname=method, allow_none=True).encode()
    headers = (
        b"CONTENT_LENGTH\0" + str(len(body)).encode() + b"\0"
        + b"SCGI\0" + b"1\0"
        + b"REQUEST_METHOD\0POST\0REQUEST_URI\0/RPC2\0CONTENT_TYPE\0text/xml\0"
    )
    request = str(len(headers)).encode() + b":" + headers + b"," + body
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as connection:
        connection.settimeout(10)
        connection.connect(str(socket_path))
        connection.sendall(request)
        response = bytearray()
        body_length: int | None = None
        header_end = -1
        while len(response) <= 4 * 1024 * 1024:
            chunk = connection.recv(65536)
            if not chunk:
                break
            response.extend(chunk)
            if header_end < 0:
                header_end = response.find(b"\r\n\r\n")
                if header_end >= 0:
                    response_headers = bytes(response[:header_end]).split(b"\r\n")
                    for line in response_headers:
                        name, separator, value = line.partition(b":")
                        if separator and name.strip().lower() == b"content-length":
                            try:
                                body_length = int(value.strip())
                            except ValueError as error:
                                raise AdapterError("rTorrent SCGI returned invalid Content-Length") from error
                            if body_length < 0 or body_length > 4 * 1024 * 1024:
                                raise AdapterError("rTorrent SCGI response exceeds 4 MiB")
                            break
            if header_end >= 0 and body_length is not None:
                if len(response) >= header_end + 4 + body_length:
                    break
        if len(response) > 4 * 1024 * 1024:
            raise AdapterError("rTorrent SCGI response exceeds 4 MiB")
    if header_end < 0:
        raise AdapterError("rTorrent SCGI response has no HTTP header terminator")
    response_headers = bytes(response[:header_end]).split(b"\r\n")
    status_line = response_headers[0] if response_headers else b""
    if status_line.startswith(b"HTTP/"):
        status_parts = status_line.split()
        if len(status_parts) < 2 or status_parts[1] != b"200":
            raise AdapterError("rTorrent SCGI returned a non-success HTTP status")
    payload = bytes(response[header_end + 4:])
    if body_length is not None and len(payload) != body_length:
        raise AdapterError("rTorrent SCGI response body is incomplete")
    values, _ = xmlrpc.client.loads(payload)
    return values[0] if values else None


def _is_task_local_ipv4(value: Any) -> bool:
    if not isinstance(value, str):
        return False
    try:
        address = ipaddress.ip_address(value)
    except ValueError:
        return False
    if not isinstance(address, ipaddress.IPv4Address):
        return False
    return (
        address.is_loopback
        or address.is_link_local
        or address in ipaddress.ip_network("10.0.0.0/8")
        or address in ipaddress.ip_network("172.16.0.0/12")
        or address in ipaddress.ip_network("192.168.0.0/16")
        or address in ipaddress.ip_network("198.18.0.0/15")
    )


def _is_task_local_ipv4_peer(value: Any) -> bool:
    if not isinstance(value, str) or ":" not in value:
        return False
    address, _, raw_port = value.rpartition(":")
    if (
        not raw_port
        or len(raw_port) > 5
        or any(character not in "0123456789" for character in raw_port)
    ):
        return False
    port = int(raw_port)
    return 1 <= port <= 65535 and _is_task_local_ipv4(address)


def _is_task_local_ipv4_tracker_url(value: Any, peer_value: Any) -> bool:
    if not isinstance(value, str) or not isinstance(peer_value, str):
        return False
    try:
        parsed = urllib.parse.urlsplit(value)
    except ValueError:
        return False
    if (
        parsed.scheme != "http"
        or parsed.path != "/announce"
        or parsed.query
        or parsed.fragment
        or parsed.username is not None
        or parsed.password is not None
        or not parsed.hostname
    ):
        return False
    try:
        tracker_ip = ipaddress.ip_address(parsed.hostname)
        tracker_port = parsed.port
        peer_ip = ipaddress.ip_address(peer_value.rpartition(":")[0])
        peer_port = int(peer_value.rpartition(":")[2])
    except (ValueError, TypeError):
        return False
    return (
        isinstance(tracker_ip, ipaddress.IPv4Address)
        and isinstance(peer_ip, ipaddress.IPv4Address)
        and _is_task_local_ipv4(str(tracker_ip))
        and tracker_ip == peer_ip
        and isinstance(tracker_port, int)
        and 1 <= tracker_port <= 65535
        and 1 <= peer_port <= 65535
        and parsed.netloc == f"{tracker_ip}:{tracker_port}"
    )


def _urtorrent_settings_file(
    profile: dict[str, Any], runtime_identity: dict[str, Any]
) -> tuple[str, dict[str, Any]]:
    raw_path = profile.get("settings_file")
    if not isinstance(raw_path, str) or not raw_path:
        raise AdapterError("urtorrent profile must declare its live runner settings_file")
    path = Path(raw_path).expanduser()
    if not path.is_absolute():
        raise AdapterError("urtorrent settings_file must be an absolute path")
    pid = runtime_identity.get("process_id")
    start_ticks = runtime_identity.get("process_start_ticks")
    if (
        isinstance(pid, bool)
        or not isinstance(pid, int)
        or isinstance(start_ticks, bool)
        or not isinstance(start_ticks, int)
    ):
        raise AdapterError("urtorrent settings readback has no pinned process identity")
    _assert_native_process_identity(pid, runtime_identity)
    try:
        raw = _read_regular_file_nofollow(path, "urtorrent settings readback", 64 * 1024)
        snapshot = strict_json_loads(raw, label="urtorrent settings snapshot")
    except (AdapterError, json.JSONDecodeError, StrictJsonError) as error:
        raise AdapterError("cannot read the bounded urtorrent settings snapshot") from error
    _assert_native_process_identity(pid, runtime_identity)
    if not isinstance(snapshot, dict) or snapshot.get("format") != "transfer-urtorrent-settings-v1":
        raise AdapterError("urtorrent runner settings snapshot has an unknown format")
    snapshot_pid = snapshot.get("process_id")
    snapshot_start_ticks = snapshot.get("process_start_ticks")
    if (
        isinstance(snapshot_pid, bool)
        or not isinstance(snapshot_pid, int)
        or isinstance(snapshot_start_ticks, bool)
        or not isinstance(snapshot_start_ticks, int)
        or snapshot_pid != pid
        or snapshot_start_ticks != start_ticks
    ):
        raise AdapterError("urtorrent settings snapshot belongs to a different process instance")
    version = snapshot.get("version")
    settings = snapshot.get("settings")
    if not isinstance(version, str) or not version or not isinstance(settings, dict):
        raise AdapterError("urtorrent settings snapshot is missing its version or settings object")
    session = settings.get("session")
    torrent = settings.get("torrent")
    control = settings.get("control")
    runner_inputs = settings.get("runner_inputs")
    if not all(isinstance(section, dict) for section in (session, torrent, control, runner_inputs)):
        raise AdapterError("urtorrent settings snapshot is missing a required settings section")
    if any(session.get(key) is not False for key in ("dht", "pex", "lsd")) or session.get("listen_v6") is not None:
        raise AdapterError("urtorrent session discovery or IPv6 settings are not isolated")
    listen_port = session.get("listen_port")
    if (
        not _is_task_local_ipv4(session.get("listen_v4"))
        or isinstance(listen_port, bool)
        or not isinstance(listen_port, int)
        or not 1 <= listen_port <= 65535
    ):
        raise AdapterError("urtorrent listener is not bound to its task-local IPv4 interface")
    if torrent.get("private") is not True:
        raise AdapterError("urtorrent benchmark metainfo is not private")
    tracker_url = control.get("tracker_url")
    expected_tracker_count = 1 if tracker_url is not None else 0
    endpoint_counts = (
        (control.get("trackers_after_isolation"), expected_tracker_count),
        (control.get("web_seeds_after_isolation"), 0),
    )
    if (
        control.get("dht_bootstrap_nodes") != []
        or control.get("peer_exchange") is not False
        or control.get("local_service_discovery") is not False
        or any(
            isinstance(count, bool) or not isinstance(count, int) or count != expected
            for count, expected in endpoint_counts
        )
        or control.get("start_barrier") is not True
        or not _is_task_local_ipv4_peer(control.get("pinned_peer"))
        or (
            tracker_url is not None
            and not _is_task_local_ipv4_tracker_url(tracker_url, control.get("pinned_peer"))
        )
    ):
        raise AdapterError("urtorrent fixture endpoints or start barrier are not isolated")
    if (
        runner_inputs.get("mode") not in ("download", "seed")
        or runner_inputs.get("hash_threads_source") != "command_line"
        or any(
            isinstance(runner_inputs.get(key), bool)
            or not isinstance(runner_inputs.get(key), int)
            or runner_inputs[key] <= 0
            for key in ("hash_threads", "timeout_seconds", "seed_seconds")
        )
    ):
        raise AdapterError("urtorrent runner inputs are incomplete or malformed")
    return version, settings


def _probe_api(client_id: str, profile: dict[str, Any], runtime: str,
               runtime_identity: dict[str, Any]) -> tuple[str, dict[str, Any]]:
    if client_id == "torrentngd":
        client = _profile_http(profile, runtime_identity=runtime_identity)
        token = _env_value(profile, "token_env")
        if token:
            client.headers["Authorization"] = f"Bearer {token}"
        health = client.json("/health")
        engine = client.json("/api/v1/engine")
        if not isinstance(health, dict) or not isinstance(engine, dict):
            raise AdapterError("TorrentNG identity endpoints returned a non-object response")
        settings = {
            "session": client.json("/api/v1/session/settings"),
            "features": client.json("/api/v1/session/features"),
        }
        backend = engine.get("backend") if isinstance(engine, dict) else None
        provenance = engine.get("provenance") if isinstance(engine, dict) else None
        version = str(
            health.get("version")
            or (backend.get("version") if isinstance(backend, dict) else None)
            or (provenance.get("daemon_version") if isinstance(provenance, dict) else None)
            or ""
        )
        if not version:
            raise AdapterError("TorrentNG health/engine endpoints did not report a version")
        return version, settings

    if client_id == "qbittorrent":
        client = _profile_http(profile, runtime_identity=runtime_identity)
        _login_qbittorrent(client, profile)
        version = client.text("/api/v2/app/version").strip()
        build = client.json("/api/v2/app/buildInfo")
        settings = client.json("/api/v2/app/preferences")
        return version, {"build": build, "preferences": settings}

    if client_id == "transmission":
        client = _profile_http(profile, allow_basic_auth=True, runtime_identity=runtime_identity)
        session = _transmission_call(client, "session-get")
        version = str(session.get("version") or session.get("rpc-version-semver") or session.get("rpc_version_semver") or "")
        if not version:
            raise AdapterError("Transmission session-get did not report a version")
        return version, {"session": session}

    if client_id == "deluge":
        client = _profile_http(profile, runtime_identity=runtime_identity)
        password = _env_value(profile, "password_env", required=True)
        if _deluge_call(client, "auth.login", [password], 1) is not True:
            raise AdapterError("Deluge Web API login failed")
        version = str(_deluge_call(client, "web.get_version", [], 2) or "")
        if not version:
            raise AdapterError("Deluge did not report a version")
        connected = _deluge_call(client, "web.connected", [], 3)
        if not connected:
            hosts = _deluge_call(client, "web.get_hosts", [], 4) or []
            host_id = hosts[0][0] if hosts else None
            if not host_id:
                raise AdapterError("Deluge WebUI has no configured daemon host")
            _deluge_call(client, "web.connect", [host_id], 5)
            if not _deluge_call(client, "web.connected", [], 6):
                raise AdapterError("Deluge WebUI could not connect to its daemon")
        keys = profile.get("settings_keys")
        args = [keys] if isinstance(keys, list) and keys else []
        settings = _deluge_call(client, "core.get_config", args, 7)
        if not isinstance(settings, dict):
            raise AdapterError("Deluge core.get_config did not return a settings object")
        return version, {"core": settings}

    if client_id == "rtorrent":
        socket_path = Path(str(profile.get("socket_path", ""))).expanduser()
        if not socket_path.is_absolute() or socket_path.is_symlink() or not socket_path.exists():
            raise AdapterError("rTorrent profile requires an existing absolute SCGI socket_path")
        if runtime == "native_binary":
            _verify_native_unix_listener(socket_path, runtime_identity)
        else:
            _verify_container_unix_listener(profile, runtime_identity, socket_path)
        version = str(_scgi_call(socket_path, "system.client_version", []) or "")
        if not version:
            raise AdapterError("rTorrent did not report a client version")
        methods = profile.get("settings_methods")
        if not isinstance(methods, dict) or not methods:
            raise AdapterError("rTorrent profile must declare settings_methods")
        settings: dict[str, Any] = {}
        for name, descriptor in methods.items():
            if isinstance(descriptor, str):
                method, arguments = descriptor, []
            elif isinstance(descriptor, dict):
                method = descriptor.get("method")
                arguments = descriptor.get("arguments", [])
            else:
                raise AdapterError(f"rTorrent settings method {name!r} must be a method or descriptor")
            if not isinstance(method, str) or not method:
                raise AdapterError(f"rTorrent settings method {name!r} has no method name")
            if not isinstance(arguments, list):
                raise AdapterError(f"rTorrent settings method {name!r} arguments must be a list")
            settings[name] = _scgi_call(socket_path, method, arguments)
        return version, {"rpc_settings": settings}

    if client_id == "rqbit":
        client = _profile_http(profile, allow_basic_auth=True, runtime_identity=runtime_identity)
        root = client.json("/")
        if not isinstance(root, dict):
            raise AdapterError("rqbit root endpoint did not return an object")
        version = str(root.get("version") or "")
        if not version:
            raise AdapterError("rqbit root endpoint did not report a version")
        if root.get("server") != "rqbit":
            raise AdapterError("rqbit root endpoint returned an unexpected server identity")
        return version, {"launch": _rqbit_launch_settings(profile, runtime, runtime_identity)}

    if client_id == "urtorrent":
        if runtime != "native_binary":
            raise AdapterError("urtorrent comparator requires the pinned native library runner")
        return _urtorrent_settings_file(profile, runtime_identity)

    raise AdapterError(f"unsupported client id {client_id!r}")


def probe_client(client_id: str, profile: dict[str, Any]) -> dict[str, Any]:
    if client_id not in CLIENT_IDS:
        raise AdapterError(f"unsupported client id {client_id!r}")
    runtime = profile.get("runtime")
    if runtime == "container":
        runtime_identity = _docker_identity(profile, capture_command=True)
    elif runtime == "native_binary":
        runtime_identity = _binary_identity(profile)
    else:
        raise AdapterError("client runtime must be container or native_binary")

    version, raw_settings = _probe_api(client_id, profile, runtime, runtime_identity)
    _check_version(profile, version)
    safe_settings = _redact(raw_settings)
    expected_settings = profile.get("expected_settings")
    if not isinstance(expected_settings, dict):
        raise AdapterError("each client profile must declare expected_settings, including an explicit empty object")
    if client_id == "rqbit":
        if safe_settings != expected_settings:
            raise AdapterError("rqbit observed launch settings must exactly match the declared pin")
        differences: list[str] = []
    else:
        if client_id == "urtorrent":
            expected_settings = expected_settings.get("settings")
            if not isinstance(expected_settings, dict):
                raise AdapterError("urtorrent expected_settings must contain a settings object")
        differences = _settings_subset(safe_settings, expected_settings)
    if differences:
        raise AdapterError("effective settings differ from the trial pin: " + ", ".join(differences))

    if not version:
        raise AdapterError(f"{client_id} did not report a client version")
    return {
        "schema_version": SCHEMA_VERSION,
        "client_id": client_id,
        "observed_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
        "version": version,
        "runtime_identity": runtime_identity,
        "settings_readback": {
            "captured": True,
            "values": safe_settings,
            "sha256": _sha256_bytes(_canonical_json(safe_settings)),
        },
    }


def _payload_copy_counters(metrics: str) -> dict[str, int]:
    metric_names = {metric: name for name, metric in PAYLOAD_COPY_METRICS.items()}
    values: dict[str, int] = {}
    for line in metrics.splitlines():
        if not line or line.startswith("#"):
            continue
        fields = line.split()
        if not fields or fields[0] not in metric_names:
            continue
        name = metric_names[fields[0]]
        if len(fields) != 2 or not fields[1].isdecimal():
            raise AdapterError(f"native payload-copy metric {fields[0]} is malformed")
        if name in values:
            raise AdapterError(f"native payload-copy metric {fields[0]} is duplicated")
        values[name] = int(fields[1])
    missing = sorted(set(PAYLOAD_COPY_METRICS) - set(values))
    if missing:
        raise AdapterError("native client is missing payload-copy metrics: " + ", ".join(missing))
    return values


def capture_payload_copy_observation(profile: dict[str, Any]) -> dict[str, Any]:
    """Capture TorrentNG's instrumented-site counters for a trial boundary."""
    runtime = profile.get("runtime")
    if runtime == "container":
        identity = _docker_identity(profile, capture_command=True)
    elif runtime == "native_binary":
        identity = _binary_identity(profile)
    else:
        raise AdapterError("TorrentNG payload-copy capture requires a pinned client runtime")
    client = _profile_http(profile, runtime_identity=identity)
    token = _env_value(profile, "token_env")
    if token:
        client.headers["Authorization"] = f"Bearer {token}"
    metrics = client.text("/metrics")
    if runtime == "native_binary":
        _assert_native_process_identity(identity["process_id"], identity)
    else:
        current = _docker_identity(profile, capture_command=True)
        if (
            current.get("process_id"),
            current.get("process_start_ticks"),
            current.get("image_digest"),
        ) != (
            identity.get("process_id"),
            identity.get("process_start_ticks"),
            identity.get("image_digest"),
        ):
            raise AdapterError("TorrentNG container restarted while payload-copy counters were captured")
    return {
        "schema_version": PAYLOAD_COPY_SCHEMA_VERSION,
        "client_id": "torrentngd",
        "observed_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="microseconds"),
        "process_id": identity["process_id"],
        "process_start_ticks": identity["process_start_ticks"],
        "metrics_sha256": _sha256_bytes(metrics.encode("utf-8")),
        "counters_bytes": _payload_copy_counters(metrics),
    }


def load_pins(path: Path) -> dict[str, dict[str, Any]]:
    if path.is_symlink() or not path.is_file():
        raise AdapterError("pin file must be an existing regular non-symlink file")
    try:
        data = strict_json_loads(path.read_bytes(), label="client pin file")
    except (OSError, json.JSONDecodeError, StrictJsonError) as error:
        raise AdapterError(f"cannot read pin file: {error}") from error
    return load_pins_document(data)


def load_pins_document(data: object) -> dict[str, dict[str, Any]]:
    """Validate pin data from an already-read JSON document."""
    if not isinstance(data, dict) or data.get("schema_version") != PINS_SCHEMA_VERSION:
        raise AdapterError(f"pin file schema_version must be {PINS_SCHEMA_VERSION!r}")
    clients = data.get("clients")
    if not isinstance(clients, dict) or not clients:
        raise AdapterError("pin file clients must be an object")
    for client_id, profile in clients.items():
        if client_id not in CLIENT_IDS:
            raise AdapterError(f"pin file has unsupported client id {client_id!r}")
        if not isinstance(profile, dict):
            raise AdapterError(f"pin profile for {client_id!r} must be an object")
        _validate_profile(client_id, profile)
    return clients


def _validate_profile(client_id: str, profile: dict[str, Any]) -> None:
    common = {
        "runtime", "expected_version", "expected_settings", "base_url", "timeout_seconds",
        "preflight_timeout_seconds",
        "username_env", "password_env", "token_env", "container", "image_ref",
        "binary_path", "expected_binary_sha256", "source_root", "expected_source_commit",
        "expected_dirty_worktree", "expected_lockfile_sha256", "expected_toolchain",
        "pid_file", "socket_path", "container_socket_path",
        "settings_methods", "settings_keys", "settings_env_allowlist", "settings_file_allowlist",
        "settings_file", "preflight_save_path", "paired_launch",
    }
    unknown = sorted(set(profile) - common)
    if unknown:
        raise AdapterError(f"pin profile for {client_id!r} has unsupported fields: {', '.join(unknown)}")
    if profile.get("runtime") not in ("container", "native_binary"):
        raise AdapterError(f"pin profile for {client_id!r} must select a supported runtime")
    if not isinstance(profile.get("expected_version"), str) or not profile["expected_version"]:
        raise AdapterError(f"pin profile for {client_id!r} must pin expected_version")
    if not isinstance(profile.get("expected_settings"), dict):
        raise AdapterError(f"pin profile for {client_id!r} must pin expected_settings as an object")
    timeout = profile.get("timeout_seconds", 10)
    if (
        isinstance(timeout, bool)
        or not isinstance(timeout, (int, float))
        or timeout <= 0
        or timeout > 120
        or not math.isfinite(timeout)
    ):
        raise AdapterError(f"pin profile for {client_id!r} timeout_seconds must be finite and within (0, 120]")
    preflight_timeout = profile.get("preflight_timeout_seconds", 60)
    if (
        isinstance(preflight_timeout, bool)
        or not isinstance(preflight_timeout, (int, float))
        or preflight_timeout <= 0
        or preflight_timeout > 300
        or not math.isfinite(preflight_timeout)
    ):
        raise AdapterError(
            f"pin profile for {client_id!r} preflight_timeout_seconds must be finite and within (0, 300]"
        )
    preflight_save_path = profile.get("preflight_save_path")
    if preflight_save_path is not None:
        preflight_path = Path(preflight_save_path) if isinstance(preflight_save_path, str) else Path()
        if (
            not isinstance(preflight_save_path, str)
            or not preflight_save_path
            or not preflight_path.is_absolute()
            or ".." in preflight_path.parts
            or len(preflight_path.parts) < 2
        ):
            raise AdapterError(
                f"pin profile for {client_id!r} preflight_save_path must be a safe absolute disposable path"
            )

    runtime = profile["runtime"]
    if runtime == "container":
        if not isinstance(profile.get("container"), str) or not profile["container"]:
            raise AdapterError(f"container profile for {client_id!r} must name a container")
        if not isinstance(profile.get("image_ref"), str) or not IMAGE_REF_RE.fullmatch(profile["image_ref"]):
            raise AdapterError(f"container profile for {client_id!r} must pin an image digest")
        native_fields = {
            "binary_path", "expected_binary_sha256", "source_root", "expected_source_commit",
            "expected_dirty_worktree", "expected_lockfile_sha256", "expected_toolchain", "pid_file",
            "settings_file",
        }
        if native_fields.intersection(profile):
            raise AdapterError(f"container profile for {client_id!r} cannot contain native binary pins")
    else:
        required_native = {"binary_path", "expected_binary_sha256", "pid_file"}
        missing = sorted(required_native - set(profile))
        if missing:
            raise AdapterError(
                f"native profile for {client_id!r} is missing: {', '.join(missing)}"
            )
        source_fields = {
            "source_root", "expected_source_commit", "expected_dirty_worktree",
            "expected_lockfile_sha256", "expected_toolchain",
        }
        supplied_source_fields = source_fields.intersection(profile)
        if supplied_source_fields and supplied_source_fields != source_fields:
            raise AdapterError(
                f"native profile for {client_id!r} must provide all source identity pins together"
            )
        if "container" in profile or "image_ref" in profile:
            raise AdapterError(f"native profile for {client_id!r} cannot contain container pins")

    if client_id == "rtorrent":
        if "base_url" in profile:
            raise AdapterError("rTorrent uses SCGI and cannot declare base_url")
        if not isinstance(profile.get("socket_path"), str) or not profile["socket_path"]:
            raise AdapterError("rTorrent profile must declare socket_path")
        if not isinstance(profile.get("settings_methods"), dict) or not profile["settings_methods"]:
            raise AdapterError("rTorrent profile must declare settings_methods")
        if runtime == "container":
            if not isinstance(profile.get("container_socket_path"), str):
                raise AdapterError("rTorrent container profile must declare container_socket_path")
        elif "container_socket_path" in profile:
            raise AdapterError("native rTorrent profile cannot declare container_socket_path")
    elif client_id == "urtorrent":
        if runtime != "native_binary":
            raise AdapterError("urtorrent profile must use the native library runner")
        if "base_url" in profile or "socket_path" in profile:
            raise AdapterError("urtorrent runner has no HTTP or SCGI control endpoint")
        settings_file = profile.get("settings_file")
        if not isinstance(settings_file, str) or not Path(settings_file).expanduser().is_absolute():
            raise AdapterError("urtorrent profile must declare an absolute settings_file")
        expected = profile["expected_settings"].get("settings")
        if not isinstance(expected, dict):
            raise AdapterError("urtorrent expected_settings must pin session and torrent readback")
        for section in ("session", "torrent", "control"):
            if not isinstance(expected.get(section), dict):
                raise AdapterError(f"urtorrent expected_settings must pin the {section} section")
        if not _is_task_local_ipv4(expected["session"].get("listen_v4")):
            raise AdapterError("urtorrent expected_settings must pin one task-local IPv4 listener")
        isolation_pins = {
            "session": {
                "dht": False,
                "pex": False,
                "lsd": False,
                "listen_v4": expected["session"].get("listen_v4"),
                "listen_v6": None,
            },
            "torrent": {"private": True},
            "control": {
                "dht_bootstrap_nodes": [],
                "peer_exchange": False,
                "local_service_discovery": False,
                "trackers_after_isolation": 0,
                "web_seeds_after_isolation": 0,
                "pinned_peer": expected["control"].get("pinned_peer"),
                "start_barrier": True,
            },
        }
        if not _is_task_local_ipv4_peer(expected["control"].get("pinned_peer")):
            raise AdapterError("urtorrent expected_settings must pin one task-local IPv4 peer")
        for section, values in isolation_pins.items():
            if any(expected[section].get(key, object()) != value for key, value in values.items()):
                raise AdapterError(f"urtorrent expected_settings must pin isolated {section} settings")
        if any(key in profile for key in ("username_env", "password_env", "token_env")):
            raise AdapterError("urtorrent runner does not use API credentials")
    else:
        base_url = profile.get("base_url")
        if not isinstance(base_url, str) or not base_url:
            raise AdapterError(f"HTTP profile for {client_id!r} must declare base_url")

    paired_launch = profile.get("paired_launch")
    if paired_launch is not None:
        fields = {
            "args",
            "api_port",
            "peer_port",
            "environment",
            "environment_from",
            "container_workdir",
            "entrypoint",
            "startup_timeout_seconds",
        }
        if not isinstance(paired_launch, dict) or set(paired_launch) != fields:
            raise AdapterError(
                f"paired_launch for {client_id!r} must declare exactly: "
                + ", ".join(sorted(fields))
            )
        args = paired_launch["args"]
        if (
            not isinstance(args, list)
            or len(args) > 256
            or any(not isinstance(value, str) or "\x00" in value for value in args)
        ):
            raise AdapterError(f"paired_launch args for {client_id!r} must be a bounded string array")
        api_port = paired_launch["api_port"]
        if client_id == "rtorrent":
            if api_port is not None:
                raise AdapterError("rTorrent paired_launch api_port must be null")
        elif (
            isinstance(api_port, bool)
            or not isinstance(api_port, int)
            or not 1 <= api_port <= 65535
        ):
            raise AdapterError(f"paired_launch api_port for {client_id!r} must be within 1..65535")
        peer_port = paired_launch["peer_port"]
        if (
            isinstance(peer_port, bool)
            or not isinstance(peer_port, int)
            or not 1 <= peer_port <= 65535
        ):
            raise AdapterError(f"paired_launch peer_port for {client_id!r} must be within 1..65535")
        environment = paired_launch["environment"]
        environment_from = paired_launch["environment_from"]
        if not isinstance(environment, dict) or not isinstance(environment_from, dict):
            raise AdapterError("paired_launch environment declarations must be objects")
        env_name_re = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
        for name, value in environment.items():
            if (
                not isinstance(name, str)
                or env_name_re.fullmatch(name) is None
                or not isinstance(value, str)
                or "\x00" in value
            ):
                raise AdapterError("paired_launch environment contains an invalid entry")
        for name, source in environment_from.items():
            if (
                not isinstance(name, str)
                or env_name_re.fullmatch(name) is None
                or not isinstance(source, str)
                or env_name_re.fullmatch(source) is None
            ):
                raise AdapterError("paired_launch environment_from contains an invalid entry")
        container_workdir = paired_launch["container_workdir"]
        if runtime == "container":
            workdir = Path(container_workdir) if isinstance(container_workdir, str) else Path()
            if (
                not isinstance(container_workdir, str)
                or not workdir.is_absolute()
                or ".." in workdir.parts
                or len(workdir.parts) < 2
            ):
                raise AdapterError(
                    "container paired_launch must pin an absolute container_workdir"
                )
        elif container_workdir is not None:
            raise AdapterError("native paired_launch container_workdir must be null")
        entrypoint = paired_launch["entrypoint"]
        if entrypoint is not None and (
            not isinstance(entrypoint, str) or not entrypoint or "\x00" in entrypoint
        ):
            raise AdapterError("paired_launch entrypoint must be a nonempty string or null")
        startup_timeout = paired_launch["startup_timeout_seconds"]
        if (
            isinstance(startup_timeout, bool)
            or not isinstance(startup_timeout, (int, float))
            or not math.isfinite(startup_timeout)
            or not 0 < startup_timeout <= 30
        ):
            raise AdapterError(
                "paired_launch startup_timeout_seconds must be finite and within (0, 30]"
            )
        if client_id == "urtorrent":
            raise AdapterError("urtorrent uses its dedicated direct runner, not paired_launch")
    if client_id == "deluge" and not isinstance(profile.get("password_env"), str):
        raise AdapterError("Deluge profile must name password_env")
    if client_id == "rqbit":
        if not isinstance(profile.get("settings_env_allowlist"), list):
            raise AdapterError("rqbit profile must declare settings_env_allowlist")
        if not isinstance(profile.get("settings_file_allowlist"), list):
            raise AdapterError("rqbit profile must declare settings_file_allowlist")
        if "launch" not in profile["expected_settings"]:
            raise AdapterError("rqbit expected_settings must pin the complete launch readback")
        if "token_env" in profile:
            raise AdapterError("rqbit uses Basic auth credentials, not token_env")
    if client_id == "transmission" and ("username_env" in profile) != ("password_env" in profile):
        raise AdapterError("Transmission profile must provide both username_env and password_env")
    if client_id == "qbittorrent" and ("username_env" in profile) != ("password_env" in profile):
        raise AdapterError("qBittorrent profile must provide both username_env and password_env")
    if client_id == "rqbit" and ("username_env" in profile) != ("password_env" in profile):
        raise AdapterError("rqbit profile must provide both username_env and password_env")


def write_new_json(path: Path, value: dict[str, Any]) -> str:
    if path.is_symlink() or path.exists():
        raise AdapterError(f"refusing to overwrite adapter observation: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    data = (json.dumps(value, sort_keys=True, indent=2, ensure_ascii=False) + "\n").encode("utf-8")
    try:
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except OSError as error:
        raise AdapterError(f"cannot create adapter observation {path}: {error}") from error
    try:
        with os.fdopen(descriptor, "wb") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
    except OSError as error:
        try:
            path.unlink()
        except OSError:
            pass
        raise AdapterError(f"cannot finish adapter observation {path}: {error}") from error
    return _sha256_bytes(data)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("client", choices=sorted(CLIENT_IDS))
    parser.add_argument("--pins", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument(
        "--payload-copy-snapshot",
        action="store_true",
        help="capture TorrentNG's instrumented payload-copy counters at a trial boundary",
    )
    args = parser.parse_args(argv)
    try:
        profiles = load_pins(args.pins)
        profile = profiles.get(args.client)
        if not isinstance(profile, dict):
            raise AdapterError(f"pin file has no profile for {args.client!r}")
        if args.payload_copy_snapshot:
            if args.client != "torrentngd":
                raise AdapterError("payload-copy snapshots are currently supported only for TorrentNG")
            observation = capture_payload_copy_observation(profile)
        else:
            observation = probe_client(args.client, profile)
        write_new_json(args.output, observation)
    except (AdapterError, OSError, ValueError) as error:
        print(f"adapter {args.client}: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
