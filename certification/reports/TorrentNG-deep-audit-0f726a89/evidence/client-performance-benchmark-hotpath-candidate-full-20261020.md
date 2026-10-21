# TorrentNG and reference client performance: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-20T23:10:49+00:00
- TorrentNG source commit: 8bd30a474b72309a4b95327f32bbd89caeb590dc
- TorrentNG source tree: 235d1080f76617cbbd675162ea7a6e42136fb4ee
- TorrentNG worktree dirty: yes
- TorrentNG tracked diff SHA-256: 19267159981e31f64a8869840cea5055b249946cf91dc740083786178e0b0321
- TorrentNG untracked files: 0; content SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG source fingerprint SHA-256: 435a6d705876e9c02d22d9ce7bfd3475b477034c24d5bfa9adca0d74cb973d8e
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 79.6 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.1 server=29.9.0; Docker Compose version 5.6.0
- Scratch filesystem: /dev/nvme0n1p2[/@] (btrfs)
- Config filesystem: /dev/nvme0n1p2[/@] (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: built from this checkout; image labels verified
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 15s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 10.25/14.45/14.44 at start; 9.21/10.47/11.96 at end; sampled 1m peak 26.18
- Public source: [official Debian torrent directory](https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/); resolved torrent https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent
- ISO: debian-13.7.0-amd64-netinst.iso; 792,723,456 bytes; torrent infohash 7acf8fb590b2060dd9c3146ef770169d593433b0
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 5/5 seed phases (best-effort kernel hint).
- Official SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Clients selected for this run: rTorrent, Deluge, TorrentNG, Transmission, qBittorrent
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This is 12.5% of the configured per-client upload ceiling. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Each public download ran alone, in this order: rTorrent, Deluge, TorrentNG, Transmission, qBittorrent
- Container image references: rTorrent: jesec/rtorrent@sha256:994728c59953e5f19c643827024dd06e9ffaf62b8d903a3f1468794415e72e03; Deluge: lscr.io/linuxserver/deluge@sha256:9505c64720afa9e5f0ac1576e660dd6d1e9d4a6733b791f4821e8f496f19f41f; TorrentNG: torrentng/torrentngd:benchmark-20261020t231049z; Transmission: lscr.io/linuxserver/transmission@sha256:1a12fef3c89eca48b7be9e7d36b17b4eb4e1bcf5e1ee7fbf372e3a38b562939d; qBittorrent: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image was built from the recorded checkout. Reference images are pinned by digest in deploy/interop/compose.yml.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| rTorrent | 0.9.8 | PASS | 100.00% | 43.3 | 17.45 | 11.32 | 6.42 | 29.23 | 13 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Deluge | unavailable (RuntimeError) | PASS | 100.00% | 40.3 | 18.76 | 30.85 | 14.15 | 27.10 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| TorrentNG | build 8bd30a474b72 | FAIL | 5.87% | 58.5 | 0.76 | 9.33 | 1.56 | 4.07 | 32 | TimeoutError: TorrentNG did not complete public_download within 60s |
| Transmission | 4.1.3 (838877323f) | FAIL | 99.81% | 58.4 | 12.92 | 33.14 | 4.25 | 10.39 | 32 | TimeoutError: Transmission did not complete public_download within 60s |
| qBittorrent | v5.2.3 | FAIL | 99.46% | 58.4 | 12.88 | 32.38 | 8.83 | 24.50 | 25 | TimeoutError: qBittorrent did not complete public_download within 60s |

## Controlled seeding

Each client seeded the same SHA-256-verified ISO through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked Debian's published SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Seed tests use a separate per-client copy of the same SHA-256-verified ISO source, either the first completed public download or the explicitly verified seed-source file, and can run even if that client's public-swarm download stalls.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| rTorrent | PASS | 4.7 | 195.0 | 3.88 | 4.72 | 0.28 | 1.93 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Deluge | PASS | 4.6 | 192.1 | 3.93 | 4.65 | 1.04 | 11.55 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| TorrentNG | PASS | 4.7 | 198.1 | 3.82 | 4.63 | 1.34 | 5.10 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | PASS | 4.7 | 197.9 | 3.82 | 4.61 | 0.54 | 1.72 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| qBittorrent | PASS | 4.6 | 192.1 | 3.94 | 4.66 | 1.04 | 6.35 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client every 15 seconds; RSS can count shared pages more than once, while PSS apportions them. TorrentNG engine metrics and detailed smaps are disabled for this run; the linked profile has process-tree samples and cgroup memory. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| rTorrent | 5.5 | 0.7 | 804.9 | 2.8 | 568.0 | 1.7 | 6.73 | 6.72 | n/a |
| Deluge | 139.3 | 79.7 | 914.1 | 86.2 | 930.5 | 83.7 | 884.62 | n/a | n/a |
| TorrentNG | 29.5 | 2.9 | 38.2 | 6.7 | 50.7 | 16.3 | 33.44 | 33.10 | n/a |
| Transmission | 23.6 | 3.1 | 109.3 | 26.5 | 56.2 | 24.9 | 39.05 | n/a | n/a |
| qBittorrent | 16.3 | 13.8 | 482.1 | 27.4 | 1059.8 | 17.6 | 782.75 | n/a | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|
| rTorrent | 3.27 | 0.00 | 682.36 | 0.76 | 982.84 | 0.00 |
| Deluge | 8.07 | 0.00 | 942.46 | 2.78 | 992.28 | 0.10 |
| TorrentNG | 22.01 | 0.00 | 1085.00 | 3.97 | 1255.56 | 0.54 |
| Transmission | 3.81 | 0.45 | 990.20 | 1.41 | 1067.06 | 0.00 |
| qBittorrent | 7.46 | 0.27 | 992.38 | 2.29 | 984.59 | 0.00 |

The tracker is restarted between clients to discard stale peer records.
- After the seeder recognizes the complete fixture and just before each timed transfer, its cached file pages are advised out so the cgroup block-read counters are not determined by which client seeded first. This is a best-effort Linux page-cache hint.

## Method and scope

- One current official Debian amd64 netinst v1 torrent per client; clients ran sequentially on the same host and local scratch filesystem.
- Public downloads used the live Debian swarm. All seed tests used the same first SHA-256-verified public download, copied into a separate seed path for each client, so seed performance is measured independently of public-swarm completion.
- Each client was configured for a 32.0 MiB/s global download and upload ceiling and 32 global/per-torrent peer ceiling.
- Sampling interval: 2 seconds. A successful download average is verified ISO bytes divided by elapsed wall time; for an incomplete download, the reported average uses client completed-byte progress and is paired with the final percentage. The seed transfer timer starts after both clients are ready, the leecher is registered, and the seeder has reannounced; leecher setup time is listed separately. Peak rates come from client payload counters. Mean/peak CPU is Docker CPU percent, where 100% represents one logical core; cgroup CPU seconds per GiB and block read/write bytes per GiB are listed separately. Each client has an idle baseline sampled for 8 seconds before adding a torrent. Cgroup anonymous/file memory and process-tree RSS/PSS are collected for every client. TorrentNG engine metrics and detailed smaps are captured only when TNG_CLIENT_BENCH_DIAGNOSTICS=1; diagnostic runs include additional measurement work and must not be compared as clean resource trials. Jemalloc counters appear only when that allocator is selected and do not represent total process allocations.
- Public download uses the current public Debian swarm with a 60s completion timeout. Controlled seeding uses a private task-local tracker and one qBittorrent leecher on the same Compose network. The tracker is reset between clients. The no-peer startup limit is 90s; full transfer limit is 900s.

## Confidence and limitations

- Hash validation gives high confidence that successful runs transferred the same ISO.
- Throughput and resource comparisons have low confidence: one sequential public-swarm pass and one seed pass per client, on a shared host with the load reported above. Public peer availability changes over time.
- Results apply to this host, storage, build, image digests, peer ceiling, rate ceiling, and test window. They do not establish a general ranking, multi-torrent capacity, or likely public seed demand.

## Artifacts and cleanup

- Raw time-series samples: [client-performance-benchmark-hotpath-candidate-full-20261020-samples.csv](client-performance-benchmark-hotpath-candidate-full-20261020-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [client-performance-benchmark-hotpath-candidate-full-20261020-memory-profile.json](client-performance-benchmark-hotpath-candidate-full-20261020-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /var/tmp/torrentng-client-bench-20261020T231049Z-qekb3lzq
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed; task-scoped BuildKit builder and cache removed**
- Official [Debian netinst page](https://www.debian.org/CD/netinst/)
