import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW_DIR = ROOT / ".github" / "workflows"


class WorkflowRunnerPolicyTests(unittest.TestCase):
    def test_workflows_use_github_hosted_runners(self) -> None:
        for workflow in WORKFLOW_DIR.glob("*.y*"):
            source = workflow.read_text(encoding="utf-8")
            self.assertNotRegex(source, r"(?m)^\s*(?:runs-on|labels):.*self-hosted")
            self.assertNotIn("TRUSTED_MAIN_RUNNERS_ENABLED", source)

    def test_windows_smoke_uses_the_hosted_windows_runner(self) -> None:
        source = (WORKFLOW_DIR / "windows-smoke.yml").read_text(encoding="utf-8")
        self.assertRegex(source, r"(?m)^\s*runs-on:\s*windows-latest\s*$")


if __name__ == "__main__":
    unittest.main()
