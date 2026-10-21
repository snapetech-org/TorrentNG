# TorrentNG reassessment implementation results

Updated 2026-10-21 from baseline revision `9e1c02925d97a690f4facceeebd23cbf99a8372a`. The focused local test counts below came from a working tree based on that commit, not an immutable source revision; treat them as historical diagnostics, not release evidence. The final pushed `main` SHA and its hosted checks remain authoritative. The imported Astra review concerns `db9e65ac`; the supplied ZIP and extracted source, probes, reports, and checksums are retained beside this note.

## Work completed

- Upload reads are grouped for up to four already queued, contiguous v1 blocks from the same piece and file, capped at 64 KiB. One storage read backs separate legal Piece payloads; each request keeps its own obligation and rate settlement, while the shared backing charge lasts until the final block view drops. The current main branch already contains this read-batching work.
- Ready TCP Piece frames are written in a bounded vectored sequence with stack-backed headers and cursors. Partial writes settle only complete frames; incomplete rate grants are refunded when their blocks drop. uTP keeps its existing sequential framed-send path.
- A forward cache range is removed when a hit consumes its final bytes. Returned `Bytes` views still own the backing charge until their consumers release them.
- Large decoded Bitfield, Extended, and Hashes payload copies now carry their `PeerBuffer` charge with the message. Message clones share the payload; unwrapping a message while another clone is live reserves a separate charge for the resulting copy. The connection retains only its read-buffer reservation after trimming.
- The bounded 512 KiB frame class is already present in current main (`5323d735`), with a two-idle-frame cap for that large class and the existing global idle limit.

## Validation

- `rt-engine`: 758 unit tests passed.
- `rt-storage`: 339 unit tests passed.
- `rt-peer-wire`: 57 passed, 1 ignored (the existing local benchmark test).
- `rt-metrics`: 12 unit tests passed.
- Focused checks also confirmed one storage read for two contiguous Piece responses, correct payload slices and accounting release, valid vectored frames across partial socket writes, complete-frame reporting after a write error, consumed-window retirement, and codec credit release through clones and codec shutdown.

## Measurements and limits

The pool probe was run against a clean `58c7fb59` worktree with the capped 512 KiB candidate. This predates the current `9e1c0292` head; its candidate corresponds to the pool change later committed as `5323d735`.

| 256 MiB storage probe | Baseline | Capped 512 KiB pool |
|---|---:|---:|
| Median CPU-s/GiB | 0.5780 | 0.5839 |
| Allocated bytes | 276,521,600 | 25,387,744 |
| Allocation events | 199,953 | 199,475 |
| Idle pool after scheduler drop | 933,888 B | 1,982,464 B |

The candidate cut cumulative allocated bytes by about 90.8%, but allocation events changed by only 0.24% and median CPU was about 1.0% higher in this small probe. This supports reduced allocator churn, not a CPU win. The upload batching changes have correctness and operation-count checks, but no new whole-client benchmark yet.

The codec regression probe on pre-fix source retained 589,830 bytes of governor credit after dropping a 512 KiB decoded extension and trimming its read buffer to 65,536 bytes. The new ownership test confirms the copied-payload charge follows message clones and returns to zero after the final message owner and codec are dropped. Governor credit measures admission accounting, not RSS.

The latest clean five-client comparison remains the older `f6cce154` result from the supplied review:

| Client | CPU-s/GiB | Anonymous RAM MiB |
|---|---:|---:|
| TorrentNG | 2.46 | 25.2 |
| qBittorrent | 2.04 | 18.0 |
| Transmission | 1.31 | 25.8 |
| Deluge | 2.27 | 89.1 |
| rTorrent | 0.56 | 1.7 |

There is no current-head five-client score, so these changes do not yet establish that TorrentNG is faster or uses less RAM than the comparison clients. The earlier private two-seeder download run was invalid because the peers did not connect; it is not a benchmark result.

## Evidence

Raw 58c7 pool and codec probe output is in [`evidence/local/probe-current-58c7fb59`](evidence/local/probe-current-58c7fb59/). The original Astra handoff, detailed review, source snapshot, experimental patch, benchmark inputs, and checksums remain in this directory.
