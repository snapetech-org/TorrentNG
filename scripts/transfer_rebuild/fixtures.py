"""Generate deterministic, task-scoped payload fixtures for transfer trials."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable
from urllib.parse import urlsplit


FORMAT = "transfer-fixture-v1"
GENERATOR = "shake256-counter-v1"
CHUNK_BYTES = 1024 * 1024
DEFAULT_PIECE_LENGTH = 1024 * 1024
TORRENT_FORMAT = "transfer-torrent-v1"
TORRENT_RECORD_FORMAT = "transfer-torrent-v2"
V2_BLOCK_BYTES = 16 * 1024
_NAME = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$")
_PROFILE_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9_./-]{0,127}$")


@dataclass(frozen=True)
class FixtureFile:
    name: str
    size_bytes: int


def _validated_files(files: Iterable[FixtureFile]) -> list[FixtureFile]:
    result = list(files)
    if not result:
        raise ValueError("a fixture must contain at least one file")
    names: list[str] = []
    for item in result:
        if not isinstance(item.name, str) or _NAME.fullmatch(item.name) is None:
            raise ValueError(f"fixture file name is not a simple label: {item.name!r}")
        names.append(item.name.casefold())
        if item.name.casefold() == "fixture.json":
            raise ValueError("fixture.json is reserved for the generator manifest")
        if (
            not isinstance(item.size_bytes, int)
            or isinstance(item.size_bytes, bool)
            or item.size_bytes < 0
        ):
            raise ValueError(f"fixture file size must be nonnegative: {item.name}")
    if len(set(names)) != len(names):
        raise ValueError("fixture file names must be unique")
    return result


def _fixture_chunk(seed: bytes, name: bytes, index: int, size: int) -> bytes:
    material = (
        b"TorrentNG transfer fixture v1\0"
        + seed
        + b"\0"
        + name
        + b"\0"
        + index.to_bytes(8, "big")
    )
    return hashlib.shake_256(material).digest(size)


def _bencode(value: object) -> bytes:
    if isinstance(value, int) and not isinstance(value, bool):
        return b"i" + str(value).encode("ascii") + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode("ascii") + b":" + value
    if isinstance(value, (list, tuple)):
        return b"l" + b"".join(_bencode(item) for item in value) + b"e"
    if isinstance(value, dict):
        if any(not isinstance(key, bytes) for key in value):
            raise ValueError("bencoded dictionary keys must be bytes")
        return b"d" + b"".join(
            _bencode(key) + _bencode(value[key]) for key in sorted(value)
        ) + b"e"
    raise TypeError(f"unsupported bencode value: {type(value).__name__}")


def _validate_torrent_options(announce_url: str, name: str, piece_length: int) -> None:
    if not isinstance(announce_url, str):
        raise ValueError("tracker announce URL must be a string")
    if any(ord(character) < 0x20 or ord(character) == 0x7F for character in announce_url):
        raise ValueError("tracker announce URL cannot contain control characters")
    try:
        parsed = urlsplit(announce_url)
        port = parsed.port
    except ValueError as error:
        raise ValueError(f"tracker announce URL is invalid: {error}") from error
    if (
        parsed.scheme not in {"http", "https", "udp"}
        or not parsed.hostname
        or parsed.username is not None
        or parsed.password is not None
    ):
        raise ValueError("tracker announce URL must be an http(s) or udp URL without credentials")
    if parsed.scheme == "udp" and port is None:
        raise ValueError("UDP tracker announce URL must include a port")
    if not isinstance(name, str) or _NAME.fullmatch(name) is None:
        raise ValueError(f"torrent name is not a simple label: {name!r}")
    if (
        not isinstance(piece_length, int)
        or isinstance(piece_length, bool)
        or piece_length < 16 * 1024
        or piece_length > 16 * 1024 * 1024
        or piece_length & (piece_length - 1)
    ):
        raise ValueError("torrent piece length must be a power of two from 16 KiB to 16 MiB")


def load_fixture_profile(path: Path, profile_id: str) -> dict[str, object]:
    """Load one strictly shaped fixture profile from a versioned profile file."""
    if not isinstance(profile_id, str) or _PROFILE_ID.fullmatch(profile_id) is None:
        raise ValueError("fixture profile id is invalid")
    profile_path = Path(path)
    if profile_path.is_symlink() or not profile_path.is_file():
        raise ValueError("fixture profile file must be a regular non-symlink file")
    try:
        document = json.loads(profile_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read fixture profiles: {error}") from error
    if (
        not isinstance(document, dict)
        or set(document) != {"format", "profiles"}
        or document["format"] != "transfer-fixture-profiles-v1"
        or not isinstance(document["profiles"], dict)
    ):
        raise ValueError("fixture profile file has an unsupported format or shape")
    raw = document["profiles"].get(profile_id)
    if not isinstance(raw, dict) or set(raw) != {
        "files",
        "piece_length",
        "torrent_kind",
        "torrent_name",
    }:
        raise ValueError(f"fixture profile is missing or malformed: {profile_id}")
    files = raw["files"]
    if not isinstance(files, list):
        raise ValueError(f"fixture profile files must be a list: {profile_id}")
    specs: list[FixtureFile] = []
    for entry in files:
        if not isinstance(entry, dict) or set(entry) != {"name", "size_bytes"}:
            raise ValueError(f"fixture profile file entry is malformed: {profile_id}")
        specs.append(FixtureFile(entry["name"], entry["size_bytes"]))
    _validated_files(specs)
    kind = raw["torrent_kind"]
    name = raw["torrent_name"]
    piece_length = raw["piece_length"]
    if not isinstance(kind, str) or kind not in {"v1", "v2", "hybrid"}:
        raise ValueError(f"fixture profile torrent kind is invalid: {profile_id}")
    if not isinstance(name, str):
        raise ValueError(f"fixture profile torrent name is invalid: {profile_id}")
    try:
        _validate_torrent_options("http://127.0.0.1:1/announce", name, piece_length)
    except ValueError as error:
        raise ValueError(f"fixture profile is invalid ({profile_id}): {error}") from error
    return {
        "files": specs,
        "piece_length": piece_length,
        "torrent_kind": kind,
        "torrent_name": name,
    }


def _hash_pair(left: bytes, right: bytes) -> bytes:
    return hashlib.sha256(left + right).digest()


def _merkle_root(leaves: list[bytes]) -> bytes:
    if not leaves:
        return bytes(32)
    level = list(leaves)
    target = 1 << (len(level) - 1).bit_length()
    level.extend([bytes(32)] * (target - len(level)))
    while len(level) > 1:
        level = [
            _hash_pair(level[index], level[index + 1])
            for index in range(0, len(level), 2)
        ]
    return level[0]


def _v2_file_hashes(
    path: Path, size_bytes: int, piece_length: int
) -> tuple[bytes | None, bytes | None]:
    """Return the BEP 52 file root and, when needed, its piece layer."""
    if size_bytes == 0:
        return None, None

    block_count_per_piece = piece_length // V2_BLOCK_BYTES
    piece_hashes: list[bytes] = []
    whole_file_hashes: list[bytes] = []
    remaining = size_bytes
    with path.open("rb") as source:
        if size_bytes <= piece_length:
            while remaining:
                block = source.read(min(V2_BLOCK_BYTES, remaining))
                if not block:
                    raise ValueError(f"fixture payload was truncated while hashing: {path.name}")
                whole_file_hashes.append(hashlib.sha256(block).digest())
                remaining -= len(block)
            if source.read(1):
                raise ValueError(f"fixture payload grew while hashing: {path.name}")
            return _merkle_root(whole_file_hashes), None

        while remaining:
            piece_bytes = min(piece_length, remaining)
            piece_remaining = piece_bytes
            block_hashes: list[bytes] = []
            while piece_remaining:
                block = source.read(min(V2_BLOCK_BYTES, piece_remaining))
                if not block:
                    raise ValueError(f"fixture payload was truncated while hashing: {path.name}")
                block_hashes.append(hashlib.sha256(block).digest())
                piece_remaining -= len(block)
            block_hashes.extend(
                [bytes(32)] * (block_count_per_piece - len(block_hashes))
            )
            piece_hashes.append(_merkle_root(block_hashes))
            remaining -= piece_bytes
        if source.read(1):
            raise ValueError(f"fixture payload grew while hashing: {path.name}")

    return _merkle_root(piece_hashes), b"".join(piece_hashes)


def _build_v2_file_tree(
    directory: Path,
    entries: list[dict[str, object]],
    piece_length: int,
) -> tuple[dict[bytes, object], dict[bytes, bytes]]:
    file_tree: dict[bytes, object] = {}
    piece_layers: dict[bytes, bytes] = {}
    for entry in entries:
        name = str(entry["name"]).encode("ascii")
        size = int(entry["size_bytes"])
        pieces_root, layer = _v2_file_hashes(
            directory / str(entry["name"]), size, piece_length
        )
        leaf: dict[bytes, object] = {b"length": size}
        if pieces_root is not None:
            leaf[b"pieces root"] = pieces_root
        file_tree[name] = {b"": leaf}
        if pieces_root is not None and layer is not None:
            previous = piece_layers.setdefault(pieces_root, layer)
            if previous != layer:
                raise ValueError("identical v2 file roots produced conflicting piece layers")
    return file_tree, piece_layers


def _v1_stream_hashes(
    directory: Path,
    entries: list[dict[str, object]],
    piece_length: int,
    *,
    align_files: bool,
) -> tuple[bytes, list[dict[bytes, object]]]:
    piece_buffer = bytearray()
    piece_hashes = bytearray()
    stream_offset = 0
    has_payload = False
    v1_files: list[dict[bytes, object]] = []

    def feed(data: bytes) -> None:
        cursor = 0
        while cursor < len(data):
            take = min(piece_length - len(piece_buffer), len(data) - cursor)
            piece_buffer.extend(data[cursor : cursor + take])
            cursor += take
            if len(piece_buffer) == piece_length:
                piece_hashes.extend(hashlib.sha1(piece_buffer).digest())
                piece_buffer.clear()

    for entry in entries:
        name = str(entry["name"])
        size = int(entry["size_bytes"])
        if align_files and size > 0 and has_payload:
            padding = (-stream_offset) % piece_length
            if padding:
                pad_name = str(padding).encode("ascii")
                v1_files.append(
                    {
                        b"attr": b"p",
                        b"length": padding,
                        b"path": [b".pad", pad_name],
                    }
                )
                remaining_padding = padding
                while remaining_padding:
                    zeros = bytes(min(CHUNK_BYTES, remaining_padding))
                    feed(zeros)
                    stream_offset += len(zeros)
                    remaining_padding -= len(zeros)

        v1_files.append(
            {
                b"length": size,
                b"path": [name.encode("ascii")],
            }
        )
        source_path = directory / name
        observed = 0
        with source_path.open("rb") as source:
            for chunk in iter(lambda: source.read(CHUNK_BYTES), b""):
                feed(chunk)
                observed += len(chunk)
                stream_offset += len(chunk)
        if observed != size:
            raise ValueError(f"fixture payload size changed while hashing: {name}")
        has_payload = has_payload or size > 0

    if piece_buffer:
        piece_hashes.extend(hashlib.sha1(piece_buffer).digest())
    return bytes(piece_hashes), v1_files


def _build_torrent(
    directory: Path,
    entries: list[dict[str, object]],
    announce_url: str,
    name: str,
    piece_length: int,
    kind: str,
) -> tuple[bytes, dict[str, str]]:
    _validate_torrent_options(announce_url, name, piece_length)
    if kind not in {"v1", "v2", "hybrid"}:
        raise ValueError("torrent kind must be v1, v2, or hybrid")
    if not any(int(entry["size_bytes"]) > 0 for entry in entries):
        raise ValueError("torrent metainfo requires at least one non-empty payload file")

    torrent_entries = (
        entries
        if kind == "v1"
        else sorted(entries, key=lambda entry: str(entry["name"]).encode("ascii"))
    )
    info: dict[bytes, object] = {
        b"name": name.encode("ascii"),
        b"piece length": piece_length,
        b"private": 1,
    }
    piece_layers: dict[bytes, bytes] = {}
    if kind in {"v1", "hybrid"}:
        v1_pieces, v1_files = _v1_stream_hashes(
            directory,
            torrent_entries,
            piece_length,
            align_files=kind == "hybrid",
        )
        info[b"files"] = v1_files
        info[b"pieces"] = v1_pieces
    if kind in {"v2", "hybrid"}:
        file_tree, piece_layers = _build_v2_file_tree(
            directory, torrent_entries, piece_length
        )
        info[b"file tree"] = file_tree
        info[b"meta version"] = 2

    info_bytes = _bencode(info)
    top_level: dict[bytes, object] = {
        b"announce": announce_url.encode("utf-8"),
        b"info": info,
    }
    if kind in {"v2", "hybrid"}:
        top_level[b"piece layers"] = piece_layers
    metainfo = _bencode(top_level)
    hashes: dict[str, str] = {}
    if kind in {"v1", "hybrid"}:
        hashes["sha1"] = hashlib.sha1(info_bytes).hexdigest()
    if kind in {"v2", "hybrid"}:
        info_hash_v2 = hashlib.sha256(info_bytes).digest()
        hashes["sha256"] = info_hash_v2.hex()
        hashes["sha256_truncated_20"] = info_hash_v2[:20].hex()
    return metainfo, hashes


def _write_torrent(
    directory: Path,
    entries: list[dict[str, object]],
    fixture_sha256: str,
    announce_url: str,
    name: str,
    piece_length: int,
    kind: str,
) -> None:
    metainfo, info_hashes = _build_torrent(
        directory, entries, announce_url, name, piece_length, kind
    )
    with (directory / "fixture.torrent").open("xb") as target:
        target.write(metainfo)
    torrent_record = {
        "format": TORRENT_RECORD_FORMAT,
        "announce_url": announce_url,
        "name": name,
        "piece_length": piece_length,
        "metainfo_kind": kind,
        "info_hashes": info_hashes,
        "torrent_sha256": hashlib.sha256(metainfo).hexdigest(),
        "fixture_sha256": fixture_sha256,
    }
    with (directory / "torrent.json").open("x", encoding="utf-8") as target:
        json.dump(torrent_record, target, indent=2, sort_keys=True)
        target.write("\n")


def _canonical_digest(value: object) -> str:
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return hashlib.sha256(encoded.encode("ascii")).hexdigest()


def _sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(CHUNK_BYTES), b""):
            digest.update(chunk)
    return digest.hexdigest()


def generate_fixture(
    output: Path,
    seed: str,
    files: Iterable[FixtureFile],
    *,
    announce_url: str | None = None,
    torrent_name: str = "transfer-fixture",
    piece_length: int = DEFAULT_PIECE_LENGTH,
    torrent_kind: str = "v1",
    profile_id: str | None = None,
) -> dict[str, object]:
    """Write files and a content manifest beneath a previously absent path.

    A private staging directory is created beside the destination and renamed
    into place only after all files and the manifest have been written. The
    function never merges into or clears a caller-owned directory.
    """
    specs = _validated_files(files)
    if not isinstance(seed, str) or not seed:
        raise ValueError("fixture seed must not be empty")
    if not isinstance(torrent_kind, str) or torrent_kind not in {"v1", "v2", "hybrid"}:
        raise ValueError("torrent kind must be v1, v2, or hybrid")
    if profile_id is not None and (
        not isinstance(profile_id, str) or _PROFILE_ID.fullmatch(profile_id) is None
    ):
        raise ValueError("fixture profile id is invalid")
    if profile_id is not None:
        _validate_torrent_options(
            "http://127.0.0.1:1/announce", torrent_name, piece_length
        )
    if announce_url is None and torrent_kind != "v1":
        raise ValueError("v2 and hybrid metainfo require a tracker URL")
    if announce_url is not None:
        _validate_torrent_options(announce_url, torrent_name, piece_length)
    output = Path(output)
    parent = output.parent
    if not parent.is_dir() or parent.is_symlink():
        raise ValueError("fixture output parent must be an existing, non-symlink directory")
    if output.exists() or output.is_symlink():
        raise FileExistsError(f"fixture output already exists: {output}")

    stage = Path(tempfile.mkdtemp(prefix=f".{output.name}.fixture-", dir=parent))
    entries: list[dict[str, object]] = []
    try:
        for spec in specs:
            path = stage / spec.name
            digest = hashlib.sha256()
            remaining = spec.size_bytes
            chunk_index = 0
            seed_bytes = seed.encode("utf-8")
            name_bytes = spec.name.encode("ascii")
            with path.open("xb") as target:
                while remaining:
                    chunk_size = min(CHUNK_BYTES, remaining)
                    chunk = _fixture_chunk(seed_bytes, name_bytes, chunk_index, chunk_size)
                    target.write(chunk)
                    digest.update(chunk)
                    remaining -= len(chunk)
                    chunk_index += 1
            entries.append(
                {
                    "name": spec.name,
                    "size_bytes": spec.size_bytes,
                    "sha256": digest.hexdigest(),
                }
            )

        identity: dict[str, object] = {
            "format": FORMAT,
            "generator": GENERATOR,
            "seed": seed,
            "files": entries,
        }
        if profile_id is not None:
            identity["profile"] = {
                "id": profile_id,
                "torrent_kind": torrent_kind,
                "torrent_name": torrent_name,
                "piece_length": piece_length,
            }
        manifest = {**identity, "fixture_sha256": _canonical_digest(identity)}
        if announce_url is not None:
            _write_torrent(
                stage,
                entries,
                str(manifest["fixture_sha256"]),
                announce_url,
                torrent_name,
                piece_length,
                torrent_kind,
            )
        with (stage / "fixture.json").open("x", encoding="utf-8") as target:
            json.dump(manifest, target, indent=2, sort_keys=True)
            target.write("\n")

        if output.exists() or output.is_symlink():
            raise FileExistsError(f"fixture output appeared during generation: {output}")
        os.rename(stage, output)
        return manifest
    except BaseException:
        shutil.rmtree(stage, ignore_errors=True)
        raise


def verify_fixture(directory: Path) -> dict[str, object]:
    """Verify manifest shape, fixture identity, and every generated payload."""
    directory = Path(directory)
    if directory.is_symlink() or not directory.is_dir():
        raise ValueError("fixture root must be an existing, non-symlink directory")
    manifest_path = directory / "fixture.json"
    if manifest_path.is_symlink() or not manifest_path.is_file():
        raise ValueError("fixture manifest must be a regular file")
    try:
        manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read fixture manifest: {error}") from error
    expected_manifest_fields = {
        "format",
        "generator",
        "seed",
        "files",
        "fixture_sha256",
    }
    if not isinstance(manifest, dict) or frozenset(manifest) not in {
        frozenset(expected_manifest_fields),
        frozenset(expected_manifest_fields | {"profile"}),
    }:
        raise ValueError("fixture manifest has missing or unknown fields")
    if manifest["format"] != FORMAT or manifest["generator"] != GENERATOR:
        raise ValueError("unsupported fixture format or generator")
    if not isinstance(manifest["seed"], str) or not manifest["seed"]:
        raise ValueError("fixture seed is missing or invalid")
    raw_files = manifest["files"]
    if not isinstance(raw_files, list):
        raise ValueError("fixture files must be a list")

    specs: list[FixtureFile] = []
    expected_digests: list[str] = []
    for entry in raw_files:
        if not isinstance(entry, dict) or set(entry) != {"name", "size_bytes", "sha256"}:
            raise ValueError("fixture file entry has missing or unknown fields")
        name = entry["name"]
        size = entry["size_bytes"]
        digest = entry["sha256"]
        if not isinstance(name, str) or not isinstance(size, int) or isinstance(size, bool):
            raise ValueError("fixture file name or size has an invalid type")
        if not isinstance(digest, str) or re.fullmatch(r"[a-f0-9]{64}", digest) is None:
            raise ValueError(f"fixture file digest is invalid: {name}")
        specs.append(FixtureFile(name, size))
        expected_digests.append(digest)
    _validated_files(specs)

    identity = {key: manifest[key] for key in ("format", "generator", "seed", "files")}
    if "profile" in manifest:
        profile = manifest["profile"]
        if not isinstance(profile, dict) or set(profile) != {
            "id",
            "torrent_kind",
            "torrent_name",
            "piece_length",
        }:
            raise ValueError("fixture profile identity has missing or unknown fields")
        if not isinstance(profile["id"], str) or _PROFILE_ID.fullmatch(profile["id"]) is None:
            raise ValueError("fixture profile identity has an invalid id")
        if (
            not isinstance(profile["torrent_kind"], str)
            or profile["torrent_kind"] not in {"v1", "v2", "hybrid"}
        ):
            raise ValueError("fixture profile identity has an invalid torrent kind")
        if not isinstance(profile["torrent_name"], str):
            raise ValueError("fixture profile identity has an invalid torrent name")
        try:
            _validate_torrent_options(
                "http://127.0.0.1:1/announce",
                profile["torrent_name"],
                profile["piece_length"],
            )
        except ValueError as error:
            raise ValueError(f"fixture profile identity is invalid: {error}") from error
        identity["profile"] = profile
    expected_fixture_digest = _canonical_digest(identity)
    if manifest["fixture_sha256"] != expected_fixture_digest:
        raise ValueError("fixture identity digest does not match its manifest")

    for spec, expected_digest in zip(specs, expected_digests, strict=True):
        path = directory / spec.name
        if path.is_symlink() or not path.is_file():
            raise ValueError(f"fixture payload is missing or not a regular file: {spec.name}")
        if path.stat().st_size != spec.size_bytes:
            raise ValueError(f"fixture payload size mismatch: {spec.name}")
        digest = hashlib.sha256()
        with path.open("rb") as source:
            for chunk in iter(lambda: source.read(CHUNK_BYTES), b""):
                digest.update(chunk)
        if digest.hexdigest() != expected_digest:
            raise ValueError(f"fixture payload digest mismatch: {spec.name}")
    _verify_torrent(directory, manifest)
    return manifest


def _verify_torrent(directory: Path, manifest: dict[str, object]) -> None:
    torrent_path = directory / "fixture.torrent"
    record_path = directory / "torrent.json"
    has_torrent = torrent_path.exists() or torrent_path.is_symlink()
    has_record = record_path.exists() or record_path.is_symlink()
    if not has_torrent and not has_record:
        return
    if not has_torrent or not has_record:
        raise ValueError("torrent metainfo and its record must both be present")
    if (
        torrent_path.is_symlink()
        or not torrent_path.is_file()
        or record_path.is_symlink()
        or not record_path.is_file()
    ):
        raise ValueError("torrent metainfo and record must be regular files")
    try:
        record = json.loads(record_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"cannot read torrent record: {error}") from error
    if not isinstance(record, dict):
        raise ValueError("torrent record must be an object")
    if record.get("format") == TORRENT_FORMAT:
        fields = {
            "format",
            "announce_url",
            "name",
            "piece_length",
            "info_hash_sha1",
            "torrent_sha256",
            "fixture_sha256",
        }
        if set(record) != fields:
            raise ValueError("legacy torrent record has missing or unknown fields")
        kind = "v1"
    elif record.get("format") == TORRENT_RECORD_FORMAT:
        fields = {
            "format",
            "announce_url",
            "name",
            "piece_length",
            "metainfo_kind",
            "info_hashes",
            "torrent_sha256",
            "fixture_sha256",
        }
        if set(record) != fields:
            raise ValueError("torrent record has missing or unknown fields")
        kind = record["metainfo_kind"]
        if not isinstance(kind, str) or kind not in {"v1", "v2", "hybrid"}:
            raise ValueError("torrent record metainfo kind is invalid")
    else:
        raise ValueError("unsupported torrent record format")
    if record["fixture_sha256"] != manifest["fixture_sha256"]:
        raise ValueError("torrent record belongs to a different fixture")
    announce_url = record["announce_url"]
    name = record["name"]
    piece_length = record["piece_length"]
    if not isinstance(announce_url, str) or not isinstance(name, str):
        raise ValueError("torrent announce URL or name is invalid")
    try:
        _validate_torrent_options(announce_url, name, piece_length)
    except ValueError as error:
        raise ValueError(f"torrent record is invalid: {error}") from error

    entries = manifest["files"]
    metainfo, info_hashes = _build_torrent(
        directory, entries, announce_url, name, piece_length, kind
    )
    expected_digest = hashlib.sha256(metainfo).hexdigest()
    actual_digest = _sha256_file(torrent_path)
    expected_hash_record = (
        {"sha1": record["info_hash_sha1"]}
        if record["format"] == TORRENT_FORMAT
        else record["info_hashes"]
    )
    if (
        expected_hash_record != info_hashes
        or record["torrent_sha256"] != expected_digest
    ):
        raise ValueError("torrent record identity does not match the fixture")
    if actual_digest != expected_digest:
        raise ValueError("torrent metainfo digest does not match its record")


def _parse_file(value: str) -> FixtureFile:
    name, separator, size = value.partition("=")
    if not separator:
        raise argparse.ArgumentTypeError("expected NAME=SIZE_BYTES")
    try:
        parsed_size = int(size, 10)
    except ValueError as error:
        raise argparse.ArgumentTypeError("file size must be an integer") from error
    if parsed_size < 0:
        raise argparse.ArgumentTypeError("file size must be nonnegative")
    return FixtureFile(name, parsed_size)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--seed", required=True)
    input_group = parser.add_mutually_exclusive_group(required=True)
    input_group.add_argument(
        "--file",
        action="append",
        type=_parse_file,
        metavar="NAME=SIZE_BYTES",
    )
    input_group.add_argument("--profile")
    parser.add_argument(
        "--profiles",
        type=Path,
        default=Path(__file__).with_name("fixture_profiles.json"),
        help="versioned fixture profile file (used with --profile)",
    )
    parser.add_argument("--announce-url")
    parser.add_argument("--torrent-name")
    parser.add_argument("--piece-length", type=int)
    parser.add_argument(
        "--torrent-kind",
        choices=("v1", "v2", "hybrid"),
        help="metainfo protocol; v2 and hybrid require --announce-url",
    )
    args = parser.parse_args(argv)
    try:
        profile_id = None
        if args.profile is not None:
            profile_id = args.profile
            profile = load_fixture_profile(args.profiles, profile_id)
            files = profile["files"]
            piece_length = profile["piece_length"]
            torrent_kind = profile["torrent_kind"]
            torrent_name = profile["torrent_name"]
            if args.piece_length is not None and args.piece_length != piece_length:
                raise ValueError("--piece-length cannot override a pinned fixture profile")
            if args.torrent_kind is not None and args.torrent_kind != torrent_kind:
                raise ValueError("--torrent-kind cannot override a pinned fixture profile")
            if args.torrent_name is not None and args.torrent_name != torrent_name:
                raise ValueError("--torrent-name cannot override a pinned fixture profile")
        else:
            files = args.file
            piece_length = (
                DEFAULT_PIECE_LENGTH if args.piece_length is None else args.piece_length
            )
            torrent_kind = args.torrent_kind or "v1"
            torrent_name = (
                "transfer-fixture" if args.torrent_name is None else args.torrent_name
            )
        manifest = generate_fixture(
            args.output,
            args.seed,
            files,
            announce_url=args.announce_url,
            torrent_name=torrent_name,
            piece_length=piece_length,
            torrent_kind=torrent_kind,
            profile_id=profile_id,
        )
    except (OSError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
