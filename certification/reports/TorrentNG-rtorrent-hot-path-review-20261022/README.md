# rTorrent hot-path review archive

The user supplied `TorrentNG-rtorrent-hot-path-review.zip` for evaluation. Its
SHA-256 is
`c882874d661727cad0f17a94c43828c0a691541ad1e8dc52e910fe675ddfb2ef`.

The archive is retained here with its extracted source snapshot. The four
reviewed Rust files (`torrent_task.rs`, `scheduler.rs`, `frame.rs`, and
`upload_scheduler.rs`) are byte-for-byte identical to commit `7ba8bb6a`:

- `crates/rt-engine/src/torrent_task.rs`
- `crates/rt-storage/src/scheduler.rs`
- `crates/rt-storage/src/frame.rs`
- `crates/rt-engine/src/transfer/upload_scheduler.rs`

The archive therefore contributes no new code delta. Its file-range upload
recommendation had already been evaluated separately: `sendfile` lowered
anonymous memory but raised CPU cost substantially, so it remains an
experiment in the comparison record rather than a production change. The
current work uses the archive as provenance and continues to target repeated
per-block CPU and memory costs with matched, verified whole-client screens.
