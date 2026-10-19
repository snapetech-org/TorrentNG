"""Request and observe file-cache state for deterministic transfer fixtures."""

from __future__ import annotations

import argparse
import ctypes
import json
import mmap
import os
import stat
import sys
from pathlib import Path
from typing import Any

from fixtures import verify_fixture
from strict_json import StrictJsonError, loads as strict_json_loads


FORMAT = "transfer-cache-control-v1"
WINDOW_BYTES = 64 * 1024 * 1024
READ_BYTES = 1024 * 1024


class CacheControlError(ValueError):
    """The fixture or cache-control request is invalid."""


def _mincore_function() -> tuple[Any, int] | None:
    if (
        not sys.platform.startswith("linux")
        or not hasattr(os, "posix_fadvise")
        or not hasattr(os, "POSIX_FADV_DONTNEED")
        or not hasattr(os, "POSIX_FADV_WILLNEED")
    ):
        return None
    try:
        libc = ctypes.CDLL(None, use_errno=True)
        function = libc.mincore
    except (AttributeError, OSError):
        return None
    function.argtypes = (
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.POINTER(ctypes.c_ubyte),
    )
    function.restype = ctypes.c_int
    return function, os.sysconf("SC_PAGE_SIZE")


def _resident_bytes(fd: int, size_bytes: int, mincore: Any, page_bytes: int) -> int:
    resident_bytes = 0
    offset = 0
    while offset < size_bytes:
        window_payload = min(WINDOW_BYTES, size_bytes - offset)
        pages = (window_payload + page_bytes - 1) // page_bytes
        mapped_bytes = pages * page_bytes
        # Python requires the mapping length to fit inside the file, while
        # mincore rounds its requested length up to whole pages. Map only the
        # bytes that exist and still ask mincore about the final partial page.
        mapping_bytes = min(mapped_bytes, size_bytes - offset)
        mapping = mmap.mmap(fd, mapping_bytes, access=mmap.ACCESS_COPY, offset=offset)
        try:
            address = ctypes.addressof(ctypes.c_char.from_buffer(mapping))
            vector_type = ctypes.c_ubyte * pages
            vector = vector_type()
            result = mincore(
                ctypes.c_void_p(address),
                ctypes.c_size_t(window_payload),
                vector,
            )
            if result != 0:
                error = ctypes.get_errno()
                raise OSError(error, os.strerror(error))
            resident_pages = sum(1 for page in vector if page & 1)
            resident_bytes += min(window_payload, resident_pages * page_bytes)
        finally:
            mapping.close()
        offset += window_payload
    return resident_bytes


def _open_payload(path: Path, *, writable: bool) -> int:
    flags = os.O_RDWR if writable else os.O_RDONLY
    flags |= getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    return os.open(path, flags)


def _apply_state(path: Path, size_bytes: int, state: str) -> int:
    flags = os.POSIX_FADV_DONTNEED if state == "cold" else os.POSIX_FADV_WILLNEED
    descriptor = _open_payload(path, writable=state == "cold")
    observed = 0
    try:
        if state == "cold":
            # DONTNEED is allowed to discard clean pages only. Flush this
            # fixture file first; no global cache controls or privileged
            # /proc/sys writes are used.
            os.fsync(descriptor)
            os.posix_fadvise(descriptor, 0, 0, flags)
        else:
            os.posix_fadvise(descriptor, 0, 0, flags)
            while observed < size_bytes:
                chunk = os.read(descriptor, min(READ_BYTES, size_bytes - observed))
                if not chunk:
                    raise CacheControlError(f"fixture payload was truncated: {path.name}")
                observed += len(chunk)
        return observed
    finally:
        os.close(descriptor)


def _reuse_verified_fixture_manifest(
    fixture: Path, verified_manifest: dict[str, object]
) -> dict[str, object]:
    """Check a previously hashed fixture's identity and file layout cheaply.

    Paired trials reuse one payload fixture. Rehashing a multi-gigabyte fixture
    for every cache transition would add hours of unmeasured work and warm the
    cache before a cold run. The runner performs the full content check once;
    subsequent calls verify the immutable manifest identity and regular-file
    sizes without reading the payload bytes.
    """
    if fixture.is_symlink() or not fixture.is_dir():
        raise CacheControlError("fixture root must be an existing, non-symlink directory")
    if not isinstance(verified_manifest, dict):
        raise CacheControlError("verified fixture manifest must be an object")
    manifest_path = fixture / "fixture.json"
    try:
        descriptor = os.open(manifest_path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
        try:
            metadata = os.fstat(descriptor)
            if not stat.S_ISREG(metadata.st_mode) or metadata.st_size > 8 * 1024 * 1024:
                raise CacheControlError("fixture manifest must be a bounded regular file")
            raw = bytearray()
            while len(raw) <= 8 * 1024 * 1024:
                chunk = os.read(descriptor, min(64 * 1024, 8 * 1024 * 1024 + 1 - len(raw)))
                if not chunk:
                    break
                raw.extend(chunk)
        finally:
            os.close(descriptor)
        observed = strict_json_loads(raw, label="fixture manifest")
    except (OSError, json.JSONDecodeError, UnicodeDecodeError, StrictJsonError) as error:
        raise CacheControlError("cannot read the verified fixture manifest without following links") from error
    if observed != verified_manifest:
        raise CacheControlError("fixture manifest changed after its full content verification")
    files = verified_manifest.get("files")
    fixture_sha256 = verified_manifest.get("fixture_sha256")
    if not isinstance(files, list) or not isinstance(fixture_sha256, str):
        raise CacheControlError("verified fixture manifest has an invalid shape")
    for item in files:
        if not isinstance(item, dict):
            raise CacheControlError("verified fixture file entry is malformed")
        name = item.get("name")
        size_bytes = item.get("size_bytes")
        if (
            not isinstance(name, str)
            or not name
            or name.startswith("/")
            or any(part in {"", ".", ".."} for part in name.split("/"))
            or isinstance(size_bytes, bool)
            or not isinstance(size_bytes, int)
            or size_bytes < 0
        ):
            raise CacheControlError("verified fixture file identity is invalid")
        path = fixture.joinpath(*name.split("/"))
        try:
            metadata = path.lstat()
        except OSError as error:
            raise CacheControlError(f"verified fixture payload is missing: {name}") from error
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_size != size_bytes:
            raise CacheControlError(f"verified fixture payload layout changed: {name}")
    return verified_manifest


def prepare_cache_state(
    fixture: Path,
    state: str,
    *,
    verified_manifest: dict[str, object] | None = None,
) -> dict[str, object]:
    if state not in {"cold", "warm"}:
        raise CacheControlError("cache state must be cold or warm")
    manifest = (
        verify_fixture(fixture)
        if verified_manifest is None
        else _reuse_verified_fixture_manifest(Path(fixture), verified_manifest)
    )
    files = manifest["files"]
    payload_bytes = sum(int(item["size_bytes"]) for item in files)
    if payload_bytes <= 0:
        raise CacheControlError("cache control requires a non-empty fixture")

    mincore_info = _mincore_function()
    if mincore_info is None:
        return {
            "format": FORMAT,
            "fixture_sha256": manifest["fixture_sha256"],
            "requested_state": state,
            "status": "unsupported",
            "method": "unavailable",
            "page_bytes": None,
            "payload_bytes": payload_bytes,
            "resident_bytes_before": None,
            "resident_bytes_after": None,
            "files": [],
        }

    mincore, page_bytes = mincore_info
    file_results: list[dict[str, object]] = []
    total_before = 0
    total_after = 0
    for item in files:
        name = str(item["name"])
        size_bytes = int(item["size_bytes"])
        path = Path(fixture) / name
        descriptor = _open_payload(path, writable=False)
        try:
            before = _resident_bytes(descriptor, size_bytes, mincore, page_bytes)
        finally:
            os.close(descriptor)
        touched_bytes = _apply_state(path, size_bytes, state)
        descriptor = _open_payload(path, writable=False)
        try:
            after = _resident_bytes(descriptor, size_bytes, mincore, page_bytes)
        finally:
            os.close(descriptor)
        total_before += before
        total_after += after
        file_results.append(
            {
                "name": name,
                "payload_bytes": size_bytes,
                "resident_bytes_before": before,
                "resident_bytes_after": after,
                "bytes_read_for_warm_state": touched_bytes,
            }
        )

    achieved = total_after == 0 if state == "cold" else total_after == payload_bytes
    return {
        "format": FORMAT,
        "fixture_sha256": manifest["fixture_sha256"],
        "requested_state": state,
        "status": "achieved" if achieved else "partial",
        "method": "linux-posix-fadvise-mincore",
        "page_bytes": page_bytes,
        "payload_bytes": payload_bytes,
        "resident_bytes_before": total_before,
        "resident_bytes_after": total_after,
        "files": file_results,
    }


def write_record(path: Path, record: dict[str, object]) -> None:
    path = Path(path)
    if path.parent.is_symlink() or not path.parent.is_dir():
        raise CacheControlError("cache report parent must be an existing non-symlink directory")
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_CLOEXEC", 0)
    descriptor = os.open(path, flags, 0o600)
    try:
        os.fchmod(descriptor, 0o600)
        payload = (json.dumps(record, indent=2, sort_keys=True) + "\n").encode("utf-8")
        with os.fdopen(descriptor, "wb", closefd=False) as target:
            target.write(payload)
            target.flush()
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


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--state", choices=("cold", "warm"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        record = prepare_cache_state(args.fixture, args.state)
        write_record(args.output, record)
    except (OSError, CacheControlError, ValueError) as error:
        parser.error(str(error))
    print(json.dumps(record, indent=2, sort_keys=True))
    return 0 if record["status"] != "unsupported" else 2


if __name__ == "__main__":
    raise SystemExit(main())
