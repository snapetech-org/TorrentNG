#!/usr/bin/env python3
"""Generate and validate an experimental PGO link order from a pinned BOLT profile.

This uses BOLT as a layout planner; its rewritten output is never executed or
installed. Every resulting build still needs independent paired qualification.
"""
from __future__ import annotations

import argparse
import gzip
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import sys

from native_seed_pgo import (validate_profile, workload_args, feature_identity,
                             feature_args, training_workloads, workload_environment,
                             optimization_identity, optimization_flags)
from seed_efficiency_tools import command, manifest, sha256, source_digest, source_provenance, verify_manifest

REPO = Path(__file__).resolve().parents[1]


def train(pgo_profile, repo, output, target, corpus, bolt, merge_fdata, client_cpus=None, features="default", optimization_level="s"):
    """Build an immutable input and collect the standard four-shape profile."""
    repo, output = Path(repo).resolve(), Path(output).resolve()
    pgo_profile = Path(pgo_profile).resolve()
    features = feature_identity(features)
    validated_profile = validate_profile(pgo_profile, repo, features, optimization_level)
    before = source_provenance(repo)
    if before["dirty"]:
        raise ValueError("layout training requires a clean source checkout")
    forbidden = {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER",
                 "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTFLAGS"}
    if "CARGO_ENCODED_RUSTFLAGS" in os.environ or any(
            key.startswith("CARGO_PROFILE_") or bool(value) for key, value in os.environ.items()
            if key.startswith("CARGO_PROFILE_") or key in forbidden):
        raise ValueError("unset compiler, flags and Cargo profile overrides before layout training")
    bolt_path, merge_path = shutil.which(str(bolt)), shutil.which(str(merge_fdata))
    if not bolt_path or not merge_path:
        raise ValueError("both LLVM BOLT and merge-fdata are required")
    workloads = training_workloads(json.loads(Path(corpus).read_text()), features)
    if not workloads:
        raise ValueError("layout training corpus is empty")
    output.mkdir(parents=True, exist_ok=False)
    target = Path(target).resolve() if target else output / "target"
    profile = output / "seed.profdata"
    opener = gzip.open if pgo_profile.suffix == ".gz" else open
    with opener(pgo_profile, "rb") as source, profile.open("wb") as destination:
        shutil.copyfileobj(source, destination)
    if sha256(profile) != validated_profile["profile_sha256"]:
        raise ValueError("PGO profile changed while preparing layout training")
    # Share immutable compiler inputs with the release helper. Cargo does not
    # fingerprint the contents of files passed through -Cprofile-use.
    target.mkdir(parents=True, exist_ok=True)
    cached_profile = target / ("torrentng-seed-" + sha256(profile) + ".profdata")
    if cached_profile.exists():
        if sha256(cached_profile) != sha256(profile):
            raise ValueError("cached PGO profile differs from its content identity")
    else:
        shutil.copyfile(profile, cached_profile)
    profile = cached_profile
    env = {**os.environ, "RUSTFLAGS": "-Cprofile-use=" + str(profile) + optimization_flags(optimization_level)}
    with (output / "training.log").open("w") as log:
        def run(argv, stdout=log):
            subprocess.run([str(arg) for arg in argv], cwd=repo, env=env,
                           stdout=stdout, stderr=log, check=True)
        run(["cargo", "+1.97.0", "rustc", "--locked", "--release", "-p", "torrentngd",
             "--bin", "torrentngd", "--target-dir", target, *feature_args(features), "--", "-Clink-arg=-Wl,--emit-relocs"])
        binary = output / "torrentngd.pgo-relocs"
        shutil.copy2(target / "release/torrentngd", binary)
        build = manifest(binary, repo, profile, features=features, optimization_level=optimization_level)
        build["rustflags"] = env["RUSTFLAGS"]
        build.setdefault("build_environment", {})["RUSTFLAGS"] = env["RUSTFLAGS"]
        build["binary_link_args"] = ["-Clink-arg=-Wl,--emit-relocs"]
        build_path = output / "input.manifest.json"
        build_path.write_text(json.dumps(build, indent=2) + "\n")
        raw = output / "raw"
        raw.mkdir()
        instrumented = output / "torrentngd.instrumented"
        run([bolt_path, binary, "-instrument", "-o", instrumented,
             "-instrumentation-file=" + str(raw / "profile.fdata"),
             "-instrumentation-file-append-pid", "-instrumentation-binpath=" + str(instrumented)])
        for index, workload in enumerate(workloads):
            argv = [sys.executable, REPO / "scripts/native_seed_cpu_ram_probe.py", "--client",
                    "training=torrentngd:" + str(instrumented), "--allow-single-client",
                    "--repeats", "1", "--output", output / f"training-{index}",
                    *workload_args(workload), *workload_environment(workload)]
            if client_cpus:
                argv += ["--client-cpus", client_cpus]
            run(argv)
        profiles = sorted(raw.glob("*.fdata*"))
        if not profiles:
            raise ValueError("BOLT training produced no daemon profiles")
        merged = output / "seed.fdata"
        with merged.open("w") as stream:
            run([merge_path, *profiles], stdout=stream)
    if source_digest(repo) != before["source_digest"]:
        raise ValueError("source changed during layout training")
    record = {"format": "torrentng-bolt-experiment-v1", "input_binary_sha256": sha256(binary),
              "instrumented_binary_sha256": sha256(instrumented), "profile_sha256": sha256(merged),
              "pgo_profile_sha256": sha256(profile), "training": workloads,
              "optimization_level": optimization_identity(optimization_level),
              "bolt_version": command(bolt_path, "--version"), "tool_sha256": sha256(bolt_path),
              "merge_tool_sha256": sha256(merge_path)}
    profile_path = output / "bolt-profile.manifest.json"
    profile_path.write_text(json.dumps(record, indent=2) + "\n")
    order = output / "function-order.txt"
    generate(binary, build_path, merged, profile_path, order, bolt_path)
    return order


def normalize_order(lines, symbol_table):
    """Translate BOLT's local-symbol disambiguators to names accepted by LLD."""
    symbols = {fields[2] for line in symbol_table.splitlines()
               if len(fields := line.split(None, 2)) == 3}
    result = list(dict.fromkeys(re.sub(r"/\d+$", "", line.strip())
                               for line in lines if line.strip()))
    if not result:
        raise ValueError("BOLT produced an empty function order")
    missing = [name for name in result if name not in symbols]
    if missing:
        raise ValueError(f"ordered symbol is absent from the input ELF: {missing[0]}")
    return result


def generate(binary, binary_manifest, bolt_profile, profile_manifest, output, bolt):
    binary, bolt_profile, output = (Path(p).resolve() for p in (binary, bolt_profile, output))
    metadata = json.loads(Path(binary_manifest).read_text())
    training = json.loads(Path(profile_manifest).read_text())
    if training.get("format") != "torrentng-bolt-experiment-v1":
        raise ValueError("unsupported BOLT training manifest")
    verify_manifest(metadata, binary)
    if metadata.get("kind") != "torrentngd" or metadata.get("dirty", True):
        raise ValueError("function ordering requires a clean TorrentNG input build")
    if not metadata.get("profile_sha256") or not metadata.get("rustc"):
        raise ValueError("input build lacks PGO/compiler provenance")
    bolt_path = shutil.which(str(bolt))
    if not bolt_path:
        raise ValueError(f"BOLT executable not found: {bolt}")
    checks = {
        "input_binary_sha256": metadata["binary_sha256"],
        "profile_sha256": sha256(bolt_profile),
        "pgo_profile_sha256": metadata["profile_sha256"],
        "tool_sha256": sha256(bolt_path),
    }
    for key, expected in checks.items():
        if training.get(key) != expected:
            raise ValueError(f"BOLT training provenance mismatch: {key}")
    if optimization_identity(training.get("optimization_level", "s")) != optimization_identity(metadata.get("optimization_level", "s")):
        raise ValueError("BOLT training provenance mismatch: optimization_level")
    if not training.get("training"):
        raise ValueError("BOLT profile lacks a recorded training corpus")
    sidecar = Path(str(output) + ".manifest.json")
    log_path = Path(str(output) + ".log")
    if any(path.exists() for path in (output, sidecar, log_path)):
        raise ValueError("function-order output, manifest and log must be new paths")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="torrentng-order-") as directory:
        planner, raw_order = Path(directory) / "planner.bolt", Path(directory) / "raw-order.txt"
        argv = [bolt_path, str(binary), "-o", str(planner), "-data=" + str(bolt_profile),
                "-reorder-blocks=ext-tsp", "-reorder-functions=cdsort",
                "-generate-function-order=" + str(raw_order)]
        with log_path.open("x") as log:
            subprocess.run(argv, stdout=log, stderr=subprocess.STDOUT, check=True)
        order = normalize_order(raw_order.read_text().splitlines(),
                                command("nm", "--defined-only", str(binary)))
    output.write_text("\n".join(order) + "\n")
    record = {
        "format": "torrentng-function-order-v1", "order_sha256": sha256(output),
        "source_digest": metadata["source_digest"], "source_commit": metadata["source_commit"],
        "source_tree": metadata.get("source_tree"), "rustc": metadata["rustc"],
        "features": feature_identity(metadata.get("features", "default")), "pgo_profile_sha256": metadata["profile_sha256"],
        "optimization_level": optimization_identity(metadata.get("optimization_level", "s")),
        "input_binary_sha256": metadata["binary_sha256"],
        "bolt_profile_sha256": checks["profile_sha256"], "bolt_tool_sha256": checks["tool_sha256"],
        "bolt_version": command(bolt_path, "--version"), "training_corpus": training["training"],
        "function_count": len(order), "planner_log_sha256": sha256(log_path),
        "claim_scope": "experimental layout input; no performance qualification is implied",
    }
    sidecar.write_text(json.dumps(record, indent=2) + "\n")
    return record


def validate(order, repo, pgo_profile, build_manifest=None, features="default", optimization_level="s"):
    order, repo = Path(order).resolve(), Path(repo).resolve()
    data = json.loads(Path(str(order) + ".manifest.json").read_text())
    if data.get("format") != "torrentng-function-order-v1":
        raise ValueError("unsupported function-order manifest")
    if data.get("order_sha256") != sha256(order):
        raise ValueError("function order differs from its manifest")
    features = feature_identity(features)
    profile = validate_profile(pgo_profile, repo, features, optimization_level)
    expected = {"source_digest": source_digest(repo), "rustc": profile["rustc"],
                "pgo_profile_sha256": profile["profile_sha256"], "features": features}
    for key, value in expected.items():
        if data.get(key) != value:
            raise ValueError(f"function order differs from this build: {key}")
    if optimization_identity(data.get("optimization_level", "s")) != optimization_identity(optimization_level):
        raise ValueError("function order differs from this build: optimization_level")
    if build_manifest is not None:
        path = Path(build_manifest)
        build = json.loads(path.read_text())
        verify_manifest(build, build["binary"])
        if feature_identity(build.get("features", "default")) != features:
            raise ValueError("output build differs from function order: features")
        if optimization_identity(build.get("optimization_level", "s")) != optimization_identity(optimization_level):
            raise ValueError("output build differs from function order: optimization_level")
        for key in ("source_digest", "rustc"):
            if build.get(key) != data[key]:
                raise ValueError(f"output build differs from function order: {key}")
        if build.get("profile_sha256") != data["pgo_profile_sha256"]:
            raise ValueError("output build differs from function order: PGO profile")
        # Even source-matched builds can vary in code generation. Do not record
        # a layout as applied if its function names disappeared from the output.
        normalize_order(order.read_text().splitlines(),
                        command("nm", "--defined-only", build["binary"]))
        build["function_order"] = data
        build["binary_link_args"] = ["-Clink-arg=-Wl,--emit-relocs",
                                      "-Clink-arg=-Wl,--symbol-ordering-file=" + str(order)]
        path.write_text(json.dumps(build, indent=2) + "\n")
    return data


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="operation", required=True)
    collect = sub.add_parser("train")
    collect.add_argument("--pgo-profile", type=Path, required=True)
    collect.add_argument("--output", type=Path, required=True)
    collect.add_argument("--repo", type=Path, default=REPO)
    collect.add_argument("--target-dir", type=Path)
    collect.add_argument("--corpus", type=Path, default=REPO / "certification/benchmarks/seed-efficiency-corpus.json")
    collect.add_argument("--bolt", default="llvm-bolt-20")
    collect.add_argument("--merge-fdata", default="merge-fdata-20")
    collect.add_argument("--client-cpus")
    collect.add_argument("--features", default="default")
    collect.add_argument("--opt-level", choices=("s", "3"), default="s")
    create = sub.add_parser("generate")
    for name in ("binary", "binary-manifest", "bolt-profile", "profile-manifest", "output"):
        create.add_argument("--" + name, type=Path, required=True)
    create.add_argument("--bolt", default="llvm-bolt-20")
    check = sub.add_parser("validate")
    check.add_argument("--order", type=Path, required=True)
    check.add_argument("--repo", type=Path, default=REPO)
    check.add_argument("--pgo-profile", type=Path, required=True)
    check.add_argument("--features", default="default")
    check.add_argument("--opt-level", choices=("s", "3"), default="s")
    check.add_argument("--build-manifest", type=Path,
                       help="also verify and record the order in a completed build manifest")
    args = parser.parse_args()
    if args.operation == "train":
        print(train(args.pgo_profile, args.repo, args.output, args.target_dir,
                    args.corpus, args.bolt, args.merge_fdata, args.client_cpus, args.features, args.opt_level))
    elif args.operation == "generate":
        result = generate(args.binary, args.binary_manifest, args.bolt_profile,
                          args.profile_manifest, args.output, args.bolt)
        print(result["order_sha256"])
    else:
        validate(args.order, args.repo, args.pgo_profile, args.build_manifest, args.features, args.opt_level)


if __name__ == "__main__":
    main()
