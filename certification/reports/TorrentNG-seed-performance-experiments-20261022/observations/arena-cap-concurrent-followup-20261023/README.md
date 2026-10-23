# Concurrent allocator follow-up — 2026-10-23

**Status: keep arena caps optional; no new default.** `MALLOC_ARENA_MAX=1` consistently reduced memory but increased CPU in this current three-peer screen. `MALLOC_ARENA_MAX=2` reduced memory across three rounds, while its CPU direction changed in the order-reversed round.

One TorrentNG seed served three independent loopback peers at 4 MiB/s each. Each peer downloaded the same deterministic 64 MiB fixture; all 12 transfers across the four new screens passed SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`. TorrentNG arms used the same eight-block release binary with `TNG_STORAGE_BACKEND=pread`; only `MALLOC_ARENA_MAX` differed. The rTorrent arm was source-built 0.16.25.

## Cap 1 screen

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Default arenas | 2.037 | 18.68 MiB | 6.12 MiB |
| `MALLOC_ARENA_MAX=1` | 2.482 (+21.8%) | 16.85 MiB (-9.8%) | 4.37 MiB (-28.6%) |
| rTorrent 0.16.25 | 1.437 | 7.79 MiB | 3.08 MiB |

Cap 1 is a memory-only option for this workload; it did not improve CPU.

## Cap 2 three-round follow-up

| Build | Median CPU-s/GiB | Median PSS | Median anonymous |
|---|---:|---:|---:|
| Default arenas | 2.336 | 18.996 MiB | 6.363 MiB |
| `MALLOC_ARENA_MAX=2` | 2.303 (-1.4%) | 18.395 MiB (-3.2%) | 5.621 MiB (-11.7%) |
| rTorrent 0.16.25 | 1.273 | 7.74 MiB | 3.08 MiB |

| Round | Run order | Cap 2 CPU change vs control | PSS change | Anonymous change |
|---|---|---:|---:|---:|
| 1 | cap 2 → rTorrent → control | -6.2% | -3.2% | -11.7% |
| 2 | cap 2 → rTorrent → control | -9.6% | -3.2% | -8.7% |
| 3 | control → cap 2 → rTorrent | +16.9% | -4.8% | -14.0% |

The CPU median is effectively tied and the candidate lost in the reversed-order round. Memory reductions repeated. TorrentNG cap 2 still used 1.81× rTorrent CPU and 2.38× its PSS. Keep this as an optional memory tuning; do not describe it as a CPU win or change the default.

The earlier high-load screens are preserved in [`concurrent-arena-cap-screen-20261022`](../concurrent-arena-cap-screen-20261022/README.md). This follow-up changes no production source. The raw summaries, runner scripts, client logs, configs, diagnostics, load snapshots, and binary/fixture identities are kept in this folder. Kernel `smaps` captures are gzip-compressed byte-for-byte. Generated payloads, temporary API tokens, session databases, and bootstrap passwords are excluded.
