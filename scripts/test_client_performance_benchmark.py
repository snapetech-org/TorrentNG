"""Regression coverage for cgroup resource accounting in the client benchmark."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

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


class SeedFixtureTests(unittest.TestCase):
    def test_peak_engine_metric_is_limited_to_the_selected_client_and_phases(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.engine_memory_metrics = [
            {
                "client": "torrentngd", "phase": "controlled_seed",
                "metrics": {"torrentng_storage_peer_read_cache_bytes": 7 * 1024 * 1024},
            },
            {
                "client": "torrentngd", "phase": "after_controlled_seed",
                "metrics": {"torrentng_storage_peer_read_cache_bytes": 8 * 1024 * 1024},
            },
            {
                "client": "torrentngd", "phase": "public_download",
                "metrics": {"torrentng_storage_peer_read_cache_bytes": 12 * 1024 * 1024},
            },
            {
                "client": "qbittorrent", "phase": "controlled_seed",
                "metrics": {"torrentng_storage_peer_read_cache_bytes": 64 * 1024 * 1024},
            },
        ]

        peak = instance._peak_engine_metric(
            "torrentngd",
            "torrentng_storage_peer_read_cache_bytes",
            {"controlled_seed", "after_controlled_seed"},
        )

        self.assertEqual(peak, 8 * 1024 * 1024)

    def test_peer_read_cache_byte_override_is_written_to_torrentng_config(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            instance = object.__new__(benchmark.Benchmark)
            instance.config_dir = Path(directory)
            instance.storage_io_worker_threads = None
            instance.storage_hash_worker_threads = None
            instance.storage_peer_read_readahead_bytes = None
            instance.storage_peer_read_cache_entries = None
            instance.storage_peer_read_cache_bytes = 4 * 1024 * 1024
            instance.piece_assembly_cap_mb = None
            instance.peer_buffer_cap_mb = None

            instance._prepare_torrentng_config()

            config = (Path(directory) / "torrentngd.config.toml").read_text()
            self.assertIn("peer_read_cache_bytes = 4194304", config)

    def test_file_cache_eviction_requests_dontneed_for_the_whole_fixture(self) -> None:
        if not hasattr(benchmark.os, "posix_fadvise"):
            self.skipTest("POSIX_FADV_DONTNEED is unavailable")
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "fixture.iso"
            fixture.write_bytes(b"benchmark fixture")
            with mock.patch.object(benchmark.os, "posix_fadvise") as fadvise:
                self.assertTrue(benchmark.evict_file_cache(fixture))

            fadvise.assert_called_once_with(
                mock.ANY, 0, 0, benchmark.os.POSIX_FADV_DONTNEED,
            )

    def test_file_cache_eviction_reports_unsupported_advice(self) -> None:
        if not hasattr(benchmark.os, "posix_fadvise"):
            self.skipTest("POSIX_FADV_DONTNEED is unavailable")
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "fixture.iso"
            fixture.write_bytes(b"benchmark fixture")
            with mock.patch.object(
                benchmark.os, "posix_fadvise", side_effect=OSError("unsupported"),
            ):
                self.assertFalse(benchmark.evict_file_cache(fixture))

    def test_rtorrent_status_queries_the_main_view_with_multicall2_target(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance._scgi = mock.Mock(return_value=[
            ["abc123", "fixture.iso", 1, 100, 50, 12, 3, 2],
        ])

        status = instance._status("rtorrent", "abc123", "fixture.iso")

        self.assertEqual(status, {
            "progress": 0.5, "down": 12, "up": 3, "peers": 2, "total": 100,
        })
        instance._scgi.assert_called_once_with(
            "d.multicall2", "", "main", "d.hash=", "d.name=", "d.complete=",
            "d.size_bytes=", "d.completed_bytes=", "d.down.rate=", "d.up.rate=",
            "d.peers_connected=",
        )

    def test_rtorrent_global_settings_include_the_empty_rpc_target(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance._scgi = mock.Mock()

        instance._configure("rtorrent")

        self.assertEqual(instance._scgi.call_args_list, [
            mock.call("throttle.max_peers.normal.set", "", benchmark.MAX_PEERS),
            mock.call("throttle.max_peers.seed.set", "", benchmark.MAX_PEERS),
            mock.call(
                "throttle.global_down.max_rate.set_kb", "", benchmark.RATE_BPS // 1024,
            ),
            mock.call(
                "throttle.global_up.max_rate.set_kb", "", benchmark.RATE_BPS // 1024,
            ),
        ])


if __name__ == "__main__":
    unittest.main()
