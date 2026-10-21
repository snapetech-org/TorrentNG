# TorrentNG native transfer architecture changes

This package changes the native Rust engine and storage path. The work was selected by tracing ownership, allocation, admission, and scheduling in the source. It contains no new client rankings or CPU/RAM benchmark claims.

Base: `7c2dd481ca224da8f5e47b783678b6110146ba9b`, also the remote main revision checked during this review. Changes are local to the review branch; they have not been pushed.

## Apply one patch

- **Clean checkout of the base:** apply `torrentng-complete.patch`. This includes the previous ZIP's batching/cache-retirement fixes and benchmark report checkpoints, plus this review's native changes.
- **Base with the previous `TorrentNG-native-fix-7c2dd48.zip` patch already applied:** apply `torrentng-architecture-only.patch`. This contains only the new work. It does not change the benchmark harness.

Do not apply both. Complete modified files are in `source/`. The two patch applicability checks are recorded in `validation.json`.

```sh
git apply --check /path/to/torrentng-architecture-only.patch
git apply /path/to/torrentng-architecture-only.patch
cargo test --locked -p rt-piece-map -p rt-storage -p rt-engine --lib
```

## Why the earlier changes could leave CPU high

Rust does not remove work that the program explicitly asks for. Serving 16 KiB blocks means up to 65,536 request units per GiB. A pooled payload can still carry an allocated region list, cloned path components, several temporary collections, an independently allocated ownership wrapper, scheduler admission, a worker handoff, and repeated metadata checks.

The previous fixes concentrated on payload allocation sizes, correctness of accounting, cache retirement, and combining requests that can be served together. Those are useful, but a cache hit still took the physical-device admission path and visited the descriptor catalog. Batching also left multiple heap-backed collections around every small batch. Downloads performed a linear search through torrent metadata for a file that the piece map had already identified.

Confidence is **high** that these operations existed and the new paths remove them. Confidence in the resulting whole-client CPU/RAM ranking is **unknown**; these correctness checks do not establish that ranking.

## Changes in this review

| Native path | Change | Work removed |
| --- | --- | --- |
| Portable pread cache hit | Take an owned payload view, upgrade its original descriptor's weak reference, and check the path identity once. | Device-slot acquisition/release, descriptor catalog lookup and LRU maintenance, a fresh operation-handle allocation, and the duplicate identity check. |
| Contended cache miss | Return an owned continuation to async device admission, then resume on a worker. | Blocking a disk worker while waiting for a device slot. No descriptor is retained during that wait. |
| Non-cacheable reads | Keep device admission before worker dispatch. | Avoids introducing the continuation round-trip for operations that cannot hit the cache. |
| File paths | Move already-absolute paths into preparation; normalized catalog entry points borrow the key. | Repeated path allocation/normalization on the scheduler path. |
| Request mapping | `PieceMap::request_regions` borrows immutable metadata; upload spans store a file ordinal. | Region `Vec`, deep path clones, repeated validation in span preparation, and one resolved path per candidate block. A batch resolves one path when it actually reads. |
| Download writes | Use the mapped file's path and full length directly. | The second, linear metadata file search on every block. |
| Upload batches | Inline collections for at most four specs, panic requests, blocks, completions, ready responses, wire views, and read permits. Remove the intermediate list of `Result` values. | Separate heap storage for those bounded lists. `smallvec` was already in the lockfile; it is now a direct engine dependency. |
| Upload ownership | Hold a single block's memory lease directly; use a shared lease for multi-block backing. | An unnecessary `Arc` allocation for an unbatched block. |
| HDD scheduler lifetime | Construct the elevator task/channel only when an eligible read uses it. Check that route before accessing its state. | An unused task and queue per applicable scheduler, and elevator mutex access on cache/read-ahead paths. |
| Storage admission labels | Format resource-error mount labels only when needed. | Successful admissions no longer allocate that error label. |

## Correctness constraints retained

Cached bytes must still belong to the current file. The weak source reference is important: comparing inode numbers alone after closing the original descriptor could accept a recycled inode. A cache hit temporarily upgrades the original descriptor before the path check. If that source has closed, the read takes the normal admitted backend path. The cache does not pin descriptor quota or reopen a file merely to qualify for the cache fast path.

The pathname check runs on a worker, outside the payload-cache mutex. A replaced path falls through to secure opening and a fresh read. A cache hit keeps its existing backing-memory charge even if consuming it retires the cache entry.

Uncontended misses keep one worker turn. Contended cacheable misses need an additional handoff; they retain bounded mount/byte credit but hold no descriptor while waiting. They rejoin the existing owner/class/priority queue. Cancellation drops that continuation and its admission resources. Existing secure opens, device fairness, write epochs, byte limits, and batch admission remain in force.

Borrowed mapping validates the complete request before exposing regions. It retains padding, cross-file, short-final-piece, zero-length-file, and overflow handling. Upload ordinals are positions in the immutable piece map, distinct from arbitrary file IDs.

Inline collections trade several small allocations for bounded inline state in futures. That is an allocation reduction, not a promise that every process-memory metric decreases. Cache entries also carry a weak source reference. The lazy elevator change removes an actual retained task/channel for schedulers that never use that route.

## Validation

`validation.json` records final results and source/binary identities. `validation/` contains the full successful logs. New regression coverage exercises:

- A cached read while the physical-device slot is occupied, with no device sequence advance or descriptor-catalog hit.
- A contended miss with the sole pread worker still available and no file opened before admission.
- Cancellation releasing the device waiter and queued resources.
- File replacement while a read waits for admission.
- Cached bytes not retaining an idle descriptor, and a closed source forcing a backend read.
- Borrowed mapping across every valid small request range in layouts with padding, empty files, arbitrary IDs, and piece boundaries; invalid and overflowing requests fail before iteration.

The native wire check launches the actual `torrentngd` binary, adds a private local torrent, and downloads 8 MiB through BitTorrent peer messages using windows of 1 and 32 requests. It verifies all block coordinates, rejects duplicate/unsolicited blocks, and checks SHA-256. It collects no CPU or memory measurements. The local build uses Rust 1.94.0 on GNU Linux with LTO disabled and 16 codegen units to keep this correctness cycle short; it is not the production image build.

```sh
cargo build --release --locked -p torrentngd
python3 reproduce/native_wire_check.py --binary target/release/torrentngd --output /absolute/new-wire-check
```

## Scope and remaining cost

This pass leaves the prior bounded coalescing and cache-retirement fixes intact. It does not enlarge caches, raise parallelism, weaken hashing, or lower resource limits to obtain a result.

The portable worker submission still uses a boxed job, a completion channel, and cancellation state. Cache hits still cross that worker boundary so filesystem validation does not block a Tokio worker. Removing that remaining per-read handoff would require explicit reusable request/completion slots or a different file-generation ownership protocol. It is separate work, not an optimization claimed by this patch.

No comparative benchmark was used to choose or certify these changes. The package documents concrete removed operations and tests the resulting behavior.
