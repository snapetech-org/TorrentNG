"""Regression sources for isolated PERF-110 native client launches."""

from __future__ import annotations

import stat
import sys
import tempfile
import tomllib
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from adapters import _task_local_api_context  # noqa: E402
from managed_client import (  # noqa: E402
    ManagedClientError,
    _prepare_torrentng_config,
    _profile_for_role,
)


class ManagedClientTests(unittest.TestCase):
    def test_torrentng_launch_config_uses_private_absolute_trial_paths(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-managed-client-") as temporary:
            root = Path(temporary)
            template = root / "torrentngd-paired.toml"
            template.write_text(
                """[daemon]
api_bind = "0.0.0.0:18080"
session_dir = "session"
log_level = "warn"

[network]
listen_port = 44444

[storage]
download_dir = "../data"
durability_mode = "checkpoint"

[auth]
api_tokens_file = "/tmp/tng-test-token"
""",
                encoding="utf-8",
            )
            state = root / "state"
            data = root / "data"
            state.mkdir(mode=0o700)
            data.mkdir(mode=0o700)
            profile: dict[str, object] = {
                "paired_launch": {
                    "api_port": 18080,
                    "peer_port": 44444,
                    "environment": {"TORRENTNGD_CONFIG": str(template)},
                }
            }
            profile["task_api_context"] = {"address": "198.18.1.3"}

            _prepare_torrentng_config(profile, {"state": state, "data": data})  # type: ignore[arg-type]

            launch = profile["paired_launch"]
            assert isinstance(launch, dict)
            config_path = Path(launch["environment"]["TORRENTNGD_CONFIG"])
            config = tomllib.loads(config_path.read_text(encoding="utf-8"))
            self.assertEqual(config["daemon"]["session_dir"], str(state / "session"))
            self.assertEqual(config["daemon"]["api_bind"], "198.18.1.3:18080")
            self.assertEqual(config["network"]["listen_port"], 44444)
            self.assertEqual(config["storage"]["download_dir"], str(data))
            self.assertEqual(config["auth"]["api_tokens_file"], "/tmp/tng-test-token")
            self.assertTrue(Path(config["daemon"]["session_dir"]).is_absolute())
            self.assertTrue(Path(config["storage"]["download_dir"]).is_absolute())
            self.assertEqual(stat.S_IMODE(config_path.stat().st_mode), 0o600)

    def test_torrentng_launch_config_refuses_a_symlinked_template(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-managed-client-link-") as temporary:
            root = Path(temporary)
            actual = root / "actual.toml"
            actual.write_text(
                '[daemon]\nsession_dir = "/tmp/session"\n',
                encoding="utf-8",
            )
            template = root / "template.toml"
            template.symlink_to(actual)
            state = root / "state"
            data = root / "data"
            state.mkdir(mode=0o700)
            data.mkdir(mode=0o700)
            profile: dict[str, object] = {
                "paired_launch": {
                    "api_port": 18080,
                    "peer_port": 44444,
                    "environment": {"TORRENTNGD_CONFIG": str(template)},
                }
            }

            with self.assertRaisesRegex(ManagedClientError, "absolute regular file"):
                _prepare_torrentng_config(profile, {"state": state, "data": data})  # type: ignore[arg-type]

    def test_role_profile_passes_verified_task_api_context_to_adapter(self) -> None:
        context = {
            "address": "198.18.1.3",
            "namespace": "tngt-a1b2c3d4",
            "namespace_identity": {"device": 8, "inode": 9},
            "subnet": "198.18.1.0/24",
            "interface": "tngveth0",
            "ifindex": 17,
            "topology_sha256": "a" * 64,
        }
        profile: dict[str, object] = {
            "paired_launch": {
                "api_port": 18080,
                "peer_port": 44444,
                "args": [],
                "environment": {},
            },
            "expected_settings": {},
        }
        topology = {
            "target": {"address": "198.18.1.3"},
            "target_namespace": "tngt-a1b2c3d4",
        }
        runtime_paths = {
            "data": Path("/tmp/data"),
            "state": Path("/tmp/state"),
            "tracker_url": "http://198.18.1.2:18081/announce",
            "peer_address": "198.18.1.2:44445",
        }

        with patch("managed_client.client_api_context", return_value=context):
            role_profile, _ = _profile_for_role(
                "torrentngd", profile, "target", topology, runtime_paths  # type: ignore[arg-type]
            )

        self.assertEqual(_task_local_api_context(role_profile), context)


if __name__ == "__main__":
    unittest.main()
