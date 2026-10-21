"""Regression coverage for cgroup resource accounting in the client benchmark."""

from __future__ import annotations

import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

sys.path.insert(0, str(Path(__file__).parent))
import client_performance_benchmark as benchmark


class CgroupCounterTests(unittest.TestCase):
    def test_benchmark_build_uses_a_task_scoped_buildkit_builder(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.project = "tngbench20261020t000000z"
        instance.env = {}
        instance.buildx_builder = None

        with mock.patch.object(benchmark, "run_command", return_value=("", 0)) as command:
            builder = instance._create_task_buildx_builder()

        self.assertEqual(instance.buildx_builder, builder)
        self.assertIn("-build-", builder)
        command.assert_called_once_with(
            ["docker", "buildx", "create", "--driver", "docker-container",
             "--driver-opt", "cpu-period=100000",
             "--driver-opt", "cpu-quota=200000",
             "--name", builder],
            env={}, quiet=True, timeout=120,
        )

    def test_failed_task_builder_creation_does_not_claim_existing_builder(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.project = "tngbench20261020t000000z"
        instance.env = {}
        instance.buildx_builder = None

        with mock.patch.object(
            benchmark, "run_command", side_effect=RuntimeError("name already exists"),
        ):
            with self.assertRaisesRegex(RuntimeError, "name already exists"):
                instance._create_task_buildx_builder()

        self.assertIsNone(instance.buildx_builder)

    def test_torrentng_build_selects_the_task_scoped_builder(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.image = "torrentng/torrentngd:benchmark-test"
        instance._create_task_buildx_builder = mock.Mock(return_value="task-builder")
        instance.compose = mock.Mock()
        instance._assert_image_labels = mock.Mock()

        instance._build_torrentng_image()

        instance.compose.assert_called_once_with(
            "build", "--builder", "task-builder", "torrentngd", timeout=3600,
        )
        instance._assert_image_labels.assert_called_once_with(instance.image)

    def test_task_builder_cleanup_removes_only_its_builder_and_cache(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.env = {}
        instance.buildx_builder = "tngbench-build-1234"

        with mock.patch.object(benchmark, "run_command", return_value=("", 0)) as command:
            status = instance._remove_task_buildx_builder()

        self.assertEqual(status, "task-scoped BuildKit builder and cache removed")
        self.assertIsNone(instance.buildx_builder)
        command.assert_called_once_with(
            ["docker", "buildx", "rm", "--force", "tngbench-build-1234"],
            env={}, check=False, quiet=True, timeout=120,
        )

    def test_task_builder_cleanup_failure_remains_visible(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.env = {}
        instance.buildx_builder = "tngbench-build-1234"
        with mock.patch.object(benchmark, "run_command", return_value=("error", 1)):
            status = instance._remove_task_buildx_builder()

        self.assertIn("cleanup failed", status)
        self.assertEqual(instance.buildx_builder, "tngbench-build-1234")

    def test_transfer_compose_override_disables_healthchecks_symmetrically(self) -> None:
        rendered = benchmark.render_benchmark_compose_override({
            "TOKIO_WORKER_THREADS": "2",
        })

        for client in benchmark.CLIENTS:
            with self.subTest(client=client):
                self.assertIn(f"  {client}:\n", rendered)
        self.assertIn('      TOKIO_WORKER_THREADS: "2"', rendered)
        self.assertEqual(rendered.count("    healthcheck:"), len(benchmark.CLIENTS))

    def test_idle_profile_schedule_is_seeded_and_balances_all_clients(self) -> None:
        first = object.__new__(benchmark.Benchmark)
        first.idle_order_seed = 12345
        second = object.__new__(benchmark.Benchmark)
        second.idle_order_seed = 12345

        schedule = first._idle_profile_order()

        self.assertEqual(schedule, second._idle_profile_order())
        self.assertCountEqual(
            schedule,
            [(client, profile) for client in benchmark.CLIENTS for profile in benchmark.IDLE_PROFILES],
        )

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

    def test_storage_backend_metrics_capture_uring_and_fixed_buffer_capabilities(self) -> None:
        metrics = benchmark.parse_torrentng_storage_backend_metrics(
            '# HELP torrentng_storage_runtime_initialized initialized\n'
            'torrentng_storage_runtime_initialized 1\n'
            'torrentng_storage_backend_selected{backend="uring"} 1\n'
            'torrentng_storage_backend_fixed_buffers_supported 1\n'
            'torrentng_storage_backend_registered_files_supported 1\n'
            'torrentng_storage_backend_max_batch_len 32\n'
            'torrentng_storage_backend_fixed_buffer_bytes 262144\n'
            'torrentng_storage_backend_fixed_buffer_strategy{strategy="frame_pool_slots"} 1\n'
            'torrentng_storage_backend_fixed_buffer_worker_copy 0\n'
            'torrentng_storage_backend_frame_pool_slots_supported 1\n'
        )

        self.assertEqual(metrics, {
            "runtime_initialized": True,
            "selected_backend": "uring",
            "fixed_buffers_supported": True,
            "registered_files_supported": True,
            "max_batch_len": 32,
            "fixed_buffer_bytes": 262144,
            "fixed_buffer_strategy": "frame_pool_slots",
            "fixed_buffer_worker_copy": False,
            "frame_pool_slots_supported": True,
        })
        self.assertIn("backend=", benchmark.describe_torrentng_storage_backend({"metrics": metrics}))

    def test_storage_backend_metrics_capture_uninitialized_runtime(self) -> None:
        metrics = benchmark.parse_torrentng_storage_backend_metrics(
            "torrentng_storage_runtime_initialized 0\n"
        )

        self.assertEqual(metrics, {"runtime_initialized": False})
        self.assertIn(
            "not initialized",
            benchmark.describe_torrentng_storage_backend({"metrics": metrics}),
        )

    def test_post_transfer_storage_backend_scrape_runs_only_for_torrentng(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance._torrentng_request = mock.Mock(return_value=(
            b'torrentng_storage_runtime_initialized 1\n'
            b'torrentng_storage_backend_selected{backend="pread"} 1\n'
            b'torrentng_storage_backend_fixed_buffers_supported 0\n'
            b'torrentng_storage_backend_registered_files_supported 0\n'
        ))

        observation = instance._capture_storage_backend_observation("torrentngd")

        self.assertEqual(
            observation["phase"],
            "after_torrentng_seed_window_before_torrent_pause",
        )
        self.assertEqual(observation["metrics"]["selected_backend"], "pread")
        instance._torrentng_request.assert_called_once_with("/metrics")

        instance._torrentng_request.reset_mock()
        self.assertIsNone(instance._capture_storage_backend_observation("qbittorrent"))
        instance._torrentng_request.assert_not_called()

    def test_storage_backend_is_captured_before_pausing_torrentng(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        events = []
        instance._capture_storage_backend_observation = mock.Mock(
            side_effect=lambda client: events.append(("backend", client)),
        )
        instance._pause = mock.Mock(
            side_effect=lambda client, info_hash: events.append(("pause", client, info_hash)),
        )
        instance._capture_engine_memory_metrics = mock.Mock(
            side_effect=lambda client, phase: events.append(("memory", client, phase)),
        )

        instance._finish_transfer_window(
            "torrentngd", "torrent-hash", "after_controlled_seed",
        )

        self.assertEqual(events, [
            ("backend", "torrentngd"),
            ("pause", "torrentngd", "torrent-hash"),
            ("memory", "torrentngd", "after_controlled_seed"),
        ])

    def test_network_namespace_parser_sums_non_loopback_interfaces(self) -> None:
        received, transmitted = benchmark.parse_proc_net_dev(
            "Inter-| Receive | Transmit\n"
            " face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n"
            "lo: 900 9 0 0 0 0 0 0 800 8 0 0 0 0 0 0\n"
            "eth0: 1000 10 0 0 0 0 0 0 2000 20 0 0 0 0 0 0\n"
            "veth1: 300 3 0 0 0 0 0 0 400 4 0 0 0 0 0 0\n"
        )

        self.assertEqual((received, transmitted), (1300, 2400))

    def test_network_namespace_parser_returns_none_without_interfaces(self) -> None:
        self.assertEqual(
            benchmark.parse_proc_net_dev(
                "Inter-| Receive | Transmit\n"
                " face |bytes packets errs drop fifo frame compressed multicast|bytes packets errs drop fifo colls carrier compressed\n"
                "lo: 900 9 0 0 0 0 0 0 800 8 0 0 0 0 0 0\n"
            ),
            (None, None),
        )

    def test_network_counters_are_zero_only_after_verified_isolation(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.network_isolated_clients = {"torrentngd"}
        instance.container_pids = {}

        self.assertEqual(instance._container_network_bytes("torrentngd"), (0, 0))
        self.assertEqual(instance._container_network_bytes("qbittorrent"), (None, None))

    def test_idle_network_isolation_detaches_and_restores_original_endpoints(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.env = {}
        instance.detached_networks = {}
        instance.network_isolated_clients = set()
        original = {
            "benchmark_interop": {
                "Aliases": ["torrentngd", "benchmark-torrentngd"],
            },
        }
        instance._container_networks = mock.Mock(side_effect=[original, {}, {}, original])

        with mock.patch.object(benchmark, "run_command") as command:
            instance._disconnect_container_networks("torrentngd", "container-id")
            self.assertIn("torrentngd", instance.network_isolated_clients)
            instance._reconnect_container_networks("torrentngd")

        self.assertNotIn("torrentngd", instance.network_isolated_clients)
        self.assertNotIn("torrentngd", instance.detached_networks)
        command.assert_has_calls([
            mock.call(
                ["docker", "network", "disconnect", "benchmark_interop", "container-id"],
                env={}, timeout=30,
            ),
            mock.call(
                ["docker", "network", "connect", "--alias", "torrentngd",
                 "--alias", "benchmark-torrentngd", "benchmark_interop", "container-id"],
                env={}, timeout=30,
            ),
        ])

    def test_resource_deltas_normalize_by_payload_gib(self) -> None:
        first = [0] * 39
        second = [0] * 39
        for row in (first, second):
            row[1] = "torrentngd"
            row[2] = "public_download"
        first[28:33] = [1_000_000, 10_000, 20_000, 10, 20]
        second[28:33] = [3_000_000, 110_000, 220_000, 110, 220]
        first[35:37] = [2 * 1024 * 1024, 1024 * 1024]
        second[35:37] = [3 * 1024 * 1024, 2 * 1024 * 1024]

        summary = benchmark.Benchmark._summary([first, second], "public_download")

        self.assertEqual(summary["cgroup_cpu_seconds"], 2.0)
        self.assertEqual(summary["cgroup_io_read_bytes"], 100_000)
        self.assertEqual(summary["cgroup_io_write_bytes"], 200_000)
        self.assertEqual(summary["cgroup_io_read_ops"], 100)
        self.assertEqual(summary["cgroup_io_write_ops"], 200)
        self.assertEqual(summary["peak_process_tree_rss"], 3 * 1024 * 1024)
        self.assertEqual(summary["peak_process_tree_pss"], 2 * 1024 * 1024)
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

    def test_idle_profile_summary_reports_absolute_resource_cost_per_minute(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        first = [0] * 35
        second = [0] * 35
        for row, elapsed in ((first, "0"), (second, "60")):
            row[0] = "2026-10-20T00:00:00+00:00"
            row[1] = "torrentngd"
            row[2] = "idle_dht_on"
            row[3] = elapsed
            row[9] = 10 * 1024 * 1024
            row[11] = 12 * 1024 * 1024
            row[13] = 8 * 1024 * 1024
            row[14] = 2 * 1024 * 1024
        second[28] = 30_000_000
        second[29] = 1024 * 1024
        second[30] = 2 * 1024 * 1024
        second[31] = 4
        second[32] = 5
        second[33] = 30_000
        second[34] = 60_000
        for row, rss, pss, processes, threads in (
            (first, 10 * 1024 * 1024, 8 * 1024 * 1024, 2, 3),
            (second, 12 * 1024 * 1024, 9 * 1024 * 1024, 3, 4),
        ):
            row.extend([rss, pss, processes, threads])
        other_client = [0] * 39
        other_client[1] = "qbittorrent"
        other_client[2] = "idle_dht_on"
        other_client[3] = "120"
        other_client[8] = "99"
        other_client[9] = 99 * 1024 * 1024
        other_client[11] = 99 * 1024 * 1024
        other_client[13] = 99 * 1024 * 1024
        instance.samples = [first, second, other_client]

        summary = instance._idle_profile_summary("torrentngd", "idle_dht_on")

        self.assertEqual(summary["cgroup_cpu_seconds_per_minute"], 30.0)
        self.assertEqual(summary["cgroup_cpu_percent_one_core"], 50.0)
        self.assertEqual(summary["block_read_mib_per_minute"], 1.0)
        self.assertEqual(summary["block_write_mib_per_minute"], 2.0)
        self.assertAlmostEqual(summary["network_rx_mib_per_minute"], 30_000 / 1048576)
        self.assertAlmostEqual(summary["network_tx_mib_per_minute"], 60_000 / 1048576)
        self.assertEqual(summary["mean_process_tree_rss"], 11 * 1024 * 1024)
        self.assertEqual(summary["peak_process_tree_rss"], 12 * 1024 * 1024)
        self.assertEqual(summary["mean_process_tree_pss"], 8.5 * 1024 * 1024)
        self.assertEqual(summary["peak_process_tree_pss"], 9 * 1024 * 1024)
        self.assertEqual(summary["peak_process_tree_count"], 3)
        self.assertEqual(summary["peak_process_tree_threads"], 4)
        self.assertEqual(summary["process_tree_memory_sample_count"], 2)

    def test_container_process_tree_memory_uses_the_shared_proc_observer(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance.container_pids = {"qbittorrent": 4321}
        observed = SimpleNamespace(
            rss_bytes=2_000, pss_bytes=1_500, processes=3, threads=8,
        )
        with mock.patch.object(benchmark, "sample_process_tree", return_value=observed) as sample:
            values = instance._container_process_tree_memory("qbittorrent")

        sample.assert_called_once_with(4321, mock.ANY)
        self.assertEqual(values, (2_000, 1_500, 3, 8))

    def test_transfer_process_tree_memory_is_sampled_at_its_own_interval(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            instance = object.__new__(benchmark.Benchmark)
            instance.mode = "transfer"
            instance._stats = mock.Mock(return_value=(0.0, 1024))
            instance._memory_components = mock.Mock(return_value=(
                {
                    "current": 1024, "peak": 1024, "anon": 512, "file": 256,
                    "slab": 0, "sock": 0, "kernel_stack": 0, "pagetables": 0,
                },
                {
                    "rss": 512, "hwm": 512, "rss_anon": 256, "rss_file": 128,
                    "rss_shmem": 0, "vm_data": 256, "vm_stk": 0, "vm_pte": 0,
                    "threads": 1, "thread_names": {}, "smaps_rollup": {}, "mappings": [],
                },
            ))
            instance._read_cgroup_cpu = mock.Mock(return_value=1000)
            instance._read_cgroup_io = mock.Mock(return_value=None)
            instance._container_network_bytes = mock.Mock(return_value=(0, 0))
            instance._container_process_tree_memory = mock.Mock(
                return_value=(2_000, 1_500, 3, 8),
            )
            instance.cgroup_paths = {}
            instance.process_memory_snapshots = []
            instance.samples = []
            instance.csv_path = Path(directory) / "samples.csv"

            status = {"progress": 0.0, "down": 0, "up": 0, "peers": 0}
            first = instance._sample("qbittorrent", "controlled_seed", 0.0, status)
            middle = instance._sample("qbittorrent", "controlled_seed", 5.0, status)
            next_sample = instance._sample("qbittorrent", "controlled_seed", 15.0, status)

            self.assertEqual(first[35:39], [2_000, 1_500, 3, 8])
            self.assertEqual(middle[35:39], ["", "", "", ""])
            self.assertEqual(next_sample[35:39], [2_000, 1_500, 3, 8])
            self.assertEqual(instance._container_process_tree_memory.call_count, 2)

    def test_idle_report_emits_one_value_per_table_column(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            instance = object.__new__(benchmark.Benchmark)
            instance.started = "2026-10-20T00:00:00+00:00"
            instance.source_commit = "abc123"
            instance.source_tree = "def456"
            instance.source_dirty = False
            instance.source_fingerprint_sha256 = "0" * 64
            instance.host = "benchmark-host"
            instance.cpu_count = 8
            instance.kernel = "test-kernel"
            instance.machine = "x86_64"
            instance.scratch_device = "/dev/test"
            instance.scratch_type = "ext4"
            instance.load_start = (0.1, 0.2, 0.3)
            instance.idle_order_seed = 123
            instance.idle_results = [{
                "client": "torrentngd",
                "profile": "network_isolated_idle",
                "profile_key": "isolated",
                "network_mode": "isolated",
                "network_isolation_verified": True,
                "dht_enabled": False,
                "dht_readback": False,
                "pex_readback": False,
                "network_traffic_observed": False,
                "sample_count": 2,
                "status": "PASS",
                "cgroup_cpu_seconds_per_minute": 1.0,
                "cgroup_cpu_percent_one_core": 1.67,
                "mean_process_tree_rss": 2 * 1024 * 1024,
                "peak_process_tree_rss": 3 * 1024 * 1024,
                "mean_process_tree_pss": 1024 * 1024,
                "peak_process_tree_pss": 2 * 1024 * 1024,
                "mean_cgroup_anon": 2 * 1024 * 1024,
                "peak_cgroup_anon": 3 * 1024 * 1024,
                "peak_cgroup_current": 4 * 1024 * 1024,
                "peak_memory": 4 * 1024 * 1024,
                "block_read_mib_per_minute": 0.1,
                "block_write_mib_per_minute": 0.2,
                "network_rx_mib_per_minute": 0.0,
                "network_tx_mib_per_minute": 0.0,
            }]
            sample = [0] * 39
            sample[1] = "torrentngd"
            sample[2] = "idle_isolated"
            sample[11] = 2 * 1024 * 1024
            instance.samples = [sample]
            instance.csv_path = Path(directory) / "samples.csv"
            instance.memory_profile = Path(directory) / "memory.json"
            instance.report = Path(directory) / "report.md"
            instance.cleanup_status = "not run"

            instance._write_idle_report({"torrentngd": "image:sha256:test"})

            table = [
                line for line in instance.report.read_text().splitlines()
                if line.startswith("|")
            ]
            self.assertEqual(len(table), 3)
            column_counts = [len(line.strip("|").split("|")) for line in table]
            self.assertEqual(column_counts, [18, 18, 18])
            self.assertIn("| TorrentNG | network_isolated_idle |", table[2])

    def test_idle_sample_does_not_query_torrentng_metrics_during_the_window(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            instance = object.__new__(benchmark.Benchmark)
            instance.mode = "idle"
            instance._stats = mock.Mock(return_value=(0.0, 1024))
            instance._memory_components = mock.Mock(return_value=(
                {
                    "current": 1024, "peak": 1024, "anon": 512, "file": 256,
                    "slab": 0, "sock": 0, "kernel_stack": 0, "pagetables": 0,
                },
                {
                    "rss": 512, "hwm": 512, "rss_anon": 256, "rss_file": 128,
                    "rss_shmem": 0, "vm_data": 256, "vm_stk": 0, "vm_pte": 0,
                    "threads": 1, "thread_names": {}, "smaps_rollup": {}, "mappings": [],
                },
            ))
            instance._read_cgroup_cpu = mock.Mock(return_value=1000)
            instance._read_cgroup_io = mock.Mock(return_value=None)
            instance._container_network_bytes = mock.Mock(return_value=(0, 0))
            instance._capture_engine_memory_metrics = mock.Mock()
            instance.cgroup_paths = {}
            instance.container_ids = {"torrentngd": "container"}
            instance.torrentng_pids = {"torrentngd": 1}
            instance.process_memory_snapshots = []
            instance.last_engine_memory_metrics_at = 0.0
            instance.samples = []
            instance.csv_path = Path(directory) / "samples.csv"

            instance._sample("torrentngd", "idle_dht_on", 0.0, {
                "progress": 0.0, "down": 0, "up": 0, "peers": 0,
            })

            instance._capture_engine_memory_metrics.assert_not_called()
            self.assertEqual(len(instance.samples), 1)


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

    def test_torrentng_idle_profiles_override_startup_dht_state(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            instance = object.__new__(benchmark.Benchmark)
            instance.config_dir = Path(directory)
            instance.storage_io_worker_threads = None
            instance.storage_hash_worker_threads = None
            instance.storage_peer_read_readahead_bytes = None
            instance.storage_peer_read_cache_entries = None
            instance.storage_peer_read_cache_bytes = None
            instance.piece_assembly_cap_mb = None
            instance.peer_buffer_cap_mb = None

            instance._prepare_torrentng_config(dht_enabled=False)
            disabled = (Path(directory) / "torrentngd.config.toml").read_text()
            self.assertRegex(disabled, r"(?ms)^\[dht\]\s*\nenabled = false\b")

            instance._prepare_torrentng_config(dht_enabled=True)
            enabled = (Path(directory) / "torrentngd.config.toml").read_text()
            self.assertRegex(enabled, r"(?ms)^\[dht\]\s*\nenabled = true\b")

    def test_torrentng_idle_runtime_disables_and_reads_back_pex(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance._torrentng_request = mock.Mock(return_value=b"")

        instance._set_idle_dht("torrentngd", True)

        instance._torrentng_request.assert_called_once_with(
            "/api/v1/session/features",
            data=b'{"dht": true, "pex": false}',
            headers={"Content-Type": "application/json"},
            method="PUT",
        )
        instance._torrentng_request = mock.Mock(
            return_value=b'{"dht": true, "pex": false}',
        )
        self.assertIs(instance._read_idle_dht("torrentngd"), True)
        self.assertIs(instance._read_idle_pex("torrentngd"), False)

    def test_reference_client_idle_pex_readback_is_normalized(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance._qbit_json = mock.Mock(return_value={"pex": False})
        instance._transmission = mock.Mock(
            return_value={"arguments": {"pex-enabled": False}},
        )
        instance._deluge = mock.Mock(return_value={"utpex": False})
        instance._scgi = mock.Mock(return_value="no")

        for client in ("qbittorrent", "transmission", "deluge", "rtorrent"):
            with self.subTest(client=client):
                self.assertIs(instance._read_idle_pex(client), False)

    def test_deluge_idle_readback_requests_only_the_selected_config_keys(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance._deluge = mock.Mock(side_effect=[{"dht": True}, {"utpex": False}])

        self.assertIs(instance._read_idle_dht("deluge"), True)
        self.assertIs(instance._read_idle_pex("deluge"), False)

        self.assertEqual(
            instance._deluge.call_args_list,
            [
                mock.call("core.get_config_values", [["dht"]]),
                mock.call("core.get_config_values", [["utpex"]]),
            ],
        )

    def test_rtorrent_idle_profiles_override_startup_dht_mode(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            instance = object.__new__(benchmark.Benchmark)
            instance.config_dir = Path(directory)

            instance._prepare_rtorrent_config(dht_enabled=False)
            disabled = (Path(directory) / "rtorrent-benchmark.rc").read_text()
            self.assertIn("dht.mode.set = disable", disabled)

            instance._prepare_rtorrent_config(dht_enabled=True)
            enabled = (Path(directory) / "rtorrent-benchmark.rc").read_text()
            self.assertIn("dht.mode.set = on", enabled)

    def test_rtorrent_idle_dht_readback_uses_reported_mode(self) -> None:
        instance = object.__new__(benchmark.Benchmark)
        instance._scgi = mock.Mock(return_value={"dht": "disable"})

        self.assertIs(instance._read_idle_dht("rtorrent"), False)
        instance._scgi.assert_called_once_with("dht.statistics")

        for mode in ("off", "auto"):
            with self.subTest(mode=mode):
                instance._scgi.return_value = {"dht": mode}
                self.assertIs(instance._read_idle_dht("rtorrent"), False)

        instance._scgi.return_value = {"dht": "on"}
        self.assertIs(instance._read_idle_dht("rtorrent"), True)

        instance._scgi.return_value = {}
        self.assertIsNone(instance._read_idle_dht("rtorrent"))

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
