# Biased peer-loop select screen — 2026-10-23

**Status: not promoted.** Replacing Tokio's randomized poll start with `biased;` produced an encouraging first pair, but the result did not survive order-balanced confirmation. The final five-pair screen used 13.7% more median CPU, with CPU lower in only one pair. Peak PSS and anonymous memory were slightly lower, but the CPU regression makes this a tradeoff.

The candidate was built from source revision `672e5233fb773b72897511f8d914b9d126dbfe59` with Rust 1.94.0. It adds only `biased;` to the peer-loop `tokio::select!` and retains the existing branch order. All arms used a 64 MiB local fixture, `pread`, a 4 MiB/s receiver, a 32-request window, and two seconds of warmup and cooldown. Every transfer verified fixture SHA-256 `6a7477ab1cb9ad9fa86da28fbc3a2b1d1f11aa299919f2c643011e9668459cd8`.

## Screen medians

| Screen | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| First randomized pair | Control | 3.996 | 18.591 MiB | 5.988 MiB |
| First randomized pair | Biased select | 3.504 (-12.3%) | 18.638 MiB (+0.3%) | 6.016 MiB (+0.5%) |
| Three-way, 3 rounds | Control | 3.834 | 19.278 MiB | 6.492 MiB |
| Three-way, 3 rounds | Biased select | 3.639 (-5.1%) | 19.071 MiB (-1.1%) | 6.469 MiB (-0.4%) |
| Order-balanced, 3 pairs | Control | 3.459 | 18.560 MiB | 6.008 MiB |
| Order-balanced, 3 pairs | Biased select | 3.526 (+1.9%) | 18.591 MiB (+0.2%) | 6.035 MiB (+0.5%) |
| Alternating, 5 pairs | Control | 3.438 | 19.052 MiB | 6.453 MiB |
| Alternating, 5 pairs | Biased select | 3.908 (+13.7%) | 18.880 MiB (-0.9%) | 6.242 MiB (-3.3%) |
| rTorrent 0.16.25, first screen | rTorrent | 2.130 | 7.041 MiB | 3.090 MiB |
| rTorrent 0.16.25, 3-way screen | rTorrent | 2.310 | 7.582 MiB | 3.090 MiB |

The first and three-way medians looked promising, but CPU changed direction when order was balanced. In the final alternating confirmation the candidate lost CPU in four of five pairs. Treat the early improvements as run-to-run variation, not a source gain. The per-run metrics, randomized orders, and binary hashes are in [`select-biased-screens.json`](select-biased-screens.json); the one-line patch is retained as [`peer-select-biased.patch.gz`](peer-select-biased.patch.gz).

The candidate passed formatting and all ten `rt-engine` tests matching `peer_loop`, including the rate-waiting input test. This was a CPU/memory screen, not a dedicated sustained-message fairness stress test. No production code or default changed.
