#!/usr/bin/env python3
"""Check that the candidate image route stays exact, private, and non-publishing."""

from __future__ import annotations

import re
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
WORKFLOW = ROOT / ".github/workflows/ci.yml"


class CandidateArtifactRouteTests(unittest.TestCase):
    def test_candidate_image_uses_exact_private_artifact_route(self) -> None:
        source = WORKFLOW.read_text(encoding="utf-8")
        match = re.search(
            r"(?ms)^  candidate-artifact:\n(?P<body>.*?)(?=^  [A-Za-z0-9_-]+:\n|\Z)",
            source,
        )
        self.assertIsNotNone(match)
        body = match.group("body")

        for condition in (
            "github.event_name == 'workflow_dispatch'",
            "github.actor == 'snapetech'",
            "github.ref == 'refs/heads/main'",
            "inputs.action == 'build'",
            "inputs.runner_pool == 'private'",
            "inputs.candidate_sha != ''",
            "inputs.candidate_sha == github.sha",
            "inputs.pr_number == ''",
            "inputs.pr_sha == ''",
            "inputs.target_run_id == ''",
        ):
            self.assertIn(condition, body)

        runner = re.search(r"(?m)^    runs-on: (?P<runner>.+)$", body)
        self.assertIsNotNone(runner)
        for route in (
            '"group":"ng-shared-main-jit"',
            '"ng-shared-main-torrentng"',
            '"gha-{0}-candidate-artifact"',
        ):
            self.assertIn(route, runner.group("runner"))
        self.assertNotIn("ubuntu-latest", runner.group("runner"))

        for build_step in (
            "ref: ${{ inputs.candidate_sha }}",
            '[[ "$candidate_sha" == "${GITHUB_SHA,,}" ]]',
            'docker build --pull --network host',
            "-f deploy/docker/Dockerfile",
            'docker save "$IMAGE_TAG"',
            "sha256sum rtorrentng-compatible-image.tar.gz > rtorrentng-compatible-image.tar.gz.sha256",
            "actions/upload-artifact@",
            "retention-days: 30",
        ):
            self.assertIn(build_step, body)

        lowered = body.lower()
        self.assertNotIn("docker push", lowered)
        self.assertNotIn("gh release", lowered)
        self.assertNotIn("git push", lowered)


if __name__ == "__main__":
    unittest.main()
