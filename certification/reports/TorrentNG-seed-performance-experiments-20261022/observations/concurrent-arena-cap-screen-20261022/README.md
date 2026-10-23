# Concurrent-peer arena-cap screen — 2026-10-22

**Status: inconclusive; no runtime default changed.** The existing Linux/glibc arena-cap candidate was screened with three simultaneous local peers because the single-peer result did not establish behavior under concurrency.

## Method

One seeded daemon served three independent loopback leechers concurrently. Each leecher downloaded the same deterministic 64 MiB fixture at 4 MiB/s, so each run served 192 MiB total at 12 MiB/s offered aggregate rate. Every leecher completed and matched fixture SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

The release and arena-cap arms used the same eight-block release daemon binary (SHA-256 `e4e63a0446632660200781f9c0f355a47c80b123be5cf6001125782c79dad276`), with `TNG_STORAGE_BACKEND=pread`; the cap arms also set `MALLOC_ARENA_MAX` to 1 or 2. The rTorrent arm used source-built 0.16.25 (SHA-256 `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`). CPU is process CPU over the complete three-peer transfer, normalized by total verified bytes; memory is the sampled process-tree peak.

## One-round results

| Screen | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| 1 | Release, default arenas | 2.285 | 18.61 MiB | 6.13 MiB |
| 1 | Release, arena cap 1 | 3.219 (+40.9%) | 17.54 MiB (-5.8%) | 4.91 MiB (-19.9%) |
| 1 | rTorrent 0.16.25 | 1.560 | 7.66 MiB | 3.07 MiB |
| 2 | Release, default arenas | 4.392 | 19.04 MiB | 6.33 MiB |
| 2 | Release, arena cap 1 | 4.048 (-7.8%) | 18.31 MiB (-3.8%) | 5.41 MiB (-14.5%) |
| 2 | Release, arena cap 2 | 3.863 (-12.0%) | 18.25 MiB (-4.2%) | 5.76 MiB (-9.0%) |

Both screens passed SHA-256 verification for all peers. Their CPU direction conflicts: cap 1 raised CPU in screen 1, then lowered it in screen 2. The absolute release CPU also differed substantially between screens. Post-screen host samples showed load averages of 34.25/22.23/18.57 and 26.02/21.46/18.43 on a 32-CPU host, alongside unrelated build activity. These one-round results are not a reliable ranking and do not qualify the arena cap under concurrency.

Keep `MALLOC_ARENA_MAX=1` as an optional memory-tuning candidate, not a product default. The earlier single-peer randomized screens still show a repeatable memory reduction, but a quiet-host paired multi-peer comparison is needed before describing its CPU effect under concurrency. The current eight-block batching improvement remains the source default and is not changed by these screens.

The raw JSON summaries and runners are kept beside this note. The runners use the repository probe `scripts/native_seed_cpu_ram_probe.py` and the exact binaries identified above; generated payload copies and API credentials are not included.
