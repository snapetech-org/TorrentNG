# Performance research

This index separates measured results from hypotheses and keeps the exact
build/configuration links close to each claim. Lower CPU and memory are better.
No tested configuration has established CPU and RAM parity with rTorrent.

## Current best evidence

| Evidence and configuration | TorrentNG result | Comparator result | Finding and links |
|---|---|---|---|
| **Latest five-client release-default screen** — 2026-10-24, three randomized 64 MiB local-seed rounds; system allocator, defaults, no PGO or seed-mmap | 4.17 CPU-s/GiB whole-window; 3.18 active; 7.2 MiB peak cgroup anonymous; 16.5 MiB median PSS when sampled | rTorrent 0.16.26: 1.12 whole-window; 0.85 active; 3.1 MiB anonymous; 18.7 MiB PSS. qBittorrent: 2.04 CPU / 6.4 MiB anon. Transmission: 2.09 / 17.5. Deluge: 3.22 / 82.8. | All 15 transfers verified. Small shared-host screen; substantial I/O pressure. [Current comparison](current-client-comparison.md), [exact run config](../../certification/reports/TorrentNG-current-client-comparison-20261024/run-config.md), [raw evidence](../../certification/reports/TorrentNG-current-client-comparison-20261024/README.md) |
| **Best repeated scoped PGO candidate against rTorrent** — 2026-10-23, ten balanced paired rounds, 50 verified transfers; scoped ordered PGO with buffered file reads | 3.409 CPU-s/GiB; 11.525 MiB peak PSS; 4.271 MiB anonymous | rTorrent 0.16.25: 2.439 CPU-s/GiB; 10.533 MiB PSS; 2.590 MiB anonymous | Paired median ratios: CPU 1.480 [1.356, 1.536], PSS 1.092 [1.076, 1.119]. Joint parity gate failed on both. PGO remains opt-in. [Frozen config](../../certification/reports/TorrentNG-seed-execution-20261023/selected-candidate.json), [qualification plan](../../certification/reports/TorrentNG-seed-execution-20261023/qualification-plan.json), [paired evidence and limitations](../../certification/reports/TorrentNG-seed-execution-20261023/README.md), [independent review](../../certification/reports/TorrentNG-seed-execution-20261023/independent-final-review.md), [Astra architecture review](../reviews/2026-10-23-astra-architecture-review.md) |
| **Mapped PGO secondary** — same ten-round qualification and source | 3.530 CPU-s/GiB; 11.105 MiB peak PSS; 3.438 MiB anonymous | Same rTorrent comparator: 2.439 CPU-s/GiB; 10.533 MiB PSS; 2.590 MiB anonymous | Paired ratios 1.409 CPU and 1.052 PSS; also failed parity. Mapped backing is an opt-in alternative, not the selected candidate. [Config and paired results](../../certification/reports/TorrentNG-seed-execution-20261023/README.md) |
| **Earlier compact release-default comparison** — 2026-10-22, different fixture, source, and measurement window | 1.90 CPU-seconds and 19.82 MiB PSS | rTorrent 0.16.25: 0.73 CPU-seconds and 10.74 MiB PSS | PSS improved 18.3% against the earlier TorrentNG configuration; CPU did not. Historical result, not directly comparable with the current per-GiB screens. [Summary](../../certification/reports/TorrentNG-CPU-RAM-final-summary-20261022.md), [full measurements](../../certification/reports/TorrentNG-native-cpu-ram-20261022.md) |

The current five-client table reports cgroup anonymous memory for every
client. Its PSS column is secondary and available only for TorrentNG and
rTorrent. Anonymous memory, RSS, PSS, file cache, and total cgroup memory are
different categories and should remain separate.

## Retained improvements and trade-offs

- The latest retained release-default upload batching result uses a 32 ms
  coalescing window and eight-block batches. Its randomized three-round
  comparison reduced CPU 22.2%, PSS 1.5%, and anonymous peak 7.5% versus the
  8 ms baseline. [Exact experiment and artifacts](../../certification/reports/TorrentNG-seed-performance-experiments-20261022/README.md#current-main-improvement-bounded-eight-block-reads).
- The October 23 architecture and ownership work lowered the paired ten-round
  CPU median by 6.6% without PGO and 25.0% with PGO against its previous-main
  controls, while lowering PSS by 5.8% and 22.3% respectively. Those are
  within-TorrentNG improvements, not rTorrent parity. See the
  [qualification report](../../certification/reports/TorrentNG-seed-execution-20261023/README.md)
  and [independent runtime review](../../certification/reports/TorrentNG-seed-execution-20261023/independent-runtime-review.md).
- The Linux file-range/sendfile seed prototype lowered anonymous peak but
  raised CPU materially; it was reverted. Its CPU loss and memory gain are
  preserved in the [candidate report](../../certification/reports/TorrentNG-native-download-efa3a607/comparison/round3-sendfile-e9727943/README.md).
- One storage worker, allocator arena caps, smaller read-ahead windows, mapped
  backing, and other candidates have mixed or workload-specific results.
  Their gains and regressions remain in the
  [experiment ledger](performance-history.md) and
  [detailed benchmark archive](client-performance-benchmark-archive.md).

## Methods and reviews

- [How to run the client benchmark](../CLIENT_PERFORMANCE_BENCHMARK.md)
- [Historical performance ledger moved out of the root README](performance-history.md)
- [Archived detailed benchmark measurements](client-performance-benchmark-archive.md)
- [Backend performance claims and measurement audit](../BACKEND_PERFORMANCE_BENCHMARKS.md)
- [Performance scale-claims audit](../PERFORMANCE_SCALE_CLAIMS_AUDIT.md)
- [Refactoring performance notes](../REFACTORING_PERFORMANCE.md)
- [Astra architecture review](../reviews/2026-10-23-astra-architecture-review.md)
- [PR #233: measured native-engine work](https://github.com/snapetech-org/TorrentNG/pull/233)
- [PR #235: comparison harness and results](https://github.com/snapetech-org/TorrentNG/pull/235)

The archived reports preserve rejected candidates and reversions so later
work can reuse their measured gains without repeating known losses.
