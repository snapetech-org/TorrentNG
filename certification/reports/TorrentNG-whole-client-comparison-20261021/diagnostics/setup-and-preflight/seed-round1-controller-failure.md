# TorrentNG and reference client performance: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-21T18:09:01+00:00
- TorrentNG source commit: 7c2dd481ca224da8f5e47b783678b6110146ba9b
- TorrentNG source tree: e39e63abbc56f91b35e5124a920793b9eea6fdb2
- TorrentNG worktree dirty: yes
- TorrentNG tracked diff SHA-256: d545e8d32ccc5dfd940a3b3b2de5cbd9f895e93aa8055e7499501fcf6613b7fe
- TorrentNG untracked files: 213; content SHA-256: 5f55c7398ad831a82d41a50cf39548eac5a1b5938e83b442e5930d40919dea93
- TorrentNG source fingerprint SHA-256: 13e1cc305b4c278f907c0a09b8b30ce589c08633e77bf9e7b777698b86af6c75
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 58.0 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.1 server=29.9.1; Docker Compose version 5.6.0
- Scratch filesystem: /dev/nvme1n1p1 (btrfs)
- Config filesystem: /dev/nvme1n1p1 (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG build CPU cap: 4 logical CPUs via persistent BuildKit cache (maximum 16GB)
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: built from this checkout; image labels verified
- TorrentNG benchmark image tag: torrentng/torrentngd:benchmark-20261021t180901z
- Benchmark mode: controlled-seed
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- TorrentNG storage backend (one post-transfer `/metrics` scrape): not observed (TorrentNG was not included in this run)
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 15s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 12.59/10.42/9.23 at start; 10.17/11.61/10.88 at end; sampled 1m peak 12.59
- Fixture source: caller-supplied local file pinned by SHA-256; no public Debian metadata or swarm was contacted.
- Fixture: debian-13.7.0-amd64-netinst.iso; 792,723,456 bytes; torrent infohash bde512393bf3804515e883cc3188c408783d7b72
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 0/0 seed phases (best-effort kernel hint).
- Pinned fixture SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Clients selected for this run: TorrentNG, qBittorrent, Transmission, Deluge, rTorrent
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This is 12.5% of the configured per-client upload ceiling. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Public-swarm downloads: not run in controlled-seed mode; interoperability is a separate qualification.
- Container image references: TorrentNG: torrentng/torrentngd:benchmark-20261021t180901z; qBittorrent: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650; Transmission: not started; Deluge: not started; rTorrent: not started; seed leecher: not started
- Seed leecher: qBittorrent unknown
- The TorrentNG image was built from the recorded checkout. Reference images are pinned by digest in deploy/interop/compose.yml.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | unavailable | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | RuntimeError: timed out waiting for TorrentNG |
| qBittorrent | n/a | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | n/a |
| Transmission | n/a | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | n/a |
| Deluge | n/a | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | n/a |
| rTorrent | n/a | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | n/a |

## Controlled seeding

Each client seeded the same SHA-256-verified fixture through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked the fixture SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Each selected client seeded the same caller-supplied fixture after its pinned SHA-256 was checked. Public-swarm testing was skipped by controlled-seed mode.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | NOT RUN | 0.0 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | RuntimeError: timed out waiting for TorrentNG |
| qBittorrent | NOT RUN | 0.0 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | not verified |
| Transmission | NOT RUN | 0.0 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | not verified |
| Deluge | NOT RUN | 0.0 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | not verified |
| rTorrent | NOT RUN | 0.0 | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | not verified |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client at the configured process-tree interval; RSS can count shared pages more than once, while PSS apportions them. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| TorrentNG | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| qBittorrent | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| Transmission | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| Deluge | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| rTorrent | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|
| TorrentNG | n/a | n/a | n/a | n/a | n/a | n/a |
| qBittorrent | n/a | n/a | n/a | n/a | n/a | n/a |
| Transmission | n/a | n/a | n/a | n/a | n/a | n/a |
| Deluge | n/a | n/a | n/a | n/a | n/a | n/a |
| rTorrent | n/a | n/a | n/a | n/a | n/a | n/a |

The tracker is restarted between clients to discard stale peer records.
- After the seeder recognizes the complete fixture and just before each timed transfer, its cached file pages are advised out so the cgroup block-read counters are not determined by which client seeded first. This is a best-effort Linux page-cache hint.

## Method and scope

- One caller-supplied local fixture was verified against its pinned SHA-256; a private v1 torrent was generated locally for each client. Clients ran sequentially on the same host and local scratch filesystem.
- Public-swarm interoperability was not run. Controlled seeding used the pinned local fixture and an isolated task-local tracker.
- Each client was configured for a 32.0 MiB/s global download and upload ceiling and 32 global/per-torrent peer ceiling.
- Sampling interval: 2 seconds. A successful download average is verified payload bytes divided by elapsed wall time; for an incomplete download, the reported average uses client completed-byte progress and is paired with the final percentage. The seed transfer timer starts after both clients are ready, the leecher is registered, and the seeder has reannounced; leecher setup time is listed separately. Peak rates come from client payload counters. Mean/peak CPU is Docker CPU percent, where 100% represents one logical core; cgroup CPU seconds per GiB and block read/write bytes per GiB are listed separately. Each client has an idle baseline sampled for 8 seconds before adding a torrent. Cgroup anonymous/file memory and process-tree RSS/PSS are collected for every client. TorrentNG engine memory metrics and detailed smaps are captured only when TNG_CLIENT_BENCH_DIAGNOSTICS=1; when TorrentNG is included, every transfer run captures backend selection once after the measured windows. Diagnostic runs include additional measurement work and must not be compared as clean resource trials. Jemalloc counters appear only when that allocator is selected and do not represent total process allocations.
- Controlled seeding uses a private task-local tracker and one qBittorrent leecher on the same Compose network. No public metadata or swarm request is made; containers are attached only to an internal Docker network, and DHT/PEX are disabled and read back before measurement. The tracker is reset between clients. The no-peer startup limit is 90s; full transfer limit is 900s.

## Confidence and limitations

- Hash validation gives high confidence that successful runs transferred the same fixture.
- Throughput and resource comparisons have low confidence: one controlled seed pass per client, on a shared host with the load reported above.
- Results apply to this host, storage, build, image digests, peer ceiling, rate ceiling, and test window. They do not establish a general ranking, multi-torrent capacity, or likely public seed demand.

## Artifacts and cleanup

- Raw time-series samples: [seed-round1-samples.csv](seed-round1-controller-failure-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [seed-round1-memory-profile.json](seed-round1-controller-failure-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /mnt/disks/gamespool0/torrentng-whole-client-architecture-comparison-20261021/run-seed-1
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed; benchmark image retained for reuse: torrentng/torrentngd:benchmark-20261021t180901z; BuildKit builder removed; TorrentNG-specific cache retained (maximum 16GB)**
