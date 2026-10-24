import gzip
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parent))
import native_seed_function_order as layout
from seed_efficiency_tools import sha256


class FunctionOrderTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.binary, self.profile, self.bolt = [self.root / name for name in ("app", "seed.fdata", "bolt")]
        for path in (self.binary, self.profile, self.bolt):
            path.write_bytes(path.name.encode())
        self.bolt.chmod(0o755)
        self.build = {"format": "torrentng-build-manifest-v1", "kind": "torrentngd",
                      "source_digest": "source", "source_commit": "commit", "source_tree": "tree",
                      "dirty": False, "rustc": "compiler", "features": "default",
                      "cargo_lock_sha256": "lock", "profile_sha256": "pgo",
                      "binary": str(self.binary), "binary_sha256": sha256(self.binary)}
        self.training = {"format": "torrentng-bolt-experiment-v1",
                         "input_binary_sha256": sha256(self.binary),
                         "profile_sha256": sha256(self.profile), "pgo_profile_sha256": "pgo",
                         "tool_sha256": sha256(self.bolt), "training": [{"mib": 16, "peers": 1}]}
        self.build_path, self.training_path = self.root / "build.json", self.root / "training.json"
        self.order = self.root / "order.txt"
        self.sidecar = Path(str(self.order) + ".manifest.json")

    def generate(self):
        self.build_path.write_text(json.dumps(self.build))
        self.training_path.write_text(json.dumps(self.training))
        def planner(argv, **kwargs):
            output = next(arg.split("=", 1)[1] for arg in argv if arg.startswith("-generate-function-order="))
            Path(output).write_text("hot/1\nwarm\nhot/2\n")
        with patch.object(layout.subprocess, "run", side_effect=planner), \
                patch.object(layout, "command", side_effect=lambda *args:
                             "0000 t hot\n0010 T warm" if args[0] == "nm" else "BOLT version"):
            return layout.generate(self.binary, self.build_path, self.profile,
                                   self.training_path, self.order, self.bolt)

    def validate(self, build_manifest=None, symbols="0000 t hot\n0010 T warm", features="default", optimization_level="s"):
        with patch.object(layout, "validate_profile", return_value={
                "rustc": "compiler", "profile_sha256": "pgo"}), \
                patch.object(layout, "source_digest", return_value="source"), \
                patch.object(layout, "command", return_value=symbols):
            return layout.validate(self.order, self.root, self.root / "pgo.profdata", build_manifest, features, optimization_level)

    def test_feature_sets_are_canonical_and_remain_bound_to_the_build(self):
        self.build["features"] = "seed-mmap,default,full-compat,seed-mmap"
        data = self.generate()
        self.assertEqual(data["features"], "full-compat,seed-mmap")
        self.validate(self.build_path, features="seed-mmap,full-compat")
        with self.assertRaisesRegex(ValueError, "features"):
            self.validate(features="default")

    def test_optimization_level_binds_training_order_and_output_build(self):
        self.build["optimization_level"] = "3"
        with self.assertRaisesRegex(ValueError, "optimization_level"):
            self.generate()  # legacy training means s, never 3
        self.training["optimization_level"] = "3"
        result = self.generate()
        self.assertEqual(result["optimization_level"], "3")
        with self.assertRaisesRegex(ValueError, "optimization_level"):
            self.validate()
        self.validate(self.build_path, optimization_level="3")
        build = json.loads(self.build_path.read_text())
        del build["optimization_level"]
        self.build_path.write_text(json.dumps(build))
        with self.assertRaisesRegex(ValueError, "optimization_level"):
            self.validate(self.build_path, optimization_level="3")

    def test_legacy_order_is_only_size_optimized(self):
        result = self.generate()
        del result["optimization_level"]
        self.sidecar.write_text(json.dumps(result))
        self.validate()
        with self.assertRaisesRegex(ValueError, "optimization_level"):
            self.validate(optimization_level="3")

    def test_generation_preserves_rank_and_binds_training_inputs(self):
        data = self.generate()
        self.assertEqual(self.order.read_text(), "hot\nwarm\n")
        self.assertEqual(data["function_count"], 2)
        self.assertEqual(data["input_binary_sha256"], sha256(self.binary))
        self.assertEqual(data["bolt_profile_sha256"], sha256(self.profile))
        self.assertEqual(self.validate()["order_sha256"], sha256(self.order))

    def test_unknown_or_empty_symbol_order_is_rejected(self):
        for lines in (["missing/1"], [], [" "]):
            with self.subTest(lines=lines), self.assertRaises(ValueError):
                layout.normalize_order(lines, "0000 T known")

    def test_changed_training_artifacts_are_rejected(self):
        for key in ("input_binary_sha256", "profile_sha256", "pgo_profile_sha256", "tool_sha256"):
            with self.subTest(key=key):
                original = self.training[key]
                self.training[key] = "changed"
                with self.assertRaisesRegex(ValueError, key):
                    self.generate()
                self.training[key] = original
        self.assertFalse(self.order.exists())

    def test_dirty_or_changed_binary_cannot_generate_an_order(self):
        self.build["dirty"] = True
        with self.assertRaisesRegex(ValueError, "clean TorrentNG"):
            self.generate()
        self.build["dirty"] = False
        self.binary.write_bytes(b"replacement")
        with self.assertRaisesRegex(ValueError, "binary hash"):
            self.generate()

    def test_changed_source_compiler_profile_features_or_order_are_rejected(self):
        data = self.generate()
        for key in ("source_digest", "rustc", "pgo_profile_sha256", "features"):
            with self.subTest(key=key):
                self.sidecar.write_text(json.dumps({**data, key: "changed"}))
                with self.assertRaisesRegex(ValueError, key):
                    self.validate()
        self.sidecar.write_text(json.dumps(data))
        self.order.write_text("warm\nhot\n")
        with self.assertRaisesRegex(ValueError, "differs from its manifest"):
            self.validate()

    def test_output_manifest_records_only_a_matching_build(self):
        self.generate()
        self.validate(self.build_path)
        result = json.loads(self.build_path.read_text())
        self.assertEqual(result["function_order"]["order_sha256"], sha256(self.order))
        self.assertIn("--symbol-ordering-file=" + str(self.order), result["binary_link_args"][-1])
        self.build["profile_sha256"] = "different-profile"
        self.build_path.write_text(json.dumps(self.build))
        with self.assertRaisesRegex(ValueError, "output build differs"):
            self.validate(self.build_path)

    def test_existing_order_outputs_are_not_overwritten(self):
        self.generate()
        before = self.order.read_bytes()
        with self.assertRaisesRegex(ValueError, "new paths"):
            self.generate()
        self.assertEqual(self.order.read_bytes(), before)

    def test_output_must_still_contain_the_ordered_function_names(self):
        self.generate()
        with self.assertRaisesRegex(ValueError, "ordered symbol is absent"):
            self.validate(self.build_path, symbols="0000 T unrelated")
        self.assertNotIn("function_order", json.loads(self.build_path.read_text()))

    def test_training_keeps_build_outputs_out_of_daemon_profiles(self):
        source_profile = self.root / "pgo.profdata.gz"
        source_profile.write_bytes(gzip.compress(b"pgo-data"))
        unpacked = self.root / "pgo.profdata"
        unpacked.write_bytes(b"pgo-data")
        target, output = self.root / "target", self.root / "training"
        merge_tool = self.root / "merge"
        merge_tool.write_bytes(b"merge-tool")
        merge_tool.chmod(0o755)
        corpus = self.root / "corpus.json"
        workloads = [{"mib": 16, "mib_per_second": 4, "peers": 1, "window": window,
                      "pattern": "sequential"} for window in (8, 16)]
        corpus.write_text(json.dumps({"training": workloads}))
        calls = []
        def run(argv, **kwargs):
            argv = list(map(str, argv))
            calls.append(argv)
            if argv[0] == "cargo":
                self.assertEqual(kwargs["env"]["RUSTFLAGS"], "-Cprofile-use=" + str(
                    target / ("torrentng-seed-" + sha256(unpacked) + ".profdata")) + " -Copt-level=3")
                binary = target / "release/torrentngd"
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"built")
                # This must not enter the daemon instrumentation profile merge.
                (target / "build-script.fdata").write_text("unrelated")
            elif "-instrument" in argv:
                Path(argv[argv.index("-o") + 1]).write_bytes(b"instrumented")
            elif argv[0] == sys.executable:
                index = sum(call[0] == sys.executable for call in calls)
                (output / f"raw/profile.fdata.{index}").write_text("daemon")
            elif argv[0] == str(merge_tool):
                self.assertEqual(len(argv), 3)
                self.assertTrue(all(Path(path).parent == output / "raw" for path in argv[1:]))
                kwargs["stdout"].write("merged profile")
            else:
                raw = next(arg.split("=", 1)[1] for arg in argv if arg.startswith("-generate-function-order="))
                Path(raw).write_text("hot/1\n")
        def build_manifest(binary, repo, profile, features="default", optimization_level="s"):
            return {**self.build, "binary": str(binary), "binary_sha256": sha256(binary),
                    "features": features, "optimization_level": optimization_level,
                    "profile_sha256": sha256(profile)}
        with patch.dict(os.environ, {}, clear=True), \
                patch.object(layout, "validate_profile", return_value={"profile_sha256": sha256(unpacked)}), \
                patch.object(layout, "source_provenance", return_value={"dirty": False, "source_digest": "source"}), \
                patch.object(layout, "source_digest", return_value="source"), \
                patch.object(layout, "manifest", side_effect=build_manifest), \
                patch.object(layout.subprocess, "run", side_effect=run), \
                patch.object(layout, "command", side_effect=lambda *args:
                             "0000 t hot" if args[0] == "nm" else "BOLT version"):
            order = layout.train(source_profile, self.root, output, target, corpus,
                                 self.bolt, merge_tool, "0-1", optimization_level="3")
        self.assertEqual(order.read_text(), "hot\n")
        probes = [call for call in calls if call[0] == sys.executable]
        self.assertEqual(len(probes), len(workloads))
        self.assertTrue(all(call[-2:] == ["--client-cpus", "0-1"] for call in probes))
        sidecar = json.loads(Path(str(order) + ".manifest.json").read_text())
        self.assertEqual(sidecar["pgo_profile_sha256"], sha256(unpacked))
        self.assertEqual(sidecar["training_corpus"], workloads)
        self.assertEqual(sidecar["optimization_level"], "3")

    def test_explicit_opt_level_rejects_inherited_layout_overrides(self):
        for overrides in ({"CARGO_PROFILE_RELEASE_OPT_LEVEL": "3"}, {"RUSTFLAGS": "-Copt-level=3"}, {"CARGO_ENCODED_RUSTFLAGS": ""}):
            with self.subTest(overrides=overrides), patch.dict(os.environ, overrides, clear=True), patch.object(layout, "validate_profile", return_value={}), patch.object(layout, "source_provenance", return_value={"dirty": False}), patch.object(layout.subprocess, "run") as run:
                with self.assertRaisesRegex(ValueError, "overrides"):
                    layout.train(self.profile, self.root, self.root / "unused", None, self.root / "corpus", self.bolt, self.bolt, optimization_level="3")
                run.assert_not_called()

    def test_build_script_keeps_default_path_and_resolves_order_before_chdir(self):
        # Exercise the real shell control flow with compiler/tool stubs. Real
        # ELF/profile identity checks are covered above and by the recorded run.
        bindir = self.root / "bin"
        bindir.mkdir()
        stub = "#!" + sys.executable + "\n" + '''import json,os,sys
from pathlib import Path
name=Path(sys.argv[0]).name
with open(os.environ['CALL_LOG'],'a') as f: f.write(json.dumps({'name':name,'args':sys.argv[1:],'rustflags':os.environ.get('RUSTFLAGS')})+'\\n')
if name=='cargo':
 p=Path(sys.argv[sys.argv.index('--target-dir')+1])/'release/torrentngd'
 p.parent.mkdir(parents=True,exist_ok=True);p.write_bytes(b'compiled')
'''
        for name in ("cargo", "rustc", "python3"):
            path = bindir / name
            path.write_text(stub)
            path.chmod(0o755)
        (self.root / "source repo").mkdir()
        (self.root / "orders").mkdir()
        (self.root / "orders/seed order.txt").write_text("hot\n")
        (self.root / "orders/seed order.txt.manifest.json").write_text("{}")
        profile = self.root / "pgo.profdata"
        profile.write_bytes(b"profile")
        log = self.root / "calls.jsonl"
        environment = {k: v for k, v in os.environ.items()
                       if not k.startswith("CARGO_PROFILE_") and k not in {
                           "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER",
                           "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTFLAGS"}}
        environment.update(PATH=str(bindir) + os.pathsep + os.environ["PATH"], CALL_LOG=str(log))
        script = Path(__file__).resolve().parent / "build_native_pgo_release.sh"
        for optional in ([], ["orders/seed order.txt"], ["", "seed-mmap"],
                         ["orders/seed order.txt", "seed-mmap"], ["", "seed-mmap", "3"],
                         ["orders/seed order.txt", "seed-mmap", "3"]):
            log.write_text("")
            subprocess.run(["bash", str(script), "pgo.profdata", "target", "source repo", *optional],
                           cwd=self.root, env=environment, check=True, capture_output=True, text=True)
            calls = [json.loads(line) for line in log.read_text().splitlines()]
            cargo = next(call["args"] for call in calls if call["name"] == "cargo")
            has_order = bool(optional and optional[0])
            self.assertEqual(cargo[1], "rustc" if has_order else "build")
            if len(optional) >= 2:
                self.assertEqual(cargo[cargo.index("--features") + 1], "seed-mmap")
                for call in calls:
                    if call["name"] == "python3":
                        args = call["args"]
                        self.assertEqual(args[args.index("--features") + 1], "seed-mmap")
            else:
                self.assertNotIn("--features", cargo)
            level = optional[2] if len(optional) == 3 else "s"
            flags = next(call["rustflags"] for call in calls if call["name"] == "cargo")
            self.assertEqual("-Copt-level=3" in flags, level == "3")
            for call in calls:
                if call["name"] == "python3":
                    self.assertEqual(call["args"][call["args"].index("--opt-level") + 1], level)
            if has_order:
                absolute = str(self.root / "orders/seed order.txt")
                staged = str(self.root / "target" / ("torrentng-seed-order-" +
                             sha256(self.root / "orders/seed order.txt") + ".txt"))
                self.assertIn("-Clink-arg=-Wl,--symbol-ordering-file=" + staged, cargo)
                validations = [call["args"] for call in calls if call["name"] == "python3"
                               and "native_seed_function_order.py" in call["args"][0]]
                self.assertEqual(len(validations), 2)
                self.assertEqual([args[args.index("--order") + 1] for args in validations],
                                 [absolute, staged])
        old_flags = next(call["rustflags"] for call in calls if call["name"] == "cargo")
        old_order = next(arg for arg in cargo if "--symbol-ordering-file=" in arg)
        profile.write_bytes(b"retrained profile at the same input path")
        (self.root / "orders/seed order.txt").write_text("warm\nhot\n")
        log.write_text("")
        subprocess.run(["bash", str(script), "pgo.profdata", "target", "source repo",
                        "orders/seed order.txt", "seed-mmap", "3"],
                       cwd=self.root, env=environment, check=True, capture_output=True, text=True)
        calls = [json.loads(line) for line in log.read_text().splitlines()]
        compiled = next(call for call in calls if call["name"] == "cargo")
        self.assertNotEqual(old_flags, compiled["rustflags"])
        self.assertNotIn(old_order, compiled["args"])
        self.assertIn(sha256(profile), compiled["rustflags"])
        log.write_text("")
        result = subprocess.run(["bash", str(script), "pgo.profdata", "target", "source repo"],
                                cwd=self.root, env={**environment, "CARGO_ENCODED_RUSTFLAGS": ""},
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 2)
        self.assertIn("unset RUSTFLAGS and CARGO_ENCODED_RUSTFLAGS", result.stderr)
        self.assertEqual(log.read_text(), "")
        for extra, override, message in [
            (["", "default", "2"], {}, "OPT_LEVEL must be s or 3"),
            (["", "default", "3"], {"CARGO_PROFILE_RELEASE_OPT_LEVEL": "3"}, "unset CARGO_PROFILE"),
            (["", "default", "3"], {"RUSTFLAGS": "-Copt-level=3"}, "unset RUSTFLAGS"),
        ]:
            result = subprocess.run(["bash", str(script), "pgo.profdata", "target", "source repo", *extra], cwd=self.root, env={**environment, **override}, capture_output=True, text=True)
            self.assertEqual(result.returncode, 2)
            self.assertIn(message, result.stderr)
            self.assertEqual(log.read_text(), "")


if __name__ == "__main__":
    unittest.main()
