#!/usr/bin/env python3
"""Launch and retire one pinned client inside a task-owned network namespace.

Native binaries are executed through the descriptor-pinned supervisor. Docker
clients start with ``--network none`` and receive only task-owned data/state
bind mounts; the task topology attaches their namespace before a trial starts.
"""

from __future__ import annotations

import json
import os
import re
import secrets
import stat
import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping

from adapters import AdapterError, probe_client
from process_supervisor import ManagedProcess, ProcessSupervisorError
from task_network import client_api_context
from strict_json import StrictJsonError, loads as strict_json_loads


SAFE_NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,62}$")
PLACEHOLDER_RE = re.compile(r"\{([a-z_]+)\}")
MAX_ENV_FILE_BYTES = 1024 * 1024
MAX_TORRENTNG_CONFIG_BYTES = 64 * 1024


class ManagedClientError(RuntimeError):
    """A pinned native or container client could not be managed safely."""


def _toml_literal(value: Any) -> str:
    if type(value) is bool:
        return "true" if value else "false"
    if type(value) is int:
        return str(value)
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, list):
        return "[" + ", ".join(_toml_literal(item) for item in value) + "]"
    raise ManagedClientError("pinned TorrentNG config has an unsupported TOML value")


def _serialize_torrentng_config(config: dict[str, Any]) -> bytes:
    lines: list[str] = []
    for section, values in config.items():
        if not isinstance(section, str) or re.fullmatch(r"[A-Za-z0-9_-]+", section) is None:
            raise ManagedClientError("pinned TorrentNG config has an unsupported table name")
        if not isinstance(values, dict):
            raise ManagedClientError("pinned TorrentNG config must contain flat tables")
        lines.append(f"[{section}]")
        for key, value in values.items():
            if not isinstance(key, str) or re.fullmatch(r"[A-Za-z0-9_-]+", key) is None:
                raise ManagedClientError("pinned TorrentNG config has an unsupported key")
            lines.append(f"{key} = {_toml_literal(value)}")
        lines.append("")
    return ("\n".join(lines) + "\n").encode("utf-8")


def _read_torrentng_config(path: Path) -> dict[str, Any]:
    if not path.is_absolute() or path.is_symlink():
        raise ManagedClientError("pinned TorrentNG config must be an absolute regular file")
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise ManagedClientError("cannot open pinned TorrentNG config without following links") from error
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_TORRENTNG_CONFIG_BYTES:
            raise ManagedClientError("pinned TorrentNG config must be a bounded regular file")
        raw = bytearray()
        while len(raw) <= MAX_TORRENTNG_CONFIG_BYTES:
            chunk = os.read(descriptor, min(16 * 1024, MAX_TORRENTNG_CONFIG_BYTES + 1 - len(raw)))
            if not chunk:
                break
            raw.extend(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    if len(raw) != before.st_size or len(raw) > MAX_TORRENTNG_CONFIG_BYTES:
        raise ManagedClientError("pinned TorrentNG config changed while it was read")
    if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns) != (
        after.st_dev,
        after.st_ino,
        after.st_size,
        after.st_mtime_ns,
    ):
        raise ManagedClientError("pinned TorrentNG config changed while it was read")
    try:
        value = tomllib.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise ManagedClientError("pinned TorrentNG config is not valid UTF-8 TOML") from error
    if not isinstance(value, dict):
        raise ManagedClientError("pinned TorrentNG config root must be a table")
    return value


def _write_torrentng_config(path: Path, config: dict[str, Any]) -> None:
    if path.exists() or path.is_symlink():
        raise ManagedClientError("trial TorrentNG config already exists")
    encoded = _serialize_torrentng_config(config)
    try:
        descriptor = os.open(
            path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
            0o600,
        )
    except OSError as error:
        raise ManagedClientError("cannot create private trial TorrentNG config") from error
    try:
        os.fchmod(descriptor, 0o600)
        view = memoryview(encoded)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise ManagedClientError("short write while creating trial TorrentNG config")
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


def _prepare_torrentng_config(
    profile: dict[str, Any], runtime_paths: dict[str, Any]
) -> None:
    launch = profile.get("paired_launch")
    if not isinstance(launch, dict) or not isinstance(launch.get("environment"), dict):
        raise ManagedClientError("TorrentNG paired launch has no pinned config environment")
    config_path = launch["environment"].get("TORRENTNGD_CONFIG")
    if not isinstance(config_path, str):
        raise ManagedClientError("TorrentNG paired launch must pin TORRENTNGD_CONFIG")
    config = _read_torrentng_config(Path(config_path))
    daemon = config.get("daemon")
    network = config.get("network")
    storage = config.get("storage")
    api_port = launch.get("api_port")
    peer_port = launch.get("peer_port")
    state_path = runtime_paths.get("state")
    data_path = runtime_paths.get("data")
    if (
        not isinstance(daemon, dict)
        or not isinstance(network, dict)
        or not isinstance(storage, dict)
        or isinstance(api_port, bool)
        or not isinstance(api_port, int)
        or not 1 <= api_port <= 65535
        or isinstance(peer_port, bool)
        or not isinstance(peer_port, int)
        or not 1 <= peer_port <= 65535
        or not isinstance(state_path, Path)
        or not state_path.is_absolute()
        or not isinstance(data_path, Path)
        or not data_path.is_absolute()
    ):
        raise ManagedClientError("TorrentNG paired launch config is missing pinned task paths or ports")
    daemon["api_bind"] = f"127.0.0.1:{api_port}"
    daemon["session_dir"] = str(state_path / "session")
    network["listen_port"] = peer_port
    storage["download_dir"] = str(data_path)
    destination = state_path / "torrentngd.toml"
    _write_torrentng_config(destination, config)
    launch["environment"]["TORRENTNGD_CONFIG"] = str(destination)


def _render(value: str, values: Mapping[str, str]) -> str:
    def replace(match: re.Match[str]) -> str:
        key = match.group(1)
        if key not in values:
            raise ManagedClientError(f"paired launch uses unsupported placeholder {{{key}}}")
        return values[key]

    rendered = PLACEHOLDER_RE.sub(replace, value)
    if "{" in rendered or "}" in rendered or "\x00" in rendered:
        raise ManagedClientError("paired launch value contains an unresolved placeholder or NUL")
    return rendered


def _launch_environment(profile: dict[str, Any], values: Mapping[str, str]) -> dict[str, str]:
    launch = profile.get("paired_launch")
    if not isinstance(launch, dict):
        raise ManagedClientError("pinned client profile has no paired_launch contract")
    environment: dict[str, str] = {}
    for name, value in launch["environment"].items():
        environment[name] = _render(value, values)
    for name, source in launch["environment_from"].items():
        if source not in os.environ:
            raise ManagedClientError(f"paired launch requires environment variable {source!r}")
        environment[name] = os.environ[source]
    for key in ("username_env", "password_env", "token_env"):
        source = profile.get(key)
        if source is None:
            continue
        if not isinstance(source, str) or not source or source not in os.environ:
            raise ManagedClientError(f"pinned client credential environment variable {source!r} is unavailable")
        environment[source] = os.environ[source]
    return environment


def _write_env_file(path: Path, environment: Mapping[str, str]) -> None:
    if path.exists() or path.is_symlink():
        raise ManagedClientError("container environment file already exists")
    rows = []
    for name, value in sorted(environment.items()):
        if "\n" in value or "\r" in value:
            raise ManagedClientError("container environment values cannot contain newlines")
        rows.append(f"{name}={value}\n")
    encoded = "".join(rows).encode("utf-8")
    if len(encoded) > MAX_ENV_FILE_BYTES:
        raise ManagedClientError("container environment exceeds its bounded file size")
    descriptor = os.open(
        path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
        0o600,
    )
    try:
        view = memoryview(encoded)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise ManagedClientError("short write while creating private container environment")
            view = view[written:]
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def _docker(argv: list[str], *, timeout: float = 15.0) -> str:
    try:
        completed = subprocess.run(
            argv,
            check=True,
            capture_output=True,
            text=True,
            timeout=timeout,
            env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C"},
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ManagedClientError(f"container runtime command failed: {argv[1]}: {error}") from error
    return completed.stdout.strip()


def _verify_pinned_image(image_ref: str) -> None:
    try:
        raw = _docker(["docker", "image", "inspect", "--format", "{{json .RepoDigests}}", image_ref])
        repo_digests = strict_json_loads(raw, label="container image inspect")
    except (ManagedClientError, json.JSONDecodeError, StrictJsonError) as error:
        raise ManagedClientError("cannot inspect the locally available pinned client image") from error
    if not isinstance(repo_digests, list) or image_ref not in repo_digests:
        raise ManagedClientError("local container image does not carry the exact pinned repository digest")


def _inspect_container(name: str) -> tuple[str, int, int, str, str]:
    try:
        raw = _docker(["docker", "inspect", "--format", "{{json .}}", name])
        value = strict_json_loads(raw, label="container inspect")
    except (ManagedClientError, json.JSONDecodeError, StrictJsonError) as error:
        raise ManagedClientError("cannot read the trial container identity") from error
    if not isinstance(value, dict):
        raise ManagedClientError("container inspect returned a non-object")
    state = value.get("State")
    if not isinstance(state, dict) or state.get("Running") is not True:
        raise ManagedClientError("trial container is not running")
    pid = state.get("Pid")
    image_id = str(value.get("Image", "")).removeprefix("sha256:")
    container_id = str(value.get("Id", "")).removeprefix("sha256:")
    if isinstance(pid, bool) or not isinstance(pid, int) or pid <= 0:
        raise ManagedClientError("container has no valid host process ID")
    try:
        stat_line = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
        fields = stat_line[stat_line.rfind(")") + 2 :].split()
        start_ticks = int(fields[19])
    except (OSError, UnicodeDecodeError, ValueError, IndexError) as error:
        raise ManagedClientError("cannot pin the container process start time") from error
    if (
        re.fullmatch(r"[a-f0-9]{64}", container_id) is None
        or re.fullmatch(r"[a-f0-9]{64}", image_id) is None
        or start_ticks < 0
    ):
        raise ManagedClientError("container inspect returned malformed process or image identity")
    network = value.get("HostConfig", {}).get("NetworkMode") if isinstance(value.get("HostConfig"), dict) else None
    return container_id, pid, start_ticks, image_id, str(network)


def _container_environment_args(environment: Mapping[str, str], env_file: Path) -> list[str]:
    _write_env_file(env_file, environment)
    return ["--env-file", str(env_file)]


@dataclass
class ManagedClient:
    client_id: str
    role: str
    profile: dict[str, Any]
    namespace: str
    namespace_identity: dict[str, int]
    process: ManagedProcess | None
    container_name: str | None
    container_id: str | None
    host_pid: int
    process_start_ticks: int
    image_id: str | None
    log_path: Path
    api_profile: dict[str, Any]
    _log_stream: Any = None
    _stopped: bool = False

    @property
    def pid(self) -> int:
        return self.host_pid

    def still_matches_pin(self) -> bool:
        if self.process is not None:
            return self.process.still_matches_pin()
        if self.container_name is None or self.container_id is None:
            return False
        try:
            container_id, pid, start_ticks, image_id, network = _inspect_container(self.container_name)
        except ManagedClientError:
            return False
        return (
            container_id == self.container_id
            and pid == self.host_pid
            and start_ticks == self.process_start_ticks
            and image_id == self.image_id
            and network == "none"
        )

    def probe(self) -> dict[str, Any]:
        if not self.still_matches_pin():
            raise ManagedClientError("client runtime identity changed before settings readback")
        try:
            observation = probe_client(self.client_id, self.api_profile)
        except (AdapterError, OSError, ValueError) as error:
            raise ManagedClientError(f"client identity/settings readback failed: {error}") from error
        if observation.get("runtime_identity", {}).get("process_id") != self.host_pid:
            raise ManagedClientError("client adapter observed a different process than the launcher")
        if observation.get("runtime_identity", {}).get("process_start_ticks") != self.process_start_ticks:
            raise ManagedClientError("client adapter observed a different process incarnation")
        return observation

    def stop(self, *, timeout_seconds: float = 10.0) -> None:
        if self._stopped:
            return
        if self.process is not None:
            try:
                self.process.stop(grace_seconds=timeout_seconds)
            except ProcessSupervisorError as error:
                raise ManagedClientError(str(error)) from error
        else:
            if self.container_name is None or self.container_id is None or not self.still_matches_pin():
                raise ManagedClientError("refusing to stop a container whose identity changed")
            _docker(["docker", "stop", "--time", "5", self.container_id], timeout=timeout_seconds + 5)
            try:
                logs = subprocess.Popen(
                    ["docker", "logs", self.container_id],
                    stdin=subprocess.DEVNULL,
                    stdout=self._log_stream,
                    stderr=subprocess.STDOUT,
                    close_fds=True,
                    start_new_session=True,
                    env={"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C"},
                )
                try:
                    logs.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    logs.kill()
                    logs.wait()
                if self._log_stream is not None:
                    self._log_stream.flush()
                    os.fsync(self._log_stream.fileno())
            except (OSError, subprocess.SubprocessError):
                pass
            finally:
                if self._log_stream is not None:
                    self._log_stream.close()
                    self._log_stream = None
                _docker(["docker", "rm", "--force", self.container_id], timeout=timeout_seconds + 5)
        self._stopped = True


def _profile_for_role(
    client_id: str,
    profile: dict[str, Any],
    role: str,
    topology: dict[str, Any],
    runtime_paths: dict[str, Any],
) -> tuple[dict[str, Any], dict[str, str]]:
    endpoint = topology.get(role)
    namespace = topology.get(f"{role}_namespace")
    if not isinstance(endpoint, dict) or not isinstance(namespace, str):
        raise ManagedClientError(f"task network has no {role} endpoint")
    address = endpoint.get("address")
    if not isinstance(address, str):
        raise ManagedClientError(f"task network has no {role} address")
    launch = profile.get("paired_launch")
    if not isinstance(launch, dict):
        raise ManagedClientError(f"client {client_id} has no paired_launch profile")
    is_container = profile.get("runtime") == "container"
    values = {
        "api_bind": "0.0.0.0",
        "api_port": str(launch["api_port"] or ""),
        "peer_port": str(launch["peer_port"]),
        "data_path": "/data" if is_container else str(runtime_paths["data"]),
        "state_path": "/state" if is_container else str(runtime_paths["state"]),
        "tracker_url": str(runtime_paths["tracker_url"]),
        "peer_address": str(runtime_paths["peer_address"]),
    }
    result = dict(profile)
    result["paired_launch"] = dict(launch)
    result["paired_launch"]["args"] = [_render(value, values) for value in launch["args"]]
    result["paired_launch"]["environment"] = {
        name: _render(value, values) for name, value in launch["environment"].items()
    }
    result["base_url"] = (
        f"http://{address}:{launch['api_port']}" if launch["api_port"] is not None else None
    )
    result["task_api_context"] = client_api_context(topology, role)
    result["expected_settings"] = _render_tree(profile["expected_settings"], values)
    if client_id == "rtorrent":
        socket_name = "rtorrent.sock"
        if is_container:
            result["container_socket_path"] = f"/state/{socket_name}"
            result["socket_path"] = str(runtime_paths["state"] / socket_name)
        else:
            result["socket_path"] = str(runtime_paths["state"] / socket_name)
    return result, values


def _render_tree(value: Any, values: Mapping[str, str]) -> Any:
    if isinstance(value, dict):
        return {key: _render_tree(child, values) for key, child in value.items()}
    if isinstance(value, list):
        return [_render_tree(child, values) for child in value]
    if isinstance(value, str):
        return _render(value, values)
    return value


def _start_container_client(
    client_id: str,
    profile: dict[str, Any],
    role: str,
    namespace: str,
    namespace_identity: dict[str, int],
    api_profile: dict[str, Any],
    launch: dict[str, Any],
    environment: dict[str, str],
    runtime_paths: dict[str, Any],
) -> ManagedClient:
    image_ref = profile.get("image_ref")
    if not isinstance(image_ref, str) or "@sha256:" not in image_ref:
        raise ManagedClientError("container profile has no immutable image digest")
    _verify_pinned_image(image_ref)
    name = f"tngbench-{secrets.token_hex(6)}"
    if SAFE_NAME_RE.fullmatch(name) is None:
        raise ManagedClientError("generated container name is invalid")
    env_file = runtime_paths["env"]
    log_fd: int | None = None
    log_stream: Any = None
    created = False
    try:
        argv = [
            "docker", "run", "--detach", "--pull=never", "--network", "none",
            "--name", name,
            "--label", "org.torrentng.transfer-rebuild=paired-v1",
            "--cap-drop=ALL", "--security-opt=no-new-privileges", "--pids-limit=512",
            "--mount", f"type=bind,source={runtime_paths['data']},target=/data",
            "--mount", f"type=bind,source={runtime_paths['state']},target=/state",
            "--workdir", launch["container_workdir"],
            *_container_environment_args(environment, env_file),
        ]
        if launch["entrypoint"] is not None:
            argv.extend(["--entrypoint", launch["entrypoint"]])
        argv.extend([image_ref, *launch["args"]])
        _docker(argv, timeout=launch["startup_timeout_seconds"] + 15)
        created = True
        try:
            env_file.unlink()
        except FileNotFoundError:
            pass
        container_id, pid, start_ticks, image_id, network_mode = _inspect_container(name)
        if network_mode != "none":
            raise ManagedClientError("container did not remain on Docker network mode none")
        log_fd = os.open(
            runtime_paths["log"],
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
            0o600,
        )
        log_stream = os.fdopen(log_fd, "wb", closefd=True)
        log_fd = None
        api_profile["container"] = name
        managed = ManagedClient(
            client_id, role, profile, namespace, namespace_identity, None, name,
            container_id, pid, start_ticks, image_id, runtime_paths["log"],
            api_profile, log_stream,
        )
        if not managed.still_matches_pin():
            raise ManagedClientError("container changed identity during launch")
        return managed
    except BaseException:
        try:
            env_file.unlink()
        except FileNotFoundError:
            pass
        if log_stream is not None:
            log_stream.close()
        elif log_fd is not None:
            os.close(log_fd)
        if created:
            try:
                container_id, _pid, _ticks, _image, _network = _inspect_container(name)
                _docker(["docker", "rm", "--force", container_id], timeout=15)
            except ManagedClientError:
                pass
        raise


def launch_client(
    client_id: str,
    profile: dict[str, Any],
    role: str,
    topology: dict[str, Any],
    runtime_paths: dict[str, Any],
    *,
    proc_root: Path = Path("/proc"),
) -> ManagedClient:
    """Start a client in its attached namespace with task-private data/state."""
    if client_id == "urtorrent":
        raise ManagedClientError("urtorrent process lifecycle is owned by urtorrent_runner")
    if role not in {"target", "seed"}:
        raise ManagedClientError("client role must be target or seed")
    if not isinstance(profile, dict) or not isinstance(runtime_paths, dict):
        raise ManagedClientError("client profile and runtime paths must be objects")
    runtime_paths = {**runtime_paths}
    for key in ("data", "state", "log", "pid", "env"):
        value = runtime_paths.get(key)
        if not isinstance(value, Path) or not value.is_absolute():
            raise ManagedClientError(f"client runtime path {key!r} must be absolute")
    if not runtime_paths["data"].is_dir() or not runtime_paths["state"].is_dir():
        raise ManagedClientError("client data and state directories must exist before launch")
    if profile.get("runtime") == "container":
        raise ManagedClientError(
            "container clients must start unattached, then bind after task_network.create_topology"
        )

    role_profile, values = _profile_for_role(client_id, profile, role, topology, runtime_paths)
    if client_id == "torrentngd":
        _prepare_torrentng_config(role_profile, runtime_paths)
    launch = role_profile["paired_launch"]
    environment = _launch_environment(role_profile, values)
    namespace = role_profile["task_api_context"]["namespace"]
    namespace_identity = role_profile["task_api_context"]["namespace_identity"]
    runtime_kind = role_profile["runtime"]

    if runtime_kind == "native_binary":
        binary_path = Path(str(role_profile.get("binary_path", ""))).expanduser()
        digest = role_profile.get("expected_binary_sha256")
        if not binary_path.is_absolute() or not isinstance(digest, str):
            raise ManagedClientError("native client profile is missing its pinned executable")
        arguments = [str(binary_path), *launch["args"]]
        state_dir = runtime_paths["state"]
        process = ManagedProcess.start(
            arguments,
            binary_path=binary_path,
            expected_binary_sha256=digest,
            log_path=runtime_paths["log"],
            environment=environment,
            pid_file_path=runtime_paths["pid"],
            cwd=state_dir,
            startup_timeout_seconds=launch["startup_timeout_seconds"],
            network_namespace=namespace,
            network_namespace_identity=namespace_identity,
            proc_root=proc_root,
        )
        role_profile["pid_file"] = str(runtime_paths["pid"])
        return ManagedClient(
            client_id, role, profile, namespace, namespace_identity, process, None,
            None, process.pid, process.identity.start_ticks, None,
            runtime_paths["log"], role_profile,
        )

    raise ManagedClientError(f"unsupported runtime kind {runtime_kind!r}")


def launch_container_unattached(
    client_id: str,
    profile: dict[str, Any],
    role: str,
    runtime_paths: dict[str, Any],
) -> ManagedClient:
    """Start a digest-pinned client on ``--network none`` before topology attach.

    Only launch arguments that do not need the eventual tracker/API address can
    be used here. Bind the returned process with ``bind_client_topology`` after
    ``task_network.create_topology`` attaches its namespace.
    """
    if role not in {"target", "seed"} or profile.get("runtime") != "container":
        raise ManagedClientError("unattached launch requires a container profile and target/seed role")
    launch = profile.get("paired_launch")
    if not isinstance(launch, dict):
        raise ManagedClientError("pinned client profile has no paired_launch contract")
    for key in ("data", "state", "log", "pid", "env"):
        path = runtime_paths.get(key)
        if not isinstance(path, Path) or not path.is_absolute():
            raise ManagedClientError(f"client runtime path {key!r} must be absolute")
    if not runtime_paths["data"].is_dir() or not runtime_paths["state"].is_dir():
        raise ManagedClientError("client data and state directories must exist before launch")
    values = {
        "api_bind": "0.0.0.0",
        "api_port": str(launch["api_port"] or ""),
        "peer_port": str(launch["peer_port"]),
        "data_path": "/data",
        "state_path": "/state",
    }
    rendered_launch = dict(launch)
    rendered_launch["args"] = [_render(value, values) for value in launch["args"]]
    rendered_launch["environment"] = {
        name: _render(value, values) for name, value in launch["environment"].items()
    }
    unattached_profile = dict(profile)
    unattached_profile["paired_launch"] = rendered_launch
    environment = _launch_environment(unattached_profile, values)
    return _start_container_client(
        client_id,
        profile,
        role,
        "",
        {},
        unattached_profile,
        rendered_launch,
        environment,
        runtime_paths,
    )


def bind_client_topology(
    client: ManagedClient,
    topology: dict[str, Any],
    runtime_paths: dict[str, Any],
) -> None:
    """Bind an already attached container's API and settings readback to its task endpoint."""
    if client.process is not None or client._stopped:
        raise ManagedClientError("only a running container can be bound to a task topology")
    if not client.still_matches_pin():
        raise ManagedClientError("container changed identity before task topology binding")
    role_profile, _values = _profile_for_role(
        client.client_id,
        client.profile,
        client.role,
        topology,
        runtime_paths,
    )
    role_profile["container"] = client.container_name
    client.namespace = role_profile["task_api_context"]["namespace"]
    client.namespace_identity = role_profile["task_api_context"]["namespace_identity"]
    client.api_profile = role_profile
