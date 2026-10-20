"""Regression coverage for cgroup resource accounting in the client benchmark."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import client_performance_benchmark as benchmark


class CgroupCounterTests(unittest.TestCase):
    def test_io_stat_sums_device_counters_and_ignores_unknown_fields(self) -> None:
        counters = benchmark.parse_cgroup_io_stat(
            "8:0 rbytes=100 wbytes=200 rios=3 wios=4 dbytes=50\n"
            "259:0 rbytes=500 wbytes=600 rios=7 wios=8\n"
        )

        self.assertEqual(counters, {
            "read_bytes": 600,
            "write_bytes": 800,
            "read_ops": 10,
            "write_ops": 12,
        })

    def test_cpu_usage_parser_returns_none_when_counter_is_unavailable(self) -> None:
        self.assertEqual(
            benchmark.parse_cgroup_cpu_usage_usec(
                "usage_usec 123456\nuser_usec 120000\nsystem_usec 3456\n"
            ),
            123456,
        )
        self.assertIsNone(benchmark.parse_cgroup_cpu_usage_usec("user_usec 10\n"))

    def test_resource_deltas_normalize_by_payload_gib(self) -> None:
        first = [0] * 33
        second = [0] * 33
        for row in (first, second):
            row[1] = "torrentngd"
            row[2] = "public_download"
        first[28:33] = [1_000_000, 10_000, 20_000, 10, 20]
        second[28:33] = [3_000_000, 110_000, 220_000, 110, 220]

        summary = benchmark.Benchmark._summary([first, second], "public_download")

        self.assertEqual(summary["cgroup_cpu_seconds"], 2.0)
        self.assertEqual(summary["cgroup_io_read_bytes"], 100_000)
        self.assertEqual(summary["cgroup_io_write_bytes"], 200_000)
        self.assertEqual(summary["cgroup_io_read_ops"], 100)
        self.assertEqual(summary["cgroup_io_write_ops"], 200)
        self.assertEqual(benchmark.per_gib(2.0, 1024 ** 3), 2.0)
        self.assertEqual(benchmark.per_gib(1024 ** 2, 1024 ** 3, 1024 ** 2), 1.0)
        self.assertIsNone(benchmark.per_gib(None, 1024 ** 3))

    def test_counter_reset_is_reported_as_unavailable(self) -> None:
        first = [0] * 33
        second = [0] * 33
        first[1] = second[1] = "torrentngd"
        first[2] = second[2] = "public_download"
        first[28] = 3_000_000
        second[28] = 1_000_000

        summary = benchmark.Benchmark._summary([first, second], "public_download")

        self.assertIsNone(summary["cgroup_cpu_seconds"])


if __name__ == "__main__":
    unittest.main()
