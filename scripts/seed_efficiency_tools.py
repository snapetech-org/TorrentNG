#!/usr/bin/env python3
"""Build provenance and paired CPU/PSS qualification for native seed probes."""
from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import random
import statistics
import subprocess
import time


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def command(*args, cwd=None):
    return subprocess.check_output(args, cwd=cwd, text=True).strip()


def source_files(repo, kind="torrentngd"):
    paths = command("git", "ls-files", "--cached", "--others", "--exclude-standard", cwd=repo).splitlines()
    sources = {}
    for path in paths:
        # Include every build input in the workspace, excluding recorded
        # experiments, targets, desktop and bridge's separate workspaces.
        if kind == "rtorrent" or path in {"Cargo.toml", "Cargo.lock"} or path.startswith("crates/") and not path.startswith("crates/torrentng-desktop/"):
            file = repo / path
            if file.is_file():
                sources[path] = sha256(file)
    return sources


def source_digest(repo):
    return hashlib.sha256(json.dumps(source_files(repo), sort_keys=True).encode()).hexdigest()


def manifest(binary, repo, profile=None, features="default", kind="torrentngd"):
    repo, binary = Path(repo).resolve(), Path(binary).resolve()
    sources = source_files(repo, kind)
    diff = subprocess.check_output(["git", "diff", "HEAD", "--binary"], cwd=repo)
    status = command("git", "status", "--porcelain", "--untracked-files=all", cwd=repo)
    libraries = {}
    try:
        for line in command("ldd", str(binary)).splitlines():
            words = line.replace("=>", " ").split()
            for word in words:
                if word.startswith("/") and Path(word).is_file():
                    libraries[word] = sha256(word)
    except (OSError, subprocess.CalledProcessError):
        pass
    return {
        "format": "torrentng-build-manifest-v1", "created_unix": time.time(), "kind": kind,
        "source_commit": command("git", "rev-parse", "HEAD", cwd=repo),
        "dirty": bool(status), "source_status": status.splitlines(),
        "tracked_diff_sha256": hashlib.sha256(diff).hexdigest(),
        "source_files": sources,
        "source_digest": hashlib.sha256(json.dumps(sources, sort_keys=True).encode()).hexdigest(),
        "cargo_lock_sha256": sha256(repo / "Cargo.lock") if kind == "torrentngd" else None,
        "binary": str(binary), "binary_sha256": sha256(binary),
        "rustc": command("rustc", "+1.97.0", "-Vv") if kind == "torrentngd" else None,
        "cargo": command("cargo", "+1.97.0", "-V") if kind == "torrentngd" else None,
        "cxx": command(os.environ.get("CXX", "c++"), "--version") if kind == "rtorrent" else None,
        "cxxflags": os.environ.get("CXXFLAGS", "") if kind == "rtorrent" else None,
        "features": features, "profile_sha256": sha256(profile) if profile else None,
        "rustflags": os.environ.get("RUSTFLAGS", ""),
        "build_environment": {key: value for key, value in os.environ.items()
                              if key.startswith("CARGO_PROFILE_") or key in {
                                  "RUSTC", "RUSTC_WRAPPER", "RUSTC_WORKSPACE_WRAPPER",
                                  "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET",
                                  "CARGO_BUILD_RUSTFLAGS", "CARGO_BUILD_RUSTC", "RUSTUP_TOOLCHAIN"}},
        "shared_libraries_sha256": libraries,
    }


def verify_manifest(data, binary):
    if data.get("format") != "torrentng-build-manifest-v1":
        raise ValueError("unsupported build manifest")
    if data.get("binary_sha256") != sha256(binary):
        raise ValueError("binary hash does not match its build manifest")
    dependencies = data.get("cargo_lock_sha256") if data.get("kind", "torrentngd") == "torrentngd" else data.get("shared_libraries_sha256")
    if not data.get("source_digest") or not dependencies:
        raise ValueError("manifest lacks source or dependency provenance")


def balanced_orders(count, repeats, seed):
    """Randomized rotations balance every treatment's position per full block."""
    if count < 1 or repeats < 1:
        raise ValueError("positive treatment and round counts are required")
    rng = random.Random(seed)
    orders = []
    while len(orders) < repeats:
        base = list(range(count))
        rng.shuffle(base)
        shifts = list(range(count))
        rng.shuffle(shifts)
        orders.extend(base[shift:] + base[:shift] for shift in shifts)
    return orders[:repeats]


def host_snapshot():
    paths = ["/proc/loadavg", "/proc/pressure/cpu", "/proc/pressure/io", "/proc/pressure/memory",
             "/proc/vmstat", "/proc/meminfo", "/sys/fs/cgroup/memory.stat", "/sys/fs/cgroup/cpu.stat"]
    result = {"time": time.time(), "affinity": sorted(os.sched_getaffinity(0))}
    for name in paths:
        try:
            result[name] = Path(name).read_text()
        except OSError:
            result[name] = None
    result["smt_siblings"] = {}
    for cpu in result["affinity"]:
        try:
            result["smt_siblings"][str(cpu)] = Path(f"/sys/devices/system/cpu/cpu{cpu}/topology/thread_siblings_list").read_text().strip()
        except OSError:
            pass
    return result


def paired_analysis(report, control, candidate, tolerance=0.02, minimum_rounds=10):
    if not 0 <= tolerance <= 0.1 or minimum_rounds < 3:
        raise ValueError("invalid predeclared tolerance or minimum rounds")
    if report.get("status") != "complete":
        raise ValueError("incomplete experiments cannot be qualified")
    groups = {control: {}, candidate: {}}
    hashes = {control: set(), candidate: set()}
    for run in report["runs"]:
        if run["name"] not in groups:
            continue
        if not run["transfer"].get("verified") or run.get("diagnostic_run"):
            raise ValueError("unverified or diagnostic runs cannot be scored")
        rounds = groups[run["name"]]
        if run["round"] in rounds:
            raise ValueError("duplicate treatment in a paired round")
        rounds[run["round"]] = run
        hashes[run["name"]].add(run["binary_sha256"])
    if not groups[control] or groups[control].keys() != groups[candidate].keys():
        raise ValueError("candidate and control require complete matched rounds")
    if any(len(values) != 1 for values in hashes.values()):
        raise ValueError("a treatment changed binary identity during the experiment")
    metrics = {
        "cpu_seconds_per_gib": lambda r: r["payload_cpu_seconds_user_system"] / (r["transfer"]["bytes"] / 1024**3),
        "peak_pss_bytes": lambda r: r["active_peak_pss_bytes"],
        "peak_anonymous_bytes": lambda r: r["active_peak_anonymous_bytes"],
    }
    results = {}
    rounds = sorted(groups[control])
    for name, metric in metrics.items():
        ratios = []
        for round_id in rounds:
            base, new = groups[control][round_id], groups[candidate][round_id]
            if base["transfer"]["bytes"] != new["transfer"]["bytes"]:
                raise ValueError("paired transfers differ in byte count")
            before, after = metric(base), metric(new)
            if not (math.isfinite(before) and math.isfinite(after) and before > 0 and after > 0):
                raise ValueError("invalid resource measurement")
            ratios.append(after / before)
        rng = random.Random(20261023)
        samples = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(10000))
        # Bonferroni-adjust the two primary resource intervals: their joint
        # coverage target is 95%, rather than calling two 95% intervals joint.
        results[name] = {"paired_ratios": ratios, "paired_median": statistics.median(ratios),
                         "wins": sum(r < 1 for r in ratios),
                         "interval": [samples[125], samples[9874]]}
    manifests = report.get("build_manifests", {})
    provenance = all(name in manifests and not manifests[name].get("dirty", True)
                     and manifests[name].get("binary_sha256") in hashes[name]
                     for name in (control, candidate))
    sufficient = len(rounds) >= minimum_rounds and provenance
    joint = all(results[m]["interval"][1] <= 1 + tolerance for m in ("cpu_seconds_per_gib", "peak_pss_bytes"))
    return {"format": "torrentng-paired-efficiency-v1", "control": control, "candidate": candidate,
            "rounds": len(rounds), "tolerance": tolerance, "minimum_rounds": minimum_rounds,
            "metrics": results, "clean_provenance": provenance,
            "resource_gate": "pass" if sufficient and joint else "inconclusive_or_regression",
            "claim_scope": "this workload and host only; correctness, goodput and latency gates remain mandatory"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    build = commands.add_parser("manifest")
    build.add_argument("--binary", required=True)
    build.add_argument("--repo", default=Path(__file__).resolve().parents[1])
    build.add_argument("--profile")
    build.add_argument("--features", default="default")
    build.add_argument("--kind", choices=("torrentngd", "rtorrent"), default="torrentngd")
    build.add_argument("--require-clean", action="store_true")
    analyze = commands.add_parser("analyze")
    analyze.add_argument("--summary", required=True)
    analyze.add_argument("--control", required=True)
    analyze.add_argument("--candidate", required=True)
    analyze.add_argument("--tolerance", type=float, default=0.02)
    analyze.add_argument("--minimum-rounds", type=int, default=10)
    for item in (build, analyze):
        item.add_argument("--output", required=True)
    args = parser.parse_args()
    if args.command == "manifest":
        result = manifest(args.binary, args.repo, args.profile, args.features, args.kind)
        if args.require_clean and result["dirty"]:
            parser.error("qualified builds require a clean source tree")
    else:
        result = paired_analysis(json.loads(Path(args.summary).read_text()), args.control,
                                 args.candidate, args.tolerance, args.minimum_rounds)
    Path(args.output).write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
