# TorrentNG memory and Debian ISO comparison

**Run date:** 2026-10-14  
**Host:** Agentic VM (`agent-gui`, Debian 13, 8 logical CPUs, 11.7 GiB RAM)  
**Torrent:** official Debian 13.7 amd64 netinst ISO, 792,723,456 bytes  
**SHA-256:** `a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355`

## Findings

The system allocator is now TorrentNG's default build. It is associated with lower observed anonymous memory than the prior jemalloc build: the recorded peak fell from 18.8 to 8.3 MiB during public download, and from 25.3 to 7.4 MiB while seeding. Controlled-seed throughput was nearly unchanged (3.66 versus 3.56 MiB/s). These were separate runs, not a concurrent or randomized A/B, so confidence in the allocator effect is **moderate**.

In the peer comparison, TorrentNG's measured anonymous peak was about 2.1–3.2 times lower than qBittorrent and Transmission, and about 11–12 times lower than Deluge. That is a useful reduction, but it is not an orders-of-magnitude reduction against every peer. At idle, TorrentNG and Transmission were effectively tied (about 3 MiB each). rTorrent timed out during startup; its logs included repeated `std::bad_alloc` messages, so there is no rTorrent performance or memory result.

The public download result remains bad for TorrentNG. The initial system-allocator pass completed at 0.49 MiB/s in 1,552.8 seconds. A later mimalloc pass reached only 86.36% after 1,799.2 seconds (0.36 MiB/s observed average) and timed out. Five minutes later, qBittorrent completed the same ISO in 39.6 seconds at 19.08 MiB/s. These sequential runs suggest a TorrentNG public-download gap beyond a generally slow swarm, but peer availability can change; confidence in that diagnosis is **low**, and the precise engine bottleneck is not isolated. TorrentNG's controlled seed rate remained close to the peers.

## Download and seed results

Memory is the peak sampled cgroup **anonymous** memory in MiB. It excludes charged file cache. CPU is mean Docker CPU percent; 100% represents one logical core. Times and rates are for a single pass on the recorded host.

| Client | Public download (MiB/s, seconds) | Public peak anon (MiB) | Public mean CPU (%) | Controlled seed (MiB/s, seconds) | Seed peak anon (MiB) | Seed mean CPU (%) |
|---|---:|---:|---:|---:|---:|---:|
| TorrentNG, system allocator | 0.49, 1,552.8 | 8.3 | 4.69 | 3.56, 212.6 | 7.4 | 5.08 |
| qBittorrent 5.2.3 | 16.82, 45.0 | 19.7 | 23.51 | 3.74, 202.4 | 15.4 | 1.74 |
| Transmission 4.1.3 | 5.60, 135.0 | 26.7 | 4.47 | 3.61, 209.5 | 19.4 | 0.71 |
| Deluge (version unavailable) | 8.27, 91.5 | 103.4 | 28.43 | 3.74, 202.2 | 86.4 | 2.49 |
| rTorrent | Did not start | — | — | Not run | — | — |

All completed transfers passed the published ISO SHA-256. Peak public/seed peer counts were TorrentNG 32/1, qBittorrent 16/1, Transmission 32/1, and Deluge 3/2. The benchmark applied a 32 MiB/s client rate ceiling and 32-peer ceiling to the fixture; these are test settings, not TorrentNG capacity claims or recommended operating limits. The controlled leecher was capped at 4 MiB/s.

TorrentNG's anonymous-memory ratios from these runs:

- Public download: 2.4× lower than qBittorrent, 3.2× lower than Transmission, and 12.5× lower than Deluge.
- Controlled seeding: 2.1× lower than qBittorrent, 2.6× lower than Transmission, and 11.7× lower than Deluge.

These are descriptive ratios from sequential runs. TorrentNG and qBittorrent were measured in the same system-matrix run. Transmission and Deluge were measured in a later peer-only rerun on the same VM with the same ISO and declared limits. The system-matrix run fingerprint was `38ab6136e565…`; the peer-only rerun fingerprint was `b8872d8a060a…`. Public swarm conditions also changed. Treat the cross-run ratios as **low-confidence comparisons**, not a controlled ranking. The qBittorrent container's working-set counter reached 779 MiB in download and 1,045 MiB in seeding, largely because it includes file cache; that counter is not compared to anonymous memory.

## Allocator comparison

| TorrentNG build | Public rate / time | Public peak anon | Seed rate / time | Seed peak anon |
|---|---:|---:|---:|---:|
| jemalloc with `thp:never,narenas:1,tcache:false,dirty_decay_ms:0,muzzy_decay_ms:0` | 16.31 MiB/s / 46.4 s | 18.8 MiB | 3.66 MiB/s / 206.7 s | 25.3 MiB |
| system allocator (current default) | 0.49 MiB/s / 1,552.8 s | 8.3 MiB | 3.56 MiB/s / 212.6 s | 7.4 MiB |
| mimalloc (follow-up, fingerprint `b8872d8a…`) | incomplete at 0.36 MiB/s / 1,799.2 s (86.36%) | 19.8 MiB | 3.52 MiB/s / 214.8 s | 23.6 MiB |

The system-allocator run used less peak anonymous memory than both alternatives. Its measured peaks were about 56% below jemalloc and 58% below mimalloc for public download, and 71% below jemalloc and 69% below mimalloc while seeding. The allocator runs were separate builds and did not all have matching dirty-worktree fingerprints; the jemalloc artifact also does not record its exact source-diff fingerprint. Treat these as **moderate-confidence** evidence for keeping System as the default, not a controlled allocator A/B. The public rate differences cannot be attributed to allocator choice from these runs.

The mimalloc run's engine memory counters peaked around 5.6 MiB during seeding while cgroup anonymous memory reached 23.6 MiB. The remaining footprint is not identified to a specific allocation. The measured result gives no reason to select mimalloc.

## Current-swarm qBittorrent check

Immediately after the mimalloc test, qBittorrent 5.2.3 downloaded the same public ISO in 39.6 seconds at 19.08 MiB/s, with a 19-peer peak, and passed the published SHA-256. Its controlled seed transfer averaged 3.60 MiB/s in 210.1 seconds and also passed SHA-256. The public rate was about 53 times the incomplete mimalloc-run average; the runs were sequential, so this is a strong warning about the current TorrentNG public-download path, not a stable speed ratio. qBittorrent's peak anonymous memory was 20.2 MiB in public download and 17.1 MiB while seeding, versus 19.8 and 23.6 MiB for the mimalloc build.

## What the profiles explain

The system-allocator seed sample peaked at 7.43 MiB cgroup anonymous memory and 7.40 MiB process anonymous RSS. TorrentNG's process RSS peaked at 23.57 MiB, including roughly 16 MiB of file-backed executable pages. The nearest engine snapshot reported about 5.3 MiB across its tracked memory classes, leaving roughly 2.1 MiB outside those class counters. That residual has not been isolated to a specific allocation or subsystem; the profile shows ordinary anonymous mappings and 17 live process threads at the peak, not one obvious multi-megabyte buffer.

The allocator is the strongest measured memory lever so far. Reducing storage I/O workers from two to one produced repeated seed peaks of 6.8–6.9 MiB instead of 7.4 MiB, with seed rate 3.51–3.53 instead of 3.56 MiB/s. The reduction is only about 7–9%, and the runs were noisy; it does not justify changing the default. Tokio worker reductions did not improve the peak consistently. No peer cap or other arbitrary memory limit was added.

## Changes made

- Made the system allocator the default for `torrentngd`; jemalloc and mimalloc remain explicit build options.
- Corrected the peer-buffer metric so `used_bytes` reflects live governor leases. Request-window and command-queue capacities are separate estimates because they can overlap leased bytes; this fixes accounting, not allocations.
- Kept the existing zero piece-assembly default, which writes verified blocks directly instead of retaining a whole-piece assembly copy.
- Improved the benchmark runner to record process RSS and cgroup anonymous memory separately, require matching commit/source fingerprint/allocator labels for prebuilt images, expose allocator and worker overrides, and keep Docker config binds on local ext4 when transfer scratch is root-squashed NFS.
- Corrected the documented storage worker defaults to the actual configuration (two I/O workers and one hash worker).

## Method and limits

The official Debian ISO torrent was used for each public download. Public downloads ran alone and sequentially on the live Debian swarm. For controlled seeding, each client served the same SHA-256-verified ISO over a private task-local tracker to one qBittorrent 5.2.3 leecher on the same Docker network. The leecher's receive limit was 4 MiB/s. Samples were taken every two seconds. Download and upload ceilings were 32 MiB/s; the common peer ceiling was 32. Payload scratch was kept on the Agentic VM's NFS mount backed by kspls0 LVM. The first system-matrix run also placed Docker configuration on NFS; root-squash then prevented Transmission and Deluge from starting. In the later peer-only rerun, configuration binds moved to local ext4 while payload scratch stayed on NFS.

Memory evidence uses sampled cgroup anonymous bytes to compare active memory without conflating file cache; process RSS is reported separately for TorrentNG. The benchmark did not test multiple simultaneous torrents, long-lived idle retention, or different storage hardware. Successful SHA-256 checks are high-confidence evidence of identical content. Performance and memory rankings are low confidence because each client had one sequential pass, live peers changed, the peer runs were separate, and the source fingerprints differed between runs.

## Raw evidence

- [System allocator versus qBittorrent run](artifacts/native-memory-optimization-20261014/system-matrix-final.md), [raw samples](artifacts/native-memory-optimization-20261014/system-matrix-final-samples.csv), [memory profile](artifacts/native-memory-optimization-20261014/system-matrix-final-memory-profile.json).
- [Transmission, Deluge, and rTorrent rerun](artifacts/native-memory-optimization-20261014/reference-peers-final.md), [raw samples](artifacts/native-memory-optimization-20261014/reference-peers-final-samples.csv), [memory profile](artifacts/native-memory-optimization-20261014/reference-peers-final-memory-profile.json).
- [One-storage-worker repeat](artifacts/native-memory-optimization-20261014/io1-worker-retest-final.md), [raw samples](artifacts/native-memory-optimization-20261014/io1-worker-retest-final-samples.csv), [memory profile](artifacts/native-memory-optimization-20261014/io1-worker-retest-final-memory-profile.json).
- [Two Tokio-worker retest](artifacts/native-memory-optimization-20261014/tokio2-worker-retest-final.md), [raw samples](artifacts/native-memory-optimization-20261014/tokio2-worker-retest-final-samples.csv), [memory profile](artifacts/native-memory-optimization-20261014/tokio2-worker-retest-final-memory-profile.json).
- [One Tokio and one storage I/O worker](artifacts/native-memory-optimization-20261014/tokio1-io1-final.md), [raw samples](artifacts/native-memory-optimization-20261014/tokio1-io1-final-samples.csv), [memory profile](artifacts/native-memory-optimization-20261014/tokio1-io1-final-memory-profile.json).
- [Mimalloc follow-up](artifacts/native-memory-optimization-20261014/mimalloc-final-20261014-b887-full.md), [raw samples](artifacts/native-memory-optimization-20261014/mimalloc-final-20261014-b887-full-samples.csv), [memory profile](artifacts/native-memory-optimization-20261014/mimalloc-final-20261014-b887-full-memory-profile.json).
- [Current-swarm qBittorrent check](artifacts/native-memory-optimization-20261014/qbittorrent-post-mimalloc-swarm-check.md), [raw samples](artifacts/native-memory-optimization-20261014/qbittorrent-post-mimalloc-swarm-check-samples.csv), [memory profile](artifacts/native-memory-optimization-20261014/qbittorrent-post-mimalloc-swarm-check-memory-profile.json).
- [Earlier jemalloc run](native-memory-optimization-20261013-agentvm-jemalloc-only.md), [raw samples](native-memory-optimization-20261013-agentvm-jemalloc-only-samples.csv), [memory profile](native-memory-optimization-20261013-agentvm-jemalloc-only-memory-profile.json).
