#!/usr/bin/env python3
"""Exploration, independent qualification, and held-out service checks."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parent
repo = root / "TorrentNG"
bins = root / "seed-execution-binaries"
parser = argparse.ArgumentParser()
parser.add_argument("mode", choices=("screen", "qualify", "services"))
parser.add_argument("--selected", choices=("scoped-pgo-buffered", "scoped-pgo-mapped",
                                        "scoped-ordered-buffered", "scoped-ordered-mapped"))
parser.add_argument("--alternate", choices=("scoped-pgo-buffered", "scoped-pgo-mapped",
                                         "scoped-ordered-buffered", "scoped-ordered-mapped"))
args = parser.parse_args()
if args.mode != "screen" and args.selected is None:
    parser.error("qualification and services require an explicitly frozen selection")
os.sched_setaffinity(0, {2, 3})


def clients(labels):
    argv = []
    for name in labels:
        kind = "torrentngd"
        env = {"TNG_SEED_MMAP": "0", "TNG_STORAGE_ANON_LARGE_FRAMES": "0"}
        if name == "rtorrent":
            kind = "rtorrent"
            binary = root / "rtorrent-prefix/bin/rtorrent"
            manifest = root / "bench-binaries/rtorrent.manifest.json"
            env = {}
        else:
            if name == "base-pgo":
                binary = bins / "base-mmap-pgo"
            elif name == "previous-selected":
                binary = bins / "refined-3-ordered"
            else:
                binary = bins / ("scoped-3-ordered" if "ordered" in name else "scoped-3-pgo")
                env = {"TNG_SEED_MMAP": "1" if name.endswith("-mapped") else "0",
                       "TNG_STORAGE_ANON_LARGE_FRAMES": "1"}
            manifest = Path(str(binary) + ".manifest.json")
        argv += ["--client", f"{name}={kind}:{binary}", "--client-manifest", f"{name}={manifest}"]
        for key, value in env.items():
            argv += ["--client-env", f"{name}:{key}={value}"]
    return argv


def run(output, labels, mib, rate, peers=1, window=16, pattern="sequential", files=1,
        cache="warm", upload_limit=0, repeats=1, warmup=5, cooldown=5, order_seed=240240):
    argv = [sys.executable, str(repo / "scripts/native_seed_cpu_ram_probe.py"),
            "--output", str(output), "--size", str(mib * 1024**2),
            "--rate", str(rate * 1024**2), "--peers", str(peers), "--window", str(window),
            "--request-pattern", pattern, "--fixture-files", str(files), "--cache-policy", cache,
            "--upload-limit", str(upload_limit), "--repeats", str(repeats),
            "--warmup", str(warmup), "--cooldown", str(cooldown),
            "--order-seed", str(order_seed), "--client-cpus", "0-1", *clients(labels)]
    subprocess.run(argv, check=True)


if args.mode == "screen":
    run("/tmp/torrentng-scoped-optimized-screen-20261024",
        ["previous-selected", "scoped-pgo-buffered", "scoped-pgo-mapped",
         "scoped-ordered-buffered", "scoped-ordered-mapped", "rtorrent"],
        mib=32, rate=4, repeats=3, warmup=2, cooldown=2, order_seed=240239)
elif args.mode == "qualify":
    run("/tmp/torrentng-seed-qualification-20261024",
        ["base-pgo", "previous-selected", args.selected,
         *([args.alternate] if args.alternate else []), "rtorrent"],
        mib=64, rate=4, repeats=10, order_seed=240240)
else:
    corpus = json.loads((repo / "certification/benchmarks/seed-efficiency-corpus.json").read_text())
    for index, work in enumerate(corpus["validation"]):
        run(Path("/tmp/torrentng-seed-held-out-20261024") / f"shape-{index}",
            ["previous-selected", args.selected,
             *([args.alternate] if args.alternate else []), "rtorrent"],
            mib=work["mib"], rate=work["mib_per_second"], peers=work["peers"],
            window=work["window"], pattern=work["pattern"], files=work.get("files", 1),
            cache=work.get("cache", "warm"), upload_limit=work.get("upload_limit", 0),
            order_seed=240241 + index)
