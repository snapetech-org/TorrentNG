# Single-peer glibc arena cap screen — 2026-10-23

**Status: unpromoted; possible memory-only runtime tuning option.** The initial screen suggested lower PSS and anonymous memory, but the order-balanced confirmation was mixed: memory fell in 5/6 pairs and CPU in 3/6. CPU pressure rose during confirmation, so the paired CPU effect is not established. No product default or source changed.

## Initial screen measurement

Three randomized 64 MiB loopback seed pairs used the same release executable, 4 MiB/s offered rate, 16 KiB blocks, request window 32, and `pread`. DHT, PEX, and tracker were disabled. All six transfers verified fixture SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`. The only treatment difference was `MALLOC_ARENA_MAX=2` for the arena-capped process.

| Treatment | Median CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Current environment | 4.767 | 18.114 MiB | 5.508 MiB |
| Arena cap 2 | 4.618 (-3.1%) | 17.751 MiB (-2.0%) | 5.039 MiB (-8.5%) |

The first screen's paired CPU changes were -15.7%, +62.9%, and +3.7%, although PSS and anonymous memory were lower in all three pairs. An order-balanced three-pair confirmation completed all transfers but remained mixed: across six pairs total, `MALLOC_ARENA_MAX=2` lowered PSS and anonymous memory in 5/6 pairs, while CPU was lower in only 3/6. The paired median changes were -1.2% CPU, -3.1% PSS, and -6.8% anonymous memory; CPU PSI rose from 1.71% to 6.67% avg10 during confirmation. Keep this as a possible memory-only runtime tuning option. Do not make it the default or claim a joint win; the paired CPU effect is not established. Confirmation evidence is in [`confirmation-summary.json`](confirmation-summary.json), [`confirmation-host-load.txt`](confirmation-host-load.txt), and [`confirmation-validation.txt`](confirmation-validation.txt).

## Scope and identities

This is a current TorrentNG release-to-release configuration screen; it does not include rTorrent and makes no comparator claim. The baseline executable SHA-256 is `b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263` and was identical for both treatments. Exact sanitized runs and medians are in [`summary.json`](summary.json); host snapshots are in [`host-load.txt`](host-load.txt). The harness deletes payload copies after verification; this archive excludes its temporary databases, API tokens, credentials, and logs.
