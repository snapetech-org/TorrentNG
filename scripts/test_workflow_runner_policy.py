import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CI_WORKFLOW = ROOT / ".github" / "workflows" / "ci.yml"
RELEASE_WORKFLOW = ROOT / ".github" / "workflows" / "release.yml"
MAIN_RUNNER_CONDITION = (
    "((github.event_name == 'push' && github.ref == 'refs/heads/main') "
    "|| (github.event_name == 'workflow_dispatch' && github.ref == 'refs/heads/main'))"
)
TRUSTED_MAIN_RUNNER_JSON = '{"group":"trusted-main","labels":["self-hosted","Linux","X64","host"]}'
TRUSTED_MAIN_RUNNER = (
    "$" + "{{ (" + MAIN_RUNNER_CONDITION
    + " && vars.TRUSTED_MAIN_RUNNERS_ENABLED == 'true') && fromJSON('" + TRUSTED_MAIN_RUNNER_JSON + "') || 'ubuntu-latest' }}"
)
TRUSTED_MAIN_SNAPSHOT_PROFILE = (
    "$" + "{{ (" + MAIN_RUNNER_CONDITION
    + " && vars.TRUSTED_MAIN_RUNNERS_ENABLED == 'true') && 'self-hosted' || 'hosted' }}"
)


def shared_runner_selectors(source: str) -> list[str]:
    return re.findall(r"(?m)^    runs-on: (\$\{\{[^\n]+\}\})$", source)


class WorkflowRunnerPolicyTests(unittest.TestCase):
    def test_ci_local_runners_are_limited_to_ready_main_pool(self) -> None:
        source = CI_WORKFLOW.read_text(encoding="utf-8")
        runners = shared_runner_selectors(source)
        self.assertEqual(len(runners), 7)
        self.assertEqual(runners, [TRUSTED_MAIN_RUNNER] * 7)
        for runner in runners:
            self.assertNotIn("github.event_name == 'pull_request'", runner)
            self.assertNotIn("kspld0", runner)
            self.assertNotIn("kspls0", runner)
            self.assertIn("fromJSON('" + TRUSTED_MAIN_RUNNER_JSON + "')", runner)

    def test_ci_runner_temp_is_resolved_before_validation(self) -> None:
        source = CI_WORKFLOW.read_text(encoding="utf-8")
        self.assertEqual(
            source.count('runner_temp_root="$(realpath -e -- "$RUNNER_TEMP")"'),
            7,
        )
        self.assertEqual(
            source.count("printf 'TMPDIR=%s\\n' \"$runner_temp_root\""),
            7,
        )
        self.assertIn('mktemp -d -- "$TMPDIR/cargo-fuzz.XXXXXX"', source)

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

    def test_snapshot_profile_matches_actual_runner_selection(self) -> None:
        source = CI_WORKFLOW.read_text(encoding="utf-8")
        snapshot_profile = next(
            line.strip().removeprefix("TNG_WEBUI_SNAPSHOT_PROFILE: ")
            for line in source.splitlines()
            if "TNG_WEBUI_SNAPSHOT_PROFILE:" in line
        )
        self.assertEqual(snapshot_profile, TRUSTED_MAIN_SNAPSHOT_PROFILE)
        self.assertNotIn("fromJSON", snapshot_profile)


if __name__ == "__main__":
    unittest.main()
