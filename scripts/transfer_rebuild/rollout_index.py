#!/usr/bin/env python3
"""Bind final-stage declarations to exact manifest and scorecard artifacts."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path
from typing import Any

try:
    from rollout_gate import (
        REQUIRED_KINDS,
        RolloutGateError,
        _read_json_artifact,
        _schema_errors,
        _write_exclusive,
    )
    from validate_artifact import validate
except ImportError as error:  # pragma: no cover - exercised by the CLI environment
    raise SystemExit("rollout_index.py must run from its scripts directory") from error


def build_index(
    manifest: dict[str, Any],
    manifest_sha256: str,
    scorecard: dict[str, Any],
    scorecard_sha256: str,
    declarations: Any,
) -> dict[str, Any]:
    if not isinstance(declarations, dict) or set(declarations) != {"candidate_client_id", "stages"}:
        raise RolloutGateError("stage declarations must contain candidate_client_id and stages")
    stages = declarations["stages"]
    if not isinstance(stages, dict) or set(stages) != set(REQUIRED_KINDS):
        raise RolloutGateError("stage declarations must include exactly IO-016, NET-116, PERF-116, SYS-015")
    manifest_artifacts = {record["sha256"]: record["kind"] for record in manifest["artifacts"]}
    for stage_id, stage in stages.items():
        if not isinstance(stage, dict) or set(stage) != {"status", "evidence", "notes"}:
            raise RolloutGateError(f"{stage_id} declaration has an invalid shape")
        if not isinstance(stage["status"], str) or stage["status"] not in {"passed", "blocked"}:
            raise RolloutGateError(f"{stage_id} status must be passed or blocked")
        if not isinstance(stage["notes"], str) or len(stage["notes"]) > 4096:
            raise RolloutGateError(f"{stage_id} notes are invalid")
        if not isinstance(stage["evidence"], list):
            raise RolloutGateError(f"{stage_id} evidence must be an array")
        kinds: set[str] = set()
        digests: set[str] = set()
        for item in stage["evidence"]:
            if not isinstance(item, dict) or set(item) != {"kind", "sha256"}:
                raise RolloutGateError(f"{stage_id} has a malformed evidence item")
            kind, digest = item["kind"], item["sha256"]
            if not isinstance(kind, str) or kind not in REQUIRED_KINDS[stage_id]:
                raise RolloutGateError(f"{stage_id} contains an unexpected evidence kind")
            if not isinstance(digest, str) or len(digest) != 64:
                raise RolloutGateError(f"{stage_id} contains an invalid artifact digest")
            if digest in digests or kind in kinds:
                raise RolloutGateError(f"{stage_id} has duplicate evidence")
            if manifest_artifacts.get(digest) != kind:
                raise RolloutGateError(f"{stage_id} evidence must identify a matching manifest artifact")
            kinds.add(kind)
            digests.add(digest)
    if scorecard.get("manifest_sha256") != manifest_sha256:
        raise RolloutGateError("scorecard does not reference the supplied manifest bytes")
    if scorecard.get("manifest_id") != manifest.get("manifest_id"):
        raise RolloutGateError("scorecard manifest_id differs from the supplied manifest")
    value = {
        "schema_version": "transfer-rollout-evidence-v1",
        "source_commit": manifest["source_commit"],
        "manifest_sha256": manifest_sha256,
        "scorecard_sha256": scorecard_sha256,
        "candidate_client_id": declarations["candidate_client_id"],
        "stages": stages,
    }
    errors = _schema_errors("rollout-evidence", value)
    if errors:
        raise RolloutGateError("rollout evidence index is invalid: " + "; ".join(errors[:20]))
    return value


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--scorecard", required=True, type=Path)
    parser.add_argument("--stages", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        manifest, manifest_sha256 = _read_json_artifact(args.manifest, "manifest")
        scorecard, scorecard_sha256 = _read_json_artifact(args.scorecard, "scorecard")
        declarations, _declarations_sha256 = _read_json_artifact(args.stages, "stage declarations")
        if not isinstance(manifest, dict) or not isinstance(scorecard, dict):
            raise RolloutGateError("manifest and scorecard must be JSON objects")
        manifest_errors = validate("manifest", args.manifest, verify_referenced_files=True)
        if manifest_errors:
            raise RolloutGateError("invalid manifest: " + "; ".join(manifest_errors[:20]))
        _manifest_again, manifest_again_digest = _read_json_artifact(args.manifest, "manifest")
        if manifest_again_digest != manifest_sha256:
            raise RolloutGateError("manifest changed while its referenced files were validated")
        scorecard_errors = validate("scorecard", args.scorecard)
        if scorecard_errors:
            raise RolloutGateError("invalid scorecard: " + "; ".join(scorecard_errors[:20]))
        _scorecard_again, scorecard_again_digest = _read_json_artifact(args.scorecard, "scorecard")
        if scorecard_again_digest != scorecard_sha256:
            raise RolloutGateError("scorecard changed while it was validated")
        index = build_index(
            manifest,
            manifest_sha256,
            scorecard,
            scorecard_sha256,
            declarations,
        )
        _write_exclusive(args.output, index)
    except (OSError, ValueError, KeyError, TypeError) as error:
        print(f"rollout_index.py: {error}", file=sys.stderr)
        return 2
    print(f"created rollout evidence index: {args.output}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
