# Seed CPU and memory implementation

The October 23 review identified repeated upload ownership, active-seed indexes,
and build attribution as opportunities to combine the earlier CPU and RAM work.
This implementation covers all seven concrete workstreams. It does not establish
rTorrent parity: performance promotion still requires the joint qualification
described below.

| Review workstream | Implementation |
| --- | --- |
| One owner across an upload batch | A contiguous batch owns its backing, memory charge, aggregate read permit, and rate grant through socket settlement. Frame descriptors borrow ranges; completed frames advance one settlement cursor. |
| Reusable prepared storage work | Typed jobs reuse bounded completion records instead of allocating a closure, oneshot, and cancellation owner for every read. Generation checks reject stale queued work after cancellation/reuse. Filesystem validation stays on disk workers. |
| Credit before payload | Acquire a bounded grant before materializing data. Limited or competing traffic takes one block; uncontended unlimited traffic can reserve the bounded batch. Cancellation, timeout, and partial completion refund unused credit, including across live rate epochs. Hybrid accounting uses wire lengths. |
| Compact seed state and code layout | Availability has explicit seed/download variants. Complete seeds release candidate/rarity indexes and empty partial-piece tables while preserving swarm counts. Uniform flags need no bit-vector allocation. A failed recheck lazily rebuilds download indexes. Upload preparation/writes are extracted from lifecycle/control code; infrequent handshake/have-state operations are marked cold. |
| Separate representation and scheduling bounds | Inline storage, intake messages, read groups, and wire frames have independent constants. The existing eight-block/128 KiB read limit and 32 ms collection policy remain. Per-peer future allocations are reused, retaining only the two common idle slots. |
| Narrow mapped-range experiment | The optional Linux `seed-mmap` build uses a tagged batch backing, identity/generation checks, shared cache admission, 256 KiB windows, and at most 512 KiB per scheduler. Only requested pages are populated and locked on workers. No Rust reference is formed over externally mutable file mappings. |
| Worker/allocation attribution before tuning | Export completion allocations/reuses, active jobs, cumulative queue/run times, and mapped cache bytes alongside existing storage counters. Diagnostic snapshots include per-thread CPU/scheduler counters and process maps. Keep two storage workers and the system allocator. |

The conservative piece-index governor reservation remains available for a later
return to downloading. Releasing actual seed allocations does not let a recheck
silently bypass admission. Existing tracker, super-seed, BEP 6, metadata, padding,
hybrid, uTP, verification, and durability paths remain in the product.

## Mapping ownership and fallback

Build with `cargo build --release --locked -p torrentngd --features seed-mmap` and
set `TNG_SEED_MMAP=1` on that daemon to exercise the experiment. Both choices are
required; normal builds contain neither its backing variant nor its larger
preparation future. The runtime defaults remain portable buffered reads.

Mapped bytes share the existing global peer-read-cache budget and resource
governor. Buffered read-ahead is disabled while mapping is enabled, including for
fallback reads, so the experiment does not silently retain two caches. Live
batches pin their window; pressure can reclaim only idle windows. Cache entries
do not retain descriptor leases. A live batch does.

Linux `mmap`, `madvise`, `mlock`, `sendmsg`, and `munmap` are isolated in the storage
module, with range checks and ownership comments at every unsafe boundary. This
follows the repository's existing narrow native-syscall boundary; it is not a
general allowance for unsafe application memory access. Requested pages must be
resident and locked before the peer can send them. Unsupported kernels, denied
memory-lock admission, exhausted budgets, unsupported backends, uTP, hybrid, and
cross-file requests use the buffered path. External truncation becomes a syscall
error/short write, rather than a Rust load from an invalid mapping. File mutation
still cannot make an entire transfer an atomic content snapshot; checksum
verification remains necessary, as with buffered file reads.

`munmap` releases both mappings and page locks when the final owner drops.
Locked bytes count against the same conservative window charge. Do not raise the
host's memory-lock limit automatically to make an experiment pass.

## Build attribution and paired analysis

Create a manifest immediately after each build, while its source is unchanged:

```sh
python3 scripts/seed_efficiency_tools.py manifest \
  --binary /absolute/path/torrentngd --repo . --require-clean \
  --output /absolute/path/torrentngd.manifest.json
```

The manifest records commit, dirty status/diff hash, hashes of workspace source
and Cargo.lock, compiler/Cargo versions, feature label, Rust flags, optional PGO
profile hash, executable hash, and shared-library hashes. Set `--features` to the
actual Cargo feature selection for an experimental build. A manifest identifies
inputs and an artifact; it is not a reproducible-build attestation. Keep the build
log with it. The probe checks the executable against its manifest before running.

The probe now accepts `--peers`, `--rate`, `--window`, `--request-pattern`,
`--fixture-files`, `--upload-limit`, `--cache-policy`, `--client-cpus`, and
`--client-manifest LABEL=JSON`. Each peer verifies the full payload. CPU per GiB
uses the total verified transferred bytes, including concurrent peers. Multi-file
fixtures deliberately cross block boundaries. `drop-file-pages` issues a
file-specific eviction hint after recheck; it does not claim a globally cold disk.

Rounds use randomized balanced rotations. Record the host's affinity, SMT siblings,
pressure, VM/swap counters, and cgroup memory/CPU state. Reserve physical cores
outside the tool and keep leechers on other cores. These snapshots reveal noise;
they do not make a shared host isolated. Detailed profiler runs are separate from
scored runs. Use the existing transfer-rebuild and API-load harnesses for mixed
download/recheck/API and many-torrent qualification.

```sh
python3 scripts/seed_efficiency_tools.py analyze \
  --summary /path/probe/summary.json --control baseline --candidate candidate \
  --minimum-rounds 10 --tolerance 0.02 --output /path/paired.json
```

The default practical tolerance is declared before measurement: 2%, with at least
ten complete matched rounds and clean artifact provenance. Report the median of
paired ratios, wins, and bootstrap intervals. CPU and PSS are co-primary; the two
intervals use Bonferroni adjustment for a joint 95% coverage target. Either resource
can veto promotion. Short, noisy, incomplete, instrumented, or mixed-artifact
results cannot qualify a release. Goodput, latency, correctness, idle costs,
resident stacks, allocator retention, socket memory, and separately attributable
file-cache costs still require review. Never add overlapping process/cgroup peaks.

## Representative PGO and held-out matrix

```sh
python3 scripts/native_seed_pgo.py train --output /absolute/path/training
scripts/build_native_pgo_release.sh /absolute/path/training/seed.profdata /absolute/path/pgo-build
```

Training uses the pinned Rust 1.97.0 compiler and its matching LLVM tools. It
includes startup, recheck, API work, paced/fast traffic, concurrent peers, random
requests, and cross-file transfers. Only daemon raw profiles are merged; build
script profiles are excluded. The profile sidecar pins source, compiler, training
corpus, and hashes. A profile cannot be silently reused after source or compiler
changes. Older archived profiles without that provenance need fresh training;
they are historical evidence, not qualified inputs to the new build script.

Train the baseline from its own source as well. Produce the four named cells
`baseline`, `baseline-pgo`, `candidate`, and `candidate-pgo`, each with a clean
manifest. Pass all four `--client LABEL=torrentngd:/path/binary` and matching
`--client-manifest LABEL=/path/manifest.json` arguments to:

```sh
python3 scripts/native_seed_pgo.py matrix --output /absolute/path/held-out \
  --repeats 10 [four client and manifest arguments]
```

The matrix checks matching source within each PGO pair, runs the held-out corpus
in `certification/benchmarks/seed-efficiency-corpus.json`, and writes separate
paired structural and PGO comparisons. Supply an independently pinned rTorrent
artifact as an additional comparator. Do not multiply gains from unrelated
experiments or interpret the four cells as additive effects.

Direct ready-range hits, duplicate-miss coalescing, and worker activation hysteresis
were conditional research suggestions, not established improvements. The typed
storage lifecycle and diagnostics implement their prerequisites. They deliberately
retain worker-side path validation and the two-worker default until operation
counts justify more policy. Arena-one, one-worker, opt-z, and blended PGO profiles
were rejected tradeoffs in the review and are not promoted by this implementation.

## Historical evidence corrections

The earlier preflush-removal executable with SHA-256
`b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263`
was an unpromoted experiment. Later worker, arena, decoder, obligation-lock,
batch-16, and compiler-profile controls that reused it are comparisons against
that artifact, not an established clean-main baseline. Their same-binary
configuration comparisons remain usable within that scope. This change retains
the preflush and starts its control at clean commit
`e01511ef30ba6983d5337d43d6cb8e03f52298df`.

The historical compact-release CPU values 1.90 and 0.73 are seconds per 256 MiB
transfer, equivalent to 7.60 and 2.92 CPU seconds/GiB. The worker report's GiB labels
were incorrect. Keep those older host results separate from October 22–23 screens.
This erratum preserves original evidence rather than rewriting recorded results.
