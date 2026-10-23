# Full-process and transfer-window PGO blend — 2026-10-22

This experiment tested whether combining two current-source LLVM profiles
could keep the transfer-window profile's CPU benefit while recovering the
full-process profile's lower memory use. Both profiles were collected with
Rust 1.97.0 / LLVM 22.1.6, from the same source and portable target. The
profile-use build completed without mismatch warnings.

## Screening results

A one-round five-client comparison initially looked promising. The blend
measured 3.106 CPU-s/GiB and 17.29 MiB PSS, versus 3.587 and 16.69 MiB for the
existing PGO control. It ran last in that round, so it was not treated as a
win. I expanded the comparison to three randomized 64 MiB rounds with the
control, transfer-window PGO, blend, and rTorrent.

| Build | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---:|---:|---:|
| Existing PGO control | 3.049 | 16.08 MiB | 6.34 MiB |
| Equal-weight profile blend | 3.229 (+5.9%) | 17.58 MiB (+9.3%) | 6.01 MiB (-5.2%) |
| Transfer-window PGO | 2.972 (-2.6%) | 18.52 MiB (+15.2%) | 6.31 MiB (-0.5%) |
| rTorrent 0.16.25 | 1.975 | 7.62 MiB | 3.05 MiB |

The blend's paired CPU changes against the PGO control were `+15.5%`,
`+13.6%`, and `-1.5%`; PSS was higher in all three rounds (`+7.9%`, `+8.0%`,
and `+13.5%`). The lower anonymous-memory median did not compensate for the
CPU and PSS regressions. The profile blend was not promoted.

All 17 transfers in the initial and repeated comparisons verified the same
fixture SHA-256
`b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`. The
fixture was 64 MiB, offered at 4 MiB/s, with 16 KiB requests and a 32-request
window. TorrentNG used `pread`; tracker, DHT, and PEX were disabled.

The repeated run orders were:

| Round | Order |
|---|---|
| 1 | rTorrent, PGO control, blend, transfer-window PGO |
| 2 | transfer-window PGO, PGO control, rTorrent, blend |
| 3 | rTorrent, PGO control, blend, transfer-window PGO |

The desktop remained busy during this screening work. A sample during the
repeated screen showed load averages `19.12, 22.59, 21.50` with about 119 GiB
of swap in use. These are screening medians, not a new final ranking.

## Profile and binary identity

- Build `HEAD`: `505f6c0cba053ac443ac1c6770cbeb85b661dfcc` (documentation only;
  Rust sources and `Cargo.lock` were unchanged from source commit
  `6cfc6a9aba79c954624d9d86f6508fb49ec0a894`).
- `Cargo.lock` SHA-256: `667fa1fe9e6d7076ecec380f59a58a45ce868ff9719d512eab37b43fca546d0d`.
- Rust `1.97.0`; LLVM profile tools `22.1.6`.
- Release profile: system allocator, `opt-level=s`, thin LTO, one codegen unit.
- Full-process profile SHA-256: `47d37cf2ba582152f86fc8e3cd9d34998f0f8485fc4433082b10c47fd66de581`.
- Transfer-window profile SHA-256: `80fda6a6a1da618cfc7e6bf364ea90c6e27d87cc7278bcc6c1fbfea151580b82`.
- Equal-weight merged profile SHA-256: `f67738faa1d2f73f71eafa664d8f70af423711d0e4d135d3e74b82ffde117dd3`.
- Blended daemon SHA-256: `7e9e56d061c8d4fb9cf19ba3359056ed579945546242ab80070e58e4a6f922ff`.
- Existing PGO control SHA-256: `8e8522729bdc06b8590995945e87544741370cb66c70eb13576d54f8e378f006`.
- Transfer-window PGO daemon SHA-256: `ce7e769b49af69509f71c031efa22b2af1c1a0e31fc453e848f59ea809513039`.
- rTorrent 0.16.25 SHA-256: `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`.

The two input profiles and their raw training runs are preserved in the sibling
[`pgo-current-source-16ms-20261022/`](../pgo-current-source-16ms-20261022/)
directory. The merged profile and optimized binary are gzip-compressed here.

## Reproduction

The LLVM profile merge used equal weights:

```sh
llvm-profdata merge --num-threads=2 \
  --weighted-input=1,full-process.profdata \
  --weighted-input=1,transfer-window.profdata \
  -o full-plus-transfer.profdata
```

The candidate build used the merged profile with the same portable target as
its training builds:

```sh
CARGO_BUILD_JOBS=1 \
CARGO_TARGET_DIR=/tmp/torrentng-seed-goal-20261022/pgo-blended-current-source-20261022/use-target \
RUSTFLAGS='-Cprofile-use=/tmp/torrentng-seed-goal-20261022/pgo-blended-current-source-20261022/profiles/full-plus-transfer.profdata' \
cargo +1.97.0 build --locked --release -p torrentngd --no-default-features --features system-allocator
```

## Retained evidence

- `runs/one-round/summary.json` records the initial five-client result.
- `runs/three-rounds/summary.json` records the repeated randomized comparison.
- Both run directories contain fixture metadata, client logs, metrics, and
  compressed process `smaps`; generated payloads and session credentials are
  excluded.
- `executables/` and `profiles/` preserve the exact tested daemon and merged
  profile. `SHA256SUMS.uncompressed` records their original hashes.
- The parent [`manifest.json`](../manifest.json) records every archived file.

The broader benchmark status and preserved tradeoffs are in the
[seed performance experiment report](../../README.md).
