# Transfer Rebuild Harness

This directory contains rebuild-specific qualification tools. Outputs belong
in a task-owned scratch directory or certification artifact directory, never
in the measured source checkout.

## Independent process observer

`observe.py` samples a candidate process tree from `/proc` and cgroup v2 from a
separate process. It records raw samples as CSV and emits a peak summary JSON.
The summary keeps RSS, PSS, cgroup anonymous memory, and cgroup current memory
as separate values. It also records cgroup file-backed and `inactive_file`
reclaimable-cache counters separately when the kernel exposes them. Unavailable
counters remain `null`; failed per-process descriptor reads do not produce a
partial FD total. CSV and summary artifacts are created exclusively with mode
`0600` and are never overwritten by a later observer run.

Example:

```sh
python3 scripts/transfer_rebuild/observe.py \
  --pid 12345 \
  --interval-ms 250 \
  --stop-file /path/to/task-scratch/transfer-run/observer.stop \
  --csv /path/to/task-scratch/transfer-run/resources.csv \
  --summary /path/to/task-scratch/transfer-run/resources.json
```

The caller creates the stop file after the measured process exits. The observer
does not start or signal the client. Its cumulative disk counters are Linux
`/proc/<pid>/io` task counters; network counters are intentionally absent until
an independent, attributable source is selected.

The result schema is described in
[`measurement-contract.md`](../../certification/transfer-rebuild/measurement-contract.md).

## Trial and manifest validation

Install the pinned schema validator and validate each record before adding it
to a manifest:

```sh
python3 -m pip install -r scripts/transfer_rebuild/requirements.txt
python3 scripts/transfer_rebuild/validate_artifact.py trial /path/to/trial.json
python3 scripts/transfer_rebuild/validate_artifact.py manifest /path/to/manifest.json
python3 scripts/transfer_rebuild/validate_artifact.py cache-state /path/to/cache-state.json
```

The validator checks the versioned Draft 2020-12 schema, including date-time
formats and the rules that apply when a trial claims a complete outcome. For a
manifest, it also resolves trial and artifact paths beneath the manifest
directory, rejects symlinks and path escapes, checks SHA-256 values, validates
each referenced trial record, and compares its pair/outcome/source identity to
the manifest. It reports field paths and returns nonzero for malformed JSON,
schemas, records, or referenced files. The focused validator regressions are in
`scripts/test_transfer_rebuild_validator.py` and belong in the bulk pass.
Cold/warm trials must link a verified `transfer-cache-state-v1` artifact;
manifest validation checks that the observed fixture and requested state match
the trial and rejects a partial or unsupported state claim.

## Paired scorecard

`scorecard.py` builds a descriptive comparison from a validated manifest. It
groups only identical workload and environment signatures, pairs observations
by `pair_id`, excludes incomplete or invalid transfer outcomes, and reports
per-pair ratios with a deterministic percentile-bootstrap 95% interval for the
median ratio. RSS, PSS, cgroup memory, CPU, I/O, descriptors, threads, and energy
remain separate metrics; unavailable measurements stay absent or `null`.
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
`scripts/test_transfer_rebuild_scorecard.py`; they belong in the requested bulk
pass and have not been run.

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
and `scripts/test_transfer_rebuild_validator.py` and remain in the bulk pass.

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
hybrid fixtures. It binds only to an explicit private, link-local, or loopback
IPv4 interface, disables IPv6, DHT, PEX, and LSD, removes tracker and web-seed
endpoints before release, and dials one explicit private, link-local, or
loopback IPv4 peer. The adapter independently verifies the listener and peer
addresses in the process-bound snapshot. It starts only after the common file
barrier is released. It requires the pinned Rust 1.98.1 toolchain and a kernel
where io_uring is available; this comparator does not qualify v2/hybrid
behavior.

Build to a task-owned target directory so build artifacts stay out of the
checkout:

```sh
CARGO_TARGET_DIR=/path/to/task-scratch/urtorrent-target \
  cargo build --release --locked \
  --manifest-path scripts/transfer_rebuild/urtorrent_runner/Cargo.toml
```

The caller prepares an existing empty download directory or verified seed
directory, creates the `tngt-*` namespace, and starts the runner with
`--namespace`, `--torrent`, `--save-path`, `--pid-file`, `--settings-file`,
`--start-gate`, `--mode download|seed`, required `--listen-v4 IP`, and required
`--peer IPv4:PORT`. Both addresses must be private, link-local, or loopback.
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
runner's `src/main.rs`; runner, adapter, and validator cases are reserved for
the requested bulk pass.

## Shared trial measurement primitives

`measurement.py` provides a task-directory file barrier, a per-file interval
union that separates unique accepted bytes from duplicate overlap, and a
monotonic timeline for barrier, transfer, flush, validation, and shutdown.
Callers must create and clean a fresh barrier directory for each trial. These
helpers do not launch clients, establish integrity, or calculate a scorecard.
Their regression sources are in `scripts/test_transfer_rebuild_measurement.py`
and belong in the bulk pass.

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
belong in the requested bulk pass; none have been run for this slice.

## Task-scoped network control

`network_control.py` applies a pinned loss-free egress profile to a `tngt*`
veth inside an explicitly named `tngt-*` Linux network namespace. It refuses
`lo`, host interfaces, non-veth links, non-default root qdiscs, and mutable
output paths. It records the namespace and interface identity, the iproute2
versions, requested settings, and `tc -j` readback in an immutable
`transfer-network-control-v1` artifact. Artifacts are created with mode `0600`.

Example for an isolated harness namespace:

```sh
python3 scripts/transfer_rebuild/network_control.py apply \
  --namespace tngt-run-a \
  --interface tngt0 \
  --profile metro-100m-10ms-v1 \
  --output /path/to/task-scratch/network-apply.json

python3 scripts/transfer_rebuild/network_control.py reset \
  --applied-report /path/to/task-scratch/network-apply.json \
  --output /path/to/task-scratch/network-reset.json
```

Reset checks the original namespace inode and interface index, then removes
only the recorded `netem` qdisc with handle `7fff:` and verifies the default
`noqueue` qdisc returned. Apply also checks the observed delay, rate, and queue
limit before it emits evidence; if evidence creation fails, it removes only
that exact qdisc. Run this helper only inside a task-owned network namespace.

Profiles shape one egress direction and apply fixed delay, rate, and queue
limit. They do not yet inject random loss or jitter, create namespaces, apply
to both directions automatically, or establish a measured RTT. Apply one
record per shaped endpoint; record the profile id and all apply artifact
digests in `trial.environment.network`. The trial validator requires those
artifacts to match the selected profile and link rate. This helper is a
reproducibility control, not a network benchmark result. Regression sources
are in `scripts/test_transfer_rebuild_network_control.py` and belong in the
requested bulk pass; none have been run.
