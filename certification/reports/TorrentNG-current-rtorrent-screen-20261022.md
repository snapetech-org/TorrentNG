# TorrentNG and reference client performance: seed-128MiB.bin

- Started (UTC): 2026-10-22T00:41:16+00:00
- TorrentNG source commit: 242e1cc4d08600fc6e5f198c22c8f21427abf19d
- TorrentNG source tree: aa43b275d8973856010d0723aaf3848682f4fbd0
- TorrentNG worktree dirty: yes
- TorrentNG tracked diff SHA-256: 9c216b3918334d505806c8dbf136465dd236fc5c2d791cb131612df6a7285c3b
- TorrentNG untracked files: 2; content SHA-256: 9c3f2fd176ef4738439a60241096fc10de4d365b492bbab25dbe3d958047d107
- TorrentNG source fingerprint SHA-256: 61d7d8d9bb0f4023214f1800664f19567038ad0265c4d21d7367fb6838750401
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 74.1 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Docker: client=29.9.1 server=29.9.1; Docker Compose version 5.6.0
- Scratch filesystem: /dev/nvme1n1p1 (btrfs)
- Config filesystem: /dev/nvme1n1p1 (btrfs)
- TorrentNG allocator: system (system-allocator)
- TorrentNG build CPU cap: 4 logical CPUs via persistent BuildKit cache (maximum 16GB)
- TorrentNG MALLOC_CONF: not applicable
- TorrentNG mimalloc purge delay: not applicable
- TorrentNG mimalloc THP override: not applicable
- TorrentNG build mode: built from this checkout; image labels verified
- TorrentNG benchmark image tag: torrentng/torrentngd:benchmark-20261022t004116z
- rTorrent comparator profile: upstream rTorrent v0.16.25 (15ccae27deb9aa1dc8e46ea0ed218a673ccc5732) with libtorrent v0.16.25 (6ad3fecf3e7e7aa86855445587b532a91256f344); image torrentng/rtorrent-benchmark:v0.16.25-15ccae27-lib6ad3fecf
- Benchmark mode: controlled-seed
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- TorrentNG storage backend (one post-transfer `/metrics` scrape): live peer scheduler backend=pread; groups=1, fixed buffers=no, registered files=no, max batch=1, fixed buffer bytes=0, buffer strategy=disabled, worker copy=no, frame-pool slots=no
- Measurement instrumentation: healthchecks disabled for all measured clients; process-tree RSS/PSS sampled every 15s; diagnostic metrics and detailed TorrentNG smaps disabled.
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): diagnostics disabled
- Host load average (1/5/15m): 3.96/3.80/3.63 at start; 2.99/3.42/3.52 at end; sampled 1m peak 4.19
- Fixture source: caller-supplied local file pinned by SHA-256; no public Debian metadata or swarm was contacted.
- Fixture: seed-128MiB.bin; 134,217,728 bytes; torrent infohash 281f10b6cd3b53f7fdc1dc9f8b73d21e58e73162
- Seed fixture page-cache eviction: POSIX_FADV_DONTNEED succeeded for 2/2 seed phases (best-effort kernel hint).
- Pinned fixture SHA-256: 245e6025bb986dc23ee882e1a22acf0f1d552dcf9a2f9069896d54982f03d531
- Clients selected for this run: TorrentNG, rTorrent
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This is 12.5% of the configured per-client upload ceiling. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Public-swarm downloads: not run in controlled-seed mode; interoperability is a separate qualification.
- Container image references: TorrentNG: torrentng/torrentngd:benchmark-20261022t004116z; rTorrent: torrentng/rtorrent-benchmark:v0.16.25-15ccae27-lib6ad3fecf; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image was built from the recorded checkout. Reference images are pinned by digest or upstream source revision in the benchmark Compose files.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | build 242e1cc4d086 | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |
| rTorrent | 0.16.25 | NOT RUN | 0.00% | 0.0 | 0.00 | 0.00 | 0.00 | 0.00 | 0 | public-swarm interoperability is a separate run |

## Controlled seeding

Each client seeded the same SHA-256-verified fixture through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked the fixture SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Each selected client seeded the same caller-supplied fixture after its pinned SHA-256 was checked. Public-swarm testing was skipped by controlled-seed mode.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | PASS | 2.5 | 37.3 | 3.43 | 4.71 | 0.45 | 1.45 | 1 | 245e6025bb986dc23ee882e1a22acf0f1d552dcf9a2f9069896d54982f03d531 |
| rTorrent | PASS | 2.7 | 97.7 | 1.31 | 4.64 | 0.09 | 0.72 | 1 | 245e6025bb986dc23ee882e1a22acf0f1d552dcf9a2f9069896d54982f03d531 |

### First peer, first payload, and active transfer window

The samples show that this was already a local artificial swarm: the private
torrent used a task-local tracker and one qBittorrent leecher on the internal
Docker network. No public tracker, metadata source, or swarm was contacted.
Both clients had one peer by the 3.020-second sample. TorrentNG first showed
payload in that sample; rTorrent first showed payload in the 66.424-second
sample, about 63.4 seconds after a peer was visible. The harness did not record
the request/bitfield exchange or explain this post-peer wait, so its cause is
still open.

Active-window values below are post-processed from the raw samples. The window
starts at the sample immediately before first positive progress and ends at the
last sample; CPU and transferred bytes use the same endpoints. Sampling
interval is about 3.02 seconds, so first-peer and first-payload times are
observations, not exact event timestamps.

| Client | First peer observed (s) | First payload observed (s) | Active window (s) | Active goodput (MiB/s) | Active CPU (s/GiB) |
|---|---:|---:|---:|---:|---:|
| TorrentNG | 3.020 | 3.020 | 36.228 | 3.53 | 1.28 |
| rTorrent v0.16.25 | 3.020 | 66.424 | 33.204 | 3.85 | 0.50 |

This one pair is exploratory. The end-to-end averages still include startup
and were 3.43 MiB/s for TorrentNG and 1.31 MiB/s for rTorrent. The active
window suggests rTorrent used less CPU per GiB once it sent data, while
TorrentNG began useful transfer much sooner. Neither result qualifies a
general efficiency ranking; repeat randomized local pairs and diagnose the
post-peer delay before drawing a service-level conclusion.

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache. Process-tree RSS and PSS are sampled for every client at the configured process-tree interval; RSS can count shared pages more than once, while PSS apportions them. TorrentNG process RSS, engine resource-governor leases, peer-buffer estimates, sampled peer-read cache bytes, and (for Jemalloc builds only) allocator metrics are in the linked profile. Allocator counters do not replace process RSS. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) | Seed process-tree RSS peak (MiB) | Seed process-tree PSS peak (MiB) | Peer-read cache sampled peak (MiB) |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| TorrentNG | 3.5 | 2.9 | n/a | n/a | 21.0 | 14.5 | 31.7 | 31.2 | n/a |
| rTorrent | 3.5 | 2.6 | n/a | n/a | 134.4 | 3.4 | 20.2 | 20.2 | n/a |

For rTorrent, the sampled Docker working-set peak was 134.4 MiB; the cgroup
current peak was 140.4 MiB. The cgroup file-cache peak was about 129.0 MiB,
while anonymous memory peaked at 3.4 MiB.

## CPU and block I/O

Cgroup v2 CPU time and block I/O counters are reported per GiB of observed payload transfer. Public rows use completed-byte progress unless the payload passed its SHA-256 check; seed rows use the leecher's completed-byte progress and count as successful only after its final SHA-256 passes. Block bytes are summed across devices from `io.stat`, so this measures host block traffic charged to the client cgroup, not logical filesystem reads/writes. A page-cache hit does not count as block I/O. `n/a` means fewer than two samples or unavailable cgroup counters.

| Client | Public CPU (s/GiB) | Public block read (MiB/GiB) | Public block write (MiB/GiB) | Seed CPU (s/GiB) | Seed block read (MiB/GiB) | Seed block write (MiB/GiB) |
|---|---:|---:|---:|---:|---:|---:|
| TorrentNG | n/a | n/a | n/a | 1.28 | 1024.00 | 0.12 |
| rTorrent | n/a | n/a | n/a | 0.66 | 1024.00 | 0.00 |

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

## What the current rTorrent source does differently

The pinned rTorrent v0.16.25/libtorrent v0.16.25 source keeps the current
upload chunk handle when another request targets the same piece. For plaintext
uploads, it iterates the requested range in the chunk and writes those mapped
data spans through its socket stream; the encrypted path materializes data in
an encryption buffer. The file-list code builds chunks from shared mapped file
ranges and can span files. This is a mapped-page-to-socket path, not `sendfile`.
It avoids an application-owned plaintext upload copy, while file-cache pages
still count toward total container memory. See the upstream
[upload implementation](https://github.com/rakshasa/libtorrent/blob/v0.16.25/src/protocol/peer_connection_base.cc)
and [mapped chunk construction](https://github.com/rakshasa/libtorrent/blob/v0.16.25/src/torrent/data/file_list.cc).

TorrentNG's measured seed path still reads through its generic storage scheduler
into an owned payload before writing the piece frame. The next low-cost
implementation experiment should test a bounded, verified mapped-range lease
for plaintext TCP, preserving the current fallback. Measure it with the same
local private swarm and collect first-peer, first-payload, active CPU/GiB,
end-to-end completion, anonymous memory, and file-cache memory. Keep a
`sendfile` variant separate so we can retain useful partial wins and understand
the tradeoffs.

## Artifacts and cleanup

- Raw time-series samples: [TorrentNG-current-rtorrent-screen-20261022-samples.csv](TorrentNG-current-rtorrent-screen-20261022-samples.csv)
- Cgroup resource and TorrentNG allocator profile: [TorrentNG-current-rtorrent-screen-20261022-memory-profile.json](TorrentNG-current-rtorrent-screen-20261022-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /mnt/disks/gamespool0/tngb/rtcur22b
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed; BuildKit builder removed; TorrentNG-specific cache retained (maximum 16GB)**
