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

## Owner's measurement priority

The next comparisons prioritize CPU seconds per hash-verified GiB and RAM.
Keep cgroup anonymous memory, process-tree RSS, and PSS as separate measures.
Disk-I/O counters may remain diagnostic context, but comparative I/O ranking
and upload-goodput parity are deferred until CPU and RAM are addressed.
Storage correctness, boundedness, and durability remain required gates. No
numeric efficiency threshold has been adopted, and this priority is not a
claim that the current candidate meets it.

## 2026-10-21 exact-main CPU/RAM check (`457df452`)

A clean image built from current `main` commit
`457df452aeef4afc6f0e16777d05662b94945dc5`. All five clients completed the
controlled seed transfer and passed Debian's SHA-256 check. They ran
sequentially with a single qBittorrent leecher capped at 4 MiB/s; average
upload rates were 3.82–3.94 MiB/s. The table reports cgroup CPU time per
verified GiB and peak cgroup anonymous memory, which excludes charged file
cache.

| Client | Seed CPU (s/GiB) ↓ | Peak anonymous memory (MiB) ↓ | Seed process-tree RSS peak (MiB) |
|---|---:|---:|---:|
| TorrentNG | 2.66 | 25.2 | 42.6 |
| qBittorrent | 2.02 | 18.0 | 779.8 |
| Transmission | 1.33 | 24.8 | 39.1 |
| Deluge | 2.43 | 85.1 | 889.3 |
| rTorrent | 0.55 | 1.9 | 6.9 |

In this pass, TorrentNG used 32% more CPU per GiB than qBittorrent, twice
Transmission, 9% more than Deluge, and 4.8 times rTorrent. Its anonymous
memory was close to Transmission, 7.2 MiB above qBittorrent, below Deluge, and
well above rTorrent. The process-tree RSS figures for qBittorrent and Deluge
are much larger than their anonymous cgroup readings; PSS was unavailable for
those clients, so those RSS values should not be used as a memory ranking.

The desktop had 32 logical CPUs; its one-minute load peak was 13.33, with
1/5/15-minute averages of 4.87/6.39/9.52 at start and 4.66/5.14/6.56 at end.
This was one sequential pass, so the small CPU and memory differences have low
confidence and are not a general client ranking. The benchmark command exited
1 because the 60-second public phases for TorrentNG, Transmission, and Deluge
did not finish; those public results are excluded. qBittorrent and rTorrent
completed the public transfer, but that live-swarm phase is not a controlled
comparison. The report records pinned image digests, exact source, raw
samples, memory profile, and cleanup status:
[`report`](../certification/reports/client-performance-benchmark-457df452-20261021T0525-cpu-ram.md),
[`samples`](../certification/reports/client-performance-benchmark-457df452-20261021T0525-cpu-ram-samples.csv),
[`memory profile`](../certification/reports/client-performance-benchmark-457df452-20261021T0525-cpu-ram-memory-profile.json).

## 2026-10-21 TorrentNG-only repeat (`8a27b32d`)

A TorrentNG-only controlled seed on clean source `8a27b32d6243d22e90bf16706ae5a79afb169f66`
passed the same Debian SHA-256 check at 3.82 MiB/s. It measured **2.57
CPU-s/GiB**, **23.3 MiB cgroup anonymous peak**, **41.5 MiB process-tree PSS**,
and **41.9 MiB process-tree RSS**. This is close to the preceding exact-main
five-client observation (2.66 CPU-s/GiB and 25.2 MiB anonymous peak), but it is
one run on a different host-load window and has no client comparator; it does
not establish a CPU or RAM improvement. The 60-second public probe reached
5.18% and timed out, so it is excluded. The selected storage backend is
unknown: this runner attempted `/metrics` after stopping the container and got
`URLError`. A follow-up runner did reach `/metrics` before container shutdown,
but after pausing TorrentNG; it reported `runtime_initialized=0`. That gauge
belongs to the separate `StorageRuntime` used for plan and import operations,
so it does not reveal which backend the live peer-transfer scheduler used.
Neither earlier run establishes the selected backend.

The [report](../certification/reports/client-performance-backend-observation-8a27b32d-20261021.md),
[samples](../certification/reports/client-performance-backend-observation-8a27b32d-20261021-samples.csv),
and [memory profile](../certification/reports/client-performance-backend-observation-8a27b32d-20261021-memory-profile.json)
record the trial and its limits.

## 2026-10-21 backend-capture follow-up (`3e7a15ed`)

A second TorrentNG-only verified seed on clean source `3e7a15edb5ba8e692c4c797097312daa7eaf0999`
again passed SHA-256 at 3.82 MiB/s. It used **2.73 CPU-s/GiB**, with **24.9
MiB cgroup anonymous peak**, **43.4 MiB process-tree PSS**, and **43.9 MiB
RSS**. Together with the preceding run, CPU was 2.57–2.73 (two-run median
2.65) and anonymous peak was 23.3–24.9 MiB (median 24.1 MiB). These two
single-client runs have no comparator and cannot establish a gain or ranking.
The public probe timed out at 6.04% and is excluded. The runner reached
`/metrics` before container shutdown but after pausing TorrentNG; it reported
`runtime_initialized=0`, which says nothing about the live peer-transfer
backend. Backend identity is still unknown because that version of `/metrics`
did not observe `MountScheduler` owners. The runner now captures after the
seed window and before pausing the torrent. The [report](../certification/reports/client-performance-backend-observation-3e7a15ed-20261021-repeat.md),
[samples](../certification/reports/client-performance-backend-observation-3e7a15ed-20261021-repeat-samples.csv),
and [memory profile](../certification/reports/client-performance-backend-observation-3e7a15ed-20261021-repeat-memory-profile.json)
record this trial.

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
`TNG_CLIENT_BENCH_DIAGNOSTICS=1`. When TorrentNG is included, each transfer run
performs one authenticated `/metrics` scrape after its measured transfer window
and before pausing the final TorrentNG torrent. The live peer-transfer backend
is read from existing `MountScheduler` owners and recorded per scheduler group,
including fixed-buffer and registered-file support and buffer strategy. The
separate `StorageRuntime` gauge remains distinct because it covers plan and
import work, not peer transfers. Earlier runs did not export live scheduler
selection, so their backend is unknown even when the scrape succeeded. This
makes io_uring findings attributable without adding metric scrapes during
scored work. Storage completion ownership is carried by
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

The runner builds with a per-run Docker Buildx builder limited to two CPUs by
default and removes that builder and its cache during cleanup. Override the
limit with `TNG_CLIENT_BENCH_BUILD_CPUS`. It does not prune or modify the shared
builder's cache.

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

## Current-main five-client CPU/RAM pass (`f6cce154`, 2026-10-21)

A clean image with source, fingerprint, and allocator labels matching
`f6cce1544c8e9f3cf260fdd6474e058b765843a8` ran all five clients in sequence on
`kspld0`. Each controlled seed used the same official Debian 13.7 ISO, a
qBittorrent leecher capped at 4 MiB/s, and a final SHA-256 check. All five
controlled seed transfers passed at 3.82–3.94 MiB/s. The sampled one-minute
host load peaked at 8.76 on a 32-logical-CPU host.

| Client | Seed CPU (s/GiB) ↓ | Peak cgroup anonymous (MiB) ↓ | Seed process-tree RSS / PSS (MiB) | Seed rate (MiB/s) |
|---|---:|---:|---:|---:|
| TorrentNG | 2.46 | 25.2 | 44.0 / 43.7 | 3.82 |
| qBittorrent | 2.04 | 18.0 | 778.3 / n/a | 3.94 |
| Transmission | 1.31 | 25.8 | 40.0 / n/a | 3.82 |
| Deluge | 2.27 | 89.1 | 884.7 / n/a | 3.94 |
| rTorrent | 0.56 | 1.7 | 6.6 / 6.5 | 3.94 |

TorrentNG used 21% more CPU per GiB than qBittorrent, 88% more than
Transmission, 8% more than Deluge, and 4.4 times rTorrent. Its anonymous
memory was 7.2 MiB above qBittorrent, 0.6 MiB below Transmission, 63.9 MiB
below Deluge, and 23.5 MiB above rTorrent. This is mixed RAM, not a win across
the field. Process-tree RSS/PSS and cgroup anonymous memory are different
measures; PSS was unavailable for three comparators, and the qBittorrent and
Deluge RSS values do not reconcile with their cgroup anonymous readings.

The previous exact-main pass at `457df452` measured TorrentNG at 2.66 CPU-s/GiB
and 25.2 MiB anonymous peak. This pass is numerically about 8% lower in CPU
with the same anonymous peak, but one run per source cannot attribute that
difference to code. The current run observed the live transfer backend as
`pread`; it did not exercise io_uring. Diagnostic metrics and detailed `smaps`
sampling were disabled.

TorrentNG's public phase timed out at 5.08%, and Transmission timed out at
99.88% after the 60-second cap. Those public rows are excluded; the benchmark
command returned 1 because of those timeouts even though all controlled seed
hashes passed. This one sequential run has low confidence for a general client
ranking and does not prove the CPU/RAM target is met. Randomized client-order
repeats remain open. A later separate diagnostic profile and its current
source follow-up are summarized below. See the [report](../certification/reports/client-performance-benchmark-f6cce154-20261021-cpu-ram.md),
[samples](../certification/reports/client-performance-benchmark-f6cce154-20261021-cpu-ram-samples.csv),
and [memory profile](../certification/reports/client-performance-benchmark-f6cce154-20261021-cpu-ram-memory-profile.json).

## Diagnostic CPU profile and upload-read context candidate (`c6984493`, 2026-10-21)

A separate instrumented TorrentNG-only seed used the same verified Debian ISO,
the `pread` backend, and a qBittorrent receiver capped at 4 MiB/s. The seed
passed its final SHA-256 at 3.82 MiB/s. The benchmark's cgroup counters report
2.77 CPU-s/GiB and 24.7 MiB peak anonymous memory; process-tree RSS/PSS peaked
at 43.0/42.6 MiB. Its public probe reached 3.38% before the 60-second timeout
and is excluded. These are diagnostic-run values, not a clean score or a
comparison with other clients.

Linux `perf` recorded 807 user-cycle samples over the controlled transfer, with
no lost samples. The largest self-sample groups were `__unlock` on Tokio workers
(11.60%), `memcpy` (9.55%), `__libc_free` (6.70%), the peer-loop closure
(5.33%), `nontrivial_free` (4.23%), `MountScheduler` destruction (3.17%), Tokio
timer-entry destruction (3.05%), `AtomicWaker::register_by_ref` (2.52%), and
`RateAdmissionGrant::settle` (2.25%). These are sampled stack weights, not
independent additive costs or proof of a single root cause.

The inclusive call tree attributed 5.56% to `start_upload_reads`, 3.17% to
dropping `UploadReadContext`/`MountScheduler`, and 1.91% to cloning
`MountScheduler`. Inspection showed that every queue refill built a new read
context and cloned it for each scheduled block, including calls where the
in-flight queue was already full. The current candidate builds one context per
peer, shares it with in-flight reads through `Arc`, and removes `Clone` from
the context type so future code cannot accidentally restore those deep clones.
The 30-second read timeout remains in place to preserve stalled-admission
handling. The candidate has passed the requested bulk correctness pass and one
five-client seed measurement. Its measured result did not establish a CPU gain.

## Five-client CPU/RAM pass after upload-context sharing (`c6984493`, 2026-10-21)

The source-bound candidate was built from the same checkout with the shared
upload-read context change. Client order was qBittorrent, Transmission,
TorrentNG, Deluge, then rTorrent. All five controlled seeds passed SHA-256 at
3.82–3.94 MiB/s through the common 4 MiB/s receiver cap. The host's sampled
one-minute load peaked at 17.21 on 32 logical CPUs.

| Client | Seed CPU (s/GiB) ↓ | Peak cgroup anonymous (MiB) ↓ | Seed process-tree RSS / PSS (MiB) | Seed rate (MiB/s) |
|---|---:|---:|---:|---:|
| TorrentNG | 2.54 | 24.4 | 39.3 / 38.8 | 3.82 |
| qBittorrent | 1.87 | 16.9 | 779.1 / n/a | 3.94 |
| Transmission | 1.37 | 23.6 | 37.0 / n/a | 3.82 |
| Deluge | 2.41 | 86.8 | 887.7 / n/a | 3.94 |
| rTorrent | 0.57 | 1.8 | 6.8 / 6.8 | 3.94 |

TorrentNG still used more CPU per GiB than every comparator: 36% more than
qBittorrent, 85% more than Transmission, 5% more than Deluge, and 4.5 times
rTorrent. Its anonymous peak was 7.5 MiB above qBittorrent, 0.8 MiB above
Transmission, 62.4 MiB below Deluge, and 22.6 MiB above rTorrent. Against the
previous `f6cce154` pass, TorrentNG was numerically 0.08 CPU-s/GiB higher and
0.8 MiB lower in anonymous memory. The one-pass result and higher host load do
not establish a change caused by the refactor; no CPU gain is demonstrated.

The public phases for Transmission, TorrentNG, and Deluge timed out and are
excluded. qBittorrent and rTorrent completed their public downloads, but those
live-swarm observations are not part of the controlled comparison. The full
report, samples, and memory profile are
[`report`](../certification/reports/client-performance-benchmark-context-sharing-candidate-20261021.md),
[`samples`](../certification/reports/client-performance-benchmark-context-sharing-candidate-20261021-samples.csv),
and [`memory profile`](../certification/reports/client-performance-benchmark-context-sharing-candidate-20261021-memory-profile.json).

## Per-block upload future allocation candidate (2026-10-21)

The upload-read queue previously boxed a dynamically dispatched future for
each requested block before inserting it into `FuturesUnordered`. The sampled
profile recorded substantial allocator/free activity, but did not attribute
those samples to this allocation specifically. The current candidate keeps the
same unwind boundary, timeout, queue ownership, and cancellation behavior while
storing the concrete future type directly in `FuturesUnordered`; this removes
the outer per-block box and virtual dispatch. `FuturesUnordered` still owns its
per-future node, so this does not eliminate all per-block allocation.

The candidate passed bulk Rust, Clippy, formatting, and Python validation. Its
first controlled seed run passed SHA-256, measuring **2.74 CPU-s/GiB**, **24.2
MiB cgroup anonymous peak**, and **41.8 MiB process-tree RSS peak**. qBittorrent
in the same run measured **2.20 CPU-s/GiB** and **18.5 MiB anonymous peak**;
TorrentNG's idle anonymous mean was lower (2.9 vs. 13.8 MiB). This was one run
on a loaded host, and earlier clients could not seed because no verified source
was available yet. The TorrentNG public probe also stopped at 5.21% and is
excluded. The run does not establish whether removing the future box improved
or regressed CPU/RAM relative to the parent source. No gain is claimed; the
change remains provisional pending randomized, hash-verified paired trials.

See the [run report](../certification/reports/client-performance-benchmark-upload-future-candidate-20261021-round1.md),
[samples](../certification/reports/client-performance-benchmark-upload-future-candidate-20261021-round1-samples.csv),
and [memory profile](../certification/reports/client-performance-benchmark-upload-future-candidate-20261021-round1-memory-profile.json).

## Full rate-grant settlement candidate (2026-10-21)

The diagnostic profile attributed 2.25% of sampled self cycles to
`RateAdmissionGrant::settle`. A fully sent payload settled all admitted bytes
and then called the refund path with zero bytes, taking the shared rate-tree
mutex a second time. Settlement and any nonzero refund now happen under one
tree lock; a full send skips the no-op refund while partial sends still return
unused bytes. Dropped grants retain their refund path. This edit has not yet
gone through a matched performance run, so no efficiency gain is claimed. It
passed the formatted workspace all-target suite on Rust 1.94.0 and 1.88.0
(2,342 passed, 0 failed, 12 ignored across 37 summaries on each), strict
workspace Clippy on both toolchains, formatting, `git diff --check`, and all
357 Python tests. The partial-send regression passed on both toolchains.

The rate-settlement candidate has not yet been measured against its parent
source. The latest available source-bound seed result is the preceding
upload-future candidate: 2.74 CPU-s/GiB and 24.2 MiB cgroup anonymous peak for
TorrentNG, versus 2.20 CPU-s/GiB and 18.5 MiB for qBittorrent in one run. That
run predates this rate-settlement edit and cannot establish its effect. No new
CPU/RAM score was collected while unrelated builds and game processes kept
`kspld0` busy; the comparison remains open.
