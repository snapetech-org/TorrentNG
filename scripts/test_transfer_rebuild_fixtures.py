"""Regression sources for deterministic transfer fixture generation."""

from __future__ import annotations

import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from cache_control import prepare_cache_state, write_record  # noqa: E402
from fixtures import (  # noqa: E402
    FixtureFile,
    _merkle_root,
    generate_fixture,
    load_fixture_profile,
    verify_fixture,
)


def _bdecode(data: bytes, offset: int = 0) -> tuple[object, int]:
    marker = data[offset : offset + 1]
    if marker == b"i":
        end = data.index(b"e", offset + 1)
        return int(data[offset + 1 : end]), end + 1
    if marker in {b"l", b"d"}:
        is_dict = marker == b"d"
        offset += 1
        values: list[object] = []
        pairs: dict[bytes, object] = {}
        while data[offset : offset + 1] != b"e":
            if is_dict:
                key, offset = _bdecode(data, offset)
                value, offset = _bdecode(data, offset)
                if not isinstance(key, bytes):
                    raise ValueError("bencoded dictionary key is not a byte string")
                pairs[key] = value
            else:
                value, offset = _bdecode(data, offset)
                values.append(value)
        return (pairs if is_dict else values), offset + 1
    colon = data.index(b":", offset)
    length = int(data[offset:colon])
    start = colon + 1
    end = start + length
    return data[start:end], end


def _decode(data: bytes) -> object:
    value, offset = _bdecode(data)
    if offset != len(data):
        raise ValueError("trailing bytes after bencoded value")
    return value


class FixtureGenerationTests(unittest.TestCase):
    def test_same_seed_produces_same_files_and_identity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary)
            specs = [FixtureFile("payload-a.bin", 4097), FixtureFile("empty.bin", 0)]
            first = generate_fixture(root / "first", "seed-7", specs)
            second = generate_fixture(root / "second", "seed-7", specs)

            self.assertEqual(first, second)
            for item in specs:
                left = (root / "first" / item.name).read_bytes()
                right = (root / "second" / item.name).read_bytes()
                self.assertEqual(left, right)
                entry = next(entry for entry in first["files"] if entry["name"] == item.name)
                self.assertEqual(hashlib.sha256(left).hexdigest(), entry["sha256"])

    def test_manifest_digest_covers_file_identity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary) / "fixture"
            manifest = generate_fixture(root, "stable", [FixtureFile("data.bin", 128)])
            saved = json.loads((root / "fixture.json").read_text(encoding="utf-8"))
            identity = {key: saved[key] for key in ("format", "generator", "seed", "files")}
            canonical = json.dumps(identity, sort_keys=True, separators=(",", ":"))
            self.assertEqual(
                hashlib.sha256(canonical.encode("ascii")).hexdigest(),
                manifest["fixture_sha256"],
            )
            self.assertEqual(verify_fixture(root), manifest)

    def test_payload_tampering_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary) / "fixture"
            generate_fixture(root, "stable", [FixtureFile("data.bin", 128)])
            (root / "data.bin").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "size mismatch|digest mismatch"):
                verify_fixture(root)

    def test_existing_destination_and_unsafe_names_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary)
            destination = root / "existing"
            destination.mkdir()
            with self.assertRaises(FileExistsError):
                generate_fixture(destination, "seed", [FixtureFile("data.bin", 1)])
            with self.assertRaises(ValueError):
                generate_fixture(root / "unsafe", "seed", [FixtureFile("../escape", 1)])
            with self.assertRaises(ValueError):
                generate_fixture(root / "reserved", "seed", [FixtureFile("fixture.json", 1)])

    def test_private_v1_metainfo_is_deterministic_and_fixture_bound(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary)
            specs = [FixtureFile("a.bin", 8193), FixtureFile("b.bin", 8193)]
            options = {
                "announce_url": "http://127.0.0.1:6969/announce",
                "torrent_name": "trial-payload",
                "piece_length": 16 * 1024,
            }
            first = generate_fixture(root / "first", "seed", specs, **options)
            second = generate_fixture(root / "second", "seed", specs, **options)

            first_torrent = (root / "first" / "fixture.torrent").read_bytes()
            second_torrent = (root / "second" / "fixture.torrent").read_bytes()
            first_record = json.loads((root / "first" / "torrent.json").read_text())
            second_record = json.loads((root / "second" / "torrent.json").read_text())
            self.assertEqual(first, second)
            self.assertEqual(first_torrent, second_torrent)
            self.assertEqual(first_record, second_record)
            self.assertEqual(first_record["format"], "transfer-torrent-v2")
            self.assertEqual(first_record["metainfo_kind"], "v1")
            self.assertEqual(first_record["fixture_sha256"], first["fixture_sha256"])
            self.assertEqual(
                hashlib.sha256(first_torrent).hexdigest(),
                first_record["torrent_sha256"],
            )
            self.assertEqual(verify_fixture(root / "first"), first)

    def test_private_v2_metainfo_has_authenticated_piece_layer_and_empty_file(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary)
            fixture = root / "fixture"
            manifest = generate_fixture(
                fixture,
                "seed-v2",
                [
                    FixtureFile("z.bin", 50_000),
                    FixtureFile("empty.bin", 0),
                    FixtureFile("a.bin", 20_000),
                ],
                announce_url="http://127.0.0.1:6969/announce",
                torrent_name="v2-payload",
                piece_length=32 * 1024,
                torrent_kind="v2",
            )
            record = json.loads((fixture / "torrent.json").read_text())
            metainfo = _decode((fixture / "fixture.torrent").read_bytes())
            self.assertIsInstance(metainfo, dict)
            info = metainfo[b"info"]
            self.assertEqual(info[b"meta version"], 2)
            self.assertNotIn(b"pieces", info)
            self.assertNotIn(b"files", info)
            self.assertIn(b"piece layers", metainfo)
            tree = info[b"file tree"]
            self.assertEqual(list(tree), [b"a.bin", b"empty.bin", b"z.bin"])
            empty_leaf = tree[b"empty.bin"][b""]
            self.assertEqual(empty_leaf[b"length"], 0)
            self.assertNotIn(b"pieces root", empty_leaf)

            z_root = tree[b"z.bin"][b""][b"pieces root"]
            layer = metainfo[b"piece layers"][z_root]
            self.assertEqual(len(layer), 2 * 32)
            layer_hashes = [layer[offset : offset + 32] for offset in range(0, len(layer), 32)]
            self.assertEqual(_merkle_root(layer_hashes), z_root)
            self.assertEqual(set(record["info_hashes"]), {"sha256", "sha256_truncated_20"})
            self.assertEqual(record["fixture_sha256"], manifest["fixture_sha256"])
            self.assertEqual(verify_fixture(fixture), manifest)

    def test_hybrid_metainfo_aligns_v1_files_and_matches_v2_order(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary)
            fixture = root / "fixture"
            manifest = generate_fixture(
                fixture,
                "seed-hybrid",
                [FixtureFile("b.bin", 18_000), FixtureFile("a.bin", 20_000)],
                announce_url="udp://127.0.0.1:6969/announce",
                torrent_name="hybrid-payload",
                piece_length=16 * 1024,
                torrent_kind="hybrid",
            )
            record = json.loads((fixture / "torrent.json").read_text())
            metainfo = _decode((fixture / "fixture.torrent").read_bytes())
            self.assertIsInstance(metainfo, dict)
            info = metainfo[b"info"]
            self.assertEqual(info[b"meta version"], 2)
            self.assertIn(b"pieces", info)
            self.assertIn(b"files", info)
            v1_files = info[b"files"]
            self.assertEqual(
                [item[b"path"][-1] for item in v1_files if item.get(b"attr") != b"p"],
                [b"a.bin", b"b.bin"],
            )
            pads = [item for item in v1_files if item.get(b"attr") == b"p"]
            self.assertEqual(len(pads), 1)
            self.assertEqual(pads[0][b"length"], 12_768)
            self.assertEqual(pads[0][b"path"], [b".pad", b"12768"])
            tree = info[b"file tree"]
            self.assertEqual(list(tree), [b"a.bin", b"b.bin"])
            self.assertEqual(set(record["info_hashes"]), {"sha1", "sha256", "sha256_truncated_20"})
            self.assertEqual(record["fixture_sha256"], manifest["fixture_sha256"])
            self.assertEqual(verify_fixture(fixture), manifest)

    def test_v2_or_hybrid_metainfo_requires_tracker(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary)
            for kind in ("v2", "hybrid"):
                with self.subTest(kind=kind):
                    with self.assertRaisesRegex(ValueError, "require a tracker URL"):
                        generate_fixture(
                            root / kind,
                            "seed",
                            [FixtureFile("data.bin", 1)],
                            torrent_kind=kind,
                        )
                    self.assertFalse((root / kind).exists())

    def test_fixture_profile_is_strict_and_bound_to_manifest_identity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixture-profile-") as temporary:
            root = Path(temporary)
            profile_path = root / "profiles.json"
            profile_path.write_text(
                json.dumps(
                    {
                        "format": "transfer-fixture-profiles-v1",
                        "profiles": {
                            "test/small": {
                                "files": [{"name": "data.bin", "size_bytes": 1024}],
                                "piece_length": 16 * 1024,
                                "torrent_kind": "v1",
                                "torrent_name": "profile-test",
                            }
                        },
                    }
                ),
                encoding="utf-8",
            )
            profile = load_fixture_profile(profile_path, "test/small")
            fixture = root / "fixture"
            manifest = generate_fixture(
                fixture,
                "profile-seed",
                profile["files"],
                torrent_name=profile["torrent_name"],
                piece_length=profile["piece_length"],
                torrent_kind=profile["torrent_kind"],
                profile_id="test/small",
            )
            self.assertEqual(manifest["profile"]["id"], "test/small")
            self.assertEqual(verify_fixture(fixture), manifest)

            saved = json.loads((fixture / "fixture.json").read_text(encoding="utf-8"))
            saved["profile"]["piece_length"] = 32 * 1024
            (fixture / "fixture.json").write_text(
                json.dumps(saved), encoding="utf-8"
            )
            with self.assertRaisesRegex(ValueError, "identity digest"):
                verify_fixture(fixture)

    def test_cache_state_record_is_observed_scoped_and_exclusive(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-cache-control-") as temporary:
            root = Path(temporary)
            fixture = root / "fixture"
            manifest = generate_fixture(
                fixture,
                "cache-seed",
                [FixtureFile("data.bin", 8193)],
            )
            record = prepare_cache_state(fixture, "warm")
            self.assertEqual(record["format"], "transfer-cache-control-v1")
            self.assertEqual(record["requested_state"], "warm")
            self.assertIn(record["status"], {"achieved", "partial", "unsupported"})
            self.assertEqual(record["fixture_sha256"], manifest["fixture_sha256"])
            if record["status"] != "unsupported":
                self.assertEqual(record["payload_bytes"], 8193)
                self.assertGreaterEqual(record["resident_bytes_after"], 0)
                self.assertLessEqual(record["resident_bytes_after"], 8193)

            report = root / "cache.json"
            write_record(report, record)
            if os.name == "posix":
                self.assertEqual(report.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(FileExistsError):
                write_record(report, record)

    def test_metainfo_tampering_and_credentialed_tracker_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-fixtures-") as temporary:
            root = Path(temporary)
            fixture = root / "fixture"
            generate_fixture(
                fixture,
                "seed",
                [FixtureFile("data.bin", 64)],
                announce_url="udp://127.0.0.1:6969/announce",
            )
            (fixture / "fixture.torrent").write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "metainfo digest"):
                verify_fixture(fixture)
            with self.assertRaisesRegex(ValueError, "without credentials"):
                generate_fixture(
                    root / "credentialed",
                    "seed",
                    [FixtureFile("data.bin", 1)],
                    announce_url="http://user:secret@127.0.0.1:6969/announce",
                )
            self.assertFalse((root / "credentialed").exists())


if __name__ == "__main__":
    unittest.main()
