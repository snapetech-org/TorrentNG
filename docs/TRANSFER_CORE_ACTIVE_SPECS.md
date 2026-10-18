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
portable scheduler. This slice covers cancellation after backend submission,
injected read/write failures, partial-write failure, retry after a failed data
barrier, and operation-level file identity fencing. Dirty path state binds each
write generation to the descriptor identity that received it. On Unix and
Windows, write and sync operations check that their descriptor still matches
the current path at their completion boundary; a mismatch fails as uncertain
and leaves dirty evidence intact. Full operation-lease accounting, restart
cuts, and platform fault qualification remain open.

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
- Regression sources cover strict-write sync failure and a canceled
  multi-path checkpoint waiter whose held sync returns an injected error; a
  later write waits for the barrier to release and dirty evidence remains
  retryable. Platform-specific storage-failure qualification remains open.
- The positioned-I/O helpers retry `Interrupted` and positive short progress;
  regression sources simulate short reads and writes across multiple calls.
- Path-replacement regressions cover secure open, cached descriptor reopening,
  scheduler snapshots, and peer-read readahead (IO-004), plus held payload
  writes and checkpoint syncs. Dirty writes remain tied to the original file
  identity, and replacement during an operation returns
  `FilesystemStateUncertain` without clearing dirty evidence. Full
  operation-lease accounting, crash cuts, and platform-specific fault
  qualification remain open.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/backend.rs`,
  `scheduler.rs`, test-only `test_support.rs`, and
  `certification/transfer-rebuild/storage-evidence-contract.md`.
- **State or wire compatibility:** no public storage or resume format change.
- **Migration / rollback:** none in this test-only slice.
- **Feature flag or rollout boundary:** injected backend is `cfg(test)` only.

### Evidence

- **Regression source added or updated:**
  `injected_read_failure_is_returned_without_marking_payload_dirty`,
  `scripted_zero_progress_read_and_write_are_reported_as_short_io`,
  `pre_submission_open_failures_release_storage_admission`,
  `injected_write_failure_is_returned_and_remains_dirty_until_barrier`,
  `strict_write_sync_failure_releases_admission_and_preserves_dirty_evidence`,
  `exact_read_retries_interrupted_and_positive_short_progress`,
  `write_all_retries_interrupted_and_positive_short_progress`,
  `cancelled_waiter_keeps_submitted_write_dirty_until_barrier`,
  `cancelled_strict_write_retains_admission_until_sync_completion`,
  `partial_write_then_error_is_not_reported_as_success`,
  `failed_barrier_keeps_dirty_evidence_for_retry`,
  `cancelled_checkpoint_waiter_keeps_dirty_evidence_after_sync_failure`, and
  `dropping_backend_releases_and_joins_held_completion_workers`.
- **Command or probe to run:** `cargo test -p rt-storage` during the requested
  bulk pass.
- **Expected result:** injected read/write failures are returned; failed reads
  do not dirty payload state, failed writes remain dirty until a barrier,
  cancellation retains dirty ownership through a barrier, partial failure
  remains an error, and a failed barrier preserves dirty state for retry.
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
  reservation into storage, which retains the same lease through backend
  completion without charging a shared payload a second time.
- **Invariant preserved:** dropping a stale, rejected, cancelled, or completed
  event releases its reservation only after its bytes stop being retained.
- **Must not:** release a webseed body reservation when fetch returns, or lose
  the piece-assembly charge while an eviction slice still references its
  backing allocation.

### Acceptance

- Peer and webseed events retain their lease through actor handling.
- Leased block writes use immutable `Bytes` slices and keep the original
  payload lease through cancellation and backend finish; they add only the
  per-job queue overhead charge.
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
  request-to-event ownership regression and a shared-payload storage
  cancellation regression.
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

Centralize path normalization, platform file identity, and replacement
detection in `open.rs`. `HandleCache` is the sole descriptor-cache
implementation; the scheduler's `FilePool` is a narrow stats/open-policy
adapter. `StorageRuntime` and scheduler resources keep separate maps so their
existing resource identities and accounting remain valid, while using the
same secure identity helpers and descriptor-lease implementation.

### Ownership and invariant

- **Current owner:** `HandleCache` owns secure normalized path keys, identity
  checks, descriptor leases, and cache lifecycle. `FilePool` delegates to it;
  `StorageRuntime` and scheduler resources retain separate maps, while
  `open.rs` owns no-follow opens and platform identity operations.
- **Policy boundary:** each scheduler resource key and the direct runtime keep
  their existing cache identity and capacity; all instances remain subject to
  the shared process descriptor gate. A process-wide registry across those
  owners is deferred until stats can identify and deduplicate cache ownership.
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
- Cache-hit validation uses the stored descriptor identity plus one path
  identity lookup; it does not `fstat` the cached descriptor on every hit.
- Reads can reuse a read+write descriptor, while read-only descriptors never
  serve writes.
- The LRU index shares each path allocation and expiry checks only the stale
  oldest prefix, avoiding a path-string clone and full-map scan on every hit.
- Replacement, unlink/recreate, descriptor-budget, and in-flight-lease
  regressions continue to cover the catalog lifecycle; a zero idle TTL keeps
  expiry disabled.
- The public `HandleCache` reuses a current read+write descriptor for reads,
  never reuses read-only descriptors for writes, and removes a cached
  read-only entry when a read+write entry is published.
- A process-wide map spanning scheduler and direct-runtime owners remains
  open; merging it now would require separate cache identity in scheduler
  metrics to avoid double-counting shared gauges and counters.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/open.rs`,
  `scheduler.rs`, `handle_cache.rs`, and `file_handle.rs`.
- **State or wire compatibility:** no storage-path, resume, or wire-format
  change.
- **Migration / rollback:** none; path opening remains no-follow.
- **Feature flag or rollout boundary:** none; qualify before broader storage
  service integration.

### Evidence

- **Regression source added or updated:**
  `read_only_handles_do_not_serve_writes_and_reads_reuse_writable_handle`,
  `reopens_path_after_unlink_and_recreate`, `zero_idle_ttl_disables_expiry`,
  `write_handle_snapshot_drops_replaced_paths`, plus existing descriptor
  budget, in-flight lease, and scheduler cache-stat regressions.
- **Command or probe to run:** `cargo test -p rt-storage` during the requested
  bulk pass.
- **Expected result:** scheduler and runtime use one cache implementation and
  identity policy; replaced files are reopened; identity checks do not hold
  the cache lock during filesystem access; reads reuse writable handles; and
  live leases continue to bound descriptors.
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
- Pre-submission open failures and injected read, write, partial-write, and
  barrier failures assert that queued bytes and device slots are returned. A
  strict-write sync failure also returns admission while preserving dirty
  evidence for a later successful barrier.
- End-to-end scheduler regressions cancel submitted writes while completion is
  held, prove queue slots and queued-disk memory leases stay charged, verify
  later admission is rejected at each limit, and prove admission returns after
  backend completion. A strict-mode case cancels during the write's sync,
  retains dirty evidence, and recovers admission only after sync completion.
- Canceling a checkpoint barrier while its per-file sync is held retains the
  device and queued-byte admission until completion; because the canceled
  caller cannot consume barrier evidence, dirty state remains for a later
  successful barrier.
- The full fault-injected device-service matrix remains open.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`,
  `backend.rs`, `test_support.rs`, and IO-006's device service.
- **State or wire compatibility:** no resume, storage-path, or wire-format
  change.
- **Migration / rollback:** none; the portable backend remains selectable.
- **Feature flag or rollout boundary:** no behavior flag; storage service
  rollout remains later work.

### Evidence

- **Regression source added or updated:**
  `cancelled_submitted_write_retains_queue_slot_until_completion`,
  `cancelled_submitted_write_retains_memory_budget_until_completion`,
  `cancelled_submitted_readahead_read_retains_extra_bytes_until_completion`,
  `cancelled_strict_write_retains_admission_until_sync_completion`,
  `cancelled_checkpoint_barrier_retains_admission_until_sync_completion`,
  `strict_write_sync_failure_releases_admission_and_preserves_dirty_evidence`,
  `cancelled_submitted_read_retains_admission_until_failure_completion`,
  plus helper-level completion-owner lifetime cases for canceled read/write
  waiters and admission-release assertions for open and injected I/O failures.
  Canceled submitted read and write cases now receive delayed backend errors;
  the strict-write cancellation case does the same for sync after the submitted
  sync completes. These prove canceled callers retain admission until
  completion while failed results leave dirty evidence for retry where needed.
- **Command or probe to run:** `cargo test -p rt-storage` and the scripted
  backend cancellation matrix during the requested bulk pass.
- **Expected result:** accepted work holds its queue/device/byte owner until
  backend completion; queue-slot and memory-budget admission recover after
  completion, including strict-write sync and the extra charge for an expanded
  peer read, with no leak after pre-submission failure.
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
storage-root-scoped fallback. The storage-job dispatcher keeps one stable
fallback root identity across its copy schedulers, so unsupported platforms
do not create a fresh, unshared queue for every plan. The queue capacity comes
from the shared storage I/O depth, while active dispatch uses one slot per
portable worker or a continuously refilled 64-operation io_uring window.
Queued work uses the `DeviceElevator` class weights with weighted round-robin
selection across mounts. A unique operation ID and permit travel through the
existing completion owner. Cache hits and file-open preparation happen before
device admission so they do not occupy a physical-device slot.

The live service currently covers scheduler positioned reads/writes/syncs,
peer-read batches, preallocation, extent probes, and secure storage-plan
copy/import steps. Checkpoint barriers group ready file fences by resolved
device and dispatch bounded batches through that device's queue, preserving a
separate completion watermark for each path. Plan steps enter the destination
mount's `MoveCopy` lane; source and destination paths each contribute
physical-device admission, with all touched device queues acquired in stable
key order. The 64 KiB streaming buffer is charged once while the operation
owns mount/device admission. NET-114 still owns peer deadlines and
choke-critical priorities. Cross-mount read coalescing and full
injected-backend accounting remain open.

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
  non-scheduler consumers. `run_device_operation_with_paths` admits every
  touched physical device in stable key order; secure copy/import steps use
  this path so reverse-direction copies cannot deadlock. Peer
  deadlines/choke-critical promotion remain in NET-114. Checkpoint barriers
  resolve path groups before admission and dispatch a bounded same-device
  batch while preserving per-path fences and operation identities.
- Canceling a caller after an external handoff starts does not release its
  queue slot, physical-device permit, or queued-byte lease before the blocking
  closure completes; another mount sharing the device remains backpressured
  until that completion.
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
  schedulers; `cancelled_external_handoff_retains_admission_until_completion`
  checks that a canceled external caller retains the shared device permit and
  queued-byte lease until the blocking closure returns an injected error;
  `multi_device_handoff_holds_source_and_destination_until_completion` checks
  both physical-device permits remain held through external work. These sources
  are not run.
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
runs as one hash-pool job. Hybrid metadata now enters the v1 actor with a
compact v2 projection containing aligned file ranges and authenticated piece
layers. The in-memory completion path computes SHA-1, the file root, and the
piece-layer root from one immutable piece buffer and one bounded pool job; the
disk fallback streams each file range once and checks both roots while keeping
BEP 47 padding only in the v1 view. Disk recheck now routes through the same
hybrid-aware verifier instead of directly invoking the v1-only verifier. The
projection maps v2 files back to v1 file indexes because synthetic v1 pad
files shift those indexes.

### Ownership and invariant

- **Current owner:** the v1 task owns hybrid piece/file geometry and its
  mutable assembly lease. Once complete, the task freezes that allocation
  into immutable `Bytes` whose owner carries the lease, shared by hashing,
  spill blocks, and file writes; storage queues retain each view through
  backend completion and charge only per-job overhead in addition to the
  existing payload reservation.
- **Target owner:** NET-114 moves the same content span and reservation into
  the shared transfer/storage boundary without changing v1/v2 root semantics.
- **Invariant preserved:** queue rejection cannot consume the caller's only
  hash state or release the payload lease early; v1 and v2 views refer to the
  same verified content while preserving their distinct padding rules. A
  completed in-memory piece has one payload allocation until its verification
  and aggregate writes finish.
- **Must not:** retain one leaf hash per byte range for a whole large file or
  duplicate payload buffers just to change hash geometry.

### Acceptance

- Hash workers remain bounded by the shared queue and owned resource limits.
- Large-file v2 verification batches leaf hashes and folds the root off the
  async executor without retaining all leaf hashes.
- Hybrid memory and disk paths check both v1 and authenticated v2 roots;
  regressions cover v1 padding, a short final piece, the one-piece file-root
  shape, v1/v2 index differences, a v2-only expected-root mismatch, and
  zero-copy freezing/slicing of the completed piece allocation. Full disk
  recheck must also reject a v1-valid piece whose authenticated v2 root differs.
- Hash-pool retries retain the immutable content and one queued-byte
  reservation across saturation; shared payload jobs carry the existing
  allocation lease and charge queue overhead once. A canceled queued closure
  retains both until dequeue, while the active closure owns them through
  finish.

### Evidence

- **Regression source added or updated:**
  `rt-storage::verify::v2_file_verify_batches_leaf_hashing_per_chunk` now
  asserts three leaf batches plus one root job; hybrid scheduler, streaming
  verifier, memory-completion, and compact metadata-projection regressions now
  cover shared payload roots, padding, short tails, v2-only mismatch, and the
  v1 index shift after a pad file. `completed_piece_freezes_and_reuses_its_assembly_allocation`
  checks pointer identity and confirms completed spill views do not allocate
  peer-buffer copies; `completed_payload_views_keep_piece_lease_after_assembly_is_dropped`
  verifies a queued immutable view keeps the original allocation charged
  after the actor-side assembly is gone.
  `cancelled_queued_hybrid_hash_retains_shared_payload_lease_until_dequeued`
  covers waiter cancellation while the accepted hash closure remains queued;
  the task-level hybrid completion/recheck regression verifies that a v1-valid
  disk piece with a mismatched v2 root is rejected. Sources remain unrun.
- **Command or probe to run:** the focused storage verifier, scheduler, and
  engine regressions in the deferred bulk pass.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** none for the bounded hash contract;
  whole-client hash throughput and CPU remain PERF-107/110/113 evidence.

## IO-008 — Completion-driven transfer actors

- **Priority / wave:** P0 / 8
- **Prerequisites:** IO-002, IO-006, IO-007, NET-104, NET-106
- **Owning component:** v1/hybrid `TorrentTask` and pure-v2 torrent actor.
- **Finding or capability:** peer progress must continue while bounded storage
  writes and piece verification complete; only the actor may apply results to
  picker, completion, and resume state.
- **Status:** `wip`

### Scope

The authored v1/hybrid slice moves direct received-block writes, completion
verification, and aggregate writes into a bounded actor-owned job set. Picker
and progress state change only when the actor applies the typed completion.
Pure-v2 moves download-rate admission, direct block writes, complete-piece
storage reads, Merkle hashing, and aggregate writes into a bounded per-peer
job set; a piece stays unavailable until its completion is applied. Existing
request and assembly memory leases remain alive through their storage/hash
consumers.

This is still partial: failure/reset and actor-level lifecycle qualification
remain open. v1/hybrid block, verification, and aggregate-write jobs carry
monotonically increasing piece-attempt generations, and stale completions
cannot mutate a retry. Pure-v2 assemblies plus block and piece completions use
per-peer monotonically increasing generations with the same stale-result
fencing. v1 lifecycle snapshots drain accepted work before shutdown, pause,
recheck, and storage-move snapshots. Pure-v2 peer shutdown
uses a per-peer lifecycle generation to stop new request and upload work,
cancels download-rate and upload-rate waits, and drains dispatched upload
reads plus already-admitted storage and verification jobs before the torrent
task finishes joining peers. Queued upload requests are discarded at shutdown.
The same signal preempts framed sends and hash-proof waits owned by the peer
loop, so a blocked socket writer does not prevent the drain; actor-level
command-loop, disconnect/quiesce regressions and bulk validation remain
deferred.

### Ownership and invariant

- **Current owner:** each torrent actor owns its picker and transfer state;
  v1/hybrid storage and verification results are actor-applied, while pure-v2
  completed-piece results are applied by the peer loop that owns that peer.
- **Target owner:** bounded job sets own storage/hash futures and payload
  leases until completion; each actor remains the sole writer of its piece,
  peer, lifecycle, and durability state.
- **Invariant preserved:** write completion precedes verification; a piece
  remains pending until its integrity result is applied; accepted work is
  serialized with piece reset and drained before v1 quiesce, move, and clean
  shutdown snapshots. Pure-v2 drains peer-owned work on protocol exit and
  orderly torrent shutdown. Both implementations fence completions by
  piece-attempt generation; the broader failure/reset matrix remains open.
- **Must not:** spawn untracked work, drop a payload lease before backend
  completion, advertise or persist unverified bytes as complete, or let an old
  job commit into a replacement attempt.

### Acceptance

- Active peer controls and lifecycle commands continue to progress while a
  storage write or hash job is held.
- Per-torrent active work and deferred input have explicit bounds; saturation
  returns request ownership for retry without losing a valid piece.
- Direct-write and in-memory assembly paths both apply storage/hash results
  through the actor; hybrid pieces still verify both roots from shared bytes.
- Failure, cancellation, retry, piece reset, pause, storage move, and shutdown
  preserve picker accounting and memory leases through late backend completion.
- Pure-v2 shutdown preempts a pending framed send and hash-proof wait, then
  stops new per-peer work, cancels upload-rate waits, and drains dispatched
  upload reads plus already-admitted writes and verification before releasing
  peer state.
- A piece becomes locally available only after verification and, for the
  aggregate path, successful payload write; fastresume barriers cover all
  accepted writes that the snapshot claims.

### Files and compatibility

- **Expected code paths:** `torrent_task.rs`,
  `torrent_task/peer_transfer.rs`, `torrent_task_v2.rs`,
  `rt-piece-picker`, and the storage completion ownership helpers.
- **State or wire compatibility:** no wire-format change; resume state keeps
  the existing format and only records actor-applied piece results.
- **Migration / rollback:** no runtime flag or persistent-format change has
  been added. The old synchronous helpers remain available for recheck and
  other non-peer paths; reverting the actor job routing remains a source
  rollback until v1/hybrid and pure-v2 failure cases, actor-level lifecycle
  cases, and bulk evidence are reviewed.
- **Feature flag or rollout boundary:** no separate flag; this is the
  production actor completion contract required before NET-114 integration.

### Evidence

- **Regression source added or updated:** `pending_verification_does_not_publish_piece_as_have`
  checks picker state stays pending until verification is applied;
  `failed_block_write_releases_picker_request`,
  `hash_failure_does_not_persist_piece_as_valid`, and
  `restored_partial_piece_writes_new_blocks_directly` cover v1 failure and
  retry behavior; `v2_piece_is_not_available_until_completion_job_finishes`
  checks a pure-v2 piece remains unavailable and unwritten while its completion
  job is queued, then publishes after successful hash and write. Existing
  v2 multi-block and request-lease sources were adapted to drain the job;
  `v2_peer_reads_control_messages_while_transfer_jobs_wait_for_rate_budget`
  covers upload and download budget waits while the peer loop handles controls,
  then cancels a blocked download-rate wait and confirms shutdown drains without
  publishing the canceled piece. `v2_peer_shutdown_cancels_a_backpressured_socket_send`
  covers a peer writer blocked by a non-reading socket. These sources are
  authored but unrun; failure/reset cases, actor-level disconnect/quiesce, and
  broader lifecycle qualification remain open. The authored but unrun
  `v2_peer_shutdown_drains_admitted_upload_reads` source holds a dispatched
  upload future and verifies shutdown drain waits for it to finish.
  `v2_torrent_shutdown_cancels_and_joins_peer_tasks` exercises the torrent
  owner’s cancellation-and-join path; command-loop, disconnect/quiesce, and
  broader lifecycle qualification remain open.
  `stale_piece_attempt_completions_cannot_mutate_a_retry` covers v1/hybrid
  block-write and verification completions from a replaced attempt, while
  `v2_stale_piece_attempt_completions_cannot_mutate_a_retry` covers pure-v2
  block-write and verification results from a replaced attempt.
- **Command or probe to run:** focused actor and picker regressions during the
  requested bulk pass.
- **Expected result:** actors process controls while accepted transfer work is
  pending and publish only verified, fully written pieces.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** saturated mixed-peer completion
  latency and resource bounds.

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
  drop; `v2_request_admission_retries_after_capacity_returns_and_owns_each_block`
  checks v2 no-send under denial, retry, per-block ownership through full Piece
  storage/verification, and remaining-request disconnect release;
  `v2_choke_preserves_fast_request_obligations`
  checks the request lease survives Fast Choke and releases on legacy Choke;
  `v2_reject_releases_only_matching_response_lease` checks matching Reject
  release and mismatched Reject retention.
  These v2 regressions are authored but remain unrun; hybrid/shared-span
  admission remains open.
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

The `Ledger` owns per-piece block state and per-connection reservations,
reconciles response/commit event reordering, accepts useful late data, and
returns bounded protocol actions. A conservative 64 MiB logical piece-state
budget, a 4,096-live-connection ceiling, a 1,024-request per-peer ceiling, a
65,536-owner torrent ceiling, and a four-owner endgame cap keep its principal
retained collections bounded. NET-114 now uses it for live v1, hybrid-v1, and
pure-v2 outbound request ownership and received-block arbitration. Pure-v2
still keeps protocol-local response leases and per-peer piece assemblies; its
reservations use piece affinity because those assemblies are not shared.

### Ownership and invariant

- **Current owner:** the ledger owns live v1/hybrid-v1 and pure-v2 outbound
  reservations, connection generations, received-block arbitration,
  fastresume-restored partial blocks, and applicable cancellation actions.
  `PiecePicker` remains the candidate-ranking and resume projection. Pure-v2
  actor maps retain response leases and per-peer assembly state, with atomic
  piece-affinity reservation.
- **Target owner:** NET-114 extends the same ownership contract to hybrid
  dual-view transfers and storage completion, with storage retaining payload
  bytes until verification commits.
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
- Piece-state admission and release preserve the configured logical byte
  budget; live connections, peer request depth, torrent-wide in-flight
  ownership, and endgame duplicate owners have explicit hard ceilings.
- Fastresume partial blocks seed the ledger's received slots under the same
  PeerBuffer charge as live piece state; a complete partial bitmap is discarded
  and can never mark a piece available without verification.
- A useful tombstoned response may supersede a replacement request and
  returns cancellation actions for the replacement owners.
- Completion, hash failure, choke, reject, timeout, disconnect, resume-have,
  partial resume, and epoch advance have regression sources.
- Seeded simulation replays churn, loss, late replies, and hash failures with
  an invariant check after every event.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/{ledger,sim}.rs`,
  `crates/rt-engine/src/torrent_task/peer_transfer.rs`, and
  `crates/rt-engine/src/torrent_task/peer_session.rs`.
- **State or wire compatibility:** the ledger is in-memory only; no wire or
  fastresume format changed. The live request adapter is staged per protocol
  plane.
- **Migration / rollback:** the picker remains the selection/resume projection;
  reverting the actor adapter returns request ownership to the prior actor path.
- **Feature flag or rollout boundary:** v1, hybrid-v1, and pure-v2 outbound
  request ownership, including pure-v2 timeout expiry, are wired but
  unqualified; hybrid dual-view integration, the simulator, and the bulk pass
  remain open before the architecture is accepted.

### Evidence

- **Regression source added or updated:** ledger transition cases and seeded
  simulator replay; current additions include bounded commit history, late
  data superseding a replacement reservation, byte-budget lifecycle, bounded
  connection registration, and request/endgame caps.
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
complete obligation state machine into the live shared runtime. The
`transfer::InboundObligations` module owns the v1 and pure-v2 inbound upload
queue/service lifecycle. The generation-aware ledger also owns live
v1/hybrid-v1 and pure-v2 outbound reservations, received-block arbitration,
and pure-v2's 60-second request expiry; hybrid dual-view ownership remains
open.

### Ownership and invariant

- **Current owner:** v1 and pure-v2 inbound upload queue, in-service, Cancel,
  choke, and completion transitions use `InboundObligations`; the peer loops
  own storage futures and wire writes. The ledger owns v1/hybrid-v1 and
  pure-v2 outbound reservation and response arbitration, including v2 timeout
  expiry; hybrid dual-view ownership remains open.
- **Target owner:** `InboundObligations` and the shared ledger own response
  obligations and outbound request state across both protocol planes.
- **Invariant preserved:** when Fast is negotiated, a request already
  accepted for processing remains owed a Piece or Reject response across
  Choke and Cancel; Reject is sent only when the peer negotiated Fast.
- **Bound preserved:** the per-connection cap covers queued and in-service
  requests together. Duplicate coordinates are recognized before capacity
  refusal, so saturation cannot turn a duplicate into a second response.
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
- Both live upload adapters apply the shared total-work bound and suppress
  duplicates before checking capacity. Legacy Choke ends in-service
  obligations without sending Reject; Fast Choke rejects queued requests
  except pieces explicitly in the local allowed-fast set.
- Once a v1 block is accepted, the actor queues endgame Cancel for peers with
  duplicate requests. A peer loop sends it only if that request reached the
  wire and keeps the response lease until its terminal event.
- Request cancellation and endgame loser handling reconcile exactly once with
  queued reads, in-flight work, Piece delivery, and Reject delivery.
- Stale, duplicate, and unsolicited responses cannot mutate current content.
- Both v1 and pure-v2 inbound upload paths use the shared obligation owner.
  v1/hybrid-v1 and pure-v2 outbound ownership now uses the ledger, including
  v2 timeout expiry; hybrid dual-view arbitration and bulk qualification must
  still close before this ticket is done.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/obligations.rs`,
  `torrent_task.rs`, `torrent_task_v2.rs`, and NET-114 peer-runtime adapters.
- **State or wire compatibility:** no wire-format change; negotiated Fast
  behavior follows the peer's handshake capabilities.
- **Migration / rollback:** no persisted state changes. The inbound owner is
  per connection and is cleared on disconnect; the v1/v2 adapters remain
  independently reversible while hybrid dual-view and shared-runtime parity
  are unfinished.
- **Feature flag or rollout boundary:** no new advertisement or capability
  until the applicable live path satisfies its obligations.

### Evidence

- **Regression source added or updated:**
  `v2_reject_obligations_require_fast_negotiation`,
  `v2_choke_preserves_fast_request_obligations`,
  `interrupted_in_service_request_can_be_requeued_without_losing_its_obligation`,
  `fast_choke_keeps_wire_request_lease_and_legacy_choke_releases_it`,
  `fast_choke_cancels_queued_requests_but_keeps_wire_requests`, and the
  existing v1 `fast_peer_request_while_choked_is_rejected_not_dropped`.
- Focused v1 regression sources now cover queued-versus-in-service Cancel,
  late Piece after outbound Cancel, and exact endgame duplicate matching.
  Shared-ledger outbound ownership, hybrid parity, public-client
  interoperability, and all bulk validation remain open. Added shared-owner
  sources cover duplicate requests at capacity, total queued-plus-in-service
  bounds, allowed-fast dispatch while choked, and legacy in-service cleanup.
- **Command or probe to run:** targeted v1/v2 Fast lifecycle and shared-ledger
  simulations during the requested bulk pass.
- **Expected result:** no owed request is lost across Fast Choke/Cancel and
  each negotiated request reaches one terminal response.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** public-client interoperability.

## NET-106 — Independent peer progress during upload work

- **Priority / wave:** P1 / 7
- **Prerequisites:** NET-103, NET-104, NET-105
- **Owning component:** v1 and pure-v2 peer protocol loops.
- **Finding or capability:** keep protocol input and control handling responsive
  while bounded upload work waits on storage or rate admission.
- **Status:** `wip`

### Scope

Move peer-requested upload reads and rate-budget waits into bounded per-peer
work. The owning protocol loop continues to read and process incoming control
messages while those jobs wait. Socket writes remain serialized by the
connection owner; actor-side received-block writes, verification, and
progress persistence remain IO-008.

### Ownership and invariant

- **Current owner:** each protocol loop owns its socket and upload response
  ordering; bounded worker futures own storage payloads and leases until the
  loop sends or rejects the response.
- **Target owner:** NET-110/114 move admission and request obligations into the
  shared upload service while preserving a single socket writer per peer.
- **Invariant preserved:** each peer has bounded active upload reads and a
  bounded pending request queue; storage/rate waits never hold the protocol
  loop's message-polling future; choke or shutdown wakes rate waiters and
  invalidates stale responses before they are sent.
- **Must not:** release a payload lease while storage completion still owns
  the read, or send a block after its upload slot was revoked.

### Acceptance

- v1 and pure-v2 continue to process incoming interest/control messages while
  an upload read or rate-budget wait is pending.
- Active read count and pending request count have explicit per-peer bounds.
- Choke, slot revocation, cancellation, and shutdown release or reject work
  according to negotiated Fast semantics.
- A deterministic regression holds upload rate admission after the storage
  read completes, sends a control message, and observes that message before
  the budget is released.
- NET-114 later adopts the shared upload service without adding a second
  socket writer or bypassing its ownership accounting.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/torrent_task.rs`,
  `torrent_task_v2.rs`, and the NET-114 peer adapters.
- **State or wire compatibility:** no wire-format change; only scheduling
  between already bounded local jobs changes.
- **Migration / rollback:** retain the per-actor worker path until shared
  upload-service parity and rollback coverage exist.
- **Feature flag or rollout boundary:** none; the shared core remains inactive
  until NET-114.

### Evidence

- **Regression source added or updated:**
  `peer_loop_keeps_reading_while_upload_waits_for_rate_budget` and
  `v2_peer_reads_control_messages_while_upload_waits_for_rate_budget`.
- **Command or probe to run:** targeted v1/v2 peer-loop regressions during the
  requested bulk pass.
- **Expected result:** each loop processes control input while its upload
  worker remains rate-limited, and shutdown releases all worker-owned state.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** mixed-speed peer control latency and
  bounded resource use under a saturated upload queue.

## NET-107 — Hierarchical fair payload rate scheduler

- **Priority / wave:** P1 / 8
- **Prerequisites:** NET-103, NET-104, NET-106
- **Owning component:** pure scheduler in `crates/rt-engine/src/transfer/rate.rs`,
  then the process-wide network budget and live peer adapters.
- **Finding or capability:** remove copied per-peer torrent buckets, avoid
  timer polling, and share configured limits fairly across live torrents and
  peers.
- **Status:** `wip`

### Scope

Use one deterministic `RateTree` to admit payload bytes under process-wide and
per-torrent limits, with weighted per-peer fairness. The core accepts an
injected monotonic timestamp, keeps
fractional byte-nanosecond credit, and may carry debt so a configured rate
below one block per second still makes progress. Wait results identify the
first useful retry deadline, and a bounded due-peer query drains wake work
without scanning every connection; weighted start-time fairness limits how
far one peer runs ahead of waiting peers in its torrent. The engine owns
wall-clock timers, limit-change notifications, cancellation, and socket or
storage work.

Deadlines describe the last grant attempt. A limit change or refund can make
parked requests eligible sooner, so the live adapter must promptly reconsider
the affected requests instead of waiting for their previous deadlines.

`TorrentKey` is unique for a task generation while stale peer work may still
exist. `TransferPeerId` combines that key with the connection-local generation-safe
`ConnectionId`, because separate torrent ledgers can both register `conn#0.1`.
NET-114 and NET-115 own actor registration, payload admission, limit updates,
cancellation/refund, and shared source scheduling.

### Ownership and invariant

- **Current owner:** `GlobalNetworkBudget` and torrent/peer actors hold the
  existing process and torrent limit state; the pure `RateTree` is not yet
  wired to live peers.
- **Target owner:** one process-shared tree owns global and per-torrent byte
  credit, peer fairness progress, unsettled grant/refund accounting, and the
  next-wakeup index.
- **Invariant preserved:** a grant debits all applicable levels atomically;
  a denial does not partially consume torrent or peer service. A rate update
  clamps existing credit and cannot mint another burst.
- **Must not:** key process-wide state by a `ConnectionId` alone, scan every
  peer on each grant/timer pass, or hold the tree lock while waiting or doing
  I/O.

### Acceptance

- Rate peer identity remains distinct when two torrent ledgers reuse the same
  connection slot and generation; removing one torrent leaves the other's
  rate state intact.
- Global and torrent limits are shared across peers and grants debit every
  limited level together or none.
- Fractional refill and debt make sub-block-per-second limits progress, with
  whole-nanosecond retry deadlines and no floating-point drift.
- Weighted peers share a saturated torrent allowance; a new peer starts at
  the current virtual floor rather than at zero.
- Updating a peer's weight preserves its fairness progress and unsettled
  grant accounting.
- Re-registering or updating a torrent limit preserves accumulated debt and
  never mints a burst; unlimited-to-limited starts without accumulated credit.
- Refunds restore only granted bytes that were not settled as transferred
  payload, and clamp at bucket capacity.
- Ordered per-torrent active/wait indexes and the global wake index stay
  coherent across grant, wait, refund, peer removal, and torrent removal.
- The due-peer query returns only indexed peers at or before the supplied
  deadline, respects its caller-provided batch bound, and leaves entries
  indexed until a grant attempt reconciles them.
- Limit changes and refunds promptly reconsider affected parked requests, so
  an earlier eligibility time is not hidden by a stale timer deadline.
- Live v1, pure-v2, and webseed adapters use the same service before the
  scheduler is considered integrated; request admission remains bounded by
  memory and device capacity.

### Files and compatibility

- **Expected code paths:** `transfer/rate.rs`, `network_budget.rs`,
  `torrent_task/peer_transfer.rs`, `torrent_task/peer_session.rs`,
  `torrent_task_v2.rs`, and NET-114/115 integration.
- **State or wire compatibility:** no wire or persisted-state change; rates
  continue to count payload bytes at the engine boundary.
- **Migration / rollback:** retain the existing actor limiters until the
  shared service has matched behavior and reversible actor coverage.
- **Feature flag or rollout boundary:** the pure module remains inactive in
  production until live adapters and bulk regressions are complete.

### Evidence

- **Regression sources:** exact deadlines, sub-block progress, shared
  global/torrent budgets, all-or-nothing grants, weighted fairness, no-burst
  limit updates, unsettled-grant refunds, settled-payload protection, peer
  weight updates, index
  cleanup, deadline-filtered bounded wake batches, unknown peers, repeated
  torrent-limit updates, and
  `reused_ledger_connections_are_isolated_between_torrents`.
- **Command or probe to run:** pure scheduler regressions and the engine rate
  integration suites during the requested bulk pass.
- **Expected result:** deterministic grants conserve each configured budget,
  return useful wake deadlines, and keep peer identities separate across
  torrent actors.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending the bulk pass.
- **External evidence still required:** live mixed-torrent fairness and
  matched whole-client CPU, memory, and goodput measurements.

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

The pure `UploadService` queues protocol obligations per torrent-scoped peer,
dispatches bounded byte-admitted reads in round-robin order, and reports whether
a read result may still be sent. Its composite identity permits one process-wide
service even though each torrent ledger reuses local connection slots. Requests
with zero or greater-than-one-block lengths are refused. The configured byte
ceiling has a one-wire-block minimum, then acts as a hard cap. A disconnect ends
protocol obligations but does not release capacity for reads already dispatched
to storage. Actor integration and independent read/write progress remain
NET-106/NET-114.

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
- Equal ledger connection slots and generations in separate torrents remain
  distinct registrations; disconnecting one cannot remove the other's work.
- Regression sources cover queue bounds, byte bounds, fairness, invalid wire
  lengths, connection-generation reuse, choke, cancel, and disconnect
  completion ownership.
- NET-114 routes live v1, v2, and hybrid uploads through the service before the
  capability is considered integrated.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/{coords,upload,obligations}.rs`
  and the NET-114 peer adapters.
- **State or wire compatibility:** no wire-format change.
- **Migration / rollback:** actor-local request handling remains selectable
  until shared-service parity and rollback coverage exist.
- **Feature flag or rollout boundary:** no release capability claim until
  live paths and cancellation accounting qualify.

### Evidence

- **Regression source added or updated:** upload service queue, fairness,
  budgets, cross-torrent identity isolation, choked reads, and disconnect-held
  budget cases.
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
operations, snapshots dirty and active paths, then groups them by resolved
device and dispatches bounded batches of ready per-file fences. Later writes,
including writes to a new path, remain queued until the evidence is dropped
after the resume commit. Dirty generations remain separate from per-file
sequence watermarks so a newer generation cannot be accidentally cleared.

### Ownership and invariant

- **Current owner:** each `MountScheduler` clone shares scheduler-wide write
  admission, a global diagnostic write sequence, weakly registered per-path
  fences, its dirty-generation map, and completion watchers for submitted
  writes and syncs. Full checkpoint barriers run in an owned task so dropping
  a waiter does not cancel a partially completed path snapshot.
- **Target owner:** IO-006's shared device service receives bounded batches
  of ready per-file fences grouped by resolved device; each path retains its
  own completion watermark and device operation identity.
- **Invariant preserved:** a scheduler-wide checkpoint admits no later
  write/preallocation/sync operation until every path in its stable snapshot
  has completed `fdatasync`; per-file fences, the detached barrier task, and
  completion watchers retain ownership across caller cancellation. A
  successful result keeps scheduler write admission in its evidence until the
  resume commit drops it.
- **Must not:** clear a newer generation, sync a clean unrelated descriptor,
  open more descriptors than the bounded batch permits, or let
  preallocation/write completion pass a barrier queued behind it.

### Acceptance

- `sync_data` and checkpoint sync capture a sequence watermark after all
  earlier admitted writes to that path complete.
- A checkpoint cannot miss a write admitted on a new path after path
  enumeration begins; that write waits for the returned barrier and remains
  dirty for the next one.
- Canceling the barrier waiter cannot release write admission while any path
  in the snapshot is still syncing; the detached barrier finishes the snapshot
  and returns no evidence to a canceled waiter.
- A write queued behind a path's exclusive fence waits until its `fdatasync`
  finishes and remains dirty for a later barrier.
- Failed barriers and partial writes retain dirty evidence; a successful
  strict write clears only its own generation.
- `sync_all_open_files` keeps descriptor use bounded and syncs only its
  scheduler's dirty or active paths.
- Ready paths on one resolved device enter a bounded batch together; batch
  size does not exceed the device dispatch window, handle-pool capacity, or
  mount queue capacity, and each path retains an independent fence and dirty
  generation.
- `sync_all_open_files_with_evidence` returns an opaque proof only after every
  path in the barrier succeeds; it reports synced-file count and the stable
  scheduler write-sequence watermark, and it retains write admission until
  the proof is dropped after its resume commit.
- Per-device batching is implemented as bounded path groups; regression and
  storage fault-matrix evidence remain pending the requested bulk pass.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`, IO-006's
  device service, and crash-cut fixtures.
- **State or wire compatibility:** no protocol change; the evidence API is
  additive and does not alter storage payload layout.
- **Migration / rollback:** none; the reference backend stays selectable.
- **Feature flag or rollout boundary:** durability mode is unchanged.

### Evidence

- **Regression source added or updated:** existing canceled-writer,
  dirty-generation, and failed-barrier retry sources; held-sync regressions
  cover a new-path write queued behind the scheduler-wide checkpoint, a
  canceled waiter whose barrier must finish a multi-path snapshot, and
  same-device paths entering one bounded batch before either sync completes.
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
clear every warm stream. The cache also retains multiple independently useful
forward ranges per file up to the configured total entry cap, evicting the
least-recently reused range. Live-engine qualification remains open.

### Ownership and invariant

- **Current owner:** `MountScheduler` owns bounded stream history, per-file
  range entries, and per-read admission; the portable backend owns the
  submitted read.
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
- A file may retain multiple disjoint useful ranges; total range entries stay
  within the configured cap and least-recently-used ranges are evicted first.
- Replacing a path or completing a write invalidates every cached range for
  that path.
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
  feedback; `peer_read_readahead_cache_retains_disjoint_ranges_for_one_file`
  covers multi-range reuse; scheduler cache regressions remain in the bulk
  pass.
- **Command or probe to run:** storage scheduler tests during the requested
  bulk pass, then matched fixture runs for reuse and read amplification.
- **Expected result:** exact reads for isolated requests, bounded growth for
  continuous streams, and no speculative budget leak.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** filesystem/backend read amplification
  and throughput under realistic peer request patterns.

## IO-014 — Schedule secure copies and verify each leaf once

- **Priority / wave:** P1 / 10
- **Prerequisites:** IO-006, IO-009, IO-012
- **Owning component:** durable storage-plan worker and `rt-storage` secure
  filesystem executor.
- **Finding or capability:** secure copies must retain bounded admission while
  preserving descriptor-anchored paths and one content verification per leaf.
- **Status:** `wip`

### Scope

`CopyVerifyRename` and `ImportExisting` plan steps dispatch through the
destination mount's shared `DeviceQueue` in the `MoveCopy` lane. Source and
destination paths are resolved to physical-device queues and admitted in
stable key order, covering source reads, staging writes, final renames, source
deletion, and rollback. The worker passes the same `ResourceGovernor` and
storage-I/O settings used by torrent actors. Checkpoint persistence stays
outside device permits. The durable plan worker owns cursor state, while the
scheduler owns each blocking quantum and its in-quantum cleanup through
completion. The charged
working set is one 64 KiB streaming buffer, not the payload length. Regular
file `CopyVerifyRename` work is split into copy, source-hash, and destination-
hash quanta; each quantum reacquires all touched device queues before it does
I/O, then yields before the next admission. The physical queue set is
resolved once for the plan step and reused rather than rediscovering mount
topology for every chunk. The cursor keeps only confined descriptors and hash
state between quanta. A failed or cancelled quantum
closes its handles and removes the partial destination while device admission
is still held. If the scheduler rejects further admission, the retained cursor
uses its root-confined descriptors for terminal cleanup before returning the
admission error. F25's nested-entry copy-only behavior still verifies each
copied leaf once per side.

When physical-device IDs are unavailable, all copy jobs through the engine's
storage dispatcher share its stable fallback queue. This conservatively bounds
unknown devices without assuming two paths are independent.

Directory verification now streams entries on Unix and Windows without
materializing per-directory name vectors. Unix recursive deletion retains at
most 256 names per batch and restarts enumeration after each batch, so removing
entries cannot invalidate an active directory cursor. Regular-file
`CopyVerifyRename` steps and regular-file imports that cannot use a hard link
now yield device slots between bounded copy and hash quanta. Directory-tree
copies still hold their admitted device slots for the full secure step;
Unix same-filesystem `ImportExisting` retains its one-operation hard-link
fast path; Windows imports use bounded quanta for regular-file copies.
Final file sync and recursive directory traversal are not preemptible yet.

### Ownership and invariant

- **Current owner:** the durable storage worker checkpoints plan steps; the
  destination mount scheduler owns each quantum's queued bytes and the
  source/destination device permits until that blocking quantum and its local
  cleanup return. The cursor retains root-confined descriptors and hash state
  between admissions.
- **Target owner:** one bounded storage service accounts for every physical
  device touched by a copy while verification uses the shared hash service.
- **Invariant preserved:** root-confined descriptor operations, rollback on
  interrupted staging, and once-per-leaf content verification remain intact.
- **Must not:** charge the full file length as retained memory or release
  scheduler admission while a blocking copy can still mutate the destination.

### Acceptance

- Secure copy/import steps wait behind and share the configured destination
  device queue across mounts.
- Queued working-set bytes and the device permit remain owned until an admitted
  quantum and its cleanup finish. Cancellation between quanta is handled by
  the next admitted quantum before its permit is released.
- Regular-file copies and both verification reads process
  no more than 64 KiB per device admission, then release permits before the
  next quantum; one-block-per-quantum and cancellation cleanup regressions
  cover Unix and Windows.
- Directory-tree copies and final syncs remain explicitly identified where a
  full step can still hold device slots; Unix same-filesystem `ImportExisting`
  retains the single-operation hard-link fast path.
- Source reads, destination writes, renames, deletes, and rollback mutations
  acquire admission for every touched physical device in a global order.
- Opposing cross-device copies cannot deadlock while holding device permits.
- Nested directory copies hash each leaf once on each side and reject
  same-length corruption.
- Verification streams directory entries without a per-directory name vector;
  recursive deletion retains at most the fixed batch size.
- Regression sources cover scheduler contention, cancellation cleanup,
  cross-device admission, and once-per-leaf hashing.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/plan.rs`,
  `crates/rt-storage/src/secure_fs.rs`,
  `crates/rt-storage/src/windows_secure_fs.rs`,
  `crates/rt-storage/src/scheduler.rs`,
  `crates/rt-engine/src/storage_jobs.rs`, and engine scheduler composition.
- **State or wire compatibility:** no resume, storage-path, or peer-wire
  format change.
- **Migration / rollback:** the direct storage-plan path remains available to
  non-engine callers; production plan copies use the configured device queue.
- **Feature flag or rollout boundary:** no performance claim until bulk
  correctness and storage measurements pass.

### Evidence

- **Regression source added or updated:**
  `secure_copy_plan_waits_for_mount_device_admission` proves a queued plan does
  not create its destination while the mount queue is full;
  `external_device_handoff_shares_admission_across_mounts` covers shared device
  admission; `multi_device_handoff_holds_source_and_destination_until_completion`
  checks both physical permits remain held. The existing secure-copy hash-count
  and cleanup sources remain in place; `directory_name_stream_visits_all_entries`
  and `directory_deletion_batches_bound_retained_names` cover the streaming
  iterator and bounded Unix deletion batch;
  `regular_file_copy_cursor_advances_in_bounded_quanta` and
  `regular_file_copy_cursor_cancellation_removes_partial_destination` cover
  bounded file work and cancellation cleanup on Unix and Windows;
  `regular_file_import_keeps_the_hard_link_fast_path` protects the Unix
  same-filesystem import optimization;
  `streamed_tree_verification_rejects_destination_only_entries` preserves
  exact directory-content matching without a name set on Unix, and
  `windows_secure_fs::tests::streamed_directory_verification_rejects_destination_only_entries`
  covers the Windows handle-relative path.
- **Command or probe to run:** storage plan, scheduler admission, cancellation,
  secure-copy hash-count, and injected-backend suites in the deferred bulk pass.
- **Expected result:** no plan step begins before all touched device queues
  admit it, no permit or byte charge is released while accepted work remains
  active, directory verification/deletion retain bounded entry state, and
  rollback/reconciliation report uncertain filesystem state explicitly.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** matched cross-device throughput and
  memory results under realistic concurrent peer I/O.

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

## SYS-004 — Compact generation-checked tier identities

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** `rt-engine::TierController` and engine runtime maps.
- **Finding or capability:** F14/F27; compact metadata identities and honest
  whole-owner memory accounting.
- **Status:** `wip`

### Scope

Replace repeated string keys inside high-cardinality runtime owners with
compact identities while retaining the canonical info hash at string-facing
boundaries. `TierController` uses owner-local slot/generation IDs: its tier
maps and timer wheels use an 8-byte ID, while one interned `Arc<str>` identity
supports lookups. `EngineRuntimeState` channel, task, activity, and pending-
promotion maps, pending-add/delete sets, and supervised task-exit events use
inline decoded keys for canonical v1/v2 hashes; the map and set adapters keep
`&str` lookup boundaries, and noncanonical placeholder keys retain exact
text. No database, API, or wire representation changes.

### Ownership and invariant

- **Current owner:** `TierController` stores full keys in its tier maps,
  dormant projections, timer queues, and timer reverse index.
- **Target owner:** an owner-local key interner retains the canonical hash;
  tier maps and timers carry only `(slot, generation)`.
- **Invariant preserved:** removing an identity invalidates its slot before
  reuse; stale timer entries cannot resolve to a newer identity that reused
  the same slot. Tier counts and dormant snapshot behavior remain unchanged.
- **Must not:** persist the runtime ID, change hash canonicalization, or let a
  stale timer act on a newly added torrent.

### Acceptance

- Tier and timer indexes use an 8-byte slot/generation ID; the key index and
  slot share one interned string allocation.
- Slot reuse increments generation. A generation that would overflow retires
  that slot instead of wrapping and aliasing an old ID.
- Lazy stale timer entries are skipped after removal and do not target an ID
  that reuses the old slot.
- Whole-owner estimates include retained map capacities, slot/free-list
  capacities, timer queue capacities and stale entries, interned string bytes,
  and separately owned dormant snapshot strings.
- Regression sources cover generation-safe slot reuse, dormant projection
  accounting, and whole-owner memory accounting.
- Task-exit IDs, the four `EngineRuntimeState` maps, and the pending add/delete
  sets keep canonical hashes inline and preserve their existing string-facing
  lookup behavior.
- Compact map/set keys do not by themselves prove a whole-process memory or
  throughput improvement; that requires the planned bulk and matched-client
  evidence.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/tier.rs`,
  `subsystems.rs`, engine lifecycle maps, and the rebuild ledger.
- **State or wire compatibility:** no persistence, API, or protocol format
  changes; public dormant snapshot hash remains a string.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** no flag; qualify with the bulk pass.

### Evidence

- **Regression source added or updated:**
  `reused_tier_identity_has_new_generation_and_skips_stale_timer`,
  `dormant_snapshot_is_accounted_and_cleared_on_promotion`, and
  `owner_bytes_cover_maps_and_timers_not_just_the_hash_string`, plus
  `compact_exit_ids_roundtrip_v1_v2_and_keep_non_hash_test_keys` and
  `runtime_torrent_maps_keep_canonical_keys_inline_and_accept_hash_strings`
  and `runtime_torrent_sets_keep_canonical_keys_inline_and_accept_hash_strings`.
- **Command or probe to run:** `cargo test -p rt-engine tier::tests` during
  the requested bulk pass.
- **Expected result:** stale timer IDs never resolve after slot reuse and the
  owner estimate includes retained tier, snapshot, interner, and timer storage.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** PERF-109/SYS-014 catalog and mixed-load
  qualification.

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

## SYS-010 — Bounded discovery and control I/O

- **Priority / wave:** P1 / 4
- **Prerequisites:** SYS-004, SYS-008
- **Owning component:** tracker workers, outbound DNS admission, DHT bootstrap
  and registration, and engine metadata loading.
- **Finding or capability:** F35 plus bounded process-wide admission for
  discovery I/O and detached metadata work.
- **Status:** `wip`

### Scope

Bound tracker announces across torrents, share one no-wait DNS budget across
tracker/webseed egress and IPv4/IPv6 DHT bootstrap, and cap persisted metadata
projection loads that wait for blocking parsing. Keep optional scrape work
independent of announce delivery. Existing DHT registration, retry, sweep,
command-delivery, bootstrap node, answer-count, and deadline limits remain part
of the same control-I/O contract. Magnet completion, torrent promotion, DHT
registration, and pure-v2 recheck keep their own admission budgets; they do not
consume the projection-load semaphore.

### Ownership and invariant

- **Current owner:** `TrackerWorkers` owns each torrent's announce futures;
  process semaphores own global announce, scrape, and DNS admission; the DHT
  task owns bootstrap and registration; `load_torrent_metadata_bounded` owns
  persisted projection-load permits.
- **Invariant preserved:** accepted announce responses reach their torrent
  actor before optional scrape work; saturated budgets do not create an
  unbounded waiter or detached-task queue; DNS permits cover resolution only.
- **Overload behavior:** tracker work stays due for a later actor turn, optional
  scrapes are skipped, DNS lookups fail quickly, and projection-load requests
  are rejected when the bounded active-plus-waiting admission is full.
- **Must not:** hold a global announce permit while scraping, release projection
  permits when an async caller is cancelled but `spawn_blocking` is still
  running, or collect unbounded resolver answers.

### Acceptance

- At most 8 tracker announces run per torrent and 64 process-wide; announce
  result channels retain at most 8 items per torrent.
- Optional HTTP scrapes remain capped process-wide at 4 and use a separate
  result channel sized to that cap.
- Tracker, webseed, and DHT bootstrap share a non-queuing 32-lookup DNS budget;
  each lookup retains its existing deadline and 64-address ceiling.
- DHT bootstrap remains capped at 256 nodes, a 10-second aggregate window, and
  5 seconds per node. Registration parse concurrency is 32, detached
  registration/retry tasks are capped at 256, sweeps at 1, and command delivery
  at 5 seconds.
- Persisted metadata projection loads allow 4 active jobs and at most 32
  additional admitted callers; the blocking closure owns both permits until
  parsing exits, even if its async caller is cancelled. Other parser entry
  points remain governed by their separately documented admission budgets.
- Saturation and scrape progress have regression sources; bulk qualification
  confirms overload recovery and whole-session responsiveness.

### Files and compatibility

- **Expected code paths:** `outbound_resolver.rs`, `egress_policy.rs`,
  `tracker_runtime.rs`, `dht_task.rs`, `engine.rs`, and the rebuild ledger.
- **State or wire compatibility:** no tracker protocol, DHT wire, metadata, or
  database format changes; only local admission and retained queue bounds.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** existing bounds apply to all
  outbound-discovery paths.

### Evidence

- **Regression sources:** `dns_budget_rejects_excess_work_without_a_wait_queue`,
  `tracker_worker_availability_obeys_the_process_wide_limit`,
  `metadata_load_admission_rejects_when_the_bounded_queue_is_full`, and the
  existing bounded-answer and announce-before-scrape regressions.
- **Command or probe:** run the targeted resolver, tracker-worker, metadata
  loader, and DHT bootstrap suites during the requested bulk pass.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** mixed-load responsiveness and recovery
  under saturated tracker, DNS, DHT, and metadata budgets.

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

## PERF-108 — Stable memory accounting domains

- **Priority / wave:** P0 / 4
- **Prerequisites:** PERF-101, PERF-102, SYS-011
- **Owning component:** `rt-engine` memory projections, the native Prometheus
  renderer, and the independent PERF-102 observer.
- **Finding or capability:** remove shadow-runtime metrics and keep governor
  leases, torrent-owner estimates, allocator values, process memory, and
  cgroup measurements distinct.
- **Status:** `wip`

### Scope

Keep existing Prometheus names stable while defining what each memory family
counts. The resource-governor family remains active leases, not RSS. The
top-torrent family remains a bounded estimate; expose outstanding-request
receive reservations separately from upload buffers because the reservation
can overlap materialized piece memory. PERF-102 remains the independent source
for process, allocator, and cgroup observations.

### Ownership and invariant

- **Current owner:** TorrentNG owners expose lease snapshots and bounded
  per-torrent counters; the external observer samples process and cgroup
  state.
- **Target owner:** unchanged; each metric reads its existing owner and each
  measurement domain remains separate.
- **Invariant preserved:** a scrape never initializes an owner, and no
  governor, RSS, allocator, cgroup, or estimate value is relabeled as another
  domain.
- **Must not:** add an aggregate that sums overlapping owner estimates and
  call it process memory, or treat unavailable measurements as zero.

### Acceptance

- Keep `torrentng_memory_used_bytes` and class usage explicitly defined as
  current resource-governor leases.
- Keep the top-torrent aggregate metric name, document its estimate and
  reservation overlap, and separately render receive reservations and upload
  buffers for each retained top torrent.
- Hash torrent identifiers by default and preserve the existing explicit
  opt-in for raw identifiers.
- Document metric meaning, source owner, and known overlap in `docs/API.md`
  and `docs/TRANSFER_CORE_OBSERVABILITY.md`.
- Renderer regressions verify the separate receive/upload values and the
  default identifier redaction.
- Process, allocator, and cgroup evidence continues to come from the
  identity-bound observer; large-catalog scrape cost remains SYS-014/PERF-109.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/command.rs`,
  `crates/rt-api-native/src/handlers.rs`, `docs/API.md`, and
  `docs/TRANSFER_CORE_OBSERVABILITY.md`.
- **State or wire compatibility:** additive gauges only; existing metric names
  and hash privacy defaults remain stable.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** no new flag; interpret owner estimates
  only within their documented accounting domain.

### Evidence

- **Regression source added or updated:** engine estimate projection and native
  metrics renderer now preserve the combined estimate while exposing receive
  reservations and upload buffers separately.
- **Command or probe to run:** engine aggregation and native metrics renderer
  regressions during the requested bulk pass.
- **Expected result:** values retain their owner and overlap semantics, with
  raw identifiers absent unless explicitly enabled.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** PERF-102 process/allocator/cgroup
  observations and SYS-014/PERF-109 collection-cost qualification.

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

## PERF-109 — Live production catalog fixtures

- **Priority / wave:** P0 / 5
- **Prerequisites:** PERF-102, PERF-103, PERF-104, PERF-105, PERF-108
- **Owning component:** `rt-testkit` catalog metainfo fixtures and
  `rt-metrics/tests/scale.rs`.
- **Finding or capability:** current scale proxies populate API registries and
  databases with fabricated rows. Add a path through valid metainfo, engine
  persistence, runtime registry, and the native API before describing a
  catalog observation as production-shaped.
- **Status:** `wip`

### Scope

Generate deterministic, unique, valid private v1 metainfo with a real
content-derived piece hash. Populate an isolated engine with paused torrents
through `EngineHandle::add_torrent_raw_with_labels`, which parses the metainfo,
writes the torrent blob, persists the row, and registers the runtime entry.
Shut down the ingest engine cleanly, restart it from the same isolated database
and metadata blobs, then measure fixture generation, engine admission, restore,
and first/cached native API list requests while draining the response body.
Keep smaller direct-registry and synthetic-database cases as explicitly named
lower-level proxies.

The catalog fixture is metadata and persistence coverage. Its one-byte payload
does not exercise payload I/O, hashing throughput, swarm activity, or useful
goodput. The 100k live-engine case is opt-in and remains unrun until the bulk
scale pass; its source is not capacity evidence.

### Ownership and invariant

- **Current owner:** `production_catalog_metainfo` builds deterministic
  bencoded metainfo; the scale harness sends it through the public engine API
  and observes the native API router.
- **Target owner:** PERF-110 launches the exact daemon and adapters as separate
  processes for resource-attributed, paired catalog and transfer trials.
- **Invariant preserved:** a live catalog observation contains distinct,
  parseable info hashes and paused entries admitted by the engine; synthetic
  registry and row tests remain labeled as proxies.
- **Must not:** treat in-process test RSS as daemon RSS, label a one-byte
  catalog as transfer capacity, or infer 100k success before the ignored case
  runs on the exact source-bound binary.

### Acceptance

- Catalog metainfo is repeatable for one index, parseable as v1, private, and
  content-hash consistent; adjacent indexes have different info hashes.
- The live 1k case starts the production engine in task-owned temporary paths,
  admits each raw metainfo record through the engine API, verifies registry
  count and paused state, then calls the native list route and drains its
  response while checking the reported total.
- The 100k case uses the same fixture and engine path, is opt-in due to its
  resource cost, and reports an explicit setup failure if the production
  admission or restore path cannot reach the target count.
- Observations report fixture-generation, engine-admission, restore,
  first-list, and cached-list timings without applying a capacity threshold or
  claiming a performance win. “First” and “cached” describe the API snapshot;
  they do not imply cold or warm filesystem cache state.
- Synthetic API and database test names include `proxy`; comments and the
  harness documentation state that they are not production-capacity evidence.
- The audit, active specification, harness README, and ticket ledger distinguish
  authored source from executed evidence. PERF-102 and PERF-110 remain
  responsible for independent process resources and matched mixed-load trials.

### Files and compatibility

- **Expected code paths:** `crates/rt-testkit/src/transfer_fixtures.rs`,
  `crates/rt-metrics/tests/scale.rs`, `scripts/transfer_rebuild/README.md`,
  and the scale-claims audit.
- **State or wire compatibility:** test-fixture and regression-source changes
  only; no product data format or runtime behavior changes.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** the 100k observation remains ignored
  by default and is invoked explicitly during scale qualification.

### Evidence

- **Regression source added or updated:** `rt-testkit` verifies deterministic
  parseable catalog metainfo; `rt-metrics/tests/scale.rs` exercises 1k and
  opt-in 100k engine-backed API paths and labels synthetic cases as proxies.
- **Command or probe to run:** the live 1k case and the explicit ignored 100k
  case during the requested bulk scale pass.
- **Expected result:** each admitted fixture has a unique valid info hash and
  paused registry entry; restart restores the full catalog; API requests return
  successfully with separate setup, restore, first-page, and cached-page
  timings.
- **Executed on exact revision:** no.
- **Artifact or report path:** none until PERF-110 writes source-bound trial
  records and independent process observations.
- **External evidence still required:** production-binary 10k/100k restore,
  process-attributed resource samples, mixed activity, and repeated trials.

## PERF-110 — Reproducible paired-run catalog and schedule

- **Priority / wave:** P0 / 6
- **Prerequisites:** PERF-103, PERF-104, PERF-105, PERF-106, PERF-107, PERF-109
- **Owning component:** certification tooling.
- **Finding or capability:** paired comparisons need a fixed workload catalog,
  randomized order, and explicit exclusions before a runner can create trial
  evidence.
- **Status:** `wip`

### Scope

This spec interprets the ticket's “28-case catalog” as the seven checked-in
client adapters crossed with the four checked-in fixture profiles. This is a
separate efficiency catalog from the 28-row protocol correctness matrix in
`docs/INTEROP_MATRIX.md`. The plan generator records all 28 slots, including
known unsupported combinations, and produces a seeded schedule for pinned
clients. The `candidate` label means only “not known to be unsupported by this
catalog”; client/profile compatibility still needs a runtime preflight before
the trial is considered eligible.

The current known exclusions are the `urtorrent` runner's v2 and hybrid slots:
it accepts private v1 metainfo only. Unpinned clients and profile/repetition
pairs with fewer than two candidate clients are explicit exclusions. Every
scheduled pair holds profile and repetition fixed while randomizing client
order, so the future runner can reuse one identical fixture and environment
for its paired clients.

### Ownership and invariant

- **Current owner:** `paired_plan.py` builds the catalog, seed-bound plan, and
  immutable output; `validate_artifact.py paired-plan` validates its schema,
  catalog digest, and deterministic schedule.
- **Target owner:** PERF-110's process runner consumes the plan, performs
  compatibility preflight, prepares shared fixtures and network state, starts
  each client, observes independent resources, and emits PERF-101 trial and
  manifest artifacts.
- **Invariant preserved:** every included schedule slot is pinned and
  catalogued; the seed, schedule algorithm, adapter pins, and fixture-profile
  digest are retained with the plan; no pair compares different profiles.
- **Must not:** treat `candidate` as verified client support, label a plan as
  measured evidence, silently skip a failed or incompatible trial, or conflate
  this efficiency catalog with protocol interop correctness.

### Acceptance

- The catalog contains exactly seven adapter IDs by four profile IDs and has a
  canonical SHA-256 digest over its workload definitions and the profile-file
  digest.
- A caller supplies a valid pin file, unsigned 64-bit seed, and 1–100
  repetitions. The plan records the pin and catalog digests and the explicit
  `sha256-fisher-yates-v1` scheduling algorithm.
- Pair-group and client order are randomized reproducibly from the seed. Every
  pair uses one profile and repetition, has distinct clients, and includes all
  pinned candidate clients for that profile. Pairs with fewer than two clients
  are excluded with a reason.
- Unsupported and unpinned catalog slots are listed with reasons. A runtime
  compatibility preflight must reject or explicitly exclude unsupported
  candidates before any measurements begin.
- Plan output is exclusive, mode `0600`, schema-validated, and rejected if its
  catalog digest or deterministic schedule no longer matches its identity.
- Regression sources cover the 28 slots, balanced pairs, stable scheduling,
  exclusions, tamper rejection, bounds, and exclusive output.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/paired_plan.py`,
  `scripts/transfer_rebuild/fixtures.py`,
  `scripts/transfer_rebuild/validate_artifact.py`,
  `certification/transfer-rebuild/paired-plan.schema.json`, harness README,
  and `scripts/test_transfer_rebuild_paired_plan.py`.
- **State or wire compatibility:** additive `transfer-paired-plan-v1`
  certification artifact; no runtime product format changes.
- **Migration / rollback:** none; plans are standalone artifacts and can be
  regenerated from pinned inputs and the recorded seed.
- **Feature flag or rollout boundary:** no comparison or performance claim is
  permitted from the schedule alone. Compatibility preflight, process launch,
  trial/manifest production, independent observation, and bulk validation
  remain required.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_paired_plan.py` and paired-plan semantic
  validation in `scripts/transfer_rebuild/validate_artifact.py`.
- **Command or probe to run:** planner, schema-validator, and regression cases
  during the requested bulk pass.
- **Expected result:** byte-identical plan content for identical inputs;
  schema-valid catalog and schedule; tampered identity or ordering rejected;
  no output overwrite.
- **Executed on exact revision:** no.
- **Artifact or report path:** none; generation and validation source only.
- **External evidence still required:** pinned compatibility preflight for each
  candidate slot, actual paired process execution, complete trial records,
  independent resources, and dedicated-host results.

## PERF-111 — Picker and changed-snapshot probes

- **Priority / wave:** P1 / 6
- **Prerequisites:** PERF-101, PERF-102, PERF-109
- **Owning component:** picker and native API performance probes.
- **Finding or capability:** F14's picker memory estimate and large-library
  snapshot work need repeatable, scale-shaped observations rather than a
  synthetic allocation claim.
- **Status:** `wip`

### Scope

Add standalone, dependency-free Rust benchmark targets for two narrow paths.
The picker probe measures a warmed pick/cancel cycle at increasing piece
counts. The native API probe measures the initial, unchanged cached, and
single-row changed `AppState::torrent_snapshot` paths at increasing torrent
counts, including allocation calls and requested allocation bytes from a
process-local counting allocator. Results are diagnostic; they are not
threshold tests or whole-client performance evidence.

### Ownership and invariant

- **Current owner:** `rt-piece-picker` and `rt-api-native` benchmark targets;
  each prints CSV to standard output for retention in a task-owned artifact
  directory.
- **Target owner:** PERF-111 provides repeatable probes for the PERF-102 / PERF-109
  scale evidence and picker implementation review.
- **Invariant preserved:** cold/full snapshot, unchanged cache hit, and
  changed-row refresh remain separately labeled; allocation traffic is never
  described as retained heap or process RSS.
- **Must not:** introduce host-dependent pass/fail timing thresholds, mix
  fixture setup into the timed picker loop, or call a method-only probe a
  whole-daemon measurement.

### Acceptance

- Picker probe covers 1k, 12k, and 100k pieces with peer availability and
  prebuilt partial state up to the 16,384-piece in-progress cap; the 100k case
  uses a sparse peer bitfield over that admitted subset. It records the
  pick/cancel cycle duration and live picker memory estimate.
- API probe covers 1k, 10k, and 100k registry rows and separately reports one
  full initial snapshot, 1,000 unchanged cached reads, and three one-row
  incremental refreshes. Changed samples wait beyond the API snapshot TTL
  outside the timed region.
- The API probe runs on a single-thread runtime with its own counting global
  allocator, and reports allocation calls/bytes only during each measured
  snapshot operation. Those numbers mean cumulative allocation traffic.
- Both probes emit stable CSV headings, have no pass/fail thresholds, and
  document the need to retain commit, toolchain, and host identity with output.
- Regression source records the intended probe workloads; the exact commands
  and bulk measurements remain unrun.

### Files and compatibility

- **Expected code paths:**
  `crates/rt-piece-picker/benches/perf.rs`,
  `crates/rt-api-native/benches/perf.rs`, their Cargo benchmark declarations,
  crate/harness READMEs, and this spec.
- **State or wire compatibility:** benchmark-only code; no runtime format or
  behavior changes.
- **Migration / rollback:** none; delete the standalone benchmark targets if
  later evidence shows they measure the wrong boundary.
- **Feature flag or rollout boundary:** no release gate is inferred from the
  output; whole-client matched trials remain PERF-110/113 and bulk qualification.

### Evidence

- **Regression source added or updated:** standalone picker and native API
  benchmark targets; no benchmark was executed for this source slice.
- **Command or probe to run:** both `cargo bench` commands documented in the
  harness README during the requested bulk pass.
- **Expected result:** valid CSV rows for each workload and explicitly
  separated initial/cache-hit/changed snapshot allocation traffic.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending a task-owned PERF-111 bulk run.
- **External evidence still required:** repeat runs on the dedicated,
  identified hardware; compare exact source/toolchain; independent process RSS
  and whole-service evidence remain separate.

## PERF-112 — V2 and persistence invariants in full engine paths

- **Priority / wave:** P0 / 5
- **Prerequisites:** PERF-104, PERF-105, PERF-106
- **Owning component:** production-shaped transfer fixtures and the
  `rt-metrics` engine integration target.
- **Finding or capability:** performance cases need protocol-correct payloads
  and must not count persisted completion state as valid after its content has
  changed.
- **Status:** `wip`

### Scope

Use the deterministic v1, pure-v2, and hybrid compatibility fixtures between
two production engine instances over loopback TCP. The receiver must obtain
all payload files through the real peer-wire, storage, and verification paths.
Persist paused completion state through a clean receiver shutdown and restore
it from the same database and metainfo blobs. Change one byte in the pure-v2
payload between process lifetimes, then require resume-time verification to
demote that torrent while the unchanged v1 and hybrid payloads remain
complete. This is a small correctness transfer and restart case, not a
transfer-rate or whole-client performance result.

### Ownership and invariant

- **Current owner:** `rt-testkit` defines payload-derived protocol fixtures;
  `rt-engine` owns real metainfo admission, peer transfer, storage
  verification, and registry updates; the `rt-metrics` integration target
  observes lifecycle results across restart.
- **Target owner:** PERF-110's paired runner must run clients against pinned
  v2/hybrid fixtures and invoke integrity/restart gates before a trial can be
  treated as comparable.
- **Invariant preserved:** the receiver gets exact expected payload bytes over
  the actual TCP peer path; a torrent that was previously complete is not
  trusted after restart until its current payload verifies; v1, pure-v2, and
  hybrid identities remain distinct, and hybrid pad bytes stay zero-filled.
- **Must not:** label local storage recheck as a peer transfer, accept the
  persisted `amount_left == 0` row without resumed verification, or infer
  filesystem durability beyond IO-009/IO-010 evidence.

### Acceptance

- Start isolated seed and receiver engines on task-scoped loopback ports.
- Materialize each seed fixture under its name-wrapped save path, leave
  receiver payloads absent, and create the expected hybrid padding file.
- Admit raw metainfo through `EngineHandle::add_torrent_raw_with_labels`,
  connect the receiver with `EngineHandle::add_peers`, and observe all three
  cases reach complete state with zero bytes remaining and exact payload-file
  bytes at the receiver.
- Pause the completed entries, shut down cleanly, and restore all three from
  the same database and metainfo blobs with their protocol identities and
  completed counters intact.
- Corrupt one pure-v2 payload byte between engine lifetimes. After resume, the
  pure-v2 entry must return to `Downloading` with bytes remaining; unchanged
  v1 and hybrid entries must reverify as `Seeding` with zero bytes remaining.
- Keep peer network transfer, matched goodput, resource attribution, crash-cut
  behavior, and strict durability qualification as separate evidence owned by
  PERF-110, PERF-102, IO-009, and IO-010.

### Files and compatibility

- **Expected code paths:**
  `crates/rt-metrics/tests/transfer_invariants.rs` and this spec.
- **State or wire compatibility:** regression-source changes only; no
  product format or runtime behavior changes.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** none; paired performance results stay
  unqualified until their network payload and restart invariants pass.

### Evidence

- **Regression source added or updated:**
  `persisted_v2_completion_is_rechecked_after_restart` transfers the v1,
  pure-v2, and hybrid fixtures between production engine instances, verifies
  receiver bytes, and corrupts only pure-v2 content between restarts.
- **Command or probe to run:**
  `cargo test -p rt-metrics --test transfer_invariants -- --nocapture`
  during the requested bulk pass.
- **Expected result:** valid protocol fixtures become complete through the
  production storage path; persisted completion is restored; corrupted v2
  content is rejected after restart while the unchanged protocol cases remain
  complete.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass.
- **External evidence still required:** matched network transfer and payload
  integrity for all applicable PERF-110 client/profile pairs.

## PERF-113 — Descriptive paired scorecard

- **Priority / wave:** P1 / 7
- **Prerequisites:** PERF-101, PERF-102, PERF-104, PERF-110
- **Owning component:** certification analysis tooling.
- **Finding or capability:** comparisons must retain exact pairing, workload
  identity, accounting domains, exclusions, and uncertainty without turning
  proposed numbers into unapproved release thresholds.
- **Status:** `wip`

### Scope

Read a validated manifest and its linked trial records, form comparison groups
only from identical workload and environment signatures, and calculate
pairwise client ratios with deterministic percentile-bootstrap 95% intervals
for their median. Preserve per-pair observations and reasons for excluded
attempts. The scorecard is descriptive; it does not make release decisions or
adopt the proposal in `TRANSFER_CORE_REBUILD.md`.

### Ownership and invariant

- **Current owner:** `scorecard.py` consumes immutable PERF-101 manifest
  evidence; PERF-110 remains responsible for producing randomized paired runs.
- **Target owner:** the paired runner and scorecard report remain separate
  stages so an analysis can be reproduced from a retained manifest.
- **Invariant preserved:** client comparisons use the same pair IDs and exact
  workload/environment identity; RSS, PSS, cgroup domains, CPU, I/O, and energy
  are not silently conflated.
- **Must not:** compare unpaired history, drop failed attempts silently, infer
  missing resources as zero, or treat the sample minimum as a pass/fail gate.

### Acceptance

- The input manifest and referenced trial/artifact digests validate before
  analysis.
- Only complete trials with passing applicable integrity enter comparisons;
  each excluded pair has an explicit reason.
- A paired comparison reports each ratio, median ratio, deterministic
  percentile-bootstrap 95% interval, paired count, and informational minimum
  sample status.
- Resource ratios retain their metric direction and accounting domain;
  missing counters remain `null` or are absent from that comparison.
- Client and resampling work are bounded; output is exclusive, mode `0600`, and
  conforms to `transfer-scorecard-v1`.
- The report states that no performance threshold was applied. Proposed
  thresholds require a separate decision before results are observed.
- Regression sources cover deterministic intervals, signature normalization,
  failed/incomplete exclusion, malformed pair membership, and work bounds.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/scorecard.py`, the
  scorecard schema and validator, measurement contract, harness README, and
  `scripts/test_transfer_rebuild_scorecard.py`.
- **State or wire compatibility:** additive `transfer-scorecard-v1` report;
  trial and manifest formats do not change.

## PERF-114 — Source-bound result manifest assembly

- **Priority / wave:** P0 / 8
- **Prerequisites:** PERF-101, PERF-113
- **Owning component:** certification evidence tooling.
- **Finding or capability:** text-grep certification can associate an older
  `PASS` with new code; qualification needs one immutable manifest that binds
  every attempt and referenced artifact to the exact measured source.
- **Status:** `wip`

### Scope

`manifest.py` accepts the exact clean committed worktree, every trial JSON
record, and each additional schema-kind artifact. It hashes the evidence,
validates the complete manifest and all linked files, rechecks input digests and
source identity, and publishes a new mode-`0600` manifest atomically. The
manifest records complete, incomplete, invalid, failed, and unmeasured
outcomes; it never reduces a run to a text summary.

### Ownership and invariant

- **Current owner:** `scripts/transfer_rebuild/manifest.py` assembles evidence;
  `validate_artifact.py` owns schema, link, and semantic validation.
- **Target owner:** PERF-110's paired executor emits trials and calls this
  writer; analysis and CI consume the retained manifest.
- **Invariant preserved:** every referenced file is a regular file below the
  manifest directory, outside the measured source checkout, with a checked
  SHA-256 and matching trial/source identity. A generated manifest describes
  one clean Git commit.
- **Must not:** overwrite evidence, silently omit failed attempts, accept
  symlinks/path escapes, or claim that a dirty flag identifies uncommitted
  source contents.

### Acceptance

- At least one trial is required; duplicate trial IDs and duplicate evidence
  paths are rejected.
- The writer rejects dirty worktrees, source mismatches, evidence under the
  measured checkout, non-regular or symlink paths, malformed/ambiguous JSON,
  digest changes during collection, and an existing output path.
- Before publication, trial and manifest schemas plus all referenced artifact
  digests and identities validate; source and input digests are checked again.
- Publication is exclusive, atomic, mode `0600`, and synced before success is
  reported.
- Regression sources cover all trial outcomes, clean-source binding, dirty
  source rejection, in-tree evidence, duplicate IDs, and overwrite rejection.
- Paired executor integration, cross-platform support, tests, and bulk
  qualification remain open.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/manifest.py`,
  `scripts/test_transfer_rebuild_validator.py`, the measurement contract,
  harness README, rebuild ledger, and capability summary.
- **State or wire compatibility:** no schema change; generates existing
  `transfer-manifest-v1` records.
- **Migration / rollback:** none; the writer creates a new artifact and never
  edits existing manifests.
- **Feature flag or rollout boundary:** none; paired measurements remain
  unqualified until PERF-110 and the requested bulk pass are complete.

### Evidence

- **Regression source added:** `TransferManifestWriterTests` in
  `scripts/test_transfer_rebuild_validator.py` covers all outcome values,
  digest-linked observation artifacts, source cleanliness, evidence location,
  duplicate JSON fields and trial IDs, and exclusive output.
- **Command or probe to run:** the focused manifest/validator suite during the
  requested bulk pass.
- **Executed on exact revision:** no. No tests or builds were run for this
  slice.
- **Artifact or report path:** pending PERF-110 paired-run integration.
- **External evidence still required:** exact matched client observations and
  trial outputs from the paired executor.
- **Migration / rollback:** none; scorecards can be regenerated from retained
  manifests and the pinned analysis revision.
- **Feature flag or rollout boundary:** no superiority or release claim until
  PERF-110 produces complete paired samples and PERF-113 receives the bulk
  pass. No threshold is adopted here.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_scorecard.py`.
- **Command or probe to run:** scorecard and artifact-validator regressions in
  the requested bulk pass.
- **Expected result:** deterministic paired ratios and intervals; malformed,
  unpaired, or incomplete evidence is rejected or explicitly excluded.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending PERF-110 paired runs and bulk pass.
- **External evidence still required:** a complete randomized paired workload
  set on the selected comparison host; scorecard source alone is not a
  performance result.

## IO-015 — Continuous io_uring completion and retained registrations

- **Priority / wave:** P1 / 10
- **Prerequisites:** IO-006, IO-009
- **Owning component:** `rt-storage::UringBackend` and its device workers.
- **Finding or capability:** F20; a slow operation must not hold completed
  I/O or newly admitted work behind a whole-batch wait.
- **Status:** `wip`

### Scope

Keep each Linux ring and its optional fixed-file and fixed-buffer registrations
alive for the worker lifetime. Refill a bounded outstanding window as CQEs
arrive, drain all currently ready completions, and preserve the portable
`pread` fallback and the existing per-device admission boundary.

### Ownership and invariant

- **Current owner:** each `UringWorker` owns its ring, pending operation map,
  fixed-file slot references, and any registered read-buffer lease; the
  `DeviceQueue` owns active and pending admission.
- **Target owner:** the same worker-local ownership remains, with CQEs driving
  progress instead of a batch-wide completion barrier.
- **Invariant preserved:** submitted reads, writes, and syncs retain their
  file, payload/frame, registration, and device ownership until their own CQE
  completes or the ring is destroyed after a fatal failure.
- **Must not:** reuse a registered file/buffer slot while a kernel operation
  may still reference it, cancel admission ownership when a caller drops its
  waiter, or raise the existing device and backend queue bounds.

### Acceptance

- A worker waits for at least one CQE, completes every ready CQE, and refills
  its outstanding set up to the 64-operation window without waiting for the
  slowest operation from a prior batch.
- An active worker uses a nonblocking receiver-lock attempt before waiting for
  ring progress, so an idle worker blocked on input cannot stall another
  worker's CQE processing.
- A closed job channel drains all submitted operations before worker exit.
- Fixed-buffer registration remains within the existing bounded memory-lock
  budget, and each fixed-file slot is cleared only after its final operation
  completes.
- Unsupported kernels, registration failures, and startup failures retain the
  positioned `pread` fallback. Short reads/writes and fatal-ring errors remain
  explicit failures with operation owners retained through ring teardown.
- Regression sources cover more than one outstanding window of concurrent
  reads, backend shutdown with pending work, fixed-slot reuse, and caller
  cancellation while an operation remains in flight.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/backend.rs`,
  `crates/rt-storage/src/device.rs`, `README.md`, and backend regression
  sources.
- **State or wire compatibility:** no storage file format or public wire
  changes; `TNG_STORAGE_BACKEND=auto|pread|uring` and fallback policy remain.
- **Migration / rollback:** select `pread` to use the existing portable
  worker pool; no payload or resume migration is involved.
- **Feature flag or rollout boundary:** the `uring` backend remains opt-in;
  performance and kernel-compatibility qualification are still required.

### Evidence

- **Regression source added or updated:**
  `forced_uring_recycles_its_bounded_submission_window`; existing tests cover
  read/write/sync roundtrips, registration limits, operation-held file slots,
  submission rejection, and fatal ring ownership.
- **Command or probe to run:** `cargo test -p rt-storage` during the requested
  bulk pass, followed by PERF-115 hardware comparison.
- **Expected result:** every submitted operation completes with exact bytes,
  registration and queue bounds remain respected, and CQE progress is not
  serialized behind the slowest operation in a prior batch.
- **Executed on exact revision:** no.
- **Artifact or report path:** pending bulk pass and hardware comparison.
- **External evidence still required:** Linux kernel coverage and paired
  device workloads demonstrating a benefit without latency or durability
  regressions.

## NET-114 — Live transfer actor ownership migration

- **Priority / wave:** P1 / 11
- **Prerequisites:** NET-104, NET-105, NET-108, NET-109, NET-110,
  IO-008, IO-010
- **Owning component:** `rt-engine/src/transfer/` with v1, pure-v2, and
  hybrid peer-runtime adapters.
- **Status:** `wip`

### Scope

Move request reservation, response arbitration, and terminal request cleanup
from actor-local state into the shared transfer core in reversible protocol
slices. The live ledger owns v1/hybrid-v1 and pure-v2 peer acquisition, plus
the serial v1 webseed fallback through a temporary source connection. Partial
fastresume block state is restored into the live ledger and charged to
PeerBuffer alongside picker resume projection. Pure-v2 retains peer-local piece
assembly and uses atomic affinity checks so one connection owns each active
piece. A centralized 60-second ledger expiry routes Cancel actions to peer
tasks and drops matching actor requests. Hybrid dual-view arbitration,
concurrent shared source scheduling, and shared rate/window/upload services
remain open.

### Ownership and invariant

- **Current owner:** the generation-aware ledger owns live v1/hybrid-v1 and
  pure-v2 peer reservations, socket-write commits, received-block arbitration,
  timeout expiry, and completion/failure cleanup. Serial webseed requests use
  a temporary, memory-accounted ledger connection and enter the same response
  arbitration path. Fastresume partial blocks also populate ledger received
  slots under an explicit PeerBuffer lease; the picker retains its candidate
  and resume projection. V2 actor maps retain response leases and peer-local
  assemblies.
- **Geometry foundation:** `PieceGeometry::block_content_ranges` projects a
  wire block into path/offset spans while retaining v1 synthetic padding;
  `HybridGeometry::blocks_match_content` identifies equivalent payload when a
  padded v1 block and its v2 block have different lengths, and
  `canonical_v1_block_for_v2`, `canonical_v2_block_for_v1`, and
  `expand_v2_block_to_v1` provide bidirectional wire projection, mapped ledger
  extents, and zero-filled padding conversion. The live actor ledger does not
  use these helpers yet.
- **Target owner:** each protocol adapter submits candidates to one shared
  content owner, while immutable payload, storage completion, and durability
  evidence retain their existing explicit owners.
- **Invariant preserved:** peer requests are reserved before they enter a
  peer command queue, committed after their wire write succeeds, and reconciled
  against responses from the same connection generation. Webseed blocks are
  reserved before fetch and committed after exact response validation.
  Duplicate or stale blocks cannot enter storage; every lifecycle exit
  releases ownership.
- **Memory bound:** connection slots and per-piece ledger state are charged to
  `MemoryClass::PeerBuffer`; per-request response capacity remains charged by
  its existing lease. The picker stays independently charged to PieceIndex.
- **Rollback:** remove the actor adapter and restore picker-owned request
  admission. No resume format or wire protocol change is required.

### Acceptance

- v1 uses the ledger for reserve, wire commit, response, Reject, Choke,
  timeout, disconnect, and piece completion/failure. Pure-v2 uses it for
  reserve, wire commit, response, Reject, Choke, 60-second timeout expiry,
  disconnect, and piece completion/failure.
- Serial v1 webseed requests reserve before fetch, commit after a validated
  response, and release on fetch failure, cancellation, or response handling;
  temporary connection and piece-state allocations are charged to
  `MemoryClass::PeerBuffer`. One transient ledger slot is reserved beyond the
  configured peer cap; peer admission itself is unchanged.
- Restored partial blocks occupy ledger received slots before live acquisition;
  a full or empty/malformed partial bitmap is not treated as verified, and
  failure to admit its state rolls the picker projection back so those blocks
  are requested again.
- Hybrid v1/v2 views arbitrate the same payload without duplicate writes or
  mismatched piece geometry.
- Peer request, ledger connection, piece state, source jobs, rate queues, and
  upload obligations stay within measured resource caps.
- Simulator and actor regressions pass, then matched v1/v2/hybrid live
  workloads show correct payload and lifecycle parity before the reference
  paths are removed.

### Evidence

- **Regression sources:** partial-resume ledger completion and verification
  boundary, padded hybrid block-range projection and zero-fill,
  reverse mapping from canonical ledger extents to v2 wire actions,
  ledger failure/expiry cancellation, v1 actor lifecycle, webseed ledger
  ownership, and pure-v2
  reservation/response/action-routing/timeout sources are present; hybrid
  dual-view cases remain open.
- **Command or probe:** run transfer-core simulator and engine regression
  suites only during the owner-requested bulk pass.
- **Executed on exact revision:** no. No build or test commands were run for
  this slice.
- **External evidence:** matched live protocol workloads and efficiency
  comparisons remain pending.

## NET-115 — Unified source scheduling and batched progress

- **Priority / wave:** P1 / 12
- **Prerequisites:** NET-106, NET-107, NET-108, NET-114, SYS-009
- **Owning component:** v1 and pure-v2 torrent actors, then the shared transfer
  scheduler.
- **Status:** `wip`

### Scope

Schedule peer and webseed work through one bounded ownership and rate-control
path, then publish transfer progress in bounded batches. The current v1
webseed implementation is a serial no-peer fallback. It now maps multi-file
blocks to bounded per-file HTTP ranges and synthesizes BEP 47 padding locally;
pure-v2 still has no webseed execution. v1 already batches progress at its
five-second dirty-stat flush, and pure-v2 now batches upload/download
statistics at the existing five-second runtime flush. Completion and
lifecycle boundaries flush immediately.

### Ownership and invariant

- **Current owner:** peer tasks and the v1 webseed fallback own source work;
  the pure-v2 actor receives verified-piece transfer totals and persists them
  through `DbWorker`.
- **Target owner:** the shared scheduler owns source admission, request
  ownership, cancellation, rate credit, and bounded retry across peers and
  webseeds. Torrent actors retain verification, storage publication, and
  persistence ordering.
- **Invariant preserved:** every admitted source request has one owner until
  response, cancellation, or failure. Progress totals advance only after the
  corresponding database transaction succeeds; a failed transaction restores
  the registry projection and leaves its pending counters available for retry.
- **Must not:** let one source bypass the aggregate or torrent rate budget,
  double-count retries, or hold actor lifecycle commands behind a long source
  wait.

### Acceptance

- Peers and supported webseeds share bounded per-torrent and process-wide
  source admission, rate budgets, and cancellation behavior.
- Source selection preserves protocol geometry, file priorities, and picker
  ownership while allowing useful sources to make progress concurrently.
- Transfer events coalesce into bounded progress publications; completion,
  pause, storage-move, recheck, and shutdown boundaries flush pending totals.
- A failed progress transaction restores the prior registry projection and
  retains each pending source total for exactly-once retry.
- Regression sources cover concurrent peer/webseed ownership, rate waits,
  cancellation, per-file webseed mapping and padding synthesis, progress
  coalescing, retry, and lifecycle flushes.
- Matched live-source workloads show no missing or duplicate payload and
  bounded progress-publication latency.

### Current slice evidence

- **Regression source added or updated:** pure-v2 progress batching and
  database-failure retry in `torrent_task_v2.rs`; multi-file URL/range mapping,
  BEP 47 padding, and excessive file-fanout bounds in `torrent_task.rs`.
- **Command or probe:** run with the requested bulk engine suite; do not run
  this source independently before that pass.
- **Executed on exact revision:** no.
- **Remaining:** concurrent peer/webseed shared scheduling, pure-v2 webseed
  execution, live-source workloads, and the bulk pass.
