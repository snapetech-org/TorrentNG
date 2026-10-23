# Lazy second-worker screen — 2026-10-23

**Status: not promoted.** Two lazy-worker triggers failed to start the second storage worker during the measured three-peer upload. The refined candidate was effectively tied on CPU and slightly worse on PSS. The production default and current best configuration remain unchanged.

The earlier static one-worker setting helped a single peer but used 12–19% more CPU than the two-worker setting in every randomized three-peer round. This follow-up tested whether starting worker two only when work overlaps could preserve the single-peer benefit without that multi-peer cost.

Both candidates were based on `dc298249eb74ab7066d28d7bb667568d553ccf3e`, built with Rust 1.94.0, and used the `pread` backend. Each three-peer screen served the same verified 64 MiB fixture to three local peers at 4 MiB/s each. The rTorrent comparison used the pinned 0.16.25 build. Every transfer passed SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

## Results

| Candidate | Build | CPU-s/GiB | Peak PSS | Peak anonymous | Worker 2 seen |
|---|---|---:|---:|---:|---|
| Queue-only trigger | Control | 4.245 | 18.970 MiB | 6.199 MiB | — |
| Queue-only trigger | Candidate | 5.469 (+28.8%) | 19.312 MiB (+1.8%) | 6.656 MiB (+7.4%) | No, across 110 snapshots |
| Queue-only trigger | rTorrent 0.16.25 | 2.765 | 7.698 MiB | 3.090 MiB | — |
| Active-plus-queued trigger | Control | 3.122 | 18.617 MiB | 6.133 MiB | — |
| Active-plus-queued trigger | Candidate | 3.031 (-2.9%) | 18.802 MiB (+1.0%) | 6.145 MiB (+0.2%) | No, across 132 snapshots |
| Active-plus-queued trigger | rTorrent 0.16.25 | 2.266 | 7.842 MiB | 3.086 MiB | — |

The refined result is one round. A 2.9% CPU difference with slightly higher memory is not a repeatable improvement, and the trigger still did not activate under this workload. The queue-only result is a clear regression. Neither candidate closes the gap to rTorrent.

The queue-only candidate had one additional single-peer screen: 4.213 vs 4.561 CPU-s/GiB for the candidate and control, with PSS 18.470 vs 18.629 MiB. That directional single-round result did not compensate for the failed multi-peer screen. The earlier static one-worker three-round CPU regression remains the reason the production setting stays at two.

## Validation and retained artifacts

The refined candidate passed formatting and all **354 `rt-storage` library tests**. Its focused overlap test verified that an active first job plus a queued second job starts worker two. Release binaries were built for both candidates; their hashes and the per-round measurements are in [`summary.json`](summary.json). Thread snapshots are retained for the queue-only and refined candidates.

The source patches are preserved compressed as [`queue-only-lazy-worker.patch.gz`](queue-only-lazy-worker.patch.gz) and [`active-queue-lazy-worker.patch.gz`](active-queue-lazy-worker.patch.gz). They are review artifacts for future refinement, not changes to `main`. Decompress a patch and pipe it to `git apply --check -` before applying. Both patches apply cleanly to the recorded source revision; the refined patch includes the later change that limits overlap-counter work to the lazy I/O pool.

The next useful worker experiment would need to explain why the production workload had no overlapping storage operations visible to this trigger. Do not promote a static one-worker setting based on the single-peer result alone.
