#!/usr/bin/env python3
"""Regression sources for identity-bound PERF-110 client controls."""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
import urllib.parse
from pathlib import Path
from unittest.mock import patch

SCRIPT_DIR = Path(__file__).resolve().parent / "transfer_rebuild"
sys.path.insert(0, str(SCRIPT_DIR))

import client_control  # noqa: E402


class _Http:
    def __init__(self, *, json_value=None, raw_value=b"", fail_json=False):
        self.json_value = json_value
        self.raw_value = raw_value
        self.fail_json = fail_json
        self.requests = []
        self.json_paths = []

    def json(self, path, **_kwargs):
        self.json_paths.append(path)
        if self.fail_json:
            raise client_control.AdapterError("HTTP 404")
        return self.json_value

    def request(self, path, **kwargs):
        self.requests.append((path, kwargs))
        return self.raw_value


def _controller(client_id: str, http: _Http) -> client_control.ClientController:
    observation = {
        "client_id": client_id,
        "version": "pinned",
        "runtime_identity": {"kind": "container", "container_id": "a" * 64},
        "settings_readback": {"captured": True, "sha256": "b" * 64},
    }
    controller = client_control.ClientController.__new__(client_control.ClientController)
    controller.client_id = client_id
    controller.profile = {}
    controller.observation = observation
    controller.runtime_identity = observation["runtime_identity"]
    controller.http = http
    controller.socket_path = None
    controller._qbit_hash_candidates = {}
    controller._qbit_hash_by_info_hash = {}
    return controller


class ClientControlRegressions(unittest.TestCase):
    def test_qbittorrent_v2_add_uses_returned_id_and_stops_before_barrier(self) -> None:
        info = b"d12:meta versioni2ee"
        metainfo = b"d4:infod12:meta versioni2eee"
        info_hash = hashlib.sha256(info).hexdigest()
        client_hash = info_hash[:40]
        state: dict[str, object] = {"row": None}

        class QbTrialHttp(_Http):
            def __init__(self, *, converge_stop=True):
                super().__init__()
                self.converge_stop = converge_stop
                self.stop_requested = False
                self.stop_polls = 0

            def json(self, path, **_kwargs):
                self.json_paths.append(path)
                query = urllib.parse.parse_qs(urllib.parse.urlsplit(path).query)
                requested_hash = query.get("hashes", [""])[0]
                row = state["row"]
                if isinstance(row, dict) and row.get("hash") == requested_hash:
                    if self.stop_requested and self.converge_stop:
                        self.stop_polls += 1
                        if self.stop_polls >= 3:
                            row["state"] = "stoppedDL"
                    return [dict(row)]
                return []

            def request(self, path, **kwargs):
                self.requests.append((path, kwargs))
                if path == "/api/v2/torrents/add":
                    state["row"] = {
                        "hash": client_hash,
                        "state": "checkingResumeData",
                        "progress": 0.0,
                        "size": 1,
                        "downloaded": 0,
                        "uploaded": 0,
                        "dlspeed": 0,
                        "upspeed": 0,
                        "num_seeds": 0,
                        "num_leechs": 0,
                    }
                    return json.dumps(
                        {
                            "added_torrent_ids": [client_hash],
                            "failure_count": 0,
                            "pending_count": 0,
                            "success_count": 1,
                        }
                    ).encode()
                if path == "/api/v2/torrents/stop":
                    self.assert_stop_hash(kwargs, client_hash)
                    self.stop_requested = True
                elif path == "/api/v2/torrents/delete":
                    self.assert_stop_hash(kwargs, client_hash)
                    state["row"] = None
                return b""

            @staticmethod
            def assert_stop_hash(kwargs, expected_hash):
                if kwargs.get("form", {}).get("hashes") != expected_hash:
                    raise AssertionError("qBittorrent control did not use its returned client hash")

        http = QbTrialHttp()
        controller = _controller("qbittorrent", http)
        with tempfile.TemporaryDirectory() as temporary:
            torrent_path = Path(temporary) / "fixture.torrent"
            torrent_path.write_bytes(metainfo)
            with patch.object(controller, "_assert_identity"):
                controller.add_paused(torrent_path, "/tmp/qbittorrent-trial", info_hash)
                status = controller.status(info_hash)
                self.assertIsNotNone(status)
                self.assertEqual(status.state, "stoppedDL")
                self.assertEqual(status.info_hash, info_hash)
                controller.remove(info_hash, delete_data=False)
            self.assertGreaterEqual(http.stop_polls, 3)

            # A setup error after qBittorrent accepts the torrent must retire
            # that trial-owned row so the next scheduled slot can add it.
            failing_http = QbTrialHttp(converge_stop=False)
            failing_controller = _controller("qbittorrent", failing_http)
            original_wait = failing_controller._wait

            def fail_pause_convergence(predicate, label, timeout_seconds=20.0):
                if label == "qBittorrent fixture to stop":
                    raise client_control.ClientControlError("timed out waiting for qB stop")
                return original_wait(predicate, label, timeout_seconds)

            with patch.object(failing_controller, "_assert_identity"):
                with patch.object(failing_controller, "_wait", side_effect=fail_pause_convergence):
                    with self.assertRaisesRegex(
                        client_control.ClientControlError,
                        "removed admitted fixture after setup failure",
                    ):
                        failing_controller.add_paused(
                            torrent_path, "/tmp/qbittorrent-trial", info_hash
                        )
            self.assertIsNone(state["row"])
            self.assertTrue(
                any(path == "/api/v2/torrents/delete" for path, _kwargs in failing_http.requests)
            )
        self.assertTrue(
            any(path == "/api/v2/torrents/stop" for path, _kwargs in http.requests)
        )
        self.assertEqual(state["row"], None)

    def test_state_convergence_retries_a_transient_http_503(self) -> None:
        controller = _controller("qbittorrent", _Http())
        attempts = 0

        def converged() -> bool:
            nonlocal attempts
            attempts += 1
            if attempts == 1:
                raise client_control.ClientControlError("HTTP 503 for /api/v2/torrents/info")
            return True

        with patch.object(client_control.time, "sleep") as sleep:
            controller._wait(converged, "test state", timeout_seconds=1)

        self.assertEqual(attempts, 2)
        sleep.assert_called_once_with(client_control.POLL_INTERVAL_SECONDS)

    def test_identity_guard_rejects_a_restarted_or_reconfigured_client(self) -> None:
        http = _Http()
        controller = _controller("qbittorrent", http)
        changed = dict(controller.observation)
        changed["runtime_identity"] = {"kind": "container", "container_id": "c" * 64}
        with patch.object(client_control, "probe_client", return_value=changed):
            with self.assertRaisesRegex(client_control.ClientControlError, "identity or settings changed"):
                controller._assert_identity()

    def test_qbittorrent_status_is_selected_by_exact_hash(self) -> None:
        wanted = {
            "hash": "1" * 40,
            "state": "downloading",
            "progress": 0.25,
            "size": 400,
            "downloaded": 100,
            "uploaded": 3,
            "dlspeed": 20,
            "upspeed": 4,
            "num_seeds": 2,
            "num_leechs": 1,
        }
        http = _Http(json_value=[dict(wanted, hash="2" * 40), wanted])
        controller = _controller("qbittorrent", http)
        with patch.object(client_control, "probe_client", return_value=controller.observation):
            status = controller.status("1" * 40)
        self.assertIsNotNone(status)
        self.assertEqual(status.progress, 0.25)
        self.assertEqual(status.peers, 3)
        self.assertEqual(http.json_paths, ["/api/v2/torrents/info?hashes=" + "1" * 40])

    def test_rqbit_status_uses_pinned_stats_api_fields(self) -> None:
        digest = "3" * 40
        http = _Http(json_value={
            "state": "live",
            "progress_bytes": 75,
            "uploaded_bytes": 11,
            "total_bytes": 100,
            "finished": False,
            "live": {
                "download_speed": {"mbps": 1.5},
                "upload_speed": {"mbps": 0.25},
            },
        })
        controller = _controller("rqbit", http)
        with patch.object(client_control, "probe_client", return_value=controller.observation):
            status = controller.status(digest)
        self.assertEqual(http.json_paths, [f"/torrents/{digest}", f"/torrents/{digest}/stats/v1"])
        self.assertEqual(status.progress, 0.75)
        self.assertEqual(status.received_bytes, 75)
        self.assertEqual(status.download_rate_bps, int(1.5 * 1024 * 1024))
        self.assertIsNone(status.peers)

    def test_rqbit_start_uses_documented_start_route(self) -> None:
        http = _Http()
        controller = _controller("rqbit", http)
        with patch.object(controller, "_assert_identity"):
            controller._resume_unchecked("4" * 40)
        self.assertEqual(http.requests[0][0], f"/torrents/{'4' * 40}/start")

    def test_rqbit_forget_keeps_payload_files(self) -> None:
        http = _Http()
        controller = _controller("rqbit", http)
        with patch.object(controller, "_wait"), patch.object(controller, "_assert_identity"):
            controller._remove_unchecked("5" * 40, delete_data=False)
        self.assertEqual(http.requests[0][0], f"/torrents/{'5' * 40}/forget")

    def test_rtorrent_does_not_claim_to_delete_payload_files(self) -> None:
        controller = _controller("rtorrent", _Http())
        with self.assertRaisesRegex(client_control.ClientControlError, "cannot delete payload"):
            controller._remove_unchecked("6" * 40, delete_data=True)

    def test_info_hash_and_rtorrent_counters_are_strict(self) -> None:
        with self.assertRaises(client_control.ClientControlError):
            client_control._canonical_info_hash("not-a-hash")
        with self.assertRaises(client_control.ClientControlError):
            client_control._rtorrent_int(True, "counter")
        self.assertTrue(client_control._rtorrent_active("1"))
        self.assertFalse(client_control._rtorrent_active("0"))
        with self.assertRaises(client_control.ClientControlError):
            client_control._rtorrent_active("maybe")

    def test_paused_state_accepts_client_specific_paused_labels(self) -> None:
        for state in ("pausedDL", "STOPPED", "checking-paused"):
            self.assertTrue(client_control._paused_state(state), state)
        for state in ("downloading", "seeding", "checking"):
            self.assertFalse(client_control._paused_state(state), state)

    def test_deluge_progress_is_percent_not_a_fraction(self) -> None:
        self.assertEqual(client_control._deluge_progress({"progress": 65.0}), 0.65)
        with self.assertRaises(client_control.ClientControlError):
            client_control._deluge_progress({"progress": 101.0})

    def test_torrent_metainfo_must_be_stable_regular_file(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "real"
            directory.mkdir()
            path = directory / "fixture.torrent"
            path.write_bytes(b"metainfo")
            self.assertEqual(client_control._torrent_bytes(path), b"metainfo")
            link = Path(temporary) / "fixture-link.torrent"
            link.symlink_to(path)
            with self.assertRaises(client_control.ClientControlError):
                client_control._torrent_bytes(link)
            parent_link = Path(temporary) / "linked-directory"
            parent_link.symlink_to(directory, target_is_directory=True)
            with self.assertRaises(client_control.ClientControlError):
                client_control._torrent_bytes(parent_link / "fixture.torrent")

    def test_scheduled_hash_must_match_canonical_bencoded_info_value(self) -> None:
        metainfo = b"d8:announce3:url4:infod3:foo3:baree"
        info_dictionary = b"d3:foo3:bare"
        digest = hashlib.sha1(info_dictionary, usedforsecurity=False).hexdigest()  # nosemgrep: python.lang.security.insecure-hash-algorithms.insecure-hash-algorithm-sha1 -- The test vector uses the mandated BitTorrent v1 info hash.
        client_control._validate_metainfo_hash(metainfo, digest)
        client_control._validate_metainfo_hash(
            metainfo, hashlib.sha256(info_dictionary).hexdigest()
        )
        with self.assertRaisesRegex(client_control.ClientControlError, "does not match"):
            client_control._validate_metainfo_hash(metainfo, "0" * 40)
        with self.assertRaisesRegex(client_control.ClientControlError, "keys are not canonical"):
            client_control._torrent_info_span(b"d4:infod3:zoo3:bar3:aaa3:baree")
        with self.assertRaises(client_control.ClientControlError):
            client_control._normalize_save_path("/tmp/trial/../other")
        self.assertEqual(client_control._canonical_info_hash(digest.upper()), digest)


if __name__ == "__main__":
    unittest.main()
