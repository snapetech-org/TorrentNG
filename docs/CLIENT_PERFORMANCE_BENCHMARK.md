# Client performance benchmark

This benchmark compares TorrentNG's native engine with qBittorrent,
Transmission, Deluge, and rTorrent using an official Debian amd64 netinst
torrent. It records public download behavior and a separate controlled seed
transfer for each client. The current dated results are in
[`certification/reports/client-performance-benchmark-20261013.md`](../certification/reports/client-performance-benchmark-20261013.md),
with the combined time-series CSV beside the report. In this run, all four
clients that started completed and verified the ISO; the rTorrent image failed
at startup. The report includes both public download and controlled seeding
KPIs, client versions, the fixed test limits, and the run's confidence limits.

## Run a comparison

The runner needs Python 3, Docker Compose v2, `aria2c`, `findmnt`, and Git. It
builds TorrentNG from the current checkout and uses reference-client images
pinned by digest in `deploy/interop/compose.yml`.

```sh
python3 scripts/client_performance_benchmark.py
```

The default run tests all five clients in sequence. Each public download uses
the live Debian swarm. Each seed test uses an isolated tracker and one common
qBittorrent leecher. The report and CSV are written under
`certification/reports/`; Docker containers, volumes, benchmark images, and
the task's scratch directory are removed when the run exits.

## Storage options

By default, scratch lives under `/var/tmp`. To keep the temporary ISO copies
and client state on the desktop's NFS mount backed by kspls0 LVM, create a
task-specific directory and set `TNG_CLIENT_BENCH_SCRATCH_ROOT`:

```sh
mkdir -p /mnt/datapool_lvm_media/TorrentNG/benchmarks/scratch
TNG_CLIENT_BENCH_SCRATCH_ROOT=/mnt/datapool_lvm_media/TorrentNG/benchmarks/scratch \
  python3 scripts/client_performance_benchmark.py
```

Using NFS for scratch changes the measured storage path and adds NFS traffic to
the run. The report records the filesystem used. Keep every client on the same
scratch filesystem for a comparison.

To measure on local storage and copy the verified ISO and official `.torrent`
to persistent storage after all transfer measurements finish, set
`TNG_CLIENT_BENCH_ARTIFACT_DIR`:

```sh
TNG_CLIENT_BENCH_ARTIFACT_DIR=/mnt/datapool_lvm_media/TorrentNG/benchmarks/inputs \
  python3 scripts/client_performance_benchmark.py
```

This copies only the source ISO and torrent file after the client measurements;
the temporary client downloads are still removed. Existing destination files
are reused only when their size and SHA-256 match.

Useful environment variables:

| Variable | Default | Purpose |
|---|---:|---|
| `TNG_CLIENT_BENCH_CLIENTS` | all five | Select client IDs for a focused run. |
| `TNG_CLIENT_BENCH_SKIP_BUILD` | `0` | Reuse a local TorrentNG image only when its revision label exactly matches the checked-out source commit. |
| `TNG_CLIENT_BENCH_PREBUILT_IMAGE` | `torrentng/torrentngd:interop` | Image checked and aliased for a run when skip-build is enabled. |
| `TNG_CLIENT_BENCH_SEED_SOURCE` | none | Use a local ISO for controlled seeding after checking its published Debian SHA-256. |
| `TNG_CLIENT_BENCH_REPORT` | dated report in `certification/reports/` | Choose the Markdown report path; the CSV uses the same basename. |
| `TNG_CLIENT_BENCH_RATE_BPS` | 33,554,432 B/s | Shared per-client global download and upload ceiling. |
| `TNG_CLIENT_BENCH_SEED_LEECHER_RATE_BPS` | 4,194,304 B/s | Common leecher receive ceiling during the controlled seed test. |
| `TNG_CLIENT_BENCH_MAX_PEERS` | 32 | Peer ceiling applied consistently to the clients. |
| `TNG_CLIENT_BENCH_TIMEOUT_SECS` | 1,800 s | Public download completion window. |
| `TNG_CLIENT_BENCH_SEED_TIMEOUT_SECS` | 900 s | Controlled seed completion window. |
| `TNG_CLIENT_BENCH_SAMPLE_SECS` | 2 s | Sampling interval for rates and resource use. |
| `TNG_CLIENT_BENCH_MALLOC_CONF` | jemalloc defaults | Pass a temporary `MALLOC_CONF` value to TorrentNG for allocator experiments. |
| `TNG_CLIENT_BENCH_TOKIO_WORKER_THREADS` | Tokio default | Set Tokio worker count for a measured TorrentNG run. |
| `TNG_CLIENT_BENCH_STORAGE_IO_WORKER_THREADS` | config default | Override TorrentNG storage I/O workers for the measured run. |
| `TNG_CLIENT_BENCH_STORAGE_HASH_WORKER_THREADS` | config default | Override TorrentNG storage hash workers for the measured run. |
| `TNG_CLIENT_BENCH_STORAGE_PEER_READ_READAHEAD_BYTES` | config default | Override the peer-upload read-ahead size in bytes for a measured run. |
| `TNG_CLIENT_BENCH_STORAGE_PEER_READ_CACHE_ENTRIES` | config default | Override the peer-upload read-ahead cache entry limit; set `0` to disable the cache. |

The common leecher limit applies to the receiver, not the client being tested.
The default is below TorrentNG's per-peer request-window guard; seed results
therefore measure service under a defined receiver load, not maximum upload
capacity. The 32-peer ceiling is applied equally as a reproducible fixture
condition; it is not a KPI or a claim that 32 is an ideal peer count. The report
records configured ceilings so capped runs are clear.

## What the report measures

- **Public download:** completion, wall time, average and sampled peak
  throughput, final progress, peers, container CPU and memory, and SHA-256
  verification against Debian's published checksum.
- **Controlled seeding:** verified bytes delivered to the common leecher,
  transfer time, average and sampled peak upload, peers, container CPU and
  memory, and the leecher's SHA-256.
- **Run context:** client and image versions, TorrentNG source commit, host and
  filesystem, load averages, configured limits, raw samples, and cleanup
  status.

One sequential pass gives **low confidence** for this host, ISO,
swarm window, and configuration. Public swarm availability changes over time;
repeat runs in different orders before drawing a general ranking. A successful
hash check is high-confidence evidence that the completed client received the
same ISO, but it does not establish performance on other networks, disks, or
loads.
