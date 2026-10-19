#!/usr/bin/env python3
"""Start and stop one exact native client process for paired transfer trials.

The supervisor is Linux-only like the independent resource observer. It does
not interpret shell syntax, inherit the caller's environment implicitly, or
signal a PID after its process identity changes. The pinned executable is
opened once and launched through that descriptor so replacing its pathname
between digest capture and exec cannot substitute another binary.
"""

from __future__ import annotations

import hashlib
import fcntl
import math
import os
import re
import signal
import stat
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Mapping, Sequence


SHA256_RE = re.compile(r"^[a-f0-9]{64}$")
ENV_NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
NETWORK_NAMESPACE_RE = re.compile(r"^tng[ts]-[a-f0-9]{8,12}$")
READ_CHUNK_BYTES = 1024 * 1024


class ProcessSupervisorError(RuntimeError):
    """A client process could not be managed without weakening its pin."""


@dataclass(frozen=True)
class ProcessIdentity:
    pid: int
    start_ticks: int
    executable_device: int
    executable_inode: int
    network_namespace_device: int | None = None
    network_namespace_inode: int | None = None


def _file_identity(metadata: os.stat_result) -> tuple[int, int, int, int, int]:
    return (
        metadata.st_dev,
        metadata.st_ino,
        metadata.st_size,
        metadata.st_mtime_ns,
        metadata.st_ctime_ns,
    )


def _open_pinned_executable(path: Path, expected_sha256: str) -> tuple[int, os.stat_result]:
    if not isinstance(expected_sha256, str) or SHA256_RE.fullmatch(expected_sha256) is None:
        raise ProcessSupervisorError("expected executable digest must be lowercase SHA-256")
    if not path.is_absolute() or ".." in path.parts:
        raise ProcessSupervisorError("pinned executable path must be absolute and normalized")
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise ProcessSupervisorError("cannot open the pinned executable without following links") from error

    digest = hashlib.sha256()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size <= 0:
            raise ProcessSupervisorError("pinned executable must be a nonempty regular file")
        while chunk := os.read(descriptor, READ_CHUNK_BYTES):
            digest.update(chunk)
        after = os.fstat(descriptor)
    except Exception:
        os.close(descriptor)
        raise

    if _file_identity(before) != _file_identity(after):
        os.close(descriptor)
        raise ProcessSupervisorError("pinned executable changed while its digest was captured")
    if digest.hexdigest() != expected_sha256:
        os.close(descriptor)
        raise ProcessSupervisorError("pinned executable digest differs from its declared identity")
    return descriptor, before


def _open_new_private_file(path: Path, label: str) -> int:
    """Create a mode-0600 file below an existing no-symlink directory tree."""
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise ProcessSupervisorError(f"{label} path must be absolute and normalized")
    if not hasattr(os, "O_NOFOLLOW") or not hasattr(os, "O_DIRECTORY"):
        raise ProcessSupervisorError("safe no-follow log creation is unavailable on this platform")

    directory_flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_DIRECTORY
    try:
        current = os.open("/", directory_flags)
    except OSError as error:
        raise ProcessSupervisorError("cannot open the filesystem root without following links") from error
    try:
        for component in path.parent.parts[1:]:
            following = os.open(component, directory_flags, dir_fd=current)
            metadata = os.fstat(following)
            if not stat.S_ISDIR(metadata.st_mode):
                os.close(following)
                raise ProcessSupervisorError("client log path contains a non-directory component")
            os.close(current)
            current = following
        try:
            return os.open(
                path.name,
                os.O_WRONLY
                | os.O_CREAT
                | os.O_EXCL
                | os.O_CLOEXEC
                | os.O_NOFOLLOW,
                0o600,
                dir_fd=current,
            )
        except OSError as error:
            raise ProcessSupervisorError(f"cannot create a new private {label}: {error}") from error
    except OSError as error:
        raise ProcessSupervisorError(
            f"{label} parent must be an existing no-symlink directory"
        ) from error
    finally:
        os.close(current)


def _process_identity(
    pid: int,
    proc_root: Path = Path("/proc"),
    expected_network_namespace: tuple[int, int] | None = None,
) -> ProcessIdentity | None:
    try:
        raw_stat = (proc_root / str(pid) / "stat").read_text(encoding="ascii")
        close_paren = raw_stat.rfind(")")
        if close_paren < 0:
            return None
        fields = raw_stat[close_paren + 2 :].split()
        if len(fields) <= 19:
            return None
        start_ticks = int(fields[19])
        executable = os.stat(proc_root / str(pid) / "exe")
        namespace = os.stat(proc_root / str(pid) / "ns" / "net") if expected_network_namespace else None
    except (OSError, UnicodeDecodeError, ValueError):
        return None
    if not stat.S_ISREG(executable.st_mode):
        return None
    if namespace is not None and (
        namespace.st_dev,
        namespace.st_ino,
    ) != expected_network_namespace:
        return None
    return ProcessIdentity(
        pid=pid,
        start_ticks=start_ticks,
        executable_device=executable.st_dev,
        executable_inode=executable.st_ino,
        network_namespace_device=namespace.st_dev if namespace is not None else None,
        network_namespace_inode=namespace.st_ino if namespace is not None else None,
    )


def _validate_network_namespace(
    name: str | None,
    identity: Mapping[str, int] | None,
) -> tuple[int, int] | None:
    if name is None and identity is None:
        return None
    if not isinstance(name, str) or NETWORK_NAMESPACE_RE.fullmatch(name) is None:
        raise ProcessSupervisorError("network namespace must use a task-owned tngt/tngs name")
    if not isinstance(identity, Mapping) or set(identity) != {"device", "inode"}:
        raise ProcessSupervisorError("network namespace launch requires its recorded device and inode")
    device = identity.get("device")
    inode = identity.get("inode")
    if (
        isinstance(device, bool)
        or not isinstance(device, int)
        or device < 0
        or isinstance(inode, bool)
        or not isinstance(inode, int)
        or inode <= 0
    ):
        raise ProcessSupervisorError("network namespace identity is malformed")
    return device, inode


def _open_network_namespace(name: str, expected: tuple[int, int]) -> int:
    try:
        parent_fd = os.open(
            "/run/netns",
            os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
        )
    except OSError as error:
        raise ProcessSupervisorError("cannot open /run/netns without following links") from error
    try:
        descriptor = os.open(
            name,
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW,
            dir_fd=parent_fd,
        )
    except OSError as error:
        os.close(parent_fd)
        raise ProcessSupervisorError("cannot open the recorded network namespace safely") from error
    else:
        os.close(parent_fd)
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or (metadata.st_dev, metadata.st_ino) != expected:
            raise ProcessSupervisorError("network namespace no longer matches its recorded identity")
        return descriptor
    except BaseException:
        os.close(descriptor)
        raise


def _validate_command(
    argv: Sequence[str],
    binary_path: Path,
    environment: Mapping[str, str],
    startup_timeout_seconds: float,
) -> list[str]:
    if not isinstance(argv, (list, tuple)) or not argv:
        raise ProcessSupervisorError("client argv must be a nonempty argument array")
    arguments = list(argv)
    if any(not isinstance(value, str) or "\x00" in value for value in arguments):
        raise ProcessSupervisorError("client argv entries must be NUL-free strings")
    if not binary_path.is_absolute() or Path(arguments[0]) != binary_path:
        raise ProcessSupervisorError("client argv[0] must exactly match the pinned absolute binary path")
    if not isinstance(environment, Mapping):
        raise ProcessSupervisorError("client environment must be an explicit mapping")
    for name, value in environment.items():
        if (
            not isinstance(name, str)
            or ENV_NAME_RE.fullmatch(name) is None
            or not isinstance(value, str)
            or "\x00" in value
        ):
            raise ProcessSupervisorError("client environment contains an invalid name or value")
    if (
        isinstance(startup_timeout_seconds, bool)
        or not isinstance(startup_timeout_seconds, (int, float))
        or not math.isfinite(startup_timeout_seconds)
        or not 0 < startup_timeout_seconds <= 30
    ):
        raise ProcessSupervisorError("startup timeout must be finite and within (0, 30] seconds")
    return arguments


class ManagedProcess:
    """A foreground native process bound to one pinned executable inode."""

    def __init__(
        self,
        process: subprocess.Popen[bytes],
        pidfd: int,
        identity: ProcessIdentity,
        binary_path: Path,
        binary_sha256: str,
        binary_size_bytes: int,
        log_path: Path,
        proc_root: Path,
    ) -> None:
        self._process = process
        self._pidfd = pidfd
        self.identity = identity
        self.binary_path = binary_path
        self.binary_sha256 = binary_sha256
        self.binary_size_bytes = binary_size_bytes
        self.log_path = log_path
        self._proc_root = proc_root

    @classmethod
    def start(
        cls,
        argv: Sequence[str],
        *,
        binary_path: Path,
        expected_binary_sha256: str,
        log_path: Path,
        environment: Mapping[str, str],
        pid_file_path: Path | None = None,
        cwd: Path | None = None,
        startup_timeout_seconds: float = 5.0,
        network_namespace: str | None = None,
        network_namespace_identity: Mapping[str, int] | None = None,
        proc_root: Path = Path("/proc"),
        monotonic: Callable[[], float] = time.monotonic,
        sleep: Callable[[float], None] = time.sleep,
    ) -> "ManagedProcess":
        """Start a pinned foreground process without a shell or inherited env."""
        arguments = _validate_command(argv, binary_path, environment, startup_timeout_seconds)
        if not hasattr(os, "pidfd_open") or not hasattr(signal, "pidfd_send_signal"):
            raise ProcessSupervisorError("pidfd process control is required for safe client shutdown")
        if cwd is not None and (not cwd.is_absolute() or not cwd.is_dir()):
            raise ProcessSupervisorError("client working directory must be an existing absolute directory")
        namespace_identity = _validate_network_namespace(
            network_namespace, network_namespace_identity
        )

        executable_fd, executable_metadata = _open_pinned_executable(
            binary_path, expected_binary_sha256
        )
        log_fd: int | None = None
        namespace_fd: int | None = None
        process: subprocess.Popen[bytes] | None = None
        pidfd: int | None = None
        try:
            if network_namespace is not None and namespace_identity is not None:
                namespace_fd = _open_network_namespace(network_namespace, namespace_identity)
            log_fd = _open_new_private_file(log_path, "client log")
            descriptor_path = f"/proc/{os.getpid()}/fd/{executable_fd}"
            if namespace_fd is None:
                launch_arguments = arguments
                launch_executable = descriptor_path
                pass_fds: tuple[int, ...] = ()
            else:
                helper = Path(__file__).with_name("namespace_exec.py")
                interpreter = Path(sys.executable).resolve(strict=True)
                launch_arguments = [
                    str(interpreter), "-I", str(helper),
                    "--namespace-fd", str(namespace_fd),
                    "--namespace-device", str(namespace_identity[0]),
                    "--namespace-inode", str(namespace_identity[1]),
                    "--executable-fd", str(executable_fd),
                    "--", *arguments,
                ]
                launch_executable = str(interpreter)
                pass_fds = (executable_fd, namespace_fd)
            try:
                process = subprocess.Popen(
                    launch_arguments,
                    executable=launch_executable,
                    stdin=subprocess.DEVNULL,
                    stdout=log_fd,
                    stderr=subprocess.STDOUT,
                    close_fds=True,
                    pass_fds=pass_fds,
                    start_new_session=True,
                    cwd=cwd,
                    env=dict(environment),
                )
            except OSError as error:
                raise ProcessSupervisorError(f"cannot start pinned client: {error}") from error
            finally:
                os.close(log_fd)
                log_fd = None
                if namespace_fd is not None:
                    os.close(namespace_fd)
                    namespace_fd = None

            opened_after_exec = os.fstat(executable_fd)
            if _file_identity(executable_metadata) != _file_identity(opened_after_exec):
                raise ProcessSupervisorError("pinned executable changed while the client was starting")
            os.close(executable_fd)
            executable_fd = -1

            try:
                pidfd = os.pidfd_open(process.pid, 0)
            except OSError as error:
                process.kill()
                process.wait()
                raise ProcessSupervisorError("cannot pin the started process with a pidfd") from error

            deadline = monotonic() + float(startup_timeout_seconds)
            expected_executable = (executable_metadata.st_dev, executable_metadata.st_ino)
            expected_namespace = namespace_identity
            while True:
                if process.poll() is not None:
                    raise ProcessSupervisorError(
                        f"pinned client exited during startup with status {process.returncode}"
                    )
                observed = _process_identity(process.pid, proc_root, expected_namespace)
                if observed is not None:
                    if (observed.executable_device, observed.executable_inode) != expected_executable:
                        if expected_namespace is None:
                            raise ProcessSupervisorError(
                                "started process does not use the pinned executable inode"
                            )
                    else:
                        if pid_file_path is not None:
                            pid_file_fd = _open_new_private_file(pid_file_path, "client PID file")
                            try:
                                pending = memoryview(f"{observed.pid}\n".encode("ascii"))
                                while pending:
                                    written = os.write(pid_file_fd, pending)
                                    if written <= 0:
                                        raise ProcessSupervisorError("short write while recording client PID")
                                    pending = pending[written:]
                                os.fsync(pid_file_fd)
                            finally:
                                os.close(pid_file_fd)
                        return cls(
                            process,
                            pidfd,
                            observed,
                            binary_path,
                            expected_binary_sha256,
                            executable_metadata.st_size,
                            log_path,
                            proc_root,
                        )
                remaining = deadline - monotonic()
                if remaining <= 0:
                    raise ProcessSupervisorError("timed out waiting for the pinned client process identity")
                sleep(min(0.01, remaining))
        except Exception:
            if process is not None and process.poll() is None:
                # This Popen instance owns the child PID; no unrelated process is signaled.
                process.kill()
                process.wait()
            if pidfd is not None:
                os.close(pidfd)
            if log_fd is not None:
                os.close(log_fd)
            if namespace_fd is not None:
                os.close(namespace_fd)
            if executable_fd >= 0:
                os.close(executable_fd)
            raise

    @property
    def pid(self) -> int:
        return self.identity.pid

    def still_matches_pin(self) -> bool:
        namespace_identity = None
        if (
            self.identity.network_namespace_device is not None
            and self.identity.network_namespace_inode is not None
        ):
            namespace_identity = (
                self.identity.network_namespace_device,
                self.identity.network_namespace_inode,
            )
        return _process_identity(self.pid, self._proc_root, namespace_identity) == self.identity

    def runtime_identity(self) -> dict[str, object]:
        """Return the native identity fields consumed by ``adapters.py``."""
        return {
            "kind": "native_binary",
            "binary_path": str(self.binary_path),
            "binary_sha256": self.binary_sha256,
            "binary_size_bytes": self.binary_size_bytes,
            "process_id": self.identity.pid,
            "process_start_ticks": self.identity.start_ticks,
            "process_exe_device": self.identity.executable_device,
            "process_exe_inode": self.identity.executable_inode,
        }

    def wait(self, timeout_seconds: float | None = None) -> int:
        """Wait for exit and return the child status."""
        try:
            return self._process.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            raise

    def stop(self, *, grace_seconds: float = 10.0) -> int:
        """Stop only the same process instance captured at launch."""
        if (
            isinstance(grace_seconds, bool)
            or not isinstance(grace_seconds, (int, float))
            or not math.isfinite(grace_seconds)
            or not 0 < grace_seconds <= 120
        ):
            raise ProcessSupervisorError("stop grace must be finite and within (0, 120] seconds")
        if self._process.poll() is not None:
            return self._process.wait()
        if self._pidfd < 0:
            raise ProcessSupervisorError("cannot stop a process after its pidfd has been closed")
        if not self.still_matches_pin():
            raise ProcessSupervisorError("refusing to signal a process whose pinned identity changed")

        try:
            signal.pidfd_send_signal(self._pidfd, signal.SIGTERM)
        except ProcessLookupError:
            return self._process.wait()
        except OSError as error:
            raise ProcessSupervisorError("cannot send SIGTERM through the pinned process handle") from error

        try:
            return self._process.wait(timeout=float(grace_seconds))
        except subprocess.TimeoutExpired:
            if not self.still_matches_pin():
                raise ProcessSupervisorError("refusing forced shutdown after process identity changed")
            try:
                signal.pidfd_send_signal(self._pidfd, signal.SIGKILL)
            except ProcessLookupError:
                return self._process.wait()
            except OSError as error:
                raise ProcessSupervisorError("cannot force-stop the pinned client process") from error
            try:
                return self._process.wait(timeout=5)
            except subprocess.TimeoutExpired as error:
                raise ProcessSupervisorError("pinned client did not exit after SIGKILL") from error

    def close(self) -> None:
        if self._pidfd >= 0:
            os.close(self._pidfd)
            self._pidfd = -1

    def __enter__(self) -> "ManagedProcess":
        return self

    def __exit__(self, _exc_type, _exc, _traceback) -> None:
        try:
            if self._process.poll() is None:
                self.stop()
        finally:
            self.close()
