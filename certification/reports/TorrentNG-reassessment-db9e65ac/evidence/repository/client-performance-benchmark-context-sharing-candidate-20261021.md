# TorrentNG and reference client performance: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-21T09:42:31+00:00
- TorrentNG source commit: c6984493307a5fb01c9182067bdcbf2512af3f49
- TorrentNG source tree: 769f83222aec9634003e98460f9780c56add6d29
- TorrentNG worktree dirty: yes
- TorrentNG tracked diff SHA-256: 08b161bd8bea8d483c3d51dd24a36dee5764e04b5fd4db21b08f6e52e237aabd
- TorrentNG untracked files: 0; content SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG source fingerprint SHA-256: 7adadbbb5e0333eb9632db4a9789f5e4069cf09f1c04d905b63aeb8d9429fd53
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 78.2 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.1 server=29.9.0; Docker Compose version 5.6.0
- Scratch filesystem: /dev/nvme1n1p1 (btrfs)
- Config filesystem: /dev/nvme1n1p1 (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG build CPU cap: 8 logical CPUs via task-scoped BuildKit
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: built from this checkout; image labels verified
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- TorrentNG storage backend (one post-transfer `/metrics` scrape): live peer scheduler backend=pread; groups=1, fixed buffers=no, registered files=no, max batch=1, fixed buffer bytes=0, buffer strategy=disabled, worker copy=no, frame-pool slots=no
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 15s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 5.46/8.32/8.85 at start; 9.44/9.15/8.08 at end; sampled 1m peak 17.21
- Public source: [official Debian torrent directory](https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/); resolved torrent https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent
- ISO: debian-13.7.0-amd64-netinst.iso; 792,723,456 bytes; torrent infohash 7acf8fb590b2060dd9c3146ef770169d593433b0
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 5/5 seed phases (best-effort kernel hint).
- Official SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Clients selected for this run: qBittorrent, Transmission, TorrentNG, Deluge, rTorrent
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This is 12.5% of the configured per-client upload ceiling. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Each public download ran alone, in this order: qBittorrent, Transmission, TorrentNG, Deluge, rTorrent
- Container image references: qBittorrent: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650; Transmission: lscr.io/linuxserver/transmission@sha256:1a12fef3c89eca48b7be9e7d36b17b4eb4e1bcf5e1ee7fbf372e3a38b562939d; TorrentNG: torrentng/torrentngd:benchmark-20261021t094231z; Deluge: lscr.io/linuxserver/deluge@sha256:9505c64720afa9e5f0ac1576e660dd6d1e9d4a6733b791f4821e8f496f19f41f; rTorrent: jesec/rtorrent@sha256:994728c59953e5f19c643827024dd06e9ffaf62b8d903a3f1468794415e72e03; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image was built from the recorded checkout. Reference images are pinned by digest in deploy/interop/compose.yml.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| qBittorrent | v5.2.3 | PASS | 100.00% | 40.3 | 18.78 | 32.14 | 8.86 | 20.11 | 18 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | 4.1.3 (838877323f) | FAIL | 99.87% | 58.4 | 12.93 | 34.51 | 4.26 | 11.58 | 32 | TimeoutError: Transmission did not complete public_download within 60s |
| TorrentNG | build c6984493307a | FAIL | 3.45% | 58.1 | 0.45 | 7.16 | 0.43 | 1.87 | 31 | TimeoutError: TorrentNG did not complete public_download within 60s |
| Deluge | unavailable (RuntimeError) | FAIL | 99.41% | 58.4 | 12.86 | 24.76 | 9.34 | 20.89 | 2 | TimeoutError: Deluge did not complete public_download within 60s |
| rTorrent | 0.9.8 | PASS | 100.00% | 37.2 | 20.31 | 12.81 | 8.31 | 41.76 | 17 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Controlled seeding

Each client seeded the same SHA-256-verified ISO through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked Debian's published SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Seed tests use a separate per-client copy of the same SHA-256-verified ISO source, either the first completed public download or the explicitly verified seed-source file, and can run even if that client's public-swarm download stalls.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| qBittorrent | PASS | 4.5 | 191.8 | 3.94 | 4.64 | 0.84 | 2.63 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | PASS | 4.6 | 197.7 | 3.82 | 4.67 | 0.48 | 1.72 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| TorrentNG | PASS | 4.6 | 197.7 | 3.82 | 4.72 | 0.76 | 5.15 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Deluge | PASS | 4.6 | 191.8 | 3.94 | 4.65 | 0.86 | 3.42 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| rTorrent | PASS | 4.7 | 191.8 | 3.94 | 4.70 | 0.26 | 0.73 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client at the configured process-tree interval; RSS can count shared pages more than once, while PSS apportions them. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| qBittorrent | 61.7 | 13.8 | 827.2 | 26.2 | 790.1 | 16.9 | 779.1 | n/a | n/a |
| Transmission | 23.6 | 3.1 | 114.9 | 26.1 | 54.2 | 23.6 | 37.0 | n/a | n/a |
| TorrentNG | 3.6 | 2.9 | 10.5 | 6.2 | 39.6 | 24.4 | 39.3 | 38.8 | n/a |
| Deluge | 139.6 | 79.8 | 944.8 | 103.3 | 881.9 | 86.8 | 887.7 | n/a | n/a |
| rTorrent | 5.6 | 0.7 | 809.4 | 2.7 | 761.0 | 1.8 | 6.8 | 6.8 | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|
| qBittorrent | 5.50 | 0.01 | 1020.32 | 1.87 | 994.20 | 3.76 |
| Transmission | 3.70 | 0.11 | 1016.73 | 1.37 | 1237.59 | 1.40 |
| TorrentNG | 12.22 | 0.00 | 1065.26 | 2.54 | 1049.58 | 0.38 |
| Deluge | 7.80 | 0.00 | 892.55 | 2.41 | 1007.77 | 0.13 |
| rTorrent | 3.27 | 0.00 | 972.47 | 0.57 | 991.49 | 0.02 |

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

- Raw time-series samples: [client-performance-benchmark-context-sharing-candidate-20261021-samples.csv](client-performance-benchmark-context-sharing-candidate-20261021-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [client-performance-benchmark-context-sharing-candidate-20261021-memory-profile.json](client-performance-benchmark-context-sharing-candidate-20261021-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /mnt/disks/gamespool0/torrentng-deep-audit-closeout-20261021/benchmark-workdir
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed; task-scoped BuildKit builder and cache removed**
- Official [Debian netinst page](https://www.debian.org/CD/netinst/)
