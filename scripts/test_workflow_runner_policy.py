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

    def test_main_ci_routes_only_trusted_main_jobs_to_the_isolated_pool(self) -> None:
        source = (WORKFLOW_DIR / "ci.yml").read_text(encoding="utf-8")
        jobs = [
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
            self.assertIn(
                "contains(fromJSON('[\"push\",\"workflow_dispatch\",\"schedule\",\"release\",\"repository_dispatch\"]'), github.event_name)",
                expression,
                name,
            )
            self.assertIn(
                "github.ref == 'refs/heads/main' || (github.ref_type == 'tag' && startsWith(github.ref_name, 'v'))",
                expression,
                name,
            )
            self.assertIn("vars.NG_HOSTED_FIRST", expression, name)
            self.assertIn("vars.NG_FORCE_JIT", expression, name)
            self.assertIn("github.run_attempt > 1", expression, name)
            self.assertIn("|| 'ubuntu-latest'", expression, name)

    def test_other_workflows_use_github_hosted_runners(self) -> None:
        for workflow in WORKFLOW_DIR.glob("*.y*"):
            source = workflow.read_text(encoding="utf-8")
            if workflow.name in {PUBLIC_FORK_WORKFLOW, PROTECTED_TRANSFER_WORKFLOW, "ci.yml"}:
                continue
            if workflow.name in {
                "codeql.yml",
                "release-notes.yml",
                "semgrep-community.yml",
                "trivy-filesystem.yml",
                "release.yml",
            }:
                expressions = re.findall(r"(?m)^\s*runs-on: (?P<runner>[^\n]+)$", source)
                routed = [expression for expression in expressions if "ng-shared-main-jit" in expression]
                self.assertTrue(routed, workflow.name)
                for expression in routed:
                    self.assertIn('"group":"ng-shared-main-jit"', expression)
                    self.assertIn('"ng-shared-main-torrentng"', expression)
                    self.assertIn('"ng-main-run-{0}"', expression)
                    if workflow.name == "release.yml":
                        self.assertIn(
                            "contains(fromJSON('[\"push\",\"workflow_dispatch\",\"schedule\",\"release\",\"repository_dispatch\"]'), github.event_name)",
                            expression,
                        )
                        self.assertIn(
                            "github.ref == 'refs/heads/main' || (github.ref_type == 'tag' && startsWith(github.ref_name, 'v'))",
                            expression,
                        )
                    else:
                        self.assertIn("github.event_name == 'push'", expression)
                        self.assertIn("github.event_name == 'workflow_dispatch'", expression)
                        self.assertIn("github.ref == 'refs/heads/main'", expression)
                    self.assertRegex(
                        expression,
                        r"\|\| (?:'ubuntu-latest'|'ubuntu-24\.04'|matrix\.os)",
                    )
                if workflow.name in {"codeql.yml", "semgrep-community.yml"}:
                    self.assertIn("github.event_name == 'schedule'", routed[0])
                if workflow.name == "semgrep-community.yml":
                    self.assertIn("docker info", source)
                    self.assertIn("docker run", source)
                continue
            self.assertNotRegex(source, r"(?m)^\s*(?:runs-on|labels):.*self-hosted")
            self.assertNotIn("TRUSTED_MAIN_RUNNERS_ENABLED", source)

    def test_transfer_hardware_runner_is_explicitly_opt_in_and_main_only(self) -> None:
        source = (WORKFLOW_DIR / PROTECTED_TRANSFER_WORKFLOW).read_text(encoding="utf-8")
        self_hosted_lines = [
            line.strip()
            for line in source.splitlines()
            if re.match(r"\s*(?:runs-on|labels):.*self-hosted", line)
        ]
        self.assertEqual(
            self_hosted_lines,
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
        self.assertIn("needs: hosted-correctness", source)
        self.assertIn("persist-credentials: false", source)
        self.assertNotIn("TRUSTED_MAIN_RUNNERS_ENABLED", source)

    def test_public_fork_validation_stays_on_github_hosted_runners(self) -> None:
        source = (WORKFLOW_DIR / PUBLIC_FORK_WORKFLOW).read_text(encoding="utf-8")
        runs_on = next(
            line.strip()
            for line in source.splitlines()
            if line.strip().startswith("runs-on:")
        )
        self.assertIn("runs-on: ubuntu-latest", runs_on)
        self.assertNotIn("self-hosted", source)
        self.assertNotIn("public-fork-jit", source)
        self.assertIn(
            "github.event.pull_request.base.repo.full_name == 'snapetech-org/TorrentNG'",
            source,
        )
        self.assertIn("contents: read", source)
        self.assertNotIn("secrets:", source)

        caller = (WORKFLOW_DIR / "public-fork-pr.yml").read_text(encoding="utf-8")
        self.assertIn("branches: [main]", caller)
        self.assertIn("github.event.pull_request.head.repo.fork == true", caller)
        self.assertIn("github.event.pull_request.base.ref == 'main'", caller)
        self.assertRegex(caller, r"(?m)^  pull_request:$")
        self.assertNotIn("pull_request_target", source + caller)

    def test_windows_smoke_uses_the_hosted_windows_runner(self) -> None:
        source = (WORKFLOW_DIR / "windows-smoke.yml").read_text(encoding="utf-8")
        self.assertRegex(source, r"(?m)^\s*runs-on:\s*windows-latest\s*$")


if __name__ == "__main__":
    unittest.main()
