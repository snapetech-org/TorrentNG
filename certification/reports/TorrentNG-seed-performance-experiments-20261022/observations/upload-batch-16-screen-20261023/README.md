# Sixteen-block upload-read screen — 2026-10-23

**Decision: do not promote or expand.** The batch-16 candidate lost CPU and memory in every paired 16 MiB screen. The transfer was verified in all six runs; the current batch-8 executable and source remain unchanged.

## Paired result

| Treatment | Median CPU-s/GiB | Peak PSS | Peak anonymous |
|---|---:|---:|---:|
| Current, batch 8 | 3.655 | 18.051 MiB | 5.445 MiB |
| Candidate, batch 16 | 4.122 (+13.9%) | 18.535 MiB (+2.7%) | 5.973 MiB (+9.7%) |

Paired deltas were CPU +22.2%, +13.9%, +7.1%, PSS +2.7%, +1.5%, +8.5%, anonymous memory +9.7%, +4.1%, +26.6%. The candidate lost all nine pair-metric comparisons. These were 16 MiB screens, not a whole-client rTorrent result. Host pressure fell during the test; see [`host-load.txt`](host-load.txt).

## Why the larger cap did not help

The debug-only current trace formed 128 full eight-block reads for 1,024 requests. The candidate trace formed 126 reads for the same requests (1.6% fewer), averaging 8.13 requests per read; only two reads reached 16 blocks. This explains why the larger cap barely reduced storage operations while increasing peak process memory. The traces are diagnostic and are not performance scores; exact histograms are in [`batch-distribution.json`](batch-distribution.json).

## Candidate and validation

The isolated patch raises the maximum upload batch and per-connection admission bound from 8 to 16, tying pending upload reads to the same constant. The 256 KiB maximum batch remained bounded. The 769-test engine suite and release build passed after adapting the old eight-job test assumptions. No source changes were promoted.

Current executable: 24,663,632 bytes, SHA-256 `b13f157809d98bdc052017f83bd2369685f7221116d8269837120f6af07cd263`. Candidate: 24,664,448 bytes, SHA-256 `d021ed061ba6e1e551d67c0723ea3118de8c4c338147ca99ada31239cfee4551`. Exact candidate diff is [`candidate.patch`](candidate.patch); paired samples and identities are in [`summary.json`](summary.json), with build/test commands in [`validation.txt`](validation.txt).
