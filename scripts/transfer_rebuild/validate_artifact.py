#!/usr/bin/env python3
"""Validate one transfer-rebuild trial or manifest against its versioned schema."""

from __future__ import annotations

import argparse
import csv
import hashlib
import json
import sys
from pathlib import Path, PurePosixPath, PureWindowsPath
from stat import S_ISREG
from typing import Any

try:
    from jsonschema import (
        Draft202012Validator,
        FormatChecker,
        SchemaError,
        ValidationError,
    )
except ImportError as error:  # pragma: no cover - exercised by the CLI environment
    raise SystemExit(
        "missing dependency: install scripts/transfer_rebuild/requirements.txt"
    ) from error


ROOT = Path(__file__).resolve().parents[2]
SCHEMAS = {
    "trial": ROOT / "certification/transfer-rebuild/trial.schema.json",
    "manifest": ROOT / "certification/transfer-rebuild/manifest.schema.json",
    "client-observation": ROOT / "certification/transfer-rebuild/client-observation.schema.json",
    "resource-observation": ROOT
    / "certification/transfer-rebuild/resource-observation.schema.json",
    "payload-copy-observation": ROOT
    / "certification/transfer-rebuild/payload-copy-observation.schema.json",
    "cache-state": ROOT / "certification/transfer-rebuild/cache-state.schema.json",
    "network-control": ROOT / "certification/transfer-rebuild/network-control.schema.json",
    "ceiling-input": ROOT / "certification/transfer-rebuild/ceiling-input.schema.json",
    "ceiling": ROOT / "certification/transfer-rebuild/ceiling-report.schema.json",
    "scorecard": ROOT / "certification/transfer-rebuild/scorecard.schema.json",
    "paired-plan": ROOT / "certification/transfer-rebuild/paired-plan.schema.json",
    "compatibility-preflight": ROOT
    / "certification/transfer-rebuild/compatibility-preflight.schema.json",
}
CLIENT_OBSERVATION_KIND = "transfer-client-observation-v1"
CACHE_STATE_KIND = "transfer-cache-state-v1"
NETWORK_CONTROL_KIND = "transfer-network-control-v1"
CEILING_INPUT_KIND = "transfer-ceiling-input-v1"
CEILING_KIND = "transfer-ceiling-v1"
CALIBRATION_RAW_KIND = "transfer-calibration-raw-v1"
RESOURCE_OBSERVATION_KIND = "transfer-resource-observation-v1"
PAYLOAD_COPY_OBSERVATION_KIND = "transfer-payload-copy-observation-v1"
RESOURCE_SAMPLES_KIND = "transfer-resource-samples-v1"
RESOURCE_SAMPLE_FIELDS = (
    "elapsed_ns",
    "processes",
    "rss_bytes",
    "pss_bytes",
    "cgroup_anon_bytes",
    "cgroup_file_bytes",
    "cgroup_reclaimable_cache_bytes",
    "cgroup_current_bytes",
    "cpu_time_ns",
    "threads",
    "fds",
    "block_read_bytes",
    "block_write_bytes",
    "network_namespace_inode",
    "network_rx_bytes",
    "network_tx_bytes",
)
RESOURCE_PEAK_FIELDS = {
    "rss_bytes": "rss_bytes",
    "pss_bytes": "pss_bytes",
    "cgroup_anon_bytes": "cgroup_anon_bytes",
    "cgroup_file_bytes": "cgroup_file_bytes",
    "cgroup_reclaimable_cache_bytes": "cgroup_reclaimable_cache_bytes",
    "cgroup_current_bytes": "cgroup_current_bytes",
    "cpu_time_ns": "cpu_time_ns",
    "threads": "threads",
    "fds": "fds",
    "block_read_bytes": "block_read_bytes",
    "block_write_bytes": "block_write_bytes",
}
NETWORK_PROFILES = {
    "lan-1g-1ms-v1": {"delay_ms": 1, "rate_bps": 1_000_000_000, "limit_packets": 1000},
    "metro-100m-10ms-v1": {"delay_ms": 10, "rate_bps": 100_000_000, "limit_packets": 1000},
    "wan-20m-30ms-v1": {"delay_ms": 30, "rate_bps": 20_000_000, "limit_packets": 1000},
}


def _load_json(path: Path) -> object:
    try:
        with path.open("r", encoding="utf-8") as source:
            return json.load(source)
    except (OSError, json.JSONDecodeError) as error:
        raise ValueError(f"{path}: {error}") from error


def _strict_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise ValueError(f"duplicate JSON field: {key}")
        value[key] = item
    return value


def _reject_nonstandard_number(value: str) -> None:
    raise ValueError(f"non-standard JSON number is not allowed: {value}")


def _load_strict_json(path: Path) -> object:
    try:
        return json.loads(
            path.read_bytes(),
            object_pairs_hook=_strict_object,
            parse_constant=_reject_nonstandard_number,
        )
    except (OSError, json.JSONDecodeError, UnicodeDecodeError, ValueError) as error:
        raise ValueError(f"{path}: {error}") from error


def _format_path(error: ValidationError) -> str:
    path = "$"
    for part in error.absolute_path:
        if isinstance(part, int):
            path += f"[{part}]"
        else:
            path += f"[{json.dumps(str(part))}]"
    return path


def _referenced_file(root: Path, value: str) -> tuple[Path | None, str | None]:
    relative = Path(value)
    windows = PureWindowsPath(value)
    if (
        relative.is_absolute()
        or windows.is_absolute()
        or windows.drive
        or "\\" in value
        or any(part in ("", ".", "..") for part in value.split("/"))
    ):
        return None, "path must be a normalized relative path"
    candidate = root
    for part in value.split("/"):
        candidate = candidate / part
        if candidate.is_symlink():
            return None, "symbolic links are not allowed"
    try:
        candidate.resolve(strict=True).relative_to(root.resolve(strict=True))
        metadata = candidate.stat(follow_symlinks=False)
    except (OSError, ValueError):
        return None, "file is missing or escapes the manifest directory"
    if not S_ISREG(metadata.st_mode):
        return None, "referenced path is not a regular file"
    return candidate, None


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _settings_digest(values: dict[str, Any]) -> str:
    canonical = json.dumps(values, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    return hashlib.sha256(canonical).hexdigest()


def _resolve_json_pointer(document: Any, pointer: str) -> Any:
    if not pointer.startswith("/"):
        raise ValueError("settings pointer must be an absolute JSON pointer")
    current = document
    for encoded in pointer[1:].split("/"):
        token_chars: list[str] = []
        index = 0
        while index < len(encoded):
            char = encoded[index]
            if char != "~":
                token_chars.append(char)
                index += 1
                continue
            if index + 1 >= len(encoded) or encoded[index + 1] not in "01":
                raise ValueError("settings pointer has an invalid escape")
            token_chars.append("~" if encoded[index + 1] == "0" else "/")
            index += 2
        token = "".join(token_chars)
        if isinstance(current, dict):
            if token not in current:
                raise ValueError(f"settings pointer field is missing: {token}")
            current = current[token]
        elif isinstance(current, list):
            if not token.isdecimal() or (len(token) > 1 and token.startswith("0")):
                raise ValueError("settings pointer list index is invalid")
            position = int(token)
            if position >= len(current):
                raise ValueError("settings pointer list index is out of range")
            current = current[position]
        else:
            raise ValueError("settings pointer traverses a scalar value")
    return current


def _client_observation_errors(observation: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    runtime = observation["runtime_identity"]
    if runtime["kind"] == "container":
        if runtime["image_digest"] != runtime["image_ref"].rsplit("@", 1)[1]:
            errors.append("runtime_identity.image_digest does not match image_ref")
    settings = observation["settings_readback"]
    if settings["sha256"] != _settings_digest(settings["values"]):
        errors.append("settings_readback.sha256 does not match canonical settings values")
    return errors


def _compatibility_preflight_errors(artifact: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    unsigned = {key: value for key, value in artifact.items() if key != "preflight_id"}
    if _settings_digest(unsigned)[:20] != artifact["preflight_id"]:
        errors.append("preflight_id does not match the artifact contents")

    clients = (
        "deluge", "qbittorrent", "rtorrent", "torrentngd", "transmission", "rqbit", "urtorrent"
    )
    profiles = {
        "G3/hybrid-padding": "hybrid",
        "G3/v2-piece-layer": "v2",
        "N1/lan_1g": "v1",
        "N1/lan_10g": "v1",
    }
    pinned = artifact["pinned_client_ids"]
    if pinned != sorted(set(pinned)):
        errors.append("pinned_client_ids must be sorted and unique")
    pinned_set = set(pinned)
    expected_case_ids = {f"{client}:{profile}" for client in clients for profile in profiles}
    results = artifact["results"]
    seen: set[str] = set()
    for index, result in enumerate(results):
        location = f"results[{index}]"
        client = result["client_id"]
        profile = result["profile_id"]
        case_id = result["case_id"]
        if case_id != f"{client}:{profile}":
            errors.append(f"{location} has an invalid case identity")
            continue
        if case_id in seen:
            errors.append(f"{location} duplicates case_id {case_id}")
        seen.add(case_id)
        protocol = profiles[profile]
        if result["protocol"] != protocol:
            errors.append(f"{location}.protocol does not match its profile")
        expected_hash_length = 64 if protocol == "v2" else 40
        if len(result["info_hash"]) != expected_hash_length:
            errors.append(f"{location}.info_hash has the wrong length for {protocol}")
        statically_unsupported = client == "urtorrent" and protocol in {"v2", "hybrid"}
        expected_applicability = "unsupported" if statically_unsupported else "candidate"
        if result["applicability"] != expected_applicability:
            errors.append(f"{location}.applicability does not match the PERF-110 catalog")
        status = result["status"]
        if statically_unsupported:
            if status != "unsupported":
                errors.append(f"{location}.status must preserve the urtorrent protocol exclusion")
        elif client not in pinned_set:
            if status != "not_pinned":
                errors.append(f"{location}.status must be not_pinned for an absent client pin")
        elif status not in {"passed", "failed", "cleanup_failed"}:
            errors.append(f"{location}.status is invalid for a pinned candidate")
        if status == "passed":
            if not all(result[field] is True for field in ("added", "paused_verified", "removed")):
                errors.append(f"{location} passed without add, paused-state, and removal evidence")
            expected_pause_mode = "after_add" if client == "rqbit" else "on_add"
            if result["pause_mode"] != expected_pause_mode:
                errors.append(f"{location}.pause_mode does not match the client admission behavior")
            if not result["observed_version"] or not result["client_identity_sha256"]:
                errors.append(f"{location} passed without pinned client identity")
            if not result["settings_sha256"]:
                errors.append(f"{location} passed without settings readback identity")
            if result["reason"] is not None:
                errors.append(f"{location}.reason must be null for a passed probe")
        elif status == "cleanup_failed":
            if result["added"] not in (True, None) or result["removed"] is not False:
                errors.append(f"{location} cleanup_failed requires a present or uncertain, unremoved probe")
            if not result["reason"]:
                errors.append(f"{location}.reason is required when cleanup failed")
        elif not result["reason"]:
            errors.append(f"{location}.reason is required for {status}")
        if status in {"unsupported", "not_pinned"} and result["pause_mode"] is not None:
            errors.append(f"{location}.pause_mode must be null when no probe was attempted")
    if seen != expected_case_ids:
        errors.append("results must contain exactly the 28 PERF-110 client/profile slots")

    expected_eligible = sorted(
        result["case_id"] for result in results if result["status"] == "passed"
    )
    if artifact["eligible_case_ids"] != expected_eligible:
        errors.append("eligible_case_ids does not match passed case results")

    pair_ids: set[str] = set()
    expected_excluded: set[str] = set()
    for index, pair in enumerate(artifact["scheduled_pairs"]):
        pair_id = pair["pair_id"]
        case_ids = pair["case_ids"]
        if pair_id in pair_ids:
            errors.append(f"scheduled_pairs[{index}] duplicates pair_id")
        pair_ids.add(pair_id)
        if len(case_ids) != len(set(case_ids)):
            errors.append(f"scheduled_pairs[{index}].case_ids contains duplicates")
        if any(
            case_id not in expected_case_ids or not case_id.endswith(f":{pair['profile_id']}")
            for case_id in case_ids
        ):
            errors.append(f"scheduled_pairs[{index}] references an invalid case")
        expected_pair_cases = {
            f"{client}:{pair['profile_id']}"
            for client in pinned_set
            if not (
                client == "urtorrent"
                and profiles[pair["profile_id"]] in {"v2", "hybrid"}
            )
        }
        if len(expected_pair_cases) < 2 or set(case_ids) != expected_pair_cases:
            errors.append(f"scheduled_pairs[{index}] does not contain every pinned candidate for its profile")
        if sum(case_id in expected_eligible for case_id in case_ids) < 2:
            expected_excluded.add(pair_id)

    excluded_ids: set[str] = set()
    for index, pair in enumerate(artifact["excluded_pairs"]):
        pair_id = pair["pair_id"]
        if pair_id is not None:
            if pair_id not in pair_ids or pair_id in excluded_ids:
                errors.append(f"excluded_pairs[{index}].pair_id is unknown or duplicated")
            excluded_ids.add(pair_id)
    if not expected_excluded.issubset(excluded_ids):
        errors.append("excluded_pairs omits a scheduled pair with fewer than two eligible clients")

    candidates = [
        result for result in results
        if result["applicability"] == "candidate" and result["client_id"] in pinned_set
    ]
    expected_ready = (
        bool(candidates)
        and bool(artifact["scheduled_pairs"])
        and all(result["status"] == "passed" for result in candidates)
        and not artifact["excluded_pairs"]
    )
    if artifact["ready"] is not expected_ready:
        errors.append("ready does not match candidate and pair preflight outcomes")
    return errors


def _resource_observation_errors(observation: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if observation["finished_monotonic_ns"] < observation["started_monotonic_ns"]:
        errors.append("finished_monotonic_ns precedes started_monotonic_ns")

    network_values = (
        observation["network_namespace_inode"],
        observation["network_rx_bytes"],
        observation["network_tx_bytes"],
    )
    if any(value is not None for value in network_values) and not all(
        value is not None for value in network_values
    ):
        errors.append("network namespace and RX/TX counters must be present together")
    if all(value is not None for value in network_values):
        if observation["network_interface"] is None:
            errors.append("network counters require a named interface")
        if observation["sample_count"] < 2:
            errors.append("network counters require at least two samples")
    if (
        observation["network_interface"] is None
        and any(value is not None for value in network_values)
    ):
        errors.append("network counters were reported without requesting an interface")
    network_note = observation["network_note"].lower()
    if observation["network_interface"] is None:
        if "not requested" not in network_note:
            errors.append("network_note does not say that network observation was not requested")
    elif "isolates exactly one client" not in network_note:
        errors.append("network_note omits the one-client namespace requirement")
    return errors


def _resource_samples_errors(path: Path, observation: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    peaks: dict[str, int | None] = {field: None for field in RESOURCE_PEAK_FIELDS}
    peak_valid = {field: True for field in RESOURCE_PEAK_FIELDS}
    sample_count = 0
    previous_elapsed_ns: int | None = None
    observation_duration_ns = (
        observation["finished_monotonic_ns"] - observation["started_monotonic_ns"]
    )
    interface = observation["network_interface"]
    network_valid = interface is not None
    network_count = 0
    namespace_inode: int | None = None
    first_network: tuple[int, int] | None = None
    last_network: tuple[int, int] | None = None
    network_values_without_interface = False

    try:
        with path.open("r", encoding="utf-8", newline="") as source:
            reader = csv.DictReader(source)
            if tuple(reader.fieldnames or ()) != RESOURCE_SAMPLE_FIELDS:
                return ["CSV header does not match transfer resource samples v1"]
            for row in reader:
                sample_count += 1
                if None in row:
                    return [f"CSV row {sample_count} has extra fields"]
                values: dict[str, int | None] = {}
                for field in RESOURCE_SAMPLE_FIELDS:
                    raw = row.get(field)
                    if raw is None:
                        return [f"CSV row {sample_count} is missing {field}"]
                    if raw == "":
                        values[field] = None
                        continue
                    if not raw.isascii() or not raw.isdecimal():
                        return [f"CSV row {sample_count} has a non-canonical {field}"]
                    try:
                        value = int(raw)
                    except ValueError:
                        return [f"CSV row {sample_count} has a non-integer {field}"]
                    if str(value) != raw:
                        return [f"CSV row {sample_count} has a non-canonical {field}"]
                    if value < 0:
                        return [f"CSV row {sample_count} has a negative {field}"]
                    values[field] = value
                elapsed_ns = values["elapsed_ns"]
                process_count = values["processes"]
                if elapsed_ns is None or process_count is None:
                    return [f"CSV row {sample_count} is missing elapsed_ns or processes"]
                if process_count < 1:
                    return [f"CSV row {sample_count} has no observed processes"]
                if previous_elapsed_ns is not None and elapsed_ns <= previous_elapsed_ns:
                    return [f"CSV row {sample_count} elapsed_ns is not strictly increasing"]
                if elapsed_ns > observation_duration_ns:
                    return [f"CSV row {sample_count} elapsed_ns exceeds the observation interval"]
                previous_elapsed_ns = elapsed_ns

                for field in RESOURCE_PEAK_FIELDS:
                    value = values[field]
                    if value is None:
                        peak_valid[field] = False
                    else:
                        previous = peaks[field]
                        peaks[field] = value if previous is None else max(previous, value)

                sample_network = (
                    values["network_namespace_inode"],
                    values["network_rx_bytes"],
                    values["network_tx_bytes"],
                )
                if interface is None:
                    if any(value is not None for value in sample_network):
                        network_values_without_interface = True
                        network_valid = False
                    continue
                if any(value is None for value in sample_network):
                    network_valid = False
                    continue
                sample_inode, received, transmitted = sample_network
                assert sample_inode is not None and received is not None and transmitted is not None
                network_count += 1
                if namespace_inode is None:
                    namespace_inode = sample_inode
                    first_network = (received, transmitted)
                    last_network = first_network
                    continue
                if (
                    sample_inode != namespace_inode
                    or last_network is None
                    or received < last_network[0]
                    or transmitted < last_network[1]
                ):
                    network_valid = False
                    continue
                last_network = (received, transmitted)
    except (OSError, UnicodeDecodeError, csv.Error) as error:
        return [f"cannot read resource samples CSV: {error}"]

    if sample_count != observation["sample_count"]:
        errors.append("CSV row count differs from sample_count")
    if network_values_without_interface:
        errors.append("CSV has network values although network observation was not requested")
    for field, observed in observation["peaks"].items():
        expected = peaks[field] if peak_valid[field] else None
        if observed != expected:
            errors.append(f"peaks.{field} differs from the CSV samples")

    network_observed = (
        interface is not None
        and network_valid
        and network_count >= 2
        and first_network is not None
        and last_network is not None
    )
    expected_network = (
        (
            namespace_inode,
            last_network[0] - first_network[0],
            last_network[1] - first_network[1],
        )
        if network_observed
        else (None, None, None)
    )
    actual_network = (
        observation["network_namespace_inode"],
        observation["network_rx_bytes"],
        observation["network_tx_bytes"],
    )
    if actual_network != expected_network:
        errors.append("network summary differs from the CSV samples")
    return errors


def _cache_state_errors(observation: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    if observation["status"] == "unsupported":
        return errors
    files = observation["files"]
    payload_bytes = sum(item["payload_bytes"] for item in files)
    before_bytes = sum(item["resident_bytes_before"] for item in files)
    after_bytes = sum(item["resident_bytes_after"] for item in files)
    names = [item["name"] for item in files]
    if len(names) != len(set(names)):
        errors.append("file names are not unique")
    if payload_bytes != observation["payload_bytes"]:
        errors.append("files payload byte sum does not match payload_bytes")
    if before_bytes != observation["resident_bytes_before"]:
        errors.append("files resident-before sum does not match report total")
    if after_bytes != observation["resident_bytes_after"]:
        errors.append("files resident-after sum does not match report total")
    for item in files:
        if item["resident_bytes_before"] > item["payload_bytes"]:
            errors.append(f"file {item['name']!r} resident-before exceeds payload size")
        if item["resident_bytes_after"] > item["payload_bytes"]:
            errors.append(f"file {item['name']!r} resident-after exceeds payload size")
        expected_read = item["payload_bytes"] if observation["requested_state"] == "warm" else 0
        if item["bytes_read_for_warm_state"] != expected_read:
            errors.append(f"file {item['name']!r} warm-read byte count is inconsistent")
    achieved = (
        after_bytes == 0
        if observation["requested_state"] == "cold"
        else after_bytes == payload_bytes
    )
    expected_status = "achieved" if achieved else "partial"
    if observation["status"] != expected_status:
        errors.append("status does not match the observed page residency")
    return errors


def _network_control_errors(observation: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    unsigned = {key: value for key, value in observation.items() if key != "artifact_sha256"}
    canonical = json.dumps(unsigned, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    if observation["artifact_sha256"] != hashlib.sha256(canonical).hexdigest():
        errors.append("artifact_sha256 does not match canonical network-control fields")
    profile_id = observation["profile_id"]
    expected_settings = NETWORK_PROFILES.get(profile_id)
    if expected_settings is None or observation["settings"] != expected_settings:
        errors.append("settings do not match the pinned network-control profile")
        return errors
    observed = observation["observed_root_qdisc"]
    if observation["action"] == "apply":
        expected_qdisc = {
            "kind": "netem",
            "handle": "7fff:",
            "delay_ns": expected_settings["delay_ms"] * 1_000_000,
            "rate_bps": expected_settings["rate_bps"],
            "limit_packets": expected_settings["limit_packets"],
        }
        if observed != expected_qdisc:
            errors.append("observed root qdisc does not match the pinned profile")
        if observation["baseline_root_qdisc"] != {"kind": "noqueue", "handle": "0:"}:
            errors.append("apply report does not record the supported noqueue baseline")
        if observation["applied_record_sha256"] is not None:
            errors.append("apply report must not refer to another applied report")
    else:
        baseline = {"kind": "noqueue", "handle": "0:"}
        if observed != baseline or observation["baseline_root_qdisc"] != baseline:
            errors.append("reset report does not show the noqueue qdisc was restored")
        if observation["applied_record_sha256"] is None:
            errors.append("reset report must identify its applied report")
    versions = {"ip": observation["ip_version"], "tc": observation["tc_version"]}
    tool_digest = hashlib.sha256(
        json.dumps(versions, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()
    ).hexdigest()
    if observation["toolchain_sha256"] != tool_digest:
        errors.append("toolchain_sha256 does not match iproute2 version strings")
    return errors


def _ceiling_input_errors(observation: dict[str, Any], linked_digests: set[str]) -> list[str]:
    errors: list[str] = []
    sources = (
        observation["network"],
        observation["disk"],
        observation["hash"],
        observation["pipeline"],
    )
    for source in sources:
        digest = source["raw_artifact_sha256"]
        if digest not in linked_digests:
            errors.append(f"raw calibration artifact {digest} is not linked from the manifest")
    return errors


def _ceiling_report_errors(
    observation: dict[str, Any],
    calibration: dict[str, Any] | None,
    calibration_digest: str | None,
) -> list[str]:
    if calibration is None or calibration_digest is None:
        return ["report does not reference a verified calibration input"]
    try:
        sys_path = str(ROOT / "scripts/transfer_rebuild")
        if sys_path not in sys.path:
            sys.path.insert(0, sys_path)
        from ceilings import CeilingError, calculate_report

        expected = calculate_report(calibration, calibration_digest)
    except (CeilingError, ImportError) as error:
        return [f"cannot reproduce ceiling report: {error}"]
    if observation != expected:
        return ["reported rates or limiting components differ from the calibration input"]
    return []


def _verify_manifest_files(manifest_path: Path, manifest: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    root = manifest_path.parent
    trial_schema = _load_json(SCHEMAS["trial"])
    trial_validator = Draft202012Validator(trial_schema, format_checker=FormatChecker())
    client_schema = _load_json(SCHEMAS["client-observation"])
    client_validator = Draft202012Validator(client_schema, format_checker=FormatChecker())
    resource_schema = _load_json(SCHEMAS["resource-observation"])
    resource_validator = Draft202012Validator(resource_schema, format_checker=FormatChecker())
    payload_copy_schema = _load_json(SCHEMAS["payload-copy-observation"])
    payload_copy_validator = Draft202012Validator(
        payload_copy_schema, format_checker=FormatChecker()
    )
    cache_schema = _load_json(SCHEMAS["cache-state"])
    cache_validator = Draft202012Validator(cache_schema, format_checker=FormatChecker())
    network_schema = _load_json(SCHEMAS["network-control"])
    network_validator = Draft202012Validator(network_schema, format_checker=FormatChecker())
    ceiling_input_schema = _load_json(SCHEMAS["ceiling-input"])
    ceiling_input_validator = Draft202012Validator(ceiling_input_schema, format_checker=FormatChecker())
    ceiling_schema = _load_json(SCHEMAS["ceiling"])
    ceiling_validator = Draft202012Validator(ceiling_schema, format_checker=FormatChecker())
    client_observations: dict[str, dict[str, Any]] = {}
    resource_observations: dict[str, dict[str, Any]] = {}
    resource_sample_artifacts: dict[str, str] = {}
    payload_copy_observations: dict[str, dict[str, Any]] = {}
    cache_observations: dict[str, dict[str, Any]] = {}
    network_observations: dict[str, dict[str, Any]] = {}
    ceiling_inputs: dict[str, dict[str, Any]] = {}
    ceiling_reports: dict[str, dict[str, Any]] = {}
    ceiling_report_inputs: dict[str, dict[str, Any]] = {}
    verified_raw_digests: set[str] = set()
    pending_ceiling_reports: list[tuple[int, dict[str, Any]]] = []

    for index, record in enumerate(manifest.get("artifacts", [])):
        if record["kind"] not in {CALIBRATION_RAW_KIND, RESOURCE_SAMPLES_KIND}:
            continue
        location = f"$.artifacts[{index}]"
        path, path_error = _referenced_file(root, record["path"])
        if path_error is not None or path is None:
            errors.append(f"{location}.path: {path_error}")
            continue
        try:
            observed_digest = _sha256(path)
        except OSError as error:
            errors.append(f"{location}.path: cannot read raw evidence artifact: {error}")
            continue
        if observed_digest != record["sha256"]:
            errors.append(f"{location}.sha256: raw evidence digest does not match {record['path']}")
            continue
        if record["kind"] == CALIBRATION_RAW_KIND:
            verified_raw_digests.add(observed_digest)
        else:
            resource_sample_artifacts[record["path"]] = observed_digest

    for index, record in enumerate(manifest.get("artifacts", [])):
        if record["kind"] != RESOURCE_OBSERVATION_KIND:
            continue
        location = f"$.artifacts[{index}]"
        path, path_error = _referenced_file(root, record["path"])
        if path_error is not None or path is None:
            errors.append(f"{location}.path: {path_error}")
            continue
        try:
            observed_digest = _sha256(path)
        except OSError as error:
            errors.append(f"{location}.path: cannot read resource observation: {error}")
            continue
        if observed_digest != record["sha256"]:
            errors.append(f"{location}.sha256: digest does not match {record['path']}")
            continue
        try:
            observation = _load_strict_json(path)
        except ValueError as error:
            errors.append(f"{location}.path: {error}")
            continue
        schema_errors = sorted(
            resource_validator.iter_errors(observation),
            key=lambda error: (list(map(str, error.absolute_path)), error.message),
        )
        if schema_errors:
            for error in schema_errors[:20]:
                errors.append(
                    f"{location}.path: invalid resource observation "
                    f"{_format_path(error)}: {error.message}"
                )
            continue
        if not isinstance(observation, dict):
            errors.append(f"{location}.path: resource observation must be an object")
            continue
        semantic_errors = _resource_observation_errors(observation)
        if semantic_errors:
            errors.extend(f"{location}.path: {error}" for error in semantic_errors)
            continue
        csv_relative = (
            PurePosixPath(record["path"]).parent / observation["samples_csv"]
        ).as_posix()
        csv_digest = resource_sample_artifacts.get(csv_relative)
        if csv_digest is None:
            errors.append(
                f"{location}.path: linked samples CSV is not a verified manifest artifact"
            )
            continue
        if csv_digest != observation["samples_csv_sha256"]:
            errors.append(f"{location}.path: samples CSV digest differs from its summary")
            continue
        csv_path, csv_path_error = _referenced_file(root, csv_relative)
        if csv_path_error is not None or csv_path is None:
            errors.append(f"{location}.path: {csv_path_error}")
            continue
        sample_errors = _resource_samples_errors(csv_path, observation)
        if sample_errors:
            errors.extend(f"{location}.path: {error}" for error in sample_errors)
            continue
        resource_observations[record["sha256"]] = observation

    for index, record in enumerate(manifest.get("artifacts", [])):
        if record["kind"] != PAYLOAD_COPY_OBSERVATION_KIND:
            continue
        location = f"$.artifacts[{index}]"
        path, path_error = _referenced_file(root, record["path"])
        if path_error is not None or path is None:
            errors.append(f"{location}.path: {path_error}")
            continue
        try:
            observed_digest = _sha256(path)
        except OSError as error:
            errors.append(f"{location}.path: cannot read payload-copy observation: {error}")
            continue
        if observed_digest != record["sha256"]:
            errors.append(f"{location}.sha256: digest does not match {record['path']}")
            continue
        try:
            observation = _load_strict_json(path)
        except ValueError as error:
            errors.append(f"{location}.path: {error}")
            continue
        schema_errors = sorted(
            payload_copy_validator.iter_errors(observation),
            key=lambda error: (list(map(str, error.absolute_path)), error.message),
        )
        if schema_errors:
            for error in schema_errors[:20]:
                errors.append(
                    f"{location}.path: invalid payload-copy observation "
                    f"{_format_path(error)}: {error.message}"
                )
            continue
        if isinstance(observation, dict):
            payload_copy_observations[record["sha256"]] = observation

    # Client identity artifacts are verified first so trial records can bind to
    # a specific readback by digest and compare their client/settings identity.
    for index, record in enumerate(manifest.get("artifacts", [])):
        if record["kind"] == CACHE_STATE_KIND:
            location = f"$.artifacts[{index}]"
            path, path_error = _referenced_file(root, record["path"])
            if path_error is not None or path is None:
                errors.append(f"{location}.path: {path_error}")
                continue
            try:
                observed = _sha256(path)
            except OSError as error:
                errors.append(f"{location}.path: cannot read referenced file: {error}")
                continue
            if observed != record["sha256"]:
                errors.append(f"{location}.sha256: digest does not match {record['path']}")
                continue
            try:
                cache_observation = _load_json(path)
            except ValueError as error:
                errors.append(f"{location}.path: {error}")
                continue
            cache_errors = sorted(
                cache_validator.iter_errors(cache_observation),
                key=lambda error: (list(map(str, error.absolute_path)), error.message),
            )
            if cache_errors:
                for error in cache_errors[:20]:
                    errors.append(
                        f"{location}.path: invalid cache-state observation "
                        f"{_format_path(error)}: {error.message}"
                    )
                continue
            if isinstance(cache_observation, dict):
                semantic_errors = _cache_state_errors(cache_observation)
                if semantic_errors:
                    errors.extend(f"{location}.path: {error}" for error in semantic_errors)
                    continue
                cache_observations[record["sha256"]] = cache_observation
            continue
        if record["kind"] == NETWORK_CONTROL_KIND:
            location = f"$.artifacts[{index}]"
            path, path_error = _referenced_file(root, record["path"])
            if path_error is not None or path is None:
                errors.append(f"{location}.path: {path_error}")
                continue
            try:
                observed_digest = _sha256(path)
            except OSError as error:
                errors.append(f"{location}.path: cannot read referenced file: {error}")
                continue
            if observed_digest != record["sha256"]:
                errors.append(f"{location}.sha256: digest does not match {record['path']}")
                continue
            try:
                network_observation = _load_json(path)
            except ValueError as error:
                errors.append(f"{location}.path: {error}")
                continue
            network_errors = sorted(
                network_validator.iter_errors(network_observation),
                key=lambda error: (list(map(str, error.absolute_path)), error.message),
            )
            if network_errors:
                for error in network_errors[:20]:
                    errors.append(
                        f"{location}.path: invalid network-control observation "
                        f"{_format_path(error)}: {error.message}"
                    )
                continue
            if isinstance(network_observation, dict):
                semantic_errors = _network_control_errors(network_observation)
                if semantic_errors:
                    errors.extend(f"{location}.path: {error}" for error in semantic_errors)
                    continue
                network_observations[record["sha256"]] = network_observation
            continue
        if record["kind"] == CEILING_INPUT_KIND:
            location = f"$.artifacts[{index}]"
            path, path_error = _referenced_file(root, record["path"])
            if path_error is not None or path is None:
                errors.append(f"{location}.path: {path_error}")
                continue
            try:
                sys_path = str(ROOT / "scripts/transfer_rebuild")
                if sys_path not in sys.path:
                    sys.path.insert(0, sys_path)
                from ceilings import read_calibration_input

                calibration, observed_digest = read_calibration_input(path)
            except (OSError, ValueError) as error:
                errors.append(f"{location}.path: cannot read calibration input: {error}")
                continue
            if observed_digest != record["sha256"]:
                errors.append(f"{location}.sha256: digest does not match {record['path']}")
                continue
            schema_errors = sorted(
                ceiling_input_validator.iter_errors(calibration),
                key=lambda error: (list(map(str, error.absolute_path)), error.message),
            )
            if schema_errors:
                for error in schema_errors[:20]:
                    errors.append(
                        f"{location}.path: invalid ceiling input {_format_path(error)}: {error.message}"
                    )
                continue
            if isinstance(calibration, dict):
                semantic_errors = _ceiling_input_errors(calibration, verified_raw_digests)
                if semantic_errors:
                    errors.extend(f"{location}.path: {error}" for error in semantic_errors)
                    continue
                ceiling_inputs[record["sha256"]] = calibration
            continue
        if record["kind"] == CEILING_KIND:
            location = f"$.artifacts[{index}]"
            path, path_error = _referenced_file(root, record["path"])
            if path_error is not None or path is None:
                errors.append(f"{location}.path: {path_error}")
                continue
            try:
                observed_digest = _sha256(path)
                report = _load_strict_json(path)
            except (OSError, ValueError) as error:
                errors.append(f"{location}.path: cannot read ceiling report: {error}")
                continue
            if observed_digest != record["sha256"]:
                errors.append(f"{location}.sha256: digest does not match {record['path']}")
                continue
            schema_errors = sorted(
                ceiling_validator.iter_errors(report),
                key=lambda error: (list(map(str, error.absolute_path)), error.message),
            )
            if schema_errors:
                for error in schema_errors[:20]:
                    errors.append(
                        f"{location}.path: invalid ceiling report {_format_path(error)}: {error.message}"
                    )
                continue
            if isinstance(report, dict):
                pending_ceiling_reports.append((index, report))
            continue
        if record["kind"] != CLIENT_OBSERVATION_KIND:
            continue
        location = f"$.artifacts[{index}]"
        path, path_error = _referenced_file(root, record["path"])
        if path_error is not None or path is None:
            errors.append(f"{location}.path: {path_error}")
            continue
        try:
            observed = _sha256(path)
        except OSError as error:
            errors.append(f"{location}.path: cannot read referenced file: {error}")
            continue
        if observed != record["sha256"]:
            errors.append(f"{location}.sha256: digest does not match {record['path']}")
            continue
        try:
            observation = _load_json(path)
        except ValueError as error:
            errors.append(f"{location}.path: {error}")
            continue
        observation_errors = sorted(
            client_validator.iter_errors(observation),
            key=lambda error: (list(map(str, error.absolute_path)), error.message),
        )
        if observation_errors:
            for error in observation_errors[:20]:
                errors.append(
                    f"{location}.path: invalid client observation "
                    f"{_format_path(error)}: {error.message}"
                )
            if len(observation_errors) > 20:
                errors.append(
                    f"{location}.path: {len(observation_errors) - 20} additional client errors omitted"
                )
            continue
        if isinstance(observation, dict):
            semantic_errors = _client_observation_errors(observation)
            if semantic_errors:
                errors.extend(f"{location}.path: {error}" for error in semantic_errors)
                continue
            client_observations[record["sha256"]] = observation

    for index, report in pending_ceiling_reports:
        location = f"$.artifacts[{index}]"
        input_digest = report["input_sha256"]
        calibration = ceiling_inputs.get(input_digest)
        semantic_errors = _ceiling_report_errors(
            report, calibration, input_digest if calibration else None
        )
        if semantic_errors:
            errors.extend(f"{location}.path: {error}" for error in semantic_errors)
            continue
        ceiling_reports[manifest["artifacts"][index]["sha256"]] = report
        ceiling_report_inputs[manifest["artifacts"][index]["sha256"]] = calibration

    applied_network_reports = {
        digest
        for digest, observation in network_observations.items()
        if observation["action"] == "apply"
    }
    for digest, observation in network_observations.items():
        if (
            observation["action"] == "reset"
            and observation["applied_record_sha256"] not in applied_network_reports
        ):
            errors.append(
                "$.artifacts: network reset report does not reference a verified apply report"
            )
    reset_network_reports = {
        observation["applied_record_sha256"]
        for observation in network_observations.values()
        if observation["action"] == "reset"
    }

    for index, record in enumerate(manifest.get("trial_files", [])):
        location = f"$.trial_files[{index}]"
        path, path_error = _referenced_file(root, record["path"])
        if path_error is not None or path is None:
            errors.append(f"{location}.path: {path_error}")
            continue
        try:
            observed = _sha256(path)
        except OSError as error:
            errors.append(f"{location}.path: cannot read referenced file: {error}")
            continue
        if observed != record["sha256"]:
            errors.append(f"{location}.sha256: digest does not match {record['path']}")
            continue
        try:
            trial = _load_json(path)
        except ValueError as error:
            errors.append(f"{location}.path: {error}")
            continue
        trial_errors = sorted(
            trial_validator.iter_errors(trial),
            key=lambda error: (list(map(str, error.absolute_path)), error.message),
        )
        if trial_errors:
            for error in trial_errors[:20]:
                errors.append(
                    f"{location}.path: invalid trial record "
                    f"{_format_path(error)}: {error.message}"
                )
            if len(trial_errors) > 20:
                errors.append(
                    f"{location}.path: {len(trial_errors) - 20} additional trial errors omitted"
                )
            continue
        if not isinstance(trial, dict):
            errors.append(f"{location}.path: trial record must be an object")
            continue
        if trial["trial_id"] != record["trial_id"]:
            errors.append(f"{location}.trial_id: does not match the referenced trial")
        if trial["pair_id"] != record["pair_id"]:
            errors.append(f"{location}.pair_id: does not match the referenced trial")
        if trial["outcome"]["status"] != record["outcome"]:
            errors.append(f"{location}.outcome: does not match the referenced trial")
        if trial["identity"]["source_commit"] != manifest["source_commit"]:
            errors.append(f"{location}.source_commit: differs from the manifest")
        if trial["identity"]["dirty_worktree"] != manifest["worktree_dirty"]:
            errors.append(f"{location}.dirty_worktree: differs from the manifest")

        resources = trial["resources"]
        resource_digest = resources.get("raw_artifact_sha256")
        if resource_digest is not None:
            resource_observation = resource_observations.get(resource_digest)
            if resource_observation is None:
                errors.append(
                    f"{location}.resources.raw_artifact_sha256: "
                    "no verified resource observation artifact"
                )
            else:
                if resources["observer_id"] != resource_observation["observer_id"]:
                    errors.append(f"{location}.resources.observer_id: differs from its observation")
                if resources["sample_interval_ms"] != resource_observation["sample_interval_ms"]:
                    errors.append(
                        f"{location}.resources.sample_interval_ms: differs from its observation"
                    )
                trial_client = trial["client_identity"]
                if trial_client is None:
                    errors.append(
                        f"{location}.resources.raw_artifact_sha256: "
                        "resource evidence has no linked client process identity"
                    )
                elif resource_observation["root_pid"] != trial_client["process_id"]:
                    errors.append(
                        f"{location}.resources.raw_artifact_sha256: observer root PID "
                        "differs from the client process"
                    )
                if trial_client is not None and resource_observation[
                    "root_process_start_ticks"
                ] != trial_client["process_start_ticks"]:
                    errors.append(
                        f"{location}.resources.raw_artifact_sha256: observer root process "
                        "start identity differs from the client process"
                    )
                resource_fields = {
                    "rss_peak_bytes": "rss_bytes",
                    "pss_peak_bytes": "pss_bytes",
                    "cgroup_anon_peak_bytes": "cgroup_anon_bytes",
                    "cgroup_file_peak_bytes": "cgroup_file_bytes",
                    "cgroup_reclaimable_cache_peak_bytes": "cgroup_reclaimable_cache_bytes",
                    "cgroup_current_peak_bytes": "cgroup_current_bytes",
                    "cpu_time_ns": "cpu_time_ns",
                    "threads_peak": "threads",
                    "fds_peak": "fds",
                    "block_read_bytes": "block_read_bytes",
                    "block_write_bytes": "block_write_bytes",
                    "network_rx_bytes": "network_rx_bytes",
                    "network_tx_bytes": "network_tx_bytes",
                }
                for trial_field, observation_field in resource_fields.items():
                    observed_value = (
                        resource_observation["peaks"][observation_field]
                        if observation_field in resource_observation["peaks"]
                        else resource_observation[observation_field]
                    )
                    if resources[trial_field] != observed_value:
                        errors.append(
                            f"{location}.resources.{trial_field}: differs from its observation"
                        )

        copy_accounting = trial["payload"].get("copy_accounting")
        if copy_accounting is not None:
            trial_client = trial["client_identity"]
            if trial_client is None:
                errors.append(
                    f"{location}.payload.copy_accounting: client process identity is missing"
                )
            elif trial_client["client_id"] != "torrentngd":
                errors.append(
                    f"{location}.payload.copy_accounting: this observation format is TorrentNG-specific"
                )
            start_sha = copy_accounting["start_observation_sha256"]
            end_sha = copy_accounting["end_observation_sha256"]
            if start_sha == end_sha:
                errors.append(
                    f"{location}.payload.copy_accounting: start and end observations must differ"
                )
            start_copy = payload_copy_observations.get(start_sha)
            end_copy = payload_copy_observations.get(end_sha)
            if start_copy is None:
                errors.append(
                    f"{location}.payload.copy_accounting.start_observation_sha256: "
                    "no verified payload-copy observation artifact"
                )
            if end_copy is None:
                errors.append(
                    f"{location}.payload.copy_accounting.end_observation_sha256: "
                    "no verified payload-copy observation artifact"
                )
            if start_copy is not None and end_copy is not None and trial_client is not None:
                client = trial_client
                for label, observation in (("start", start_copy), ("end", end_copy)):
                    if observation["process_id"] != client["process_id"]:
                        errors.append(
                            f"{location}.payload.copy_accounting.{label}_observation_sha256: "
                            "process ID differs from the client identity"
                        )
                    if observation["process_start_ticks"] != client["process_start_ticks"]:
                        errors.append(
                            f"{location}.payload.copy_accounting.{label}_observation_sha256: "
                            "process start identity differs from the client identity"
                        )
                for counter, end_value in end_copy["counters_bytes"].items():
                    start_value = start_copy["counters_bytes"][counter]
                    if end_value < start_value:
                        errors.append(
                            f"{location}.payload.copy_accounting: {counter} counter reset during trial"
                        )
                    elif copy_accounting["copied_bytes"][counter] != end_value - start_value:
                        errors.append(
                            f"{location}.payload.copy_accounting.copied_bytes.{counter}: "
                            "does not match the start/end counter difference"
                        )

        environment = trial["environment"]
        network = environment["network"]
        ceiling_digest = environment.get("independent_ceiling_sha256")
        ceiling_calibration: dict[str, Any] | None = None
        if ceiling_digest is not None:
            report = ceiling_reports.get(ceiling_digest)
            if report is None:
                errors.append(
                    f"{location}.environment.independent_ceiling_sha256: no verified ceiling report"
                )
            else:
                report_environment = report["environment"]
                storage = environment["storage"]
                if report_environment.get("host_profile") != environment["host_profile"]:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: host profile differs")
                if report_environment.get("os") != environment["os"]:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: OS differs")
                if report_environment.get("kernel") != environment["kernel"]:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: kernel differs")
                if report_environment.get("cpu_model") != environment["cpu_model"]:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: CPU differs")
                if report_environment.get("filesystem") != storage["filesystem"]:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: filesystem differs")
                if storage["device_id"] is None or report_environment.get("device_id") != storage["device_id"]:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: storage device differs or is unknown")
                control_profile = network.get("control_profile_id")
                trial_network_profile = network.get("network_profile_id") or control_profile
                if trial_network_profile is None:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: network profile is unknown")
                elif report_environment.get("network_profile_id") != trial_network_profile:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: network profile differs")
                workload_protocol = trial["workload"].get("protocol")
                if workload_protocol is None:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: workload protocol is unknown")
                elif report["protocol"] != workload_protocol:
                    errors.append(f"{location}.environment.independent_ceiling_sha256: protocol differs")
                calibration = ceiling_report_inputs.get(ceiling_digest)
                if calibration is not None:
                    ceiling_calibration = calibration
                    target_identity = trial["client_identity"]
                    if target_identity is None:
                        errors.append(
                            f"{location}.environment.independent_ceiling_sha256: "
                            "client process identity is missing"
                        )
                    else:
                        for source_name in ("network", "disk", "hash", "pipeline"):
                            source = calibration[source_name]
                            if (
                                source["process_id"] == target_identity["process_id"]
                                and source["process_start_ticks"] == target_identity["process_start_ticks"]
                            ):
                                errors.append(
                                    f"{location}.environment.independent_ceiling_sha256: "
                                    f"{source_name} calibration uses the candidate process"
                                )
        control_profile = network.get("control_profile_id")
        control_digests = network.get("control_observation_sha256s", [])
        if control_profile is None and control_digests:
            errors.append(
                f"{location}.environment.network.control_profile_id: required when control artifacts are linked"
            )
        if control_profile is not None:
            expected_profile = NETWORK_PROFILES.get(control_profile)
            if expected_profile is None:
                errors.append(f"{location}.environment.network.control_profile_id: unknown profile")
            elif network["link_limit_bps"] != expected_profile["rate_bps"]:
                errors.append(
                    f"{location}.environment.network.link_limit_bps: differs from the controlled profile"
                )
            for digest in control_digests:
                control = network_observations.get(digest)
                if control is None:
                    errors.append(
                        f"{location}.environment.network.control_observation_sha256s: "
                        "no verified network-control artifact"
                    )
                elif control["action"] != "apply":
                    errors.append(
                        f"{location}.environment.network.control_observation_sha256s: "
                        "trial must reference apply reports"
                    )
                elif control["profile_id"] != control_profile:
                    errors.append(
                        f"{location}.environment.network.control_observation_sha256s: "
                        "profile differs from the trial"
                    )
                elif trial["outcome"]["status"] == "complete" and digest not in reset_network_reports:
                    errors.append(
                        f"{location}.environment.network.control_observation_sha256s: "
                        "complete trial has no verified qdisc reset report"
                    )
        cache_state = environment["cache_state"]
        cache_sha = environment.get("cache_state_observation_sha256")
        if cache_state in {"cold", "warm"} or cache_sha is not None:
            cache_observation = cache_observations.get(cache_sha)
            if cache_observation is None:
                errors.append(
                    f"{location}.environment.cache_state_observation_sha256: no verified cache-state artifact"
                )
            else:
                if cache_observation["fixture_sha256"] != trial["workload"]["fixture_sha256"]:
                    errors.append(
                        f"{location}.environment.cache_state_observation_sha256: fixture identity differs"
                    )
                if cache_observation["requested_state"] != cache_state:
                    errors.append(
                        f"{location}.environment.cache_state_observation_sha256: requested state differs"
                    )
                if cache_state in {"cold", "warm"} and cache_observation["status"] != "achieved":
                    errors.append(
                        f"{location}.environment.cache_state_observation_sha256: cache state was not achieved"
                    )

        client_identity = trial["client_identity"]
        if client_identity is None:
            if trial["outcome"]["status"] != "unmeasured":
                errors.append(
                    f"{location}.client_identity: only an unmeasured trial may omit client identity"
                )
            continue
        observation = client_observations.get(client_identity["observation_sha256"])
        if observation is None:
            errors.append(
                f"{location}.client_identity.observation_sha256: no verified client observation artifact"
            )
            continue
        if ceiling_digest is not None and ceiling_calibration is not None:
            pipeline = ceiling_calibration["pipeline"]
            if pipeline["client_id"] != client_identity["client_id"]:
                errors.append(
                    f"{location}.environment.independent_ceiling_sha256: pipeline client differs"
                )
            if pipeline["client_version"] != client_identity["version"]:
                errors.append(
                    f"{location}.environment.independent_ceiling_sha256: pipeline client version differs"
                )
            if pipeline["settings_sha256"] != client_identity["settings_sha256"]:
                errors.append(
                    f"{location}.environment.independent_ceiling_sha256: pipeline settings digest differs"
                )
            if pipeline["peer_count"] != trial["workload"].get("peer_count"):
                errors.append(
                    f"{location}.environment.independent_ceiling_sha256: pipeline peer count differs"
                )
            if pipeline["block_bytes"] != trial["workload"].get("block_bytes"):
                errors.append(
                    f"{location}.environment.independent_ceiling_sha256: pipeline block size differs"
                )
            try:
                request_count = _resolve_json_pointer(
                    observation["settings_readback"]["values"],
                    pipeline["settings_json_pointer"],
                )
            except ValueError as error:
                errors.append(
                    f"{location}.environment.independent_ceiling_sha256: {error}"
                )
            else:
                if (
                    isinstance(request_count, bool)
                    or not isinstance(request_count, int)
                    or request_count != pipeline["requests_per_peer"]
                ):
                    errors.append(
                        f"{location}.environment.independent_ceiling_sha256: "
                        "pipeline request count differs from settings readback"
                    )
        runtime_identity = observation["runtime_identity"]
        if client_identity["client_id"] != observation["client_id"]:
            errors.append(f"{location}.client_identity.client_id: differs from its observation")
        if client_identity["version"] != observation["version"]:
            errors.append(f"{location}.client_identity.version: differs from its observation")
        if client_identity["runtime_kind"] != runtime_identity["kind"]:
            errors.append(f"{location}.client_identity.runtime_kind: differs from its observation")
        if client_identity["process_id"] != runtime_identity["process_id"]:
            errors.append(f"{location}.client_identity.process_id: differs from its observation")
        if client_identity["process_start_ticks"] != runtime_identity["process_start_ticks"]:
            errors.append(f"{location}.client_identity.process_start_ticks: differs from its observation")
        if client_identity["settings_sha256"] != observation["settings_readback"]["sha256"]:
            errors.append(f"{location}.client_identity.settings_sha256: differs from its observation")

    # Validate the remaining artifact file references and digests as before.
    for index, record in enumerate(manifest.get("artifacts", [])):
        if record["kind"] in {
            CLIENT_OBSERVATION_KIND,
            CACHE_STATE_KIND,
            NETWORK_CONTROL_KIND,
            RESOURCE_OBSERVATION_KIND,
            RESOURCE_SAMPLES_KIND,
        }:
            continue
        location = f"$.artifacts[{index}]"
        path, path_error = _referenced_file(root, record["path"])
        if path_error is not None or path is None:
            errors.append(f"{location}.path: {path_error}")
            continue
        try:
            observed = _sha256(path)
        except OSError as error:
            errors.append(f"{location}.path: cannot read referenced file: {error}")
            continue
        if observed != record["sha256"]:
            errors.append(f"{location}.sha256: digest does not match {record['path']}")
    return errors


def validate(
    kind: str,
    artifact_path: Path,
    *,
    verify_referenced_files: bool = False,
) -> list[str]:
    schema = _load_json(SCHEMAS[kind])
    artifact = (
        _load_strict_json(artifact_path)
        if kind in {"paired-plan", "compatibility-preflight"}
        else _load_json(artifact_path)
    )
    try:
        Draft202012Validator.check_schema(schema)
    except SchemaError as error:
        raise ValueError(f"{SCHEMAS[kind]}: invalid schema: {error.message}") from error
    validator = Draft202012Validator(schema, format_checker=FormatChecker())
    errors = sorted(
        validator.iter_errors(artifact),
        key=lambda error: (list(map(str, error.absolute_path)), error.message),
    )
    formatted = [f"{_format_path(error)}: {error.message}" for error in errors]
    if kind == "client-observation" and not formatted and isinstance(artifact, dict):
        formatted.extend(f"$.{error}" for error in _client_observation_errors(artifact))
    if kind == "resource-observation" and not formatted and isinstance(artifact, dict):
        formatted.extend(f"$.{error}" for error in _resource_observation_errors(artifact))
    if kind == "cache-state" and not formatted and isinstance(artifact, dict):
        formatted.extend(f"$.{error}" for error in _cache_state_errors(artifact))
    if kind == "network-control" and not formatted and isinstance(artifact, dict):
        formatted.extend(f"$.{error}" for error in _network_control_errors(artifact))
    if kind == "paired-plan" and not formatted:
        transfer_tools = str(ROOT / "scripts/transfer_rebuild")
        if transfer_tools not in sys.path:
            sys.path.insert(0, transfer_tools)
        from paired_plan import paired_plan_errors

        formatted.extend(f"$.{error}" for error in paired_plan_errors(artifact))
    if kind == "compatibility-preflight" and not formatted and isinstance(artifact, dict):
        formatted.extend(f"$.{error}" for error in _compatibility_preflight_errors(artifact))
    if (
        verify_referenced_files
        and kind == "manifest"
        and not formatted
        and isinstance(artifact, dict)
    ):
        formatted.extend(_verify_manifest_files(artifact_path, artifact))
    return formatted


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("kind", choices=sorted(SCHEMAS))
    parser.add_argument("artifact", type=Path)
    args = parser.parse_args(argv)

    try:
        errors = validate(
            args.kind,
            args.artifact,
            verify_referenced_files=args.kind == "manifest",
        )
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 2

    if errors:
        print(f"{args.artifact}: invalid {args.kind} ({len(errors)} validation errors)")
        for error in errors[:100]:
            print(f"  {error}")
        if len(errors) > 100:
            print(f"  … {len(errors) - 100} additional errors omitted")
        return 1

    print(f"{args.artifact}: valid {args.kind}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
