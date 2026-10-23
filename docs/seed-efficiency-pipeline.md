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

The follow-up future-state refactor removes redundant async forwarding and
captures connection inputs directly. Upload-read completion coordinates are
projected after the awaited read, while the original request specifications
retain ownership throughout cancellation and error handling. With Rust 1.97.0
on x86_64, compiler layout diagnostics show incoming/outgoing connection futures
shrinking by 1,872 bytes and reusable upload-read futures by 744 bytes per
allocated slot. These object sizes are not whole-process RAM measurements.
No new protocol fork, task, queue, or per-batch allocation is introduced.

## Mapping ownership and fallback

Build with `cargo build --release --locked -p torrentngd --features seed-mmap` and
set `TNG_SEED_MMAP=1` on that daemon to exercise the experiment. Both choices are
required; normal builds contain neither its backing variant nor its larger
preparation future. The runtime defaults remain portable buffered reads.

Mapped bytes share the existing global peer-read-cache budget and resource
governor. Buffered read-ahead is disabled while mapping is enabled, including for
fallback reads, so the experiment does not silently retain two caches. Live
batches pin their window; pressure can reclaim only idle windows. Cache entries
do not retain descriptor leases. A live batch does. Window charges round up to
whole pages even for tiny files; at most eight cache entries are retained.

Linux `mmap`, `mlock`, `sendmsg`, and `munmap` are isolated in the storage
module, with range checks and ownership comments at every unsafe boundary. This
follows the repository's existing narrow native-syscall boundary; it is not a
general allowance for unsafe application memory access. Requested pages must be
resident and locked before the peer can send them. Ordinary `mlock` populates
and locks pages in one operation; no separate `MADV_POPULATE_READ` walk is needed.
Denied memory-lock admission, exhausted budgets, unsupported backends, uTP, hybrid, and
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

Diagnostic metrics now carry snapshot generation and age measured from
collection start. The probe waits for a snapshot whose collection began after
the diagnostic boundary, outside scored windows. Old binaries are explicitly
marked `unsupported`; a timeout is not treated as fresh data. Frame-pool gauges
observe the shared pool even when only MountScheduler is initialized, without
creating the older global storage runtime. Archived single-scrape counter
snapshots may describe an earlier phase; OS CPU/PSS scores are independent.

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

Train the baseline from its own clean source as well, without copying these
tools into that checkout:

```sh
python3 scripts/native_seed_pgo.py train --repo /absolute/path/baseline \
  --output /absolute/path/baseline-training
scripts/build_native_pgo_release.sh /absolute/path/baseline-training/seed.profdata \
  /absolute/path/baseline-pgo-build /absolute/path/baseline
```

Produce the four named cells
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

Build manifests now retain `source_tree` alongside the commit and hashes of actual
build inputs. Publishing through the GitHub Git-data API can change commit
metadata while preserving the entire tree. Tree equality reconciles that case;
it does not excuse dirty files or replace the input hashes. The earlier training
commit `f8772ff26b28e919431ff7dd73cd1917e3121f2d` and published commit
`a826610742469278cd0d3a77aa897dd934e09bd8` both identify tree
`91051f264644241d37150143ef95d00babfd3ea2`; all 258 training input hashes match
the published tree. The original evidence manifest remains unchanged.

For a native rTorrent comparator, record its separately built libtorrent source
in addition to the executable and all linked-library hashes:

```sh
python3 scripts/seed_efficiency_tools.py manifest --kind rtorrent \
  --binary /absolute/path/rtorrent --repo /absolute/path/rtorrent-source \
  --dependency-repo libtorrent=/absolute/path/libtorrent-source \
  --require-clean --output /absolute/path/rtorrent.manifest.json
```

Any dirty dependency makes the combined manifest dirty. The probe verifies that
recorded shared libraries still match before measuring, so replacing a library
without replacing its manifest fails attribution. Record compiler/configure flags
and compare clients on the same host; an older rTorrent score is not a control.

## Experimental PGO function ordering

The code-placement experiment uses LLVM BOLT to learn a function order and LLD
to apply it during the normal Rust PGO link. It does not execute BOLT's rewritten
planner output. Full BOLT rewriting and regular-page rewriting remain rejected
configurations in the [layout report](../certification/reports/TorrentNG-seed-bolt-screen-20261023/README.md).
The same report records the separate link-order qualification, its exact binary
identities, and the remaining rTorrent gap. Do not infer CPU improvement from a
smaller resident executable or promote a new build from the archived scores.

On Linux with LLVM BOLT 20 and `merge-fdata-20`, train the layout against a clean
source checkout and its already validated PGO profile:

```sh
python3 scripts/native_seed_function_order.py train \
  --repo /absolute/path/source --pgo-profile /absolute/path/training/seed.profdata \
  --output /absolute/path/layout-training --client-cpus 0-1
scripts/build_native_pgo_release.sh /absolute/path/training/seed.profdata \
  /absolute/path/ordered-build /absolute/path/source \
  /absolute/path/layout-training/function-order.txt
```

Training builds a PGO input with relocations, freezes that executable, instruments
it, runs the four-shape corpus, and merges only its daemon profiles. The input,
instrumentation, corpus, tools, merged profile and generated order are recorded.
The `generate` subcommand also accepts an existing pinned input binary and BOLT
profile with their manifests, avoiding unnecessary retraining during analysis.

The fourth build argument is optional. Without it the PGO build path is unchanged.
With it, the build checks order contents, source digest, compiler, PGO profile and
selected feature identity before linking, and records the order in the output
manifest. Changed inputs require retraining. No application release setting,
worker count, allocator or storage policy is changed by this option. Every output
artifact needs independent paired CPU/PSS validation and functional coverage;
the helper does not declare a performance pass.

Pass `--features seed-mmap` to both training commands and `seed-mmap` as the fifth
build argument (an empty fourth argument selects no function order). Training
then exercises mapped activation and buffered fallback in separate processes
using the same feature-enabled executable. Additional workload environment
settings in the corpus are preserved. Default-feature profiles are rejected for
feature-enabled builds, and vice versa.

The build helper and layout trainer stage profiles under content-hashed paths;
the helper also stages linker-order files by content hash. Cargo fingerprints
compiler arguments, so replacing profile/order contents at one unchanged path
must not silently reuse an older optimized artifact. Keep frozen measured
executables and their manifests separate from reusable target directories.

Unset `CARGO_ENCODED_RUSTFLAGS` even when its value is empty. Cargo gives it
[precedence over `RUSTFLAGS`](https://doc.rust-lang.org/cargo/reference/config.html#buildrustflags),
so accepting an empty override could silently omit the intended PGO flags.

Direct ready-range hits, duplicate-miss coalescing, and worker activation hysteresis
were conditional research suggestions, not established improvements. The typed
storage lifecycle and diagnostics implement their prerequisites. They deliberately
retain worker-side path validation and the two-worker default until operation
counts justify more policy. Arena-one, one-worker, opt-z, and blended PGO profiles
were rejected tradeoffs in the review and are not promoted by this implementation.

## Historical evidence corrections

The [matched rTorrent follow-up](../certification/reports/TorrentNG-seed-poll-boundary-20261023/README.md)
ran both normal and PGO builds before and after an explicit peer polling
boundary, plus the pinned native rTorrent/libtorrent comparator. That boundary
reduced PGO code size but failed the joint CPU/PSS qualification gate and was
reverted. The complete evidence is retained. Current main PGO still exceeded
rTorrent's CPU and PSS in the matched workload; previous PGO gains do not imply
client parity.

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

Implementation tests, release transfer checks, and the local ten-round comparison
are recorded in the [validation report](../certification/reports/TorrentNG-owned-seed-pipeline-20261023/README.md).
