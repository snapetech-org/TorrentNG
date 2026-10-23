#!/usr/bin/env python3
"""Train representative PGO, or run a held-out four-cell seed comparison.

Training includes startup, recheck, API and multiple transfer shapes. Validation
uses different sizes, windows and peer counts; profile data is never trained on
the held-out runs. All subprocesses use explicit argv, without shell evaluation.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

from seed_efficiency_tools import command, manifest, paired_analysis, sha256, source_digest

REPO = Path(__file__).resolve().parents[1]
PROBE = REPO / "scripts/native_seed_cpu_ram_probe.py"


def run(args, **kwargs):
    subprocess.run([str(arg) for arg in args], cwd=kwargs.pop("cwd", REPO), check=True, **kwargs)


def workload_args(workload):
    return ["--size", str(workload["mib"] * 1024**2), "--rate", str(workload["mib_per_second"] * 1024**2),
            "--peers", str(workload["peers"]), "--window", str(workload["window"]),
            "--request-pattern", workload["pattern"], "--cache-policy", workload.get("cache", "warm"),
            "--fixture-files", str(workload.get("files", 1)),
            "--upload-limit", str(workload.get("upload_limit", 0))]


def validate_profile(profile, repo=REPO):
    profile = Path(profile).resolve()
    metadata = json.loads(Path(str(profile) + ".manifest.json").read_text())
    if metadata["source_digest"] != source_digest(repo):
        raise ValueError("PGO training source differs from this build")
    if metadata["rustc"] != command("rustc", "+1.97.0", "-Vv"):
        raise ValueError("PGO training compiler differs from this build")
    if profile.suffix == ".gz":
        digest = hashlib.sha256()
        with gzip.open(profile, "rb") as stream:
            for chunk in iter(lambda: stream.read(1024**2), b""):
                digest.update(chunk)
        actual_hash = digest.hexdigest()
    else:
        actual_hash = sha256(profile)
    if metadata["profile_sha256"] != actual_hash:
        raise ValueError("PGO profile hash differs from its manifest")
    return metadata


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("train", "validate", "matrix"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--profile", type=Path)
    parser.add_argument("--repo", type=Path, default=REPO,
                        help="source checkout to train or validate, including an older clean baseline")
    parser.add_argument("--corpus", type=Path, default=REPO / "certification/benchmarks/seed-efficiency-corpus.json")
    parser.add_argument("--client", action="append", default=[], help="matrix label=kind:binary; supply baseline, baseline-pgo, candidate, candidate-pgo and optionally rtorrent")
    parser.add_argument("--client-manifest", action="append", default=[])
    parser.add_argument("--repeats", type=int, default=10)
    parser.add_argument("--client-cpus")
    args = parser.parse_args()
    source_repo = args.repo.resolve()
    if args.command == "validate":
        if args.profile is None:
            parser.error("--profile is required")
        validate_profile(args.profile, source_repo)
        return
    if args.output is None:
        parser.error("--output is required")
    if args.command == "matrix":
        cells = {"baseline", "baseline-pgo", "candidate", "candidate-pgo"}
        if not cells.issubset(value.split("=", 1)[0] for value in args.client):
            parser.error("matrix requires baseline, baseline-pgo, candidate and candidate-pgo labels")
        metadata = {name: json.loads(Path(path).read_text()) for name, path in
                    (value.split("=", 1) for value in args.client_manifest)}
        if not cells.issubset(metadata):
            parser.error("every matrix cell requires a build manifest")
        for name in ("baseline", "candidate"):
            plain, optimized = metadata[name], metadata[name + "-pgo"]
            if (plain.get("source_digest") != optimized.get("source_digest")
                    or plain.get("profile_sha256") or not optimized.get("profile_sha256")
                    or plain.get("rustc") != optimized.get("rustc")
                    or plain.get("features") != optimized.get("features")
                    or plain.get("dirty", True) or optimized.get("dirty", True)):
                parser.error("each clean source requires matching non-PGO and PGO builds")
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    corpus = json.loads(args.corpus.read_text())
    if args.command == "train":
        before = source_digest(source_repo)
        profile_dir = output / "raw"
        profile_dir.mkdir()
        target = output / "instrumented"
        environment = dict(os.environ)
        if any(value for key, value in environment.items() if key.startswith("CARGO_PROFILE_")
               or key in {"RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER",
                          "RUSTC_WORKSPACE_WRAPPER", "CARGO_BUILD_RUSTC", "CARGO_BUILD_RUSTFLAGS"}):
            parser.error("unset compiler, flag and release-profile overrides before PGO training")
        environment["RUSTFLAGS"] = f"-Cprofile-generate={profile_dir}"
        run(["cargo", "+1.97.0", "build", "--locked", "--release", "-p", "torrentngd", "--target-dir", target], env=environment, cwd=source_repo)
        binary = target / "release/torrentngd"
        for index, workload in enumerate(corpus["training"]):
            argv = [sys.executable, PROBE, "--client", f"training=torrentngd:{binary}",
                    "--allow-single-client", "--repeats", "1", "--output", output / f"training-{index}",
                    "--client-env", f"training:LLVM_PROFILE_FILE={profile_dir}/daemon-{index}-%p.profraw",
                    *workload_args(workload)]
            if args.client_cpus: argv += ["--client-cpus", args.client_cpus]
            run(argv)
        sysroot = Path(command("rustc", "+1.97.0", "--print", "sysroot"))
        host = next(line.split(": ", 1)[1] for line in command("rustc", "+1.97.0", "-Vv").splitlines() if line.startswith("host: "))
        profdata = sysroot / "lib/rustlib" / host / "bin/llvm-profdata"
        if not profdata.is_file():
            run(["rustup", "component", "add", "llvm-tools", "--toolchain", "1.97.0"])
        profiles = sorted(profile_dir.glob("daemon-*.profraw"))
        if not profiles: raise RuntimeError("training produced no daemon profiles")
        profile = output / "seed.profdata"
        run([profdata, "merge", "-o", profile, *profiles])
        if before != source_digest(source_repo): raise RuntimeError("source changed during training")
        metadata = manifest(binary, source_repo, profile)
        metadata["training_corpus"] = corpus["training"]
        metadata["training_binary_sha256"] = sha256(binary)
        metadata["instrumentation_flags"] = environment["RUSTFLAGS"]
        Path(str(profile) + ".manifest.json").write_text(json.dumps(metadata, indent=2) + "\n")
        print(profile)
    else:
        analyses = []
        for index, workload in enumerate(corpus["validation"]):
            argv = [sys.executable, PROBE, "--repeats", args.repeats,
                    "--output", output / f"held-out-{index}", *workload_args(workload)]
            for client in args.client: argv += ["--client", client]
            for value in args.client_manifest: argv += ["--client-manifest", value]
            if args.client_cpus: argv += ["--client-cpus", args.client_cpus]
            run(argv)
            report = json.loads((output / f"held-out-{index}/summary.json").read_text())
            comparisons = [("baseline", "candidate"), ("baseline-pgo", "candidate-pgo"),
                           ("baseline", "baseline-pgo"), ("candidate", "candidate-pgo")]
            if "rtorrent" in metadata:
                comparisons.append(("rtorrent", "candidate-pgo"))
            analyses.append({"workload": workload, "comparisons": [
                paired_analysis(report, control, candidate) for control, candidate in comparisons]})
            (output / "paired-analysis.json").write_text(json.dumps(analyses, indent=2) + "\n")


if __name__ == "__main__":
    main()
