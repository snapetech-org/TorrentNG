# Current-thread runtime screen — 2026-10-22

**Status: not promoted; one screening round lost on CPU and PSS.** The candidate uses Tokio's current-thread runtime when configured for one worker and keeps the multithread runtime for explicit worker counts above one. Two daemon tests verified both runtime flavors; the override behavior remains intact.

Validation passed on Rust 1.94: the two runtime flavor tests, release build, and `cargo fmt --all -- --check`. The candidate binary SHA-256 is `367dc0257954d2eff4ffb00006bbc06063adc25d0c27ac1edf927d7945ec2602`. The exact patch is [`current-thread-runtime.patch`](../../experiments/current-thread-runtime.patch).

One randomized 64 MiB screen compared the candidate, current one-worker multithread control, and rTorrent 0.16.25. Every transfer verified the same payload SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Current-thread candidate | 4.714 | 18.82 MiB | 6.05 MiB |
| One-worker multithread control | 4.362 | 18.50 MiB | 6.02 MiB |
| rTorrent 0.16.25 | 2.602 | 7.62 MiB | 3.08 MiB |

The candidate used 8.1% more CPU, 0.32 MiB more PSS, and 0.03 MiB more anonymous memory than control. It ran first; host load averaged `17.75/15.67/16.47` before and `15.84/15.56/16.36` after the run on 32 CPUs. This is an order-confounded single screen, but it does not support promotion or expansion now. The current one-worker multithread runtime remains the default. The patch, binary, fixture metainfo, complete summary, logs, load snapshots, and binary identities are preserved here.

No production source changed.
