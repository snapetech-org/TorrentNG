# rTorrent upload hot-path review

## Input

- Original archive: `TorrentNG-rtorrent-hot-path-review.zip`
- Original Downloads path: `/home/keith/Downloads/TorrentNG-rtorrent-hot-path-review.zip`
- Archive SHA-256: `c882874d661727cad0f17a94c43828c0a691541ad1e8dc52e910fe675ddfb2ef`
- Contents: four Rust source snapshots; no patch, README, or benchmark payload.

## Source comparison

The extracted files are byte-for-byte identical to the reviewed repository source at `d2f7269364572403714b7352248c1e1812a89d2f`:

| Archive file | Repository file | SHA-256 |
| --- | --- | --- |
| `torrent_task.rs` | `crates/rt-engine/src/torrent_task.rs` | `d540bb7e4e0e350a380793d3efb1562a494b527f33b27546bc262f77b532ef69` |
| `scheduler.rs` | `crates/rt-storage/src/scheduler.rs` | `e8a0bbed6abb40c8b2b428cd19d26137f16359ad648be2059761587644014400` |
| `frame.rs` | `crates/rt-storage/src/frame.rs` | `4657f38c240631151f94b79fdc7f8de21866dcd94dfd207a863b9778fbbcdd83` |
| `upload_scheduler.rs` | `crates/rt-engine/src/transfer/upload_scheduler.rs` | `21eae6dcdda9387b8c44b9c2646c15c05ae6ce288d9b1a821dae4ca6865b4d22` |

There was no source delta in this archive to apply. The recommendation is still relevant: an eligible verified seed response may avoid the generic read scheduler and owned payload buffer by holding a file lease and sending a file range directly to a TCP peer. The current buffered path must remain the fallback for unsupported transports or ranges. The production work is tracked separately under the Native seed file-range upload fast path initiative.

The candidate must preserve request validation, upload-rate admission and settlement, piece-frame ordering, cancellation, connection ownership, and bounded worker responsiveness. A direct file operation must not block a Tokio worker. Correct wire-transfer and fallback tests come first; CPU and anonymous-memory effects must be measured in a matched source-bound whole-client seed comparison before claiming an improvement.

## Validation status

No production source changes came from this archive. Its extracted source files were verified against the reviewed commit. The comparison harness had a separate reproduced setup bug: after resetting OpenTracker, it trusted qBittorrent's cached tracker status instead of the live scrape. See `../TorrentNG-native-download-efa3a607/comparison/round1-invalid-stale-seeder.md` for the preserved invalid run and its evidence.
