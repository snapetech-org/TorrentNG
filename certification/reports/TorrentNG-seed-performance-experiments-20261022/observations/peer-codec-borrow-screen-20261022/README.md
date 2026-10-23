# PeerCodec ResourceGovernor borrow screen — 2026-10-22

**Status: not promoted; first screen did not improve CPU or PSS and was order-confounded.** This candidate replaces `ResourceGovernor` clones with borrows in `PeerCodec::reserve_capacity` and `reserve_write_capacity`. It changes no accounting rules. The exact patch, release binary, summaries, logs, and fixture metainfo are retained beside this report.

The candidate passed `cargo +1.94.0 test --locked -p rt-peer-wire` (59 passed, 1 ignored) and `cargo +1.94.0 fmt --all -- --check`. Its release binary SHA-256 is `ec5cd76de2fdfc92d4c09aef24beb0794a0494431f43bc8a3016ddd75436450f`. The patch is [`peer-codec-governor-borrow.patch`](../../experiments/peer-codec-governor-borrow.patch).

One randomized 64 MiB screen compared the current eight-block release binary, the candidate, and rTorrent 0.16.25. All three transfers verified the same payload SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Eight-block control | 3.560 | 18.52 MiB | 6.01 MiB |
| Codec borrow candidate | 4.175 | 18.63 MiB | 6.01 MiB |
| rTorrent 0.16.25 | 2.371 | 7.69 MiB | 3.08 MiB |

The candidate was 17.3% higher on CPU and 0.11 MiB higher on PSS; anonymous memory tied. The order was control, rTorrent, candidate. Host load increased from `14.43/17.11/17.12` to `23.98/18.94/17.72` on 32 CPUs, so this single screen is not a clean rejection. It is not promising enough to justify more runs under these conditions. Keep the candidate patch and binary for a quiet-host, reversed-order screen if a future profile still points to codec admission.

No production source changed. The measured 32 ms/eight-block upload-read configuration remains the current source best.
