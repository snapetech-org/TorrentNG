# TorrentNG deep performance audit: 0f726a89

The original audit package is retained at
[`certification/reports/TorrentNG-deep-audit-0f726a89.zip`](../certification/reports/TorrentNG-deep-audit-0f726a89.zip)
and unpacked beside it. The review, implementation handoff, source manifest,
reproducer, mechanism benchmark, raw results, and reviewed source copies are in
[`certification/reports/TorrentNG-deep-audit-0f726a89/`](../certification/reports/TorrentNG-deep-audit-0f726a89/).
The copied code and measurements describe the reviewed `0f726a89` source; they
are evidence for the finding, not evidence for later code.

## Findings and disposition

### P0: stale rate demand can block live peers

The audit compiled its reproduction against the production `RateTree` and
observed zero grants in 1,000 attempts while an inactive peer remained
registered. Removing that peer restored the grants, but would also discard
live identity and outstanding-grant accounting, so it is not a valid repair.
The archived run also passed the 19 then-existing `RateTree` tests. This is a
confirmed scheduler liveness defect; its contribution to any particular
benchmark remains unmeasured.

The current fix gives pending demand an owner separate from peer registration
and granted bytes. Synchronous peer refills have one retireable demand per
peer; asynchronous admissions have distinct demand IDs, so cancellation of
one request preserves its siblings. V1, pure-v2, and webseed paths retire
demand on choke, unavailable work, completion, cancellation, or teardown.
Timer waits retain real refill deadlines. Fairness-only waits use a
per-torrent progress signal and do not schedule a 1 ns retry. A live peer and
its unsettled grants remain registered when its demand is retired.

Requestability changes also retry parked demand. The v1 actor keeps a sparse
set of rate-waiting peers and revisits only those peers when availability,
request ownership, or piece completion changes. Pure-v2 peers have individual
retry signals; a requestability epoch check closes the race where a picker or
ledger change lands just before a peer registers as waiting. The v2 webseed
actor uses the same event-driven wake path, so it does not wait for its
60-second idle retry after a useful block becomes unavailable.

Regression sources cover the reported live-peer case, demand retirement after
another peer consumes the last available piece, targeted v2 wakeups,
event-only waits, independent asynchronous demands, and mixed timer/event wait
indexing. They pass in the 753-test `rt-engine` suite and in the full workspace
test pass. The archived standalone reproduction also still reports zero
grants in 1,000 attempts while demand is stale and 1,000 grants after removing
the peer. The targeted private two-seeder download fixture from the handoff
remains required before attributing the public-download plateau to this defect.

### Prior CPU and RAM evidence in the archive

The archived five-client controlled seed run used a dirty candidate tree based
on `8bd30a474b72309a4b95327f32bbd89caeb590dc`, not current `main`. It measured
TorrentNG at **3.97 CPU-s/GiB**, compared with qBittorrent 2.29, Transmission
1.41, Deluge 2.78, and rTorrent 0.76. TorrentNG used about **2.8×
Transmission's CPU per GiB**. Seed goodput was receiver-capped at about 3.8
MiB/s for all clients, so the result compares CPU under equal delivered work.

The same one-pass report measured cgroup anonymous peak RAM during seeding at
**16.3 MiB for TorrentNG**, 17.6 MiB for qBittorrent, 24.9 MiB for
Transmission, 83.7 MiB for Deluge, and 1.7 MiB for rTorrent. That is
encouraging against three clients and substantially above rTorrent. The
TorrentNG cache-disabled probe reported 3.65 CPU-s/GiB and 7.4 MiB anonymous
peak in a separate run; it is not a matched cross-client comparison. The
candidate's public download phase timed out at 5.87%, so it does not rank
download performance.

All clients passed controlled seed hash checks. The runs were one pass each on
a shared host with a sampled one-minute load peak of 26.18. These are useful
diagnostic results, not repeated or current-source rankings. See the archived
[five-client report](../certification/reports/TorrentNG-deep-audit-0f726a89/evidence/client-performance-benchmark-hotpath-candidate-full-20261020.md)
and [cache-disabled probe](../certification/reports/TorrentNG-deep-audit-0f726a89/evidence/client-performance-benchmark-hotpath-cache-disabled-torrentng-20261020.md)
for the trial setup, source-tree identity, time series, and limits.

### P1: storage dispatch has repeated handoffs

The audit measured 2.02 CPU-s/GiB for its synthetic two-round-trip mechanism
and 0.124 CPU-s/GiB for inline execution. These numbers are mechanism costs,
not TorrentNG application scores. Source review confirms the ordinary read
miss path still prepares a descriptor/cache decision, returns to the caller,
then dispatches positioned I/O and returns its completion. Cache hits still
use the preparation worker.

This is the strongest active-CPU architecture candidate after the scheduler
repair. A fused dispatch must retain bounded admission, descriptor identity,
replacement fences, cancellation ownership, cache invalidation, and the
portable backend path.

The archive's three-repeat mechanism result was 2.016 CPU-s/GiB for two worker
round trips and 0.124 for inline execution. Re-running its included harness on
this checkout with rustc 1.94 measured medians of 0.588 (range 0.581–0.677)
and 0.106 (range 0.103–0.119), respectively. The different environment and
compiler make the exact values unsuitable for comparison; both runs are
synthetic queue costs, not TorrentNG application scores.

**Implementation update, 2026-10-20:** the portable `pread` path now submits
descriptor lookup, cache lookup, positioned read, and final path-identity
validation as one bounded disk-worker operation with one completion. The job
retains mount/device admission, queued-byte credit, the leased descriptor, and
the frame until the read finishes. Source regressions cover a read miss/cache
hit on this route, backend worker dispatch, and owner retention after caller
cancellation of started work.
These regressions passed in the 335-test `rt-storage` suite and in the full
workspace suites on Rust 1.94 and 1.88. The io_uring and injected-backend routes
remain unchanged pending backend-specific profiling. The portable route also
briefly holds device admission during a cache hit; its effect on queue fairness
is still unmeasured.

### Conditional io_uring work

The audit identifies fixed-file registration churn, allocation before
registered-buffer selection, and registered-buffer cache pinning. These can
only affect runs that actually selected io_uring. Record backend and fixed
buffer support in the diagnostic profile first; do not treat these as causes
for pread results. Any registration cache must retain the leased descriptor
and generation, not only a numeric file descriptor.

### RAM retention

The audit distinguishes per-scheduler read-cache budgets, backing allocations
kept alive by slices, and idle buffers retained by the shared frame pool.
Returning a cache buffer to a free list can keep memory resident. The current
worktree now adds a process-wide 16 MiB backing-allocation budget alongside the
existing per-scheduler payload limits. Cache entries and the first returned
slice share an owner that retains both the PeerBuffer lease and global budget
charge until the last alias is dropped. ResourceGovernor reclaim is scoped to
the governor that admitted each cache, and total-cap pressure can also trim
idle buffers from the shared frame pool. Trimming releases reusable Vec
allocations from the pool; the allocator may retain their pages, so this does
not promise an immediate RSS decrease. Regression coverage for these lifetimes,
pressure eviction, governor scope, and pool trimming passed on both supported
workspace toolchains. The one-peer measurement below did not trigger or qualify
cross-torrent memory reclamation. Future pressure runs should report
per-scheduler logical cache bytes, process-wide backing bytes, active and idle
frame bytes, cgroup anonymous memory, PSS, and process RSS separately because
some counters overlap.

### Integrated-candidate validation and CPU/RAM measurement (2026-10-21)

Both full Rust workspace all-target suites passed with 2,339 passed, 0 failed,
and 12 ignored tests across 37 summaries on Rust 1.94 and 1.88. Strict
workspace Clippy passed on both toolchains, and all 351 discovered Python
tests passed. A parallel Rust 1.94 run once hit an unrelated IPv6 DHT test
failure; that test passed alone, and the serialized full suites passed on both
toolchains.

One controlled seed pair compared clean `bf37a330` with the integrated dirty
worktree, using the same SHA-256-verified Debian ISO and one qBittorrent leecher
capped at 4 MiB/s. TorrentNG measured **3.29 CPU-s/GiB** on clean main and
**3.06 CPU-s/GiB** on the candidate (about 7% lower). Seed goodput was 3.82
MiB/s in both runs. Candidate peak cgroup anonymous memory was **25.0 MiB**,
up from **23.5 MiB**, while process-tree PSS rose from **38.7 to 43.1 MiB**.
The host was shared: its sampled one-minute load peak was 14.95 for the
baseline and 21.17 for the candidate. This is a CPU-positive hint, not a
confirmed win; memory did not improve and one run per source cannot separate
the change from run variation. The public phase timed out in both runs and is
excluded. The single-torrent workload also does not establish the cache cap's
effect under concurrent cross-torrent memory pressure.

Full reports, time-series samples, and memory profiles:
[clean-main baseline](../certification/reports/torrentng-deep-audit-baseline-20261021.md)
and [integrated candidate](../certification/reports/torrentng-deep-audit-candidate-20261021.md).

The next exact-main five-client pass built clean `457df452` and verified all
controlled seed payloads. TorrentNG used **2.66 CPU-s/GiB** and peaked at
**25.2 MiB** cgroup anonymous memory, compared with CPU/memory of 2.02/18.0
for qBittorrent, 1.33/24.8 for Transmission, 2.43/85.1 for Deluge, and
0.55/1.9 for rTorrent. The run used one receiver capped at 4 MiB/s and was a
single sequential pass on a shared host; it shows that CPU parity is still
unproven and memory is mixed, not a client ranking. The public phases for
TorrentNG, Transmission, and Deluge timed out and are excluded. The current
host also had no `perf`, `bpftrace`, `strace`, `samply`, or `valgrind` command
available, so this pass gives aggregate CPU without stack attribution. See
the [exact-main report](../certification/reports/client-performance-benchmark-457df452-20261021T0525-cpu-ram.md)
and its [samples](../certification/reports/client-performance-benchmark-457df452-20261021T0525-cpu-ram-samples.csv)
and [memory profile](../certification/reports/client-performance-benchmark-457df452-20261021T0525-cpu-ram-memory-profile.json).

A TorrentNG-only repeat on clean `8a27b32d` then verified the same payload at
3.82 MiB/s, with **2.57 CPU-s/GiB**, **23.3 MiB cgroup anonymous peak**,
**41.5 MiB process-tree PSS**, and **41.9 MiB process-tree RSS**. These single
observations are close to the preceding TorrentNG row, but the repeat has no
client comparator and ran in a different host-load window; it does not confirm
a CPU or RAM improvement. Its public probe timed out at 5.18% and is excluded.
The backend remains unknown: the preceding run's `/metrics` request failed
after container shutdown, and this run reached the service before shutdown but
after pausing TorrentNG, when the separate `StorageRuntime` gauge read zero.
That runtime is used for plan and import work; it does not own the live peer
transfer backend. The harness now captures before the final torrent pause, but
these older metrics did not expose active `MountScheduler` owners and cannot
recover either missed backend value.
See the [repeat report](../certification/reports/client-performance-backend-observation-8a27b32d-20261021.md),
[samples](../certification/reports/client-performance-backend-observation-8a27b32d-20261021-samples.csv),
and [memory profile](../certification/reports/client-performance-backend-observation-8a27b32d-20261021-memory-profile.json).

A second TorrentNG-only seed on source `3e7a15ed` also passed the hash at 3.82
MiB/s, measuring 2.73 CPU-s/GiB, 24.9 MiB anonymous peak, 43.4 MiB PSS, and
43.9 MiB RSS. Across both one-run samples, median CPU is 2.65 CPU-s/GiB and
median anonymous peak is 24.1 MiB; this is a small repeatability check without
a client comparator. Its metrics scrape ran before container shutdown but
after the torrent was paused, when `runtime_initialized=0`. That was not an
ordering failure: the metric observed the separate `StorageRuntime`, not the
live peer-transfer `MountScheduler`. The following 1a748ba3 repeat moved the
scrape before pause and still could not identify the backend for the same
reason. Current instrumentation now reports live scheduler groups and their
backend capabilities directly; a clean run is still needed to record the
selected backend on a new source revision.
See the [report](../certification/reports/client-performance-backend-observation-3e7a15ed-20261021-repeat.md),
[samples](../certification/reports/client-performance-backend-observation-3e7a15ed-20261021-repeat-samples.csv),
and [profile](../certification/reports/client-performance-backend-observation-3e7a15ed-20261021-repeat-memory-profile.json).

The [1a748ba3 follow-up](../certification/reports/client-performance-backend-confirm-1a748ba3-20261021.md)
captured before pausing and measured 2.47 CPU-s/GiB, 24.3 MiB cgroup
anonymous peak, 42.4 MiB PSS, and 42.8 MiB RSS. It passed the same hash at
3.82 MiB/s, but ran once on a shared host with a 1-minute load peak of 11.92
and has no comparator. Its `runtime_initialized=0` reading is only the legacy
global runtime gauge; the peer scheduler backend was not exported by that
source. See its [samples](../certification/reports/client-performance-backend-confirm-1a748ba3-20261021-samples.csv)
and [memory profile](../certification/reports/client-performance-backend-confirm-1a748ba3-20261021-memory-profile.json).

### Current-main CPU/RAM remeasurement (`f6cce154`, 2026-10-21)

A clean current-main image completed a controlled seed with all five clients;
each leecher passed the same ISO SHA-256 check. TorrentNG measured **2.46
CPU-s/GiB** and **25.2 MiB** peak cgroup anonymous memory. The corresponding
CPU/RAM figures were qBittorrent 2.04/18.0, Transmission 1.31/25.8, Deluge
2.27/89.1, and rTorrent 0.56/1.7. This keeps the audit's CPU concern open:
TorrentNG remained above every comparator. Its RAM result is mixed: below
Transmission and Deluge, above qBittorrent and rTorrent. The run was one
sequential pass on a shared host, so it does not establish a general ranking
or a change beyond run variation.

The live transfer used the portable `pread` backend. The fused portable read
path was exercised; the audit's conditional io_uring findings were not. The
process-wide cache budget and frame-pool trimming are present, but this
single-torrent run does not qualify cross-torrent memory reclamation. Public
swarm probes timed out for TorrentNG and Transmission and are excluded from
the comparison. A separate profile on source `c6984493` is now available;
randomized repeats and multi-torrent memory-pressure measurements remain
open. See the [full
report](../certification/reports/client-performance-benchmark-f6cce154-20261021-cpu-ram.md),
[samples](../certification/reports/client-performance-benchmark-f6cce154-20261021-cpu-ram-samples.csv),
and [memory profile](../certification/reports/client-performance-benchmark-f6cce154-20261021-cpu-ram-memory-profile.json).

### Diagnostic profile and profile-guided source candidate (`c6984493`, 2026-10-21)

An instrumented one-peer seed on `c6984493307a5fb01c9182067bdcbf2512af3f49`
passed SHA-256 with 3.82 MiB/s goodput. It recorded 2.77 CPU-s/GiB, 24.7 MiB
cgroup anonymous peak, and 43.0/42.6 MiB process-tree RSS/PSS. This is a
diagnostic run, not a clean score; the public probe timed out at 3.38% and is
excluded. The backend was portable `pread`.

`perf record -F 99 -e cycles:u --call-graph dwarf` collected 807 samples with
no losses. Self samples included 11.60% in `__unlock` on Tokio workers, 9.55%
in `memcpy`, 6.70% in `__libc_free`, 5.33% in the peer-loop closure, 3.17% in
`MountScheduler` destruction, 3.05% in Tokio timer-entry destruction, and
2.25% in `RateAdmissionGrant::settle`. The inclusive tree assigned 5.56% to
`start_upload_reads`, 3.17% to dropping `UploadReadContext`/`MountScheduler`,
and 1.91% to cloning `MountScheduler`. These sampled weights guide the next
experiment but do not establish that any one cost explains the client CPU gap.

The source candidate now creates one `UploadReadContext` per peer, shares it
among in-flight reads through `Arc`, and removes `Clone` from that context.
Before this change, every upload-queue refill created a context and cloned it
per scheduled request, even when the queue was already full. The per-read
timeout remains to preserve stalled-admission handling. The bulk correctness
pass and one five-client seed measurement are complete; the measurement did
not establish a CPU gain, as detailed below.

### Five-client remeasurement after context sharing (`c6984493`, 2026-10-21)

All five controlled seeds passed SHA-256 at 3.82–3.94 MiB/s. TorrentNG used
**2.54 CPU-s/GiB** and reached **24.4 MiB** cgroup anonymous memory, with
39.3/38.8 MiB process-tree RSS/PSS. qBittorrent measured 1.87/16.9, Transmission
1.37/23.6, Deluge 2.41/86.8, and rTorrent 0.57/1.8 for CPU-s/GiB and
anonymous peak. TorrentNG remains slower in CPU per GiB than all four and has
mixed RAM results.

Compared with the preceding `f6cce154` single pass (2.46 CPU-s/GiB, 25.2 MiB
anonymous peak), the candidate was 0.08 CPU-s/GiB higher and 0.8 MiB lower in
anonymous memory. This pass ran on a shared host with one-minute load peaking at
17.21; it cannot attribute either difference to context sharing. The public
phases for Transmission, TorrentNG, and Deluge timed out and are excluded. See
the [full report](../certification/reports/client-performance-benchmark-context-sharing-candidate-20261021.md),
[samples](../certification/reports/client-performance-benchmark-context-sharing-candidate-20261021-samples.csv),
and [memory profile](../certification/reports/client-performance-benchmark-context-sharing-candidate-20261021-memory-profile.json).

### Download assembly and parsing

The handoff proposes a bounded global incomplete-piece budget and parsing that
retains the received payload, compared at 0, 4, and 8 MiB. TorrentNG currently
defaults this budget to zero. This remains an independent CPU/RAM experiment;
the audit did not establish its contribution, and it must preserve v1, v2,
hybrid, padding, cancellation, and crash-checkpoint behavior.

## Validation sequence

1. Run the rate scheduler regressions and the private two-seeder download
   reproduction. The scheduler regressions and full workspace tests now pass;
   the private fixture remains open.
2. Completed a sampled user-cycle profile for exact source `c6984493`, with
   backend selection and hash verification. The current experiment targets
   repeated upload-read-context construction and cloning; thread/system CPU,
   context-switch, syscall, and worker-dispatch counts remain unmeasured.
3. Compare the current and fused storage paths with the same backend, fixture,
   cache state, and receiver, recording CPU/GiB, goodput, memory series, and
   worker dispatch counts.
4. Run io_uring and retained-memory experiments only where measurements show
   they apply; keep assembly-budget trials independent.
5. Re-run the comparative client suite only after the selected candidate passes
   correctness and shows improvement beyond run variation.

The original archived audit did not run a full-daemon benchmark. Later
single-pair and exact-main client runs add preliminary source-level evidence,
but still do not establish a repeatable CPU speedup, a broad RAM advantage, or
a universal client ranking.
