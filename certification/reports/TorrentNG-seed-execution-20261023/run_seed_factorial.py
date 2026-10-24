#!/usr/bin/env python3
"""Predeclared exploratory matrix; qualifying data must be collected separately."""
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).parent
binary_dir = root / "seed-execution-binaries"
args = [sys.executable, root / "TorrentNG/scripts/native_seed_cpu_ram_probe.py",
        "--size", str(32 * 1024**2), "--rate", str(4 * 1024**2),
        "--window", "16", "--repeats", "3", "--warmup", "2", "--cooldown", "2",
        "--order-seed", "240236", "--client-cpus", "0-1",
        "--output", root / "seed-execution-factorial"]


def client(label, binary, kind="torrentngd", mapped=None, manifest=None):
    args.extend(["--client", f"{label}={kind}:{binary}", "--client-manifest",
                 f"{label}={manifest or str(binary) + '.manifest.json'}"])
    if mapped is not None:
        args.extend(["--client-env", f"{label}:TNG_SEED_MMAP={mapped}"])


client("base-pgo", binary_dir / "base-mmap-pgo", mapped=0)
client("normal", binary_dir / "refined-normal-default", mapped=0)
for level in ("s", "3"):
    for layout in ("pgo", "ordered"):
        for mapped in (0, 1):
            client(f"{level}-{layout}-{'mapped' if mapped else 'buffered'}",
                   binary_dir / f"refined-{level}-{layout}", mapped=mapped)
client("rtorrent", root / "rtorrent-prefix/bin/rtorrent", kind="rtorrent",
       manifest=root / "bench-binaries/rtorrent.manifest.json")
os.sched_setaffinity(0, {2, 3})
subprocess.run([str(value) for value in args], check=True)
