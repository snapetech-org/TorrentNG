"""Regression sources for PERF-110's local paired-runner invariants."""

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
from client_control import TorrentState  # noqa: E402
from cache_control import _reuse_verified_fixture_manifest  # noqa: E402
from fixtures import FixtureFile, generate_fixture, verify_fixture  # noqa: E402
from paired_runner import (  # noqa: E402
    Endpoint,
    PairedRunnerError,
    RuntimePaths,
    DEFAULT_PROFILE_PATH,
    _aggregate_payload_digest,
    _candidate_info_hash,
    _environment_for_trial,
    _fixture_file_identities,
    _fixture_matches_identities,
    _fixture_for_plan,
    _load_json,
    _network_seed,
    _profile_listener_ports,
    _scheduled_trial_key,
    _sha256_file,
    _seed_payload,
    _start_urtorrent,
    _stage_input,
    _urtorrent_profile_for_trial,
    _validate_payload,
    _wait_for_observer_sample,
    _wait_for_transfer,
)


class _StatusSequence:
    def __init__(self, states: list[TorrentState]) -> None:
        self.states = states
        self.calls = 0

    def status(self, _info_hash: str) -> TorrentState:
        index = min(self.calls, len(self.states) - 1)
        self.calls += 1
        return self.states[index]


class _LiveProcess:
    def poll(self) -> None:
        return None


def _state(*, received: int, complete: bool = False) -> TorrentState:
    return TorrentState(
        info_hash="a" * 40,
        state="complete" if complete else "downloading",
        progress=1.0 if complete else received / 200,
        total_bytes=200,
        received_bytes=received,
        uploaded_bytes=0,
        download_rate_bps=0,
        upload_rate_bps=0,
        peers=1,
    )


class PairedRunnerTests(unittest.TestCase):
    def test_scheduled_trial_key_accepts_catalog_profile_and_repetition_ids(self) -> None:
        trial_id = "c5a1e354ae46fc313dc8:N1/lan_1g:rep-001:torrentngd:attempt-001"

        key = _scheduled_trial_key(trial_id, "download")

        self.assertEqual(
            key,
            hashlib.sha256(f"{trial_id}:download".encode("utf-8")).hexdigest()[:20],
        )
        self.assertRegex(key, r"^[a-f0-9]{20}$")

    def test_scheduled_trial_key_rejects_unsupported_characters(self) -> None:
        with self.assertRaisesRegex(PairedRunnerError, "scheduled trial identity"):
            _scheduled_trial_key("trial with spaces", "download")

    def test_profile_listener_ports_cover_api_peer_and_direct_runner_ports(self) -> None:
        profile = {
            "paired_launch": {"api_port": 18080, "peer_port": 44444},
            "expected_settings": {"settings": {"session": {"listen_port": 51413}}},
        }
        self.assertEqual(_profile_listener_ports(profile), {18080, 44444, 51413})

    def test_fixture_generation_uses_the_pinned_profile_file(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-profile-") as directory:
            root = Path(directory)
            profiles = json.loads(DEFAULT_PROFILE_PATH.read_text(encoding="utf-8"))
            profiles["profiles"]["N1/lan_1g"]["files"][0]["size_bytes"] = 64 * 1024
            profile_path = root / "profiles.json"
            profile_path.write_text(json.dumps(profiles), encoding="utf-8")
            (root / "fixtures").mkdir(mode=0o700)
            plan = {"plan_id": "scoped-test-plan"}
            planned = {
                "pair_id": "scoped-test-pair",
                "profile_id": "N1/lan_1g",
                "repetition": 1,
            }

            fixture_dir, manifest = _fixture_for_plan(root, plan, planned, profile_path)

            self.assertEqual(manifest["files"][0]["size_bytes"], 64 * 1024)
            self.assertTrue((fixture_dir / "fixture.torrent").is_file())

    def test_urtorrent_profile_binds_dynamic_peer_addresses_and_workload_mode(self) -> None:
        profile = {
            "expected_settings": {
                "settings": {
                    "session": {"listen_v4": "127.0.0.1", "dht": False},
                    "control": {"pinned_peer": "127.0.0.1:1234"},
                    "runner_inputs": {"mode": "download", "hash_threads": 2},
                }
            }
        }

        trial = _urtorrent_profile_for_trial(
            profile,
            "198.18.1.3",
            "198.18.1.2:44444",
            "http://198.18.1.2:45555/announce",
            "seed",
        )

        settings = trial["expected_settings"]["settings"]
        self.assertEqual(settings["session"]["listen_v4"], "198.18.1.3")
        self.assertEqual(settings["control"]["pinned_peer"], "198.18.1.2:44444")
        self.assertEqual(settings["control"]["tracker_url"], "http://198.18.1.2:45555/announce")
        self.assertEqual(settings["control"]["trackers_after_isolation"], 1)
        self.assertEqual(settings["runner_inputs"]["mode"], "seed")
        self.assertEqual(profile["expected_settings"]["settings"]["session"]["listen_v4"], "127.0.0.1")
        self.assertEqual(profile["expected_settings"]["settings"]["runner_inputs"]["mode"], "download")

    def test_urtorrent_trial_profile_rejects_unknown_mode(self) -> None:
        with self.assertRaisesRegex(PairedRunnerError, "mode"):
            _urtorrent_profile_for_trial(
                {}, "127.0.0.1", "127.0.0.1:1", "http://127.0.0.1:2/announce", "upload"
            )

    def test_urtorrent_start_reads_nested_settings_pin_and_keeps_task_tracker(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-urtorrent-start-") as directory:
            root = Path(directory)
            data = root / "data"
            state = root / "state"
            data.mkdir()
            state.mkdir()
            tracker_url = "http://198.18.2.2:45555/announce"
            profile = {
                "binary_path": "/task/bin/urtorrent-runner",
                "expected_binary_sha256": "a" * 64,
                "preflight_timeout_seconds": 30,
                "expected_settings": {
                    "settings": {
                        "session": {"max_connections": 40, "max_peers_per_torrent": 12},
                        "torrent": {"private": True},
                        "control": {"pinned_peer": "127.0.0.1:1234"},
                        "runner_inputs": {"hash_threads": 3},
                    }
                },
            }
            endpoint = Endpoint(
                "target",
                "urtorrent",
                profile,
                RuntimePaths(
                    root=root,
                    data=data,
                    state=state,
                    log=root / "client.log",
                    pid=state / "launcher.pid",
                    env=state / "client.env",
                    tracker_url=tracker_url,
                    peer_address="198.18.2.2:44444",
                ),
            )
            topology = {
                "target": {
                    "address": "198.18.2.3",
                    "namespace_identity": {"device": 1, "inode": 2},
                },
                "target_namespace": "tngt-test-run",
            }
            with patch("paired_runner.ManagedProcess.start") as start:
                _start_urtorrent(
                    endpoint,
                    topology,
                    root / "fixture.torrent",
                    "198.18.2.2:44444",
                    "download",
                    root / "barriers",
                    120,
                )

        args = start.call_args.args[0]
        self.assertIn("--tracker-url", args)
        self.assertEqual(args[args.index("--tracker-url") + 1], tracker_url)
        self.assertEqual(args[args.index("--max-connections") + 1], "40")
        self.assertEqual(args[args.index("--max-peers") + 1], "12")
        self.assertEqual(args[args.index("--hash-threads") + 1], "3")
        settings = endpoint.direct_profile["expected_settings"]["settings"]
        self.assertEqual(settings["control"]["pinned_peer"], "198.18.2.2:44444")
        self.assertEqual(settings["control"]["tracker_url"], tracker_url)
        self.assertEqual(settings["control"]["trackers_after_isolation"], 1)

    def test_runner_input_json_rejects_overflow_numbers(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-json-") as directory:
            artifact = Path(directory) / "plan.json"
            artifact.write_text('{"duration_seconds":1e999}', encoding="utf-8")

            with self.assertRaisesRegex(PairedRunnerError, "outside the finite range"):
                _load_json(artifact, "paired plan")

    def test_runner_input_json_rejects_nested_duplicate_fields(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-json-") as directory:
            artifact = Path(directory) / "plan.json"
            artifact.write_text('{"profile":{"id":"first","id":"second"}}', encoding="utf-8")

            with self.assertRaisesRegex(PairedRunnerError, "duplicate JSON member"):
                _load_json(artifact, "paired plan")

    def test_counter_reset_after_barrier_is_rejected(self) -> None:
        controller = _StatusSequence([_state(received=9)])
        candidate = Endpoint(
            "target",
            "candidate",
            {},
            RuntimePaths(
                root=Path("/tmp/c"),
                data=Path("/tmp"),
                state=Path("/tmp"),
                log=Path("/tmp"),
                pid=Path("/tmp"),
                env=Path("/tmp"),
            ),
        )
        helper = Endpoint(
            "seed",
            "helper",
            {},
            RuntimePaths(
                root=Path("/tmp/h"),
                data=Path("/tmp"),
                state=Path("/tmp"),
                log=Path("/tmp"),
                pid=Path("/tmp"),
                env=Path("/tmp"),
            ),
        )
        with self.assertRaisesRegex(PairedRunnerError, "counter regressed"):
            _wait_for_transfer(
                target_controller=controller,  # type: ignore[arg-type]
                helper_controller=controller,  # type: ignore[arg-type]
                target_role="target",
                target_endpoint=candidate,
                candidate_endpoint=candidate,
                helper_endpoint=helper,
                info_hash="a" * 40,
                mode="download",
                timeout_seconds=1,
                counter_baseline=_state(received=10),
            )

    def test_staging_refuses_symlinked_input_directory(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-stage-") as directory:
            root = Path(directory) / "scratch"
            root.mkdir(mode=0o700)
            outside = Path(directory) / "outside"
            outside.mkdir(mode=0o700)
            (root / "inputs").symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(PairedRunnerError, "real task-local directory"):
                _stage_input(root, Path("unused"), b"private", "client-pins.json")

    def test_staging_refuses_group_accessible_input_directory(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-stage-mode-") as directory:
            root = Path(directory) / "scratch"
            root.mkdir(mode=0o700)
            inputs = root / "inputs"
            inputs.mkdir(mode=0o700)
            inputs.chmod(0o710)

            with self.assertRaisesRegex(PairedRunnerError, "mode 0700"):
                _stage_input(root, Path("unused"), b"private", "client-pins.json")

    def test_seed_link_refuses_fixture_changed_after_verified_identity_snapshot(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-seed-") as directory:
            root = Path(directory)
            fixture = root / "fixture"
            manifest = generate_fixture(
                fixture,
                "seed-identity",
                [FixtureFile("payload.bin", 32)],
                announce_url="http://127.0.0.1:9/announce",
                torrent_name="trial-root",
                profile_id="test-profile",
            )
            identities = _fixture_file_identities(fixture, manifest)  # type: ignore[arg-type]
            with (fixture / "payload.bin").open("r+b") as payload:
                payload.write(b"x" * 32)
            data = root / "data"
            data.mkdir(mode=0o700)
            with self.assertRaisesRegex(PairedRunnerError, "changed before it was linked"):
                _seed_payload(fixture, manifest, data, "trial-root", identities)  # type: ignore[arg-type]

    def test_candidate_hash_uses_full_supported_hash_and_v1_for_urtorrent(self) -> None:
        hashes = {
            "sha1": "a" * 40,
            "sha256": "b" * 64,
            "sha256_truncated_20": "b" * 40,
        }
        self.assertEqual(_candidate_info_hash({"info_hashes": hashes}, "torrentng"), "a" * 40)
        self.assertEqual(_candidate_info_hash({"info_hashes": hashes}, "urtorrent"), "a" * 40)
        hashes.pop("sha1")
        self.assertEqual(_candidate_info_hash({"info_hashes": hashes}, "torrentng"), "b" * 64)
        with self.assertRaisesRegex(PairedRunnerError, "supported"):
            _candidate_info_hash({"info_hashes": hashes}, "urtorrent")

    def test_observer_must_write_a_sample_before_the_barrier(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-observer-") as directory:
            csv_path = Path(directory) / "resources.csv"
            csv_path.write_text("elapsed_ns,rss_bytes\n1,4096\n", encoding="utf-8")

            _wait_for_observer_sample(_LiveProcess(), csv_path)  # type: ignore[arg-type]

    def test_verified_fixture_reuse_rejects_changed_layout_without_rehashing(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-cache-") as directory:
            fixture = Path(directory) / "fixture"
            generate_fixture(
                fixture,
                "cache-reuse-seed",
                [FixtureFile("payload.bin", 4096)],
                announce_url="http://127.0.0.1:9/announce",
                torrent_name="fixture-root",
                profile_id="test-profile",
            )
            manifest = verify_fixture(fixture)

            reused = _reuse_verified_fixture_manifest(fixture, manifest)

            self.assertEqual(reused["fixture_sha256"], manifest["fixture_sha256"])
            with (fixture / "payload.bin").open("ab") as payload:
                payload.write(b"x")
            with self.assertRaisesRegex(ValueError, "layout changed"):
                _reuse_verified_fixture_manifest(fixture, manifest)

    def test_fixture_identity_detects_mutation_through_seed_hardlink(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-fixture-") as directory:
            root = Path(directory)
            fixture = root / "fixture"
            generate_fixture(
                fixture,
                "stable-fixture-seed",
                [FixtureFile("payload.bin", 4096)],
                announce_url="http://127.0.0.1:9/announce",
                torrent_name="fixture-root",
                profile_id="test-profile",
            )
            manifest = verify_fixture(fixture)
            identities = _fixture_file_identities(fixture, manifest)  # type: ignore[arg-type]
            seed_copy = root / "seed-payload.bin"
            os.link(fixture / "payload.bin", seed_copy)
            self.assertFalse(_fixture_matches_identities(fixture, manifest, identities))  # type: ignore[arg-type]

            # The new link itself changes ctime, so refresh the captured
            # identity before detecting a same-size write through that link.
            identities = _fixture_file_identities(fixture, manifest)  # type: ignore[arg-type]
            with seed_copy.open("r+b") as payload:
                payload.seek(0)
                payload.write(b"x")

            self.assertFalse(_fixture_matches_identities(fixture, manifest, identities))  # type: ignore[arg-type]

    def test_stream_hash_rejects_links(self) -> None:
        with tempfile.TemporaryDirectory(prefix="tng-paired-hash-") as directory:
            root = Path(directory)
            payload = root / "payload.bin"
            payload.write_bytes(b"fixture bytes")
            self.assertEqual(_sha256_file(payload, "payload"), hashlib.sha256(b"fixture bytes").hexdigest())
            link = root / "payload-link.bin"
            link.symlink_to(payload)
            with self.assertRaisesRegex(PairedRunnerError, "without following links"):
                _sha256_file(link, "payload link")

    def test_payload_failure_reports_observed_digest_not_a_placeholder(self) -> None:
        expected = b"expected fixture bytes"
        observed = b"different fixture bytes"
        expected_entry = {
            "name": "payload.bin",
            "size_bytes": len(expected),
            "sha256": hashlib.sha256(expected).hexdigest(),
        }
        manifest = {"files": [expected_entry]}
        expected_aggregate = _aggregate_payload_digest(
            [(expected_entry["name"], expected_entry["size_bytes"], expected_entry["sha256"])]
        )

        with tempfile.TemporaryDirectory(prefix="tng-paired-payload-") as directory:
            root = Path(directory)
            payload_dir = root / "fixture"
            payload_dir.mkdir()
            (payload_dir / "payload.bin").write_bytes(observed)

            byte_count, expected_digest, observed_digest, failures = _validate_payload(
                root, manifest, "fixture"
            )

        self.assertEqual(byte_count, len(expected))
        self.assertEqual(expected_digest, expected_aggregate)
        self.assertEqual(failures, 1)
        self.assertNotEqual(observed_digest, expected_digest)
        self.assertNotEqual(observed_digest, "0" * 64)

    def test_payload_validation_does_not_follow_a_parent_directory_link(self) -> None:
        expected = b"protected payload"
        entry = {
            "name": "nested/payload.bin",
            "size_bytes": len(expected),
            "sha256": hashlib.sha256(expected).hexdigest(),
        }
        manifest = {"files": [entry]}

        with tempfile.TemporaryDirectory(prefix="tng-paired-symlink-") as directory:
            root = Path(directory)
            (root / "fixture").mkdir()
            outside = root / "outside"
            outside.mkdir()
            (outside / "payload.bin").write_bytes(expected)
            (root / "fixture" / "nested").symlink_to(outside, target_is_directory=True)

            _byte_count, expected_digest, observed_digest, failures = _validate_payload(
                root, manifest, "fixture"
            )

        self.assertEqual(failures, 1)
        self.assertNotEqual(expected_digest, observed_digest)

    def test_transfer_progress_uses_the_pre_barrier_counter_baseline(self) -> None:
        info_hash = "a" * 40
        baseline = _state(received=100)
        controller = _StatusSequence([_state(received=150), _state(received=200, complete=True)])
        paths = RuntimePaths(
            root=Path("/tmp/endpoint"),
            data=Path("/tmp/endpoint/data"),
            state=Path("/tmp/endpoint/state"),
            log=Path("/tmp/endpoint/client.log"),
            pid=Path("/tmp/endpoint/state/client.pid"),
            env=Path("/tmp/endpoint/state/client.env"),
        )
        endpoint = Endpoint("target", "torrentngd", {}, paths, controller=controller)  # type: ignore[arg-type]
        helper = Endpoint("seed", "qbittorrent", {}, paths, controller=controller)  # type: ignore[arg-type]

        first_ns, last_ns, candidate_bytes = _wait_for_transfer(
            target_controller=controller,  # type: ignore[arg-type]
            helper_controller=controller,  # type: ignore[arg-type]
            target_role="target",
            target_endpoint=endpoint,
            candidate_endpoint=endpoint,
            helper_endpoint=helper,
            info_hash=info_hash,
            mode="download",
            timeout_seconds=1,
            counter_baseline=baseline,
        )

        self.assertIsNotNone(first_ns)
        self.assertEqual(first_ns, last_ns)
        self.assertEqual(candidate_bytes, 200)

    def test_missing_task_topology_remains_null_in_trial_environment(self) -> None:
        template = {
            "host_profile": "test-host",
            "os": "Linux",
            "kernel": "test-kernel",
            "cpu_model": "test-cpu",
            "cgroup_version": "v2",
            "memory_limit_bytes": None,
            "storage": {"profile": "local", "filesystem": "ext4", "device_id": None},
            "network": {"link_limit_bps": None, "rtt_ms": None},
            "independent_ceiling_sha256": None,
            "cache_state": "unknown",
            "cache_state_observation_sha256": None,
            "durability_mode": "checkpoint",
        }

        environment = _environment_for_trial(template, None, None, None, None, [])

        self.assertIsNone(environment["network"]["task_topology_sha256"])
        self.assertNotEqual(environment["network"]["task_topology_sha256"], "0" * 64)

    def test_network_seed_is_stable_for_a_pair_and_separate_by_endpoint(self) -> None:
        target_seed = _network_seed(
            "plan-x:G3/v2-piece-layer:rep-002",
            "wan-20m-30ms-jitter-loss-v1",
            "target",
        )
        self.assertEqual(
            target_seed,
            _network_seed(
                "plan-x:G3/v2-piece-layer:rep-002",
                "wan-20m-30ms-jitter-loss-v1",
                "target",
            ),
        )
        self.assertNotEqual(
            target_seed,
            _network_seed(
                "plan-x:G3/v2-piece-layer:rep-002",
                "wan-20m-30ms-jitter-loss-v1",
                "seed",
            ),
        )
        self.assertNotEqual(
            target_seed,
            _network_seed(
                "plan-x:G3/v2-piece-layer:rep-003",
                "wan-20m-30ms-jitter-loss-v1",
                "target",
            ),
        )

    def test_environment_records_the_seeded_loss_and_jitter_profile(self) -> None:
        template = {
            "host_profile": "test-host",
            "os": "Linux",
            "kernel": "test-kernel",
            "cpu_model": "test-cpu",
            "cgroup_version": "v2",
            "memory_limit_bytes": None,
            "storage": {"profile": "local", "filesystem": "ext4", "device_id": None},
            "network": {"link_limit_bps": None, "rtt_ms": None},
            "independent_ceiling_sha256": None,
            "cache_state": "unknown",
            "cache_state_observation_sha256": None,
            "durability_mode": "checkpoint",
        }
        environment = _environment_for_trial(
            template,
            "a" * 64,
            None,
            None,
            "wan-20m-30ms-jitter-loss-v1",
            ["b" * 64, "c" * 64],
            [101, 202],
        )
        network = environment["network"]
        self.assertIsNone(network["rtt_ms"])
        self.assertEqual(network["one_way_delay_ms"], 30)
        self.assertEqual(network["jitter_ms"], 8)
        self.assertEqual(network["loss_percent"], 0.5)
        self.assertEqual(network["control_prng_seeds"], [101, 202])


if __name__ == "__main__":
    unittest.main()
