# PeerCodec read-capacity fast path screen — 2026-10-23

**Decision: not promoted.** The change stayed in an isolated worktree. The main source was not changed.

The candidate skipped the completed-frame read-capacity reservation when the existing peer-buffer lease already covered the decoder buffer. It retained admission for a first prefilled complete frame and for any later buffer growth. This targeted the repeated no-op part of `PeerCodec::reserve_capacity`; it did not alter lease ownership or accounting.

## Validation

- Rust 1.94 peer-wire tests: 60 passed, 0 failed, 1 ignored.
- `cargo +1.94.0 fmt --all -- --check`: passed.
- Release build: passed.
- All 12 whole-client transfers completed the same 64 MiB transfer and matched fixture SHA-256 `6a7477ab1cb9ad9fa86da28fbc3a2b1d1f11aa299919f2c643011e9668459cd8`.

The exact source change is in [`candidate.patch.gz`](candidate.patch.gz); apply it with `gzip -cd candidate.patch.gz | git apply`. The compressed release executable and its checksum are also retained.

## Three-round screen

The randomized screen used the current release binary, the candidate, and rTorrent 0.16.25. It used `pread`, loopback TCP, 16 KiB blocks, a 32-request window, and a paced 4 MiB/s receiver. Each run took about 16 seconds of payload time.

| Build | Median CPU-s/GiB | Median peak PSS | Median peak anonymous |
|---|---:|---:|---:|
| Current release control | 6.183 | 18.66 MiB | 5.98 MiB |
| Capacity-guard candidate | 8.937 | 18.80 MiB | 6.29 MiB |
| rTorrent 0.16.25 | 3.264 | 7.50 MiB | 3.10 MiB |

Against control, the candidate median used **44.5% more CPU**, **0.8% more PSS**, and **5.2% more anonymous memory**. It used more CPU in two of three paired rounds: -11.6%, +81.9%, and +68.0%. The candidate did not show a dependable gain.

An initial single round appeared to show 41.6% less CPU for the candidate. That result did not repeat: the control used 10.112 CPU-s/GiB in round 1 of the repeated screen, while its rounds ranged from 5.288 to 10.112. The first screen likewise showed large client differences. This is too variable to call a true regression or a win.

The host was heavily loaded throughout. The three-round run was pinned to logical CPU 8 while its sibling CPU 24 was 96% busy in a one-second sample; host load average was 27.05 before and 24.64 after. The initial round was pinned to CPU 4 at load averages 29.82 before and 32.74 after. Treat all CPU figures here as noisy screening evidence. PSS was nearly tied in the repeated screen, with the candidate slightly higher.

## Reproduction identities

- Source parent: `6683d993`.
- Control binary SHA-256: `844479163668b69bfe378838540dd9973fb60b3a781c237a2eefbd0d33d2fd07`.
- Candidate binary SHA-256: recorded in [`candidate-binary-sha256.txt`](candidate-binary-sha256.txt).
- rTorrent 0.16.25 binary SHA-256: `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`.
- Per-run sanitized values and order are in [`summary.json`](summary.json).

This probe does not replace the current eight-block source or the measured full-profile PGO candidate. Revisit only if a cleaner profile or workload shows this admission check is a meaningful hot path.
