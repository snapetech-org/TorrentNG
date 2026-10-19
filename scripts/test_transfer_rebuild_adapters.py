"""Regression sources for pinned transfer benchmark client adapters."""

from __future__ import annotations

import hashlib
import json
import os
import sys
import tempfile
import unittest
import urllib.parse
from types import SimpleNamespace
from unittest.mock import patch
from pathlib import Path

sys.path.insert(0, str(__file__).rsplit("/", 1)[0] + "/transfer_rebuild")
from adapters import (  # noqa: E402
    AdapterError,
    HttpJson,
    _assert_native_process_identity,
    _binary_identity,
    _docker_identity,
    _login_qbittorrent,
    _profile_http,
    _redact,
    _sha256_regular_file_nofollow,
    _tcp_listener_addresses,
    _tcp_listener_inodes,
    _task_local_api_context,
    _urtorrent_settings_file,
    _verify_container_http_endpoint,
    load_pins,
    probe_client,
)


class TransferClientAdapterTests(unittest.TestCase):
    @staticmethod
    def urtorrent_isolated_settings() -> dict[str, object]:
        return {
            "session": {
                "dht": False,
                "pex": False,
                "lsd": False,
                "listen_v4": "10.77.0.1",
                "listen_v6": None,
                "listen_port": 45678,
            },
            "torrent": {"private": True},
            "control": {
                "dht_bootstrap_nodes": [],
                "peer_exchange": False,
                "local_service_discovery": False,
                "trackers_after_isolation": 0,
                "web_seeds_after_isolation": 0,
                "manual_peer": "127.0.0.1:1234",
                "start_barrier": True,
            },
            "runner_inputs": {
                "mode": "download",
                "timeout_seconds": 1800,
                "seed_seconds": 60,
                "hash_threads": 2,
                "hash_threads_source": "command_line",
            },
        }

    def test_http_json_rejects_duplicate_fields_and_overflow_numbers(self) -> None:
        client = HttpJson("http://127.0.0.1:1")
        for body in (
            b'{"settings":{"dht":false,"dht":true}}',
            b'{"download_rate":1e999}',
            b'{"download_rate":NaN}',
        ):
            with self.subTest(body=body), patch.object(client, "request", return_value=body):
                with self.assertRaises(AdapterError):
                    client.json("/api/settings")

    def test_task_local_api_context_is_bound_to_the_recorded_private_subnet(self) -> None:
        digest = "a" * 64
        context = {
            "address": "198.18.0.3",
            "namespace": "tngt-a1b2c3d4",
            "namespace_identity": {"device": 1, "inode": 2},
            "subnet": "198.18.0.0/24",
            "interface": "tngveth0",
            "ifindex": 9,
            "topology_sha256": digest,
        }
        self.assertEqual(_task_local_api_context({"_task_local_api": context}), context)
        invalid = dict(context, address="192.168.1.3")
        with self.assertRaisesRegex(AdapterError, "does not match"):
            _task_local_api_context({"_task_local_api": invalid})
        invalid_identity = dict(context, namespace_identity={"device": True, "inode": 2})
        with self.assertRaisesRegex(AdapterError, "invalid namespace"):
            _task_local_api_context({"_task_local_api": invalid_identity})

    def test_native_process_identity_detects_reexec_of_same_pid(self) -> None:
        identity = {
            "process_start_ticks": 777,
            "process_exe_device": 11,
            "process_exe_inode": 22,
        }
        with (
            patch("adapters._process_start_ticks", return_value=777),
            patch("adapters.os.stat", return_value=SimpleNamespace(st_dev=11, st_ino=22)),
        ):
            _assert_native_process_identity(12345, identity)

        with (
            patch("adapters._process_start_ticks", return_value=777),
            patch("adapters.os.stat", return_value=SimpleNamespace(st_dev=11, st_ino=23)),
        ):
            with self.assertRaises(AdapterError):
                _assert_native_process_identity(12345, identity)

    def test_native_binary_digest_uses_a_regular_non_symlink_file(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-client-binary-") as directory:
            root = Path(directory)
            binary = root / "torrent-client"
            binary.write_bytes(b"pinned executable")
            link = root / "torrent-client-link"
            link.symlink_to(binary)

            digest, identity = _sha256_regular_file_nofollow(binary, "native client binary")

            self.assertEqual(digest, hashlib.sha256(b"pinned executable").hexdigest())
            self.assertEqual(identity.st_ino, binary.stat().st_ino)
            with self.assertRaises(AdapterError):
                _sha256_regular_file_nofollow(link, "native client binary")

    def test_native_source_identity_uses_the_package_toolchain_and_lockfile(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-native-source-") as directory:
            root = Path(directory)
            package = root / "runner"
            package.mkdir()
            binary = root / "runner-bin"
            binary.write_bytes(b"pinned runner binary")
            binary_identity = binary.stat()
            pid_file = root / "runner.pid"
            pid_file.write_text("123\n", encoding="ascii")
            lockfile = package / "Cargo.lock"
            lockfile_bytes = b"runner-specific dependency lock"
            lockfile.write_bytes(lockfile_bytes)
            expected_toolchain = "rustc 1.98.1\nhost: test-target"

            profile = {
                "binary_path": str(binary),
                "expected_binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                "pid_file": str(pid_file),
                "source_root": str(package),
                "expected_source_commit": "a" * 40,
                "expected_dirty_worktree": True,
                "expected_lockfile_sha256": hashlib.sha256(lockfile_bytes).hexdigest(),
                "expected_toolchain": expected_toolchain,
            }
            original_stat = os.stat

            def stat_path(path: str, *args: object, **kwargs: object) -> object:
                if path == "/proc/123/exe":
                    return SimpleNamespace(
                        st_dev=binary_identity.st_dev,
                        st_ino=binary_identity.st_ino,
                    )
                return original_stat(path, *args, **kwargs)

            def subprocess_run(command: list[str], **kwargs: object) -> SimpleNamespace:
                if command[:3] == ["git", "-C", str(package)]:
                    if command[3] == "rev-parse":
                        return SimpleNamespace(stdout="a" * 40 + "\n")
                    if command[3] == "status":
                        return SimpleNamespace(stdout=" M src/main.rs\n")
                if command == ["rustc", "-Vv"]:
                    self.assertEqual(kwargs.get("cwd"), package)
                    return SimpleNamespace(stdout=expected_toolchain)
                raise AssertionError(f"unexpected subprocess: {command}")

            with (
                patch("adapters.subprocess.run", side_effect=subprocess_run),
                patch("adapters.os.stat", side_effect=stat_path),
                patch("adapters._process_start_ticks", return_value=777),
                patch("adapters._assert_native_process_identity"),
            ):
                identity = _binary_identity(profile)

            self.assertEqual(identity["lockfile_sha256"], profile["expected_lockfile_sha256"])
            self.assertEqual(identity["toolchain"], expected_toolchain)

    def test_qbittorrent_login_accepts_its_plain_text_success_response(self) -> None:
        client = HttpJson("http://127.0.0.1:8080")
        profile = {"username_env": "TNG_TEST_QBIT_USER", "password_env": "TNG_TEST_QBIT_PASS"}
        environment = {
            "TNG_TEST_QBIT_USER": "benchmark",
            "TNG_TEST_QBIT_PASS": "secret-value",
        }
        with patch.dict(os.environ, environment), patch.object(HttpJson, "request", return_value=b"Ok."):
            _login_qbittorrent(client, profile)

    def test_http_profile_rejects_credentials_in_url_and_does_not_send_api_login_as_basic(self) -> None:
        with self.assertRaises(AdapterError):
            _profile_http({"base_url": "http://user:password@example.test"})
        with self.assertRaises(AdapterError):
            _profile_http({"base_url": "http://example.test/?token=secret"})

        profile = {
            "base_url": "http://127.0.0.1:8080",
            "username_env": "TNG_TEST_DELUGE_USER",
            "password_env": "TNG_TEST_DELUGE_PASS",
        }
        with patch.dict(os.environ, {
            "TNG_TEST_DELUGE_USER": "benchmark",
            "TNG_TEST_DELUGE_PASS": "secret-value",
        }):
            client = _profile_http(profile)
        self.assertNotIn("Authorization", client.headers)

    def test_settings_redaction_covers_secret_keys_urls_and_authorization_values(self) -> None:
        source = {
            "password": "password-value",
            "tracker_url": "https://alice:secret@example.test/announce?token=query-secret",
            "headers": ["Bearer bearer-secret"],
            "nested": [{"apiKey": "api-secret"}],
        }

        redacted = _redact(source)
        serialized = json.dumps(redacted)

        for secret in ("password-value", "secret@example", "query-secret", "bearer-secret", "api-secret"):
            self.assertNotIn(secret, serialized)
        self.assertEqual(redacted["password"], "<redacted>")

    def test_proc_tcp_listener_parser_uses_inode_and_listen_state(self) -> None:
        header = "sl local_address rem_address st tx_queue rx_queue tr tm->when retrnsmt uid timeout inode"
        row = "0: 0100007F:1F90 00000000:0000 0A 0 0 00:00000000 00000000 0 1000 0 123456"
        closed_row = "1: 0100007F:1F90 00000000:0000 01 0 0 00:00000000 00000000 0 1000 0 999999"

        self.assertEqual(_tcp_listener_inodes(f"{header}\n{row}\n{closed_row}", 8080), {"123456"})
        listeners = _tcp_listener_addresses(f"{header}\n{row}\n{closed_row}", 8080)
        self.assertEqual(
            [(str(address), inode) for address, inode in listeners],
            [("127.0.0.1", "123456")],
        )

    def test_container_http_binding_must_publish_only_on_loopback(self) -> None:
        container_id = "a" * 64
        pid = 12345
        profile = {"container": "qbittorrent"}
        identity = {"container_id": container_id, "process_id": pid, "process_start_ticks": 777}

        def inspect(host_ips: list[str]) -> SimpleNamespace:
            record = {
                "Id": f"sha256:{container_id}",
                "State": {"Running": True, "Pid": pid},
                "HostConfig": {"NetworkMode": "bridge"},
                "NetworkSettings": {
                    "Ports": {
                        "8080/tcp": [
                            {"HostIp": host_ip, "HostPort": str(8080 + index)}
                            for index, host_ip in enumerate(host_ips)
                        ]
                    }
                },
            }
            return SimpleNamespace(stdout=json.dumps(record))

        parsed = urllib.parse.urlsplit("http://127.0.0.1:8080")
        with (
            patch("adapters.subprocess.run", side_effect=lambda *args, **kwargs: inspect(["0.0.0.0"])),
            patch("adapters._process_start_ticks", return_value=777),
            patch("adapters._namespace_has_listener", return_value=True),
        ):
            with self.assertRaises(AdapterError):
                _verify_container_http_endpoint(profile, identity, parsed)

        with (
            patch("adapters.subprocess.run", side_effect=lambda *args, **kwargs: inspect(["127.0.0.1"])),
            patch("adapters._process_start_ticks", return_value=777),
            patch("adapters._namespace_has_listener", return_value=True),
        ):
            _verify_container_http_endpoint(profile, identity, parsed)

        with (
            patch(
                "adapters.subprocess.run",
                side_effect=lambda *args, **kwargs: inspect(["127.0.0.1", "0.0.0.0"]),
            ),
            patch("adapters._process_start_ticks", return_value=777),
            patch("adapters._namespace_has_listener", return_value=True),
        ):
            with self.assertRaises(AdapterError):
                _verify_container_http_endpoint(profile, identity, parsed)

    def test_host_network_container_api_requires_loopback_listener(self) -> None:
        container_id = "a" * 64
        pid = 12345
        inspected = {
            "Id": f"sha256:{container_id}",
            "State": {"Running": True, "Pid": pid},
            "HostConfig": {"NetworkMode": "host"},
            "NetworkSettings": {"Ports": {}},
        }
        identity = {"container_id": container_id, "process_id": pid, "process_start_ticks": 777}
        with (
            patch("adapters.subprocess.run", return_value=SimpleNamespace(stdout=json.dumps(inspected))),
            patch("adapters._process_start_ticks", return_value=777),
            patch("adapters._namespace_has_listener", return_value=False) as listener_check,
        ):
            with self.assertRaises(AdapterError):
                _verify_container_http_endpoint(
                    {"container": "qbittorrent"},
                    identity,
                    urllib.parse.urlsplit("http://127.0.0.1:8080"),
                )
        listener_check.assert_called_once_with(pid, 8080, loopback_only=True)

    def test_pin_file_uses_a_distinct_schema_version_and_requires_image_digest(self) -> None:
        profile = {
            "schema_version": "transfer-client-pins-v1",
            "clients": {
                "qbittorrent": {
                    "runtime": "container",
                    "container": "qbittorrent",
                    "image_ref": "example.test/qbittorrent@sha256:" + "d" * 64,
                    "base_url": "http://127.0.0.1:8080",
                    "expected_version": "5.0.0",
                    "expected_settings": {},
                }
            },
        }
        with tempfile.TemporaryDirectory(prefix="tng-client-pins-") as directory:
            path = Path(directory) / "pins.json"
            path.write_text(json.dumps(profile), encoding="utf-8")
            self.assertIn("qbittorrent", load_pins(path))

            profile["schema_version"] = "transfer-client-observation-v1"
            path.write_text(json.dumps(profile), encoding="utf-8")
            with self.assertRaises(AdapterError):
                load_pins(path)

    def test_urtorrent_native_profile_requires_bound_settings_readback(self) -> None:
        profile = {
            "schema_version": "transfer-client-pins-v1",
            "clients": {
                "urtorrent": {
                    "runtime": "native_binary",
                    "binary_path": "/task/bin/urtorrent-runner",
                    "expected_binary_sha256": "a" * 64,
                    "pid_file": "/task/run/urtorrent.pid",
                    "settings_file": "/task/run/urtorrent-settings.json",
                    "expected_version": "0.14.3",
                    "expected_settings": {
                        "settings": {
                            "session": {
                                "dht": False,
                                "pex": False,
                                "lsd": False,
                                "listen_v4": "10.77.0.1",
                                "listen_v6": None,
                            },
                            "torrent": {"private": True},
                            "control": {
                                "dht_bootstrap_nodes": [],
                                "peer_exchange": False,
                                "local_service_discovery": False,
                                "trackers_after_isolation": 0,
                                "web_seeds_after_isolation": 0,
                                "manual_peer": "10.77.0.2:45679",
                                "start_barrier": True,
                            },
                        }
                    },
                }
            },
        }
        with tempfile.TemporaryDirectory(prefix="tng-client-pins-") as directory:
            path = Path(directory) / "pins.json"
            path.write_text(json.dumps(profile), encoding="utf-8")
            self.assertIn("urtorrent", load_pins(path))

            profile["clients"]["urtorrent"]["expected_settings"]["settings"]["session"]["pex"] = True
            path.write_text(json.dumps(profile), encoding="utf-8")
            with self.assertRaises(AdapterError):
                load_pins(path)

            expected = profile["clients"]["urtorrent"]["expected_settings"]["settings"]
            expected["session"]["pex"] = False
            expected["session"]["listen_v4"] = "8.8.8.8"
            path.write_text(json.dumps(profile), encoding="utf-8")
            with self.assertRaises(AdapterError):
                load_pins(path)

            expected["session"]["listen_v4"] = "10.77.0.1"
            expected["control"]["manual_peer"] = "8.8.8.8:45679"
            path.write_text(json.dumps(profile), encoding="utf-8")
            with self.assertRaises(AdapterError):
                load_pins(path)

            expected["control"]["manual_peer"] = "10.77.0.2:45679"
            profile["clients"]["urtorrent"]["runtime"] = "container"
            profile["clients"]["urtorrent"]["container"] = "urtorrent"
            profile["clients"]["urtorrent"]["image_ref"] = "example.test/urtorrent@sha256:" + "b" * 64
            path.write_text(json.dumps(profile), encoding="utf-8")
            with self.assertRaises(AdapterError):
                load_pins(path)

    def test_urtorrent_readback_must_belong_to_the_live_process_instance(self) -> None:
        identity = {"process_id": 456, "process_start_ticks": 789}
        profile = {"settings_file": "/task/run/settings.json"}
        snapshot = {
            "format": "transfer-urtorrent-settings-v1",
            "version": "0.14.3",
            "process_id": 456,
            "process_start_ticks": 789,
            "settings": self.urtorrent_isolated_settings(),
        }
        with (
            patch("adapters._assert_native_process_identity") as identity_check,
            patch("adapters._read_regular_file_nofollow", return_value=json.dumps(snapshot).encode()),
        ):
            version, settings = _urtorrent_settings_file(profile, identity)
        self.assertEqual(version, "0.14.3")
        self.assertEqual(settings, self.urtorrent_isolated_settings())
        self.assertEqual(identity_check.call_count, 2)

        snapshot["settings"]["control"]["trackers_after_isolation"] = 1
        with (
            patch("adapters._assert_native_process_identity"),
            patch("adapters._read_regular_file_nofollow", return_value=json.dumps(snapshot).encode()),
        ):
            with self.assertRaises(AdapterError):
                _urtorrent_settings_file(profile, identity)

        snapshot["process_start_ticks"] = 790
        with (
            patch("adapters._assert_native_process_identity"),
            patch("adapters._read_regular_file_nofollow", return_value=json.dumps(snapshot).encode()),
        ):
            with self.assertRaises(AdapterError):
                _urtorrent_settings_file(profile, identity)

        snapshot["process_start_ticks"] = 789
        snapshot["process_id"] = True
        with (
            patch("adapters._assert_native_process_identity"),
            patch("adapters._read_regular_file_nofollow", return_value=json.dumps(snapshot).encode()),
        ):
            with self.assertRaises(AdapterError):
                _urtorrent_settings_file(profile, {"process_id": 1, "process_start_ticks": 789})

    def test_urtorrent_probe_compares_the_nested_settings_pin(self) -> None:
        settings = self.urtorrent_isolated_settings()
        profile = {
            "runtime": "native_binary",
            "expected_version": "0.14.3",
            "expected_settings": {"settings": settings},
        }
        with (
            patch("adapters._binary_identity", return_value={"process_id": 456}),
            patch("adapters._probe_api", return_value=("0.14.3", settings)),
        ):
            observation = probe_client("urtorrent", profile)

        self.assertEqual(observation["client_id"], "urtorrent")
        self.assertEqual(observation["settings_readback"]["values"], settings)

    def test_urtorrent_readback_rejects_public_listener_and_peer(self) -> None:
        identity = {"process_id": 456, "process_start_ticks": 789}
        profile = {"settings_file": "/task/run/settings.json"}
        snapshot = {
            "format": "transfer-urtorrent-settings-v1",
            "version": "0.14.3",
            "process_id": 456,
            "process_start_ticks": 789,
            "settings": self.urtorrent_isolated_settings(),
        }
        snapshot["settings"]["session"]["listen_v4"] = "198.18.1.3"
        snapshot["settings"]["control"]["manual_peer"] = "198.19.254.2:1234"
        with (
            patch("adapters._assert_native_process_identity"),
            patch("adapters._read_regular_file_nofollow", return_value=json.dumps(snapshot).encode()),
        ):
            _urtorrent_settings_file(profile, identity)

        snapshot["settings"] = self.urtorrent_isolated_settings()
        for section, key, value in (
            ("session", "listen_v4", "8.8.8.8"),
            ("control", "manual_peer", "8.8.8.8:1234"),
            ("control", "manual_peer", "[::1]:1234"),
        ):
            snapshot["settings"][section][key] = value
            with (
                patch("adapters._assert_native_process_identity"),
                patch("adapters._read_regular_file_nofollow", return_value=json.dumps(snapshot).encode()),
            ):
                with self.assertRaises(AdapterError):
                    _urtorrent_settings_file(profile, identity)
            snapshot["settings"] = self.urtorrent_isolated_settings()

    def test_container_observation_binds_command_without_recording_secret_arguments(self) -> None:
        container_id = "a" * 64
        image_id = "b" * 64
        image_ref = "example.test/rqbit@sha256:" + "c" * 64
        pid = 12345
        outputs = [
            f"sha256:{container_id} sha256:{image_id} true {pid}\n",
            json.dumps([image_ref]),
            json.dumps({
                "State": {"Running": True, "Pid": pid},
                "Path": "/usr/bin/rqbit",
                "Args": ["--token", "command-secret"],
                "Config": {"Env": ["PATH=/usr/bin"]},
                "Mounts": [],
            }),
        ]
        completed = [SimpleNamespace(stdout=output) for output in outputs]
        with patch("adapters.subprocess.run", side_effect=completed), patch(
            "adapters._process_start_ticks", return_value=777
        ):
            identity = _docker_identity(
                {"container": "rqbit", "image_ref": image_ref}, capture_command=True
            )

        self.assertEqual(identity["container_id"], container_id)
        self.assertEqual(identity["image_id"], image_id)
        self.assertEqual(identity["process_id"], pid)
        self.assertEqual(identity["process_start_ticks"], 777)
        self.assertEqual(identity["effective_argv_count"], 3)
        self.assertNotIn("effective_argv", identity)
        self.assertNotIn("command-secret", json.dumps(identity))


if __name__ == "__main__":
    unittest.main()
