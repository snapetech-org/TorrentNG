import unittest
import os
from pathlib import Path
import subprocess
import tempfile
import textwrap

from scripts.release_notes import parse_release_note


def release_note(body: str, action: str = "none") -> dict:
    content = f"""---
category: fixed
audience: users, operators
area: storage
action: {action}
breaking: false
---
{body}
"""
    return parse_release_note("release-notes/test.md", content)


class ReleaseNoteValidationTests(unittest.TestCase):
    def test_queued_pr_validation_uses_event_base_after_merge(self) -> None:
        root = Path(__file__).resolve().parents[1]
        workflow = (root / ".github/workflows/release-notes.yml").read_text()
        self.assertIn("PR_BASE_SHA: ${{ github.event.pull_request.base.sha }}", workflow)
        script = textwrap.dedent(workflow.split("- name: Validate and preview release notes", 1)[1]
                                .split("        run: |\n", 1)[1])
        with tempfile.TemporaryDirectory() as directory:
            repo = Path(directory)
            def git(*args):
                return subprocess.check_output(["git", "-c", "user.name=Test", "-c",
                    "user.email=test@example.invalid", *args], cwd=repo, text=True).strip()
            git("init", "-q", "-b", "main")
            (repo / "scripts").mkdir()
            (repo / "scripts/release_notes.py").write_bytes((root / "scripts/release_notes.py").read_bytes())
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD")
            git("checkout", "-qb", "feature")
            (repo / "release-notes").mkdir()
            (repo / "release-notes/test.md").write_text("""---
category: fixed
audience: users
area: storage
action: none
breaking: false
---
Torrent recovery resumes after the saved download path is changed.
""")
            git("add", ".")
            git("commit", "-qm", "feature")
            head = git("rev-parse", "HEAD")
            git("checkout", "-q", "main")
            git("merge", "--no-ff", "-qm", "merge", "feature")
            git("update-ref", "refs/remotes/origin/main", "HEAD")
            result = subprocess.run(["bash", "-c", script], cwd=repo, text=True,
                capture_output=True, env={**os.environ, "GITHUB_EVENT_NAME": "pull_request",
                    "PR_BASE_SHA": base, "PR_BASE_REF": "main", "PR_HEAD_SHA": head, "PR_BODY": ""})
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Torrent recovery resumes", result.stdout)

    def test_accepts_valid_note(self) -> None:
        note = release_note(
            "Torrent recovery now resumes from the saved log after the data path changes."
        )

        self.assertEqual(note["errors"], [])

    def test_accepts_brand_styled_rtorrent_at_sentence_start(self) -> None:
        note = release_note(
            "rTorrent log recovery is now recorded consistently when a previously missing log file appears."
        )

        self.assertEqual(note["errors"], [])

    def test_rejects_html_comment_delimiters_in_action(self) -> None:
        for delimiter in ("<!--", "-->"):
            with self.subTest(delimiter=delimiter):
                note = release_note(
                    "Torrent recovery now resumes from the saved log after the data path changes.",
                    f"restart the service {delimiter}",
                )

                self.assertIn(
                    "action contains a placeholder or HTML comment", note["errors"]
                )

    def test_rejects_html_comment_delimiters_in_body(self) -> None:
        for delimiter in ("<!--", "-->"):
            with self.subTest(delimiter=delimiter):
                note = release_note(
                    f"Torrent recovery now resumes from the saved log {delimiter} after the data path changes."
                )

                self.assertIn(
                    "body contains a placeholder or HTML comment", note["errors"]
                )

    def test_rejects_placeholders_case_insensitively(self) -> None:
        action_note = release_note(
            "Torrent recovery now resumes from the saved log after the data path changes.",
            "Complete the TODO item",
        )
        body_note = release_note(
            "Torrent recovery now resumes from the saved log after the TODO is complete."
        )

        self.assertIn(
            "action contains a placeholder or HTML comment", action_note["errors"]
        )
        self.assertIn("body contains a placeholder or HTML comment", body_note["errors"])


if __name__ == "__main__":
    unittest.main()
