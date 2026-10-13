from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"


class WorkflowRunnerPolicyTests(unittest.TestCase):
    def test_ci_local_runners_are_limited_to_main(self) -> None:
        source = CI_WORKFLOW.read_text(encoding="utf-8")
        runners = [
            line.strip()
            for line in source.splitlines()
            if line.lstrip().startswith("runs-on:") and "self-hosted" in line
        ]
        self.assertEqual(len(runners), 7)
        for runner in runners:
            self.assertNotIn("github.event_name == 'pull_request'", runner)
            self.assertIn(
                "github.event_name == 'push' && github.ref == 'refs/heads/main'",
                runner,
            )
            self.assertIn(
                "github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main'",
                runner,
            )
            self.assertIn('["self-hosted","Linux","X64","host"]', runner)
            self.assertIn('["ubuntu-latest"]', runner)
            self.assertNotIn("kspld0", runner)
            self.assertNotIn("kspls0", runner)

    def test_release_workflow_has_no_pull_request_trigger(self) -> None:
        source = RELEASE_WORKFLOW.read_text(encoding="utf-8")
        trigger_block = source.split("jobs:", maxsplit=1)[0]
        self.assertNotIn("pull_request:", trigger_block)
        runners = [
            line.strip()
            for line in source.splitlines()
            if line.lstrip().startswith("runs-on:") and "self-hosted" in line
        ]
        self.assertEqual(len(runners), 2)
        for runner in runners:
            self.assertIn("host", runner)

    def test_all_pull_requests_use_hosted_runners_and_snapshot_profile(self) -> None:
        source = CI_WORKFLOW.read_text(encoding="utf-8")
        local_runner = next(
            line.strip()
            for line in source.splitlines()
            if line.lstrip().startswith("runs-on:") and "self-hosted" in line
        )
        snapshot_profile = next(
            line.strip()
            for line in source.splitlines()
            if "TNG_WEBUI_SNAPSHOT_PROFILE:" in line
        )
        for condition in (
            "github.event_name == 'push' && github.ref == 'refs/heads/main'",
            "github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main'",
        ):
            self.assertIn(condition, local_runner)
            self.assertIn(condition, snapshot_profile)
        self.assertNotIn("github.event_name == 'pull_request'", local_runner)
        self.assertNotIn("github.event_name == 'pull_request'", snapshot_profile)
        self.assertIn("&& 'self-hosted' || 'hosted'", snapshot_profile)


if __name__ == "__main__":
    unittest.main()
