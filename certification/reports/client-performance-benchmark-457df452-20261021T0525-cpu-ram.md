# TorrentNG and reference client performance: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-21T05:25:55+00:00
- TorrentNG source commit: 457df452aeef4afc6f0e16777d05662b94945dc5
- TorrentNG source tree: db186666c2bc2eade55cda1281e04e428a615e2f
- TorrentNG worktree dirty: no
- TorrentNG tracked diff SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG untracked files: 0; content SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG source fingerprint SHA-256: 039ec8b0c81f97c6aed25ec5adeeb952786b75a2ba2dce478016c08cf84d5b92
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 68.6 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.1 server=29.9.0; Docker Compose version 5.6.0
- Scratch filesystem: /dev/nvme1n1p1 (btrfs)
- Config filesystem: /dev/nvme1n1p1 (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: built from this checkout; image labels verified
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 10s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 4.87/6.39/9.52 at start; 4.66/5.14/6.56 at end; sampled 1m peak 13.33
- Public source: [official Debian torrent directory](https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/); resolved torrent https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent
- ISO: debian-13.7.0-amd64-netinst.iso; 792,723,456 bytes; torrent infohash 7acf8fb590b2060dd9c3146ef770169d593433b0
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 5/5 seed phases (best-effort kernel hint).
- Official SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Clients selected for this run: TorrentNG, qBittorrent, Transmission, Deluge, rTorrent
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This is 12.5% of the configured per-client upload ceiling. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Each public download ran alone, in this order: TorrentNG, qBittorrent, Transmission, Deluge, rTorrent
- Container image references: TorrentNG: torrentng/torrentngd:benchmark-20261021t052555z; qBittorrent: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650; Transmission: lscr.io/linuxserver/transmission@sha256:1a12fef3c89eca48b7be9e7d36b17b4eb4e1bcf5e1ee7fbf372e3a38b562939d; Deluge: lscr.io/linuxserver/deluge@sha256:9505c64720afa9e5f0ac1576e660dd6d1e9d4a6733b791f4821e8f496f19f41f; rTorrent: jesec/rtorrent@sha256:994728c59953e5f19c643827024dd06e9ffaf62b8d903a3f1468794415e72e03; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image was built from the recorded checkout. Reference images are pinned by digest in deploy/interop/compose.yml.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | build 457df452aeef | FAIL | 6.07% | 58.4 | 0.79 | 8.60 | 1.00 | 3.69 | 32 | TimeoutError: TorrentNG did not complete public_download within 60s |
| qBittorrent | v5.2.3 | PASS | 100.00% | 46.3 | 16.33 | 31.86 | 12.89 | 28.98 | 19 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | 4.1.3 (838877323f) | FAIL | 99.90% | 58.4 | 12.94 | 36.36 | 4.00 | 10.13 | 32 | TimeoutError: Transmission did not complete public_download within 60s |
| Deluge | unavailable (RuntimeError) | FAIL | 99.08% | 58.5 | 12.81 | 32.57 | 10.31 | 27.87 | 2 | TimeoutError: Deluge did not complete public_download within 60s |
| rTorrent | 0.9.8 | PASS | 100.00% | 40.2 | 18.78 | 12.33 | 5.04 | 8.45 | 13 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Controlled seeding

Each client seeded the same SHA-256-verified ISO through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked Debian's published SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Seed tests use a separate per-client copy of the same SHA-256-verified ISO source, either the first completed public download or the explicitly verified seed-source file, and can run even if that client's public-swarm download stalls.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | PASS | 4.5 | 197.9 | 3.82 | 4.69 | 0.97 | 3.24 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| qBittorrent | PASS | 4.8 | 191.8 | 3.94 | 4.70 | 0.74 | 2.99 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | PASS | 4.7 | 197.7 | 3.82 | 4.68 | 0.51 | 1.56 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Deluge | PASS | 4.8 | 191.8 | 3.94 | 4.64 | 0.97 | 4.69 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| rTorrent | PASS | 4.6 | 194.7 | 3.88 | 4.69 | 0.22 | 1.15 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client at the configured process-tree interval; RSS can count shared pages more than once, while PSS apportions them. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| TorrentNG | 3.6 | 2.9 | 11.8 | 6.7 | 32.6 | 25.2 | 42.6 | 42.2 | n/a |
| qBittorrent | 16.8 | 13.8 | 785.1 | 33.1 | 836.9 | 18.0 | 779.8 | n/a | n/a |
| Transmission | 23.7 | 3.1 | 92.5 | 26.0 | 51.6 | 24.8 | 39.1 | n/a | n/a |
| Deluge | 139.3 | 79.7 | 908.4 | 96.8 | 1660.9 | 85.1 | 889.3 | n/a | n/a |
| rTorrent | 5.5 | 0.7 | 781.1 | 2.7 | 764.7 | 1.9 | 6.9 | 6.8 | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|
| TorrentNG | 14.19 | 0.00 | 1102.48 | 2.66 | 1024.00 | 0.41 |
| qBittorrent | 7.67 | 0.00 | 1009.74 | 2.02 | 1010.46 | 14.34 |
| Transmission | 3.48 | 0.02 | 1018.72 | 1.33 | 1024.00 | 0.04 |
| Deluge | 8.27 | 0.00 | 991.26 | 2.43 | 1003.15 | 0.13 |
| rTorrent | 3.41 | 0.00 | 790.76 | 0.55 | 986.07 | 0.02 |

The tracker is restarted between clients to discard stale peer records.
- After the seeder recognizes the complete fixture and just before each timed transfer, its cached file pages are advised out so the cgroup block-read counters are not determined by which client seeded first. This is a best-effort Linux page-cache hint.

## Method and scope

- One current official Debian amd64 netinst v1 torrent per client; clients ran sequentially on the same host and local scratch filesystem.
- Public downloads used the live Debian swarm. All seed tests used the same SHA-256-verified ISO source (the first completed public download or an explicitly supplied verified seed file), copied into a separate seed path for each client, so seed performance is measured independently of public-swarm completion.
- Each client was configured for a 32.0 MiB/s global download and upload ceiling and 32 global/per-torrent peer ceiling.
- Sampling interval: 2 seconds. A successful download average is verified ISO bytes divided by elapsed wall time; for an incomplete download, the reported average uses client completed-byte progress and is paired with the final percentage. The seed transfer timer starts after both clients are ready, the leecher is registered, and the seeder has reannounced; leecher setup time is listed separately. Peak rates come from client payload counters. Mean/peak CPU is Docker CPU percent, where 100% represents one logical core; cgroup CPU seconds per GiB and block read/write bytes per GiB are listed separately. Each client has an idle baseline sampled for 8 seconds before adding a torrent. Cgroup anonymous/file memory and process-tree RSS/PSS are collected for every client. TorrentNG engine metrics and detailed smaps are captured only when TNG_CLIENT_BENCH_DIAGNOSTICS=1; diagnostic runs include additional measurement work and must not be compared as clean resource trials. Jemalloc counters appear only when that allocator is selected and do not represent total process allocations.
- Public download uses the current public Debian swarm with a 60s completion timeout. Controlled seeding uses a private task-local tracker and one qBittorrent leecher on the same Compose network. The tracker is reset between clients. The no-peer startup limit is 90s; full transfer limit is 900s.

## Confidence and limitations

- Hash validation gives high confidence that successful runs transferred the same ISO.
- Throughput and resource comparisons have low confidence: one sequential public-swarm pass and one seed pass per client, on a shared host with the load reported above. Public peer availability changes over time.
- Results apply to this host, storage, build, image digests, peer ceiling, rate ceiling, and test window. They do not establish a general ranking, multi-torrent capacity, or likely public seed demand.

## Artifacts and cleanup

- Raw time-series samples: [client-performance-benchmark-457df452-20261021T0525-cpu-ram-samples.csv](client-performance-benchmark-457df452-20261021T0525-cpu-ram-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [client-performance-benchmark-457df452-20261021T0525-cpu-ram-memory-profile.json](client-performance-benchmark-457df452-20261021T0525-cpu-ram-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /mnt/disks/gamespool0/torrentng-client-bench-20261021T052555Z-a_7qbnpw
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed; task-scoped BuildKit builder and cache removed**
- Official [Debian netinst page](https://www.debian.org/CD/netinst/)
