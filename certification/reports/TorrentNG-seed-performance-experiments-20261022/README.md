# Native seed performance follow-up — 2026-10-22

**Status:** main commit `39554462` now collects upload reads for up to 32 ms and batches up to eight contiguous 16 KiB blocks. Three-round release and PGO screens show lower CPU with small PSS reductions; PGO remains opt-in. The current rTorrent 0.16.25 parity goal remains unmet. io_uring stays selectable after its startup fix, but its repeated screen did not beat `pread` on CPU.

A current-source configuration screen found a single-peer win with `storage.io_worker_threads = 1`: three-round median CPU fell 23.6%, PSS 6.0%, and anonymous peak 14.2% against the default of two workers. A randomized three-peer follow-up reversed CPU in all rounds (+11.9% to +19.0%) while PSS fell 4.5–5.6% and anonymous peak fell 13.4–18.0%. Keep the default at two. Two lazy second-worker prototypes were then screened: neither started worker two under the three-peer workload, and neither showed a reliable CPU/PSS win. Their patches and thread snapshots remain in [`observations/lazy-worker-trigger-screen-20261023/`](observations/lazy-worker-trigger-screen-20261023/).

A bounded batch-rate-admission refactor was screened against the current 16 ms release and rTorrent 0.16.25. It did not establish a CPU and PSS win together and was not promoted; its patch and complete two-screen evidence are in [`observations/upload-rate-batch-20261022/`](observations/upload-rate-batch-20261022/).

The objective remains to meet or beat current rTorrent in both process CPU and memory. These local tests do not reach that objective. The last published whole-client comparison remains the reference: TorrentNG at 1.90 CPU seconds and 19.82 MiB PSS versus rTorrent at 0.73 seconds and 10.74 MiB PSS.


## Current main improvement: bounded eight-block reads

Commit `39554462` raises the bounded upload-read collection delay to 32 ms and allows up to eight contiguous legal 16 KiB requests in one 128 KiB storage read. The per-connection cap remains eight; global admission remains capped at 64 jobs and 16 MiB. The work preserves individual wire requests and their ownership/cancellation behavior.

Randomized matched local loopback screens used the same 4 MiB/s offered rate, 32-request window, and SHA-256-verified payloads. At 64 MiB, the release candidate reduced median CPU by 22.2%, PSS by 1.5%, and anonymous peak by 7.5% versus the prior release source. The fresh PGO candidate improved on its matched PGO control at both tested sizes:

| Fixture | Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| 64 MiB, 3 rounds | Release control | 4.504 | 18.875 MiB | 6.500 MiB |
| 64 MiB, 3 rounds | Eight-block release | **3.506** (-22.2%) | **18.586 MiB** (-1.5%) | **6.012 MiB** (-7.5%) |
| 64 MiB, 3 rounds | PGO control | 3.481 | 16.037 MiB | 6.020 MiB |
| 64 MiB, 3 rounds | Eight-block PGO | **3.016** (-13.4%) | **15.791 MiB** (-1.5%) | **5.953 MiB** (-1.1%) |
| 64 MiB, 3 rounds | rTorrent 0.16.25 | 2.211 | 7.676 MiB | 3.074 MiB |
| 128 MiB, 3 rounds | PGO control | 4.019 | 16.184 MiB | 6.477 MiB |
| 128 MiB, 3 rounds | Eight-block PGO | **3.348** (-16.7%) | **15.593 MiB** (-3.6%) | **6.023 MiB** (-7.0%) |
| 128 MiB, 3 rounds | rTorrent 0.16.25 | 2.361 | 7.607 MiB | 3.070 MiB |

The 128 MiB PGO candidate remains about 1.42× rTorrent CPU and 2.05× its PSS. A 128 MiB release check was a single round and is screening-only. These results support retaining the change; they do not replace the official whole-client qualification. All scored transfers passed SHA-256, and the release-source change passed 769 `rt-engine` tests plus the formatting check before commit.

`MALLOC_ARENA_MAX=1` improved the eight-block PGO candidate's 64 MiB three-round medians by 2.1% CPU, 7.4% PSS, and 20.8% anonymous memory, but a one-round 128 MiB check used 2.3% more CPU for 8.7% less PSS. Follow-up randomized release-build screens improved CPU/PSS/anonymous memory together at 64 MiB (-19.0%/-8.0%/-22.8%) and 128 MiB (-6.7%/-9.6%/-27.2%). A later randomized three-round multi-peer screen found cap 1 used 9.6–40.3% more CPU in every round (median +40.3%), while reducing median PSS 7.9% and anonymous memory 27.9%. Keep it as an optional Linux/glibc memory-for-CPU tradeoff, not a general improvement; the [follow-up results](observations/arena-cap-concurrent-followup-20261023/README.md) record the exact comparison. The full single-peer results and diagnostics remain under [`observations/seed-goal-next-avenues-20261022/`](observations/seed-goal-next-avenues-20261022/). Sixteen-block variants did not improve CPU and PSS together: the fixed 64 ms candidate raised CPU 18% and PSS 26%; a 32 ms one-round variant lowered CPU 2.1% but raised PSS 4.7%. Their patches, failure, logs, binaries, profiles, and JSON summaries remain archived under [`observations/seed-goal-next-avenues-20261022/`](observations/seed-goal-next-avenues-20261022/).

## Method

- Earlier PGO and io_uring screens used source checkout `7ba8bb6a5b3505d4d263e35c649e24a9f0761d44`. Coalescing screens used `4341f2819ee4bd10160f153c13c67d27346ce19b` as their common source base and changed only `UPLOAD_READ_COALESCE_MAX_DELAY` from 8 ms to 16 ms in the candidate.
- Host: `kspld0`, AMD Ryzen 9 9950X3D.
- The earlier screens used `rustc 1.97.0 (2d8144b78 2026-07-07)`, LLVM 22.1.6. Coalescing release binaries used Rust 1.94.0; coalescing PGO binaries used Rust 1.97.0.
- Workload: private loopback seed, one 16 KiB block size, a 32-block request window, 4 MiB/s paced receiver, and tracker/DHT/PEX disabled. The 128 MiB coalescing comparisons pinned `pread`. At 64 MiB, storage backend `auto` selected the same backend within each paired round, but varied across rounds (release: `pread`, `uring`, `uring`; PGO: `pread`, `pread`, `uring`).
- Payloads were deterministic (probe seed `20261022`) and SHA-256 verified. The 64, 128, and 256 MiB fixtures have hashes `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`, `7fae01c69f5254b9fbf3cdf6db292cc5da49ee3fa4bfbb995de8aa3b66dd6e67`, and `937d7bd20b42ad9d0130419a685ce1e481f4330ab2a211dde1e16403c7854f81` respectively. Raw summaries and logs are kept under [`observations/`](observations/); oversized generated payload copies are omitted and can be regenerated from the seed and size documented there.
- CPU is process CPU time from first payload to payload completion, normalized per GiB. PSS and anonymous memory are active-window peaks. Transfer duration is a validity check, not a performance score.

These are controlled local screens, not the repository's official whole-client qualification. Their source-built binaries and load shape differ from the release comparison. The 128 MiB and 64 MiB three-client screens include the same local benchmark rTorrent binary as a comparator.

## Strongest candidate: full-profile PGO

The same full-profile PGO binary improved on the unprofiled build in repeated 128 MiB and 256 MiB screens. The 128 MiB result is consistently positive across all three paired rounds; at 256 MiB the CPU gain is small and round-to-round results vary. Memory improved in both sizes.

| Fixture | Build | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---|---:|---:|---:|
| 128 MiB, 3 rounds | Release baseline | 5.04 | 19.43 MiB | 6.53 MiB |
| 128 MiB, 3 rounds | Full-profile PGO | **3.68** (-27.0%) | **16.10 MiB** (-17.1%) | **6.28 MiB** (-3.8%) |
| 256 MiB, 3 rounds | Release baseline | 4.88 | 19.44 MiB | 6.54 MiB |
| 256 MiB, 3 rounds | Full-profile PGO | **4.60** (-5.7%) | **16.22 MiB** (-16.6%) | **6.29 MiB** (-3.8%) |

The follow-up three-client 128 MiB comparison reused the exact fixture and binaries, randomized order across three rounds, and verified every transfer. Medians were:

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Release baseline | 5.70 | 19.38 MiB | 6.55 MiB |
| Full-profile PGO | **3.68** (-35.4%) | **15.80 MiB** (-18.5%) | **6.08 MiB** (-7.2%) |
| rTorrent 0.16.25 | 2.15 | 7.90 MiB | 3.04 MiB |

PGO still used 1.71× rTorrent CPU and 2.00× its PSS. This strengthens the case for preserving and qualifying the PGO build while making clear it does not meet the target. The three-round CPU values were 3.680, 3.415, and 3.702 CPU-s/GiB for PGO; 4.301, 6.729, and 5.700 for baseline; and 2.106, 2.258, and 2.152 for rTorrent. All nine transfers passed SHA-256 against fixture `7fae01c69f5254b9fbf3cdf6db292cc5da49ee3fa4bfbb995de8aa3b66dd6e67`.

The PGO candidate binary used for the 128/256 MiB tables has SHA-256 `4b95eafd08d685e89272b7bc6bb24e37b906bbab6bfaa208a14f5026df801547`; the baseline binary has SHA-256 `368bb71e0d7bfae9322aeb3a2dfac7648d57e515984c852d433713b78bae8754`. The merged profile SHA-256 is `45c1f0352f9fe9f1622b783ced1a48a93269cc2eb79ce77549468e35401fc4ed`; the compressed profile and its training summary are preserved in [`observations/pgo-full-training/`](observations/pgo-full-training/). The training run used a 64 MiB verified seed and an instrumented daemon with SHA-256 `18d9aa4e4221df8b03e76b4ece08943514cbb04b98ab7d06c81879e705a530e9`.

The opt-in [`build_native_pgo_release.sh`](../../../scripts/build_native_pgo_release.sh) accepts the archived compressed profile, pins Rust 1.97.0, applies the workspace release settings, and prints the profile and output-binary hashes. Three clean builds from the same source and profile produced different binary hashes (`4b95eafd08d685e89272b7bc6bb24e37b906bbab6bfaa208a14f5026df801547`, `9697ac512212d97cc3d3c23f8c5f38cfbdfae7cf30d390c77bb04e946c64c6cf`, and `80d20641f4611979bfdd9cd12254bab524436267bdf9333938f9a786b453041e`); the exact cause of this codegen variation is unresolved, so those hashes identify individual artifacts rather than a byte-for-byte reproducibility promise. The full-profile behavior was screened on the `80d206…` script-built binary in a separate 64 MiB, one-round comparison. It used 3.21 CPU-s/GiB, 15.72 MiB PSS, and 5.97 MiB anonymous memory, versus the release baseline at 4.43, 18.85 MiB, and 6.01 MiB; rTorrent measured 2.40, 7.88 MiB, and 3.03 MiB. All three transfers passed SHA-256 verification. This confirms the direction in a quick screen but does not qualify that exact artifact; keep the build opt-in and re-screen each built binary before broader use. The profile was trained on one 64 MiB seed fixture, so PGO still needs download, multi-peer, and official release qualification before becoming a default.

An earlier three-client, three-round 64 MiB screen measured PGO at 4.007 CPU-s/GiB and 15.63 MiB PSS, against baseline at 6.529 and 18.80 MiB, and rTorrent at 2.310 and 8.33 MiB. PGO remained about 1.74× rTorrent CPU and 1.88× its PSS. A separate one-round 256 MiB comparison likewise left PGO above rTorrent: 3.565 versus 2.112 CPU-s/GiB and 16.25 versus 8.29 MiB PSS. Those observations show PGO is useful, but does not meet the goal.

The transfer-only PGO profile did not beat the full profile in its one-round 64 MiB screen: 3.832 versus 3.527 CPU-s/GiB and 18.22 versus 13.21 MiB PSS. PGO is not enabled by the product release profile. The archived profile now supports rebuilding and screening the candidate, but clean binary hashes vary and the profile lacks download/multi-peer training coverage. Keep the build opt-in until a clean-build artifact passes repeated qualification.

## Upload request coalescing: 8 ms to 16 ms

The peer loop originally waited up to 8 ms to collect pipelined upload requests. A matched diagnostic trace showed the request burst usually dispatched three contiguous 16 KiB blocks together. Raising only the maximum wait to 16 ms let the same pattern fill four-block reads and vectored writes. Both 16 MiB traces verified the same payload and completed the 4 MiB/s payload window in 4.0001 seconds; debug logging was enabled, so the traces are for batch attribution, not CPU scoring.

| Build and fixture | 8 ms CPU-s/GiB | 16 ms CPU-s/GiB | 8 ms peak PSS | 16 ms peak PSS | 8 ms peak anonymous | 16 ms peak anonymous |
|---|---:|---:|---:|---:|---:|---:|
| Release, 64 MiB, 3 pairs | 5.96 | **5.10** (-14.4%) | 18.80 MiB | **18.56 MiB** (-1.3%) | 6.09 MiB | **5.80 MiB** (-4.8%) |
| Release, 128 MiB, 3 pairs | 4.57 | **4.09** (-10.6%) | 19.21 MiB | **18.90 MiB** (-1.6%) | 6.61 MiB | **6.24 MiB** (-5.6%) |
| Full-profile PGO, 64 MiB, 3 pairs | 3.95 | **3.46** (-12.4%) | 16.34 MiB | **16.24 MiB** (-0.6%) | **6.09 MiB** | 6.42 MiB (+5.5%) |
| Full-profile PGO, 128 MiB, 3 pairs | 3.41 | **3.32** (-2.6%) | 16.65 MiB | **16.19 MiB** (-2.8%) | 6.55 MiB | **6.19 MiB** (-5.5%) |

All 27 transfers in the four scored comparisons passed SHA-256 verification and ran for the same offered-rate window. The PGO screens used same-source binaries and archived profile SHA-256 `45c1f0352f9fe9f1622b783ced1a48a93269cc2eb79ce77549468e35401fc4ed`. That profile predates the io_uring worker-selection fix; the candidate build emitted three known profile-mismatch warnings for storage-backend functions. Treat those PGO values as a paired screen, not a clean profile qualification. The 64 MiB screens happened to use the same treatment order in all three repeats; the 128 MiB release screen used the reverse order, while the three-client PGO screen varied order as listed in its summary. These small screens are evidence for retaining the bounded change, not a final qualification across storage backends.

The same-fixture 128 MiB PGO comparison included source-built rTorrent 0.16.25: it used 2.32 CPU-s/GiB, 7.90 MiB peak PSS, and 3.04 MiB peak anonymous memory. The 16 ms TorrentNG build remained about **1.43× rTorrent CPU, 2.05× PSS, and 2.04× anonymous memory**. This is a measured step, not parity. The earlier retained full-profile PGO result (3.68 CPU-s/GiB, 15.80 MiB PSS, 6.08 MiB anonymous) remains in the report; the new PGO screen improves CPU but does not beat those historical absolute memory minima. The 64 MiB PGO anonymous-memory increase is retained as a size-dependent tradeoff.

The 16 MiB diagnostic trace counted 350 storage reads for 1,024 blocks at 8 ms (`320×3`, `8×4`, `10×2`, `12×1`) and 257 at 16 ms (`255×4`, `2×2`), a **26.6% reduction**. This supports the mechanism but does not replace the whole-client measurements. At that point, the 16 ms setting was in the release source; commit 39554462 later superseded it with the measured 32 ms/eight-block default. Exact 8 ms and 16 ms binaries, original and normalized summaries, logs, metrics, compressed smaps, and the one-line patch are preserved under [`observations/upload-coalesce-16ms/`](observations/upload-coalesce-16ms/).

Validation after the source change: `cargo +1.94.0 fmt --all -- --check`, the focused `peer_loop_flushes_small_upload_batches_after_the_short_deadline` test, and all 769 `rt-engine` tests passed.

## Fresh PGO profiles trained from the 16 ms source

The older full-profile PGO data predates the io_uring worker-selection fix. I retrained two profiles from exact source commit `6cfc6a9aba79c954624d9d86f6508fb49ec0a894` using Rust 1.97.0 / LLVM 22.1.6, then built each with the pinned PGO release script. Both training transfers were 64 MiB, SHA-256 verified, and pinned to `pread`.

The transfer-window profile reset LLVM counters at the first scheduled upload batch. In a five-pair 128 MiB comparison against the existing 16 ms PGO binary, its median CPU improved by 5.1% and anonymous memory by 4.7%, while median PSS rose by 11.2%:

| PGO profile | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---:|---:|---:|
| Existing profile control | 3.16 | **16.58 MiB** | 6.47 MiB |
| Fresh transfer-window profile | **3.00** (-5.1%) | 18.44 MiB (+11.2%) | **6.17 MiB** (-4.7%) |

All ten scored transfers passed SHA-256 verification and took the same 32-second payload window. The backend was pinned to `pread`. The five rounds ran control first in three pairs and candidate first in two; this is a useful repeated screen, not a fully balanced order or a new whole-client qualification. The current-source PGO profile improves CPU and anonymous memory but loses on PSS, so it does not replace the existing PGO candidate or the release default. Its exact profile SHA-256 is `80fda6a6a1da618cfc7e6bf364ea90c6e27d87cc7278bcc6c1fbfea151580b82`; the binary SHA-256 is `ce7e769b49af69509f71c031efa22b2af1c1a0e31fc453e848f59ea809513039`.

The second fresh profile counted the full process from startup rather than resetting at the first upload. Its one-pair 128 MiB screen was worse than the existing-profile control on all three resource metrics: 3.42 vs. 3.04 CPU-s/GiB (+12.6%), 16.74 vs. 15.91 MiB PSS (+5.2%), and 6.53 vs. 6.08 MiB anonymous memory (+7.3%). That screen ran the full-profile candidate first, verified both transfers, and is only screening evidence. Its profile SHA-256 is `47d37cf2ba582152f86fc8e3cd9d34998f0f8485fc4433082b10c47fd66de581`; its binary SHA-256 is `635a9cf74459ee3b63dd9279910c44ebe62a2baf68a7ba6118ef2e25fcae66cd`.

The existing 16 ms PGO binary used as the control has SHA-256 `8e8522729bdc06b8590995945e87544741370cb66c70eb13576d54f8e378f006`. The new profile-use builds produced no profile-mismatch warnings; this fixes the stale-profile concern but does not fix the PSS tradeoff. Neither screen included rTorrent, so these runs make no new claim about the comparator gap. The earlier best PGO PSS result remains 15.80 MiB and is preserved above. All new profiles, exact instrumented and optimized binaries, training records, paired summaries, logs, metrics, smaps, and checksums are under [`observations/pgo-current-source-16ms-20261022/`](observations/pgo-current-source-16ms-20261022/).

### Matched native-target PGO follow-up

The earlier `target-cpu=native` attempt reused a portable-target profile and was stopped after LLVM reported control-flow mismatches. I trained a new profile with the native target enabled in both the instrumented and profile-use builds. The profile-use build completed without mismatch warnings. This removes the profile mismatch as an explanation for the result, but the candidate still lost the short whole-client screen.

Five clients ran one randomized 64 MiB local seed transfer each, all at 4 MiB/s with the same 16 KiB requests, 32-request window, `pread`, disabled DHT/PEX, and no tracker. Every payload matched SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`. The single-round order was PGO control, native-target PGO, release, rTorrent, then portable-target PGO.

| Build | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Retained PGO control | 2.812 | 16.37 MiB | 6.33 MiB |
| Matched native-target PGO | 3.025 (+7.6%) | 18.22 MiB (+11.3%) | 6.00 MiB (-5.2%) |
| Fresh portable-target PGO | 2.795 (-0.6%) | 18.54 MiB (+13.3%) | 6.50 MiB (+2.7%) |
| Release | 3.675 | 18.79 MiB | 6.06 MiB |
| rTorrent 0.16.25 | 2.068 | 7.59 MiB | 3.05 MiB |

The native-target candidate therefore did not improve CPU and PSS together against its PGO control. Its slightly lower anonymous peak is retained as a separate memory observation, but does not offset the CPU and PSS regressions. The run started while `kspld0` was busy (load averages 23.27/20.21/18.89 and about 117 GiB of swap in use), so these one-round values are exploratory rather than a new ranking. The existing release and PGO choices are unchanged. The candidate's optimized and instrumented binaries, raw and merged LLVM profiles, verified summary, logs, metrics, compressed `smaps`, and checksums are retained under [`observations/pgo-native-target-screen-20261022/`](observations/pgo-native-target-screen-20261022/).

### Equal-weight full-process and transfer-window profile blend

I combined the current-source full-process and transfer-window profiles at equal weight. Both inputs were trained with Rust 1.97.0 / LLVM 22.1.6 from the same source and portable target, and the profile-use build completed without mismatch warnings. A first five-client 64 MiB round looked encouraging against the PGO control: 3.106 vs. 3.587 CPU-s/GiB, with PSS 17.29 vs. 16.69 MiB. The blend ran last, so I expanded it to three randomized rounds rather than treating that as a win.

| Build | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---:|---:|---:|
| Existing PGO control | 3.049 | 16.08 MiB | 6.34 MiB |
| Equal-weight profile blend | 3.229 (+5.9%) | 17.58 MiB (+9.3%) | 6.01 MiB (-5.2%) |
| Transfer-window PGO | 2.972 (-2.6%) | 18.52 MiB (+15.2%) | 6.31 MiB (-0.5%) |
| rTorrent 0.16.25 | 1.975 | 7.62 MiB | 3.05 MiB |

All 17 transfers across the initial and repeated screens verified the same fixture SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`. Across the repeated screen, the blend's CPU was worse than the control in two rounds and slightly better in one; its PSS was higher in all three. It did lower anonymous memory, but did not improve CPU and PSS together against the retained PGO control. No build setting changed. The one-round summary, three-round summaries, exact merged profile and binary, and process diagnostics are preserved under [`observations/pgo-profile-blend-20261022/`](observations/pgo-profile-blend-20261022/).

## Source-level candidates and reversions

Every rejected or inconsistent source candidate is kept as a patch under [`experiments/`](experiments/) so it can be revisited without losing its result.

| Candidate | Result | Decision |
|---|---|---|
| Initial eight-block variant with the shorter 16 ms collection window | One 64 MiB round: CPU rose 14.9%, PSS 7.1%, and anonymous peak 8.4% versus its baseline. This was the earlier short-window candidate, not the later 32 ms change now on main. | Rejected as measured; retain [`upload-batch8.patch`](experiments/upload-batch8.patch) and the later successful candidate evidence under [`observations/seed-goal-next-avenues-20261022/`](observations/seed-goal-next-avenues-20261022/). |
| 32 ms collection window with up to eight contiguous blocks | Three-round release screen: CPU -22.2%, PSS -1.5%, anonymous peak -7.5%. Fresh PGO screens improved CPU/PSS at 64 and 128 MiB. | Retained on main as `39554462`; preserve the complete measurements and candidate artifacts under [`observations/seed-goal-next-avenues-20261022/`](observations/seed-goal-next-avenues-20261022/). |
| 32 ms collection window with up to twelve contiguous blocks | One 64 MiB screen: CPU 1.9%, PSS 2.4%, and anonymous peak 4.3% above the current eight-block source. All checksums passed, but the run was under high host load. | Not promoted; keep [`upload-batch12.patch`](experiments/upload-batch12.patch) and its test/build/benchmark evidence under [`observations/batch12-screen-20261022/`](observations/batch12-screen-20261022/). A quiet-host repeat is needed before drawing a firm conclusion. |
| Remove the explicit TCP `Framed::flush()` before raw Piece writes | One screen appeared 21.8% faster with 6.1% lower PSS, but the randomized confirm was nearly tied in round 1 and 6.6% slower with 2.7% higher PSS in round 2. Host load was elevated, and remaining rounds were stopped. | Not promoted; 770 engine tests and the wire-order test passed. Preserve [`skip-peer-framed-preflush.patch`](experiments/skip-peer-framed-preflush.patch), exact binary, and all completed summaries under [`observations/framed-preflush-screen-20261022/`](observations/framed-preflush-screen-20261022/). |
| Borrow `ResourceGovernor` in peer codec capacity checks instead of cloning its `Arc` | One randomized 64 MiB screen: candidate CPU +17.3%, PSS +0.11 MiB, anonymous memory tied. Candidate ran last as host load rose; profile signal was only 7 ms in a 332 ms sample. | Not promoted; focused peer-wire tests passed. Keep [`peer-codec-governor-borrow.patch`](experiments/peer-codec-governor-borrow.patch), candidate binary, and raw screen under [`observations/peer-codec-borrow-screen-20261022/`](observations/peer-codec-borrow-screen-20261022/); profile is in [`observations/current-eight-block-profile-20261022/`](observations/current-eight-block-profile-20261022/). Revisit only with a quiet, reversed-order screen if profiling still points to codec admission. |
| Use Tokio current-thread runtime for one worker; preserve multithread when explicitly raised | One randomized 64 MiB screen: CPU +8.1%, PSS +0.32 MiB, anonymous memory +0.03 MiB. Candidate ran first while host load eased. | Not promoted; runtime flavor tests passed. Keep [`current-thread-runtime.patch`](experiments/current-thread-runtime.patch), binary, and raw screen under [`observations/current-thread-runtime-screen-20261022/`](observations/current-thread-runtime-screen-20261022/). The one-worker multithread runtime remains the default. |
| Cap glibc arenas under three concurrent seed peers | Cap 1's three-round median used 40.3% more CPU while lowering PSS 7.9% and anonymous memory 27.9%; CPU was worse in all rounds. Cap 2's three-round median lowered PSS 3.2% and anonymous memory 11.7%, while CPU was effectively tied and reversed direction in the order-reversed round. | Keep arena caps optional for memory tuning; no CPU win or default change. Exact summaries are in [`observations/arena-cap-concurrent-followup-20261023/`](observations/arena-cap-concurrent-followup-20261023/). |
| Keep the active peer-read stream outside its path map on current main | Two 64 MiB screens gave opposing CPU results by order. The candidate median used 1.3% more CPU, 1.6% more PSS, and 8.4% more anonymous memory than control. | Not promoted. Preserve the current-main patch, binary, focused tests, and both verified runs under [`observations/active-stream-current-main-screen-20261023/`](observations/active-stream-current-main-screen-20261023/). |
| Keep the active `PeerReadStreams` entry outside the lookup map | Three 16 MiB rounds used 4.4% less median CPU, then three 64 MiB rounds used 13.0% more CPU. PSS changed by -0.2% and -0.5%, while anonymous memory changed by -0.8% and +0.2%, respectively. | Not promoted: the CPU direction reversed with fixture size and memory was effectively tied. Preserve the patch and verified summaries under [`observations/peer-read-current-entry-screen-20261023/`](observations/peer-read-current-entry-screen-20261023/). |
| Use Tokio `select! { biased; }` in the peer loop | The first pair used 12.3% less CPU, and an initial three-way median used 5.1% less. Order-balanced confirmation used 1.9% more CPU; a five-pair alternating screen used 13.7% more median CPU, with CPU losses in four pairs. PSS was slightly lower in that final screen. | Not promoted: the CPU change was not repeatable and moved against control in the balanced confirms. Preserve the patch and all verified runs under [`observations/peer-select-biased-screen-20261023/`](observations/peer-select-biased-screen-20261023/). |
| Start a second storage worker only when jobs overlap | The queue-only trigger raised three-peer CPU 28.8% and PSS 1.8%; a refined active-plus-queued trigger did not start worker two and was only one round, with CPU -2.9% and PSS +1.0%. | Not promoted. Preserve both patches and thread snapshots under [`observations/lazy-worker-trigger-screen-20261023/`](observations/lazy-worker-trigger-screen-20261023/). |
| Feed Piece-frame `writev` from a bounded mmap window | Four counter-balanced local socket pairs produced mixed paired CPU results: two faster and two slower, with less than 1% mean difference. No process memory or whole-client result was measured. | Do not add production mmap ownership on this evidence. Keep the C probe and full results under [`observations/mapped-window-writev-probe-20261023/`](observations/mapped-window-writev-probe-20261023/). |
| Reduce peer read-ahead from 512 KiB to 128 or 256 KiB | Preliminary screens favored the smaller windows on CPU, but two randomized confirmation rounds measured median CPU +14.3% at 128 KiB and +6.0% at 256 KiB; PSS and anonymous memory did not improve. | Keep the 512 KiB default. Preserve the exact temporary config harnesses, per-run TOML, logs, and summaries under [`observations/readahead-window-screen-20261023/`](observations/readahead-window-screen-20261023/). |
| Batch per-block upload-rate admissions under one tree lock | Two randomized one-round 64 MiB screens: median CPU rose 8.3%, PSS 0.8%, and anonymous memory 2.9%; CPU moved in opposite directions in the two rounds. | Not promoted; keep the exact patch, candidate executable, and verified summaries under [`observations/upload-rate-batch-20261022/`](observations/upload-rate-batch-20261022/). |
| Cache-hit single-lookup refactor | One round looked faster, but the three-round median was 5.801 CPU-s/GiB versus 3.780 for the PGO control; PSS was essentially tied. | Reverted as inconsistent; keep [`cache-hit-single-hash.patch`](experiments/cache-hit-single-hash.patch). |
| Active stream kept outside the cache map | At 64 MiB under `opt-level=s`, the three-round median improved CPU 16.3% and PSS was tied. At 256 MiB, the single candidate round used 9.8% more CPU with effectively tied PSS. In a refreshed PGO comparison, it was 6.7% slower and used 0.46 MiB more PSS than the PGO control. | Reverted as size/profile dependent; keep [`stream-cache-active.patch`](experiments/stream-cache-active.patch). |
| Wait for a fourth request before dispatch | The runtime setting disabled the read elevator, so the probe did not exercise batching and is not a scored result. | Do not infer a gain; preserve [`fourth-request-probe.patch`](experiments/fourth-request-probe.patch) for a corrected test. |
| io_uring startup and queue-sized ring | The earlier requested arm actually selected `pread`. Direct worker startup now selects io_uring at queue depth 64. A three-round verified comparison found higher CPU with slightly lower PSS and anonymous memory. | Keep the startup correction and explicit backend available; do not promote io_uring as a performance winner or change the default queue depth. Results and invalid attempts are retained under [`observations/io-uring-queue-depth-64/`](observations/io-uring-queue-depth-64/). |
| Opportunistically join a second ready four-block read for an eight-frame TCP write | One instrumented 16 MiB trace produced two eight-frame writes and saved three calls out of 257; a second intake-cap-eight variant raised storage reads to 402 and socket writes to 280. Short diagnostic CPU/PSS readings were worse and are not scores. | Do not promote either socket-write variant; retain their patches and traces under [`observations/socket-write-coalesce-screen-20261022/`](observations/socket-write-coalesce-screen-20261022/). The separate measured 32 ms/eight-block read improvement is retained on main. |
| Current-source mimalloc, including zero-delay purge | Two one-pair 16 MiB verified screens showed no stable CPU gain; mimalloc used 4.5–5.1 MiB more PSS and 4.4–4.8 MiB more anonymous memory. | Stop allocator tuning before a larger comparison. Keep the system allocator default and retain the summaries under [`observations/mimalloc-current-source-screen-20261022/`](observations/mimalloc-current-source-screen-20261022/). |
| io_uring at queue depths 1, 4, and 8 | Depth 4 looked faster in two single pairs, but its randomized 64 MiB three-pair repeat used 7.3% more median CPU than `pread`, with tied PSS. Depth 8 used 11.5% more CPU in its one-pair screen; depth 1 saturated the scheduler before a valid transfer. | Keep `pread` as default. Preserve the startup fix and all mixed/failed evidence under [`observations/io-uring-queue-depth-followup-20261022/`](observations/io-uring-queue-depth-followup-20261022/). |

### Allocator and codegen screens

Six one-round randomized, SHA-256-verified 64 MiB screens compared the system
allocator with jemalloc and two jemalloc refinements, a portable build with
`target-cpu=native`, and Rust 1.94 with Rust 1.97. None lowered CPU and PSS
together. Jemalloc used 0.8% more CPU and 14.0% more PSS than the
system allocator; one arena used 0.4% more CPU and 12.0% more PSS, while
immediate decay used 6.2% more CPU and 29.7% more PSS. Host-tuned codegen used
9.2% more CPU for 1.2% lower PSS. Rust 1.97 used 1.2% more CPU and 2.5% more
PSS. These are one-round screens, not medians; no build default changed. Exact
candidate executables and raw summaries are retained under
[`observations/low-risk-tuning-screen-20261022/`](observations/low-risk-tuning-screen-20261022/).

The `opt-level=3` screen reduced CPU by 10.5% against the compact release but
raised PSS by 14.9%. In that same round, the PGO control used 4.851 CPU-s/GiB
and 16.25 MiB PSS, versus 6.259 CPU-s/GiB and 21.48 MiB for `opt-level=3`.
Host load was high, so this remains exploratory and does not replace any
previous best. A separate mapped-window probe used 8.6% less CPU than already
batched 64 KiB `pread` reads in a hot data-only test; the result excludes the
scheduler and socket path and is not a whole-client gain. Both reports and
raw evidence are retained under
[`observations/low-risk-tuning-screen-20261022/`](observations/low-risk-tuning-screen-20261022/)
and [`observations/mapped-window-read-probe-20261022/`](observations/mapped-window-read-probe-20261022/).

An `opt-level=3` candidate reduced CPU by 10.5% against the compact release in
one 64 MiB round, but raised PSS by 14.9% and lost to the PGO control on both
CPU and PSS. Host load was high during that screen, so it remains exploratory.
The exact binary and run summary are retained. A separate hot-file probe found
that 1 MiB mmap windows reduced median CPU by 8.6% versus already-batched
64 KiB `pread` reads; this is a read-stage result, not a whole-client gain.
The profile and raw measurements are under
[`observations/mapped-window-read-probe-20261022/`](observations/mapped-window-read-probe-20261022/).

### io_uring startup attribution and measured tradeoff

The old `TNG_STORAGE_BACKEND=uring` screen was not an io_uring result: its runtime gauge showed `pread`. A focused reproduction found that selection allocated and registered a probe ring, then the real worker failed with `ENOMEM`; starting the worker directly at queue depth 64 succeeded. The selector now starts the actual worker and falls back to `pread` only if that startup fails. The benchmark harness accepts a bounded `--storage-io-queue-depth` setting and records the active backend label, preventing another silent fallback from being scored as io_uring. Queue depth and release defaults were not changed.

Three randomized 64 MiB rounds compared the same release binary with `pread` and io_uring, using queue depth 64 for both. All six transfers matched fixture SHA-256 `b5ae92fdc692ec945b19d5a8beea2b0640bf3b4f1675c961dc44a988d0721901`, and the diagnostic gauge confirmed the requested backend in every round.

| Backend | CPU-s/GiB median | Peak PSS median | Peak anonymous median |
|---|---:|---:|---:|
| `pread` | **4.46** | 19.11 MiB | 6.41 MiB |
| io_uring, queue depth 64 | 5.02 (+12.5%) | **18.62 MiB** (-2.5%) | **6.09 MiB** (-5.0%) |

The CPU cost is higher, while the memory reductions are small and these screens do not establish a general performance win. The one-round cross-client screen and follow-up PGO screen on the same 64 MiB fixture were:

| Build | Backend | CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---|---:|---:|---:|
| Release | `pread` | 5.91 | 19.01 MiB | 6.51 MiB |
| Release | io_uring, queue depth 64 | 8.42 | 18.58 MiB | 6.01 MiB |
| rTorrent 0.16.25 | — | 3.46 | 7.90 MiB | 3.04 MiB |
| Updated-source PGO | `pread` | 4.03 | 16.18 MiB | 6.49 MiB |
| Updated-source PGO | io_uring, queue depth 64 | 6.15 | 15.97 MiB | 5.87 MiB |
| rTorrent 0.16.25 | — | 2.28 | 7.90 MiB | 3.04 MiB |

The updated-source PGO build emitted profile-mismatch warnings for the changed backend functions, so those two rows are screening evidence only. Neither screen displaces the retained full-profile PGO result of 3.68 CPU-s/GiB / 15.80 MiB PSS at 128 MiB.

### io_uring queue-depth follow-up

Because the first queue-depth-64 screen traded higher CPU for slightly lower
memory, smaller rings were screened before dropping the backend. At depth 4,
one 16 MiB pair and one 64 MiB pair appeared to improve CPU, PSS, and anonymous
memory. A randomized three-pair 64 MiB repeat did not confirm the CPU gain:
median CPU was 6.040 CPU-s/GiB for io_uring versus 5.628 for `pread` (+7.3%),
while PSS was effectively tied (18.50 versus 18.51 MiB) and anonymous peak was
2.5% higher (6.11 versus 5.96 MiB). io_uring was slower in each paired round.
The depth-8 one-pair screen also used 11.5% more CPU; depth 1 failed with
scheduler queue-full errors before a valid transfer. Backend-selection gauges
were checked for every scored run, and all scored transfers verified their
fixture hashes. No storage default changed. Full measurements and failed
attempt evidence are in
[`observations/io-uring-queue-depth-followup-20261022/`](observations/io-uring-queue-depth-followup-20261022/).

The first three-client attempt is preserved but invalid: the harness correctly removed inherited loader variables, and rTorrent then exited 127 because `libtorrent.so.53` was not found. The valid retry passed the pinned `LD_LIBRARY_PATH` explicitly. The exact output binary hashes, selected-backend snapshots, process maps, logs, and original plus relocated summaries are in the evidence directory above. The code and harness correction were committed on `main` as `85dc7800`.

The experiments also keep evidence that did not fit a clean score: an initial rTorrent launch failed because its loader could not find `libtorrent.so.53`; the later valid runs use a pinned `LD_LIBRARY_PATH`. Failed and diagnostic summaries remain in the observations directory and are excluded from performance claims.

## Runtime and codegen screens

Fat-LTO PGO produced a smaller executable but did not improve runtime resources:
one verified 64 MiB round measured 3.8804 CPU-s/GiB and 18.96 MiB PSS, versus
3.6020 and 16.50 MiB for the PGO control. `MALLOC_ARENA_MAX=1` was a repeatable
memory tradeoff: across three rounds it reduced median PSS by 7.4% and anonymous
memory by 19.5%, while increasing CPU by 5.4%. A one-round `MALLOC_ARENA_MAX=2`
screen raised CPU by 42.3% for 4.0% less PSS. None changes the release or PGO
defaults; all raw evidence and the retained fat-LTO executable are in
[`observations/runtime-tuning-screen-20261022/`](observations/runtime-tuning-screen-20261022/).

One exact-binary, 256 MiB PGO CPU profile sampled broad peer-task and supervised
torrent-task poll frames most often among resolved TorrentNG frames. `PreparedPread::execute`
was 2.9% of the TorrentNG leaf sample weight. The 47.1% of all sample weight
inside libc is now symbol-resolved using the matching debug ELF: the largest
entry was `__syscall_cancel_arch` (257 ms), with caller samples mapping to TCP
read (129 ms), vectored write (67 ms), and a broad Tokio worker-poll stack (54
ms). A further 23 ms in `statx` came from `HandleCache::touch_for_access` and
its deliberate path-replacement fence. This points toward transport and poll
work but does not identify removable kernel work or a whole-client CPU gain;
the identity check remains required. `PeerReadStreams::observe` was not sampled
as a separate symbol, which is not proof that it is free. This is diagnostic
evidence only, not a client comparison. It does not justify spending a
matched-run cycle on the path-map refactor alone. The profile, exact summary,
and resolution notes are in
[`observations/pgo-hotpath-profile-20261022/`](observations/pgo-hotpath-profile-20261022/).

A 64 MiB profile of the exact current eight-block release binary likewise found
no dominant TorrentNG leaf: 52% of 332 ms of samples landed in libc, and
`PeerCodec::reserve_capacity` accounted for 7 ms (2.1%). A borrow-only refactor
removed the codec's `ResourceGovernor` clones and passed 59 peer-wire tests,
but the first matched 64 MiB screen measured 4.175 versus 3.560 CPU-s/GiB and
18.63 versus 18.52 MiB PSS; it ran last as host load rose. Do not promote it
from this screen. Preserve the complete profile and candidate artifacts under
[`observations/current-eight-block-profile-20261022/`](observations/current-eight-block-profile-20261022/)
and [`observations/peer-codec-borrow-screen-20261022/`](observations/peer-codec-borrow-screen-20261022/).

## Upstream rTorrent path and mapped-range evidence

The comparator is built from the official [rTorrent v0.16.25 release](https://github.com/rakshasa/rtorrent/releases/tag/v0.16.25). Its paired [libTorrent v0.16.25 README](https://github.com/rakshasa/libtorrent/blob/v0.16.25/README.md) describes transferring directly from file pages to the network stack. That supports testing file-page-backed payload ownership. It does not establish that raw `sendfile` is the right implementation: our sendfile measurement traded a large anonymous-memory reduction for a 4.14× CPU increase.

TorrentNG currently resolves a verified v1 upload to a file span, then calls `read_owned_at_with_priority`; storage schedules a positioned read into an owned frame, and the peer writer sends the resulting `Bytes`. Earlier bounded mapped-range probes did not improve whole-client CPU and PSS together: the latest mapped-window candidate tied CPU within 0.3% but raised PSS by about 0.84 MiB. The read-only mapping route remains a revisit candidate only if it can share bounded mappings across reads and remove enough scheduler, allocation, or copy work to beat the current eight-block main source. Any future screen must keep the buffered fallback and check hot and cold file pages, fairness, cancellation, and path-replacement fencing. A map-per-block implementation is not a candidate.

## Reproducibility and evidence

[`observations/`](observations/) contains the complete probe JSON and captured client logs for the scored and invalid screens. [`manifest.json`](observations/manifest.json) records each file's byte length and SHA-256; it also records the uncompressed PGO profile hash. The summaries bind measurements to binary checksums, fixture hashes, round order, transfer completion, and payload verification.

The benchmark probe's regression tests cover its process sampling, profile-window setup, configuration, and cleanup behavior. They must pass before another qualification run. The 32 ms/eight-block source change is already on main after randomized paired 64 MiB release and 64/128 MiB PGO screens; future candidates must use current main as control, keep the exact current rTorrent binary, repeat at 128 and 256 MiB, and pass correctness plus the official release comparison before any further default change.

The separate supplied source-review archive is assessed in [`TorrentNG-rtorrent-hot-path-review-20261022/README.md`](../TorrentNG-rtorrent-hot-path-review-20261022/README.md). Its source files match the reviewed `7ba8bb6a` checkout, so it adds no code change.
