# Current client comparison — 2026-10-24

## What this says

The latest release-default screen compared TorrentNG with current qBittorrent,
Transmission, Deluge, and rTorrent builds in three randomized rounds. Every
client sent the same locally seeded 64 MiB fixture, and all 15 leecher copies
passed SHA-256 verification.

TorrentNG has **not reached CPU or anonymous-memory parity with rTorrent**.
Across the three runs, TorrentNG used a median 4.17 CPU-s/GiB over the full
transfer window and 3.18 active CPU-s/GiB, versus rTorrent at 1.12 and 0.85.
TorrentNG peak cgroup anonymous memory was 7.2 MiB versus 3.1 MiB. TorrentNG
used less anonymous memory than Transmission and Deluge, and was close to
qBittorrent; CPU was higher than all four clients in the whole-transfer window.

| Client | Version | CPU (s/GiB) | Active CPU (s/GiB) | Peak cgroup anonymous (MiB) |
|---|---|---:|---:|---:|
| TorrentNG | source build 304a45e | 4.17 | 3.18 | 7.2 |
| [qBittorrent](https://www.qbittorrent.org/download) | 5.3.0 | 2.04 | 2.33 (2 valid rounds) | 6.4 |
| [Transmission](https://github.com/transmission/transmission/releases/) | 4.1.3 | 2.09 | 1.42 | 17.5 |
| [Deluge](https://pypi.org/project/deluge/) | 2.2.0 | 3.22 | 3.22 | 82.8 |
| [rTorrent](https://github.com/rakshasa/rtorrent/releases/latest) | 0.16.26 | 1.12 | 0.85 | 3.1 |

These are medians of three rounds. Per-round paired ratios differ from ratios
of the medians; the full report includes the paired ratios, observed ranges,
memory categories, source identity, and limitations.

## Context

The seed transfer was local and isolated, with one qBittorrent leecher capped
at 4 MiB/s. It avoids public-swarm delay and variability. It does not measure
public downloads, multi-torrent capacity, or multi-peer scaling. rTorrent's
first payload arrived after a median 61.4 seconds; TorrentNG's was 6.1 seconds.
Active-window CPU excludes the wait before the first payload and is shown
alongside whole-window CPU.

Anonymous memory is the primary cross-client RAM column because it was
available for every container. PSS was sampled only every 15 seconds and was
available for TorrentNG and rTorrent in these runs; their medians were 16.5
and 18.7 MiB. This different category does not overturn the cgroup-anonymous
comparison or establish a broad memory ranking.

Host I/O pressure remained high during testing: median I/O PSI some was
32.61% and median I/O PSI full was 16.34%. Median host CPU PSI some was 2.76%.
The three-round result is useful for direction and client comparison on this
machine, but small CPU differences need a cleaner, longer qualification.

## Exact evidence

- [Full comparison report and aggregate table](../../certification/reports/TorrentNG-current-client-comparison-20261024/README.md)
- [Exact inputs, randomized orders, and reproduction settings](../../certification/reports/TorrentNG-current-client-comparison-20261024/run-config.md)
- [Round reports, CSV samples, memory profiles, logs, and host-pressure trace](../../certification/reports/TorrentNG-current-client-comparison-20261024/)
- [Benchmark configuration](../../deploy/interop/compose.yml)
- [Benchmark harness](../../scripts/client_performance_benchmark.py)

The more rigorous ten-round qualification against rTorrent and its candidate
configurations are listed in the
[performance research index](README.md). That PGO candidate also failed the
joint CPU/PSS parity gate.
