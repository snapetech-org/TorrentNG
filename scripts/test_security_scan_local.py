from __future__ import annotations

import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SCANNER = REPO_ROOT / "scripts" / "security-scan-local.sh"


class TrivyCiFailClosedTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temp_dir = tempfile.TemporaryDirectory(prefix="torrentng-trivy-ci-")
        self.addCleanup(self.temp_dir.cleanup)
        self.repo = Path(self.temp_dir.name)
        self.bin_dir = self.repo / "bin"
        self.bin_dir.mkdir()
        (self.repo / "scripts").mkdir()
        shutil.copy2(SCANNER, self.repo / "scripts" / "security-scan-local.sh")

        for path in (
            "Cargo.lock",
            "webui/package-lock.json",
            "deploy/docker/Dockerfile",
            "crates/torrentngd/src/main.rs",
        ):
            input_path = self.repo / path
            input_path.parent.mkdir(parents=True, exist_ok=True)
            input_path.write_text("test input\n", encoding="utf-8")

        self.capture_path = self.repo / "trivy-args.txt"
        self.count_path = self.repo / "trivy-count.txt"
        self._write_executable(
            self.bin_dir / "git",
            "#!/bin/sh\n"
            'if [ "$1" = "rev-parse" ] && [ "$2" = "--show-toplevel" ]; then\n'
            '  printf "%s\\n" "$TNG_TEST_REPO"\n'
            "  exit 0\n"
            "fi\n"
            "exit 2\n",
        )
        self._write_trivy()
        self.env = os.environ.copy()
        self.env.update(
            {
                "PATH": str(self.bin_dir),
                "TNG_TEST_REPO": str(self.repo),
                "TRIVY_CAPTURE": str(self.capture_path),
                "TRIVY_COUNT": str(self.count_path),
                "TRIVY_FAKE_FAIL_CALL": "0",
                # CI must ignore this local report-only override.
                "TRIVY_EXIT_CODE": "0",
            }
        )

    @staticmethod
    def _write_executable(path: Path, contents: str) -> None:
        path.write_text(contents, encoding="utf-8")
        path.chmod(0o755)

    def _write_trivy(self) -> None:
        self._write_executable(
            self.bin_dir / "trivy",
            "#!/bin/sh\n"
            'count=0\n'
            'if [ -f "$TRIVY_COUNT" ]; then IFS= read -r count < "$TRIVY_COUNT"; fi\n'
            'count=$((count + 1))\n'
            'printf "%s\\n" "$count" > "$TRIVY_COUNT"\n'
            'printf "CALL:%s\\n%s\\n" "$count" "$*" >> "$TRIVY_CAPTURE"\n'
            'if [ "$count" = "${TRIVY_FAKE_FAIL_CALL:-0}" ]; then exit 1; fi\n'
            'exit 0\n',
        )

    def _run(self) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            ["/bin/bash", "scripts/security-scan-local.sh", "--trivy-only", "--ci"],
            cwd=self.repo,
            env=self.env,
            text=True,
            capture_output=True,
            check=False,
        )

    def test_ci_scans_categories_with_correct_scope_without_rg(self) -> None:
        result = self._run()

        self.assertEqual(result.returncode, 0, result.stderr)
        args = self.capture_path.read_text(encoding="utf-8").splitlines()
        self.assertEqual([args[i] for i in (0, 2, 4)], ["CALL:1", "CALL:2", "CALL:3"])
        vuln_args, secret_args, config_args = (args[i] for i in (1, 3, 5))
        self.assertIn("fs --scanners vuln", vuln_args)
        self.assertIn("certification/reports", vuln_args)
        self.assertIn("fs --scanners secret", secret_args)
        self.assertNotIn("certification/reports", secret_args)
        self.assertIn("config ", config_args)
        self.assertIn("certification/reports", config_args)
        for call_args in (vuln_args, secret_args, config_args):
            self.assertIn("--exit-code 1", call_args)
            self.assertIn("--config=", call_args)
            self.assertIn("--ignorefile=", call_args)
        self.assertIn("--secret-config=", secret_args)

    def test_ci_propagates_findings_from_every_scan_phase(self) -> None:
        for failing_call in (1, 2, 3):
            with self.subTest(failing_call=failing_call):
                self.capture_path.unlink(missing_ok=True)
                self.count_path.unlink(missing_ok=True)
                self.env["TRIVY_FAKE_FAIL_CALL"] = str(failing_call)

                result = self._run()

                self.assertNotEqual(result.returncode, 0)
                calls = [
                    line
                    for line in self.capture_path.read_text(encoding="utf-8").splitlines()
                    if line.startswith("CALL:")
                ]
                self.assertEqual(len(calls), failing_call)

    def test_ci_fails_when_an_expected_input_is_missing(self) -> None:
        (self.repo / "deploy/docker/Dockerfile").unlink()

        result = self._run()

        self.assertEqual(result.returncode, 2)
        self.assertIn("missing required input: deploy/docker/Dockerfile", result.stderr)
        self.assertFalse(self.capture_path.exists())

    def test_ci_fails_when_trivy_is_unavailable(self) -> None:
        (self.bin_dir / "trivy").unlink()

        result = self._run()

        self.assertEqual(result.returncode, 127)
        self.assertIn("Trivy is unavailable", result.stderr)


if __name__ == "__main__":
    unittest.main()
