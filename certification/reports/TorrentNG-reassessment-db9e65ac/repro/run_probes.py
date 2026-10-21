#!/usr/bin/env python3
"""Reproduce the current-source storage/codec probes without editing the input repo.

Requires cargo/rustc on PATH. Usage: python3 run_probes.py /path/TorrentNG /new/workdir
The destination must not exist. Builds are diagnostic GNU-target release builds,
not the daemon's musl/LTO image. Results are not whole-client benchmark scores.
"""
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

SOURCE = "db9e65ac5807d7b3e3d80842d9e447b331fa81c5"
HERE = Path(__file__).resolve().parent

def run(args, **kwargs):
    return subprocess.run([str(x) for x in args], check=True, **kwargs)

def prepare_probe(repo, destination, lock):
    destination.mkdir()
    shutil.copytree(HERE / "src", destination / "src")
    manifest = (HERE / "Cargo.toml.in").read_text().replace("__REPO__", repo.as_posix())
    (destination / "Cargo.toml").write_text(manifest)
    shutil.copy2(lock, destination / "Cargo.lock")

def check_lock(repo, probe):
    a = tomllib.loads((repo / "Cargo.lock").read_text())
    b = tomllib.loads((probe / "Cargo.lock").read_text())
    allowed = {(x["name"], x["version"], x.get("checksum")) for x in a["package"]}
    for x in b["package"]:
        if "source" in x:
            assert (x["name"], x["version"], x.get("checksum")) in allowed, x["name"]

def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    repo, out = (Path(x).resolve() for x in sys.argv[1:])
    assert not out.is_relative_to(repo), "Use a work directory outside the input checkout"
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo, text=True).strip()
    assert revision == SOURCE, f"Expected {SOURCE}, found {revision}"
    assert not subprocess.check_output(["git", "status", "--porcelain"], cwd=repo, text=True).strip(), "Input repo must be clean"
    out.mkdir(parents=True, exist_ok=False)
    base = out / "baseline-probe"
    prepare_probe(repo, base, repo / "Cargo.lock")
    env = dict(os.environ, CARGO_TARGET_DIR=str(out / "target"))
    cargo = ["cargo", "build", "--release", "--manifest-path", base / "Cargo.toml", "-j", "2"]
    run(cargo, env=env)
    check_lock(repo, base)
    binary = out / "target/release/tng-current-audit-probe"
    baseline = out / "baseline"
    shutil.copy2(binary, baseline)
    codec = subprocess.check_output([str(out / "target/release/codec_credit")], text=True)
    (out / "codec-credit-results.txt").write_text(codec)
    print(codec)

    rows = []
    variants = [("cache", 16384), ("no_cache", 16384), ("small_cache", 16384), ("no_cache", 262144)]
    for repeat in range(3):
        for mode, block in variants[repeat:] + variants[:repeat]:
            result = subprocess.check_output([str(baseline), mode, str(block), str(64 * 1024**2)], text=True)
            row, after = map(json.loads, result.splitlines())
            row.update(after)
            row["repeat"] = repeat
            rows.append(row)
    (out / "storage-probe-results.json").write_text(json.dumps(rows, indent=2) + "\n")

    candidate = out / "candidate"
    candidate.mkdir()
    for name in ["Cargo.toml", "Cargo.lock"]:
        shutil.copy2(repo / name, candidate / name)
    shutil.copytree(repo / "crates", candidate / "crates")
    patch = HERE.parent / "experimental-bounded-512k-pool.patch"
    run(["git", "apply", "--check", patch], cwd=candidate)
    run(["git", "apply", patch], cwd=candidate)
    probe = out / "candidate-probe"
    prepare_probe(candidate, probe, base / "Cargo.lock")
    shutil.copytree(HERE / "tests", probe / "tests")
    run(["cargo", "build", "--release", "--manifest-path", probe / "Cargo.toml", "--bin", "tng-current-audit-probe", "-j", "2"], env=env)
    check_lock(repo, probe)
    candidate_binary = out / "candidate-binary"
    shutil.copy2(binary, candidate_binary)
    run(["cargo", "test", "--release", "--manifest-path", probe / "Cargo.toml", "--test", "pool_patch", "-j", "2"], env=env)
    rows = []
    for repeat in range(3):
        cases = [("baseline", baseline), ("512k-capped", candidate_binary)]
        if repeat % 2:
            cases.reverse()
        for variant, exe in cases:
            result = subprocess.check_output([str(exe), "cache", "16384", str(256 * 1024**2)], text=True)
            row, after = map(json.loads, result.splitlines())
            row.update(after)
            row.update(repeat=repeat, variant=variant)
            rows.append(row)
    (out / "512k-pool-capped-results.json").write_text(json.dumps(rows, indent=2) + "\n")
    print(f"Results: {out}")

if __name__ == "__main__":
    main()
