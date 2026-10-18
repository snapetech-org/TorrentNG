# Active Transfer Rebuild Ticket Specs

This file instantiates the template in
[`TRANSFER_CORE_TICKET_TEMPLATE.md`](TRANSFER_CORE_TICKET_TEMPLATE.md) for the
implementation slices currently in progress. A ticket stays `wip` until its
written acceptance and exact-revision evidence are complete.

## PERF-105 — Scoped deterministic fixture generation

- **Priority / wave:** P0 / 3
- **Prerequisites:** PERF-101
- **Owning component:** certification tooling.
- **Finding or capability:** repeatable fixture identity without writing
  generated payloads into the measured source checkout.
- **Status:** `wip`

### Scope

Generate deterministic per-file payloads beneath a fresh caller-selected
directory and record a manifest with size, SHA-256, generator, seed, and a
fixture identity digest. With a caller-supplied tracker URL, generate private
v1, v2, or hybrid metainfo and a separate identity record. Pinned workload
profiles define payload shape and torrent geometry. A Linux helper requests
cold or warm page-cache state for the fixture files and records measured
residency; it does not modify global cache settings. Callers own fixture
retention and may remove only their task-scoped output after preserving the
required result artifacts.

### Ownership and invariant

- **Current owner:** the trial runner chooses and owns the output directory;
  the generator writes only into a private staging directory beside it.
- **Target owner:** PERF-105 supplies fixture identities to paired trial
  execution and adapters.
- **Invariant preserved:** fixture identity is content-derived and each
  payload digest is independently recorded.
- **Must not:** merge into or clear an existing output directory, or call a
  fixture cold/warm based only on its generation state.

### Acceptance

- Same seed, file labels, sizes, and generator version reproduce the same
  payload bytes and manifest identity; verifier checks the manifest and every
  file digest before use.
- Names are flat filename-safe labels and sizes are nonnegative.
- Output must be new; generation is staged and task-scoped.
- Optional private v1/v2/hybrid metainfo derives every hash from the payload
  and records all applicable info hashes and the metainfo digest; verification
  rebuilds those values.
- V2 fixtures use BEP 52 16 KiB SHA-256 leaves, zero-hash padding, a mandatory
  piece-layers dictionary, and authenticated piece layers for files larger
  than the piece length. Hybrid fixtures preserve the same sorted file order
  in both views and use BEP 47 zero-filled padding files for piece alignment.
- Named profiles pin the file list, protocol kind, torrent name, and piece
  length into the fixture identity; profile options cannot be overridden.
- Cache preparation is scoped to fixture payload files and reports both the
  requested state and mincore-observed residency. A partial/unsupported state
  cannot be reported as an achieved cold/warm trial.
- Regression sources cover deterministic output, digest identity, and refusal
  to reuse an existing path, deterministic and tamper-checked v1/v2/hybrid
  metainfo, profile identity, and cache-state record shape.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/fixtures.py`,
  `fixture_profiles.json`, `cache_control.py`, the cache-state schema,
  artifact validator, trial schema, regression sources, and harness README.
- **State or wire compatibility:** certification fixture format only; payload
  identity remains `transfer-fixture-v1`, and torrent records use a versioned
  v2 envelope while the verifier retains support for v1-only records.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** a fixture is comparable only when its
  profile, generated metainfo, client settings, and cache-state observation
  are linked to the trial artifact.

### Evidence

- **Regression source added or updated:** `scripts/test_transfer_rebuild_fixtures.py`.
- **Validator regression source:** `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** `python3 scripts/test_transfer_rebuild_fixtures.py`
  during the requested bulk pass.
- **Expected result:** deterministic file bytes, correct v1/v2/hybrid
  metainfo, matching profile identity, and accurate cache-state outcome; no
  existing path or global cache setting is modified.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** review of the pinned workload sizes,
  and cache-control qualification on each dedicated benchmark filesystem.

## PERF-104 — Common trial timing and useful-byte accounting

- **Priority / wave:** P0 / 4
- **Prerequisites:** PERF-101, PERF-102, PERF-103
- **Owning component:** certification tooling.
- **Finding or capability:** matched paired starts and an explicit useful
  payload denominator for goodput.
- **Status:** `wip`

### Scope

Provide shared measurement primitives for a matched start barrier, unique
accepted payload range accounting, and separate monotonic barrier, transfer,
flush, validation, and shutdown intervals. Fixture/client adapters, paired
trial execution, and artifact assembly remain open.

### Ownership and invariant

- **Current owner:** trial runner (barrier directory lifecycle), verifier
  (which ranges are accepted), and timeline caller (phase boundaries).
- **Target owner:** PERF-104 harness consumes pinned PERF-103 adapters and
  PERF-105 fixtures without controlling the client's resource observer.
- **Invariant preserved:** goodput uses unique verifier-accepted payload bytes
  divided by transfer time; padding, duplicate overlaps, flush, validation, and
  shutdown are accounted separately.
- **Must not:** infer a start time from process launch or count bytes merely
  offered by peers.

### Acceptance

- A fresh task-owned file barrier requires every declared participant and has
  a bounded timeout.
- Range accounting is per file, merges overlapping ranges, excludes padding,
  and reports new and duplicate bytes.
- Timeline markers use a monotonic clock, require all contract phases, and
  reject zero transfer duration and overlapping/out-of-order phases.
- Regressions cover participant release, unique/duplicate ranges, padding, and
  phase boundaries.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/measurement.py`, its
  regression source, and the measurement contract.
- **State or wire compatibility:** certification artifacts only; the existing
  `transfer-trial-v1` schema now requires an observation-linked client
  identity. No trial evidence has been qualified against the earlier draft.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** no scorecard or release comparison
  until the runner and adapters are qualified.

### Evidence

- **Regression source added or updated:** `scripts/test_transfer_rebuild_measurement.py`.
- **Command or probe to run:** `python3 scripts/test_transfer_rebuild_measurement.py`
  during the requested bulk pass.
- **Expected result:** a matched release, exact interval-union accounting, and
  separate ordered phase durations.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** cross-process barrier characterization
  and paired-run reproducibility.

## NET-109 — Active-candidate piece picker

- **Priority / wave:** P1 / 6
- **Prerequisites:** NET-104
- **Owning component:** `rt-piece-picker`
- **Finding or capability:** F14/F15; bounded piece selection and honest index
  accounting.
- **Status:** `wip`

### Scope

Replace per-piece tree nodes and full-torrent fresh-work scans with
array-backed rarity buckets and an incrementally maintained candidate index.
The live v1/v2 actor integration to the shared ledger remains NET-114.

### Ownership and invariant

- **Current owner:** `PiecePicker` owns requestability and partial block state.
- **Target owner:** `PiecePicker` continues to own selection policy; NET-114
  will move cross-peer block ownership to the shared ledger.
- **Invariant preserved:** cancellation, completion, rejection, restore, and
  enable/disable transitions update both the outstanding count and candidate
  membership consistently. Endgame sees only bounded in-progress pieces.
- **Must not:** report a piece as verified by selection alone, or include
  received bytes in a “have” result before verification.

### Acceptance

- Rarity and active-work indexes use dense arrays with swap-remove positions.
- Selection chooses uniformly among matching peers in the rarest matching
  bucket; sampling is bounded for the common case and falls back to uniform
  reservoir sampling when matches are sparse.
- Sources with out-of-band full availability, including webseeds, randomize
  zero-availability ties too.
- Priority, sequential, cancellation, partial restore, and endgame behavior
  remain covered by regression cases.
- Static and live estimates include packed flags, index arrays, bucket
  capacities, and the configured peer ceiling.

### Files and compatibility

- **Expected code paths:** `crates/rt-piece-picker/src/{availability,flags,picker}.rs`,
  engine reservation accounting, picker README.
- **State or wire compatibility:** no resume or wire-format change.
- **Migration / rollback:** none; the reference API remains the current picker.
- **Feature flag or rollout boundary:** none; qualification is required before
  the new picker is relied on for release claims.

### Evidence

- **Regression source added or updated:** availability bucket movement,
  seeded random tie, candidate lifecycle, request cancellation, and picker
  memory estimate cases.
- **Command or probe to run:** `cargo test -p rt-piece-picker` and the engine
  picker/accounting regressions during the requested bulk pass.
- **Expected result:** all cases pass; no requestable piece is lost from the
  candidate index and no inactive piece remains indexed.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** PERF-111 picker/snapshot allocation
  probes and whole-engine qualification.

## IO-001 — Storage evidence contract and deterministic fault harness

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** `rt-storage`
- **Finding or capability:** explicit separation of write completion,
  servability, payload durability, and resume commit; cancellation-safe
  completion ownership.
- **Status:** `wip`

### Scope

Document storage evidence and introduce deterministic backend faults for the
portable scheduler. This slice covers cancellation after backend submission
and partial-write failure. Operation leases, file replacement, restart cuts,
and full generation fences remain later IO tickets.

### Ownership and invariant

- **Current owner:** `MountScheduler` admits and sequences positioned I/O;
  backend workers own submitted operations until completion.
- **Target owner:** IO-006 device service, with per-operation identity and
  retained completion ownership.
- **Invariant preserved:** cancellation of a waiter cannot clear dirty evidence
  for a submitted write; failed partial writes cannot be reported as complete.
- **Must not:** equate successful write completion with durability or resume
  commitment.

### Acceptance

- Contract defines received, verified, write-complete, servable,
  payload-durable, and resume-committed separately.
- Test-only backend can inject read/write/sync failures, hold a completion, and
  write a prefix before returning an error.
- Cancellation while a submitted write is held leaves dirty state until a
  covering barrier completes.
- Partial-write failure is returned to the caller and remains dirty.
- Failed data sync leaves the dirty generation intact for a successful retry.
- The scripted scheduler regression covers data-sync failure and retry; the
  broader sync-failure matrix remains open.
- File replacement, short-I/O, end-to-end operation-lease accounting, and
  crash-cut cases remain explicitly open until implemented.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/backend.rs`,
  `scheduler.rs`, test-only `test_support.rs`, and
  `certification/transfer-rebuild/storage-evidence-contract.md`.
- **State or wire compatibility:** no public storage or resume format change.
- **Migration / rollback:** none in this test-only slice.
- **Feature flag or rollout boundary:** injected backend is `cfg(test)` only.

### Evidence

- **Regression source added or updated:**
  `cancelled_waiter_keeps_submitted_write_dirty_until_barrier` and
  `partial_write_then_error_is_not_reported_as_success` and
  `failed_barrier_keeps_dirty_evidence_for_retry`.
- **Command or probe to run:** `cargo test -p rt-storage` during the requested
  bulk pass.
- **Expected result:** cancellation retains dirty ownership through barrier;
  partial failure remains an error, and a failed barrier preserves dirty state
  for retry.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** real-device and filesystem-specific
  crash/replacement qualification.

## IO-002 — Typed v2 block and piece extents

- **Priority / wave:** P0 / 3
- **Prerequisites:** IO-001, SYS-002, NET-101
- **Owning component:** `rt-engine` v2 actor and transfer geometry adapters.
- **Finding or capability:** F01; wire-block coordinates must not size a
  whole-piece assembly.
- **Status:** `wip`

### Scope

Use typed geometry for v1 and v2 receive/serve range validation, and for
whole-piece v2 assembly sizing. The existing piece maps remain the storage-path
adapters; shared request ownership and actor migration remain NET-104/NET-114.

### Ownership and invariant

- **Current owner:** `torrent_task.rs` and `torrent_task_v2.rs` own peer
  requests; the v2 task owns per-peer assemblies.
- **Target owner:** NET-114 shared ledger owns request/content state; geometry
  remains a pure adapter.
- **Invariant preserved:** block length bounds one wire response; piece length
  bounds full verification, read-back, and publication.
- **Must not:** use a block extent as a hash or persistence extent.

### Acceptance

- Outgoing/received block coordinates validate as aligned, nonempty, bounded
  blocks within the mapped piece.
- Peer upload requests validate as bounded served blocks; invalid ranges are
  rejected through the existing protocol behavior.
- Assembly capacity and whole-piece length use the typed piece extent.
- Multi-block memory and spill-path regressions cover full-piece completion,
  short final pieces, and request pipelining.
- v1 and hybrid cross-plane ownership are not claimed by this ticket.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/coords.rs`,
  `torrent_task.rs`, `torrent_task_v2.rs`, and the existing v2 tests.
- **State or wire compatibility:** no format or wire change.
- **Migration / rollback:** none; old storage map remains the reference.
- **Feature flag or rollout boundary:** current v2 actor only; shared-core
  rollout remains NET-114.

### Evidence

- **Regression source added or updated:** existing `v2_*` multi-block memory
  and spill cases plus the within-piece request-window case.
- **Command or probe to run:** targeted v2 engine regressions during the
  requested bulk pass.
- **Expected result:** request coordinates agree with their geometry plane;
  all requested v2 blocks fill the piece and whole-piece assembly length
  matches the geometry adapter.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** none for this code slice; full shared
  actor parity remains a later gate.

## IO-003 — Lease-owning immutable receive events

- **Priority / wave:** P0 / 3
- **Prerequisites:** IO-001
- **Owning component:** rt-engine v1 peer and webseed receive paths.
- **Finding or capability:** F16; a queued payload and its memory reservation
  must share one lifetime across asynchronous boundaries.
- **Status:** wip

### Scope

Carry the existing receive allocation lease inside a private event wrapper
through peer mailboxes, actor processing, and storage waits. Keep the public
BlockEvent shape unchanged. Eviction spill must retain ownership of its
original allocation without creating a second piece-sized copy.

### Ownership and invariant

- **Current owner:** the v1 actor admits peer-response bytes before requests;
  the peer loop carries the lease into the received event; webseed fetch
  reserves its response body.
- **Target owner:** the private LeasedBlockEvent carries both content and
  reservation until the final async consumer releases it.
- **Invariant preserved:** dropping a stale, rejected, cancelled, or completed
  event releases its reservation only after its bytes stop being retained.
- **Must not:** release a webseed body reservation when fetch returns, or lose
  the piece-assembly charge while an eviction slice still references its
  backing allocation.

### Acceptance

- Peer and webseed events retain their lease through actor handling.
- A webseed fetch regression proves the body charge remains live after the
  fetch helper returns and releases when the event drops.
- Evicted spill ranges share the original immutable allocation and lease.
- Partial flush copies reserve their transient memory before allocation.
- Shared v1/v2/hybrid ledger spans and device-operation ownership remain later
  IO-003/IO-005 work; this slice does not claim them complete.

### Files and compatibility

- **Expected code paths:** crates/rt-engine/src/torrent_task.rs,
  torrent_task/peer_session.rs, torrent_task/peer_transfer.rs.
- **State or wire compatibility:** no public BlockEvent, resume, or wire
  format change.
- **Migration / rollback:** none; the wrapper is internal to the actor.
- **Feature flag or rollout boundary:** v1 peer and webseed receive path only.

### Evidence

- **Regression source added or updated:** webseed event lease lifetime,
  piece-assembly copied-range lease accounting, and
  evicted_block_views_share_the_assembly_memory_lease, plus the NET-103
  request-to-event ownership regression.
- **Command or probe to run:** targeted rt-engine lease and spill regressions
  during the requested bulk pass.
- **Expected result:** charges remain live through the final event/slice owner
  and return to zero after it drops.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** cross-plane payload ownership and
  storage-operation completion retention.

## IO-004 — Shared secure file identity and descriptor catalog

- **Priority / wave:** P0 / 3
- **Prerequisites:** IO-001
- **Owning component:** `rt-storage` secure open path, scheduler cache, and
  public `HandleCache`.
- **Finding or capability:** F17; cached descriptors must remain tied to the
  filesystem object the authorized path currently names, and metadata I/O
  must not serialize unrelated cache operations.
- **Status:** `wip`

### Scope

Centralize platform file identity and replacement detection in `open.rs` and
use it from scheduler and handle-cache paths. The scheduler's two cache
implementations and a single secure file catalog remain to be unified.

### Ownership and invariant

- **Current owner:** scheduler `FilePool` and `HandleCache` each retain their
  own bounded descriptor maps and leases; `open.rs` owns no-follow opens.
- **Target owner:** one catalog owns secure normalized keys, identity checks,
  descriptor leases, and cache lifecycle across scheduler and direct cache
  users.
- **Invariant preserved:** unlink/recreate or symlink replacement invalidates
  a cached descriptor before it serves a later path-based operation.
- **Must not:** perform a filesystem identity check while holding the cache
  mutex or reuse a descriptor after its path has been replaced.

### Acceptance

- Scheduler and public cache share the same OS-specific identity helpers.
- Path identity checks happen outside the cache mutex, followed by a pointer
  check before refreshing an entry.
- A newly opened descriptor is checked against the current path before it is
  published; repeated replacement races fail after a bounded retry count.
- Reads can reuse a read+write descriptor, while read-only descriptors never
  serve writes.
- Replacement, unlink/recreate, descriptor-budget, and in-flight-lease
  regressions continue to cover the catalog lifecycle.
- A single catalog and duplicate cache removal remain open work.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/open.rs`,
  `scheduler.rs`, `handle_cache.rs`, and `file_handle.rs`.
- **State or wire compatibility:** no storage-path, resume, or wire-format
  change.
- **Migration / rollback:** none; path opening remains no-follow.
- **Feature flag or rollout boundary:** none; qualify before broader storage
  service integration.

### Evidence

- **Regression source added or updated:** existing cache replacement and
  scheduler replacement cases; no new test was added in this slice.
- **Command or probe to run:** `cargo test -p rt-storage` during the requested
  bulk pass.
- **Expected result:** replaced files are reopened, identity checks do not
  hold the cache lock during filesystem access, and live leases continue to
  bound descriptors.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** NFS and real-device replacement
  behavior under concurrent move and transfer load.

## IO-005 — Cancellation-safe storage admission ownership

- **Priority / wave:** P0 / 4
- **Prerequisites:** IO-003, IO-004
- **Owning component:** `rt-storage::MountScheduler` and backend completion
  watchers.
- **Finding or capability:** admitted queue slots and byte budgets must remain
  owned until blocking preparation or submitted backend work stops retaining
  the operation.
- **Status:** `wip`

### Scope

Carry `DiskSubmission` through queued file preparation and backend completion
for reads, writes, and sync barriers. The completion watcher owns queue/device
permits and queued-byte charges if the caller cancels while backend work is
still running. Strict writes retain the same admission through their own sync.

The elevator worker already owns its independent request, queue, and device
leases through its backend read. Operation tracking, resource-credit transfer,
and the full injected-fault matrix remain open.

### Ownership and invariant

- **Current owner:** the scheduler admits queue and device slots plus queued
  bytes in `DiskSubmission`; completion helpers now can retain that owner.
- **Target owner:** IO-006's device service owns one bounded operation record
  from admission through exactly one backend completion result.
- **Invariant preserved:** dropping a waiter cannot make a queued slot or
  byte reservation available while its blocking or backend operation still
  retains work.
- **Must not:** leak admission on a pre-submission error or release it before
  an accepted backend operation completes.

### Acceptance

- File-open preparation closures retain admission while running after caller
  cancellation and release it promptly on error.
- Backend read, write, and sync completion watchers retain operation
  admission until their backend receiver resolves, even if their return
  channel is closed.
- Readahead's additional queued-byte charge follows its larger read through
  backend completion.
- Strict writes hold admission through their fdatasync; checkpoint writes
  release it only after the write completion.
- End-to-end queue-capacity and memory-budget cancellation cases remain to be
  added alongside the fault-injected device-service matrix.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`,
  `backend.rs`, `test_support.rs`, and IO-006's device service.
- **State or wire compatibility:** no resume, storage-path, or wire-format
  change.
- **Migration / rollback:** none; the portable backend remains selectable.
- **Feature flag or rollout boundary:** no behavior flag; storage service
  rollout remains later work.

### Evidence

- **Regression source added or updated:** completion-owner lifetime cases for
  canceled read/write waiters; they exercise helper ownership, not yet the
  whole scheduler semaphore and resource-governor accounting path.
- **Command or probe to run:** `cargo test -p rt-storage` and the scripted
  backend cancellation matrix during the requested bulk pass.
- **Expected result:** accepted work holds its queue/device/byte owner until
  backend completion, with no leak after pre-submission failure.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** cancellation and queue behavior under
  real-device and NFS latency.

## IO-006 — Shared device service over the portable backend

- **Priority / wave:** P0 / 5
- **Prerequisites:** IO-005
- **Owning component:** `rt-storage` device admission and positioned-I/O path.
- **Finding or capability:** F22; mounts on one physical device need one
  scheduling and admission authority.
- **Status:** `wip`

### Scope

Device admission identity is independent of each mount's storage-profile label
and per-mount queue limit. Schedulers that resolve to the same device share a
bounded `DeviceQueue`; schedulers without a device identity retain a
storage-root-scoped fallback. The queue capacity comes from the shared storage
I/O depth, while active dispatch uses one slot per portable worker or one
bounded io_uring submission batch.
Queued work uses the `DeviceElevator` class weights with weighted round-robin
selection across mounts. A unique operation ID and permit travel through the
existing completion owner. Cache hits and file-open preparation happen before
device admission so they do not occupy a physical-device slot.

The live service currently covers scheduler positioned reads/writes/syncs,
peer-read batches, preallocation, and extent probes. The follow-on tickets own
the remaining consumers: IO-014 routes secure copy plans, IO-009 batches grouped
barriers, and NET-114 supplies peer deadlines and choke-critical priorities.
`MountScheduler::run_device_operation` now provides a reusable handoff for
blocking disk work owned by another subsystem. It retains mount/device/byte
admission through closure completion; secure-copy plans have not adopted the
handoff yet. Cross-mount read coalescing and full injected-backend accounting
remain open.

### Ownership and invariant

- **Current owner:** a weak registry shares one bounded, class-aware dispatcher
  by device ID. Per-mount semaphores still enforce local queue and class caps;
  `DiskSubmission` retains byte, mount, and device-operation ownership through
  the selected backend's completion path.
- **Target owner:** the device service owns every disk-bound operation record
  from dispatch through completion and applies class, deadline, and
  choke-critical policy across all mounts on that device.
- **Invariant preserved:** one physical device ID maps to one admission pool,
  independent of profile labels and per-mount `max_queue` values.
- **Must not:** create a second device gate because the same device was
  classified differently, or release a permit before accepted backend work
  completes.

### Acceptance

- Schedulers with one device ID share the same admission owner across profile
  and per-mount queue differences.
- Engine queue-capacity/available summaries count one shared device ID once,
  even when its mounts expose different profile labels.
- Unknown topology remains bounded by a storage-root fallback.
- Scheduler reads, writes, syncs, preallocation, extent probes, and coalesced
  peer reads pass through the device owner with one operation identity.
- Queued classes receive the documented weighted share; canceled waiters are
  removed and accepted operations retain the device slot through late backend
  completion. The service caps active operations to its backend dispatch
  window and bounds pending records by the remaining device queue depth.
- `MountScheduler::run_device_operation` is the reusable dispatch handoff for
  non-scheduler consumers. Routing secure-copy plans, peer
  deadlines/choke-critical promotion, and grouped barriers belongs to IO-014,
  NET-114, and IO-009 respectively.
- Cancellation and injected-backend regression sources cover the full
  scheduler permit and queued-byte accounting path.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`,
  `elevator.rs`, the portable backend, and deterministic backend fixtures.
- **State or wire compatibility:** no resume, storage-path, or wire-format
  change.
- **Migration / rollback:** preserve the portable direct path until the
  service passes correctness and cancellation qualification.
- **Feature flag or rollout boundary:** no service rollout until all operations
  use the shared dispatcher.

### Evidence

- **Regression source added or updated:**
  `schedulers_on_same_device_share_global_queue_across_profiles` covers one
  physical ID with different profile and `max_queue` values;
  `engine_stats_does_not_double_count_shared_device_queue_across_profiles`
  covers its aggregate queue counters; `device_dispatch_uses_weighted_class_fairness`
  records the class share and `cancelled_device_waiter_releases_bounded_queue_capacity`
  covers pre-dispatch cancellation; `cancelled_waiter_keeps_submitted_write_dirty_until_barrier`
  now also checks that caller cancellation retains the device slot until the
  injected backend completion; `external_device_handoff_shares_admission_across_mounts`
  checks that an external blocking operation shares device admission across
  schedulers. These sources are not run.
- **Command or probe to run:** `cargo test -p rt-storage` in the requested bulk
  pass.
- **Expected result:** every device-bound operation uses the same bounded
  service while per-mount queue limits remain independent; operation ownership
  survives cancellation through backend completion.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** fairness and device saturation under
  HDD, NVMe, network mounts, and concurrent multi-mount workloads.

## IO-007 — Batched hash jobs and shared hybrid content views

- **Priority / wave:** P0 / 5
- **Prerequisites:** IO-002, IO-003, IO-005
- **Owning component:** `rt-storage` hashing and the production hybrid actor.
- **Finding or capability:** v1 and BEP 52 checks must reuse immutable payload
  bytes, submit bounded hash work in batches, and verify both integrity views
  for hybrid torrents.
- **Status:** `wip`

### Scope

`MountScheduler` already shares a bounded hash worker pool across schedulers
with matching storage configuration. v1 recheck streams SHA-1 updates, v2
recheck hashes up to 64 leaves per job, and the final streaming Merkle fold now
runs as one hash-pool job. The remaining work is a common immutable content
view that can feed v1 and v2 verification without copying or rereading the same
piece, including file-boundary and BEP 47 padding behavior. Hybrid production
metadata currently enters the v1 actor, so this view is not live yet.

### Ownership and invariant

- **Current owner:** each verification task owns its piece/file geometry and
  its immutable `Bytes`; the scheduler owns bounded hash-worker admission.
- **Target owner:** the shared transfer/storage boundary derives v1 and v2
  ranges from one immutable content lease and retains it until all requested
  hash jobs finish.
- **Invariant preserved:** queue rejection cannot consume the caller's only
  hash state or release the payload lease early; v1 and v2 views refer to the
  same verified content while preserving their distinct padding rules.
- **Must not:** retain one leaf hash per byte range for a whole large file or
  duplicate payload buffers just to change hash geometry.

### Acceptance

- Hash workers remain bounded by the shared queue and owned resource limits.
- Large-file v2 verification batches leaf hashes and folds the root off the
  async executor without retaining all leaf hashes.
- Hybrid tests cover ranges that align, cross v1/v2 boundaries, include v1
  padding, and end at a short final file piece; both hashes must be checked.
- Cancellation and queue-full retry retain immutable content and capacity
  ownership through the final hash completion.

### Evidence

- **Regression source added or updated:**
  `rt-storage::verify::v2_file_verify_batches_leaf_hashing_per_chunk` now
  asserts three leaf batches plus one root job. Hybrid shared-view and
  cancellation cases remain open.
- **Command or probe to run:** `cargo test -p rt-storage verify::tests` and the
  focused hybrid actor regressions in the deferred bulk pass.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** none for the bounded hash contract;
  whole-client hash throughput and CPU remain PERF-107/110/113 evidence.

## NET-103 — Reserve receive capacity before requests

- **Priority / wave:** P0 / 4
- **Prerequisites:** NET-101, NET-102, IO-003
- **Owning component:** `rt-engine` v1 and pure-v2 peer request paths.
- **Finding or capability:** F13/F16; an admitted wire response needs a byte
  reservation before the request can cause the receive allocation.
- **Status:** `wip`

### Scope

Acquire `PeerBuffer` capacity before each v1 or pure-v2 request reaches the
wire. The v1 reservation moves through the peer command and outstanding state
into the matching `LeasedBlockEvent`; the pure-v2 reservation stays with its
outstanding coordinate through piece storage and verification. Both paths retry
after temporary capacity denial.

This slice does not wire the pure transfer ledger into the actors, introduce
cross-plane immutable spans, or integrate hybrid payload ownership.

### Ownership and invariant

- **Current owner:** v1 `TorrentTask` and `V2TorrentTask` choose coordinates;
  their peer loops own outstanding responses.
- **Target owner:** the request lease moves from the actor's command to the
  v1 outstanding request and then its received block event; pure-v2 retains it
  through the request's final block consumer.
- **Invariant preserved:** no v1 or pure-v2 request reaches the wire unless
  its expected payload bytes have already been admitted under
  `MemoryClass::PeerBuffer`.
- **Must not:** reserve again after receiving an admitted response or strand a
  picker coordinate when a request command, socket write, peer event, or
  terminal path fails.

### Acceptance

- A denied reservation prevents command enqueue and returns the picker block.
- The peer command owns the reservation while queued; a choked or abandoned
  command releases it.
- A matching response transfers the reservation to its `LeasedBlockEvent`
  without an extra allocation charge.
- A pure-v2 request stores its lease with the outstanding coordinate until
  Piece, Reject, legacy Choke, or disconnect; temporary admission denial is
  retried on the one-second peer tick.
- Reject, legacy Choke, timeout, disconnect, and failed delivery release byte
  leases and return coordinates through the existing lifecycle events. Fast
  Choke retains on-wire requests and their leases until a terminal response.
- A failed memory admission returns the picker coordinate and schedules a
  bounded retry on the existing one-second peer-control tick.
- Unsolicited or mismatched blocks do not inherit a request lease.
- Shared v1/v2/hybrid spans and device-operation ownership remain outside this
  slice and are tracked by IO-003/IO-005/NET-104/NET-114.

### Files and compatibility

- **Expected code paths:** `torrent_task.rs`,
  `torrent_task/peer_transfer.rs`, `torrent_task_v2.rs`, and the pure
  `transfer::Ledger` contracts.
- **State or wire compatibility:** no wire or resume-state change; request
  admission may stop earlier under the existing memory cap.
- **Migration / rollback:** none; the actor remains on its current picker and
  request maps.
- **Feature flag or rollout boundary:** current v1 and pure-v2 peer request
  paths; hybrid/shared-ledger rollout remains NET-114.

### Evidence

- **Regression source added or updated:**
  `drained_download_bucket_schedules_a_token_ready_wakeup` checks the queued
  request owns a byte lease;
  `peer_request_admission_retries_after_payload_capacity_returns` checks a
  denied request releases its picker slot and retries after capacity returns;
  `peer_loop_moves_request_reservation_into_piece_event` checks that the same
  reservation remains charged through the received event and releases at
  drop. Focused v2 admission and lease-lifetime regression coverage remains
  open.
- **Command or probe to run:** targeted request-admission and peer-loop lease
  regressions during the requested bulk pass.
- **Expected result:** request bytes are charged before enqueue, stay charged
  without a second lease while outstanding and through event handling, then
  return to zero on all terminal paths.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** bounded mixed-peer and v2/hybrid
  qualification.

## NET-104 — Authoritative request and content ownership ledger

- **Priority / wave:** P0 / 5
- **Prerequisites:** NET-101, NET-102, NET-103, IO-005
- **Owning component:** pure transfer core in `rt-engine`.
- **Finding or capability:** torrent-wide single ownership, bounded late
  responses, and generation-safe peer identity.
- **Status:** `wip`

### Scope

The pure `Ledger` owns per-piece block state and per-connection reservations,
reconciles response/commit event reordering, accepts useful late data, and
returns bounded protocol actions. v1, pure-v2, hybrid, and storage execution
remain on their reference paths until NET-114.

### Ownership and invariant

- **Current owner:** actor-local maps and piece assemblies remain authoritative
  in production; the pure ledger owns only simulator/replay state.
- **Target owner:** NET-114 makes this ledger authoritative for all protocol
  planes, with storage retaining payload bytes until verification commits.
- **Invariant preserved:** each missing block is reserved once outside
  endgame; connection generations isolate stale events; received blocks are
  verified as complete pieces before `have` is set.
- **Must not:** infer a peer identity from an address or let a late response
  mutate a replacement connection or newer content epoch.

### Acceptance

- The per-piece slots move only through missing, reserved, received, and
  verification outcomes; invariants cross-check slot owners, connection maps,
  and in-flight totals.
- Response-before-commit is reconciled once; its pending reconciliation
  history and late-response tombstones have hard per-connection caps.
- A useful tombstoned response may supersede a replacement request and
  returns cancellation actions for the replacement owners.
- Completion, hash failure, choke, reject, timeout, disconnect, resume-have,
  and epoch advance have regression sources.
- Seeded simulation replays churn, loss, late replies, and hash failures with
  an invariant check after every event.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/{ledger,sim}.rs`.
- **State or wire compatibility:** pure module only; no live protocol behavior
  changes until NET-114.
- **Migration / rollback:** keep actor-local reference paths selectable while
  NET-114 rolls protocol planes over.
- **Feature flag or rollout boundary:** not live; simulator and bulk pass must
  qualify before actors rely on the ledger.

### Evidence

- **Regression source added or updated:** ledger transition cases and seeded
  simulator replay; current additions include bounded commit history and late
  data superseding a replacement reservation.
- **Command or probe to run:** transfer-core and simulator tests during the
  requested bulk pass.
- **Expected result:** all transitions preserve ledger conservation and all
  configured simulator seeds complete reproducibly.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** live actor parity, hybrid cross-plane
  transfer, and storage commit ownership.

## NET-105 — Negotiated peer request obligations

- **Priority / wave:** P0 / 6
- **Prerequisites:** NET-104
- **Owning component:** v1 and pure-v2 peer protocol handlers, then the shared
  transfer runtime.
- **Finding or capability:** F08; Fast negotiation changes request, choke,
  reject, and cancellation lifecycles.
- **Status:** `wip`

### Scope

Honor negotiated BEP 6 behavior in the existing handlers while moving the
complete obligation state machine into the live shared runtime. The pure
`transfer::InboundObligations` module remains the protocol reference; v1 and
v2 currently have actor-local request state.

### Ownership and invariant

- **Current owner:** v1 upload work is owned by the peer loop; pure-v2 request
  and piece state is owned by its peer protocol loop.
- **Target owner:** `InboundObligations` and the shared ledger own response
  obligations across both protocol planes.
- **Invariant preserved:** when Fast is negotiated, a request already
  accepted for processing remains owed a Piece or Reject response across
  Choke and Cancel; Reject is sent only when the peer negotiated Fast.
- **Must not:** clear a Fast request merely because Choke arrived, send a
  Reject to a legacy peer, or answer one request more than once.

### Acceptance

- v1 and v2 emit Reject only when Fast was negotiated.
- A Fast Choke preserves on-wire v1 and v2 requests. The v1 peer loop retains
  their response-byte leases and its actor cancels commands that had not
  reached the wire; v2 retains pending request coordinates and assembly
  state. Legacy Choke may clear that state.
- A remote Cancel removes a v1 upload request only while it is still queued
  for storage admission. Under Fast, that canceled queued request gets a
  Reject; an in-service read keeps its Piece/Reject obligation.
- Once a v1 block is accepted, the actor queues endgame Cancel for peers with
  duplicate requests. A peer loop sends it only if that request reached the
  wire and keeps the response lease until its terminal event.
- Request cancellation and endgame loser handling reconcile exactly once with
  queued reads, in-flight work, Piece delivery, and Reject delivery.
- Stale, duplicate, and unsolicited responses cannot mutate current content.
- The production v1, v2, and hybrid actors use the shared obligation ledger;
  pure-module tests alone do not close this ticket.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/obligations.rs`,
  `torrent_task.rs`, `torrent_task_v2.rs`, and NET-114 peer-runtime adapters.
- **State or wire compatibility:** no wire-format change; negotiated Fast
  behavior follows the peer's handshake capabilities.
- **Migration / rollback:** none; retain current actor-local state until the
  shared runtime has parity and rollback coverage.
- **Feature flag or rollout boundary:** no new advertisement or capability
  until the applicable live path satisfies its obligations.

### Evidence

- **Regression source added or updated:**
  `v2_reject_obligations_require_fast_negotiation`,
  `v2_choke_preserves_fast_request_obligations`,
  `fast_choke_keeps_wire_request_lease_and_legacy_choke_releases_it`,
  `fast_choke_cancels_queued_requests_but_keeps_wire_requests`, and the
  existing v1 `fast_peer_request_while_choked_is_rejected_not_dropped`.
- The new v1 queued-versus-in-service Cancel and endgame Cancel paths still
  need focused regression sources before NET-105 can close.
- **Command or probe to run:** targeted v1/v2 Fast lifecycle and shared-ledger
  simulations during the requested bulk pass.
- **Expected result:** no owed request is lost across Fast Choke/Cancel and
  each negotiated request reaches one terminal response.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** public-client interoperability.

## NET-111 — Fair seed and download choking policies

- **Priority / wave:** P1 / 10
- **Prerequisites:** NET-104, NET-107, NET-110
- **Owning component:** `rt-peer-manager` policy, v1 and pure-v2 peer actors.
- **Finding or capability:** F12; choose upload slots from useful peer feedback
  and preserve exploration for peers without a recent history.
- **Status:** `wip`

### Scope

The shared `Choker` ranks peers by download contribution while downloading and
upload contribution while seeding. It assigns regular slots plus a rotating
optimistic slot. Pure-v2 peer loops now apply the same policy through a
task-owned watch keyed by connection identity, so reconnecting at the same
address does not inherit an old slot. The choker accepts a seed for
deterministic tie replay and an explicit monotonic timestamp for replay; the
full swarm-trace input/output harness remains open.

### Ownership and invariant

- **Current owner:** v1 `TorrentTask` owns choker cadence and peer snapshots;
  pure-v2 now uses that policy but still owns its peer state separately.
- **Target owner:** one mode-aware policy ranks comparable peer snapshots;
  socket-owning peer loops serialize the resulting wire transitions.
- **Invariant preserved:** only interested peers compete, slot count stays
  bounded, optimistic rotation avoids repeatedly selecting the same waiting
  peer, ranking uses the rate appropriate to download or seed mode, and a
  revoked slot is rechecked after storage and rate waits before sending data.
  A replacement connection receives a fresh policy identity even when its
  socket address is reused.
- **Must not:** infer useful transfer from slot membership or let the choker
  write peer sockets directly.

### Acceptance

- Downloading mode rewards recent bytes received; seeding mode rewards recent
  bytes sent.
- Not-interested peers stay choked; regular and optimistic slots are bounded.
- Optimistic ties can be replayed from a fixed seed and waiting peers receive
  fair rotation.
- A caller-supplied monotonic timeline drives rotation without wall-clock
  sleeps.
- v1 and pure-v2 actors send only actual choke-state transitions.
- A pure-v2 upload that finishes after its slot is revoked is discarded and
  resolved according to the negotiated Fast behavior.
- Reconnecting at the same socket address does not inherit the prior
  connection's selected slot.
- A deterministic swarm trace covers mode changes, stale rates, interest
  changes, joins, departures, and optimistic-slot fairness.

### Files and compatibility

- **Expected code paths:** `crates/rt-peer-manager/src/choker.rs`,
  `crates/rt-engine/src/torrent_task.rs`, `torrent_task_v2.rs`, and a future
  trace-replay fixture.
- **State or wire compatibility:** no wire-format change; slot behavior changes
  only peer upload scheduling.
- **Migration / rollback:** retain the reference actor path until shared-core
  migration has parity and rollback coverage.
- **Feature flag or rollout boundary:** none; external swarm evidence remains
  qualification work.

### Evidence

- **Regression source added or updated:** mode-aware rankings, optimistic
  history, seeded tie replay on virtual monotonic time, interest transitions,
  v2 wire-state changes, and connection identity across reconnect.
- **Command or probe to run:** peer-manager and v2 peer-policy tests during the
  requested bulk pass.
- **Expected result:** mode changes select the matching feedback rate, seeded
  traces replay identically, and v2 emits no duplicate choke transitions.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** real-client seed and download swarm
  behavior under mixed peer speeds.

## NET-110 — Bounded shared upload service

- **Priority / wave:** P1 / 9
- **Prerequisites:** NET-103, NET-105, NET-106, NET-107
- **Owning component:** pure transfer core, then live peer upload runtime.
- **Finding or capability:** F06/F08; valid upload demand must not be silently
  dropped, and accepted requests keep their negotiated terminal response.
- **Status:** `wip`

### Scope

The pure `UploadService` queues protocol obligations per connection, dispatches
bounded byte-admitted reads in round-robin order, and reports whether a read
result may still be sent. Requests with zero or greater-than-one-block lengths
are refused. The configured byte ceiling has a one-wire-block minimum, then
acts as a hard cap. A disconnect ends protocol obligations but does not release
capacity for reads already dispatched to storage. Actor integration and
independent read/write progress remain NET-106/NET-114.

### Ownership and invariant

- **Current owner:** current v1/v2 peer loops own upload reads and responses;
  the pure service is not live.
- **Target owner:** the shared upload service owns accepted request obligations
  and in-flight read accounting until storage completion.
- **Invariant preserved:** queued and active reads stay within per-peer queue,
  global job, per-peer job, and byte limits; a connection identity is not
  reused while a storage read from that generation is live; completed reads
  for a closed peer are discarded without reopening admission early.
- **Must not:** drop valid demand because of an arbitrary time window or reclaim
  an in-flight budget while its backend read may still be running.

### Acceptance

- Fast overload and choked/canceled requests receive the required Reject;
  legacy peers receive no unsupported Reject.
- Per-connection round robin prevents one peer's queue from occupying all
  dispatches.
- Job and byte budgets bound dispatched reads, including after disconnect;
  one valid block is admitted even when configuration is below one block.
- Re-registering a live connection identity is refused, including while its
  disconnected generation still owns an unfinished read.
- Regression sources cover queue bounds, byte bounds, fairness, invalid wire
  lengths, connection-generation reuse, choke, cancel, and disconnect
  completion ownership.
- NET-114 routes live v1, v2, and hybrid uploads through the service before the
  capability is considered integrated.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/{upload,obligations}.rs`
  and the NET-114 peer adapters.
- **State or wire compatibility:** no wire-format change.
- **Migration / rollback:** actor-local request handling remains selectable
  until shared-service parity and rollback coverage exist.
- **Feature flag or rollout boundary:** no release capability claim until
  live paths and cancellation accounting qualify.

### Evidence

- **Regression source added or updated:** upload service queue, fairness,
  budgets, choked reads, and disconnect-held budget cases.
- **Command or probe to run:** transfer-core upload tests during the requested
  bulk pass.
- **Expected result:** no accepted request disappears and disconnected reads
  remain accounted until completion.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** real-client Fast behavior and disk
  cancellation latency under load.

## IO-009 — Per-file durability watermarks

- **Priority / wave:** P0 / 9
- **Prerequisites:** IO-004, IO-006, IO-008
- **Owning component:** `rt-storage::MountScheduler` write and sync paths.
- **Finding or capability:** F19 and F21; barriers must cover the admitted
  file writes they claim to make durable and must not erase newer dirty
  evidence.
- **Status:** `wip`

### Scope

Order checkpoint barriers against writes to each normalized path and across
the scheduler's complete path set. Ordinary writes share scheduler admission
and each file gate, so they remain concurrent. A checkpoint takes scheduler
write admission exclusively, waits for admitted write/preallocation/sync
operations, snapshots dirty and active paths, then syncs those paths one at a
time under exclusive per-file fences. Later writes, including writes to a new
path, remain queued until the evidence is dropped after the resume commit.
Dirty generations remain separate from per-file sequence watermarks so a newer
generation cannot be accidentally cleared.

### Ownership and invariant

- **Current owner:** each `MountScheduler` clone shares scheduler-wide write
  admission, a global diagnostic write sequence, weakly registered per-path
  fences, its dirty-generation map, and completion watchers for submitted
  writes and syncs.
- **Target owner:** IO-006's shared device service batches ready per-file
  fences by device while each path retains its own completion watermark.
- **Invariant preserved:** a scheduler-wide checkpoint admits no later
  write/preallocation/sync operation until every path in its stable snapshot
  has completed `fdatasync`; per-file fences and completion watchers retain
  ownership across caller cancellation.
- **Must not:** clear a newer generation, sync a clean unrelated descriptor,
  or let preallocation/write completion pass a barrier queued behind it.

### Acceptance

- `sync_data` and checkpoint sync capture a sequence watermark after all
  earlier admitted writes to that path complete.
- A checkpoint cannot miss a write admitted on a new path after path
  enumeration begins; that write waits for the returned barrier and remains
  dirty for the next one.
- A write queued behind a path's exclusive fence waits until its `fdatasync`
  finishes and remains dirty for a later barrier.
- Failed barriers and partial writes retain dirty evidence; a successful
  strict write clears only its own generation.
- `sync_all_open_files` keeps descriptor use bounded and syncs only its
  scheduler's dirty or active paths.
- `sync_all_open_files_with_evidence` returns an opaque proof only after every
  path in the barrier succeeds; it reports synced-file count and the stable
  scheduler write-sequence watermark, and it retains write admission until
  the proof is dropped after its resume commit.
- Per-device grouped barrier batching is still part of IO-006 and remains open.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`, IO-006's
  device service, and crash-cut fixtures.
- **State or wire compatibility:** no protocol change; the evidence API is
  additive and does not alter storage payload layout.
- **Migration / rollback:** none; the reference backend stays selectable.
- **Feature flag or rollout boundary:** durability mode is unchanged.

### Evidence

- **Regression source added or updated:** existing canceled-writer,
  dirty-generation, and failed-barrier retry sources; a held-sync regression
  now covers a new-path write queued behind the scheduler-wide checkpoint.
- **Command or probe to run:** storage fault matrix and barrier ordering
  regressions during the requested bulk pass.
- **Expected result:** a returned barrier covers every earlier admitted write
  to its path, preserves all later dirty generations, and leaves unrelated
  paths untouched.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** target-filesystem crash validation.

## IO-010 — Resume persistence tied to barrier evidence

- **Priority / wave:** P0 / 10
- **Prerequisites:** IO-009, SYS-009
- **Owning component:** `rt-engine` fast-resume save path and `rt-fastresume` durability watermark.
- **Finding or capability:** a persisted resume record must distinguish data
  proven durable by a completed storage barrier from a save that skipped or
  could not perform a data sync.
- **Status:** `wip`

### Scope

Consume `rt-storage::DurabilityBarrierEvidence` when a successful checkpoint is
attached to fast-resume state. The proof retains scheduler write admission
through the blocking atomic resume-file write, even if the async caller is
canceled. Persist the synced-file count and the scheduler write-sequence
watermark for audit and recovery diagnostics. A fast-mode save
or unsupported filesystem records `synced = false` and zeroes the current
barrier evidence; a failed barrier leaves the checkpoint unclean. Additive
watermark fields default to zero when reading existing records.

### Ownership and invariant

- **Current owner:** `MountScheduler` constructs opaque evidence; the
  `TorrentTask` save path passes it to `FastresumeStore`, which attaches the
  resulting watermark and keeps the proof alive in its blocking writer until
  the existing atomic fast-resume write returns.
- **Target owner:** the same storage-to-resume evidence boundary remains, with
  engine/database publication ordering integrated under SYS-009.
- **Invariant preserved:** a record can set `synced = true` only after
  consuming evidence returned by a fully successful scheduler barrier. A
  skipped, unsupported, failed, or missing proof cannot be converted into a
  synced record.
- Canceling the async waiter after the atomic writer starts does not release
  scheduler write admission before the resume write completes.
- **Must not:** fabricate proof in `rt-fastresume`, infer durability from
  `clean_shutdown`, or break decoding of pre-evidence records.

### Acceptance

- Fast-resume state consumes evidence rather than accepting a no-argument
  “barrier completed” call.
- Synced records persist the evidence's file count and write-sequence
  watermark; legacy records decode with zero for those new fields.
- Fast/unsupported saves explicitly remain unsynced and carry no current
  barrier evidence.
- A storage barrier failure leaves the state unclean and does not clear the
  engine's dirty-piece watermark.
- Regression sources cover real storage evidence, fast-mode state, and forced
  completion barriers.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`,
  `crates/rt-fastresume/src/state.rs`, `crates/rt-fastresume/src/store.rs`, and
  `crates/rt-engine/src/torrent_task.rs`.
- **State or wire compatibility:** additive JSON fields use serde defaults;
  `FASTRESUME_VERSION` and torrent protocol formats do not change.
- **Migration / rollback:** existing resume records remain readable; a rollback
  must retain the `synced` safety flag and legacy decode defaults.
- **Feature flag or rollout boundary:** none; durability modes are unchanged.

### Evidence

- **Regression source added or updated:** storage creates evidence only after a
  successful barrier; fast-resume consumes it and persists the diagnostic
  watermarks. Sources await the requested bulk pass.
- **Command or probe to run:** storage barrier-ordering and fast-resume
  durability suites during the bulk pass.
- **Expected result:** no save claims stable payload data without an opaque
  successful barrier proof, while legacy fast-resume records remain readable.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** crash-cut qualification on supported
  filesystems and integration with SYS-009 persistence ordering.

## IO-011 — Reuse-driven peer-read readahead

- **Priority / wave:** P1 / 9
- **Prerequisites:** IO-004, IO-006, IO-008
- **Owning component:** `rt-storage::MountScheduler` peer-read path.
- **Finding or capability:** F18; speculative reads should follow observed
  sequential demand and remain within admitted memory/queue budgets.
- **Status:** `wip`

### Scope

Sequential-only readahead now starts at an exact first read and bootstraps one
prefetch on the first continuation. Later growth is driven by cache hits and is
gradual; a miss holds the current recommendation rather than expanding
speculative I/O. Recommendations remain bounded by the configured maximum, and
a discontinuity resets to the requested range. Stream history is bounded with
single-entry least-recently-used eviction, so admitting a new file does not
clear every warm stream. The stored cache is still the existing bounded
forward-read cache; a full useful-range policy and live-engine qualification
remain open.

### Ownership and invariant

- **Current owner:** `MountScheduler` owns stream history, cache entries, and
  per-read admission; the portable backend owns the submitted read.
- **Target owner:** IO-006 device service owns speculative capacity and IO-008
  actor progress consumes completed reads without waiting on unrelated work.
- **Invariant preserved:** readahead never exceeds the configured cap or file
  extent, and its additional queued-byte charge plus frame reservation are
  acquired before the backend read is submitted.
- **Must not:** expand an isolated seek or discard a requested range when
  speculative admission is denied.

### Acceptance

- The first read and any nonsequential jump remain exact when sequential-only
  mode is enabled.
- Repeated continuation grows only to the configured maximum; per-file
  histories remain bounded.
- Cache hits may grow the recommendation gradually; misses do not expand the
  speculative range.
- Added buffer bytes are charged before allocation/submission and released only
  after completion.
- Toggle-off preserves the existing unconditional configured range behavior.
- Regression sources cover one-block start, growth, discontinuity reset,
  single-entry stream-history eviction, cache behavior, and queue/frame
  pressure.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs` and storage
  configuration docs.
- **State or wire compatibility:** no persistent or wire-format change.
- **Migration / rollback:** the existing `peer_read_readahead_sequential_only`
  setting can disable sequential gating.
- **Feature flag or rollout boundary:** enabled by default through storage
  configuration; performance claims wait for PERF-109 and IO-016.

### Evidence

- **Regression source added or updated:** `readahead_expands_only_continuing_streams`
  covers bounded per-file growth and discontinuity reset;
  `readahead_stops_growing_when_prefetch_is_not_reused` covers hit/miss
  feedback; scheduler cache regressions remain in the bulk pass.
- **Command or probe to run:** storage scheduler tests during the requested
  bulk pass, then matched fixture runs for reuse and read amplification.
- **Expected result:** exact reads for isolated requests, bounded growth for
  continuous streams, and no speculative budget leak.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** filesystem/backend read amplification
  and throughput under realistic peer request patterns.

## PERF-101 — Trial measurement contract and schemas

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** certification tooling and evidence format.
- **Finding or capability:** reproducible trials tied to immutable source,
  executable, fixture, settings, and environment identity.
- **Status:** `wip`

### Scope

Define trial timing, byte accounting, result outcomes, provenance, strict
trial/manifest records, and schema validation. This does not implement client
adapters, fixture controls, paired-run analysis, or adopt the proposed
scorecard thresholds.

### Ownership and invariant

- **Current owner:** certification scripts and report authors.
- **Target owner:** a shared harness that emits immutable manifest-linked
  evidence.
- **Invariant preserved:** incomplete, invalid, failed, and unmeasured attempts
  remain visible; only complete trials with applicable passing integrity may
  enter scorecards.
- **Must not:** infer identity from a filename or treat missing counters as
  zero.

### Acceptance

- Contract separates barrier, transfer, flush, validation, and shutdown time.
- Useful payload excludes duplicates, protocol overhead, and padding.
- Schemas reject malformed identities and invalid complete-trial combinations.
- The client-observation schema pins runtime identity and redacted settings.
- The pinned validator checks trial, manifest, and client-observation
  schemas; manifest validation verifies non-symlink artifact/trial digests,
  source identity, and linked client/version/settings/process identity.
- Missing or unattributable platform counters remain explicit `null` values.
- Scorecard thresholds remain proposed until separately adopted.

### Files and compatibility

- **Expected code paths:** `certification/transfer-rebuild/` schemas and
  `measurement-contract.md`, plus `scripts/transfer_rebuild/` validation.
- **State or wire compatibility:** new evidence format `transfer-trial-v1` and
  `transfer-manifest-v1`; no product state change.
- **Migration / rollback:** schema versioning is additive; old reports remain
  historical evidence.
- **Feature flag or rollout boundary:** no benchmark claims until PERF-104 and
  PERF-113 build on this contract.

### Evidence

- **Regression source added or updated:** complete-trial rules, malformed
  hashes, unknown fields, manifest digest shape, and referenced-file/provenance
  checks in
  `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** schema validation and contract fixtures during
  the requested bulk pass; install the pinned helper dependency first.
- **Expected result:** invalid types, hashes, and complete-trial omissions are
  rejected.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending schema fixtures and bulk pass.
- **External evidence still required:** no performance evidence is produced by
  this contract.

## PERF-102 — Independent resource observer

- **Priority / wave:** P0 / 3
- **Prerequisites:** PERF-101
- **Owning component:** Linux certification tooling.
- **Finding or capability:** independent cumulative process-tree and cgroup-v2
  resource observation.
- **Status:** `wip`

### Scope

Sample process-tree RSS/PSS, cgroup memory, CPU, threads, descriptors, and
`/proc/<pid>/io` counters without retaining the sample series in memory.
Network attribution, fixture control, and a runner integration remain open.

### Ownership and invariant

- **Current owner:** external observer process; it does not control the client.
- **Target owner:** PERF-102 observer, consumed by PERF-104 trial adapters.
- **Invariant preserved:** RSS/PSS, cgroup anonymous/file/current, and process
  I/O remain separate accounting domains; `inactive_file` is kept distinct as
  reclaimable cache, and unavailable values stay null.
- **Must not:** signal or restart the measured process, or present
  unattributed network bytes as client traffic.

### Acceptance

- Descendant discovery covers children spawned by any process thread.
- Cumulative samples are streamed to CSV; peak summary has constant memory.
- CSV and summary artifacts are created exclusively with private permissions;
  rerunning an observer cannot replace prior raw evidence.
- Missing process counters do not become partial undercounts.
- Network counters remain null until a process-attributable independent source
  is implemented.
- Observer exits clearly when the process was never observable.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/observe.py`, observer
  README, and trial schema.
- **State or wire compatibility:** certification artifact format only.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** Linux process and cgroup v2 only;
  observer does not claim support for other platforms.

### Evidence

- **Regression source added or updated:** peak/null-counter fixtures and
  synthetic process-tree descendants, including a non-leader thread child.
- **Command or probe to run:** `python3 scripts/test_transfer_rebuild_observer.py`
  during the requested bulk pass.
- **Expected result:** cumulative counters and process-tree attribution match
  synthetic fixtures without retaining samples.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** Linux cgroup-v2 permission and overhead
  characterization on the dedicated qualification host.

## PERF-103 — Pinned client adapters and settings readback

- **Priority / wave:** P0 / 3
- **Prerequisites:** PERF-101
- **Owning component:** `scripts/transfer_rebuild/adapters.py` and the client
  observation schema.
- **Finding or capability:** every paired trial identifies the client process
  and reads back the settings declared by its profile.
- **Status:** `wip`

### Scope

Capture version and effective settings for TorrentNG, qBittorrent,
Transmission, Deluge, rTorrent, rqbit, and a direct `urtorrent` library runner.
Pin container clients by immutable
image digest and runtime container/process identity; pin native clients by
binary digest, executable inode, and process start time. Also pin the source
commit, dirty state, lockfile, and toolchain when a source checkout is
available. Redact credentials before hashing or writing observations. Bind
every trial to its observation through the manifest.

rqbit's API exposes server identity and status, not resolved settings. Its
adapter pins its running command line, an allowlist of environment settings,
and declared configuration-file digests exactly; it does not claim to observe
hidden defaults. The full paired runner and settings-surface review remain
open.

The `urtorrent` comparator is a separate Linux-only binary pinned to library
0.14.3 and Rust 1.98.1; it is not a TorrentNG production dependency. It records
`Session::settings()` and per-torrent status against the live process identity.
The API does not expose the configured hash-pool thread count, so the snapshot
labels that value as a command-line input instead of claiming effective
readback. The runner requires private v1 metainfo, rejects v2/hybrid fixtures,
binds to one private, link-local, or loopback IPv4 interface, removes tracker
and web-seed endpoints before release, and dials one explicit private,
link-local, or loopback IPv4 peer. IPv6, DHT, PEX, and LSD are disabled, and
the adapter independently checks both addresses from the live process-bound
snapshot. Its control loop stays inside the measured process. Its
barrier-to-trial-end duration is diagnostic and does not replace the
measurement contract's `transfer_ns`.
This comparator covers v1 only; it cannot decide whether the native core should
be kept, refactored, or replaced until the broader paired comparison is
implemented and qualified.

### Ownership and invariant

- **Current owner:** the read-only adapter captures identity/settings; the
  manifest validator binds observations to trial records.
- **Target owner:** PERF-104 consumes a verified observation before a trial.
- **Invariant preserved:** the executable or image and API endpoint belong to
  the observed process/container; expected version/settings match before an
  artifact is emitted.
- **Must not:** store credentials, accept mutable image tags, or treat missing
  readback as a successful observation.

### Acceptance

- Pin profiles require expected version, expected settings, and immutable
  runtime identity pins.
- Native HTTP APIs are loopback-only and owned by the pinned process; container
  bridge mappings must bind only to loopback and map into the pinned container's
  network namespace. Host-network containers must expose the API only on a
  loopback listener.
- Failed login, unknown version, settings mismatch, unpinned identity, or an
  unobservable API endpoint returns an error without writing an artifact.
- The observation is created once with restrictive permissions and cannot
  overwrite an earlier record.
- Trial and manifest validation bind the observation digest, client/version,
  settings digest, runtime kind, and process start identity.
- The `urtorrent` profile requires the pinned native runner, an absolute live
  settings snapshot, the Linux task namespace, private metainfo, discovery
  disabled, an explicit private IPv4 listener and peer, and v1-only geometry;
  unobservable hash-thread count is labeled as a launch input. The runner
  rejects public peer addresses, existing start-gate files, and v2/hybrid
  metainfo.
- Regression sources cover auth text responses, credential redaction,
  endpoint/process binding, schema linkage, urtorrent snapshot/process binding,
  IPv4 listener and peer policy, fixture-isolation rejection, and mismatch
  rejection.

### Files and compatibility

- **Expected code paths:** adapter source, client-observation and trial
  schemas, validator, measurement contract, harness README, regressions, and
  the standalone `urtorrent_runner` crate with its own lockfile.
- **State or wire compatibility:** versioned certification artifacts only;
  `client_identity` is required in the current `transfer-trial-v1` draft.
- **Migration / rollback:** no trial evidence has been qualified on this branch.
- **Feature flag or rollout boundary:** no comparison is valid until each
  client has a reviewed pin profile and the exact revision passes bulk
  validation.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_adapters.py` (including runner snapshot
  identity and isolation binding), the runner's local-peer policy unit case,
  and `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** `cargo test --locked --manifest-path
  scripts/transfer_rebuild/urtorrent_runner/Cargo.toml`, then the adapter and
  validator regressions during the requested bulk pass.
- **Expected result:** only identity-bound, version-matched, settings-matched
  readbacks produce an observation; mismatches are rejected.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** all seven client pin profiles, API
  surface review for locked versions, Linux io_uring and namespace
  qualification for urtorrent, and local process/container qualification.

## SYS-009 — Production database worker actor coverage

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** `rt-engine` `DbExecutor` and `TorrentTask` progress
  persistence.
- **Finding or capability:** F30; engine actor tests should cover the same
  ordered, supervised SQLite boundary used in production.
- **Status:** `wip`

### Scope

Keep fast in-memory direct executors for most state-machine fixtures, while
adding a file-backed actor regression that opts into the production
`DbExecutor::Worker` path for coalesced transfer-stat persistence.

### Ownership and invariant

- **Current owner:** engine actors order persistence requests; `DbWorker`
  owns its dedicated SQLite connection and commits queued work.
- **Target owner:** unchanged; SYS-009 adds evidence around the existing owner.
- **Invariant preserved:** in-memory progress is not considered persisted
  until the worker's transaction returns successfully; the reopened database
  reflects the last acknowledged row.
- **Must not:** weaken `DbWorker` supervision or make actor tests mutate its
  private SQLite connection.

### Acceptance

- A torrent progress flush runs through `DbExecutor::Worker` against a real
  file-backed database.
- The actor sees old durable values before the flush and updated values after
  it, including after worker shutdown and database reopen.
- Worker metrics show the batched-transaction path ran.
- Existing worker regressions continue to cover concurrent batched siblings,
  command isolation, rollback, panic containment, and shutdown.
- The broader engine fixture set may continue using direct in-memory DBs.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/db_worker.rs`,
  `torrent_task.rs`, and the existing `db_worker.rs` tests.
- **State or wire compatibility:** no database schema or public API change.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** tests opt into the worker; production
  behavior is unchanged.

### Evidence

- **Regression source added or updated:**
  `transfer_stats_are_batched_until_progress_flush_on_db_worker`.
- **Command or probe to run:** during the requested bulk pass, run:

  ```sh
  cargo test -p rt-engine --lib transfer_stats_are_batched_until_progress_flush_on_db_worker
  ```

  followed by the full `rt-engine` bulk suite.
- **Expected result:** the worker-backed progress update is visible after
  restart and increments the batched-transaction counter.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** hosted current-commit CI remains a
  release gate, not part of this ticket.

## SYS-011 — Passive, owner-specific metrics

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** native metrics handler and the snapshots exposed by
  engine, storage, persistence, API, peer-ingress, transport, and crash-safety
  owners.
- **Finding or capability:** F29 and the need to identify which live owner
  supplies every exported metric family.
- **Status:** wip

### Scope

Keep /metrics observational: it reads existing owner snapshots and never
initializes a runtime or worker. Document the metric owner, scope, and limits.
This ticket does not make metrics an activation path for otherwise dormant
subsystems.

### Ownership and invariant

- **Current owner:** each subsystem owns its counters; the native handler
  combines snapshots into Prometheus exposition.
- **Target owner:** unchanged; metrics remain projections of subsystem-owned
  state.
- **Invariant preserved:** collection does not create owners or duplicate
  shared storage accounting; absent owners remain absent or unavailable.
- **Must not:** create StorageRuntime from a scrape, multiply shared storage
  gauges by torrent count, or expose raw torrent IDs by default.

### Acceptance

- The handler uses passive accessors for process-global storage/runtime state.
- The descriptor-budget scrape reports whether its gate exists and never
  raises RLIMIT_NOFILE.
- Metric families and their source owners, scope, privacy, and known limits
  are documented in TRANSFER_CORE_OBSERVABILITY.md.
- Renderer tests cover absent and initialized runtime output and ownership
  labels without requiring a live transfer.
- A fresh-process regression proves a scrape does not create the storage
  runtime or worker.
- Large-catalog and mixed-load scrape cost remains SYS-014/PERF-109 evidence.

### Files and compatibility

- **Expected code paths:** crates/rt-api-native/src/handlers.rs, engine and
  storage metric snapshots, TRANSFER_CORE_OBSERVABILITY.md.
- **State or wire compatibility:** Prometheus names remain stable unless a
  separately reviewed compatibility change is needed.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** existing native /metrics route.

### Evidence

- **Regression source added or updated:** F29 uses StorageRuntime::initialized
  in the renderer; a fresh-process descriptor-gate/RLIMIT passivity regression
  is authored but unrun.
- **Command or probe to run:** native metrics renderer and fresh-process
  passivity regressions during the requested bulk pass.
- **Expected result:** a scrape never creates the storage runtime and each
  shared metric is emitted once per owner/resource identity.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** large-catalog and mixed-load
  collection latency and resource impact.

## SYS-012 — Production-shaped protocol and platform compatibility fixtures

- **Priority / wave:** P0 / 3
- **Prerequisites:** SYS-001, SYS-002, SYS-003
- **Owning component:** `rt-testkit`, with production parsing and geometry
  supplied by `rt-metainfo` and `rt-piece-map`.
- **Finding or capability:** preserve v1, pure-v2, and hybrid file/piece
  semantics while the transfer implementation changes, and keep the portable
  product build working on every released platform.
- **Status:** `wip`

### Scope

Provide one deterministic, payload-derived Rust fixture corpus consumed by
production metainfo parsing and piece-map code. Exercise the v1 and v2 views of
the same multi-file torrent, including a multi-piece file, nested path, empty
file, authenticated v2 piece layer, and BEP 47 alignment padding in hybrid
metainfo. The shared core remains optional until SYS-013; this ticket does not
switch production actors or claim equivalent runtime behavior across backends.

The release build matrix remains Linux x86_64, Linux aarch64, macOS aarch64,
and Windows x86_64. The normal workspace suite covers the fixture and full
metainfo/piece-map crates on Linux; the Windows smoke workflow runs those same
crates on Windows and separately exercises the portable storage plan
executor. macOS and Linux aarch64 are release-build targets, not test runners
in this ticket. The fixtures themselves avoid platform-specific paths,
sockets, or filesystem timing.

### Ownership and invariant

- **Current owner:** metainfo and file geometry regressions have separate
  crate-local builders; Windows storage compatibility is covered by its smoke
  workflow.
- **Target owner:** `rt-testkit` owns reusable fixture bytes and payloads;
  parsers and piece maps remain the source of semantic truth.
- **Invariant preserved:** infohashes derive from the exact encoded `info`
  dictionary, while v1 piece hashes and v2 file roots derive from payload
  bytes; hybrid v1 padding is synthetic zero content and is absent from the v2
  payload view.
- **Must not:** duplicate production parsing/geometry rules inside the fixture
  crate, write padding to disk as real content, or describe an authored fixture
  as platform test evidence before it runs there.

### Acceptance

- A repeatable fixture matrix contains v1, pure-v2, and hybrid torrents with
  byte-for-byte stable metainfo and payload.
- V1 piece hashes cover the generated byte stream, including hybrid zero pads;
  v2 file roots and any required piece layer are computed from that file's
  payload blocks.
- Production parsing preserves file order, offsets, piece counts, private
  status, and both hybrid identity views.
- Piece-map regressions show hybrid padding as a synthetic region that does
  not become wanted file content.
- `rt-testkit`, `rt-metainfo`, and `rt-piece-map` run in Linux workspace tests
  and the Windows smoke workflow; existing release builds continue to cover
  the four shipped target combinations above.
- Feature-path changes retain a rollback route through the existing actors
  until SYS-013 qualifies migration and cutover.

### Files and compatibility

- **Expected code paths:** `crates/rt-testkit/src/transfer_fixtures.rs`, its
  dependencies and README, `docs/TRANSFER_CORE_CAPABILITIES.md`, and
  `.github/workflows/windows-smoke.yml`.
- **State or wire compatibility:** no product state, network protocol, or
  stored metainfo format change; fixtures use valid BEP 3, BEP 47, and BEP 52
  encodings.
- **Migration / rollback:** none; production routing remains unchanged.
- **Feature flag or rollout boundary:** protocol variants are fixture cases,
  not runtime feature toggles. Shared-core production cutover remains SYS-013.

### Evidence

- **Regression source added or updated:** `rt-testkit` deterministic corpus
  tests production parsing, v2 payload hashes, hybrid alignment, and piece-map
  treatment of padding.
- **Command or probe to run:** during the requested bulk pass,
  `cargo test --locked -p rt-testkit -p rt-metainfo -p rt-piece-map` on Linux
  and Windows; retain the existing release target builds for macOS aarch64 and
  Linux aarch64.
- **Expected result:** stable metainfo and hashes, consistent hybrid extents,
  and no padding file treated as payload content.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending the bulk pass.
- **External evidence still required:** hosted Windows smoke and release-matrix
  results for this exact revision.

## NET-112 — Select and qualify the uTP protocol core

- **Priority / wave:** P1 / 4
- **Prerequisites:** NET-101, NET-102
- **Owning component:** `rt-utp` protocol adapter and the engine-owned shared
  peer runtime.
- **Finding or capability:** F07: the current outbound writer is stop-and-wait;
  the current receiver drops reordered DATA and does not apply SACK.
- **Status:** `wip`

### Scope

Keep the native runtime implementation in TorrentNG. Cargo metadata reports
that both reviewed `urtorrent-utp` releases, 0.14.2 and 0.14.3, require Rust
1.98; adopting either would break the workspace's Rust 1.88 floor. Use upstream
as a protocol and implementation reference for windowed sends, selective ACK,
reordering, retransmission, congestion response, MTU handling, and wire
behavior checked against libtorrent captures. Implement the protocol core,
connection manager, stream adapter, queue admission, and memory accounting in
TorrentNG. Target one UDP socket per address family and deadline plus ready
queues rather than per-peer sockets or unconditional full-table scans. Do not
adopt the full `urtorrent` engine or enable uTP by default before NET-116.

The reference's `Vec` API reports three user-space payload copies, its default
receive capacity is 1 MiB per peer, and its manager scans the connection table
for several operations. These are evaluation findings and design constraints,
not claims about the implementation TorrentNG will produce. Keep payload copy
count, admitted bytes, per-peer overhead, and scheduling work measurable.

### Ownership and invariant

- **Current owner:** `rt-utp::UtpStream`, `UtpEndpoint`, and the new
  `UtpRuntime`, with the engine's `GlobalNetworkBudget` owning and sharing the
  runtime across peer and metadata tasks.
- **Target owner:** one TorrentNG uTP runtime owns sockets, protocol state,
  connection identities, deadlines, readiness, and byte admission; each
  logical `UtpStream` is a bounded handle into that owner. The source currently
  has shared socket demultiplexing and bounded endpoint queues; central deadline
  and ready-queue scheduling remain open.
- **Invariant preserved:** every protocol and application buffer stays charged
  until acknowledged, read by the consumer, or safely discarded after stream
  termination; cancellation cannot release memory still owned by the runtime.
- **Must not:** use an ambient blocking thread, silently fall back from a
  rejected uTP datagram into an unbounded queue, exceed aggregate or per-peer
  receive/retransmit budgets, raise the workspace MSRV for the reference
  crate, or claim performance from source inspection.

### Acceptance

- The native workspace remains Rust 1.88 compatible and has no direct or
  transitive dependency on the Rust-1.98-only reference crate.
- Outbound and inbound logical streams share the engine-owned UDP socket for
  their address family; no per-peer UDP socket is created.
- A SYN flood cannot exceed the configured stream cap. Read/write queues,
  receive capacity, retransmit storage, pending datagrams, and deadline/ready
  metadata have hard aggregate budgets.
- The adapter preserves the peer-wire byte-stream interface, configured TCP
  fallback, close/shutdown behavior, and current metrics ownership.
- Deterministic tests cover handshake, window refill, loss/retransmission,
  selective ACK, out-of-order delivery, sequence wrap, zero-window recovery,
  duplicate SYN, inbound admission cap, and two peers sharing a port.
- The seeded transfer simulator and a lossy/reordered local UDP harness agree
  on byte and connection ownership.
- Matched efficiency results report verified goodput, CPU/GiB, process and
  cgroup memory, socket count, payload copies, retransmissions, and queue
  latency. The current backend remains available as a test reference until
  NET-116 accepts the candidate.

### Files and compatibility

- **Expected code paths:** `crates/rt-utp/`, engine uTP endpoint setup,
  `GlobalNetworkBudget`, outgoing connect sites, `docs/protocol/UTP.md`, and
  the uTP evaluation note.
- **State or wire compatibility:** BEP 29 packet and peer-wire behavior must
  remain interoperable; no persisted state format changes.
- **Migration / rollback:** keep the current transport behind a test-only or
  explicit fallback path through correctness and performance qualification;
  do not run two writers for one peer.
- **Feature flag or rollout boundary:** existing incoming/outgoing uTP
  controls; default behavior stays unchanged until NET-116.

### Evidence

- **Regression source added or updated:** existing `rt-utp` transport tests
  remain the baseline; add deterministic state/window and adapter cases with
  NET-112/113.
- **Command or probe to run:** `cargo test -p rt-utp` and the focused engine
  uTP integration cases during the requested bulk pass.
- **Expected result:** all protocol, ownership, socket-sharing, and boundedness
  cases pass on the exact integration revision.
- **Executed on exact revision:** no; all uTP validation is deferred to the
  requested bulk pass.
- **Artifact or report path:** pending implementation and bulk pass.
- **External evidence still required:** matched hardware/network trials under
  PERF-112 through PERF-116; source review is not a performance result.

## NET-113 — Bounded windowed uTP runtime integration

- **Priority / wave:** P1 / 8
- **Prerequisites:** NET-103, NET-106, NET-112
- **Owning component:** `rt-utp` plus the engine's shared network service.
- **Finding or capability:** use one shared UDP receive/demultiplexing path per
  address family for production uTP peers, while keeping protocol-owned bytes
  bounded and safe across async cancellation.
- **Status:** `wip`

### Scope

The source now contains a capped packet/byte send window, cumulative plus SACK
acknowledgement accounting, bounded receive reordering, stream-owned
retransmission payloads, and `UtpRuntime`. The production v1, pure-v2, and
metadata paths connect through that runtime. Incoming and outgoing IPv4 streams
share the listener endpoint when incoming uTP is enabled; outbound-only address
families lazily bind one shared endpoint. Listener rebind retires the old
endpoint for existing routes and limits retained generations. Central timer
scheduling, ready-queue fairness, aggregate memory charging, and the full
loss/reorder acceptance matrix are still open.

### Acceptance

- Every production outbound uTP call site uses the engine-owned runtime; direct
  per-peer `UtpStream::connect` remains only as a standalone crate API and test
  primitive.
- Streams in one endpoint generation share a local UDP socket. Rebinding
  changes the socket for new connections without silently replacing an active
  route; stale endpoints reject new SYNs and are bounded.
- Retransmission bytes survive canceled writes, and receive reorder bytes stay
  under the configured receive budget until delivered or rejected.
- Regression sources cover multi-packet window fill, SACK, reordering, wrap,
  cancellation, multi-stream socket sharing, rebind with an active stream, and
  route caps. All remain unrun until the requested bulk pass.
- Deadline scheduling, queue fairness, and resource/efficiency claims require
  their own implementation and matched evidence; a shared socket alone is not
  a performance result.

### Evidence

- **Regression source added or updated:** `rt-utp` window and transport tests,
  including the shared runtime/rebind case.
- **Command or probe to run:** `cargo test -p rt-utp` and the engine's uTP
  integration tests during the requested bulk pass.
- **Executed on exact revision:** no.
- **External evidence still required:** matched whole-client trials under
  PERF-112 through PERF-116.

## PERF-106 — Deterministic independent swarm and network controls

- **Priority / wave:** P0 / 4
- **Prerequisites:** PERF-101, PERF-105
- **Owning component:** `scripts/transfer_rebuild/network_control.py`,
  network-control schema, trial/manifest validator, and later the paired-run
  harness.
- **Finding or capability:** every claimed network profile is applied to an
  isolated fixture link and bound to immutable readback evidence.
- **Status:** `wip`

### Scope

Control the local peer network and its link profile so capacity and matched
service runs do not depend on public-swarm availability. The first landed
slice applies pinned, loss-free Linux egress profiles to a named task-owned
veth in a `tngt-*` network namespace. It records namespace/interface identity,
iproute2 versions, requested delay/rate/queue, and normalized `tc -j` qdisc
readback. Reset removes only the exact recorded qdisc and verifies the default
root qdisc returns.

The profiles are one-way egress controls. Apply one profile artifact per
shaped endpoint when both directions need shaping. The helper does not create
namespaces, shape host or loopback interfaces, apply random loss/jitter, or
claim measured RTT. Independent tracker/seed/leecher setup, deterministic
peer-start barriers, and paired endpoint orchestration remain open.

### Ownership and invariant

- **Current owner:** `network_control.py` owns the task-namespace qdisc apply
  and reset lifecycle; the manifest validator owns artifact digest and trial
  linkage checks.
- **Target owner:** PERF-104/106's paired runner owns fixture startup,
  independent peers, two-sided shaping, trial start barrier, and cleanup.
- **Invariant preserved:** a profile claim is backed by exact pinned settings
  and qdisc readback on a stable task namespace and interface identity.
- **Must not:** replace an existing non-default root qdisc, overwrite a report,
  touch `lo` or a host interface, treat an unmeasured RTT as profile truth, or
  record randomized loss/jitter without deterministic seed semantics.

### Acceptance

- Only `tngt-*` namespaces and task-named veth interfaces are accepted; the
  helper refuses missing/ambiguous interfaces, non-veth links, changed
  namespace/interface identity, and non-default root qdiscs.
- Apply verifies the pinned profile through `tc -j` before writing a new
  mode-0600 artifact. Failure to persist the apply artifact removes only the
  exact qdisc that this operation installed.
- Reset requires the immutable apply record, rechecks namespace inode and
  interface index, verifies the handle/kind/settings, removes only that qdisc,
  and writes a separate reset artifact.
- Trials with a controlled profile link all apply-artifact digests. Manifest
  validation verifies the file digest, schema, profile settings, observed
  qdisc, tool identity digest, link-rate value, and reset-to-apply reference.
- No performance result is accepted until the isolated swarm fixture and
  paired-run orchestration establish the same workload for each candidate.

### Files and compatibility

- **Expected code paths:** network control helper, `network-control.schema.json`,
  trial environment fields, manifest validator, harness README, measurement
  contract, and focused regressions.
- **State or wire compatibility:** versioned certification artifacts only;
  existing unshaped trial records remain valid.
- **Migration / rollback:** every apply has a distinct reset report; no
  production interface is changed by this helper.
- **Feature flag or rollout boundary:** network-control observations remain
  optional until a paired fixture claims a profile; no profile is a product
  runtime setting.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_network_control.py` and
  `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** network-control and validator regressions during
  the requested bulk pass; do not run `tc` on the desktop/server host as part of
  implementation.
- **Expected result:** apply/reset refuses unrelated qdiscs, records only
  verified settings, and manifest validation rejects unlinked or mismatched
  profile evidence.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending isolated-network qualification.
- **External evidence still required:** task-owned namespace lifecycle,
  two-ended control, independent swarm readiness, and repeatability on the
  qualification kernel/iproute2 matrix.

## PERF-107 — Independent network, hash, disk, and pipeline ceilings

- **Priority / wave:** P0 / 5
- **Prerequisites:** PERF-102, PERF-105, PERF-106
- **Owning component:** `scripts/transfer_rebuild/ceilings.py` and the
  versioned calibration/report schemas.
- **Finding or capability:** estimate the workload's independent service
  ceilings so a slow client run can be attributed to fixture limits, an
  undersized request pipeline, storage, hashing, or the client path.
- **Status:** `wip`

### Scope

Accept independently collected network, disk, SHA-1/SHA-256, and peer RTT
observations; record the exact host, filesystem/device, network profile, tool
identity, and raw evidence digests; then derive protocol-aware download and
upload bounds. The pipeline estimate is `peers × requests per peer × block
bytes / measured RTT`. Download is bounded by receive capacity, disk writes,
the required protocol hash rate, and the request pipeline. Upload is bounded
by transmit capacity, disk reads, and the request pipeline.

The output is a diagnostic model, not a candidate measurement or a performance
claim. Missing or invalid inputs fail closed rather than becoming zero or an
infinite ceiling. The model does not decide scorecard thresholds, normalize
unrelated hosts, or replace the matched whole-client trials.

### Ownership and invariant

- **Current owner:** `ceilings.py` validates the calibration input and emits an
  immutable derived report; manifest validation binds both to trial records.
- **Target owner:** PERF-110's paired harness requests a ceiling report for the
  same host, storage device, network profile, and protocol before trials run.
- **Invariant preserved:** every derived rate is traceable to independent raw
  artifacts and the exact tool/version; unknown resources remain unknown.
- **Must not:** derive calibration from the candidate's measured transfer,
  call a modeled ceiling observed client throughput, or treat a missing hash
  or RTT result as unlimited capacity.

### Acceptance

- Versioned input and output schemas reject missing identities, invalid rates,
  unsupported protocol values, malformed digests, and unmeasured hybrid hash
  capacity. Each probe records its PID and process-start ticks so the manifest
  can reject calibration collected inside the candidate process.
- The model uses SHA-1 for v1, SHA-256 for v2, and an independently measured
  combined hash rate for hybrid.
- Network receive/transmit and storage read/write ceilings retain separate
  directions; pipeline capacity uses measured RTT and explicit peer/request/
  block geometry. Its client, version, settings digest, and JSON pointer bind
  request depth to the process-bound settings readback; trial peer and block
  counts must match the calibration.
- The report records every component rate, exact limiting components, and a
  fixed diagnostic-only interpretation.
- Input and output are immutable artifacts; manifest validation checks all raw
  artifact digests and recomputes the report from the linked calibration.
- Trial linkage requires matching host, OS, kernel, CPU, filesystem, device,
  network profile, and protocol. It rejects calibration process identities
  that match the measured client.
- Regression sources cover hash/protocol selection, each direction's
  bottleneck, pipeline math, invalid values, symlinks, immutable writes, raw
  artifact linkage, report recomputation, and candidate-process reuse.

### Files and compatibility

- **Expected code paths:** `ceilings.py`, the ceiling input/report schemas,
  `validate_artifact.py`, trial schema, harness README, and focused regression
  sources.
- **State or wire compatibility:** new `transfer-ceiling-input-v1` and
  `transfer-ceiling-v1` certification artifacts; optional trial linkage only.
- **Migration / rollback:** existing trial records remain valid without a
  ceiling digest; no product runtime behavior changes.
- **Feature flag or rollout boundary:** PERF-110 may use the model only as
  diagnostic context; scorecard decisions remain gated by PERF-113.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_ceilings.py` and linked-artifact cases in
  `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** ceiling and validator regressions during the
  requested bulk pass.
- **Expected result:** complete, identity-matched independent calibrations
  produce deterministic bounds; missing or conflicting evidence is rejected.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending calibration and bulk pass.
- **External evidence still required:** repeated network, disk, hash, and RTT
  calibration on the dedicated comparison host.
