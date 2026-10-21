# Linux TCP file-range seed experiment

## Decision

The candidate at `e9727943033850c6258d282bb0a54311cf9d05b0` was measured and
then reverted by `1044556cfeb8aea090feb2d1b879bf14b79636fa`. Keep its memory
result as design evidence; do not treat the candidate as the current engine or
as a whole-client win. The experiment reduced sampled anonymous memory, but
its single measured pass used substantially more CPU per GiB.

## What the candidate changed

On eligible verified, single-file TCP v1 seed requests, the candidate held a
file-range lease and sent the Piece header and file data with Linux `sendfile`.
Other transports and ranges used the existing buffered path. The goal was to
avoid routing each payload through the general storage read and an owned upload
buffer, while preserving request validation, rate accounting, ordering,
cancellation, and connection ownership.

## Comparison

The candidate completed a controlled private-tracker seed transfer of the
792,723,456-byte Debian 13.7 netinst ISO. All five leecher payloads passed the
same pinned SHA-256 check. The receiver was capped at 4 MiB/s, and the tested
clients ran sequentially on the shared `kspld0` host.

| Client | Average upload (MiB/s) | CPU (s/GiB) ↓ | Peak anonymous memory (MiB) ↓ |
|---|---:|---:|---:|
| TorrentNG candidate (`e9727943`) | 3.89 | 5.51 | **4.3** |
| qBittorrent | 3.86 | 2.45 | 6.2 |
| Transmission | 3.75 | 1.45 | 14.8 |
| Deluge | 3.88 | 2.39 | 82.7 |
| rTorrent | 3.89 | **0.55** | **1.0** |

The earlier clean-source baseline at `cfb63f91` recorded TorrentNG at 3.88
MiB/s, 1.33 CPU-s/GiB, and 16.6 MiB anonymous peak. Comparing those two
single-pass observations, candidate CPU was 4.14 times the baseline value
(314% higher), and candidate anonymous memory was 74.1% lower, with effectively
unchanged transfer rate. In its own pass, candidate CPU was higher than every
reference client; its anonymous peak was lower than qBittorrent, Transmission,
and Deluge, but higher than rTorrent.

These measurements preserve both sides of the result. The candidate has a
clear memory signal worth factoring into later designs, and a CPU cost that
does not meet the goal of improving on the other clients. Each comparison was
one pass per client on a shared host, and the candidate run's sampled 1-minute
load average peaked at 35.46. Therefore, the observed cross-run CPU and memory
deltas are exploratory and do not isolate causality or establish a general
ranking. The common receiver cap also means these rates are not maximum-upload
capacity measurements.

## What to carry forward

- Preserve the reduction in anonymous payload-buffer ownership as a design
  objective. Lower anonymous memory does not mean lower total resident data;
  file-backed pages can still be charged as page cache.
- Treat CPU cost per GiB as a first-class acceptance measure alongside memory.
  A future file-range or lease-based design should reduce per-request work and
  retain the existing cancellation, ordering, rate, and path-safety guarantees.
- Re-run the same verified whole-client workload with rotated client order
  and repeated trials before accepting another implementation. Keep the
  raw samples and memory profile so both resource changes remain visible.

## Candidate validation and source state

The experimental source passed formatting, clippy, release compilation, and
serial library tests for `rt-engine` (769 tests) and `rt-storage` (350 tests).
The backpressure regression also verified 16 ordered Piece frames with a
constrained socket buffer and a delayed reader. These checks validate the
candidate implementation and its wire behavior; they do not offset its CPU
result. The current `main` source has the candidate changes reverted.

## Evidence

- [Full benchmark report](controlled-seed-file-range-e9727943.md)
- [Time-series samples](controlled-seed-file-range-e9727943-samples.csv)
- [Memory profile](controlled-seed-file-range-e9727943-memory-profile.json)
- [Runner output](controlled-seed-file-range-e9727943-console.log)
- [Pre-experiment baseline](../round2-corrected-cfb63f91/README.md)

`SHA256SUMS` verifies the copied benchmark report and raw evidence files.
