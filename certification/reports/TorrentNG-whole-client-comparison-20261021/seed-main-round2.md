# TorrentNG and reference client performance: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-21T19:31:02+00:00
- TorrentNG source commit: 24e521aed22a9cfb4b905c75a208e06077ad6332
- TorrentNG source tree: a83706e42237af37bada4a7244f4da12a19291e4
- TorrentNG worktree dirty: no
- TorrentNG tracked diff SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG untracked files: 0; content SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG source fingerprint SHA-256: da08674a960f9f43044ffc64483c95d272434844434b2db795fc25141772d6ca
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 79.4 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.1 server=29.9.1; Docker Compose version 5.6.0
- Scratch filesystem: /dev/nvme1n1p1 (btrfs)
- Config filesystem: /dev/nvme0n1p2[/@] (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG build CPU cap: not used (prebuilt image)
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: prebuilt torrentng/torrentngd:benchmark-20261021t191128z (source, fingerprint, and allocator verified)
- TorrentNG benchmark image tag: torrentng/torrentngd:benchmark-20261021t191128z
- Benchmark mode: controlled-seed
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- TorrentNG storage backend (one post-transfer `/metrics` scrape): live peer scheduler backend=pread; groups=1, fixed buffers=no, registered files=no, max batch=1, fixed buffer bytes=0, buffer strategy=disabled, worker copy=no, frame-pool slots=no
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 15s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 10.47/22.76/22.86 at start; 2.68/5.01/11.85 at end; sampled 1m peak 43.43
- Fixture source: caller-supplied local file pinned by SHA-256; no public Debian metadata or swarm was contacted.
- Fixture: debian-13.7.0-amd64-netinst.iso; 792,723,456 bytes; torrent infohash bde512393bf3804515e883cc3188c408783d7b72
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 5/5 seed phases (best-effort kernel hint).
- Pinned fixture SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Clients selected for this run: Deluge, rTorrent, TorrentNG, qBittorrent, Transmission
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This is 12.5% of the configured per-client upload ceiling. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Public-swarm downloads: not run in controlled-seed mode; interoperability is a separate qualification.
- Container image references: Deluge: lscr.io/linuxserver/deluge@sha256:9505c64720afa9e5f0ac1576e660dd6d1e9d4a6733b791f4821e8f496f19f41f; rTorrent: jesec/rtorrent@sha256:994728c59953e5f19c643827024dd06e9ffaf62b8d903a3f1468794415e72e03; TorrentNG: torrentng/torrentngd:benchmark-20261021t193102z; qBittorrent: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650; Transmission: lscr.io/linuxserver/transmission@sha256:1a12fef3c89eca48b7be9e7d36b17b4eb4e1bcf5e1ee7fbf372e3a38b562939d; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image torrentng/torrentngd:benchmark-20261021t191128z carries matching source, fingerprint, and allocator labels. Reference images are pinned by digest in deploy/interop/compose.yml.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| Deluge | unavailable (RuntimeError) | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |
| rTorrent | 0.9.8 | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |
| TorrentNG | build 24e521aed22a | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |
| qBittorrent | v5.2.3 | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |
| Transmission | 4.1.3 (838877323f) | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |

## Controlled seeding

Each client seeded the same SHA-256-verified fixture through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked the fixture SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Each selected client seeded the same caller-supplied fixture after its pinned SHA-256 was checked. Public-swarm testing was skipped by controlled-seed mode.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Deluge | PASS | 5.1 | 193.3 | 3.91 | 4.69 | 0.92 | 4.27 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| rTorrent | PASS | 4.7 | 192.6 | 3.93 | 4.70 | 0.27 | 1.72 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| TorrentNG | PASS | 4.6 | 197.6 | 3.83 | 4.71 | 0.53 | 1.92 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| qBittorrent | PASS | 4.6 | 191.7 | 3.94 | 4.73 | 1.08 | 2.51 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | PASS | 4.6 | 197.7 | 3.82 | 4.66 | 0.47 | 1.55 | 2 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client at the configured process-tree interval; RSS can count shared pages more than once, while PSS apportions them. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Deluge | 84.4 | 79.7 | n/a | n/a | 844.6 | 82.7 | 886.8 | n/a | n/a |
| rTorrent | 1.1 | 0.7 | n/a | n/a | 761.9 | 1.0 | 5.9 | 5.9 | n/a |
| TorrentNG | 29.6 | 2.9 | n/a | n/a | 48.8 | 16.3 | 33.9 | 33.5 | n/a |
| qBittorrent | 7.9 | 5.4 | n/a | n/a | 772.5 | 6.2 | 771.8 | n/a | n/a |
| Transmission | 7.5 | 2.0 | n/a | n/a | 37.8 | 17.1 | 30.1 | n/a | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|
| Deluge | n/a | n/a | n/a | 2.68 | 991.49 | 0.16 |
| rTorrent | n/a | n/a | n/a | 0.70 | 1002.33 | 0.00 |
| TorrentNG | n/a | n/a | n/a | 1.33 | 1024.00 | 0.50 |
| qBittorrent | n/a | n/a | n/a | 2.30 | 992.85 | 0.00 |
| Transmission | n/a | n/a | n/a | 1.19 | 1024.00 | 0.00 |

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

- Raw time-series samples: [seed-main-round2-samples.csv](seed-main-round2-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [seed-main-round2-memory-profile.json](seed-main-round2-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /mnt/disks/gamespool0/torrentng-whole-client-architecture-comparison-20261021/run-seed-main-2
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed**
