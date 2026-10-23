#!/usr/bin/env python3
"""Offline regression tests for build attribution and paired qualification."""
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

import native_seed_pgo as pgo
import seed_efficiency_tools as tools


def experiment(control, candidate, *, clean=True):
    runs = []
    for label, values in (("control", control), ("candidate", candidate)):
        for index, value in enumerate(values):
            runs.append({"name": label, "round": index + 1, "binary_sha256": label,
                         "transfer": {"verified": True, "bytes": 1024**3},
                         "payload_cpu_seconds_user_system": value,
                         "active_peak_pss_bytes": value * 1024,
                         "active_peak_anonymous_bytes": value * 512})
    return {"status": "complete", "runs": runs,
            "build_manifests": {name: {"dirty": not clean, "binary_sha256": name}
                                for name in ("control", "candidate")}}


class QualificationTests(unittest.TestCase):
    def test_paired_ratios_not_ratio_of_unrelated_medians(self):
        result = tools.paired_analysis(experiment([1, 100, 101], [2, 50, 202]),
                                       "control", "candidate", minimum_rounds=3)
        metric = result["metrics"]["cpu_seconds_per_gib"]
        self.assertEqual(metric["paired_ratios"], [2, .5, 2])
        self.assertEqual(metric["paired_median"], 2)
        self.assertEqual(result["resource_gate"], "inconclusive_or_regression")

    def test_joint_gate_requires_both_resources_and_clean_provenance(self):
        report = experiment([10] * 10, [9] * 10)
        self.assertEqual(tools.paired_analysis(report, "control", "candidate")["resource_gate"], "pass")
        report["runs"][-1]["active_peak_pss_bytes"] *= 2
        # Repeat the memory regression in all candidate rounds.
        for run in report["runs"][10:]: run["active_peak_pss_bytes"] = 11 * 1024
        self.assertNotEqual(tools.paired_analysis(report, "control", "candidate")["resource_gate"], "pass")
        report = experiment([10] * 10, [9] * 10, clean=False)
        self.assertFalse(tools.paired_analysis(report, "control", "candidate")["clean_provenance"])

    def test_incomplete_mismatched_instrumented_and_rebuilt_runs_fail_closed(self):
        original = experiment([10] * 10, [9] * 10)
        mutations = [lambda r: r.update(status="running"),
                     lambda r: r["runs"].pop(),
                     lambda r: r["runs"][0].update(diagnostic_run=True),
                     lambda r: r["runs"][0].update(binary_sha256="different"),
                     lambda r: r["runs"][0]["transfer"].update(verified=False),
                     lambda r: r["runs"][0]["transfer"].update(bytes=123),
                     lambda r: r["runs"].append(copy.deepcopy(r["runs"][0]))]
        for mutation in mutations:
            with self.subTest(mutation=mutation):
                report = copy.deepcopy(original)
                mutation(report)
                with self.assertRaises(ValueError): tools.paired_analysis(report, "control", "candidate")

    def test_concurrent_cpu_normalization_uses_transferred_bytes(self):
        report = experiment([10] * 10, [9] * 10)
        for run in report["runs"]: run["transfer"]["bytes"] *= 3
        result = tools.paired_analysis(report, "control", "candidate")
        self.assertAlmostEqual(result["metrics"]["cpu_seconds_per_gib"]["paired_median"], .9)

    def test_every_position_is_balanced_in_each_full_block(self):
        orders = tools.balanced_orders(5, 15, 42)
        self.assertEqual(orders, tools.balanced_orders(5, 15, 42))
        for start in range(0, 15, 5):
            for position in range(5):
                self.assertEqual(sorted(order[position] for order in orders[start:start + 5]), list(range(5)))
        with self.assertRaises(ValueError): tools.balanced_orders(0, 1, 0)

    def test_binary_and_profile_hash_mismatches_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "binary"
            binary.write_bytes(b"a")
            data = {"format": "torrentng-build-manifest-v1", "binary_sha256": tools.sha256(binary),
                    "source_digest": "source", "cargo_lock_sha256": "lock"}
            tools.verify_manifest(data, binary)
            binary.write_bytes(b"b")
            with self.assertRaises(ValueError): tools.verify_manifest(data, binary)
            profile = Path(directory) / "seed.profdata"
            profile.write_bytes(b"profile")
            metadata = {"source_digest": "source", "rustc": "compiler", "profile_sha256": tools.sha256(profile)}
            Path(str(profile) + ".manifest.json").write_text(json.dumps(metadata))
            with patch.object(pgo, "source_digest", return_value="source"), patch.object(pgo, "command", return_value="compiler"):
                pgo.validate_profile(profile)
                profile.write_bytes(b"changed")
                with self.assertRaises(ValueError): pgo.validate_profile(profile)
            with patch.object(pgo, "source_digest", return_value="different"):
                with self.assertRaises(ValueError): pgo.validate_profile(profile)


if __name__ == "__main__":
    unittest.main()
