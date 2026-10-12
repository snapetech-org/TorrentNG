from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from scripts import validate_development as engine
from scripts.validation_engine.trusted_sandbox import require_trusted_local_test_sandbox


def make_repository() -> Path:
    root = Path(tempfile.mkdtemp(prefix="torrentng-validation-engine-test-"))
    files = {
        "README.md": "fixture\n",
        "scripts/test_fixture.py": "import unittest\n",
        "scripts/validation_selftest.sh": "#!/usr/bin/env bash\n",
        "crates/demo/src/lib.rs": "#[cfg(test)]\nmod tests {}\n",
        "crates/demo/tests/contract.rs": "#[test]\nfn works() {}\n",
        "webui/src/demo.test.tsx": "describe('demo', () => {});\n",
        "scripts/validate_development.py": "# engine fixture\n",
        "scripts/test_validation_engine.py": "# engine test fixture\n",
    }
    for relative, content in files.items():
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")
    commands = (
        ["git", "init", "--quiet"],
        ["git", "config", "user.email", "validation@example.invalid"],
        ["git", "config", "user.name", "TorrentNG validation test"],
        ["git", "add", "--all"],
        ["git", "commit", "--quiet", "-m", "validation fixture"],
    )
    for command in commands:
        subprocess.run(command, cwd=root, check=True, capture_output=True)
    return root


def fixture_tasks() -> tuple[dict, ...]:
    return ({
        "id": "fixture-test",
        "name": "Slow fixture task",
        "kind": "test",
        "command": [sys.executable, "-c", "print('Ran 1 test in 0.01s')"],
        "case_parser": "python-unittest",
        "minimum_cases": 1,
        "lanes": ["python-tests"],
        "timeout": 10,
    },)


class ValidationEngineTests(unittest.TestCase):
    def setUp(self) -> None:
        self.root = make_repository()
        self.addCleanup(shutil.rmtree, self.root, ignore_errors=True)

    def test_local_test_execution_requires_matching_outer_sandbox_marker(self) -> None:
        marker = self.root / "sandbox-marker"
        token = "a" * 64
        with self.assertRaisesRegex(RuntimeError, "trusted bubblewrap launcher"):
            require_trusted_local_test_sandbox({}, marker)
        marker.write_text("b" * 64, encoding="utf-8")
        with self.assertRaisesRegex(RuntimeError, "trusted bubblewrap launcher"):
            require_trusted_local_test_sandbox({"VALIDATION_OUTER_SANDBOX_TOKEN": token}, marker)
        marker.write_text(token, encoding="utf-8")
        require_trusted_local_test_sandbox({"VALIDATION_OUTER_SANDBOX_TOKEN": token}, marker)
        require_trusted_local_test_sandbox({"GITHUB_ACTIONS": "true"}, marker)

    def test_trusted_launcher_host_resource_canary(self) -> None:
        node = shutil.which("node")
        if node is None:
            self.skipTest("Node.js is unavailable for the trusted-launcher canary")
        canary = engine.ROOT / "scripts/validation_engine/trusted-test-launcher.contract.mjs"
        result = subprocess.run(
            [node, "--test", str(canary)],
            cwd=engine.ROOT,
            capture_output=True,
            check=False,
            text=True,
            timeout=30,
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_child_environment_uses_allowlist_and_run_owned_paths(self) -> None:
        run_root = self.root / "evidence"
        run_root.mkdir(mode=0o700)
        environment = engine._safe_environment(
            {
                "id": "fixture",
                "kind": "test",
                "env": {"FIXTURE_SETTING": "expected"},
            },
            run_root,
            {
                "PATH": "/toolchain/bin",
                "RUSTUP_HOME": "/toolchain/rustup",
                "HOME": "/operator/home",
                "TMPDIR": "/operator/tmp",
                "CARGO_HOME": "/operator/cargo",
                "PYTHONPATH": "/operator/python",
                "NODE_OPTIONS": "--require=/operator/injected.cjs",
                "GITHUB_TOKEN": "synthetic-token",
                "HTTPS_PROXY": "http://proxy.invalid",
                "RANDOM_PRIVATE_VALUE": "synthetic-private-value",
                "XDG_CONFIG_HOME": "/operator/config",
            },
        )
        environment_root = run_root / "fixture-environment"
        self.assertEqual(environment["PATH"], "/toolchain/bin")
        self.assertEqual(environment["RUSTUP_HOME"], "/toolchain/rustup")
        self.assertEqual(environment["HOME"], str(environment_root / "home"))
        self.assertEqual(environment["USERPROFILE"], environment["HOME"])
        self.assertEqual(environment["TMPDIR"], str(environment_root / "tmp"))
        self.assertEqual(environment["XDG_CONFIG_HOME"], str(environment_root / "xdg-config"))
        self.assertEqual(environment["CARGO_HOME"], str(run_root / "cargo-home"))
        self.assertEqual(environment["PIP_CACHE_DIR"], str(environment_root / "pip-cache"))
        self.assertEqual(environment["PYTHONNOUSERSITE"], "1")
        self.assertEqual(environment["NODE_ENV"], "test")
        self.assertEqual(environment["CI"], "true")
        self.assertEqual(environment["FIXTURE_SETTING"], "expected")
        for key in ("PYTHONPATH", "NODE_OPTIONS", "GITHUB_TOKEN", "HTTPS_PROXY", "RANDOM_PRIVATE_VALUE"):
            self.assertNotIn(key, environment)
        with self.assertRaisesRegex(engine.ValidationError, "unapproved inherited environment variable"):
            engine._safe_environment(
                {"id": "fixture", "kind": "test", "pass_env": ["GITHUB_TOKEN"]},
                run_root,
                {"GITHUB_TOKEN": "synthetic-token"},
            )
        with self.assertRaisesRegex(engine.ValidationError, "safe lowercase identifier"):
            engine._safe_environment({"id": "../outside", "kind": "test"}, run_root, {"PATH": "/toolchain/bin"})

        fallback_environment = engine._safe_environment(
            {"id": "fallback", "kind": "check"},
            run_root,
            {"PATH": "/toolchain/bin", "HOME": "/operator/home"},
        )
        self.assertEqual(fallback_environment["RUSTUP_HOME"], "/operator/home/.rustup")

    def test_plan_rejects_unapproved_inherited_environment_names(self) -> None:
        tasks = tuple(dict(task) for task in engine.TASKS)
        tasks[0]["pass_env"] = ["GITHUB_TOKEN"]
        with self.assertRaisesRegex(engine.ValidationError, "unapproved inherited environment variable"):
            engine.create_plan(self.root, tasks=tasks, tests_only=True)

    def test_inventory_captures_native_test_sources_by_lane(self) -> None:
        inventory = engine.discover_inventory(self.root)
        self.assertEqual([entry["file"] for entry in inventory["python-tests"]], ["scripts/test_fixture.py"])
        self.assertEqual(
            [entry["file"] for entry in inventory["rust-workspace-tests"]],
            ["crates/demo/src/lib.rs", "crates/demo/tests/contract.rs"],
        )
        self.assertEqual([entry["file"] for entry in inventory["webui-tests"]], ["webui/src/demo.test.tsx"])
        self.assertEqual(
            [entry["file"] for entry in inventory["certification-selftests"]],
            ["scripts/validation_selftest.sh"],
        )
        self.assertEqual(len(inventory["validation-engine"]), 2)

    def test_plan_is_source_bound_and_selects_registered_tests(self) -> None:
        tasks = fixture_tasks()
        plan = engine.create_plan(self.root, tasks, tests_only=True)
        self.assertEqual([task["id"] for task in plan["tasks"] if task["selected"]], ["fixture-test"])
        self.assertEqual(plan["inventory_counts"]["python-tests"], 1)
        self.assertEqual(len(plan["candidate"]["source_sha256"]), 64)
        changed = dict(plan)
        changed["tasks"] = [dict(task, name="tampered") for task in plan["tasks"]]
        with self.assertRaisesRegex(engine.ValidationError, "plan digest"):
            engine.execute_plan(self.root, changed, tasks)

    def test_clean_source_gate_selftest_is_opt_in_for_dirty_local_worktrees(self) -> None:
        opted_in = {
            "id": "clean-source-selftest",
            "name": "Clean-source self-test",
            "kind": "test",
            "command": [sys.executable, "-c", "pass"],
            "lanes": ["certification-selftests"],
            "default_selected": False,
            "exclusion_reason": "requires a clean checkout",
        }
        tasks = (*fixture_tasks(), opted_in)
        local_plan = engine.create_plan(self.root, tasks, tests_only=True)
        self.assertEqual([task["id"] for task in local_plan["tasks"] if task["selected"]], ["fixture-test"])
        self.assertEqual(local_plan["tasks"][1]["exclusion_reason"], "requires a clean checkout")
        ci_plan = engine.create_plan(self.root, tasks, task_id="clean-source-selftest")
        self.assertEqual([task["id"] for task in ci_plan["tasks"] if task["selected"]], ["clean-source-selftest"])

    def test_progress_summary_counts_blocked_and_active_tasks(self) -> None:
        tasks = [
            {"id": "passed", "name": "Passed"},
            {"id": "blocked", "name": "Blocked"},
            {"id": "active", "name": "Slow test"},
        ]
        receipts = {
            "passed": {"status": "passed"},
            "blocked": {"status": "blocked"},
        }
        self.assertEqual(
            engine.format_progress("sample", tasks, receipts, tasks[2], 65),
            "[validation] sample: 1/3 selected tasks complete (1 passed, 0 failed, 1 blocked, 1 pending); running Slow test (1m05s)",
        )
        self.assertEqual(
            engine.format_elapsed(65),
            "1m05s",
        )

    def test_native_summary_parsers_count_rust_python_and_vitest_cases(self) -> None:
        self.assertEqual(
            engine._count_cases(
                "cargo-test",
                b"test result: ok. 12 passed; 0 failed; 1 ignored;\ntest result: ok. 5 passed; 0 failed;",
            ),
            17,
        )
        self.assertEqual(
            engine._count_cases("python-unittest", b"Ran 1 test in 0.01s\nRan 54 tests in 1.2s\n"),
            54,
        )
        self.assertEqual(engine._count_cases("vitest", b"Tests 36 passed (36)"), 36)

    def test_executor_emits_live_start_heartbeat_and_completion(self) -> None:
        tasks = ({
            **fixture_tasks()[0],
            "command": [
                sys.executable,
                "-c",
                "import time; print('Ran 1 test in 0.01s', flush=True); time.sleep(1.2)",
            ],
        },)
        plan = engine.create_plan(self.root, tasks, tests_only=True)
        evidence = Path(tempfile.mkdtemp(prefix="torrentng-validation-evidence-test-"))
        self.addCleanup(shutil.rmtree, evidence, ignore_errors=True)
        messages: list[str] = []
        result = engine.execute_plan(
            self.root,
            plan,
            tasks,
            progress=messages.append,
            heartbeat_seconds=0.05,
            run_root=evidence,
        )
        self.assertEqual(result["status"], "passed")
        self.assertEqual(result["totals"]["active_cases"], 1)
        self.assertTrue(any("running Slow fixture task (0m00s)" in message for message in messages))
        self.assertTrue(any("running Slow fixture task (0m01s)" in message for message in messages))
        self.assertIn("1/1 selected tasks complete (1 passed", messages[-1])
        self.assertEqual((evidence / "fixture-test.log").stat().st_mode & 0o777, 0o600)
        self.assertTrue((evidence / "receipt.json").is_file())

    def test_executor_blocks_later_tasks_after_candidate_changes(self) -> None:
        first = {
            **fixture_tasks()[0],
            "id": "mutating-test",
            "name": "Mutating fixture task",
            "command": [
                sys.executable,
                "-c",
                "from pathlib import Path; Path('README.md').write_text('changed\\n'); print('Ran 1 test in 0.01s')",
            ],
        }
        second = {
            **fixture_tasks()[0],
            "id": "later-test",
            "name": "Must stay blocked",
        }
        tasks = (first, second)
        plan = engine.create_plan(self.root, tasks)
        result = engine.execute_plan(self.root, plan, tasks, progress=lambda _message: None)
        self.assertEqual(result["status"], "failed")
        self.assertEqual(result["tasks"][0]["status"], "passed")
        self.assertEqual(result["tasks"][1]["status"], "blocked")


if __name__ == "__main__":
    unittest.main()
