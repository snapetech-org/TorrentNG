import unittest
from argparse import Namespace
from unittest.mock import patch

from scripts import github_release


class ReleaseDispatchTests(unittest.TestCase):
    def test_dispatch_sends_each_package_workflows_required_inputs(self) -> None:
        cases = (
            ("release-aur.yml", "publish-aur"),
            ("release-copr.yml", "publish-copr"),
        )
        for workflow, action in cases:
            with self.subTest(workflow=workflow):
                args = Namespace(
                    workflow=workflow,
                    ref="main",
                    tag="main-20261115-1",
                )
                with patch("scripts.github_release.api_request") as request:
                    self.assertEqual(github_release.dispatch(args), 0)

                request.assert_called_once_with(
                    "POST",
                    f"actions/workflows/{workflow}/dispatches",
                    {
                        "ref": "main",
                        "inputs": {
                            "action": action,
                            "runner_pool": "github",
                            "tag": "main-20261115-1",
                        },
                    },
                )

    def test_dispatch_rejects_unapproved_workflow(self) -> None:
        args = Namespace(
            workflow="unexpected.yml",
            ref="main",
            tag="main-20261115-1",
        )
        with patch("scripts.github_release.api_request") as request:
            with self.assertRaisesRegex(RuntimeError, "not allowed"):
                github_release.dispatch(args)

        request.assert_not_called()
