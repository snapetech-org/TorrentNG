"""Regression sources for PERF-110 task-owned network isolation."""

from __future__ import annotations

import sys
import unittest
from pathlib import Path
from unittest.mock import call, patch

TOOLS = Path(__file__).parent / "transfer_rebuild"
sys.path.insert(0, str(TOOLS))

import task_network  # noqa: E402
import validate_artifact  # noqa: E402


def _record() -> dict[str, object]:
    topology = task_network.TaskNetwork(
        suffix="a1b2c3d4",
        target_namespace="tngt-a1b2c3d4",
        seed_namespace="tngs-a1b2c3d4",
        bridge="tngba1b2c3d4",
        bridge_ifindex=20,
        subnet="198.18.0.0/24",
        gateway="198.18.0.1",
        target_address="198.18.0.3",
        seed_address="198.18.0.2",
        target_interface="tngveth0",
        target_ifindex=30,
        seed_interface="tngveth0",
        seed_ifindex=40,
        target_host_interface="tngha1b2c3d4",
        target_host_ifindex=50,
        seed_host_interface="tngsa1b2c3d4",
        seed_host_ifindex=60,
        target_namespace_identity={"device": 1, "inode": 100},
        seed_namespace_identity={"device": 1, "inode": 200},
    )
    return topology.as_record()


class TaskNetworkTests(unittest.TestCase):
    def test_task_names_and_namespace_identity_records_are_strict(self) -> None:
        for suffix in ("a1b2c3d4", "deadbeef01"):
            self.assertIsNotNone(task_network.SUFFIX_RE.fullmatch(suffix))
        for suffix in ("../../xx", "ABCDEF12", "short", "g1234567"):
            self.assertIsNone(task_network.SUFFIX_RE.fullmatch(suffix))
        self.assertTrue(task_network._namespace_identity_record({"device": 0, "inode": 1}))
        self.assertFalse(task_network._namespace_identity_record({"device": True, "inode": 1}))
        self.assertFalse(task_network._namespace_identity_record({"device": 1, "inode": 0}))

    def test_api_contexts_are_derived_from_the_digest_bound_topology(self) -> None:
        record = _record()
        target = task_network.client_api_context(record, "target")
        seed = task_network.client_api_context(record, "seed")
        self.assertEqual((target["address"], target["namespace"]), ("198.18.0.3", "tngt-a1b2c3d4"))
        self.assertEqual((seed["address"], seed["namespace"]), ("198.18.0.2", "tngs-a1b2c3d4"))
        record["target"]["address"] = "198.18.0.9"  # type: ignore[index]
        with self.assertRaisesRegex(task_network.TaskNetworkError, "invalid task topology"):
            task_network.client_api_context(record, "target")

    def test_validator_checks_the_self_digest_and_reserved_network_geometry(self) -> None:
        record = _record()
        self.assertEqual(validate_artifact._task_network_errors(record), [])
        changed = dict(record, gateway="198.18.0.254")
        changed["artifact_sha256"] = task_network._record_sha256(changed)
        self.assertIn(
            "gateway does not match the task-network subnet",
            validate_artifact._task_network_errors(changed),
        )

    def test_topology_can_attach_both_isolated_container_namespaces(self) -> None:
        identities = [
            {"device": 1, "inode": 100},
            {"device": 1, "inode": 200},
        ]
        link_indexes = iter([30, 40, 20])
        host_indexes = iter([50, 60])
        with (
            patch.object(task_network.os, "geteuid", return_value=0),
            patch.object(task_network, "_ip", return_value="/sbin/ip"),
            patch.object(
                task_network,
                "_choose_subnet",
                return_value=task_network.ipaddress.ip_network("198.18.0.0/24"),
            ),
            patch.object(task_network, "_namespace_identity", side_effect=identities),
            patch.object(task_network, "_link_index", side_effect=lambda *_args: next(link_indexes)),
            patch.object(task_network, "_host_veth_index", side_effect=lambda *_args: next(host_indexes)),
            patch.object(task_network, "_assert_isolated_namespace") as isolated,
            patch.object(task_network, "_run", return_value="") as run,
        ):
            topology = task_network.create_topology(
                "a1b2c3d4",
                target_pid=101,
                seed_pid=202,
            )

        self.assertEqual(topology.target_namespace_identity, identities[0])
        self.assertEqual(topology.seed_namespace_identity, identities[1])
        self.assertEqual(isolated.call_count, 2)
        run.assert_any_call(["/sbin/ip", "netns", "attach", "tngt-a1b2c3d4", "101"])
        run.assert_any_call(["/sbin/ip", "netns", "attach", "tngs-a1b2c3d4", "202"])

    def test_topology_rejects_one_process_for_both_attached_roles(self) -> None:
        with (
            patch.object(task_network.os, "geteuid", return_value=0),
            patch.object(task_network, "_ip") as ip,
        ):
            with self.assertRaisesRegex(task_network.TaskNetworkError, "separate processes"):
                task_network.create_topology(
                    "a1b2c3d4",
                    target_pid=101,
                    seed_pid=101,
                )
        ip.assert_not_called()

    def test_cleanup_checks_all_recorded_identities_and_removes_only_task_links(self) -> None:
        record = _record()
        namespaces = [
            record["target"]["namespace_identity"],  # type: ignore[index]
            record["seed"]["namespace_identity"],  # type: ignore[index]
        ]
        host_indexes = [50, 60]
        link_indexes = [20, 30, 40]
        with (
            patch.object(task_network.os, "geteuid", return_value=0),
            patch.object(task_network, "_ip", return_value="/sbin/ip"),
            patch.object(task_network, "_namespace_identity", side_effect=namespaces),
            patch.object(task_network, "_host_veth_index", side_effect=host_indexes),
            patch.object(task_network, "_link_index", side_effect=link_indexes),
            patch.object(task_network, "_run", return_value="") as run,
        ):
            result = task_network.cleanup_topology_record(record)

        self.assertTrue(result["cleaned"])
        self.assertEqual(result["topology_sha256"], record["artifact_sha256"])
        self.assertIn(call(["/sbin/ip", "netns", "pids", "tngs-a1b2c3d4"]), run.call_args_list)
        self.assertNotIn(call(["/sbin/ip", "netns", "pids", "tngt-a1b2c3d4"]), run.call_args_list)
        self.assertIn(call(["/sbin/ip", "-n", "tngt-a1b2c3d4", "link", "del", "tngveth0"]), run.call_args_list)

    def test_cleanup_rejects_a_modified_or_misnamed_record_before_network_access(self) -> None:
        record = _record()
        record["bridge"] = "tngbdeadbeef"
        with (
            patch.object(task_network.os, "geteuid", return_value=0),
            patch.object(task_network, "_ip") as ip,
        ):
            with self.assertRaisesRegex(task_network.TaskNetworkError, "digest"):
                task_network.cleanup_topology_record(record)
        ip.assert_not_called()


if __name__ == "__main__":
    unittest.main()
