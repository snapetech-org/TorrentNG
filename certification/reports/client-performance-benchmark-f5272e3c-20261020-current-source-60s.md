# TorrentNG and reference client performance: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-20T21:41:32+00:00
- TorrentNG source commit: f5272e3cbd92a923ca52852a615ed55a3d234e76
- TorrentNG source tree: 6f59005c125dd8a061202326b3c72f01056d52b9
- TorrentNG worktree dirty: no
- TorrentNG tracked diff SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG untracked files: 0; content SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG source fingerprint SHA-256: 583629bc6a3803bd3c4d308899d9aa6ae6eaa959ebdcdc18e5956756184a8ca5
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 77.8 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.1 server=29.9.0; Docker Compose version 5.6.0
- Scratch filesystem: /dev/nvme0n1p2[/@] (btrfs)
- Config filesystem: /dev/nvme0n1p2[/@] (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: prebuilt torrentng/torrentngd:benchmark-f5272e3c-exact (source, fingerprint, and allocator verified)
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): controlled_seed=4, idle=4, public_download=3
- Host load average (1/5/15m): 14.04/18.25/19.00 at start; 9.72/9.58/11.84 at end; sampled 1m peak 17.85
- Public source: [official Debian torrent directory](https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/); resolved torrent https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent
- ISO: debian-13.7.0-amd64-netinst.iso; 792,723,456 bytes; torrent infohash 7acf8fb590b2060dd9c3146ef770169d593433b0
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 5/5 seed phases (best-effort kernel hint).
- Official SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Clients selected for this run: TorrentNG, qBittorrent, Transmission, Deluge, rTorrent
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This stays below TorrentNG's peer-upload guard of 4,096 16 KiB requests per 10 seconds (about 6.4 MiB/s), with 37.5% headroom. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Each public download ran alone, in this order: TorrentNG, qBittorrent, Transmission, Deluge, rTorrent
- Container image references: TorrentNG: torrentng/torrentngd:benchmark-20261020t214132z; qBittorrent: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650; Transmission: lscr.io/linuxserver/transmission@sha256:1a12fef3c89eca48b7be9e7d36b17b4eb4e1bcf5e1ee7fbf372e3a38b562939d; Deluge: lscr.io/linuxserver/deluge@sha256:9505c64720afa9e5f0ac1576e660dd6d1e9d4a6733b791f4821e8f496f19f41f; rTorrent: jesec/rtorrent@sha256:994728c59953e5f19c643827024dd06e9ffaf62b8d903a3f1468794415e72e03; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image torrentng/torrentngd:benchmark-f5272e3c-exact carries matching source, fingerprint, and allocator labels. Reference images are pinned by digest in deploy/interop/compose.yml.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | build f5272e3cbd92 | FAIL | 4.92% | 58.5 | 0.64 | 11.71 | 1.30 | 5.29 | 26 | TimeoutError: TorrentNG did not complete public_download within 60s |
| qBittorrent | v5.2.3 | PASS | 100.00% | 43.3 | 17.46 | 30.51 | 10.31 | 22.10 | 24 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | 4.1.3 (838877323f) | FAIL | 99.89% | 58.5 | 12.91 | 33.91 | 4.80 | 13.38 | 32 | TimeoutError: Transmission did not complete public_download within 60s |
| Deluge | unavailable (RuntimeError) | FAIL | 99.24% | 58.5 | 12.82 | 28.39 | 9.38 | 26.03 | 2 | TimeoutError: Deluge did not complete public_download within 60s |
| rTorrent | 0.9.8 | PASS | 100.00% | 31.2 | 24.22 | 11.88 | 6.97 | 10.75 | 13 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Controlled seeding

Each client seeded the same SHA-256-verified ISO through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked Debian's published SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Seed tests use a separate per-client copy of the first SHA-256-verified public download or the explicitly verified seed-source file, and can run even if that client's public-swarm download stalls.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | PASS | 4.6 | 197.4 | 3.83 | 4.71 | 1.93 | 6.78 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| qBittorrent | PASS | 6.8 | 192.3 | 3.93 | 4.70 | 1.10 | 3.58 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | PASS | 5.0 | 198.1 | 3.82 | 4.72 | 0.53 | 1.79 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Deluge | PASS | 5.2 | 192.2 | 3.93 | 4.66 | 1.23 | 4.56 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| rTorrent | PASS | 5.1 | 192.0 | 3.94 | 4.62 | 0.25 | 0.79 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache, so these columns help separate process/runtime growth from data caching. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|
| TorrentNG | 3.6 | 2.9 | 12.0 | 6.4 | 31.0 | 16.1 | 8.00 |
| qBittorrent | 57.9 | 13.8 | 822.7 | 18.1 | 825.4 | 15.2 | n/a |
| Transmission | 23.3 | 3.1 | 132.0 | 25.9 | 53.8 | 21.1 | n/a |
| Deluge | 138.7 | 79.7 | 907.4 | 96.7 | 1666.0 | 86.5 | n/a |
| rTorrent | 5.5 | 0.7 | 784.6 | 2.5 | 767.5 | 1.7 | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|
| TorrentNG | 23.85 | 54.38 | 1121.80 | 5.10 | 1288.41 | 0.52 |
| qBittorrent | 6.28 | 0.00 | 1016.25 | 2.96 | 1014.48 | 0.00 |
| Transmission | 4.12 | 0.02 | 1003.53 | 1.62 | 1016.41 | 0.00 |
| Deluge | 7.80 | 0.00 | 671.64 | 3.18 | 983.69 | 0.10 |
| rTorrent | 3.49 | 0.00 | 339.04 | 0.71 | 978.27 | 0.00 |

The tracker is restarted between clients to discard stale peer records.
- After the seeder recognizes the complete fixture and just before each timed transfer, its cached file pages are advised out so the cgroup block-read counters are not determined by which client seeded first. This is a best-effort Linux page-cache hint.

## Method and scope

- One current official Debian amd64 netinst v1 torrent per client; clients ran sequentially on the same host and local scratch filesystem.
- Public downloads used the live Debian swarm. All seed tests used the same first SHA-256-verified public download, copied into a separate seed path for each client, so seed performance is measured independently of public-swarm completion.
- Each client was configured for a 32.0 MiB/s global download and upload ceiling and 32 global/per-torrent peer ceiling.
- Sampling interval: 2 seconds. A successful download average is verified ISO bytes divided by elapsed wall time; for an incomplete download, the reported average uses client completed-byte progress and is paired with the final percentage. The seed transfer timer starts after both clients are ready, the leecher is registered, and the seeder has reannounced; leecher setup time is listed separately. Peak rates come from client payload counters. Mean/peak CPU is Docker CPU percent, where 100% represents one logical core; cgroup CPU seconds per GiB and block read/write bytes per GiB are listed separately. Each client has an idle baseline sampled for 8 seconds before adding a torrent. Cgroup anonymous/file memory and TorrentNG process/engine metrics help attribute the sampled container footprint. Jemalloc counters appear only when that allocator is selected and do not represent total process allocations.
- Public download uses the current public Debian swarm with a 60s completion timeout. Controlled seeding uses a private task-local tracker and one qBittorrent leecher on the same Compose network. The tracker is reset between clients. The no-peer startup limit is 90s; full transfer limit is 900s.

## Confidence and limitations

- Hash validation gives high confidence that successful runs transferred the same ISO.
- Throughput and resource comparisons have low confidence: one sequential public-swarm pass and one seed pass per client, on a shared host with the load reported above. Public peer availability changes over time.
- Results apply to this host, storage, build, image digests, peer ceiling, rate ceiling, and test window. They do not establish a general ranking, multi-torrent capacity, or likely public seed demand.

## Artifacts and cleanup

- Raw time-series samples: [client-performance-benchmark-f5272e3c-20261020-current-source-60s-samples.csv](client-performance-benchmark-f5272e3c-20261020-current-source-60s-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [client-performance-benchmark-f5272e3c-20261020-current-source-60s-memory-profile.json](client-performance-benchmark-f5272e3c-20261020-current-source-60s-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /var/tmp/tngng-active-bench-f5272e3c-20261020/work-short
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed**
- Official [Debian netinst page](https://www.debian.org/CD/netinst/)
