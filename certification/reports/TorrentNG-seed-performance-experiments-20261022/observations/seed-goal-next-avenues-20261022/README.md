# Seed CPU and memory follow-up candidates — 2026-10-22

This folder preserves the next round of local seed-path experiments. The eight-block change is retained on main as commit 39554462. The other changes remain reviewable candidates with their raw measurements and patches.

## Method

The scored comparisons used a deterministic local loopback seed and leecher, a 4 MiB/s offered rate, 16 KiB blocks, a 32-request window, and no tracker, DHT, or peer exchange. Storage was pinned to pread. CPU is process CPU time during the payload window, normalized to CPU-s/GiB; PSS and anonymous memory are active-window peaks. All completed scored transfers passed SHA-256 verification.

The rTorrent comparator was built from upstream rTorrent 0.16.25 and had SHA-256 698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46. These local screens use a different fixture and measurement window from the published whole-client comparison, so they are optimization evidence rather than a replacement certification.

## Retained improvement: eight-block upload reads

The release-source default now collects for at most 32 ms and batches up to eight contiguous 16 KiB requests into one 128 KiB storage read. Global admission remains bounded at 64 jobs and 16 MiB, and the per-connection in-flight limit remains eight.

The 16 MiB diagnostic trace confirms the change forms larger batches under normal paced local traffic:

| Build | Storage reads | Batch-size histogram |
|---|---:|---|
| Four-block, 16 ms control | 257 | 255 batches of 4, one of 3, one of 1 |
| Eight-block, 32 ms candidate | 129 | 127 batches of 8, one of 7, one of 1 |

That is half as many storage reads for the same 1,024 blocks. The two diagnostic client logs are preserved here; debug logging was enabled, so neither trace is used as a CPU score.

### Matched release and PGO screens

Medians from three randomized rounds:

| Fixture and build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| 64 MiB, release control | 4.504 | 18.875 MiB | 6.500 MiB |
| 64 MiB, eight-block release | **3.506** | **18.586 MiB** | **6.012 MiB** |
| 64 MiB, rTorrent 0.16.25 | 2.211 | 7.676 MiB | 3.074 MiB |
| 64 MiB, current PGO control | 3.481 | 16.037 MiB | 6.020 MiB |
| 64 MiB, eight-block PGO | **3.016** | **15.791 MiB** | **5.953 MiB** |
| 64 MiB, rTorrent 0.16.25 | 2.092 | 7.702 MiB | 3.070 MiB |
| 128 MiB, release control | 6.063 | 19.185 MiB | 6.164 MiB |
| 128 MiB, current PGO control | 4.019 | 16.184 MiB | 6.477 MiB |
| 128 MiB, eight-block PGO | **3.348** | **15.593 MiB** | **6.023 MiB** |
| 128 MiB, rTorrent 0.16.25 | 2.361 | 7.607 MiB | 3.070 MiB |

The eight-block release reduced the 64 MiB control medians by 22% CPU, 1.5% PSS, and 7.5% anonymous memory. The eight-block PGO build reduced the matched PGO control by 13% CPU and 1.5% PSS at 64 MiB, and by 17% CPU, 3.6% PSS, and 7.0% anonymous memory at 128 MiB.

At 128 MiB, the eight-block PGO candidate still used about 1.42 times rTorrent's CPU and 2.05 times its PSS. The goal remains unmet. The normal release build was also screened once at 128 MiB: it used 4.926 CPU-s/GiB and 18.94 MiB PSS versus 5.651 and 19.36 MiB for the release control. That single round is screening evidence; the three-round 64 MiB release screen and the three-round PGO screens carry the stronger result.

The release-source change was tested with all 769 rt-engine tests and cargo fmt check. The commit hook scanned both changed Rust files with Semgrep and found no findings.

## Profile-guided build

A fresh profile was trained on one verified 64 MiB local transfer using the eight-block source and Rust 1.97.0 / LLVM 22.1.6. Only the daemon's transfer profile was merged; instrumented Cargo build-script profiles were excluded. The compressed profile is batch8-pgo-full.profdata.gz, with uncompressed SHA-256 0805e91d8bee6612dae8b717ba17fc8f2ffe8058190382ad97c62ecf2a138520. The measured PGO candidate binary had SHA-256 fe1dc0a1b5b3f1605045f4e839cf574041ff6535d7aa309f0b49a9487cde026f and is preserved as batch8-pgo-candidate.torrentngd.gz.

The candidate can be rebuilt with the repository's build_native_pgo_release.sh, passing this compressed profile and a task-scoped target directory. Rust PGO output hashes can vary between clean builds; the compressed measured binary is retained for exact re-screening. PGO remains opt-in.

## Earlier PGO arena-cap screen: memory gain with size-dependent CPU

MALLOC_ARENA_MAX=1 was screened on the eight-block PGO binary:

| Fixture | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| 64 MiB, 3 rounds | Eight-block PGO | 2.482 | 15.652 MiB | 5.957 MiB |
| 64 MiB, 3 rounds | Eight-block PGO with arena cap 1 | **2.429** | **14.489 MiB** | **4.719 MiB** |
| 128 MiB, 1 round | Eight-block PGO | 2.314 | 16.01 MiB | — |
| 128 MiB, 1 round | Eight-block PGO with arena cap 1 | 2.367 | 14.61 MiB | — |

The 64 MiB screen improved all three medians. The one-round 128 MiB result used 2.3% more CPU and 8.7% less PSS, so this remains an optional Linux/glibc runtime candidate rather than a new default. An interrupted three-round attempt is preserved separately; its unpaired second-round sample is excluded from these results.

## Arena cap on the current release candidate

A follow-up tested `MALLOC_ARENA_MAX=1` on the exact eight-block release binary (SHA-256 `e4e63a0446632660200781f9c0f355a47c80b123be5cf6001125782c79dad276`), against the same binary without the variable and source-built rTorrent 0.16.25 (SHA-256 `698a91e38ee6624aa5c3c6a63aa835c182e41956a2b4156f61da3511d3411f46`). Each transfer used a private loopback seed, 4 MiB/s offered rate, 16 KiB blocks, a 32-request window, `pread`, and disabled tracker/DHT/PEX. All scored transfers passed SHA-256 verification.

| Fixture and rounds | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| 64 MiB, 1 round | Release, default arenas | 3.479 | 18.777 MiB | 6.004 MiB |
| 64 MiB, 1 round | Release, arena cap 1 | 4.308 | 17.319 MiB | 4.762 MiB |
| 64 MiB, 1 round | rTorrent | 2.121 | 7.766 MiB | 3.070 MiB |
| 64 MiB, 3 rounds | Release, default arenas | 5.280 | 18.634 MiB | 6.004 MiB |
| 64 MiB, 3 rounds | Release, arena cap 1 | **4.275** (-19.0%) | **17.147 MiB** (-8.0%) | **4.637 MiB** (-22.8%) |
| 64 MiB, 3 rounds | rTorrent | 2.526 | 7.682 MiB | 3.074 MiB |
| 128 MiB, 3 rounds | Release, default arenas | 3.846 | 19.279 MiB | 6.555 MiB |
| 128 MiB, 3 rounds | Release, arena cap 1 | **3.589** (-6.7%) | **17.428 MiB** (-9.6%) | **4.770 MiB** (-27.2%) |
| 128 MiB, 3 rounds | rTorrent | 2.210 | 7.650 MiB | 3.074 MiB |

The first 64 MiB one-round result showed a CPU regression, but the three randomized rounds reversed it. In the repeated 64 MiB comparison the arena-capped arm used less CPU, PSS, and anonymous memory in every pair; the same held in all three 128 MiB pairs. The absolute 64 MiB CPU values varied across rounds, so treat the paired medians as screening evidence.

At 128 MiB, the capped release build still used about 1.62× rTorrent CPU and 2.28× its PSS. The cap is a promising Linux/glibc runtime option, but it is not enabled by default: concurrent-peer allocator contention and non-glibc deployments have not been qualified. The one-round result, three-round summaries, exact fixture torrents, per-run client logs, smaps, and metrics are preserved in this folder.

## Wider batches: failed and regressed candidates

A 16-block, 64 ms candidate first failed with BatchTooLarge because the scheduler's per-connection admission limit was still eight. That incomplete candidate transfer was stopped and preserved. Raising the experimental per-connection limit to 16 while keeping the global 64-job and 16 MiB limits allowed the transfer to finish, but a 64 MiB screen used 18% more CPU and 26% more PSS than the eight-block build.

A 16-block candidate with a shorter 32 ms window used 2% less CPU in one 64 MiB round, but 4.7% more PSS. Neither 16-block variant passes the joint CPU/PSS gate. The production per-connection limit remains eight. Both patches and all three screen summaries are preserved for later refinement.

## Other screened avenues

- The bounded mapped-range cache passed focused storage and wire tests. Its first two versions had very high CPU in short screens; the third tied CPU within 0.3% but raised PSS by about 0.84 MiB. The mapped-window patch and all three summaries are preserved.
- The active-stream hash-lookup change had mixed size-dependent results. Its three-round 16 MiB CPU median was essentially tied with the control, and the 64 MiB screen was slower. Its patch and summaries are preserved.
- These experiments did not change main. The measured eight-block batch is the only source change from this folder promoted in commit 39554462.

## Files

The JSON summaries bind results to binary hashes, fixture hashes, run order, environment overrides, transfer completion, and verification. The client logs hold the batch-size traces and the failed BatchTooLarge candidate. The compressed release and PGO binaries and the PGO profile make the leading candidate reproducible without retaining generated payloads. Patches for the mapped-range, active-stream, eight-block, and both 16-block candidates are included.
