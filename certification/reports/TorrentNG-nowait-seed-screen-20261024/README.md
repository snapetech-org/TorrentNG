# Linux NOWAIT seed-read screen — 2026-10-24

## Result

An opt-in Linux `preadv2(RWF_NOWAIT)` path is a promising way to remove storage-worker work from warm seed reads. In one matched 1 GiB run per setting, it used 34.6% fewer instructions/GiB and 39.9% fewer user cycles/GiB than the same TorrentNG release binary with the path disabled. In the 4 GiB screen it used 34.7% fewer instructions/GiB. All completed transfers passed SHA-256.

This is **screening evidence, not a promotion or parity result**. Each setting ran once. The 4 GiB off/on runs saw materially different host load, and TorrentNG's thread set changed during every perf capture. Keep the feature opt-in until randomized repeated comparisons under quieter load confirm the direction.

## Measurements

All CPU and hardware-counter values are per SHA-256-verified GiB. The process-counter window starts immediately before the qBittorrent leecher adds the torrent and ends when completion is observed; it includes announce/startup work. PSS and anonymous memory are separate measurements.

| Fixture / setting | Client | Whole CPU s/GiB | Active CPU s/GiB | Instructions/GiB | User cycles/GiB | Perf context switches/GiB | Peak anonymous MiB | Peak PSS MiB |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 1 GiB, NOWAIT off | TorrentNG | 1.10 | 1.10 | 3.818 B | 2.410 B | 57,926 | 9.3 | 22.7 |
| 1 GiB, NOWAIT on | TorrentNG | 0.85 | 0.82 | 2.496 B | 1.449 B | 2,554 | 6.0 | 19.3 |
| 1 GiB, NOWAIT off | rTorrent 0.16.26 | 0.43 | n/a | 158.6 M | 104.0 M | 1,080 | 3.2 | 19.0 |
| 1 GiB, NOWAIT on | rTorrent 0.16.26 | 0.48 | n/a | 156.9 M | 109.0 M | 698 | 3.2 | 19.0 |
| 4 GiB, NOWAIT off | TorrentNG | 1.81 | 1.79 | 3.920 B | 3.659 B | 81,595 | 10.8 | 20.6 |
| 4 GiB, NOWAIT on | TorrentNG | 0.83 | 0.75 | 2.560 B | 1.433 B | 2,405 | 7.1 | 19.3 |
| 4 GiB, NOWAIT on | rTorrent 0.16.26 | 0.42 | 0.16 | 148.8 M | 94.9 M | 522 | 2.3 | 10.3 |

The 1 GiB TorrentNG off/on pair had similar host load averages (about 34–38 on this 32-logical-CPU desktop). Its whole CPU fell 22.7%, anonymous peak fell 35.5%, and PSS fell 15.0%. The 4 GiB pass had higher load for NOWAIT off (start average 56.16) than on (36.39), so its wall-time and cgroup CPU differences are not a causal estimate.

NOWAIT reduced work but changed cache behavior. On the 4 GiB run, inline reads handled 258,996 requests and 3,055 fell back to the worker. Prepared jobs fell from 266,236 reuses to 7,147. Backend operations rose from 238,064 to 261,706, while the recorded backend read size fell from 18.3 KiB to 16.0 KiB per operation and the cgroup block-read counter fell from about 2,070 to 1,026 MiB/GiB. Treat those as one-run tradeoffs; cold-file and mixed workloads still need checking.

TorrentNG still trails rTorrent. On the 4 GiB runs, TorrentNG with NOWAIT used 0.83 CPU-s/GiB versus rTorrent's 0.42, about 2.0× as much. Its PSS was 19.3 versus 10.3 MiB and anonymous peak was 7.1 versus 2.3 MiB. The 1 GiB on-mode run also remained above rTorrent in CPU and anonymous memory. This change moves the measurements in the right direction; it does not meet the goal.

## What changed

The experimental path is Linux-only and disabled by default. Set `TNG_STORAGE_PEER_READ_NOWAIT=1` to try it. For a peer read, TorrentNG borrows an already-cached file descriptor and attempts `preadv2(RWF_NOWAIT)` into an owned storage frame. If the descriptor or frame is unavailable, the kernel reports an unsupported operation or `EAGAIN`, or the read is short, the existing bounded storage-worker path handles the request. Existing file-identity checks remain in place.

The data syscall is nonblocking, but path identity validation is still synchronous: the cached-handle lookup and post-read check call `symlink_metadata` on the caller task. That can block on a slow or remote filesystem. This is why NOWAIT remains an opt-in diagnostic and is not presented as a fully nonblocking peer path. Do not enable it for remote, FUSE, or otherwise slow mounts until identity validation is moved off the peer task or an explicitly weaker revalidation policy is designed and tested.

Storage counters now expose successful inline reads and fallbacks through `StorageIoStats`, engine stats, and `/metrics`. The benchmark harness reports these counters and can switch the path with `TNG_CLIENT_BENCH_STORAGE_PEER_READ_NOWAIT=1`.

The controlled-seed harness was also corrected for transfers that finish between polls. It accepts a tracker state with two complete peers only after checking the live qBittorrent status; if that status is not complete, it enters the live peer/status loop. The final result still requires the full fixture SHA-256. Counter collection begins before torrent addition, so fast transfers no longer outrun `perf`; the report explicitly includes announce/startup overhead. Cache eviction now occurs before the leecher starts.

## Limits and next check

- One 1 GiB pair and one 4 GiB pair per setting; these are not repeated performance medians.
- The 32-core host remained heavily loaded. The 4 GiB off and on runs had different load and peer counts.
- TorrentNG's process/thread IDs were stable by PID, but one thread disappeared before each final `/proc` snapshot. Perf counters were enabled for 100% of their reported interval; context-switch records from `/proc` are marked unavailable when the thread set changed. Treat all counters as diagnostic until a repeated run confirms them.
- Syscall totals were not collected in this batch. `strace` remains a useful next probe, but its scheduling overhead must not be mixed into CPU or throughput claims.
- The 1 GiB run configured a 1 GiB/s client ceiling and an unlimited qBittorrent receive rate. It did not exercise TorrentNG's no-rate-limit path.
- Although `preadv2(RWF_NOWAIT)` does not wait for cold payload pages, synchronous path identity checks still run on the peer task; behavior on slow metadata paths is unmeasured.
- No cold-cache, varied-rate, multi-peer, download, or non-Linux test was part of this screen.

Next: repeat randomized 4 GiB off/on pairs with one unchanged image and identical CPU placement under lower host load, then collect syscall counts separately. Keep the runtime option opt-in until CPU, PSS, anonymous memory, and correctness hold across repeated runs.

## Exact evidence

- [1 GiB NOWAIT off](evidence/nowait-off-1g-r2.md) · [samples](evidence/nowait-off-1g-r2-samples.csv) · [memory](evidence/nowait-off-1g-r2-memory-profile.json) · [perf counters](evidence/nowait-off-1g-r2-process-counters.json)
- [1 GiB NOWAIT on](evidence/nowait-on-1g-r2.md) · [samples](evidence/nowait-on-1g-r2-samples.csv) · [memory](evidence/nowait-on-1g-r2-memory-profile.json) · [perf counters](evidence/nowait-on-1g-r2-process-counters.json)
- [4 GiB NOWAIT off](evidence/nowait-off-4g-r3.md) · [samples](evidence/nowait-off-4g-r3-samples.csv) · [memory](evidence/nowait-off-4g-r3-memory-profile.json) · [perf counters](evidence/nowait-off-4g-r3-process-counters.json)
- [4 GiB NOWAIT on](evidence/nowait-on-4g-r3.md) · [samples](evidence/nowait-on-4g-r3-samples.csv) · [memory](evidence/nowait-on-4g-r3-memory-profile.json) · [perf counters](evidence/nowait-on-4g-r3-process-counters.json)
- [4 GiB rTorrent](evidence/rtorrent-4g-r1.md) · [samples](evidence/rtorrent-4g-r1-samples.csv) · [memory](evidence/rtorrent-4g-r1-memory-profile.json) · [perf counters](evidence/rtorrent-4g-r1-process-counters.json)
- [Exact reproduction settings](run-config.md)
- [Excluded initial runs and tracker failures](excluded/)

Fixture hashes: 1 GiB `342049a2faa17d05512e1089cc0810046568876cf7de1b1133afad2afa65fccc`; 4 GiB `826ccedc7d79ec09eb8aa5e35eed560e0113fec36024a63f650785f845bbb9b4`. Both were caller-supplied local fixtures. The runs used source-built rTorrent/libtorrent 0.16.26 and qBittorrent 5.2.3 as the task-local leecher; public swarms were not used.

TorrentNG's Git commit was `b27fcd9337a22bc231b74d0640083f4a823a9091` with source tree `cfd1a43b00b75f0bf6f75f30aa0ffc33562a014d`. The 1 GiB report source fingerprint is `a92373574c5d01c9030a8449b00a081d3905a5f01e9b5545914a3b093daf206e`; the 4 GiB report fingerprint is `2c90a33ad485658a0a1fd4f533671ddd31f60919554080e3f5521890597b7d03`. The Rust engine source and built release binary were unchanged between these two harness revisions.

The full 4 GiB fixture remains in task scratch at `/var/tmp/torrentng-perf-counters-20261024/r8-nowait-crosscheck/fixture-4g.bin`; it is intentionally not committed.
