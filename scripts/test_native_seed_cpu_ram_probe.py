#!/usr/bin/env python3
"""Focused offline tests for the local native seed resource probe."""
from __future__ import annotations

import hashlib
import argparse
import socket
import sys
import tempfile
import subprocess
import threading
import time
import unittest
from unittest.mock import patch
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import native_seed_cpu_ram_probe as probe


def decode(data):
    def one(offset):
        marker = data[offset:offset + 1]
        if marker == b"i":
            end = data.index(b"e", offset)
            return int(data[offset + 1:end]), end + 1
        if marker == b"l":
            values, offset = [], offset + 1
            while data[offset:offset + 1] != b"e":
                value, offset = one(offset); values.append(value)
            return values, offset + 1
        if marker == b"d":
            values, offset = {}, offset + 1
            while data[offset:offset + 1] != b"e":
                key, offset = one(offset); value, offset = one(offset); values[key] = value
            return values, offset + 1
        colon = data.index(b":", offset)
        length = int(data[offset:colon])
        end = colon + 1 + length
        return data[colon + 1:end], end
    result, end = one(0)
    if end != len(data): raise AssertionError("trailing bencode bytes")
    return result


class ProbeTests(unittest.TestCase):
    def test_fixture_is_private_and_piece_hashes_match_payload(self):
        with tempfile.TemporaryDirectory() as temporary:
            payload, torrent, info_hash, expected = probe.make_fixture(
                Path(temporary) / "fixture", 2 * probe.PIECE + 37, 123)
            metainfo = decode(torrent)
            info = metainfo[b"info"]
            data = payload.read_bytes()
            self.assertEqual(info[b"private"], 1)
            self.assertNotIn(b"announce", metainfo)
            self.assertEqual(info[b"pieces"], b"".join(
                hashlib.sha1(data[offset:offset + probe.PIECE], usedforsecurity=False).digest()
                for offset in range(0, len(data), probe.PIECE)))
            self.assertEqual(hashlib.sha1(probe.bencode(info), usedforsecurity=False).digest(), info_hash)
            self.assertEqual(hashlib.sha256(data).hexdigest(), expected)

    def test_scgi_environment_uses_nul_then_ascii_one(self):
        headers = probe.scgi_headers(42)
        self.assertIn(b"\0SCGI\0" + b"1\0", headers)
        self.assertNotIn(b"\0SCGI\x01\0", headers)

    def test_wait_for_retries_false_result(self):
        class Alive:
            returncode = None
            def poll(self): return None
        attempts = []
        def check():
            attempts.append(1)
            return None if len(attempts) < 2 else {"ready": True}
        result = probe.wait_for(check, Alive(), 1, "test readiness")
        self.assertEqual(result, {"ready": True})
        self.assertEqual(len(attempts), 2)

    def test_recv_exact_keeps_partial_bytes_across_socket_timeout(self):
        left, right = socket.socketpair()
        left.settimeout(.01)
        def sender():
            right.sendall(b"ab")
            time.sleep(.04)
            right.sendall(b"cd")
        thread = threading.Thread(target=sender)
        thread.start()
        try:
            self.assertEqual(probe.recv_exact(left, 4, time.monotonic() + 1), b"abcd")
        finally:
            thread.join(); left.close(); right.close()

    def test_seed_readiness_requires_full_active_seed(self):
        self.assertTrue(probe.torrentng_seed_ready({"progress": 1, "state": "uploading"}))
        self.assertFalse(probe.torrentng_seed_ready({"progress": 1, "state": "checkingUP"}))
        self.assertTrue(probe.rtorrent_seed_ready(["hash", 1, 1, 100, 100]))
        self.assertFalse(probe.rtorrent_seed_ready(["hash", 1, 0, 100, 100]))

    def test_incomplete_bitfield_can_be_completed_by_have(self):
        available = probe.update_available_pieces(bytes((5, 0b11000000)), 3, set())
        self.assertEqual(available, {0, 1})
        available = probe.update_available_pieces(bytes((4, 0, 0, 0, 2)), 3, available)
        self.assertEqual(available, {0, 1, 2})

    def test_pacing_starts_at_first_payload(self):
        self.assertEqual(probe.pacing_deadline(500.0, 4 * 1024 * 1024), 501.0)
        self.assertEqual(probe.pacing_delay(501.0, 500.5), 0.5)
        self.assertEqual(probe.pacing_delay(501.0, 501.000001), 0.0)

    def test_client_environment_overrides_validate_labels_and_apply(self):
        parsed = probe.parse_client_env("verify-oneworker:TOKIO_WORKER_THREADS=1")
        overrides = probe.client_env_overrides([parsed], ["baseline", "verify-oneworker"])
        self.assertEqual(overrides, {"verify-oneworker": {"TOKIO_WORKER_THREADS": "1"}})
        environment = probe.apply_client_environment({"PATH": "/bin", "TOKIO_WORKER_THREADS": "2"},
                                                     overrides["verify-oneworker"])
        self.assertEqual(environment, {"PATH": "/bin", "TOKIO_WORKER_THREADS": "1"})

    @patch.object(probe.time, "clock_gettime_ns")
    @patch.object(probe, "process_cpu_clock_id")
    def test_process_tree_cpu_clock_sums_root_and_children(self, get_clock_id, get_time_ns):
        get_clock_id.side_effect = {10: 100, 11: 101}.__getitem__
        get_time_ns.side_effect = {100: 1_000_000_000, 101: 250_000_000}.__getitem__
        self.assertEqual(probe.process_tree_cpu_time_ns(10, {10, 11}), 1_250_000_000)

    def test_cpu_delta_prefers_precise_clock_and_keeps_tick_fallback(self):
        precise_start = {"cpu_ticks_user_system": 100, "cpu_time_user_system_ns": 10_000}
        precise_end = {"cpu_ticks_user_system": 200, "cpu_time_user_system_ns": 1_234_010}
        self.assertAlmostEqual(probe.cpu_seconds_between(precise_start, precise_end, 100), 0.00122401)
        tick_start = {"cpu_ticks_user_system": 100}
        tick_end = {"cpu_ticks_user_system": 125}
        self.assertEqual(probe.cpu_seconds_between(tick_start, tick_end, 100), 0.25)

    @patch.object(probe.subprocess, "run")
    def test_cpu_profile_control_uses_gdb_and_quotes_profile_path(self, run):
        run.side_effect = [
            subprocess.CompletedProcess([], 0, stdout="$1 = 1\n", stderr=""),
            subprocess.CompletedProcess([], 0, stdout="", stderr=""),
        ]
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            profile = root / 'profile with "quotes".prof'
            start_log = root / "start.log"
            stop_log = root / "stop.log"
            probe.gdb_profile_control(1234, "start", profile, start_log)
            probe.gdb_profile_control(1234, "stop", profile, stop_log)
            start_command = run.call_args_list[0].args[0]
            stop_command = run.call_args_list[1].args[0]
            self.assertIn("--pid", start_command)
            self.assertIn("1234", start_command)
            self.assertIn("ProfilerStart(", start_command[-1])
            self.assertIn(r'\"quotes\"', start_command[-1])
            self.assertEqual(stop_command[-1], "call (void)ProfilerStop()")
            self.assertIn("$1 = 1", start_log.read_text())
            self.assertEqual(stop_log.read_text(), "")

    def test_pgo_profile_gdb_script_resets_at_upload_start_and_records_child_pid(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            script = root / "profile-window.gdb"
            pid_file = root / "torrentngd.pid"
            probe.write_pgo_profile_window_script(script, pid_file)
            contents = script.read_text()
            self.assertIn("rbreak rt_engine::torrent_task::start_upload_reads", contents)
            self.assertIn("call (void)__llvm_profile_reset_counters()", contents)
            self.assertIn("TORRENTNG_PGO_PROFILE_RESET", contents)
            self.assertIn(str(pid_file), contents)
            self.assertIn("handle SIGTERM nostop noprint pass", contents)

    def test_child_environment_strips_controls_and_records_only_safe_values(self):
        source = {
            "PATH": "/bin", "HTTP_PROXY": "http://proxy.invalid",
            "TOKIO_WORKER_THREADS": "8", "MALLOC_ARENA_MAX": "4",
            "LD_PRELOAD": "/tmp/libprofiler.so", "TORRENTNGD_API_TOKEN": "secret-token",
            "TNG_STORAGE_FRAME_CAP_MB": "4096", "RUST_LOG": "debug",
        }
        child, inherited = probe.sanitize_client_environment(
            source, {"TOKIO_WORKER_THREADS": "1", "MALLOC_ARENA_MAX": "2"})
        self.assertEqual(child["PATH"], "/bin")
        self.assertEqual(child["HTTP_PROXY"], "http://proxy.invalid")
        self.assertEqual(child["TOKIO_WORKER_THREADS"], "1")
        self.assertEqual(child["MALLOC_ARENA_MAX"], "2")
        self.assertNotIn("LD_PRELOAD", child)
        self.assertNotIn("TORRENTNGD_API_TOKEN", child)
        self.assertNotIn("TNG_STORAGE_FRAME_CAP_MB", child)
        self.assertEqual(inherited["safe_values"]["RUST_LOG"], "debug")
        self.assertNotIn("TORRENTNGD_API_TOKEN", inherited["safe_values"])
        self.assertIn("TORRENTNGD_API_TOKEN", inherited["names"])

    def test_client_environment_rejects_unknown_label_bad_name_and_duplicates(self):
        with self.assertRaises(ValueError):
            probe.client_env_overrides([("typo", "TOKIO_WORKER_THREADS", "1")], ["verify"])
        with self.assertRaises(ValueError):
            probe.client_env_overrides([("verify", "RUST_LOG", "info"),
                                        ("verify", "RUST_LOG", "warn")], ["verify"])
        with self.assertRaises(argparse.ArgumentTypeError):
            probe.parse_client_env("verify:1INVALID=value")

    def test_single_client_requires_explicit_unpaired_recovery_mode(self):
        with self.assertRaises(ValueError):
            probe.validate_client_count(1, 1, False)
        with self.assertRaises(ValueError):
            probe.validate_client_count(1, 2, True)
        probe.validate_client_count(1, 1, True)

    def test_pgo_profile_window_requires_one_uninstrumented_controlled_torrentng_run(self):
        valid = [("training", "torrentngd", "/tmp/instrumented-torrentngd")]
        probe.validate_pgo_profile_window(valid, 1, False, {})
        with self.assertRaisesRegex(ValueError, "one TorrentNG client"):
            probe.validate_pgo_profile_window(valid + [("rt", "rtorrent", "/tmp/rtorrent")], 1, False, {})
        with self.assertRaisesRegex(ValueError, "--repeats 1"):
            probe.validate_pgo_profile_window(valid, 3, False, {})
        with self.assertRaisesRegex(ValueError, "cannot be combined"):
            probe.validate_pgo_profile_window(valid, 1, True, {})
        with self.assertRaisesRegex(ValueError, "controls LLVM_PROFILE_FILE"):
            probe.validate_pgo_profile_window(valid, 1, False, {"training": {"LLVM_PROFILE_FILE": "/tmp/custom.profraw"}})
        probe.validate_client_count(2, 3, False)

    def test_storage_io_queue_depth_config_is_optional_and_bounded(self):
        self.assertEqual(probe.storage_io_queue_depth_line(None), "")
        self.assertEqual(probe.storage_io_queue_depth_line(64), "io_queue_depth = 64\n")
        self.assertEqual(
            probe.storage_io_queue_depth_line(probe.MAX_STORAGE_IO_QUEUE_DEPTH),
            f"io_queue_depth = {probe.MAX_STORAGE_IO_QUEUE_DEPTH}\n",
        )
        for invalid in (0, -1, probe.MAX_STORAGE_IO_QUEUE_DEPTH + 1):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                probe.storage_io_queue_depth_line(invalid)

    def test_selected_backend_diagnostic_preserves_backend_label(self):
        metrics = "\n".join((
            'torrentng_storage_scheduler_backend_selected{backend="pread",scheduler_group="1"} 0',
            'torrentng_storage_scheduler_backend_selected{backend="uring",scheduler_group="1"} 1',
        ))
        self.assertEqual(
            probe.parse_selected_metrics(metrics),
            {"storage_scheduler_selected_backends": ["uring"]},
        )

    def test_payload_cleanup_removes_only_generated_copies_inside_run(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "run-01-client"
            download = root / "client" / "download"
            download.mkdir(parents=True)
            (root / "downloaded.bin").write_bytes(b"leecher copy")
            (download / "payload.bin").write_bytes(b"seed copy")
            evidence = root / "summary-fragment.json"
            evidence.write_text("keep")
            removed = probe.cleanup_run_payloads(root, "client")
            self.assertEqual(set(removed), {"downloaded.bin", "client/download/payload.bin"})
            self.assertFalse((root / "downloaded.bin").exists())
            self.assertFalse((download / "payload.bin").exists())
            self.assertEqual(evidence.read_text(), "keep")

    def test_payload_cleanup_rejects_symlink_escape(self):
        with tempfile.TemporaryDirectory() as temporary:
            parent = Path(temporary)
            root = parent / "run"
            download = root / "client" / "download"
            download.mkdir(parents=True)
            external = parent / "external.bin"
            external.write_bytes(b"outside")
            (download / "payload.bin").symlink_to(external)
            with self.assertRaises(ValueError):
                probe.cleanup_run_payloads(root, "client")
            self.assertEqual(external.read_bytes(), b"outside")

    def test_rtorrent_config_matches_current_native_profile(self):
        config = probe.rtorrent_config_text("/tmp/down", "/tmp/session", 23456, 6789)
        for expected in (
            "directory.default.set = /tmp/down", "session.path.set = /tmp/session",
            "network.bind_address.ipv4.set = 127.0.0.1",
            "network.listen.port.range.set = 23456-23456", "dht.mode.set = disable",
            "protocol.pex.set = no", "network.scgi.open_port = 127.0.0.1:6789",
        ):
            self.assertIn(expected, config)

    def test_metrics_parser_extracts_cache_and_frame_memory_counters(self):
        metrics = """# HELP torrentng_storage_peer_read_cache_global_backing_bytes backing
torrentng_storage_peer_read_cache_global_backing_bytes 32768
torrentng_storage_peer_read_cache_hits_total{scope="global"} 7
torrentng_storage_peer_read_cache_misses_total 3
torrentng_storage_peer_read_cache_evictions_total 2
torrentng_storage_frame_bytes_in_use 65536
torrentng_storage_frame_bytes_retained 131072
other_metric 999
"""
        selected = probe.parse_selected_metrics(metrics)
        self.assertEqual(selected["torrentng_storage_peer_read_cache_global_backing_bytes"], 32768)
        self.assertEqual(selected["torrentng_storage_peer_read_cache_hits_total"], 7)
        self.assertEqual(selected["torrentng_storage_peer_read_cache_misses_total"], 3)
        self.assertEqual(selected["torrentng_storage_peer_read_cache_evictions_total"], 2)
        self.assertEqual(selected["torrentng_storage_frame_bytes_in_use"], 65536)
        self.assertEqual(selected["torrentng_storage_frame_bytes_retained"], 131072)
        self.assertNotIn("other_metric", selected)

    def test_host_proc_mapping_resolves_child_pid(self):
        child = subprocess.Popen(["/bin/sleep", "5"])
        try:
            host_pid = probe.host_proc_pid(child.pid, "/bin/sleep")
            self.assertGreater(host_pid, 0)
            sample = probe.process_sample(host_pid)
            self.assertIn(host_pid, sample["pids"])
            self.assertGreater(sample["rss_bytes"], 0)
            self.assertGreater(sample["pss_bytes"], 0)
        finally:
            child.terminate()
            child.wait(timeout=5)


if __name__ == "__main__":
    unittest.main()
