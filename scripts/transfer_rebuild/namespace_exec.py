#!/usr/bin/env python3
"""Enter a recorded network namespace, then exec the already-open client inode."""

from __future__ import annotations

import argparse
import ctypes
import fcntl
import os
import stat
import sys

CLONE_NEWNET = 0x40000000


def _enter_network_namespace(namespace_fd: int, expected_device: int, expected_inode: int) -> None:
    namespace = os.fstat(namespace_fd)
    if (
        not stat.S_ISREG(namespace.st_mode)
        or (namespace.st_dev, namespace.st_ino) != (expected_device, expected_inode)
    ):
        raise RuntimeError("inherited namespace descriptor no longer matches the topology record")
    libc = ctypes.CDLL(None, use_errno=True)
    setns = getattr(libc, "setns", None)
    if setns is None:
        raise RuntimeError("libc does not expose setns")
    setns.argtypes = (ctypes.c_int, ctypes.c_int)
    setns.restype = ctypes.c_int
    if setns(namespace_fd, CLONE_NEWNET) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error), "setns")
    current = os.stat("/proc/self/ns/net")
    if (current.st_dev, current.st_ino) != (expected_device, expected_inode):
        raise RuntimeError("entered network namespace does not match the topology record")


def _close_on_exec(descriptor: int) -> None:
    flags = fcntl.fcntl(descriptor, fcntl.F_GETFD)
    fcntl.fcntl(descriptor, fcntl.F_SETFD, flags | fcntl.FD_CLOEXEC)


def main(argv: list[str] | None = None) -> int:
    arguments = sys.argv[1:] if argv is None else argv
    try:
        separator = arguments.index("--")
    except ValueError as error:
        raise RuntimeError("client argument separator is missing") from error
    parser = argparse.ArgumentParser()
    parser.add_argument("--namespace-fd", type=int, required=True)
    parser.add_argument("--namespace-device", type=int, required=True)
    parser.add_argument("--namespace-inode", type=int, required=True)
    parser.add_argument("--executable-fd", type=int, required=True)
    options = parser.parse_args(arguments[:separator])
    client_arguments = arguments[separator + 1 :]
    if not client_arguments or any("\x00" in value for value in client_arguments):
        raise RuntimeError("pinned client argument vector is empty or malformed")
    if os.execve not in os.supports_fd:
        raise RuntimeError("this Python runtime cannot exec an already-open executable descriptor")
    _enter_network_namespace(
        options.namespace_fd,
        options.namespace_device,
        options.namespace_inode,
    )
    _close_on_exec(options.namespace_fd)
    _close_on_exec(options.executable_fd)
    # Executing by FD preserves the digest-checked inode even if its pathname is
    # replaced after the supervisor opened it.
    os.execve(options.executable_fd, client_arguments, dict(os.environ))
    return 127


if __name__ == "__main__":
    raise SystemExit(main())
