import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_DIR = ROOT / ".github" / "workflows"
PUBLIC_JIT_WORKFLOW = "fork-safe-validation.yml"


class WorkflowRunnerPolicyTests(unittest.TestCase):
    def test_other_workflows_use_github_hosted_runners(self) -> None:
        for workflow in WORKFLOW_DIR.glob("*.y*"):
            source = workflow.read_text(encoding="utf-8")
            if workflow.name == PUBLIC_JIT_WORKFLOW:
                continue
            self.assertNotRegex(source, r"(?m)^\s*(?:runs-on|labels):.*self-hosted")
            self.assertNotIn("TRUSTED_MAIN_RUNNERS_ENABLED", source)

    def test_public_jit_runner_is_limited_to_fork_pull_requests(self) -> None:
        source = (WORKFLOW_DIR / PUBLIC_JIT_WORKFLOW).read_text(encoding="utf-8")
        runs_on = next(
            line.strip()
            for line in source.splitlines()
            if line.strip().startswith("runs-on:")
        )
        self.assertIn("github.event_name == 'pull_request'", runs_on)
        self.assertIn("github.event.pull_request.head.repo.fork == true", runs_on)
        self.assertIn(
            "fromJSON('{\"group\":\"public-fork-jit\","
            "\"labels\":[\"self-hosted\",\"linux\",\"x64\",\"public-pr-torrentng\"]}')",
            runs_on,
        )
        self.assertIn("|| 'ubuntu-latest' }}", runs_on)
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
        self.assertNotIn("pull_request_target", source + caller)

    def test_windows_smoke_uses_the_hosted_windows_runner(self) -> None:
        source = (WORKFLOW_DIR / "windows-smoke.yml").read_text(encoding="utf-8")
        self.assertRegex(source, r"(?m)^\s*runs-on:\s*windows-latest\s*$")


if __name__ == "__main__":
    unittest.main()
