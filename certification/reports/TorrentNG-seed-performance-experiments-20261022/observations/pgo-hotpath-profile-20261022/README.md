# PGO seed hot-path profile — 2026-10-22

This is one diagnostic profile of the retained 16 ms PGO daemon, not a matched
performance comparison. The private 256 MiB loopback transfer used `pread`, a
16 KiB block, a 32-block request window, and a 4 MiB/s paced receiver. The full
payload verified against SHA-256
`937d7bd20b42ad9d0130419a685ce1e481f4330ab2a211dde1e16403c7854f81`.

## Identity and scope

- Daemon SHA-256: `8e8522729bdc06b8590995945e87544741370cb66c70eb13576d54f8e378f006`.
- The exact daemon is retained in
  [`upload-coalesce-16ms/executables/`](../upload-coalesce-16ms/executables/)
  as `coalesce16-pgo.torrentngd.gz`.
- The profile sampled from the first valid Piece block through final transfer
  completion with gperftools at 1,000 Hz.
- This was a single-client diagnostic run. It did not include rTorrent and is
  not evidence that any code change improved whole-client CPU or RAM.

The probe recorded 0.868 CPU-seconds for this transfer (3.47 CPU-s/GiB when
normalized) and an active peak of 25.20 MiB PSS / 14.97 MiB anonymous memory.
Those numbers are profiler-run observations only: there is no matched control,
the profiling preload adds work and memory, and the sampled PSS includes an
initial burst. Do not use them as a benchmark score.

## Sampled functions

The 366 sampled stacks represented 739 ms of sample weight. Addresses were
resolved with `addr2line` against the exact daemon above. The function names
show where leaf samples landed; async coroutine poll frames can cover work from
many source operations.

| Function | Sampled leaf time | Share of TorrentNG leaf samples |
|---|---:|---:|
| Tokio poll for `TorrentTask::accept_peer` | 151 ms | 39.2% |
| Supervised `TorrentTask::run` future poll | 75 ms | 19.5% |
| `PreparedPread::execute` | 11 ms | 2.9% |
| `PeerIo::try_next_ready` | 11 ms | 2.9% |
| Prepared `pread` worker closure | 9 ms | 2.3% |
| `RateTree::try_grant_probe_tracked` | 5 ms | 1.3% |
| `PreparedPread::execute_miss` | 2 ms | 0.5% |

`PeerReadStreams::observe` did not appear as a sampled symbol. It may be inlined
or below this profile's resolution, so this is not proof that it is free; it
does mean the proposed path-map refactor has no current profile support as a
large CPU target. Storage preparation is a small enough share that removing it
alone is unlikely to close the measured whole-client gap.

## Decision

No production source or default changed. Do not spend a full comparison cycle
on a path-map-only optimization based on this profile. The next source-level
work should identify a concrete operation inside the peer task's broad async
poll path, then use a matched short screen and fairness/cancellation checks.

Tokio documents that `select!` randomly chooses its first branch by default;
`biased;` removes that RNG cost but makes starvation prevention the caller's
responsibility. This peer loop selects among commands, inbound messages, read
completions, dispatch deadlines, idle timeout, and accounting flushes, so a
fixed order is not adopted from this profile alone. See the [Tokio `select!`
fairness documentation](https://docs.rs/tokio/latest/tokio/macro.select.html).

The raw summary, fixture metainfo, profile, and debugger start/stop logs are
retained here. The generated payload, API/bootstrap credentials, database, and
profiler preload helper are excluded.
