#!/usr/bin/env python3
"""Evaluate source-bound transfer-core rollout evidence without performance guesswork."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

try:
    from jsonschema import Draft202012Validator, FormatChecker
    from strict_json import StrictJsonError, loads as strict_json_loads
    from trial_record import _open_directory_tree_nofollow
    from validate_artifact import SCHEMAS, _format_path, _referenced_file, validate
except ImportError as error:  # pragma: no cover - exercised by the CLI environment
    raise SystemExit("rollout_gate.py must run from its scripts directory") from error


MAX_JSON_BYTES = 16 * 1024 * 1024
STAGES = ("IO-016", "NET-116", "PERF-116", "SYS-015")
REQUIRED_KINDS: dict[str, frozenset[str]] = {
    "IO-016": frozenset(
        {
            "transfer-live-parity-v1",
            "transfer-storage-qualification-v1",
            "transfer-recovery-qualification-v1",
            "transfer-gate-policy-v1",
        }
    ),
    "NET-116": frozenset(
        {
            "transfer-live-parity-v1",
            "transfer-simulator-qualification-v1",
            "transfer-performance-policy-v1",
        }
    ),
    "PERF-116": frozenset(
        {
            "transfer-soak-qualification-v1",
            "transfer-canary-qualification-v1",
            "transfer-recovery-qualification-v1",
            "transfer-gate-policy-v1",
        }
    ),
    "SYS-015": frozenset(
        {
            "transfer-release-bundle-v1",
            "transfer-catalog-load-v1",
            "transfer-gate-policy-v1",
        }
    ),
}
REQUIRED_PROTOCOLS = ("v1", "v2", "hybrid")
REQUIRED_WORKLOADS = ("download", "seed")
REQUIRED_METRICS = frozenset(
    {"goodput_bytes_per_second", "rss_peak_bytes", "cpu_seconds_per_gib"}
)
REQUIRED_CHECKS: dict[str, frozenset[str]] = {
    "transfer-live-parity-v1": frozenset(
        {"live-v1-parity", "live-v2-parity", "live-hybrid-v1-v2-parity"}
    ),
    "transfer-storage-qualification-v1": frozenset(
        {"storage-fault-matrix", "real-device-workload", "bounded-resource-ownership"}
    ),
    "transfer-recovery-qualification-v1": frozenset(
        {"crash-cut-recovery", "resume-restoration", "one-writer-recovery"}
    ),
    "transfer-gate-policy-v1": frozenset(
        {"performance-thresholds-explicitly-unapplied"}
    ),
    "transfer-simulator-qualification-v1": frozenset(
        {"seeded-invariant-replay", "hybrid-race-replay", "generation-reuse-replay"}
    ),
    "transfer-performance-policy-v1": frozenset(
        {
            "matched-whole-client-metrics",
            "candidate-and-baseline-paired",
            "thresholds-explicitly-unapplied",
        }
    ),
    "transfer-soak-qualification-v1": frozenset(
        {"soak-completed", "integrity-preserved", "resource-accounting-complete"}
    ),
    "transfer-canary-qualification-v1": frozenset(
        {"candidate-cutover", "reference-rollback", "single-writer-throughout"}
    ),
    "transfer-release-bundle-v1": frozenset(
        {"clean-source-rebuild", "artifact-provenance", "release-notes-and-bundle"}
    ),
    "transfer-catalog-load-v1": frozenset(
        {
            "production-catalog-persist-restore",
            "mixed-native-api-latencies-retained",
            "host-and-source-identity-recorded",
        }
    ),
}
REQUIRED_PROOF_KINDS: dict[str, dict[str, str]] = {
    "transfer-catalog-load-v1": {
        "production-catalog-persist-restore": "transfer-catalog-load-report-v1",
        "mixed-native-api-latencies-retained": "transfer-catalog-load-report-v1",
        "host-and-source-identity-recorded": "transfer-catalog-load-report-v1",
    }
}


class RolloutGateError(ValueError):
    """The supplied rollout inputs cannot be safely evaluated."""


def _proof_kind_errors(
    qualification_kind: str,
    check: dict[str, Any],
    artifact_index: dict[str, tuple[str, str]],
) -> list[str]:
    expected_kind = REQUIRED_PROOF_KINDS.get(qualification_kind, {}).get(check["id"])
    if expected_kind is None:
        return []
    errors: list[str] = []
    for digest in check["evidence_sha256s"]:
        record = artifact_index.get(digest)
        if record is not None and record[0] != expected_kind:
            errors.append(
                f"{qualification_kind}: check {check['id']} requires {expected_kind} proof, "
                f"not {record[0]}"
            )
    return errors


def _read_json_artifact(path: Path, label: str) -> tuple[object, str]:
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise RolloutGateError(f"cannot open {label}: {error}") from error
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise RolloutGateError(f"{label} must be a regular file")
        if before.st_size > MAX_JSON_BYTES:
            raise RolloutGateError(f"{label} exceeds {MAX_JSON_BYTES} bytes")
        content = bytearray()
        digest = hashlib.sha256()
        while chunk := os.read(descriptor, 1024 * 1024):
            content.extend(chunk)
            digest.update(chunk)
            if len(content) > MAX_JSON_BYTES:
                raise RolloutGateError(f"{label} exceeds {MAX_JSON_BYTES} bytes")
        after = os.fstat(descriptor)
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_dev,
            after.st_ino,
            after.st_size,
            after.st_mtime_ns,
            after.st_ctime_ns,
        ):
            raise RolloutGateError(f"{label} changed while it was read")
    finally:
        os.close(descriptor)
    try:
        value = strict_json_loads(bytes(content), label=label)
    except StrictJsonError as error:
        raise RolloutGateError(f"{label} is not strict JSON: {error}") from error
    return value, digest.hexdigest()


def _schema_errors(kind: str, value: object) -> list[str]:
    try:
        schema = strict_json_loads(
            SCHEMAS[kind].read_bytes(),
            label=f"{kind} schema",
        )
        Draft202012Validator.check_schema(schema)
    except (OSError, StrictJsonError, KeyError) as error:
        raise RolloutGateError(f"cannot load {kind} schema: {error}") from error
    validator = Draft202012Validator(schema, format_checker=FormatChecker())
    return [
        f"{_format_path(error)}: {error.message}"
        for error in sorted(
            validator.iter_errors(value),
            key=lambda error: (list(map(str, error.absolute_path)), error.message),
        )
    ]


def _hash_regular_artifact(path: Path) -> str:
    flags = os.O_RDONLY | getattr(os, "O_CLOEXEC", 0) | getattr(os, "O_NOFOLLOW", 0)
    try:
        descriptor = os.open(path, flags)
    except OSError as error:
        raise RolloutGateError(f"cannot open manifest artifact {path}: {error}") from error
    digest = hashlib.sha256()
    try:
        before = os.fstat(descriptor)
        if not stat.S_ISREG(before.st_mode):
            raise RolloutGateError(f"manifest artifact {path} is not a regular file")
        while chunk := os.read(descriptor, 1024 * 1024):
            digest.update(chunk)
        after = os.fstat(descriptor)
        if (before.st_dev, before.st_ino, before.st_size, before.st_mtime_ns, before.st_ctime_ns) != (
            after.st_dev,
            after.st_ino,
            after.st_size,
            after.st_mtime_ns,
            after.st_ctime_ns,
        ):
            raise RolloutGateError(f"manifest artifact {path} changed while it was read")
    finally:
        os.close(descriptor)
    return digest.hexdigest()


def _artifact_index(manifest: dict[str, Any], root: Path) -> tuple[dict[str, tuple[str, str]], list[str]]:
    index: dict[str, tuple[str, str]] = {}
    errors: list[str] = []
    for position, record in enumerate(manifest.get("artifacts", [])):
        digest = record["sha256"]
        if digest in index:
            errors.append(f"manifest repeats artifact digest {digest}")
            continue
        path, error = _referenced_file(root, record["path"])
        if error is not None or path is None:
            errors.append(f"manifest artifact {record['path']}: {error}")
            continue
        observed = _hash_regular_artifact(path)
        if observed != digest:
            errors.append(f"manifest artifact {record['path']} digest changed")
            continue
        index[digest] = (record["kind"], record["path"])
    return index, errors


def _candidate_trial_errors(
    manifest_path: Path,
    manifest: dict[str, Any],
    candidate: str,
    minimum_pairs: int,
) -> list[str]:
    errors: list[str] = []
    coverage = {
        (protocol, workload): set()
        for protocol in REQUIRED_PROTOCOLS
        for workload in REQUIRED_WORKLOADS
    }
    for position, record in enumerate(manifest.get("trial_files", [])):
        path, path_error = _referenced_file(manifest_path.parent, record["path"])
        if path_error is not None or path is None:
            errors.append(f"trial {record['path']}: {path_error}")
            continue
        try:
            trial, digest = _read_json_artifact(path, f"trial {record['path']}")
        except RolloutGateError as error:
            errors.append(str(error))
            continue
        if digest != record["sha256"] or not isinstance(trial, dict):
            errors.append(f"trial {record['path']} digest or shape is invalid")
            continue
        if trial.get("client_identity") is None or trial["client_identity"].get("client_id") != candidate:
            continue
        if trial.get("identity", {}).get("source_commit") != manifest["source_commit"]:
            errors.append(f"candidate trial {record['path']} belongs to a different source commit")
            continue
        if trial.get("identity", {}).get("dirty_worktree") is not False:
            errors.append(f"candidate trial {record['path']} was produced from a dirty worktree")
        workload = trial.get("workload", {})
        protocol = workload.get("protocol")
        kind = workload.get("kind")
        if protocol not in REQUIRED_PROTOCOLS or kind not in REQUIRED_WORKLOADS:
            continue
        if trial.get("outcome", {}).get("status") != "complete":
            continue
        if trial.get("integrity", {}).get("status") != "passed":
            errors.append(f"candidate trial {record['path']} has no passed integrity result")
            continue
        payload = trial.get("payload", {})
        if payload.get("useful_bytes") != workload.get("expected_payload_bytes"):
            errors.append(f"candidate trial {record['path']} did not deliver the expected useful bytes")
            continue
        coverage[(protocol, kind)].add(
            (trial.get("pair_id", ""), workload.get("case_id", ""))
        )
    for key, pairs in sorted(coverage.items()):
        count = len(pairs)
        if count < minimum_pairs:
            errors.append(
                f"candidate coverage {key[0]}/{key[1]} has {count} complete trials; "
                f"requires {minimum_pairs}"
            )
    return errors


def _scorecard_errors(scorecard: dict[str, Any], candidate: str, manifest: dict[str, Any]) -> list[str]:
    errors: list[str] = []
    analysis = scorecard["analysis"]
    minimum = analysis["minimum_paired_release_trials"]
    if scorecard["manifest_id"] != manifest["manifest_id"]:
        errors.append("scorecard manifest_id differs from the source manifest")
    if scorecard["analysis"]["performance_thresholds_applied"] is not False:
        errors.append("scorecard must remain descriptive and apply no unapproved performance thresholds")
    groups = scorecard["groups"]
    for protocol in REQUIRED_PROTOCOLS:
        for workload in REQUIRED_WORKLOADS:
            eligible = [group for group in groups if group["protocol"] == protocol and group["kind"] == workload]
            if not eligible:
                errors.append(f"scorecard lacks a {protocol}/{workload} comparison group")
                continue
            for group in eligible:
                candidate_keys = {
                    client["client_key"]
                    for client in group["clients"]
                    if client["client_id"] == candidate
                }
                if not candidate_keys:
                    errors.append(f"scorecard {protocol}/{workload} group lacks candidate {candidate}")
                    continue
                if group["valid_pair_count"] < minimum:
                    errors.append(
                        f"scorecard {protocol}/{workload} has {group['valid_pair_count']} valid pairs; "
                        f"requires {minimum}"
                    )
                matched_candidate = False
                for comparison in group["comparisons"]:
                    if comparison["client_a"] not in candidate_keys and comparison["client_b"] not in candidate_keys:
                        continue
                    metrics = comparison["metrics"]
                    if REQUIRED_METRICS.issubset(metrics):
                        matched_candidate = True
                    for metric in REQUIRED_METRICS.intersection(metrics):
                        result = metrics[metric]
                        if not result["minimum_sample_met"] or result["paired_sample_count"] < minimum:
                            errors.append(
                                f"scorecard {protocol}/{workload} {metric} lacks {minimum} paired samples"
                            )
                if not matched_candidate:
                    errors.append(
                        f"scorecard {protocol}/{workload} has no candidate comparison with "
                        "goodput, RSS, and CPU metrics"
                    )
    return errors


def evaluate_rollout(
    *,
    manifest_path: Path,
    scorecard_path: Path,
    evidence_path: Path,
) -> dict[str, Any]:
    manifest_value, manifest_digest = _read_json_artifact(manifest_path, "manifest")
    if not isinstance(manifest_value, dict):
        raise RolloutGateError("manifest must be a JSON object")
    manifest_errors = validate("manifest", manifest_path, verify_referenced_files=True)
    if manifest_errors:
        raise RolloutGateError("invalid manifest: " + "; ".join(manifest_errors[:20]))
    _validated_manifest, validated_manifest_digest = _read_json_artifact(manifest_path, "manifest")
    if validated_manifest_digest != manifest_digest:
        raise RolloutGateError("manifest changed while its referenced files were validated")

    scorecard_value, scorecard_digest = _read_json_artifact(scorecard_path, "scorecard")
    scorecard_schema_errors = _schema_errors("scorecard", scorecard_value)
    if scorecard_schema_errors:
        raise RolloutGateError("invalid scorecard: " + "; ".join(scorecard_schema_errors[:20]))
    _validated_scorecard, validated_scorecard_digest = _read_json_artifact(scorecard_path, "scorecard")
    if validated_scorecard_digest != scorecard_digest:
        raise RolloutGateError("scorecard changed while it was validated")
    evidence_value, _evidence_digest = _read_json_artifact(evidence_path, "rollout evidence index")
    evidence_schema_errors = _schema_errors("rollout-evidence", evidence_value)
    if evidence_schema_errors:
        raise RolloutGateError("invalid rollout evidence index: " + "; ".join(evidence_schema_errors[:20]))

    assert isinstance(scorecard_value, dict) and isinstance(evidence_value, dict)
    candidate = evidence_value["candidate_client_id"]
    gate_stages: dict[str, dict[str, Any]] = {}
    reasons: dict[str, list[str]] = {stage: [] for stage in STAGES}
    index, index_errors = _artifact_index(manifest_value, manifest_path.parent)
    for error in index_errors:
        for stage in STAGES:
            reasons[stage].append(error)

    if manifest_value["worktree_dirty"] is not False:
        for stage in STAGES:
            reasons[stage].append("source manifest records a dirty worktree")
    if evidence_value["source_commit"] != manifest_value["source_commit"]:
        for stage in STAGES:
            reasons[stage].append("evidence index source_commit differs from the manifest")
    if evidence_value["manifest_sha256"] != manifest_digest:
        for stage in STAGES:
            reasons[stage].append("evidence index manifest_sha256 does not match the manifest bytes")
    if evidence_value["scorecard_sha256"] != scorecard_digest:
        for stage in STAGES:
            reasons[stage].append("evidence index scorecard_sha256 does not match the scorecard bytes")
    if scorecard_value["manifest_sha256"] != manifest_digest:
        for stage in STAGES:
            reasons[stage].append("scorecard manifest_sha256 does not match the manifest bytes")
    if scorecard_value["manifest_id"] != manifest_value["manifest_id"]:
        for stage in STAGES:
            reasons[stage].append("scorecard manifest_id differs from the manifest")

    minimum_pairs = scorecard_value["analysis"]["minimum_paired_release_trials"]
    trial_errors = _candidate_trial_errors(manifest_path, manifest_value, candidate, minimum_pairs)
    scorecard_errors = _scorecard_errors(scorecard_value, candidate, manifest_value)
    for error in trial_errors + scorecard_errors:
        reasons["NET-116"].append(error)
        reasons["PERF-116"].append(error)

    declared_stages = evidence_value["stages"]
    for stage in STAGES:
        declaration = declared_stages[stage]
        seen_kinds: set[str] = set()
        seen_digests: set[str] = set()
        for item in declaration["evidence"]:
            kind = item["kind"]
            digest = item["sha256"]
            if kind not in REQUIRED_KINDS[stage]:
                reasons[stage].append(f"unexpected evidence kind {kind}")
            if kind in seen_kinds or digest in seen_digests:
                reasons[stage].append(f"duplicate evidence entry for {kind}/{digest}")
            seen_kinds.add(kind)
            seen_digests.add(digest)
            record = index.get(digest)
            if record is None:
                reasons[stage].append(f"evidence digest {digest} is not a verified manifest artifact")
            elif record[0] != kind:
                reasons[stage].append(
                    f"evidence digest {digest} is manifest kind {record[0]}, not {kind}"
                )
            elif kind in REQUIRED_KINDS[stage]:
                evidence_path, path_error = _referenced_file(
                    manifest_path.parent, record[1]
                )
                if path_error is not None or evidence_path is None:
                    reasons[stage].append(f"{kind}: {path_error}")
                    continue
                try:
                    qualification, observed_digest = _read_json_artifact(
                        evidence_path, f"{kind} qualification"
                    )
                except RolloutGateError as error:
                    reasons[stage].append(str(error))
                    continue
                if observed_digest != digest:
                    reasons[stage].append(f"{kind}: qualification digest changed")
                    continue
                qualification_errors = _schema_errors("qualification", qualification)
                if qualification_errors:
                    reasons[stage].extend(
                        f"{kind}: {error}" for error in qualification_errors[:10]
                    )
                    continue
                if not isinstance(qualification, dict):
                    reasons[stage].append(f"{kind}: qualification must be an object")
                    continue
                if qualification["kind"] != kind:
                    reasons[stage].append(f"{kind}: qualification kind does not match its manifest record")
                if qualification["source_commit"] != manifest_value["source_commit"]:
                    reasons[stage].append(f"{kind}: qualification source commit differs from the manifest")
                if qualification["status"] != "passed":
                    reasons[stage].append(f"{kind}: qualification status is blocked")
                if not qualification["checks"] or any(
                    check["status"] != "passed" for check in qualification["checks"]
                ):
                    reasons[stage].append(f"{kind}: one or more qualification checks did not pass")
                check_ids = {check["id"] for check in qualification["checks"]}
                missing_checks = REQUIRED_CHECKS[kind] - check_ids
                if missing_checks:
                    reasons[stage].append(
                        f"{kind}: missing required checks: " + ", ".join(sorted(missing_checks))
                    )
                for check in qualification["checks"]:
                    reasons[stage].extend(
                        _proof_kind_errors(kind, check, index)
                    )
                    for proof_digest in check["evidence_sha256s"]:
                        if proof_digest == digest or proof_digest not in index:
                            reasons[stage].append(
                                f"{kind}: check {check['id']} references evidence outside the manifest"
                            )
                        elif index[proof_digest][0] in REQUIRED_CHECKS:
                            reasons[stage].append(
                                f"{kind}: check {check['id']} cites another qualification wrapper as raw proof"
                            )
        missing = REQUIRED_KINDS[stage] - seen_kinds
        if missing:
            reasons[stage].append("missing required evidence kinds: " + ", ".join(sorted(missing)))
        if declaration["status"] != "passed":
            reasons[stage].append("stage is declared blocked")

    for stage in STAGES:
        unique_reasons = sorted(set(reasons[stage]))
        gate_stages[stage] = {
            "status": "passed" if not unique_reasons else "blocked",
            "evidence_sha256s": sorted(
                {item["sha256"] for item in declared_stages[stage]["evidence"]}
            ),
            "reasons": unique_reasons,
        }

    decision = (
        "release_eligible"
        if all(gate_stages[stage]["status"] == "passed" for stage in STAGES)
        else "blocked"
    )
    return {
        "schema_version": "transfer-rollout-gate-v1",
        "evaluated_at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
        "source_commit": manifest_value["source_commit"],
        "manifest_sha256": manifest_digest,
        "scorecard_sha256": scorecard_digest,
        "candidate_client_id": candidate,
        "decision": decision,
        "stages": gate_stages,
    }


def _write_exclusive(path: Path, value: dict[str, Any]) -> None:
    data = (json.dumps(value, sort_keys=True, indent=2, allow_nan=False) + "\n").encode()
    path = Path(path)
    if not path.is_absolute() or ".." in path.parts or not path.name:
        raise RolloutGateError("gate output path must be absolute and normalized")
    repo_root = Path(__file__).resolve().parents[2]
    try:
        path.relative_to(repo_root)
    except ValueError:
        pass
    else:
        raise RolloutGateError("gate output must be outside the source worktree")
    try:
        parent_fd = _open_directory_tree_nofollow(path.parent)
    except OSError as error:
        raise RolloutGateError("gate output parent must be an existing no-symlink directory") from error
    flags = os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC | os.O_NOFOLLOW
    descriptor: int | None = None
    created_identity: tuple[int, int] | None = None
    try:
        descriptor = os.open(path.name, flags, 0o600, dir_fd=parent_fd)
        metadata = os.fstat(descriptor)
        created_identity = (metadata.st_dev, metadata.st_ino)
        os.fchmod(descriptor, 0o600)
        view = memoryview(data)
        while view:
            written = os.write(descriptor, view)
            if written <= 0:
                raise RolloutGateError("short write while publishing rollout gate")
            view = view[written:]
        os.fsync(descriptor)
    except BaseException as error:
        try:
            current = os.stat(path.name, dir_fd=parent_fd, follow_symlinks=False)
            if created_identity == (current.st_dev, current.st_ino):
                os.unlink(path.name, dir_fd=parent_fd)
        except OSError:
            pass
        os.close(parent_fd)
        if isinstance(error, OSError):
            raise RolloutGateError(f"cannot publish gate output: {error}") from error
        raise
    finally:
        if descriptor is not None:
            try:
                os.close(descriptor)
            except OSError:
                pass
    try:
        os.fsync(parent_fd)
    finally:
        os.close(parent_fd)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--scorecard", required=True, type=Path)
    parser.add_argument("--evidence-index", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        gate = evaluate_rollout(
            manifest_path=args.manifest,
            scorecard_path=args.scorecard,
            evidence_path=args.evidence_index,
        )
        schema_errors = _schema_errors("rollout-gate", gate)
        if schema_errors:
            raise RolloutGateError("generated gate decision is invalid: " + "; ".join(schema_errors))
        _write_exclusive(args.output, gate)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"rollout_gate.py: {error}", file=sys.stderr)
        return 2
    print(f"{gate['decision']}: {args.output}")
    return 0 if gate["decision"] == "release_eligible" else 1


if __name__ == "__main__":
    raise SystemExit(main())
