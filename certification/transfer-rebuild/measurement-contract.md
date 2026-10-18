# Transfer Rebuild Measurement Contract

Version: `transfer-trial-v1` / `transfer-manifest-v1`  
Status: measurement contract only; proposed scorecard values remain unadopted.

This contract defines what a transfer-rebuild result must say. It does not
qualify a client, establish a performance target, or turn an incomplete run into
a measurement. Machine-readable trial and manifest records use the adjacent
JSON Schemas.

## Identity and provenance

Every trial records the full harness source commit, dirty-worktree state,
lockfile digest, executable digest and size, fixture digest, environment
identity, client settings readback, and the exact trial case. Client identity
is stored in a separate `transfer-client-observation-v1` artifact and linked
from each trial by its file digest, settings digest, version, runtime kind, and
process start identity. A container image is identified by immutable digest;
a native process is tied to the pinned binary inode, digest, and PID start
time, plus source commit, lockfile, and toolchain when a source checkout is
available. Mutable tags and filenames are descriptive only.
A result is not comparable when source, binary, fixture, environment, client
process, or settings identity is missing or differs from its declared pair.
New complete trials link a `transfer-resource-observation-v1` summary through
`resources.raw_artifact_sha256`. The summary binds the root PID and process
start time to `client_identity`, carries the digest of its streamed CSV, and
is checked against the trial's resource totals. The manifest validator also
recomputes peak and network totals from the CSV rows. Historical trial-v1
records without this link remain readable, but the scorecard excludes them.

The trial's top-level `identity` describes the measurement harness invocation,
while `client_identity` describes the measured torrent client. For this Python
harness, the executable fields identify the Python interpreter; the dependency
lock digest covers the workspace `Cargo.lock` and pinned
`scripts/transfer_rebuild/requirements.txt`. The client executable or immutable
container image remains bound separately by the linked client observation.

The canonical `manifest.py` writer accepts only a clean committed worktree and
records `worktree_dirty: false`; commit the measured source before publishing
the manifest. The schema keeps the dirty-state field explicit for externally
produced records, but the field alone does not capture the contents of a dirty
tree and therefore cannot make that tree reproducible.

When `environment.cache_state` is `cold` or `warm`, the trial also records
`cache_state_observation_sha256`, linking a `transfer-cache-state-v1` artifact
from the manifest. The validator checks its digest, fixture identity, requested
state, file-level residency totals, and achieved status. `partial` and
`unsupported` observations cannot back a cold/warm trial claim. A `controlled`
state is for an independently documented method and does not claim measured
page-cache residency.

When network shaping is claimed, `environment.network.control_profile_id`
names the pinned profile and `control_observation_sha256s` links every apply
artifact from the manifest. The validator checks each artifact digest, its
namespace and interface identity, its requested settings, and its `tc -j`
qdisc readback against that profile. Current profiles shape one egress path
with fixed delay, rate, and queue limit; they do not establish RTT or inject
random loss/jitter. Trials must report a measured RTT separately or leave it
`null`. Reset artifacts remain in the manifest as cleanup evidence and point
back to the apply artifact they removed.

Client profiles require an exact version and expected settings. Readback is
client-specific: declared API/RPC settings for qBittorrent, Transmission, Deluge,
TorrentNG, and rTorrent; rqbit binds its running command line, explicitly
allowlisted environment, and explicitly listed configuration-file digests
because its HTTP API exposes status rather than an effective settings
endpoint. The rqbit profile must pin the complete observed launch record; it
does not infer unobservable defaults. Secret fields and credential-bearing
URLs are redacted before the observation digest is calculated.
HTTP adapters accept loopback API URLs only. Native and host-network API
listeners must bind only to loopback, while bridge-network container API ports
must be published only on loopback addresses in the pinned container's network
namespace. The native executable digest and PID file are opened without
following symlinks; binary metadata is checked before and after hashing, then
matched to the live process executable inode.

The direct `urtorrent` comparator uses a task-owned standalone runner pinned to
the exact upstream library version and runner binary. It emits a mode-0600
snapshot of effective session and torrent settings bound to its live PID and
process start time; the adapter verifies that identity before accepting the
readback. Hash-pool threads are recorded as a command-line input because the
public settings API does not expose that value. The runner requires a private
fixture, binds to one explicit task-local IPv4 interface, removes its trackers
and web seeds, and dials one explicit private, link-local, or loopback IPv4
peer. The adapter independently checks both addresses in the process-bound
snapshot. The runner refuses to start outside the named `tngt-*` task network
namespace, disables IPv6/DHT/PEX/LSD, and waits for the common start barrier. It
rejects v2 and hybrid metainfo. Its `barrier_to_trial_end_ms` result includes
resume/connect setup and completion polling, plus the configured seed
observation window for seed trials, so it is diagnostic and does not replace
`transfer_ns`. The control loop is part of the measured process. The comparison
is Linux/v1 only; it does not qualify v2 or hybrid behavior or establish a
production-engine replacement decision.

Artifacts are referenced from a manifest by relative path and SHA-256. The
manifest itself is immutable after the trial. Keep observer samples, client
logs, transfer validation output, and failure diagnostics with the manifest;
do not place trial outputs in the measured checkout.
The manifest writer requires referenced files to be regular, non-symlink files
below the manifest directory and outside the measured checkout. It preserves
all trial outcomes, validates every linked digest and identity before
publication, and refuses to replace an existing manifest.

## Clocks and useful bytes

Use monotonic time for durations. Record these intervals separately:

1. `barrier_wait_ns`: waiting until the matched start barrier is released.
2. `transfer_ns`: first useful payload byte through the last useful payload
   byte; excludes preallocation, final flush, validation, and shutdown.
3. `flush_ns`: the declared durability boundary after payload transfer.
4. `validation_ns`: payload digest and expected-length verification.
5. `shutdown_ns`: orderly client shutdown and cleanup.

`useful_payload_bytes` counts unique payload bytes accepted for the fixture.
It excludes protocol overhead, duplicate blocks, padding files, and bytes that
were not accepted by the verifier. Report received, written, read, and duplicate
bytes separately. Goodput is useful payload bytes divided by `transfer_ns`;
never infer it from process lifetime or bytes offered by a peer.

### Payload-copy accounting

When TorrentNG exposes its instrumented copy counters, capture a
`transfer-payload-copy-observation-v1` artifact immediately before and after
the transfer interval. Both observations must identify the same live process
by PID and process start ticks. `payload.copy_accounting` links the two
artifact digests and records each nonnegative counter difference; a reset,
missing counter, process change, or mismatched delta invalidates that
accounting record.

The current TorrentNG counters cover only three explicit payload-copy sites:

- `peer_v2_assembly`: accepted peer blocks copied into a pure-v2 piece
  assembly;
- `peer_file_segment`: peer payload copied into a file-segment buffer when
  the shared payload lease is unavailable;
- `webseed_reassembly`: HTTP range bodies copied into a logical webseed block.

They are cumulative process counters exported by TorrentNG's `/metrics`
endpoint and sampled by `adapters.py --payload-copy-snapshot`. They exclude
all uninstrumented copies and are not a whole-client or whole-process copy
total. Leave `copy_accounting` absent when a client has no compatible
instrumentation; never represent missing coverage as zero or compare these
partial TorrentNG counts directly with a different client's total-copy
counter. The counters add an atomic increment at each measured copy site, so
that instrumentation overhead is part of the measured TorrentNG build and
must be stated with results.

## Observer and accounting domains

The observer runs outside the candidate process and samples cumulative resource
counters on a monotonic schedule. It records the sample interval and source for
each counter. Keep these memory values separate:

- process RSS and PSS;
- cgroup anonymous memory;
- cgroup file-backed memory;
- cgroup reclaimable cache (`inactive_file`) when the kernel reports it;
- cgroup current memory;
- file-backed memory and reclaimable cache when available.

Never substitute one accounting domain for another. Record CPU time, file
descriptors, thread count, block-device reads and writes, and network bytes.
Uninstrumented energy is `null`, never zero. Missing platform counters are
`null` with an explanation in the observer artifact. The initial process
observer reads optional network bytes from a named interface in the root
process's network namespace through `/proc/<pid>/net/dev`. It records the
namespace inode and uses the counter difference between the first and last
valid samples. A missing interface, namespace change, counter reset, or fewer
than two valid samples leaves the totals `null`. These counters cover all
traffic on that interface, not an individual PID, so they count as client
traffic only when the runner isolates one client in the private namespace and
links the matching network-control record to the trial. In a shared namespace,
leave them `null` for client comparisons. The observer summary repeats this
attribution warning. If any sample lacks a process counter, that summary peak
is `null` rather than a maximum from only the remaining samples. A linked
observation must contain at least one row, each row must report a live process
tree, and elapsed times must increase strictly within the summary's monotonic
start/finish interval.

## Independent service ceilings

`transfer-ceiling-input-v1` records separate network receive/transmit capacity,
storage read/write capacity, protocol hash throughput, and the available peer
request window divided by measured RTT. Each measurement identifies its tool,
version, executable digest, PID/start-time identity, and raw evidence digest.
The manifest rejects a probe whose process identity matches the measured
candidate and must include the raw evidence as `transfer-calibration-raw-v1`
artifacts.

`transfer-ceiling-v1` derives a diagnostic upper bound for download and upload.
V1 uses the SHA-1 calibration, v2 uses SHA-256, and hybrid requires a separate
combined hash calibration. Download is bounded by network receive, disk write,
hash, and pipeline rates; upload is bounded by network transmit, disk read,
and pipeline rates. The pipeline record is client/version/settings specific;
the validator reads its JSON pointer in the process-bound settings observation
and checks request depth, peer count, and block size against the trial. A trial
may link a ceiling report only when its host,
kernel, CPU, filesystem, storage device, network profile, and declared protocol
match the calibration. This is a model for explaining a trial, not an observed
client rate or a performance claim. Do not use it as scorecard evidence.

## Outcomes and comparisons

Each trial is `complete`, `incomplete`, `invalid`, `failed`, or `unmeasured`.
Only `complete` trials with passing applicable integrity checks can enter a
scorecard. A timeout is `incomplete`; startup failure is `unmeasured`; invalid
identity, fixture, or settings readback is `invalid`; an observed runtime error
is `failed`. An `unmeasured` startup failure has no client process identity,
settings readback, or process resource observation; preserve the launch failure
reason and any separate launcher log. Preserve the evidence and reason for all
outcomes.

Paired trials use the same fixture, resource envelope, cache state, durability
boundary, and matched start barrier. Re-prepare cache state before each paired
attempt; a preceding client can change residency even when both clients use
the same fixture. Cache preparation and fixture verification are outside the
transfer interval. Randomize client order and retain every
attempt, including failures. Report the valid sample count, invalid and
incomplete counts, uncertainty method, and per-pair results. Do not compare a
single successful run with an unmatched historical result.

No proposed scorecard number in `docs/TRANSFER_CORE_REBUILD.md` is adopted by
this schema. Threshold adoption and release gates require a separate owner
decision before results are observed.

The descriptive `transfer-scorecard-v1` report groups only trials with matching
workload and environment identities, then pairs clients by `pair_id`. It emits
each pair's metric values and client-B/client-A ratio, plus a deterministic
paired percentile-bootstrap 95% interval for the median ratio. Its minimum
sample indicator is informational; the report does not apply performance
thresholds or declare a winner. Incomplete, failed, unmeasured, invalid, and
integrity-failing download or seed trials remain listed as excluded pairs with
their reasons. Resource accounting domains remain separate and unavailable
counters remain null. The scorecard schema is validated independently from the
trial and manifest schemas.

The shared harness primitives in `scripts/transfer_rebuild/measurement.py`
implement a file-backed matched-start gate, per-file unique-range accounting,
and distinct monotonic phase durations. The barrier directory is trial-owned
scratch and must be fresh for every trial. Readiness and release markers are
created exclusively as regular files; symlink and non-file markers are
rejected rather than followed. The unique-range ledger is an accounting aid;
only ranges accepted by the transfer verifier may be submitted to it. Client
launch, settings readback, fixture integrity, artifact digest verification,
and paired-trial analysis remain separate responsibilities.
