# Reproduction configuration

This records the comparison run that produced the three round reports in this
directory. The benchmark source is pinned by commit and fingerprint in each
report. It used one retained TorrentNG image for all rounds.

## Fixed inputs

- Mode: controlled private seed; no public swarm, DHT, peer exchange, or public
  tracker requests.
- TorrentNG source commit: 304a45eeaa30a70360980700cb8473bc8a17c5ee.
- TorrentNG image: torrentng/torrentngd:benchmark-20261024t021957z;
  manifest digest sha256:9c3549149dfb0a7c1f699378640645faaf8063e48eac1144748361162adb399c.
- Build: release, system allocator, no PGO and no seed-mmap override.
- Build CPU cap: four logical CPUs. Build and image preparation completed
  before measurement; rounds two and three reused the exact retained image.
- Fixture: 67,108,864 bytes, SHA-256
  f46d783f5397975f64a3a4628a259fbb6681f5dae379855838badeba88b68e78.
- Each comparison round transferred the fixture once through each seeder to
  one qBittorrent 5.2.3 leecher. The leecher cap was 4 MiB/s; each seeder was
  capped at 32 MiB/s; peer limit was 32.
- Cgroup samples: 2 seconds. Process-tree RSS/PSS samples: 15 seconds.
  Diagnostic metrics and detailed smaps were disabled.
- Reference versions/images and their digests are recorded in each round
  report. The upstream rTorrent build used rTorrent/libtorrent 0.16.26.

## Randomized client order

| Round | Order seed | Order |
|---|---:|---|
| 1 | 20261024 | Deluge, TorrentNG, Transmission, qBittorrent, rTorrent |
| 2 | 20261025 | qBittorrent, Transmission, TorrentNG, rTorrent, Deluge |
| 3 | 20261026 | Deluge, rTorrent, Transmission, qBittorrent, TorrentNG |

## Command shape

Set SEED_SOURCE to a local copy of the pinned fixture, then run once per
round from the repository root with a unique work directory and report path:

    TNG_CLIENT_BENCH_MODE=controlled-seed \
    TNG_CLIENT_BENCH_SEED_SOURCE="$SEED_SOURCE" \
    TNG_CLIENT_BENCH_SEED_SHA256=f46d783f5397975f64a3a4628a259fbb6681f5dae379855838badeba88b68e78 \
    TNG_CLIENT_BENCH_ORDER_SEED=20261024 \
    TNG_CLIENT_BENCH_BUILD_CPUS=4 \
    TNG_CLIENT_BENCH_KEEP_IMAGE=1 \
    TNG_CLIENT_BENCH_PROCESS_TREE_SAMPLE_SECS=15 \
    TNG_CLIENT_BENCH_SCRATCH_ROOT=/path/to/task-scratch \
    TNG_CLIENT_BENCH_WORKDIR=/path/to/new-work-round-01 \
    TNG_CLIENT_BENCH_REPORT=certification/reports/TorrentNG-current-client-comparison-20261024/round-01.md \
    python3 scripts/client_performance_benchmark.py

Rounds two and three reused the retained source-built image and changed only
the order seed, unique work directory, and report path:

    TNG_CLIENT_BENCH_MODE=controlled-seed \
    TNG_CLIENT_BENCH_SEED_SOURCE="$SEED_SOURCE" \
    TNG_CLIENT_BENCH_SEED_SHA256=f46d783f5397975f64a3a4628a259fbb6681f5dae379855838badeba88b68e78 \
    TNG_CLIENT_BENCH_ORDER_SEED=20261025 \
    TNG_CLIENT_BENCH_SKIP_BUILD=1 \
    TNG_CLIENT_BENCH_PREBUILT_IMAGE=torrentng/torrentngd:benchmark-20261024t021957z \
    TNG_CLIENT_BENCH_PROCESS_TREE_SAMPLE_SECS=15 \
    TNG_CLIENT_BENCH_SCRATCH_ROOT=/path/to/task-scratch \
    TNG_CLIENT_BENCH_WORKDIR=/path/to/new-work-round-02 \
    TNG_CLIENT_BENCH_REPORT=certification/reports/TorrentNG-current-client-comparison-20261024/round-02.md \
    python3 scripts/client_performance_benchmark.py

The harness defaults for rate caps, peer limit, storage/runtime workers, and
memory budgets were left unchanged. The 15-second process-tree sample interval
was explicitly set for all rounds. The full list of supported variables and
the controlled-download procedure are in the
[benchmark guide](../../../docs/CLIENT_PERFORMANCE_BENCHMARK.md).

## Data files

- Per-round reports: [round 1](round-01.md), [round 2](round-02.md),
  [round 3](round-03.md)
- Raw samples and memory profiles are linked from each round report.
- [Host pressure samples](host-pressure.csv)
- Captured benchmark console output: [round 1](console/round-01.log),
  [round 2](console/round-02.log), [round 3](console/round-03.log)
