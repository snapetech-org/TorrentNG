#!/usr/bin/env python3
"""Diagnostic screen of scoped verification reuse and mapped seed activation."""
from pathlib import Path
import os
import subprocess
import sys

root = Path(__file__).resolve().parent
binary = root / "seed-execution-binaries/scoped-verification-diagnostic"
args = [sys.executable, root / "TorrentNG/scripts/native_seed_cpu_ram_probe.py",
        "--diagnostics", "--size", str(32 * 1024**2), "--rate", str(4 * 1024**2),
        "--window", "16", "--repeats", "3", "--warmup", "2", "--cooldown", "2",
        "--order-seed", "240238", "--client-cpus", "0-1",
        "--output", "/tmp/torrentng-scoped-verification-screen-20261024"]
for scope in (0, 1):
    for mapped in (0, 1):
        name = f"{'scoped' if scope else 'legacy'}-{'mapped' if mapped else 'buffered'}"
        args += ["--client", f"{name}=torrentngd:{binary}", "--client-manifest",
                 f"{name}={binary}.manifest.json", "--client-env",
                 f"{name}:TNG_STORAGE_ANON_LARGE_FRAMES={scope}",
                 "--client-env", f"{name}:TNG_SEED_MMAP={mapped}"]
os.sched_setaffinity(0, {2, 3})
subprocess.run([str(value) for value in args], check=True)
