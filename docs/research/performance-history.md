# Performance history from the project README

This is the dated performance and optimization history formerly embedded in the root README. Treat each result according to its source, fixture, run count, memory category, and limitations. The current all-client screen is in [Current client comparison](current-client-comparison.md); the research index links the exact reports and candidate configurations.

## Before using a real library

TorrentNG is pre-1.0. Settings and APIs may change. Back up your client state
and downloaded files before importing, moving, or deleting data. Migration
starts with a report; review it before using `--apply`. Some imported progress
may need a full data check before another client can resume it.

The dated [Debian ISO comparison and native memory investigation](../../certification/reports/native-memory-optimization-20261014-agentvm.md)
records public download, controlled seeding, and anonymous memory against
qBittorrent, Transmission, Deluge, and rTorrent. TorrentNG averaged 0.49 MiB/s
on its one public-swarm pass, slower than the completed peers at 5.60–16.82
MiB/s; in the controlled seed test it averaged 3.56 MiB/s, close to their
3.61–3.74 MiB/s. Its measured anonymous peaks were 8.3 MiB in public download
and 7.4 MiB while seeding. That was about 2.1–3.2 times less than qBittorrent
and Transmission, and 11–12 times less than Deluge. The peer ratios combine
sequential runs, with Transmission and Deluge measured in a later rerun; they
are not a general ranking. A separate system-allocator run used less memory
than the previous jemalloc run (18.8 MiB public / 25.3 MiB seed), while seed
rates were 3.56 vs 3.66 MiB/s. This allocator comparison is not a concurrent
A/B. A mimalloc follow-up used more anonymous memory (19.8 MiB public / 23.6
MiB seed) and did not complete the public download within 30 minutes. A
qBittorrent check right afterward completed in 39.6 seconds at 19.08 MiB/s,
which flags a current TorrentNG download gap; sequential testing leaves the
precise cause unresolved. The results do not show orders-of-magnitude savings
against every peer; rTorrent did not start, and public swarm speeds varied
sharply. The earlier
[unoptimized baseline](../../certification/reports/client-performance-benchmark-20261013.md)
is retained for reference. Neither report establishes a maximum torrent count.
See the dated [release evidence](../../docs/RELEASE_EVIDENCE.md) for what was
exercised and where.

A corrected comparison on clean main commit `cfb63f91` completed both
controlled download and seed transfers for TorrentNG, qBittorrent, Transmission,
Deluge, and rTorrent, with SHA-256 verification. In this one-pass baseline,
TorrentNG's download used 6.504 CPU-s/GiB and 6.3 MiB anonymous memory, versus
8.218 CPU-s/GiB and 16.6 MiB for qBittorrent; TorrentNG transferred more slowly
(3.07 versus 3.83 MiB/s). On the seed workload, TorrentNG used 1.33 CPU-s/GiB
and 16.6 MiB anonymous memory: lower CPU than qBittorrent and Deluge, higher
than Transmission and rTorrent, and higher memory than qBittorrent and
rTorrent. These are single-pass, shared-host observations, not a client
ranking. The [corrected comparison and raw evidence](../../certification/reports/TorrentNG-native-download-efa3a607/comparison/round2-corrected-cfb63f91/README.md)
is the pre-change baseline for the seed-upload optimization.

An exploratory Linux TCP file-range seed candidate recorded 4.3 MiB sampled
anonymous peak versus 16.6 MiB in the clean-source baseline (74% lower), while
measured CPU was 5.51 versus 1.33 CPU-s/GiB (4.14 times higher). Transfer rate
stayed near 3.9 MiB/s under the shared 4 MiB/s receiver cap. The candidate was reverted
because the CPU cost missed our goal; the memory gain and CPU loss are both
preserved as design evidence for later work. These were single-pass observations
on a shared host, not a causal estimate or a client ranking. See the
[candidate results and raw evidence](../../certification/reports/TorrentNG-native-download-efa3a607/comparison/round3-sendfile-e9727943/README.md)
and the [three-round whole-client seed comparison](../../certification/reports/TorrentNG-whole-client-comparison-20261021/README.md).
The next review archive repeated the `sendfile` recommendation and added no new
patch. Its source and older benchmark snapshots are recorded in the
[archive assessment](../../certification/reports/TorrentNG-download-review-20261021/README.md).

On 2026-10-22, a private local screen against source-built rTorrent/libtorrent
v0.16.25 measured active-window CPU at 1.28 CPU-s/GiB for TorrentNG and 0.50
for rTorrent, with active goodput of 3.53 and 3.85 MiB/s. Anonymous seed-memory
peaks were 14.5 MiB and 3.4 MiB. rTorrent's first payload appeared about 63
seconds after its peer connected; the run did not record the peer's interest,
choke, or request state. This was one exploratory pair, not a ranking. See the
[current-source screen report](../../certification/reports/TorrentNG-current-rtorrent-screen-20261022.md).

A separate hot-file probe found that a single mapped view, read in 16 KiB
blocks, used 56% less CPU per GiB than a reused-buffer `pread` path across three
paired samples, but the absolute saving was 0.060 CPU-s/GiB. That is about 8%
of the 0.78 CPU-s/GiB gap in the separate screen, only a magnitude comparison.
The probe excludes scheduler, socket, and cold-file costs, so mapped reads
alone will not close the gap. A prototype is worth pursuing only if it can also
remove repeated scheduler or buffer-ownership work; the [method and raw
measurements](../../certification/reports/TorrentNG-seed-read-path-probe-20261022.md)
are recorded separately.

The final compact-default comparison reported three-run medians of 1.90 CPU
seconds and 19.82 MiB PSS for TorrentNG, versus 0.73 seconds and 10.74 MiB for
rTorrent. PSS improved 18.3% against TorrentNG's previous best configuration;
CPU did not improve, and remains about 2.60× rTorrent's. The final summary and
its evidence limits are in the [CPU/RAM results note](../../certification/reports/TorrentNG-CPU-RAM-final-summary-20261022.md),
with the [full report and verified run summaries](../../certification/reports/TorrentNG-native-cpu-ram-20261022.md).
Follow-up matched local screens found a release-default improvement: increasing
the upload-read coalescing window from 8 ms to 16 ms reduced median CPU by
10.6% and also reduced PSS and anonymous memory on the 128 MiB release fixture.
The paired 64 MiB release screen moved in the same direction. The matching PGO
128 MiB screen reduced median CPU by 2.6%, PSS by 2.8%, and anonymous memory by
5.5%; its 64 MiB screen had a 5.5% anonymous-memory increase, which is recorded
alongside the gains. The 16 ms setting was retained at that stage; the exact 8 ms baseline, 16 ms
candidate, raw summaries, logs, and diagnostics remain in the [seed performance
experiments report](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/README.md).
It was later superseded by the measured 32 ms/eight-block default described below.

A further measured source improvement is now retained on main as commit `39554462`: upload reads collect for up to 32 ms and combine up to eight contiguous 16 KiB blocks. In three randomized 64 MiB release rounds, CPU fell 22.2% (4.504 to 3.506 CPU-s/GiB), PSS fell 1.5% (18.875 to 18.586 MiB), and anonymous peak fell 7.5% (6.500 to 6.012 MiB), with every transfer SHA-256 verified. Fresh PGO screens improved on matched PGO controls at 64 MiB (3.016 vs 3.481 CPU-s/GiB; 15.791 vs 16.037 MiB PSS) and 128 MiB (3.348 vs 4.019; 15.593 vs 16.184 MiB PSS). PGO remains opt-in, and at 128 MiB the candidate still used about 1.42× rTorrent CPU and 2.05× its PSS. Sixteen-block experiments did not improve CPU and PSS together; all failed and losing patches and measurements are preserved. The detailed method, matched results, optional allocator tradeoff, and exact artifacts are in the [seed experiment report](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/README.md#current-main-improvement-bounded-eight-block-reads) and [evidence directory](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/seed-goal-next-avenues-20261022/README.md).
The 12-block compromise passed the engine suite and a three-peer wire check, but a single 64 MiB screen used 1.9% more CPU and 2.4% more PSS than eight-block batching while the host was loaded. It remains an archived candidate, not a default; its exact patch, binary, and screen are in the [12-block evidence folder](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/batch12-screen-20261022/README.md).
Removing the explicit TCP preflush before Piece writes also passed the engine tests and a new wire-order test. Its first screen looked 21.8% faster, but randomized confirmation was tied and then worse on CPU/PSS under high host load, so it was not promoted; the [patch and completed results](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/framed-preflush-screen-20261022/README.md) are preserved.
A current-source CPU profile found no dominant TorrentNG leaf (332 ms of samples; 52% was in libc). A codec refactor removing `ResourceGovernor` reference-count clones passed its focused tests, but a noisy first matched screen used 17.3% more CPU and 0.11 MiB more PSS than the eight-block control. It was not promoted; the [profile and candidate evidence](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/current-eight-block-profile-20261022/README.md) are preserved.
A narrower codec guard that skipped already-covered read-buffer admission also passed peer-wire tests but did not show a repeatable whole-client gain. Three randomized 64 MiB rounds measured median CPU 44.5% higher, PSS 0.8% higher, and anonymous memory 5.2% higher; the host was heavily loaded and the CPU result varied sharply by round. It remains an off-main candidate with its exact patch and evidence archived in the [capacity-guard screen](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/peer-codec-capacity-fastpath-screen-20261023/README.md).
A current-thread Tokio runtime also passed its flavor and override tests, but one 64 MiB screen used 8.1% more CPU and 0.32 MiB more PSS than the current one-worker multithread runtime. It was not promoted; the [patch and run evidence](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/current-thread-runtime-screen-20261022/README.md) are preserved.
Three-peer arena-cap follow-up screens show repeatable memory reductions but no dependable CPU win: cap 2's three-round median cut PSS 3.2% and anonymous memory 11.7%, while CPU moved both ways. Arena caps remain optional; the [matched results](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/arena-cap-concurrent-followup-20261023/README.md) are preserved.
The active-stream cache refactor was re-screened on the current eight-block source. Its CPU result changed direction with run order, while PSS and anonymous memory were higher in both screens; it remains archived and was not promoted. See the [current-source results](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/active-stream-current-main-screen-20261023/README.md).
An earlier `PeerReadStreams` current-entry variant saved 4.4% CPU on a 16 MiB screen, then used 13.0% more CPU at 64 MiB, with memory effectively tied. It remains a size-dependent candidate, not a production change; see the [two-size results](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/peer-read-current-entry-screen-20261023/README.md).
Reducing peer read-ahead from 512 KiB to 128 or 256 KiB did not improve CPU and memory together. A two-round confirmation was slower on CPU, with slightly higher median PSS; the [window-sweep data](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/readahead-window-screen-20261023/README.md) keeps the exact configs and results.
An opt-in `storage.io_worker_threads = 1` screen improved single-peer CPU and memory, but the randomized three-peer confirmation used 12–19% more CPU in every round while reducing PSS 4.5–5.6% and anonymous memory 13.4–18.0%. Two lazy second-worker prototypes also failed to start worker two in the three-peer screen; the refined candidate was near-tied on CPU and slightly worse on PSS. Keep the default at two; see the [worker-count](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/io-worker-count-screen-20261023/README.md) and [lazy-trigger](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/lazy-worker-trigger-screen-20261023/README.md) evidence.
The same release binary with `MALLOC_ARENA_MAX=1` reduced median CPU/PSS/anonymous memory by 19.0%/8.0%/22.8% at 64 MiB and 6.7%/9.6%/27.2% at 128 MiB in randomized three-round single-peer screens. In a separate randomized three-peer, three-round follow-up, cap 1 used 9.6–40.3% more CPU every round (median +40.3%) while lowering median PSS 7.9% and anonymous memory 27.9%. It remains an optional memory-for-CPU tradeoff, not a general improvement; see the [concurrent results](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/arena-cap-concurrent-followup-20261023/README.md).
The bounded mmap-window plus vectored-write probe showed mixed CPU results and no process-memory measurement. It is not a whole-client result and did not justify production mmap ownership; the [C probe and counter-balanced results](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/mapped-window-writev-probe-20261023/README.md) preserve the experiment.
Adding `biased;` to the peer-loop `tokio::select!` gave inconsistent CPU results: an alternating five-pair confirmation used 13.7% more median CPU, despite slightly lower memory. It is archived without changing production behavior; see the [per-run evidence](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/peer-select-biased-screen-20261023/README.md).
Current-main debug telemetry from a verified 16 MiB transfer showed 127 of 129 storage-read batches already contain eight blocks, with one initial singleton and one seven-block tail. This diagnostic has no performance score; it points the next CPU work toward per-block cost inside the full batches, not a longer collection wait. See the [batch counts](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/batch-distribution-current-main-20261023/README.md).

An Oct 24 TNG-only 128 MiB diagnostic added a post-transfer `/metrics` snapshot.
It counted 8,192 logical 16 KiB peer-read requests, 7,086 backend reads totaling
149,376,320 bytes, 1,106 peer-read cache hits, four prepared worker-job
allocations and 8,316 reuses; reported prepared-job queue wait/run totals were
94.8/199.4 ms. These are logical request and scheduler counters, not a batch
size distribution. The shared host was heavily loaded (load average 31.5 at
start), so the run is not a performance comparison. See the [counter report and
raw samples](../../certification/reports/TorrentNG-storage-worker-counter-confirmation-20261023/README.md)
and [baseline run](../../certification/reports/TorrentNG-storage-worker-baseline-20261023/README.md).

A min-two-request, 8 ms coalescing prototype passed its focused peer-loop test,
but the optimized image build was interrupted before any transfer. It was not
retained after the independent batch trace showed 127/129 batches already at the
eight-block cap. No CPU, RAM, or throughput effect was measured. Its unscored
[run record](../../certification/reports/TorrentNG-upload-coalesce-candidate-20261023.md)
and [exact patch](../../certification/reports/TorrentNG-upload-coalesce-candidate-20261023.patch)
are preserved. The scheduler's per-batch worker/cache-hit path remains a more
useful next target; any direct page-cache read experiment needs a nonblocking
probe and the existing worker fallback.

The earlier matched 128 MiB comparison left the 16 ms PGO candidate at about
1.43× rTorrent 0.16.25 CPU, 2.05× its PSS, and 2.04× its anonymous memory. The
newer 32 ms/eight-block PGO screen reports 1.42× rTorrent CPU and 2.05× its PSS;
its 15.59 MiB PSS is the lowest recorded 128 MiB PGO median in these local
screens. Earlier full-profile PGO evidence remains archived for comparison.
PGO is still opt-in. The separate io_uring screen confirmed the requested
backend but measured about 13% more CPU than `pread` for roughly 2.5% lower PSS;
that tradeoff and the worker-startup correction are documented in the same
report. Fresh PGO profiles trained from the current source lowered CPU in the
transfer-window screen, but raised PSS by 11.2%; full-process training also
lost in its one-pair screen. Both candidates and the exact evidence are
preserved without changing the release or PGO defaults.
The follow-up upload-rate admission batching refactor also failed to beat the
16 ms release on CPU and PSS together in two randomized 64 MiB screens. Its
patch and verified measurements are retained in the same report for later
profiling.
Six one-round 64 MiB screens tested jemalloc, allocator refinements,
`target-cpu=native`, Rust 1.97, and `opt-level=3`. None improved CPU and PSS
together, so the current system allocator, portable target, and compact
release profile remain in place. The exact builds, verified summaries, and
diagnostic profile are in the
[low-risk tuning screen](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/low-risk-tuning-screen-20261022/README.md).
The separate [mapped-window probe](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/mapped-window-read-probe-20261022/README.md)
found an 8.6% read-stage CPU reduction against batched `pread`, but no whole-client result.
I also rebuilt PGO with a profile trained for `target-cpu=native` to avoid the
control-flow mismatch in the earlier portable-profile attempt. In one matched
64 MiB screen, it used 7.6% more CPU and 11.3% more PSS than the retained PGO
control, though anonymous memory was 5.2% lower. All transfers verified, but
the host was busy and this one-round result is only screening evidence. The
candidate was not promoted; its exact profile, binaries, and run data are
preserved in the [native-target PGO screen](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/pgo-native-target-screen-20261022/README.md).
An equal-weight blend of the current-source full-process and transfer-window
profiles looked promising in one pass, then lost its three-round screen: CPU
was 5.9% higher and PSS 9.3% higher than the PGO control, though anonymous
memory was 5.2% lower. The initial and repeated results, binary, and merged
profile are preserved in the [PGO blend screen](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/pgo-profile-blend-20261022/README.md).
Fat-LTO PGO made the executable smaller but raised CPU by 7.7% and PSS by
15.0% versus the PGO control in its verified screen. A three-round
`MALLOC_ARENA_MAX=1` run lowered PSS by 7.4% and anonymous memory by 19.5%,
with 5.4% more CPU; this remains an optional memory tradeoff, not a new default.
The one-round arena-cap-2 screen raised CPU by 42.3%. Exact results and
reproducible artifacts are in the
[runtime and codegen screen](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/runtime-tuning-screen-20261022/README.md).
An exact-binary 256 MiB CPU profile showed broad peer/task polling among the
largest resolved TorrentNG frames, but 47.1% of its sample weight mapped to
libc without reliable symbol attribution. It is diagnostic-only and does not
support the peer-read map refactor as a large target. The run changed no source
or defaults; limits and the raw profile are in the
[hot-path profile note](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/observations/pgo-hotpath-profile-20261022/README.md).

## Latest controlled-seed comparison (2026-10-23)

Three randomized rounds compared the current native engine with qBittorrent
5.3.0, Transmission 4.1.3, Deluge 2.2.0, and source-built rTorrent 0.16.26.
All 15 local 64 MiB transfers passed SHA-256 verification.

| Client | CPU (s/GiB) ↓ | Peak cgroup anonymous (MiB) ↓ |
|---|---:|---:|
| TorrentNG | 2.94 | 7.6 |
| qBittorrent | 2.65 | 6.4 |
| Transmission | 1.92 | 15.1 |
| Deluge | 3.05 | 80.7 |
| rTorrent | 1.01 | 3.1 |

TorrentNG remains above rTorrent by 2.9× in whole-window CPU and 2.45× in
anonymous memory; active-window CPU was 4.1× higher. Its process-tree PSS was
21.0 MiB versus rTorrent's 19.4 MiB. TorrentNG used less anonymous memory than
Transmission and Deluge, but slightly more than qBittorrent. The desktop was
under sustained I/O pressure, and TorrentNG's active goodput was lower than
the other clients, so these are screening results rather than a parity claim.
The [full comparison report](../../certification/reports/TorrentNG-current-client-seed-comparison-20261023/README.md)
includes current version pins, the methodology, separate memory categories,
all three raw reports and profiles, host pressure, and excluded attempts.
