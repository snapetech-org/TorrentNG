# Current-source storage worker count screen — 2026-10-23

A local single-peer screen on the current batch-8 daemon compared the configured default of two storage I/O workers with one worker. The same release binary and `pread` backend were used in three alternating-order 64 MiB pairs; all six transfers verified the same fixture.

| Setting | Median CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| 2 workers | 4.188 | 19.031 MiB | 6.504 MiB |
| 1 worker | 3.918 (-4.0% paired median) | 17.669 MiB (-7.2%) | 5.188 MiB (-19.4%) |

The one-worker setting won CPU in two of three pairs and reduced PSS and anonymous memory in all three. This is a promising single-peer option. It is **not promoted as the default**: the prior three-peer screen raised CPU by 12–19% in every round at one worker. The configuration does not yet adapt to peer concurrency.

This is a local screen, not the rTorrent comparison. Exact paired results and identities are in [`summary.json`](summary.json). The probe CLI gained the worker-count setting in commit `1360dc24`; its 24 focused tests passed.
