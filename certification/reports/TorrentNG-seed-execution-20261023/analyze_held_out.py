#!/usr/bin/env python3
"""Replay the seven one-round service screens from archived, verified summaries."""
import gzip
import json
from pathlib import Path

root = Path(__file__).resolve().parent
labels = {"previous-selected", "scoped-ordered-buffered", "scoped-ordered-mapped", "rtorrent"}
shapes = []
for index in range(7):
    path = root / f"shape-{index}-held-out-summary.json.gz"
    raw = json.loads(gzip.decompress(path.read_bytes()))
    assert raw["status"] == "complete", path
    assert len(raw["runs"]) == 4, path
    assert {run["name"] for run in raw["runs"]} == labels, path
    rows = []
    for run in raw["runs"]:
        transfer = run["transfer"]
        assert transfer["verified"] and not run["diagnostic_run"], path
        peers = transfer.get("peer_transfers", [transfer])
        delays = [peer["first_payload_delay_seconds"] * 1000 for peer in peers]
        rows.append({
            "name": run["name"],
            "binary_sha256": run["binary_sha256"],
            "environment": run["environment_overrides"],
            "verified": True,
            "bytes": transfer["bytes"],
            "payload_goodput_mib_s": transfer["bytes"] / transfer["payload_elapsed_seconds"] / 2**20,
            "cpu_seconds_per_gib": run["payload_cpu_seconds_user_system"] / (transfer["bytes"] / 2**30),
            "peak_pss_mib": run["active_peak_pss_bytes"] / 2**20,
            "peak_anonymous_mib": run["active_peak_anonymous_bytes"] / 2**20,
            "peer_first_payload_ms": delays,
            "max_peer_first_payload_ms": max(delays),
        })
    offered = raw["offered_rate_bytes_per_second"] * raw["peer_count"]
    cap = raw["upload_limit"]
    shapes.append({
        "shape": index,
        "fixture_bytes": raw["fixture_bytes"],
        "peers": raw["peer_count"],
        "window": raw["request_window"],
        "pattern": raw["request_pattern"],
        "cache_policy": raw["cache_policy"],
        "files": raw["fixture_files"],
        "offered_aggregate_mib_s": offered / 2**20,
        "upload_limit_bytes_s": cap,
        "nominal_target_mib_s": min(offered, cap if cap else offered) / 2**20,
        "rows": rows,
    })

result = {
    "format": "torrentng-seed-held-out-service-screen-v1",
    "verified_runs": sum(len(shape["rows"]) for shape in shapes),
    "shapes": shapes,
    "limitations": [
        "One round per shape: descriptive service screen, not statistical CPU/PSS or latency qualification.",
        "Rates are aggregate verified payload goodput; configured offer is per peer, upload cap is per daemon.",
        "drop-file-pages is a cache hint, not proof of cold physical storage.",
        "Peer first-payload samples do not certify latency tails or mixed-workload behavior.",
    ],
}
(root / "held-out-analysis.json").write_text(json.dumps(result, indent=2) + "\n")
for shape in shapes:
    for row in shape["rows"]:
        print(shape["shape"], row["name"],
              {key: round(row[key], 6) for key in
               ("payload_goodput_mib_s", "cpu_seconds_per_gib", "peak_pss_mib", "max_peer_first_payload_ms")})
