# Native fix integration notes

Reviewed and integrated the supplied `TorrentNG-native-fix-7c2dd48.zip` into the checkout at `7c2dd481ca224da8f5e47b783678b6110146ba9b`.

## Changes integrated

- Added bounded upload-read coalescing in `crates/rt-engine/src/torrent_task.rs`. Staggered requests from a pipelined peer can accumulate for at most 8 ms, up to the existing four-block batch. Single-request traffic dispatches immediately; a peer that shrinks its request window recovers after one timeout flush. Choke controls and v2 uploads bypass the collector.
- Fixed overlapping read-ahead cache retirement in `crates/rt-storage/src/scheduler.rs`. Consumed full granules are recorded without a per-hit allocation, including out-of-order completions. Unread gaps stay cached, and returned `Bytes` retain their backing lease.
- Integrated benchmark report checkpointing and `NOT RUN` labels in `scripts/client_performance_benchmark.py`; repaired the mock import and added harness tests.
- Kept the supplied ZIP and extracted evidence under this report directory. The ZIP SHA-256 is `62a86fdcb2d5c2d7148e65c98cc81e7a62fa44641b0ee9f5e5aead371801a869`; its internal file checksums passed. The delivered patch matched its source provenance before formatting.

## Validation performed here

- `cargo test --quiet --locked -j4 -p rt-engine --lib`: **762 passed**.
- `cargo test --quiet --locked -j4 -p rt-storage --lib -- --test-threads=1`: **344 passed**. The first parallel storage run had four unrelated queue-capacity `QueueFull` failures; the complete serial rerun passed.
- `PYTHONDONTWRITEBYTECODE=1 python3 scripts/test_client_performance_benchmark.py`: **45 passed**.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

## Performance evidence and limits

The archive reports three matched diagnostic native-daemon runs on a staggered 32-request workload. Its CPU median fell from **15.8725 to 9.5933 CPU-s/GiB** (39.6%); storage read operations fell from **8,158 to 2,814**. Anonymous RAM did not improve: the median rose from **8.0273 to 8.4766 MiB**, with overlapping run ranges. Logical backend read volume rose about **5.8%**. The fixed and current runs completed in essentially the same 32 seconds because the receiver was rate-capped.

Those binaries were measured from `d57218d`, with source equivalence to this base documented in the archive; they were not newly built as exact-`7c2dd48` binaries. They used diagnostic GNU/system-allocator build settings, not the production musl/thin-LTO build. No current five-client comparison was run here. The measurements support this specific CPU bottleneck and fix; they do **not** show that TorrentNG now uses less total CPU or RAM than qBittorrent, Transmission, or Deluge.

See [README.md](README.md) for the full methodology, tradeoffs, and reproduction commands; raw evidence and checksums are preserved alongside it.
