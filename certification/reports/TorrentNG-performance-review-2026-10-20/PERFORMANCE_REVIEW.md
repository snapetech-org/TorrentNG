# TorrentNG: where I would change CPU, RAM, and I/O behavior next

## Assessment

**Recommendation confidence: moderate.** Refactor the work done for each payload
block. Current source still contains concrete task, channel, queue, buffer, and
metadata costs that the recent accounting and cache changes did not remove.
The possible end-to-end improvement is **unknown**.

This review examined TorrentNG main at
`8bd30a474b72309a4b95327f32bbd89caeb590dc`, including upload-accounting batching,
peer-owned upload-read futures, and the 8 MiB peer-read-cache default.
The source was inspected without modifications. No new build or transfer
benchmark was executed during this review.

The latest examined five-client seed pass completed and all seed transfers
passed their payload hash checks. Its reported CPU seconds per GiB were:

| Client | Seed CPU seconds/GiB |
| --- | ---: |
| TorrentNG | 3.84 |
| qBittorrent | 2.33 |
| Transmission | 1.47 |
| Deluge | 2.54 |
| rTorrent | 0.72 |

These are historical, controlled-load observations on a shared host. The
TorrentNG accounting candidate measured 3.84, 3.72, and 4.53 CPU seconds/GiB.
Its baseline measured 3.74, 3.98, and 4.52. The ranges overlap substantially:
the candidate has no demonstrated CPU win.

## 1. Isolate extra work in transfer measurements

**High confidence in the harness behavior; moderate confidence in the proposed
explanation for the RAM spikes.**

The transfer benchmark leaves TorrentNG's container healthcheck active:
`curl -fsS http://127.0.0.1:8080/health` every 10 seconds. The new idle-only
benchmark mode disables healthchecks, but transfer mode does not apply that
override. Healthcheck processes execute inside the measured container.

Transfer sampling also requests TorrentNG's full `/metrics` response at an
approximately five-second interval. Filtering memory metric names occurs after
the daemon has rendered the response. The endpoint can request a background
engine-statistics refresh, which includes database and child-task work.

The three candidate seed profiles each contain one sample where cgroup
anonymous memory exceeds the daemon's anonymous RSS by more than 1 MiB.
In the full pass, the exceptional sample reports about 13.04 MiB cgroup anon
versus 6.42 MiB process anon. Detailed `smaps` agrees with process RSS.
The neighboring samples return to close agreement.

The healthcheck is a plausible source of a brief extra process and its resource
use. The saved samples do not identify the responsible process at that instant,
so this remains a hypothesis. Do not attribute the entire CPU deficit to it.

**Change:** use equivalent healthcheck policy for all clients during transfer
measurements. Keep core comparison windows free of extra TorrentNG-only metrics
scrapes. Collect process-tree memory consistently and capture process identities
when cgroup/process readings diverge. Run detailed diagnostics separately.

**Files:** `scripts/client_performance_benchmark.py`,
`deploy/interop/compose.yml`,
`crates/rt-api-native/src/handlers.rs`.

## 2. Move completion ownership into the backend job

**High confidence in the code cost; moderate confidence in optimization value.**

`backend_completion_with_fence` creates a second oneshot channel and spawns a
Tokio task to await the backend's original completion receiver. Its write
counterpart performs the same forwarding and cache invalidation.

This remains in current source even though outer upload-read work now runs as
peer-owned futures. On an uncoalesced 16 KiB read path, one forwarding task per
backend read corresponds to 65,536 forwarding tasks per GiB. Caching and
coalescing can reduce the number of backend reads.

Before backend submission, `read_owned_at_with_priority` performs preparation
through `BlockingPool::run`. That helper creates another channel, cancellation
state, and queued closure. The caller resumes after preparation and then submits
the actual read. A peer-read-cache hit also goes through the preparation worker.

Post-read identity validation can perform filesystem metadata work on the async
runtime thread. The checks protect against path replacement; their placement
and repetition deserve redesign without weakening that contract.

**Change:** make the actual backend operation own its resource credits, permits,
fence, payload, and completion cleanup until I/O really finishes. Return one
completion directly to the requester. Consolidate preparation and submission
where possible, and keep blocking identity work off the async runtime.

The forwarding tasks currently serve a correctness purpose. Removing a spawn
without transferring its ownership obligations would allow cancellation to
release resources too early or leave cache invalidation incomplete.

**Files:** `crates/rt-storage/src/scheduler.rs:2033–2071, 3742–3981`,
`crates/rt-storage/src/backend.rs`.

## 3. Make immediate upload admission cheap

**High confidence in behavior; moderate confidence in benefit, especially with
many registered peers.**

`UploadReadScheduler::acquire` allocates a oneshot and enqueues a waiter even
when a permit can be granted immediately. Admission and permit release each
lock shared state, then call a pump that locks it again.

The pump rotates through a ring containing every registered connection,
including connections without queued requests. Work therefore grows with
registered connections even when relatively few have pending reads.

The storage device queue already has an immediate-grant branch, but creates
its channel before deciding whether that branch applies.

**Change:**

- Grant directly under one lock when capacity and fairness allow.
- Allocate a waiter only when a request really must wait.
- Maintain a queue of connections with pending demand.
- Avoid repeatedly visiting empty connections.
- Defer device-queue channel allocation to the waiting branch.
- Preserve process/per-connection limits, fairness, cancellation, and
  connection-generation safety.

**Files:** `crates/rt-engine/src/transfer/upload_scheduler.rs:162–350`,
`crates/rt-storage/src/scheduler.rs:2251–2311`.

## 4. Keep payload ownership through transmission

**High confidence that the copies exist; moderate confidence in the proposed
refactor; end-to-end saving unknown.**

`read_upload_block` allocates an assembly Vec and copies the storage result
into it. Encoding `Message::Piece` then copies the payload into the framed
output buffer. A piece is sent individually through `framed.send`.

The previous single-region shortcut did not demonstrate an overall CPU win.
An effective next experiment needs to cover the complete lifecycle from read
buffer to socket completion.

**Change:** carry a leased payload buffer or buffer range to the send path.
Use a small owned header plus payload for vectored TCP writes. Batch already-ready
messages within explicit byte and delay limits. Keep fallback assembly for
cross-file and padding cases, and preserve uTP behavior.

The write cursor must handle partial headers, partial payloads, cancellation,
choke transitions, rate-grant settlement, and buffer lifetime. Memory credit
must follow the backing allocation until the final consumer releases it.

### Conditional io_uring work

When fixed buffers are available, the backend receives an already allocated
ordinary frame, selects a registered slot, reads into that slot, then replaces
the ordinary frame backing on completion. Choosing the final buffer earlier
can avoid an unused allocation.

Registered-file slots are also removed when their in-flight reference count
falls to zero. At low queue depth, repeated read bursts may register and clear
the same file repeatedly. Measure actual backend selection and registration
activity before redesigning this cache.

**Files:** `crates/rt-engine/src/torrent_task.rs:8170–8179, 9595–9672`,
`crates/rt-peer-wire/src/message.rs:171–176`,
`crates/rt-storage/src/backend.rs:1020–1078, 1145–1220, 1240–1275`,
`crates/rt-storage/src/frame.rs:265–303`.

## 5. Budget retained buffers and avoid redundant clearing

**High confidence in the retention bound and clearing behavior.**

The frame-pool admission cap tracks active allocations. Free lists can retain
256 buffers in each of three size classes: 16, 64, and 256 KiB. Their combined
possible idle retention is 84 MiB. This is a high-water bound, not the observed
footprint of the measured seed runs.

On return, a pooled buffer is cleared and resized with zeroes across its size
class, even though a later successful positioned read may overwrite it all.

**Change:** make active and retained allocations share a resident-memory budget.
Trim retention under pressure and after workload decay. Introduce a read-specific
reuse contract for buffers guaranteed to be fully overwritten before their
contents become visible. Preserve zero-initialized semantics for partial writes,
padding, assembly, and any callers that depend on them.

Also avoid treating `Frame::into_bytes` as deallocation: that operation can
transfer an allocation to another owner after releasing the frame-pool charge.
Audit the replacement owner's accounting across the entire lifetime.

**File:** `crates/rt-storage/src/frame.rs:24–163, 265–303`.

## 6. Test a small piece-assembly budget for downloads

**High confidence in the configured path; unknown net CPU/RAM tradeoff.**

The shipped native configuration sets `piece_assembly_cap_mb = 0`. The download
path therefore writes blocks individually and verifies completed pieces from
storage. Verification reads may hit the page cache, but still require work.

**Experiment:** compare zero assembly with a small, globally bounded assembly
budget. Hash complete pieces in memory and write larger ranges where supported.
Retain the direct-write fallback for pressure, eviction, and restored partial
pieces. Evaluate physical I/O, logical I/O, CPU, peak memory, and post-load
retention together.

Public-swarm download performance is a separate unresolved concern. Reproduce
it with a controlled seeder before assigning the cause to hashing or storage.

**Files:** `deploy/native/config.toml`,
`crates/rt-engine/src/torrent_task/peer_transfer.rs`,
`crates/rt-engine/src/torrent_task.rs:2632–2732`,
`crates/rt-storage/src/verify.rs`.

## What transfers from slskR

The strongest transferable examples eliminate work:

- Share-cache serialization: 408,828 allocator calls became 20 on its measured
  50,000-entry fixture, with identical output.
- Search-result persistence: appending accepted batches inserted 10,000 rows;
  repeatedly rewriting the accumulated result inserted 255,000.

These are path-specific diagnostics. slskR's whole-client comparisons measured
idle workloads and do not establish active-transfer efficiency.

The TorrentNG equivalent is reducing operations per payload block while
preserving the same protocol and persistence behavior. Accounting batching
addresses one part of this work. Storage completion, admission, and buffer
ownership remain larger structural opportunities.

## Recommended sequence

1. Correct transfer measurement isolation and source/backend identification.
2. Refactor backend completion ownership.
3. Add immediate admission and queues containing pending demand.
4. Carry payload buffers through transmission.
5. Unify active and retained buffer accounting.
6. Evaluate bounded download assembly as a separate experiment.

Keep each candidate separately measurable and reversible. The current read
volume is already roughly one payload pass in the main seed result. Increasing
cache size or adding read-ahead should require evidence of useful reuse.

