"""Regression sources for deterministic PERF-110 trial assembly."""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from trial_record import (  # noqa: E402
    TrialRecordError,
    _hash_regular_nofollow,
    build_trial_record,
    write_trial,
)
from validate_artifact import validate  # noqa: E402


SHA = "a" * 64
COMMIT = "b" * 40
CLIENT_ID = "torrentngd"
PROCESS_ID = 321
PROCESS_START_TICKS = 987654
EXPECTED_BYTES = 64 * 1024


def harness_identity() -> dict[str, object]:
    return {
        "source_commit": COMMIT,
        "dirty_worktree": False,
        "lockfile_sha256": "c" * 64,
        "binary_sha256": "d" * 64,
        "binary_size_bytes": 4096,
        "binary_path": "/usr/bin/python3",
        "image_digest": None,
        "toolchain": "CPython 3.13.0",
    }


def client_observation() -> dict[str, object]:
    return {
        "schema_version": "transfer-client-observation-v1",
        "client_id": CLIENT_ID,
        "observed_at": "2026-10-18T18:00:00Z",
        "version": "0.1.0",
        "runtime_identity": {
            "kind": "native_binary",
            "binary_path": "/opt/torrentngd",
            "binary_sha256": SHA,
            "binary_size_bytes": 8192,
            "process_id": PROCESS_ID,
            "process_start_ticks": PROCESS_START_TICKS,
            "process_exe_device": 1,
            "process_exe_inode": 2,
            "source_commit": COMMIT,
            "dirty_worktree": False,
            "lockfile_sha256": "e" * 64,
            "toolchain": "rustc 1.88.0",
        },
        "settings_readback": {
            "captured": True,
            "values": {"max_connections": 200},
            "sha256": "f" * 64,
        },
    }


def resource_observation() -> dict[str, object]:
    return {
        "schema_version": "transfer-resource-observation-v1",
        "observer_id": "transfer-proc-cgroup-v2",
        "sample_count": 2,
        "sample_interval_ms": 250,
        "peaks": {
            "rss_bytes": 1024,
            "pss_bytes": 900,
            "cgroup_anon_bytes": 800,
            "cgroup_file_bytes": 100,
            "cgroup_reclaimable_cache_bytes": 50,
            "cgroup_current_bytes": 1000,
            "cpu_time_ns": 500_000,
            "threads": 4,
            "fds": 9,
            "block_read_bytes": 128,
            "block_write_bytes": 256,
        },
        "counter_note": "Process counters are summed over the observed tree.",
        "network_note": "Network byte counters were not requested.",
        "network_interface": None,
        "network_namespace_inode": None,
        "network_rx_bytes": None,
        "network_tx_bytes": None,
        "root_pid": PROCESS_ID,
        "root_process_start_ticks": PROCESS_START_TICKS,
        "started_monotonic_ns": 10,
        "finished_monotonic_ns": 500_000_010,
        "samples_csv": "resources.csv",
        "samples_csv_sha256": "1" * 64,
    }


def trial_inputs() -> dict[str, object]:
    client = client_observation()
    return {
        "planned_trial": {
            "trial_id": "plan-1:G1:rep-001:torrentngd:attempt-001",
            "pair_id": "plan-1:G1:rep-001",
            "attempt": 1,
            "case_id": "torrentngd:G1",
            "client_id": CLIENT_ID,
        },
        "harness_identity": harness_identity(),
        "client_observation": client,
        "client_observation_sha256": "2" * 64,
        "workload": {
            "case_id": "torrentngd:G1",
            "kind": "download",
            "protocol": "v1",
            "fixture_sha256": "3" * 64,
            "expected_payload_bytes": EXPECTED_BYTES,
        },
        "environment": {
            "host_profile": "test-host",
            "os": "Linux",
            "kernel": "6.12-test",
            "cpu_model": "test CPU",
            "cgroup_version": "v2",
            "memory_limit_bytes": None,
            "storage": {
                "profile": "local-nvme",
                "filesystem": "ext4",
                "device_id": "8:1",
            },
            "network": {"link_limit_bps": 1_000_000_000, "rtt_ms": 1.0},
            "cache_state": "controlled",
            "durability_mode": "checkpoint",
        },
        "durations_ns": {
            "barrier_wait": 100,
            "transfer": 500_000_000,
            "flush": 100,
            "validation": 200,
            "shutdown": 100,
        },
        "payload": {
            "useful_bytes": EXPECTED_BYTES,
            "received_bytes": EXPECTED_BYTES,
            "written_bytes": EXPECTED_BYTES,
            "read_bytes": 0,
            "duplicate_bytes": 0,
        },
        "resource_observation": resource_observation(),
        "resource_observation_sha256": "4" * 64,
        "integrity": {
            "status": "passed",
            "expected_sha256": "5" * 64,
            "observed_sha256": "5" * 64,
            "hash_failures": 0,
            "false_completions": 0,
        },
        "outcome": {"status": "complete", "reason": None, "exit_code": 0},
        "started_at": "2026-10-18T18:00:00Z",
        "finished_at": "2026-10-18T18:00:01Z",
    }


def build(**changes: object) -> dict[str, object]:
    values = trial_inputs()
    values.update(changes)
    return build_trial_record(**values)  # type: ignore[arg-type]


class TrialRecordTests(unittest.TestCase):
    def test_complete_trial_binds_client_and_resource_process_identity(self) -> None:
        trial = build()

        self.assertEqual(trial["client_identity"]["process_id"], PROCESS_ID)  # type: ignore[index]
        self.assertEqual(trial["resources"]["raw_artifact_sha256"], "4" * 64)  # type: ignore[index]
        self.assertEqual(trial["settings_readback"]["artifact_sha256"], "2" * 64)  # type: ignore[index]
        self.assertEqual(validate_document(trial), [])

    def test_unmeasured_startup_trial_has_no_client_or_observer_identity(self) -> None:
        values = trial_inputs()
        values.update(
            client_observation=None,
            client_observation_sha256=None,
            resource_observation=None,
            resource_observation_sha256=None,
            outcome={"status": "unmeasured", "reason": "pinned client did not start", "exit_code": 127},
            integrity={
                "status": "not_run",
                "expected_sha256": None,
                "observed_sha256": None,
                "hash_failures": 0,
                "false_completions": 0,
            },
            payload={
                "useful_bytes": 0,
                "received_bytes": 0,
                "written_bytes": 0,
                "read_bytes": 0,
                "duplicate_bytes": 0,
            },
            durations_ns={
                "barrier_wait": 0,
                "transfer": 0,
                "flush": 0,
                "validation": 0,
                "shutdown": 0,
            },
        )

        trial = build_trial_record(**values)  # type: ignore[arg-type]

        self.assertIsNone(trial["client_identity"])
        self.assertIsNone(trial["resources"]["raw_artifact_sha256"])  # type: ignore[index]
        self.assertEqual(validate_document(trial), [])

    def test_non_unmeasured_trial_requires_client_observation(self) -> None:
        with self.assertRaisesRegex(TrialRecordError, "must be unmeasured"):
            build(client_observation=None, client_observation_sha256=None)

    def test_unmeasured_trial_cannot_claim_a_started_client(self) -> None:
        values = trial_inputs()
        values["outcome"] = {
            "status": "unmeasured",
            "reason": "settings were not captured",
            "exit_code": None,
        }

        with self.assertRaisesRegex(TrialRecordError, "cannot contain a client process"):
            build_trial_record(**values)  # type: ignore[arg-type]

    def test_complete_trial_requires_exact_payload_and_matching_integrity_hashes(self) -> None:
        payload = dict(trial_inputs()["payload"])
        payload["useful_bytes"] -= 1
        with self.assertRaisesRegex(TrialRecordError, "full useful payload"):
            build(payload=payload)

        integrity = dict(trial_inputs()["integrity"])
        integrity["observed_sha256"] = "6" * 64
        with self.assertRaisesRegex(TrialRecordError, "matching payload digests"):
            build(integrity=integrity)

    def test_observation_schema_and_process_mismatch_are_rejected(self) -> None:
        observation = client_observation()
        observation["runtime_identity"]["process_id"] = True  # type: ignore[index]
        with self.assertRaisesRegex(TrialRecordError, "schema"):
            build(client_observation=observation)

        resource = resource_observation()
        resource["root_pid"] = PROCESS_ID + 1
        with self.assertRaisesRegex(TrialRecordError, "root PID"):
            build(resource_observation=resource)

    def test_trial_artifact_is_private_exclusive_and_schema_valid(self) -> None:
        trial = build()
        with tempfile.TemporaryDirectory(prefix="tng-transfer-trial-record-") as directory:
            path = Path(directory) / "trial.json"

            digest = write_trial(path, trial)

            raw = path.read_bytes()
            self.assertEqual(digest, hashlib.sha256(raw).hexdigest())
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            self.assertEqual(validate("trial", path), [])
            with self.assertRaisesRegex(TrialRecordError, "cannot create"):
                write_trial(path, trial)

    def test_trial_artifact_cannot_be_written_into_the_source_worktree(self) -> None:
        trial = build()
        path = Path(__file__).resolve().parents[1] / "trial-record-must-stay-external.json"

        with self.assertRaisesRegex(TrialRecordError, "outside the source worktree"):
            write_trial(path, trial)

        self.assertFalse(path.exists())

    def test_python_executable_hashing_is_streamed(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-hash-") as directory:
            path = Path(directory) / "large-interpreter-image"
            content = b"transfer" * 131_072
            path.write_bytes(content)

            digest, metadata = _hash_regular_nofollow(path, "test executable")

            self.assertEqual(digest, hashlib.sha256(content).hexdigest())
            self.assertEqual(metadata.st_size, len(content))


def validate_document(trial: dict[str, object]) -> list[str]:
    with tempfile.TemporaryDirectory(prefix="tng-transfer-trial-schema-") as directory:
        path = Path(directory) / "trial.json"
        path.write_text(json.dumps(trial), encoding="utf-8")
        return validate("trial", path)


if __name__ == "__main__":
    unittest.main()
