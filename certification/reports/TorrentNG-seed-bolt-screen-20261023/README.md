# PGO code placement experiments — October 23, 2026

**Decision: retain function ordering as an opt-in research path; do not promote
it as a joint CPU/RAM improvement.** In the independent ten-round comparison,
paired PSS fell 16.4%, with an interval entirely below parity. The CPU median
ratio was 0.989, but its interval was [0.978, 1.092], failing the predeclared 2%
non-regression limit. Confidence is high in the scoped PSS reduction; CPU
non-regression is unresolved. Anonymous memory's paired median rose 6.8%, with
an interval crossing parity; there is no established anonymous-memory win.

The ordered build still trails matched rTorrent: paired median CPU ratio 1.312
and PSS ratio 1.242, with both intervals entirely above parity. Neither this
build nor the earlier polling experiment meets the goal. Normal release and
ordinary PGO defaults remain unchanged. Full BOLT rewriting is not promoted.

These exploratory screens test executable residency after the peer-poll change
failed its joint CPU/PSS gate. They do not establish rTorrent parity. The separate
[ten-round comparison](../TorrentNG-seed-poll-boundary-20261023/README.md) remains
the stronger measurement of current main against the pinned comparator.

## Inputs and method

All TorrentNG binaries use unchanged main
`7836fa77f4bdd26ac78c5bf436e8d88968bf1f60`, Rust 1.97.0, the normal release
settings, default features, two storage workers, and the same validated PGO
profile `abdc6344cb9137a24356b533bcd7bd4f43a9e365219dd479325f0336a1f27bc6`.
The input build adds `--emit-relocs` for BOLT. Its manifest records the exact
binary, source tree, compiler, profile and linked libraries. The rTorrent control
is the same native 0.16.25/libtorrent 0.16.25 build used in the ten-round report.

LLVM BOLT 20.1.2 instrumented the input binary. Four verified training workloads
from `certification/benchmarks/seed-efficiency-corpus.json` cover startup, recheck,
API use, paced and fast transfers, concurrent peers, random requests, and file
boundaries. Only the four daemon profiles were merged. No validation transfer
was included in training. Tool, input, output, instrumentation and merged-profile
hashes are retained in `bolt-profile.manifest.json.gz`.

The candidates are:

- **pgo-bolt:** BOLT `ext-tsp` block layout, `cdsort` function ordering and cold
  function/exception splitting, using the documented default page alignment.
- **pgo-bolt-4k:** the same transformation with `--no-huge-pages`.
- **pgo-link-order:** BOLT generates a function order; LLD uses that order while
  linking the Rust PGO build. The BOLT-rewritten planner output is never run.
  BOLT's `/N` local-symbol suffixes are removed and every resulting name is
  checked against the input ELF symbol table. The linker reports three shared
  symbols it cannot order; that warning is retained. Runtime source, compiler and
  PGO data are fixed, but this is not proof of byte-identical code generation.

BOLT reported 6,136 profiled functions out of 33,447, including 57 profiled
functions it could not optimize. Its relocation, internal-call and exception
frame warnings are preserved in the logs. Successful transfers do not constitute
broad qualification of rewritten Rust/native dependency code.

Each candidate separately verified three concurrent random-request peers over an
unaligned five-file fixture, 24 MiB per peer, with file-page eviction hints. These
are functional checks; their measurements are not pooled with scored runs.

`experiment-plan.json` and `followup-plan.json` were written before their
respective measurements. Each screen uses three randomized rounds with a fresh
64 MiB payload, 4 MiB/s offered rate, one peer, window 16, warm file pages, five
seconds of warmup and two seconds of cooldown. The first seed is 20261026; the
follow-up uses 20261027. Daemons run on CPUs 0–1 and the probe on 2–3. All own
compilation and instrumentation finish before scoring. No profiler or extra
mid-transfer diagnostics run in scored transfers.

CPU covers first payload through completion, normalized by verified bytes. PSS
is the sampled whole-process peak from connection through transfer. The host is
shared; PSI is unavailable. Host snapshots are retained, and these observations
are not dedicated-host certification. Three rounds cannot qualify a candidate.
The predeclared triage rule requires both paired median resource ratios to be
at most 1.02 before a separate ten-round experiment with a new fixture seed.

Link ordering passed that triage rule (CPU ratio 0.985, PSS ratio 0.818). The
conditional `qualification-plan.json` was recorded before qualification began.
Ten fresh balanced randomized rounds then compared the exact input, exact ordered
binary and rTorrent using seed 20261028 and the same workload settings. All thirty
transfers verified. Each primary resource uses a 97.5% paired bootstrap interval,
targeting joint 95% coverage within a comparison, not across all exploratory
comparisons. Both upper bounds must be at most 1.02 for a joint pass.

## Results

The generated [result tables](result-tables.md) contain all absolute medians,
paired ratios and qualification intervals. All nine default-BOLT transfers,
twelve follow-up transfers and thirty qualification transfers verified. The
analysis retains the ten-round minimum, so a small exploratory screen cannot
accidentally return a qualification pass.

| Ten-round build | CPU-s/GiB | Peak PSS MiB | Peak anonymous MiB |
| --- | ---: | ---: | ---: |
| PGO input | 3.442 | 15.786 | 5.928 |
| PGO with function order | 3.493 | 13.193 | 6.414 |
| rTorrent 0.16.25 | 2.538 | 10.581 | 2.592 |

These are separate medians; decisions use matched-round ratios. Ordered versus
input PSS has a ratio interval [0.792, 0.880]. Ordered versus rTorrent has CPU
interval [1.212, 1.512] and PSS interval [1.183, 1.298]. The remaining gap is
established on this workload, even though CPU non-regression against the PGO
input is inconclusive. Do not combine these percentages with older unpaired
screens or multiply them by earlier PGO improvements.

All qualification payload rates were between 3.9998 and 4.0000 MiB/s. Median
first-payload delays were 1.683 ms for the input, 1.719 ms for the ordered build
and 1.682 ms for rTorrent. `qualification-service-level.json` retains the ranges.
This checks the offered workload; it does not qualify maximum throughput or
tail latency under other loads.

## Residency evidence

The default BOLT output adds 7,214,121 bytes to the ELF `size` text total. The
link-order build changes that total by only -112 bytes. File size or virtual
address space is not resident memory; the scored PSS and raw process maps are
the evidence for residency.

In the first default-BOLT post-transfer snapshot, the read-only executable data
mapping had 4,096 KiB `FilePmdMapped` and 4,096 KiB PSS, compared with 2,068 KiB
PSS in the input's corresponding mapping. Both the original executable code
segment and BOLT's additional code segment remained resident. This motivated
the regular-page and link-order follow-ups, rather than treating a smaller hot
section as sufficient evidence of lower whole-process RAM.

The first round's raw after-transfer maps for every arm are retained along with
parsed mapping records. These snapshots follow the scored transfer and are not
an attribution of its active peak. PSS, anonymous memory and huge-page fields
overlap; do not add them together.

## Reproduction and limits

The task-local orchestration scripts preserve the exact commands and original
scratch paths. Adapt those paths to an isolated clean checkout and use the
recorded training corpus and pinned tools. Rebuild the input with the validated
PGO profile and `-Clink-arg=-Wl,--emit-relocs`; instrument, train and merge before
optimizing or generating the order. The official
[LLVM BOLT documentation](https://github.com/llvm/llvm-project/blob/main/bolt/README.md)
describes these flags. Executables, instrumented binaries and the 23 MiB merged
profile are generated build outputs, not committed inputs.

For the linker path, pass
`-Clink-arg=-Wl,--symbol-ordering-file=/absolute/path/lld-function-order.txt`
on the final `cargo rustc` invocation. This experiment is source- and
compiler-specific; do not apply the archived order to another source/compiler
without retraining. Retain the executable manifest and rerun matched validation
for every candidate artifact. Clean PGO builds have not been demonstrated to be
byte-for-byte reproducible.

Raw summaries, training and functional reports, manifests, warnings, ELF layout,
symbol order and process maps are checksummed in `SHA256SUMS.json`. Failed
candidates remain recorded. No full-BOLT output is selected as a release default.

## Retained tooling and next evidence

`scripts/native_seed_function_order.py` provides training, generation from pinned
existing inputs, and validation. The PGO build script accepts its order as an
optional fourth argument and records the order provenance in the build manifest.
The production Rust source is unchanged. The generator reproduced the measured
6,185-symbol order exactly (SHA-256
`1ac0462fb81fc9356ef1299af8cc9e5ffb60b89af0dc6aa3c842393a50186e4f`). The tooling
and workflow suite passed 102 tests, including changed artifacts, stale source,
compiler/features, daemon-only training profiles, and real shell argument handling.

The updated build helper completed a real Rust 1.97 PGO release and verified all
6,185 ordered names in its output. That separately built binary
`1169c8d4de5e4018ee3da385e74d2f1156b7de323486d55d6ce4e436ad962a94` then verified
72 MiB across three random-request peers and five unaligned files after eviction
hints. Its manifest truthfully records a dirty tree because the tooling edits
were still uncommitted; all runtime hashes match the clean measured source.
This is functional build validation only. Its measurements are not scored and
it does not inherit the other artifact's performance results. Hosted CI is
separate from these completed local checks.

The most useful next combinations are feature-matched PGO plus this layout with
the existing optional mapped backing, followed by attribution of ready-range
hits and cross-thread wakeups. Those are hypotheses, not implemented policy or
proven wins. They require their own clean training, held-out matched runs and
multi-peer/multi-file checks. This experiment supports working on executable
residency; it does not justify reducing worker counts, weakening file validation,
or increasing retained payloads to force a memory or CPU score.
