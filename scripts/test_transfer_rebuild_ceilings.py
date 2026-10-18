"""Regression sources for independently derived transfer ceilings."""

from __future__ import annotations

import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from ceilings import (
    CeilingError,
    _write_exclusive,
    calculate_report,
    read_calibration_input,
)


SHA = "a" * 64


def calibration(protocol: str = "v1") -> dict[str, object]:
    return {
        "schema_version": "transfer-ceiling-input-v1",
        "calibration_id": "fixture-independent-01",
        "created_at": "2026-10-17T12:00:00Z",
        "environment": {
            "host_profile": "runner-a",
            "os": "Linux",
            "kernel": "6.8.0",
            "cpu_model": "test-cpu",
            "filesystem": "ext4",
            "device_id": "8:1",
            "network_profile_id": "lan-1g-1ms-v1",
        },
        "protocol": protocol,
        "network": {
            "download_bytes_per_second": 40_000_000,
            "upload_bytes_per_second": 35_000_000,
            "tool": "iperf3",
            "tool_version": "3.16",
            "tool_sha256": SHA,
            "raw_artifact_sha256": "b" * 64,
            "process_id": 101,
            "process_start_ticks": 10,
        },
        "disk": {
            "read_bytes_per_second": 55_000_000,
            "write_bytes_per_second": 45_000_000,
            "tool": "fio",
            "tool_version": "3.38",
            "tool_sha256": "c" * 64,
            "raw_artifact_sha256": "d" * 64,
            "process_id": 102,
            "process_start_ticks": 20,
        },
        "hash": {
            "sha1_bytes_per_second": 60_000_000,
            "sha256_bytes_per_second": 50_000_000,
            "hybrid_bytes_per_second": 30_000_000,
            "tool": "rt-hash-probe",
            "tool_version": "0.1",
            "tool_sha256": "e" * 64,
            "raw_artifact_sha256": "f" * 64,
            "process_id": 103,
            "process_start_ticks": 30,
        },
        "pipeline": {
            "peer_count": 4,
            "requests_per_peer": 32,
            "block_bytes": 16_384,
            "rtt_ns": 50_000_000,
            "rtt_source": "isolated-peer-rtt-v1",
            "raw_artifact_sha256": "1" * 64,
            "process_id": 104,
            "process_start_ticks": 40,
            "client_id": "torrentngd",
            "client_version": "0.1.0",
            "settings_sha256": SHA,
            "settings_json_pointer": "/request_queue_size",
        },
    }


class CeilingCalculationTests(unittest.TestCase):
    def test_download_and_upload_report_the_independent_bottleneck(self) -> None:
        report = calculate_report(calibration(), "2" * 64)

        self.assertAlmostEqual(report["components"]["pipeline"], 41_943_040)
        self.assertAlmostEqual(report["download"]["bytes_per_second"], 40_000_000)
        self.assertEqual(report["download"]["limiting_components"], ["network"])
        self.assertAlmostEqual(report["upload"]["bytes_per_second"], 35_000_000)
        self.assertEqual(report["upload"]["limiting_components"], ["network"])
        self.assertIn("not a performance claim", report["interpretation"])

    def test_protocol_selects_the_matching_hash_ceiling(self) -> None:
        v1 = calculate_report(calibration("v1"), "2" * 64)
        v2 = calculate_report(calibration("v2"), "2" * 64)
        hybrid = calculate_report(calibration("hybrid"), "2" * 64)

        self.assertEqual(v1["components"]["hash"], 60_000_000)
        self.assertEqual(v2["components"]["hash"], 50_000_000)
        self.assertEqual(hybrid["components"]["hash"], 30_000_000)
        self.assertEqual(hybrid["download"]["bytes_per_second"], 30_000_000)
        self.assertEqual(hybrid["download"]["limiting_components"], ["hash"])

    def test_pipeline_uses_peer_window_bytes_over_measured_rtt(self) -> None:
        record = calibration()
        record["pipeline"] = {
            "peer_count": 2,
            "requests_per_peer": 8,
            "block_bytes": 16_384,
            "rtt_ns": 100_000_000,
            "rtt_source": "private-peer-01",
            "raw_artifact_sha256": "1" * 64,
            "process_id": 104,
            "process_start_ticks": 40,
            "client_id": "torrentngd",
            "client_version": "0.1.0",
            "settings_sha256": SHA,
            "settings_json_pointer": "/request_queue_size",
        }

        report = calculate_report(record, "2" * 64)

        self.assertEqual(report["components"]["pipeline"], 2_621_440)
        self.assertEqual(report["download"]["limiting_components"], ["pipeline"])

    def test_missing_hybrid_measurement_and_non_finite_rates_fail_closed(self) -> None:
        record = calibration("hybrid")
        record["hash"]["hybrid_bytes_per_second"] = None
        with self.assertRaises(CeilingError):
            calculate_report(record, "2" * 64)

        record = calibration()
        record["network"]["download_bytes_per_second"] = float("nan")
        with self.assertRaises(CeilingError):
            calculate_report(record, "2" * 64)

    def test_boolean_rates_and_zero_pipeline_fields_are_rejected(self) -> None:
        record = calibration()
        record["disk"]["read_bytes_per_second"] = True
        with self.assertRaises(CeilingError):
            calculate_report(record, "2" * 64)

        record = calibration()
        record["pipeline"]["peer_count"] = 0
        with self.assertRaises(CeilingError):
            calculate_report(record, "2" * 64)

    def test_arbitrary_precision_inputs_cannot_overflow_derived_rates(self) -> None:
        record = calibration()
        record["network"]["download_bytes_per_second"] = 10**1000
        with self.assertRaises(CeilingError):
            calculate_report(record, "2" * 64)

        record = calibration()
        record["pipeline"]["peer_count"] = 10**1000
        with self.assertRaises(CeilingError):
            calculate_report(record, "2" * 64)


class CeilingArtifactTests(unittest.TestCase):
    def test_input_digest_and_exclusive_mode_0600_output(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-ceiling-") as temporary:
            root = Path(temporary)
            source = root / "input.json"
            source.write_text(json.dumps(calibration()), encoding="utf-8")
            observed, digest = read_calibration_input(source)
            self.assertEqual(digest, hashlib.sha256(source.read_bytes()).hexdigest())

            output = root / "report.json"
            report = calculate_report(observed, digest)
            _write_exclusive(output, report)
            self.assertEqual(os.stat(output).st_mode & 0o777, 0o600)
            with self.assertRaises(CeilingError):
                _write_exclusive(output, report)

    def test_input_and_output_symlinks_are_rejected(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-ceiling-") as temporary:
            root = Path(temporary)
            source = root / "input.json"
            target = root / "target.json"
            target.write_text(json.dumps(calibration()), encoding="utf-8")
            source.symlink_to(target)
            with self.assertRaises(CeilingError):
                read_calibration_input(source)

            output = root / "linked-report.json"
            output.symlink_to(target)
            with self.assertRaises(CeilingError):
                _write_exclusive(output, {"report": True})


if __name__ == "__main__":
    unittest.main()
