# rt-piece-picker

Piece selection for the TorrentNG client:

- rarest-first peer selection by default
- randomized tie-breaking within the rarest available group
- array-backed availability buckets and an incrementally maintained fresh-work candidate index
- priority pieces before normal selection, used for first/last file pieces and preview workflows
- sequential mode for client compatibility and streaming-oriented downloads
- configurable sequential start piece for Transmission-style from-piece behavior
- endgame duplicate requests after fresh work is exhausted

Priority pieces always win before sequential or rarest-first ordering. Sequential mode walks from the configured start piece to the end, then wraps to lower pieces so a torrent still completes. Endgame uses the same effective piece order as normal picking. Completed, disabled, and fully requested pieces leave the fresh-work index; cancellation or request reset restores them when a new request can be issued. The admission estimate includes bucket membership, candidate positions, vector capacity slack, and the configured global peer cap; `PiecePicker::memory_bytes` reports current vector capacities.

## Picker performance probe

The `benches/perf.rs` probe reports request-selection plus cancellation time at
1k, 12k, and 100k pieces. It prebuilds partial state up to the picker's
16,384-piece in-progress cap so the timed loop does not include first-use
state allocation. At 100k pieces the peer bitfield covers that admitted subset,
which also probes sparse-peer selection. The result is a diagnostic
measurement, not a timing threshold or a pass/fail test. Run it during the
PERF-111 bulk probe pass:

```sh
cargo bench -p rt-piece-picker --bench perf
```

Keep the CSV together with the tested commit, Rust toolchain, and host details.

## Status: Implemented — TorrentNG client support
