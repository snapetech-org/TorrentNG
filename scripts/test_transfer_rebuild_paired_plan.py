"""Regression sources for the PERF-110 workload catalog and pair scheduler."""

from __future__ import annotations

import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from validate_artifact import validate
from paired_plan import (
    DEFAULT_PROFILE_PATH,
    EXPECTED_CLIENT_IDS,
    EXPECTED_PROFILE_IDS,
    PlanError,
    build_case_catalog,
    build_paired_plan,
    write_new_plan,
)


class PairedPlanTests(unittest.TestCase):
    def setUp(self) -> None:
        self.catalog = build_case_catalog()

    def test_catalog_has_the_seven_by_four_case_matrix(self) -> None:
        self.assertEqual(len(self.catalog["cases"]), 28)
        self.assertEqual(self.catalog["client_ids"], list(EXPECTED_CLIENT_IDS))
        self.assertEqual(self.catalog["profile_ids"], list(EXPECTED_PROFILE_IDS))
        self.assertEqual(len({case["case_id"] for case in self.catalog["cases"]}), 28)

    def test_same_seed_produces_same_balanced_pair_schedule(self) -> None:
        pins = set(EXPECTED_CLIENT_IDS)
        first = build_paired_plan(
            self.catalog, pins, repetitions=2, seed=17, pins_sha256="a" * 64
        )
        repeated = build_paired_plan(
            self.catalog, pins, repetitions=2, seed=17, pins_sha256="a" * 64
        )

        self.assertEqual(first, repeated)
        self.assertEqual(first["schedule_algorithm"], "sha256-fisher-yates-v1")
        self.assertEqual(len(first["schedule"]), 2 * (7 + 7 + 6 + 6))
        for profile_id in EXPECTED_PROFILE_IDS:
            for repetition in (1, 2):
                pair = [
                    trial
                    for trial in first["schedule"]
                    if trial["profile_id"] == profile_id
                    and trial["repetition"] == repetition
                ]
                self.assertGreaterEqual(len(pair), 2)
                self.assertEqual(len({trial["client_id"] for trial in pair}), len(pair))
                self.assertEqual(len({trial["pair_id"] for trial in pair}), 1)

    def test_catalog_mutation_after_digest_is_rejected(self) -> None:
        changed = {**self.catalog, "cases": [dict(case) for case in self.catalog["cases"]]}
        changed["cases"][0]["expected_payload_bytes"] += 1
        with self.assertRaisesRegex(PlanError, "digest"):
            build_paired_plan(
                changed,
                {"torrentngd", "qbittorrent"},
                repetitions=1,
                seed=17,
                pins_sha256="e" * 64,
            )

    def test_paired_plan_validator_checks_the_schema_and_seeded_schedule(self) -> None:
        plan = build_paired_plan(
            self.catalog,
            {"torrentngd", "qbittorrent"},
            repetitions=1,
            seed=21,
            pins_sha256="f" * 64,
        )
        with tempfile.TemporaryDirectory() as temporary:
            artifact = Path(temporary) / "plan.json"
            artifact.write_text(json.dumps(plan), encoding="utf-8")
            self.assertEqual(validate("paired-plan", artifact), [])

            plan["schedule"][0]["pair_id"] = "tampered"
            artifact.write_text(json.dumps(plan), encoding="utf-8")
            self.assertTrue(validate("paired-plan", artifact))

    def test_unpinned_and_unsupported_slots_are_explicitly_excluded(self) -> None:
        plan = build_paired_plan(
            self.catalog,
            {"torrentngd", "qbittorrent"},
            repetitions=1,
            seed=5,
            pins_sha256="b" * 64,
        )

        excluded_ids = {entry["case_id"] for entry in plan["excluded_cases"]}
        self.assertIn("urtorrent:G3/v2-piece-layer", excluded_ids)
        self.assertIn("urtorrent:G3/hybrid-padding", excluded_ids)
        self.assertIn("transmission:N1/lan_1g", excluded_ids)
        self.assertEqual(len(plan["schedule"]), 8)

    def test_a_pair_is_not_scheduled_with_fewer_than_two_pinned_clients(self) -> None:
        plan = build_paired_plan(
            self.catalog,
            {"torrentngd"},
            repetitions=1,
            seed=3,
            pins_sha256="c" * 64,
        )

        self.assertEqual(plan["schedule"], [])
        self.assertEqual(len(plan["excluded_pairs"]), 4)

    def test_profile_scope_allows_v1_only_comparisons_and_binds_custom_geometry(self) -> None:
        profile_document = json.loads(DEFAULT_PROFILE_PATH.read_text(encoding="utf-8"))
        profile_document["profiles"]["N1/lan_1g"]["files"][0]["size_bytes"] = 128 * 1024 * 1024
        with tempfile.TemporaryDirectory() as temporary:
            profiles = Path(temporary) / "profiles.json"
            profiles.write_text(json.dumps(profile_document), encoding="utf-8")
            catalog = build_case_catalog(profiles)
            plan = build_paired_plan(
                catalog,
                {"torrentngd", "urtorrent"},
                repetitions=3,
                seed=17,
                pins_sha256="d" * 64,
                profile_scope=["N1/lan_1g"],
            )
            plan_path = Path(temporary) / "scoped-plan.json"
            plan_path.write_text(json.dumps(plan), encoding="utf-8")

            self.assertEqual(plan["profile_scope"], ["N1/lan_1g"])
            self.assertEqual(len(plan["schedule"]), 6)
            self.assertEqual(plan["excluded_pairs"], [])
            self.assertTrue(all(item["profile_id"] == "N1/lan_1g" for item in plan["schedule"]))
            self.assertEqual(
                next(
                    case["expected_payload_bytes"]
                    for case in catalog["cases"]
                    if case["case_id"] == "torrentngd:N1/lan_1g"
                ),
                128 * 1024 * 1024,
            )
            self.assertEqual(validate("paired-plan", plan_path), [])

    def test_profile_scope_rejects_duplicates_and_unknown_profiles(self) -> None:
        for scope in (["N1/lan_1g", "N1/lan_1g"], ["N1/not-pinned"]):
            with self.subTest(scope=scope), self.assertRaises(PlanError):
                build_paired_plan(
                    self.catalog,
                    {"torrentngd", "urtorrent"},
                    repetitions=1,
                    seed=17,
                    pins_sha256="e" * 64,
                    profile_scope=list(scope),
                )

    def test_invalid_bounds_and_overwrite_fail_closed(self) -> None:
        with self.assertRaises(PlanError):
            build_paired_plan(
                self.catalog,
                {"torrentngd", "qbittorrent"},
                repetitions=0,
                seed=1,
                pins_sha256="d" * 64,
            )

        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "plan.json"
            plan = build_paired_plan(
                self.catalog,
                {"torrentngd", "qbittorrent"},
                repetitions=1,
                seed=1,
                pins_sha256="d" * 64,
            )
            write_new_plan(output, plan)
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
            with self.assertRaises(PlanError):
                write_new_plan(output, plan)


if __name__ == "__main__":
    unittest.main()
