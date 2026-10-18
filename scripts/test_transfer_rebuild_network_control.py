"""Regression sources for task-scoped transfer benchmark network controls."""

from __future__ import annotations

import hashlib
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).parent / "transfer_rebuild"))
from network_control import (  # noqa: E402
    NetworkControlError,
    PROFILES,
    _expected_netem,
    _record_digest,
    _validate_names,
    apply_profile,
    reset_profile,
)

TOOLCHAIN_SHA = hashlib.sha256(b'{"ip":"ip v1","tc":"tc v1"}').hexdigest()


def link_info(ifindex: int = 17) -> dict[str, object]:
    return {
        "ifname": "tngt0",
        "ifindex": ifindex,
        "flags": ["BROADCAST", "MULTICAST", "UP", "LOWER_UP"],
        "linkinfo": {"info_kind": "veth"},
    }


def netem_root(profile_id: str = "lan-1g-1ms-v1") -> dict[str, object]:
    expected = _expected_netem(profile_id)
    return {
        "kind": "netem",
        "handle": "7fff:",
        "root": True,
        "options": {
            "delay": {"delay": expected["delay_ns"] / 1_000_000_000, "jitter": 0.0},
            "rate": {"rate": expected["rate_bps"], "packetoverhead": 0, "cellsize": 0},
            "limit": expected["limit_packets"],
        },
    }


class TransferNetworkControlTests(unittest.TestCase):
    def test_network_names_are_limited_to_task_owned_namespace_and_veth(self) -> None:
        _validate_names("tngt-case-1", "tngt0")
        with self.assertRaises(NetworkControlError):
            _validate_names("default", "tngt0")
        with self.assertRaises(NetworkControlError):
            _validate_names("tngt-case-1", "lo")
        with self.assertRaises(NetworkControlError):
            _validate_names("tngt-case-1", "eth0")

    def test_apply_records_only_a_verified_pinned_profile(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-network-control-") as directory:
            output = Path(directory) / "apply.json"
            with (
                patch("network_control._tools", return_value=("/sbin/ip", "/sbin/tc")),
                patch("network_control._namespace_identity", return_value={"device": 8, "inode": 9}),
                patch("network_control._link_info", side_effect=[link_info(), link_info()]),
                patch(
                    "network_control._root_qdisc",
                    side_effect=[
                        {"kind": "noqueue", "handle": "0:", "root": True},
                        netem_root(),
                    ],
                ),
                patch("network_control._toolchain_identity", return_value=("ip v1", "tc v1", TOOLCHAIN_SHA)),
                patch("network_control._run") as run,
            ):
                record = apply_profile("tngt-case-1", "tngt0", "lan-1g-1ms-v1", output)

            argv = run.call_args.args[0]
            self.assertEqual(argv[:4], ["/sbin/ip", "netns", "exec", "tngt-case-1"])
            self.assertIn("/sbin/tc", argv)
            self.assertIn("qdisc", argv)
            self.assertEqual(record["action"], "apply")
            self.assertEqual(record["observed_root_qdisc"], _expected_netem("lan-1g-1ms-v1"))
            disk_record = json.loads(output.read_text(encoding="utf-8"))
            self.assertEqual(disk_record["artifact_sha256"], _record_digest(disk_record))
            self.assertEqual(stat_mode(output), 0o600)

    def test_apply_refuses_to_replace_an_existing_root_qdisc(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-network-control-") as directory:
            output = Path(directory) / "apply.json"
            with (
                patch("network_control._tools", return_value=("/sbin/ip", "/sbin/tc")),
                patch("network_control._namespace_identity", return_value={"device": 8, "inode": 9}),
                patch("network_control._link_info", return_value=link_info()),
                patch(
                    "network_control._root_qdisc",
                    return_value={"kind": "fq_codel", "handle": "0:", "root": True},
                ),
                patch("network_control._run") as run,
            ):
                with self.assertRaises(NetworkControlError):
                    apply_profile("tngt-case-1", "tngt0", "lan-1g-1ms-v1", output)
            run.assert_not_called()
            self.assertFalse(output.exists())

    def test_apply_rolls_back_its_qdisc_when_artifact_creation_fails(self) -> None:
        profile_id = "lan-1g-1ms-v1"
        with tempfile.TemporaryDirectory(prefix="tng-network-control-") as directory:
            with (
                patch("network_control._tools", return_value=("/sbin/ip", "/sbin/tc")),
                patch("network_control._namespace_identity", return_value={"device": 8, "inode": 9}),
                patch("network_control._link_info", side_effect=[link_info(), link_info()]),
                patch(
                    "network_control._root_qdisc",
                    side_effect=[{"kind": "noqueue", "handle": "0:"}, netem_root(), netem_root()],
                ),
                patch("network_control._toolchain_identity", return_value=("ip v1", "tc v1", TOOLCHAIN_SHA)),
                patch("network_control._run"),
                patch("network_control._write_record", side_effect=OSError("disk full")),
                patch("network_control._cleanup_if_owned") as cleanup,
            ):
                with self.assertRaises(OSError):
                    apply_profile(
                        "tngt-case-1", "tngt0", profile_id, Path(directory) / "apply.json"
                    )
            cleanup.assert_called_once()
            self.assertEqual(cleanup.call_args.args[4], _expected_netem(profile_id))

    def test_reset_requires_the_same_namespace_interface_and_qdisc(self) -> None:
        profile_id = "lan-1g-1ms-v1"
        with tempfile.TemporaryDirectory(prefix="tng-network-control-") as directory:
            applied_path = Path(directory) / "apply.json"
            output = Path(directory) / "reset.json"
            record = {
                "format": "transfer-network-control-v1",
                "action": "apply",
                "created_at": "2026-10-17T18:00:00Z",
                "namespace_name": "tngt-case-1",
                "namespace_device": 8,
                "namespace_inode": 9,
                "interface": "tngt0",
                "ifindex": 17,
                "profile_id": profile_id,
                "settings": dict(PROFILES[profile_id]),
                "baseline_root_qdisc": {"kind": "noqueue", "handle": "0:"},
                "observed_root_qdisc": _expected_netem(profile_id),
                "ip_version": "ip v1",
                "tc_version": "tc v1",
                "toolchain_sha256": TOOLCHAIN_SHA,
                "applied_record_sha256": None,
            }
            record["artifact_sha256"] = _record_digest(record)
            applied_path.write_text(json.dumps(record), encoding="utf-8")
            with (
                patch("network_control._tools", return_value=("/sbin/ip", "/sbin/tc")),
                patch("network_control._namespace_identity", return_value={"device": 8, "inode": 9}),
                patch("network_control._link_info", return_value=link_info()),
                patch(
                    "network_control._root_qdisc",
                    side_effect=[netem_root(profile_id), {"kind": "noqueue", "handle": "0:"}],
                ),
                patch("network_control._toolchain_identity", return_value=("ip v1", "tc v1", TOOLCHAIN_SHA)),
                patch("network_control._run") as run,
            ):
                reset = reset_profile(applied_path, output)

            self.assertIn("del", run.call_args.args[0])
            self.assertEqual(reset["action"], "reset")
            self.assertEqual(reset["applied_record_sha256"], hashlib.sha256(applied_path.read_bytes()).hexdigest())

    def test_reset_refuses_a_replaced_interface_before_mutation(self) -> None:
        profile_id = "lan-1g-1ms-v1"
        with tempfile.TemporaryDirectory(prefix="tng-network-control-") as directory:
            applied_path = Path(directory) / "apply.json"
            record = {
                "format": "transfer-network-control-v1",
                "action": "apply",
                "created_at": "2026-10-17T18:00:00Z",
                "namespace_name": "tngt-case-1",
                "namespace_device": 8,
                "namespace_inode": 9,
                "interface": "tngt0",
                "ifindex": 17,
                "profile_id": profile_id,
                "settings": dict(PROFILES[profile_id]),
                "baseline_root_qdisc": {"kind": "noqueue", "handle": "0:"},
                "observed_root_qdisc": _expected_netem(profile_id),
                "ip_version": "ip v1",
                "tc_version": "tc v1",
                "toolchain_sha256": TOOLCHAIN_SHA,
                "applied_record_sha256": None,
            }
            record["artifact_sha256"] = _record_digest(record)
            applied_path.write_text(json.dumps(record), encoding="utf-8")
            with (
                patch("network_control._tools", return_value=("/sbin/ip", "/sbin/tc")),
                patch("network_control._namespace_identity", return_value={"device": 8, "inode": 9}),
                patch("network_control._link_info", return_value=link_info(ifindex=99)),
                patch("network_control._run") as run,
            ):
                with self.assertRaises(NetworkControlError):
                    reset_profile(applied_path, Path(directory) / "reset.json")
            run.assert_not_called()


def stat_mode(path: Path) -> int:
    return os.stat(path, follow_symlinks=False).st_mode & 0o777


if __name__ == "__main__":
    unittest.main()
