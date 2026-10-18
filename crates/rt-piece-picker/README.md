# rt-piece-picker

Piece selection for the TorrentNG client:

- rarest-first peer selection by default
- randomized tie-breaking within the rarest available group
- array-backed availability buckets and an incrementally maintained fresh-work candidate index
- priority pieces before normal selection, used for first/last file pieces and preview workflows
- sequential mode for client compatibility and streaming-oriented downloads
- configurable sequential start piece for Transmission-style from-piece behavior
- endgame duplicate requests after fresh work is exhausted

Rarest-first selection samples buckets from lowest availability upward. Small
buckets are fully checked after bounded probes miss. For a large bucket, the
engine's packed peer bitfield visits its set pieces directly and
reservoir-samples the rarest requestable matches, avoiding a per-candidate
bitmap check for sparse peers. Incoming and departing peer bitfields also
skip empty bytes and update only their announced set pieces.

Priority pieces always win before sequential or rarest-first ordering. Sequential mode walks from the configured start piece to the end, then wraps to lower pieces so a torrent still completes. Endgame uses the same effective piece order as normal picking. Completed, disabled, and fully requested pieces leave the fresh-work index; cancellation or request reset restores them when a new request can be issued. The admission estimate includes bucket membership, candidate positions, vector capacity slack, and the configured global peer cap; `PiecePicker::memory_bytes` reports current vector capacities.

Each in-progress piece keeps a cursor to its first block that may still be requested. Normal request fill advances through the piece without rescanning the requested prefix; cancellation rewinds the cursor when an earlier block becomes requestable, while restore and disconnect reset it around received blocks.

Partial-piece state stores received-block flags as a one-bit-per-block bitmap and maintains an O(1) received count for completion checks. Fastresume snapshots and byte-progress calculations walk set bits directly; the admission estimate and live memory report include the packed bitmap capacity.

## Picker performance probe

The `benches/perf.rs` probe reports request-selection plus cancellation and
availability bitfield add/remove time at 1k, 12k, and 100k pieces. It
prebuilds partial state up to the picker's 16,384-piece in-progress cap so the
timed selection loop does not include first-use state allocation. The
availability probe compares empty, one-in-64 sparse, and near-dense bitfields
with membership moves between nonempty rarity buckets. At 100k pieces the
picker probe also uses a sparse peer bitfield. Results are diagnostic
measurements, not timing thresholds or pass/fail tests. Run them during the
PERF-111 bulk probe pass:

```sh
cargo bench -p rt-piece-picker --bench perf
```

Keep the CSV together with the tested commit, Rust toolchain, and host details.

## Status: Implemented — TorrentNG client support
