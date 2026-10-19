"""Regression sources for strict, source-bound live-catalog reports."""

from __future__ import annotations

import hashlib
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from catalog_load_report import (  # noqa: E402
    CatalogReportError,
    _extract_observations,
    build_report,
    write_report,
)
from validate_artifact import validate  # noqa: E402
from test_transfer_rebuild_trial_record import valid_unmeasured_trial  # noqa: E402


COMMIT = "b" * 40
ROUTES = (
    "/api/v1/torrents?limit=200&sort=name&reverse=true",
    "/api/v1/torrents?limit=200&offset=200",
    "/api/v1/transfer/info",
    "/metrics",
)


def observation(catalog_rows: int) -> dict[str, object]:
    routes = []
    for route in ROUTES:
        samples = list(range(1, 17))
        routes.append(
            {
                "route": route,
                "sample_count": 16,
                "p50_ns": 8,
                "p95_ns": 16,
                "samples_ns": samples,
            }
        )
    return {
        "schema_version": "transfer-live-catalog-mixed-v1",
        "catalog_rows": catalog_rows,
        "catalog_state": "restored_paused",
        "rounds": 16,
        "concurrent_routes_per_round": 4,
        "routes": routes,
    }


def retained_log(*records: dict[str, object]) -> bytes:
    return "\n".join(
        "scale_live_native_catalog_mixed_json="
        + json.dumps(record, sort_keys=True, separators=(",", ":"))
        for record in records
    ).encode("utf-8")


class CatalogLoadReportTests(unittest.TestCase):
    def test_report_binds_both_catalog_sizes_and_raw_log(self) -> None:
        log = retained_log(observation(1000), observation(100000))
        report = build_report(log, hashlib.sha256(log).hexdigest(), COMMIT)
        self.assertEqual(report["source_commit"], COMMIT)
        self.assertEqual(report["raw_log"]["size_bytes"], len(log))
        self.assertEqual(
            [item["catalog_rows"] for item in report["observations"]],
            [1000, 100000],
        )
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            self.assertEqual(validate("catalog-load-report", path), [])

    def test_parser_rejects_duplicate_fields_and_overflow_numbers(self) -> None:
        for raw in (
            '{"catalog_rows":1000,"catalog_rows":100000}',
            '{"catalog_rows":1e999}',
        ):
            with self.subTest(raw=raw), self.assertRaisesRegex(
                CatalogReportError,
                "strict JSON",
            ):
                _extract_observations(
                    ("scale_live_native_catalog_mixed_json=" + raw).encode("utf-8")
                )

    def test_parser_rejects_missing_or_duplicate_catalog_observation(self) -> None:
        with self.assertRaisesRegex(CatalogReportError, "both 1k and 100k"):
            _extract_observations(retained_log(observation(1000)))
        with self.assertRaisesRegex(CatalogReportError, "duplicate 1000-row"):
            _extract_observations(
                retained_log(observation(1000), observation(1000), observation(100000))
            )

    def test_parser_rejects_missing_route_and_untrusted_percentiles(self) -> None:
        missing_route = observation(1000)
        missing_route["routes"] = missing_route["routes"][:-1]
        with self.assertRaisesRegex(CatalogReportError, "each required route"):
            _extract_observations(retained_log(missing_route, observation(100000)))

        wrong_percentile = observation(1000)
        wrong_percentile["routes"][0]["p95_ns"] = 15
        with self.assertRaisesRegex(CatalogReportError, "percentiles do not match"):
            _extract_observations(retained_log(wrong_percentile, observation(100000)))

    def test_validator_recomputes_percentiles_after_report_creation(self) -> None:
        log = retained_log(observation(1000), observation(100000))
        report = build_report(log, hashlib.sha256(log).hexdigest(), COMMIT)
        report["observations"][0]["routes"][0]["p50_ns"] = 9
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "report.json"
            path.write_text(json.dumps(report), encoding="utf-8")
            self.assertTrue(any("p50_ns does not match" in error for error in validate("catalog-load-report", path)))

    def test_report_output_is_exclusive_and_private(self) -> None:
        log = retained_log(observation(1000), observation(100000))
        report = build_report(log, hashlib.sha256(log).hexdigest(), COMMIT)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repo = root / "source"
            repo.mkdir()
            output = root / "report.json"
            write_report(output, report, repo)
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(CatalogReportError):
                write_report(output, report, repo)

    def test_manifest_requires_the_retained_raw_log_and_same_source(self) -> None:
        log = retained_log(observation(1000), observation(100000))
        report = build_report(log, hashlib.sha256(log).hexdigest(), COMMIT)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "trials").mkdir()
            artifacts = root / "artifacts"
            artifacts.mkdir()
            log_path = artifacts / "catalog-load.log"
            log_path.write_bytes(log)
            report_path = artifacts / "catalog-load-report.json"
            report_path.write_text(json.dumps(report), encoding="utf-8")
            trial = valid_unmeasured_trial()
            trial["identity"]["source_commit"] = COMMIT
            trial_bytes = json.dumps(trial).encode("utf-8")
            trial_path = root / "trials" / "trial.json"
            trial_path.write_bytes(trial_bytes)
            manifest = {
                "schema_version": "transfer-manifest-v1",
                "manifest_id": "catalog-load-manifest",
                "created_at": "2026-10-18T12:00:00Z",
                "source_commit": COMMIT,
                "worktree_dirty": False,
                "trial_files": [
                    {
                        "path": "trials/trial.json",
                        "sha256": hashlib.sha256(trial_bytes).hexdigest(),
                        "trial_id": trial["trial_id"],
                        "pair_id": trial["pair_id"],
                        "outcome": "unmeasured",
                    }
                ],
                "artifacts": [
                    {
                        "path": "artifacts/catalog-load-report.json",
                        "sha256": hashlib.sha256(report_path.read_bytes()).hexdigest(),
                        "kind": "transfer-catalog-load-report-v1",
                    },
                    {
                        "path": "artifacts/catalog-load.log",
                        "sha256": hashlib.sha256(log).hexdigest(),
                        "kind": "transfer-catalog-load-log-v1",
                    },
                ],
            }
            manifest_path = root / "manifest.json"
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            self.assertEqual(
                validate("manifest", manifest_path, verify_referenced_files=True),
                [],
            )

            manifest["artifacts"].pop()
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            errors = validate("manifest", manifest_path, verify_referenced_files=True)
            self.assertTrue(any("retained transfer-catalog-load-log-v1" in error for error in errors))


if __name__ == "__main__":
    unittest.main()
