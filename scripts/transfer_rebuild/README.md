# Transfer Rebuild Harness

This directory contains rebuild-specific qualification tools. Outputs belong
in a task-owned scratch directory or certification artifact directory, never
in the measured source checkout.

## Validation status

The latest valid source-bound PERF-110 runs use release source
`01a3464d5a6b1f3496dde2f91ab35dcc931089a3`; they compare a 128 MiB private-v1
`N1/lan_1g` transfer against urtorrent `0.14.3`. The detailed four-worker and
two-worker results, pair counts, artifact hashes, and limits are in PERF-110
of `docs/TRANSFER_CORE_ACTIVE_SPECS.md`. These are engine-runner results, not
whole-client rankings, and the two worker profiles use different randomized
fixture hashes.

Those profile runs used a four-worker default. The two-worker profile kept
download goodput near 76 MB/s and had about 13–16% lower TorrentNG RSS/PSS
than the four-worker profile. Runtime source `3191f56e` now defaults to two
workers and batches ordinary upload-accounting notifications at 64 KiB or
25 ms; seed-ratio and seed-idle limits retain per-block accounting. The
exact-source workspace tests and strict Clippy passed on Rust 1.94.0 and
1.88.0, and the repository Python suite passed 329 tests plus 157
subtests. The candidate has also had three hash-verified resource runs. Those
runs showed no clear CPU or I/O change from the cache-disabled baseline, and
the process/cgroup memory readings remain unreconciled. They do not qualify a
resource-efficiency gain.

In the two-worker scorecard TorrentNG still used about twice urtorrent's RSS
and wrote 13.38 MiB/GiB while seeding versus 0.97 MiB/GiB. Its download CPU
cost also remained higher. These are the main resource gaps for the next
measurement and profiling pass; cgroup memory and energy were unavailable.
qBittorrent was only a helper in these runs. The pre-cutover reference is
`354ad504f75b33c712b5ccd827cfef3eb0cf4414`, built with one isolated
benchmark-egress backport in baseline commit
`8023ce77b6bea4d237b4a7e329b56901fab94ba1`; no transfer or picker code
changed in that backport. The earlier full bulk run on exact source
`354ad504` passed the workspace on Rust 1.94.0 and 1.88.0 with 2,302 passed,
0 failed, and 12 ignored across 37 summaries on each toolchain. Release
`rt-engine` passed 734 tests; 22 transfer-rebuild Python scripts and 11
runnable ignored cases passed. Strict Clippy, formatting, the trusted
bubblewrap contract task (67/67), and runner-policy suite (4/4) passed.

The `354ad504` run predates later production changes. On runtime source
`af99733fd462532488a0eda208743fdb1b807cb6`, the fresh post-reconciliation bulk pass had 2,314 passed, 0 failed,
and 12 ignored results across 37 workspace groups on both Rust 1.94.0 and
1.88.0. Strict Clippy passed on stable 1.99.1 and both supported toolchains;
Python passed 329 tests plus 157 subtests; formatting passed; release
`rt-engine` passed 743 tests. The 11 runnable ignored diagnostics passed on
the parent source before the atomic-only compatibility fix. The PERF-110
paired scorecards still use source `01a3464` and do not qualify this newer
runtime for matched performance. The exact test and scale outputs are summarized in
[`TRANSFER_CORE_ACTIVE_SPECS.md`](../../docs/TRANSFER_CORE_ACTIVE_SPECS.md).

The standalone Rust 1.98.1 urtorrent runner passed all six tests and its help
smoke; its format check used Rust 1.94.0 because rustfmt is not installed for
1.98.1.

The release 100k catalog case admitted paused rows in 213.730 s, restored in
2.055 s, and listed the first sorted page in 807 ms; the cached page took under
1 ms. In 16 mixed-load rounds, default and sorted list p95 were 773.313 ms and
799.208 ms, transfer-info p95 was 3.225 ms, and `/metrics` p95 was 181.965 µs.
Cold snapshot construction drives list p95. Seven release storage tests passed
on `/tmp` tmpfs (`profile=Unknown`, `fs=None`); 256 MiB positioned I/O reported
5,913.90 MiB/s writes and 8,840.05 MiB/s reads, readahead served 4,096 reads
with 133 backend reads, and a 64 MiB recheck reported 151.51 MiB/s. The
64-file/64 MiB move-import-delete and 10,000-frame peer-wire probes passed.
These results are diagnostics, not physical-device qualification or
whole-client efficiency evidence. Full commands and logs are recorded in
[`TRANSFER_CORE_ACTIVE_SPECS.md`](../../docs/TRANSFER_CORE_ACTIVE_SPECS.md) and
`/tmp/tng-transfer-core-final-20261019.GSdus5/`.

A follow-up local Btrfs run passed all seven storage probes after the
shuffled-read benchmark was capped at 256 in-flight requests. The 64-file
move/import/delete probe passed too. This is local filesystem evidence, not
ext4 device qualification.

The fixed-counterpart PERF-110 comparison ran ten randomized cold-cache
pairs for each build on the same 128 MiB private v1 profile, against the pinned
TorrentNG-compatible urtorrent 0.14.3 direct library runner, with 1 Gbit/s and
1 ms shaping. The environment-template and fixture-profile digests match. Each
run contains only TorrentNG as a measured candidate, so its validated
scorecard correctly emits zero candidate-to-candidate pairs; the table below
summarizes TorrentNG trial records directly.

| Mode | Pre-cutover baseline | Current candidate |
|---|---:|---:|
| Download | 0/10 completed; no valid goodput | 66.63 MB/s median (65.96–102.39), 10/10 complete |
| Seed | 12.92 MB/s median (12.63–13.24), 10/10 complete | 12.95 MB/s median (12.84–13.24), 10/10 complete |

The pre-cutover build failed every download attempt before the helper verified
the payload; median task-interface receive was about 0.59 MiB, not verified
useful data. It has no download goodput sample, so there is no numeric download
speedup to report. Seeding goodput is effectively unchanged. Median seed CPU
fell from 5.96 to 4.60 CPU-s/GiB; median peak RSS/PSS fell from 36.24/33.37 to
35.65/32.77 MiB. This is a controlled pre-cutover/current result, not a
whole-client ranking or paired-bootstrap comparison: plan-specific fixture
payload hashes differ even though the profile and environment inputs match.

The earlier post-choker-fix scorecard on source `7bc43bf` remains historical.
The latest N1 engine-runner scorecards on source `01a3464` compare TorrentNG
with urtorrent. Exact medians, confidence intervals, source pins, sample gates,
and artifact hashes are recorded under PERF-110 in
[Transfer Core Active Specs](../../docs/TRANSFER_CORE_ACTIVE_SPECS.md). The
current source also fixes disconnect-triggered choker refill, replaces peer
scans and sorted snapshots with picker-owned endgame counts, and charges
retained upload-request bookkeeping to the shared PeerBuffer budget. A
synthetic 1/4/16/64-peer endgame probe measured lower picker-query cost and no
candidate-vector allocation. Multi-peer process-memory attribution and
whole-client comparisons against qBittorrent, Transmission, and Deluge remain
open.

An older reciprocal run reported 83.77/13.07 MB/s downloading and 12.88/93.08
MB/s seeding (TorrentNG/urtorrent), but changed both endpoints between client
rows. Those figures remain historical and cannot isolate which endpoint caused
the difference.

The 16-read upload-window candidate was tested in a second source-bound
10-pair run on the same local-NVMe Btrfs host profile. All 40 records completed
with verified hashes and achieved cold-cache state. TorrentNG's median seed
goodput was 12.84 MB/s versus 12.88 MB/s in the 8-read baseline, so the change
did not improve the intended path; its default is back to 8. The candidate
download median was lower than the earlier run and remains unexplained. Exact
candidate results and artifact digests are recorded under PERF-110 in
[`TRANSFER_CORE_ACTIVE_SPECS.md`](../../docs/TRANSFER_CORE_ACTIVE_SPECS.md).

The direct `urtorrent` preflight now starts with a per-process 64 MiB
`RLIMIT_MEMLOCK`; the earlier default 8 MiB limit caused `io_uring_setup` to
fail. One compatibility preflight and the scoped pair passed; broader client
and profile coverage remains open. TorrentNG's pinned paired-run config enables
`tracker.allow_private_egress` for the isolated `198.18.0.0/15` tracker and
peer endpoints; this reserved network remains denied by default. Hosted
Transfer Rebuild and Windows Smoke passed on
`94c8c1b3ff14106db93ab1a9750a1bb00c808b7c`, before this harness-only change.
The protected hardware job was skipped because no runner is registered. The
current read-only `kspls0` check confirmed the ext4 LVM path and 25 T free,
but load averages were 89.93/89.71/94.70 with 17 users, so no new hardware
benchmark was run. Prior dedicated ext4 evidence remains bound to `eb5f3158`.
Additional pinned-client trials, active-transfer/process resource qualification,
protected-runner hardware evidence, crash-cut, soak/canary,
and release evidence remain open; no whole-client efficiency or superiority
claim is made.

The disconnect-triggered choker refill, picker-owned endgame counts, and
governor-backed inbound request accounting are in earlier measured source.
The two-worker default and upload-accounting batches are in runtime source
`3191f56e`, whose full local test and Clippy pass is recorded above. Its
three-run resource comparison did not show a clear CPU or I/O gain, and
multi-peer memory attribution remains open. The ledger remains 12/63 `done`,
51/63 `wip`, and 0 `todo`; those WIP rows need the runtime and external
acceptance evidence listed above. See
[`TRANSFER_CORE_ACTIVE_SPECS.md`](../../docs/TRANSFER_CORE_ACTIVE_SPECS.md)
for the full run record.

## Independent process observer

`observe.py` samples a candidate process tree from `/proc` and cgroup v2 from a
separate process. It records raw samples as CSV and emits a peak summary JSON.
The summary keeps RSS, PSS, cgroup anonymous memory, and cgroup current memory
as separate values. It also records cgroup file-backed and `inactive_file`
reclaimable-cache counters separately when the kernel exposes them. Unavailable
counters remain `null`; failed per-process descriptor reads do not produce a
partial FD total. CSV and summary artifacts are created exclusively with mode
`0600` and are never overwritten by a later observer run. The CSV and summary
must share a directory. The `transfer-resource-observation-v1` summary records
the CSV digest and binds the observed root PID and process start time.

Example:

```sh
python3 scripts/transfer_rebuild/observe.py \
  --pid 12345 \
  --interval-ms 250 \
  --network-interface tngveth0 \
  --stop-file /path/to/task-scratch/transfer-run/observer.stop \
  --csv /path/to/task-scratch/transfer-run/resources.csv \
  --summary /path/to/task-scratch/transfer-run/resources.json
```

The caller creates the stop file after the measured process exits. The observer
does not start or signal the client. Its cumulative disk counters are Linux
`/proc/<pid>/io` task counters. When `--network-interface` is supplied, network
bytes are read from that interface in the root process's network namespace via
`/proc/<pid>/net/dev`. The summary records the namespace inode and RX/TX deltas
from the first to last valid sample; missing interfaces, namespace changes,
counter resets, or fewer than two valid samples leave the totals `null`. If the
interface or process disappears after at least two valid samples, the summary
keeps the delta through the last valid sample and labels it partial; samples
after the gap are not included. This preserves valid observations from failed
trials without implying that they cover the full transfer interval.
Network counters include every process using that namespace and interface, so
the runner must isolate exactly one client there before treating them as that
client's traffic. The summary JSON repeats this scope warning. Link the matching
network-control record to the trial.

The result schema is described in
[`measurement-contract.md`](../../certification/transfer-rebuild/measurement-contract.md).

## Native client process supervision

`process_supervisor.py` supplies the PERF-110 runner's native-process
lifecycle primitive. It accepts a pinned absolute executable and an explicit
argument array, opens and hashes the executable without following its final
path component, and executes that same opened file. It never interprets shell
syntax or inherits the invoking process environment; callers provide the
specific environment entries the client needs. Logs are new mode-`0600` files
created below an existing no-symlink directory tree.

The returned handle binds PID, `/proc` start time, and executable device/inode.
Its `runtime_identity()` fields match the native identity used by
`adapters.py`; an optional per-trial PID file is created exclusively with mode
`0600`. Shutdown uses a Linux pidfd and refuses to signal after that identity changes.
The client command must stay in the foreground; daemonizing wrappers are not
valid launch targets. `paired_runner.py` now connects this primitive to the
native and container adapters, isolated seed/target endpoints, task tracker,
resource observer, trial writer, and manifest assembler. Its regression suite
passed on the exact source. Paired runtime trials have started, but no run has
yet produced validated whole-client measurement artifacts.

## Trial record assembly

`trial_record.py` captures the measurement harness identity and assembles
versioned trial records from one paired-plan entry, the client adapter's
identity/settings observation, the independent resource observation, workload
and environment data, phase timings, byte accounting, integrity, and outcome.
The top-level `identity` binds the TorrentNG source revision and dirty state,
the Python interpreter, toolchain, and a versioned lockset digest. The lockset
digest is SHA-256 over canonical JSON containing the SHA-256 values of
`Cargo.lock` and `scripts/transfer_rebuild/requirements.txt`.

Client and resource observations are schema-checked before their process IDs,
start times, settings, and counters are copied into the trial. A complete
payload transfer must account for the full useful payload, pass applicable
integrity checks, and link independent resource evidence. A startup failure is
recorded as `unmeasured` with null client identity and no observer link; a
missing client observation cannot be relabeled as another outcome.
`write_trial()` creates a new, exclusive mode-`0600` JSON file and syncs the
file and its directory. It returns the artifact digest for manifest assembly.
Unobserved received, written, read, and duplicate byte counters remain `null`;
the runner must not fill missing client instrumentation with zeros.

`client_control.py` provides the identity-bound transfer API used by the
paired runner for TorrentNG, qBittorrent, Transmission, Deluge, rTorrent, and
rqbit. It admits metainfo only after validating its canonical info hash, adds
it paused, reads status by exact hash, starts or pauses it, and removes it by
exact hash. Where an API supports it, the caller must explicitly choose whether
to delete downloaded data; rTorrent's control API cannot make that choice, so
the caller removes its task-owned output directory separately. Runtime
identity and settings are checked around mutations and at a bounded polling
interval. `urtorrent` remains controlled by its dedicated Rust runner.
`paired_runner.py` composes these controls with process/container launch, task
namespaces, the isolated seed peer, observer lifecycle, fixture verification,
and `manifest.py`. Client-control regressions are in
`scripts/test_transfer_rebuild_client_control.py`; trial-record regressions are
in `scripts/test_transfer_rebuild_trial_record.py`; trial/manifest validation
regressions remain in `scripts/test_transfer_rebuild_validator.py`. These
regressions passed on exact main `eb5f3158`; pinned-client trials remain
separate qualification.

## Trial and manifest validation

Install the pinned schema validator and validate each record before adding it
to a manifest:

```sh
python3 -m pip install -r scripts/transfer_rebuild/requirements.txt
python3 scripts/transfer_rebuild/validate_artifact.py trial /path/to/trial.json
python3 scripts/transfer_rebuild/validate_artifact.py resource-observation /path/to/resources.json
python3 scripts/transfer_rebuild/validate_artifact.py manifest /path/to/manifest.json
python3 scripts/transfer_rebuild/validate_artifact.py cache-state /path/to/cache-state.json
```

The validator checks the versioned Draft 2020-12 schema, including date-time
formats and the rules that apply when a trial claims a complete outcome. For a
manifest, it also resolves trial and artifact paths beneath the manifest
directory, rejects symlinks and path escapes, checks SHA-256 values, validates
each referenced trial record, and compares its pair/outcome/source identity to
the manifest. The writer requires a resource-observation digest for newly
published complete trials; validation matches linked summaries to the trial's
process and resource totals, checks the CSV digest, and recomputes peak and
network totals from the streamed rows. Historical complete trial-v1 records
without that link remain readable, but the scorecard excludes them. It reports
field paths and returns nonzero for malformed JSON,
schemas, records, or referenced files. The focused validator regressions are in
`scripts/test_transfer_rebuild_validator.py`; these regressions passed on exact
main `eb5f3158` in the final 21-script regression pass.
Every external JSON record is decoded strictly through
`strict_json.py`: duplicate member names, nonstandard constants, and numbers
that overflow the finite range are rejected before schema or semantic checks.
The same parser covers evidence files and rollout inputs, client pin/profile
inputs, client API readbacks, container and network-command identity output,
catalog-load log records, paired-runner output, and referenced cache, network,
client, tracker, fixture, and trial records.
Cold/warm trials must link a verified `transfer-cache-state-v1` artifact;
manifest validation checks that the observed fixture and requested state match
the trial and rejects a partial or unsupported state claim.

`manifest.py` assembles trial outcomes and evidence artifacts into one
source-bound manifest. Give it the exact committed worktree root, a new
manifest path outside that worktree, every trial JSON file (when trials exist),
and each additional artifact with its schema kind:

```sh
python3 scripts/transfer_rebuild/manifest.py \
  --repo /path/to/clean/TorrentNG \
  --output /path/to/task-scratch/run-01/manifest.json \
  --trial /path/to/task-scratch/run-01/trials/trial-01.json \
  --artifact transfer-client-observation-v1=/path/to/task-scratch/run-01/artifacts/client.json \
  --artifact transfer-resource-observation-v1=/path/to/task-scratch/run-01/artifacts/resources.json \
  --artifact transfer-resource-samples-v1=/path/to/task-scratch/run-01/artifacts/resources.csv
```

At least one trial or one evidence artifact is required. Evidence-only
manifests are supported for qualifications such as SYS-014, where inventing a
trial record would misstate the workload. For a catalog-only report, omit
`--trial` and pass both linked catalog artifacts:

```sh
python3 scripts/transfer_rebuild/manifest.py \
  --repo /path/to/clean/TorrentNG \
  --output /path/to/task-scratch/run-01/catalog-manifest.json \
  --artifact transfer-catalog-load-report-v1=/path/to/task-scratch/run-01/catalog-load-report.json \
  --artifact transfer-catalog-load-log-v1=/path/to/task-scratch/run-01/catalog-load.log
```

The evidence files must be regular files below the manifest directory and
outside the measured checkout. The writer rejects dirty source worktrees,
source-commit or worktree mismatches, duplicate trial IDs, symlinks, duplicate
evidence paths, and output replacement. It hashes each input, validates the
complete manifest and all referenced files, checks the inputs and source again,
then publishes the mode-`0600` manifest atomically. Failed, invalid,
incomplete, unmeasured, and successful trials are all retained. Manifest-writer
regressions are alongside the validator cases.

## Paired scorecard

`scorecard.py` builds a descriptive comparison from a validated manifest. It
groups by workload case, mode, protocol, and pinned environment settings, then
pairs observations by `pair_id`. Per-run artifact digests, topology IDs, and
randomized per-pair network seeds do not split otherwise comparable groups;
the paired records must still match on fixture digest, network seeds, and
environment settings. `fixture_sha256s` lists every exact fixture in a group.
When a group spans multiple fixtures, `fixture_sha256` is the SHA-256 of the
canonical JSON encoding of that sorted digest list. The scorecard excludes
incomplete or invalid outcomes, unverified cache states, and missing resource
evidence, then reports per-pair ratios with a deterministic percentile-bootstrap
95% interval for the median ratio. RSS, PSS, cgroup
memory, CPU, I/O, descriptors, threads, and energy remain separate metrics;
unavailable measurements stay absent or `null`.
`minimum_sample_met` is informational. The scorecard applies no performance
threshold and does not establish that one client is superior.

```sh
python3 scripts/transfer_rebuild/scorecard.py \
  --manifest /path/to/task-scratch/transfer-run/manifest.json \
  --output /path/to/task-scratch/transfer-run/scorecard.json \
  --bootstrap-samples 10000 \
  --seed 17
python3 scripts/transfer_rebuild/validate_artifact.py scorecard \
  /path/to/task-scratch/transfer-run/scorecard.json
```

The scorecard retains pair-level metric samples, manifest digest, comparison
signature, excluded-pair reasons, and sample counts. Output is created
exclusively with mode `0600`. Safety limits cap input trials, clients per
comparison group, and total resampled values. Regression sources are in
`scripts/test_transfer_rebuild_scorecard.py`; these regressions passed on exact
main `eb5f3158` in the final 21-script regression pass.

## Independent capacity ceilings

`ceilings.py` derives diagnostic upload/download limits from a
`transfer-ceiling-input-v1` calibration record. Collect network, storage,
hash, and private-peer RTT evidence independently of the candidate transfer;
record each tool identity and link each raw file into the manifest as
`transfer-calibration-raw-v1`. The input also pins the host profile, filesystem,
storage device, network profile, and torrent protocol.

Example:

```sh
python3 scripts/transfer_rebuild/validate_artifact.py ceiling-input \
  /path/to/task-scratch/ceiling-input.json
python3 scripts/transfer_rebuild/ceilings.py \
  --input /path/to/task-scratch/ceiling-input.json \
  --output /path/to/task-scratch/ceiling-report.json
python3 scripts/transfer_rebuild/validate_artifact.py ceiling \
  /path/to/task-scratch/ceiling-report.json
```

The report uses the protocol's independent hash calibration and computes the
request-pipeline limit from peer count, request depth, block size, and measured
RTT. Its client/version/settings digest and JSON pointer tie request depth to
the client's process-bound settings readback. Manifest validation verifies raw evidence digests and recomputes the
report from its linked input. Trials can optionally reference the report by
digest when the host, storage, network profile, and protocol match. A ceiling
is diagnostic context; it is not measured client performance or scorecard
evidence. Regression sources are in `scripts/test_transfer_rebuild_ceilings.py`
and `scripts/test_transfer_rebuild_validator.py`; these regressions passed on
exact main `eb5f3158` in the final 21-script regression pass.

## Client identity and settings adapters

`adapters.py` captures a versioned `transfer-client-observation-v1` record for
TorrentNG, qBittorrent, Transmission, Deluge, rTorrent, rqbit, and the direct
`urtorrent` library runner. The pin file
uses schema version `transfer-client-pins-v1`; it must pin the expected client
version, an expected settings object, and either an immutable container image
digest or a native binary digest and live executable identity. Native source
commit, dirty state, lockfile digest, and toolchain are also pinned when a
source checkout is available. HTTP credentials
come from named environment variables and are never copied to the observation.
The output is created exclusively with mode `0600`.
PERF-110 profiles also declare an absolute `preflight_save_path` that is
available inside the client as a disposable base directory; the preflight adds
a unique child path for each case. For the direct `urtorrent` runner, that base
directory must already exist on the host. `preflight_timeout_seconds` optionally
sets its bounded runner timeout (default 60 seconds, maximum 300).

Example invocation:

```sh
python3 scripts/transfer_rebuild/adapters.py qbittorrent \
  --pins /path/to/task-scratch/client-pins.json \
  --output /path/to/task-scratch/client-observation.json
python3 scripts/transfer_rebuild/validate_artifact.py client-observation \
  /path/to/task-scratch/client-observation.json
```

The adapter checks the observed version and declared settings before writing.
It binds a running container to its image digest, container ID, command hash,
published loopback API port, network namespace listener, and process start
time. Container port mappings on wildcard or non-loopback host addresses are
rejected, including a second public mapping of the same API port; host-network
containers must have only loopback API listeners. Native API ports must have
only loopback listeners owned by the pinned process. The native binary digest
is captured from one no-follow regular-file handle and checked for mutation
while hashing. Native processes are matched to that exact executable inode.
When source is available, observations also bind the source, lockfile, compiler,
and process start identity against the expected toolchain pin. Their loopback
API socket must belong to that process. Trial records point to the observation
artifact by SHA-256; manifest validation checks that the trial's client,
version, runtime, settings digest, and process identity agree with that artifact.

The APIs expose different settings surfaces. TorrentNG readback covers its
API-visible listen port and DHT/PEX switches; rTorrent uses the profile's
explicit XML-RPC method map. rqbit's HTTP API reports server identity and
runtime status but does not expose effective settings, so its profile must
exactly pin the running command line, an explicit environment-variable
allowlist, and any relevant configuration-file digests. This captures launch
inputs and pins defaults through the exact client version; it does not claim
that rqbit reports hidden resolved settings. Native adapters require their
loopback API listener to be owned by the pinned process; container adapters
require the loopback API port to map to the pinned container's network
namespace. qBittorrent, Transmission, and Deluge read back their documented
Web API/RPC settings surfaces. The expected settings contract should include
every setting relevant to a paired trial.

The `urtorrent` profile pins a standalone native runner built against exactly
`urtorrent = 0.14.3`. The runner embeds the library in the measured process;
there is no daemon/API wrapper. Its private settings snapshot records the live
`Session::settings()` and per-torrent readback, and binds both to the runner's
PID and `/proc` start time, including the assigned listen port for peer setup.
The adapter also pins the runner binary digest and checks that the snapshot
belongs to that same process instance. Include the
runner process in resource observation; its small control loop is part of the
measured candidate and may matter in small workloads. The public settings API
does not expose the hash-pool thread count, so the runner records it separately
as a command-line input rather than an effective readback.

The runner is Linux-only and refuses to start outside a named task-owned
`tngt-*` network namespace. It requires private v1 metainfo and rejects v2 or
hybrid fixtures. It binds only to an explicit task-local IPv4 interface,
disables IPv6, DHT, PEX, and LSD, retains exactly the task-local tracker, and
removes web-seed endpoints before release. The tracker returns the pinned seed
only to the target; the seed gets an empty peer list so only the target dials.
Task-network addresses
come from the reserved `198.18.0.0/15` pool; private, link-local, and loopback
addresses are also accepted for isolated probes. The adapter independently
verifies the listener and peer addresses in the process-bound snapshot. It
starts only after the common file barrier is released. It requires the pinned
Rust 1.98.1 toolchain and a kernel where io_uring is available; this comparator
does not qualify v2/hybrid behavior.

Build to a task-owned target directory so build artifacts stay out of the
checkout:

```sh
CARGO_TARGET_DIR=/path/to/task-scratch/urtorrent-target \
  cargo +1.98.1 build --release --locked \
  --manifest-path scripts/transfer_rebuild/urtorrent_runner/Cargo.toml
```

The caller prepares an existing empty download directory or verified seed
directory, creates the `tngt-*` namespace, and starts the runner with
`--namespace`, `--torrent`, `--save-path`, `--pid-file`, `--settings-file`,
`--start-gate`, `--mode download|seed`, required `--listen-v4 IP`, required
`--peer IPv4:PORT`, and required `--tracker-url http://IP:PORT/announce`.
The tracker host must match the pinned peer IP. Both addresses must belong to
the isolated task network or use a private, link-local, or loopback range.
The runner also checks that the process is in the named namespace. Download
mode requires an empty directory; seed mode checks that the existing payload
is complete before waiting at the barrier. The runner never deletes files. Its
settings snapshot is created exclusively with mode `0600`.

The caller supplies a fresh start-gate path that is absent at process launch,
waits until every participant has published its ready state, then creates the
gate file. The runner's snapshot is available before the gate, so the harness
can use its effective listen port when it configures the other peer.

The runner emits final status counters and `barrier_to_trial_end_ms`. The
`downloaded` counter can include corrupt and redundant payload, so the fixture
validator remains the source of useful bytes. The duration is diagnostic: it
includes resume/connect setup and completion polling, plus the configured seed
observation window for seed trials. It does not replace the contract's
externally measured `transfer_ns`.

Regression sources are in `scripts/test_transfer_rebuild_adapters.py` and the
runner's `src/main.rs`; adapter and validator regressions passed in the exact
source suite. The standalone release runner also built and passed all 6 tests
under pinned Rust 1.98.1, and its CLI help smoke passed. Linux io_uring
preflight and pinned-client qualification remain open.

## Shared trial measurement primitives

`measurement.py` provides a task-directory file barrier, a per-file interval
union that separates unique accepted bytes from duplicate overlap, and a
monotonic timeline for barrier, transfer, flush, validation, and shutdown.
Callers must create and clean a fresh barrier directory for each trial. The
barrier creates readiness and release markers exclusively as regular files and
rejects symlinks or non-file markers. These helpers do not launch clients,
establish integrity, or calculate a scorecard.
Their regression sources are in `scripts/test_transfer_rebuild_measurement.py`;
they passed on exact main `eb5f3158` in the final 21-script regression pass.

TorrentNG builds expose three cumulative, site-scoped payload-copy counters on
`/metrics`. Capture one immutable observation at each trial boundary with the
pinned client profile:

```sh
python3 scripts/transfer_rebuild/adapters.py torrentngd \
  --pins /path/to/task-scratch/client-pins.json \
  --output /path/to/task-scratch/copy-start.json \
  --payload-copy-snapshot
```

Repeat after transfer into a different output path. The observations bind the
counter values to the process ID and start ticks; trial assembly records the
per-path differences and links both observation digests. Only the v2 assembly,
peer file-segment, and webseed reassembly sites listed in the measurement
contract are counted. This is partial instrumentation, not an estimate of all
copies or a directly comparable total for other clients.
Include both JSON files in the manifest with repeated
`--artifact transfer-payload-copy-observation-v1=PATH` options; the validator
checks the process identity and recomputes the trial deltas from those records.

## Deterministic fixtures

`fixtures.py` creates reproducible payload files and a content manifest. It
requires a new output path, writes through a private staging directory, and
records the generator identity, seed, file sizes, and SHA-256 digests. Example:

```sh
python3 scripts/transfer_rebuild/fixtures.py \
  --output /path/to/task-scratch/transfer-run/fixture \
  --seed trial-seed-01 \
  --profile G3/hybrid-padding \
  --announce-url http://127.0.0.1:6969/announce
```

The manifest digest identifies the ordered fixture definition and each file
digest. `verify_fixture()` checks manifest identity, sizes, and every payload
digest before a trial uses the fixture. `fixture_profiles.json` pins the
`N1/lan_1g`, `N1/lan_10g`, v2 piece-layer, and hybrid-padding workload shapes.
The selected profile and torrent parameters are bound into the fixture digest;
explicit `--file` inputs remain available for focused cases. Profile values
cannot be silently overridden on the command line.

When `--announce-url` is supplied, the generator writes deterministic private
v1, v2, or hybrid metainfo and a `torrent.json` record containing applicable
info hashes and the metainfo digest. V2 roots use [BEP 52](https://www.bittorrent.org/beps/bep_0052.html)
16 KiB SHA-256 leaves, zero-hash padding, and authenticated piece layers.
Hybrid files are sorted into the same v1/v2 path order and use [BEP 47](https://www.bittorrent.org/beps/bep_0047.html)
padding files to align payload files at piece boundaries. The verifier rebuilds
metainfo from the payload. The tracker URL must be a local or otherwise
caller-controlled endpoint without credentials. The verifier still accepts
the earlier v1-only torrent record.

Prepare and measure cache state separately from the transfer:

```sh
python3 scripts/transfer_rebuild/cache_control.py \
  --fixture /path/to/task-scratch/transfer-run/fixture \
  --state cold \
  --output /path/to/task-scratch/transfer-run/cache-state.json
```

On Linux, the helper verifies the fixture, then uses per-file `fsync` and
`POSIX_FADV_DONTNEED` for cold requests or `POSIX_FADV_WILLNEED` plus a full
read for warm requests. It samples residency with `mincore`, never writes
global drop-cache controls, and records `achieved`, `partial`, or `unsupported`.
Only `achieved` is eligible for a trial claiming that cache state. The caller
owns generated files and may clean up only its task directory after retaining
the result manifest and required artifacts. Fixture verification and cache
preparation happen outside the transfer measurement interval.

Regression sources are in `scripts/test_transfer_rebuild_fixtures.py` and
passed on exact main `eb5f3158` in the final 21-script regression pass.
Filesystem cache qualification remains open.

## Live native dormant-catalog observation

`rt-testkit::production_catalog_metainfo()` produces deterministic, private
v1 metainfo with a unique info hash, one tracker, one file, and a one-byte
content-derived piece hash. The `rt-metrics` live catalog case adds each record
through the production engine's raw-metainfo API. That path parses the torrent,
writes its metadata blob, persists its database row, and registers it paused.
The harness shuts down that engine, restarts from the database and blobs, then
measures the native API list route over the restored registry. It records
fixture-generation, engine-admission, restore, first-list, and cached-list
timings, drains each API response, and checks the API's reported catalog total.
First/cached refer to the API snapshot, not filesystem cache state.

The ordinary 1k live case and the opt-in 100k case are regression and
qualification sources, not results. The 100k case is ignored by default due to
its resource cost. Run them during the requested bulk scale pass:

```sh
cargo test -p rt-metrics --test scale live_native_engine_catalog_1k_uses_valid_persisted_metainfo -- --nocapture
cargo test -p rt-metrics --test scale live_native_engine_catalog_100k_uses_valid_persisted_metainfo -- --ignored --nocapture
```

The mixed-load probes reuse the restored catalog and release 16 rounds of
four concurrent native requests: two bounded/sorted list pages, transfer info,
and `/metrics`. Each response is drained. The probe records every route's raw
latencies plus p50/p95 in one `scale_live_native_catalog_mixed_json=` line so
the bulk run can retain and compare the full scrape and API distributions.
The 1k probe is suitable for the hosted correctness lane; the 100k probe stays
opt-in with the other large-catalog work:

```sh
cargo test -p rt-metrics --test scale live_native_engine_catalog_scrape_mixed_load_1k -- --nocapture
cargo test -p rt-metrics --test scale live_native_engine_catalog_scrape_mixed_load_100k -- --ignored --nocapture
```

Retain the full output of both mixed-load commands outside the measured
worktree. After the exact source revision is committed and the worktree is
clean, assemble the report from that log:

```sh
python3 scripts/transfer_rebuild/catalog_load_report.py \
  --repo /path/to/clean/TorrentNG \
  --source-commit <40-character-commit> \
  --log /path/to/task-scratch/run-01/catalog-load.log \
  --output /path/to/task-scratch/run-01/catalog-load-report.json
```

The assembler requires one 1k and one 100k restored-catalog observation,
recomputes every route percentile from its 16 raw latency samples, records host
and source identity, and writes an exclusive mode-`0600` report. Include both
files in the manifest as
`transfer-catalog-load-report-v1=<report.json>` and
`transfer-catalog-load-log-v1=<catalog-load.log>`. Manifest validation checks
that the report cites the retained raw-log digest and the same clean source
revision. For the `transfer-catalog-load-v1` SYS-015 qualification, all three
required checks must cite the report artifact digest; the gate checks its
manifest kind. The qualification wrapper goes in the SYS-015 stage
declaration; its checks link to the report, which links to the retained raw
log.
The regression source is `scripts/test_transfer_rebuild_catalog_load_report.py`
and passed on exact main `eb5f3158`. Live 1k/100k catalog observations remain
separate qualification evidence.

These are diagnostic latencies, not fixed release thresholds. They exercise
the in-process production engine and native router with a paused catalog; they
do not model concurrent payload transfer, production-daemon RSS, or multiple
independent processes. Keep output with the exact source revision and host
details before using it as SYS-014 evidence.

The one-byte paused catalog does not measure payload I/O, hashing throughput,
peer activity, or useful goodput. In-process test measurements are not
production-daemon RSS or descriptor evidence; PERF-102 and PERF-110 own those
separate-process trials. Existing direct-registry and synthetic-database cases
remain useful for lower-level regressions and are explicitly named as proxies.

## PERF-110 paired plan

`paired_plan.py` creates a reproducible plan for seven pinned client adapters
crossed with the four profiles in `fixture_profiles.json`. This is the
efficiency workload catalog, separate from the 28-row protocol correctness
matrix in `docs/INTEROP_MATRIX.md`. The catalog binds a SHA-256 digest of the
profile file and records every client/profile slot. `urtorrent` v2 and hybrid
slots are explicitly unsupported because its pinned runner accepts private v1
metainfo only. An unpinned client is excluded from the schedule with a reason.
The catalog always retains all 28 slots, while optional `--profile-scope`
arguments limit scheduled profile groups for partial-protocol clients.
Optional `--profiles` inputs let a task bind custom workload geometry while
keeping the same four profile IDs; the exact file digest is included in the
catalog and staged into the final manifest. Pass the same profile file to
planning, compatibility preflight, and execution.

Create the plan in a new task-owned directory with a valid
`transfer-client-pins-v1` file:

```sh
python3 scripts/transfer_rebuild/paired_plan.py \
  --pins /path/to/task-scratch/client-pins.json \
  --profiles /path/to/task-scratch/fixture-profiles.json \
  --output /path/to/task-scratch/transfer-run/paired-plan.json \
  --repetitions 10 \
  --seed 17 \
  --profile-scope N1/lan_1g

python3 scripts/transfer_rebuild/validate_artifact.py paired-plan \
  /path/to/task-scratch/transfer-run/paired-plan.json
```

The scheduler uses the versioned `sha256-fisher-yates-v1` algorithm and
records its seed, plan identity, pins digest, catalog digest, exclusions, pair
IDs, and trial order. Each pair fixes the fixture profile and repetition while
randomizing the order of its pinned candidate clients. A pair with fewer than
two clients is excluded. `--profile-scope` is repeatable and accepts catalog
profile IDs; the full catalog and compatibility matrix remain in the plan
even when only selected profiles are scheduled. The plan is written
exclusively with mode `0600`; an existing output is never replaced. Its schema
validator also recomputes the catalog digest and seeded schedule, so edits to
the plan are rejected.

`candidate` means the checked-in catalog does not already know that a slot is
unsupported; it does not prove the pinned client can run that protocol. A
compatibility preflight must qualify every candidate before the runner starts
measurements. Each pinned profile must provide an absolute, client-side
`preflight_save_path` reserved for disposable qualification files. Run the
preflight against disposable client instances. It adds a unique private probe,
verifies its paused/stopped state, removes the exact client hash and local data
where the API supports it, and records the pinned client identity/settings and
exact result. qBittorrent 5.2 returns JSON add summaries with the client-facing
torrent ID; for v2 and hybrid torrents that can be the truncated v2 hash. The
preflight follows that ID, explicitly stops the probe after add, and records
`pause_mode: after_add`. rqbit also has no pause-on-add option in this adapter,
so it is added and immediately paused. Failed add responses are reconciled by
querying the unique fixture hash and attempting cleanup if the server accepted
the mutation. An unreadable add outcome or failed cleanup blocks readiness.

Probes retain each profile's protocol shape, including a v2 piece layer and
hybrid padding, while keeping payloads below 3 MiB. Their only tracker is
`127.0.0.1:9`, and the pinned private-torrent settings keep them isolated.
TorrentNG, Transmission, Deluge and rTorrent are checked in their
paused/stopped-on-add state. qBittorrent is explicitly stopped after add and
read back as paused/stopped because its queue-aware API can report `queuedDL`
while a paused add is being initialized. Deluge also has automatic management
disabled. qBittorrent and rqbit are labeled `after_add`.
urtorrent uses its pinned native runner for v1 probes and remains explicitly
unsupported for v2 and hybrid.

```sh
python3 scripts/transfer_rebuild/compatibility.py \
  --plan /path/to/task-scratch/transfer-run/paired-plan.json \
  --pins /path/to/task-scratch/client-pins.json \
  --profiles /path/to/task-scratch/fixture-profiles.json \
  --output /path/to/task-scratch/transfer-run/compatibility-preflight.json \
  --scratch-parent /path/to/task-scratch/transfer-run

python3 scripts/transfer_rebuild/validate_artifact.py compatibility-preflight \
  /path/to/task-scratch/transfer-run/compatibility-preflight.json
```

The command refuses a changed plan/pin/profile digest and records all 28 slots,
including unpinned, statically unsupported, failed, and cleanup-failed cases.
A scheduled comparison is excluded when fewer than two clients pass; every
failed case remains visible in the artifact. The artifact is exclusive, mode
`0600`, and contains no API credentials. `client_control.py` owns the six
HTTP/RPC clients' exact-hash transfer operations and metainfo identity check;
`urtorrent_runner` owns its direct library path.

### Execute paired download and seed trials

`paired_runner.py` consumes the pinned plan and ready compatibility report. It
stages the source inputs, verifies or generates one fixture per randomized
pair, creates an isolated target/seed namespace and task-local tracker, starts
fresh pinned clients, and releases them through a shared barrier. Each
scheduled candidate produces a download trial and a seed trial. The runner
records exact-hash transfer counters as deltas from the paused baseline,
independent process/resource observations, cache and optional network-control
evidence, payload flush and independent digests, cleanup outcomes, and a
source-bound manifest. Missing byte observations remain null. A trial is not
complete unless the receiver payload verifies and both client identity and
resource evidence were captured. Resource sampling stops at the transfer
boundary, before client teardown and harness-side payload verification.

An API-controlled preloaded seed is started to verify its files while the
receiver stays paused; the shared barrier release starts the measured receive.
This prevents a client that verifies files only after starting from deadlocking
seed setup, and the seed's pre-barrier counters are included in the baseline.

Manifest validation also checks paired evidence meaning and cross-links: it
re-verifies fixture payloads and the original torrent record, validates the
retargeted metainfo record against the original info hashes and retained
binary, binds the private tracker snapshot to the task topology and those info
hashes, and verifies that the observer sampled the candidate's target
namespace for a download or seed namespace for a seed trial. Complete trials
must retain both tracker peers and a ready seed. These checks do not replace
real-client execution or host qualification.

The external scratch directory must already exist, be outside the source tree,
and be reserved for this task. The host needs Linux network namespaces,
`iproute2`, and the privileges required to create the task bridge and shape its
interfaces. Every selected client must have pinned runtime settings, and every
selected profile must have passed compatibility preflight. The direct
`urtorrent` runner can occupy either endpoint; `--helper-client auto` pairs
each candidate with the other of exactly two pinned clients. An explicit
helper ID may equal the candidate, which runs two separate instances of that
client in the task namespaces. This supports crossed trials that hold the
receiver constant while comparing senders. The direct `urtorrent` runner still
cannot occupy both endpoints in one trial.

Every new trial records `counterpart_client_id`. The scorecard includes that
field in its workload signature, so it will not compare uploaders observed
through different receivers or downloaders served by different senders.

When urtorrent participates, the task-local tracker is hosted in the namespace
of its pinned peer, matching the direct runner's tracker-address check. The
trial timeout and seed-stop window are bound into the readback profile, so a
trial cannot silently run with different timeout settings than the pin.

```sh
python3 scripts/transfer_rebuild/paired_runner.py \
  --plan /path/to/task-scratch/transfer-run/paired-plan.json \
  --pins /path/to/task-scratch/client-pins.json \
  --preflight /path/to/task-scratch/transfer-run/compatibility-preflight.json \
  --environment /path/to/task-scratch/benchmark-environment.json \
  --profiles /path/to/task-scratch/fixture-profiles.json \
  --scratch-root /path/to/task-scratch/transfer-run \
  --helper-client auto \
  --cache-state cold \
  --manifest /path/to/task-scratch/transfer-run/manifest.json
```

Use `--trial-id` once per scheduled ID to run a subset; both workload modes
are emitted for each selected ID. `--network-profile` selects one of the
versioned shaping profiles. The manifest output must be directly in the
scratch root so every linked trial and evidence file remains beneath its
manifest. When requesting a manifest, first commit the exact harness source
and use a clean worktree; the runner rejects a dirty source before launching
clients. The source-bound 10-pair TorrentNG/urtorrent v1 result is recorded
under PERF-110. Other pinned client/profile trials and dedicated-host
qualification remain open. Regression sources are in
`scripts/test_transfer_rebuild_paired_plan.py`,
`scripts/test_transfer_rebuild_compatibility.py`, and
`scripts/test_transfer_rebuild_paired_runner.py`; those local regressions
passed in the exact-source suite. The scorecard strips a candidate's client ID prefix
from the catalog case ID for grouping, then verifies that both records in a
pair share the same normalized workload, fixture, and environment.

## PERF-111 picker and changed-snapshot probes

Two standalone benchmark targets emit diagnostic CSV with no pass/fail timing
thresholds:

```sh
cargo bench -p rt-piece-picker --bench perf \
  > /path/to/task-scratch/perf-111-picker.csv
cargo bench -p rt-api-native --bench perf \
  > /path/to/task-scratch/perf-111-snapshots.csv
rustc -Vv > /path/to/task-scratch/perf-111-rustc.txt
uname -a > /path/to/task-scratch/perf-111-host.txt
git rev-parse HEAD > /path/to/task-scratch/perf-111-commit.txt
```

The picker probe times 100k pick/cancel cycles at 1k, 12k, and 100k pieces
after warming partial-piece state up to the 16,384-piece admission cap; its
100k case uses a sparse peer bitfield over that admitted subset. It also times
availability bitfield add/remove with empty, one-in-64 sparse, and near-dense
inputs, moving pieces between nonempty rarity buckets. The API
snapshot probe reports the initial `AppState::torrent_snapshot`, 1,000
unchanged cached reads, and three one-row incremental refreshes at 1k, 10k, and
100k torrents. It uses a single-thread runtime and a process-local counting
allocator; allocation
numbers are requested allocation traffic around the snapshot method, not
retained heap size. A changed-row sample waits outside the timed region for
the API's 750 ms snapshot freshness window to expire. The probe does not
include HTTP routing or JSON serialization. Retain host details and the exact
commit with the CSV; compare results only on a controlled host and toolchain.
The probes were run locally and are recorded in
[`TRANSFER_CORE_PERF111_LOCAL.md`](../../docs/TRANSFER_CORE_PERF111_LOCAL.md).
Their results are diagnostic samples, not adopted thresholds or whole-client
qualification. Both probes ran twice on clean exact main `eb5f3158` on `kspls0`;
the retained evidence is summarized in the active ticket spec.
The current picker probe also compares the previous endgame peer-request
collect/sort/deduplicate path with direct picker-state counting at 1, 4, 16,
and 64 peers. Three process repetitions of 5,000 calls measured medians of
4/4,311/8,065/18,259 ns per call for the old path and below 1/668/664/373 ns
for the new path. The old candidate-vector capacities were
0/3,072/7,680/26,112 bytes; the new path allocated no candidate vector. The
one-peer case uses the production no-other-source fast path. These are
diagnostic picker measurements, not process CPU, RSS, or disk-I/O results.

## PERF-115 private JIT and dedicated storage lanes

`.github/workflows/transfer-rebuild.yml` provides a manually dispatched private
JIT lane for the transfer-core Rust suites and rebuild-harness regression
sources. Its optional storage and performance job runs only from `main`,
requires the protected `transfer-rebuild-hardware` environment, and selects a
run-scoped runner from `ng-shared-main-jit`. Pull-request code cannot enter that
protected environment.

The storage and performance job requires an operator-provisioned disposable
storage directory visible to the private JIT runner and records the exact
commit, worktree state, Rust toolchain, host, and mount identity.
`hardware_lane.sh` retains the picker and changed-snapshot benchmark logs and
runs the real-root storage move/import exercise against the supplied directory.
It preserves all exit statuses so later checks can diagnose each command.
These outputs are diagnostic artifacts; workflow completion alone does not
qualify a storage or performance gate. Previous hosted Transfer Rebuild and
Windows Smoke results passed on exact commit `94c8c1b3`; the protected hardware
job was skipped because no runner was registered for its label.
`hardware_lane.sh` was run twice manually on `kspls0` at `eb5f3158`, so that
hardware evidence remains historical and is not a protected-runner result for
the current source.

## PERF-112 v2 and restart invariants

The `rt-metrics` integration source transfers deterministic v1, pure-v2, and
hybrid fixtures between isolated production engines over loopback TCP. It
checks receiver payload bytes, persists completed state across a clean restart,
then damages one pure-v2 payload byte between runs. Resume-time verification
must mark that torrent incomplete while the unchanged v1 and hybrid payloads
remain complete. This source passed in the exact-source workspace bulk run at
`354ad504`.

Run it with:

```sh
cargo test -p rt-metrics --test transfer_invariants -- --nocapture
```

This exercises metainfo admission, peer-wire transfer, disk hashing,
persistence, and restart recheck. It is a small correctness case, not a
goodput result; it does not qualify crash cuts or prove the durability
properties owned by IO-009/IO-010. The PERF-110 paired runner must separately
validate downloaded bytes and record applicable protocol support before
emitting comparable transfer results.

## Task-owned trial topology and tracker

`task_network.py` creates a task bridge with separate target and seed network
namespaces, one `/24` selected from the reserved `198.18.0.0/15` pool, and no
default route. The target and seed receive only their recorded veth links;
cleanup validates bridge, namespace, interface, and inode identities before
removing those task-owned links. For a container, start it with Docker
`--network none` and pass its PID to `create --target-pid`; attachment refuses
a namespace that already has another interface or a main-table route.

```sh
sudo python3 scripts/transfer_rebuild/task_network.py create \
  --output /path/to/task-scratch/transfer-run/task-network.json

sudo python3 scripts/transfer_rebuild/task_network.py cleanup \
  --topology /path/to/task-scratch/transfer-run/task-network.json
```

The topology record contains a canonical self-digest. Its API-control context
uses that digest, while a trial's `task_topology_sha256` links the SHA-256 of
the topology JSON file recorded in the manifest. Those are different digests
with different purposes. `client_control.py` can receive an ephemeral
`_task_local_api` context so the adapter checks the client's API address,
listener, PID, namespace inode, veth ifindex, address, and connected route
before each mutation.

`task_tracker.py` serves HTTP announces only on a recorded task endpoint. It
accepts only the target and seed source addresses and pinned v1/v2 info hashes.
It learns each endpoint's peer ID and listening port from its announce, then
returns the other peer only to the selected dialer. A TorrentNG endpoint is
selected as dialer when paired with another client, so it receives the peer
from its tracker response and can satisfy private-peer admission; the target
is selected when both endpoints are TorrentNG or neither is. The non-dialing
endpoint receives an empty peer list, preventing reciprocal dials. Later
announces must retain the same identity. Its snapshot
contains the actual bound task address, counts, and hashes of both peer IDs,
not the peer IDs themselves. The retargeted announce URL is checked against
that bound address, which can be the helper endpoint rather than the topology
gateway.
Topology and tracker are control-plane
helpers; they are not part of the measured client's resource totals. Malformed
percent escapes, unescaped non-ASCII bytes, and unescaped control bytes are
rejected before the tracker interprets binary hashes and peer IDs. The paired
runner's staged input directory must have exact mode `0700`.

`metainfo.py` prepares a private per-task copy of a generated torrent. It
checks the canonical fixture-manifest digest, the source torrent digest, the
recorded v1/v2 info hashes, and the torrent's name, piece length, protocol
fields, and original announce URL. It then replaces only the top-level
`announce` value and verifies that the exact encoded `info` dictionary is
unchanged. Paired clients can therefore use one payload fixture and stable
info hash while receiving a task-local tracker address, without rebuilding or
rehashing multi-gigabyte payloads for each run. The output is exclusive and
mode `0600`; alternate trackers, web seeds, and embedded DHT nodes are
rejected.

## Task-scoped network control

`network_control.py` applies a pinned egress profile to a `tngt*` veth inside
an explicitly named `tngt-*` Linux network namespace. Profiles cover clean
paths and seeded packet loss/jitter. It refuses
`lo`, host interfaces, non-veth links, non-default root qdiscs, and mutable
output paths. It records the namespace and interface identity, the iproute2
versions, requested settings, and `tc -j` readback in an immutable
`transfer-network-control-v1` artifact. Artifacts are created with mode `0600`.

Example for an isolated harness namespace:

```sh
python3 scripts/transfer_rebuild/network_control.py apply \
  --namespace tngt-run-a \
  --interface tngt0 \
  --profile wan-20m-30ms-jitter-loss-v1 \
  --seed 123456789 \
  --output /path/to/task-scratch/network-apply.json

python3 scripts/transfer_rebuild/network_control.py reset \
  --applied-report /path/to/task-scratch/network-apply.json \
  --output /path/to/task-scratch/network-reset.json
```

Reset checks the original namespace inode and interface index, then removes
only the recorded `netem` qdisc with handle `7fff:` and verifies the default
`noqueue` qdisc returned. Apply checks the observed delay, jitter, loss rate,
link rate, queue limit, and 64-bit PRNG seed before it emits evidence. If
readback or artifact creation fails, it removes only the qdisc it installed.
Run this helper only inside a task-owned network namespace. The pinned
`tc`/kernel pair must accept and report `netem seed`; unsupported combinations
fail closed.

Profiles shape one egress direction. The paired runner shapes both endpoints,
deriving separate deterministic seeds from the plan pair, profile, and endpoint
role so reruns reproduce a pair without correlating both directions to the
same random stream. Apply one record per shaped endpoint; the trial records
both ordered seed values and apply artifact digests in
`trial.environment.network`. `rtt_ms` remains `null` because the runner does
not measure RTT; configured one-way delay and jitter are recorded separately.
The trial validator checks profile settings against `tc -j` readback and
requires cleanup records for completed shaped trials. This helper controls the
workload; it is not a network benchmark result. Regression sources are in
`scripts/test_transfer_rebuild_network_control.py` and
`scripts/test_transfer_rebuild_paired_runner.py`; these regressions passed on
exact main `eb5f3158` in the final 21-script regression pass.

## Rollout qualification records

Use `qualification_record.py` to turn named checks and retained artifact
digests into a strict qualification record. Add that record to the immutable
manifest as the matching `transfer-…-v1` artifact kind, build the descriptive
scorecard from that manifest, then use `rollout_index.py` to bind stage
declarations to the exact manifest and scorecard digests. `rollout_gate.py`
validates all links and emits one exclusive decision artifact. A missing or
blocked check produces a blocked decision. The gate requires complete v1, v2,
and hybrid candidate trials and the scorecard's configured minimum paired
samples; it does not apply any performance threshold that has not been
approved in D1.

For SYS-014, the `transfer-catalog-load-v1` checks
`production-catalog-persist-restore`, `mixed-native-api-latencies-retained`,
and `host-and-source-identity-recorded` each require the digest of a
`transfer-catalog-load-report-v1` manifest artifact. That report in turn must
cite a retained `transfer-catalog-load-log-v1`; the evaluator rejects an
unlinked log, another source revision, a dirty source manifest, or altered
percentiles.
