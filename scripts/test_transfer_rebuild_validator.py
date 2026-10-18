"""Regression cases for the transfer rebuild's versioned evidence schemas."""

from __future__ import annotations

import json
import hashlib
import csv
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from validate_artifact import _resource_samples_errors, validate  # noqa: E402
from ceilings import calculate_report  # noqa: E402
from manifest import ManifestError, create_manifest  # noqa: E402


SHA = "a" * 64
COMMIT = "b" * 40
SETTINGS_VALUES = {"request_queue_size": 32}
SETTINGS_SHA = hashlib.sha256(
    json.dumps(SETTINGS_VALUES, sort_keys=True, separators=(",", ":")).encode()
).hexdigest()
RESOURCE_SAMPLE_FIELDS = (
    "elapsed_ns",
    "processes",
    "rss_bytes",
    "pss_bytes",
    "cgroup_anon_bytes",
    "cgroup_file_bytes",
    "cgroup_reclaimable_cache_bytes",
    "cgroup_current_bytes",
    "cpu_time_ns",
    "threads",
    "fds",
    "block_read_bytes",
    "block_write_bytes",
    "network_namespace_inode",
    "network_rx_bytes",
    "network_tx_bytes",
)


def valid_trial() -> dict[str, object]:
    return {
        "schema_version": "transfer-trial-v1",
        "trial_id": "trial-1",
        "pair_id": "pair-1",
        "attempt": 1,
        "started_at": "2026-10-17T18:00:00Z",
        "finished_at": "2026-10-17T18:00:01Z",
        "identity": {
            "source_commit": COMMIT,
            "dirty_worktree": False,
            "lockfile_sha256": SHA,
            "binary_sha256": SHA,
            "binary_size_bytes": 1,
        },
        "client_identity": {
            "client_id": "torrentngd",
            "version": "0.1.0",
            "runtime_kind": "native_binary",
            "process_id": 1,
            "process_start_ticks": 1,
            "observation_sha256": SHA,
            "settings_sha256": SETTINGS_SHA,
        },
        "workload": {
            "case_id": "download-small",
            "kind": "download",
            "fixture_sha256": SHA,
            "expected_payload_bytes": 10,
            "peer_count": 4,
            "block_bytes": 16_384,
        },
        "environment": {
            "host_profile": "fixture",
            "os": "Linux",
            "kernel": "6.1",
            "cpu_model": "test cpu",
            "cgroup_version": "none",
            "memory_limit_bytes": None,
            "storage": {
                "profile": "ssd",
                "filesystem": "ext4",
                "device_id": None,
            },
            "network": {"link_limit_bps": None, "rtt_ms": None},
            "cache_state": "controlled",
            "durability_mode": "checkpoint",
        },
        "settings_readback": {"captured": True, "values": SETTINGS_VALUES},
        "durations_ns": {
            "barrier_wait": 0,
            "transfer": 1,
            "flush": 0,
            "validation": 0,
            "shutdown": 0,
        },
        "payload": {
            "useful_bytes": 10,
            "received_bytes": 10,
            "written_bytes": 10,
            "read_bytes": 0,
            "duplicate_bytes": 0,
        },
        "resources": {
            "observer_id": "transfer-proc-cgroup-v2",
            "sample_interval_ms": 250,
            "rss_peak_bytes": 1,
            "pss_peak_bytes": None,
            "cgroup_anon_peak_bytes": None,
            "cgroup_file_peak_bytes": None,
            "cgroup_reclaimable_cache_peak_bytes": None,
            "cgroup_current_peak_bytes": None,
            "cpu_time_ns": 1,
            "threads_peak": 1,
            "fds_peak": 1,
            "block_read_bytes": None,
            "block_write_bytes": None,
            "network_rx_bytes": None,
            "network_tx_bytes": None,
            "energy_joules": None,
            "raw_artifact_sha256": SHA,
        },
        "integrity": {
            "status": "passed",
            "expected_sha256": SHA,
            "observed_sha256": SHA,
            "hash_failures": 0,
            "false_completions": 0,
        },
        "outcome": {"status": "complete", "reason": None, "exit_code": 0},
    }


def attach_resource_artifacts(root: Path, trial: dict[str, object]) -> list[dict[str, str]]:
    artifacts_dir = root / "artifacts"
    artifacts_dir.mkdir(parents=True, exist_ok=True)
    trial_id = trial["trial_id"]
    resources = trial["resources"]
    if not isinstance(resources, dict):
        raise AssertionError("trial resources fixture must be an object")
    metric_fields = {
        "rss_bytes": "rss_peak_bytes",
        "pss_bytes": "pss_peak_bytes",
        "cgroup_anon_bytes": "cgroup_anon_peak_bytes",
        "cgroup_file_bytes": "cgroup_file_peak_bytes",
        "cgroup_reclaimable_cache_bytes": "cgroup_reclaimable_cache_peak_bytes",
        "cgroup_current_bytes": "cgroup_current_peak_bytes",
        "cpu_time_ns": "cpu_time_ns",
        "threads": "threads_peak",
        "fds": "fds_peak",
        "block_read_bytes": "block_read_bytes",
        "block_write_bytes": "block_write_bytes",
    }
    peaks = {field: resources[trial_field] for field, trial_field in metric_fields.items()}
    sample = {field: "" for field in RESOURCE_SAMPLE_FIELDS}
    sample.update({"elapsed_ns": 0, "processes": 1})
    for field, value in peaks.items():
        sample[field] = "" if value is None else int(value)
    csv_name = f"resources-{trial_id}.csv"
    csv_path = artifacts_dir / csv_name
    with csv_path.open("w", encoding="utf-8", newline="") as output:
        writer = csv.DictWriter(output, fieldnames=RESOURCE_SAMPLE_FIELDS)
        writer.writeheader()
        writer.writerow(sample)
    csv_bytes = csv_path.read_bytes()
    csv_sha = hashlib.sha256(csv_bytes).hexdigest()

    observation = {
        "schema_version": "transfer-resource-observation-v1",
        "observer_id": "transfer-proc-cgroup-v2",
        "sample_count": 1,
        "sample_interval_ms": resources["sample_interval_ms"],
        "peaks": peaks,
        "counter_note": (
            "RSS/PSS are summed across the observed process tree; cgroup values "
            "are separate domains."
        ),
        "network_note": "Network byte counters were not requested.",
        "network_interface": None,
        "network_namespace_inode": None,
        "network_rx_bytes": None,
        "network_tx_bytes": None,
        "root_pid": trial["client_identity"]["process_id"],
        "root_process_start_ticks": trial["client_identity"]["process_start_ticks"],
        "started_monotonic_ns": 1,
        "finished_monotonic_ns": 2,
        "samples_csv": csv_name,
        "samples_csv_sha256": csv_sha,
    }
    observation_path = artifacts_dir / f"resources-{trial_id}.json"
    observation_bytes = json.dumps(observation, sort_keys=True, indent=2).encode() + b"\n"
    observation_path.write_bytes(observation_bytes)
    observation_sha = hashlib.sha256(observation_bytes).hexdigest()
    resources["raw_artifact_sha256"] = observation_sha
    return [
        {
            "path": f"artifacts/{csv_name}",
            "sha256": csv_sha,
            "kind": "transfer-resource-samples-v1",
        },
        {
            "path": f"artifacts/{observation_path.name}",
            "sha256": observation_sha,
            "kind": "transfer-resource-observation-v1",
        },
    ]


def validate_document(kind: str, document: object) -> list[str]:
    with tempfile.TemporaryDirectory(prefix="tng-transfer-schema-") as directory:
        path = Path(directory) / "record.json"
        path.write_text(json.dumps(document), encoding="utf-8")
        return validate(kind, path)


def valid_client_observation() -> dict[str, object]:
    return {
        "schema_version": "transfer-client-observation-v1",
        "client_id": "torrentngd",
        "observed_at": "2026-10-17T18:00:00Z",
        "version": "0.1.0",
        "runtime_identity": {
            "kind": "native_binary",
            "binary_path": "/opt/torrentngd",
            "binary_sha256": SHA,
            "binary_size_bytes": 1,
            "process_id": 1,
            "process_start_ticks": 1,
            "process_exe_device": 1,
            "process_exe_inode": 1,
            "source_commit": COMMIT,
            "dirty_worktree": False,
            "lockfile_sha256": SHA,
            "toolchain": "rustc 1.88.0",
        },
        "settings_readback": {
            "captured": True,
            "values": SETTINGS_VALUES,
            "sha256": SETTINGS_SHA,
        },
    }


def valid_cache_state_observation() -> dict[str, object]:
    return {
        "format": "transfer-cache-control-v1",
        "fixture_sha256": SHA,
        "requested_state": "cold",
        "status": "achieved",
        "method": "linux-posix-fadvise-mincore",
        "page_bytes": 4096,
        "payload_bytes": 10,
        "resident_bytes_before": 10,
        "resident_bytes_after": 0,
        "files": [
            {
                "name": "data.bin",
                "payload_bytes": 10,
                "resident_bytes_before": 10,
                "resident_bytes_after": 0,
                "bytes_read_for_warm_state": 0,
            }
        ],
    }


def valid_network_control_observation() -> dict[str, object]:
    observation: dict[str, object] = {
        "format": "transfer-network-control-v1",
        "action": "apply",
        "created_at": "2026-10-17T18:00:00Z",
        "namespace_name": "tngt-fixture",
        "namespace_device": 1,
        "namespace_inode": 2,
        "interface": "tngt0",
        "ifindex": 3,
        "profile_id": "lan-1g-1ms-v1",
        "settings": {"delay_ms": 1, "rate_bps": 1_000_000_000, "limit_packets": 1000},
        "baseline_root_qdisc": {"kind": "noqueue", "handle": "0:"},
        "observed_root_qdisc": {
            "kind": "netem",
            "handle": "7fff:",
            "delay_ns": 1_000_000,
            "rate_bps": 1_000_000_000,
            "limit_packets": 1000,
        },
        "ip_version": "iproute2 v1",
        "tc_version": "iproute2 v1",
        "toolchain_sha256": hashlib.sha256(
            json.dumps(
                {"ip": "iproute2 v1", "tc": "iproute2 v1"},
                sort_keys=True,
                separators=(",", ":"),
                ensure_ascii=False,
            ).encode()
        ).hexdigest(),
        "applied_record_sha256": None,
    }
    canonical = json.dumps(
        observation, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode()
    observation["artifact_sha256"] = hashlib.sha256(canonical).hexdigest()
    return observation


def valid_ceiling_input() -> dict[str, object]:
    return {
        "schema_version": "transfer-ceiling-input-v1",
        "calibration_id": "ceiling-1",
        "created_at": "2026-10-17T18:00:00Z",
        "environment": {
            "host_profile": "fixture",
            "os": "Linux",
            "kernel": "6.1",
            "cpu_model": "test cpu",
            "filesystem": "ext4",
            "device_id": "8:1",
            "network_profile_id": "lan-1g-1ms-v1",
        },
        "protocol": "v1",
        "network": {
            "download_bytes_per_second": 40_000_000,
            "upload_bytes_per_second": 35_000_000,
            "tool": "iperf3",
            "tool_version": "3.16",
            "tool_sha256": "c" * 64,
            "raw_artifact_sha256": "1" * 64,
            "process_id": 101,
            "process_start_ticks": 10,
        },
        "disk": {
            "read_bytes_per_second": 55_000_000,
            "write_bytes_per_second": 45_000_000,
            "tool": "fio",
            "tool_version": "3.38",
            "tool_sha256": "d" * 64,
            "raw_artifact_sha256": "2" * 64,
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
            "raw_artifact_sha256": "3" * 64,
            "process_id": 103,
            "process_start_ticks": 30,
        },
        "pipeline": {
            "peer_count": 4,
            "requests_per_peer": 32,
            "block_bytes": 16_384,
            "rtt_ns": 50_000_000,
            "rtt_source": "private-peer-rtt",
            "raw_artifact_sha256": "4" * 64,
            "process_id": 104,
            "process_start_ticks": 40,
            "client_id": "torrentngd",
            "client_version": "0.1.0",
            "settings_sha256": SETTINGS_SHA,
            "settings_json_pointer": "/request_queue_size",
        },
    }


class TransferRebuildValidatorTests(unittest.TestCase):
    def test_complete_trial_with_integrity_and_settings_is_valid(self) -> None:
        self.assertEqual(validate_document("trial", valid_trial()), [])

    def test_historical_trial_without_resource_link_remains_schema_valid(self) -> None:
        trial = valid_trial()
        trial["resources"].pop("raw_artifact_sha256")  # type: ignore[index]

        self.assertEqual(validate_document("trial", trial), [])

    def test_client_observation_requires_pinned_runtime_and_settings(self) -> None:
        self.assertEqual(validate_document("client-observation", valid_client_observation()), [])
        observation = valid_client_observation()
        observation["runtime_identity"]["source_commit"] = "bad"  # type: ignore[index]
        errors = validate_document("client-observation", observation)
        self.assertTrue(any("source_commit" in error for error in errors))

        tampered = valid_client_observation()
        tampered["settings_readback"]["values"] = {"download_limit": 1}  # type: ignore[index]
        errors = validate_document("client-observation", tampered)
        self.assertTrue(any("canonical settings values" in error for error in errors))

    def test_resource_observation_requires_bound_namespace_scope(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-resource-schema-") as directory:
            root = Path(directory)
            trial = valid_trial()
            resource_artifacts = attach_resource_artifacts(root, trial)
            summary_path = root / resource_artifacts[1]["path"]

            self.assertEqual(validate("resource-observation", summary_path), [])

            summary = json.loads(summary_path.read_text(encoding="utf-8"))
            summary["network_interface"] = "veth0"
            summary_path.write_text(json.dumps(summary), encoding="utf-8")
            errors = validate("resource-observation", summary_path)
            self.assertTrue(any("network_note" in error for error in errors))

    def test_resource_samples_require_live_processes_and_monotonic_elapsed_time(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-resource-samples-") as directory:
            root = Path(directory)
            trial = valid_trial()
            resource_artifacts = attach_resource_artifacts(root, trial)
            summary_path = root / resource_artifacts[1]["path"]
            csv_path = root / resource_artifacts[0]["path"]
            summary = json.loads(summary_path.read_text(encoding="utf-8"))

            with csv_path.open("r", encoding="utf-8", newline="") as source:
                reader = csv.DictReader(source)
                sample = next(reader)
            sample["processes"] = "0"
            with csv_path.open("w", encoding="utf-8", newline="") as output:
                writer = csv.DictWriter(output, fieldnames=RESOURCE_SAMPLE_FIELDS)
                writer.writeheader()
                writer.writerow(sample)
            errors = _resource_samples_errors(csv_path, summary)
            self.assertTrue(any("no observed processes" in error for error in errors))

            sample["processes"] = "1"
            sample["elapsed_ns"] = "2"
            with csv_path.open("w", encoding="utf-8", newline="") as output:
                writer = csv.DictWriter(output, fieldnames=RESOURCE_SAMPLE_FIELDS)
                writer.writeheader()
                writer.writerow(sample)
            errors = _resource_samples_errors(csv_path, summary)
            self.assertTrue(any("exceeds the observation interval" in error for error in errors))

            sample["elapsed_ns"] = "0"
            with csv_path.open("w", encoding="utf-8", newline="") as output:
                writer = csv.DictWriter(output, fieldnames=RESOURCE_SAMPLE_FIELDS)
                writer.writeheader()
                writer.writerow(sample)
                writer.writerow(sample)
            summary["sample_count"] = 2
            errors = _resource_samples_errors(csv_path, summary)
            self.assertTrue(any("not strictly increasing" in error for error in errors))

    def test_resource_observation_requires_at_least_one_sample(self) -> None:
        observation = {
            "schema_version": "transfer-resource-observation-v1",
            "observer_id": "transfer-proc-cgroup-v2",
            "sample_count": 0,
            "sample_interval_ms": 250,
            "peaks": {
                "rss_bytes": None,
                "pss_bytes": None,
                "cgroup_anon_bytes": None,
                "cgroup_file_bytes": None,
                "cgroup_reclaimable_cache_bytes": None,
                "cgroup_current_bytes": None,
                "cpu_time_ns": None,
                "threads": None,
                "fds": None,
                "block_read_bytes": None,
                "block_write_bytes": None,
            },
            "counter_note": "Counters are scoped by source.",
            "network_note": "Network byte counters were not requested.",
            "network_interface": None,
            "network_namespace_inode": None,
            "network_rx_bytes": None,
            "network_tx_bytes": None,
            "root_pid": 1,
            "root_process_start_ticks": None,
            "started_monotonic_ns": 1,
            "finished_monotonic_ns": 2,
            "samples_csv": "resources.csv",
            "samples_csv_sha256": SHA,
        }

        errors = validate_document("resource-observation", observation)

        self.assertTrue(any("sample_count" in error for error in errors))

    def test_cache_state_schema_checks_observed_outcome(self) -> None:
        observation = valid_cache_state_observation()
        self.assertEqual(validate_document("cache-state", observation), [])
        observation["status"] = "partial"
        errors = validate_document("cache-state", observation)
        self.assertTrue(any("status does not match" in error for error in errors))

    def test_network_control_observation_checks_profile_and_self_digest(self) -> None:
        observation = valid_network_control_observation()
        self.assertEqual(validate_document("network-control", observation), [])
        observation["settings"]["rate_bps"] = 1  # type: ignore[index]
        errors = validate_document("network-control", observation)
        self.assertTrue(any("settings do not match" in error for error in errors))

    def test_complete_trial_requires_settings_transfer_and_integrity(self) -> None:
        trial = valid_trial()
        trial["settings_readback"] = {"captured": False, "values": {}}
        trial["durations_ns"]["transfer"] = 0  # type: ignore[index]
        trial["integrity"]["status"] = "failed"  # type: ignore[index]

        errors = validate_document("trial", trial)

        self.assertTrue(any("settings_readback" in error for error in errors))
        self.assertTrue(any("durations_ns" in error for error in errors))
        self.assertTrue(any("integrity" in error for error in errors))

    def test_trial_rejects_unknown_fields_and_malformed_hashes(self) -> None:
        trial = valid_trial()
        trial["unexpected"] = True
        trial["identity"]["binary_sha256"] = "bad"  # type: ignore[index]

        errors = validate_document("trial", trial)

        self.assertTrue(any("unexpected" in error for error in errors))
        self.assertTrue(any("binary_sha256" in error for error in errors))

    def test_manifest_requires_digest_bound_trial_entries(self) -> None:
        manifest = {
            "schema_version": "transfer-manifest-v1",
            "manifest_id": "manifest-1",
            "created_at": "2026-10-17T18:00:00Z",
            "source_commit": COMMIT,
            "worktree_dirty": False,
            "trial_files": [
                {
                    "path": "trials/trial.json",
                    "sha256": "bad",
                    "trial_id": "trial-1",
                    "pair_id": "pair-1",
                    "outcome": "complete",
                }
            ],
            "artifacts": [],
        }

        errors = validate_document("manifest", manifest)

        self.assertTrue(any("sha256" in error for error in errors))

    def test_manifest_verifies_trial_digest_identity_and_provenance(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-manifest-") as directory:
            root = Path(directory)
            (root / "trials").mkdir()
            trial = valid_trial()
            observation = valid_client_observation()
            observation_path = root / "artifacts" / "client-observation.json"
            observation_path.parent.mkdir()
            observation_bytes = json.dumps(observation).encode("utf-8")
            observation_path.write_bytes(observation_bytes)
            observation_sha = hashlib.sha256(observation_bytes).hexdigest()
            trial["client_identity"]["observation_sha256"] = observation_sha  # type: ignore[index]
            trial["client_identity"]["settings_sha256"] = SETTINGS_SHA  # type: ignore[index]
            resource_artifacts = attach_resource_artifacts(root, trial)
            trial_path = root / "trials" / "trial.json"
            trial_bytes = json.dumps(trial).encode("utf-8")
            trial_path.write_bytes(trial_bytes)
            manifest = {
                "schema_version": "transfer-manifest-v1",
                "manifest_id": "manifest-1",
                "created_at": "2026-10-17T18:00:00Z",
                "source_commit": COMMIT,
                "worktree_dirty": False,
                "trial_files": [
                    {
                        "path": "trials/trial.json",
                        "sha256": hashlib.sha256(trial_bytes).hexdigest(),
                        "trial_id": "trial-1",
                        "pair_id": "pair-1",
                        "outcome": "complete",
                    }
                ],
                "artifacts": [
                    {
                        "path": "artifacts/client-observation.json",
                        "sha256": observation_sha,
                        "kind": "transfer-client-observation-v1",
                    },
                    *resource_artifacts,
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

            self.assertEqual(
                validate("manifest", manifest_path, verify_referenced_files=True), []
            )

            trial["resources"]["cpu_time_ns"] = 99  # type: ignore[index]
            altered_trial = json.dumps(trial).encode("utf-8")
            trial_path.write_bytes(altered_trial)
            manifest["trial_files"][0]["sha256"] = hashlib.sha256(altered_trial).hexdigest()  # type: ignore[index]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            errors = validate("manifest", manifest_path, verify_referenced_files=True)
            self.assertTrue(any("resources.cpu_time_ns: differs" in error for error in errors))

            trial["resources"].pop("raw_artifact_sha256")  # type: ignore[index]
            legacy_bytes = json.dumps(trial).encode("utf-8")
            trial_path.write_bytes(legacy_bytes)
            manifest["trial_files"][0]["sha256"] = hashlib.sha256(legacy_bytes).hexdigest()  # type: ignore[index]
            manifest["artifacts"] = manifest["artifacts"][:1]  # type: ignore[index]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            self.assertEqual(
                validate("manifest", manifest_path, verify_referenced_files=True), []
            )

    def test_manifest_binds_claimed_cold_cache_to_fixture_and_observation(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-manifest-cache-") as directory:
            root = Path(directory)
            (root / "trials").mkdir()
            (root / "artifacts").mkdir()
            observation = valid_client_observation()
            observation_bytes = json.dumps(observation).encode("utf-8")
            (root / "artifacts" / "client.json").write_bytes(observation_bytes)
            observation_sha = hashlib.sha256(observation_bytes).hexdigest()
            cache_state = valid_cache_state_observation()
            cache_bytes = json.dumps(cache_state).encode("utf-8")
            (root / "artifacts" / "cache.json").write_bytes(cache_bytes)
            cache_sha = hashlib.sha256(cache_bytes).hexdigest()
            trial = valid_trial()
            trial["client_identity"]["observation_sha256"] = observation_sha  # type: ignore[index]
            trial["environment"]["cache_state"] = "cold"  # type: ignore[index]
            trial["environment"]["cache_state_observation_sha256"] = cache_sha  # type: ignore[index]
            resource_artifacts = attach_resource_artifacts(root, trial)
            trial_bytes = json.dumps(trial).encode("utf-8")
            (root / "trials" / "trial.json").write_bytes(trial_bytes)
            manifest = {
                "schema_version": "transfer-manifest-v1",
                "manifest_id": "manifest-cache-1",
                "created_at": "2026-10-17T18:00:00Z",
                "source_commit": COMMIT,
                "worktree_dirty": False,
                "trial_files": [
                    {
                        "path": "trials/trial.json",
                        "sha256": hashlib.sha256(trial_bytes).hexdigest(),
                        "trial_id": "trial-1",
                        "pair_id": "pair-1",
                        "outcome": "complete",
                    }
                ],
                "artifacts": [
                    {
                        "path": "artifacts/client.json",
                        "sha256": observation_sha,
                        "kind": "transfer-client-observation-v1",
                    },
                    {
                        "path": "artifacts/cache.json",
                        "sha256": cache_sha,
                        "kind": "transfer-cache-state-v1",
                    },
                    *resource_artifacts,
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

            self.assertEqual(
                validate("manifest", manifest_path, verify_referenced_files=True), []
            )

            cache_state["fixture_sha256"] = "c" * 64
            cache_bytes = json.dumps(cache_state).encode("utf-8")
            (root / "artifacts" / "cache.json").write_bytes(cache_bytes)
            manifest["artifacts"][1]["sha256"] = hashlib.sha256(cache_bytes).hexdigest()  # type: ignore[index]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            errors = validate("manifest", manifest_path, verify_referenced_files=True)
            self.assertTrue(any("fixture identity differs" in error for error in errors))

    def test_manifest_requires_trial_to_match_observation_identity(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-manifest-client-") as directory:
            root = Path(directory)
            (root / "trials").mkdir()
            (root / "artifacts").mkdir()
            observation = valid_client_observation()
            observation_bytes = json.dumps(observation).encode("utf-8")
            (root / "artifacts" / "client.json").write_bytes(observation_bytes)
            observation_sha = hashlib.sha256(observation_bytes).hexdigest()
            trial = valid_trial()
            trial["client_identity"]["observation_sha256"] = observation_sha  # type: ignore[index]
            trial["client_identity"]["version"] = "other"  # type: ignore[index]
            resource_artifacts = attach_resource_artifacts(root, trial)
            trial_bytes = json.dumps(trial).encode("utf-8")
            (root / "trials" / "trial.json").write_bytes(trial_bytes)
            manifest = {
                "schema_version": "transfer-manifest-v1",
                "manifest_id": "manifest-1",
                "created_at": "2026-10-17T18:00:00Z",
                "source_commit": COMMIT,
                "worktree_dirty": False,
                "trial_files": [
                    {
                        "path": "trials/trial.json",
                        "sha256": hashlib.sha256(trial_bytes).hexdigest(),
                        "trial_id": "trial-1",
                        "pair_id": "pair-1",
                        "outcome": "complete",
                    }
                ],
                "artifacts": [
                    {
                        "path": "artifacts/client.json",
                        "sha256": observation_sha,
                        "kind": "transfer-client-observation-v1",
                    },
                    *resource_artifacts,
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

            errors = validate("manifest", manifest_path, verify_referenced_files=True)

            self.assertTrue(any("client_identity.version" in error for error in errors))

    def test_manifest_binds_network_profile_to_verified_apply_report(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-manifest-network-") as directory:
            root = Path(directory)
            (root / "trials").mkdir()
            (root / "artifacts").mkdir()
            client = valid_client_observation()
            client_bytes = json.dumps(client).encode("utf-8")
            (root / "artifacts" / "client.json").write_bytes(client_bytes)
            client_sha = hashlib.sha256(client_bytes).hexdigest()
            network = valid_network_control_observation()
            network_bytes = json.dumps(network).encode("utf-8")
            (root / "artifacts" / "network.json").write_bytes(network_bytes)
            network_sha = hashlib.sha256(network_bytes).hexdigest()
            reset = dict(network)
            reset["action"] = "reset"
            reset["observed_root_qdisc"] = {"kind": "noqueue", "handle": "0:"}
            reset["applied_record_sha256"] = network_sha
            reset.pop("artifact_sha256")
            reset["artifact_sha256"] = hashlib.sha256(
                json.dumps(reset, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
            ).hexdigest()
            reset_bytes = json.dumps(reset).encode("utf-8")
            (root / "artifacts" / "network-reset.json").write_bytes(reset_bytes)
            reset_sha = hashlib.sha256(reset_bytes).hexdigest()
            trial = valid_trial()
            trial["client_identity"]["observation_sha256"] = client_sha  # type: ignore[index]
            trial["environment"]["network"].update(  # type: ignore[index]
                {
                    "link_limit_bps": 1_000_000_000,
                    "control_profile_id": "lan-1g-1ms-v1",
                    "control_observation_sha256s": [network_sha],
                }
            )
            resource_artifacts = attach_resource_artifacts(root, trial)
            trial_bytes = json.dumps(trial).encode("utf-8")
            (root / "trials" / "trial.json").write_bytes(trial_bytes)
            manifest = {
                "schema_version": "transfer-manifest-v1",
                "manifest_id": "manifest-network-1",
                "created_at": "2026-10-17T18:00:00Z",
                "source_commit": COMMIT,
                "worktree_dirty": False,
                "trial_files": [
                    {
                        "path": "trials/trial.json",
                        "sha256": hashlib.sha256(trial_bytes).hexdigest(),
                        "trial_id": "trial-1",
                        "pair_id": "pair-1",
                        "outcome": "complete",
                    }
                ],
                "artifacts": [
                    {
                        "path": "artifacts/client.json",
                        "sha256": client_sha,
                        "kind": "transfer-client-observation-v1",
                    },
                    {
                        "path": "artifacts/network.json",
                        "sha256": network_sha,
                        "kind": "transfer-network-control-v1",
                    },
                    {
                        "path": "artifacts/network-reset.json",
                        "sha256": reset_sha,
                        "kind": "transfer-network-control-v1",
                    },
                    *resource_artifacts,
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

            self.assertEqual(validate("manifest", manifest_path, verify_referenced_files=True), [])

            trial["environment"]["network"]["control_profile_id"] = "wan-20m-30ms-v1"  # type: ignore[index]
            altered_trial = json.dumps(trial).encode("utf-8")
            (root / "trials" / "trial.json").write_bytes(altered_trial)
            manifest["trial_files"][0]["sha256"] = hashlib.sha256(altered_trial).hexdigest()  # type: ignore[index]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            errors = validate("manifest", manifest_path, verify_referenced_files=True)
            self.assertTrue(any("profile differs from the trial" in error for error in errors))

    def test_manifest_recomputes_independent_ceiling_and_binds_raw_inputs(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-ceiling-manifest-") as directory:
            root = Path(directory)
            (root / "trials").mkdir()
            artifacts_dir = root / "artifacts"
            artifacts_dir.mkdir()

            raw_records = []
            for index in range(4):
                raw_bytes = f"calibration-source-{index}".encode()
                digest = hashlib.sha256(raw_bytes).hexdigest()
                raw_path = artifacts_dir / f"raw-{index}.bin"
                raw_path.write_bytes(raw_bytes)
                raw_records.append(
                    {"path": f"artifacts/raw-{index}.bin", "sha256": digest, "kind": "transfer-calibration-raw-v1"}
                )

            calibration = valid_ceiling_input()
            for index, field in enumerate(("network", "disk", "hash", "pipeline")):
                calibration[field]["raw_artifact_sha256"] = raw_records[index]["sha256"]  # type: ignore[index]
            input_bytes = json.dumps(calibration).encode("utf-8")
            input_sha = hashlib.sha256(input_bytes).hexdigest()
            (artifacts_dir / "ceiling-input.json").write_bytes(input_bytes)
            report = calculate_report(calibration, input_sha)
            report_bytes = json.dumps(report, sort_keys=True, indent=2).encode("utf-8") + b"\n"
            report_sha = hashlib.sha256(report_bytes).hexdigest()
            (artifacts_dir / "ceiling-report.json").write_bytes(report_bytes)

            client = valid_client_observation()
            client_bytes = json.dumps(client).encode("utf-8")
            client_sha = hashlib.sha256(client_bytes).hexdigest()
            (artifacts_dir / "client.json").write_bytes(client_bytes)

            trial = valid_trial()
            trial["client_identity"]["observation_sha256"] = client_sha  # type: ignore[index]
            trial["workload"]["protocol"] = "v1"  # type: ignore[index]
            trial["environment"]["storage"]["device_id"] = "8:1"  # type: ignore[index]
            trial["environment"]["network"]["network_profile_id"] = "lan-1g-1ms-v1"  # type: ignore[index]
            trial["environment"]["independent_ceiling_sha256"] = report_sha  # type: ignore[index]
            resource_artifacts = attach_resource_artifacts(root, trial)
            trial_bytes = json.dumps(trial).encode("utf-8")
            (root / "trials" / "trial.json").write_bytes(trial_bytes)

            manifest = {
                "schema_version": "transfer-manifest-v1",
                "manifest_id": "manifest-ceiling-1",
                "created_at": "2026-10-17T18:00:00Z",
                "source_commit": COMMIT,
                "worktree_dirty": False,
                "trial_files": [
                    {
                        "path": "trials/trial.json",
                        "sha256": hashlib.sha256(trial_bytes).hexdigest(),
                        "trial_id": "trial-1",
                        "pair_id": "pair-1",
                        "outcome": "complete",
                    }
                ],
                "artifacts": [
                    {"path": "artifacts/ceiling-report.json", "sha256": report_sha, "kind": "transfer-ceiling-v1"},
                    {"path": "artifacts/ceiling-input.json", "sha256": input_sha, "kind": "transfer-ceiling-input-v1"},
                    *raw_records,
                    {"path": "artifacts/client.json", "sha256": client_sha, "kind": "transfer-client-observation-v1"},
                    *resource_artifacts,
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

            self.assertEqual(validate("manifest", manifest_path, verify_referenced_files=True), [])

            calibration["network"]["process_id"] = 1  # type: ignore[index]
            calibration["network"]["process_start_ticks"] = 1  # type: ignore[index]
            input_bytes = json.dumps(calibration).encode("utf-8")
            input_sha = hashlib.sha256(input_bytes).hexdigest()
            (artifacts_dir / "ceiling-input.json").write_bytes(input_bytes)
            report = calculate_report(calibration, input_sha)
            report_bytes = json.dumps(report, sort_keys=True, indent=2).encode("utf-8") + b"\n"
            report_sha = hashlib.sha256(report_bytes).hexdigest()
            (artifacts_dir / "ceiling-report.json").write_bytes(report_bytes)
            trial["environment"]["independent_ceiling_sha256"] = report_sha  # type: ignore[index]
            trial_bytes = json.dumps(trial).encode("utf-8")
            (root / "trials" / "trial.json").write_bytes(trial_bytes)
            manifest["trial_files"][0]["sha256"] = hashlib.sha256(trial_bytes).hexdigest()  # type: ignore[index]
            manifest["artifacts"][0]["sha256"] = report_sha  # type: ignore[index]
            manifest["artifacts"][1]["sha256"] = input_sha  # type: ignore[index]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            errors = validate("manifest", manifest_path, verify_referenced_files=True)
            self.assertTrue(any("uses the candidate process" in error for error in errors))

            client = valid_client_observation()
            client_values = {"request_queue_size": 64}
            client_settings_sha = hashlib.sha256(
                json.dumps(client_values, sort_keys=True, separators=(",", ":")).encode()
            ).hexdigest()
            client["settings_readback"]["values"] = client_values  # type: ignore[index]
            client["settings_readback"]["sha256"] = client_settings_sha  # type: ignore[index]
            client_bytes = json.dumps(client).encode("utf-8")
            client_sha = hashlib.sha256(client_bytes).hexdigest()
            (artifacts_dir / "client.json").write_bytes(client_bytes)
            trial["client_identity"]["observation_sha256"] = client_sha  # type: ignore[index]
            trial["client_identity"]["settings_sha256"] = client_settings_sha  # type: ignore[index]
            trial["settings_readback"]["values"] = client_values  # type: ignore[index]
            trial_bytes = json.dumps(trial).encode("utf-8")
            (root / "trials" / "trial.json").write_bytes(trial_bytes)
            manifest["trial_files"][0]["sha256"] = hashlib.sha256(trial_bytes).hexdigest()  # type: ignore[index]
            for artifact in manifest["artifacts"]:  # type: ignore[union-attr]
                if artifact["kind"] == "transfer-client-observation-v1":
                    artifact["sha256"] = client_sha
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            errors = validate("manifest", manifest_path, verify_referenced_files=True)
            self.assertTrue(
                any("pipeline request count differs from settings readback" in error for error in errors)
            )

            report["components"]["network_download"] = 1  # type: ignore[index]
            changed_report = json.dumps(report, sort_keys=True, indent=2).encode("utf-8") + b"\n"
            (artifacts_dir / "ceiling-report.json").write_bytes(changed_report)
            manifest["artifacts"][0]["sha256"] = hashlib.sha256(changed_report).hexdigest()  # type: ignore[index]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            errors = validate("manifest", manifest_path, verify_referenced_files=True)
            self.assertTrue(any("differ from the calibration input" in error for error in errors))

            manifest["artifacts"] = [
                item
                for item in manifest["artifacts"]
                if item["kind"] != "transfer-calibration-raw-v1"
            ]  # type: ignore[index]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            errors = validate("manifest", manifest_path, verify_referenced_files=True)
            self.assertTrue(any("raw calibration artifact" in error for error in errors))

    def test_manifest_rejects_digest_mismatch_and_path_escape(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-transfer-manifest-") as directory:
            root = Path(directory)
            (root / "artifact.bin").write_bytes(b"actual content")
            manifest = {
                "schema_version": "transfer-manifest-v1",
                "manifest_id": "manifest-1",
                "created_at": "2026-10-17T18:00:00Z",
                "source_commit": COMMIT,
                "worktree_dirty": False,
                "trial_files": [
                    {
                        "path": "../outside.json",
                        "sha256": SHA,
                        "trial_id": "trial-1",
                        "pair_id": "pair-1",
                        "outcome": "complete",
                    }
                ],
                "artifacts": [{"path": "artifact.bin", "sha256": SHA, "kind": "observer"}],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")

            errors = validate("manifest", manifest_path, verify_referenced_files=True)

            self.assertTrue(any("normalized relative path" in error for error in errors))
            self.assertTrue(any("digest does not match" in error for error in errors))


class TransferManifestWriterTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="tng-transfer-manifest-writer-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.repo = self.root / "source"
        self.repo.mkdir()
        self._git("init", "-q", "-b", "main")
        self._git("config", "user.name", "TorrentNG manifest test")
        self._git("config", "user.email", "manifest-test@example.invalid")
        (self.repo / "source.txt").write_text("committed source\n", encoding="utf-8")
        self._git("add", "source.txt")
        self._git("commit", "-q", "-m", "fixture source")
        self.commit = self._git("rev-parse", "HEAD").stdout.strip()

    def _git(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["git", "-C", str(self.repo), *args],
            check=True,
            capture_output=True,
            text=True,
        )

    def _evidence_bundle(
        self, statuses: tuple[str, ...] = ("complete",)
    ) -> tuple[Path, list[Path], list[tuple[str, Path]]]:
        evidence = self.root / "evidence"
        trials_dir = evidence / "trials"
        artifacts_dir = evidence / "artifacts"
        trials_dir.mkdir(parents=True)
        artifacts_dir.mkdir()
        trial_paths: list[Path] = []
        artifact_specs: list[tuple[str, Path]] = []

        for index, status in enumerate(statuses, start=1):
            observation = valid_client_observation()
            observation["runtime_identity"]["source_commit"] = self.commit  # type: ignore[index]
            observation["runtime_identity"]["process_id"] = index  # type: ignore[index]
            observation["runtime_identity"]["process_start_ticks"] = index  # type: ignore[index]
            observation_path = artifacts_dir / f"client-{index}.json"
            observation_bytes = json.dumps(observation, sort_keys=True).encode("utf-8")
            observation_path.write_bytes(observation_bytes)
            observation_sha = hashlib.sha256(observation_bytes).hexdigest()
            artifact_specs.append(("transfer-client-observation-v1", observation_path))

            trial = valid_trial()
            trial["trial_id"] = f"trial-{status}-{index}"
            trial["pair_id"] = f"pair-{status}-{index}"
            trial["identity"]["source_commit"] = self.commit  # type: ignore[index]
            trial["client_identity"]["process_id"] = index  # type: ignore[index]
            trial["client_identity"]["process_start_ticks"] = index  # type: ignore[index]
            trial["client_identity"]["observation_sha256"] = observation_sha  # type: ignore[index]
            trial["outcome"]["status"] = status  # type: ignore[index]
            if status != "complete":
                trial["outcome"]["reason"] = "preserved regression fixture"  # type: ignore[index]
                trial["outcome"]["exit_code"] = None  # type: ignore[index]
                trial["integrity"]["status"] = "not_run"  # type: ignore[index]
            resource_artifacts = attach_resource_artifacts(evidence, trial)
            for artifact in resource_artifacts:
                artifact_specs.append((artifact["kind"], evidence / artifact["path"]))
            trial_path = trials_dir / f"trial-{index}.json"
            trial_path.write_text(json.dumps(trial, sort_keys=True), encoding="utf-8")
            trial_paths.append(trial_path)

        return evidence, trial_paths, artifact_specs

    def test_manifest_retains_all_outcomes_and_binds_exact_inputs(self) -> None:
        evidence, trials, artifacts = self._evidence_bundle(
            ("complete", "incomplete", "invalid", "failed", "unmeasured")
        )
        output = evidence / "manifest.json"

        manifest = create_manifest(output, trials, artifacts, repo=self.repo)

        self.assertFalse(manifest["worktree_dirty"])
        self.assertEqual(
            {item["outcome"] for item in manifest["trial_files"]},
            {"complete", "incomplete", "invalid", "failed", "unmeasured"},
        )
        self.assertEqual(validate("manifest", output, verify_referenced_files=True), [])
        self.assertEqual(output.stat().st_mode & 0o777, 0o600)

    def test_manifest_writer_requires_resource_observer_for_new_complete_trials(self) -> None:
        evidence, trials, artifacts = self._evidence_bundle()
        trial = json.loads(trials[0].read_text(encoding="utf-8"))
        trial["resources"].pop("raw_artifact_sha256")
        trials[0].write_text(json.dumps(trial), encoding="utf-8")

        with self.assertRaisesRegex(ManifestError, "must link a verified"):
            create_manifest(
                evidence / "manifest.json",
                trials,
                artifacts,
                repo=self.repo,
            )

    def test_manifest_rejects_dirty_source(self) -> None:
        evidence = self.root / "evidence"
        evidence.mkdir()
        (self.repo / "source.txt").write_text("uncommitted source\n", encoding="utf-8")

        with self.assertRaisesRegex(ManifestError, "source worktree is dirty"):
            create_manifest(
                evidence / "manifest.json",
                [evidence / "not-read.json"],
                [],
                repo=self.repo,
            )

    def test_manifest_rejects_evidence_inside_measured_checkout(self) -> None:
        inside = self.repo / "inside-trial.json"
        inside.write_text("not read because its location is invalid\n", encoding="utf-8")
        self._git("add", "inside-trial.json")
        self._git("commit", "-q", "-m", "fixture an in-tree evidence file")
        self.commit = self._git("rev-parse", "HEAD").stdout.strip()

        with self.assertRaisesRegex(ManifestError, "outside the measured source worktree"):
            create_manifest(self.root / "manifest.json", [inside], [], repo=self.repo)

    def test_manifest_rejects_duplicate_json_fields(self) -> None:
        evidence = self.root / "evidence"
        evidence.mkdir()
        trial = valid_trial()
        trial["identity"]["source_commit"] = self.commit  # type: ignore[index]
        serialized = json.dumps(trial, sort_keys=True)
        serialized = serialized.replace(
            '"trial_id": "trial-1",',
            '"trial_id": "trial-1", "trial_id": "shadow",',
            1,
        )
        trial_path = evidence / "trial.json"
        trial_path.write_text(serialized, encoding="utf-8")

        with self.assertRaisesRegex(ManifestError, "duplicate JSON field: trial_id"):
            create_manifest(evidence / "manifest.json", [trial_path], [], repo=self.repo)

    def test_manifest_rejects_duplicate_trial_ids(self) -> None:
        evidence, trials, artifacts = self._evidence_bundle(("complete", "failed"))
        duplicate = json.loads(trials[1].read_text(encoding="utf-8"))
        duplicate["trial_id"] = json.loads(trials[0].read_text(encoding="utf-8"))["trial_id"]
        trials[1].write_text(json.dumps(duplicate), encoding="utf-8")

        with self.assertRaisesRegex(ManifestError, "duplicate trial id"):
            create_manifest(evidence / "manifest.json", trials, artifacts, repo=self.repo)

    def test_manifest_refuses_overwrite(self) -> None:
        evidence = self.root / "evidence"
        evidence.mkdir()
        output = evidence / "manifest.json"
        output.write_text("preserve existing evidence\n", encoding="utf-8")

        with self.assertRaisesRegex(ManifestError, "refusing to overwrite"):
            create_manifest(output, [evidence / "not-read.json"], [], repo=self.repo)


if __name__ == "__main__":
    unittest.main()
