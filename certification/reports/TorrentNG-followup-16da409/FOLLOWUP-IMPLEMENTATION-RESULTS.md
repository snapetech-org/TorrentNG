# Results for the 16da409 follow-up review

The supplied review archive is saved at [TorrentNG-followup-16da409.zip](TorrentNG-followup-16da409.zip) and extracted beside it. The original package members and checksums are preserved.

The four reproduced findings were fixed in commit `1564fa32`, now on `main` at `e4c01bc5`:

- **Batch admission:** job and byte permits are acquired all at once. A batch no longer holds partial permits while waiting for other work to release capacity.
- **Cache retirement:** consumed ranges are tracked so a tail-first read cannot evict unread bytes from the same window.
- **Piece backing accounting:** Piece views retain the charge for their full shared backing allocation until the final view is released.
- **Batch formation:** the peer loop drains up to four immediately-ready requests, reuses validated file spans, and preserves per-request responses and cancellation. A short deadline still flushes small batches.

Validation passed on the implementation source:

- `cargo test --locked -j4 -p rt-engine --lib`: **762 passed**.
- `cargo test --locked -j4 -p rt-storage --lib`: **340 passed**.
- `cargo test --locked -j4 -p rt-peer-wire --lib`: **59 passed, 1 ignored**.

## Whole-client measurement

A fresh Docker transfer benchmark was built from the candidate worktree recorded as source commit `5a44498e` and source fingerprint `e753fba8fcd84987bae7efc6d1f220fc09927905`. TorrentNG's public Debian ISO download reached 98.886%, then stalled at zero peers. I stopped that run after it made no progress. It did not reach the controlled seed phase, so it produced no upload-batching distribution or client comparison. The benchmark report is explicitly marked **incomplete**; its CSV and memory profile contain partial TorrentNG download samples only:

- [Incomplete benchmark report](../TorrentNG-client-benchmark-20261021T152853Z.md)
- [Partial samples](../TorrentNG-client-benchmark-20261021T152853Z-samples.csv)
- [Partial memory profile](../TorrentNG-client-benchmark-20261021T152853Z-memory-profile.json)

This supplies no new whole-client CPU or RAM result and does not establish that TorrentNG beats qBittorrent, Transmission, or Deluge. The previous complete comparison and its limits remain in the imported review evidence. A useful next comparison needs a verified local seed, a matched private upload run across clients, observed batch sizes, and whole-client CPU, memory, and verified goodput.
