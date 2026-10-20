# Implementation handoff

## Goal and starting point

Reduce CPU seconds per verified GiB and resident anonymous memory while
maintaining throughput, protocol compatibility, fairness, integrity, and crash
recovery. Use the findings in PERFORMANCE_REVIEW.md as hypotheses to implement
and test individually.

Reviewed TorrentNG commit:
`8bd30a474b72309a4b95327f32bbd89caeb590dc`.

Before editing a newer checkout, read its applicable AGENTS.md instructions,
inspect local changes, and locate these functions again. Preserve unrelated
work. Source line numbers in this package refer to the reviewed snapshot.

Existing current-source changes include accounting batching, an 8 MiB peer-read
cache default, and peer-owned read futures. Do not present those as new work.

## Stage A — make the comparison diagnostic

### Implementation

- Apply an explicit, consistent healthcheck policy to all measured services.
- Keep readiness checks before and after transfer measurement.
- Remove extra TorrentNG-only metrics scrapes from comparative measurement.
- Retain a separate diagnostic mode with full counters and detailed process maps.
- Collect process-tree memory for all clients; preserve distinctions among RSS,
  PSS, anonymous pages, page cache, kernel memory, and Docker working set.
- Record selected storage backend and fallback reason, worker counts, cache and
  assembly budgets, allocator, source commit, dirty-tree fingerprint, and image ID.
- Remove the stale report explanation referring to the already-removed
  requests-per-interval upload guard.
- Use the same verified payload and equivalent load for every client.

### Acceptance

- Every successful transfer must pass the final payload hash.
- Show the effective measurement configuration, including healthchecks.
- Capture process identity when investigating transient cgroup-only memory.
- Keep original raw samples and failed runs with their failure reasons.
- Do not retroactively subtract an estimated healthcheck cost from old results.

## Stage B — completion ownership in actual backend operations

### Implementation

Trace read, write, and sync submissions and their cancellation paths. Replace the
extra forwarding task/channel with completion ownership retained by the actual
job or pending kernel operation. Ensure that the backend can complete cleanup
even after the requester drops its receiver.

Preserve post-write cache invalidation on successful, failed, and cancelled
caller paths as required by the current contract. Preserve permits and resource
credits through real completion. Integrate preparation and dispatch where doing
so reduces handoffs without blocking unrelated async work.

### Required regression cases

- Caller cancellation before dispatch and during in-flight I/O.
- Receiver drop while kernel work still owns buffers and file handles.
- Queue full, backend failure, short read, short write, and shutdown.
- Read/write overlap, cache epoch changes, and atomic path replacement.
- Actual operation lifetime remains bounded and accounted.
- Every release occurs once; no premature reuse of payload buffers or file slots.

Use the existing scripted backend and cancellation tests before introducing
new machinery. Add focused tests only for behavior changed by this refactor.

### Measurements

Backend operations/GiB, forwarding tasks/GiB, allocated bytes/GiB, CPU
seconds/GiB, scheduler wakeups/context switches, throughput, and memory peaks.
Demonstrate that the intended forwarding hop has been removed.

## Stage C — immediate admission and pending-demand scheduling

### Implementation

Grant upload-read permits directly when limits permit and older eligible
waiters are not bypassed. Create the waiter and reply channel only on contention.
Track connections with queued demand and explicitly update that membership on
enqueue, cancellation, limit transitions, permit release, and unregister.

Defer device-queue waiter allocations until queuing is necessary. Consider
precomputed mount identifiers where the current success path formats strings.

### Required regression cases

- Process and per-connection job/byte limits.
- Fairness under sustained demand from several peers.
- Eligible peers progress when other peers hit their per-connection cap.
- Queued cancellation and failed reply delivery.
- Connection removal/replacement and generation safety.
- No recursive mutex acquisition when failed sends return RAII permits.
- Exactly-once credit release.

### Measurements

Separate one-peer, many-idle-peer, and many-demanding-peer workloads. Count
admission allocations, lock contention, queue visits, and CPU/GiB. The low-peer
benchmark alone cannot quantify the scaling benefit.

## Stage D — payload lifetime from storage to socket

### Implementation

Introduce an owned payload representation that can retain a frame, registered
slot, or immutable buffer range along with its memory credit. Support a header
plus payload write cursor rather than flattening every piece into a new buffer.

Use bounded batching only for messages already ready to send. Do not add an
unbounded delay or queue to manufacture large batches. Keep cross-file/pad
assembly and uTP support explicit.

If io_uring is selected and fixed buffers are active, acquire the actual backing
storage before allocating an unused ordinary frame. Retain a portable fallback.

### Required regression cases

- Partial writes at every header boundary and within payload.
- Connection close, write timeout, cancellation, and choke transitions.
- Allowed-fast and rejection obligations.
- Correct rate-grant settlement/refund and upload-byte accounting.
- Buffer and memory-credit lifetime through queued and partial writes.
- Single-file, cross-file, padding, hybrid/v2 projection, and uTP fallback.
- Final payload hash equality.

### Measurements

Payload copy bytes/GiB, allocation counts/GiB, socket writes/GiB, CPU/GiB,
throughput, resident memory, and tail response latency. Verify that reduced
copies do not create excessive retained read-ahead buffers or registered slots.

## Stage E — bounded resident buffer retention

### Implementation

Define the budget for active plus idle buffers. Account ownership transfers
consistently when a frame becomes immutable bytes or a payload retained by a
socket. Release memory credit only when the allocation is actually released or
transferred to an equivalent charged owner.

Trim free lists after pressure or load decay. Add a read-specific buffer reuse
path only where successful I/O guarantees full overwrite. Keep ordinary
zero-initialized buffer semantics for other users.

### Required regression cases

- Mixed size classes and high-water retention after a burst.
- Pressure and cancellation while buffers are outstanding.
- Cross-owner accounting through cache and socket lifetime.
- Short I/O never exposes stale payload.
- Padding and assembly paths remain correctly initialized.

### Measurements

Active bytes, retained bytes, actual process/cgroup anonymous memory, denial
counts, allocation churn, CPU/GiB, and memory after transfers stop. A lower
logical counter alone is insufficient.

## Stage F — controlled download assembly experiment

Compare zero assembly with a small explicit shared budget. Retain fallback to
direct writes when memory cannot be reserved. Test partial-piece restore,
eviction/spill, hash failure, multifile pieces, and crash recovery.

Measure completed-piece verification from memory versus storage, backend
write sizes, logical reads, physical reads/writes, CPU/GiB, throughput, and
peak/post-load memory. The best seed configuration need not be the best download
configuration.

## Qualification method

Start with the smallest workload that exercises each changed mechanism, then
use a controlled transfer with a final hash. Compare one change at a time
against the same source/configuration baseline. Alternate or randomize candidate
and baseline order and retain enough repeats to distinguish a stable change
from observed variation.

Use the established capped receiver load for continuity, then a higher-demand
load to expose saturation and queue behavior. Add multi-peer or multi-torrent
cases when the changed code is intended to improve that behavior.

Report:

| Area | Required evidence |
| --- | --- |
| Integrity | Complete transfer and final payload hash |
| CPU | Cgroup CPU time and process attribution; CPU seconds/GiB |
| RAM | Process-tree RSS/PSS, cgroup anon/file, active and retained buffers |
| I/O | Logical operations, backend operations, physical bytes and operations |
| Performance | Verified throughput and latency under the same offered load |
| Mechanism | Relevant tasks, allocations, copies, queue visits, or syscalls |
| Provenance | Commit plus dirty diff fingerprint, image, config, selected backend |

Do not prescribe a percentage improvement before measurement. Reject a proposed
win that is within run-to-run variability, harms correctness, or moves resource
use into an unmeasured owner. Keep an inconclusive outcome explicit.

## Delivery per stage

Provide a small reviewable commit or patch, the changed contract, focused
regression results, reproducible commands/configuration, raw measurements, and a
conclusion with an explicit confidence level. Advance only when the stage's
result is understood. The whole-engine CPU win remains unknown until measured.

