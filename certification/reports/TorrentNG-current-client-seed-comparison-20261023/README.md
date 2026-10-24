# Current TorrentNG seed comparison — 2026-10-23

## Result

Three randomized rounds compared the current TorrentNG engine with qBittorrent,
Transmission, Deluge, and rTorrent while each client seeded the same 64 MiB
fixture to one local qBittorrent leecher. All **15 transfers passed the same
SHA-256 check**. The current build did **not** meet rTorrent on CPU or process
memory.

| Client | Version | CPU (s/GiB) ↓ | Active CPU (s/GiB) ↓ | Peak cgroup anonymous (MiB) ↓ |
|---|---|---:|---:|---:|
| TorrentNG | current native engine | 2.94 | 2.89 | 7.6 |
| qBittorrent | 5.3.0 | 2.65 | 2.65 | 6.4 |
| Transmission | 4.1.3 | 1.92 | 1.33 | 15.1 |
| Deluge | 2.2.0 | 3.05 | 3.05 | 80.7 |
| rTorrent | 0.16.26 | 1.01 | 0.70 | 3.1 |

Against rTorrent, TorrentNG used **2.9× the whole-window CPU per GiB** and
**4.1× the active-window CPU per GiB**. Peak cgroup anonymous memory was
**2.45× higher**. Process-tree PSS, available for these two processes, was
21.0 MiB for TorrentNG and 19.4 MiB for rTorrent. TorrentNG was also about
11% higher in CPU and 19% higher in anonymous memory than qBittorrent. It used
less anonymous memory than Transmission and Deluge, and slightly less CPU than
Deluge.

These are medians of three 64 MiB passes, not a parity qualification. The host
was shared and under sustained I/O pressure, the fixture is small, and active
goodput was lower for TorrentNG than for the other clients. Treat the values as
a current local screen; they do not establish a general ranking or behavior on
other hardware and swarm workloads.

## Service and memory categories

The same 4 MiB/s receiver cap did not produce identical goodput. TorrentNG's
median active goodput was 3.02 MiB/s, versus 3.51–3.52 MiB/s for the other
clients. Its 64 MiB transfer took 25.3 seconds after both clients were ready;
qBittorrent and Deluge took 19.3 seconds, Transmission 25.3 seconds, and
rTorrent 79.8 seconds. rTorrent's median first-payload delay was 63.6 seconds,
compared with 6.1 seconds for TorrentNG. Since whole-window CPU includes that
wait, active-window CPU is the cleaner per-byte comparison for rTorrent.
rTorrent's active goodput was 3.52 MiB/s versus 3.02 MiB/s for TorrentNG, so
this is not an equal-goodput comparison; the per-GiB CPU measure still shows
the active CPU cost of delivering the verified payload.

| Client | First payload (s) ↓ | Active goodput (MiB/s) ↑ | 64 MiB transfer (s) ↓ |
|---|---:|---:|---:|
| TorrentNG | 6.1 | 3.02 | 25.3 |
| qBittorrent | 3.0 | 3.51 | 19.3 |
| Transmission | 9.1 | 3.52 | 25.3 |
| Deluge | 3.0 | 3.51 | 19.3 |
| rTorrent | 63.6 | 3.52 | 79.8 |

Memory figures need their accounting labels. The primary cross-client process
measure here is cgroup anonymous memory; it excludes file cache. Sampled peak
cgroup file and total-current values are shown separately. Docker working set
is another counter and must not be read as total cgroup memory.

| Client | Peak cgroup anonymous (MiB) ↓ | Peak cgroup file (MiB) | Peak cgroup current (MiB) | Docker working set (MiB) | Process-tree RSS (MiB) | Process-tree PSS (MiB) |
|---|---:|---:|---:|---:|---:|---:|
| TorrentNG | 7.6 | 64.5 | 79.3 | 12.0 | 21.3 | 21.0 |
| qBittorrent | 6.4 | 65.3 | 77.5 | 12.0 | 28.3 | n/a |
| Transmission | 15.1 | 73.6 | 96.6 | 27.0 | 27.9 | n/a |
| Deluge | 80.7 | 74.6 | 161.7 | 149.6 | 203.0 | n/a |
| rTorrent | 3.1 | 96.8 | 101.4 | 73.3 | 19.4 | 19.4 |

TorrentNG's cgroup-current median was lower than rTorrent's, alongside lower
sampled file-cache peaks. That result is useful under a cgroup memory limit,
but it does not change the process-memory comparison: TorrentNG's anonymous
memory and PSS were both higher. These are independently sampled peaks, so do
not subtract the file-cache peak from the cgroup-current peak as if they were
simultaneous. PSS was unavailable for qBittorrent, Transmission, and Deluge, so
their RSS must not be substituted for PSS. The raw sample files contain each
counter by client and phase.

## Exact builds and method

- TorrentNG was built in release mode from source commit
  `c35fe68e8c2ec64da07e5954facfffccbbce8241`, source tree
  `b5f3250ac03760a53e6c9a7ce7fc06eba4c76094`, using the system allocator and
  default runtime/storage settings. Its Engine source is identical to main at
  `7cb3c216`; the intervening main merge changed benchmark tooling and evidence,
  with no `Cargo.toml`, `Cargo.lock`, or `crates/` changes. This comparison
  branch changed benchmark pins and harness only.
- qBittorrent 5.3.0 used image digest
  `e887804fa9de4c60f5f0fb1f3a7e4febe40a8f69fe736f70c4e8589b1f868dab`.
  The scored build is the current 5.3.0 release listed on the [official download
  page](https://www.qbittorrent.org/download); the fixed receiver used qBittorrent
  5.2.3 for every client.
- Transmission 4.1.3 used digest
  `55aaeedf9fc354d34136387e15ee0ba4679096f20398dd2875867e5cd5cf8be5`; the
  version is listed in the [official release history](https://github.com/transmission/transmission/releases/).
- Deluge used digest
  `bf306ddd5e6d4a85d3d05ce1b947ab6e344e7fca1b7740b77c58ce170118c7ff`.
  Its scored executable reported Deluge 2.2.0 and libtorrent 2.0.13.0; 2.2.0
  is the current [PyPI release](https://pypi.org/project/deluge/2.2.0/).
- rTorrent and libtorrent were source-built from v0.16.26 at commits
  `62899bd053c39b70380464e8f64dbcfce92579aa` and
  `21daa3b96a3b8cfe14961545b5115de7ed12da95`. The upstream release pages mark
  [rTorrent v0.16.26](https://github.com/rakshasa/rtorrent/releases/tag/v0.16.26)
  and [libtorrent v0.16.26](https://github.com/rakshasa/libtorrent/releases/tag/v0.16.26).

The fixture was 67,108,864 bytes, SHA-256
`f46d783f5397975f64a3a4628a259fbb6681f5dae379855838badeba88b68e78`, with
private v1 infohash `00be8e58981ad0424b586825b9cf014ff4af1639`. The same qBittorrent
5.2.3 leecher capped its receive rate at 4 MiB/s. Each seed client had a 32
MiB/s global rate ceiling and a 32-peer ceiling. DHT and PEX were disabled; the
containers used an internal Docker network and task-local OpenTracker. No
public torrent metadata or swarm was contacted. The fixture bytes are retained
as local evidence outside Git; the pinned SHA-256 lets future copies be checked
without adding a 64 MiB binary to the repository.

The randomized execution orders were:

| Order seed | Client order |
|---:|---|
| 20261023 | Deluge, qBittorrent, TorrentNG, rTorrent, Transmission |
| 20261024 | Deluge, TorrentNG, Transmission, qBittorrent, rTorrent |
| 20261025 | qBittorrent, Transmission, TorrentNG, rTorrent, Deluge |

CPU is cgroup v2 CPU time divided by verified bytes, scaled to one GiB. The
active-window value excludes the wait before the first sampled positive
progress. Memory samples were taken every two seconds; process-tree RSS/PSS
every two seconds. A two-second interval and a 64 MiB transfer limit the
precision of short active windows. The source image used portable `pread`,
system allocation, and default settings; no PGO or experimental storage
backend was enabled. The post-transfer scheduler scrape reported fixed buffers
and registered files disabled, with a maximum storage batch of one.

## Host pressure and limits

The host was `kspld0` with 32 logical CPUs and 123.4 GiB RAM. One-minute load
started between 22.52 and 35.81 and peaked between 34.74 and 40.95 across the
rounds. Host-wide CPU PSI `some avg10` averaged 0.9–4.4% within the three runs;
I/O PSI `full avg10` averaged 7.2–9.5%. These counters include unrelated host
work and are preserved in `host-pressure.csv`. All clients in a round ran
sequentially, and client order was randomized, but the host was not quiet.

The first failed TorrentNG attempt exposed a harness race: it waited for the
tracker to report a seeder but did not wait for the leecher registration before
forcing a reannounce. The readiness check now requires both tracker counts;
all later scored rounds passed. After the fix, all 66 tests in
`scripts/test_client_performance_benchmark.py` passed. The failed attempts and
diagnostic reports are retained in [`excluded/`](excluded/), with task-local
generated credentials omitted.

This is a local seed-path comparison only. It does not test public downloads,
multi-peer or multi-torrent capacity, idle operation, or maximum upload
throughput. A clean-host, longer-fixture matched-goodput qualification is still
needed before treating the results as a release-level efficiency ranking.

## Evidence

- [Round 1 report](round-01.md) · [samples](round-01-samples.csv) · [memory profile](round-01-memory-profile.json)
- [Round 2 report](round-02.md) · [samples](round-02-samples.csv) · [memory profile](round-02-memory-profile.json)
- [Round 3 report](round-03.md) · [samples](round-03-samples.csv) · [memory profile](round-03-memory-profile.json)
- [Host pressure samples](host-pressure.csv)
- [Excluded attempts and diagnostics](excluded/EXCLUDED-ATTEMPTS.md)

CSV line endings were normalized from CRLF to LF for Git storage; sample values
were not changed.
