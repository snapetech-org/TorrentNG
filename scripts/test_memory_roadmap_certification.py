#!/usr/bin/env python3
"""The memory roadmap certification must not pass on another commit's report."""

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent / "memory_roadmap_certification.sh"
PASSING_TESTS = """\
test idle_memory_100k_keeps_fixed_rss_task_fd_budget ... ok
test hot_seeding_1k_memory_attribution_stays_under_cop... ok
"""


def git(root: Path, *args: str) -> str:
    return subprocess.run(
        ["git", "-C", str(root), *args],
        check=True,
        capture_output=True,
        text=True,
        env={
            "GIT_AUTHOR_NAME": "t",
            "GIT_AUTHOR_EMAIL": "t@example.invalid",
            "GIT_COMMITTER_NAME": "t",
            "GIT_COMMITTER_EMAIL": "t@example.invalid",
            "PATH": "/usr/bin:/bin:/usr/local/bin",
        },
    ).stdout.strip()


class CertificationIdentityTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="tng-cert-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        (self.tmp / "scripts").mkdir()
        shutil.copy(SCRIPT, self.tmp / "scripts" / SCRIPT.name)
        (self.tmp / "certification" / "reports").mkdir(parents=True)
        (self.tmp / "README").write_text("x\n")
        git(self.tmp, "init", "-q")
        git(self.tmp, "add", "-A")
        git(self.tmp, "commit", "-q", "-m", "init")
        self.head = git(self.tmp, "rev-parse", "HEAD")

    def run_script(self, commit: str) -> str:
        report = self.tmp / "certification" / "reports" / "local-release-test.md"
        report.write_text(f"# local\n\n- Commit: {commit}\n\n{PASSING_TESTS}")
        out = self.tmp / "out.md"
        subprocess.run(
            ["bash", str(self.tmp / "scripts" / SCRIPT.name), str(out)],
            check=False,
            capture_output=True,
            text=True,
        )
        return out.read_text()

    def row(self, text: str, name: str) -> str:
        return next(line for line in text.splitlines() if line.startswith(f"| {name} "))

    def test_report_for_the_current_commit_passes(self):
        text = self.run_script(self.head[:8])
        self.assertIn("| PASS |", self.row(text, "idle RSS/task/fd regression proxy"))

    def test_report_for_another_commit_is_not_accepted(self):
        text = self.run_script("deadbeef")
        row = self.row(text, "idle RSS/task/fd regression proxy")
        self.assertIn("| FAIL |", row)
        self.assertIn("not valid for this commit", row)

    def test_report_without_a_commit_is_not_accepted(self):
        text = self.run_script("unavailable")
        self.assertIn("| FAIL |", self.row(text, "idle RSS/task/fd regression proxy"))


if __name__ == "__main__":
    unittest.main()
