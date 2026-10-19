#!/usr/bin/env python3
"""Build a reproducible randomized schedule for the 28 PERF-110 slots.

This writes a plan only. PERF-110's process launcher still has to consume the
schedule, run each client against a shared fixture, and produce trial records.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import sys
from pathlib import Path
from typing import Any

from adapters import AdapterError, CLIENT_IDS, load_pins_document
from fixtures import load_fixture_profile_document
from strict_json import StrictJsonError, loads as strict_json_loads


PLAN_SCHEMA_VERSION = "transfer-paired-plan-v1"
SCHEDULE_ALGORITHM = "sha256-fisher-yates-v1"
SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_PROFILE_PATH = SCRIPT_DIR / "fixture_profiles.json"
MAX_INPUT_BYTES = 8 * 1024 * 1024
EXPECTED_CLIENT_IDS = (
    "deluge",
    "qbittorrent",
    "rtorrent",
    "torrentngd",
    "transmission",
    "rqbit",
    "urtorrent",
)
EXPECTED_PROFILE_IDS = (
    "G3/hybrid-padding",
    "G3/v2-piece-layer",
    "N1/lan_1g",
    "N1/lan_10g",
)
UNSUPPORTED_CLIENT_PROFILES = {
    ("urtorrent", "G3/hybrid-padding"): "the pinned urtorrent runner supports v1 only",
    ("urtorrent", "G3/v2-piece-layer"): "the pinned urtorrent runner supports v1 only",
}
MAX_REPETITIONS = 100
MAX_SEED = (1 << 64) - 1


class PlanError(ValueError):
    """A workload catalog or randomized paired schedule is invalid."""


def _canonical_json(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode(
        "utf-8"
    )


def _read_regular_file(path: Path, label: str) -> tuple[bytes, str]:
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW)
    except OSError as error:
        raise PlanError(f"cannot open {label} without following links") from error
    content = bytearray()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise PlanError(f"{label} must be a regular file")
        if before.st_size > MAX_INPUT_BYTES:
            raise PlanError(f"{label} exceeds the {MAX_INPUT_BYTES}-byte input limit")
        while len(content) <= MAX_INPUT_BYTES:
            chunk = os.read(descriptor, min(1024 * 1024, MAX_INPUT_BYTES + 1 - len(content)))
            if not chunk:
                break
            content.extend(chunk)
        after = os.fstat(descriptor)
    finally:
        os.close(descriptor)
    identity = lambda info: (
        info.st_dev,
        info.st_ino,
        info.st_size,
        info.st_mtime_ns,
        info.st_ctime_ns,
    )
    if (
        len(content) > MAX_INPUT_BYTES
        or identity(before) != identity(after)
        or len(content) != before.st_size
    ):
        raise PlanError(f"{label} changed while its digest was captured")
    value = bytes(content)
    return value, hashlib.sha256(value).hexdigest()


def _shuffle(values: list[Any], seed: int, counter: list[int]) -> None:
    """Use a versioned, Python-runtime-independent Fisher-Yates shuffle."""
    seed_bytes = seed.to_bytes(8, "big")
    space = 1 << 256
    for index in range(len(values) - 1, 0, -1):
        upper = index + 1
        limit = space - (space % upper)
        while True:
            material = (
                b"TorrentNG PERF-110 schedule\0"
                + seed_bytes
                + counter[0].to_bytes(8, "big")
            )
            counter[0] += 1
            choice = int.from_bytes(hashlib.sha256(material).digest(), "big")
            if choice < limit:
                selected = choice % upper
                break
        values[index], values[selected] = values[selected], values[index]


def build_case_catalog(profile_path: Path = DEFAULT_PROFILE_PATH) -> dict[str, Any]:
    """Create the fixed seven-client by four-profile catalog."""
    if set(CLIENT_IDS) != set(EXPECTED_CLIENT_IDS):
        raise PlanError("client adapter set changed; update the PERF-110 case catalog")
    profile_path = Path(profile_path)
    try:
        profiles_bytes, profiles_sha256 = _read_regular_file(profile_path, "fixture profile file")
        profiles_document = strict_json_loads(profiles_bytes, label="fixture profile file")
    except (OSError, json.JSONDecodeError, UnicodeDecodeError, StrictJsonError) as error:
        raise PlanError(f"cannot read fixture profile file: {error}") from error
    if (
        not isinstance(profiles_document, dict)
        or not isinstance(profiles_document.get("profiles"), dict)
        or set(profiles_document["profiles"]) != set(EXPECTED_PROFILE_IDS)
    ):
        raise PlanError("fixture profiles changed; update the PERF-110 case catalog")

    cases = []
    for profile_id in EXPECTED_PROFILE_IDS:
        try:
            profile = load_fixture_profile_document(profiles_document, profile_id)
        except ValueError as error:
            raise PlanError(f"invalid fixture profile {profile_id}: {error}") from error
        expected_payload_bytes = sum(file.size_bytes for file in profile["files"])
        for client_id in EXPECTED_CLIENT_IDS:
            unsupported_reason = UNSUPPORTED_CLIENT_PROFILES.get((client_id, profile_id))
            cases.append(
                {
                    "case_id": f"{client_id}:{profile_id}",
                    "client_id": client_id,
                    "profile_id": profile_id,
                    "kind": "download",
                    "protocol": profile["torrent_kind"],
                    "expected_payload_bytes": expected_payload_bytes,
                    "piece_length": profile["piece_length"],
                    "applicability": "unsupported" if unsupported_reason else "candidate",
                    "applicability_reason": unsupported_reason,
                }
            )

    cases.sort(key=lambda case: case["case_id"])
    catalog = {
        "schema_version": "transfer-workload-catalog-v1",
        "client_ids": list(EXPECTED_CLIENT_IDS),
        "profile_ids": list(EXPECTED_PROFILE_IDS),
        "profiles_sha256": profiles_sha256,
        "cases": cases,
    }
    catalog["catalog_sha256"] = hashlib.sha256(_canonical_json(catalog)).hexdigest()
    if len(cases) != 28:
        raise PlanError(f"PERF-110 catalog must contain 28 slots, found {len(cases)}")
    return catalog


def _validate_catalog(catalog: Any) -> None:
    required_fields = {
        "schema_version",
        "client_ids",
        "profile_ids",
        "profiles_sha256",
        "cases",
        "catalog_sha256",
    }
    if not isinstance(catalog, dict) or set(catalog) != required_fields:
        raise PlanError("paired plan requires a complete PERF-110 catalog")
    if (
        catalog["schema_version"] != "transfer-workload-catalog-v1"
        or catalog["client_ids"] != list(EXPECTED_CLIENT_IDS)
        or catalog["profile_ids"] != list(EXPECTED_PROFILE_IDS)
        or not isinstance(catalog["profiles_sha256"], str)
        or re.fullmatch(r"[0-9a-f]{64}", catalog["profiles_sha256"]) is None
    ):
        raise PlanError("paired plan catalog identity is invalid")

    claimed_digest = catalog["catalog_sha256"]
    if not isinstance(claimed_digest, str) or re.fullmatch(r"[0-9a-f]{64}", claimed_digest) is None:
        raise PlanError("paired plan catalog digest is invalid")
    digest_input = {key: value for key, value in catalog.items() if key != "catalog_sha256"}
    if hashlib.sha256(_canonical_json(digest_input)).hexdigest() != claimed_digest:
        raise PlanError("paired plan catalog digest does not match its contents")

    cases = catalog["cases"]
    if not isinstance(cases, list) or len(cases) != 28:
        raise PlanError("paired plan requires the complete 28-slot PERF-110 catalog")
    expected_slots = {
        (client_id, profile_id)
        for client_id in EXPECTED_CLIENT_IDS
        for profile_id in EXPECTED_PROFILE_IDS
    }
    seen_slots: set[tuple[str, str]] = set()
    profile_geometry: dict[str, tuple[str, int, int]] = {}
    for case in cases:
        if not isinstance(case, dict) or set(case) != {
            "case_id",
            "client_id",
            "profile_id",
            "kind",
            "protocol",
            "expected_payload_bytes",
            "piece_length",
            "applicability",
            "applicability_reason",
        }:
            raise PlanError("paired plan catalog contains a malformed case")
        client_id = case["client_id"]
        profile_id = case["profile_id"]
        if not isinstance(client_id, str) or not isinstance(profile_id, str):
            raise PlanError("paired plan catalog case identity is invalid")
        slot = (client_id, profile_id)
        if slot not in expected_slots or slot in seen_slots:
            raise PlanError("paired plan catalog has an unknown or duplicate case slot")
        seen_slots.add(slot)
        if case["case_id"] != f"{client_id}:{profile_id}" or case["kind"] != "download":
            raise PlanError("paired plan catalog case identity is invalid")
        protocol = case["protocol"]
        payload_bytes = case["expected_payload_bytes"]
        piece_length = case["piece_length"]
        if (
            not isinstance(protocol, str)
            or protocol not in {"v1", "v2", "hybrid"}
            or isinstance(payload_bytes, bool)
            or not isinstance(payload_bytes, int)
            or payload_bytes < 0
            or isinstance(piece_length, bool)
            or not isinstance(piece_length, int)
            or piece_length < 16 * 1024
            or piece_length > 16 * 1024 * 1024
            or piece_length & (piece_length - 1)
        ):
            raise PlanError("paired plan catalog workload geometry is invalid")
        geometry = (protocol, payload_bytes, piece_length)
        prior_geometry = profile_geometry.setdefault(profile_id, geometry)
        if prior_geometry != geometry:
            raise PlanError("paired plan catalog workload differs within a fixture profile")
        unsupported_reason = UNSUPPORTED_CLIENT_PROFILES.get(slot)
        if unsupported_reason is None:
            if case["applicability"] != "candidate" or case["applicability_reason"] is not None:
                raise PlanError("paired plan catalog has an unsupported compatibility claim")
        elif (
            case["applicability"] != "unsupported"
            or case["applicability_reason"] != unsupported_reason
        ):
            raise PlanError("paired plan catalog has an unsupported compatibility claim")
    if seen_slots != expected_slots:
        raise PlanError("paired plan catalog is missing case slots")


def build_paired_plan(
    catalog: dict[str, Any],
    pinned_client_ids: set[str],
    *,
    repetitions: int,
    seed: int,
    pins_sha256: str,
    profile_scope: list[str] | None = None,
) -> dict[str, Any]:
    """Randomize profile-pair order and client order using one recorded seed."""
    if isinstance(repetitions, bool) or not isinstance(repetitions, int):
        raise PlanError("repetitions must be an integer")
    if not 1 <= repetitions <= MAX_REPETITIONS:
        raise PlanError(f"repetitions must be within 1..={MAX_REPETITIONS}")
    if isinstance(seed, bool) or not isinstance(seed, int) or not 0 <= seed <= MAX_SEED:
        raise PlanError("seed must be an unsigned 64-bit integer")
    if not isinstance(pins_sha256, str) or re.fullmatch(r"[0-9a-f]{64}", pins_sha256) is None:
        raise PlanError("pins_sha256 must be a 64-character lowercase hex digest")
    if (
        not isinstance(pinned_client_ids, set)
        or not pinned_client_ids
        or any(not isinstance(client_id, str) for client_id in pinned_client_ids)
        or not pinned_client_ids <= set(EXPECTED_CLIENT_IDS)
    ):
        raise PlanError("at least one known pinned client is required")
    _validate_catalog(catalog)
    catalog_cases = catalog["cases"]
    available_profiles = catalog["profile_ids"]
    scoped = profile_scope is not None
    if profile_scope is None:
        selected_profiles = list(available_profiles)
    else:
        if (
            not isinstance(profile_scope, list)
            or not profile_scope
            or any(not isinstance(profile_id, str) for profile_id in profile_scope)
            or len(profile_scope) != len(set(profile_scope))
            or any(profile_id not in available_profiles for profile_id in profile_scope)
        ):
            raise PlanError("profile scope must select unique catalog profiles")
        selected_set = set(profile_scope)
        selected_profiles = [profile_id for profile_id in available_profiles if profile_id in selected_set]

    excluded_cases = []
    eligible_by_profile: dict[str, list[str]] = {profile: [] for profile in EXPECTED_PROFILE_IDS}
    for case in catalog_cases:
        client_id = case["client_id"]
        profile_id = case["profile_id"]
        if case["applicability"] == "unsupported":
            excluded_cases.append(
                {
                    "case_id": case["case_id"],
                    "reason": case["applicability_reason"],
                }
            )
        elif client_id not in pinned_client_ids:
            excluded_cases.append(
                {"case_id": case["case_id"], "reason": "client is not present in the pin file"}
            )
        else:
            eligible_by_profile[profile_id].append(client_id)

    identity = {
        "catalog_sha256": catalog["catalog_sha256"],
        "pins_sha256": pins_sha256,
        "repetitions": repetitions,
        "seed": seed,
        "schedule_algorithm": SCHEDULE_ALGORITHM,
    }
    if scoped:
        identity["profile_scope"] = selected_profiles
    plan_id = hashlib.sha256(_canonical_json(identity)).hexdigest()[:20]
    shuffle_counter = [0]
    pair_groups = [
        (profile_id, repetition)
        for repetition in range(1, repetitions + 1)
        for profile_id in selected_profiles
    ]
    _shuffle(pair_groups, seed, shuffle_counter)

    schedule = []
    excluded_pairs = []
    execution_index = 0
    for pair_order, (profile_id, repetition) in enumerate(pair_groups, start=1):
        clients = list(eligible_by_profile[profile_id])
        if len(clients) < 2:
            excluded_pairs.append(
                {
                    "profile_id": profile_id,
                    "repetition": repetition,
                    "reason": "fewer than two pinned compatible clients",
                }
            )
            continue
        _shuffle(clients, seed, shuffle_counter)
        pair_id = f"{plan_id}:{profile_id}:rep-{repetition:03d}"
        for order_in_pair, client_id in enumerate(clients, start=1):
            execution_index += 1
            schedule.append(
                {
                    "execution_index": execution_index,
                    "pair_order": pair_order,
                    "order_in_pair": order_in_pair,
                    "trial_id": f"{pair_id}:{client_id}:attempt-001",
                    "pair_id": pair_id,
                    "attempt": 1,
                    "repetition": repetition,
                    "case_id": f"{client_id}:{profile_id}",
                    "client_id": client_id,
                    "profile_id": profile_id,
                }
            )

    plan = {
        "schema_version": PLAN_SCHEMA_VERSION,
        "plan_id": plan_id,
        **identity,
        "catalog": catalog,
        "pinned_client_ids": sorted(pinned_client_ids),
        "excluded_cases": excluded_cases,
        "excluded_pairs": excluded_pairs,
        "schedule": schedule,
    }
    if scoped:
        plan["profile_scope"] = selected_profiles
    return plan


def paired_plan_errors(plan: Any) -> list[str]:
    """Check that a stored plan is the exact schedule implied by its identity."""
    if not isinstance(plan, dict):
        return ["paired plan must be an object"]
    try:
        pinned_client_ids = plan["pinned_client_ids"]
        if not isinstance(pinned_client_ids, list):
            return ["pinned_client_ids must be an array"]
        expected = build_paired_plan(
            plan["catalog"],
            set(pinned_client_ids),
            repetitions=plan["repetitions"],
            seed=plan["seed"],
            pins_sha256=plan["pins_sha256"],
            profile_scope=plan.get("profile_scope"),
        )
    except (KeyError, TypeError, PlanError) as error:
        return [f"paired plan identity is invalid: {error}"]
    if plan != expected:
        return ["paired plan fields or schedule differ from the deterministic identity"]
    return []


def write_new_plan(path: Path, plan: dict[str, Any]) -> None:
    path = Path(path)
    if path.exists() or path.is_symlink():
        raise PlanError(f"refusing to overwrite paired plan: {path}")
    parent = path.parent
    if parent.is_symlink() or not parent.is_dir():
        raise PlanError("paired plan output parent must be an existing non-symlink directory")
    try:
        descriptor = os.open(
            path,
            os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW,
            0o600,
        )
    except OSError as error:
        raise PlanError(f"cannot create paired plan {path}: {error}") from error
    with os.fdopen(descriptor, "w", encoding="utf-8") as output:
        output.write(json.dumps(plan, sort_keys=True, indent=2, ensure_ascii=False) + "\n")
        output.flush()
        os.fsync(output.fileno())
    directory_flags = os.O_RDONLY | os.O_CLOEXEC | os.O_NOFOLLOW
    if hasattr(os, "O_DIRECTORY"):
        directory_flags |= os.O_DIRECTORY
    directory_descriptor = os.open(parent, directory_flags)
    try:
        os.fsync(directory_descriptor)
    finally:
        os.close(directory_descriptor)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--pins", type=Path, required=True)
    parser.add_argument("--profiles", type=Path, default=DEFAULT_PROFILE_PATH)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--repetitions", type=int, default=10)
    parser.add_argument("--seed", type=int, required=True)
    parser.add_argument(
        "--profile-scope",
        action="append",
        choices=EXPECTED_PROFILE_IDS,
        help="run only selected fixture profiles; repeat to select more than one",
    )
    args = parser.parse_args(argv)
    try:
        pins_bytes, pins_sha256 = _read_regular_file(args.pins, "client pin file")
        try:
            pins_document = strict_json_loads(pins_bytes, label="client pin file")
        except (json.JSONDecodeError, UnicodeDecodeError, StrictJsonError) as error:
            raise PlanError(f"cannot read client pin file: {error}") from error
        pinned_clients = load_pins_document(pins_document)
        catalog = build_case_catalog(args.profiles)
        plan = build_paired_plan(
            catalog,
            set(pinned_clients),
            repetitions=args.repetitions,
            seed=args.seed,
            pins_sha256=pins_sha256,
            profile_scope=args.profile_scope,
        )
        plan_errors = paired_plan_errors(plan)
        if plan_errors:
            raise PlanError("generated plan failed validation: " + "; ".join(plan_errors))
        write_new_plan(args.output, plan)
    except (AdapterError, OSError, PlanError, ValueError) as error:
        print(f"paired plan error: {error}", file=sys.stderr)
        return 2
    print(
        f"wrote {len(plan['schedule'])} randomized trial slots across "
        f"{len(plan['excluded_pairs'])} excluded pairs to {args.output}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
