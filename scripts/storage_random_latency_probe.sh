#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
STORAGE_ROOT="${TNG_STORAGE_LATENCY_ROOT:-}"
OUT="${TNG_STORAGE_LATENCY_REPORT:-$ROOT/certification/reports/storage-random-latency-$STAMP.md}"
FILE_MIB="${TNG_STORAGE_LATENCY_FILE_MIB:-128}"
SAMPLE_COUNT="${TNG_STORAGE_LATENCY_SAMPLE_COUNT:-2000}"
TRIALS="${TNG_STORAGE_LATENCY_TRIALS:-3}"
BLOCK_SIZE=4096
SOURCE_REVISION="${TNG_STORAGE_LATENCY_SOURCE_REVISION:-$(git -C "$ROOT" rev-parse --verify HEAD 2>/dev/null || echo unknown)}"
SOURCE_TREE="${TNG_STORAGE_LATENCY_SOURCE_TREE:-$(git -C "$ROOT" rev-parse 'HEAD^{tree}' 2>/dev/null || echo unknown)}"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "missing required command: $1" >&2
    exit 127
  }
}

usage() {
  cat <<'USAGE'
usage: scripts/storage_random_latency_probe.sh

Writes a disposable data file on the selected storage mount, performs
synchronous 4 KiB O_DIRECT random reads, records per-read latency percentiles,
and removes the test data on exit.

Required:
  TNG_STORAGE_LATENCY_ROOT         Existing non-symlink writable mount

Optional:
  TNG_STORAGE_LATENCY_REPORT       Persistent Markdown report path
  TNG_STORAGE_LATENCY_FILE_MIB     Test file size, 32–1024 MiB (default: 128)
  TNG_STORAGE_LATENCY_SAMPLE_COUNT Reads per trial, 100–100000 (default: 2000)
  TNG_STORAGE_LATENCY_TRIALS       Trial count, 1–9 (default: 3)
USAGE
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi
[[ $# -eq 0 ]] || { usage >&2; exit 2; }

for cmd in awk date dd findmnt git hostname iostat ionice lsblk mkdir mktemp nice nproc paste python3 realpath rm sed sha256sum tail uname; do
  need "$cmd"
done

[[ -n "$STORAGE_ROOT" && -d "$STORAGE_ROOT" && ! -L "$STORAGE_ROOT" ]] || {
  echo "set TNG_STORAGE_LATENCY_ROOT to an existing non-symlink physical mount" >&2
  exit 2
}
STORAGE_ROOT="$(realpath -e -- "$STORAGE_ROOT")"
[[ "$STORAGE_ROOT" != / && -x "$STORAGE_ROOT" && -w "$STORAGE_ROOT" ]] || {
  echo "storage root must be writable and cannot be /" >&2
  exit 2
}
[[ "$FILE_MIB" =~ ^[0-9]+$ && "$FILE_MIB" -ge 32 && "$FILE_MIB" -le 1024 ]] || exit 2
[[ "$SAMPLE_COUNT" =~ ^[0-9]+$ && "$SAMPLE_COUNT" -ge 100 && "$SAMPLE_COUNT" -le 100000 ]] || exit 2
[[ "$TRIALS" =~ ^[1-9]$ ]] || exit 2

mkdir -p "$(dirname "$OUT")"
SCRATCH="$(mktemp -d "$STORAGE_ROOT/.torrentng-random-latency.XXXXXX")"
chmod 700 "$SCRATCH"
DATA_FILE="$SCRATCH/random-read.bin"
RESULTS="$SCRATCH/results.tsv"
IOSTAT_BEFORE="$SCRATCH/iostat-before.txt"
IOSTAT_AFTER="$SCRATCH/iostat-after.txt"

cleanup() {
  local status=$?
  if [[ -d "$SCRATCH" && ! -L "$SCRATCH" ]]; then
    if ! rm -rf -- "$SCRATCH"; then
      status=1
    fi
  fi
  if [[ -e "$SCRATCH" ]]; then
    status=1
  fi
  if [[ -f "$OUT" ]]; then
    {
      echo
      echo "## Cleanup"
      echo
      echo "- Storage scratch removed: $([[ ! -e "$SCRATCH" ]] && echo yes || echo no)"
      echo "- Exit status: $status"
    } >>"$OUT"
  fi
  return "$status"
}
trap cleanup EXIT

{
  echo "# TorrentNG Direct Random-Read Latency Probe"
  echo
  echo "- Started: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "- Host: $(hostname -f 2>/dev/null || hostname)"
  echo "- Source revision: $SOURCE_REVISION"
  echo "- Source tree: $SOURCE_TREE"
  echo "- Probe script SHA-256: $(sha256sum "${BASH_SOURCE[0]}" | awk '{print $1}')"
  echo "- Kernel: $(uname -srmo)"
  echo "- Host CPU threads: $(nproc --all)"
  echo "- Probe CPU affinity: $(awk '/Cpus_allowed_list/ {print $2}' /proc/self/status)"
  echo "- Host load at start: $(awk '{print $1 ", " $2 ", " $3}' /proc/loadavg)"
  echo "- Available RAM at start: $(awk '/MemAvailable/ {printf "%.1f GiB", $2/1048576}' /proc/meminfo)"
  echo "- Target filesystem: $(findmnt -n -T "$STORAGE_ROOT" -o SOURCE,FSTYPE)"
  echo "- Target devices: $(lsblk -s -n -o NAME,ROTA "$(findmnt -n -T "$STORAGE_ROOT" -o SOURCE | sed 's/\[.*//')" 2>/dev/null | paste -sd ',' - || echo unknown)"
  echo "- Test file: $FILE_MIB MiB, written with direct I/O and fdatasync"
  echo "- Workload: synchronous 4 KiB random reads with O_DIRECT; $SAMPLE_COUNT operations per trial; $TRIALS trials"
  echo "- Scheduling: nice 15, idle I/O priority"
  echo
  echo "Latency is end-to-end read completion time through the filesystem, device-mapper, and block layer. It is not a direct measurement of physical head travel. O_DIRECT bypasses the page cache for data reads."
  echo
  echo "System-wide device sample before the probe (includes unrelated workloads):"
  iostat -dx 1 2 >"$IOSTAT_BEFORE" 2>&1 || true
  tail -40 "$IOSTAT_BEFORE"
} >"$OUT"

echo "==> writing $FILE_MIB MiB direct-I/O scratch file on $STORAGE_ROOT" >&2
nice -n 15 ionice -c 3 dd if=/dev/zero of="$DATA_FILE" bs=1M count="$FILE_MIB" oflag=direct conv=fdatasync status=none

echo "==> measuring synchronous random-read completion latency" >&2
nice -n 15 ionice -c 3 python3 - "$DATA_FILE" "$FILE_MIB" "$SAMPLE_COUNT" "$TRIALS" "$BLOCK_SIZE" "$RESULTS" <<'PY'
import math
import mmap
import os
import random
import statistics
import sys
import time

path, file_mib, sample_count, trials, block_size, output = sys.argv[1:]
file_bytes = int(file_mib) * 1024 * 1024
sample_count = int(sample_count)
trials = int(trials)
block_size = int(block_size)
if file_bytes % block_size:
    raise SystemExit("test file size must be divisible by block size")
if not hasattr(os, "O_DIRECT") or not hasattr(os, "preadv"):
    raise SystemExit("Python or this kernel does not expose O_DIRECT/preadv")

fd = os.open(path, os.O_RDONLY | os.O_DIRECT)
buffer = mmap.mmap(-1, block_size)
view = memoryview(buffer)
rng = random.Random(20261012)

def percentile(values, pct):
    values = sorted(values)
    return values[max(0, math.ceil(pct * len(values)) - 1)]

with open(output, "w", encoding="utf-8") as result:
    result.write("trial\treads\tp50_us\tp95_us\tp99_us\tmean_us\tmax_us\treads_per_sec\tMiB_per_sec\n")
    for trial in range(1, trials + 1):
        latencies_us = []
        started = time.perf_counter_ns()
        for _ in range(sample_count):
            offset = rng.randrange(file_bytes // block_size) * block_size
            before = time.perf_counter_ns()
            read = os.preadv(fd, [view], offset)
            elapsed = time.perf_counter_ns() - before
            if read != block_size:
                raise OSError(f"short direct read at offset {offset}: {read} bytes")
            latencies_us.append(elapsed / 1000.0)
        wall_seconds = (time.perf_counter_ns() - started) / 1_000_000_000
        read_count = len(latencies_us)
        read_mib = read_count * block_size / 1048576
        result.write(
            f"{trial}\t{read_count}\t{percentile(latencies_us, .50):.2f}"
            f"\t{percentile(latencies_us, .95):.2f}\t{percentile(latencies_us, .99):.2f}"
            f"\t{statistics.fmean(latencies_us):.2f}\t{max(latencies_us):.2f}"
            f"\t{read_count / wall_seconds:.2f}\t{read_mib / wall_seconds:.2f}\n"
        )
        result.flush()

view.release()
buffer.close()
os.close(fd)
PY

python3 - "$RESULTS" "$OUT" <<'PY'
import csv
import sys

rows = list(csv.DictReader(open(sys.argv[1], newline=""), delimiter="\t"))
with open(sys.argv[2], "a", encoding="utf-8") as report:
    report.write("\n## Direct-I/O random-read results\n\n")
    report.write("| Trial | Reads | p50 latency (µs) | p95 latency (µs) | p99 latency (µs) | Mean latency (µs) | Max latency (µs) | IOPS | MiB/s |\n")
    report.write("|---:|---:|---:|---:|---:|---:|---:|---:|---:|\n")
    for row in rows:
        report.write(
            f"| {row['trial']} | {row['reads']} | {row['p50_us']} | {row['p95_us']} | "
            f"{row['p99_us']} | {row['mean_us']} | {row['max_us']} | "
            f"{row['reads_per_sec']} | {row['MiB_per_sec']} |\n"
        )
    report.write("\n")
    report.write("System-wide device sample after the probe (includes unrelated workloads):\n\n")
PY
iostat -dx 1 2 >"$IOSTAT_AFTER" 2>&1 || true
tail -40 "$IOSTAT_AFTER" >>"$OUT"

{
  echo
  echo "- Host load at end: $(awk '{print $1 ", " $2 ", " $3}' /proc/loadavg)"
  echo "- Available RAM at end: $(awk '/MemAvailable/ {printf "%.1f GiB", $2/1048576}' /proc/meminfo)"
} >>"$OUT"
