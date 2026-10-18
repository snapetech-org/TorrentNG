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

## Status: Implemented — TorrentNG client support
