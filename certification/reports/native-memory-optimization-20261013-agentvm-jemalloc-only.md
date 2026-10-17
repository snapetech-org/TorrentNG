# TorrentNG and reference client performance: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-13T21:30:53+00:00
- TorrentNG source commit: efa6f32b365b66a93fa2823dbe16d6887abbf87d
- TorrentNG source tree: 1faa2fc1dd0471313849626055908d32e08243da
- Host: agent-gui; 8 logical CPUs; 11.7 GiB RAM; 2.1 GiB available at start; Linux 6.12.111+deb13-amd64 (x86_64)
- Docker: client=26.1.5+dfsg1 server=26.1.5+dfsg1; Docker Compose version 2.26.1-4
- Scratch filesystem: /dev/vda1 (ext4)
- TorrentNG MALLOC_CONF: thp:never,narenas:1,tcache:false,dirty_decay_ms:0,muzzy_decay_ms:0
- TorrentNG build mode: built from this checkout
- Requested TorrentNG runtime/storage worker overrides: defaults
- Requested TorrentNG memory-cap overrides: config defaults
- Peak `tokio-rt-worker`-named threads by phase (async + blocking): controlled_seed=6, idle=6, public_download=6
- Host load average (1/5/15m): 0.23/0.46/1.14 at start; 0.17/1.29/1.55 at end; sampled 1m peak 4.91
- Public source: [official Debian torrent directory](https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/); resolved torrent https://cdimage.debian.org/debian-cd/current/amd64/bt-cd/debian-13.7.0-amd64-netinst.iso.torrent
- ISO: debian-13.7.0-amd64-netinst.iso; 792,723,456 bytes; torrent infohash 7acf8fb590b2060dd9c3146ef770169d593433b0
- Official SHA-256: a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Clients selected for this run: TorrentNG
- Client download and upload limits: 33,554,432 B/s (32.0 MiB/s); max peers 32
- Controlled seed leecher receive limit: 4,194,304 B/s (4.0 MiB/s), shared across all seeders. This stays below TorrentNG's peer-upload guard of 4,096 16 KiB requests per 10 seconds (about 6.4 MiB/s), with 37.5% headroom. Seed rates are therefore measured under a controlled receiver load, not as maximum upload capacity.
- Each public download ran alone, in this order: TorrentNG
- Container image references: TorrentNG: torrentng/torrentngd:benchmark-20261013t213053z; seed leecher: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Seed leecher: qBittorrent v5.2.3
- The TorrentNG image was built from the recorded checkout. Reference images are pinned by digest in deploy/interop/compose.yml.

## Public download

| Client | Version | Result | Progress (%) | Time (s) | Average (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | SHA-256 / failure |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | build efa6f32b365b | PASS | 100.00% | 46.4 | 16.31 | 24.33 | 30.51 | 46.07 | 32 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Controlled seeding

Each client seeded the same SHA-256-verified ISO through a private single-file torrent on the isolated Compose OpenTracker. A separate qBittorrent container with BitTorrent encryption disabled and TCP enabled downloaded and checked Debian's published SHA-256. This measures a controlled one-seeder/one-leecher transfer, not public swarm demand. Seed tests use a separate per-client copy of the first SHA-256-verified public download or the explicitly verified seed-source file, and can run even if that client's public-swarm download stalls.

| Client | Result | Leecher setup (s) | Transfer (s) | Average upload (MiB/s) | Peak sampled (MiB/s) | Mean CPU (%) | Peak CPU (%) | Peak peers | Leecher SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | PASS | 4.5 | 206.7 | 3.66 | 4.65 | 2.03 | 6.70 | 1 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Memory profile

Memory uses the container cgroup and Docker working-set counter, sampled at the transfer interval. Anonymous memory excludes charged file cache, so these columns help separate process/runtime growth from data caching. TorrentNG process RSS and its engine allocation counters are in the linked profile. `n/a` means the runtime did not expose that counter.

| Client | Idle container mean (MiB) | Idle anonymous mean (MiB) | Download peak container (MiB) | Download peak anonymous (MiB) | Seed peak container (MiB) | Seed peak anonymous (MiB) |
|---|---:|---:|---:|---:|---:|---:|
| TorrentNG | 11.3 | 9.1 | 41.6 | 18.8 | 166.2 | 25.3 |

The tracker is restarted between clients to discard stale peer records.

## Method and scope

- One current official Debian amd64 netinst v1 torrent per client; clients ran sequentially on the same host and local scratch filesystem.
- Public downloads used the live Debian swarm. All seed tests used the same first SHA-256-verified public download, copied into a separate seed path for each client, so seed performance is measured independently of public-swarm completion.
- Each client was configured for a 32.0 MiB/s global download and upload ceiling and 32 global/per-torrent peer ceiling.
- Sampling interval: 2 seconds. A successful download average is verified ISO bytes divided by elapsed wall time; for an incomplete download, the reported average uses client completed-byte progress and is paired with the final percentage. The seed transfer timer starts after both clients are ready, the leecher is registered, and the seeder has reannounced; leecher setup time is listed separately. Peak rates come from client payload counters. CPU is Docker CPU percent, where 100% represents one logical core. Each client has an idle baseline sampled for 8 seconds before adding a torrent. Cgroup anonymous/file memory and TorrentNG engine allocation metrics help attribute the sampled container footprint.
- Public download uses the current public Debian swarm with a 1800s completion timeout. Controlled seeding uses a private task-local tracker and one qBittorrent leecher on the same Compose network. The tracker is reset between clients. The no-peer startup limit is 90s; full transfer limit is 900s.

## Confidence and limitations

- Hash validation gives high confidence that successful runs transferred the same ISO.
- Throughput and resource comparisons have low confidence: one sequential public-swarm pass and one seed pass per client, on a shared host with the load reported above. Public peer availability changes over time.
- Results apply to this host, storage, build, image digests, peer ceiling, rate ceiling, and test window. They do not establish a general ranking, multi-torrent capacity, or likely public seed demand.

## Artifacts and cleanup

- Raw time-series samples: [native-memory-optimization-20261013-agentvm-jemalloc-only-samples.csv](native-memory-optimization-20261013-agentvm-jemalloc-only-samples.csv)
- Cgroup and TorrentNG allocator profile: [native-memory-optimization-20261013-agentvm-jemalloc-only-memory-profile.json](native-memory-optimization-20261013-agentvm-jemalloc-only-memory-profile.json)
- Preserved source artifacts: not requested
- Temporary benchmark directory: /var/tmp/torrentng-client-bench-20261013T213053Z-by57zdbq
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed**
- Official [Debian netinst page](https://www.debian.org/CD/netinst/)
