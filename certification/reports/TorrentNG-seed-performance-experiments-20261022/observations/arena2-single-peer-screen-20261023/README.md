# Single-peer glibc arena cap screen — 2026-10-23

**Status: promising memory screen; confirmation needed before use.** Setting `MALLOC_ARENA_MAX=2` reduced PSS and anonymous memory in every pair. The median CPU was slightly lower, but paired CPU varied sharply while host CPU pressure rose. No product default or source changed.

## Measurement

Three randomized 64 MiB loopback seed pairs used the same release executable, 4 MiB/s offered rate, 16 KiB blocks, request window 32, and `pread`. DHT, PEX, and tracker were disabled. All six transfers verified fixture SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`. The only treatment difference was `MALLOC_ARENA_MAX=2` for the arena-capped process.

| Treatment | Median CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Current environment | 4.767 | 18.114 MiB | 5.508 MiB |
| Arena cap 2 | 4.618 (-3.1%) | 17.751 MiB (-2.0%) | 5.039 MiB (-8.5%) |

PSS and anonymous memory were lower in all three pairs. Paired CPU changes were -15.7%, +62.9%, +3.7%; the cap ran first in two pairs. CPU pressure rose from 0.96% to 3.33% avg10 during the screen, and later rose further. Treat the CPU median as an early signal, not a reliable win. An order-balanced repeat is required before adopting this as an optional runtime profile.

## Scope and identities

This is a current TorrentNG release-to-release configuration screen; it does not include rTorrent and makes no comparator claim. The baseline executable SHA-256 is `b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263` and was identical for both treatments. Exact sanitized runs and medians are in [`summary.json`](summary.json); host snapshots are in [`host-load.txt`](host-load.txt). The harness deletes payload copies after verification; this archive excludes its temporary databases, API tokens, credentials, and logs.
