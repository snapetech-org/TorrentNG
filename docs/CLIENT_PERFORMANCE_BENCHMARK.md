# Client performance benchmark

This benchmark compares TorrentNG's native engine with qBittorrent,
Transmission, Deluge, and rTorrent using an official Debian amd64 netinst
torrent. It records public download behavior and a separate controlled seed
transfer for each client. The latest comparison and memory investigation are in
[`certification/reports/native-memory-optimization-20261014-agentvm.md`](../certification/reports/native-memory-optimization-20261014-agentvm.md),
with raw time-series and allocator/process profiles beside the report. It
includes public download and controlled seed results, client versions, fixed
test limits, measured memory, and confidence limits. The previous
[unoptimized baseline](../certification/reports/client-performance-benchmark-20261013.md)
is retained to show the before measurements.

The native daemon defaults to the system allocator; jemalloc and mimalloc are
opt-in build features. It also defaults to direct block writes with
`memory.piece_assembly_cap_mb = 0`, avoiding a second in-memory copy of each
incomplete piece. When a jemalloc build is selected, the native container
entrypoint defaults to `MALLOC_CONF=thp:never`, while allowing an operator
override. The report gives the measured effects and their limits: TorrentNG's public
swarm rate was much lower in its one pass, while controlled seed rate was close
to the peers. Its anonymous memory was lower in the recorded peer comparison,
but the ratios combine sequential runs and are not orders of magnitude against
every client.

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

When scratch is on root-squashed NFS, keep client configuration on a writable
local filesystem so Docker does not need to `chown` a bind mount on NFS. For
example, set `TNG_CLIENT_BENCH_CONFIG_ROOT=/var/tmp` while keeping
`TNG_CLIENT_BENCH_SCRATCH_ROOT` on the LVM-backed NFS share. The report records
both filesystems because the download path affects measured performance.

Useful environment variables:

| Variable | Default | Purpose |
|---|---:|---|
| `TNG_CLIENT_BENCH_CLIENTS` | all five | Select client IDs for a focused run. |
| `TNG_CLIENT_BENCH_SKIP_BUILD` | `0` | Reuse an image only when its source commit, source fingerprint, and allocator labels match this checkout. |
| `TNG_CLIENT_BENCH_PREBUILT_IMAGE` | `torrentng/torrentngd:interop` | Image checked and aliased for a run when skip-build is enabled. |
| `TNG_CLIENT_BENCH_ALLOCATOR` | `system` | Select the build allocator: `system`, `jemalloc`, or `mimalloc`. |
| `TNG_CLIENT_BENCH_CONFIG_ROOT` | scratch directory | Put container configuration bind mounts on a writable local filesystem such as `/var/tmp`. |
| `TNG_CLIENT_BENCH_SEED_SOURCE` | none | Use a local ISO for controlled seeding after checking its published Debian SHA-256. |
| `TNG_CLIENT_BENCH_REPORT` | dated report in `certification/reports/` | Choose the Markdown report path; the CSV uses the same basename. |
| `TNG_CLIENT_BENCH_RATE_BPS` | 33,554,432 B/s | Shared per-client global download and upload ceiling. |
| `TNG_CLIENT_BENCH_SEED_LEECHER_RATE_BPS` | 4,194,304 B/s | Common leecher receive ceiling during the controlled seed test. |
| `TNG_CLIENT_BENCH_MAX_PEERS` | 32 | Peer ceiling applied consistently to the clients. |
| `TNG_CLIENT_BENCH_TIMEOUT_SECS` | 1,800 s | Public download completion window. |
| `TNG_CLIENT_BENCH_SEED_TIMEOUT_SECS` | 900 s | Controlled seed completion window. |
| `TNG_CLIENT_BENCH_SAMPLE_SECS` | 2 s | Sampling interval for rates and resource use. |
| `TNG_CLIENT_BENCH_MALLOC_CONF` | allocator default | Set jemalloc options for TorrentNG; pass an empty value to test jemalloc defaults. |
| `TNG_CLIENT_BENCH_MIMALLOC_PURGE_DELAY` | mimalloc default | Override mimalloc's purge delay in milliseconds; requires `TNG_CLIENT_BENCH_ALLOCATOR=mimalloc`. |
| `TNG_CLIENT_BENCH_MIMALLOC_ALLOW_THP` | mimalloc default | Override mimalloc's transparent huge-page policy; requires the mimalloc allocator. |
| `TNG_CLIENT_BENCH_PIECE_ASSEMBLY_CAP_MB` | config default | Override the incomplete-piece assembly cap; `0` tests direct block writes without whole-piece assembly buffers. |
| `TNG_CLIENT_BENCH_PEER_BUFFER_CAP_MB` | config default | Override the process-wide peer buffer cap. |
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
