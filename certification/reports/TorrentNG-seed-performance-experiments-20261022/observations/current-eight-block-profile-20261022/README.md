# Current-source peer upload profile — 2026-10-22

This is one diagnostic profile of the exact current eight-block release binary at source revision `72132b24c8f22842216eb6f3ef332bd032f24ed8`. It is not a matched comparison.

The private 64 MiB loopback transfer used the fixed 4 MiB/s offered rate, 16 KiB blocks, and 32-request window. Its SHA-256 matched the generated fixture. The gperftools profile ran at 1,000 Hz from the first valid Piece through completion and contained 332 ms of sample weight across 332 ticks.

The profiler run recorded 0.330 process CPU-seconds, 28.06 MiB peak PSS, and 13.75 MiB peak anonymous memory over a 16.30 second transfer. The preload and debugger add substantial overhead, so these are diagnostic observations, not benchmark scores.

| Symbolized leaf samples | Sample time | Share |
|---|---:|---:|
| `libc.so.6` | 172 ms | 51.8% |
| `run_peer_loop_with_upload_future` | 12 ms | 3.6% |
| `PeerCodec::reserve_capacity` | 7 ms | 2.1% |
| `RawTableInner::rehash_in_place` | 7 ms | 2.1% |
| `tokio::sync::mpsc::chan::channel` | 6 ms | 1.8% |
| `GlobalOrphanQueue::reap_orphans` | 5 ms | 1.5% |

The sample is small and broad: no TorrentNG leaf function reached 4%. The one profile motivated an inexpensive check of repeated `ResourceGovernor` `Arc` cloning in the codec; that candidate did not show a resource win in its first whole-client screen. The raw profile, pprof addresses, resolved sample summary, complete probe summary, client log, debugger logs, fixture metainfo, and binary identity are retained here. The baseline binary is also archived under [`seed-goal-next-avenues-20261022/`](../seed-goal-next-avenues-20261022/).
