#!/usr/bin/env python3
"""Build a descriptive, paired scorecard from a validated transfer manifest."""

from __future__ import annotations

import argparse
import hashlib
import itertools
import json
import math
import os
import random
import statistics
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

try:
    from validate_artifact import _load_strict_json, validate
except ImportError as error:  # pragma: no cover - exercised by the CLI environment
    raise SystemExit("scorecard.py must run from its scripts directory") from error


SCORECARD_VERSION = "transfer-scorecard-v1"
DEFAULT_BOOTSTRAP_SAMPLES = 10_000
DEFAULT_MIN_PAIRED_RELEASE_TRIALS = 10
MAX_TRIALS = 100_000
MAX_BOOTSTRAP_SAMPLES = 100_000
MAX_CLIENTS_PER_GROUP = 16
MAX_BOOTSTRAP_WORK = 100_000_000
GIB = 1024**3

RESOURCE_METRICS: tuple[tuple[str, str, str], ...] = (
    ("rss_peak_bytes", "resources.rss_peak_bytes", "lower"),
    ("pss_peak_bytes", "resources.pss_peak_bytes", "lower"),
    ("cgroup_anon_peak_bytes", "resources.cgroup_anon_peak_bytes", "lower"),
    ("cgroup_file_peak_bytes", "resources.cgroup_file_peak_bytes", "lower"),
    (
        "cgroup_reclaimable_cache_peak_bytes",
        "resources.cgroup_reclaimable_cache_peak_bytes",
        "lower",
    ),
    ("cgroup_current_peak_bytes", "resources.cgroup_current_peak_bytes", "lower"),
    ("threads_peak", "resources.threads_peak", "lower"),
    ("fds_peak", "resources.fds_peak", "lower"),
    ("block_read_bytes_per_gib", "resources.block_read_bytes", "lower"),
    ("block_write_bytes_per_gib", "resources.block_write_bytes", "lower"),
    ("energy_joules_per_gib", "resources.energy_joules", "lower"),
    ("cpu_seconds_per_gib", "resources.cpu_time_ns", "lower"),
)


class ScorecardError(ValueError):
    """Raised when evidence cannot support a comparable scorecard."""


def _canonical(value: Any) -> bytes:
    return json.dumps(
        value,
        sort_keys=True,
        separators=(",", ":"),
        ensure_ascii=False,
        allow_nan=False,
    ).encode("utf-8")


def _comparison_signature(trial: dict[str, Any]) -> tuple[str, str, str, str, str]:
    workload = trial["workload"]
    environment = json.loads(json.dumps(trial["environment"]))
    environment.pop("independent_ceiling_sha256", None)
    environment["network"].pop("control_observation_sha256s", None)
    signature = {
        "workload": workload,
        "environment": environment,
    }
    signature_digest = hashlib.sha256(_canonical(signature)).hexdigest()
    return (
        workload["case_id"],
        workload["kind"],
        workload.get("protocol", "unspecified"),
        workload["fixture_sha256"],
        signature_digest,
    )


def _client_key(trial: dict[str, Any]) -> str:
    identity = trial["client_identity"]
    return f"{identity['client_id']}@{identity['version']}"


def _eligibility_reason(trial: dict[str, Any]) -> str | None:
    if trial["outcome"]["status"] != "complete":
        return f"outcome is {trial['outcome']['status']}"
    kind = trial["workload"]["kind"]
    expected = trial["workload"]["expected_payload_bytes"]
    useful = trial["payload"]["useful_bytes"]
    if kind in ("download", "seed"):
        if trial["integrity"]["status"] != "passed":
            return "transfer integrity was not passed"
        if useful != expected:
            return f"verified useful bytes {useful} differ from expected {expected}"
    if trial["durations_ns"]["transfer"] <= 0:
        return "transfer duration is not positive"
    return None


def _finite_number(value: Any, field: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ScorecardError(f"{field} must be a finite number")
    try:
        number = float(value)
    except (OverflowError, ValueError) as error:
        raise ScorecardError(f"{field} must be a finite number") from error
    if not math.isfinite(number) or number < 0:
        raise ScorecardError(f"{field} must be a finite nonnegative number")
    return number


def _metric_values(trial: dict[str, Any]) -> dict[str, float | None]:
    useful = _finite_number(trial["payload"]["useful_bytes"], "payload.useful_bytes")
    gib = useful / GIB
    transfer_ns = _finite_number(trial["durations_ns"]["transfer"], "durations_ns.transfer")
    values: dict[str, float | None] = {
        "goodput_bytes_per_second": None,
    }
    if useful > 0 and transfer_ns > 0:
        values["goodput_bytes_per_second"] = _finite_number(
            useful * 1_000_000_000 / transfer_ns,
            "goodput_bytes_per_second",
        )
    for name, path, _direction in RESOURCE_METRICS:
        value: Any = trial
        for part in path.split("."):
            value = value.get(part) if isinstance(value, dict) else None
        if value is None:
            values[name] = None
        else:
            numeric = _finite_number(value, f"resources.{name}")
            if name == "cpu_seconds_per_gib":
                metric = numeric / 1_000_000_000 / gib if gib > 0 else None
            elif name.endswith("_per_gib"):
                metric = numeric / gib if gib > 0 else None
            else:
                metric = numeric
            values[name] = (
                _finite_number(metric, f"resources.{name}_normalized")
                if metric is not None
                else None
            )
    return values


def _percentile(sorted_values: list[float], percentile: float) -> float:
    if not sorted_values:
        raise ScorecardError("cannot calculate a percentile without samples")
    position = (len(sorted_values) - 1) * percentile
    lower = int(position)
    upper = min(lower + 1, len(sorted_values) - 1)
    fraction = position - lower
    return sorted_values[lower] * (1 - fraction) + sorted_values[upper] * fraction


def _bootstrap_median_interval(
    values: list[float],
    *,
    seed: int,
    samples: int,
) -> tuple[float, float, float]:
    if not values:
        raise ScorecardError("cannot bootstrap an empty paired sample")
    median = statistics.median(values)
    seed_value = int.from_bytes(hashlib.sha256(str(seed).encode()).digest()[:8], "big")
    rng = random.Random(seed_value)
    draws = sorted(
        statistics.median(rng.choices(values, k=len(values)))
        for _ in range(samples)
    )
    return median, _percentile(draws, 0.025), _percentile(draws, 0.975)


def _paired_seed(seed: int, group_id: str, client_a: str, client_b: str, metric: str) -> int:
    material = f"{seed}\0{group_id}\0{client_a}\0{client_b}\0{metric}".encode()
    return int.from_bytes(hashlib.sha256(material).digest()[:8], "big")


def _group_trials(trials: list[dict[str, Any]]) -> dict[tuple[str, ...], list[dict[str, Any]]]:
    groups: dict[tuple[str, ...], list[dict[str, Any]]] = {}
    for trial in trials:
        groups.setdefault(_comparison_signature(trial), []).append(trial)
    return groups


def analyze_trials(
    trials: list[dict[str, Any]],
    *,
    bootstrap_samples: int = DEFAULT_BOOTSTRAP_SAMPLES,
    seed: int = 0,
    minimum_pairs: int = DEFAULT_MIN_PAIRED_RELEASE_TRIALS,
) -> list[dict[str, Any]]:
    """Summarize complete matched trials without applying performance gates."""
    if len(trials) > MAX_TRIALS:
        raise ScorecardError(f"trial count exceeds the safety limit of {MAX_TRIALS}")
    if type(bootstrap_samples) is not int or not 1_000 <= bootstrap_samples <= MAX_BOOTSTRAP_SAMPLES:
        raise ScorecardError(
            f"bootstrap samples must be between 1000 and {MAX_BOOTSTRAP_SAMPLES}"
        )
    if type(minimum_pairs) is not int or not 1 <= minimum_pairs <= 10_000:
        raise ScorecardError("minimum paired sample count must be between 1 and 10000")
    if type(seed) is not int:
        raise ScorecardError("random seed must be an integer")

    result: list[dict[str, Any]] = []
    for signature, group in sorted(_group_trials(trials).items()):
        case_id, kind, protocol, fixture_digest, signature_digest = signature
        grouped_pairs: dict[str, list[dict[str, Any]]] = {}
        for trial in group:
            grouped_pairs.setdefault(trial["pair_id"], []).append(trial)

        valid_pairs: dict[str, dict[str, dict[str, Any]]] = {}
        excluded_pairs: list[dict[str, str]] = []
        for pair_id, records in sorted(grouped_pairs.items()):
            reasons = [
                reason
                for record in records
                if (reason := _eligibility_reason(record)) is not None
            ]
            clients = [_client_key(record) for record in records]
            if reasons:
                excluded_pairs.append({"pair_id": pair_id, "reason": "; ".join(sorted(set(reasons)))})
                continue
            if len(set(clients)) != len(clients):
                excluded_pairs.append({"pair_id": pair_id, "reason": "duplicate client trial in pair"})
                continue
            if len(clients) < 2:
                excluded_pairs.append({"pair_id": pair_id, "reason": "pair has fewer than two clients"})
                continue
            valid_pairs[pair_id] = dict(zip(clients, records, strict=True))

        client_keys = sorted({client for pair in valid_pairs.values() for client in pair})
        if len(client_keys) > MAX_CLIENTS_PER_GROUP:
            raise ScorecardError(
                f"comparison group has more than {MAX_CLIENTS_PER_GROUP} clients"
            )
        paired_trials: dict[str, list[dict[str, Any]]] = {client: [] for client in client_keys}
        for pair in valid_pairs.values():
            for client, trial in pair.items():
                paired_trials[client].append(trial)

        client_summaries = []
        for client in client_keys:
            metric_rows = [_metric_values(trial) for trial in paired_trials[client]]
            metrics: dict[str, float | None] = {}
            all_metric_names = sorted({name for row in metric_rows for name in row})
            for name in all_metric_names:
                values = [row[name] for row in metric_rows if row.get(name) is not None]
                metrics[name] = statistics.median(values) if values else None
            identity = paired_trials[client][0]["client_identity"]
            client_summaries.append(
                {
                    "client_key": client,
                    "client_id": identity["client_id"],
                    "version": identity["version"],
                    "paired_trial_count": len(paired_trials[client]),
                    "metrics_median": metrics,
                }
            )

        comparisons = []
        bootstrap_work = 0
        for client_a, client_b in itertools.combinations(client_keys, 2):
            pair_samples: dict[str, list[dict[str, float | str]]] = {}
            for metric in ["goodput_bytes_per_second"] + [entry[0] for entry in RESOURCE_METRICS]:
                pair_samples[metric] = []
            matched_pair_ids = []
            for pair_id, pair in sorted(valid_pairs.items()):
                if client_a not in pair or client_b not in pair:
                    continue
                matched_pair_ids.append(pair_id)
                values_a = _metric_values(pair[client_a])
                values_b = _metric_values(pair[client_b])
                for metric in pair_samples:
                    a = values_a.get(metric)
                    b = values_b.get(metric)
                    if a is None or b is None or a <= 0:
                        continue
                    pair_samples[metric].append(
                        {
                            "pair_id": pair_id,
                            "client_a": a,
                            "client_b": b,
                            "ratio_b_over_a": _finite_number(
                                b / a,
                                f"{metric} ratio",
                            ),
                        }
                    )

            metric_comparisons: dict[str, Any] = {}
            comparison_work = sum(
                len(samples_for_metric) * bootstrap_samples
                for samples_for_metric in pair_samples.values()
            )
            if bootstrap_work + comparison_work > MAX_BOOTSTRAP_WORK:
                raise ScorecardError(
                    "paired bootstrap work exceeds the safety budget of "
                    f"{MAX_BOOTSTRAP_WORK} resampled values"
                )
            bootstrap_work += comparison_work
            for metric, samples_for_metric in pair_samples.items():
                ratios = [sample["ratio_b_over_a"] for sample in samples_for_metric]
                if ratios:
                    median, lower, upper = _bootstrap_median_interval(
                        ratios,
                        seed=_paired_seed(seed, signature_digest, client_a, client_b, metric),
                        samples=bootstrap_samples,
                    )
                    _direction = "higher" if metric == "goodput_bytes_per_second" else "lower"
                    metric_comparisons[metric] = {
                        "direction": _direction,
                        "ratio_b_over_a_median": median,
                        "ci95_low": lower,
                        "ci95_high": upper,
                        "paired_sample_count": len(ratios),
                        "paired_samples": samples_for_metric,
                        "minimum_sample_met": len(ratios) >= minimum_pairs,
                    }
            comparisons.append(
                {
                    "client_a": client_a,
                    "client_b": client_b,
                    "matched_pair_ids": matched_pair_ids,
                    "metrics": metric_comparisons,
                }
            )

        result.append(
            {
                "case_id": case_id,
                "kind": kind,
                "protocol": protocol,
                "fixture_sha256": fixture_digest,
                "comparison_signature_sha256": signature_digest,
                "paired_release_sample_minimum": minimum_pairs,
                "valid_pair_count": len(valid_pairs),
                "excluded_pairs": excluded_pairs,
                "clients": client_summaries,
                "comparisons": comparisons,
            }
        )
    return result


def build_scorecard(
    manifest_path: Path,
    *,
    bootstrap_samples: int = DEFAULT_BOOTSTRAP_SAMPLES,
    seed: int = 0,
    minimum_pairs: int = DEFAULT_MIN_PAIRED_RELEASE_TRIALS,
) -> dict[str, Any]:
    errors = validate("manifest", manifest_path, verify_referenced_files=True)
    if errors:
        preview = "\n".join(f"  {error}" for error in errors[:20])
        raise ScorecardError(f"manifest validation failed:\n{preview}")
    manifest = _load_strict_json(manifest_path)
    if not isinstance(manifest, dict):
        raise ScorecardError("manifest root must be an object")
    if len(manifest["trial_files"]) > MAX_TRIALS:
        raise ScorecardError(f"trial count exceeds the safety limit of {MAX_TRIALS}")

    trials: list[dict[str, Any]] = []
    for record in manifest["trial_files"]:
        path = manifest_path.parent / record["path"]
        trial = _load_strict_json(path)
        if not isinstance(trial, dict):
            raise ScorecardError(f"trial {record['path']} root must be an object")
        trials.append(trial)

    manifest_digest = hashlib.sha256(manifest_path.read_bytes()).hexdigest()
    return {
        "schema_version": SCORECARD_VERSION,
        "created_at": datetime.now(timezone.utc).isoformat().replace("+00:00", "Z"),
        "manifest_sha256": manifest_digest,
        "manifest_id": manifest["manifest_id"],
        "analysis": {
            "method": "paired_percentile_bootstrap_median_ratio",
            "confidence_level": 0.95,
            "bootstrap_samples": bootstrap_samples,
            "random_seed": seed,
            "minimum_paired_release_trials": minimum_pairs,
            "performance_thresholds_applied": False,
        },
        "groups": analyze_trials(
            trials,
            bootstrap_samples=bootstrap_samples,
            seed=seed,
            minimum_pairs=minimum_pairs,
        ),
    }


def write_exclusive(path: Path, value: dict[str, Any]) -> None:
    absolute_parent = path.parent.absolute()
    if any(parent.is_symlink() for parent in (absolute_parent, *absolute_parent.parents)):
        raise ScorecardError("output path cannot contain a symbolic-link directory")
    if not absolute_parent.is_dir():
        raise ScorecardError("output parent must be an existing, non-symlink directory")
    payload = (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + "\n").encode()
    descriptor = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(payload)
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        try:
            path.unlink()
        except OSError:
            pass
        raise


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--bootstrap-samples", type=int, default=DEFAULT_BOOTSTRAP_SAMPLES)
    parser.add_argument("--seed", type=int, default=0)
    parser.add_argument("--minimum-pairs", type=int, default=DEFAULT_MIN_PAIRED_RELEASE_TRIALS)
    args = parser.parse_args(argv)
    try:
        scorecard = build_scorecard(
            args.manifest,
            bootstrap_samples=args.bootstrap_samples,
            seed=args.seed,
            minimum_pairs=args.minimum_pairs,
        )
        write_exclusive(args.output, scorecard)
    except (OSError, ValueError) as error:
        print(error, file=sys.stderr)
        return 2
    print(f"{args.output}: wrote {SCORECARD_VERSION}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
