"""Keep the opt-in hardware lane runnable from checkout and CI."""

from __future__ import annotations

import stat
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


class HardwareLaneWiringTests(unittest.TestCase):
    def test_hardware_lane_entrypoint_is_executable(self) -> None:
        entrypoint = ROOT / "scripts/transfer_rebuild/hardware_lane.sh"
        self.assertTrue(entrypoint.stat().st_mode & stat.S_IXUSR)

    def test_workflow_invokes_hardware_lane_with_bash(self) -> None:
        workflow = (ROOT / ".github/workflows/transfer-rebuild.yml").read_text(
            encoding="utf-8"
        )
        self.assertIn(
            'bash scripts/transfer_rebuild/hardware_lane.sh "$output" "$STORAGE_ROOT"',
            workflow,
        )


if __name__ == "__main__":
    unittest.main()
