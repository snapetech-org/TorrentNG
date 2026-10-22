# Batched upload rate admission screen

## Result

The candidate tried to reserve the existing per-block upload-rate grants while
holding the rate-tree lock once per contiguous read batch. It kept one grant
per Piece response so cancellation, fairness, and settlement stayed attached
to the same payload boundaries.

Two one-round screens did not show a reliable improvement over the current
16 ms release. The candidate's median CPU was **8.3% higher**, peak PSS was
**0.8% higher**, and peak anonymous memory was **2.9% higher**. Its CPU result
varied sharply between the two rounds, and its PSS was higher in both. The
candidate was not promoted; the exact source patch, executable, summaries, and
run diagnostics are retained here for another look if rate-tree profiling
later shows this lock is a real hot spot.

| Screen | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| 1 | Current 16 ms release | 3.483 | 19.355 MiB | 6.520 MiB |
| 1 | Batched rate candidate | 4.261 | 19.582 MiB | 6.945 MiB |
| 1 | rTorrent 0.16.25 | 1.880 | 7.743 MiB | 3.043 MiB |
| 2 | Current 16 ms release | 4.093 | 18.586 MiB | 6.078 MiB |
| 2 | Batched rate candidate | 3.942 | 18.652 MiB | 6.020 MiB |
| 2 | rTorrent 0.16.25 | 2.044 | 7.768 MiB | 3.047 MiB |
| Median of 2 screens | Current 16 ms release | **3.788** | **18.971 MiB** | **6.299 MiB** |
| Median of 2 screens | Batched rate candidate | 4.102 (+8.3%) | 19.117 MiB (+0.8%) | 6.482 MiB (+2.9%) |
| Median of 2 screens | rTorrent 0.16.25 | 1.962 | 7.755 MiB | 3.045 MiB |

These two single-round screens are a cheap rejection check, not a stable
performance estimate. In round 1 the candidate used 22.4% more CPU than the
current build. In round 2 it used 3.7% less CPU, while PSS remained 0.35%
higher. The candidate never improved both CPU and PSS in the same round.

## Method and artifacts

- Host: `kspld0`, AMD Ryzen 9 9950X3D.
- Workload: deterministic 64 MiB private loopback seed; 16 KiB blocks; 32
  outstanding requests; 4 MiB/s offered rate; DHT, PEX, and tracker disabled.
- The storage backend was pinned to `pread` for both TorrentNG builds.
- Both screens used the same fixture SHA-256:
  `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.
- All six transfers passed SHA-256 validation and used the same 16-second
  payload window. Round order was changed between screens.
- Control binary SHA-256: `44cac06c59daa4d1a48a053448679ab3c26ed8edc1cdcf0b1ccca0d331df432a`.
- Candidate binary SHA-256: `7e33315b39427171cbd4dc580b96a59fc76f6740be87f1ca99f4fba6646b3b50`.
- rTorrent binary SHA-256:
  `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`.
- The candidate was built from `main` at `19971f5a` plus the retained patch,
  with Rust 1.94.0 and the normal size-optimized release profile.

`screen-1/` and `screen-2/` contain source and normalized probe summaries.
`runs/` keeps client logs, sanitized settings, selected-backend metrics, and
process maps. Generated payloads, API/bootstrap tokens, and temporary session
databases are excluded. The exact candidate executable is gzip-compressed in
`executables/`; its uncompressed hash is in `SHA256SUMS.uncompressed`. The
source patch, including its focused rate-grant tests, is in
`experiments/upload-rate-batch.patch`.

The candidate passed `cargo +1.94.0 fmt --all -- --check` and all 771
`rt-engine` tests. The two new tests cover independent grant settlement and
refunds plus rate-limited cancellation after partial admission. The release
source was restored to the tested 16 ms default after the screen.
