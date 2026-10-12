# Backend Performance Benchmark Plan

Status: first bounded run completed on 2026-10-12. It is source-bound and
checksum-verified; additional controlled repeats remain open.

## Goal

Attach measurements to the backend engine's resource and storage decisions.
Keep each result tied to the source revision, build, host, storage target,
workload, and run conditions. Treat synthetic tests as evidence for the exact
synthetic case they exercise.

The repository already has focused storage probes and an earlier performance
claims audit. This plan uses those probes, refreshes them against the current
source, and adds process-level measurements during an isolated public torrent
transfer. It does not combine those different workloads into one broad speed
or capacity claim.

## First-run matrix

| Workload | Measurement | Evidence boundary |
| --- | --- | --- |
| Official Debian netinst torrent | Completion, external SHA-256, elapsed time, bytes, API download rate, connected peers | One official v1 torrent on one host; tracker and peer availability affect elapsed time |
| Isolated native daemon | Per-process CPU time and percent of one CPU, sampled RSS, `/proc` block read/write bytes, resource-governor leases, frame-pool cap/in-use bytes | Measures one daemon and one transfer; it does not establish multi-torrent or long-soak ceilings |
| Physical rotational LVM target | `pread`/`io_uring` stream, readahead, file-handle reuse, recheck/hash throughput, shuffled peer-read baseline versus HDD elevator; three elevator trials | Measures the mounted ext4 LV and the harness's access patterns; ordinary writes do not control placement on individual PVs |
| Direct random-read latency on rotational LVM | Three trials of 2,000 synchronous 4 KiB `O_DIRECT` reads from a 128 MiB scratch file; p50/p95/p99 completion latency | End-to-end filesystem/device-mapper/block latency under one observed load; not a direct measurement of physical head travel |

The public transfer is capped at 2 MiB/s down and 1 MiB/s up, with a 32-peer
limit and a 20-minute completion bound. TorrentNG runs at lower CPU and I/O
priority. Its API binds to loopback. The process state, database, and downloaded
ISO use a unique directory under `/var/tmp`; the storage probes use a unique
directory under the selected mount. The harness removes both directories and
retains only its Markdown report and the separate storage matrix report.

The source is the official Debian 13.7 amd64 netinst BitTorrent image. The
harness checks TorrentNG's completed torrent against the ISO's official
`SHA256SUMS` entry before deleting the payload. Debian describes netinst images
and their BitTorrent links on its [official download page](https://www.debian.org/CD/netinst/); the current [amd64 torrent directory](https://ftp.be.debian.org/debian-cd/current/amd64/bt-cd/) lists the torrent.

## Execution

Build the release daemon on `kspls0` from the source revision under review, then
run:

```sh
TNG_BENCH_BIN=target/release/torrentngd \
TNG_BENCH_STORAGE_ROOT=/mnt/datapool_lvm_media \
scripts/backend_engine_benchmark.sh

TNG_STORAGE_LATENCY_ROOT=/mnt/datapool_lvm_media \
scripts/storage_random_latency_probe.sh
```

The script can be pointed at another official single-file ISO with
`TNG_BENCH_TORRENT_URL`, `TNG_BENCH_TORRENT_NAME`, and
`TNG_BENCH_SHA256SUMS_URL`. It fails before creating scratch if either port is
already occupied or the requested storage root is not writable.

The first run used source revision `181f2453e62a4a02cab2f3a22f97e6fbc610d005`
and tree `12bccc8037cef1aba657400954d60bfbce79f431`. The daemon and harness
were pinned to CPUs 30–31; the outer command used nice 15 and child processes
ran at effective nice 19 with idle I/O priority. The daemon received a 2 MiB/s
download cap and a 1 MiB/s upload cap.

## First run results

| Probe | Result | Confidence and scope |
| --- | --- | --- |
| Debian 13.7 netinst torrent | 836,665,344 bytes; complete in 422 s; mean 1.89 MiB/s; official SHA-256 matched | **High** confidence in completion and checksum; **low** confidence in throughput as a host baseline because the host was heavily loaded |
| Daemon CPU and memory during transfer | Mean 7.22% and peak 13.47% of one core; peak RSS 71.91 MiB; CPU time 31.06 s | **Moderate** confidence for this single capped transfer; does not establish scale or long-soak limits |
| Process I/O and managed memory | 14,409,728 bytes read and 1,606,553,600 bytes written; peak sampled governor usage 35,126,290 of 536,870,912 bytes; sampled storage-frame use stayed at 0 of 134,217,728 bytes | **Moderate** confidence for this workload and instrumentation; process I/O is block I/O, not network traffic |
| HDD shuffled-read matrix | 128 MiB per baseline/elevator trial; 2.97× median baseline/elevator wall-clock ratio across 3 trials; each elevator trial coalesced 8,192 reads to 1 backend read | **Low** confidence in elapsed-time ratio on this busy host; **high** confidence in the reported operation counts for this harness case |
| Direct random-read completion latency | 2,000 synchronous 4 KiB `O_DIRECT` reads per trial: p50 was 0.218/0.159/22.822 ms, p95 was 50.15/65.07/129.88 ms, and p99 was 114.80/149.75/352.73 ms | **Low** confidence as a target latency baseline: p50 changed sharply between trials and host load remained high. This measures end-to-end completion latency, not actuator-only seek time |

Host load was 170.43/158.33/130.80 at benchmark start and
214.09/192.77/162.29 at the end on a 32-logical-CPU host. The process was
affinity-limited to two CPUs. Throughput and HDD elapsed time therefore need
quiet-host repeats before they support general performance claims. The
reported RSS is an observed peak for this torrent; the 512 MiB governor cap is
not a process-memory limit.

## Host controls and result quality

The preflight and run on `kspls0` found load well above the 32-thread CPU
count, active disk traffic, and other busy services. The run was therefore
bounded, pinned to two CPUs, and low priority. The harness records load,
available RAM, system-wide I/O samples, per-process counters, and storage
topology. Shared-host contention can depress or vary throughput; use the
confidence levels in the first-run table rather than treating these results as
quiet-host baselines.

Process RSS and resource-governor leases are recorded separately. The 512 MiB
governor limit applies to selected managed allocations; it is not a process-RAM
limit. The `rt-metrics` 100k-row and hot-set tests remain synthetic proxies and
will not be presented as actual daemon memory ceilings.

## Follow-up stages

1. Repeat the public-transfer run for a second official Linux distribution and
   repeat each run enough times to report distributions, not just one elapsed
   value.
2. Compare TorrentNG with a reference client on the same host, ISO, storage
   target, rate cap, peer limits, and time window. Keep swarm variability
   visible and repeat in alternating order.
3. Sweep active torrent counts and peer counts while measuring process RSS,
   governor leases, file descriptors, task count, CPU time per GiB, and API
   latency. Include idle, downloading, seeding, and recheck phases.
4. Measure preallocation and durability modes on scratch targets with
   filesystem and mount details recorded. Report completion latency and
   process/block I/O deltas alongside correctness results.
5. Repeat storage scheduling measurements on a quiet HDD and a physical SSD or
   NVMe target. Repeat the direct random-read latency probe and report p50/p95/
   p99 separately from the shuffled-read elevator result. Keep HDD results
   separate from SSD/NVMe results.

The first run is not considered complete evidence for CPU or memory claims
until these controlled repeats exist. It is a source-bound measurement pass
that identifies what the current implementation does under one real transfer
and one physical storage workload.

## Evidence

See the [backend resource benchmark report](../certification/reports/backend-engine-benchmark-20261012T174337Z.md)
and [storage matrix report](../certification/reports/storage-hardware-20261012T174336Z.md).
The [direct random-read latency report](../certification/reports/storage-random-latency-20261012T175749Z.md)
records the initial p50/p95/p99 sample.
