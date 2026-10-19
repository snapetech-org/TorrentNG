"""Regression sources for fixture-preserving task tracker retargeting."""

from __future__ import annotations

import hashlib
import json
import stat
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).parent / "transfer_rebuild"
sys.path.insert(0, str(TOOLS))

from metainfo import (  # noqa: E402
    MetainfoError,
    _top_level_entries,
    retarget_fixture_torrent,
    retarget_metainfo,
)


def _bencode(value: object) -> bytes:
    if isinstance(value, int):
        return b"i" + str(value).encode("ascii") + b"e"
    if isinstance(value, bytes):
        return str(len(value)).encode("ascii") + b":" + value
    if isinstance(value, list):
        return b"l" + b"".join(_bencode(item) for item in value) + b"e"
    if isinstance(value, dict):
        return b"d" + b"".join(
            _bencode(key) + _bencode(value[key]) for key in sorted(value)
        ) + b"e"
    raise TypeError(type(value).__name__)


def _torrent() -> tuple[bytes, bytes]:
    info = _bencode({
        b"files": [{b"length": 1, b"path": [b"payload.bin"]}],
        b"name": b"fixture",
        b"piece length": 16 * 1024,
        b"pieces": b"p" * 20,
        b"private": 1,
    })
    announce = _bencode(b"http://127.0.0.1:9/announce")
    # `info` is a raw bencoded dictionary, not a byte string containing one.
    torrent = b"d8:announce" + announce + b"4:info" + info + b"e"
    return torrent, info


def _fixture_record() -> dict[str, object]:
    identity: dict[str, object] = {
        "format": "transfer-fixture-v1",
        "generator": "shake256-counter-v1",
        "seed": "manifest-seed",
        "files": [
            {
                "name": "payload.bin",
                "size_bytes": 1,
                "sha256": hashlib.sha256(b"p").hexdigest(),
            }
        ],
    }
    canonical = json.dumps(identity, sort_keys=True, separators=(",", ":"), ensure_ascii=True)
    return {**identity, "fixture_sha256": hashlib.sha256(canonical.encode("ascii")).hexdigest()}


class MetainfoRetargetTests(unittest.TestCase):
    def test_retarget_preserves_exact_info_dictionary_and_hashes(self) -> None:
        source, info = _torrent()
        updated, hashes = retarget_metainfo(source, "http://198.18.4.1:51415/announce")
        before = dict(_top_level_entries(source))
        after = dict(_top_level_entries(updated))
        self.assertEqual(before[b"info"], info)
        self.assertEqual(after[b"info"], before[b"info"])
        self.assertEqual(hashes["sha1"], hashlib.sha1(info).hexdigest())
        self.assertEqual(hashes["sha256"], hashlib.sha256(info).hexdigest())

    def test_rejects_public_and_non_benchmark_tracker_endpoints(self) -> None:
        source, _ = _torrent()
        for announce in (
            "http://8.8.8.8:8080/announce",
            "http://198.20.0.1:8080/announce",
            "https://198.18.4.1:8080/announce",
        ):
            with self.subTest(announce=announce), self.assertRaises(MetainfoError):
                retarget_metainfo(source, announce)

    def test_rejects_alternate_tracker_and_webseed_fields(self) -> None:
        _, info = _torrent()
        source = _bencode({
            b"announce": b"http://127.0.0.1:9/announce",
            b"info": info,
            b"url-list": [b"http://example.invalid/payload"],
        })
        with self.assertRaisesRegex(MetainfoError, "alternate trackers"):
            retarget_metainfo(source, "http://198.18.4.1:51415/announce")

    def test_fixture_record_binds_source_digest_and_publishes_private_output(self) -> None:
        source, info = _torrent()
        fixture_record = _fixture_record()
        fixture_sha256 = fixture_record["fixture_sha256"]
        torrent_record = {
            "format": "transfer-torrent-v2",
            "announce_url": "http://127.0.0.1:9/announce",
            "name": "fixture",
            "piece_length": 16 * 1024,
            "metainfo_kind": "v1",
            "info_hashes": {"sha1": hashlib.sha1(info).hexdigest()},
            "torrent_sha256": hashlib.sha256(source).hexdigest(),
            "fixture_sha256": fixture_sha256,
        }
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            fixture = root / "fixture"
            fixture.mkdir()
            (fixture / "fixture.json").write_text(json.dumps(fixture_record))
            (fixture / "torrent.json").write_text(json.dumps(torrent_record))
            (fixture / "fixture.torrent").write_bytes(source)
            output = root / "retargeted.torrent"
            report = retarget_fixture_torrent(
                fixture,
                output,
                "http://198.18.4.1:51415/announce",
            )
            self.assertEqual(report["fixture_sha256"], fixture_sha256)
            self.assertEqual(report["info_hashes"], torrent_record["info_hashes"])
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
            with self.assertRaises(MetainfoError):
                retarget_fixture_torrent(
                    fixture,
                    output,
                    "http://198.18.4.1:51415/announce",
                )

    def test_rejects_fixture_metainfo_changed_after_generation(self) -> None:
        source, info = _torrent()
        fixture_record = _fixture_record()
        torrent_record = {
            "format": "transfer-torrent-v2",
            "announce_url": "http://127.0.0.1:9/announce",
            "name": "fixture",
            "piece_length": 16 * 1024,
            "metainfo_kind": "v1",
            "info_hashes": {"sha1": hashlib.sha1(info).hexdigest()},
            "torrent_sha256": "0" * 64,
            "fixture_sha256": fixture_record["fixture_sha256"],
        }
        with tempfile.TemporaryDirectory() as scratch:
            fixture = Path(scratch) / "fixture"
            fixture.mkdir()
            (fixture / "fixture.json").write_text(json.dumps(fixture_record))
            (fixture / "torrent.json").write_text(json.dumps(torrent_record))
            (fixture / "fixture.torrent").write_bytes(source)
            with self.assertRaisesRegex(MetainfoError, "changed after"):
                retarget_fixture_torrent(
                    fixture,
                    Path(scratch) / "retargeted.torrent",
                    "http://198.18.4.1:51415/announce",
                )

    def test_rejects_fixture_manifest_digest_mismatch(self) -> None:
        source, info = _torrent()
        fixture_record = _fixture_record()
        torrent_record = {
            "format": "transfer-torrent-v2",
            "announce_url": "http://127.0.0.1:9/announce",
            "name": "fixture",
            "piece_length": 16 * 1024,
            "metainfo_kind": "v1",
            "info_hashes": {"sha1": hashlib.sha1(info).hexdigest()},
            "torrent_sha256": hashlib.sha256(source).hexdigest(),
            "fixture_sha256": fixture_record["fixture_sha256"],
        }
        fixture_record["seed"] = "changed-after-digest"
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            fixture = root / "fixture"
            fixture.mkdir()
            (fixture / "fixture.json").write_text(json.dumps(fixture_record))
            (fixture / "torrent.json").write_text(json.dumps(torrent_record))
            (fixture / "fixture.torrent").write_bytes(source)
            with self.assertRaisesRegex(MetainfoError, "identity digest"):
                retarget_fixture_torrent(
                    fixture,
                    root / "retargeted.torrent",
                    "http://198.18.4.1:51415/announce",
                )

    def test_rejects_duplicate_manifest_keys(self) -> None:
        source, _ = _torrent()
        with tempfile.TemporaryDirectory() as scratch:
            root = Path(scratch)
            fixture = root / "fixture"
            fixture.mkdir()
            (fixture / "fixture.json").write_text('{"format":"one","format":"two"}')
            (fixture / "torrent.json").write_text("{}")
            (fixture / "fixture.torrent").write_bytes(source)
            with self.assertRaisesRegex(MetainfoError, "duplicate JSON member"):
                retarget_fixture_torrent(
                    fixture,
                    root / "retargeted.torrent",
                    "http://198.18.4.1:51415/announce",
                )


if __name__ == "__main__":
    unittest.main()
