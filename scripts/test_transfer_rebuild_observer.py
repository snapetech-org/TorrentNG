"""Unit tests for the transfer rebuild's bounded resource observer."""

import importlib.util
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock


SCRIPT = Path(__file__).parent / "transfer_rebuild" / "observe.py"
SPEC = importlib.util.spec_from_file_location("transfer_rebuild_observe", SCRIPT)
assert SPEC and SPEC.loader
observer = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = observer
SPEC.loader.exec_module(observer)


class PeakSummaryTests(unittest.TestCase):
    def test_summary_tracks_peaks_without_retaining_samples(self):
        summary = observer.PeakSummary()
        summary.observe(observer.Sample(0, 1, 100, 70, 60, 20, 10, 100, 4, 2, 3, 10, 11))
        summary.observe(observer.Sample(1, 1, 80, 90, 55, 25, 15, 95, 9, 2, 4, 8, 15))

        result = summary.as_json()
        self.assertEqual(result["sample_count"], 2)
        self.assertEqual(result["peaks"]["rss_bytes"], 100)
        self.assertEqual(result["peaks"]["pss_bytes"], 90)
        self.assertEqual(result["peaks"]["cgroup_reclaimable_cache_bytes"], 15)
        self.assertEqual(result["peaks"]["cpu_time_ns"], 9)
        self.assertFalse(hasattr(summary, "samples"))

    def test_missing_counters_remain_null(self):
        summary = observer.PeakSummary()
        summary.observe(observer.Sample(0, 1, None, None, None, None, None, None, None, None, None, None, None))

        self.assertIsNone(summary.as_json()["peaks"]["cgroup_anon_bytes"])
        self.assertIsNone(summary.as_json()["peaks"]["cgroup_reclaimable_cache_bytes"])
        self.assertIsNone(summary.as_json()["peaks"]["block_read_bytes"])


class ProcessTreeTests(unittest.TestCase):
    def test_process_tree_reads_direct_children(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for pid in (100, 101, 102):
                (root / str(pid) / "task" / str(pid)).mkdir(parents=True)
            (root / "100" / "task" / "100" / "children").write_text("101 102\n")
            (root / "101" / "task" / "101" / "children").write_text("")
            (root / "102" / "task" / "102" / "children").write_text("")

            self.assertEqual(observer.process_tree(100, root), [100, 101, 102])

    def test_process_tree_follows_children_spawned_by_non_leader_threads(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / "100" / "task" / "100").mkdir(parents=True)
            (root / "100" / "task" / "110").mkdir(parents=True)
            (root / "101" / "task" / "101").mkdir(parents=True)
            (root / "100" / "task" / "100" / "children").write_text("")
            (root / "100" / "task" / "110" / "children").write_text("101\n")
            (root / "101" / "task" / "101" / "children").write_text("")

            self.assertEqual(observer.process_tree(100, root), [100, 101])


class OutputArtifactTests(unittest.TestCase):
    def test_observer_outputs_are_private_and_never_overwritten(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            csv_path = root / "resources.csv"
            summary_path = root / "resources.json"
            sample = observer.Sample(1, 1, 100, 80, 60, 30, 10, 100, 5, 2, 3, None, None)

            with mock.patch.object(observer, "process_tree", side_effect=[[123], []]):
                with mock.patch.object(observer, "sample_process_tree", return_value=sample):
                    self.assertEqual(
                        observer.observe(123, csv_path, summary_path, 1, 1, None),
                        0,
                    )

            csv_before = csv_path.read_bytes()
            summary_before = summary_path.read_bytes()
            if os.name == "posix":
                self.assertEqual(csv_path.stat().st_mode & 0o777, 0o600)
                self.assertEqual(summary_path.stat().st_mode & 0o777, 0o600)

            with self.assertRaises(FileExistsError):
                observer.observe(123, csv_path, summary_path, 1, 1, None)

            self.assertEqual(csv_path.read_bytes(), csv_before)
            self.assertEqual(summary_path.read_bytes(), summary_before)


if __name__ == "__main__":
    unittest.main()
