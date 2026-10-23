# Remove redundant TCP Piece preflush — 2026-10-22

**Status: not promoted; inconclusive and inconsistent.** The candidate removed the explicit `Framed::flush()` before raw TCP Piece writes. Control messages use `SinkExt::send`, which flushes before returning; a new wire-order test verified control frames precede both single and batched Piece frames. The candidate passed all 770 `rt-engine` tests and `cargo fmt --all -- --check`.

The release daemon has SHA-256 `b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263`. The exact patch and compressed binary are retained beside the summaries.

## Results

All complete transfers used the same deterministic 64 MiB loopback fixture, 4 MiB/s offered rate, 16 KiB requests, and 32-request window. Every completed transfer passed SHA-256 verification.

| Screen / round | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| First, 1 | No explicit preflush | 3.499 | 18.59 MiB | 5.95 MiB |
| First, 1 | Eight-block control | 4.476 | 19.80 MiB | 7.02 MiB |
| First, 1 | rTorrent 0.16.25 | 2.183 | 7.77 MiB | 3.08 MiB |
| Confirm, 1 | No explicit preflush | 3.794 | 18.91 MiB | 6.01 MiB |
| Confirm, 1 | Eight-block control | 3.838 | 18.57 MiB | 6.00 MiB |
| Confirm, 1 | rTorrent 0.16.25 | 2.212 | 7.57 MiB | 3.08 MiB |
| Confirm, 2 | No explicit preflush | 3.623 | 19.16 MiB | 6.49 MiB |
| Confirm, 2 | Eight-block control | 3.399 | 18.66 MiB | 6.00 MiB |

The first screen appeared to cut CPU 21.8% and PSS 6.1%, but the candidate ran first and the control last while load rose. In the confirm, the first pair was nearly tied on CPU with slightly higher candidate PSS; the second pair was worse on both CPU and PSS. Host load measured 14.61/14.81/15.54 before the first screen, 22.67/16.86/16.18 before the confirm, and 18.91/17.32/16.43 afterward, on 32 CPUs. The remaining confirm rounds were stopped after the candidate failed to show a joint CPU/PSS gain.

Keep the 32 ms/eight-block main implementation unchanged. This flush removal remains archived for a quiet-host revisit, but these results do not support promoting it.
