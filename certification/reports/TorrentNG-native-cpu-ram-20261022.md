# TorrentNG native CPU and RAM benchmark report

**Status:** partial benchmark evidence. The 256 MiB matrix failed at 11 of 12 cells because the final rTorrent run ran out of disk space. A separate recovered rTorrent sample is reported separately. Parity remains unresolved; this report makes no deployment recommendation.

## Scope and build provenance

CPU and memory were measured for native TorrentNG seeding against pinned rTorrent 0.16.25. Transfers used fixed 4 MiB/s pacing and verified the received fixture by SHA-256. CPU values below are process user+system ticks at 100 Hz, from the `payload_cpu_seconds_user_system` field: first-payload sample through payload-complete sample. Memory values labeled as active peaks are maxima over a broader resource-sampling interval from the pre-leecher sample through the post-leecher boundary; this includes connection setup and wait, payload transfer, and the post-transfer sample. CPU/GiB divides CPU seconds by fixture size in GiB.

| Item | Value |
|---|---|
| Source HEAD | `a8154943f9bd489d79e84ab13423befb576d2789` |
| 128 MiB comparison toolchain | Rust stable 1.99.1; opt 3, thin LTO, codegen units 1, system allocator |
| 256 MiB matrix toolchain | Rust 1.97.0 (LLVM 22.1.6); opt 3, thin LTO, codegen units 1, strip debuginfo; system allocator |
| Host | Linux x86_64, AMD EPYC 9V74; available affinity CPUs 0–8 |
| rTorrent reference | rTorrent/libtorrent 0.16.25; pinned SHAs and build flags in [build info](TorrentNG-native-cpu-ram-20261022/rtorrent-build-info.txt) |
| Current probe artifact SHA-256 | `0182e232bc439f37471a9569832111e52ae73846e7e8dfcbdd1378d3381e4f21` |
| Current probe-test artifact SHA-256 | `a2f81dd199a9bc730f8973cda5a42a97f6733e4caeb585bc27c51430eb2ccc6a` |
| Current probe tests | 16 passed |
| Probe and recipe | [probe script](../../scripts/native_seed_cpu_ram_probe.py); [probe tests](../../scripts/test_native_seed_cpu_ram_probe.py); [benchmark recipe](../benchmarks/rtorrent-current/Dockerfile) |

The matrix binary identities are recorded in the original summaries and [Rust 1.97 build metadata](TorrentNG-native-cpu-ram-20261022/build-metadata-197.txt): baseline `a2812d73b9b70e562ceb4dfeb5089aeeeffe558a775ff58ed40ff221cefc0e7d`; verify-only `94a24269b7aefb191be39d7d955c9e25cf93bdacd3914bd6d36756569a356cd0`; rTorrent `b7828655feb7fa0b15d6a091d2bdbb0656be88a010d0c59d275d086ef40c61a8`. The verify-only and one-worker rows use the same binary; the latter sets `TOKIO_WORKER_THREADS=1`. The 1.99.1 host environment capture applies to the earlier 128 MiB run; the matrix compiler is documented separately in build metadata.

The hashes above identify current artifacts, not the historical probe revisions used for the 128 MiB, matrix, or recovery measurements. Those runner revisions used the same CPU/RAM sampling mechanics but differ from the current revision in recovery, payload-cleanup, and environment-guard handling. The run summaries preserve per-run settings; they do not record historical runner-file hashes.

## Preliminary 128 MiB comparison

The `screen128-1` comparison used a 128 MiB fixture, 32 seconds of payload at 4 MiB/s, a 32-request window, loopback, and no DHT, PEX, or tracker. All six transfers completed and verified 134,217,728 bytes. `combined batch + dense` identifies a binary containing both changes; its results do not isolate the verify-only patch. The batch-release variant is exploratory and was removed from the working patch.

| Variant | N | CPU seconds, median (range) | CPU seconds/GiB, median (range) | Peak RSS MiB, median (range) | Peak PSS MiB, median (range) | Peak anonymous MiB, median (range) |
|---|---:|---:|---:|---:|---:|---:|
| baseline | 2 | 1.15 (1.12–1.18) | 9.20 (8.96–9.44) | 26.54 (26.52–26.57) | 25.83 (25.81–25.84) | 8.79 (8.67–8.91) |
| combined batch + dense | 2 | 1.235 (1.19–1.28) | 9.88 (9.52–10.24) | 24.74 (24.38–25.11) | 24.02 (23.66–24.39) | 6.34 (6.01–6.66) |
| batch release experiment | 2 | 1.245 (1.15–1.34) | 9.96 (9.20–10.72) | 27.43 (26.80–28.05) | 26.71 (26.09–27.33) | 8.64 (8.36–8.92) |

The combined variant's CPU range exceeded baseline; its lower observed memory does not identify which of its two changes caused the difference. The grouped upload-release experiment did not establish CPU benefit and was removed from the working patch. Its saved [comparison diff](TorrentNG-native-cpu-ram-20261022/batch-permits-complete.patch) is in the evidence folder. These two repeats do not establish general CPU or RAM improvement.

## 256 MiB matrix: 11 verified cells, one failed cell

The intended 12-cell matrix was baseline, verify-only, verify-only with one Tokio worker, and rTorrent, each for three randomized rounds. Fixture size was 256 MiB; each completed transfer verified 268,435,456 bytes at 4 MiB/s with a 32-request window. The original summary is **failed**, not complete: 11 cells completed and verified; the round 3 rTorrent cell encountered `ENOSPC` during the final run and is excluded. Thus rTorrent has two paired-matrix samples, while each TorrentNG variant has three.

| Variant | N verified | CPU seconds, median (range) | CPU seconds/GiB, median (range) | Peak RSS MiB, median (range) | Peak PSS MiB, median (range) | Peak anonymous MiB, median (range) |
|---|---:|---:|---:|---:|---:|---:|
| baseline-197 | 3 | 2.37 (2.33–2.42) | 9.48 (9.32–9.68) | 27.72 (27.48–28.07) | 27.01 (26.76–27.36) | 9.19 (8.94–9.52) |
| verify-197 | 3 | 2.47 (2.10–2.49) | 9.88 (8.40–9.96) | 25.24 (24.92–25.29) | 24.52 (24.20–24.56) | 6.75 (6.73–7.00) |
| verify-oneworker-197 | 3 | 2.04 (1.91–2.05) | 8.16 (7.64–8.20) | 24.48 (24.27–25.02) | 23.76 (23.56–24.31) | 6.07 (6.07–7.14) |
| rTorrent matrix cells | 2 | 0.69 (0.67–0.71) | 2.76 (2.68–2.84) | 12.48 (12.42–12.54) | 10.73 (10.63–10.83) | 2.63 (2.63–2.63) |

The verify-only change showed lower median PSS and anonymous memory than baseline in these runs, while its CPU median was slightly higher. The one-worker variant had lower medians for CPU and memory than baseline in this matrix; this is one host, one fixture, and three samples per TorrentNG variant. The matrix is incomplete, and these results do not establish CPU or RAM parity.

### Separate rTorrent recovery sample

After the final-cell disk-space failure, rTorrent was rerun separately in single-client mode. The 256 MiB payload verified. This is a single recovery sample, not a third paired-matrix observation:

| Sample | N | CPU seconds | CPU seconds/GiB | Peak RSS MiB | Peak PSS MiB | Peak anonymous MiB |
|---|---:|---:|---:|---:|---:|---:|
| rTorrent recovery | 1 | 0.61 | 2.44 | 12.40 | 10.68 | 2.64 |

The two matrix rTorrent rows and this recovery row use the same recorded binary SHA-256, but the recovery run remains separately identified because its run mode and pairing context differ.

## Retained change, checks, and exploratory batch work

The retained source diff is a 43-line verify-only change in [`crates/rt-storage/src/verify.rs`](../../crates/rt-storage/src/verify.rs), saved exactly as [retained verify-only patch](TorrentNG-native-cpu-ram-20261022/retained-verify-only.patch). For a request covered by one dense extent it returns the owned read and avoids allocating and copying a second full-range buffer; sparse and multi-extent reads retain their existing path. This change was tested separately in the 256 MiB matrix. Its patch SHA-256 is `eebbcaa4010b7bac4e4f0ee55871505e3d3843384c47634dbb613cc44c9d5c26`.

Recorded checks: 31 `rt-storage` tests passed; Clippy passed for `rt-storage --lib --tests` with the retained change. The rejected grouped-release experiment passed 12 scheduler tests, 53 upload tests, 2 remote-cancel tests, 1 disconnect test, and 2 batched-piece-writer tests; Clippy passed for `rt-engine --lib`. The grouped-release source change was not retained.

## Profile, cache, and mapping observations

A preliminary whole-process gperftools profile collected 235 samples across daemon startup, hashing/recheck, and seeding. It is not a transfer-only profile and cannot support an under-load CPU attribution. One captured `/metrics` snapshot showed 512 KiB of peer-read cache, 634 hits, 77 misses, and no evictions. The endpoint returns the last cached engine snapshot and schedules an asynchronous refresh, so these are snapshot values, not necessarily current-at-scrape values or full-transfer totals. The configured 16 MiB budget is not evidence of 16 MiB resident or used.

Completed TNG post-transfer `smaps` snapshots from the matrix show approximately 15.1–15.9 MiB PSS in the executable mapping and 2.4–2.6 MiB in read-only file mappings, alongside roughly 5.4–8.9 MiB in anonymous mappings. The largest mapping is the `r-xp` `torrentngd` executable. The rTorrent summary has RSS/PSS/anonymous process values but no saved `smaps`, so its file-backed versus anonymous mapping categories are unknown. The available data shows TNG's mapped image is a substantial part of its own PSS; it cannot show how much of the cross-client gap is due to static image pages.

The following are per-process diagnostic snapshots (median and range across the three corresponding TNG runs), not phase peak values. Entries are MiB; each field is shown as median (min–max).

| Snapshot | Variant | RSS | PSS | Anonymous |
|---|---|---:|---:|---:|
| Before torrent add | baseline-197 | 18.93 (18.74–18.98) | 18.23 (18.03–18.29) | 3.23 (3.23–3.23) |
| Before torrent add | verify-197 | 18.72 (18.68–18.86) | 18.03 (17.99–18.17) | 3.22 (3.22–3.23) |
| Before torrent add | verify-oneworker-197 | 18.73 (17.83–18.94) | 18.02 (17.13–18.24) | 3.20 (3.20–3.20) |
| After seed recheck | baseline-197 | 27.22 (27.22–27.27) | 26.50 (26.50–26.55) | 8.91 (8.84–8.92) |
| After seed recheck | verify-197 | 24.00 (23.87–24.27) | 23.28 (23.15–23.55) | 5.94 (5.91–5.94) |
| After seed recheck | verify-oneworker-197 | 23.74 (23.38–24.06) | 23.02 (22.66–23.34) | 5.76 (5.76–5.76) |
| After transfer | baseline-197 | 27.72 (27.18–28.07) | 27.00 (26.46–27.35) | 9.19 (8.94–9.52) |
| After transfer | verify-197 | 25.24 (24.92–25.29) | 24.52 (24.19–24.56) | 6.75 (6.73–7.00) |
| After transfer | verify-oneworker-197 | 24.27 (23.87–25.02) | 23.55 (23.14–24.29) | 6.07 (6.07–7.14) |

The matrix does not save rTorrent `smaps` snapshots, so these diagnostic memory tables are TNG-only. They are point samples and should not be read as the active-interval peaks reported in the 256 MiB comparison table.

### Separate thread CPU diagnostic

One additional 256 MiB verify-oneworker diagnostic run used per-thread CPU sampling. It is separate from the failed 11-cell matrix and is not a clean performance comparison. The run verified the payload. During the first-payload-to-complete interval, the process used 2.01 CPU seconds; per-thread counters attributed 1.39 seconds to the Tokio worker, 0.31 seconds to each of the two disk workers, and 0.00 tick seconds to the hash thread. The matched-thread sum was 2.02 seconds, a 0.01-second difference from process ticks. The `+8s` metrics snapshot showed a zero cache gauge, while the post-transfer snapshot showed 641 hits and 74 misses. Those metrics came from the last cached engine snapshot with asynchronous refresh; review indicates the post-transfer values reflect about 34 MiB and 8 seconds of activity, not the full 256 MiB transfer. Neither snapshot proves the cache was empty at `+8s` or that 641/74 are whole-transfer totals. These are one-run diagnostic counters, not a general CPU profile. The [thread diagnostic summary](TorrentNG-native-cpu-ram-20261022/thread-cpu-verify197-1worker-256-summary.json) and [thread sampler](TorrentNG-native-cpu-ram-20261022/proc_thread_cpu_probe.py) are preserved separately.

## Harness validity and limits

The harness samples the host-visible process PID and records `payload_cpu_seconds_user_system`, RSS, PSS, anonymous memory, and authorized TorrentNG metrics. Child processes strip inherited loader, profiler, allocator, logging, Tokio, TNG, and TorrentNG environment controls; the harness records safe control names and values, then applies explicit client overrides. It does not record full environment or token values. It paces the first payload and captures first-payload and completion boundaries. The pinned rTorrent seed-side probe sends an explicit empty Bitfield before Interested; an earlier stalled smoke omitted that message and did not produce an UNCHOKE, so that run is excluded from parity evidence. The 128 MiB summary and failed 256 MiB summary preserve every completed sample; the rTorrent recovery summary is kept separate.

After a verified transfer and client shutdown, the harness removes only the generated per-run seed and downloaded payload copies. `--keep-payloads` retains them; failed or unverified runs retain generated payloads for diagnosis. The fixture source is not removed.

CPU is quantized to 0.01 seconds from 100 Hz process ticks. Memory figures are sampled phase peaks, not whole-machine RAM. The 256 MiB matrix's ENOSPC failure, two rather than three paired rTorrent samples, single-host scope, and one-worker setting limit interpretation. No deployment recommendation is made.

## Bounded evidence

The evidence folder contains the run summaries, separate recovery and thread-diagnostic summaries, build and host metadata, pinned rTorrent build information, the exact verify-only and grouped-release patches, and the thread sampler. It excludes fixture payloads, run logs, binaries, and profiler files.

- [128 MiB summary](TorrentNG-native-cpu-ram-20261022/screen128-1-summary.json)
- [Failed 256 MiB matrix summary](TorrentNG-native-cpu-ram-20261022/screen256-rust197-1-summary.json)
- [Separate rTorrent recovery summary](TorrentNG-native-cpu-ram-20261022/rtorrent-recovery-rust197-final-summary.json)
- [Rust 1.97 build metadata](TorrentNG-native-cpu-ram-20261022/build-metadata-197.txt)
- [Host environment capture for the 1.99.1 preliminary run](TorrentNG-native-cpu-ram-20261022/host-environment-screen128-rust199.json)
- [Pinned rTorrent build metadata](TorrentNG-native-cpu-ram-20261022/rtorrent-build-info.txt)
- [Thread diagnostic summary](TorrentNG-native-cpu-ram-20261022/thread-cpu-verify197-1worker-256-summary.json)
- [Thread sampler](TorrentNG-native-cpu-ram-20261022/proc_thread_cpu_probe.py)
- [Grouped-release comparison patch](TorrentNG-native-cpu-ram-20261022/batch-permits-complete.patch)
