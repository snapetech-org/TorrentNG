# TorrentNG and reference client performance: payload.bin

- Started (UTC): 2026-10-24T03:54:23+00:00
- TorrentNG source commit: 5a030f7f543f964db219a8f8bf4451f71bb6e689
- TorrentNG source tree: 9892ea81b0dcac40824853c2b12c0a396029ca58
- TorrentNG worktree dirty: no
- TorrentNG tracked diff SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG untracked files: 0; content SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG source fingerprint SHA-256: bea543b38d68e1f216c8ab9e6adffd36dcdf5b014004eacf1d64350db0689a86
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 58.0 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.2 server=29.9.1; Docker Compose version 5.6.1
- Scratch filesystem: /dev/nvme0n1p2[/@home] (btrfs)
- Config filesystem: /dev/nvme0n1p2[/@home] (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG build CPU cap: 4 logical CPUs via persistent BuildKit cache (maximum 16GB)
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: built from this checkout; image labels verified
- TorrentNG benchmark image tag: torrentng/torrentngd:benchmark-20261024t035423z
- rTorrent comparator profile: not selected
- Benchmark mode: controlled-seed
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- TorrentNG storage backend (post-transfer `/metrics` snapshot): live peer scheduler backend=pread; groups=1, fixed buffers=no, registered files=no, max batch=1, fixed buffer bytes=0, buffer strategy=disabled, worker copy=no, frame-pool slots=no, peer-read ops=8192 logical/7086 backend, peer-read backend avg=20.6 KiB/op, peer-read cache=1106 hits/7086 misses, prepared jobs=4 allocated/8316 reused, prepared wait/run=94.8/199.4 ms; stats snapshot generation=1, age=102 ms
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 15s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 31.53/40.47/42.45 at start; 28.81/36.99/41.02 at end; sampled 1m peak 29.00
- Fixture source: caller-supplied local file pinned by SHA-256; no public Debian metadata or swarm was contacted.
- Fixture: payload.bin; 134,217,728 bytes; torrent infohash bbca7e0d94b2da9aff644f7b39983ac27b0659a3
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 1/1 seed phases (best-effort kernel hint).
- Pinned fixture SHA-256: 789a368ff6d8b3871c733f709d721867dbb932b0297a2c173e654e980aa84399
- Clients selected for this run: TorrentNG
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This is 12.5% of the configured per-client upload ceiling. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Public-swarm downloads: not run in controlled-seed mode; interoperability is a separate qualification.
- Container image references: TorrentNG: torrentng/torrentngd:benchmark-20261024t035423z; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image was built from the recorded checkout. Reference images are pinned by digest or upstream source revision in the benchmark Compose files.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | First peer observed (s) | First payload observed (s) | Active goodput (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | build 5a030f7f543f | NOT RUN | 0.00% | 0.0 | 0.00 | n/a | n/a | n/a | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |

## Controlled seeding

Each client seeded the same SHA-256-verified fixture through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked the fixture SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Each selected client seeded the same caller-supplied fixture after its pinned SHA-256 was checked. Public-swarm testing was skipped by controlled-seed mode.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | First peer observed (s) | First payload observed (s) | Active goodput (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | PASS | 4.7 | 40.4 | 3.17 | 6.0 | 6.0 | 3.53 | 4.59 | 0.66 | 1.80 | 1 | 789a368ff6d8b3871c733f709d721867dbb932b0297a2c173e654e980aa84399 |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client at the configured process-tree interval; RSS can count shared pages more than once, while PSS apportions them. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| TorrentNG | 3.1 | 2.5 | n/a | n/a | 14.0 | 8.2 | 21.6 | 21.4 | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. The active-window fields separate startup-to-first-payload delay from payload transfer: first payload is the first sampled positive progress, and active CPU/goodput use the sample immediately before it as the baseline. Their timing uncertainty is at most one sampling interval; whole-transfer time and CPU/GiB remain reported separately. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public active CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed active CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| TorrentNG | n/a | n/a | n/a | n/a | 2.38 | 2.37 | 1024.00 | 0.78 |

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

- Raw time-series samples: [README-samples.csv](README-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [README-memory-profile.json](README-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /home/keith/.cache/torrentng-perf110-storage-counter-confirmation-20261023
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed; BuildKit builder removed; TorrentNG-specific cache retained (maximum 16GB)**
