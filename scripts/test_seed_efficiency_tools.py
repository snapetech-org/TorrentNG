#!/usr/bin/env python3
"""Offline regression tests for build attribution and paired qualification."""
import copy
import gzip
import json
import os
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
    def test_tree_identity_survives_commit_metadata_changes_but_source_edits_do_not(self):
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            def git(*args):
                return tools.command("git", "-c", "user.name=Manifest Test", "-c",
                                     "user.email=manifest@example.invalid", *args, cwd=repo)
            git("init", "-q")
            (repo / "Cargo.toml").write_text("original source")
            git("add", "Cargo.toml")
            git("commit", "-qm", "local metadata")
            original = tools.source_provenance(repo)
            git("commit", "--amend", "-qm", "published metadata")
            published = tools.source_provenance(repo)
            self.assertNotEqual(original["source_commit"], published["source_commit"])
            self.assertEqual(original["source_tree"], published["source_tree"])
            self.assertEqual(original["source_digest"], published["source_digest"])
            self.assertFalse(published["dirty"])
            (repo / "Cargo.toml").write_text("changed source")
            dirty = tools.source_provenance(repo)
            self.assertEqual(dirty["source_tree"], published["source_tree"])
            self.assertNotEqual(dirty["source_digest"], published["source_digest"])
            self.assertTrue(dirty["dirty"])

    def test_dependency_source_edits_make_the_entire_manifest_dirty(self):
        source = {"dirty": False, "source_commit": "app", "source_tree": "app-tree"}
        dependency = {"dirty": True, "source_commit": "lib", "source_tree": "lib-tree"}
        with patch.object(tools, "source_provenance", side_effect=[source, dependency]), \
                patch.object(tools, "sha256", return_value="hash"), \
                patch.object(tools, "command", return_value="compiler"):
            result = tools.manifest("/binary", "/app", kind="rtorrent",
                                    dependency_repos={"libtorrent": "/libtorrent"})
        self.assertTrue(result["dirty"])
        self.assertEqual(result["dependency_sources"]["libtorrent"], dependency)

    def test_runtime_shared_library_must_still_match_the_manifest(self):
        with tempfile.TemporaryDirectory() as directory:
            binary, library = Path(directory) / "app", Path(directory) / "lib.so"
            binary.write_bytes(b"app")
            library.write_bytes(b"library")
            data = {"format": "torrentng-build-manifest-v1", "kind": "rtorrent",
                    "source_digest": "source", "binary_sha256": tools.sha256(binary),
                    "shared_libraries_sha256": {str(library): tools.sha256(library)}}
            tools.verify_manifest(data, binary)
            library.write_bytes(b"changed")
            with self.assertRaisesRegex(ValueError, "shared library differs"):
                tools.verify_manifest(data, binary)
            library.unlink()
            with self.assertRaisesRegex(ValueError, "shared library differs"):
                tools.verify_manifest(data, binary)

    def test_training_can_use_an_older_clean_checkout_without_copying_tools(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "older checkout"
            source.mkdir()
            output = root / "training"
            corpus = root / "corpus.json"
            corpus.write_text(json.dumps({"training": [{"mib": 16, "mib_per_second": 4,
                "peers": 1, "window": 16, "pattern": "sequential"}]}))
            profdata = root / "lib/rustlib/test-host/bin/llvm-profdata"
            profdata.parent.mkdir(parents=True)
            profdata.touch()

            def fake_run(argv, **kwargs):
                if argv[0] == "cargo":
                    self.assertEqual(kwargs["cwd"], source)
                    binary = output / "instrumented/release/torrentngd"
                    binary.parent.mkdir(parents=True)
                    binary.write_bytes(b"instrumented daemon")
                elif argv[0] == profdata:
                    Path(argv[3]).write_bytes(b"profile")
                else:
                    self.assertEqual(argv[1], pgo.PROBE)
                    (output / "raw/daemon-test.profraw").write_bytes(b"raw")

            argv = ["native_seed_pgo.py", "train", "--repo", str(source),
                    "--output", str(output), "--corpus", str(corpus)]
            with patch.object(sys, "argv", argv), patch.dict(os.environ, {}, clear=True), \
                    patch.object(pgo, "run", side_effect=fake_run), \
                    patch.object(pgo, "source_digest", return_value="source") as digest, \
                    patch.object(pgo, "manifest", return_value={}) as manifest, \
                    patch.object(pgo, "command", side_effect=[str(root), "host: test-host"]), \
                    patch("builtins.print"):
                pgo.main()
                self.assertTrue(all(call.args == (source,) for call in digest.call_args_list))
                self.assertEqual(manifest.call_args.args[1], source)
            self.assertEqual(list(source.iterdir()), [], "the source checkout stays untouched")
            self.assertTrue((output / "seed.profdata.manifest.json").is_file())

    def test_validate_cli_passes_the_selected_source_checkout(self):
        source = Path("/source/baseline")
        argv = ["native_seed_pgo.py", "validate", "--repo", str(source), "--profile", "seed.profdata"]
        with patch.object(sys, "argv", argv), patch.object(pgo, "validate_profile") as validate:
            pgo.main()
            validate.assert_called_once_with(Path("seed.profdata"), source, "default")

    def test_mapped_training_exercises_activation_and_buffered_fallback(self):
        workloads = [{"mib": 16, "peers": 1, "environment": {"TNG_SEED_DRIVER": "1"}}]
        self.assertEqual(pgo.training_workloads({"training": workloads}, "default"), workloads)
        variants = pgo.training_workloads({"training": workloads}, "seed-mmap")
        self.assertEqual([w["environment"]["TNG_SEED_MMAP"] for w in variants], ["0", "1"])
        self.assertTrue(all(w["environment"]["TNG_SEED_DRIVER"] == "1" for w in variants))
        self.assertEqual(pgo.feature_identity("seed-mmap,full-compat,default,seed-mmap"),
                         "full-compat,seed-mmap")
        for bad in ("", "seed-mmap --release", "x/y", ",seed-mmap"):
            with self.assertRaises(ValueError): pgo.feature_identity(bad)

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
        with self.assertRaises(ValueError): tools.paired_analysis(original, "control", "control")
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
                with self.assertRaisesRegex(ValueError, "features"):
                    pgo.validate_profile(profile, features="seed-mmap")
                compressed = Path(str(profile) + ".gz")
                with gzip.open(compressed, "wb") as stream: stream.write(profile.read_bytes())
                Path(str(compressed) + ".manifest.json").write_text(json.dumps(metadata))
                pgo.validate_profile(compressed)
                profile.write_bytes(b"changed")
                with self.assertRaises(ValueError): pgo.validate_profile(profile)
            with patch.object(pgo, "source_digest", return_value="different"):
                with self.assertRaises(ValueError): pgo.validate_profile(profile)


if __name__ == "__main__":
    unittest.main()
