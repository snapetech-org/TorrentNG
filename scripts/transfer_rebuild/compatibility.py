#!/usr/bin/env python3
"""Probe pinned client support for PERF-110 fixture protocols."""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import json
import secrets
import shutil
import stat
import subprocess
import sys
import time
import urllib.parse
import xmlrpc.client
from pathlib import Path
from typing import Any, Callable

from adapters import (
    AdapterError,
    CLIENT_IDS,
    HttpJson,
    _canonical_json,
    _deluge_call,
    _env_value,
    _login_qbittorrent,
    _profile_http,
    _scgi_call,
    _sha256_bytes,
    _sha256_regular_file_nofollow,
    _transmission_call,
    load_pins_document,
    probe_client,
    write_new_json,
)
from strict_json import StrictJsonError, loads as strict_json_loads
from fixtures import (
    FixtureFile,
    generate_fixture,
    load_fixture_profile_document,
)
from paired_plan import _read_regular_file
from validate_artifact import validate


PREFLIGHT_SCHEMA_VERSION = "transfer-compatibility-preflight-v1"
SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_PROFILE_PATH = SCRIPT_DIR / "fixture_profiles.json"
POLL_INTERVAL_SECONDS = 0.1
POLL_TIMEOUT_SECONDS = 10.0
KNOWN_URTORRENT_UNSUPPORTED = "the pinned urtorrent runner supports private v1 metainfo only"


class PreflightError(ValueError):
    """A pinned-client compatibility probe could not complete safely."""


class PreflightCleanupError(PreflightError):
    """A probe may have left disposable state that could not be removed."""


def _strict_json(raw: bytes, label: str) -> Any:
    try:
        return strict_json_loads(raw, label=label)
    except StrictJsonError as error:
        raise PreflightError(str(error)) from error


def _probe_files(profile: dict[str, Any]) -> list[FixtureFile]:
    """Keep each profile's metainfo edge case while capping probe payload size."""
    original = profile["files"]
    piece_length = int(profile["piece_length"])
    kind = str(profile["torrent_kind"])
    nonempty_seen = False
    secondary_hybrid_file_seen = False
    result: list[FixtureFile] = []
    for item in original:
        if not isinstance(item, FixtureFile):
            raise PreflightError("fixture profile contains an invalid file record")
        if item.size_bytes == 0:
            result.append(FixtureFile(item.name, 0))
        elif not nonempty_seen:
            if kind == "v2":
                size = piece_length + 16 * 1024 + 1
            else:
                size = piece_length + 1
            result.append(FixtureFile(item.name, size))
            nonempty_seen = True
        elif kind == "hybrid" and not secondary_hybrid_file_seen:
            result.append(FixtureFile(item.name, 16 * 1024 + 1))
            secondary_hybrid_file_seen = True
        elif kind == "hybrid":
            result.append(FixtureFile(item.name, 0))
        else:
            result.append(FixtureFile(item.name, 16 * 1024 + 1))
    if not nonempty_seen:
        raise PreflightError("fixture profile has no non-empty payload file")
    if _probe_payload_bytes(result, kind, piece_length) >= 3 * 1024 * 1024:
        raise PreflightError("profile-shaped compatibility probe must remain below 3 MiB")
    return result


def _probe_payload_bytes(files: list[FixtureFile], kind: str, piece_length: int) -> int:
    total = 0
    has_payload = False
    for item in files:
        if kind == "hybrid" and item.size_bytes > 0 and has_payload:
            total += (-total) % piece_length
        total += item.size_bytes
        has_payload = has_payload or item.size_bytes > 0
    return total


def _profile_save_path(profile: dict[str, Any], run_id: str, case_id: str) -> str:
    base = profile.get("preflight_save_path")
    if not isinstance(base, str) or not base:
        raise PreflightError("pin profile must declare preflight_save_path as a disposable client-side directory")
    base_path = Path(base)
    if not base_path.is_absolute() or ".." in base_path.parts:
        raise PreflightError("preflight_save_path must be absolute and may not contain '..'")
    suffix = re_safe_component(case_id)
    return str(base_path / f"tng-preflight-{run_id}-{suffix}")


def re_safe_component(value: str) -> str:
    return "".join(character if character.isalnum() or character in "-_" else "-" for character in value)


def _multipart(fields: list[tuple[str, str | None, bytes, str]]) -> tuple[bytes, str]:
    boundary = "----TorrentNGPreflight" + secrets.token_hex(16)
    chunks: list[bytes] = []
    for name, filename, value, media_type in fields:
        disposition = f'Content-Disposition: form-data; name="{name}"'
        if filename is not None:
            disposition += f'; filename="{filename}"'
        chunks.append(
            f"--{boundary}\r\n{disposition}\r\nContent-Type: {media_type}\r\n\r\n".encode("ascii")
        )
        chunks.extend((value, b"\r\n"))
    chunks.append(f"--{boundary}--\r\n".encode("ascii"))
    return b"".join(chunks), f"multipart/form-data; boundary={boundary}"


def _poll(read: Callable[[], Any], ready: Callable[[Any], bool], label: str) -> Any:
    deadline = time.monotonic() + POLL_TIMEOUT_SECONDS
    last: Any = None
    while time.monotonic() < deadline:
        last = read()
        if ready(last):
            return last
        time.sleep(POLL_INTERVAL_SECONDS)
    raise PreflightError(f"timed out waiting for {label}")


def _run_mutation(
    add: Callable[[], Any],
    verify_paused: Callable[[Any], bool],
    remove: Callable[[Any], None],
    recover: Callable[[], Any | None],
) -> dict[str, Any]:
    handle: Any = None
    added: bool | None = False
    paused_verified = False
    removed = False
    error_message: str | None = None
    try:
        handle = add()
        added = True
    except Exception as error:  # client APIs expose several unrelated exception types
        error_message = str(error)[:500]
        try:
            handle = recover()
        except Exception as recovery_error:
            added = None
            recovery_message = f"cannot establish whether add succeeded: {recovery_error}"
            error_message = f"{error_message}; {recovery_message}"[:500]
        else:
            added = handle is not None
            if handle is None:
                # The exact generated info hash is confirmed absent.
                removed = True
    if handle is not None:
        try:
            paused_verified = verify_paused(handle)
            if not paused_verified:
                message = "client did not report the probe torrent paused"
                error_message = f"{error_message}; {message}" if error_message else message
        except Exception as error:
            message = str(error)[:500]
            error_message = f"{error_message}; {message}" if error_message else message
        try:
            remove(handle)
            removed = True
        except Exception as error:
            cleanup_message = str(error)[:500]
            error_message = f"{error_message}; cleanup failed: {cleanup_message}" if error_message else cleanup_message
            try:
                remaining = recover()
            except Exception as recovery_error:
                recovery_message = f"cannot verify probe removal: {recovery_error}"
                error_message = f"{error_message}; {recovery_message}"[:500]
            else:
                if remaining is None:
                    removed = True
                else:
                    try:
                        remove(remaining)
                        removed = True
                    except Exception as retry_error:
                        retry_message = f"cleanup retry failed: {retry_error}"
                        error_message = f"{error_message}; {retry_message}"[:500]
    status = (
        "passed"
        if added is True and paused_verified and removed and error_message is None
        else "cleanup_failed"
        if not removed and added in (True, None)
        else "failed"
    )
    return {
        "status": status,
        "added": added,
        "paused_verified": paused_verified,
        "removed": removed,
        "reason": error_message,
    }


def _qbit_like_preflight(
    client: HttpJson,
    api_prefix: str,
    metainfo: bytes,
    torrent_name: str,
    save_path: str,
    info_hash: str,
) -> dict[str, Any]:
    info_path = f"{api_prefix}/torrents/info?{urllib.parse.urlencode({'hashes': info_hash})}"

    def row() -> dict[str, Any] | None:
        values = client.json(info_path)
        if not isinstance(values, list):
            raise PreflightError("qBittorrent-compatible info endpoint did not return a list")
        return next(
            (
                item for item in values
                if isinstance(item, dict)
                and str(item.get("hash", "")).lower() == info_hash.lower()
            ),
            None,
        )

    if row() is not None:
        raise PreflightError("probe info hash already exists; refusing to alter a pre-existing torrent")

    def add() -> str:
        body, content_type = _multipart(
            [
                ("torrents", f"{torrent_name}.torrent", metainfo, "application/x-bittorrent"),
                ("savepath", None, save_path.encode(), "text/plain"),
                ("paused", None, b"true", "text/plain"),
                ("autoTMM", None, b"false", "text/plain"),
            ]
        )
        response = client.request(
            f"{api_prefix}/torrents/add",
            method="POST",
            raw_body=body,
            content_type=content_type,
        ).decode("utf-8", "replace").strip()
        if response.lower() not in {"ok", "ok."}:
            raise PreflightError(f"qBittorrent-compatible add returned {response[:100]!r}")
        return info_hash

    def verify(_handle: str) -> bool:
        current = _poll(
            row,
            lambda value: value is not None
            and str(value.get("state", "")).lower().startswith(("paused", "stopped")),
            "probe torrent to be added paused",
        )
        return current is not None

    def remove(_handle: str) -> None:
        client.request(
            f"{api_prefix}/torrents/delete",
            method="POST",
            form={"hashes": info_hash, "deleteFiles": "true"},
        )
        _poll(row, lambda value: value is None, "probe torrent removal")

    return _run_mutation(
        add,
        verify,
        remove,
        lambda: info_hash if row() is not None else None,
    )


def _transmission_preflight(
    client: HttpJson,
    metainfo: bytes,
    save_path: str,
    info_hash: str,
) -> dict[str, Any]:
    def find(handle: str) -> dict[str, Any] | None:
        response = _transmission_call(
            client,
            "torrent-get",
            {"fields": ["hashString", "name", "status"], "ids": [handle]},
        )
        torrents = response.get("torrents")
        if not isinstance(torrents, list):
            raise PreflightError("Transmission torrent-get did not return a torrent list")
        return next(
            (
                item for item in torrents
                if isinstance(item, dict)
                and str(item.get("hashString", "")).lower() == handle.lower()
            ),
            None,
        )

    if find(info_hash) is not None:
        raise PreflightError("probe info hash already exists; refusing to alter a pre-existing torrent")

    def add() -> str:
        response = _transmission_call(
            client,
            "torrent-add",
            {
                "metainfo": base64.b64encode(metainfo).decode("ascii"),
                "download-dir": save_path,
                "paused": True,
            },
        )
        if "torrent-duplicate" in response:
            raise PreflightError("Transmission reported a duplicate probe torrent")
        result = response.get("torrent-added")
        if not isinstance(result, dict) or not isinstance(result.get("hashString"), str):
            raise PreflightError("Transmission did not return the newly added torrent identity")
        return result["hashString"]

    def verify(handle: str) -> bool:
        if handle.lower() != info_hash.lower():
            raise PreflightError("Transmission added a torrent with an unexpected info hash")
        result = _poll(
            lambda: find(handle),
            lambda value: value is not None and value.get("status") == 0,
            "Transmission probe torrent to be added stopped",
        )
        return result is not None

    def remove(handle: str) -> None:
        _transmission_call(
            client,
            "torrent-remove",
            {"ids": [handle], "delete-local-data": True},
        )
        _poll(lambda: find(handle), lambda value: value is None, "Transmission probe torrent removal")

    return _run_mutation(add, verify, remove, lambda: info_hash if find(info_hash) is not None else None)


def _connect_deluge(client: HttpJson, profile: dict[str, Any]) -> None:
    password = _env_value(profile, "password_env", required=True)
    if _deluge_call(client, "auth.login", [password], 1) is not True:
        raise PreflightError("Deluge Web API login failed")
    if not _deluge_call(client, "web.connected", [], 2):
        hosts = _deluge_call(client, "web.get_hosts", [], 3) or []
        host_id = hosts[0][0] if hosts else None
        if not host_id:
            raise PreflightError("Deluge WebUI has no configured daemon host")
        _deluge_call(client, "web.connect", [host_id], 4)
        if not _deluge_call(client, "web.connected", [], 5):
            raise PreflightError("Deluge WebUI could not connect to its daemon")


def _deluge_preflight(
    client: HttpJson,
    metainfo: bytes,
    torrent_name: str,
    save_path: str,
    info_hash: str,
) -> dict[str, Any]:
    def state(handle: str) -> str | None:
        result = _deluge_call(
            client,
            "core.get_torrents_status",
            [{"id": [handle]}, ["state"]],
            11,
        )
        if not isinstance(result, dict):
            raise PreflightError("Deluge get_torrents_status did not return an object")
        for torrent_id, status in result.items():
            if str(torrent_id).lower() == handle.lower() and isinstance(status, dict):
                value = status.get("state")
                return value if isinstance(value, str) else None
        return None

    if state(info_hash) is not None:
        raise PreflightError("probe info hash already exists; refusing to alter a pre-existing torrent")

    def add() -> str:
        result = _deluge_call(
            client,
            "core.add_torrent_file",
            [
                f"{torrent_name}.torrent",
                base64.b64encode(metainfo).decode("ascii"),
                {
                    "download_location": save_path,
                    "add_paused": True,
                    "auto_managed": False,
                },
            ],
            10,
        )
        if not isinstance(result, str) or not result:
            raise PreflightError("Deluge did not return the newly added torrent hash")
        return result

    def verify(handle: str) -> bool:
        if handle.lower() != info_hash.lower():
            raise PreflightError("Deluge added a torrent with an unexpected info hash")
        return _poll(
            lambda: state(handle),
            lambda value: value == "Paused",
            "Deluge probe torrent to be added paused",
        ) == "Paused"

    def remove(handle: str) -> None:
        if _deluge_call(client, "core.remove_torrent", [handle, True], 13) is not True:
            raise PreflightError("Deluge refused to remove the probe torrent")
        _poll(lambda: state(handle), lambda value: value is None, "Deluge probe torrent removal")

    return _run_mutation(add, verify, remove, lambda: info_hash if state(info_hash) is not None else None)


def _rtorrent_rows(socket_path: Path) -> list[list[Any]]:
    result = _scgi_call(socket_path, "d.multicall2", "", "d.hash=", "d.name=", "d.is_active=")
    if not isinstance(result, list):
        raise PreflightError("rTorrent d.multicall2 did not return a list")
    return [row for row in result if isinstance(row, list) and len(row) >= 3]


def _rtorrent_is_inactive(row: list[Any] | None) -> bool:
    if row is None:
        return False
    active = row[2]
    if active in (False, 0, "0"):
        return True
    if active in (True, 1, "1"):
        return False
    raise PreflightError("rTorrent d.is_active returned an invalid status")


def _rtorrent_preflight(
    profile: dict[str, Any],
    metainfo: bytes,
    save_path: str,
    info_hash: str,
) -> dict[str, Any]:
    socket_path = Path(str(profile.get("socket_path", ""))).expanduser()
    before = _rtorrent_rows(socket_path)
    if any(str(row[0]).lower() == info_hash.lower() for row in before):
        raise PreflightError("probe info hash already exists; refusing to alter a pre-existing torrent")

    def add() -> str:
        _scgi_call(socket_path, "load.raw", xmlrpc.client.Binary(metainfo))
        _poll(row, lambda value: value is not None, "rTorrent probe torrent to appear")
        _scgi_call(socket_path, "d.directory_base.set", info_hash, save_path)
        return info_hash

    def row() -> list[Any] | None:
        return next((entry for entry in _rtorrent_rows(socket_path) if str(entry[0]).lower() == info_hash.lower()), None)

    def verify(_handle: str) -> bool:
        value = _poll(row, _rtorrent_is_inactive, "rTorrent probe torrent to be loaded stopped")
        return _rtorrent_is_inactive(value)

    def remove(handle: str) -> None:
        _scgi_call(socket_path, "d.erase", handle)
        _poll(row, lambda value: value is None, "rTorrent probe torrent removal")

    return _run_mutation(add, verify, remove, lambda: info_hash if row() is not None else None)


def _rqbit_paused(document: Any) -> bool:
    if not isinstance(document, dict):
        return False
    if document.get("paused") is True:
        return True
    for key in ("state", "status"):
        value = document.get(key)
        if isinstance(value, str) and value.lower() in {"paused", "stopped"}:
            return True
    return False


def _rqbit_preflight(
    client: HttpJson,
    metainfo: bytes,
    save_path: str,
    info_hash: str,
) -> dict[str, Any]:
    quoted_hash = urllib.parse.quote(info_hash, safe="")
    detail_path = f"/torrents/{quoted_hash}"

    def read() -> Any:
        return client.json(detail_path)

    def present() -> bool:
        try:
            value = read()
        except AdapterError as error:
            if "HTTP 404" in str(error):
                return False
            raise
        return isinstance(value, dict)

    if present():
        raise PreflightError("probe info hash already exists; refusing to alter a pre-existing torrent")

    def add() -> str:
        query = urllib.parse.urlencode({"output_folder": save_path, "overwrite": "false"})
        client.request(
            f"/torrents?{query}",
            method="POST",
            raw_body=metainfo,
            content_type="application/x-bittorrent",
        )
        return info_hash

    def verify(_handle: str) -> bool:
        _poll(read, lambda value: isinstance(value, dict), "rqbit probe torrent to appear")
        client.request(detail_path + "/pause", method="POST")
        result = _poll(read, _rqbit_paused, "rqbit probe torrent to enter a paused state")
        return _rqbit_paused(result)

    def remove(_handle: str) -> None:
        client.request(detail_path + "/delete", method="POST")
        deadline = time.monotonic() + POLL_TIMEOUT_SECONDS
        while time.monotonic() < deadline:
            try:
                read()
            except AdapterError as error:
                if "HTTP 404" in str(error):
                    return
                raise
            time.sleep(POLL_INTERVAL_SECONDS)
        raise PreflightError("timed out waiting for rqbit probe torrent removal")

    return _run_mutation(add, verify, remove, lambda: info_hash if present() else None)


def _urtorrent_preflight(
    profile: dict[str, Any], torrent_path: Path, save_path: Path
) -> tuple[dict[str, Any], str, dict[str, Any]]:
    binary = Path(str(profile.get("binary_path", ""))).expanduser()
    expected_digest = profile.get("expected_binary_sha256")
    observed_digest, _ = _sha256_regular_file_nofollow(binary, "urtorrent runner binary")
    if observed_digest != expected_digest:
        raise PreflightError("urtorrent runner binary digest differs from its pin")
    parent = save_path.parent
    try:
        parent_metadata = parent.lstat()
    except OSError as error:
        raise PreflightError(f"urtorrent preflight parent is unavailable: {error}") from error
    if stat.S_ISLNK(parent_metadata.st_mode) or not stat.S_ISDIR(parent_metadata.st_mode):
        raise PreflightError("urtorrent preflight parent must be an existing non-symlink directory")

    primary_error: Exception | None = None
    document: dict[str, Any] | None = None
    save_path_created = False
    try:
        save_path.mkdir(mode=0o700, parents=False, exist_ok=False)
        save_path_created = True
        result = subprocess.run(
            [str(binary), "preflight", "--torrent", str(torrent_path), "--save-path", str(save_path)],
            check=False,
            capture_output=True,
            text=True,
            timeout=float(profile.get("preflight_timeout_seconds", 60)),
        )
        if result.returncode != 0:
            raise PreflightError(f"urtorrent runner preflight failed: {result.stderr[-500:].strip()}")
        response = _strict_json(result.stdout.encode("utf-8"), "urtorrent preflight response")
        if (
            not isinstance(response, dict)
            or response.get("format") != "transfer-urtorrent-preflight-v1"
            or response.get("version") != profile.get("expected_version")
            or response.get("added") is not True
            or response.get("paused") is not True
            or response.get("removed") is not True
        ):
            raise PreflightError("urtorrent runner returned an unexpected preflight response")
        settings = response.get("settings")
        session_settings = settings.get("session") if isinstance(settings, dict) else None
        runner_settings = settings.get("runner") if isinstance(settings, dict) else None
        probe_settings = settings.get("probe") if isinstance(settings, dict) else None
        if not all(isinstance(section, dict) for section in (session_settings, runner_settings, probe_settings)):
            raise PreflightError("urtorrent runner omitted its effective preflight settings")
        if (
            session_settings.get("listen_v4") != "127.0.0.1"
            or session_settings.get("listen_v6") is not None
            or isinstance(session_settings.get("listen_port"), bool)
            or not isinstance(session_settings.get("listen_port"), int)
            or not 1 <= session_settings["listen_port"] <= 65535
            or any(session_settings.get(key) is not False for key in ("dht", "pex", "lsd"))
            or session_settings.get("max_connections") != 1
            or session_settings.get("max_peers_per_torrent") != 1
            or runner_settings != {"hash_threads": 1, "dht_bootstrap_nodes": []}
            or probe_settings.get("paused") is not True
            or probe_settings.get("private") is not True
            or probe_settings.get("max_peers") != 1
            or probe_settings.get("tracker_urls") != ["http://127.0.0.1:9/announce"]
            or probe_settings.get("web_seed_urls") != []
        ):
            raise PreflightError("urtorrent effective preflight settings do not match the isolation contract")
        document = response
        if save_path.exists():
            raise PreflightError("urtorrent runner reported removal but left its disposable directory")
    except Exception as error:
        primary_error = error

    metadata = None
    if save_path_created:
        try:
            metadata = save_path.lstat()
        except FileNotFoundError:
            pass
        except OSError as error:
            detail = f"cannot inspect urtorrent disposable directory {save_path}: {error}"
            if primary_error is not None:
                detail = f"{primary_error}; cleanup failed: {detail}"
            raise PreflightCleanupError(detail) from error
    if metadata is not None:
        try:
            if stat.S_ISLNK(metadata.st_mode) or not stat.S_ISDIR(metadata.st_mode):
                raise OSError("preflight path was replaced by a non-directory or symbolic link")
            shutil.rmtree(save_path)
        except OSError as error:
            detail = f"cannot remove urtorrent disposable directory {save_path}: {error}"
            if primary_error is not None:
                detail = f"{primary_error}; cleanup failed: {detail}"
            raise PreflightCleanupError(detail) from error

    if primary_error is not None:
        raise primary_error
    assert document is not None
    return (
        {"runtime": "native_binary", "binary_sha256": observed_digest},
        document["version"],
        document["settings"],
    )


def _prepare_http_client(
    client_id: str,
    profile: dict[str, Any],
    observation: dict[str, Any],
) -> HttpJson:
    runtime_identity = observation["runtime_identity"]
    if client_id == "torrentngd":
        client = _profile_http(profile, runtime_identity=runtime_identity)
        token = _env_value(profile, "token_env")
        if token:
            client.headers["Authorization"] = f"Bearer {token}"
        return client
    if client_id == "qbittorrent":
        client = _profile_http(profile, runtime_identity=runtime_identity)
        _login_qbittorrent(client, profile)
        return client
    if client_id == "transmission":
        return _profile_http(profile, allow_basic_auth=True, runtime_identity=runtime_identity)
    if client_id in {"deluge", "rqbit"}:
        client = _profile_http(profile, allow_basic_auth=True, runtime_identity=runtime_identity)
        if client_id == "deluge":
            _connect_deluge(client, profile)
        return client
    raise PreflightError(f"{client_id} has no HTTP preflight adapter")


def _client_probe(
    client_id: str,
    profile: dict[str, Any],
    observation: dict[str, Any] | None,
    fixture_dir: Path,
    fixture_record: dict[str, Any],
    save_path: str,
) -> dict[str, Any]:
    record = {
        "status": "failed",
        "added": False,
        "paused_verified": False,
        "pause_mode": "after_add" if client_id == "rqbit" else "on_add",
        "removed": False,
        "reason": None,
    }
    torrent_path = fixture_dir / "fixture.torrent"
    metainfo = torrent_path.read_bytes()
    info_hashes = fixture_record["info_hashes"]
    kind = fixture_record["metainfo_kind"]
    info_hash = info_hashes["sha256"] if kind == "v2" else info_hashes["sha1"]

    try:
        if client_id == "urtorrent":
            try:
                identity, version, settings = _urtorrent_preflight(
                    profile,
                    torrent_path,
                    Path(save_path),
                )
            except OSError as error:
                raise PreflightError(f"cannot create or remove urtorrent preflight files: {error}") from error
            record.update({"status": "passed", "added": True, "paused_verified": True, "removed": True})
            record.update(
                {
                    "pause_mode": "on_add",
                    "observed_version": version,
                    "client_identity_sha256": _sha256_bytes(_canonical_json(identity)),
                    "settings_sha256": _sha256_bytes(_canonical_json(settings)),
                }
            )
            return record

        torrent_name = str(fixture_record["name"])
        if client_id == "rtorrent":
            result = _rtorrent_preflight(
                profile,
                metainfo,
                save_path,
                info_hash,
            )
        else:
            assert observation is not None
            client = _prepare_http_client(client_id, profile, observation)
            if client_id in {"torrentngd", "qbittorrent"}:
                result = _qbit_like_preflight(
                    client,
                    "/api/qb/v2" if client_id == "torrentngd" else "/api/v2",
                    metainfo,
                    torrent_name,
                    save_path,
                    info_hash,
                )
            elif client_id == "transmission":
                result = _transmission_preflight(client, metainfo, save_path, info_hash)
            elif client_id == "deluge":
                result = _deluge_preflight(client, metainfo, torrent_name, save_path, info_hash)
            elif client_id == "rqbit":
                result = _rqbit_preflight(client, metainfo, save_path, info_hash)
            else:
                raise PreflightError(f"unsupported client id {client_id!r}")
        record.update(result)
        record["observed_version"] = observation["version"]
        record["client_identity_sha256"] = _sha256_bytes(
            _canonical_json(observation["runtime_identity"])
        )
        record["settings_sha256"] = observation["settings_readback"]["sha256"]
    except Exception as error:
        record["status"] = "failed"
        record["reason"] = str(error)[:500]
    return record


def build_preflight(
    plan: dict[str, Any],
    plan_sha256: str,
    pins: dict[str, dict[str, Any]],
    pins_sha256: str,
    scratch_parent: Path,
    *,
    profile_path: Path = DEFAULT_PROFILE_PATH,
) -> dict[str, Any]:
    if not scratch_parent.is_absolute() or scratch_parent.is_symlink() or not scratch_parent.is_dir():
        raise PreflightError("scratch parent must be an existing absolute non-symlink directory")
    if plan.get("pins_sha256") != pins_sha256:
        raise PreflightError("pin file digest does not match the paired plan")
    if set(plan.get("pinned_client_ids", [])) != set(pins):
        raise PreflightError("pin client set does not match the paired plan")
    profiles_bytes, profiles_sha256 = _read_regular_file(profile_path, "fixture profile file")
    if profiles_sha256 != plan["catalog"]["profiles_sha256"]:
        raise PreflightError("fixture profile file digest does not match the paired plan")
    profiles_document = _strict_json(profiles_bytes, "fixture profile file")

    run_id = secrets.token_hex(8)
    scratch = Path(tempfile_name(scratch_parent, run_id))
    scratch.mkdir(mode=0o700)
    fixtures: dict[str, tuple[Path, dict[str, Any]]] = {}
    try:
        for profile_id in plan["catalog"]["profile_ids"]:
            profile = load_fixture_profile_document(profiles_document, profile_id)
            probe_specs = _probe_files(profile)
            fixture_name = f"tngpf-{run_id}-{re_safe_component(profile_id)[:24]}"
            fixture_dir = scratch / re_safe_component(profile_id)
            manifest = generate_fixture(
                fixture_dir,
                f"{plan['plan_id']}:{run_id}:{profile_id}",
                probe_specs,
                announce_url="http://127.0.0.1:9/announce",
                torrent_name=fixture_name,
                piece_length=int(profile["piece_length"]),
                torrent_kind=str(profile["torrent_kind"]),
                profile_id=profile_id,
            )
            record = _strict_json((fixture_dir / "torrent.json").read_bytes(), "generated torrent record")
            record.update(
                {
                    "fixture_sha256": manifest["fixture_sha256"],
                    "fixture_torrent_sha256": record["torrent_sha256"],
                }
            )
            fixtures[profile_id] = (fixture_dir, record)

        client_observations: dict[str, dict[str, Any] | None] = {}
        client_errors: dict[str, str] = {}
        for client_id in plan["pinned_client_ids"]:
            if client_id == "urtorrent":
                client_observations[client_id] = None
                continue
            try:
                client_observations[client_id] = probe_client(client_id, pins[client_id])
            except Exception as error:
                client_observations[client_id] = None
                client_errors[client_id] = str(error)[:500]

        results: list[dict[str, Any]] = []
        for case in plan["catalog"]["cases"]:
            client_id = case["client_id"]
            profile_id = case["profile_id"]
            fixture_dir, fixture_record = fixtures[profile_id]
            result: dict[str, Any] = {
                "case_id": case["case_id"],
                "client_id": client_id,
                "profile_id": profile_id,
                "protocol": case["protocol"],
                "applicability": case["applicability"],
                "status": "not_pinned",
                "fixture_sha256": fixture_record["fixture_sha256"],
                "torrent_sha256": fixture_record["fixture_torrent_sha256"],
                "info_hash": (
                    fixture_record["info_hashes"]["sha256"]
                    if case["protocol"] == "v2"
                    else fixture_record["info_hashes"]["sha1"]
                ),
                "observed_version": None,
                "client_identity_sha256": None,
                "settings_sha256": None,
                "added": None,
                "paused_verified": None,
                "pause_mode": None,
                "removed": None,
                "reason": None,
            }
            if case["applicability"] == "unsupported":
                result["status"] = "unsupported"
                result["reason"] = case["applicability_reason"]
            elif client_id not in pins:
                result["status"] = "not_pinned"
                result["reason"] = "client has no pin in the supplied pin file"
            elif client_id == "urtorrent" and case["protocol"] != "v1":
                result["status"] = "unsupported"
                result["reason"] = KNOWN_URTORRENT_UNSUPPORTED
            elif client_id in client_errors:
                result["status"] = "failed"
                result["reason"] = client_errors[client_id]
            else:
                profile_pin = pins[client_id]
                try:
                    save_path = _profile_save_path(profile_pin, run_id, case["case_id"])
                    if client_id == "urtorrent":
                        host_save = Path(save_path)
                        identity, version, settings = _urtorrent_preflight(
                            profile_pin,
                            fixture_dir / "fixture.torrent",
                            host_save,
                        )
                        result.update(
                            {
                                "status": "passed",
                                "observed_version": version,
                                "client_identity_sha256": _sha256_bytes(_canonical_json(identity)),
                                "settings_sha256": _sha256_bytes(_canonical_json(settings)),
                                "added": True,
                                "paused_verified": True,
                                "pause_mode": "on_add",
                                "removed": True,
                            }
                        )
                    else:
                        observation = client_observations[client_id]
                        if observation is None:
                            raise PreflightError("pinned client identity/settings observation is unavailable")
                        outcome = _client_probe(
                            client_id,
                            profile_pin,
                            observation,
                            fixture_dir,
                            fixture_record,
                            save_path,
                        )
                        result.update(outcome)
                        try:
                            after = probe_client(client_id, profile_pin)
                        except Exception as error:
                            changed_reason = f"client identity could not be re-read after preflight: {error}"
                        else:
                            before_identity = _sha256_bytes(
                                _canonical_json(observation["runtime_identity"])
                            )
                            after_identity = _sha256_bytes(
                                _canonical_json(after["runtime_identity"])
                            )
                            if (
                                after["version"] != observation["version"]
                                or after_identity != before_identity
                                or after["settings_readback"]["sha256"]
                                != observation["settings_readback"]["sha256"]
                            ):
                                changed_reason = "client identity or pinned settings changed during preflight"
                            else:
                                changed_reason = None
                        if changed_reason is not None:
                            result["reason"] = (
                                f"{result['reason']}; {changed_reason}"
                                if result["reason"]
                                else changed_reason
                            )
                            if result["status"] != "cleanup_failed":
                                result["status"] = "failed"
                except PreflightCleanupError as error:
                    result.update(
                        {
                            "status": "cleanup_failed",
                            "added": None,
                            "paused_verified": False,
                            "pause_mode": "on_add",
                            "removed": False,
                            "reason": str(error)[:500],
                        }
                    )
                except Exception as error:
                    result["status"] = "failed"
                    result["reason"] = str(error)[:500]
            if client_id in client_observations and client_observations[client_id] is not None:
                observation = client_observations[client_id]
                result["observed_version"] = observation["version"]
                result["client_identity_sha256"] = _sha256_bytes(
                    _canonical_json(observation["runtime_identity"])
                )
                result["settings_sha256"] = observation["settings_readback"]["sha256"]
            results.append(result)

        eligible = sorted(item["case_id"] for item in results if item["status"] == "passed")
        scheduled_pairs = []
        pair_exclusions = [
            {
                "pair_id": None,
                "profile_id": item["profile_id"],
                "repetition": item["repetition"],
                "reason": item["reason"],
            }
            for item in plan.get("excluded_pairs", [])
        ]
        for pair in _schedule_pairs(plan.get("schedule", [])):
            first = pair[0]
            case_ids = [f"{slot['client_id']}:{slot['profile_id']}" for slot in pair]
            scheduled_pairs.append(
                {
                    "pair_id": first["pair_id"],
                    "profile_id": first["profile_id"],
                    "repetition": first["repetition"],
                    "case_ids": case_ids,
                }
            )
            ready_clients = [slot["client_id"] for slot in pair if f"{slot['client_id']}:{slot['profile_id']}" in eligible]
            if len(ready_clients) < 2:
                pair_exclusions.append(
                    {
                        "pair_id": first["pair_id"],
                        "profile_id": first["profile_id"],
                        "repetition": first["repetition"],
                        "reason": "fewer than two scheduled clients passed compatibility preflight",
                    }
                )
        candidates = [
            item for item in results
            if item["applicability"] == "candidate" and item["client_id"] in pins
        ]
        ready = (
            bool(candidates)
            and bool(plan.get("schedule"))
            and all(item["status"] == "passed" for item in candidates)
            and not pair_exclusions
        )
        artifact: dict[str, Any] = {
            "schema_version": PREFLIGHT_SCHEMA_VERSION,
            "plan_id": plan["plan_id"],
            "plan_sha256": plan_sha256,
            "pins_sha256": pins_sha256,
            "catalog_sha256": plan["catalog_sha256"],
            "created_at": dt.datetime.now(dt.timezone.utc).isoformat(timespec="seconds"),
            "pinned_client_ids": list(plan["pinned_client_ids"]),
            "ready": ready,
            "eligible_case_ids": eligible,
            "scheduled_pairs": scheduled_pairs,
            "excluded_pairs": pair_exclusions,
            "results": results,
        }
        artifact["preflight_id"] = _sha256_bytes(_canonical_json(artifact))[:20]
        return artifact
    finally:
        shutil.rmtree(scratch)


def tempfile_name(parent: Path, run_id: str) -> str:
    return str(parent / f"transfer-preflight-{run_id}")


def _schedule_pairs(schedule: list[dict[str, Any]]) -> list[list[dict[str, Any]]]:
    groups: dict[str, list[dict[str, Any]]] = {}
    for slot in schedule:
        groups.setdefault(slot["pair_id"], []).append(slot)
    return list(groups.values())


def create_preflight(
    plan_path: Path,
    pins_path: Path,
    output_path: Path,
    scratch_parent: Path,
) -> dict[str, Any]:
    plan_bytes, plan_sha256 = _read_regular_file(plan_path, "paired plan")
    pins_bytes, pins_sha256 = _read_regular_file(pins_path, "client pin file")
    plan = _strict_json(plan_bytes, "paired plan")
    pin_document = _strict_json(pins_bytes, "client pin file")
    if not isinstance(plan, dict) or not isinstance(pin_document, dict):
        raise PreflightError("paired plan and pin file must be JSON objects")
    errors = validate("paired-plan", plan_path)
    if errors:
        raise PreflightError("paired plan is invalid: " + "; ".join(errors[:5]))
    pins = load_pins_document(pin_document)
    if set(pins) - set(CLIENT_IDS):
        raise PreflightError("pin file contains an unsupported client id")
    artifact = build_preflight(plan, plan_sha256, pins, pins_sha256, scratch_parent)
    plan_after, plan_digest_after = _read_regular_file(plan_path, "paired plan")
    pins_after, pins_digest_after = _read_regular_file(pins_path, "client pin file")
    if plan_digest_after != plan_sha256 or plan_after != plan_bytes:
        raise PreflightError("paired plan changed while compatibility preflight was running")
    if pins_digest_after != pins_sha256 or pins_after != pins_bytes:
        raise PreflightError("client pin file changed while compatibility preflight was running")
    _, profile_digest_after = _read_regular_file(DEFAULT_PROFILE_PATH, "fixture profile file")
    if profile_digest_after != plan["catalog"]["profiles_sha256"]:
        raise PreflightError("fixture profile file changed while compatibility preflight was running")
    write_new_json(output_path, artifact)
    return artifact


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--plan", type=Path, required=True)
    parser.add_argument("--pins", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--scratch-parent", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        artifact = create_preflight(args.plan, args.pins, args.output, args.scratch_parent)
    except (AdapterError, OSError, ValueError, subprocess.SubprocessError, xmlrpc.client.Error) as error:
        print(f"compatibility preflight: {error}", file=sys.stderr)
        return 2
    if not artifact["ready"]:
        print("compatibility preflight completed with excluded or failed candidates", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
