#!/usr/bin/env python3
"""Retarget generated fixture metainfo without rehashing its payload."""

from __future__ import annotations

import hashlib
import ipaddress
import json
import os
import re
import stat
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit

from strict_json import StrictJsonError, loads as strict_json_loads


MAX_METAINFO_BYTES = 64 * 1024 * 1024
MAX_FIXTURE_RECORD_BYTES = 4 * 1024 * 1024
MAX_BENCODE_DEPTH = 128
HEX_SHA1_RE = re.compile(r"^[a-f0-9]{40}$")
HEX_SHA256_RE = re.compile(r"^[a-f0-9]{64}$")
ROOT = Path(__file__).resolve().parents[2]


class MetainfoError(ValueError):
    """Generated metainfo is malformed or does not match its fixture record."""


def _strict_json_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise MetainfoError(f"fixture JSON contains duplicate field {key!r}")
        result[key] = value
    return result


def _parse_string(data: bytes, offset: int) -> tuple[bytes, int]:
    colon = data.find(b":", offset)
    if colon < 0 or colon - offset > 12:
        raise MetainfoError("bencoded string has an invalid length prefix")
    digits = data[offset:colon]
    if not digits or not digits.isdigit() or (len(digits) > 1 and digits.startswith(b"0")):
        raise MetainfoError("bencoded string length is not canonical")
    length = int(digits)
    end = colon + 1 + length
    if end > len(data):
        raise MetainfoError("bencoded string is truncated")
    return data[colon + 1:end], end


def _value_end(data: bytes, offset: int, depth: int = 0) -> int:
    if depth > MAX_BENCODE_DEPTH or offset >= len(data):
        raise MetainfoError("bencoded value exceeds the depth or input bound")
    marker = data[offset]
    if marker in b"0123456789":
        _, end = _parse_string(data, offset)
        return end
    if marker == ord("i"):
        end = data.find(b"e", offset + 1)
        if end < 0:
            raise MetainfoError("bencoded integer is unterminated")
        digits = data[offset + 1:end]
        if not digits or digits in {b"-0", b"+0"}:
            raise MetainfoError("bencoded integer is not canonical")
        numeric = digits[1:] if digits.startswith(b"-") else digits
        if not numeric.isdigit() or (len(numeric) > 1 and numeric.startswith(b"0")):
            raise MetainfoError("bencoded integer is not canonical")
        return end + 1
    if marker == ord("l"):
        cursor = offset + 1
        while cursor < len(data) and data[cursor] != ord("e"):
            cursor = _value_end(data, cursor, depth + 1)
        if cursor >= len(data):
            raise MetainfoError("bencoded list is unterminated")
        return cursor + 1
    if marker == ord("d"):
        cursor = offset + 1
        prior_key: bytes | None = None
        while cursor < len(data) and data[cursor] != ord("e"):
            key, cursor = _parse_string(data, cursor)
            if prior_key is not None and key <= prior_key:
                raise MetainfoError("bencoded dictionary keys must be unique and sorted")
            prior_key = key
            cursor = _value_end(data, cursor, depth + 1)
        if cursor >= len(data):
            raise MetainfoError("bencoded dictionary is unterminated")
        return cursor + 1
    raise MetainfoError("bencoded value has an unknown marker")


def _top_level_entries(data: bytes) -> list[tuple[bytes, bytes]]:
    if not isinstance(data, bytes) or not data or len(data) > MAX_METAINFO_BYTES:
        raise MetainfoError("metainfo must be a nonempty bounded byte string")
    if data[0] != ord("d"):
        raise MetainfoError("torrent metainfo must be a top-level dictionary")
    entries: list[tuple[bytes, bytes]] = []
    cursor = 1
    prior_key: bytes | None = None
    while cursor < len(data) and data[cursor] != ord("e"):
        key, cursor = _parse_string(data, cursor)
        if prior_key is not None and key <= prior_key:
            raise MetainfoError("top-level metainfo keys must be unique and sorted")
        prior_key = key
        end = _value_end(data, cursor, 1)
        entries.append((key, data[cursor:end]))
        cursor = end
    if cursor != len(data) - 1 or data[cursor] != ord("e"):
        raise MetainfoError("torrent metainfo has trailing data or an unterminated dictionary")
    return entries


def _decode_string_value(encoded: bytes, label: str) -> bytes:
    value, end = _parse_string(encoded, 0)
    if end != len(encoded):
        raise MetainfoError(f"{label} is not one complete bencoded string")
    return value


def _info_hashes(info_value: bytes) -> dict[str, str]:
    if not info_value.startswith(b"d") or _value_end(info_value, 0) != len(info_value):
        raise MetainfoError("metainfo info field is not one complete dictionary")
    info_sha1 = hashlib.sha1(info_value, usedforsecurity=False).hexdigest()  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- BitTorrent v1 info hashes are protocol-mandated SHA-1 values.
    info_sha256 = hashlib.sha256(info_value).digest()
    return {
        "sha1": info_sha1,
        "sha256": info_sha256.hex(),
        "sha256_truncated_20": info_sha256[:20].hex(),
    }


def _dictionary_entries(encoded: bytes, label: str) -> dict[bytes, bytes]:
    if not encoded.startswith(b"d") or _value_end(encoded, 0) != len(encoded):
        raise MetainfoError(f"{label} is not one complete bencoded dictionary")
    cursor = 1
    prior_key: bytes | None = None
    entries: dict[bytes, bytes] = {}
    while cursor < len(encoded) and encoded[cursor] != ord("e"):
        key, cursor = _parse_string(encoded, cursor)
        if prior_key is not None and key <= prior_key:
            raise MetainfoError(f"{label} keys must be unique and sorted")
        prior_key = key
        end = _value_end(encoded, cursor, 1)
        entries[key] = encoded[cursor:end]
        cursor = end
    if cursor != len(encoded) - 1 or encoded[cursor] != ord("e"):
        raise MetainfoError(f"{label} contains trailing data")
    return entries


def _decode_integer_value(encoded: bytes, label: str) -> int:
    if len(encoded) < 3 or encoded[0] != ord("i") or encoded[-1] != ord("e"):
        raise MetainfoError(f"{label} is not one complete bencoded integer")
    digits = encoded[1:-1]
    if not digits or digits in {b"-0", b"+0"}:
        raise MetainfoError(f"{label} is not a canonical bencoded integer")
    numeric = digits[1:] if digits.startswith(b"-") else digits
    if not numeric.isdigit() or (len(numeric) > 1 and numeric.startswith(b"0")):
        raise MetainfoError(f"{label} is not a canonical bencoded integer")
    return int(digits)


def _validate_hash_record(value: Any, kind: str) -> dict[str, str]:
    if not isinstance(value, dict) or any(not isinstance(key, str) for key in value):
        raise MetainfoError("fixture torrent info_hashes must be an object")
    if not isinstance(kind, str) or kind not in {"v1", "v2", "hybrid"}:
        raise MetainfoError("fixture torrent metainfo kind is unsupported")
    expected_fields = {
        "v1": {"sha1"},
        "v2": {"sha256", "sha256_truncated_20"},
        "hybrid": {"sha1", "sha256", "sha256_truncated_20"},
    }.get(kind)
    if expected_fields is None or set(value) != expected_fields:
        raise MetainfoError("fixture torrent info_hashes do not match its metainfo kind")
    result: dict[str, str] = {}
    for key, digest in value.items():
        pattern = HEX_SHA256_RE if key == "sha256" else HEX_SHA1_RE
        if not isinstance(digest, str) or pattern.fullmatch(digest) is None:
            raise MetainfoError(f"fixture torrent {key} info hash is malformed")
        result[key] = digest
    return result


def validate_task_announce_url(value: str) -> str:
    if not isinstance(value, str) or any(ord(char) < 0x20 or ord(char) == 0x7f for char in value):
        raise MetainfoError("task announce URL must be a printable string")
    try:
        parsed = urlsplit(value)
        port = parsed.port
    except ValueError as error:
        raise MetainfoError("task announce URL is malformed") from error
    if (
        parsed.scheme != "http"
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
        or parsed.path != "/announce"
        or parsed.query
        or parsed.fragment
        or port is None
        or not 1 <= port <= 65535
    ):
        raise MetainfoError("task announce URL must be an explicit private HTTP /announce endpoint")
    try:
        address = ipaddress.ip_address(parsed.hostname)
    except ValueError as error:
        raise MetainfoError("task announce URL host must be a private benchmark IPv4 address") from error
    if address.version != 4 or address not in ipaddress.ip_network("198.18.0.0/15"):
        raise MetainfoError("task announce URL host is outside the reserved benchmark subnet pool")
    return value


def retarget_metainfo(data: bytes, announce_url: str) -> tuple[bytes, dict[str, str]]:
    """Replace only the top-level announce value and preserve the exact info bytes."""
    announce_url = validate_task_announce_url(announce_url)
    entries = _top_level_entries(data)
    mapping = dict(entries)
    if len(mapping) != len(entries):
        raise MetainfoError("metainfo has duplicate top-level fields")
    if b"announce" not in mapping or b"info" not in mapping:
        raise MetainfoError("metainfo must contain announce and info fields")
    if any(key in mapping for key in (b"announce-list", b"url-list", b"httpseeds", b"nodes")):
        raise MetainfoError("fixture metainfo must not contain alternate trackers, web seeds, or DHT nodes")
    _decode_string_value(mapping[b"announce"], "announce")
    info_hashes = _info_hashes(mapping[b"info"])
    replacement = announce_url.encode("utf-8")
    encoded_announce = str(len(replacement)).encode("ascii") + b":" + replacement
    updated = [(key, value) for key, value in entries if key != b"announce"]
    updated.append((b"announce", encoded_announce))
    updated.sort(key=lambda entry: entry[0])
    output = b"d" + b"".join(
        str(len(key)).encode("ascii") + b":" + key + value for key, value in updated
    ) + b"e"
    if len(output) > MAX_METAINFO_BYTES:
        raise MetainfoError("retargeted metainfo exceeds the size limit")
    verified = dict(_top_level_entries(output))
    if verified.get(b"info") != mapping[b"info"]:
        raise MetainfoError("retargeting changed the exact encoded info dictionary")
    if _decode_string_value(verified[b"announce"], "announce") != replacement:
        raise MetainfoError("retargeted announce URL did not round-trip")
    return output, info_hashes


def _read_regular_nofollow(
    path: Path,
    label: str,
    *,
    limit_bytes: int = MAX_METAINFO_BYTES,
) -> bytes:
    if isinstance(limit_bytes, bool) or not isinstance(limit_bytes, int) or limit_bytes <= 0:
        raise MetainfoError("file read limit must be a positive integer")
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise MetainfoError(f"cannot open {label} without following links") from error
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode) or before.st_size > limit_bytes:
            raise MetainfoError(f"{label} must be a bounded regular file")
        chunks: list[bytes] = []
        total = 0
        while True:
            chunk = os.read(descriptor, min(1024 * 1024, limit_bytes + 1 - total))
            if not chunk:
                break
            chunks.append(chunk)
            total += len(chunk)
            if total > limit_bytes:
                raise MetainfoError(f"{label} exceeds its size limit")
        after = os.fstat(descriptor)
        identity = lambda value: (value.st_dev, value.st_ino, value.st_size, value.st_mtime_ns, value.st_ctime_ns)
        if identity(before) != identity(after) or total != before.st_size:
            raise MetainfoError(f"{label} changed while being read")
        return b"".join(chunks)
    finally:
        os.close(descriptor)


def _write_new_file(path: Path, data: bytes) -> None:
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise MetainfoError("retargeted metainfo output must be an absolute normalized path")
    try:
        path.relative_to(ROOT)
    except ValueError:
        pass
    else:
        raise MetainfoError("retargeted metainfo must be written outside the source worktree")
    parent = path.parent
    try:
        if parent.resolve(strict=True) != parent.absolute() or not parent.is_dir():
            raise MetainfoError("retargeted metainfo parent must be an existing non-symlink directory")
    except OSError as error:
        raise MetainfoError("retargeted metainfo parent must already exist") from error
    directory_fd = os.open(parent, os.O_RDONLY | os.O_CLOEXEC | os.O_DIRECTORY | os.O_NOFOLLOW)
    file_fd: int | None = None
    created_identity: tuple[int, int] | None = None
    try:
        file_fd = os.open(
            path.name,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
            0o600,
            dir_fd=directory_fd,
        )
        metadata = os.fstat(file_fd)
        created_identity = (metadata.st_dev, metadata.st_ino)
        if not stat.S_ISREG(metadata.st_mode):
            raise MetainfoError("retargeted metainfo output must be a regular file")
        os.fchmod(file_fd, 0o600)
        pending = memoryview(data)
        while pending:
            written = os.write(file_fd, pending)
            if written <= 0:
                raise MetainfoError("short write while publishing retargeted metainfo")
            pending = pending[written:]
        os.fsync(file_fd)
        os.close(file_fd)
        file_fd = None
        os.fsync(directory_fd)
    except FileExistsError as error:
        raise MetainfoError("refusing to replace existing retargeted metainfo output") from error
    except BaseException:
        if file_fd is not None:
            os.close(file_fd)
            file_fd = None
        if created_identity is not None:
            try:
                current = os.stat(path.name, dir_fd=directory_fd, follow_symlinks=False)
                if (current.st_dev, current.st_ino) == created_identity:
                    os.unlink(path.name, dir_fd=directory_fd)
            except OSError:
                pass
        raise
    finally:
        if file_fd is not None:
            os.close(file_fd)
        os.close(directory_fd)


def retarget_fixture_torrent(
    fixture_directory: Path,
    output_path: Path,
    announce_url: str,
) -> dict[str, Any]:
    """Retarget a fixture torrent after verifying its immutable generator records."""
    fixture_directory = Path(fixture_directory)
    if (
        fixture_directory.is_symlink()
        or not fixture_directory.is_dir()
        or fixture_directory.resolve(strict=True) != fixture_directory.absolute()
    ):
        raise MetainfoError("fixture directory must be an existing non-symlink directory")
    try:
        fixture_record = strict_json_loads(
            _read_regular_nofollow(
                fixture_directory / "fixture.json",
                "fixture manifest",
                limit_bytes=MAX_FIXTURE_RECORD_BYTES,
            ),
            label="fixture manifest",
        )
        torrent_record = strict_json_loads(
            _read_regular_nofollow(
                fixture_directory / "torrent.json",
                "fixture torrent record",
                limit_bytes=MAX_FIXTURE_RECORD_BYTES,
            ),
            label="fixture torrent record",
        )
    except StrictJsonError as error:
        raise MetainfoError(f"fixture records are not valid strict JSON: {error}") from error
    if not isinstance(fixture_record, dict) or not isinstance(torrent_record, dict):
        raise MetainfoError("fixture records must be JSON objects")
    _validate_fixture_identity(fixture_record)
    torrent_fields = {
        "format",
        "announce_url",
        "name",
        "piece_length",
        "metainfo_kind",
        "info_hashes",
        "torrent_sha256",
        "fixture_sha256",
    }
    if set(torrent_record) != torrent_fields:
        raise MetainfoError("fixture torrent record has missing or unknown fields")
    if torrent_record.get("format") != "transfer-torrent-v2":
        raise MetainfoError("fixture torrent record has an unsupported format")
    fixture_sha256 = fixture_record.get("fixture_sha256")
    if (
        not isinstance(fixture_sha256, str)
        or HEX_SHA256_RE.fullmatch(fixture_sha256) is None
        or torrent_record.get("fixture_sha256") != fixture_sha256
    ):
        raise MetainfoError("fixture and torrent records do not share one fixture digest")
    torrent_kind = torrent_record.get("metainfo_kind")
    if not isinstance(torrent_kind, str) or torrent_kind not in {"v1", "v2", "hybrid"}:
        raise MetainfoError("fixture torrent kind is unsupported")
    profile = fixture_record.get("profile")
    if profile is not None and (
        profile["torrent_kind"] != torrent_kind
        or profile["torrent_name"] != torrent_record.get("name")
        or profile["piece_length"] != torrent_record.get("piece_length")
    ):
        raise MetainfoError("fixture torrent differs from its profile identity")
    expected_hashes = _validate_hash_record(torrent_record.get("info_hashes"), torrent_kind)
    source_digest = torrent_record.get("torrent_sha256")
    if not isinstance(source_digest, str) or HEX_SHA256_RE.fullmatch(source_digest) is None:
        raise MetainfoError("fixture torrent source digest is malformed")
    source_data = _read_regular_nofollow(fixture_directory / "fixture.torrent", "fixture metainfo")
    if hashlib.sha256(source_data).hexdigest() != source_digest:
        raise MetainfoError("fixture metainfo changed after its generator record was written")
    source_announce = torrent_record.get("announce_url")
    name = torrent_record.get("name")
    piece_length = torrent_record.get("piece_length")
    if (
        not isinstance(source_announce, str)
        or not source_announce
        or not isinstance(name, str)
        or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", name) is None
        or isinstance(piece_length, bool)
        or not isinstance(piece_length, int)
        or piece_length < 16 * 1024
        or piece_length > 16 * 1024 * 1024
        or piece_length & (piece_length - 1)
    ):
        raise MetainfoError("fixture torrent record has no original announce URL")
    retargeted, observed_hashes = retarget_metainfo(source_data, announce_url)
    source_entries = dict(_top_level_entries(source_data))
    if _decode_string_value(source_entries[b"announce"], "announce") != source_announce.encode():
        raise MetainfoError("fixture announce URL differs from its torrent record")
    info_fields = _dictionary_entries(source_entries[b"info"], "torrent info dictionary")
    if (
        _decode_string_value(info_fields.get(b"name", b""), "torrent name") != name.encode("ascii")
        or _decode_integer_value(info_fields.get(b"piece length", b""), "torrent piece length") != piece_length
        or _decode_integer_value(info_fields.get(b"private", b""), "torrent private flag") != 1
    ):
        raise MetainfoError("fixture torrent record differs from the metainfo identity")
    has_v1 = b"pieces" in info_fields and b"files" in info_fields
    has_v2 = b"meta version" in info_fields and b"file tree" in info_fields
    expected_kind = "hybrid" if has_v1 and has_v2 else "v2" if has_v2 else "v1" if has_v1 else None
    if expected_kind != torrent_kind:
        raise MetainfoError("fixture torrent kind differs from its metainfo fields")
    expected_info_fields = {b"name", b"piece length", b"private"}
    if torrent_kind in {"v1", "hybrid"}:
        expected_info_fields.update({b"files", b"pieces"})
    if torrent_kind in {"v2", "hybrid"}:
        expected_info_fields.update({b"file tree", b"meta version"})
    if set(info_fields) != expected_info_fields:
        raise MetainfoError("fixture torrent info fields do not match its declared kind")
    if torrent_kind == "v1" and b"piece layers" in source_entries:
        raise MetainfoError("v1 fixture metainfo must not contain v2 piece layers")
    if torrent_kind in {"v2", "hybrid"}:
        if b"piece layers" not in source_entries:
            raise MetainfoError("v2 fixture metainfo is missing its piece layers dictionary")
        _dictionary_entries(source_entries[b"piece layers"], "torrent piece layers")
    if has_v2 and _decode_integer_value(info_fields[b"meta version"], "torrent meta version") != 2:
        raise MetainfoError("fixture v2 metainfo version is unsupported")
    if set(expected_hashes).difference(observed_hashes) or any(
        observed_hashes[key] != digest for key, digest in expected_hashes.items()
    ):
        raise MetainfoError("fixture info hash does not match its generator record")
    _write_new_file(Path(output_path), retargeted)
    return {
        "schema_version": "transfer-retargeted-metainfo-v1",
        "fixture_sha256": fixture_sha256,
        "source_torrent_sha256": source_digest,
        "torrent_sha256": hashlib.sha256(retargeted).hexdigest(),
        "announce_url": announce_url,
        "metainfo_kind": torrent_kind,
        "info_hashes": expected_hashes,
    }


def _validate_fixture_identity(record: dict[str, Any]) -> dict[str, Any]:
    required = {"format", "generator", "seed", "files", "fixture_sha256"}
    allowed = required | {"profile"}
    if frozenset(record) not in {frozenset(required), frozenset(allowed)}:
        raise MetainfoError("fixture manifest has missing or unknown fields")
    if record.get("format") != "transfer-fixture-v1":
        raise MetainfoError("fixture manifest has an unsupported format")
    if record.get("generator") != "shake256-counter-v1":
        raise MetainfoError("fixture manifest generator is unsupported")
    seed = record.get("seed")
    files = record.get("files")
    if not isinstance(seed, str) or not seed or len(seed.encode("utf-8")) > 4096:
        raise MetainfoError("fixture manifest seed is missing or invalid")
    if not isinstance(files, list) or not files:
        raise MetainfoError("fixture manifest files must be a nonempty array")
    seen_names: set[str] = set()
    for entry in files:
        if not isinstance(entry, dict) or set(entry) != {"name", "size_bytes", "sha256"}:
            raise MetainfoError("fixture file record has missing or unknown fields")
        name = entry.get("name")
        size = entry.get("size_bytes")
        digest = entry.get("sha256")
        if not isinstance(name, str) or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", name) is None:
            raise MetainfoError("fixture file name is invalid")
        if name.casefold() == "fixture.json" or name.casefold() in seen_names:
            raise MetainfoError("fixture file names are reserved or duplicated")
        seen_names.add(name.casefold())
        if isinstance(size, bool) or not isinstance(size, int) or size < 0:
            raise MetainfoError("fixture file size is invalid")
        if not isinstance(digest, str) or HEX_SHA256_RE.fullmatch(digest) is None:
            raise MetainfoError("fixture file digest is invalid")
    identity = {key: record[key] for key in ("format", "generator", "seed", "files")}
    if "profile" in record:
        profile = record["profile"]
        if not isinstance(profile, dict) or set(profile) != {
            "id", "torrent_kind", "torrent_name", "piece_length"
        }:
            raise MetainfoError("fixture profile has missing or unknown fields")
        profile_id = profile.get("id")
        profile_kind = profile.get("torrent_kind")
        profile_name = profile.get("torrent_name")
        profile_length = profile.get("piece_length")
        if (
            not isinstance(profile_id, str)
            or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_./-]{0,127}", profile_id) is None
            or not isinstance(profile_kind, str)
            or profile_kind not in {"v1", "v2", "hybrid"}
            or not isinstance(profile_name, str)
            or re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9_.-]{0,127}", profile_name) is None
            or isinstance(profile_length, bool)
            or not isinstance(profile_length, int)
            or profile_length < 16 * 1024
            or profile_length > 16 * 1024 * 1024
            or profile_length & (profile_length - 1)
        ):
            raise MetainfoError("fixture profile identity is invalid")
        identity["profile"] = profile
    canonical = json.dumps(identity, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    expected = hashlib.sha256(canonical.encode("ascii")).hexdigest()
    claimed = record.get("fixture_sha256")
    if not isinstance(claimed, str) or HEX_SHA256_RE.fullmatch(claimed) is None or claimed != expected:
        raise MetainfoError("fixture identity digest does not match its manifest")
    return identity
