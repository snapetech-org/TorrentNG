"""Regression sources for the descriptive paired transfer scorecard."""

from __future__ import annotations

import sys
import json
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from scorecard import ScorecardError, analyze_trials
from validate_artifact import validate


GIB = 1024**3
FIXTURE = "a" * 64


def trial(
    client_id: str,
    *,
    pair_id: str = "pair-01",
    version: str = "1.0",
    transfer_ns: int = 1_000_000_000,
    useful_bytes: int = GIB,
    rss_peak_bytes: int = 100,
    outcome: str = "complete",
    integrity: str = "passed",
    ceiling_sha256: str = "b" * 64,
    control_sha256: str = "c" * 64,
) -> dict[str, object]:
    return {
        "pair_id": pair_id,
        "client_identity": {"client_id": client_id, "version": version},
        "workload": {
            "case_id": "N1/lan_1g",
            "kind": "download",
            "protocol": "v1",
            "fixture_sha256": FIXTURE,
            "expected_payload_bytes": GIB,
        },
        "environment": {
            "independent_ceiling_sha256": ceiling_sha256,
            "network": {"control_observation_sha256s": [control_sha256]},
        },
        "outcome": {"status": outcome},
        "integrity": {"status": integrity},
        "payload": {"useful_bytes": useful_bytes},
        "durations_ns": {"transfer": transfer_ns},
        "resources": {"rss_peak_bytes": rss_peak_bytes, "raw_artifact_sha256": "a" * 64},
    }


class PairedScorecardTests(unittest.TestCase):
    def test_comparison_uses_paired_ratios_and_ignores_artifact_ids(self) -> None:
        records = [
            trial("torrentngd", transfer_ns=1_000_000_000, rss_peak_bytes=100),
            trial(
                "qbittorrent",
                transfer_ns=2_000_000_000,
                rss_peak_bytes=80,
                ceiling_sha256="d" * 64,
                control_sha256="e" * 64,
            ),
        ]

        first = analyze_trials(records, bootstrap_samples=1_000, seed=17)
        second = analyze_trials(records, bootstrap_samples=1_000, seed=17)

        comparison = first[0]["comparisons"][0]
        goodput = comparison["metrics"]["goodput_bytes_per_second"]
        memory = comparison["metrics"]["rss_peak_bytes"]
        self.assertEqual(first, second)
        self.assertAlmostEqual(goodput["ratio_b_over_a_median"], 2.0)
        self.assertAlmostEqual(memory["ratio_b_over_a_median"], 1.25)
        self.assertEqual(goodput["paired_sample_count"], 1)
        self.assertFalse(goodput["minimum_sample_met"])

    def test_incomplete_or_integrity_failed_pair_is_excluded(self) -> None:
        cases = [
            ([trial("torrentngd"), trial("qbittorrent", outcome="incomplete")], "outcome is incomplete"),
            ([trial("torrentngd"), trial("qbittorrent", integrity="failed")], "integrity was not passed"),
        ]
        for records, reason in cases:
            with self.subTest(reason=reason):
                group = analyze_trials(records, bootstrap_samples=1_000)[0]
                self.assertEqual(group["valid_pair_count"], 0)
                self.assertEqual(group["clients"], [])
                self.assertIn(reason, group["excluded_pairs"][0]["reason"])

    def test_legacy_complete_pair_without_resource_provenance_is_excluded(self) -> None:
        records = [trial("torrentngd"), trial("qbittorrent")]
        for record in records:
            record["resources"].pop("raw_artifact_sha256")

        group = analyze_trials(records, bootstrap_samples=1_000)[0]

        self.assertEqual(group["valid_pair_count"], 0)
        self.assertIn(
            "resource observer artifact is not linked",
            group["excluded_pairs"][0]["reason"],
        )

    def test_duplicate_client_record_excludes_pair(self) -> None:
        records = [trial("torrentngd"), trial("torrentngd")]

        group = analyze_trials(records, bootstrap_samples=1_000)[0]

        self.assertEqual(group["valid_pair_count"], 0)
        self.assertIn("duplicate client", group["excluded_pairs"][0]["reason"])

    def test_client_and_resampling_budgets_fail_before_bootstrap(self) -> None:
        too_many_clients = [trial(f"client-{index:02}") for index in range(17)]
        with self.assertRaises(ScorecardError):
            analyze_trials(too_many_clients, bootstrap_samples=1_000)

        large_pair_set = [
            record
            for index in range(510)
            for record in (
                trial("torrentngd", pair_id=f"pair-{index:04}"),
                trial("qbittorrent", pair_id=f"pair-{index:04}"),
            )
        ]
        with self.assertRaisesRegex(ScorecardError, "bootstrap work"):
            analyze_trials(
                large_pair_set,
                bootstrap_samples=100_000,
                minimum_pairs=1,
            )

    def test_invalid_analysis_bounds_are_rejected(self) -> None:
        with self.assertRaises(ScorecardError):
            analyze_trials([], bootstrap_samples=True)
        with self.assertRaises(ScorecardError):
            analyze_trials([], minimum_pairs=False)

    def test_non_finite_metric_inputs_are_rejected(self) -> None:
        records = [trial("torrentngd"), trial("qbittorrent")]
        records[0]["resources"]["rss_peak_bytes"] = float("inf")

        with self.assertRaisesRegex(ScorecardError, "finite"):
            analyze_trials(records, bootstrap_samples=1_000)


class ScorecardSchemaTests(unittest.TestCase):
    def test_versioned_scorecard_schema_rejects_threshold_claims(self) -> None:
        scorecard = {
            "schema_version": "transfer-scorecard-v1",
            "created_at": "2026-10-17T12:00:00Z",
            "manifest_sha256": "a" * 64,
            "manifest_id": "manifest-01",
            "analysis": {
                "method": "paired_percentile_bootstrap_median_ratio",
                "confidence_level": 0.95,
                "bootstrap_samples": 10_000,
                "random_seed": 17,
                "minimum_paired_release_trials": 10,
                "performance_thresholds_applied": False,
            },
            "groups": [],
        }
        with tempfile.TemporaryDirectory(prefix="transfer-scorecard-") as temporary:
            path = Path(temporary) / "scorecard.json"
            path.write_text(json.dumps(scorecard), encoding="utf-8")

            self.assertEqual(validate("scorecard", path), [])
            scorecard["analysis"]["performance_thresholds_applied"] = True
            path.write_text(json.dumps(scorecard), encoding="utf-8")
            self.assertTrue(validate("scorecard", path))


if __name__ == "__main__":
    unittest.main()
