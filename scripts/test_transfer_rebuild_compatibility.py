"""Regression sources for PERF-110 client protocol preflight."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).parent / "transfer_rebuild"
sys.path.insert(0, str(TOOLS))

from adapters import AdapterError  # noqa: E402
from compatibility import (  # noqa: E402
    PreflightError,
    _poll,
    _probe_files,
    _probe_payload_bytes,
    _qbit_add_result,
    _run_mutation,
)
from fixtures import load_fixture_profile_document  # noqa: E402
from validate_artifact import _settings_digest, validate  # noqa: E402


class CompatibilityPreflightTests(unittest.TestCase):
    def test_qbittorrent_add_accepts_legacy_and_json_acceptance_responses(self) -> None:
        info_hash = "a" * 40
        self.assertEqual(_qbit_add_result("Ok.", [info_hash]), (None, False))
        self.assertEqual(
            _qbit_add_result(
                json.dumps(
                    {
                        "success_count": 1,
                        "pending_count": 0,
                        "failure_count": 0,
                        "added_torrent_ids": [info_hash],
                    }
                ),
                [info_hash],
            ),
            (info_hash, False),
        )
        self.assertEqual(
            _qbit_add_result(
                json.dumps(
                    {
                        "success_count": 0,
                        "pending_count": 1,
                        "failure_count": 0,
                        "added_torrent_ids": [info_hash.upper()],
                    }
                ),
                [info_hash],
            ),
            (info_hash.upper(), True),
        )
        self.assertEqual(
            _qbit_add_result(
                json.dumps(
                    {
                        "success_count": 1,
                        "pending_count": 0,
                        "failure_count": 0,
                        "added_torrent_ids": ["b" * 40],
                    }
                ),
                [info_hash, "b" * 40],
            ),
            ("b" * 40, False),
        )

    def test_qbittorrent_add_rejects_unaccepted_or_mismatched_json_response(self) -> None:
        info_hash = "a" * 40
        for response in (
            "not-json",
            json.dumps(
                {
                    "success_count": 0,
                    "pending_count": 0,
                    "failure_count": 1,
                    "added_torrent_ids": [],
                }
            ),
            json.dumps(
                {
                    "success_count": 1,
                    "pending_count": 0,
                    "failure_count": 0,
                    "added_torrent_ids": ["b" * 40],
                }
            ),
        ):
            with self.subTest(response=response), self.assertRaises(PreflightError):
                _qbit_add_result(response, [info_hash])

    def test_poll_retries_transient_http_503_until_state_converges(self) -> None:
        calls = 0

        def read() -> str:
            nonlocal calls
            calls += 1
            if calls < 3:
                raise AdapterError("HTTP 503 for /api/qb/v2/torrents/info")
            return "removed"

        self.assertEqual(_poll(read, lambda value: value == "removed", "probe removal"), "removed")
        self.assertEqual(calls, 3)

    def test_probe_payloads_preserve_protocol_edges_without_large_payloads(self) -> None:
        profile_path = TOOLS / "fixture_profiles.json"
        document = json.loads(profile_path.read_text(encoding="utf-8"))
        for profile_id in (
            "G3/hybrid-padding",
            "G3/v2-piece-layer",
            "N1/lan_1g",
            "N1/lan_10g",
        ):
            profile = load_fixture_profile_document(document, profile_id)
            probe_files = _probe_files(profile)
            self.assertEqual([item.name for item in probe_files], [item.name for item in profile["files"]])
            self.assertLess(
                _probe_payload_bytes(
                    probe_files,
                    profile["torrent_kind"],
                    profile["piece_length"],
                ),
                3 * 1024 * 1024,
            )
            if profile["torrent_kind"] in {"v2", "hybrid"}:
                self.assertGreater(probe_files[0].size_bytes, profile["piece_length"])
            if profile["torrent_kind"] == "v2":
                self.assertEqual(probe_files[1].size_bytes, 0)
            if profile["torrent_kind"] == "hybrid":
                self.assertGreater(probe_files[1].size_bytes, 0)
                self.assertEqual(probe_files[2].size_bytes, 0)

    def test_failed_pause_verification_still_removes_the_new_probe(self) -> None:
        calls: list[str] = []
        result = _run_mutation(
            lambda: calls.append("add") or "new-probe",
            lambda _handle: calls.append("verify") or False,
            lambda _handle: calls.append("remove"),
            lambda: None,
        )
        self.assertEqual(calls, ["add", "verify", "remove"])
        self.assertEqual(result["status"], "failed")
        self.assertTrue(result["added"])
        self.assertFalse(result["paused_verified"])
        self.assertTrue(result["removed"])

    def test_cleanup_failure_is_a_distinct_blocking_outcome(self) -> None:
        result = _run_mutation(
            lambda: "new-probe",
            lambda _handle: True,
            lambda _handle: (_ for _ in ()).throw(RuntimeError("remove failed")),
            lambda: "new-probe",
        )
        self.assertEqual(result["status"], "cleanup_failed")
        self.assertTrue(result["added"])
        self.assertTrue(result["paused_verified"])
        self.assertFalse(result["removed"])

    def test_uncertain_add_is_recovered_and_exact_probe_is_removed(self) -> None:
        calls: list[str] = []

        def add() -> str:
            calls.append("add")
            raise TimeoutError("add response was lost")

        result = _run_mutation(
            add,
            lambda handle: calls.append(f"verify:{handle}") or True,
            lambda handle: calls.append(f"remove:{handle}"),
            lambda: calls.append("recover") or "new-probe",
        )
        self.assertEqual(calls, ["add", "recover", "verify:new-probe", "remove:new-probe"])
        self.assertEqual(result["status"], "failed")
        self.assertTrue(result["added"])
        self.assertTrue(result["removed"])

    def test_unknown_add_state_is_a_cleanup_failure(self) -> None:
        result = _run_mutation(
            lambda: (_ for _ in ()).throw(TimeoutError("add response was lost")),
            lambda _handle: True,
            lambda _handle: None,
            lambda: (_ for _ in ()).throw(RuntimeError("client is unreachable")),
        )
        self.assertEqual(result["status"], "cleanup_failed")
        self.assertIsNone(result["added"])
        self.assertFalse(result["removed"])

    def _valid_artifact(self) -> dict[str, object]:
        clients = ("deluge", "qbittorrent", "rtorrent", "torrentngd", "transmission", "rqbit", "urtorrent")
        profiles = {
            "G3/hybrid-padding": "hybrid",
            "G3/v2-piece-layer": "v2",
            "N1/lan_1g": "v1",
            "N1/lan_10g": "v1",
        }
        pinned = ["qbittorrent", "rqbit", "torrentngd"]
        results = []
        for client_id in clients:
            for profile_id, protocol in profiles.items():
                unsupported = client_id == "urtorrent" and protocol in {"v2", "hybrid"}
                if unsupported:
                    status = "unsupported"
                    reason = "urtorrent v1-only comparator"
                elif client_id not in pinned:
                    status = "not_pinned"
                    reason = "client pin is absent"
                else:
                    status = "passed"
                    reason = None
                results.append(
                    {
                        "case_id": f"{client_id}:{profile_id}",
                        "client_id": client_id,
                        "profile_id": profile_id,
                        "protocol": protocol,
                        "applicability": "unsupported" if unsupported else "candidate",
                        "status": status,
                        "fixture_sha256": "a" * 64,
                        "torrent_sha256": "b" * 64,
                        "info_hash": "c" * (64 if protocol == "v2" else 40),
                        "observed_version": "test-version" if status == "passed" else None,
                        "client_identity_sha256": "d" * 64 if status == "passed" else None,
                        "settings_sha256": "e" * 64 if status == "passed" else None,
                        "added": True if status == "passed" else None,
                        "paused_verified": True if status == "passed" else None,
                        "pause_mode": (
                            "after_add" if client_id in {"qbittorrent", "rqbit"} else "on_add"
                        ) if status == "passed" else None,
                        "removed": True if status == "passed" else None,
                        "reason": reason,
                    }
                )
        scheduled_pairs = []
        for index, profile_id in enumerate(profiles):
            scheduled_pairs.append(
                {
                    "pair_id": f"pair-{index + 1}",
                    "profile_id": profile_id,
                    "repetition": 1,
                    "case_ids": [
                        f"qbittorrent:{profile_id}",
                        f"rqbit:{profile_id}",
                        f"torrentngd:{profile_id}",
                    ],
                }
            )
        eligible = sorted(result["case_id"] for result in results if result["status"] == "passed")
        artifact: dict[str, object] = {
            "schema_version": "transfer-compatibility-preflight-v1",
            "preflight_id": "0" * 20,
            "plan_id": "1" * 20,
            "plan_sha256": "2" * 64,
            "pins_sha256": "3" * 64,
            "catalog_sha256": "4" * 64,
            "created_at": "2026-10-18T12:00:00Z",
            "pinned_client_ids": pinned,
            "ready": True,
            "eligible_case_ids": eligible,
            "scheduled_pairs": scheduled_pairs,
            "excluded_pairs": [],
            "results": results,
        }
        artifact["preflight_id"] = _settings_digest(
            {key: value for key, value in artifact.items() if key != "preflight_id"}
        )[:20]
        return artifact

    def test_preflight_artifact_requires_cleanup_evidence(self) -> None:
        artifact = self._valid_artifact()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "preflight.json"
            path.write_text(json.dumps(artifact), encoding="utf-8")
            self.assertEqual(validate("compatibility-preflight", path), [])
            rqbit_case = next(
                result for result in artifact["results"]
                if result["case_id"] == "rqbit:G3/hybrid-padding"
            )
            self.assertEqual(rqbit_case["pause_mode"], "after_add")
            qbittorrent_case = next(
                result for result in artifact["results"]
                if result["case_id"] == "qbittorrent:G3/hybrid-padding"
            )
            self.assertEqual(qbittorrent_case["pause_mode"], "after_add")

            passed_case = next(
                result for result in artifact["results"]
                if result["case_id"] == "qbittorrent:G3/hybrid-padding"
            )
            passed_case["removed"] = False
            artifact["preflight_id"] = _settings_digest(
                {key: value for key, value in artifact.items() if key != "preflight_id"}
            )[:20]
            path.write_text(json.dumps(artifact), encoding="utf-8")
            errors = validate("compatibility-preflight", path)
            self.assertTrue(any("passed without add" in error for error in errors))

    def test_preflight_artifact_binds_pause_mode_and_complete_pair_membership(self) -> None:
        artifact = self._valid_artifact()
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "preflight.json"
            rqbit_case = next(
                result for result in artifact["results"]
                if result["case_id"] == "rqbit:G3/hybrid-padding"
            )
            rqbit_case["pause_mode"] = "on_add"
            artifact["preflight_id"] = _settings_digest(
                {key: value for key, value in artifact.items() if key != "preflight_id"}
            )[:20]
            path.write_text(json.dumps(artifact), encoding="utf-8")
            errors = validate("compatibility-preflight", path)
            self.assertTrue(any("pause_mode" in error for error in errors))

            rqbit_case["pause_mode"] = "after_add"
            artifact["scheduled_pairs"][0]["case_ids"].remove("rqbit:G3/hybrid-padding")
            artifact["preflight_id"] = _settings_digest(
                {key: value for key, value in artifact.items() if key != "preflight_id"}
            )[:20]
            path.write_text(json.dumps(artifact), encoding="utf-8")
            errors = validate("compatibility-preflight", path)
            self.assertTrue(any("every pinned candidate" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
