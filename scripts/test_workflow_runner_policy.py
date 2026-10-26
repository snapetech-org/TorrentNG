import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_DIR = ROOT / ".github" / "workflows"
PUBLIC_FORK_WORKFLOW = "fork-safe-validation.yml"
PROTECTED_TRANSFER_WORKFLOW = "transfer-rebuild.yml"


class WorkflowRunnerPolicyTests(unittest.TestCase):
    def test_release_build_starts_only_from_version_tag_pushes(self) -> None:
        source = (WORKFLOW_DIR / "release.yml").read_text(encoding="utf-8")
        self.assertRegex(source, r"(?m)^  push:$")
        self.assertIn('      - "v*"', source)
        self.assertNotRegex(source, r"(?m)^  workflow_dispatch:")
        self.assertNotIn("inputs.tag", source)

    def test_codeql_installs_node_before_javascript_typescript_analysis(self) -> None:
        source = (WORKFLOW_DIR / "codeql.yml").read_text(encoding="utf-8")
        setup_match = re.search(
            r"(?ms)^      - name: Set up Node\.js for CodeQL\n"
            r"(?P<step>.*?)(?=^      - name: |\Z)",
            source,
        )
        self.assertIsNotNone(setup_match)
        setup_step = setup_match.group("step")
        self.assertRegex(
            setup_step,
            r"(?m)^        if: matrix\.language == 'javascript-typescript'$",
        )
        self.assertRegex(
            setup_step,
            r"(?m)^        uses: actions/setup-node@[a-f0-9]{40} # v7$",
        )
        self.assertIn('node-version: "22.23.4"', setup_step)
        self.assertIn("package-manager-cache: false", setup_step)
        self.assertLess(setup_match.start(), source.index("- name: Initialize CodeQL"))

    def test_main_ci_routes_release_jobs_only_to_private_jit(self) -> None:
        source = (WORKFLOW_DIR / "ci.yml").read_text(encoding="utf-8")
        self.assertIn("# ng-runner-policy: private-jit-only-v1", source)
        self.assertRegex(
            source,
            r"(?ms)^  pull_request_target:\n"
            r"    branches:\n      - main\n"
            r"    types:\n      - opened\n      - synchronize\n"
            r"      - reopened\n      - ready_for_review\n",
        )
        dispatcher = re.search(
            r"(?ms)^  dispatch-owner-pr:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
            source,
        )
        self.assertIsNotNone(dispatcher)
        self.assertIn("github.event.pull_request.user.login == 'snapetech'", dispatcher.group("body"))
        self.assertIn("github.event.pull_request.head.repo.id == github.repository_id", dispatcher.group("body"))
        self.assertIn("contents: none", dispatcher.group("body"))
        self.assertNotIn("actions/checkout", dispatcher.group("body"))
        self.assertNotIn("secrets.", dispatcher.group("body"))
        jobs = [
            "native-quality",
            "backend-fault-matrix",
            "backend-api-load",
            "msrv-check",
            "msrv-check-sidecar",
            "fuzz-smoke",
            "sidecar",
            "desktop",
            "backup-restore",
            "security-audit",
        ]
        for name in jobs:
            match = re.search(
                rf"(?ms)^  {re.escape(name)}:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
                source,
            )
            self.assertIsNotNone(match, name)
            runner = re.search(r"(?m)^    runs-on: (?P<runner>[^\n]+)$", match.group("body"))
            self.assertIsNotNone(runner, name)
            expression = runner.group("runner")
            self.assertIn('"group":"ng-shared-main-jit"', expression, name)
            self.assertIn('"ng-shared-main-torrentng"', expression, name)
            self.assertIn('"ng-main-run-{0}"', expression, name)
            self.assertIn("github.run_id", expression, name)
            self.assertNotIn("ubuntu-latest", expression, name)
            self.assertNotIn("NG_HOSTED_FIRST", expression, name)
            self.assertNotIn("NG_FORCE_JIT", expression, name)
            self.assertNotIn("||", expression, name)

    def test_webui_uses_private_jit_and_installs_the_locked_playwright_browser(self) -> None:
        source = (WORKFLOW_DIR / "ci.yml").read_text(encoding="utf-8")
        match = re.search(
            r"(?ms)^  webui:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
            source,
        )
        self.assertIsNotNone(match)
        body = match.group("body")
        runner = re.search(r"(?m)^    runs-on: (?P<runner>[^\n]+)$", body)
        self.assertIsNotNone(runner)
        self.assertIn('"group":"ng-shared-main-jit"', runner.group("runner"))
        self.assertIn('"gha-{0}-webui"', runner.group("runner"))
        self.assertNotIn("ubuntu-latest", runner.group("runner"))
        self.assertNotIn("container:", body)
        self.assertIn("npx playwright install --with-deps chromium", body)
        self.assertIn("inputs.candidate_sha == ''", body)

    def test_private_candidate_artifact_dispatch_is_exact_and_does_not_publish(self) -> None:
        source = (WORKFLOW_DIR / "ci.yml").read_text(encoding="utf-8")
        self.assertRegex(
            source,
            r"(?m)^      candidate_sha:\n        description: Exact main SHA for a private Linux x64 build artifact \(no release\)\n        required: false\n        type: string$",
        )

        match = re.search(
            r"(?ms)^  candidate-artifact:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
            source,
        )
        self.assertIsNotNone(match)
        body = match.group("body")
        self.assertIn("github.event_name == 'workflow_dispatch'", body)
        self.assertIn("github.actor == 'snapetech'", body)
        self.assertIn("github.ref == 'refs/heads/main'", body)
        self.assertIn("inputs.candidate_sha != ''", body)
        self.assertIn("inputs.candidate_sha == github.sha", body)
        self.assertIn("inputs.pr_number == ''", body)
        self.assertIn('"group":"ng-shared-main-jit"', body)
        self.assertIn('"ng-shared-main-torrentng"', body)
        self.assertIn('"gha-{0}-candidate-artifact"', body)
        self.assertNotIn("ubuntu-latest", body)
        self.assertIn('[[ "$candidate_sha" == "${GITHUB_SHA,,}" ]]', body)
        self.assertIn("cargo build --release --locked -p torrentngd", body)
        self.assertIn("scripts/backend_burndown_native_release_smoke.sh", body)
        self.assertIn("torrentngd-linux-x64.tar.gz", body)
        self.assertIn('cd "$BUILD_BUNDLE"', body)
        self.assertIn(
            "sha256sum torrentngd torrentngd-linux-x64.tar.gz > SHA256SUMS",
            body,
        )
        self.assertIn("actions/upload-artifact@", body)
        self.assertIn("retention-days: 30", body)
        self.assertNotIn("gh release", body.lower())
        self.assertNotIn("git push", body.lower())

        for name in [
            "native-quality",
            "backend-fault-matrix",
            "backend-api-load",
            "msrv-check",
            "msrv-check-sidecar",
            "fuzz-smoke",
            "sidecar",
            "desktop",
            "webui",
            "backup-restore",
            "security-audit",
        ]:
            job = re.search(
                rf"(?ms)^  {re.escape(name)}:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
                source,
            )
            self.assertIsNotNone(job, name)
            job_body = job.group("body")
            self.assertIn("inputs.candidate_sha == ''", job_body, name)
            condition = re.search(
                r"(?ms)^    if: >-\n(?P<condition>.*?)(?=^    (?:name|needs|timeout-minutes|runs-on):)",
                job_body,
            )
            self.assertIsNotNone(condition, name)
            if name != "webui":
                self.assertRegex(
                    condition.group("condition"),
                    r"inputs\.candidate_sha == ''\s*&&\s*\(\s*\(\(",
                )
                self.assertTrue(condition.group("condition").rstrip().endswith(")"), name)

    def test_other_workflows_use_declared_runner_routes(self) -> None:
        for workflow in WORKFLOW_DIR.glob("*.y*"):
            source = workflow.read_text(encoding="utf-8")
            if workflow.name == PROTECTED_TRANSFER_WORKFLOW:
                self.assertIn('"group":"ng-shared-main-jit"', source)
                self.assertIn('"gha-{0}-transfer-rebuild-correctness"', source)
                self.assertNotIn("ubuntu-latest", source)
                continue
            if workflow.name == "ci.yml":
                continue
            if workflow.name == PUBLIC_FORK_WORKFLOW:
                self.assertIn('"group":"public-fork-jit"', source)
                self.assertNotIn("ubuntu-latest", source)
                self.assertNotIn("ng-shared-main-jit", source)
                self.assertIn("contents: read", source)
                self.assertNotIn("secrets:", source)
                continue
            if workflow.name == "windows-smoke.yml":
                self.assertIn("packer-windows-jit-v1", source)
                self.assertIn('"group":"packer-windows"', source)
                self.assertIn('"on-demand"', source)
                self.assertIn('"TorrentNG"', source)
                self.assertIn("gha-{0}-smoke", source)
                self.assertNotIn("runs-on: windows-latest", source)
                self.assertRegex(source, r"scripts[\\/]+enable-msvc-environment\.ps1")
                continue
            if workflow.name in {
                "codeql.yml",
                "release-notes.yml",
                "semgrep-community.yml",
                "trivy-filesystem.yml",
                "release.yml",
                "release-aur.yml",
                "release-copr.yml",
                "release-ppa.yml",
                "backend-evidence.yml",
                "package-smoke-disabled.yml",
            }:
                expressions = re.findall(r"(?m)^\s*runs-on: (?P<runner>[^\n]+)$", source)
                routed = [expression for expression in expressions if "ng-shared-main-jit" in expression]
                self.assertTrue(routed, workflow.name)
                for expression in routed:
                    self.assertIn('"group":"ng-shared-main-jit"', expression)
                    self.assertIn('"ng-shared-main-torrentng"', expression)
                    self.assertIn('"ng-main-run-{0}"', expression)
                    if workflow.name == "release.yml":
                        self.assertNotIn("workflow_dispatch", expression)
                        self.assertNotIn("ubuntu-latest", expression)
                        if "matrix.os == 'windows'" in expression:
                            self.assertIn('"group":"packer-windows"', expression)
                            self.assertIn('"on-demand"', expression)
                            self.assertIn('"TorrentNG"', expression)
                            self.assertIn("strategy.job-index", expression)
                            self.assertRegex(
                                expression,
                                r"gha-\{0\}-\{1\}-(?:native-binaries|desktop-bundles)",
                            )
                            self.assertIn("|| matrix.os == 'macos-14'", expression)
                        elif "matrix.os == 'macos-14'" in expression:
                            self.assertIn('"group":"private-apple-builds"', expression)
                            self.assertIn('"on-demand"', expression)
                            self.assertIn('"TorrentNG"', expression)
                            self.assertIn("strategy.job-index", expression)
                            self.assertRegex(
                                expression,
                                r"gha-\{0\}-\{1\}-(?:native-binaries|desktop-bundles)",
                            )
                            self.assertNotIn("matrix.os }}", expression)
                        else:
                            self.assertNotIn("||", expression)
                            self.assertIn("fromJSON(format(", expression)
                    else:
                        self.assertNotIn("ubuntu-latest", expression)
                        self.assertNotIn("||", expression)
                if workflow.name in {"release-aur.yml", "release-copr.yml", "release-ppa.yml"}:
                    self.assertIn("github.ref == 'refs/heads/main'", source)
                if workflow.name in {"codeql.yml", "semgrep-community.yml"}:
                    self.assertIn("schedule:", source)
                if workflow.name == "semgrep-community.yml":
                    self.assertIn("docker info", source)
                    self.assertIn("docker run", source)
                continue
            self.assertNotRegex(source, r"(?m)^\s*(?:runs-on|labels):.*self-hosted")
            self.assertNotIn("TRUSTED_MAIN_RUNNERS_ENABLED", source)

    def test_transfer_hardware_runner_is_explicitly_opt_in_and_main_only(self) -> None:
        source = (WORKFLOW_DIR / PROTECTED_TRANSFER_WORKFLOW).read_text(encoding="utf-8")
        dedicated_hardware_lines = [
            line.strip()
            for line in source.splitlines()
            if re.match(r"\s*(?:runs-on|labels):.*self-hosted", line)
            and "torrentng-transfer-perf" in line
        ]
        self.assertEqual(
            dedicated_hardware_lines,
            ["runs-on: [self-hosted, linux, x64, torrentng-transfer-perf]"],
        )
        self.assertRegex(
            source,
            r"(?m)^\s*if: github\.ref == 'refs/heads/main' && "
            r"inputs\.run_dedicated_hardware == true\s*$",
        )
        self.assertRegex(
            source,
            r"(?m)^\s*run_dedicated_hardware:\n"
            r"\s+description:.*\n\s+required: true\n"
            r"\s+default: false\n\s+type: boolean\s*$",
        )
        self.assertRegex(
            source,
            r"(?m)^\s*environment: transfer-rebuild-hardware\s*$",
        )
        self.assertIn("needs: private-jit-correctness", source)
        self.assertIn("persist-credentials: false", source)
        self.assertNotIn("TRUSTED_MAIN_RUNNERS_ENABLED", source)

    def test_public_fork_validation_uses_the_isolated_no_secret_jit_pool(self) -> None:
        source = (WORKFLOW_DIR / PUBLIC_FORK_WORKFLOW).read_text(encoding="utf-8")
        runs_on = next(
            line.strip()
            for line in source.splitlines()
            if line.strip().startswith("runs-on:")
        )
        self.assertIn('"group":"public-fork-jit"', runs_on)
        self.assertIn("self-hosted", runs_on)
        self.assertNotIn("ubuntu-latest", source)
        self.assertNotIn("ng-shared-main-jit", source)
        self.assertIn(
            "github.event.pull_request.base.repo.full_name == 'snapetech-org/TorrentNG'",
            source,
        )
        self.assertIn("contents: read", source)
        self.assertNotIn("secrets:", source)

        caller = (WORKFLOW_DIR / "public-fork-pr.yml").read_text(encoding="utf-8")
        self.assertRegex(caller, r"(?m)^  workflow_dispatch:\s*\{\}\s*$")
        self.assertNotRegex(caller, r"(?m)^  pull_request(?:_target)?:")
        self.assertIn("uses: ./.github/workflows/fork-safe-validation.yml", caller)
        self.assertNotIn("@7cb3c216a2473fd6db8ec07f70be209ac4fefe46", caller)
        self.assertIn("github.event.pull_request.head.repo.fork == true", caller)
        self.assertIn("github.event.pull_request.base.ref == 'main'", caller)
        self.assertNotIn("pull_request_target", source + caller)

    def test_windows_smoke_uses_the_private_packer_runner(self) -> None:
        source = (WORKFLOW_DIR / "windows-smoke.yml").read_text(encoding="utf-8")
        self.assertIn("packer-windows-jit-v1", source)
        self.assertIn('"group":"packer-windows"', source)
        self.assertIn("gha-{0}-smoke", source)
        self.assertNotIn("runs-on: windows-latest", source)


if __name__ == "__main__":
    unittest.main()
