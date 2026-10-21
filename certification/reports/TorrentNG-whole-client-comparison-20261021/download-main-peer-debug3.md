# TorrentNG controlled private download CPU/RAM: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-21T20:33:15+00:00
- TorrentNG source commit: f886f4699997119366fe80d7161a1820bf10879a
- TorrentNG source tree: 6eb7ec9a48789a94e7f1738786b4cd54b8c3c473
- TorrentNG worktree dirty: no
- TorrentNG source fingerprint SHA-256: 306507753fd2d5847f6d282b479a660ca59dc9b8946405b0eea14227ae194fef
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 69.2 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Host load average (1/5/15m): 23.26/18.28/11.07 at start; 14.59/15.89/10.91 at end; sampled 1m peak 19.71
- Fixture: private v1 torrent `debian-13.7.0-amd64-netinst.iso`, 792,723,456 bytes; infohash 7acf8fb590b2060dd9c3146ef770169d593433b0; SHA-256 a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Receiver rate cap: 33,554,432 B/s (32.0 MiB/s); fixed seeder upload cap: 4,194,304 B/s (4.0 MiB/s); maximum peers 32
- Piece assembly cap: config default MiB
- Client order seed: 36105; order: TorrentNG
- Process-tree RSS/PSS sampling interval: 15s; cgroup sampling interval: 2s
- TorrentNG storage backend: live peer scheduler backend=pread; groups=1, fixed buffers=no, registered files=no, max batch=1, fixed buffer bytes=0, buffer strategy=disabled, worker copy=no, frame-pool slots=no
- TorrentNG used the recorded prebuilt image torrentng/torrentngd:benchmark-20261021t201229z; its source, fingerprint, and allocator labels were checked.
- Container images: TorrentNG: torrentng/torrentngd:benchmark-20261021t203315z; fixed qBittorrent seeder: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Fixed seeder version: qBittorrent v5.2.3

## Controlled download results

Only completed transfers with a matching SHA-256 are eligible for CPU/GiB comparison. The receiver cgroup is measured; qBittorrent's fixed seeder runs in its own container and is excluded from receiver CPU/RAM.

| Client | Result | Verified GiB | Time (s) | Average (MiB/s) | CPU (s/GiB) ↓ | Peak cgroup anonymous RAM (MiB) ↓ | Process-tree RSS peak (MiB) | Process-tree PSS peak (MiB) | SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| TorrentNG | FAIL | 0.000 | 59.59 | n/a | n/a | 6.5 | 23.4 | 22.9 | TimeoutError: TorrentNG did not complete controlled_download within 60s |

## Method and limits

- A single SHA-256-verified Debian ISO is served by one pinned qBittorrent seeder capped at 4,194,304 B/s through the task's private Compose network. Each receiver runs alone, with DHT and peer exchange disabled. The tracker is restarted between clients and the seeder reannounces before the next receiver starts.
- The benchmark starts its measured CPU/RAM window after the receiver is ready and the fixed seeder is announcing. CPU seconds per GiB use the receiver's cgroup v2 CPU counter and only hash-verified payload bytes. Anonymous memory is the receiver cgroup peak; process-tree RSS/PSS are sampled independently and can miss short spikes at the configured interval.
- This run measures active private downloads, not the public swarm. The qBittorrent seeder's CPU/RAM is excluded from client rows. Disk counters are not used as a score. One pass on a shared host is exploratory and does not establish an overall client ranking or a general performance claim.

## Artifacts and cleanup

- Raw time-series samples: [download-main-peer-debug3-samples.csv](download-main-peer-debug3-samples.csv)
- Cgroup resource profile: [download-main-peer-debug3-memory-profile.json](download-main-peer-debug3-memory-profile.json)
- Temporary benchmark directory: /mnt/disks/gamespool0/torrentng-whole-client-architecture-comparison-20261021/run-download-main-peer-debug3
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed**
