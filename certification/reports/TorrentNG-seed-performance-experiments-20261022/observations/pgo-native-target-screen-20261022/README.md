# Matched native-target PGO screen — 2026-10-22

This screen checks whether a transfer-window profile trained with
`target-cpu=native` can improve on the retained PGO build. A prior attempt
reused portable-target profile data and stopped on LLVM control-flow mismatch
warnings. This profile was generated and used with matching native-target
settings; the profile-use build completed without mismatch warnings.

## Result

One randomized round compared five clients on the same private 64 MiB seed
fixture. All transfers verified SHA-256
`b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901` and ran at
the same 4 MiB/s offered payload rate. The order was PGO control, native PGO,
release, rTorrent, portable PGO.

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Retained PGO control | 2.812 | 16.37 MiB | 6.33 MiB |
| Matched native-target PGO | 3.025 (+7.6%) | 18.22 MiB (+11.3%) | 6.00 MiB (-5.2%) |
| Fresh portable-target PGO | 2.795 (-0.6%) | 18.54 MiB (+13.3%) | 6.50 MiB (+2.7%) |
| Release | 3.675 | 18.79 MiB | 6.06 MiB |
| rTorrent 0.16.25 | 2.068 | 7.59 MiB | 3.05 MiB |

The native-target profile reduced anonymous memory slightly but raised both
CPU and PSS against the retained PGO control. It does not improve the combined
resource result and was not promoted. The control and release settings remain
unchanged. This is one small screening round, not a median or qualification.

The run began on `kspld0` with load averages `23.27, 20.21, 18.89` and about
117 GiB of swap in use. Treat the result as exploratory. The screen was fully
local: no tracker, DHT, or PEX; 16 KiB requests; a 32-request leecher window;
and `TNG_STORAGE_BACKEND=pread` for all TorrentNG variants. rTorrent used the
source-built 0.16.25 comparator and its pinned runtime libraries.

## Build and fixture identity

- Repository `HEAD` at build: `7b5e1cc8b63137766be4c399741e5d99c417f8c9`.
- Cargo source and lockfile were unchanged from `6cfc6a9aba79c954624d9d86f6508fb49ec0a894`.
- `Cargo.lock` SHA-256: `667fa1fe9e6d7076ecec380f59a58a45ce868ff9719d512eab37b43fca546d0d`.
- Rust: `1.97.0`; LLVM profile tools: `22.1.6`.
- Release profile: system allocator, `opt-level=s`, thin LTO, one codegen unit.
- PGO data SHA-256: `8dc7f3d6da282a854dca4f1746526736c4b13cc2e95b6950d09c19c96e0c949c`.
- Native PGO daemon SHA-256: `3f2e9626c83292c028fcb6c23637fc1cf490380330c326219cbc1b3276000fc9`.
- Instrumented daemon SHA-256: `6e35cb9d576413e989fd3711f26a134d27faaaab46cb45185108a7aaf763bc28`.
- Existing PGO control SHA-256: `8e8522729bdc06b8590995945e87544741370cb66c70eb13576d54f8e378f006`.
- Portable-target PGO SHA-256: `ce7e769b49af69509f71c031efa22b2af1c1a0e31fc453e848f59ea809513039`.
- Release daemon SHA-256: `44cac06c59daa4d1a48a053448679ab3c26ed8edc1cdcf0b1ccca0d331df432a`.
- rTorrent 0.16.25 SHA-256: `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`.
- Fixture: 67,108,864 bytes, seed `20261022`, SHA-256 shown above.

The profile was collected from the first upload batch through daemon shutdown.
Both the instrumented and optimized builds used `-Ctarget-cpu=native`; the
profile-use build emitted no mismatch warning. The payload CPU value is the
daemon's user plus system CPU during the verified payload window, divided by
the transferred size in GiB. PSS and anonymous memory are active-window peaks.

## Reproduction commands

The instrumented and profile-use builds used the repository's locked release
configuration and system allocator:

```sh
CARGO_BUILD_JOBS=1 \
CARGO_TARGET_DIR=/tmp/torrentng-seed-goal-20261022/pgo-native-matched-20261022/target \
RUSTFLAGS='-Cprofile-generate=/tmp/torrentng-seed-goal-20261022/pgo-native-matched-20261022/profile-raw -Ctarget-cpu=native' \
cargo +1.97.0 build --locked --release -p torrentngd --no-default-features --features system-allocator
```

After the verified transfer-window training run, LLVM 22 merged the raw
profile. The profile-use build then used that merged profile and the same
native target:

```sh
llvm-profdata merge -o native-transfer.profdata transfer-window.profraw
CARGO_BUILD_JOBS=1 \
CARGO_TARGET_DIR=/tmp/torrentng-seed-goal-20261022/pgo-native-matched-20261022/use-target \
RUSTFLAGS='-Cprofile-use=/tmp/torrentng-seed-goal-20261022/pgo-native-matched-20261022/native-transfer.profdata -Ctarget-cpu=native' \
cargo +1.97.0 build --locked --release -p torrentngd --no-default-features --features system-allocator
```

## Preserved evidence

- `runs/screen-64m/summary.json` contains the exact client order, binary hashes,
  transfer hashes, process CPU, PSS, anonymous-memory samples, and environment.
- `runs/training-64m/summary.json` records the profile training run.
- `executables/` contains the optimized and instrumented daemons, gzip-compressed.
- `profiles/` contains both the raw and merged LLVM profiles, gzip-compressed.
- `runs/` contains client logs, metrics snapshots, and compressed process maps.
- `SHA256SUMS.uncompressed` records the original hashes of the binaries and
  profile files. The parent observation manifest records every archived file.

Generated payloads, session databases, API tokens, and bootstrap passwords
are excluded. The broader results and next source-level candidate are in the
[seed performance experiment report](../../README.md).
