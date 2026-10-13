# Torrent client comparison: Debian 13.7 amd64 netinst

**Performance confidence: low.** This is one sequential pass per client on one host and one public-swarm window. The transfer hashes provide **high confidence** that successful runs moved the same ISO; the throughput and resource results are a snapshot, not a general ranking.

## Results at a glance

All four clients that started completed the public download and passed the official SHA-256 check. qBittorrent had the highest measured public-download average in this run at 17.18 MiB/s; TorrentNG measured 14.12 MiB/s. The observed seed rates clustered near the common leecher receive limit, so they describe service under that controlled load, not each client's maximum upload capacity. rTorrent produced no measurements because its pinned container repeatedly failed to start.

## Test object and host

- Torrent: Debian 13.7 amd64 netinst from the [official Debian torrent directory](https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/).
- File: debian-13.7.0-amd64-netinst.iso, 792,723,456 bytes.
- Infohash: 7acf8fb590b2060dd9c3146ef770169d593433b0.
- Official SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355.
- Test host: agent-gui, Debian 13, Linux 6.12.111+deb13-amd64, 8 logical CPUs, 11.7 GiB RAM.
- Docker 26.1.5 and Docker Compose 2.26.1-4.
- Each client used an isolated Compose project and ran alone. Scratch was on local ext4 at /dev/vda1; the benchmark did not use NFS for measured I/O.
- Public runs were sequential on 2026-10-13. The host's 1/5/15-minute load averages at each run's start and end, plus sampled 1-minute peak, were:

| Client | Started (UTC) | Load at start (1/5/15m) | Load at end (1/5/15m) | Sampled 1m peak |
|---|---|---|---|---:|
| Deluge | 04:22:21 | 0.45 / 0.39 / 0.30 | 0.44 / 0.35 / 0.29 | 0.79 |
| Transmission | 04:30:42 | 0.10 / 0.20 / 0.24 | 0.29 / 0.19 / 0.21 | 0.35 |
| qBittorrent | 04:37:24 | 0.25 / 0.19 / 0.21 | 0.64 / 0.36 / 0.26 | 1.12 |
| TorrentNG | 04:42:26 | 0.80 / 0.43 / 0.28 | 0.34 / 1.19 / 1.01 | 3.87 |

The client runs did not overlap.

## Public download

Each client fetched the live Debian torrent. Average throughput is the verified ISO size divided by elapsed wall time. Peak throughput is the highest client payload rate sampled every two seconds. CPU is Docker CPU usage, where 100% represents one logical core; memory is peak container usage.

| Client | Result | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean / peak CPU (%) | Peak container memory (MiB) | Peak peers | SHA-256 |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | PASS | 53.6 | 14.12 | 25.01 | 20.91 / 32.24 | 224.3 | 32 | PASS |
| qBittorrent | PASS | 44.0 | 17.18 | 32.42 | 33.76 / 105.31 | 808.7 | 20 | PASS |
| Transmission | PASS | 136.1 | 5.55 | 31.27 | 2.98 / 18.50 | 129.2 | 32 | PASS |
| Deluge | PASS | 71.7 | 10.55 | 18.59 | 26.09 / 45.85 | 870.4 | 2 | PASS |
| rTorrent | Not measured | — | — | — | — | — | — | Did not start |

## Controlled seeding

Each client seeded a separate copy of the same verified ISO through a private, task-local tracker. One qBittorrent leecher downloaded the complete file and checked the Debian SHA-256. The setup time is reported separately; transfer time begins after the leecher and seeder were ready and the seeder had reannounced.

The common leecher receive ceiling was 4 MiB/s. It is a controlled receiver operating point below TorrentNG's approximately 6.4 MiB/s per-peer upload guard, with 37.5% headroom. Because every observed average is close to that receiver ceiling, these figures do not measure maximum upload capacity.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean / peak CPU (%) | Peak container memory (MiB) | Peak peers | Leecher SHA-256 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | PASS | 4.5 | 208.8 | 3.62 | 4.59 | 2.60 / 6.37 | 209.5 | 2 | PASS |
| qBittorrent | PASS | 4.6 | 192.2 | 3.93 | 4.63 | 2.25 / 15.58 | 883.1 | 2 | PASS |
| Transmission | PASS | 4.5 | 198.1 | 3.82 | 4.63 | 0.43 / 1.36 | 50.1 | 1 | PASS |
| Deluge | PASS | 13.5 | 192.5 | 3.93 | 4.67 | 2.61 / 19.75 | 1006.0 | 1 | PASS |
| rTorrent | Not measured | — | — | — | — | — | — | — | Did not start |

The 32-peer ceiling and 32 MiB/s per-client download/upload ceilings were applied consistently across the fixture. They are recorded test conditions, not KPIs or claims about an ideal peer count. The peak-peer column reports observed connections within that common ceiling.

## Client builds

Reference images were pinned by digest in deploy/interop/compose.yml. Deluge's version probe returned an error, so its version is left unknown rather than guessed.

| Client | Version or build | Image reference |
|---|---|---|
| TorrentNG | Source commit 2c676ba82bf92da661b99c239ba059394c7f3bcc | Built locally from that checkout |
| qBittorrent | v5.2.3 | lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650 |
| Transmission | 4.1.3 (838877323f) | lscr.io/linuxserver/transmission@sha256:1a12fef3c89eca48b7be9e7d36b17b4eb4e1bcf5e1ee7fbf372e3a38b562939d |
| Deluge | Version probe unavailable | lscr.io/linuxserver/deluge@sha256:9505c64720afa9e5f0ac1576e660dd6d1e9d4a6733b791f4821e8f496f19f41f |
| rTorrent | No stable process/version | jesec/rtorrent@sha256:994728c59953e5f19c643827024dd06e9ffaf62b8d903a3f1468794415e72e03 |

rTorrent's pinned image repeatedly exited with std::bad_alloc during startup attempts, including after isolating its system configuration. A full readiness attempt timed out; no public download or seed transfer began. This is a startup failure in this test environment, not a zero-throughput measurement.

## Method, limits, and excluded pilot

- The same official torrent and SHA-256 were used for every public download and leecher verification. All four completed public downloads and all four leecher downloads passed the hash check.
- Clients had a common 32 MiB/s global download/upload ceiling and a 32-peer ceiling. Samples were collected every two seconds.
- Each public download ran alone against the changing public swarm. Client order and swarm conditions therefore affect the result; the data do not establish a stable ranking.
- Controlled seed runs used one leecher, a private tracker reset between clients, TCP enabled, and a 4 MiB/s receive ceiling. This measures a controlled one-seeder/one-leecher operating point, not demand or maximum upload capacity in a public swarm.
- An earlier desktop pilot was excluded: the desktop was heavily loaded with about 92% I/O wait during that run. The figures above are from the later sequential agent-VM runs, not that saturated pilot.
- Performance confidence is low because there was one pass per client. Hash confidence is high for the completed transfers. Results apply to the listed versions, host, storage, swarm window, and limits; they do not predict other hardware, disks, networks, swarms, or multi-torrent capacity.

## Artifacts and cleanup

- Combined 2-second samples for the four successful clients: [client-performance-benchmark-20261013-samples.csv](client-performance-benchmark-20261013-samples.csv). rTorrent has no sample rows because it never started.
- The verified ISO and original .torrent file are preserved on kspls0 LVM at /mnt/datapool_lvm_media/TorrentNG/benchmarks/client-performance-20261013-agentvm-followup/.
- Per-run Compose containers and named volumes were removed after each measurement. The repeatable benchmark script and installed tooling remain available for future runs.
- See the [benchmark guide](../../docs/CLIENT_PERFORMANCE_BENCHMARK.md) for rerunning the comparison and selecting scratch or persistent artifact storage.
- Debian's [netinst page](https://www.debian.org/CD/netinst/).
