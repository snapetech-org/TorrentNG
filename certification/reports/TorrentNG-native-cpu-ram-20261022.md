# TorrentNG native CPU and RAM benchmark report

**Status:** the clean profile/arena matrix, the completed current-thread cycle, and the final consolidated 12/12 comparison are complete and verified. The consolidated result supports the size-optimized release profile as the default: process PSS fell in every paired old/new comparison, while CPU differences were inconsistent. TorrentNG still used about 2.60× rTorrent CPU and 1.85× its PSS in this workload, so parity remains unresolved. The project has adopted the tested compact profile and keeps explicit opt-3 available; this is a measured default choice, not a claim that performance parity is reached. Earlier interrupted and ENOSPC attempts remain documented and excluded from clean matrices.

## Executive summary

The final consolidated size-profile build (same source as opt-3) had 18.3% lower median PSS than the previous best one-worker binary, with CPU medians of 1.90 s versus 1.88 s and paired CPU ratios spanning below and above 1. The result does not establish a CPU gain; it does support the compact profile for lower PSS. The size build's ELF `.text` section was 30.8% smaller than opt-3, while its full file was 4.8% smaller; section size is not itself resident memory. The consolidated source includes dense verification, the immediate rate-grant probe, one default Tokio worker in the multithread runtime, and optional Transmission/Deluge APIs. These are measured together; the cycle does not isolate each source change's contribution.

Earlier evidence remains useful with its limitations: the verify-only comparison in the incomplete 11/12 matrix showed 9.2% lower median PSS and 26.5% lower anonymous memory than baseline, but no CPU benefit. The current-thread cycle did not show a consistent benefit and that candidate was rejected. Fat LTO lowered median CPU slightly while increasing anonymous memory. Arena tuning showed no clear benefit.

The selected release configuration uses opt-level `s`, the multithread Tokio runtime with a default of one worker, the system allocator, and thin LTO with one codegen unit. The default API surface retains native and qBittorrent and makes Transmission and Deluge opt-in; restore them with `--features full-compat` (or select either compatibility feature individually). Opt-level 3 remains available by building with `CARGO_PROFILE_RELEASE_OPT_LEVEL=3`. Build, test, and Clippy gates passed on Rust 1.97.0; detailed build provenance follows below.

## Completed consolidated source and profile comparison

The final authorized comparison tested three paired randomized rounds for each of four variants using a 256 MiB fixture at 4 MiB/s, a 32-request window, and no DHT, PEX, or tracker. All 12 transfers verified. TorrentNG used the same consolidated source for opt-3 and size-profile variants; both used the multithread runtime with the new one-worker default, system allocator, and thin LTO. The old-best comparator used the earlier verify-only binary with `TOKIO_WORKER_THREADS=1`. rTorrent used the pinned 0.16.25 binary. RSS, PSS, and anonymous memory in the table are active-interval peaks from the pre-leecher sample through the post-leecher boundary, not a payload-only instantaneous sample.

The run manifest records no environment overrides for the new opt-3 and opt-s candidates. Warn-level startup logs do not print the selected worker count; attribution to the one-worker default comes from source and the default-worker tests. Authenticated API and `smaps` snapshots were collected before torrent add, after seed recheck, and after transfer. Published evidence contains compact results and build identities; raw API metrics, process samples, timestamps, and machine paths are omitted.

| Variant | N verified | CPU seconds, median (range) | PSS MiB, median (range) | RSS MiB, median (range) | Anonymous MiB, median (range) |
|---|---:|---:|---:|---:|---:|
| Old best, one Tokio worker | 3 | 1.88 (1.84–2.14) | 24.26 (23.64–24.48) | 24.98 (24.36–25.21) | 6.55 (6.05–6.55) |
| Consolidated source, opt-3 | 3 | 1.92 (1.75–2.10) | 23.12 (22.81–23.16) | 23.85 (23.53–23.88) | 6.37 (6.05–6.55) |
| Consolidated source, opt-s | 3 | 1.90 (1.71–1.93) | 19.82 (19.45–19.96) | 20.54 (20.17–20.67) | 6.50 (6.13–6.63) |
| Pinned rTorrent 0.16.25 | 3 | 0.73 (0.68–0.76) | 10.74 (10.72–10.75) | 12.46 (12.45–12.48) | 2.64 (2.63–2.64) |

Opt-s median PSS was 18.3% lower than old-best; all three paired PSS comparisons were lower (ratios 0.844, 0.817, 0.794). CPU was inconclusive: the opt-s median was 1.90 s versus 1.88 s old-best (+1.1%), and paired CPU ratios were 0.888, 1.049, and 0.910. Compared with opt-3, opt-s median PSS was 14.3% lower while median CPU was 1.90 versus 1.92 s; the paired CPU ratios again mixed across 1. The evidence supports the compact profile for lower PSS, not a CPU win.

Against rTorrent, opt-s used 2.60× CPU (1.90 vs 0.73 s) and 1.846× PSS (19.82 vs 10.74 MiB). The active-interval RAM and CPU gap remains. This single-host, three-round, paced workload does not establish broader behavior or host-wide page-cache parity.

The same-source opt-s binary's ELF `.text` section was 12,670,727 bytes versus 18,320,887 bytes for opt-3 (30.8% smaller); full binary file size was 27,685,088 versus 29,070,400 bytes (4.8% smaller). After-transfer `smaps` separates the app executable mapping from anonymous memory: median executable PSS was 11.09 MiB for opt-s, 14.71 MiB for opt-3, and 15.71 MiB for old-best; anonymous PSS was 5.79, 5.75, and 5.91 MiB respectively. The executable-mapping reduction explains most of the PSS improvement; anonymous PSS was similar. ELF `.text` and process PSS are distinct measurements.

The project adopted opt-level `s` in `[profile.release]`. The environment-unset Rust 1.97 build reproduced the exact tested opt-s binary SHA-256 `4aaa587c0953a8a4d73a01c52ea4429cb03699f232d63c6c4fb0d2ef6b407e63`; opt-3 remains available through `CARGO_PROFILE_RELEASE_OPT_LEVEL=3`. The default runtime remains Tokio multithread with one worker and the system allocator. Native and qBittorrent APIs remain enabled by default; Transmission and Deluge are opt-in, individually or together with `--features full-compat`. This default API change can require enabling compatibility features for existing users of those endpoints.

The consolidated build used source HEAD `152be9714b4b229194877476023f4061784bc946`, compared with baseline `a8154943f9bd489d79e84ab13423befb576d2789`. The production diff SHA-256 at profile adoption was `502fe6d88db3fbb07d4ae09a33cc94024aba250c809e4b014b199833b576b3ff`. On Rust 1.97.0, 769 `rt-engine` tests passed; daemon default, full-compat, and opt-s configurations each passed 32 tests with one ignored. Clippy passed for engine, daemon default, and full-compat with warnings denied. The workflow runner policy tests (4) and API contract checks (63 paths, 89 operations) passed; shell syntax and source/document diff checks passed. The environment-unset build confirmed that the adopted Cargo default reproduces the tested opt-s artifact exactly.

The old-best versus new opt-3 comparison combines the rate-probe and API changes, so it cannot isolate those source changes individually. Both sides retain dense verification and use one worker. The same-source opt-3 versus opt-s comparison does isolate the compiler profile: median PSS fell from 23.12 to 19.82 MiB. The immediate grant probe preserves grant reservation and fairness semantics; its standalone CPU effect remains unknown. Performance with the optional compatibility APIs re-enabled was not measured. Opt-s is selected because it had consistently lower PSS than old-best and lower executable mapping PSS than opt-3, with no measured CPU advantage claimed.

## Scope and build provenance

CPU and memory were measured for native TorrentNG seeding against pinned rTorrent 0.16.25. Transfers used fixed 4 MiB/s pacing and verified the received fixture by SHA-256. CPU values below are process user+system ticks at 100 Hz, from the `payload_cpu_seconds_user_system` field: first-payload sample through payload-complete sample. Memory values labeled as active peaks are maxima over a broader resource-sampling interval from the pre-leecher sample through the post-leecher boundary; this includes connection setup and wait, payload transfer, and the post-transfer sample. CPU/GiB divides CPU seconds by fixture size in GiB.

| Item | Value |
|---|---|
| Baseline source for prior matrices | `a8154943f9bd489d79e84ab13423befb576d2789` |
| 128 MiB comparison toolchain | Rust stable 1.99.1; opt 3, thin LTO, codegen units 1, system allocator |
| Prior 256 MiB matrices toolchain | Rust 1.97.0 (LLVM 22.1.6); opt 3, codegen units 1, strip debuginfo; thin LTO except the labeled fat-LTO variant; system allocator |
| Host | Linux x86_64, AMD EPYC 9V74; available affinity CPUs 0–8 |
| rTorrent reference | rTorrent/libtorrent 0.16.25; pinned SHAs and build flags in [build info](TorrentNG-native-cpu-ram-20261022/rtorrent-build-info.txt) |
| Clean-matrix/current probe artifact SHA-256 | `05cff349eeb248f41290fdd7e889b4d3946020d1137b914fe2f8e8f24812fc74` |
| Clean-matrix/current probe-test artifact SHA-256 | `e715448c3154508f8d590d6ec645f76d8a96a54c12afc016e07611add52c6023` |
| Current probe tests | 16 passed |
| Clean 256 MiB summary SHA-256 | `3e952166903a81eda236d5af1b26061033088efe4c26acf1a7727fd91b75a34d` |
| Probe and recipe | [probe script](../../scripts/native_seed_cpu_ram_probe.py); [probe tests](../../scripts/test_native_seed_cpu_ram_probe.py); [benchmark recipe](../benchmarks/rtorrent-current/Dockerfile) |

The earlier baseline/verify matrix binary identities are recorded in its summary and [Rust 1.97 build metadata](TorrentNG-native-cpu-ram-20261022/build-metadata-197.txt): baseline `a2812d73b9b70e562ceb4dfeb5089aeeeffe558a775ff58ed40ff221cefc0e7d`; verify-only `94a24269b7aefb191be39d7d955c9e25cf93bdacd3914bd6d36756569a356cd0`; rTorrent `b7828655feb7fa0b15d6a091d2bdbb0656be88a010d0c59d275d086ef40c61a8`. The verify-only and one-worker rows use the same binary; the latter sets `TOKIO_WORKER_THREADS=1`. The 1.99.1 host environment capture applies to the earlier 128 MiB run; the matrix compiler is documented separately in build metadata.

The hashes above identify the clean-matrix/current probe artifacts. Earlier 128 MiB and incomplete-matrix/recovery runs used historical revisions with the same CPU/RAM sampling mechanics but different pacing, recovery, payload-cleanup, and environment-guard handling. Their summaries do not record historical runner-file hashes.

## Preliminary 128 MiB comparison

The `screen128-1` comparison used a 128 MiB fixture, 32 seconds of payload at 4 MiB/s, a 32-request window, loopback, and no DHT, PEX, or tracker. All six transfers completed and verified 134,217,728 bytes. `combined batch + dense` identifies a binary containing both changes; its results do not isolate the verify-only patch. The batch-release variant is exploratory and was removed from the working patch.

| Variant | N | CPU seconds, median (range) | CPU seconds/GiB, median (range) | Peak RSS MiB, median (range) | Peak PSS MiB, median (range) | Peak anonymous MiB, median (range) |
|---|---:|---:|---:|---:|---:|---:|
| baseline | 2 | 1.15 (1.12–1.18) | 9.20 (8.96–9.44) | 26.54 (26.52–26.57) | 25.83 (25.81–25.84) | 8.79 (8.67–8.91) |
| combined batch + dense | 2 | 1.235 (1.19–1.28) | 9.88 (9.52–10.24) | 24.74 (24.38–25.11) | 24.02 (23.66–24.39) | 6.34 (6.01–6.66) |
| batch release experiment | 2 | 1.245 (1.15–1.34) | 9.96 (9.20–10.72) | 27.43 (26.80–28.05) | 26.71 (26.09–27.33) | 8.64 (8.36–8.92) |

The combined variant's CPU range exceeded baseline; its lower observed memory does not identify which of its two changes caused the difference. The grouped upload-release experiment did not establish CPU benefit and was removed from the working patch. Its saved [comparison diff](TorrentNG-native-cpu-ram-20261022/batch-permits-complete.patch) is in the evidence folder. These two repeats do not establish general CPU or RAM improvement.

## Prior incomplete 256 MiB matrix: 11 verified cells, one failed cell (excluded)

An earlier intended 12-cell matrix was baseline, verify-only, verify-only with one Tokio worker, and rTorrent, each for three randomized rounds. Fixture size was 256 MiB; each completed transfer verified 268,435,456 bytes at 4 MiB/s with a 32-request window. The original summary is **failed**, not complete: 11 cells completed and verified; the round 3 rTorrent cell encountered `ENOSPC` during the final run and is excluded. Thus rTorrent has two paired-matrix samples, while each TorrentNG variant has three. This failed attempt is retained only as history; none of these observations are included in the clean matrix below.

| Variant | N verified | CPU seconds, median (range) | CPU seconds/GiB, median (range) | Peak RSS MiB, median (range) | Peak PSS MiB, median (range) | Peak anonymous MiB, median (range) |
|---|---:|---:|---:|---:|---:|---:|
| baseline-197 | 3 | 2.37 (2.33–2.42) | 9.48 (9.32–9.68) | 27.72 (27.48–28.07) | 27.01 (26.76–27.36) | 9.19 (8.94–9.52) |
| verify-197 | 3 | 2.47 (2.10–2.49) | 9.88 (8.40–9.96) | 25.24 (24.92–25.29) | 24.52 (24.20–24.56) | 6.75 (6.73–7.00) |
| verify-oneworker-197 | 3 | 2.04 (1.91–2.05) | 8.16 (7.64–8.20) | 24.48 (24.27–25.02) | 23.76 (23.56–24.31) | 6.07 (6.07–7.14) |
| rTorrent matrix cells | 2 | 0.69 (0.67–0.71) | 2.76 (2.68–2.84) | 12.48 (12.42–12.54) | 10.73 (10.63–10.83) | 2.63 (2.63–2.63) |

The verify-only change showed lower median PSS and anonymous memory than baseline in these runs, while its CPU median was slightly higher. The one-worker variant had lower medians for CPU and memory than baseline in this earlier attempt. This incomplete attempt is historical context only; it is excluded from the clean matrix below.

### Separate rTorrent recovery sample

After the final-cell disk-space failure, rTorrent was rerun separately in single-client mode. The 256 MiB payload verified. This is a single recovery sample, not a third paired-matrix observation:

| Sample | N | CPU seconds | CPU seconds/GiB | Peak RSS MiB | Peak PSS MiB | Peak anonymous MiB |
|---|---:|---:|---:|---:|---:|---:|
| rTorrent recovery | 1 | 0.61 | 2.44 | 12.40 | 10.68 | 2.64 |

The two matrix rTorrent rows and this recovery row use the same recorded binary SHA-256, but the recovery run remains separately identified because its run mode and pairing context differ.

## Clean 256 MiB profile and arena matrix: 12/12 verified

The clean matrix in the [summary JSON](TorrentNG-native-cpu-ram-20261022/clean-screen256-profile-arena-2-summary.json) (SHA-256 `3e952166903a81eda236d5af1b26061033088efe4c26acf1a7727fd91b75a34d`) completed all 12 randomized cells: four variants across three rounds, with every 268,435,456-byte transfer verified. The fixture ran at 4 MiB/s with a 32-request window; DHT, PEX, and trackers were disabled. It is a fresh set and does not merge the six-cell interrupted workspace attempt, the earlier failed `/tmp` attempt, the prior 11/12 ENOSPC matrix, or the separate rTorrent recovery sample.

Each TorrentNG variant used one Tokio worker. `thin-control` used thin LTO and the default glibc arena configuration. `thin-arena2` used the same thin binary with `MALLOC_ARENA_MAX=2`. `fat-oneworker` used the verify-only source with a fat-LTO build and one Tokio worker; its build provenance is in [fat-LTO metadata](TorrentNG-native-cpu-ram-20261022/build-info-verify-fat197.txt). rTorrent used the pinned reference binary. The thin-control and thin-arena2 rows share binary SHA-256 `94a24269b7aefb191be39d7d955c9e25cf93bdacd3914bd6d36756569a356cd0`; fat-oneworker is `ce889879c020cecb964519611e7176c724be87eaa43a440c2f3df64f9305b587`; rTorrent is `b7828655feb7fa0b15d6a091d2bdbb0656be88a010d0c59d275d086ef40c61a8`.

| Variant | N verified | CPU seconds, median (range) | CPU seconds/GiB, median (range) | Peak RSS MiB, median (range) | Peak PSS MiB, median (range) | Peak anonymous MiB, median (range) |
|---|---:|---:|---:|---:|---:|---:|
| thin-control | 3 | 1.93 (1.82–2.01) | 7.72 (7.28–8.04) | 24.66 (24.10–26.03) | 23.93 (23.38–25.30) | 6.05 (6.04–7.36) |
| thin-arena2 | 3 | 1.90 (1.76–1.97) | 7.60 (7.04–7.88) | 24.38 (23.91–24.70) | 23.66 (23.20–23.97) | 6.27 (5.58–6.51) |
| fat-oneworker | 3 | 1.81 (1.71–1.82) | 7.24 (6.84–7.28) | 24.81 (24.77–25.18) | 24.09 (24.04–24.46) | 6.58 (6.53–7.02) |
| pinned rTorrent | 3 | 0.62 (0.61–0.75) | 2.48 (2.44–3.00) | 12.53 (12.51–12.58) | 10.80 (10.78–10.85) | 2.64 (2.64–2.64) |

The clean matrix shows lower memory and CPU medians for these TorrentNG settings than in the earlier default-worker baseline rows, but CPU and PSS remain above pinned rTorrent in this 256 MiB loopback workload. Fat LTO and arena2 are measured settings, not deployment recommendations. This one host and three rounds per variant do not establish broad performance behavior.

## Retained change, checks, and exploratory batch work

The retained source diff is a 43-line verify-only change in [`crates/rt-storage/src/verify.rs`](../../crates/rt-storage/src/verify.rs), saved exactly as [retained verify-only patch](TorrentNG-native-cpu-ram-20261022/retained-verify-only.patch). For a request covered by one dense extent it returns the owned read and avoids allocating and copying a second full-range buffer; sparse and multi-extent reads retain their existing path. This change was tested separately in the 256 MiB matrix. Its patch SHA-256 is `eebbcaa4010b7bac4e4f0ee55871505e3d3843384c47634dbb613cc44c9d5c26`.

Recorded checks: 31 `rt-storage` tests passed; Clippy passed for `rt-storage --lib --tests` with the retained change. The rejected grouped-release experiment passed 12 scheduler tests, 53 upload tests, 2 remote-cancel tests, 1 disconnect test, and 2 batched-piece-writer tests; Clippy passed for `rt-engine --lib`. The grouped-release source change was not retained.

## Profile, cache, and mapping observations

A preliminary whole-process gperftools profile collected 235 samples across daemon startup, hashing/recheck, and seeding. It is not a transfer-only profile and cannot support an under-load CPU attribution. One captured `/metrics` snapshot showed 512 KiB of peer-read cache, 634 hits, 77 misses, and no evictions. The endpoint returns the last cached engine snapshot and schedules an asynchronous refresh, so these are snapshot values, not necessarily current-at-scrape values or full-transfer totals. The configured 16 MiB budget is not evidence of 16 MiB resident or used.

Post-transfer `smaps` snapshots from the clean matrix show executable file-backed mappings with median PSS of 15.51–16.04 MiB and read-only file mappings of 2.17–2.49 MiB across the three TorrentNG builds. Their combined medians are 17.95–18.43 MiB, above rTorrent's whole-process PSS median of 10.80 MiB. Anonymous mapping PSS medians are 5.42–6.05 MiB. The largest mapping is the `r-xp` `torrentngd` executable. The rTorrent summary has RSS/PSS/anonymous process values but no saved `smaps`, so its file-backed versus anonymous mapping categories are unknown. The source identifies the mapped executable, but these numbers do not show which daemon features account for its image size. The bounded [mapping totals JSON](TorrentNG-native-cpu-ram-20261022/smaps-mapping-totals.json) preserves mapping-category totals and input hashes for all 27 existing clean-matrix snapshots, and is derived without new process sampling.

The following are per-process diagnostic snapshots (median and range across the three corresponding TNG runs), not phase peak values. Entries are MiB; each field is shown as median (min–max).

| Snapshot | Variant | RSS | PSS | Anonymous |
|---|---|---:|---:|---:|
| Before torrent add | thin-control | 18.81 (18.63–18.86) | 18.12 (17.94–18.17) | 3.18 (3.17–3.19) |
| Before torrent add | thin-arena2 | 18.60 (18.53–18.64) | 17.90 (17.84–17.95) | 3.09 (3.08–3.11) |
| Before torrent add | fat-oneworker | 19.27 (19.26–19.30) | 18.58 (18.57–18.61) | 3.08 (3.08–3.09) |
| After seed recheck | thin-control | 24.20 (23.52–24.26) | 23.49 (22.80–23.54) | 5.73 (5.72–5.73) |
| After seed recheck | thin-arena2 | 23.43 (23.39–23.62) | 22.71 (22.67–22.91) | 5.51 (5.50–5.54) |
| After seed recheck | fat-oneworker | 23.66 (23.59–23.68) | 22.95 (22.88–22.96) | 5.63 (5.63–5.64) |
| After transfer | thin-control | 24.66 (24.10–26.03) | 23.93 (23.38–25.30) | 6.05 (6.04–7.36) |
| After transfer | thin-arena2 | 24.38 (23.91–24.70) | 23.66 (23.19–23.97) | 6.27 (5.58–6.51) |
| After transfer | fat-oneworker | 24.81 (24.77–25.18) | 24.09 (24.04–24.46) | 6.58 (6.53–7.02) |

The clean matrix does not save rTorrent `smaps` snapshots, so these diagnostic memory tables are TNG-only. They are point samples and should not be read as the active-interval peaks reported in the clean 256 MiB comparison table.

### Separate thread CPU diagnostic

One additional 256 MiB verify-oneworker diagnostic run used per-thread CPU sampling. It is separate from the failed 11-cell matrix and is not a clean performance comparison. The run verified the payload. During the first-payload-to-complete interval, the process used 2.01 CPU seconds; per-thread counters attributed 1.39 seconds to the Tokio worker, 0.31 seconds to each of the two disk workers, and 0.00 tick seconds to the hash thread. The matched-thread sum was 2.02 seconds, a 0.01-second difference from process ticks. The `+8s` metrics snapshot showed a zero cache gauge, while the post-transfer snapshot showed 641 hits and 74 misses. Those metrics came from the last cached engine snapshot with asynchronous refresh; review indicates the post-transfer values reflect about 34 MiB and 8 seconds of activity, not the full 256 MiB transfer. Neither snapshot proves the cache was empty at `+8s` or that 641/74 are whole-transfer totals. These are one-run diagnostic counters, not a general CPU profile. The [thread diagnostic summary](TorrentNG-native-cpu-ram-20261022/thread-cpu-verify197-1worker-256-summary.json) and [thread sampler](TorrentNG-native-cpu-ram-20261022/proc_thread_cpu_probe.py) are preserved separately.

## Harness validity and limits

The harness samples the host-visible process PID and records `payload_cpu_seconds_user_system`, RSS, PSS, anonymous memory, and authorized TorrentNG metrics. Child processes strip inherited loader, profiler, allocator, logging, Tokio, TNG, and TorrentNG environment controls; the harness records safe control names and values, then applies explicit client overrides. It does not record full environment or token values. It paces the first payload and captures first-payload and completion boundaries. The pinned rTorrent seed-side probe sends an explicit empty Bitfield before Interested; an earlier stalled smoke omitted that message and did not produce an UNCHOKE, so that run is excluded from parity evidence. A separate current-screen report records about 63 seconds between rTorrent's first connected peer and first payload without the peer interest/choke/request state needed to diagnose it; this delay is not attributed to that missing Bitfield or treated as evidence about the clean matrix ([current rTorrent screen](TorrentNG-current-rtorrent-screen-20261022.md)). The preliminary, incomplete, recovery, and clean summaries are retained separately.

After a verified transfer and client shutdown, the harness removes only the generated per-run seed and downloaded payload copies. `--keep-payloads` retains them; failed or unverified runs retain generated payloads for diagnosis. The fixture source is not removed.

CPU is quantized to 0.01 seconds from 100 Hz process ticks. Memory figures are sampled active-interval peaks, not whole-machine RAM. The clean matrix has three samples per variant and covers one host, one fixture size, one paced workload, and a small set of explicit runtime/build settings.

## Source comparison and engineering implications

The following source comparison describes baseline commit `a8154943f9bd489d79e84ab13423befb576d2789`, which was the source used for the completed CPU/RAM matrix. It is not a description of all current working-tree changes. In that baseline, each successful upload block entered the demand registration, grant, demand retirement, and settlement path described below.

The measured gap has a concrete implementation-level explanation, though the source audit does not assign a causal percentage. In pinned libtorrent, the peer write callback validates the queued request, loads the requested piece as a mapped chunk, and streams its address through the socket write path. It keeps the mapped chunk while subsequent requests use that same piece; moving to another piece releases the old chunk and maps the new extent. There is no per-block pread job, owned payload allocation, or disk-worker completion handoff on this plain TCP upload path. See the pinned [request-to-chunk path](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/protocol/peer_connection_base.cc#L315-L331), [piece write path](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/protocol/peer_connection_base.cc#L716-L764), [mapped-byte iterator](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/data/chunk_iterator.h#L78-L84), and [mmap creation and chunk reuse](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/torrent/data/file_list.cc#L582-L656) plus [chunk lifetime](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/data/chunk_list.cc#L82-L143). The project README describes this as transferring directly from file pages to the network stack. That means no userspace payload copy; it does not mean NIC zero-copy.

rTorrent's peer request/write work runs in the libtorrent poll event thread ([pinned rTorrent startup and event loop](https://github.com/rakshasa/rtorrent/blob/15ccae27deb9aa1dc8e46ea0ed218a673ccc5732/src/main.cc#L164-L168) and [event-loop entry](https://github.com/rakshasa/rtorrent/blob/15ccae27deb9aa1dc8e46ea0ed218a673ccc5732/src/main.cc#L489); [pinned libtorrent peer write callback](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/protocol/peer_connection_leech.cc#L582-L629)). rTorrent also has separate SCGI and session threads, so this describes the peer transfer path rather than the entire process. A cold mapped page can fault in while that transfer callback accesses it, which can delay the event thread. The implementation validates the mapped extent and contains SIGBUS handling; these are real caveats, not evidence that it has no handling for file-size changes ([mapping bounds](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/data/socket_file.cc#L131-L155); [SIGBUS handling](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/data/chunk.cc#L51-L70)).

TorrentNG already removed avoidable copies from its complete V1/TCP path: it batches up to four contiguous blocks (at most 64 KiB), issues one owned read for that span, exposes per-block `Bytes` slices into shared backing, and writes ready frames with vectored I/O. See [bounded upload read batch](../../crates/rt-engine/src/torrent_task.rs#L10483-L10606), [batch intake](../../crates/rt-engine/src/torrent_task.rs#L10707-L10749), [shared frame ownership](../../crates/rt-storage/src/frame.rs#L332-L355), and [vectored TCP writer](../../crates/rt-engine/src/torrent_task.rs#L8457-L8576). In baseline `a815494`, each block received a separate hierarchical upload-rate grant after the batch read (`torrent_task.rs:10617-10684`); with unlimited configured limits, the successful path still did a demand-map lookup plus grant, demand-retire, and settle tree locks, while tracking grant identities for fairness and refunds. See the [pinned baseline rate service](https://github.com/snapetech-org/TorrentNG/blob/a8154943f9bd489d79e84ab13423befb576d2789/crates/rt-engine/src/network_budget.rs#L924-L930) and [pinned baseline rate tree](https://github.com/snapetech-org/TorrentNG/blob/a8154943f9bd489d79e84ab13423befb576d2789/crates/rt-engine/src/transfer/rate.rs#L557-L660). This is source-based work accounting, not CPU attribution.

The immediate-grant probe attempts the usual tracked grant under the rate-tree lock before allocating a demand ID or waiter guard. A `Wait` falls through to the existing registration/generation-snapshot/retry loop, preserving its lost-wakeup protection. The probe records no waiter and therefore does not retire or overwrite existing waiter owners. A successful grant still uses the same reservation identity, fairness/debit path, burst size, settlement, and refund behavior. It removes demand-map registration and guard-retirement work on uncontended grants without batching grants or changing grant size, fairness, or epochs. The final comparison measured this together with other source and profile changes, so it does not isolate the probe's CPU/RAM effect.

Storage reads also cross an async-to-disk-worker boundary: the storage scheduler admits the operation, uses a bounded worker queue for `pread`, then returns the owned frame to the awaiting peer task (`rt-storage/src/scheduler.rs:4493-4549`; `rt-storage/src/backend.rs:1753-1808,1826-1842,1923-1952`). There are two different cache-hit paths. On a prepared peer-read cache hit, `PreparedPread::execute` runs on the disk worker, upgrades a weak descriptor reference, checks path identity with `identity_matches_path`, and returns cached `Bytes`; it explicitly avoids open/catalog/LRU work (`rt-storage/src/scheduler.rs:2190-2235`). If that peer-read cache misses but the handle cache is warm, `HandleCache::touch` checks filesystem identity and updates its LRU under locks (`rt-storage/src/handle_cache.rs:478-509`; identity check at `rt-storage/src/open.rs:90-109`). The mapping totals above separate executable/read-only image mappings from anonymous mappings; the latter includes runtime heaps, stacks, and live state. The current upload path bounds itself at 16 pending read requests and batches of four blocks/64 KiB (`torrent_task.rs:163-172`), but a one-peer bounded request window is not a measurement of total per-peer memory under larger peer counts.

The retained 43-line dense-verify change concerns verification reads and does not remove work from this active seed-upload path. It should not be expected to close the measured upload CPU gap by itself. The preliminary trial history is descriptive only: the transfer-cutover candidate showed about 23% lower seed CPU in that specific run; the 16-read-window candidate was rejected; accounting/admission experiments showed no clear gain. The separate `urtorrent` comparison is not the pinned rTorrent comparator and is not used as rTorrent evidence.

An earlier [seed-read-path microprobe](TorrentNG-seed-read-path-probe-20261022.md) already compared a reused 16 KiB warm `pread` buffer to a single reused mmap over a hot 64 MiB file. Checksummed paired runs measured 0.1064 versus 0.0465 CPU-s/GiB median (mmap lower by 0.0599 CPU-s/GiB, 56.3%). That is about 8% of the separate 0.78 CPU-s/GiB whole-client gap as a scale comparison only: the probe used a different host/workload and excluded scheduler admission, descriptor/path checks, socket writes, rate accounting, cancellation, cold-page faults, and concurrent mutation. It does not predict an integrated transfer gain. Putting mmap behind the same generic scheduler and rate/ownership path may retain most of the measured overhead.

For further work, center source review on amortizing or eliminating repeated control-path ownership costs in the complete V1/TCP path. A bounded shard owner could carry a shared range read and completion while retaining separate per-block protocol obligations, rate fairness, cancellation, accounting, and peer-memory reservations. Rate-grant coarsening would need epoch-correct dynamic-limit updates and correct unused-credit refunds; it is not a safe shortcut around the fairness model. If mmap is considered, decide explicitly how the storage contract handles cold-page stalls and external replacement/truncation. The audited rTorrent `SocketFile::open` delegates to `fd_open_file`, then maps validated extents ([open and mapping](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/data/socket_file.cc#L20-L40), [mapped extent bounds](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/data/socket_file.cc#L131-L155)); this upload path does not show TorrentNG's explicit no-follow ancestor walk, regular-file validation, and descriptor/path identity checks ([secure open](../../crates/rt-storage/src/open.rs#L156-L194), [Unix no-follow traversal](../../crates/rt-storage/src/open.rs#L806-L856)). That is a scoped observation about these paths, not a global security claim about rTorrent. Do not remove those guarantees implicitly as a performance optimization.

There are two separate engineering goals here. For CPU, review and amortize work on the active upload control path. For fixed resident code, review linked image and always-constructed application/API state separately; payload-buffer or allocator tuning does not directly address mapped executable pages. Baseline `a815494` eagerly constructs and merges native, qBittorrent, Transmission, and Deluge API routers in the daemon (`crates/torrentngd/src/main.rs:325-410`). ELF inspection measured about 1.35 MiB of directly attributed API symbols against roughly 18.86 MiB of `.text`; static symbol sizes do not identify resident pages. The final source keeps native and qBittorrent APIs and makes Transmission and Deluge opt-in. Its final comparison lowered executable mapping PSS, but that result also includes code and profile changes and cannot assign savings to the API split. The binaries are already built with debug info stripped; arena2 did not produce a clear CPU improvement, and the warm mmap microprobe alone accounts for only about 8% of the separate whole-client CPU gap as a scale comparison. Do not treat debug stripping, an allocator switch, or replacing pread with mmap behind the same control path as a demonstrated solution. Keep fixed image/code residency separate from payload and per-peer memory when evaluating RAM options.

At catalog scale, the inspected implementations make different choices. Opening an rTorrent download sizes a `ChunkList` vector and `ChunkStatistics` vector to the piece count ([`DownloadMain::open`](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/download/download_main.cc#L110-L121), [`ChunkList` base](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/data/chunk_list.h#L20-L25), [`ChunkStatistics::initialize`](https://github.com/rakshasa/libtorrent/blob/6ad3fecf3e7e7aa86855445587b532a91256f344/src/download/chunk_statistics.cc#L55-L60)); data mappings are created lazily when chunks are requested. This is per-open-torrent state proportional to piece count, not evidence that every cold catalog row has these vectors. TorrentNG's restore path keeps dormant rows as compact durable projections and defers metainfo parsing, live tasks, and scheduler allocation until promotion ([restore path](../../crates/rt-engine/src/engine/restore.rs#L176-L270); [dormant seeding restore test](../../crates/rt-engine/src/engine.rs#L21129-L21243)). Preserve that cold-catalog strategy; do not copy rTorrent's opened-per-piece vectors into every cold row. No per-piece byte-size estimate is inferred here.

Process PSS is not total host memory. TorrentNG's `pread` path can use kernel page cache without mapping those pages into the daemon; rTorrent's resident mapped payload pages appear in its process RSS/PSS. That accounting difference can make a process-only comparison favor TorrentNG relative to total host memory, but it does not establish host-RAM parity. The available rTorrent summary has no `smaps` or cgroup page-cache series, so total host/cache comparison remains unknown.

### Remaining implementation directions

1. **Reduce ownership transitions in complete V1/plain-TCP seeding.** The immediate grant probe is implemented and measured only as part of the combined candidate; further work could give a bounded group of peers one owner for ready payload ranges, request obligations, and socket progress. Submit storage misses and return completions in bounded batches. An already-ready range should not need a fresh storage job merely to expose another block slice. This requires an explicit descriptor/range-lease validity contract: preserve secure opens, invalidate on engine-controlled move/delete/recheck, and decide how external replacement/truncation is detected before changing the present per-read validation behavior. Swapping `pread` for `mmap` behind the existing scheduler would leave much of this work intact.
2. **Preserve grant semantics when considering wider amortization.** The probe leaves grant size unchanged. Reserving a bounded byte grant for a batch or peer window is a separate, unimplemented design that must handle partial writes, cancellation, refunds, and live limit changes through the existing generation/epoch rules. The source shows where baseline work occurs; it does not prove how much CPU a wider grant would save.
3. **Interpret fixed daemon residency separately.** The same-source profile comparison reduced executable mapping PSS. Attribution among the combined API and rate-path source changes remains unresolved. Continue to separate code pages from heaps, payload, and per-peer state.
4. **Keep the runtime and profile tied to the measured result.** The current-thread cycle was rejected. Opt-s is the selected release profile for lower PSS; no CPU benefit was established. The cold-catalog strategy and bounded resource model remain important for large libraries; no per-library-scale estimate is inferred here.

The current-thread performance cycle documented below is complete and rejected. The consolidated source/profile comparison is complete; it supports the opt-s default for lower PSS but shows no CPU win or rTorrent parity.

### Completed current-thread runtime cycle (rejected)

This completed cycle compared the then-existing multithread runtime with `TOKIO_WORKER_THREADS=1` against an opt-in current-thread runtime under the same explicit one-worker setting, plus pinned rTorrent. Each variant ran three randomized 256 MiB transfers at 4 MiB/s; all 9 payloads verified and generated run payloads were removed after verification and client shutdown. The current-thread candidate was rejected.

| Variant | N | CPU seconds, median (range) | Anonymous MiB, median (range) | PSS MiB, median (range) | RSS MiB, median (range) |
|---|---:|---:|---:|---:|---:|
| Normal runtime, one worker | 3 | 1.92 (1.78–2.13) | 6.36 (6.10–6.87) | 24.02 (23.72–24.57) | 24.74 (24.45–25.30) |
| Current-thread runtime, one worker | 3 | 2.01 (1.62–2.03) | 6.35 (5.81–7.03) | 23.65 (23.44–24.85) | 24.34 (24.14–25.55) |
| Pinned rTorrent 0.16.25 | 3 | 0.66 (0.60–0.75) | 2.63 (2.63–2.64) | 10.80 (10.68–10.82) | 12.53 (12.41–12.55) |

The paired current-thread/normal CPU ratios were `0.910`, `1.047`, and `0.953`; current-thread used less CPU in two pairs and more in one. Its median CPU was 2.01 seconds versus 1.92 seconds for normal runtime, so the cycle shows no clear CPU win. Paired PSS ratios were `0.984`, `0.988`, and `1.011`, also small and inconsistent. The current-thread change is not retained. Although tests passed, it makes the main thread poll all Tokio tasks; a long synchronous section can stall network, timers, or shutdown work. These results do not justify a current-thread runtime; the final consolidated multithread one-worker result is measured separately above.

The binaries were `torrentngd-verify-197` (SHA-256 `94a24269b7aefb191be39d7d955c9e25cf93bdacd3914bd6d36756569a356cd0`), `torrentngd-current-thin197` (SHA-256 `b4b7d2ab72b360d97cadd9b0a6b1a222d0bfa750853bd73becd7373a27890b96`), and rTorrent (SHA-256 `b7828655feb7fa0b15d6a091d2bdbb0656be88a010d0c59d275d086ef40c61a8`). The current-thread candidate passed 33 tests with one ignored. Its patch and build metadata are retained as review evidence only: [source patch](TorrentNG-native-cpu-ram-20261022/runtime-current-thread-source.patch), [build and test metadata](TorrentNG-native-cpu-ram-20261022/runtime-current-thread-build-metadata.md). The candidate's semantic risk is described in that metadata.

Final-cycle provenance: runner SHA-256 `05cff349eeb248f41290fdd7e889b4d3946020d1137b914fe2f8e8f24812fc74`; summary SHA-256 `d9d1d1e73d9053d0dd02dfcf2eda1f14eaa463f8ed96730f2528b24f4cafac77`; cycle metadata SHA-256 `1289568df5add37f9ffb7e4b77a5b00a45176eeb62df644af82a4d8940880da4`. The runner hash is the final current artifact hash; earlier cycles used historical runner revisions.

## Bounded evidence

The evidence folder contains the run summaries, separate recovery and thread-diagnostic summaries, clean-matrix build metadata, host metadata, pinned rTorrent build information, the exact verify-only and grouped-release patches, the final consolidated source/profile comparison, the build/test gate, and the thread sampler. The newly published evidence includes build/test results with normalized build paths and compact measurement summaries. Those new files exclude raw process samples, timestamps, machine paths, environment captures, API telemetry, payloads, binaries, and profiler files; earlier published evidence is preserved. Input hashes identify the original local diagnostics; published-summary hashes identify the compact files.

- [128 MiB summary](TorrentNG-native-cpu-ram-20261022/screen128-1-summary.json)
- [Failed 256 MiB matrix summary](TorrentNG-native-cpu-ram-20261022/screen256-rust197-1-summary.json)
- [Clean 256 MiB profile/arena matrix summary](TorrentNG-native-cpu-ram-20261022/clean-screen256-profile-arena-2-summary.json)
- [Separate rTorrent recovery summary](TorrentNG-native-cpu-ram-20261022/rtorrent-recovery-rust197-final-summary.json)
- [Rust 1.97 build metadata](TorrentNG-native-cpu-ram-20261022/build-metadata-197.txt)
- [Fat-LTO build metadata](TorrentNG-native-cpu-ram-20261022/build-info-verify-fat197.txt)
- [Host environment capture for the 1.99.1 preliminary run](TorrentNG-native-cpu-ram-20261022/host-environment-screen128-rust199.json)
- [Pinned rTorrent build metadata](TorrentNG-native-cpu-ram-20261022/rtorrent-build-info.txt)
- [Thread diagnostic summary](TorrentNG-native-cpu-ram-20261022/thread-cpu-verify197-1worker-256-summary.json)
- [Thread sampler](TorrentNG-native-cpu-ram-20261022/proc_thread_cpu_probe.py)
- [Grouped-release comparison patch](TorrentNG-native-cpu-ram-20261022/batch-permits-complete.patch)
- [Clean-matrix smaps mapping totals and input hashes](TorrentNG-native-cpu-ram-20261022/smaps-mapping-totals.json)
- [Final current-thread cycle summary](TorrentNG-native-cpu-ram-20261022/final-current-thread-1-summary.json)
- [Final current-thread cycle metadata](TorrentNG-native-cpu-ram-20261022/final-current-thread-1-metadata.json)
- [Current-thread candidate source patch, not retained](TorrentNG-native-cpu-ram-20261022/runtime-current-thread-source.patch)
- [Current-thread candidate build and test metadata](TorrentNG-native-cpu-ram-20261022/runtime-current-thread-build-metadata.md)
- [Final consolidated 12/12 summary](TorrentNG-native-cpu-ram-20261022/consolidated-final-footprint-1-summary.json) (SHA-256 `46dc29a64b0172a994a2acd48fc378da432a68b785f350337c4e37ccf80aeb5a`)
- [Final consolidated run metadata](TorrentNG-native-cpu-ram-20261022/consolidated-final-footprint-1-metadata.json) (SHA-256 `cb4930d8a5298b8af8012014bf0dc8ba8222dc7899dd8ec840d18648ba4186a6`)
- [Final consolidated build and correctness gate](TorrentNG-native-cpu-ram-20261022/consolidated-build-gate-197.md) (SHA-256 `b112aefc1bb827f15a857a6aba60efa26a60d0a8d6aabc37e963e4ecca291d70`)
- [Adopted default profile equivalence check](TorrentNG-native-cpu-ram-20261022/adopted-default-profile-197.md)
- [Adopted default profile build output](TorrentNG-native-cpu-ram-20261022/adopted-default-profile-197-build.log)
- [Final consolidated smaps mapping totals](TorrentNG-native-cpu-ram-20261022/consolidated-final-footprint-1-smaps-mapping-totals.json) (SHA-256 `b5700c5c1ff3202b93bfe68e5033fb48e4816b93ecec8738be4a40af5444c484`)

## Implementation provenance

The implementation is published in commit [0070ef7](https://github.com/snapetech-org/TorrentNG/commit/0070ef7bcd2f2831a0466fb74860c1e2ded554cf). The final production diff SHA-256 is `e53b04955c457eb7de702b60f956110d3f1152aac0ee45b219d1bf851204cdfd`. A root Cargo.toml comment-only edit after the profile equivalence check changed no compiler setting; the four-file runtime/rate source hash and tested compiler profile remain unchanged.
