#!/usr/bin/env python3
"""Identity-bound torrent controls shared by PERF-110 trial execution."""

from __future__ import annotations

import base64
import hashlib
import json
import math
import os
import posixpath
import re
import stat
import time
import urllib.parse
import xmlrpc.client
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Callable

from strict_json import StrictJsonError, loads as strict_json_loads

from adapters import (
    AdapterError,
    HttpJson,
    _deluge_call,
    _login_qbittorrent,
    _profile_http,
    _scgi_call,
    _transmission_call,
    _env_value,
    probe_client,
)
from compatibility import _connect_deluge, _multipart


POLL_INTERVAL_SECONDS = 0.2
MAX_TORRENT_BYTES = 8 * 1024 * 1024
INFO_HASH_RE = re.compile(r"^(?:[a-fA-F0-9]{40}|[a-fA-F0-9]{64})$")
TORRENT_FILENAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$")


class ClientControlError(RuntimeError):
    """A pinned client could not be controlled without weakening its identity."""


@dataclass(frozen=True)
class TorrentState:
    info_hash: str
    state: str
    progress: float
    total_bytes: int
    received_bytes: int
    uploaded_bytes: int
    download_rate_bps: int
    upload_rate_bps: int
    peers: int | None

    @property
    def complete(self) -> bool:
        return self.progress >= 1.0 or (
            self.total_bytes > 0 and self.received_bytes >= self.total_bytes
        )


def _strict_int(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise ClientControlError(f"client returned an invalid {label}")
    return value


def _rtorrent_int(value: Any, label: str) -> int:
    if isinstance(value, bool):
        raise ClientControlError(f"rTorrent returned an invalid {label}")
    if isinstance(value, int):
        return _strict_int(value, label)
    if isinstance(value, str) and value.isdecimal():
        return int(value)
    raise ClientControlError(f"rTorrent returned an invalid {label}")


def _rtorrent_active(value: Any) -> bool:
    if value in (False, 0, "0", "false", "False"):
        return False
    if value in (True, 1, "1", "true", "True"):
        return True
    raise ClientControlError("rTorrent returned an invalid active state")


def _number(row: dict[str, Any], names: tuple[str, ...], label: str) -> int:
    for name in names:
        if name in row:
            value = row[name]
            if isinstance(value, bool):
                break
            if isinstance(value, int) and value >= 0:
                return value
            if isinstance(value, str) and value.isdecimal():
                return int(value)
            break
    raise ClientControlError(f"client status is missing a valid {label}")


def _mib_rate(value: Any, label: str) -> int:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ClientControlError(f"client status is missing a valid {label}")
    rate = float(value)
    if not math.isfinite(rate) or rate < 0:
        raise ClientControlError(f"client status returned an invalid {label}")
    return int(round(rate * 1024 * 1024))


def _progress(row: dict[str, Any], names: tuple[str, ...], label: str) -> float:
    for name in names:
        if name in row:
            value = row[name]
            if isinstance(value, bool) or not isinstance(value, (int, float)):
                break
            result = float(value)
            if not 0.0 <= result <= 1.0:
                break
            return result
    raise ClientControlError(f"client status is missing a valid {label}")


def _deluge_progress(row: dict[str, Any]) -> float:
    value = row.get("progress")
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ClientControlError("Deluge status is missing a valid progress percentage")
    percent = float(value)
    if not math.isfinite(percent) or not 0 <= percent <= 100:
        raise ClientControlError("Deluge progress is outside 0..100 percent")
    return percent / 100.0


def _paused_state(state: str) -> bool:
    normalized = state.strip().lower().replace("_", "-")
    return normalized.startswith(("paused", "stopped")) or normalized == "checking-paused"


def _canonical_info_hash(value: str) -> str:
    if not isinstance(value, str) or INFO_HASH_RE.fullmatch(value) is None:
        raise ClientControlError("torrent info hash must be a 40- or 64-character hexadecimal value")
    return value.lower()


def _normalize_save_path(value: str) -> str:
    if (
        not isinstance(value, str)
        or not value.startswith("/")
        or value == "/"
        or any(ord(character) < 0x20 or ord(character) == 0x7F for character in value)
        or posixpath.normpath(value) != value
        or any(part in {"", ".", ".."} for part in value.split("/")[1:])
    ):
        raise ClientControlError("client save path must be an absolute normalized trial directory")
    return value


def _torrent_info_span(data: bytes) -> tuple[int, int]:
    """Return the byte range of the top-level info dictionary in canonical bencode."""
    budget = [0]

    def parse_string(offset: int) -> tuple[bytes, int]:
        colon = data.find(b":", offset)
        if colon < 0:
            raise ClientControlError("torrent metainfo contains an invalid byte string")
        digits = data[offset:colon]
        if not digits or not digits.isdigit() or (len(digits) > 1 and digits.startswith(b"0")):
            raise ClientControlError("torrent metainfo contains a non-canonical byte-string length")
        end = colon + 1 + int(digits)
        if end > len(data):
            raise ClientControlError("torrent metainfo contains a truncated byte string")
        return data[colon + 1:end], end

    def value_end(offset: int, depth: int = 0) -> int:
        if depth > 64 or offset >= len(data):
            raise ClientControlError("torrent metainfo nesting or value boundary is invalid")
        budget[0] += 1
        if budget[0] > 1_000_000:
            raise ClientControlError("torrent metainfo contains too many bencoded values")
        token = data[offset:offset + 1]
        if token == b"i":
            end = data.find(b"e", offset + 1)
            if end < 0:
                raise ClientControlError("torrent metainfo contains an unterminated integer")
            number = data[offset + 1:end]
            if not number or re.fullmatch(rb"-?(?:0|[1-9][0-9]*)", number) is None or number == b"-0":
                raise ClientControlError("torrent metainfo contains a non-canonical integer")
            return end + 1
        if token in {b"l", b"d"}:
            dictionary = token == b"d"
            cursor = offset + 1
            previous_key: bytes | None = None
            while cursor < len(data) and data[cursor:cursor + 1] != b"e":
                if dictionary:
                    key, cursor = parse_string(cursor)
                    if previous_key is not None and key <= previous_key:
                        raise ClientControlError("torrent metainfo dictionary keys are not canonical")
                    previous_key = key
                cursor = value_end(cursor, depth + 1)
            if cursor >= len(data):
                raise ClientControlError("torrent metainfo contains an unterminated collection")
            return cursor + 1
        if token.isdigit():
            _value, end = parse_string(offset)
            return end
        raise ClientControlError("torrent metainfo contains an invalid bencode token")

    if not data.startswith(b"d"):
        raise ClientControlError("torrent metainfo top-level value must be a dictionary")
    cursor = 1
    previous_key: bytes | None = None
    info_span: tuple[int, int] | None = None
    while cursor < len(data) and data[cursor:cursor + 1] != b"e":
        key, cursor = parse_string(cursor)
        if previous_key is not None and key <= previous_key:
            raise ClientControlError("torrent metainfo dictionary keys are not canonical")
        previous_key = key
        value_start = cursor
        value_finish = value_end(value_start)
        if key == b"info":
            if data[value_start:value_start + 1] != b"d":
                raise ClientControlError("torrent metainfo info value must be a dictionary")
            info_span = (value_start, value_finish)
        cursor = value_finish
    if cursor != len(data) - 1 or data[cursor:cursor + 1] != b"e" or info_span is None:
        raise ClientControlError("torrent metainfo has trailing data or no info dictionary")
    return info_span


def _validate_metainfo_hash(data: bytes, info_hash: str) -> None:
    start, end = _torrent_info_span(data)
    digest = (
        hashlib.sha1(data[start:end], usedforsecurity=False).hexdigest()
        if len(info_hash) == 40
        else hashlib.sha256(data[start:end]).hexdigest()
    )
    if digest != info_hash:
        raise ClientControlError("torrent metainfo does not match the scheduled info hash")


def _torrent_bytes(path: Path) -> bytes:
    if not path.is_absolute() or ".." in path.parts or len(path.parts) < 3:
        raise ClientControlError("torrent metainfo path must be absolute and normalized")
    directory_fd: int | None = None
    descriptor: int | None = None
    try:
        directory_fd = os.open(
            path.anchor,
            os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
        )
        for component in path.parts[1:-1]:
            next_fd = os.open(
                component,
                os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW,
                dir_fd=directory_fd,
            )
            os.close(directory_fd)
            directory_fd = next_fd
        descriptor = os.open(
            path.parts[-1],
            os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW | os.O_NONBLOCK,
            dir_fd=directory_fd,
        )
    except (OSError, ValueError) as error:
        if descriptor is not None:
            os.close(descriptor)
        if directory_fd is not None:
            os.close(directory_fd)
        raise ClientControlError("cannot open torrent metainfo without following links") from error
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size > MAX_TORRENT_BYTES:
            raise ClientControlError("torrent metainfo must be a bounded regular file")
        chunks = bytearray()
        while len(chunks) <= MAX_TORRENT_BYTES:
            part = os.read(descriptor, min(64 * 1024, MAX_TORRENT_BYTES + 1 - len(chunks)))
            if not part:
                break
            chunks.extend(part)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
        os.close(directory_fd)
    identity = lambda value: (
        value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns
    )
    if len(chunks) > MAX_TORRENT_BYTES or len(chunks) != before.st_size or identity(before) != identity(after):
        raise ClientControlError("torrent metainfo changed while it was read")
    return bytes(chunks)


def _row_by_hash(rows: Any, info_hash: str, field_names: tuple[str, ...]) -> dict[str, Any] | None:
    if not isinstance(rows, list):
        raise ClientControlError("client status endpoint did not return a list")
    for item in rows:
        if not isinstance(item, dict):
            continue
        for name in field_names:
            value = item.get(name)
            if isinstance(value, str) and value.lower() == info_hash.lower():
                return item
    return None


class ClientController:
    """Operate on one already-supervised client without crossing process identity."""

    def __init__(
        self,
        client_id: str,
        profile: dict[str, Any],
        observation: dict[str, Any] | None = None,
    ) -> None:
        if client_id == "urtorrent":
            raise ClientControlError("urtorrent transfers are owned by urtorrent_runner")
        if client_id not in {
            "torrentngd", "qbittorrent", "transmission", "deluge", "rtorrent", "rqbit"
        }:
            raise ClientControlError(f"unsupported client ID {client_id!r}")
        self.client_id = client_id
        self.profile = profile
        self.observation = observation or probe_client(client_id, profile)
        if self.observation.get("client_id") != client_id:
            raise ClientControlError("client observation belongs to a different adapter")
        self.runtime_identity = self.observation.get("runtime_identity")
        if not isinstance(self.runtime_identity, dict):
            raise ClientControlError("client observation has no runtime identity")

        if client_id == "rtorrent":
            socket_key = "container_socket_path" if profile.get("runtime") == "container" else "socket_path"
            self.socket_path = Path(str(profile.get(socket_key, ""))).expanduser()
            self.http: HttpJson | None = None
        else:
            self.socket_path = None
            self.http = _profile_http(
                profile,
                allow_basic_auth=client_id in {"transmission", "deluge", "rqbit"},
                runtime_identity=self.runtime_identity,
            )
            if client_id == "torrentngd":
                token = _env_value(profile, "token_env")
                if token:
                    self.http.headers["Authorization"] = f"Bearer {token}"
            elif client_id == "qbittorrent":
                _login_qbittorrent(self.http, profile)
            elif client_id == "deluge":
                _connect_deluge(self.http, profile)

    def _assert_identity(self, *, force: bool = False) -> None:
        now = time.monotonic()
        last_check = getattr(self, "_last_identity_check", 0.0)
        if not force and now - last_check < 1.0:
            return
        try:
            current = probe_client(self.client_id, self.profile)
        except Exception as error:
            raise ClientControlError(f"pinned client identity/settings readback failed: {error}") from error
        if (
            current.get("version") != self.observation.get("version")
            or current.get("runtime_identity") != self.runtime_identity
            or current.get("settings_readback") != self.observation.get("settings_readback")
        ):
            raise ClientControlError("pinned client identity or settings changed during the trial")
        self._last_identity_check = now

    def _guarded(self, operation: Callable[[], Any]) -> Any:
        self._assert_identity(force=True)
        result = operation()
        self._assert_identity(force=True)
        return result

    def add_paused(self, torrent_path: Path, save_path: str, info_hash: str) -> None:
        """Add exactly one private fixture and leave it stopped before the barrier."""
        info_hash = _canonical_info_hash(info_hash)
        save_path = _normalize_save_path(save_path)
        if TORRENT_FILENAME_RE.fullmatch(torrent_path.name) is None:
            raise ClientControlError("torrent metainfo filename must be a safe single component")
        metainfo = _torrent_bytes(torrent_path)
        _validate_metainfo_hash(metainfo, info_hash)
        self._assert_identity(force=True)
        current = self.status(info_hash)
        if current is not None:
            raise ClientControlError("fixture info hash already exists; refusing to alter it")
        try:
            if self.client_id in {"torrentngd", "qbittorrent"}:
                prefix = "/api/qb/v2" if self.client_id == "torrentngd" else "/api/v2"
                body, content_type = _multipart([
                    ("torrents", torrent_path.name, metainfo, "application/x-bittorrent"),
                    ("savepath", None, save_path.encode(), "text/plain"),
                    ("paused", None, b"true", "text/plain"),
                    ("autoTMM", None, b"false", "text/plain"),
                ])
                response = self.http.request(
                    f"{prefix}/torrents/add", method="POST", raw_body=body,
                    content_type=content_type,
                ).decode("utf-8", "replace").strip()
                if response.lower() not in {"ok", "ok."}:
                    raise ClientControlError(f"client rejected fixture add: {response[:100]!r}")
            elif self.client_id == "transmission":
                response = _transmission_call(self.http, "torrent-add", {
                    "metainfo": base64.b64encode(metainfo).decode("ascii"),
                    "download-dir": save_path,
                    "paused": True,
                })
                added = response.get("torrent-added") if isinstance(response, dict) else None
                if not isinstance(added, dict) or str(added.get("hashString", "")).lower() != info_hash.lower():
                    raise ClientControlError("Transmission returned a different fixture identity")
            elif self.client_id == "deluge":
                added = _deluge_call(self.http, "core.add_torrent_file", [
                    torrent_path.name,
                    base64.b64encode(metainfo).decode("ascii"),
                    {"download_location": save_path, "add_paused": True, "auto_managed": False},
                ], 110)
                if not isinstance(added, str) or added.lower() != info_hash.lower():
                    raise ClientControlError("Deluge returned a different fixture identity")
            elif self.client_id == "rtorrent":
                rows = self._rtorrent_rows()
                if any(str(row[0]).lower() == info_hash.lower() for row in rows):
                    raise ClientControlError("rTorrent fixture info hash already exists")
                self._guarded(lambda: _scgi_call(
                    self.socket_path, "load.raw", [xmlrpc.client.Binary(metainfo)]
                ))
                self._wait(lambda: self.status(info_hash) is not None, "rTorrent fixture admission")
                self._guarded(lambda: _scgi_call(
                    self.socket_path, "d.directory_base.set", [info_hash, save_path]
                ))
                self.pause(info_hash)
            elif self.client_id == "rqbit":
                query = urllib.parse.urlencode({"output_folder": save_path, "overwrite": "false"})
                response = self.http.request(
                    f"/torrents?{query}", method="POST", raw_body=metainfo,
                    content_type="application/x-bittorrent",
                )
                try:
                    added = strict_json_loads(response, label="rqbit add response")
                except (UnicodeDecodeError, json.JSONDecodeError, StrictJsonError) as error:
                    raise ClientControlError("rqbit add returned invalid JSON") from error
                details = added.get("details") if isinstance(added, dict) else None
                observed_hash = details.get("info_hash") if isinstance(details, dict) else None
                if not isinstance(observed_hash, str) or observed_hash.lower() != info_hash.lower():
                    raise ClientControlError("rqbit returned a different fixture identity")
                self.pause(info_hash)  # rqbit has no pause-on-add option.
            self._wait(lambda: self.status(info_hash) is not None, f"{self.client_id} fixture admission")
            state = self.status(info_hash)
            if state is None or not _paused_state(state.state):
                raise ClientControlError(f"{self.client_id} fixture was not paused before the start barrier")
        except Exception as error:
            # Reconcile uncertain API responses by the unique info hash. If the
            # add landed, the caller owns cleanup and must not retry blindly.
            try:
                present = self.status(info_hash) is not None
            except Exception:
                present = None
            qualifier = "; fixture may have been admitted" if present else ""
            if isinstance(error, ClientControlError):
                raise ClientControlError(f"{error}{qualifier}") from error
            raise ClientControlError(f"{self.client_id} fixture add failed{qualifier}: {error}") from error
        self._assert_identity(force=True)

    def _wait(self, predicate: Callable[[], bool], label: str, timeout_seconds: float = 20.0) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(POLL_INTERVAL_SECONDS)
        raise ClientControlError(f"timed out waiting for {label}")

    def _rtorrent_rows(self) -> list[list[Any]]:
        result = _scgi_call(self.socket_path, "d.multicall2", [
            "", "d.hash=", "d.name=", "d.is_active=", "d.size_bytes=",
            "d.completed_bytes=", "d.down.rate=", "d.up.rate=", "d.up.total=",
            "d.peers_connected=",
        ])
        if not isinstance(result, list):
            raise ClientControlError("rTorrent multicall did not return a list")
        return [row for row in result if isinstance(row, list) and len(row) >= 9]

    def status(self, info_hash: str) -> TorrentState | None:
        """Read a single torrent by exact hash; never fall back to its display name."""
        info_hash = _canonical_info_hash(info_hash)
        self._assert_identity()
        try:
            if self.client_id in {"torrentngd", "qbittorrent"}:
                prefix = "/api/qb/v2" if self.client_id == "torrentngd" else "/api/v2"
                rows = self.http.json(f"{prefix}/torrents/info?hashes={urllib.parse.quote(info_hash)}")
                row = _row_by_hash(rows, info_hash, ("hash",))
                if row is None:
                    return None
                size = _number(row, ("size",), "total size")
                return TorrentState(
                    info_hash, str(row.get("state", "unknown")),
                    _progress(row, ("progress",), "progress"), size,
                    _number(row, ("downloaded", "downloaded_session", "completed"), "received bytes"),
                    _number(row, ("uploaded", "uploaded_session"), "uploaded bytes"),
                    _number(row, ("dlspeed",), "download rate"),
                    _number(row, ("upspeed",), "upload rate"),
                    _number(row, ("num_seeds",), "seed count")
                    + _number(row, ("num_leechs",), "leecher count"),
                )
            if self.client_id == "transmission":
                response = _transmission_call(self.http, "torrent-get", {
                    "fields": ["hashString", "status", "percentDone", "totalSize", "downloadedEver", "uploadedEver", "rateDownload", "rateUpload", "peersConnected"],
                    "ids": [info_hash],
                })
                rows = response.get("torrents") if isinstance(response, dict) else None
                row = _row_by_hash(rows, info_hash, ("hashString", "hash_string"))
                if row is None:
                    return None
                status = _number(row, ("status",), "status")
                labels = {0: "stopped", 1: "check-wait", 2: "checking", 3: "download-wait", 4: "downloading", 5: "seed-wait", 6: "seeding"}
                return TorrentState(
                    info_hash, labels.get(status, "unknown"),
                    _progress(row, ("percentDone", "percent_done"), "progress"),
                    _number(row, ("totalSize", "total_size"), "total size"),
                    _number(row, ("downloadedEver", "downloaded_ever"), "received bytes"),
                    _number(row, ("uploadedEver", "uploaded_ever"), "uploaded bytes"),
                    _number(row, ("rateDownload", "rate_download"), "download rate"),
                    _number(row, ("rateUpload", "rate_upload"), "upload rate"),
                    _number(row, ("peersConnected", "peers_connected"), "peer count"),
                )
            if self.client_id == "deluge":
                fields = ["state", "progress", "total_size", "total_done", "total_uploaded", "download_payload_rate", "upload_payload_rate", "num_peers"]
                rows = _deluge_call(self.http, "core.get_torrents_status", [{"id": [info_hash]}, fields], 111)
                if not isinstance(rows, dict):
                    raise ClientControlError("Deluge status endpoint returned a non-object")
                row = next((value for key, value in rows.items() if str(key).lower() == info_hash.lower()), None)
                if row is None:
                    return None
                return TorrentState(
                    info_hash, str(row.get("state", "unknown")).lower(),
                    _deluge_progress(row),
                    _number(row, ("total_size",), "total size"),
                    _number(row, ("total_done",), "received bytes"),
                    _number(row, ("total_uploaded",), "uploaded bytes"),
                    _number(row, ("download_payload_rate",), "download rate"),
                    _number(row, ("upload_payload_rate",), "upload rate"),
                    _number(row, ("num_peers",), "peer count"),
                )
            if self.client_id == "rtorrent":
                row = next((entry for entry in self._rtorrent_rows() if str(entry[0]).lower() == info_hash.lower()), None)
                if row is None:
                    return None
                total = _strict_int(int(row[3]), "rTorrent total size")
                done = _strict_int(int(row[4]), "rTorrent completed bytes")
                active = _rtorrent_active(row[2])
                return TorrentState(
                    info_hash, "seeding" if done >= total and active else "downloading" if active else "stopped",
                    1.0 if total == 0 else min(1.0, done / total), total, done,
                    _rtorrent_int(row[7], "uploaded bytes"),
                    _rtorrent_int(row[5], "download rate"),
                    _rtorrent_int(row[6], "upload rate"),
                    _rtorrent_int(row[8], "peer count"),
                )
            if self.client_id == "rqbit":
                quoted = urllib.parse.quote(info_hash, safe="")
                try:
                    details = self.http.json(f"/torrents/{quoted}")
                except AdapterError as error:
                    if "HTTP 404" in str(error):
                        return None
                    raise
                if not isinstance(details, dict):
                    raise ClientControlError("rqbit torrent details endpoint returned a non-object")
                state = "paused" if details.get("paused") is True else details.get(
                    "state", details.get("status", "unknown")
                )
                if not isinstance(state, str):
                    raise ClientControlError("rqbit torrent details omitted a valid state")
                try:
                    row = self.http.json(f"/torrents/{quoted}/stats/v1")
                except AdapterError as error:
                    if "HTTP 404" in str(error):
                        return None
                    raise
                if not isinstance(row, dict):
                    raise ClientControlError("rqbit torrent endpoint returned a non-object")
                received = _number(row, ("progress_bytes",), "verified progress bytes")
                total = _number(row, ("total_bytes",), "total size")
                progress = 1.0 if total == 0 and row.get("finished") is True else (
                    min(1.0, received / total) if total > 0 else 0.0
                )
                live = row.get("live")
                if live is not None and not isinstance(live, dict):
                    raise ClientControlError("rqbit live stats must be an object or null")
                download_rate = upload_rate = 0
                if isinstance(live, dict):
                    down = live.get("download_speed")
                    up = live.get("upload_speed")
                    if not isinstance(down, dict) or not isinstance(up, dict):
                        raise ClientControlError("rqbit live stats omitted speed snapshots")
                    download_rate = _mib_rate(down.get("mbps"), "download speed")
                    upload_rate = _mib_rate(up.get("mbps"), "upload speed")
                return TorrentState(
                    info_hash, state.lower(),
                    progress, total, received,
                    _number(row, ("uploaded_bytes",), "uploaded bytes"),
                    download_rate, upload_rate, None,
                )
        except ClientControlError:
            raise
        except Exception as error:
            raise ClientControlError(f"{self.client_id} status read failed: {error}") from error
        raise ClientControlError(f"{self.client_id} has no status adapter")

    def resume(self, info_hash: str) -> None:
        info_hash = _canonical_info_hash(info_hash)
        self._guarded(lambda: self._resume_unchecked(info_hash))

    def _resume_unchecked(self, info_hash: str) -> None:
        if self.client_id in {"torrentngd", "qbittorrent"}:
            prefix = "/api/qb/v2" if self.client_id == "torrentngd" else "/api/v2"
            self.http.request(f"{prefix}/torrents/start", method="POST", form={"hashes": info_hash})
        elif self.client_id == "transmission":
            _transmission_call(self.http, "torrent-start", {"ids": [info_hash]})
        elif self.client_id == "deluge":
            _deluge_call(self.http, "core.resume_torrent", [[info_hash]], 112)
        elif self.client_id == "rtorrent":
            _scgi_call(self.socket_path, "d.start", [info_hash])
        else:
            self.http.request(f"/torrents/{urllib.parse.quote(info_hash, safe='')}/start", method="POST")

    def pause(self, info_hash: str) -> None:
        info_hash = _canonical_info_hash(info_hash)
        self._guarded(lambda: self._pause_unchecked(info_hash))

    def _pause_unchecked(self, info_hash: str) -> None:
        if self.client_id in {"torrentngd", "qbittorrent"}:
            prefix = "/api/qb/v2" if self.client_id == "torrentngd" else "/api/v2"
            self.http.request(f"{prefix}/torrents/stop", method="POST", form={"hashes": info_hash})
        elif self.client_id == "transmission":
            _transmission_call(self.http, "torrent-stop", {"ids": [info_hash]})
        elif self.client_id == "deluge":
            _deluge_call(self.http, "core.pause_torrent", [[info_hash]], 113)
        elif self.client_id == "rtorrent":
            _scgi_call(self.socket_path, "d.stop", [info_hash])
        else:
            self.http.request(f"/torrents/{urllib.parse.quote(info_hash, safe='')}/pause", method="POST")

    def remove(self, info_hash: str, *, delete_data: bool) -> None:
        info_hash = _canonical_info_hash(info_hash)
        if not isinstance(delete_data, bool):
            raise ClientControlError("delete_data must be an explicit boolean")
        self._guarded(lambda: self._remove_unchecked(info_hash, delete_data))

    def _remove_unchecked(self, info_hash: str, delete_data: bool) -> None:
        if self.client_id == "rtorrent" and delete_data:
            raise ClientControlError(
                "rTorrent cannot delete payload data through this control API; remove the torrent with delete_data=False and clean its task-owned directory separately"
            )
        if self.client_id in {"torrentngd", "qbittorrent"}:
            prefix = "/api/qb/v2" if self.client_id == "torrentngd" else "/api/v2"
            self.http.request(
                f"{prefix}/torrents/delete", method="POST",
                form={"hashes": info_hash, "deleteFiles": "true" if delete_data else "false"},
            )
        elif self.client_id == "transmission":
            _transmission_call(self.http, "torrent-remove", {
                "ids": [info_hash], "delete-local-data": delete_data,
            })
        elif self.client_id == "deluge":
            result = _deluge_call(self.http, "core.remove_torrent", [info_hash, delete_data], 114)
            if result is not True:
                raise ClientControlError("Deluge refused to remove the trial torrent")
        elif self.client_id == "rtorrent":
            _scgi_call(self.socket_path, "d.erase", [info_hash])
        else:
            action = "delete" if delete_data else "forget"
            self.http.request(
                f"/torrents/{urllib.parse.quote(info_hash, safe='')}/{action}",
                method="POST",
            )

        self._wait(lambda: self.status(info_hash) is None, f"{self.client_id} trial torrent removal")
        self._assert_identity()
