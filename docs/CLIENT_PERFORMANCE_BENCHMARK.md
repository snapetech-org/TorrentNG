# Client performance benchmark

This benchmark compares TorrentNG's native engine with qBittorrent,
Transmission, Deluge, and rTorrent using an official Debian amd64 netinst
torrent. It records public download behavior and a separate controlled seed
transfer for each client. The previous upload-accounting comparison used
commit `3191f56e` and is recorded in
[`client-performance-benchmark-3191f56e-20261020-upload-accounting-cache-disabled.md`](../certification/reports/client-performance-benchmark-3191f56e-20261020-upload-accounting-cache-disabled.md).
All five controlled seeds passed SHA-256 checks. At a shared 4 MiB/s receiver
cap, TorrentNG used 3.84 CPU-s/GiB, above qBittorrent (2.33), Transmission
(1.47), Deluge (2.54), and rTorrent (0.72). Its cgroup anonymous peak was
13.0 MiB; the same profile's TorrentNG process anonymous peak was 6.8 MiB,
which does not reconcile with the cgroup reading. The other clients' cgroup
anonymous peaks were 20.6, 23.3, 91.1, and 1.7 MiB, respectively, so the
TorrentNG memory row needs attribution before using it to rank memory use.
Block reads were 1,031.77 MiB/GiB and writes 0.67 MiB/GiB. This is one
sequential pass on a loaded host. TorrentNG, Transmission, and Deluge did not
finish the 60-second public-swarm phase, so those rows are not download
rankings.

## 2026-10-20 hot-path follow-up

The uploaded source review and verified evidence archive are preserved at
[`certification/reports/TorrentNG-performance-review-2026-10-20/`](../certification/reports/TorrentNG-performance-review-2026-10-20/),
with the original ZIP alongside it. The review identified instrumentation
contamination and repeated work around storage completion, upload admission,
and TCP piece transmission. The code changes have passed the affected crate
tests and Clippy, and a corrected five-client comparison plus a TorrentNG-only
cache-disabled probe have completed. The results are exploratory; they do not
yet show that this candidate improves whole-client efficiency.

An exact clean-source five-client baseline was completed at `f5272e3c` before
these hot-path changes. All five controlled seed transfers passed SHA-256.
TorrentNG measured 5.10 CPU-s/GiB, compared with 2.96 for qBittorrent, 1.62 for
Transmission, 3.18 for Deluge, and 0.71 for rTorrent. The host load averaged
14.04 at the start. That run still used the old measurement path: healthchecks
were active, `/metrics` was sampled during transfer, and process-tree memory
was not recorded. It is a useful baseline, not evidence of change impact or a
general client ranking. The [report](../certification/reports/client-performance-benchmark-f5272e3c-20261020-current-source-60s.md),
[samples](../certification/reports/client-performance-benchmark-f5272e3c-20261020-current-source-60s-samples.csv),
and [memory profile](../certification/reports/client-performance-benchmark-f5272e3c-20261020-current-source-60s-memory-profile.json)
record the full workload and environment.

The implementation disables container healthchecks symmetrically, samples
process-tree RSS/PSS for every client, and keeps detailed TorrentNG `/metrics`,
`smaps`, and process-spike attribution behind
`TNG_CLIENT_BENCH_DIAGNOSTICS=1`. Storage completion ownership is carried by
the actual Pread or io_uring job, removing the forwarding Tokio task and
second completion channel while retaining operation guards through completion
and invalidating the peer-read cache after writes. Upload admission grants
uncontended reads under one lock and rotates only peers with queued work; the
device scheduler now allocates its waiter channel only when it must queue.

The TCP upload path now moves a single-region storage buffer into immutable
payload bytes and writes the piece header plus payload with partial-write-safe
vectored I/O. Cross-file and padding assembly and the uTP encoder keep their
existing paths. The frame pool now caps idle retention at 16 MiB per pool,
down from an 84 MiB possible total under the former per-class limits. Read
acquisitions can reuse a buffer without clearing it because the backend marks
it readable only after a complete overwrite; generic acquisitions clear reused
buffers before exposing them. Multi-block socket batching and the registered-
buffer copy path remain unimplemented. The existing bounded piece-assembly
configuration remains disabled by default; compare a small global assembly
budget as a separate download experiment before changing that default.

The corrected five-client pass used source fingerprint
`435a6d705876e9c02d22d9ce7bfd3475b477034c24d5bfa9adca0d74cb973d8e` from a
dirty worktree based on `8bd30a47`. All five controlled seeds passed final
SHA-256 verification. At the shared 4 MiB/s receiver cap, TorrentNG measured
3.97 CPU-s/GiB, 1,255.56 MiB/GiB block reads, and 0.54 MiB/GiB writes. The
comparators measured 2.29/984.59/0 for qBittorrent, 1.41/1,067.06/0 for
Transmission, 2.78/992.28/0.10 for Deluge, and 0.76/982.84/0 for rTorrent.
TorrentNG therefore used more CPU and read more block data in this one pass.
The result began at host load 10.25/14.45/14.44 and peaked at 26.18, so it is
not a qualification score or a causal A/B result.

TorrentNG's controlled-seed cgroup anonymous peak was 16.3 MiB, process-tree
RSS was 33.44 MiB, and PSS was 33.10 MiB. The corresponding anonymous peaks
were 17.6 MiB for qBittorrent, 24.9 MiB for Transmission, 83.7 MiB for Deluge,
and 1.7 MiB for rTorrent. Process-tree RSS/PSS were 782.75 MiB/unavailable,
39.05 MiB/unavailable, 884.62 MiB/unavailable, and 6.73/6.72 MiB respectively.
RSS/PSS includes each container process tree and is distinct from cgroup
anonymous memory; the very large qBittorrent and Deluge RSS values do not
reconcile with their cgroup anonymous values. TorrentNG's anonymous memory is
competitive with qBittorrent and Transmission in this run, while rTorrent is
lower. No single memory number establishes a general ranking.

The public phase did not finish for TorrentNG (5.87%), Transmission (99.81%),
or qBittorrent (99.46%) before its 60-second cap; those rows are not download
rankings. rTorrent and Deluge finished. A TorrentNG-only cache-disabled probe
measured 3.65 CPU-s/GiB, 1,016.32 MiB/GiB reads, 7.4 MiB cgroup anonymous
peak, and 25.91/25.58 MiB process-tree RSS/PSS. That looks lower than the
full-pass default-cache sample, but the runs had different host loads and only
one observation each. Keep the cache default unchanged until an alternating,
matched repeat measures the effect. The [corrected five-client report](../certification/reports/client-performance-benchmark-hotpath-candidate-full-20261020.md),
[samples](../certification/reports/client-performance-benchmark-hotpath-candidate-full-20261020-samples.csv),
and [memory profile](../certification/reports/client-performance-benchmark-hotpath-candidate-full-20261020-memory-profile.json)
are retained with the [incomplete first attempt](../certification/reports/client-performance-benchmark-hotpath-candidate-20261020.md),
[default-cache TorrentNG repeat](../certification/reports/client-performance-benchmark-hotpath-candidate-torrentng-20261020.md),
and [cache-disabled probe](../certification/reports/client-performance-benchmark-hotpath-cache-disabled-torrentng-20261020.md)
and their raw data. The first attempt is kept as failure evidence; TorrentNG
had no verified ISO seed source in that run, so its seed phase did not execute.

The `3191f56e` accounting-batching candidate then had two TorrentNG-only
cache-disabled repeats:
[`repeat 1`](../certification/reports/client-performance-benchmark-3191f56e-20261020-upload-accounting-repeat-1-cache-disabled.md)
and
[`repeat 2`](../certification/reports/client-performance-benchmark-3191f56e-20261020-upload-accounting-repeat-2-cache-disabled.md).
All three candidate seed transfers passed hash verification. Their CPU
readings were 3.84, 3.72, and 4.53 CPU-s/GiB (median 3.84), versus 3.74,
3.98, and 4.52 (median 3.98) for the three `7545fc0` cache-disabled runs.
The ranges overlap, so the 3.5% median difference is not a demonstrated CPU
gain. Median seed goodput was effectively unchanged at 3.82 versus 3.81
MiB/s. Median block reads changed from 1,016.66 to 1,031.77 MiB/GiB (+1.5%),
and writes from 0.54 to 0.52 MiB/GiB; this is no clear I/O improvement.
Process anonymous peak medians were 6.84 MiB for the candidate and 7.73 MiB
for the baseline, while cgroup anonymous peak medians moved from 7.73 to
12.91 MiB. The cgroup/process gap is unresolved, so there is no supported
memory-efficiency claim. The candidate five-client pass also began under
higher host load than the baseline pass. These results keep batching as an
exploratory candidate rather than a proven efficiency win.

The prior `7545fc0` five-client report and its two cache-disabled repeats
remain the baseline. That source measured 3.74 CPU-s/GiB in the full pass,
then 3.98 and 4.52 in the TNG-only repeats; the second repeat also recorded
2,092.75 MiB/GiB block reads, an unexplained outlier. The zero-byte cache is
not accepted as a default based on these trials. Two earlier 8 MiB-cache runs
measured 4.46 and 4.75 CPU-s/GiB, 17.4 MiB anonymous peaks, and 1,016.66 and
1,016.20 MiB/GiB reads. All these runs are exploratory cross-run comparisons
on a shared desktop, not a randomized qualification scorecard. Reports,
samples, and memory profiles are checked in beside each report.

The [slskR idle reports](https://github.com/snapetech-org/slskR/blob/2207a92/benchmarks/artifacts/20261012-idle-container-slskr-slskd.md)
measured about 16x lower CPU and 85–89% lower process RSS in a disconnected
comparison with slskd; its [connected-idle report](https://github.com/snapetech-org/slskR/blob/2207a92/benchmarks/artifacts/20261012-live-soulseek-idle.md)
measured about 30x lower CPU. Those runs had no transfers, shares, searches,
or UI/API traffic. They show an idle-footprint result for the pinned images
and setups, not the cause of the gap and not active-transfer efficiency.
TorrentNG's earlier transfer harness sampled an 8-second no-torrent baseline,
but it did not separate DHT-off from network-enabled idle and queried engine
metrics during that interval. The harness now has a distinct idle-only mode
with three longer profiles: no torrents and no Docker network endpoints;
no torrents with DHT disabled while network-attached; and no torrents with DHT
enabled while network-attached for bootstrap and routing maintenance. It
randomizes and records the complete client/network/DHT profile schedule. It
records cgroup CPU seconds, memory domains, block I/O, network-namespace RX/TX, and
process-tree RSS/PSS. RSS and PSS come from host `/proc`;
RSS sums resident pages and can count shared pages more than once, while PSS
divides shared pages proportionally. They are sampled less often than the
cgroup counters and are reported separately from cgroup memory and Docker's
working-set value. The harness makes no API requests during the measurement
window. DHT being enabled does not prove a peer connection; namespace traffic
is recorded separately and is not DHT-specific. The isolated profile verifies
that all Docker network endpoints are detached for the warmup and measurement
window, then restores them before client API readback. DHT and PEX state and
zero torrent count are read back outside the measured windows; Deluge's DHT
and PEX readback requests only those keys through `core.get_config_values`.
For rTorrent, disabled and enabled DHT profiles use `disable` and `on`,
respectively; the measured setting is read back from `dht.statistics`.
The profiles do not reproduce slskR's Soulseek-server-connected workload,
but the isolated profile provides a genuine disconnected baseline. An earlier
partial attempt was stopped after the pinned Deluge and rTorrent clients
rejected invalid config readback calls; observations from those attempts are
not valid benchmark results.

slskR also measured a share-cache serializer that wrote output directly:
50,000 entries used 20 allocations versus 408,828 for its prior serializer,
with identical output. That allocation count is a path-specific diagnostic,
not a general throughput claim. Its incremental database updates avoid
rewriting a full projection for each search result, and query-plan checks
found indexes that helped particular pages plus one redundant index to remove.
These are useful techniques for persistence and serialization paths, not
direct replacements for TorrentNG's piece-send loop. TorrentNG already has
incremental registry deltas, copy-on-write API projection chunks, and native
snapshot probes from 1k to 100k rows; future work should compare allocation
counts and query plans on the corresponding TorrentNG paths before changing
them. The current transfer candidate applies the same measurement discipline:
upload-accounting batching was retained as an exploratory code change, but its
matched cache-disabled seed runs do not establish CPU, memory, or I/O gains.
The [slskR performance analysis](https://github.com/snapetech-org/slskR/blob/2207a92/docs/performance-analysis.md)
is a useful model for keeping each workload and claim explicit.

TorrentNG's 2026-10-20 seed profile attributed 13.7% of sampled cycles to
SHA-1 compression and 8.9% to `SyncWaker` notification/registration. The
accounting-batching candidate reduces actor notifications: successful piece
sends add their exact byte count to a peer-shared counter, then notify the
torrent actor at 64 KiB or after 25 ms. With a seed-ratio or idle limit active,
the threshold falls to one block so limit checks remain per-block. The pending
total and threshold share one per-peer allocation, and the flush timer stays
on the peer task's stack. Peer removal and shutdown wait for the sender to
stop, then drain any remaining bytes. The three candidate seed runs show no
clear CPU or I/O improvement over the three cache-disabled baseline runs.
Process anonymous memory is lower, but cgroup anonymous memory rises and is
not reconciled with process samples. Keep this as an exploratory candidate;
the reports do not establish an efficiency gain.

Earlier evidence remains useful with its scope. The
[October 14 comparison](../certification/reports/native-memory-optimization-20261014-agentvm.md)
and [October 13 baseline](../certification/reports/client-performance-benchmark-20261013.md)
are historical whole-client measurements. The 2026-10-20 direct-engine
comparison against urtorrent covers a different fixture and answers a
different question.

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

Transfer mode needs Python 3, Docker Compose v2, Docker Buildx, `aria2c`,
`findmnt`, and Git. Idle mode needs Python 3, Docker Compose v2, Docker Buildx,
`findmnt`, and Git. Both modes build TorrentNG from the current checkout
unless a matching image is selected, and use reference-client images pinned
by digest in `deploy/interop/compose.yml`.

The runner builds with a per-run Docker Buildx builder and removes that builder
and its cache during cleanup. It does not prune or modify the shared builder's
cache.

```sh
python3 scripts/client_performance_benchmark.py
```

To run only the idle profiles, without downloading or seeding a payload:

```sh
TNG_CLIENT_BENCH_MODE=idle python3 scripts/client_performance_benchmark.py
```

Idle mode uses a 30-second warmup and a 60-second measured window per profile
by default. Override those with `TNG_CLIENT_BENCH_IDLE_WARMUP_SECS` and
`TNG_CLIENT_BENCH_IDLE_MEASURE_SECS`. The client/network/DHT profile order is
seeded and recorded in the report. Process-tree RSS/PSS are sampled
every 15 seconds by default;
`TNG_CLIENT_BENCH_IDLE_PROCESS_SAMPLE_SECS` controls that interval. An idle run
has no payload denominator, so CPU, disk, and network activity are reported
per minute instead of per GiB.

The default run tests all five clients in sequence. Each public download uses
the live Debian swarm. Each seed test uses an isolated tracker and one common
qBittorrent leecher. The report and CSV are written under
`certification/reports/`; Docker containers, volumes, the benchmark image and
its task-scoped BuildKit cache, and the task's scratch directory are removed
when the run exits.

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
| `TNG_CLIENT_BENCH_TOKIO_WORKER_THREADS` | daemon default (maximum 2) | Set Tokio worker count for a measured TorrentNG run; the daemon default scales down with available parallelism, and `TOKIO_WORKER_THREADS` overrides it. |
| `TNG_CLIENT_BENCH_STORAGE_IO_WORKER_THREADS` | config default | Override TorrentNG storage I/O workers for the measured run. |
| `TNG_CLIENT_BENCH_STORAGE_HASH_WORKER_THREADS` | config default | Override TorrentNG storage hash workers for the measured run. |
| `TNG_CLIENT_BENCH_STORAGE_PEER_READ_READAHEAD_BYTES` | config default | Override the peer-upload read-ahead size in bytes for a measured run. |
| `TNG_CLIENT_BENCH_STORAGE_PEER_READ_CACHE_ENTRIES` | config default | Override the peer-upload read-ahead cache entry limit; set `0` to disable the cache. |
| `TNG_CLIENT_BENCH_STORAGE_PEER_READ_CACHE_BYTES` | config default | Override the retained peer-read cache byte budget; set `0` to disable cached readahead reuse. |

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
- **Resource cost per payload GiB:** cgroup v2 CPU time from `cpu.stat` and
  block read/write bytes and operations from `io.stat`, plus process RSS,
  cgroup memory, and TorrentNG engine/allocator observations. The report
  normalizes CPU seconds and block bytes by observed payload bytes; unavailable
  cgroup counters are recorded as `n/a`.
- **Run context:** client and image versions, TorrentNG source commit, host and
  filesystem, load averages, configured limits, raw samples, and cleanup
  status.

The cgroup block counters measure device traffic attributed to the client
container. They do not count page-cache hits or represent logical filesystem
bytes. Compare those values only when every client uses the same filesystem,
cache state, payload, and phase. CPU seconds per GiB are cgroup usage deltas
normalized by bytes transferred; CPU percentages remain available for
diagnosing the time series.

One sequential pass gives **low confidence** for this host, ISO,
swarm window, and configuration. Public swarm availability changes over time;
repeat runs in different orders before drawing a general ranking. A successful
hash check is high-confidence evidence that the completed client received the
same ISO, but it does not establish performance on other networks, disks, or
loads.

## Upload-read scheduling experiment (October 20, 2026)

A candidate moved each upload block read from its own Tokio task into a boxed
future polled by the peer loop. Two controlled seed passes delivered and
verified the same Debian ISO through one qBittorrent leecher capped at 4 MiB/s.
The first pass ran TorrentNG first; the second ran Transmission before
TorrentNG. Both were on `kspld0`, where the sampled one-minute host load peaked
at 29.21 and 27.54 respectively.

| Measurement | Previous three-pass TorrentNG range | Candidate pass 1 | Candidate pass 2 |
|---|---:|---:|---:|
| Seed goodput (MiB/s) | 3.80–3.82 | 3.81 | 3.81 |
| CPU (s/GiB) | 3.99–5.07; median 4.87 | 5.05 | 4.28 |
| Peak cgroup anonymous memory (MiB) | 11.8–15.3 | 23.9 | 27.7 |
| TorrentNG process RSS high-water mark (MiB) | 30.1–32.8 | 41.9 | 45.8 |
| Block read (MiB/GiB) | 165.57–165.80 | 166.11 | 215.21 |
| Block write (MiB/GiB) | 0.39–0.63 | 0.46 | 0.39 |

The candidate CPU readings average 4.67 CPU-s/GiB, a small change from the
previous median that stays within the previous run range. Anonymous memory and
process RSS were consistently higher, while the second block-read result
varied substantially. This does not establish a CPU or I/O improvement. The
peer-loop future implementation is present in current source `7545fc0`; its
earlier benchmark does not isolate a CPU gain, and the later cache-budget runs
do not separate its effect from the other changed code. The one-peer test does
not attribute the separate high-peer request-bookkeeping risk.

The raw reports, CSV samples, and memory profiles for pass 1 and pass 2 are
checked in under `certification/reports/`.
Both public-swarm probes timed out in the short diagnostic window; the
controlled seed rows passed hash verification. Pass 2 has a different overall
worktree fingerprint because `deploy/certification/README.md` changed one
documentation line between runs. The Rust crate inputs were unchanged and the
container build reused the cached Rust build layer. These results apply only
to the recorded source and fixture; they are not a whole-client resource
ranking.

## Single-region upload path experiment (October 20, 2026)

A candidate avoided the per-request region vector and cloned file path for
single-file peer requests, and moved ordinary owned storage frames directly
into the upload payload. Multi-file requests and cached `Bytes` retained their
existing copy path to preserve bounded memory ownership.

One qBittorrent/TorrentNG pass used the same hash-verified Debian ISO and one
qBittorrent leecher capped at 4 MiB/s. TorrentNG's controlled seed completed at
3.81 MiB/s and verified the leecher hash. It measured 5.32 CPU-s/GiB, 13.3 MiB
peak cgroup anonymous memory, 31.4 MiB process RSS high-water mark, and 0.38
MiB/GiB block writes. In the same pass, qBittorrent measured 1.94 CPU-s/GiB
and 18.6 MiB peak cgroup anonymous memory. The previous three reference passes
measured 3.99–5.07
CPU-s/GiB (4.87 median), 11.8–15.3 MiB anonymous memory, 30.1–32.8 MiB RSS,
and 0.39–0.63 MiB/GiB block writes. Those passes used source revision
`7a944072`; `78eff536` contains later engine changes, so these values are
historical context rather than a source-matched before/after baseline. The
candidate did not show lower CPU use. Its anonymous memory was lower than the
same-pass qBittorrent result, but that is one run and falls within the earlier
TorrentNG range; it does not show that this code change caused a memory win.

TorrentNG recorded 1,016.25 MiB/GiB of block reads, versus 165.57–165.80 in the
baseline passes. This I/O result is inconclusive: TorrentNG ran second, the
host had substantial concurrent file activity and a sampled 1-minute load
peak of 25.56, and host-wide cache pressure can change which reads reach the
block device. The run had one pass and does not establish that the code caused
the difference. Its public-swarm probe timed out at 3.79%; only the controlled
seed result passed.

The code candidate was reverted because it showed no measured CPU improvement
and no attributable memory improvement. Its report, samples, and memory profile
remain under `certification/reports/` as one-peer diagnostic evidence, not a
whole-client comparison.

## Deep-audit storage and memory candidate (October 21, 2026)

One controlled seed pair compared clean `bf37a330` against the integrated
storage/cache candidate in a dirty worktree based on the same commit. Both runs
used the same SHA-256-verified Debian ISO and a qBittorrent leecher capped at
4 MiB/s. The candidate fused portable `pread` preparation and completion and
added shared backing-memory accounting and reclaim. Only TorrentNG was scored;
the qBittorrent container was the receiver, not a scored comparison client.

| TorrentNG source | Seed hash | CPU (s/GiB) | Goodput (MiB/s) | Peak cgroup anonymous (MiB) | Peak process-tree PSS (MiB) |
|---|---|---:|---:|---:|---:|
| Clean `bf37a330` | pass | 3.29 | 3.82 | 23.5 | 38.7 |
| Integrated dirty candidate | pass | 3.06 | 3.82 | 25.0 | 43.1 |

CPU was about 7% lower in the candidate pass, while anonymous peak memory rose
1.5 MiB and process-tree PSS rose 4.4 MiB. The candidate's sampled one-minute
host load peak was 21.17 versus 14.95 for baseline. With one pass per source on
a shared host, this is a CPU-positive hint, not a demonstrated efficiency win;
it does not show a RAM reduction. The single-torrent workload does not exercise
concurrent cross-torrent cache pressure. Both 60-second public-swarm phases
timed out and are excluded from download comparisons.

The full reports and raw evidence are
[clean-main baseline](../certification/reports/torrentng-deep-audit-baseline-20261021.md)
and [integrated candidate](../certification/reports/torrentng-deep-audit-candidate-20261021.md),
with adjacent CSV and memory-profile files. Repeat these runs and test
multi-torrent memory pressure before claiming a CPU or RAM improvement.
