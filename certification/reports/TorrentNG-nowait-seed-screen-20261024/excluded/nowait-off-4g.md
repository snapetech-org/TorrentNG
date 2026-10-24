# TorrentNG and reference client performance: fixture-4g.bin

- Started (UTC): 2026-10-24T21:00:40+00:00
- TorrentNG source commit: b27fcd9337a22bc231b74d0640083f4a823a9091
- TorrentNG source tree: cfd1a43b00b75f0bf6f75f30aa0ffc33562a014d
- TorrentNG worktree dirty: yes
- TorrentNG tracked diff SHA-256: c24f22363cf43e115de7bff3e9e5660946c26b4cf9e90e8eabe09fa99e3ee834
- TorrentNG untracked files: 0; content SHA-256: e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855
- TorrentNG source fingerprint SHA-256: de8892b431934ccbc4f9433ef2e8702c51003b451a599568f314f03d951dc33a
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 51.4 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.2 server=29.9.1; Docker Compose version 5.6.1
- Scratch filesystem: /dev/nvme0n1p2[/@] (btrfs)
- Config filesystem: /dev/nvme0n1p2[/@] (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG build CPU cap: not used (prebuilt image)
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: prebuilt torrentng/torrentngd:benchmark-20261024t204502z (source, fingerprint, and allocator verified)
- TorrentNG benchmark image tag: torrentng/torrentngd:benchmark-20261024t204502z
- rTorrent comparator profile: upstream rTorrent v0.16.26 (62899bd053c39b70380464e8f64dbcfce92579aa) with libtorrent v0.16.26 (21daa3b96a3b8cfe14961545b5115de7ed12da95); image torrentng/rtorrent-benchmark:v0.16.26-62899bd-lib21daa3b
- Benchmark mode: controlled-seed
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- Process-counter mode: perf; client CPU set: 18-19; seed-leecher CPU set: 20-21.
- qBittorrent global rate preference readback: seed_leecher: down=0 B/s, up=1073741824 B/s, LAN-limits=True.
- TorrentNG storage backend (post-transfer `/metrics` snapshot): live peer scheduler backend=pread; groups=1, fixed buffers=no, registered files=no, max batch=1, fixed buffer bytes=0, buffer strategy=disabled, worker copy=no, frame-pool slots=no, peer-read ops=0 logical/0 backend, peer-read cache=0 hits/0 misses, peer-read NOWAIT=0 inline/0 fallbacks, prepared jobs=1 allocated/4095 reused, prepared wait/run=10.5/1023.4 ms; stats snapshot generation=1, age=102 ms
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 2s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 46.74/37.43/34.14 at start; 21.11/28.60/31.37 at end; sampled 1m peak 44.71
- Fixture source: caller-supplied local file pinned by SHA-256; no public Debian metadata or swarm was contacted.
- Fixture: fixture-4g.bin; 4,294,967,296 bytes; torrent infohash b4286c65a63de51e39b35d9a0402c07ed79dea5a
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 0/0 seed phases (best-effort kernel hint).
- Pinned fixture SHA-256: 826ccedc7d79ec09eb8aa5e35eed560e0113fec36024a63f650785f845bbb9b4
- Clients selected for this run: TorrentNG, rTorrent
- Client download and upload limits: 1,073,741,824 B/s (1024.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: unlimited (qBittorrent readback 0 B/s); LAN rate limiting is enabled.
- Public-swarm downloads: not run in controlled-seed mode; interoperability is a separate qualification.
- Container image references: TorrentNG: torrentng/torrentngd:benchmark-20261024t210040z; rTorrent: torrentng/rtorrent-benchmark:v0.16.26-62899bd-lib21daa3b; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image torrentng/torrentngd:benchmark-20261024t204502z carries matching source, fingerprint, and allocator labels. Reference images are pinned by digest or upstream source revision.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | First peer observed (s) | First payload observed (s) | Active goodput (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | build b27fcd9337a2 | NOT RUN | 0.00% | 0.0 | 0.00 | n/a | n/a | n/a | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |
| rTorrent | 0.16.26 | NOT RUN | 0.00% | 0.0 | 0.00 | n/a | n/a | n/a | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |

## Controlled seeding

Each client seeded the same SHA-256-verified fixture through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked the fixture SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Each selected client seeded the same caller-supplied fixture after its pinned SHA-256 was checked. Public-swarm testing was skipped by controlled-seed mode.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | First peer observed (s) | First payload observed (s) | Active goodput (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | FAIL | 0.0 | 0.0 | 0.00 | n/a | n/a | n/a | 0.00 | 0.00 | 0.00 | 0 | TimeoutError: OpenTracker live scrape did not report both the seed and leecher for the test torrent (last scrape: 2 complete, 0 incomplete) |
| rTorrent | FAIL | 0.0 | 0.0 | 0.00 | n/a | n/a | n/a | 0.00 | 0.00 | 0.00 | 0 | TimeoutError: OpenTracker live scrape did not report both the seed and leecher for the test torrent (last scrape: 2 complete, 0 incomplete) |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client at the configured process-tree interval; RSS can count shared pages more than once, while PSS apportions them. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| TorrentNG | 14.8 | 2.5 | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| rTorrent | 11.1 | 2.6 | n/a | n/a | n/a | n/a | n/a | n/a | n/a |

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. The active-window fields separate startup-to-first-payload delay from payload transfer: first payload is the first sampled positive progress, and active CPU/goodput use the sample immediately before it as the baseline. Their timing uncertainty is at most one sampling interval; whole-transfer time and CPU/GiB remain reported separately. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public active CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed active CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| TorrentNG | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |
| rTorrent | n/a | n/a | n/a | n/a | n/a | n/a | n/a | n/a |

## Diagnostic process counters

Counters start after the first sampled positive payload progress and are normalized by the remaining payload bytes from that point through completion. The full leecher file must still pass its pinned SHA-256 check before the counters are retained. They are diagnostic evidence, not scoreable CPU or throughput results. `strace` changes scheduling and must not be used to compare CPU time or transfer speed.

Collector mode: `perf`. Machine-readable records: [nowait-off-4g-process-counters.json](nowait-off-4g-process-counters.json).

| Client | Instructions / verified GiB | Cycles / verified GiB | Perf context switches / verified GiB | CPU migrations / verified GiB | Voluntary context switches / verified GiB | Involuntary context switches / verified GiB | Stable process tree |
|---|---:|---:|---:|---:|---:|---:|---|

The tracker is restarted between clients to discard stale peer records.
- After the seeder recognizes the complete fixture and just before each timed transfer, its cached file pages are advised out so the cgroup block-read counters are not determined by which client seeded first. This is a best-effort Linux page-cache hint.

## Method and scope

- One caller-supplied local fixture was verified against its pinned SHA-256; a private v1 torrent was generated locally for each client. Clients ran sequentially on the same host and local scratch filesystem.
- Public-swarm interoperability was not run. Controlled seeding used the pinned local fixture and an isolated task-local tracker.
- Each client was configured for a 1024.0 MiB/s global download and upload ceiling and 32 global/per-torrent peer ceiling.
- Sampling interval: 0.5 seconds. A successful download average is verified payload bytes divided by elapsed wall time; for an incomplete download, the reported average uses client completed-byte progress and is paired with the final percentage. The seed transfer timer starts after both clients are ready, the leecher is registered, and the seeder has reannounced; leecher setup time is listed separately. Peak rates come from client payload counters. Mean/peak CPU is Docker CPU percent, where 100% represents one logical core; cgroup CPU seconds per GiB and block read/write bytes per GiB are listed separately. Each client has an idle baseline sampled for 8 seconds before adding a torrent. Cgroup anonymous/file memory and process-tree RSS/PSS are collected for every client. TorrentNG engine memory metrics and detailed smaps are captured only when TNG_CLIENT_BENCH_DIAGNOSTICS=1; when TorrentNG is included, every transfer run captures backend selection once after the measured windows. Diagnostic runs include additional measurement work and must not be compared as clean resource trials. Jemalloc counters appear only when that allocator is selected and do not represent total process allocations.
- Controlled seeding uses a private task-local tracker and one qBittorrent leecher on the same Compose network. No public metadata or swarm request is made; containers are attached only to an internal Docker network, and DHT/PEX are disabled and read back before measurement. The tracker is reset between clients. The no-peer startup limit is 90s; full transfer limit is 900s.

## Confidence and limitations

- Hash validation gives high confidence that successful runs transferred the same fixture.
- Throughput and resource comparisons have low confidence: one controlled seed pass per client, on a shared host with the load reported above.
- Results apply to this host, storage, build, image digests, peer ceiling, rate ceiling, and test window. They do not establish a general ranking, multi-torrent capacity, or likely public seed demand.

## Artifacts and cleanup

- Raw time-series samples: [nowait-off-4g-samples.csv](nowait-off-4g-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [nowait-off-4g-memory-profile.json](nowait-off-4g-memory-profile.json)
- Process-counter profile: [nowait-off-4g-process-counters.json](nowait-off-4g-process-counters.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /var/tmp/torrentng-perf-counters-20261024/r8-nowait-crosscheck/off-4g-work
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed**
