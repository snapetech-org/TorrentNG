"""Regression sources for PERF-110's isolated, pinned local tracker."""

from __future__ import annotations

import sys
import threading
import time
import unittest
import urllib.parse
from pathlib import Path

TOOLS = Path(__file__).parent / "transfer_rebuild"
sys.path.insert(0, str(TOOLS))

from task_tracker import TaskTracker, TaskTrackerError, _bencode  # noqa: E402


class TaskTrackerTests(unittest.TestCase):
    def setUp(self) -> None:
        self.seed_peer_id = b"-TNG100-" + b"s" * 12
        self.hash_v1 = b"v" * 20
        self.hash_v2 = b"w" * 32
        self.tracker = TaskTracker(
            gateway="198.18.4.1",
            target_address="198.18.4.3",
            seed_address="198.18.4.2",
            seed_port=51413,
            seed_peer_id=self.seed_peer_id,
            info_hashes={self.hash_v1, self.hash_v2},
            topology_sha256="a" * 64,
        )

    def test_target_gets_only_the_pinned_seed_as_a_compact_peer(self) -> None:
        query = (
            "info_hash=" + urllib.parse.quote_from_bytes(self.hash_v1, safe="")
            + "&peer_id=" + urllib.parse.quote_from_bytes(b"-CMP100-" + b"t" * 12, safe="")
            + "&port=45678&event=started&compact=1"
        )
        response = self.tracker.response("198.18.4.3", query)
        expected_peer = bytes((198, 18, 4, 2)) + (51413).to_bytes(2, "big")
        self.assertIn(b"5:peers" + str(len(expected_peer)).encode() + b":" + expected_peer, response)
        self.assertEqual(self.tracker.snapshot()["announce_counts"]["started"], 1)

    def test_seed_does_not_receive_itself_as_a_peer(self) -> None:
        query = (
            "info_hash=" + urllib.parse.quote_from_bytes(self.hash_v2, safe="")
            + "&peer_id=" + urllib.parse.quote_from_bytes(self.seed_peer_id, safe="")
            + "&port=51413&event=started&compact=1"
        )
        response = self.tracker.response("198.18.4.2", query)
        self.assertIn(b"5:peers0:", response)

    def test_rejects_foreign_hash_source_and_spoofed_seed(self) -> None:
        base = (
            "info_hash=" + urllib.parse.quote_from_bytes(b"x" * 20, safe="")
            + "&peer_id=" + urllib.parse.quote_from_bytes(b"t" * 20, safe="")
            + "&port=1234"
        )
        with self.assertRaisesRegex(TaskTrackerError, "not pinned"):
            self.tracker.response("198.18.4.3", base)
        pinned = (
            "info_hash=" + urllib.parse.quote_from_bytes(self.hash_v1, safe="")
            + "&peer_id=" + urllib.parse.quote_from_bytes(self.seed_peer_id, safe="")
            + "&port=1234"
        )
        with self.assertRaisesRegex(TaskTrackerError, "outside the task network"):
            self.tracker.response("198.18.4.9", pinned)
        with self.assertRaisesRegex(TaskTrackerError, "appeared in the target"):
            self.tracker.response("198.18.4.3", pinned)

    def test_rejects_malformed_percent_encoding(self) -> None:
        query = (
            "info_hash=%A&peer_id="
            + urllib.parse.quote_from_bytes(b"t" * 20, safe="")
            + "&port=1234"
        )
        with self.assertRaisesRegex(TaskTrackerError, "malformed percent encoding"):
            self.tracker.response("198.18.4.3", query)

        query = (
            "info_hash="
            + urllib.parse.quote_from_bytes(self.hash_v1, safe="")
            + "&peer_id=é&port=1234"
        )
        with self.assertRaisesRegex(TaskTrackerError, "percent-encode non-ASCII"):
            self.tracker.response("198.18.4.3", query)

        query = (
            "info_hash="
            + urllib.parse.quote_from_bytes(self.hash_v1, safe="")
            + "&peer_id="
            + urllib.parse.quote_from_bytes(b"t" * 20, safe="")
            + "&port=1234\x00"
        )
        with self.assertRaisesRegex(TaskTrackerError, "percent-encode control bytes"):
            self.tracker.response("198.18.4.3", query)

    def test_addresses_must_match_task_network_geometry(self) -> None:
        with self.assertRaisesRegex(TaskTrackerError, "benchmark subnet"):
            TaskTracker(
                gateway="198.18.4.1",
                target_address="198.18.5.3",
                seed_address="198.18.4.2",
                seed_port=51413,
                seed_peer_id=self.seed_peer_id,
                info_hashes={self.hash_v1},
                topology_sha256="a" * 64,
            )

    def test_prebound_tracker_accepts_one_hash_pin_after_fixture_creation(self) -> None:
        tracker = TaskTracker(
            gateway="198.18.4.1",
            target_address="198.18.4.3",
            seed_address="198.18.4.2",
            seed_port=51413,
            seed_peer_id=self.seed_peer_id,
            info_hashes=set(),
            topology_sha256="a" * 64,
        )
        tracker.pin_info_hashes({self.hash_v1})
        with self.assertRaisesRegex(TaskTrackerError, "only once"):
            tracker.pin_info_hashes({self.hash_v2})

    def test_seed_announce_pins_peer_identity_before_target_can_connect(self) -> None:
        tracker = TaskTracker(
            gateway="198.18.4.1",
            target_address="198.18.4.3",
            seed_address="198.18.4.2",
            seed_port=None,
            seed_peer_id=None,
            info_hashes={self.hash_v1},
            topology_sha256="a" * 64,
            seed_ready_timeout_seconds=1.0,
        )
        target_query = (
            "info_hash=" + urllib.parse.quote_from_bytes(self.hash_v1, safe="")
            + "&peer_id=" + urllib.parse.quote_from_bytes(b"-CMP100-" + b"t" * 12, safe="")
            + "&port=45678&event=started&compact=1"
        )
        target_responses: list[bytes] = []
        target_errors: list[BaseException] = []

        def announce_target() -> None:
            try:
                target_responses.append(tracker.response("198.18.4.3", target_query))
            except BaseException as error:
                target_errors.append(error)

        target_thread = threading.Thread(target=announce_target)
        target_thread.start()
        time.sleep(0.02)
        self.assertTrue(target_thread.is_alive(), "target announce must wait for seed identity")

        seed_query = (
            "info_hash=" + urllib.parse.quote_from_bytes(self.hash_v1, safe="")
            + "&peer_id=" + urllib.parse.quote_from_bytes(self.seed_peer_id, safe="")
            + "&port=51413&event=started&compact=1"
        )
        seed_response = tracker.response("198.18.4.2", seed_query)
        self.assertIn(b"5:peers0:", seed_response)
        target_thread.join(timeout=1.0)
        self.assertFalse(target_thread.is_alive(), "seed announce must release the target wait")
        self.assertEqual(target_errors, [])
        self.assertEqual(len(target_responses), 1)
        target_response = target_responses[0]
        expected_peer = bytes((198, 18, 4, 2)) + (51413).to_bytes(2, "big")
        self.assertIn(b"5:peers" + str(len(expected_peer)).encode() + b":" + expected_peer, target_response)
        self.assertTrue(tracker.snapshot()["seed_ready"])
        self.assertEqual(tracker.snapshot()["seed_port"], 51413)

        changed_port = seed_query.replace("port=51413", "port=51414")
        with self.assertRaisesRegex(TaskTrackerError, "port changed"):
            tracker.response("198.18.4.2", changed_port)

    def test_target_announce_fails_after_bounded_seed_readiness_wait(self) -> None:
        tracker = TaskTracker(
            gateway="198.18.4.1",
            target_address="198.18.4.3",
            seed_address="198.18.4.2",
            seed_port=None,
            seed_peer_id=None,
            info_hashes={self.hash_v1},
            topology_sha256="a" * 64,
            seed_ready_timeout_seconds=0.01,
        )
        query = (
            "info_hash=" + urllib.parse.quote_from_bytes(self.hash_v1, safe="")
            + "&peer_id=" + urllib.parse.quote_from_bytes(b"-CMP100-" + b"t" * 12, safe="")
            + "&port=45678&event=started&compact=1"
        )
        with self.assertRaisesRegex(TaskTrackerError, "readiness deadline"):
            tracker.response("198.18.4.3", query)

    def test_seedless_target_mode_returns_an_empty_peer_set_for_direct_peer_runner(self) -> None:
        tracker = TaskTracker(
            gateway="198.18.4.1",
            target_address="198.18.4.3",
            seed_address="198.18.4.2",
            seed_port=None,
            seed_peer_id=None,
            info_hashes={self.hash_v1},
            topology_sha256="a" * 64,
            allow_seedless_target=True,
        )
        query = (
            "info_hash=" + urllib.parse.quote_from_bytes(self.hash_v1, safe="")
            + "&peer_id=" + urllib.parse.quote_from_bytes(b"-CMP100-" + b"t" * 12, safe="")
            + "&port=45678&event=started&compact=1"
        )
        response = tracker.response("198.18.4.3", query)
        self.assertIn(b"5:peers0:", response)
        self.assertFalse(tracker.snapshot()["seed_ready"])
        self.assertTrue(tracker.snapshot()["seedless_target_mode"])

    def test_bencode_is_canonical_and_rejects_boolean_counters(self) -> None:
        self.assertEqual(_bencode({b"z": 1, b"a": b"x"}), b"d1:a1:x1:zi1ee")
        with self.assertRaises(TypeError):
            _bencode(True)


if __name__ == "__main__":
    unittest.main()
