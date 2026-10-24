#!/usr/bin/env python3
"""Analyze the frozen experiment; never qualify partial/diagnostic data."""
import json
from pathlib import Path
import random
import statistics
import sys

root = Path(__file__).resolve().parent
sys.path.insert(0, str(root / "TorrentNG/scripts"))
from seed_efficiency_tools import paired_analysis

source = Path("/tmp/torrentng-seed-qualification-20261024/summary.json")
out = root / "TorrentNG/certification/reports/TorrentNG-seed-execution-20261023"
report = json.loads(source.read_text())
assert report["status"] == "complete"
assert len(report["runs"]) == 50
assert all(r["transfer"]["verified"] and not r["diagnostic_run"] for r in report["runs"])
labels = list(dict.fromkeys(r["name"] for r in report["runs"]))
assert all(sum(r["name"] == label for r in report["runs"]) == 10 for label in labels)


def setup_cpu(run):
    d = run.get("torrentng_diagnostics")
    if not d:
        return None
    first = d["before_torrent_add"]["process"]["cpu_time_user_system_ns"]
    last = d["after_seed_recheck"]["process"]["cpu_time_user_system_ns"]
    return (last - first) / 1e9


def metrics(run):
    transfer = run["transfer"]
    peers = transfer.get("peer_transfers", [transfer])
    return {
        "cpu_seconds_per_gib": run["payload_cpu_seconds_user_system"] / (transfer["bytes"] / 2**30),
        "peak_pss_mib": run["active_peak_pss_bytes"] / 2**20,
        "peak_anonymous_mib": run["active_peak_anonymous_bytes"] / 2**20,
        "payload_goodput_mib_s": transfer["bytes"] / transfer["payload_elapsed_seconds"] / 2**20,
        "max_peer_first_payload_ms": max(p["first_payload_delay_seconds"] for p in peers) * 1000,
        "connection_wait_cpu_seconds": run["connection_wait_cpu_seconds"],
        "setup_recheck_cpu_seconds": setup_cpu(run),
    }


def interval(ratios):
    rng = random.Random(20261024)
    draws = sorted(statistics.median(rng.choices(ratios, k=len(ratios))) for _ in range(10000))
    return {"paired_median": statistics.median(ratios), "interval95": [draws[250], draws[9749]],
            "paired_ratios": ratios, "scope": "secondary descriptive interval; outside co-primary family"}


cells = {}
for label in labels:
    rows = [{"round": r["round"], **metrics(r)} for r in report["runs"] if r["name"] == label]
    cells[label] = {"rows": rows, "metrics": {}}
    for key in rows[0]:
        if key == "round":
            continue
        values = [row[key] for row in rows if row[key] is not None]
        cells[label]["metrics"][key] = ({"median": statistics.median(values), "min": min(values),
                                        "max": max(values)} if values else None)

comparisons = [paired_analysis(report, control, candidate) for candidate in
               ("scoped-ordered-buffered", "scoped-ordered-mapped")
               for control in ("base-pgo", "previous-selected", "rtorrent")]
comparisons.append(paired_analysis(report, "scoped-ordered-buffered", "scoped-ordered-mapped"))
by = {(r["name"], r["round"]): r for r in report["runs"]}
secondary = []
for candidate in ("scoped-ordered-buffered", "scoped-ordered-mapped"):
    for control in ("base-pgo", "previous-selected"):
        secondary.append({"candidate": candidate, "control": control,
                          "setup_recheck_cpu": interval([setup_cpu(by[candidate, n]) /
                                                         setup_cpu(by[control, n]) for n in range(1, 11)])})
result = {"format": "torrentng-seed-execution-qualified-v1", "primary": "scoped-ordered-buffered",
          "secondary": "scoped-ordered-mapped", "cells": cells, "comparisons": comparisons,
          "secondary_phase_comparisons": secondary,
          "interpretation": ["Each resource interval has Bonferroni joint95 coverage within its comparison.",
                             "No correction covers the entire exploratory family of comparisons.",
                             "Primary selection was frozen before this data; secondary may not replace it post hoc.",
                             "Resource gates do not certify every workload or service-latency parity."]}
(out / "paired-analysis.json").write_text(json.dumps(result, indent=2) + "\n")
for label, cell in cells.items():
    print(label, {key: round(value["median"], 6) for key, value in cell["metrics"].items() if value})
for c in comparisons:
    print(c["candidate"], "vs", c["control"], c["resource_gate"],
          {key: {"ratio": round(value["paired_median"], 6),
                 "interval": [round(v, 6) for v in value["interval"]]}
           for key, value in c["metrics"].items()})
