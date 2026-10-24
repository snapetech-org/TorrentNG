# Seed execution and facade ownership experiments — 2026-10-23

Status: implementation, 50-run resource qualification and 28 held-out service
checks complete. **The joint gate failed. rTorrent parity
was not reached.** Peak PSS improved against both TorrentNG controls, but CPU
non-regression was not established. Experimental build/runtime options remain
opt-in; ordinary release settings and buffered file I/O remain the defaults.

The goal is simultaneous whole-client CPU and peak PSS parity with the pinned
rTorrent comparator, with verified transfers and service/correctness checks.
A subsystem size reduction, a three-round screen, or an isolated PGO improvement
is not evidence that this goal has been reached.

## Implemented ownership and evidence changes

- Fresh diagnostics fence engine snapshot generation and collection age without
  making the metrics API blocking; frame-pool counters follow the active pool.
- Remove the redundant peer async forwarding wrapper; move batch request
  coordinates until after storage completion. Rust 1.97 unoptimized layout
  diagnostics show incoming/outgoing task futures each 1,872 bytes smaller and
  allocated upload-future slots each 744 bytes smaller. These are object sizes,
  not a measured process RAM reduction.
- Share each native/qBittorrent AppState through one Arc. Configuration changes
  use copy-on-write and retain existing shared stores/locks. This keeps the
  benefit after compatibility API activation, without changing routes.
- Ordinary mlock populates and locks requested mapped pages on storage workers;
  remove the redundant MADV_POPULATE_READ call. Range, identity, generation,
  descriptor, admission and syscall-only access fences remain. Buffered reads
  stay the default. Tests cover requested pages, denied memlock, replacement,
  truncation, and budget release.
- Bind PGO and function orders to source/compiler/features and stage immutable
  hash-named inputs so Cargo rebuilds when profile/order contents change.
- Add opt-in, Linux-only anonymous verification scratch with explicit lifetime,
  bounded reuse, shared-cap accounting, pressure eviction and last-owner
  release. This targets the observed post-recheck allocator retention without
  introducing process-wide allocator trimming.

## Experiments rejected before promotion

1. Separate seed/general macro drivers saved only 24 bytes in the generated
   driver while adding boxed state/timers. This was not a useful physical state
   reduction; the prototype was reverted.
2. Moving event handlers outside the readiness select did not change generated
   future sizes. The prototype was reverted.
3. Lazy qBittorrent construction passed six focused routing/security/logging tests
   and full facade/daemon tests. The benchmark already activates that API for
   readiness. A same-binary three-round diagnostic observed eager/lazy cold
   anonymous medians 3.285/2.285 MiB, activated 3.504/3.191 MiB; first API request
   medians 0.474/2.486 ms. Active PSS medians 19.422/19.233 MiB and CPU/GiB
   4.336/4.500 do not establish a joint win. Dirty-source diagnostic only.
   The implementation was archived and reverted, rather than adding a new
   startup-only optimization flag to production.

## Validation completed before qualification

- Peer/storage/picker/wire/native suite: 1,401 tests passed with seed-mmap.
- Compact facade/daemon/full-compat/storage suite: 639 tests passed; existing
  ignored tests retained. Includes isolated denied-memlock subprocess.
- Clippy with warnings denied passed for the changed runtime crates and daemon
  with full-compat and seed-mmap. Formatting passed.
- 61 Python seed-tooling tests passed. Default and full-compat,seed-mmap Rust
  1.97 release builds succeeded. PGO/layout qualification follows below.

## Why these changes follow the comparator research

The retained-chunk and readiness-driven path in the pinned libtorrent source
remains the useful model. This iteration removes concrete ownership/representation
costs while retaining TorrentNG's pathname and generation fences, bounded
admission and portable fallback. It does not assume that rTorrent's advantage
comes from fewer threads, fewer send calls, or its implementation language.
The independent review is [recorded in the repository](../../../docs/reviews/2026-10-23-astra-architecture-review.md).

The worker population simplification uses the ordinary
[Linux mlock contract](https://man7.org/linux/man-pages/man2/mlock.2.html): success
means requested pages are resident and locked. This does not prevent external
truncation/hole-punch revocation; only kernel syscalls may access payload
mappings. No Rust slice is formed and no host memlock limit is raised.

## Provenance and scope

The baseline runtime+diagnostics source is c0cc191681bb73147d3aec6655cdb2cf17b73232.
The original compact-state/mapping screen source is 9c33b240e9ca9932607428c2b2e938edf5a635ec.
Both are published commits with exact source hashes in build manifests. Future
state changes were committed separately as 48f396be0f59a67003deac168225c4ccf392fb3b.
The final verification-scope source is 589b97a42938b0884f241d76711e2d7e5e8d75ff.
Its buffered primary and mapped secondary configurations were frozen before
fresh qualification in [selected-candidate.json](selected-candidate.json).

The comparator remains rTorrent 0.16.25, binary SHA-256
8c27d0f49bfdb9c4b79dc29f9b8b13197dafd6dc55384237429e274d6758dedb,
with source/library identity in its manifest. No claim covers every rTorrent
build or deployment. The shared host is not a dedicated performance machine;
CPU affinity separates local probe and daemons, and scoring is separated from
compilation. Missing host pressure information is a limitation.

The independent runtime review found no blocking correctness issue. Stats
freshness is a collection-time fence, not an atomic snapshot across counters.
Rust embedders that previously moved owned public AppState fields must now
borrow or clone them through the shared handle; HTTP routes and behavior remain
unchanged. Further storage-operation hypotheses and required proof/tests are
[recorded separately](independent-runtime-review.md), not included in this
measured source.

## Qualification results

All 50 transfers verified: five frozen configurations in ten balanced paired
rounds, each transferring 64 MiB at 4 MiB/s, one peer, request window 16,
five-second warmup/cooldown, warm file data. The primary and secondary were
frozen before these measurements. Raw evidence is in
[qualification-summary.json.gz](qualification-summary.json.gz), with per-round
values and analysis in [paired-analysis.json](paired-analysis.json).

| Configuration | Median CPU-s/GiB | Median peak PSS MiB | Median peak anonymous MiB |
| --- | ---: | ---: | ---: |
| Earlier baseline PGO, buffered | 3.564 | 16.051 | 6.041 |
| Previous selected ordered PGO, buffered | 3.455 | 12.539 | 5.469 |
| **Primary: scoped ordered PGO, buffered** | **3.409** | **11.525** | **4.271** |
| Secondary: scoped ordered PGO, mapped | 3.530 | 11.105 | 3.438 |
| Pinned rTorrent | 2.439 | 10.533 | 2.590 |

The table above contains separate medians. Inferential comparisons below use
the **median of within-round ratios**, not the ratio of those medians. Lower
is better. The 10,000-resample bootstrap uses the existing Bonferroni joint
95% rule for co-primary CPU and PSS within each comparison; it does not adjust
for the entire exploratory family. Both upper bounds must be at most 1.02
to pass the 2% joint non-regression/parity gate. Absence of a statistically
established difference is not a pass.

| Candidate / control | Paired CPU ratio [joint 95% interval] | Paired PSS ratio [joint 95% interval] | Joint gate |
| --- | --- | --- | --- |
| Primary / baseline PGO | 1.002 [0.895, 1.062] | 0.715 [0.698, 0.739] | Failed: CPU unresolved |
| Primary / previous selection | 1.071 [0.877, 1.129] | 0.917 [0.893, 0.940] | Failed: CPU unresolved |
| **Primary / rTorrent** | **1.480 [1.356, 1.536]** | **1.092 [1.076, 1.119]** | **Failed: both worse** |
| Secondary / baseline PGO | 0.954 [0.907, 1.065] | 0.691 [0.674, 0.703] | Failed: CPU unresolved |
| Secondary / previous selection | 1.058 [0.898, 1.165] | 0.880 [0.855, 0.889] | Failed: CPU unresolved |
| Secondary / rTorrent | 1.409 [1.364, 1.615] | 1.052 [1.041, 1.073] | Failed: both worse |
| Secondary / primary | 0.997 [0.910, 1.092] | 0.964 [0.944, 0.969] | Failed: CPU unresolved |

Thus the primary establishes a 28.5% peak-PSS reduction versus the earlier
PGO baseline and 8.3% versus the previous selection. It does **not** establish
a CPU improvement or CPU non-regression. Its paired rTorrent gap remains
48.0% CPU and 9.2% PSS. The mapped secondary reduces memory further but does
not close the gap or establish joint non-regression against the primary.
It does not replace the frozen primary after the results are known.

All five configurations delivered approximately 4.000 MiB/s. The primary's
maximum per-peer first-payload delay had median 1.666 ms and observed range
1.232–3.206 ms; these ten observations are not tail-latency certification.
Setup/recheck CPU is recorded separately and includes add/readiness and
diagnostic collection, not isolated hash throughput. Its descriptive paired
intervals cross equality and do not establish a setup-CPU benefit.

These are complete configuration comparisons. The earlier baseline uses
opt-level s PGO; the new configuration uses opt-level 3, newly trained PGO,
function ordering and scoped scratch. Even the previous-selection comparison
changes trained profile/order as well as source. This evidence cannot assign
the whole memory reduction to any individual refactor, and it does not
qualify the performance of the normal default build. No experimental setting
is promoted to a default on the basis of this failed joint gate.

An [independent Astra audit](independent-final-review.md) reproduced the
paired calculations, checked frozen binary/source/profile identities, and
verified all 28 held-out runs and their individual peer checksums.

## Held-out service screen

All 28 transfers verified: seven checked-in validation shapes, one round per
shape across the previous selection, frozen primary, frozen secondary and
rTorrent. This is a descriptive correctness/service screen, not a statistical
CPU/PSS qualification or a guarantee of latency tails. Raw per-shape summaries
and [held-out-analysis.json](held-out-analysis.json) retain all clients and
metrics; `python3 analyze_held_out.py` replays the analysis from those archives.

Rates below are aggregate verified payload goodput. The offered rate is per
peer; the upload cap is per daemon. The file-page-drop policy is a cache hint,
not proof of cold physical storage.

| Shape | Target MiB/s | Primary MiB/s | Mapped MiB/s | rTorrent MiB/s | Max peer first payload, primary / mapped / rTorrent (ms) |
| --- | ---: | ---: | ---: | ---: | --- |
| 128 MiB sequential, 1 peer, window 16 | 4 | 4.000 | 4.000 | 4.000 | 1.691 / 2.902 / 1.552 |
| 128 MiB sequential, 4 peers, window 64 | 16 | 15.997 | 15.993 | 15.994 | 17.891 / 4.077 / 4.579 |
| 128 MiB random, 1 peer, window 64 | 64 | 63.455 | 63.480 | 63.997 | 3.034 / 3.048 / 3.111 |
| 32 MiB sequential, 1 peer, window 1 | 1 | 1.000 | 1.000 | 0.355 | 1.411 / 1.554 / 41.528 |
| 128 MiB sequential, 3 peers, window 32, file-page-drop hint | 48 | 47.980 | 47.967 | 47.980 | 5.522 / 8.928 / 8.408 |
| 32 MiB random, 3 peers, 5 files, window 64, file-page-drop hint | 24 | 23.940 | 23.852 | 23.860 | 8.285 / 30.481 / 26.459 |
| 16 MiB, 1 peer, window 32, 8 MiB/s offered, 1 MiB/s upload cap | 1 | 1.007 | 1.007 | 1.266 | 3.218 / 2.231 / 1.538 |

The capped short transfer does not demonstrate identical throttle/burst
behavior: rTorrent averaged 1.266 MiB/s against the configured 1 MiB/s limit,
while the TorrentNG variants averaged 1.007 MiB/s. Neither a strict instantaneous
cap nor the cause of that difference is established by this screen. The
window-one throughput difference likewise describes this observed setup,
not general superiority. Single-round multi-peer first-response differences
remain a caution; this data cannot establish latency non-regression.

CPU varies substantially by workload. In the capped shape, the primary used
13.826 CPU-s/GiB, the previous selection 18.383, mapped 17.525 and rTorrent
1.019; these short, differently paced runs are not a fair qualification of a
percentage CPU improvement. They identify low-rate scheduling as another
operation-budget target. Mixed download/recheck/API traffic, many torrents,
long-duration throttle behavior and dedicated-host runs remain unqualified.

## Completed exploratory build matrix

All 33 transfers verified (eleven cells, three balanced rounds). This is a
selection screen, not qualifying evidence. Each run transferred 32 MiB at
4 MiB/s, one peer, window 16, with two-second warmup/cooldown. The complete
raw summary and selection analysis are archived beside this report.

| Build / backing | CPU-s/GiB | Peak PSS MiB | Peak anonymous MiB |
| --- | ---: | ---: | ---: |
| Baseline PGO buffered | 3.677 | 17.400 | 5.988 |
| Refined normal default | 4.065 | 18.796 | 5.488 |
| Refined s PGO buffered | 3.704 | 15.174 | 5.414 |
| Refined s PGO mapped | 3.738 | 15.802 | 5.383 |
| Refined s ordered buffered | 3.941 | 12.482 | 5.426 |
| Refined s ordered mapped | 3.467 | 12.592 | 5.414 |
| Refined 3 PGO buffered | 4.179 | 16.146 | 5.723 |
| Refined 3 PGO mapped | 3.520 | 16.126 | 5.461 |
| Refined 3 ordered buffered | 3.287 | 12.752 | 5.457 |
| Refined 3 ordered mapped | 3.607 | 13.160 | 5.457 |
| Pinned rTorrent | 2.560 | 10.504 | 2.574 |

These are separate medians, not paired confidence intervals. The screen
selected opt-level 3, ordered PGO, buffered backing as the CPU-oriented
candidate before collecting qualifying data. Mapped activation was not a
consistent additive win. The later verification-lifetime experiment is a
separate source change and requires fresh selection/training/qualification.

## Recheck retention and API graph experiments

Fresh metrics showed zero live/retained ordinary frame-pool buffers after
recheck, while anonymous residency rose by roughly 2.6 MiB. An isolated
diagnostic helper called `malloc_trim(0)` after recheck. Anonymous residency
fell from 5.277 to 4.137 MiB (1,196,032 bytes) without changing live allocator
bytes (1,898,016). This establishes that part of the residency was reclaimable
allocator retention. The helper and all its scores are diagnostic only;
process-wide allocator trimming is not a production change.

Two independent prototypes were then tested alone and together in the same
normal binary, with three balanced rounds and 16 MiB transfers. The facade
was activated before adding the torrent; each run also performed 10,000
authenticated requests alternating the two qBittorrent aliases.

| Prototype | Transfer CPU-s/GiB | Peak PSS MiB | Post-recheck anonymous MiB | Setup/recheck CPU-s | API CPU-s / 10k |
| --- | ---: | ---: | ---: | ---: | ---: |
| Legacy | 3.980 | 19.110 | 5.348 | .03144 | .26972 |
| Shared route graph | 4.777 | 18.563 | 4.883 | .03146 | .29224 |
| Per-chunk anonymous buffers | 4.126 | 18.556 | 3.340 | .03597 | .26468 |
| Both | 4.666 | 17.860 | 2.867 | .03628 | .27207 |

The shared graph reduced live allocator bytes by 320,112 in an isolated
allocation test, but introduced another dispatch and per-request matched-path
allocation. Early outer-layer rejection route labels also differed. Its 155
tests and Clippy passed, but this evidence does not justify promotion.

Anonymous buffers reduced post-recheck anonymous residency in this diagnostic,
but allocating and unmapping each chunk added setup/recheck CPU. The refinement uses an explicit
verification lifetime: at most two idle mappings of at most 1 MiB are reused
and remain charged to the frame cap; the final scope releases idle maps and
outstanding owners release on their last drop. Oversized allocations outside
verification keep vector backing. Kernel mappings are private anonymous
memory, never externally mutable torrent file mappings. Performance selection
for this refinement is recorded separately.

The setup/recheck number includes add/readiness and diagnostic collection;
it is not isolated SHA throughput. The API test is a persistent-connection
version request microbenchmark, not a full API load certification. All these
runs use a dirty prototype and diagnostic instrumentation, so none qualifies
CPU/PSS parity.

The refined scope-gated prototype completed twelve more diagnostic transfers
(32 MiB, 4 MiB/s, window 16; three balanced rounds):

| Backing / scope | CPU-s/GiB | Peak PSS MiB | Peak anonymous MiB | Setup/recheck CPU-s |
| --- | ---: | ---: | ---: | ---: |
| Buffered, legacy | 4.079 | 18.738 | 5.500 | .04548 |
| Buffered, scoped | 4.168 | 18.359 | 4.820 | .04184 |
| Mapped, legacy | 4.540 | 19.168 | 5.500 | .04569 |
| Mapped, scoped | 4.501 | 17.355 | 3.500 | .04589 |

All six scope-enabled runs reported zero frame bytes in use/retained and zero
storage-frame denials after recheck. The buffered cache subsequently occupies
some of the reclaimed memory during transfer; mapped backing preserves more
of the anonymous saving. These small screens do not establish CPU equivalence.
The implementation was therefore committed as an opt-in Linux experiment,
`TNG_STORAGE_ANON_LARGE_FRAMES=1`, with unchanged ordinary defaults.

Source `589b97a42938b0884f241d76711e2d7e5e8d75ff`, tree
`f87f2834292a07183f4b3fc7d35567329d688814`, has build-input digest
`347b31f6d7db9ae52931bdb349aca27289a22f868aa215d2f6227141bd10d7bf`.
It passed 771 engine and 370 storage tests with the option enabled, 370 storage
tests with it disabled, and engine/storage all-target Clippy with warnings
denied. The 52 workflow/security/release-tool unit tests and release-fragment
validation passed. CI now explicitly exercises scoped scratch with seed-mmap.

An additional isolated real-daemon witness completed a verified 128 MiB
recheck, then observed cancellation at 72 of 128 pieces. Three fresh samples
after each outcome confirmed zero frame bytes in use/retained and zero active
prepared jobs. The first witness attempt had an incorrect SQLite job-kind
filter (`recheck` instead of `recheck_torrent`); its timeout is preserved and
excluded. The corrected script and positive evidence are archived. It is a
functional observation, not a deterministic timing test or performance score.

The opt-in mode retains the shared frame-pool cap, a two-map idle bound and
pressure eviction. Concurrent verification can cause another oversized pool
acquisition to use anonymous backing while a scope is active; mixed workload
default promotion remains unqualified. As with ordinary mapping ownership,
kernel policies that refuse valid unmapping are not handled as a recoverable
allocation condition. No externally mutable file mapping is exposed as a Rust
slice, and ordinary buffered reads remain the file I/O path.

## Final-source optimized selection screen

All eighteen transfers verified (three balanced rounds, 32 MiB at 4 MiB/s,
window 16). All new-source cells use the same scoped-scratch setting, source,
Rust 1.97 compiler, seed-mmap feature and opt-level 3 PGO profile. Ordered and
unordered builds differ in function ordering; mapped and buffered cells use
the same corresponding binary with runtime activation off/on.

| Configuration | CPU-s/GiB | Peak PSS MiB | Peak anonymous MiB |
| --- | ---: | ---: | ---: |
| Previous selected source 9c, ordered, buffered | 3.395 | 12.527 | 5.453 |
| Scoped PGO, buffered | 3.272 | 15.942 | 4.754 |
| Scoped PGO, mapped | 3.567 | 14.898 | 3.434 |
| Scoped ordered PGO, buffered | 3.252 | 11.637 | 4.273 |
| Scoped ordered PGO, mapped | 3.286 | 11.108 | 3.434 |
| Pinned rTorrent | 2.431 | 10.557 | 2.578 |

The predeclared rule preferred joint screen improvements versus the previous
selection, then minimized the larger paired CPU/PSS median ratio to rTorrent.
Ordered buffered won that rule (worst ratio 1.311); ordered mapped retained a
lower-PSS alternative (worst ratio 1.352). Buffered remains the primary and
mapped the secondary throughout fresh qualification; neither can be selected
after seeing qualifying scores. These small-screen numbers do not establish
improvement, equivalence or parity by themselves.

The final binary is `4f6fc030f19589be5cd6ba71d7f87cc347ac0ed5a33de3a95ef6c08921dfb6d8`.
Its PGO profile is `64253846872def473a80a8267c02a22231249c4039448bb75ae3188455a195c9`
and order input is `eb86a6c364f3e7e55fcd73fb397062fee37a66a6e9a45924541cdff78b3204ac`.
The profile and layout each trained on eight verified transfers, exercising
both mapped and buffered backing with scoped scratch enabled. The linker
cannot reorder three external shared-library symbols (`__udivti3`,
`__floattidf`, `round`); those warnings are retained in the build log.

## Reproduction

From a checkout containing this report, create a clean source worktree at the
recorded runtime commit. The helper accepts the compressed profile directly;
its adjacent `.gz.manifest.json` records the uncompressed profile hash. Keep
the order's `.txt.manifest.json` beside the text file. The helper validates
source contents, compiler, features, optimization level, profile and order.

```sh
git worktree add --detach /tmp/torrentng-qualified-source \
  589b97a42938b0884f241d76711e2d7e5e8d75ff
scripts/build_native_pgo_release.sh \
  certification/reports/TorrentNG-seed-execution-20261023/scoped-3.profdata.gz \
  /tmp/torrentng-qualified-build /tmp/torrentng-qualified-source \
  certification/reports/TorrentNG-seed-execution-20261023/scoped-3-function-order.txt \
  seed-mmap 3
```

The primary runtime settings are `TNG_STORAGE_ANON_LARGE_FRAMES=1` and
`TNG_SEED_MMAP=0`; the secondary changes only the latter to `1`. Normal release
defaults remain unchanged. A rebuild receives a new binary hash/manifest;
byte-for-byte reproducibility is not claimed. Use that new manifest for a new
experiment, not an archived manifest naming the originally measured binary.

The qualification probe uses `--size 67108864 --rate 4194304 --window 16
--peers 1 --repeats 10 --warmup 5 --cooldown 5 --order-seed 240240
--client-cpus 0-1`, with the probe pinned to CPUs 2-3 and no concurrent local
compilation. Supply every client binary through `--client NAME=KIND:BIN`, its
verified build manifest through `--client-manifest NAME=JSON`, and the explicit
runtime settings through `--client-env NAME:KEY=VALUE`. The archived
`run_seed_final.py` records the full five-client argv and seven held-out
shapes. Helper snapshots were executed from a task workspace containing the
TorrentNG checkout, comparator and frozen binaries; their absolute paths are
evidence of that execution, not portable installation paths.

To retrain instead of using the archived profile, use `native_seed_pgo.py train`
and then `native_seed_function_order.py train`, both with the clean source,
`--features seed-mmap --opt-level 3 --corpus
certification/reports/TorrentNG-seed-execution-20261023/scoped-training-corpus.json`.
The layout tool also requires the newly trained profile, LLVM BOLT 20 and
merge-fdata 20. Keep retraining data separate from fresh qualification. The
pinned rTorrent and libtorrent commits, linked-library hashes and extracted
Autotools compiler flags are recorded in the comparator manifest and
`comparator-build-flags.json`.

## CPU attribution and hosted validation

The opt-level 3 ordered buffered candidate completed a separate verified
256 MiB transfer at 8 MiB/s under a gperftools sampling helper. Its 161 samples
were concentrated in recv (21.7%), generic syscall (12.4%), writev (11.2%),
the peer driver (8.7%), epoll_wait (8.7%), pread (5.6%) and statx (5.6%).
This coarse diagnostic points to transport/storage work; it does not establish
that a Tokio replacement would close the gap. The sampled score is excluded
from qualification. Source, raw profile and attribution output are archived.

Published runtime head `addea38181a3dae22d69ae9a363c331bb808b6fc` passed all
hosted CI jobs, Windows Smoke, CodeQL, Semgrep, Trivy and release-note validation
([workflow and job snapshot](ci-runtime-addea38.json)). Its runtime source is
identical to the frozen source `589b97a`; the intervening commit fixes
release-note metadata. CI
includes workspace/full-compat/mapped tests, MSRV, browser certification,
fuzz smoke, backup/restore, fault containment and API/SSE load. Separate
WebUI fixes restored accessible names and updated the vulnerable Zod lockfile
entry; local 49 WebUI tests, build, lint and audit also passed. The older
`9a63039` snapshot remains archived separately and predates scoped scratch.

## Remaining gap and next decision

The measured RAM savings justify retaining the opt-in experiment and its
reproduction evidence. They do not satisfy the simultaneous CPU/RAM goal.
Default ownership/diagnostic refactors have functional coverage; their CPU
effect is not independently certified by the optimized bundle comparison.

The next CPU investigation needs an actual operation budget: worker submissions,
completion wakeups, metadata checks, fallback preparations, and active peer
polls per verified MiB for both clients. The coarse sampling profile is not
that budget. A narrower physical seed-state split or amortized preparation
across already-admitted fragmented requests remains plausible only if those
counts identify a cost that can be removed. The attempted macro split, event
boundary and shared route graph did not provide that proof. A wholesale Tokio
replacement, larger unbounded buffers, or relaxed pathname/mutation checks
is not justified by these results. Further improvements require another
frozen-source, independently paired qualification; the current goal remains
open.
