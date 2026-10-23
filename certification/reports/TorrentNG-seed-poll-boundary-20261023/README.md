# Matched rTorrent and peer-poll experiment — October 23, 2026

**Decision: do not promote the polling boundary.** Neither its normal build nor
its PGO build passed the predeclared joint CPU/PSS non-regression gate. This is
inconclusive evidence of its effect, not proof of a universal regression. The
production Rust change was reverted; its patch, published source commit, build
identities and all fifty verified transfers remain available here.

The matched measurements establish that current main with PGO still trails
rTorrent on this workload: the paired median CPU ratio is 1.530 and PSS ratio is
1.632, with both adjusted intervals entirely above one. PGO itself improves both
resources versus the normal main build in this same experiment. Confidence is
high for these scoped observations; this is not general client certification.

## Method and attribution

The control is main `7836fa77f4bdd26ac78c5bf436e8d88968bf1f60`. The two retained
control binaries were built before the polling experiment; every one of their
258 runtime input hashes matches this main commit (`baseline-source-check.json`).
The candidate is published commit `97ac56ec850a1cd53e81f878cf6ea40d918d7ba0`.
Its fresh PGO training manifest records the published commit and full Git tree.
Both PGO builds use the same four-shape training corpus and Rust 1.97.0. Each
source has both a normal and a source-matched PGO build; this factorial comparison
applies to this polling change, not retrospectively to the seven earlier changes.

The native comparator is rTorrent 0.16.25 at
`15ccae27deb9aa1dc8e46ea0ed218a673ccc5732`, linked to libtorrent 0.16.25 at
`6ad3fecf3e7e7aa86855445587b532a91256f344`. Both clean source trees, linked-library
hashes, compiler version and configure logs are retained. It uses GCC 13.3,
`-O2 -DNDEBUG`, XML-RPC and the same source/configure choices as the repository's
comparator Dockerfile. This is a native Ubuntu build, not that Debian image.
Do not substitute the older 8.119 MiB rTorrent screen for this matched control.

`experiment-plan.json` was written before measurement: ten balanced randomized
rounds, five treatments, 64 MiB deterministic payload, fixture seed 20261025,
4 MiB/s offered load, one peer, a 16-request window and warm file pages. Every
fresh daemon had five seconds of warmup and two seconds of cooldown. Daemons
were pinned to CPUs 0–1 and the probe/leecher to CPUs 2–3. Own compilation and
tests finished before scoring. No profiler or mid-transfer diagnostics ran in
scored transfers. Host/VM/cgroup snapshots and all resource samples are in the
raw summary. PSI is unavailable here; missing counters are not evidence of an
uncontended host. This is not a dedicated-host certification run.

CPU is precise process CPU from first payload through completion, normalized by
verified bytes. Peak PSS includes connection and transfer samples. Neither is
a startup cost or a complete kernel/cgroup memory bill. The 2% tolerance and ten
rounds were fixed in advance. Within each comparison, bootstrap intervals are
97.5% for each primary resource, targeting joint 95% coverage for the two; they
are not a family-wide confidence claim over all exploratory comparisons.

## Results

These absolute values are medians; decisions use paired-round ratios below.

| Build | CPU-s/GiB | Peak PSS MiB | Peak anonymous MiB | Payload MiB/s |
| --- | ---: | ---: | ---: | ---: |
| main | 4.354 | 19.583 | 6.473 | 4.000 |
| main-pgo | 3.480 | 17.384 | 5.902 | 4.000 |
| poll | 4.078 | 19.816 | 6.486 | 4.000 |
| poll-pgo | 3.368 | 16.640 | 6.271 | 4.000 |
| rtorrent | 2.336 | 10.694 | 2.592 | 4.000 |

| Control → candidate | Paired CPU ratio [interval] | Paired PSS ratio [interval] | Joint gate |
| --- | ---: | ---: | --- |
| main → poll | 0.995 [0.803, 1.045] | 0.994 [0.974, 1.039] | inconclusive_or_regression |
| main-pgo → poll-pgo | 1.003 [0.816, 1.119] | 0.947 [0.905, 1.032] | inconclusive_or_regression |
| main → main-pgo | 0.802 [0.748, 0.898] | 0.880 [0.844, 0.938] | pass |
| rtorrent → main-pgo | 1.530 [1.319, 1.668] | 1.632 [1.556, 1.707] | inconclusive_or_regression |
| rtorrent → poll-pgo | 1.481 [1.325, 1.556] | 1.568 [1.544, 1.606] | inconclusive_or_regression |

A pass permits non-regression within 2%; it is not automatically an improvement.
The polling change's PGO median PSS reduction alone does not establish a joint
CPU/RAM benefit. The original earlier report's PGO result also remains a scoped
build improvement, not rTorrent parity.

## What the experiment tested

The previous PGO diagnostic attributed 11,000 KiB of post-transfer PSS to the
exact executable mapping: 8,276 KiB executable text, 2,048 KiB read-only pages,
and 676 KiB relocated/writable anonymous pages. These categories overlap with
anonymous-memory totals; never add PSS and anonymous memory. This is a diagnostic
snapshot, not attribution of the active peak. Its raw smaps and parsed mapping
records are retained here.

Symbol inspection found repeated large peer-loop bodies inside PGO-generated
connection/task pollers. The patch used a concrete non-inlined poll function,
without allocation, dynamic dispatch, new scheduling or changed ownership.
The PGO ELF text-size total fell by 276,796 bytes; the normal image barely changed.
That binary result did not translate into a qualified CPU/PSS win.

The Rust Reference describes inline attributes on async polling, but the pinned
1.97.0 compiler still inlined the minimal async example retained here. An explicit
ordinary poll function made the intended boundary observable in the output.
See the [compiler issue](https://github.com/rust-lang/rust/issues/129347) and
[language reference](https://doc.rust-lang.org/reference/attributes/codegen.html).
The local generated IR is the evidence for this compiler, not an assumption about
all future Rust versions.

rTorrent's pinned `PeerConnection::event_write` and `PeerConnectionBase::up_chunk`
use a compact event-driven write state machine, retaining the chunk and updating
its offset after successful writes. Its chunk manager accounts shared mappings
and bounds reclamation work. TorrentNG already preserves the analogous lifetime,
credit-before-payload and bounded batching invariants. The current evidence directs
further experiments toward executable residency and event-loop work rather than
assuming another buffer reduction will close the full gap.

## Provenance and validation fixes retained

`pgo-commit-reconciliation.json` establishes that historical local training commit
`f8772ff` and published `a826610` have the identical tree
`91051f264644241d37150143ef95d00babfd3ea2` and all 258 matching input hashes. Commit
metadata changed during GitHub publication. The historical manifest is untouched.
New manifests record tree identity and separately built dependency sources, and
benchmark startup verifies recorded shared libraries have not changed.

The prior PR's queued release-note job fetched main after merge, causing an empty
three-dot diff and a false missing-fragment failure. Validation now uses the PR
event's base SHA. A real temporary-repository test merges the PR before executing
the workflow shell and verifies the fragment is still found.

The candidate passed 770 default engine tests, 770 mmap-enabled engine tests,
feature-enabled all-target engine Clippy, formatting and a release/PGO build.
The final provenance/probe/release-note/workflow suite passed 92 Python tests;
OpenAPI validation passed. These checks did not override the failed resource
gate. Hosted CI status must be checked separately from these local results.

## Reproduction

Preserved artifacts include the unpromoted patch, original training manifests,
paired raw summary, analysis, comparator build records, validation logs and
code-size/smaps diagnostics. Executables and generated profiles are build outputs;
regenerate them from the pinned source and four-shape corpus. Recompute a result:

```sh
gzip -dc paired-summary.json.gz > /tmp/poll-summary.json
python3 ../../../scripts/seed_efficiency_tools.py analyze \
  --summary /tmp/poll-summary.json --control main-pgo --candidate poll-pgo \
  --minimum-rounds 10 --tolerance 0.02 --output /tmp/poll-analysis.json
```

The normal and PGO application defaults remain unchanged by this experiment.
