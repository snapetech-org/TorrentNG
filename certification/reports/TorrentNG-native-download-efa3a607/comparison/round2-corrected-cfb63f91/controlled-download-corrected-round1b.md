# TorrentNG controlled private download CPU/RAM: debian-13.7.0-amd64-netinst.iso

- Started (UTC): 2026-10-21T22:23:05+00:00
- TorrentNG source commit: cfb63f91b79ead5ce46a3ffc32a79b6250df0a53
- TorrentNG source tree: 01eaaf29b21fa1bd6f2f1d8e37a15c416a54602a
- TorrentNG worktree dirty: no
- TorrentNG source fingerprint SHA-256: d17321068f89eba7b22371fa85a30ba373784813948a5a55e1d4380fd81724dd
- Host: kspld0; 32 logical CPUs; 123.4 GiB RAM; 66.5 GiB available at start; Linux 7.1.7-arch1-1 (x86_64)
- Host load average (1/5/15m): 3.87/6.56/5.60 at start; 2.89/4.32/5.92 at end; sampled 1m peak 22.02
- Fixture: private v1 torrent `debian-13.7.0-amd64-netinst.iso`, 792,723,456 bytes; infohash bde512393bf3804515e883cc3188c408783d7b72; SHA-256 a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355
- Receiver rate cap: 33,554,432 B/s (32.0 MiB/s); fixed seeder upload cap: 4,194,304 B/s (4.0 MiB/s); maximum peers 32
- Piece assembly cap: config default MiB
- Client order seed: 2026102101; order: qBittorrent, TorrentNG, rTorrent, Deluge, Transmission
- Process-tree RSS/PSS sampling interval: 15s; cgroup sampling interval: 2s
- TorrentNG storage backend: live peer scheduler backend=pread; groups=1, fixed buffers=no, registered files=no, max batch=1, fixed buffer bytes=0, buffer strategy=disabled, worker copy=no, frame-pool slots=no
- TorrentNG was built from the recorded checkout with a 16GB TorrentNG-specific persistent BuildKit cache; image source and allocator labels were checked.
- Container images: qBittorrent: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650; TorrentNG: torrentng/torrentngd:benchmark-20261021t222305z; rTorrent: jesec/rtorrent@sha256:994728c59953e5f19c643827024dd06e9ffaf62b8d903a3f1468794415e72e03; Deluge: lscr.io/linuxserver/deluge@sha256:9505c64720afa9e5f0ac1576e660dd6d1e9d4a6733b791f4821e8f496f19f41f; Transmission: lscr.io/linuxserver/transmission@sha256:1a12fef3c89eca48b7be9e7d36b17b4eb4e1bcf5e1ee7fbf372e3a38b562939d; fixed qBittorrent seeder: lscr.io/linuxserver/qbittorrent@sha256:2be038f3421f60f62e8e4bf201f66f385b68e4fbc9ed3ab79051069ea22e2650
- Fixed seeder version: qBittorrent v5.2.3

## Controlled download results

Only completed transfers with a matching SHA-256 are eligible for CPU/GiB comparison. The receiver cgroup is measured; qBittorrent's fixed seeder runs in its own container and is excluded from receiver CPU/RAM.

| Client | Result | Verified GiB | Time (s) | Average (MiB/s) | CPU (s/GiB) ↓ | Peak cgroup anonymous RAM (MiB) ↓ | Process-tree RSS peak (MiB) | Process-tree PSS peak (MiB) | SHA-256 / failure |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---|
| qBittorrent | PASS | 0.738 | 197.34 | 3.83 | 8.218 | 16.6 | 747.9 | n/a | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| TorrentNG | PASS | 0.738 | 245.90 | 3.07 | 6.504 | 6.3 | 24.1 | 23.6 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| rTorrent | PASS | 0.738 | 194.18 | 3.89 | 1.875 | 1.1 | 118.6 | 118.6 | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Deluge | PASS | 0.738 | 194.39 | 3.89 | 6.940 | 84.8 | 855.6 | n/a | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |
| Transmission | PASS | 0.738 | 203.19 | 3.72 | 1.498 | 7.9 | 22.2 | n/a | a7ef94ac2fb9a7fec454552abd629b7cc9d5155c886165a45649f5ce6167e355 |

## Method and limits

- A single SHA-256-verified Debian ISO is served by one pinned qBittorrent seeder capped at 4,194,304 B/s through the task's private Compose network. Each receiver runs alone, with DHT and peer exchange disabled. The tracker is restarted between clients and the seeder reannounces before the next receiver starts.
- The benchmark starts its measured CPU/RAM window after the receiver is ready and the fixed seeder is announcing. CPU seconds per GiB use the receiver's cgroup v2 CPU counter and only hash-verified payload bytes. Anonymous memory is the receiver cgroup peak; process-tree RSS/PSS are sampled independently and can miss short spikes at the configured interval.
- This run measures active private downloads, not the public swarm. The qBittorrent seeder's CPU/RAM is excluded from client rows. Disk counters are not used as a score. One pass on a shared host is exploratory and does not establish an overall client ranking or a general performance claim.

## Artifacts and cleanup

- Raw time-series samples: [controlled-download-corrected-round1b-samples.csv](controlled-download-corrected-round1b-samples.csv)
- Cgroup resource profile: [controlled-download-corrected-round1b-memory-profile.json](controlled-download-corrected-round1b-memory-profile.json)
- Temporary benchmark directory: /mnt/disks/gamespool0/tngb/dl1
- Compose cleanup status: **isolated Compose project, containers, and named volumes removed; benchmark image retained for reuse: torrentng/torrentngd:benchmark-20261021t222305z; BuildKit builder removed; TorrentNG-specific cache retained (maximum 16GB)**
