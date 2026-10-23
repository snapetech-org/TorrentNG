# Active peer-read stream on current main — 2026-10-23

**Status: not promoted.** Re-testing the active-stream-outside-`HashMap` refactor on current main produced order-sensitive CPU results and higher memory in both screens. The eight-block source remains unchanged.

The candidate was based on source revision `4b187a8d` plus [`active-stream-current-main.patch`](active-stream-current-main.patch). Rust 1.94 formatting, two focused storage tests, and the release build passed; exact commands and binary identity are recorded alongside this note. Both one-peer screens used a local 64 MiB fixture at 4 MiB/s, `pread`, and rTorrent 0.16.25. Every transfer verified SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

| Screen | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| 1, candidate first | Control | 3.810 | 18.92 MiB | 6.02 MiB |
| 1 | Active-stream candidate | 4.616 (+21.2%) | 19.11 MiB (+0.19 MiB) | 6.51 MiB (+8.1%) |
| 1 | rTorrent 0.16.25 | 2.091 | 7.63 MiB | 3.08 MiB |
| 2, control first | Control | 4.106 | 18.64 MiB | 6.01 MiB |
| 2 | Active-stream candidate | 3.405 (-17.1%) | 19.04 MiB (+0.40 MiB) | 6.53 MiB (+8.7%) |
| 2 | rTorrent 0.16.25 | 1.952 | 7.70 MiB | 3.08 MiB |

Across the two rounds, the candidate median CPU was 1.3% higher, PSS 1.6% higher, and anonymous peak 8.4% higher than control. The candidate lost CPU when it ran first during elevated host load, then won when it ran second after control. Its memory use was higher in both rounds. The mixed CPU signal and repeatable memory regression do not justify promotion or further rounds on this patch.

The older [`stream-cache-active.patch`](../../experiments/stream-cache-active.patch) remains archived with its separate profile-dependent results: it won CPU at 64 MiB under an older profile, then lost at 256 MiB and in a refreshed PGO comparison. This current-source follow-up confirms it is not a reliable joint CPU/RAM improvement.

Candidate binary SHA-256: `47ec146c033a175adb87cbc40a814f95e44e6cea4729750451b9a31661779a7f`. The compressed binary, two summaries, fixture metainfo, logs, configs, diagnostics, load snapshots, and source/build notes are retained here. Kernel `smaps` captures are gzip-compressed byte-for-byte. Payload copies, temporary API tokens, session databases, and bootstrap passwords are excluded. No production source changed.
