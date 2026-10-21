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
2. Profile an exact-source hash-verified seed separately from scored runs;
   identify thread CPU, backend selection, handoffs, context switches, and
   relevant syscalls.
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
