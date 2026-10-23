# Independent Astra architecture review — 2026-10-23

## Main judgment

**A targeted refactor is warranted; a wholesale engine/runtime rewrite is not yet justified.** TorrentNG has made the correct ownership improvements, but its complete-seed path still passes through a general download/upload connection state machine and an asynchronous storage preparation transaction for each batch, including cache hits. rTorrent's relevant advantage is the combination of compact readiness-driven progress, retained chunk ownership, and shared file-backed payloads. It is not simply C++, fewer threads, mmap, or fewer socket calls.

The best next path is to establish an operation and resident-memory budget, qualify the existing mapped backing together with source-matched PGO/layout, then refactor the **physical seed execution state and batch preparation boundary** where those measurements show remaining cost. Keep the existing engine, APIs, resource governor, storage authority and fallback. Do not start by replacing Tokio, deleting correctness checks, enlarging buffers, or consolidating everything behind a global lock.

Confidence is **high** that current measured builds miss the joint goal, **moderate** that the proposed direction targets real architectural differences, and **unknown** that it can close the entire gap. No combination measured so far establishes parity. No credible numerical parity forecast can be obtained by adding the earlier gains.

## Scope and evidence

Reviewed current main `7cb3c216a2473fd6db8ec07f70be209ac4fefe46`, with runtime source unchanged from `7836fa77f4bdd26ac78c5bf436e8d88968bf1f60`; the design document, owned-pipeline report, poll-boundary report, BOLT/layout report, historical seed experiments and archive review; and the actual local rTorrent/libtorrent 0.16.25 source. The comparator commits are rTorrent `15ccae27deb9aa1dc8e46ea0ed218a673ccc5732` and libtorrent `6ad3fecf3e7e7aa86855445587b532a91256f344`. This review performed source inspection and parsed retained smaps; it ran no builds or performance tests and changed no runtime code.

No AGENTS.md was found in the repository or applicable parents. `docs/agent-rules.md` is implementation guidance; its old categorical payload-mmap prohibition conflicts with the subsequently accepted, narrowly bounded experiment documented in `docs/seed-efficiency-pipeline.md`. The latter is the current experimental design, not permission for arbitrary unsafe payload references or blocking work in async tasks. Reconcile those documents before a future implementation PR to prevent contradictory guidance.

Local pinned comparator code is the primary source for the implementation claims below. Paths beginning `comparator-libtorrent/` or `comparator-rtorrent/` refer to sibling checkouts, not files vendored into TorrentNG. Function names are included so references survive line movement.

### What the measurements actually establish

| Evidence | Finding | Confidence and implication |
| --- | --- | --- |
| Owned pipeline, ten paired rounds | Normal candidate passed the 2% joint non-regression gate versus old normal; CPU/PSS intervals crossed equality. Candidate PGO improved both versus candidate normal. | High for that workload; normal structural improvement is not independently established, and old-baseline PGO cell was missing. |
| Matched poll-boundary experiment | Main PGO/rTorrent paired CPU 1.530 [1.319, 1.668], PSS 1.632 [1.556, 1.707]. Main PGO improved both versus main normal. | High scoped gap; no parity. Failed poll extraction is not proof that state-machine redesign cannot help. |
| Independent link-order qualification | Ordered/input CPU .989 [.978, 1.092], PSS .836 [.792, .880]; ordered/rTorrent CPU 1.312 [1.212, 1.512], PSS 1.242 [1.183, 1.298]. | High scoped PSS benefit. CPU non-regression unresolved, anonymous-memory benefit absent, parity absent. |
| Full BOLT rewriting | Old and added text remained resident; default huge-page residency also increased. | High reason to reject that exact output. Link ordering avoids that particular representation cost, not all codegen risks. |
| Historical worker/arena screens | One worker sometimes saves RAM; concurrent CPU regressions. Arena-one has substantial multi-peer CPU regressions. Arena-two evidence is noisy. | Sufficient to reject blanket defaults, insufficient to rule out a structurally different ownership/pool policy. |
| Historical batch/read/socket experiments | Larger batches often retained more data without materially reducing operations; read-ahead, active cache, decoder, lock, socket-coalescing, and preflush variants did not establish a robust joint win. | Good reason to stop repeating isolated tweaks without causal counts. Some controls were the unpromoted preflush-removal artifact. |
| io_uring, allocators, compiler size settings | No qualified joint improvement; actual backend selection was initially misattributed and later corrected. opt-z and full BOLT traded CPU or memory. | Retain current defaults. Backend or allocator labels are not architectural explanations. |
| Existing mmap | Correctness/fallback checks and bounded ownership exist. Older mapped tests and a data-only probe do not qualify the current feature as a whole-client win. | Unknown performance effect for current feature-matched PGO/layout. It is a meaningful untested combination. |
| Current-thread Tokio and lazy workers | Current-thread had a noisy unfavorable pair; lazy worker prototypes either regressed or never exercised worker activation. | Not new ideas and not qualified architectural vetoes. Require changed causal hypothesis before another trial. |

The historical hot-path archives contain source snapshots, not additional patches or independent evidence of an improvement. The main design document's errata supersede the old report's CPU-unit labels and source shorthand. The 8 MiB-era rTorrent PSS figures are not a current matched control.

## What rTorrent actually does

`comparator-libtorrent/src/protocol/peer_connection_leech.cc:PeerConnection<type>::event_write` (around line 582) progresses a small IDLE/MSG/WRITE_PIECE/WRITE_EXTENSION switch. It drains ready work, removes write interest when no output exists, and retains progress when a write cannot finish. `peer_connection_base.cc:up_chunk` (around line 716) obtains throttle quota, writes mapped chunk ranges, charges actual bytes, then advances the remaining piece offset/length. It does not need to recreate an asynchronous read transaction for each adjacent request.

`PeerConnectionBase::load_up_chunk` (around line 315) returns immediately when the retained chunk already has the requested piece index. Otherwise it releases the old chunk and acquires a new chunk handle. Optional preload is conditional on rate, size and time; it is not an unconditional per-block population ritual. `data/socket_file.cc` creates mappings; `data/chunk_manager.cc:allocate`, `try_free_memory`, and `sync_all` account memory, reclaim under pressure and rotate across chunk lists. The important unit is a retained, shared chunk with bounded resource management.

This is not a claim that rTorrent is wholly single-threaded. `torrent/torrent.cc:initialize` creates disk, network and tracker threads; `comparator-rtorrent/src/main.cc` creates SCGI and session threads. Its data-path ownership is more direct despite that threaded application topology. Conversely, TorrentNG already defaults to one Tokio worker (`crates/torrentngd/src/main.rs:tokio_worker_threads_for`); reducing a large async worker pool is not an undiscovered easy win.

rTorrent also writes protocol headers and chunk data in separate states. TorrentNG's vectored writes can already combine headers and multiple payloads. Therefore fewer send calls cannot be assumed to explain the comparator's advantage. A lower-operation implementation can still spend more instructions in admission, task progress, validation and bookkeeping, or retain more runtime state.

There is a safety-contract difference. The inspected retained-chunk fast return does not repeat TorrentNG's per-batch pathname identity fence. The benchmark's stable files do not measure the cost/benefit of that difference. Copy the lifetime and locality ideas; do not silently copy weaker external-mutation semantics to obtain a score.

## Where the current architecture still spends work

### Data path: ownership is improved, execution is still general

`crates/rt-engine/src/torrent_task/upload_pipeline.rs:upload_read_batch_future` acquires rate credit before reading. `read_upload_batch_blocks` validates contiguous extents, acquires global read admission and memory credit, resolves the file path and prepares storage. One completion owns the backing and permits through settlement. These are substantive improvements worth retaining.

`crates/rt-engine/src/torrent_task.rs:run_peer_loop_with_upload_future` (around lines 8890–9976) still serves both download and upload behavior, extension/control traffic, intake/coalescing timers, read completion and socket progress. `UploadReadTasks` (around line 2385) reuses boxed future slots and retains only two common idle slots, so claims of a fresh heap allocation for every read would be obsolete. Nevertheless, the seed connection still carries the general future's state and polls a general select loop. The cost of its large generated symbol is not known from symbol size.

`crates/rt-engine/src/transfer/shared_upload.rs:SharedUploadConnection` already holds a per-connection `Arc<Mutex<SharedUploadConnectionState>>`; it does not perform a global registry lookup for every block. Repeated per-connection locks are worth counting, but bare lock-elision already has weak historical evidence. A meaningful future change would give one protocol owner mutable obligations and expose generation-safe read capabilities, with lifecycle/cancellation tests. Do not claim global obligation serialization as the present bottleneck.

A physical state split means a small `SeedPeerState` owning seed I/O state and a separately allocated/lazy download state; a common lifecycle/control interface; and explicit transitions when verification or mode changes require downloading. It must avoid an inline enum whose size is simply the maximum of both large variants, and avoid retaining a large suspended fallback future alongside the seed future. A shared bounded upload engine should be used by both modes, rather than copying protocol handling into two diverging implementations.

This differs from the rejected poll wrapper: that changed a compiler call boundary, while leaving the logical state and work intact. A true split can reduce resident per-peer task allocations, branches touched, and code working set. It can also add allocation, dispatch and duplicate text, so confidence in a joint gain is only moderate as a hypothesis. Measure future/task allocation sizes and active poll work before and after, and test enough peers to separate fixed executable cost from per-peer savings.

### Storage: even a hit still crosses the worker boundary

`crates/rt-storage/src/scheduler.rs:read_owned_at_with_priority` (around line 4654) creates a prepared job and awaits the reusable completion. `PreparedPread::execute` (around line 2288) performs the read-cache lookup and pathname validation on the worker; only then can it return a cached slice. This is deliberate: synchronous metadata checks may block. It also means cache-hit rate alone is not the fraction of reads that avoid scheduling.

The mapped route is not yet equivalent to rTorrent's retained chunk route. `read_upload_mapped` (around line 4578) acquires mount/class/device admission and submits `PreparedReadJob::Mapped`; that execution obtains/validates the file, performs identity checks before and after cache access, and calls `MappedUploadCache::read`. `crates/rt-storage/src/mapped_upload.rs:read` (around line 214) checks metadata and prefaults even an existing-window hit. A mapping can remove an owned payload copy while preserving almost all per-batch scheduling and validation work.

Two immediate discriminators follow:

1. Count actual worker submissions, cache hits, mapped hits, identity/metadata checks, fallback submissions and completion wakeups per verified MiB. Count backend reads separately from API-level `read_ops`, which include cache hits. A mmap fallback can involve the failed mapped preparation and a buffered preparation; that frequency matters.
2. Separate cache lookup/admission from physical-device admission where safe. Buffered hits already avoid the physical-device permit; mapped hits acquire it before knowing whether they need population. A worker-side ready-range classification could avoid unnecessary device contention while retaining class/mount budgets and filesystem validation. It will not remove the worker hop and is unlikely alone to explain the entire one-peer CPU gap. Define resident-range state correctly: a mapping hit is not proof all requested pages were previously populated/locked.

A smaller preparatory refactor is an immutable file/range identity carried from the piece map into preparation, avoiding repeated path construction/hash work while still resolving and validating the pathname on the worker. `read_upload_batch_blocks` currently resolves the path for every batch. The amount saved is unknown; a stable identity must not bypass safe-path or storage-root checks.

The next structural storage experiment should amortize preparation across already-admitted requests, without enlarging the live payload budget or inventing speculative upload demand. A typed prepared batch can return several validated subranges in one bounded completion. The current contiguous eight-block path already does this for its common case; benefit would need to come from currently fragmented/non-contiguous jobs, duplicate concurrent requests, or repeated metadata/pool work—not merely renaming that batch.

### Direct ready-range hits have a hard correctness boundary

An internal generation detects TorrentNG invalidation; it does not detect arbitrary external rename, replacement or truncation. A descriptor pins an inode, not the pathname-to-inode binding. Inotify-style notification is not an atomic proof and has races/overflow. A ready-hit cache on the Tokio thread therefore cannot simply replace the current worker-side identity check while claiming identical semantics.

Safe near-term savings include retaining the existing fence once per current bounded admitted batch; sharing a completion among simultaneous requests whose validation is genuinely the same operation; and removing redundant in-memory ownership work after a validated completion. Coalescing must define the validation linearization point and maintain range/generation/descriptor ownership for every follower. Do not reuse a completed validation indefinitely for later requests.

A longer-lived validated file session could remove more work, but descriptor-based session semantics would be a product contract change unless an equivalent fencing mechanism is demonstrated. That option should be explicitly separated from the no-correctness-loss goal. No direct blocking stat, file read, prefault or mmap population belongs on Tokio. The current syscall-only mapped exposure and bounded page locks must remain; a safe Rust slice over externally mutable/truncatable file memory is not a substitute.

### Startup/control plane and anonymous memory are a separate budget

`crates/torrentngd/src/main.rs` starts the engine, constructs native and qBittorrent router/state trees, then conditionally constructs Transmission/Deluge facades. `crates/torrentngd/Cargo.toml` default features are system allocator only; full compatibility is not present in the scored default artifact. Removing Transmission/Deluge cannot explain or fix that artifact's gap. Removing native/qBittorrent behavior would fail the useful-behavior condition.

Router construction, schema/DB initialization, restore/recheck, TLS/HTTP machinery and task startup can touch executable pages and leave allocation high-water marks. Those pages can remain resident during a seed transfer even when the API is idle. Link-order evidence makes this an especially credible memory issue. Prefer compact shared API projections, lazy state for unused features, and bounded retention to turning off features or pushing them into an uncounted process. Splitting a control process is only interesting if the sum of all client processes improves with IPC, latency and reliability included.

`crates/rt-engine/src/torrent_task.rs:run` has per-torrent choke (10s), tracker (5s), peer retry (30s) and control retry (1s) timers plus webseed/download machinery. The peer loop has a 5s timeout tick and bounded accounting/coalescing timers. This warrants idle/many-torrent timer and snapshot attribution, but does not prove these low-frequency ticks dominate the 4 MiB/s one-peer case. Disabling trackers in the benchmark is not proof their scheduler branches or timer allocations vanish.

`crates/rt-engine/src/db_worker.rs` has a dedicated DB thread and bounded command processing; `crates/rt-storage/src/runtime.rs` a handle sweeper; storage has its worker pool; startup/lifecycle also uses Tokio blocking tasks. Thread stack virtual reservations are not resident RAM. Lowering stack size does not automatically recover megabytes, and removing threads can concentrate allocator contention or make API/durability work block traffic.

I independently parsed the retained first-round after-transfer smaps. Ordered PGO totals were 13,447 KiB PSS and 6,568 KiB anonymous; its heap was 2,140 KiB anonymous, two unnamed mappings were 1,540 and 1,032 KiB, and a relocated read-only executable mapping contributed 628 KiB anonymous. rTorrent totaled 9,909 KiB PSS and 2,652 KiB anonymous, with a 1,012 KiB heap. These are **post-transfer diagnostics, not active peaks or heap allocation attribution**. The unnamed mappings cannot be called payloads, arenas or stacks without additional evidence. Anonymous memory includes relocation/COW pages; it is not synonymous with live Rust heap.

`scheduler.rs:create_scheduler_resources` separately constructs the general `rt-storage-io` pool, the selected disk backend pool and the hash pool. Prepared reads already use the disk backend directly. Lazy creation of an entirely unused general pool is therefore a different experiment from changing two active disk workers to one. The root review found eight surviving threads in both clients in the retained first-round snapshots; thread count alone is not an explanation. Before consolidating pools, measure resident pages and verify nested work/resource ordering cannot deadlock. A physically smaller seed state may mostly remove zero-capacity vectors in the one-peer case; it is not an explanation for several MiB without allocation evidence.

That evidence supports inspecting allocation origins and retention, not merely lowering payload caps. A 128 KiB per-batch buffer saving cannot by itself be asserted to close a multi-MiB anonymous gap. Conversely, a narrow seed future split may help many-peer scaling much more than the fixed one-peer baseline. Measure fixed and per-peer components separately.

## Combinations: which are justified to test

| Combination | Why test or defer | Required control |
| --- | --- | --- |
| PGO + LLD order + existing mapped backing | Targets executable residency and payload representation through different mechanisms; current mmap still has scheduler cost. Credible, untested interaction. | Feature-matched binary with mmap runtime off/on, and ordered/unordered builds trained from matching inputs. Do not reuse default-feature order/profile blindly. |
| Physical seed state + ordered PGO | May reduce both live state and touched hot code; ordering may hide or amplify source effect. | Same source/build factorial or source-matched normal and PGO controls; retrain layout. |
| Shared/coalesced preparation + mapped backing | May remove scheduling that mmap alone retains. Most valuable if counts show duplicate or fragmented preparation. | Keep range/budget limits fixed and include random/multi-file/competing peers. |
| Lower allocator arenas + fewer workers | Prior single-peer savings and multi-peer regressions can interact adversely; interrupted combined run is no result. | Defer until allocation ownership changes justify a new hypothesis. |
| Current-thread Tokio + cold-control separation | Only reconsider if attribution shows executor handoff/idle topology is material; earlier current-thread pair was unfavorable. | Same useful services, all threads/processes counted, API responsiveness and mixed load. |
| Bigger batches + fewer workers + mmap | Multiple unproven knobs can obscure which resource pays; bigger batches already failed. | Do not begin here. Fixed memory/service bounds and operation counts first. |

No percentage in one row can be added to or multiplied by an earlier improvement. PGO may optimize away costs a source refactor targets; mappings can reduce anonymous memory while raising file-backed PSS; fewer jobs can alter batching and latency; reduced worker count can save allocator residency but create contention.

## Ordered implementation-ready experiment sequence

### 1. Establish the remaining budget before another runtime patch

**First repair diagnostic freshness.** `scripts/native_seed_cpu_ram_probe.py:collect_diagnostics` takes one metrics response per named snapshot. `rt-api-native/src/handlers.rs:metrics` requests engine stats, while `rt-engine/src/engine/read_model.rs:request_engine_stats` returns cached/initial stats and refreshes asynchronously. A response labeled after-transfer can therefore describe an earlier phase. Add a bounded timestamp/generation-aware freshness barrier to diagnostic collection, outside scored windows, preserving the API's nonblocking design. Do not infer upload operation counts from the archived snapshot labels until freshness is established. This limitation does not invalidate OS-derived process CPU/PSS or verified transfer scores.

Use the exact current PGO and opt-in ordered artifacts plus pinned rTorrent. In separate diagnostic runs, collect per-thread CPU, task polls/wakes, storage submission/completion counts, syscalls by purpose, cache/mapped misses, allocations by origin/lifetime, and mappings at empty-ready, loaded/rechecked-idle, active seed, post-transfer and sustained idle. Include startup and recheck CPU separately. Avoid active diagnostics in scored runs.

Use at least two offered rates and a fast/unpaced case, and 1 versus several peers. At 4 MiB/s, 16 KiB represents about 3.906 ms and eight blocks about 31.25 ms—close to the current 32 ms collection bound. That arithmetic is not evidence of actual request arrival timing, but makes rate/window sensitivity especially important. The existing held-out corpus already spans rates/windows; the recent qualifying scores exercise only one paced shape. A roughly fixed CPU-per-second component will inflate CPU/GiB at low rate; per-byte transport work behaves differently. Vary duration to reveal warmup/retention and recurring jobs. Report all raw costs rather than subtracting an idle baseline out of the whole-client score. This is a minimal way to distinguish a control-plane floor, worker round-trip cost and transport-byte cost.

Success criterion: explain the largest measured anonymous mappings/allocation classes and identify whether worker/poll counts materially exceed useful read/write work. If no large removable category appears, do not launch a runtime rewrite based on symbol size. Historical libc and broad poll samples are clues, not a causal operation budget for current PR233.

### 2. Qualify the existing representation/layout combination

Before implementing a second mapped design, train/build the current seed-mmap feature with matching PGO and layout provenance. First use one feature-enabled artifact with runtime mmap off/on to isolate activation; compare feature-off default separately because merely compiling the variant can enlarge futures/text. Compare ordered and unordered feature-matched builds to detect interaction. Include the pinned comparator in fresh balanced rounds.

Start with the known warm workload, then cold-file hints, random requests, 3+ peers, unaligned multi-file and a low upload limit. Record memlock/fallback behavior, locked bytes and file-backed memory in addition to CPU/PSS/anonymous memory. Do not raise memlock limits to manufacture success. If mmap saves anonymous memory but loses PSS or CPU, retain the result as a tradeoff and proceed only if counts identify a specific removable mechanism.

### 3. Implement a small seed-state prototype if attribution supports it

Create an explicit seed execution state owning the existing upload engine and transport progress. Keep handshake, BEP 6 obligations, extension metadata, choke/shutdown and accounting behavior shared. Lazily instantiate download-specific state; on failed recheck or mode transition, quiesce upload admission, advance generations, settle/refund ownership, rebuild admitted download state and resume through the general path. Make transitions explicit and testable.

Prototype one eligible verified v1 plaintext TCP mode first, with ordinary fallback for uTP, hybrid/v2 and unsupported conditions; eligibility is not permission to drop valid protocol messages. Bound each ready-progress turn by bytes/frames so control and other peers remain responsive. Maintain a single socket writer and settle partial frames exactly once. Avoid a second permanent task or queue just to implement the fast path.

Discriminator: demonstrably smaller active connection allocation and fewer polls/instructions per useful batch at unchanged operation counts, followed by joint CPU/PSS qualification. If the prototype only moves text or duplicates state, stop. This is the first justified substantial refactor, not a commitment to rewrite the app.

### 4. Refactor preparation only where measured counts justify it

Preserve fair per-peer byte admission while batching execution across already-admitted work. The current rate admission limits competing or limited demand to one block; simply raising the batch cap does not test shared execution of fairly admitted peer work.

If worker round trips remain material, introduce bounded preparation envelopes for already-admitted requests and duplicate-miss sharing keyed by exact file identity, generation and range. Retain worker validation and physical-device accounting for actual population/read work. Return one owned completion with per-consumer cancellation/settlement, not copies of payload ownership. Ensure one canceled follower cannot cancel other live consumers or retain a dead connection indefinitely.

Mapped ready-range classification can avoid physical-device admission for already-populated ranges, while validation stays on workers. Measure cold contention to ensure this does not let cache hits starve real reads, rechecks or writes. A direct async-side ready hit is a separate contract problem described above and should not be the first implementation.

If ready reuse is rare under random/cold traffic, do not grow caches to force hit rates. If current eight-block batching already amortizes nearly all preparation, focus on state/runtime cost rather than adding another coalescer.

### 5. Reduce fixed control/retention cost from attribution

Select the largest verified retained allocation or idle wake source. Examples to investigate, not assumed findings: router/state duplication, restore/recheck scratch retained by a long-lived worker, unused seed-only download state, repeated periodic projections, or extra runtime blocking-pool residency. Change one ownership/lifetime boundary at a time; preserve API support, persistence semantics and recovery guarantees.

Only after data ownership and retention improve should worker activation/affinity or allocator settings be reconsidered. Use queue delay plus active work and service latency, not queue length alone, for any adaptive worker experiment. Startup activation/retirement hysteresis must be exercised; the previous tests that never started worker two cannot validate it. Do not use process-wide allocator caps as a surrogate for fixing cross-thread allocation lifetime.

### 6. Escalation criterion for a deeper reactor redesign

Consider a dedicated seed reactor or thread-local peer executor only if the above leaves a clearly measured task/wake/ownership bottleneck and the narrow seed-state experiment demonstrates enough value to justify it. A runtime replacement must remove work, not add another actor handoff between the socket and existing storage path. It must account for control isolation, mixed workloads and all process/thread memory. Current evidence does not justify removing Tokio from the application, rewriting the storage subsystem wholesale, or switching implementation language.

## Correctness and promotion gates

For each candidate, freeze source/compiler/features/profile/order/binary/library identities. Keep the predeclared joint CPU/PSS gate and a fresh independently balanced qualification set, rather than selecting the best noisy three-round screen. Include source-matched build controls when changing runtime source; fresh layout/PGO training is part of the treatment. A 2% non-regression pass is not proof of improvement or parity.

Functional coverage must include partial writes across header/payload boundaries, cancellation before/after admission, choke/allowed-fast handling, connection teardown, live rate epoch changes, zero-rate/low-rate limits, storage errors, unsupported mmap fallback, path replacement/truncation, failed recheck and return to downloading, and global memory/descriptor pressure. Shared preparations need follower cancellation and generation-reuse tests. New seed execution must preserve metadata/extension/control behavior, not merely transfer the fixture correctly.

Service qualification includes serial requesters, small windows, many competing peers, fast peers beside slow peers, cold and warm data, random/non-contiguous/multi-file reads, mixed download/recheck/API use, many torrents, idle-to-active transitions, restart and durability. Track goodput, first response and tail latency; a batching delay that meets a paced throughput target can still harm useful behavior.

The existing scored CPU covers first payload through completion and PSS samples connection/transfer; startup CPU, complete kernel/socket memory, file cache and all deployment modes are not certified. Cgroup/file-cache accounting must avoid double counting shared pages. Shared-host intervals and ten rounds are a local decision tool, not dedicated-host certification or family-wide assurance across many exploratory comparisons. The current comparator is a particular native GCC -O2 XML-RPC-enabled build; fairness requires reporting enabled services and workload, not describing it as every rTorrent build.

**Decision:** preserve the present ownership/safety improvements and defaults; spend the next cycle on attribution plus the feature-matched mmap/layout experiment, then a measured physical seed-state refactor. This is a credible route toward the goal, with explicit stop conditions. It is not evidence that the goal has already been reached or that any one refactor will reach it.
