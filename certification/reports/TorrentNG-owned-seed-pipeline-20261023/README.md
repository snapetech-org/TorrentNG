# Owned seed pipeline implementation validation — October 23, 2026

This record validates the seven implementation workstreams described in
[`docs/seed-efficiency-pipeline.md`](../../../docs/seed-efficiency-pipeline.md).
It is a local correctness and performance screen, not rTorrent parity or broad
release qualification. No rTorrent executable or baseline PGO build was run here.
The complete four-cell structural-change × PGO comparison and broader workload
qualification remain necessary before attributing gains to either factor.

## Artifacts and method

The control is clean main `e01511ef30ba6983d5337d43d6cb8e03f52298df`.
The candidate manifests identify clean commit
`f06de86a68446573eab4ec5050e4b8880514eacf`. Later commits change tooling,
documentation and evidence only; the measured Rust source digest is unchanged.
Builds use Rust 1.97.0, the existing compact release profile, the system allocator,
and the existing worker defaults. The candidate PGO profile was trained after the
Rust changes on the four representative training workloads. A matching-compiler
profile-use build completed and passed transfer verification.

The comparison uses ten balanced randomized rounds, fresh daemon processes,
64 MiB verified payloads, 4 MiB/s offered load, a 16-request window, fixture seed
20261024, one peer and warm data. This window and fixture differ from training.
Daemon affinity is CPUs 0–1; measurement/leecher affinity is CPUs 2–3. Own builds
and test jobs completed before these runs. This container exposes no SMT siblings
for these CPUs, and does not expose PSI files. Missing host counters are recorded
as null, not interpreted as absence of interference. This is not dedicated-host
certification. No profiler or mid-transfer diagnostics run in scored transfers.

CPU uses a nanosecond process clock translated through the container's PID
namespace. PSS peaks include connection and transfer samples; CPU covers first
payload through completion. These scopes are explicit and must not be confused
with startup cost, steady-state memory, or the full cgroup/kernel memory bill.
The probe records idle/recheck/transfer snapshots and available host VM/cgroup
counters. Do not add overlapping process and cgroup values.

The practical tolerance was 2% with at least ten rounds before measurement.
The two primary bootstrap intervals are Bonferroni-adjusted for a joint 95%
coverage target. CPU and PSS must both have an upper ratio bound at or below 1.02.
Anonymous memory is secondary. All thirty payloads passed SHA-256 verification.

## Measurements

Absolute values below are medians across runs. Decisions use matched-round ratios
in the second table, not ratios of these separate medians.

| Build | CPU seconds/GiB | Peak PSS MiB | Peak anonymous MiB | Payload MiB/s | First payload delay ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| baseline | 4.860 | 21.253 | 7.852 | 4.000 | 2.299 |
| candidate | 4.771 | 19.515 | 6.123 | 4.000 | 1.932 |
| candidate-pgo | 3.733 | 16.324 | 5.910 | 4.000 | 2.125 |

| Comparison | Paired CPU ratio [interval] | Paired PSS ratio [interval] | Joint resource gate |
| --- | ---: | ---: | --- |
| baseline → candidate | 0.934 [0.878, 1.010] | 0.942 [0.822, 1.005] | pass |
| baseline → candidate-pgo | 0.750 [0.682, 0.850] | 0.777 [0.702, 0.842] | pass |
| candidate → candidate-pgo | 0.783 [0.759, 0.854] | 0.838 [0.819, 0.851] | pass |

Ratios below one favor the candidate. The local gate does not establish goodput,
latency or resource parity across other workloads. It cannot separate structural
and PGO effects without the missing baseline PGO cell. PGO and mmap remain opt-in;
no allocator, worker-count, batch-size or compiler-profile default was changed.

## Correctness and build validation

Hosted GitHub checks were still queued when this evidence was published. The
results below are local validation, not claims that hosted CI completed.

- 2,392 workspace library, binary and integration tests passed, 12 ignored; two process-detection tests were
  excluded after both failed identically on unchanged main in this container.
  Its `/proc` mount uses an ancestor PID namespace. The excluded tests are
  `source_state::tests::this_process_is_seen_running_with_its_own_name` and
  `source_state::tests::an_explicit_lock_held_by_a_live_process_means_running`.
- Optional mmap engine/storage tests passed, including partial socket writes,
  cancellation/refunds, replacement, truncation, bounded pinned windows, and
  page-rounded tiny-file accounting. Dedicated final budget tests passed.
- Default all-target workspace Clippy and full-compat/mmap feature Clippy passed
  with warnings denied. Full compatibility daemon tests passed (32 passed,
  one ignored). Formatting, OpenAPI and release-note checks passed.
- 87 Python measurement, provenance, workflow and release-note tests passed.
- The PGO release verified three concurrent random-request peers against an
  unaligned five-file fixture after file-page eviction hints (48 MiB aggregate).
- The buffered release verified a 16 MiB transfer constrained to 1 MiB/s;
  observed payload goodput was approximately 1.007 MiB/s including normal burst
  and measurement-boundary effects.
- The mapped release verified a 64 MiB transfer after file-page eviction hints.
  Its diagnostic snapshot showed 524,288 mapped bytes, no buffered read-ahead
  entries, two completion-slot allocations and 355 reuses. These are snapshots,
  not a complete operation budget. A separate three-peer random five-file run
  verified mapping/fallback behavior over 48 MiB aggregate.

The optional mapping still needs broader cold-data, external-mutation and memory
qualification. Page locking prevents ordinary reclaim faults for admitted pages;
external mutation cannot provide an atomic content snapshot. No mapped Rust
references are exposed, and truncation/range failures are tested at the syscall
boundary. Unsupported/admission-denied cases retain buffered fallback.

## Evidence and reproduction

`paired-summary.json.gz` contains every scored run, clocks, raw samples, ordering,
source/executable/shared-library identities, harness hashes and host snapshots.
`paired-analysis.json` contains all three paired comparisons, including failures
of the resource gate. Separate functional and training summaries are labelled;
they are not added to scored rounds. Compressed logs preserve local validation.
Absolute scratch paths in raw reports identify the original run; raw `.smaps`
files are not included, but their reported totals and process samples are.

Binary and profile manifests preserve hashes and source/compiler provenance.
Executables and the generated 23 MiB profile are build outputs, not committed
inputs. Regenerate the representative profile using the documented training
command; clean-build executable hashes need not be byte-identical. The training
manifest records the actual profile used for this screen.

Recompute the paired results with:

```sh
gzip -dc paired-summary.json.gz > /tmp/owned-seed-summary.json
python3 ../../../scripts/seed_efficiency_tools.py analyze \
  --summary /tmp/owned-seed-summary.json --control baseline --candidate candidate \
  --minimum-rounds 10 --tolerance 0.02 --output /tmp/owned-seed-paired.json
```

The source-repository option in the PGO tools permits training and building a
clean historical baseline without changing that checkout. Use that capability
for the full four-cell held-out matrix before any broad performance promotion.
