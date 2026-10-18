# rt-storage

`rt-storage` owns storage planning, verification reads, and the per-mount disk
scheduler used by the TorrentNG client.

## Architecture

- `MountScheduler` enforces per-`IoClass` concurrency with separate semaphores.
- `StorageIoConfig` controls the disk layer: file-pool size, idle fd TTL,
  worker threads, queue depth, preallocation, durability, and peer-read
  readahead target.
- `scheduled_read` and `scheduled_write` are compatibility wrappers over
  positioned `read_at` and `write_at`; `scheduled_read_owned` preserves a
  pooled `StorageRead::Frame` for exact backend reads when a caller can avoid
  converting to `Bytes`.
- The path-backed `MountScheduler` file pool avoids per-block fd churn and
  records hits, misses, evictions, idle closes, and current cached-entry count.
  Its configured capacity is capped at 60% of the raised soft `RLIMIT_NOFILE`
  (up to 65,536 entries). A lifecycle lease bounds cached plus in-flight file
  descriptors to each `FilePool`'s capacity; a zero-entry pool permits one
  transient open. `StorageRuntime::HandleCache` has its own local lease cap,
  and both cache types also share one process-level managed-storage lease
  budget. `FilePoolStats.open_files` counts cached entries;
  `active_descriptors` includes cached, opening, and in-flight leases. These are
  per-cache counts; Prometheus also exports aggregate managed-storage lease
  use, capacity, and admission waits. Reads reuse a cached read+write descriptor;
  publishing a writable entry replaces the read-only cache entry, while any
  existing read lease keeps its descriptor charged until the operation ends.
  Unrelated process descriptors remain outside this storage budget.
- Disk operations are submitted through the selected `DiskBackend`, behind
  per-mount bounded queues and one shared `DeviceQueue` per detected physical
  device, instead of Tokio's shared blocking pool. Unknown device identity uses
  a storage-root-scoped service. The service caps active work to one job per
  portable worker or one bounded io_uring submission batch, bounds pending
  admission by the remaining device queue depth, and selects queued classes
  with the weighted round-robin policy from `DeviceElevator`. Operation IDs
  stay attached to their permits through backend completion. Cache hits and
  file-open preparation do not consume a device slot. Scheduler reads, writes,
  syncs, preallocation and extent probes use the service.
  `MountScheduler::run_device_operation` is the bounded handoff for other
  subsystems that need the same mount/device admission and completion
  ownership. Secure plan steps acquire every touched device in stable order.
  Regular-file `CopyVerifyRename` steps move and hash at most 64 KiB per
  admission, releasing device slots between quanta while retaining their
  root-confined file handles and cancellation cleanup. Their touched-device
  queue set is resolved once and reused across quanta. Directory-tree copies
  still run as one admitted operation. On Unix, `ImportExisting` keeps its
  same-device hard-link fast path and uses bounded quanta when it needs a
  regular-file copy. Final sync latency, deadline dispatch, and choke-critical
  dispatch remain open.
- Piece and BEP52 hashing runs on a separate bounded hashing pool. V1 recheck
  streams SHA-1 updates; v2 recheck hashes up to 64 leaves per job and finishes
  the bounded streaming Merkle accumulator on that pool. A shared immutable
  dual-view path for live hybrid piece verification remains open under IO-007.
- `DeviceElevator` captures the remaining per-device Phase B policy: budgeted
  batching, deadline and foreground bypass, choke-critical promotion, offset
  ordering, and adjacent read coalescing. Live device admission now uses its
  class weights and bounded fairness across mounts. HDD peer-read misses still
  use the separate per-mount `PeerReadElevator` batching path.
- `prepare_file` creates parent directories and applies preallocation before
  the first write. The default `Auto` policy resolves from mount/sysfs
  topology: full allocation for rotational non-CoW local storage, sparse
  allocation for SSD/NVMe, network, unknown, and CoW filesystems.
- `sync_data` and `sync_all_open_files` are used by higher layers to make
  fastresume trust conditional on configured durability.
- `StorageIoStats` exposes file-pool, queue, dirty-file, preallocation, sync,
  hash, and bytes-by-class counters for metrics integration.
- `IoClass::PeerRead` can read ahead into a small per-file cache while returning
  exactly the requested block bytes to the caller.
- `MountScheduler` and `StorageRuntime` use a probe-selected disk backend. The
  path-backed `MountScheduler` instances share the bounded file cache, disk
  backend, I/O workers, hash workers for one process/configuration, and device
  admission when topology identifies the same device; per-mount semaphores,
  dirty generations, peer-read caches, and counters remain isolated. Set
  `TNG_STORAGE_BACKEND=auto|pread|uring` to request a backend. `auto` currently
  selects the conservative portable worker-pool baseline, `pread` requests that
  baseline explicitly, and `uring` uses Linux `io_uring` positioned SQEs when
  the kernel allows it, keeps stable per-worker fixed-file slots by file
  identity, returns reads from registered frame slots when accepted, and falls back
  with a diagnostic when it cannot.

## Correctness Expectations

- Reads use `create(false)` and never create or truncate missing files.
- Writes create files only when the caller explicitly allows creation for a
  known torrent file.
- Positioned I/O validates short reads/writes and returns `StorageError`.
- File-pool keys are normalized absolute paths.
- Preallocation policy is resolved once when the scheduler is created; hot-path
  writes receive a concrete mode.
- Preallocation failures are surfaced before the engine marks blocks or pieces
  valid.
- Recursive storage-plan walks for copy, delete, content length, and verification
  stop at 64 directory levels and fail closed; a failed copy removes its partial
  destination. The bound protects worker stacks from adversarially deep trees.

## Tests

Unit tests cover scheduler class isolation, read-missing behavior, write create
policy, fd-pool hit/miss/eviction behavior, sparse preparation, and concurrent
positioned writes on a cached fd. They also assert backend request parsing,
probe-selected fallback behavior, forced `io_uring` roundtrips on kernels that
support it, and storage stats for read, write, sync, and hash work. Engine tests
cover upload block assembly across multi-file regions and fastresume/recheck
workflows.

Run:

```sh
cargo test -p rt-storage
cargo test -p rt-engine
```
