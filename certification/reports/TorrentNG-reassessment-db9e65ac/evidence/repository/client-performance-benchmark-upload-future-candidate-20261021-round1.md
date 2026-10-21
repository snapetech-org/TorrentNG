# TorrentNG and reference client performance: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-21T10:37:59+00:00
- TorrentNG source commit: c83f2840639d49f9c98f1f2174ff5d462682a6de
- TorrentNG source tree: f9bf7849ff1f9da4a72e80ca2dfb47419fcaceed
- TorrentNG worktree dirty: yes
- TorrentNG tracked diff SHA-256: e37bc3710c6950e2ead06f24f841b6c8554a01f43398abb3f11b85c80db6fb07
- TorrentNG untracked files: 0; content SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG source fingerprint SHA-256: 6a0da56d105c9566c2b3ce24d60b340d2724560292115760a03b629519cd9b68
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 77.2 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.1 server=29.9.0; Docker Compose version 5.6.0
- Scratch filesystem: /dev/nvme1n1p1 (btrfs)
- Config filesystem: /dev/nvme1n1p1 (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG build CPU cap: 2 logical CPUs via task-scoped BuildKit
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: built from this checkout; image labels verified
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- TorrentNG storage backend (one post-transfer `/metrics` scrape): live peer scheduler backend=pread; groups=1, fixed buffers=no, registered files=no, max batch=1, fixed buffer bytes=0, buffer strategy=disabled, worker copy=no, frame-pool slots=no
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 15s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 14.50/14.88/12.02 at start; 10.17/8.40/8.73 at end; sampled 1m peak 12.52
- Public source: [official Debian torrent directory](https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/); resolved torrent https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent
- ISO: debian-13.7.0-amd64-netinst.iso; 792,723,456 bytes; torrent infohash 7acf8fb590b2060dd9c3146ef770169d593433b0
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 2/2 seed phases (best-effort kernel hint).
- Official SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Clients selected for this run: Transmission, rTorrent, Deluge, qBittorrent, TorrentNG
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This is 12.5% of the configured per-client upload ceiling. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Each public download ran alone, in this order: Transmission, rTorrent, Deluge, qBittorrent, TorrentNG
- Container image references: Transmission: lscr.io/linuxserver/transmission@sha256:1a12fef3c89eca48b7be9e7d36b17b4eb4e1bcf5e1ee7fbf372e3a38b562939d; rTorrent: jesec/rtorrent@sha256:994728c59953e5f19c643827024dd06e9ffaf62b8d903a3f1468794415e72e03; Deluge: lscr.io/linuxserver/deluge@sha256:9505c64720afa9e5f0ac1576e660dd6d1e9d4a6733b791f4821e8f496f19f41f; qBittorrent: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650; TorrentNG: torrentng/torrentngd:benchmark-20261021t103759z; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image was built from the recorded checkout. Reference images are pinned by digest in deploy/interop/compose.yml.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| Transmission | 4.1.3 (838877323f) | FAIL | 99.84% | 58.4 | 12.93 | 35.71 | 4.18 | 10.79 | 32 | TimeoutError: Transmission did not complete public_download within 60s |
| rTorrent | unavailable | FAIL | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | RuntimeError: timed out waiting for rTorrent |
| Deluge | unavailable (RuntimeError) | FAIL | 99.90% | 58.4 | 12.93 | 22.65 | 6.67 | 14.62 | 3 | TimeoutError: Deluge did not complete public_download within 60s |
| qBittorrent | v5.2.3 | PASS | 100.00% | 43.3 | 17.47 | 32.28 | 7.55 | 15.01 | 23 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| TorrentNG | build c83f2840639d | FAIL | 5.21% | 58.4 | 0.67 | 8.08 | 0.93 | 4.11 | 32 | TimeoutError: TorrentNG did not complete public_download within 60s |

## Controlled seeding

Each client seeded the same SHA-256-verified ISO through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked Debian's published SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Seed tests use a separate per-client copy of the same SHA-256-verified ISO source, either the first completed public download or the explicitly verified seed-source file, and can run even if that client's public-swarm download stalls.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Transmission | NOT RUN | 0.0 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | no SHA-256-verified ISO source is available |
| rTorrent | NOT RUN | 0.0 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | RuntimeError: timed out waiting for rTorrent |
| Deluge | NOT RUN | 0.0 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | no SHA-256-verified ISO source is available |
| qBittorrent | PASS | 4.6 | 194.8 | 3.88 | 4.52 | 0.85 | 3.44 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| TorrentNG | PASS | 4.7 | 197.9 | 3.82 | 4.62 | 0.99 | 2.76 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client at the configured process-tree interval; RSS can count shared pages more than once, while PSS apportions them. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Transmission | 24.2 | 3.1 | 105.1 | 25.3 | n/a | n/a | n/a | n/a | n/a |
| rTorrent | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| Deluge | 136.3 | 79.7 | 876.4 | 93.4 | n/a | n/a | n/a | n/a | n/a |
| qBittorrent | 53.1 | 13.8 | 824.6 | 19.4 | 817.8 | 18.5 | 781.7 | n/a | n/a |
| TorrentNG | 29.4 | 2.9 | 36.5 | 7.0 | 34.4 | 24.2 | 41.8 | 41.4 | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|
| Transmission | 3.69 | 0.78 | 1018.65 | n/a | n/a | n/a |
| rTorrent | n/a | n/a | n/a | n/a | n/a | n/a |
| Deluge | 6.11 | 0.00 | 939.07 | n/a | n/a | n/a |
| qBittorrent | 4.98 | 0.00 | 1019.28 | 2.20 | 994.21 | 0.01 |
| TorrentNG | 13.55 | 0.00 | 1135.24 | 2.74 | 1063.87 | 0.58 |

The tracker is restarted between clients to discard stale peer records.
- After the seeder recognizes the complete fixture and just before each timed transfer, its cached file pages are advised out so the cgroup block-read counters are not determined by which client seeded first. This is a best-effort Linux page-cache hint.

## Method and scope

- One current official Debian amd64 netinst v1 torrent per client; clients ran sequentially on the same host and local scratch filesystem.
- Public downloads used the live Debian swarm. All seed tests used the same SHA-256-verified ISO source (the first completed public download or an explicitly supplied verified seed file), copied into a separate seed path for each client, so seed performance is measured independently of public-swarm completion.
- Each client was configured for a 32.0 MiB/s global download and upload ceiling and 32 global/per-torrent peer ceiling.
- Sampling interval: 2 seconds. A successful download average is verified ISO bytes divided by elapsed wall time; for an incomplete download, the reported average uses client completed-byte progress and is paired with the final percentage. The seed transfer timer starts after both clients are ready, the leecher is registered, and the seeder has reannounced; leecher setup time is listed separately. Peak rates come from client payload counters. Mean/peak CPU is Docker CPU percent, where 100% represents one logical core; cgroup CPU seconds per GiB and block read/write bytes per GiB are listed separately. Each client has an idle baseline sampled for 8 seconds before adding a torrent. Cgroup anonymous/file memory and process-tree RSS/PSS are collected for every client. TorrentNG engine memory metrics and detailed smaps are captured only when TNG_CLIENT_BENCH_DIAGNOSTICS=1; when TorrentNG is included, every transfer run captures backend selection once after the measured windows. Diagnostic runs include additional measurement work and must not be compared as clean resource trials. Jemalloc counters appear only when that allocator is selected and do not represent total process allocations.
- Public download uses the current public Debian swarm with a 60s completion timeout. Controlled seeding uses a private task-local tracker and one qBittorrent leecher on the same Compose network. The tracker is reset between clients. The no-peer startup limit is 90s; full transfer limit is 900s.

## Confidence and limitations

- Hash validation gives high confidence that successful runs transferred the same ISO.
- Throughput and resource comparisons have low confidence: one sequential public-swarm pass and one seed pass per client, on a shared host with the load reported above. Public peer availability changes over time.
- Results apply to this host, storage, build, image digests, peer ceiling, rate ceiling, and test window. They do not establish a general ranking, multi-torrent capacity, or likely public seed demand.

## Artifacts and cleanup

- Raw time-series samples: [client-performance-benchmark-upload-future-candidate-20261021-round1-samples.csv](client-performance-benchmark-upload-future-candidate-20261021-round1-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [client-performance-benchmark-upload-future-candidate-20261021-round1-memory-profile.json](client-performance-benchmark-upload-future-candidate-20261021-round1-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /mnt/disks/gamespool0/torrentng-upload-future-candidate-20261021/benchmark-workdir-round1
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed; task-scoped BuildKit builder and cache removed**
- Official [Debian netinst page](https://www.debian.org/CD/netinst/)
