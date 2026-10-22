# TorrentNG CPU and RAM comparison: final defaults

## Final reported comparison

The final comparison summary reported medians from three matched runs per
build:

| Build | CPU seconds per transfer ↓ | Process PSS (MiB) ↓ |
|---|---:|---:|
| Previous best TorrentNG configuration | 1.88 | 24.26 |
| **TorrentNG defaults at `0070ef7`** | **1.90** | **19.82** |
| rTorrent | 0.73 | 10.74 |

The compact defaults reduced TorrentNG PSS by **18.3%** from its previous best
configuration. CPU was effectively unchanged, up about **1.1%**, so this is a
memory improvement rather than a CPU improvement. Against rTorrent, TorrentNG
still used about **2.60× the CPU** and **1.85× the PSS** in this comparison.
CPU parity remains unsolved.

The final validation summary reported 769 engine tests, 32 daemon tests in
each of three API configurations, Clippy checks, and 12 benchmark transfers
passing. The associated final-results ZIP was not present in this checkout's
`~/Downloads`: the only TorrentNG ZIP there is a four-file source snapshot,
and its files match the current source byte for byte. The exact fixture,
per-run samples, and binary identities for the final three-run table therefore
are not independently archived here.

The detailed [native CPU/RAM report](TorrentNG-native-cpu-ram-20261022.md)
contains separate preliminary 128 MiB and Rust 1.97 256 MiB screens. The
256 MiB matrix is explicitly incomplete: 11 of 12 cells passed; its final
rTorrent cell hit `ENOSPC`, with a separate recovery sample reported apart.
Those raw screens are useful evidence, but they are not the missing raw bundle
for the final-default table above.

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
the current evidence and its limits.
