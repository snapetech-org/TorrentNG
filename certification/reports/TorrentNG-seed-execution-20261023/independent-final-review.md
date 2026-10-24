# Independent final review — 2026-10-24

**Conclusion: the memory improvement is supported; simultaneous CPU/PSS
non-regression and rTorrent parity are not. No experimental configuration is
qualified for performance promotion. No known functional blocker was found.**

This review was read-only apart from this file. Analysis ran on CPUs 4–5;
no builds, benchmarks, runtime edits or changes to selection were made.

## Qualification and provenance

I checked all 50 verified runs in `qualification-summary.json.gz`, independently
reconstructed every within-round resource ratio and its median from raw run
values, and reproduced all seven stored confidence intervals and gate decisions
with the checked-in scoring implementation. The sample consists of ten paired
rounds per configuration, not 50 independent candidate/control pairs. All
per-run binary identities and runtime settings agree with their treatment
manifests. No run changed binary identity within a treatment.

The final source is `589b97a42938b0884f241d76711e2d7e5e8d75ff`, build-input digest
`347b31f6d7db9ae52931bdb349aca27289a22f868aa215d2f6227141bd10d7bf`.
The ordered binary is
`4f6fc030f19589be5cd6ba71d7f87cc347ac0ed5a33de3a95ef6c08921dfb6d8`.
I checked the archived decompressed profile hash
`64253846872def473a80a8267c02a22231249c4039448bb75ae3188455a195c9`
and function-order hash
`eb86a6c364f3e7e55fcd73fb397062fee37a66a6e9a45924541cdff78b3204ac`
against their manifests. Buffered primary and mapped secondary use that same
binary and scoped scratch setting; mapped activation is their runtime
difference. Their identities and selection were recorded before qualification.

The primary/baseline paired CPU ratio is **1.00230 [0.89504, 1.06227]**;
PSS is **0.71537 [0.69790, 0.73939]**. This supports a 28.46% paired-median PSS
reduction, but neither a CPU improvement nor the required 2% CPU
non-regression. A confidence interval crossing equality does not prove either
equivalence or regression.

Against the previous selected configuration, primary CPU is
1.07097 [0.87691, 1.12895] and PSS is 0.91733 [0.89267, 0.93960]. Against
rTorrent, primary CPU is **1.47972 [1.35561, 1.53569]** and PSS is
**1.09217 [1.07603, 1.11924]**: both remaining gaps are established in this
workload. The mapped secondary also remains worse than rTorrent:
CPU 1.40874 [1.36415, 1.61462], PSS 1.05156 [1.04056, 1.07288].
Secondary/primary PSS improves, but CPU non-regression is unresolved; it must
not replace the preselected primary after qualification.

These are medians of paired ratios, not ratios of separate medians. Bootstrap
intervals target Bonferroni joint 95% coverage for CPU/PSS within each comparison;
they do not correct the entire family of comparisons. Setup/recheck CPU
intervals are secondary/descriptive and include readiness/diagnostic work.
None establishes a setup-CPU benefit. Configuration comparisons do not isolate
the effects of individual refactors: optimization level, profile, ordering,
source and runtime settings differ across the relevant controls.

## Completed held-out service evidence

All **28 client-runs across seven shapes completed and verified**, including
all recorded individual peer checksums. Their binary identities match their
manifests. Shapes cover sequential paced service, four peers, random high-rate
requests, a one-request window, cold-file-page hints with three peers, random
unaligned five-file transfers, and an upload limit.

No correctness failure or gross new throughput failure was observed. Relevant
limits and exceptions must remain visible:

- At random 64 MiB/s offered load, TorrentNG delivered approximately
  63.45–63.48 MiB/s and rTorrent 64.00 MiB/s in this single round.
- Four-peer maximum first-payload delays were 17.891 ms primary, 12.108 ms
  previous selection, 4.077 ms mapped and 4.579 ms rTorrent. The five-file
  random/cold-hint case reached 30.481 ms mapped and 26.459 ms rTorrent.
  These observations do not establish latency non-regression or tail bounds.
- At window 1 and 1 MiB/s offered load, TorrentNG delivered approximately
  1 MiB/s; rTorrent delivered 0.3545 MiB/s with 41.528 ms first-payload delay.
  This is an observed service distinction, not general comparator inferiority.
- With a configured 1 MiB/s upload limit and a 16 MiB transfer, TorrentNG
  averaged about 1.0072 MiB/s and rTorrent 1.2665 MiB/s over the payload window.
  Do not call all clients strictly rate-equivalent or infer the cause from
  this short run; startup credit and rate semantics were not isolated.
- `drop-file-pages` is a cache hint, not proof of cold physical storage.
  One round per shape is correctness/service screening, not broad resource
  or service non-inferiority certification.

## Merge and promotion decision

Keep scoped scratch, mapped backing, opt-level 3 PGO and function-order recipes
opt-in. The performance-promotion gate failed. Functional coverage and the
reviewed ownership/accounting contracts support retaining the implementation
and evidence; they do not establish normal default-build CPU performance.
The optimized bundle is not an isolated before/after qualification of default
refactors. This distinction is not a blanket prohibition on merging source:
no known correctness/security/ownership blocker was found in the reviewed
changes, but merge must not be described as performance certification.

The actual-daemon witness corroborates completion of a 128 MiB recheck and
cancellation at 72/128 pieces, followed by three fresh zero-frame/zero-active-job
samples. The earlier harness job-kind error is preserved and excluded. The
witness is observational, not a deterministic cancellation timing test.
Mixed download/recheck default promotion remains unqualified, and the reported
Rust AppState move-compatibility change and unsupported unmap-refusal policy
remain applicable limitations.

The updated README's qualification numbers, paired interpretation, source
separation, frozen selection and reproduction instructions agree with this
review. Its temporary held-out “being finalized” wording should be updated
once the complete service evidence is archived. Preserve the short-transfer
upload-limit distinction above in that final service summary. No numerical
correction to the qualification tables is needed. The shared-host, single-peer
qualification does not certify every deployment or every rTorrent build.

Publication note: the complete service summaries are now archived; the README
status and service table have been finalized, including the upload-limit
difference. The review findings above are unchanged.
