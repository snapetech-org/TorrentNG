"""Source regressions for paired-trial artifact linkage semantics."""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

TOOLS = Path(__file__).parent / "transfer_rebuild"
sys.path.insert(0, str(TOOLS))

from validate_artifact import (  # noqa: E402
    _load_strict_json,
    _cache_state_link_errors,
    _network_control_errors,
    _resource_namespace_errors,
    _retargeted_metainfo_errors,
    _retargeted_topology_errors,
    _task_tracker_errors,
    _task_tracker_topology_errors,
)
from network_control import expected_netem, profile_settings  # noqa: E402


def _signed_record(value: dict[str, object]) -> dict[str, object]:
    record = dict(value)
    canonical = json.dumps(record, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    record["artifact_sha256"] = hashlib.sha256(canonical).hexdigest()
    return record


class PairedEvidenceLinkTests(unittest.TestCase):
    def test_strict_artifact_json_rejects_duplicates_and_overflow_numbers(self) -> None:
        with tempfile.TemporaryDirectory(prefix="transfer-strict-json-") as directory:
            artifact = Path(directory) / "artifact.json"
            artifact.write_text('{"value":1,"value":2}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "duplicate JSON field"):
                _load_strict_json(artifact)

            artifact.write_text('{"value":1e999}', encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "outside the finite range"):
                _load_strict_json(artifact)

    def setUp(self) -> None:
        self.topology = {
            "artifact_sha256": "a" * 64,
            "gateway": "198.18.4.1",
            "target": {
                "address": "198.18.4.3",
                "namespace_identity": {"device": 1, "inode": 101},
            },
            "seed": {
                "address": "198.18.4.2",
                "namespace_identity": {"device": 1, "inode": 202},
            },
        }

    def test_seed_trial_observer_must_sample_seed_namespace(self) -> None:
        observation = {"network_namespace_inode": 202}
        self.assertEqual(_resource_namespace_errors("seed", observation, self.topology), [])
        observation["network_namespace_inode"] = 101
        self.assertEqual(
            _resource_namespace_errors("seed", observation, self.topology),
            ["resource observer used a different seed namespace"],
        )

    def test_unknown_cache_state_preserves_partial_or_unsupported_evidence(self) -> None:
        observation = {
            "fixture_sha256": "a" * 64,
            "requested_state": "cold",
            "status": "partial",
        }
        self.assertEqual(_cache_state_link_errors("unknown", "a" * 64, observation), [])
        observation["status"] = "unsupported"
        self.assertEqual(_cache_state_link_errors("unknown", "a" * 64, observation), [])
        observation["status"] = "achieved"
        self.assertEqual(
            _cache_state_link_errors("unknown", "a" * 64, observation),
            ["an achieved cache state cannot be recorded as unknown"],
        )

    def test_claimed_cache_state_requires_matching_achieved_observation(self) -> None:
        observation = {
            "fixture_sha256": "a" * 64,
            "requested_state": "cold",
            "status": "partial",
        }
        self.assertEqual(
            _cache_state_link_errors("cold", "a" * 64, observation),
            ["cache state was not achieved"],
        )

    def test_download_trial_observer_must_sample_target_namespace(self) -> None:
        observation = {"network_namespace_inode": 101}
        self.assertEqual(_resource_namespace_errors("download", observation, self.topology), [])
        observation["network_namespace_inode"] = 202
        self.assertEqual(
            _resource_namespace_errors("download", observation, self.topology),
            ["resource observer used a different target namespace"],
        )

    def test_seeded_loss_jitter_artifact_must_match_profile_and_observed_seed(self) -> None:
        profile_id = "wan-20m-30ms-jitter-loss-v1"
        seed = 0x123456789ABCDEF0
        versions = {"ip": "ip v1", "tc": "tc v1"}
        toolchain_sha = hashlib.sha256(
            json.dumps(versions, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest()
        record = {
            "format": "transfer-network-control-v1",
            "action": "apply",
            "created_at": "2026-10-18T12:00:00Z",
            "namespace_name": "tngt-case-1",
            "namespace_device": 8,
            "namespace_inode": 9,
            "interface": "tngt0",
            "ifindex": 17,
            "profile_id": profile_id,
            "settings": profile_settings(profile_id, seed),
            "baseline_root_qdisc": {"kind": "noqueue", "handle": "0:"},
            "observed_root_qdisc": expected_netem(profile_id, seed),
            "ip_version": versions["ip"],
            "tc_version": versions["tc"],
            "toolchain_sha256": toolchain_sha,
            "applied_record_sha256": None,
        }
        record = _signed_record(record)
        self.assertEqual(_network_control_errors(record), [])

        record["settings"]["seed"] = seed + 1
        record = _signed_record({key: value for key, value in record.items() if key != "artifact_sha256"})
        self.assertIn(
            "observed root qdisc does not match the pinned profile",
            _network_control_errors(record),
        )

    def test_retargeted_tracker_url_must_use_the_recorded_task_gateway(self) -> None:
        record = {"announce_url": "http://198.18.4.1:51415/announce"}
        self.assertEqual(_retargeted_topology_errors(record, self.topology), [])
        record["announce_url"] = "http://198.18.4.9:51415/announce"
        self.assertEqual(
            _retargeted_topology_errors(record, self.topology),
            ["retargeted announce URL does not use the task gateway tracker"],
        )

    def test_retargeted_v2_hash_record_checks_truncated_info_hash(self) -> None:
        record = {
            "metainfo_kind": "v2",
            "info_hashes": {
                "sha256": "a" * 64,
                "sha256_truncated_20": "a" * 40,
            },
            "announce_url": "http://198.18.4.1:51415/announce",
        }
        self.assertEqual(_retargeted_metainfo_errors(record), [])
        record["info_hashes"]["sha256_truncated_20"] = "b" * 40  # type: ignore[index]
        self.assertEqual(
            _retargeted_metainfo_errors(record),
            ["sha256_truncated_20 is not the first 20 bytes of sha256"],
        )

    def test_tracker_snapshot_binds_counts_seed_identity_and_topology(self) -> None:
        tracker = _signed_record({
            "schema_version": "transfer-task-tracker-v1",
            "topology_sha256": "a" * 64,
            "gateway": "198.18.4.1",
            "target_address": "198.18.4.3",
            "target_port": 45678,
            "target_peer_id_sha256": "c" * 64,
            "target_ready": True,
            "seed_address": "198.18.4.2",
            "seed_port": 51413,
            "seed_peer_id_sha256": "b" * 64,
            "seed_ready": True,
            "info_hashes_sha256": ["c" * 64],
            "announce_counts": {"started": 2, "completed": 1, "stopped": 0, "other": 0},
            "request_count": 3,
        })
        self.assertEqual(_task_tracker_errors(tracker), [])
        self.assertEqual(_task_tracker_topology_errors(tracker, self.topology), [])
        tracker["request_count"] = 4
        self.assertIn("request_count does not equal the announce counters", _task_tracker_errors(tracker))
        self.assertEqual(
            _task_tracker_topology_errors(
                {**tracker, "gateway": "198.18.4.254"}, self.topology
            ),
            ["tracker gateway differs from the task-network topology"],
        )


if __name__ == "__main__":
    unittest.main()
