"""Regression sources for common transfer-trial measurement primitives."""

from __future__ import annotations

import tempfile
import sys
import unittest
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from measurement import (
    BarrierResult,
    FileStartBarrier,
    TrialTimeline,
    UsefulRangeLedger,
)


class FileStartBarrierTests(unittest.TestCase):
    def test_barrier_duration_rejects_a_backwards_clock(self) -> None:
        with self.assertRaises(ValueError):
            _ = BarrierResult(20, 10).wait_ns

    def test_participants_release_together_in_task_owned_directory(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-barrier-") as temporary:
            barrier = FileStartBarrier(Path(temporary), ["client-a", "client-b"])
            with ThreadPoolExecutor(max_workers=2) as workers:
                results = list(
                    workers.map(
                        lambda participant: barrier.wait(
                            participant, timeout_seconds=2.0
                        ),
                        ("client-a", "client-b"),
                    )
                )

            self.assertTrue((Path(temporary) / "released").is_file())
            self.assertEqual(len(results), 2)
            self.assertTrue(all(result.wait_ns >= 0 for result in results))

    def test_symlinked_readiness_or_release_markers_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-barrier-") as temporary:
            root = Path(temporary)
            target = root / "target"
            target.write_text("marker", encoding="utf-8")
            readiness_link = root / "client-b.ready"
            try:
                readiness_link.symlink_to(target)
            except (NotImplementedError, OSError) as error:
                self.skipTest(f"symlinks are unavailable: {error}")

            barrier = FileStartBarrier(root, ["client-a", "client-b"])
            with self.assertRaisesRegex(ValueError, "not a regular file"):
                barrier.wait("client-a", timeout_seconds=1.0)

        with tempfile.TemporaryDirectory(prefix="transfer-barrier-") as temporary:
            root = Path(temporary)
            target = root / "target"
            target.write_text("marker", encoding="utf-8")
            try:
                (root / "released").symlink_to(target)
            except (NotImplementedError, OSError) as error:
                self.skipTest(f"symlinks are unavailable: {error}")

            barrier = FileStartBarrier(root, ["client-a"])
            with self.assertRaisesRegex(ValueError, "not a regular file"):
                barrier.wait("client-a", timeout_seconds=1.0)

    def test_unknown_duplicate_or_unsafe_participants_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-barrier-") as temporary:
            root = Path(temporary)
            with self.assertRaises(ValueError):
                FileStartBarrier(root, [])
            with self.assertRaises(ValueError):
                FileStartBarrier(root, ["same", "same"])
            with self.assertRaises(ValueError):
                FileStartBarrier(root, ["../outside"])
            barrier = FileStartBarrier(root, ["known"])
            with self.assertRaises(ValueError):
                barrier.wait("unknown", timeout_seconds=1.0)


class UsefulRangeLedgerTests(unittest.TestCase):
    def test_counts_unique_bytes_and_duplicate_overlap_by_file(self) -> None:
        ledger = UsefulRangeLedger()
        self.assertEqual(ledger.accept(0, 0, 10), (10, 0))
        self.assertEqual(ledger.accept(0, 5, 10), (5, 5))
        self.assertEqual(ledger.accept(0, 3, 4), (0, 4))
        self.assertEqual(ledger.accept(1, 5, 10), (10, 0))
        self.assertEqual((ledger.useful_bytes, ledger.duplicate_bytes), (25, 9))

    def test_padding_and_empty_payload_do_not_count(self) -> None:
        ledger = UsefulRangeLedger()
        self.assertEqual(ledger.accept(0, 0, 4096, pad=True), (0, 0))
        self.assertEqual(ledger.accept(0, 0, 0), (0, 0))
        self.assertEqual((ledger.useful_bytes, ledger.duplicate_bytes), (0, 0))

    def test_negative_coordinates_are_rejected(self) -> None:
        with self.assertRaises(ValueError):
            UsefulRangeLedger().accept(0, -1, 1)


class TrialTimelineTests(unittest.TestCase):
    def test_reports_distinct_intervals_from_monotonic_markers(self) -> None:
        timeline = TrialTimeline()
        markers = {
            "barrier_wait_started": 10,
            "barrier_released": 20,
            "first_useful_byte": 30,
            "last_useful_byte": 80,
            "flush_started": 90,
            "flush_completed": 110,
            "validation_started": 120,
            "validation_completed": 150,
            "shutdown_started": 160,
            "shutdown_completed": 200,
        }
        for name, timestamp in markers.items():
            timeline.mark(name, timestamp)

        self.assertEqual(
            timeline.durations_ns(),
            {
                "barrier_wait": 10,
                "transfer": 50,
                "flush": 20,
                "validation": 30,
                "shutdown": 40,
            },
        )

    def test_missing_reversed_and_duplicate_markers_are_rejected(self) -> None:
        timeline = TrialTimeline()
        timeline.mark("barrier_wait_started", 10)
        with self.assertRaises(ValueError):
            timeline.mark("barrier_wait_started", 11)
        with self.assertRaises(ValueError):
            timeline.durations_ns()

        reversed_timeline = TrialTimeline()
        for name, timestamp in (
            ("barrier_wait_started", 20),
            ("barrier_released", 10),
            ("first_useful_byte", 30),
            ("last_useful_byte", 31),
            ("flush_started", 40),
            ("flush_completed", 41),
            ("validation_started", 50),
            ("validation_completed", 51),
            ("shutdown_started", 60),
            ("shutdown_completed", 61),
        ):
            reversed_timeline.mark(name, timestamp)
        with self.assertRaises(ValueError):
            reversed_timeline.durations_ns()

    def test_overlapping_phases_and_zero_transfer_are_rejected(self) -> None:
        def timeline_with(markers: dict[str, int]) -> TrialTimeline:
            timeline = TrialTimeline()
            for name, timestamp in markers.items():
                timeline.mark(name, timestamp)
            return timeline

        ordered_names = (
            "barrier_wait_started",
            "barrier_released",
            "first_useful_byte",
            "last_useful_byte",
            "flush_started",
            "flush_completed",
            "validation_started",
            "validation_completed",
            "shutdown_started",
            "shutdown_completed",
        )
        overlap = timeline_with(dict(zip(ordered_names, (0, 1, 2, 4, 3, 5, 6, 7, 8, 9))))
        with self.assertRaises(ValueError):
            overlap.durations_ns()

        zero_transfer = timeline_with(
            dict(zip(ordered_names, (0, 1, 2, 2, 3, 4, 5, 6, 7, 8)))
        )
        with self.assertRaises(ValueError):
            zero_transfer.durations_ns()


if __name__ == "__main__":
    unittest.main()
