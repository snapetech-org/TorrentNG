# TorrentNG CPU and RAM comparison: final defaults

## Final reported comparison

The final comparison summary reported medians from three matched runs per
build:

| Build | CPU seconds per transfer ↓ | Process PSS (MiB) ↓ |
|---|---:|---:|
| Previous best TorrentNG configuration | 1.88 | 24.26 |
| Consolidated TorrentNG source, opt-3 | 1.92 | 23.12 |
| **TorrentNG defaults at `0070ef7` (opt-s)** | **1.90** | **19.82** |
| rTorrent | 0.73 | 10.74 |

The compact defaults reduced TorrentNG PSS by **18.3%** from its previous best
configuration. CPU was effectively unchanged, up about **1.1%**, so this is a
memory improvement rather than a CPU improvement. Against rTorrent, TorrentNG
still used about **2.60× the CPU** and **1.85× the PSS** in this comparison.
CPU parity remains unsolved.

The final report records three randomized 256 MiB rounds for each of four
variants, with all **12/12 transfers verified**, plus source/binary identities,
build metadata, compact run summaries, mapping totals, 769 engine tests, 32
daemon tests in each of three API configurations, and Clippy results. The
older 11/12 matrix that hit `ENOSPC` is retained as history and explicitly
excluded from the final comparison. The detailed
[native CPU/RAM report](TorrentNG-native-cpu-ram-20261022.md) links the
consolidated evidence. Raw process timestamps, machine paths, payloads, and
binaries are intentionally omitted; compact summaries and hashes are checked
in.

## What improved and what did not

- The dense verification change avoids allocating and copying a second buffer
  for one contiguous extent. In the earlier 256 MiB screen, verify-only
  reduced median PSS from 27.01 to 24.52 MiB and anonymous memory from 9.19 to
  6.75 MiB, while median CPU rose from 9.48 to 9.88 CPU-s/GiB.
- One Tokio worker plus dense verification measured 23.76 MiB median PSS and
  6.07 MiB anonymous memory in that screen. Its median CPU was 8.16 CPU-s/GiB.
  This was one host with three TorrentNG samples per variant and an incomplete
  rTorrent matrix.
- Size-optimized release settings and the leaner default API feature set
  reduce the default image footprint. Transmission and Deluge APIs remain
  available through `full-compat` or their individual Cargo features.
- The opt-s build's `.text` section was 30.8% smaller than opt-3. Its median
  executable-mapping PSS was 11.09 MiB versus 15.71 MiB for old-best, while
  anonymous PSS was similar (5.79 versus 5.91 MiB). The PSS win mostly came
  from mapped code pages, not smaller runtime heaps or payload buffers.
- The immediate rate-grant path avoids persistent waiter bookkeeping for
  uncontended grants while retaining the existing cancellation, fairness,
  settlement, and refund paths.
- The final defaults produced no established CPU improvement over the previous
  best configuration, and remain behind rTorrent on both reported metrics.

## Next economical CPU and memory work

The 2026-10-22 seed-read microprobe found that a single hot mapped range used
56.3% less CPU than a reused-buffer `pread` loop, but saved only 0.0599
CPU-s/GiB—about 8% of the 0.78 CPU-s/GiB gap in the separate whole-client
screen. A mapped read alone is too small to close the gap. A separate
`sendfile` candidate cut anonymous memory by 74% but raised CPU from 1.33 to
5.51 CPU-s/GiB, so that implementation was reverted.

The next useful low-cost step is a transfer-window profile of the current
one-worker build on the local private swarm. The existing per-thread sample
puts most CPU on the Tokio worker, but does not identify which functions
consume it; the earlier whole-process profile also included startup and
verification. Use that profile to choose one narrow candidate—most plausibly a
verified mapped-range lease combined with reuse of queued upload work—then
screen it with a small set of randomized, SHA-256-verified local transfers.
Keep the buffered path as fallback and record CPU, PSS, anonymous memory, file
cache, and goodput together. Do not scale the run matrix unless the first
matched screen improves CPU without giving back the memory gain.

The [seed-read probe report](TorrentNG-seed-read-path-probe-20261022.md) and
the [detailed benchmark report](TorrentNG-native-cpu-ram-20261022.md) preserve
the current evidence, raw-summary links, and rejected experiments.
