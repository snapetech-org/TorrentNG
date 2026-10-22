# TorrentNG review archive intake — 2026-10-21

## Files moved from Downloads

| Archive | SHA-256 | Contents |
|---|---|---|
| `inputs/TorrentNG-906f5a99-review-files.zip` | `69bbb3bd7b8f6288f48407a6b25c560478058a7e7361d31c6d3199ebee447716` | Four source snapshots and an older whole-client benchmark report. |

The newly downloaded `TorrentNG-rtorrent-hot-path-review(1).zip` had SHA-256 `c882874d661727cad0f17a94c43828c0a691541ad1e8dc52e910fe675ddfb2ef`, identical to the earlier `TorrentNG-rtorrent-hot-path-review.zip` already retained at [`../TorrentNG-rtorrent-hot-path-review/`](../TorrentNG-rtorrent-hot-path-review/). Its four extracted source files also match commit `d2f7269364572403714b7352248c1e1812a89d2f`. Since the archive and source snapshots were already present, the duplicate extracted copies were discarded after verification rather than stored twice.

The `906f5a99` archive was extracted under `extracted/`. Its four Rust files match commit `906f5a996e4c949833adc67987657cb5f1a63a3b`. The review report is retained at [`extracted/TorrentNG-906f5a99-review-files/certification/reports/client-performance-benchmark-hotpath-candidate-full-20261020.md`](extracted/TorrentNG-906f5a99-review-files/certification/reports/client-performance-benchmark-hotpath-candidate-full-20261020.md). It contains summary measurements but no raw sample CSV or memory-profile JSON.

## What the older report measured

That single, shared-host pass completed a SHA-256-verified controlled seed transfer for each client. TorrentNG averaged `3.82 MiB/s`, used `3.78 CPU-s/GiB`, and peaked at `14.7 MiB` cgroup anonymous memory. The references measured `2.29` CPU-s/GiB and `22.5 MiB` for qBittorrent, `2.47` and `20.6 MiB` for Transmission, `2.85` and `85.1 MiB` for Deluge, and `0.68` and `1.9 MiB` for rTorrent. TorrentNG's public download stopped at `6.01%` and timed out; no completed download score resulted.

The run recorded sampled host load up to `30.21` and only one pass per client. It is useful historical evidence, not a current ranking. The later [three-round whole-client seed comparison](../TorrentNG-whole-client-comparison-20261021/README.md) is the stronger current comparison: TorrentNG measured `1.33 CPU-s/GiB` and `16.9 MiB` anonymous peak, versus qBittorrent `2.39` and `6.2 MiB`, Transmission `1.19` and `16.4 MiB`, Deluge `2.68` and `82.7 MiB`, and rTorrent `0.70` and `1.0 MiB`. That comparison still covers one host and one controlled seed workload and has three rounds, below the ten-round target.

## Decision on the file-range upload proposal

The advice in the duplicate review archive recommends a Linux TCP `sendfile` path for verified single-file seed ranges. That exact approach was already implemented as candidate `e9727943033850c6258d282bb0a54311cf9d05b0`, measured, and reverted by `1044556cfeb8aea090feb2d1b879bf14b79636fa`.

In the exploratory comparison, the candidate reduced sampled anonymous peak memory from `16.6 MiB` to `4.3 MiB` (74.1% lower), while CPU rose from `1.33` to `5.51 CPU-s/GiB` (4.14 times the baseline). Throughput stayed around `3.9 MiB/s` under a shared `4 MiB/s` receiver cap. The measurement was one pass per client on a busy shared host, so it does not prove causality; the CPU regression is large enough that this implementation is not a viable change for the CPU-and-RAM goal. See the [candidate report and raw evidence](../TorrentNG-native-download-efa3a607/comparison/round3-sendfile-e9727943/README.md).

**No production code from these archives was applied.** The advice contributes no new patch, and the proposed `sendfile` implementation has already demonstrated an unacceptable CPU tradeoff. Keep its memory reduction as a design target. Any follow-up should preserve the current vectored socket-write path and prove a bounded, path-safe lifetime for any file-backed payload before benchmarking it; do not accept a memory-only win as a client-efficiency win. The next comparison must measure CPU, anonymous memory, file-backed memory, throughput, and transfer correctness on the same source and fixture, with repeated rotated trials.
