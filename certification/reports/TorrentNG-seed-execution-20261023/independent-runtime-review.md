# Deferred operation experiments — read-only review

Frozen runtime reviewed: 9c33; do not implement until current combination results warrant another build cycle.

## Candidate 1: remove duplicate mapped-view PeerBuffer credit

`crates/rt-engine/src/torrent_task/upload_pipeline.rs:379–410` reserves PeerBuffer bytes before mapped preparation and retains them in the mapped payload variant. `crates/rt-storage/src/mapped_upload.rs:333–352` also charges the whole-page Window to PeerBuffer; MappedUploadRead holds Arc<Window> through send completion. No second payload allocation exists for the view.

Necessary proof before removing any credit: Window must carry a live lease from the SAME resource governor as upload.resources. Storage resources are optional; successful mapping alone is not this proof. Use explicit governor identity (ResourceGovernor::reclaim_scope_id), and preserve original accounting when resources are absent or different. Keep global upload read job/byte admission, mount/worker queues, mapped window/global cache limits, all generation and worker pathname checks. Acquire the original peer buffer credit before buffered fallback allocation. Do not add a new per-batch shared-lease allocation to eliminate the old accounting operation.

Tests: same-governor live whole-page charges; no release while any read owns the Window; absent/different-governor rejection or conservative lease; cancelled or timed-out reads; denied mlock; cache eviction with live send; bounded multi-peer demand under memory pressure; buffered fallback still charged. Removing conservative duplicate credit changes admission pressure and must be explicitly validated, not called behavior-neutral.

## Candidate 2: lazy mount-name construction

`scheduler.rs:4530` formats storage-root-N before calling DeviceQueue::acquire_with_priority. The latter already has an immediate no-wait arm (`~3053`), so this is not an async-wakeup elimination. Try existing try_acquire_uncontended first, then construct mount name and enter normal admission only on contention. Its pending_count check preserves queued class/owner fairness. Slow path pays an extra mutex attempt: test contention and cancellation, and reject on a joint score regression.

## Rejected shortcut

Do not skip mapped physical-device admission simply on mapped-window cache hit. `Window::prefault` still calls ordinary mlock for each requested range: pages may be previously untouched or externally invalidated. A generation/identity match is not a no-I/O proof. Buffered prepared-cache hits already bypass physical device admission after worker-side checking (`scheduler.rs:2307–2378`). The diagnostic 166 hits/122 misses are buffered cache counters, not mapped-window counters.

## Causal gate

Record same-workload per-batch governor acquisitions and mount-name allocations, then balanced CPU/PSS. Reduced operation counts without joint whole-client benefit are a stop condition. Neither operation has a measured budget that promises parity.

## Final runtime PR review

Read-only review of 7cb3..9c33 found no new blocking runtime correctness issue in freshness generation/age, frame-pool observation, forwarding/capture changes, COW state, or mlock refinement. Snapshot fencing is conservative and asynchronous, not atomic across all counters. COW preserves shared lock stores and clone-local configuration assignment; replacing owned structs with Deref handles changes Rust source compatibility for clients moving public owned fields, which now require borrow/clone. HTTP semantics remain unchanged. External file mutation remains non-atomic; mlock prevents ordinary reclaim but not backing revocation. No edits or builds were made as part of that final review.
