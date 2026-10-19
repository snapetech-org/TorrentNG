#!/usr/bin/env python3
"""Serve one private, task-local HTTP tracker for a paired transfer trial."""

from __future__ import annotations

import argparse
import hashlib
import ipaddress
import json
import math
import os
import signal
import sys
import threading
import time
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any


class TaskTrackerError(ValueError):
    """A tracker request does not belong to the current isolated trial."""


def _decode_query_component(value: str) -> bytes:
    """Decode form-style query bytes while rejecting malformed escapes."""
    index = 0
    while index < len(value):
        character = value[index]
        if character == "%":
            if (
                index + 2 >= len(value)
                or value[index + 1] not in "0123456789abcdefABCDEF"
                or value[index + 2] not in "0123456789abcdefABCDEF"
            ):
                raise TaskTrackerError("announce query contains malformed percent encoding")
            index += 3
            continue
        codepoint = ord(character)
        if codepoint > 0x7F:
            raise TaskTrackerError("announce query must percent-encode non-ASCII bytes")
        if codepoint <= 0x20 or codepoint == 0x7F:
            raise TaskTrackerError("announce query must percent-encode control bytes")
        index += 1
    return urllib.parse.unquote_to_bytes(value.replace("+", " "))


def _bencode(value: Any) -> bytes:
    if isinstance(value, bool):
        raise TypeError("tracker bencode does not accept booleans")
    if isinstance(value, int):
        return b"i" + str(value).encode("ascii") + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode("ascii") + b":" + value
    if isinstance(value, (list, tuple)):
        return b"l" + b"".join(_bencode(item) for item in value) + b"e"
    if isinstance(value, dict):
        if any(not isinstance(key, bytes) for key in value):
            raise TypeError("tracker bencode dictionary keys must be bytes")
        return b"d" + b"".join(
            _bencode(key) + _bencode(value[key]) for key in sorted(value)
        ) + b"e"
    raise TypeError(f"unsupported tracker bencode value: {type(value).__name__}")


def _query_bytes(raw_query: str) -> dict[bytes, bytes]:
    result: dict[bytes, bytes] = {}
    if len(raw_query) > 16 * 1024:
        raise TaskTrackerError("announce query exceeds the size limit")
    for pair in raw_query.split("&"):
        if not pair:
            continue
        key, separator, value = pair.partition("=")
        if not separator:
            raise TaskTrackerError("announce query contains a field without a value")
        decoded_key = _decode_query_component(key)
        decoded_value = _decode_query_component(value)
        if decoded_key in result:
            raise TaskTrackerError("announce query contains a duplicate field")
        result[decoded_key] = decoded_value
    return result


class TaskTracker:
    """Offer one pinned peer only to the selected endpoint dialer."""

    def __init__(
        self,
        *,
        gateway: str,
        target_address: str,
        seed_address: str,
        seed_port: int | None,
        seed_peer_id: bytes | None,
        info_hashes: set[bytes],
        topology_sha256: str,
        dialer_role: str = "target",
        seed_ready_timeout_seconds: float = 10.0,
    ) -> None:
        self.gateway = self._ipv4(gateway, "gateway")
        self.target_address = self._ipv4(target_address, "target address")
        self.seed_address = self._ipv4(seed_address, "seed address")
        if len({self.gateway, self.target_address, self.seed_address}) != 3:
            raise TaskTrackerError("task tracker endpoints must have distinct addresses")
        subnet = self._network(self.gateway, self.target_address, self.seed_address)
        if (
            not subnet.subnet_of(ipaddress.ip_network("198.18.0.0/15"))
            or self.gateway != subnet.network_address + 1
            or self.seed_address != subnet.network_address + 2
            or self.target_address != subnet.network_address + 3
        ):
            raise TaskTrackerError("task tracker addresses do not match the recorded benchmark subnet")
        if seed_port is not None and (
            isinstance(seed_port, bool)
            or not isinstance(seed_port, int)
            or not 1 <= seed_port <= 65535
        ):
            raise TaskTrackerError("seed peer port must be within 1..65535 or be discovered")
        if seed_peer_id is not None and (
            not isinstance(seed_peer_id, bytes) or len(seed_peer_id) != 20
        ):
            raise TaskTrackerError("seed peer ID must contain exactly 20 bytes or be discovered")
        if dialer_role not in {"target", "seed"}:
            raise TaskTrackerError("dialer role must be target or seed")
        if (
            not isinstance(info_hashes, set)
            or any(not isinstance(value, bytes) or len(value) not in (20, 32) for value in info_hashes)
        ):
            raise TaskTrackerError("tracker must pin one or more v1/v2 info hashes")
        if not isinstance(topology_sha256, str) or len(topology_sha256) != 64 or any(
            character not in "0123456789abcdef" for character in topology_sha256
        ):
            raise TaskTrackerError("task topology digest must be lowercase SHA-256")
        if (
            isinstance(seed_ready_timeout_seconds, bool)
            or not isinstance(seed_ready_timeout_seconds, (int, float))
            or not math.isfinite(seed_ready_timeout_seconds)
            or not 0 < seed_ready_timeout_seconds <= 30
        ):
            raise TaskTrackerError("seed readiness wait must be finite and within (0, 30] seconds")
        self.seed_port = seed_port
        self.seed_peer_id = seed_peer_id
        self.dialer_role = dialer_role
        self.target_port: int | None = None
        self.target_peer_id: bytes | None = None
        self.bind_address: str | None = None
        self.seed_ready_timeout_seconds = float(seed_ready_timeout_seconds)
        self.info_hashes = frozenset(info_hashes)
        self._hashes_pinned = bool(info_hashes)
        self.topology_sha256 = topology_sha256
        self._lock = threading.Condition()
        self._announces: dict[str, int] = {"started": 0, "completed": 0, "stopped": 0, "other": 0}
        self._request_count = 0

    @property
    def seed_ready(self) -> bool:
        with self._lock:
            return self.seed_port is not None and self.seed_peer_id is not None

    @property
    def target_ready(self) -> bool:
        with self._lock:
            return self.target_port is not None and self.target_peer_id is not None

    def pin_info_hashes(self, info_hashes: set[bytes]) -> None:
        """Arm a pre-bound tracker once fixture metainfo has supplied its hashes."""
        if (
            not isinstance(info_hashes, set)
            or not info_hashes
            or any(not isinstance(value, bytes) or len(value) not in (20, 32) for value in info_hashes)
        ):
            raise TaskTrackerError("tracker must pin one or more v1/v2 info hashes")
        with self._lock:
            if self._hashes_pinned or self._request_count:
                raise TaskTrackerError("tracker info hashes can be pinned only once before announces")
            self.info_hashes = frozenset(info_hashes)
            self._hashes_pinned = True

    @staticmethod
    def _ipv4(value: str, label: str) -> ipaddress.IPv4Address:
        try:
            address = ipaddress.ip_address(value)
        except ValueError as error:
            raise TaskTrackerError(f"{label} must be an IPv4 address") from error
        if not isinstance(address, ipaddress.IPv4Address):
            raise TaskTrackerError(f"{label} must be an IPv4 address")
        return address

    @staticmethod
    def _network(*addresses: ipaddress.IPv4Address) -> ipaddress.IPv4Network:
        if not addresses:
            raise TaskTrackerError("task tracker requires at least one endpoint")
        candidate = ipaddress.ip_network(f"{addresses[0]}/24", strict=False)
        if all(address in candidate for address in addresses):
            return candidate
        raise TaskTrackerError("task tracker endpoints must share one benchmark subnet (/24)")

    def response(self, source_address: str, raw_query: str) -> bytes:
        source = self._ipv4(source_address, "announce source")
        if source not in {self.target_address, self.seed_address}:
            raise TaskTrackerError("announce source is outside the task network")
        query = _query_bytes(raw_query)
        info_hash = query.get(b"info_hash")
        peer_id = query.get(b"peer_id")
        port_value = query.get(b"port", b"")
        if info_hash not in self.info_hashes:
            raise TaskTrackerError("announce info hash is not pinned for this trial")
        if not isinstance(peer_id, bytes) or len(peer_id) != 20:
            raise TaskTrackerError("announce peer ID must contain exactly 20 bytes")
        if not port_value.isdigit():
            raise TaskTrackerError("announce peer port must be an integer")
        peer_port = int(port_value)
        if not 1 <= peer_port <= 65535:
            raise TaskTrackerError("announce peer port must be within 1..65535")
        event = query.get(b"event", b"")
        if event not in {b"", b"started", b"completed", b"stopped"}:
            raise TaskTrackerError("announce event is unsupported")
        compact = query.get(b"compact", b"1")
        if compact not in {b"0", b"1"}:
            raise TaskTrackerError("compact must be 0 or 1")
        is_seed = source == self.seed_address
        with self._lock:
            if is_seed:
                if peer_id == self.target_peer_id:
                    raise TaskTrackerError("target peer ID appeared in the seed namespace")
                if self.seed_peer_id is None:
                    self.seed_peer_id = peer_id
                elif peer_id != self.seed_peer_id:
                    raise TaskTrackerError("unexpected peer announced from the seed namespace")
                if self.seed_port is None:
                    self.seed_port = peer_port
                elif peer_port != self.seed_port:
                    raise TaskTrackerError("seed peer port changed during the task")
            else:
                if peer_id == self.seed_peer_id:
                    raise TaskTrackerError("seed peer ID appeared in the target namespace")
                if self.target_peer_id is None:
                    self.target_peer_id = peer_id
                elif peer_id != self.target_peer_id:
                    raise TaskTrackerError("unexpected peer announced from the target namespace")
                if self.target_port is None:
                    self.target_port = peer_port
                elif peer_port != self.target_port:
                    raise TaskTrackerError("target peer port changed during the task")

            self._lock.notify_all()
            dialer_address = (
                self.target_address if self.dialer_role == "target" else self.seed_address
            )
            if source != dialer_address:
                # The non-dialing endpoint needs no tracker peer, which avoids
                # competing reciprocal connections.
                peer_address = None
                peer_port = None
                peer_id = None
            else:
                peer_is_seed = self.dialer_role == "target"
                ready_deadline = time.monotonic() + self.seed_ready_timeout_seconds
                while (
                    self.seed_port is None or self.seed_peer_id is None
                    if peer_is_seed
                    else self.target_port is None or self.target_peer_id is None
                ):
                    remaining = ready_deadline - time.monotonic()
                    if remaining <= 0:
                        raise TaskTrackerError(
                            "other endpoint did not announce before the readiness deadline"
                        )
                    self._lock.wait(remaining)
                if peer_is_seed:
                    peer_address = self.seed_address
                    peer_port = self.seed_port
                    peer_id = self.seed_peer_id
                else:
                    peer_address = self.target_address
                    peer_port = self.target_port
                    peer_id = self.target_peer_id
            self._request_count += 1
            self._announces[event.decode("ascii") if event else "other"] += 1

        if peer_address is None:
            peers: Any = b"" if compact == b"1" else []
        elif compact == b"1":
            peers: Any = peer_address.packed + peer_port.to_bytes(2, "big")
        else:
            peers = [
                {
                    b"peer id": peer_id,
                    b"ip": str(peer_address).encode("ascii"),
                    b"port": peer_port,
                }
            ]
        return _bencode({
            b"complete": 1,
            b"incomplete": 0 if is_seed else 1,
            b"interval": 30,
            b"min interval": 10,
            b"peers": peers,
            b"tracker id": hashlib.sha256(self.topology_sha256.encode("ascii")).digest()[:8],
        })

    def snapshot(self) -> dict[str, Any]:
        with self._lock:
            value: dict[str, Any] = {
                "schema_version": "transfer-task-tracker-v1",
                "topology_sha256": self.topology_sha256,
                "gateway": str(self.gateway),
                "bind_address": self.bind_address,
                "dialer_role": self.dialer_role,
                "target_address": str(self.target_address),
                "target_port": self.target_port,
                "target_peer_id_sha256": (
                    hashlib.sha256(self.target_peer_id).hexdigest()
                    if self.target_peer_id is not None
                    else None
                ),
                "target_ready": self.target_port is not None and self.target_peer_id is not None,
                "seed_address": str(self.seed_address),
                "seed_port": self.seed_port,
                "seed_peer_id_sha256": (
                    hashlib.sha256(self.seed_peer_id).hexdigest()
                    if self.seed_peer_id is not None
                    else None
                ),
                "seed_ready": self.seed_port is not None and self.seed_peer_id is not None,
                "info_hashes_sha256": sorted(
                    hashlib.sha256(info_hash).hexdigest() for info_hash in self.info_hashes
                ),
                "announce_counts": dict(self._announces),
                "request_count": self._request_count,
            }
        value["artifact_sha256"] = hashlib.sha256(
            json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
        ).hexdigest()
        return value


def serve_tracker(
    tracker: TaskTracker, *, port: int = 0, bind_address: str | None = None
) -> ThreadingHTTPServer:
    """Bind a private tracker to an address owned by the isolated task network."""
    if isinstance(port, bool) or not isinstance(port, int) or not 0 <= port <= 65535:
        raise TaskTrackerError("tracker port must be within 0..65535")
    bind_ip = tracker.gateway if bind_address is None else tracker._ipv4(bind_address, "bind address")
    if bind_ip not in {tracker.gateway, tracker.target_address, tracker.seed_address}:
        raise TaskTrackerError("tracker bind address must belong to the task network")

    class Handler(BaseHTTPRequestHandler):
        server_version = "TorrentNGTaskTracker/1"
        sys_version = ""

        def log_message(self, _format: str, *_args: Any) -> None:
            return

        def do_GET(self) -> None:
            parsed = urllib.parse.urlsplit(self.path)
            if parsed.path != "/announce":
                self.send_error(404)
                return
            try:
                body = tracker.response(self.client_address[0], parsed.query)
            except TaskTrackerError:
                self.send_error(400)
                return
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.send_header("Content-Length", str(len(body)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self) -> None:
            self.send_error(405)

    class Server(ThreadingHTTPServer):
        daemon_threads = True
        block_on_close = False
        allow_reuse_address = False

    try:
        server = Server((str(bind_ip), port), Handler)
        with tracker._lock:
            tracker.bind_address = str(server.server_address[0])
        return server
    except OSError as error:
        raise TaskTrackerError(f"cannot bind tracker to task network address: {error}") from error


def _write_worker_json(path: Path, value: dict[str, Any]) -> None:
    if path.exists() or path.is_symlink():
        raise TaskTrackerError(f"tracker worker output already exists: {path}")
    encoded = (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()
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
                raise TaskTrackerError("short write while publishing tracker worker output")
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


def _serve_worker(args: argparse.Namespace) -> int:
    server: ThreadingHTTPServer | None = None
    try:
        hashes = {bytes.fromhex(value) for value in args.info_hash}
        tracker = TaskTracker(
            gateway=args.gateway,
            target_address=args.target_address,
            seed_address=args.seed_address,
            seed_port=args.seed_port or None,
            seed_peer_id=None,
            info_hashes=hashes,
            topology_sha256=args.topology_sha256,
            dialer_role=args.dialer_role,
        )
        reserved = set(args.reserved_port)
        for _attempt in range(128):
            candidate = serve_tracker(tracker, port=args.port, bind_address=args.bind_address)
            if candidate.server_address[1] not in reserved:
                server = candidate
                break
            candidate.server_close()
        if server is None:
            raise TaskTrackerError("tracker could not find a port outside client listener pins")
        server.timeout = 0.25
        stopping = False

        def request_stop(_signum: int, _frame: Any) -> None:
            nonlocal stopping
            stopping = True

        signal.signal(signal.SIGTERM, request_stop)
        signal.signal(signal.SIGINT, request_stop)
        _write_worker_json(
            args.ready_path,
            {
                "schema_version": "transfer-task-tracker-ready-v1",
                "address": str(server.server_address[0]),
                "port": server.server_address[1],
            },
        )
        while not stopping:
            server.handle_request()
        server.server_close()
        server = None
        _write_worker_json(args.snapshot_path, tracker.snapshot())
        return 0
    except Exception as error:
        print(f"task tracker worker failed: {type(error).__name__}: {error}", file=sys.stderr)
        if server is not None:
            server.server_close()
        return 2


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--worker", action="store_true")
    parser.add_argument("--gateway")
    parser.add_argument("--target-address")
    parser.add_argument("--seed-address")
    parser.add_argument("--dialer-role", choices=("target", "seed"), default="target")
    parser.add_argument("--bind-address")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--seed-port", type=int, default=0)
    parser.add_argument("--reserved-port", type=int, action="append", default=[])
    parser.add_argument("--topology-sha256")
    parser.add_argument("--info-hash", action="append", default=[])
    parser.add_argument("--ready-path", type=Path)
    parser.add_argument("--snapshot-path", type=Path)
    args = parser.parse_args(argv)
    if (
        not args.worker
        or not args.gateway
        or not args.target_address
        or not args.seed_address
        or not args.bind_address
        or not args.topology_sha256
        or not args.info_hash
        or args.ready_path is None
        or args.snapshot_path is None
    ):
        parser.error("worker mode requires task addresses, hashes, and ready/snapshot paths")
    if any(len(value) not in {40, 64} for value in args.info_hash):
        parser.error("worker info hashes must contain 40 or 64 hexadecimal characters")
    return _serve_worker(args)


if __name__ == "__main__":
    raise SystemExit(main())
