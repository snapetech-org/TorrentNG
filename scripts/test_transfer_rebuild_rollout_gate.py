"""Regression sources for the source-bound rollout qualification gate."""

from __future__ import annotations

import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from rollout_gate import (  # noqa: E402
    REQUIRED_CHECKS,
    REQUIRED_KINDS,
    RolloutGateError,
    _candidate_trial_errors,
    _proof_kind_errors,
    _read_json_artifact,
    _write_exclusive,
)
from qualification_record import QualificationRecordError, build_record  # noqa: E402
from rollout_index import build_index  # noqa: E402
from validate_artifact import validate  # noqa: E402


COMMIT = "b" * 40
DIGEST = "a" * 64


def trial(
    *,
    trial_id: str,
    pair_id: str,
    protocol: str,
    workload: str,
) -> dict[str, object]:
    return {
        "trial_id": trial_id,
        "pair_id": pair_id,
        "identity": {"source_commit": COMMIT, "dirty_worktree": False},
        "client_identity": {"client_id": "torrentngd"},
        "workload": {
            "protocol": protocol,
            "kind": workload,
            "case_id": f"{protocol}-{workload}",
            "expected_payload_bytes": 4096,
        },
        "outcome": {"status": "complete"},
        "integrity": {"status": "passed"},
        "payload": {"useful_bytes": 4096},
    }


class RolloutGateTests(unittest.TestCase):
    def test_each_final_stage_has_explicit_evidence_kinds(self) -> None:
        self.assertEqual(set(REQUIRED_KINDS), {"IO-016", "NET-116", "PERF-116", "SYS-015"})
        self.assertNotIn("transfer-independent-review-v1", REQUIRED_KINDS["SYS-015"])
        self.assertEqual(
            REQUIRED_CHECKS["transfer-gate-policy-v1"],
            {"performance-thresholds-explicitly-unapplied"},
        )
        self.assertIn("transfer-catalog-load-v1", REQUIRED_KINDS["SYS-015"])
        self.assertIn(
            "mixed-native-api-latencies-retained",
            REQUIRED_CHECKS["transfer-catalog-load-v1"],
        )
        self.assertEqual(
            _proof_kind_errors(
                "transfer-catalog-load-v1",
                {
                    "id": "mixed-native-api-latencies-retained",
                    "evidence_sha256s": [DIGEST],
                },
                {DIGEST: ("transfer-catalog-load-report-v1", "catalog-report.json")},
            ),
            [],
        )
        self.assertTrue(
            any(
                "requires transfer-catalog-load-report-v1 proof" in error
                for error in _proof_kind_errors(
                    "transfer-catalog-load-v1",
                    {
                        "id": "mixed-native-api-latencies-retained",
                        "evidence_sha256s": [DIGEST],
                    },
                    {DIGEST: ("transfer-catalog-load-log-v1", "catalog.log")},
                )
            )
        )
        self.assertIn("transfer-recovery-qualification-v1", REQUIRED_KINDS["PERF-116"])

    def test_candidate_coverage_counts_unique_pairs_and_requires_all_protocols(self) -> None:
        records = []
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            trial_files = []
            for protocol in ("v1", "v2", "hybrid"):
                for workload in ("download", "seed"):
                    for repeat in range(2):
                        value = trial(
                            trial_id=f"{protocol}-{workload}-{repeat}",
                            pair_id=f"{protocol}-{workload}-{repeat}",
                            protocol=protocol,
                            workload=workload,
                        )
                        payload = json.dumps(value, sort_keys=True).encode()
                        path = root / f"{value['trial_id']}.json"
                        path.write_bytes(payload)
                        trial_files.append(
                            {"path": path.name, "sha256": hashlib.sha256(payload).hexdigest()}
                        )
            manifest = {"source_commit": COMMIT, "trial_files": trial_files}
            self.assertEqual(_candidate_trial_errors(root / "manifest.json", manifest, "torrentngd", 2), [])

            duplicate = trial_files[0]
            manifest["trial_files"].append(duplicate)
            errors = _candidate_trial_errors(root / "manifest.json", manifest, "torrentngd", 3)
            self.assertTrue(any("v1/download has 2 complete trials" in error for error in errors))

    def test_incomplete_or_dirty_candidate_trials_block_coverage(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            value = trial(trial_id="bad", pair_id="p1", protocol="v1", workload="download")
            value["identity"]["dirty_worktree"] = True
            value["integrity"]["status"] = "failed"
            payload = json.dumps(value).encode()
            (root / "trial.json").write_bytes(payload)
            manifest = {
                "source_commit": COMMIT,
                "trial_files": [{"path": "trial.json", "sha256": hashlib.sha256(payload).hexdigest()}],
            }
            errors = _candidate_trial_errors(root / "manifest.json", manifest, "torrentngd", 1)
            self.assertTrue(any("dirty worktree" in error for error in errors))
            self.assertTrue(any("integrity" in error for error in errors))
            self.assertTrue(any("v2/download" in error for error in errors))

    def test_qualification_record_cannot_pass_a_failed_check(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "qualification.json"
            path.write_text(
                json.dumps(
                    {
                        "schema_version": "transfer-qualification-v1",
                        "kind": "transfer-live-parity-v1",
                        "source_commit": COMMIT,
                        "status": "passed",
                        "checks": [
                            {"id": "parity", "status": "failed", "evidence_sha256s": [DIGEST]}
                        ],
                        "notes": "",
                    }
                ),
                encoding="utf-8",
            )
            self.assertTrue(any("cannot pass" in error for error in validate("qualification", path)))

    def test_qualification_builder_requires_every_named_check(self) -> None:
        checks = [
            {"id": check_id, "status": "passed", "evidence_sha256s": [DIGEST]}
            for check_id in sorted(REQUIRED_CHECKS["transfer-live-parity-v1"])
        ]
        record = build_record("transfer-live-parity-v1", COMMIT, checks)
        self.assertEqual(record["status"], "passed")
        incomplete = build_record(
            "transfer-live-parity-v1",
            COMMIT,
            [{"id": "x", "status": "passed", "evidence_sha256s": [DIGEST]}],
        )
        self.assertEqual(incomplete["status"], "blocked")
        with self.assertRaises(QualificationRecordError):
            build_record("transfer-live-parity-v1", "not-a-commit", checks)

    def test_rollout_artifact_reader_rejects_duplicate_and_overflow_json(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "artifact.json"
            for raw in ('{"outer":{"value":1,"value":2}}', '{"value":1e999}'):
                path.write_text(raw, encoding="utf-8")
                with self.subTest(raw=raw), self.assertRaisesRegex(
                    RolloutGateError,
                    "strict JSON",
                ):
                    _read_json_artifact(path, "test artifact")

    def test_gate_output_is_private_exclusive_and_not_overwritten(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "gate.json"
            _write_exclusive(path, {"decision": "blocked"})
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(RolloutGateError):
                _write_exclusive(path, {"decision": "release_eligible"})
            self.assertEqual(json.loads(path.read_text(encoding="utf-8"))["decision"], "blocked")

    def test_rollout_index_binds_manifest_and_scorecard_digests(self) -> None:
        manifest = {
            "manifest_id": "manifest-1",
            "source_commit": COMMIT,
            "artifacts": [],
        }
        scorecard = {"manifest_id": "manifest-1", "manifest_sha256": DIGEST}
        declarations = {
            "candidate_client_id": "torrentngd",
            "stages": {
                stage: {"status": "blocked", "evidence": [], "notes": "pending"}
                for stage in REQUIRED_KINDS
            },
        }
        index = build_index(manifest, DIGEST, scorecard, "c" * 64, declarations)
        self.assertEqual(index["manifest_sha256"], DIGEST)
        self.assertEqual(index["scorecard_sha256"], "c" * 64)
        with self.assertRaises(RolloutGateError):
            build_index(manifest, DIGEST, scorecard, "c" * 64, declarations | {
                "stages": {**declarations["stages"], "IO-016": {
                    "status": "passed",
                    "evidence": [{"kind": "transfer-live-parity-v1", "sha256": DIGEST}],
                    "notes": "",
                }}
            })


if __name__ == "__main__":
    unittest.main()
