# Current torrent-client comparison — 2026-10-24

## Result

Three randomized rounds compared the TorrentNG release-default engine with
qBittorrent, Transmission, Deluge, and upstream rTorrent in a private local
seed test. All 15 transfers sent the same 64 MiB fixture and passed its
SHA-256 check.

TorrentNG remains well above rTorrent on CPU and anonymous memory in this
screen. It used less anonymous memory than Transmission and Deluge, and was
near qBittorrent, but its CPU was higher than all four peers in the
whole-transfer window. This is a small, shared-host screening comparison, not
a parity qualification.

| Client | Version | Whole-window CPU (s/GiB) ↓ | Active-window CPU (s/GiB) ↓ | Peak cgroup anonymous MiB ↓ | Process-tree PSS peak MiB |
|---|---|---:|---:|---:|---:|
| TorrentNG | source build 304a45e | 4.17 (3.20–5.54) | 3.18 (3.14–5.37) | 7.2 (6.1–7.2) | 16.5 (15.2–18.6) |
| qBittorrent | 5.3.0 | 2.04 (1.53–2.61) | 2.33 (2.04–2.61), 2 valid rounds | 6.4 (4.9–6.4) | n/a |
| Transmission | 4.1.3 | 2.09 (2.02–2.13) | 1.42 (1.40–2.07) | 17.5 (15.5–18.5) | n/a |
| Deluge | 2.2.0 | 3.22 (3.09–3.26) | 3.22 (3.09–3.26) | 82.8 (82.7–84.7) | n/a |
| rTorrent | 0.16.26 | 1.12 (1.02–1.19) | 0.85 (0.66–1.01) | 3.1 (3.1–3.1) | 18.7 (18.2–19.8) |

Each value is the median of three rounds; the range in parentheses is the
observed minimum and maximum. CPU and anonymous-memory comparisons use
per-round ratios as well as the independent medians above. The median paired
CPU ratio was 4.09x for TorrentNG/rTorrent, 1.60x for TorrentNG/qBittorrent,
2.00x for TorrentNG/Transmission, and 1.30x for TorrentNG/Deluge. Median paired
anonymous-memory ratios were 2.32x, 1.13x, 0.39x, and 0.087x respectively.

The CPU table separates all measured transfer-window CPU from active payload
CPU. rTorrent's first payload arrived after a median 61.4 seconds, compared
with 6.1 seconds for TorrentNG. The active-window measure excludes that
pre-payload wait; it is still approximately 4.76x rTorrent's on the median of
paired rounds. The measured active goodput was close to the shared 4 MiB/s
receiver cap for every client.

TorrentNG and rTorrent had process-tree PSS samples in all three rounds:
medians were 16.5 and 18.7 MiB. PSS sampling ran every 15 seconds and was
unavailable for qBittorrent, Transmission, and Deluge. The primary
cross-client RAM measure here is cgroup anonymous memory sampled every two
seconds; it excludes charged file cache. These categories should not be
collapsed into a single “RAM” ranking.

## Test setup

- TorrentNG application source: commit 304a45eeaa30a70360980700cb8473bc8a17c5ee,
  clean tree fingerprint
  ce7108e1bc1384ec0d7b10a066103ee7482dbd2379c53fa0c87de3ab72f4343a.
- Current main at publication is 5e2c11e. The commits after the measured source
  changed GitHub Actions release and Unraid alias workflows; the native engine,
  workspace manifests, and lockfile match the measured build. The comparison
  branch changes benchmark tooling and evidence, not engine code.
- TorrentNG build: system allocator, release profile, no PGO or seed-mmap
  override, runtime/storage worker and memory-cap defaults. The same retained
  image was used in all three rounds; image manifest digest
  sha256:9c3549149dfb0a7c1f699378640645faaf8063e48eac1144748361162adb399c.
- Reference versions: qBittorrent 5.3.0, Transmission 4.1.3, Deluge 2.2.0,
  and rTorrent/libtorrent 0.16.26. Reference container images were pinned by
  digest or source revision in the Compose configuration.
- Fixture: 67,108,864 bytes, SHA-256
  f46d783f5397975f64a3a4628a259fbb6681f5dae379855838badeba88b68e78.
- Each client seeded the same fixture to one local qBittorrent leecher over an
  isolated Docker network and private tracker. The receiver was capped at
  4 MiB/s; seeders had a 32 MiB/s cap. DHT, peer exchange, and public swarms
  were not used.
- Client order was randomized with seeds 20261024, 20261025, and 20261026.
  Cgroup samples were taken every two seconds; process-tree RSS/PSS every
  fifteen seconds. The report retains each per-round result and raw sample.
- Host pressure was substantial: across 336 five-second samples, median
  one-minute load was 42.41 and peaked at 77.76 on a 32-logical-CPU host.
  Median CPU PSI some was 2.76%; median I/O PSI some was 32.61% and median
  I/O PSI full was 16.34%. The workload was sequential, but concurrent host
  activity and sustained I/O pressure limit confidence in small differences.

This test compared one local seed transfer per client per round. It did not
measure public-swarm behavior, multi-torrent capacity, a concurrent peer mix,
or idle memory as a release qualification. rTorrent's long start-to-payload
delay is reported separately from active-seed cost.

## Configuration and evidence

- [Exact run configuration and reproduction notes](run-config.md)
- [Benchmark Compose configuration](../../../deploy/interop/compose.yml)
- [Pinned upstream rTorrent build configuration](../../../deploy/interop/compose.benchmark-rtorrent-current.yml)
- [Benchmark harness](../../../scripts/client_performance_benchmark.py)
- Round 1: [report](round-01.md), [raw samples](round-01-samples.csv),
  [memory profile](round-01-memory-profile.json), [console log](console/round-01.log)
- Round 2: [report](round-02.md), [raw samples](round-02-samples.csv),
  [memory profile](round-02-memory-profile.json), [console log](console/round-02.log)
- Round 3: [report](round-03.md), [raw samples](round-03-samples.csv),
  [memory profile](round-03-memory-profile.json), [console log](console/round-03.log)
- [Host pressure samples](host-pressure.csv)
- [Client performance research index](../../../docs/research/README.md)

The individual reports include the exact container references and each
client's transfer result. They preserve startup timing, cgroup memory
categories, block I/O counters, and per-round measurements.
