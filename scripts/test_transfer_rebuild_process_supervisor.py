"""Regression sources for exact pinned-process lifecycle management."""

from __future__ import annotations

import hashlib
import os
import signal
import sys
import tempfile
import time
import unittest
from pathlib import Path

TOOLS = Path(__file__).parent / "transfer_rebuild"
sys.path.insert(0, str(TOOLS))

from process_supervisor import (  # noqa: E402
    ManagedProcess,
    ProcessSupervisorError,
)


class ProcessSupervisorTests(unittest.TestCase):
    def setUp(self) -> None:
        self.binary = Path(sys.executable).resolve(strict=True)
        self.digest = hashlib.sha256(self.binary.read_bytes()).hexdigest()
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)

    def _start(
        self,
        log_path: Path,
        argument: str = "literal ; $NOT_A_SHELL",
        pid_file_path: Path | None = None,
    ) -> ManagedProcess:
        return ManagedProcess.start(
            [
                str(self.binary),
                "-c",
                "import sys, time; print(sys.argv[1], flush=True); time.sleep(30)",
                argument,
            ],
            binary_path=self.binary,
            expected_binary_sha256=self.digest,
            log_path=log_path,
            environment={"PYTHONIOENCODING": "utf-8"},
            pid_file_path=pid_file_path,
        )

    def test_launch_binds_executable_identity_and_passes_arguments_without_a_shell(self) -> None:
        log_path = self.root / "client.log"
        pid_file = self.root / "client.pid"
        process = self._start(log_path, pid_file_path=pid_file)
        self.addCleanup(process.close)
        try:
            self.assertGreater(process.pid, 0)
            self.assertTrue(process.still_matches_pin())
            self.assertEqual(pid_file.read_text(encoding="ascii"), f"{process.pid}\n")
            identity = process.runtime_identity()
            self.assertEqual(identity["process_id"], process.pid)
            self.assertEqual(identity["binary_sha256"], self.digest)
            self.assertEqual(identity["process_start_ticks"], process.identity.start_ticks)
            deadline = time.monotonic() + 2
            while time.monotonic() < deadline and not log_path.read_text(encoding="utf-8"):
                time.sleep(0.01)
            self.assertEqual(log_path.read_text(encoding="utf-8").strip(), "literal ; $NOT_A_SHELL")
            self.assertEqual(os.stat(log_path).st_mode & 0o777, 0o600)
            self.assertEqual(os.stat(pid_file).st_mode & 0o777, 0o600)
            return_code = process.stop(grace_seconds=2)
            self.assertEqual(return_code, -signal.SIGTERM)
        finally:
            if process._process.poll() is None:
                process.stop(grace_seconds=2)

    def test_wrong_digest_fails_before_creating_a_log_or_process(self) -> None:
        log_path = self.root / "client.log"
        with self.assertRaisesRegex(ProcessSupervisorError, "digest differs"):
            ManagedProcess.start(
                [str(self.binary), "-c", "pass"],
                binary_path=self.binary,
                expected_binary_sha256="0" * 64,
                log_path=log_path,
                environment={},
            )
        self.assertFalse(log_path.exists())

    def test_existing_log_is_not_overwritten(self) -> None:
        log_path = self.root / "client.log"
        log_path.write_text("previous evidence\n", encoding="utf-8")
        with self.assertRaisesRegex(ProcessSupervisorError, "cannot create"):
            self._start(log_path)
        self.assertEqual(log_path.read_text(encoding="utf-8"), "previous evidence\n")

    def test_shell_wrappers_are_not_accepted_as_the_pinned_binary(self) -> None:
        with self.assertRaisesRegex(ProcessSupervisorError, r"argv\[0\]"):
            ManagedProcess.start(
                ["/bin/sh", "-c", "true"],
                binary_path=self.binary,
                expected_binary_sha256=self.digest,
                log_path=self.root / "client.log",
                environment={},
            )


if __name__ == "__main__":
    unittest.main()
