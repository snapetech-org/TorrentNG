#!/usr/bin/env python3
"""Summarize completed, hash-verified native-daemon trials without pooling workloads."""
import argparse
import json
import statistics
from pathlib import Path

METRICS = {
    "read_jobs": 'torrentng_storage_read_ops_by_class_total{class="peer_read"}',
    "backend_reads": 'torrentng_storage_backend_read_ops_by_class_total{class="peer_read"}',
    "backend_bytes": 'torrentng_storage_backend_bytes_read_by_class_total{class="peer_read"}',
    "cache_entries_end": "torrentng_storage_peer_read_cache_entries",
    "cache_bytes_end": "torrentng_storage_peer_read_cache_bytes",
    "cache_backing_bytes_end": "torrentng_storage_peer_read_cache_global_backing_bytes",
}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("directories", type=Path, nargs="+")
    args = parser.parse_args()
    for directory in args.directories:
        rows = []
        groups = {}
        for path in sorted(directory.glob("*/result.json")):
            result = json.loads(path.read_text())
            metrics = {}
            for line in path.with_name("metrics.txt").read_text().splitlines():
                if line and not line.startswith("#"):
                    key, value = line.rsplit(" ", 1)
                    metrics[key] = float(value)
            row = {
                "trial": path.parent.name,
                "label": result["label"],
                "cpu_s_per_gib": result["cpu_s_per_gib"],
                "peak_anon_mib": result["peak_anon_kib"] / 1024,
                "seconds": result["elapsed_seconds"],
                "payload_bytes": result["payload_bytes"],
                "refill": result["refill_blocks"],
                "window": result["request_window"],
                "rate_cap_mib_s": result["rate_cap_mib_s"],
                **{key: metrics.get(name) for key, name in METRICS.items()},
            }
            if row["backend_bytes"] is not None:
                row["backend_mib_per_gib"] = row["backend_bytes"] / result["payload_bytes"] * 1024
            rows.append(row)
            group = (row["label"], row["refill"], row["window"], row["rate_cap_mib_s"], row["payload_bytes"])
            groups.setdefault(group, []).append(row)
        medians = []
        for group, values in groups.items():
            medians.append({
                "label": group[0], "refill": group[1], "window": group[2],
                "rate_cap_mib_s": group[3], "payload_bytes": group[4], "n": len(values),
                **{key: statistics.median(row[key] for row in values)
                   for key in ("cpu_s_per_gib", "peak_anon_mib", "seconds", *METRICS, "backend_mib_per_gib")
                   if all(row.get(key) is not None for row in values)},
            })
        print(json.dumps({"directory": str(directory), "rows": rows, "medians": medians}, indent=2))


if __name__ == "__main__":
    main()
