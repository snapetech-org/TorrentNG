# Storage Evidence Contract

Status: design contract for IO-001/IO-005; a test-only scripted backend is
wired into scheduler tests for held write completions, injected failures, and
partial-write errors. Scheduler completion watchers now retain queue/device
permits and queued-byte charges for admitted reads, writes, and syncs if their
caller is canceled. Disk submissions also retain the opened descriptor handle
and an immutable write-payload reference until completion. Dispatch and
owner-drop traces record logical queued bytes, governor-backed queue credit,
shared payload class/lease/view bytes, permit counts, and retained
descriptor/payload owners. Read dispatch also records the actual frame-pool
charge after size-class rounding; coalesced peer reads aggregate their queued
credits and report their one shared frame charge separately from
governor-backed bytes.
The helper-level owner regressions are authored but unrun. Dropping the
scripted backend releases any held test completions to avoid leaving a blocked
helper thread during failure cleanup. Zero-progress read/write error mapping
now has authored scheduler regressions. Path identity replacement is covered
for cached descriptors and readahead entries under IO-004. The identity
captured by secure open now travels with the leased descriptor. On Unix and
Windows, payload reads, writes, and data/checkpoint syncs verify that identity
against the current path at operation boundaries; a mismatch prevents stale
read bytes from returning and leaves dirty write evidence intact. Held-read,
held-write, and held-checkpoint-sync replacement regression sources are
authored but unrun. Frame-pool charge attribution is now present in read
dispatch traces. Stable lease identities, consumer/hash credit reconciliation,
crash cuts, and the remaining fault-matrix cases are still open.

## Evidence states

Every accepted payload range has one explicit lifecycle:

`received → verified → write-complete → servable → payload-durable → resume-committed`

- **Received**: validated bytes are owned by an operation, together with the
  memory lease that covers them. A peer response alone is not durable state.
- **Verified**: the complete protocol-specific piece or leaf passed its hash
  check. No HAVE or resume bit is published before this state.
- **Write-complete**: all positioned writes for the verified range completed
  exactly, or failed with a typed error. Short I/O must continue until exact
  completion or an explicit error.
- **Servable**: the path and file generation still identify the content that
  was written, and the requested range can be read back through the secure
  catalog. This is the minimum state for HAVE publication.
- **Payload-durable**: the barrier covering the range's write sequence
  completed at the configured durability boundary.
- **Resume-committed**: the resume/database record references only content
  covered by the payload durability evidence.

`write-complete`, `payload-durable`, and `resume-committed` are distinct
states. A successful write future is not evidence that data survived a host
crash; a successful sync is not evidence that a later resume transaction
committed.

## Operation identity and ownership

The target record for each asynchronous storage operation includes an
operation id, content epoch, secure file identity and generation, byte range,
write sequence, and capacity lease. The operation must retain its descriptor,
immutable payload owner, and admission permit until the backend reports
completion, even if its waiting future is cancelled. A late completion from a
previous epoch or file generation may release resources and report
diagnostics, but cannot publish content into the current generation.

Current implementation: `LeasedFile` carries the secure-open identity and
descriptor permit together; `DiskSubmission` carries a process-wide typed
`StorageOperationId`, its device-local sequence paired with the stable device
queue key, file identity and content epoch, the write sequence for dirty
payload, and the payload byte range where applicable, plus the per-mount queue
permit, shared physical-device permit, and queued-disk byte charge through
queued open and backend completion watchers. Dirty clearing requires the
admitted record and exact file/content/write-sequence identity. The queue key
distinguishes reused local sequence values from independent physical-device
queues; coalesced peer reads carry the same sequence/key pair plus a process-wide
ID and byte range for tracing. `DiskSubmission` directly retains the opened
descriptor handle and, for writes, an immutable `Bytes` reference until
backend completion. Dispatch and owner-drop traces expose logical queued bytes,
governor-backed queue credit, shared payload class and lease/view byte counts,
and retained permit/owner counts. Coalesced peer reads report the aggregate
queued credits of their retained requests and the size-class charge of their
single shared frame, separately from governor-backed bytes. The device
admission pool is keyed
by physical device identity rather than mount profile; unidentified topology
falls back to a storage-root-scoped pool. Strict writes move the same
submission through their data sync. Reads recheck the descriptor identity
against the current path before returning. Stable lease identities, consumer
and hash credit reconciliation, deterministic credit before/after assertions,
crash cuts, and platform fault qualification remain open.

Dirty evidence is tracked per normalized path, write sequence, and file
identity. A barrier snapshots the dirty files and highest covered sequence for
each file. On Unix and Windows, before clearing a generation, sync verifies
that the open descriptor matches the current path and the identity recorded by
that dirty generation; a replacement leaves the evidence dirty and reports
uncertain filesystem state. A newer write remains dirty. One torrent checkpoint
does not sync unrelated scheduler or torrent files.

## Deterministic fault matrix

The IO-001 harness must be able to pause or fail each transition without
relying on filesystem timing:

| Injection point | Required observation |
| --- | --- |
| Before admission | No descriptor, lease, dirty marker, or completion is retained |
| After admission, before submit | Cancellation releases unsubmitted work exactly once; queued blocking preparation retains its admission until it returns |
| After backend submit, before completion | Caller cancellation does not release the operation owner or hide dirty evidence; descriptor, payload, queue/device admission, and byte credits stay attached to the completion watcher |
| Short read / short write | Exact loop continues; zero progress or terminal error is surfaced |
| Partial write then error | Range is not advertised or resume-committed; affected state remains recoverable |
| Completion after cancel | Late completion releases ownership once and cannot mutate a newer epoch |
| File replacement before read-back | Identity mismatch prevents stale content publication |
| Data sync failure | Dirty generation remains; resume transaction is not advanced |
| Crash after payload sync, before resume commit | Restart rechecks or replays without claiming uncommitted progress |
| Crash after resume commit | Recovery observes only the covered durable range |

Each scenario records the injected event sequence, operation/file generations,
lease and permit counts before/after, completion state, dirty sequences, and
recovered content. Current dispatch/drop traces expose submitted credit and
retained-owner counts; deterministic snapshots and assertions across all
credit classes are still required. Wall-clock sleeps and “usually happens”
race tests do not satisfy this contract.

## Qualification boundary

Portable fault tests establish state-machine and cancellation behavior. They
do not replace real-device durability probes, filesystem replacement tests,
mount-specific recovery, or hardware qualification. The repository's current
portable positioned-I/O backend remains the reference path until the full
matrix and exact-current-commit acceptance gates pass.
