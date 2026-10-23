# Direct Request-frame decode screen — 2026-10-23

**Decision: not promoted.** The patch passed focused tests and made the isolated decoder much faster, but a matched three-round whole-client screen did not show a CPU or PSS win. Main source remains unchanged.

## Candidate

For a complete 17-byte BEP 3 Request frame (`length=13`, message ID 6), decode the twelve request-body bytes directly from the receive buffer, then advance past the frame. This skips the temporary `BytesMut::split_to(...).freeze()` ownership path. Request length validation and malformed-frame consumption are preserved. All other messages use the current decoder.

The archived patch is [`candidate.patch.gz`](candidate.patch.gz); its compressed SHA-256 is `7742a92acf845b1e78622e8a568e507dfa92745b8106d61e654cc060b5aa01b4`, and its uncompressed SHA-256 is `e526b3e7bab5bb59e8094219a5f71ccc7f8d7504b86b36d58ee14e41cbc7cfc6`.

## Isolated decoder result

A release-mode harness decoded eight million prebuilt Request frames per run. Input construction was outside the timer. All control and candidate checksums matched.

| Decoder mode | Control median CPU | Candidate median CPU | Paired change |
|---|---:|---:|---:|
| `PeerCodec::with_resources` | 0.347 s | 0.217 s | **-37.7%** |
| `PeerCodec::default` | 0.262 s | 0.141 s | **-46.2%** |

Raw timings and the harness are retained alongside this report. The decoder microbenchmark does not model networking, scheduling, disk reads, or the number of requests in the whole-client run.

## Whole-client screen

The current source and candidate were built with Rust 1.94.0 and the same release profile. A local 16 MiB fixture was transferred at 4 MiB/s using 16 KiB blocks and a 32-request window. Three randomized paired rounds alternated which build ran first. All six transfers passed SHA-256 `8720b45a392e63940e529552c5cb8be01e422e0fc58b538302c952f19c37f012`.

| Build | Median CPU-s/GiB | Median peak PSS | Median peak anonymous |
|---|---:|---:|---:|
| Current source | 3.461 | 18.060 MiB | 5.516 MiB |
| Request-decode candidate | 3.591 | 18.087 MiB | 5.402 MiB |

The candidate median used **3.7% more CPU**, PSS was effectively tied (+0.03 MiB), and anonymous memory was 2.1% lower. Paired CPU changes were +3.3%, -5.5%, and +17.5%; this is inconclusive as a regression, and it establishes no improvement. A one-pair screen also ran under rising host load and is not scored.

The 16 MiB screen used only 1,024 Request frames. The microbenchmark's per-frame CPU saving implies about 17 microseconds over that transfer (an estimate, not a measured whole-client saving). This explains why a large decoder-only percentage did not create a visible end-to-end win here. Do not spend a full-size comparison on this patch unless a higher-request workload or profile gives it a larger share.

## rTorrent launch attempt

The pinned rTorrent 0.16.25 binary matched SHA-256 `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`, but exited with signal 11 before SCGI readiness when launched directly on the host. No rTorrent transfer was scored. The failed launch is recorded in [`rtorrent-launch-failure.txt`](rtorrent-launch-failure.txt); it is not a benchmark result.

## Validation and identities

- `cargo +1.94.0 fmt --all -- --check`: passed.
- `cargo +1.94.0 test --locked -p rt-peer-wire`: 61 passed, 0 failed, 1 ignored.
- Candidate release daemon SHA-256: `581f20d3a198ebc3f62e7e173a08b39aabdd8f30ca9b26e13b85a820791b2cb0`.
- Control release daemon SHA-256: `b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263`.
- Both source builds use base commit `c10e914db6ee6b707cda1cf9eac91243927a2de8`; the candidate applies only the archived peer-wire patch.

[`summary.json`](summary.json) contains the sanitized per-run CPU, PSS, anonymous-memory, order, fixture, and binary identities. `microbench-resources.jsonl` and `microbench-default.jsonl` preserve the raw decoder timings.
