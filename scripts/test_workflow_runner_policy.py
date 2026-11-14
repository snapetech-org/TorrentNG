import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_DIR = ROOT / ".github" / "workflows"
PUBLIC_FORK_WORKFLOW = "fork-safe-validation.yml"
PUBLIC_FORK_WORKFLOW_APPROVED_SHA = "50baefbbe1b12f1bfed7918fb376982402cfab96"
PROTECTED_TRANSFER_WORKFLOW = "transfer-rebuild.yml"


class WorkflowRunnerPolicyTests(unittest.TestCase):
    def job(self, workflow: str, name: str) -> str:
        source = (WORKFLOW_DIR / workflow).read_text(encoding="utf-8")
        match = re.search(
            rf"(?ms)^  {re.escape(name)}:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
            source,
        )
        self.assertIsNotNone(match, f"{workflow}:{name}")
        return match.group("body")

    def test_release_requires_manual_exact_tag_and_github_runner(self) -> None:
        source = (WORKFLOW_DIR / "release.yml").read_text(encoding="utf-8")
        self.assertRegex(source, r"(?m)^  workflow_dispatch:$")
        self.assertNotRegex(source, r"(?m)^  push:")
        self.assertIn("inputs.action == 'release'", source)
        self.assertIn("inputs.runner_pool == 'github'", source)
        self.assertIn("options: [github]", source)
        self.assertIn("inputs.release_tag", source)
        self.assertIn("runs-on: ubuntu-24.04", source)
        self.assertIn("matrix.os == 'windows-x64' && 'windows-2022'", source)
        self.assertIn("matrix.os == 'macos-arm64' && 'macos-15'", source)
        self.assertIn("matrix.os == 'linux-arm64' && 'ubuntu-24.04-arm'", source)
        self.assertNotIn('"group":"ng-shared-main-jit"', source)

    def test_codeql_workflow_is_disabled(self) -> None:
        self.assertFalse((WORKFLOW_DIR / "codeql.yml").exists())

    def test_aur_publish_uses_public_hosted_ssh_without_private_proxy(self) -> None:
        source = (WORKFLOW_DIR / "release-aur.yml").read_text(encoding="utf-8")
        self.assertIn("runs-on: ubuntu-24.04", source)
        self.assertIn("aur.archlinux.org", source)
        self.assertNotIn("ProxyCommand", source)
        self.assertNotIn("10.0.2.100", source)

    def test_main_ci_uses_ephemeral_github_runners_and_trusted_owner_pr_gate(self) -> None:
        source = (WORKFLOW_DIR / "ci.yml").read_text(encoding="utf-8")
        self.assertIn("# ng-runner-policy: github-hosted-default-v1", source)
        self.assertNotRegex(source, r"(?m)^  push:")
        self.assertRegex(
            source,
            r"(?ms)^  pull_request_target:\n"
            r"    branches:\n      - main\n"
            r"    types:\n      - labeled\n",
        )
        self.assertNotRegex(source, r"(?m)^      - (?:opened|synchronize|reopened|ready_for_review)$")
        self.assertIn("inputs.action == 'ci'", source)
        self.assertIn("inputs.runner_pool == 'github'", source)
        dispatcher = self.job("ci.yml", "dispatch-owner-pr")
        self.assertIn("github.event.pull_request.user.login == 'snapetech'", dispatcher)
        self.assertIn("github.event.pull_request.head.repo.id == github.repository_id", dispatcher)
        self.assertIn("github.event.label.name == 'action:ci'", dispatcher)
        self.assertIn("contains(github.event.pull_request.labels.*.name, 'pool:github')", dispatcher)
        self.assertIn("contents: none", dispatcher)
        self.assertNotIn("actions/checkout", dispatcher)
        self.assertNotIn("secrets.", dispatcher)
        for name in [
            "native-quality", "backend-fault-matrix", "backend-api-load", "msrv-check",
            "msrv-check-sidecar", "fuzz-smoke", "sidecar", "desktop", "webui",
            "backup-restore", "security-audit", "candidate-artifact", "dispatch-owner-pr",
        ]:
            body = self.job("ci.yml", name)
            self.assertIn("runs-on: ubuntu-24.04", body, name)
            self.assertNotIn("self-hosted", body, name)
        self.assertIn('"runner_pool": "github"', source)

    def test_webui_uses_github_runner_and_pinned_playwright_container(self) -> None:
        body = self.job("ci.yml", "webui")
        self.assertIn("runs-on: ubuntu-24.04", body)
        self.assertNotIn("container:", body)
        self.assertIn(
            "mcr.microsoft.com/playwright@sha256:2f05f8a734509a7b3f26a912639488ee08f58a7b292d14d62271bcff9038768f",
            body,
        )
        self.assertIn("Expected Playwright 1.64.1", body)
        self.assertIn("docker run --rm --init --ipc=host", body)
        self.assertIn("npm run test:e2e -- --reporter=line", body)
        self.assertNotIn("npx playwright install --with-deps chromium", body)
        self.assertIn("inputs.candidate_sha == ''", body)

    def test_candidate_artifact_uses_exact_source_and_never_publishes(self) -> None:
        source = (WORKFLOW_DIR / "ci.yml").read_text(encoding="utf-8")
        self.assertIn("Exact main SHA for a GitHub-hosted Linux x64 build artifact (no release)", source)
        body = self.job("ci.yml", "candidate-artifact")
        for condition in (
            "github.event_name == 'workflow_dispatch'", "github.actor == 'snapetech'",
            "github.ref == 'refs/heads/main'", "inputs.action == 'build'",
            "inputs.runner_pool == 'github'", "inputs.candidate_sha != ''",
            "inputs.candidate_sha == github.sha", "inputs.pr_number == ''",
            '[[ "$candidate_sha" == "${GITHUB_SHA,,}" ]]',
            "cargo build --release --locked -p torrentngd",
            "scripts/backend_burndown_native_release_smoke.sh",
            "-f deploy/native/Dockerfile", "torrentng-native-image.tar.gz",
            "-f deploy/docker/Dockerfile.phase1", "torrentng-phase1-image.tar.gz",
            "actions/upload-artifact@", "retention-days: 30",
        ):
            self.assertIn(condition, body)
        self.assertIn("runs-on: ubuntu-24.04", body)
        self.assertNotIn("ref: ${{ inputs.candidate_sha }}", body)
        self.assertNotIn("gh release", body.lower())
        self.assertNotIn("git push", body.lower())
        fixture_path = ROOT / "certification" / "fixtures" / "backend-burndown-native-release-smoke.toml"
        fixture = fixture_path.read_text(encoding="utf-8")
        token_match = re.search(r'(?m)^api_tokens = \["([^\"]+)"\]$', fixture)
        self.assertIsNotNone(token_match, "release-smoke fixture must declare one API token")
        self.assertIn(f"TNG_RELEASE_TOKEN={token_match.group(1)}", body)

    def test_standard_workflows_use_github_hosted_runners(self) -> None:
        for workflow_name in (
            "backend-evidence.yml", "release-notes.yml", "semgrep-community.yml",
            "trivy-filesystem.yml", "release-aur.yml", "release-copr.yml",
            "release-ppa.yml", "package-smoke-disabled.yml",
        ):
            source = (WORKFLOW_DIR / workflow_name).read_text(encoding="utf-8")
            self.assertIn("options: [github]", source, workflow_name)
            self.assertIn("inputs.runner_pool == 'github'", source, workflow_name)
            self.assertIn("runs-on: ubuntu-24.04", source, workflow_name)
            self.assertNotIn('"group":"ng-shared-main-jit"', source, workflow_name)
        semgrep = (WORKFLOW_DIR / "semgrep-community.yml").read_text(encoding="utf-8")
        self.assertIn("inputs.action == 'semgrep'", semgrep)
        self.assertIn("docker info", semgrep)
        self.assertIn("docker run", semgrep)
        trivy = (WORKFLOW_DIR / "trivy-filesystem.yml").read_text(encoding="utf-8")
        self.assertIn("inputs.action == 'trivy'", trivy)

    def test_windows_and_untrusted_fork_jobs_use_isolated_hosted_runners(self) -> None:
        windows = (WORKFLOW_DIR / "windows-smoke.yml").read_text(encoding="utf-8")
        self.assertIn("options: [github]", windows)
        self.assertIn("runs-on: windows-2022", windows)
        self.assertIn("scripts/enable-msvc-environment.ps1", windows.replace("\\", "/"))
        msvc_environment = (ROOT / "scripts" / "enable-msvc-environment.ps1").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            "$writer.WriteLine($name + '=' + [string]$toolchainEnvironment[$name])",
            msvc_environment,
        )

        fork = (WORKFLOW_DIR / PUBLIC_FORK_WORKFLOW).read_text(encoding="utf-8")
        self.assertIn("runs-on: ubuntu-24.04", fork)
        self.assertIn("workflow_call:", fork)
        self.assertIn("inputs.runner_pool == 'public' || inputs.runner_pool == 'hybrid'", fork)
        self.assertIn("github.event.pull_request.base.repo.full_name == 'snapetech-org/TorrentNG'", fork)
        self.assertIn("contents: read", fork)
        self.assertNotIn("secrets:", fork)
        self.assertNotIn("self-hosted", fork)

        caller = (WORKFLOW_DIR / "public-fork-pr.yml").read_text(encoding="utf-8")
        reference = re.search(
            r"(?m)^\s+uses: snapetech-org/TorrentNG/\.github/workflows/"
            r"fork-safe-validation\.yml@([0-9a-f]{40})$",
            caller,
        )
        self.assertIsNotNone(reference)
        self.assertEqual(reference.group(1), PUBLIC_FORK_WORKFLOW_APPROVED_SHA)

    def test_only_hardware_and_sandbox_specific_workflows_keep_private_jit(self) -> None:
        transfer = (WORKFLOW_DIR / PROTECTED_TRANSFER_WORKFLOW).read_text(encoding="utf-8")
        self.assertIn("runs-on: ubuntu-24.04", self.job(PROTECTED_TRANSFER_WORKFLOW, "hosted-correctness"))
        self.assertIn('"group":"ng-shared-main-jit"', transfer)
        self.assertIn('"torrentng-transfer-perf"', transfer)
        self.assertIn('"gha-{0}-transfer-rebuild-dedicated-hardware"', transfer)
        self.assertIn("inputs.runner_pool == 'github' && inputs.run_dedicated_hardware == true", transfer)
        self.assertRegex(transfer, r"(?m)^\s*environment: transfer-rebuild-hardware$")
        self.assertIn("persist-credentials: false", transfer)

        gvisor = (WORKFLOW_DIR / "gvisor-runner-smoke.yml").read_text(encoding="utf-8")
        self.assertIn("group: gvisor-sandbox", gvisor)
        self.assertIn("labels: gvisor-sandbox", gvisor)
        self.assertIn('inputs.action == \'gvisor-smoke\'', gvisor)

        for workflow in WORKFLOW_DIR.glob("*.yml"):
            source = workflow.read_text(encoding="utf-8")
            if workflow.name not in {PROTECTED_TRANSFER_WORKFLOW, "gvisor-runner-smoke.yml"}:
                self.assertNotRegex(source, r"(?m)^\s*runs-on:.*self-hosted")
                self.assertNotIn('"group":"ng-shared-main-jit"', source)


if __name__ == "__main__":
    unittest.main()
