# Low-risk allocator and codegen screens — 2026-10-22

These one-round screens tested inexpensive build options against the current
16 ms upload-coalescing release binary and source-built rTorrent
0.16.25. Every transfer used the same deterministic 64 MiB local fixture,
16 KiB blocks, a 32-request window, 4 MiB/s pacing, `pread`, and disabled
DHT/PEX/trackers. The first five screens used three clients; a later codegen
screen added the retained PGO candidate. All nineteen payloads passed
SHA-256 verification against
`b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`.

Each screen is one randomized round. These are candidate screens, not medians
or new whole-client rankings.

| Screen | Current build CPU / PSS | Candidate CPU / PSS | rTorrent CPU / PSS |
|---|---:|---:|---:|
| Allocator: system → jemalloc | 3.925 CPU-s/GiB / 19.12 MiB | 3.956 CPU-s/GiB / 21.80 MiB | 2.199 CPU-s/GiB / 7.71 MiB |
| Jemalloc: system → one arena, default decay | 4.812 CPU-s/GiB / 19.17 MiB | 4.831 CPU-s/GiB / 21.48 MiB | 2.413 CPU-s/GiB / 7.55 MiB |
| Jemalloc: system → one arena, immediate decay | 4.632 CPU-s/GiB / 19.11 MiB | 4.918 CPU-s/GiB / 24.79 MiB | 2.625 CPU-s/GiB / 7.67 MiB |
| Codegen: portable → `target-cpu=native` | 4.722 CPU-s/GiB / 18.94 MiB | 5.158 CPU-s/GiB / 18.70 MiB | 1.960 CPU-s/GiB / 7.66 MiB |
| Compiler: Rust 1.94 → 1.97 | 5.252 CPU-s/GiB / 18.56 MiB | 5.314 CPU-s/GiB / 19.03 MiB | 2.137 CPU-s/GiB / 7.58 MiB |
| Codegen: compact `opt-level=s` → `opt-level=3` | 6.995 CPU-s/GiB / 18.70 MiB | 6.259 CPU-s/GiB / 21.48 MiB | 2.800 CPU-s/GiB / 7.61 MiB |

| Screen | Current anonymous peak | Candidate anonymous peak | Change |
|---|---:|---:|---:|
| System allocator → jemalloc | 6.39 MiB | 8.64 MiB | +35.2% |
| System allocator → jemalloc, one arena | 6.45 MiB | 8.56 MiB | +32.7% |
| System allocator → jemalloc, immediate decay | 6.51 MiB | 11.82 MiB | +81.7% |
| Portable → `target-cpu=native` | 6.38 MiB | 6.07 MiB | −4.8% |
| Rust 1.94 → 1.97 | 6.08 MiB | 6.02 MiB | −1.0% |
| Compact `opt-level=s` → `opt-level=3` | 6.14 MiB | 5.98 MiB | −2.6% |

No candidate lowered both CPU and PSS against its paired build. The jemalloc
candidate was slightly slower and had materially higher PSS and anonymous
memory. A one-arena setting still used 0.4% more CPU and 12.0% more PSS; adding
immediate dirty/muzzy decay raised CPU by 6.2% and PSS by 29.7%. Host-tuned
codegen reduced PSS by 0.24 MiB but used 9.2% more CPU. Rust 1.97 used 1.2%
more CPU and 2.5% more PSS. The tested 16 ms build remains on the system
allocator, portable target, and Rust 1.94. These options remain available for
later tests; the exact binaries and raw verified summaries are retained here.

The `opt-level=3` screen reduced CPU by 10.5% against the paired compact
release, but increased PSS by 14.9%. The PGO control in the same round was
better on both CPU (4.851 vs. 6.259 CPU-s/GiB) and PSS (16.25 vs. 21.48 MiB),
so the candidate does not improve the best measured build. It used Rust 1.94,
the system allocator, portable target, and the same thin-LTO release profile
with only `CARGO_PROFILE_RELEASE_OPT_LEVEL=3` changed. The raw one-round summary
and compressed executable are retained under `runs/` and `executables/`.

The codegen screen ran while `kspld0` was busy: its 32 logical CPUs had load
averages of 20.43 before and 29.48 after the screen, with 110 GiB of swap in
use in the after sample. Treat the comparison as exploratory, not a score or
promotion result. The exact binary SHA-256 is
`0d9d738c1993df2906cdc139e526f138b5389a0e1e43b0bf3cbbaa9153298efe`.

## Build and source identity

- Repository `HEAD` at build: `f1041100af68bddf2f2055bc2ca2f377d7b56c23`.
- Performance-source commit: `6cfc6a9aba79c954624d9d86f6508fb49ec0a894`; the
  intervening commits changed documentation and CI configuration.
- `Cargo.lock` SHA-256: `667fa1fe9e6d7076ecec380f59a58a45ce868ff9719d512eab37b43fca546d0d`.
- Release profile: workspace `opt-level = "s"`, thin LTO, one codegen unit,
  stripped debuginfo.
- Baseline binary SHA-256: `44cac06c59daa4d1a48a053448679ab3c26ed8edc1cdcf0b1ccca0d331df432a`.
- rTorrent binary SHA-256: `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`.
- Candidate binaries are gzip-compressed under `executables/`; their
  uncompressed hashes are in `SHA256SUMS.uncompressed`.

Build commands:

```sh
cargo +1.94.0 build --locked --release -p torrentngd --no-default-features --features jemalloc-allocator
RUSTFLAGS='-C target-cpu=native' cargo +1.94.0 build --locked --release -p torrentngd --no-default-features --features system-allocator
cargo +1.97.0 build --locked --release -p torrentngd --no-default-features --features system-allocator
CARGO_PROFILE_RELEASE_OPT_LEVEL=3 cargo +1.94.0 build --locked --release -p torrentngd --no-default-features --features system-allocator
```

An attempted PGO plus `target-cpu=native` refinement reused the current-source
portable transfer-window profile. Rust reported control-flow profile
mismatches in eight functions, including the peer-read cache consumer and
upload window controller, so that build was stopped without producing a
binary. A follow-up trained a matching native-target profile and built cleanly,
but its one-round whole-client result was worse than the retained PGO control
on CPU and PSS. The matching profile and result are preserved in
[`../pgo-native-target-screen-20261022/`](../pgo-native-target-screen-20261022/);
neither attempt changed release defaults.

## Diagnostic CPU profile

The exact 16 ms system-allocator baseline also completed a profiler-only,
SHA-256-verified 256 MiB seed transfer. The 1 kHz gperftools profile captured
779 ms of samples in 440 distinct stack traces. Its largest individual
TorrentNG function was the peer upload-loop closure at 37 ms, or 7.6% of
TorrentNG executable leaf samples. The profile did not identify one dominant
application function to target. Profiler-instrumented CPU samples are
diagnostic and are not a benchmark score. The raw profile and probe summary
are preserved in `profile/`.
