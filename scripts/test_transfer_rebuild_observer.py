"""Unit tests for the transfer rebuild's bounded resource observer."""

import importlib.util
import hashlib
import json
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

    def test_one_missing_sample_keeps_a_peak_null_instead_of_using_a_partial_maximum(self):
        summary = observer.PeakSummary()
        summary.observe(observer.Sample(0, 1, 100, 70, 60, 20, 10, 100, 4, 2, 3, 10, 11))
        summary.observe(observer.Sample(1, 1, None, 80, 55, 25, 15, 95, None, 2, 4, 8, 15))

        peaks = summary.as_json()["peaks"]
        self.assertIsNone(peaks["rss_bytes"])
        self.assertIsNone(peaks["cpu_time_ns"])
        self.assertEqual(peaks["pss_bytes"], 80)


class NetworkObservationTests(unittest.TestCase):
    @staticmethod
    def sample(namespace_inode: int, received: int, transmitted: int) -> observer.Sample:
        return observer.Sample(
            0,
            1,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            network_namespace_inode=namespace_inode,
            network_rx_bytes=received,
            network_tx_bytes=transmitted,
        )

    def test_reads_only_the_named_interface_from_the_process_network_namespace(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            process = root / "123"
            (process / "net").mkdir(parents=True)
            (process / "ns").mkdir()
            (process / "ns" / "net").write_text("namespace marker")
            (process / "net" / "dev").write_text(
                "Inter-| Receive | Transmit\n"
                " face |bytes packets errs drop fifo frame compressed multicast|"
                "bytes packets errs drop fifo colls carrier compressed\n"
                " veth0: 120 0 0 0 0 0 0 0 340 0 0 0 0 0 0 0\n"
                " eth0: 900 0 0 0 0 0 0 0 800 0 0 0 0 0 0 0\n"
            )

            inode, received, transmitted = observer._network_counters(123, "veth0", root)

            self.assertEqual(inode, (process / "ns" / "net").stat().st_ino)
            self.assertEqual((received, transmitted), (120, 340))

    def test_summary_reports_namespace_counter_deltas(self):
        summary = observer.PeakSummary("veth0")
        summary.observe(self.sample(77, 100, 200))
        summary.observe(self.sample(77, 145, 280))

        result = summary.as_json()
        self.assertEqual(result["network_interface"], "veth0")
        self.assertEqual(result["network_namespace_inode"], 77)
        self.assertEqual(result["network_rx_bytes"], 45)
        self.assertEqual(result["network_tx_bytes"], 80)
        self.assertIn("isolates exactly one client", result["network_note"])
        self.assertNotIn("network_rx_bytes", result["peaks"])

    def test_network_bytes_stay_null_if_the_interface_or_counter_epoch_changes(self):
        summary = observer.PeakSummary("veth0")
        summary.observe(self.sample(77, 100, 200))
        summary.observe(self.sample(77, 20, 280))  # interface counter reset

        changed_namespace = observer.PeakSummary("veth0")
        changed_namespace.observe(self.sample(77, 100, 200))
        changed_namespace.observe(self.sample(78, 120, 220))

        for result in (summary.as_json(), changed_namespace.as_json()):
            self.assertIsNone(result["network_rx_bytes"])
            self.assertIsNone(result["network_tx_bytes"])

    def test_unavailable_interface_keeps_network_bytes_null(self):
        summary = observer.PeakSummary("missing0")
        summary.observe(self.sample(77, 100, 200))
        summary.observe(
            observer.Sample(
                1,
                1,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
        )
        self.assertIsNone(summary.as_json()["network_rx_bytes"])
        self.assertIsNone(summary.as_json()["network_tx_bytes"])


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
                    with mock.patch.object(observer, "_process_start_ticks", return_value=987):
                        self.assertEqual(
                            observer.observe(123, csv_path, summary_path, 1, 1, None),
                            0,
                        )

            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            self.assertEqual(summary["schema_version"], "transfer-resource-observation-v1")
            self.assertEqual(summary["root_process_start_ticks"], 987)
            self.assertEqual(
                summary["samples_csv_sha256"],
                hashlib.sha256(csv_path.read_bytes()).hexdigest(),
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
