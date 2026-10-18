# Transfer Core Rebuild Plan

Status: **active**. This is the working plan and tracker for the transfer-core
rebuild proposed by the external audit
*TorrentNG Code Audit and Rebuild Blueprint* (17 Oct 2026, audited revision
`05d5c23886446f380552c73e0be480f03ae2d2ab`; the source PDF is preserved at
[`docs/rebuild-pack/TorrentNG_Code_Audit_and_Rebuild_Blueprint.pdf`](rebuild-pack/TorrentNG_Code_Audit_and_Rebuild_Blueprint.pdf),
SHA-256 `8ec614a08e8bfe9b13b8a9f4494231e282d78e3654f2e4531cb2390b69a2593b`).
It complements, and does not replace,
`ENGINE.md`, `ENGINE_REWRITE.md`, `STORAGE_NG.md` and `CRASH_SAFETY.md`.

How to use this file: pick the lowest-numbered ready ticket (all prerequisites
`done`), land it as its own PR/commit, tick its row in section 6, and record
the evidence (test name, probe output, report path) in the "Evidence" column.
Never tick a row on the strength of a design alone.

## 1. Review of the audit

### Baseline source review

The audit's line references and initial finding checks describe the audited
baseline `05d5c23886446f380552c73e0be480f03ae2d2ab`. That source review
predates the later mainline changes now included in this rebuild branch; the
old byte-identity and commit-count note no longer describes the current
checkout. Treat those line references as historical. The ticket table and each
ticket's current scope/evidence are authoritative for the implementation
status here, and no audit finding is considered closed solely because its
original line reference changed.

| Finding | Re-verified in source | Result |
|---|---|---|
| F01 v2 block extent used as piece extent | `handle_v2_piece` sizes the assembly and the read-back with `region.length` from `validate_request`, which is the *block* extent | **Confirmed.** A multi-block v2 piece cannot complete. |
| F02 v2 request window cannot fill within a piece | `fill_v2_requests` takes `next_begin()`; if that block is outstanding it `continue`s to the next piece | **Confirmed.** |
| F10 `debug_assert!` hides the webseed debit | `peer_session.rs:656` | **Confirmed.** Release builds skip the debit. |
| F14 picker estimate assumes bit-packing | `picker.rs` fields are `Vec<bool>`; the doc comment claims they are bit-packed | **Confirmed by inspection.** The 8 MB vs 4.44 MB figures are the audit's allocator measurements and are not re-run yet (PERF-111). |

Everything else (F03-F09, F11-F13, F15-F35) is **accepted provisionally on the
audit's evidence**. Each owning ticket starts with a failing regression or a
measurement that reproduces the finding on current HEAD, and the finding is
only marked confirmed (or dropped) in section 5 once that exists.

### Gaps and risks in the plan as delivered

1. **The implementation pack is not in the repo.** Only the PDF arrived. The
   ZIP (`BACKLOG.json`, per-ticket scope/paths/acceptance, transfer and
   storage specs, regression patches, benchmark workload JSON) did not. The PDF
   gives ticket titles and dependencies only. Consequence: the v2 correction
   has to be re-implemented here (SYS-002), and each ticket's detailed
   acceptance must be written when the ticket is picked up (SYS-001 owns the
   template). If the ZIP is available, drop it under
   `docs/rebuild-pack/` and this plan should defer to its `BACKLOG.json` for
   cross-stream prerequisites, as the PDF itself says.
2. **Proposed gates are not release gates yet.** `CLAUDE.md` currently states
   that numeric torrent-count capacity proofs and 24-hour stability are not
   release gates. The audit's scorecard (section 4) is explicitly "proposed,
   not measured". Treat it as a diagnostic target set until the owner adopts it
   by a recorded decision, and never edit a gate after seeing a bad result.
3. **Estimated effort is low confidence.** The audit says 6-12+ months for a
   small team and longer solo. G0 and G1 are the only gates whose cost can be
   estimated now; re-plan after the first v1 vertical slice (G2).
4. **A rewrite cliff is the main structural risk.** The audit itself
   recommends vertical slices, one writer per payload, and keeping the
   reference path selectable. This plan keeps that: no gate may delete the old
   live path before the new path passes correctness and resource gates.
5. **Existing invariants constrain the rewrite.** All new code must preserve:
   - the crash-safety rule: never persist a claim before the data it describes
     is durable (`CLAUDE.md`, `CRASH_SAFETY.md`), including
     `CrashSafetyRuntime::for_location` for per-location policy;
   - the per-install peer-id suffix and tracker identity rules
     (`TRACKER-IDENTITY.md`);
   - no-follow/confined secure filesystem semantics and move/rollback journals
     (explicitly "retain" in the audit);
   - the `rt-migrate` round-trip certification and public API compatibility.
6. **Some findings have a cheaper interim than the full redesign.** For
   example F08 (advertising Fast without its full contract) can be closed
   either by implementing the BEP 6 obligations (NET-105) or, as an interim
   in G0, by not advertising Fast until they exist. The interim is tracked in
   SYS-003 and must be reverted/superseded by NET-105, not left in place.
7. **Benchmarks need dedicated hardware and scratch space.** The qualification
   suite wants a separate helper host, cgroup v2 access and 10 Gb links. None
   of that is assumed in hosted CI. Build and test scratch on this workstation
   belongs on the NFS pool (`/mnt/datapool_lvm_media/tmp-claude-*`), not
   `/tmp` or `/home`.
8. **Two audit claims are explicitly low confidence and must not be quoted as
   fact:** F21 (cancellation may release admission/dirty evidence early —
   ownership risk, no reproduced data loss) and every arithmetic throughput
   ceiling (F05, F06, F07 — illustrations, not measurements). The historical
   14 Oct NFS report (0.49 vs 16.82 MiB/s) is a warning signal, not a ranking.

### Recorded decision

**D2 — uTP backend (NET-112/113):** keep the runtime implementation in
TorrentNG. The reviewed `urtorrent-utp` releases (0.14.2 and 0.14.3) declare
Rust 1.98, above TorrentNG's Rust 1.88 floor, so they cannot be workspace
dependencies without an unjustified MSRV increase. Use their window, SACK,
reordering, timeout, and MTU behavior as a protocol reference; implement a
bounded compatible protocol core and shared-socket manager behind the existing
stream API. The source now wires v1, pure-v2, and metadata outbound paths
through that manager, with incoming and outgoing peers sharing the listener
endpoint when incoming uTP is enabled. The protocol/runtime changes remain
unrun and unqualified. This is an implementation direction, not a performance
result; keep rollout reversible until bulk correctness and matched whole-client
efficiency evidence.

See [`TRANSFER_CORE_UTP_EVALUATION.md`](TRANSFER_CORE_UTP_EVALUATION.md) for
the source review and adapter acceptance.

**D3 — integration and rollout:** keep the rebuild on the local
`feat/transfer-core-rebuild` integration branch while the 14-wave implementation
and documentation are being completed. Preserve the existing portable path as
the selectable reference during staged live integration. The owner has since
directed that reviewed local work be committed, consolidated into `main`, and
pushed after code and documentation are complete. That source-control cleanup
does not qualify a release: the exact consolidated revision still needs the
owner-requested bulk validation and review before any release or capability
rollout claim.

**D5 — native engine versus maintained implementation:** continue the current
native rebuild as a reversible candidate; do not restart or embed a maintained
client based on code inspection alone. `librqbit` is a serious v1 efficiency
comparator and possible alternate engine, but its current public feature list
does not claim BEP 52 and its README describes sequential-only piece selection;
it is not presently a drop-in match for TorrentNG's v1/v2/hybrid target.
libtorrent remains a mature behavior and scheduling reference. Compare exact pinned clients with the PERF-101–116
fixtures and settings, then prefer the smallest subsystem replacement that
passes correctness, Rust 1.88, storage/persistence ownership, and matched
whole-client efficiency gates. A full-engine replacement remains on the table
if the native architecture fails those gates and an alternative preserves the
product contracts. This is a provisional architecture decision, not a claim
that the in-house candidate is better. See
[`TRANSFER_CORE_REFERENCE_EVALUATION.md`](TRANSFER_CORE_REFERENCE_EVALUATION.md).

### Decisions still open with the owner (not blocking G0)

| # | Decision | Needed before |
|---|---|---|
| D1 | Adopt, amend, or reject the proposed scorecard numbers | PERF-110 |
| D4 | Dedicated hardware lane (HDD / NVMe / 10 Gb helper) availability | PERF-115 |

## 2. Architecture target (summary)

One authoritative owner per kind of work. Keep current crate boundaries unless
a dependency cycle forces a small extraction.

| Owner | Responsibility | Must not |
|---|---|---|
| Session supervisor (`rt-engine`) | lifecycle, config, admission across torrents, shutdown | copy payloads, scan all task handles per command, run long jobs inline |
| Pure transfer core (`rt-engine/src/transfer/`) | geometry adapters, request/content ledger, picker policy, protocol obligations, piece evidence | own sockets, fds, SQLite, sleeping futures, unbounded tasks |
| Peer runtime (`rt-peer-wire` / transport) | bounded parsing, independent receive and ordered send progress | decide global content ownership, silently abandon negotiated requests |
| Device service (`rt-storage`) | secure file catalog, all I/O admission, write sequences, barriers, completion retention | lose ownership because a caller was cancelled; sync unrelated files |
| CPU service (`rt-hash` + runtime) | bounded hash jobs over immutable segments, Merkle work | one task per tiny leaf; copy payload to cross an async boundary |
| Persistence worker (`rt-db`) | ordered batched transactions, checkpoint evidence | mark payload clean before covering writes are durable |
| Read model (`rt-session`, `rt-api-model`) | stable chunks, revision deltas, compact identities | deep-clone cold strings on a counter change |

Evidence states are distinct and partly orthogonal: *received* → *verified* →
*write-complete* → *servable* → *payload-durable* → *resume-committed*. First
implementation advertises HAVE after verified write completion.

Retain and verify (do not rewrite): secure descriptor-relative filesystem
code, validated wire/metainfo parsing, v1/v2 mapping rules, move/rollback
journals, durability policy, the dedicated DB worker, v1 chunked verification,
API adapters and incremental API snapshot machinery, the portable exact-I/O
loops (reference backend).

Defer until measured: global allocator changes, custom unsafe buffer
initialisation, mmap/direct-I/O defaults, SQPOLL, sendfile, SIMD tuning.

## 3. Execution rules

1. **Vertical slices.** One complete production v1 transfer through the new
   core before spreading interfaces across v2/hybrid/uTP.
2. **One writer per payload.** Old and new engines never write the same files
   or issue duplicate network requests. Shadow comparison is read-only and runs
   on fake backends or isolated fixtures.
3. **Red first.** A regression that fails on current HEAD lands before (or in
   the same commit as) its fix. A known-open regression stays failing in a
   dedicated `#[ignore = "TNG-<ticket>"]`-style marker that names its ticket,
   not in a broad CI exclusion.
4. **Every PR states**: which owner changed, which invariant it preserves,
   which production route now uses it, which obsolete route was retired, and
   the exact evidence.
5. **Every optimisation proves its whole-engine effect** (API latency,
   recovery, duplicate traffic, total memory), not only a microbenchmark.
6. **State-format changes** need their own migration + rollback tests, including
   a crash between every publication step. New caches are versioned and
   disposable; database/resume compatibility is preserved initially.
7. **Features stay toggleable and documented** (runtime option, docs, UI
   affordance) per repo convention.
8. **One integration owner** for transfer/storage/persistence contracts; nobody
   invents another pool, scheduler, file cache or snapshot model.

## 4. Gates

| Gate | Shippable result | Exit evidence |
|---|---|---|
| **G0 Correctness and honest evidence** | regressions committed; v2 extent, release-accounting, aggregate upload cap, pacing wakeup and Fast lifecycle repaired; valid baseline controller started | multi-block v2 memory + spill tests; release-mode rate tests; request lifecycle fixtures; source/build identity; no unsupported performance claims |
| **G1 Ownership reference** | typed extents/epochs, retained buffer owners, bounded operation/completion lifecycle, portable file/device service | deterministic cancellation / late-completion / short-I/O tests; live paths inventoried; accounting survives last slice and kernel lifetime |
| **G2 V1 pipeline** | production v1 through shared ledger, duplex I/O, fair grants, completion-driven hash/disk | independent peers; no actor stall under delayed I/O; low-rate progress; bounded pressure; endgame and choke races |
| **G3 V2 and hybrid** | correct complete-piece handling, shared reservations, authenticated proof store, both hash views | large/short/multi-block files; pure-v2 interop; padding and nonzero offsets; large-file proofs; staged hybrid overlap |
| **G4 Durability and storage** | dirty-only fences, restart evidence, adaptive range cache, chunked recheck, scheduled secure copy | crash matrix; no unrelated sync; cancelled-write accounting; copy-depth invariance; secure-fs invariants unchanged |
| **G5 Scale and transports** | real catalog tests, delta publication, event-driven reaping, bounded timers/control traffic, qualified uTP | live 10k/100k catalog trials; mixed activity; partial/reordered/lossy transport tests; no task or memory leak |
| **G6 Hardware optimisation** | continuous io_uring, proven cache/window/queue policies, same-source allocator experiments | paired whole-client trials; compatible durability; helper ceilings; no bottleneck migration hidden by averages |
| **G7 Release qualification** | reproducible binaries, preserved capabilities, canary and rollback | required matrix pass; recovery + bounded-resource soak; result manifests; controlled dwell with retained fallback |

Exit gates are dependency-based: a fast NVMe microbenchmark never waives a
failed v2 or crash test. Hosted CI runs fast correctness subsets per PR;
hardware qualification runs on dedicated machines and failures are never
relabelled flaky without diagnosis.

### Proposed scorecard (targets, not achieved; see review item 2)

| Gate | Workload | Proposed acceptance |
|---|---|---|
| Integrity | every applicable workload incl. v2 multi-block and R1/R2 faults | zero unexpected hash mismatches or false completions |
| LAN capacity | `N1/lan_1g` 8 GiB; `N1/lan_10g` 32 GiB, 16 GiB client limit, ≥64 GiB helper, x64 NVMe, one TCP peer, 0.2 ms RTT | ≥105 and ≥900 MiB/s, each ≥90% of calibrated ceiling |
| Empty idle | `M1/metrics-off` / `metrics-on`, no torrents, 600 s | anon ≤32 MiB, PSS ≤64 MiB, mean CPU ≤0.5% core, ≤32 threads |
| Dormant catalog | `M2/catalog-100000`, names ≤80 B, one tracker each, no peers, 600 s | anon ≤256 MiB, ≤2 KiB/torrent incremental, ≤512 fds |
| Active catalog | `M3/1000-hot`, 4 peers each, 16 GiB, 100 MiB/s offered | ≥98 MiB/s; anon ≤512 MiB; total charged ≤2 GiB; ≤4,640 fds |
| Download writes | `S1/1-torrents`, one 64 GiB file, ext4 HDD, 16 peers, 512 MiB limit, declared fsync boundary | payload write amp ≤1.15; whole-device ≤1.30 |
| Random seed reads | `S2/128-leechers`, HDD, 512 MiB limit, 32-block windows | physical reads / unique verified bytes ≤1.25 |
| Control latency | `A1/api-16`, 100 MiB/s offered, 16 clients, 4 SSE, `a1-ui-mix-v1` at 100 req/s | scheduled-to-accepted p99 ≤100 ms; transfer ≥98 MiB/s |
| Overload | `A1/api-overload-8000`, `A1/sse-stalled-60s` | pending API/SSE ≤8/4 MiB; explicit rejection ≤1 s; recovery ≤10 s |
| Retention | `R3/72h-candidate`, `R3/7d-release` | last-quarter median anon ≤ first steady-state quarter + max(16 MiB, 5%); no per-cycle fd/task growth |

Comparative claims (need ten randomised paired trials, 95% CIs): ≥95% of the
fastest valid comparator's goodput at identical resources/cache/durability;
at matched load ≥98% of offered rate before comparing CPU s/GiB or joules/GiB;
a *leadership* claim needs ≥10% improvement on a named metric. Uninstrumented
energy is `null`, never zero.

## 5. Findings register

Priority and gate are the audit's. **Status** is this repo's state:
`open` (not started), `confirmed` (reproduced on HEAD by a test/probe),
`fixed` (fix landed + evidence), `accepted` (taken from audit, not yet
reproduced).

| ID | P | Finding | Tickets | Gate | Status |
|---|---|---|---|---|---|
| F01 | P0 | v2 multi-block completion uses block extent as piece extent | SYS-002, IO-002 | G0 | fixed (SYS-002; typed block/piece extents still IO-002) |
| F02 | P0 | v2 request window cannot fill within a piece | SYS-002, NET-104 | G0 | fixed locally (SYS-002; shared ledger is NET-104) |
| F03 | P0 | v2 peers lack shared acquisition state | NET-104, NET-114 | G3 | accepted |
| F04 | P0 | v1 actor awaits disk/hash/persistence | IO-008, NET-106 | G2 | accepted |
| F05 | P1 | fixed 512 KiB v1 flight window | NET-108 | G2 | partly fixed (rate-proportional window, 32..128 blocks, torrent-wide cap of 2,048 requests, never under memory pressure; measured response delay and device-service backlog feedback remain NET-108) |
| F06 | P0 | 4,096 requests/10 s upload guard | NET-110 | G2 | partly fixed (count ceiling removed; bounded queue saturation now rejects for Fast and silently drops only for legacy peers; strict byte-admitted service is pure but live integration remains NET-114) |
| F07 | P0 | uTP writer is stop-and-wait | NET-112, NET-113 | G5 | partly fixed (bounded windowing, SACK accounting, and reorder handling are authored with production shared-runtime wiring; bulk protocol validation and matched efficiency evidence remain open) |
| F08 | P0 | Fast advertised without full request termination | SYS-003, NET-105 | G0 | partly fixed (v1/v2 Reject behavior honors Fast negotiation; v1/v2 preserve wire requests across Fast Choke; v1 releases queued-only requests and returns Reject for a canceled queued upload when Fast is active; v1 sends best-effort endgame Cancel while retaining wire-response ownership; v2/shared-core cancellation remains NET-105) |
| F09 | P0 | torrent upload limit copied into per-peer buckets | SYS-003, NET-107 | G0 | fixed (SYS-003; hierarchical scheduler still NET-107) |
| F10 | P0 | webseed debit inside `debug_assert!` | SYS-003 | G0 | fixed (SYS-003) |
| F11 | P1 | no token-ready wakeup for download pacing | SYS-003, NET-107 | G0 | fixed (SYS-003; deadline-driven, not per-peer polling) |
| F12 | P1 | choker feedback/exploration | NET-111 | G2 | partly fixed (v1 and v2 use mode-aware ranking and fair optimistic slots; seeded tie-breaking supports repeatable policy traces, while the swarm-trace replay harness remains NET-111) |
| F13 | P0 | memory pressure discards received data | NET-103 | G2 | partly fixed (evicted assemblies spill to payload files; v1 and pure-v2 peer requests reserve response bytes before enqueue and keep leases through block handling; shared spans and hybrid/cross-plane admission remain NET-103) |
| F14 | P0 | picker memory estimate wrong (`Vec<bool>`) | SYS-004, NET-109 | G0 | fixed (packed `PieceFlags`; estimate counts bitsets, availability arrays, candidate positions and bucket entries) |
| F15 | P1 | picker scans all pieces before rarity buckets | NET-109 | G2 | partly fixed (active fresh-work index, array-backed rarity buckets, and random tie-breaking are implemented; churn now shrinks over-capacity rarity buckets; regression tests await the bulk pass) |
| F16 | P0/P1 | buffers lose coherent owner across boundaries | IO-003 | G1 | accepted (v1 peer/webseed receive events now retain their byte lease through actor handling; shared immutable spans and storage-operation ownership remain open) |
| F17 | P1 | shared file cache serialises metadata work | IO-004 | G1 | partly fixed (scheduler and `HandleCache` identity stats and file opens run outside cache locks, interval-limited idle sweep, write descriptor serves reads; one secure catalog with compact IDs remains IO-004) |
| F18 | P1 | default readahead 16 KiB → 512 KiB | IO-011 | partly fixed (readahead starts at one block; sequential continuation bootstraps one prefetch, cache hits permit gradual bounded growth, and misses hold the recommendation; speculative bytes/frame are admitted before dispatch; reusable range policy remains IO-011) |
| F19 | P0 | torrent flush syncs unrelated writable files | IO-009 | G4 | partly fixed (barrier scopes sync to this scheduler and now closes scheduler-wide write admission across its stable dirty/active path snapshot; per-path fences and dirty generations remain; held-sync new-path regression is authored; grouped device batching and bulk evidence remain IO-009) |
| F20 | P1 | io_uring worker is a batch barrier | IO-015 | G6 | accepted |
| F21 | P0 | cancellation may release admission/dirty evidence early (moderate confidence) | IO-005 | G1 | partly fixed (read/write/sync watchers retain queue/device/byte admission through backend completion; helper-level regressions and end-to-end queue-slot/byte-budget cancellation cases are authored; the full cancellation and accounting matrix remains IO-005) |
| F22 | P1 | several storage schedulers, no fairness authority | IO-006 | G1 | partly addressed (shared per-device weighted class dispatcher now spans mounts, and secure plan steps acquire all touched device queues in stable order; deadline promotion, grouped barriers, and qualification remain open) |
| F23 | P1 | v2 recheck does per-leaf small I/O | IO-012 | G4 | partly fixed (1 MiB chunks: one extent probe, read and hash job per 64 leaves; persistent sparse cursors and hybrid shared read stream remain IO-012/IO-007) |
| F24 | P0/P1 | v2 proof serving rereads file; 64 MiB ceiling | IO-013 | G3 | partly fixed (64 MiB ceiling removed; leaves hashed in 256 KiB reads; layers proven against the metainfo root and kept in a 64 MiB LRU; persisted node store and subtree-only work remain IO-013) |
| F25 | P1 | secure copy re-verifies leaves per ancestor | IO-014 | G4 | fixed for hashing (nested entries copy-only; top-level verifies the tree once); source/destination device admission is ordered, directory verification streams names, and regular-file copy/hash work yields every 64 KiB; directory-tree copy preemption, final sync latency, and qualification remain open |
| F26 | P1 | one row change rebuilds a deep-cloned registry vector | SYS-005, SYS-006 | G5 | partly fixed (snapshot rows are shared `Arc<TorrentEntry>` and flattening clones pointers; bounded coalesced delta pages cap journal copies and API page projections are routed through them, while cold/hot metadata split and compact identities remain SYS-005/SYS-004) |
| F27 | P1 | dormant estimates incomplete; quadratic timer cancel | SYS-004, SYS-008 | G5 | partly fixed (lazy-deletion timer, bounded due-pop, and staggered idle deadlines; `TierController` tier/timer indexes now use 8-byte slot/generation IDs, and `estimated_owner_bytes` counts retained maps, queues, interned keys and dormant snapshots; engine runtime maps remain SYS-004) |
| F28 | P1 | every engine command scans all task handles | SYS-007 | G5 | fixed (supervised exit queue on the command path; 1 s full scan kept as reconciliation) |
| F29 | P0 | metrics scrape builds a shadow storage runtime | SYS-011, PERF-108 | G0 | fixed (scrape uses `StorageRuntime::initialized()`; owner-specific metrics still SYS-011) |
| F30 | P1 | engine unit tests bypass production DB worker | SYS-009 | G0 | partly addressed (a file-backed progress-persistence regression now opts into `DbExecutor::Worker`; authored but unrun; most fast fixtures remain direct) |
| F31 | P0 | scale checks certify synthetic projections | PERF-109 | G5 | accepted |
| F32 | P0 | throughput comparisons don't establish capacity | PERF-104..107 | G0 | accepted |
| F33 | P1 | CPU/cache/backend microbenchmarks reward wrong behaviour | PERF-102, PERF-107 | G0 | accepted |
| F34 | P0 | certification stamps current commit on historical PASS text | PERF-114 | G0 | fixed for `memory_roadmap_certification.sh` (source reports must record the current HEAD; other aggregators and machine-readable manifests remain PERF-114) |
| F35 | P1 | announce waits for optional scrape | SYS-010 | G5 | fixed (announce delivered first; scrape separate event, process-wide cap of 4; broader discovery/control budgets still SYS-010) |

## 6. Work packages

63 packages; ordering is topological. A *wave* is a dependency frontier, not a
sprint. Current snapshot: **4 done, 43 wip, 16 todo**. Status values: `todo`,
`wip`, `done`, `blocked`. Detailed per-ticket scope/acceptance is created from
the template in SYS-001 when picked up.

| Wave | Ticket | P | Work package | Prerequisites | Status | Evidence |
|---|---|---|---|---|---|---|
| 1 | SYS-001 | P0 | Pin the reference, capability matrix and shared ownership contracts | none | done | reference and shared ownership are pinned here; capability matrix, ticket template, and active ticket specs added and linked |
| 2 | IO-001 | P0 | Define storage evidence contracts and deterministic fault harness | SYS-001 | wip | evidence lifecycle and fault matrix documented; test-only scripted backend covers injected errors, held completions, and partial-write errors, and releases held test workers during teardown; read/write, cancellation, partial-write, and failed-barrier retry regressions authored, bulk pass pending |
| 2 | NET-101 | P0 | Define typed transfer coordinates and protocol geometry adapters | SYS-001 | wip | pure module landed in `crates/rt-engine/src/transfer/` with unit tests (compiled and clippy-clean, NOT yet run); live integration pending NET-114 |
| 2 | PERF-101 | P0 | Commit measurement contract and strict result schemas | SYS-001 | wip | timing/provenance contract, trial/manifest/client-observation schemas, and pinned Draft 2020-12 validator are authored; manifest checks referenced digests, source identity, and trial-to-client observation identity; scorecard and bulk pass remain open |
| 2 | SYS-002 | P0 | Land v2 reproductions and a reviewed extent correction | SYS-001 | done | `torrent_task_v2::tests::v2_*` multi-block tests (5 completion cases x memory/spill, red without fix) + `v2_request_window_pipelines_every_block_of_a_piece` (red: 4 of 8 requests) |
| 2 | SYS-003 | P0 | Repair immediate release-rate and negotiated-capability behavior | SYS-001 | done | F10, F09, F11 fixed with tests; F08 v1/v2 request rejection follows Fast negotiation (outgoing Cancel remains NET-105) |
| 2 | SYS-004 | P0 | Introduce compact identities and honest whole-owner metadata accounting | SYS-001 | wip | F14 packed flags + honest picker estimate; `TierController` tier and timer indexes now use 8-byte generation-checked IDs over one interned identity allocation, and owner accounting covers retained capacities; `EngineRuntimeState` maps/completion IDs and bulk evidence remain open |
| 2 | SYS-009 | P0 | Exercise the production database worker and coalesce progress | SYS-001 | wip | one torrent progress-flush regression now uses a real file-backed `DbWorker` through `DbExecutor`, checks the batched transaction metric, and reopens the database; authored but unrun |
| 2 | SYS-011 | P0 | Make live observability passive and owner-specific | SYS-001 | wip | `/metrics` uses passive storage-runtime and descriptor-budget snapshots (no runtime creation or RLIMIT_NOFILE change); owner inventory and blind spots documented in TRANSFER_CORE_OBSERVABILITY.md; fresh-process and mixed-load evidence pending |
| 3 | IO-002 | P0 | Separate block and piece extents and repair multi-block v2 transfers | IO-001, SYS-002, NET-101 | wip | v1 peer receive/upload and v2 receive/serve paths validate typed block extents; v2 assembly/read-back/progress use the full typed piece extent; regressions remain unrun pending bulk pass |
| 3 | IO-003 | P0 | Carry capacity leases through immutable payload ownership | IO-001 | wip | v1 peer and webseed receive events retain reservations through actor/storage waits; assembly spill uses zero-copy slices sharing the assembly lease; tests authored, bulk pass pending |
| 3 | IO-004 | P0 | Unify the secure file catalog and descriptor lifecycle | IO-001 | wip | platform file identity now lives in `open.rs` and is shared by descriptor and peer-read cache checks; scheduler and `HandleCache` path checks run outside catalog mutexes, and newly opened descriptors are identity-checked before publication with bounded replacement retries; public `HandleCache` reads reuse read+write descriptors and publishing a writer removes the redundant cached read-only entry; single-catalog unification and remaining descriptor-lifecycle coverage are open |
| 3 | NET-102 | P0 | Build a deterministic transfer simulator and invariant oracle | NET-101 | wip | pure module landed in `crates/rt-engine/src/transfer/` with unit tests (compiled and clippy-clean, NOT yet run); live integration pending NET-114 |
| 3 | PERF-102 | P0 | Build independent cumulative resource observer | PERF-101 | wip | `scripts/transfer_rebuild/observe.py` streams process-tree `/proc` and cgroup v2 samples; follows thread-spawned descendants, keeps unavailable counters null, and creates private immutable CSV/summary outputs; network attribution and schema integration remain open |
| 3 | PERF-103 | P0 | Implement client adapters with pinned identity and settings readback | PERF-101 | wip | stdlib adapters cover TorrentNG, qBittorrent, Transmission, Deluge, rTorrent, rqbit, and a standalone `urtorrent` 0.14.3 library runner pinned to Rust 1.98.1; runner rejects v2/hybrid input, snapshots effective settings plus separately labeled hash-thread launch input, removes trackers/web seeds from a private fixture, binds only to a task-local IPv4 interface and explicit private IPv4 peer, disables IPv6/discovery, and waits in a named namespace; adapter independently validates both addresses and process-bound snapshot identity; runner unbuilt/unrun, exact profiles, full paired harness, and bulk validation remain open |
| 3 | PERF-105 | P0 | Build scoped deterministic fixtures and cache controls | PERF-101 | wip | `fixtures.py` writes v1/v2/hybrid metainfo with payload-derived hashes, authenticated v2 piece layers, hybrid BEP 47 alignment pads, and versioned info-hash records; pinned profiles bind protocol/piece geometry into fixture identity; `cache_control.py` requests per-file cold/warm state and records mincore residency without global cache writes; trial records and manifest validation bind achieved cache evidence to the fixture; caller-owned task cleanup policy documented; regressions authored, bulk pass and filesystem cache qualification remain open |
| 3 | SYS-005 | P1 | Publish registry chunks and revision deltas without global deep clones | SYS-004 | wip | snapshot rows share `Arc<TorrentEntry>`; capped coalesced pages now feed SSE and qBittorrent cursors, and API snapshot caches fold pages into changed-hash sets; cold/hot metadata split, compact identities, and bulk evidence remain open |
| 3 | SYS-007 | P1 | Replace per-command task scans with supervised completions | SYS-004 | done | supervised exit queue (`TaskExits`); command path O(exits) |
| 3 | SYS-008 | P1 | Replace quadratic timer cancellation and stagger bulk work | SYS-004 | wip | O(1)/lazy timer cancel, bounded due-pop, and idle-check deadlines staggered across coarse buckets (10% of delay, capped at 30 s); broader bulk pacing and bulk evidence remain open |
| 3 | SYS-012 | P0 | Preserve feature and platform compatibility through production fixtures | SYS-001, SYS-002, SYS-003 | wip | `rt-testkit` builds deterministic payload-derived v1/v2/hybrid metainfo fixtures with multi-piece, nested, empty-file, piece-layer, and hybrid-padding cases; production parsing plus v1/v2 piece-map regressions and Windows smoke wiring are authored, not run |
| 4 | IO-005 | P0 | Make admission and completion ownership bounded and cancellation-safe | IO-003, IO-004 | wip | `DiskSubmission` queue/device permits and queued-byte charges now travel with blocking preparation and backend completion watchers for reads, writes, and syncs; helper-level owner regressions plus end-to-end held-write queue-slot/byte-budget, strict-sync ownership and failure, expanded peer-read charge-retention, and failure-release cases are authored; full injected-device cancellation matrix remains open |
| 4 | NET-103 | P0 | Reserve payload capacity before requests and carry leased buffer spans | NET-101, NET-102, IO-003 | wip | v1 peer requests reserve PeerBuffer bytes before enqueue and move the lease through outstanding ownership into the block event; pure-v2 requests reserve before wire send and retain leases through block handling; both retry after capacity returns; v2 denial/retry, per-block lease through full Piece storage/verification, matching/mismatched Reject, Choke, and disconnect regression sources are authored but unrun; shared spans/hybrid admission remain open |
| 4 | NET-112 | P1 | Select and qualify a uTP protocol core using a reproducible integration evaluation | NET-101, NET-102 | wip | Decision recorded: `urtorrent-utp` is a protocol/wire reference, not a dependency (reviewed releases require Rust 1.98 vs native 1.88). TorrentNG-owned bounded window/SACK/reorder code is authored; qualification and matched efficiency remain pending; see TRANSFER_CORE_UTP_EVALUATION.md |
| 4 | PERF-104 | P0 | Implement common timing barriers and useful-byte accounting | PERF-101, PERF-102, PERF-103 | wip | `measurement.py` provides a file-backed matched-start barrier, per-file unique/duplicate range accounting, and separate monotonic trial phases; paired harness and artifact assembly remain open; regression sources await bulk pass |
| 4 | PERF-106 | P0 | Create deterministic independent swarm and network controls | PERF-101, PERF-105 | wip | pinned Linux egress shaper and readback artifact for task-owned veths in `tngt-*` namespaces; trial/manifest digest linkage and regression sources authored, bulk pass pending; independent swarm fixture, paired endpoint orchestration, and loss/jitter profiles remain open |
| 4 | PERF-108 | P0 | Remove metrics-created shadow storage runtime and stabilize accounting | PERF-101, PERF-102, SYS-011 | wip | shadow storage runtime removed from `/metrics`; stable accounting not started |
| 4 | SYS-006 | P1 | Route APIs through bounded delta, page and sort projections | SYS-005 | wip | native and qBittorrent list snapshots already use stable bounded pages and sort/filter projections; SSE and qBittorrent sync now emit capped delta pages, with a continuation RID that distinguishes empty revision zero from the first mutation; response-budget qualification and bulk regressions remain open |
| 4 | SYS-010 | P1 | Bound discovery and control I/O and decouple scrape from announce | SYS-004, SYS-008 | wip | announce delivered before scrape; scrape capped process-wide; DNS/DHT/metadata budgets not started |
| 5 | IO-006 | P0 | Build one device service over the portable reference backend | IO-005 | wip | bounded `DeviceQueue` is shared by physical ID; active dispatch uses one slot per portable worker or one bounded io_uring batch, and queued operations receive weighted class fairness across mounts; operation IDs travel with completion owners; cache hits/file-open preparation do not hold device slots; scheduler reads/writes/syncs, coalesced peer reads, preallocation, extent probes, and secure copy/import steps are routed; copy scratch is charged at 64 KiB through completion; secure plan steps acquire all touched device queues in stable order and use one stable fallback queue when physical IDs are unavailable; canceled external handoffs retain shared device and byte admission through closure completion. Deadline/choke-critical policy, grouped barriers, and full fault qualification remain open |
| 5 | IO-007 | P0 | Create batched hash jobs and shared hybrid content views | IO-002, IO-003, IO-005 | wip | v1 streaming SHA-1 and v2 64-leaf batches already use the bounded hash pool shared by matching storage schedulers; v2 Merkle accumulator finalization now uses that pool too, retaining only bounded accumulator state on queue-full retry. Hybrid metadata still enters the v1 actor without a shared immutable dual-hash view; dual verification and common-view integration remain open. Regression source now asserts three leaf batches plus one root job; not run |
| 5 | NET-104 | P0 | Implement the authoritative request and content ownership ledger | NET-101, NET-102, NET-103, IO-005 | wip | in-progress piece arrays use a conservative 64 MiB logical byte budget; live connections cap at 4,096, per-peer request depth at 1,024, torrent-wide in-flight owners at 65,536, and endgame owners at four per block; budget/cap regression sources authored, bulk pass and live integration pending |
| 5 | PERF-107 | P0 | Implement independent network, hash, disk and pipeline ceilings | PERF-102, PERF-105, PERF-106 | wip | `ceilings.py` derives directional network/disk/hash/pipeline bounds from identity-pinned independent calibration inputs; versioned schemas, manifest digest/recompute checks, and optional trial linkage bind the host/device/network/protocol and client/version/settings pointer, then verify peer/block/request counts; regression sources authored; no calibration, tests, or qualification run |
| 5 | PERF-109 | P0 | Replace synthetic capacity proxies with live production fixtures | PERF-102, PERF-103, PERF-104, PERF-105, PERF-108 | todo | |
| 5 | PERF-112 | P0 | Enforce v2 and persistence invariants in full transfer paths | PERF-104, PERF-105, PERF-106 | todo | |
| 6 | IO-012 | P1 | Stream recheck through large chunks and persistent sparse cursors | IO-006, IO-007 | wip | 1 MiB chunked v2 verify + batched leaf hashing; persistent sparse cursors not started |
| 6 | NET-105 | P0 | Implement negotiated peer protocol request semantics | NET-104 | wip | pure obligations module present; v1/v2 Reject follows negotiated Fast and v1/v2 retain wire requests across Fast Choke; v1 invalidates queued requests by generation, returns queued-only picker slots, rejects canceled queued uploads only for Fast peers, and sends best-effort endgame Cancel without releasing possible late-response leases; focused Cancel regressions and full live-core integration remain pending NET-114 |
| 6 | NET-109 | P1 | Rebuild piece selection around active candidates and accurate accounting | NET-104 | wip | array-backed rarity buckets, swap-remove positions, fresh-work candidate index, randomized equal-rarity choice, peer-cap-aware capacity accounting, and capacity trimming under availability churn implemented; regression tests authored, bulk pass pending |
| 6 | PERF-110 | P0 | Automate 28-case catalog and randomized paired execution | PERF-103, PERF-104, PERF-105, PERF-106, PERF-107, PERF-109 | todo | |
| 6 | PERF-111 | P1 | Add picker timer and changed-snapshot allocation regression probes | PERF-101, PERF-102, PERF-109 | todo | |
| 6 | SYS-014 | P1 | Qualify complete catalog cost and mixed-load responsiveness | SYS-005, SYS-006, SYS-007, SYS-008, SYS-009, SYS-010, PERF-109 | todo | |
| 7 | NET-106 | P1 | Separate peer read progress from write and storage waits | NET-103, NET-104, NET-105 | todo | |
| 7 | PERF-113 | P1 | Implement statistical analysis and explicit scorecard | PERF-101, PERF-102, PERF-104, PERF-110 | wip | descriptive paired scorecard, output schema, validator kind, and regression sources authored; automated paired runner and bulk pass remain open; no thresholds adopted |
| 8 | IO-008 | P0 | Move transfer actors to completion-driven storage and hashing | IO-002, IO-006, IO-007, NET-104, NET-106 | todo | |
| 8 | NET-107 | P1 | Replace replicated token buckets with hierarchical fair rate scheduling | NET-103, NET-104, NET-106 | wip | pure module landed in `crates/rt-engine/src/transfer/` with unit tests (compiled and clippy-clean, NOT yet run); live integration pending NET-114 |
| 8 | NET-113 | P1 | Deliver the bounded windowed uTP transport behind the shared runtime | NET-103, NET-106, NET-112 | wip | `UtpRuntime` now backs production v1, pure-v2, and metadata outbound connects; shared endpoint demultiplexing, bounded window/reorder state, and rebind-retired routes are authored. Central deadline/ready scheduling, aggregate memory qualification, and bulk validation remain open |
| 8 | PERF-114 | P0 | Replace text-grep qualification with exact artifact manifests | PERF-101, PERF-113 | wip | `memory_roadmap_certification.sh` commit-checked; manifests not started |
| 9 | IO-009 | P0 | Implement dirty-only generation fences and grouped barriers | IO-004, IO-006, IO-008 | wip | checkpoint sync scopes to this scheduler's dirty/active files and takes scheduler-wide write admission so a new-path write cannot pass its snapshot; per-path exclusive fences and dirty generations remain, with a held-sync ordering regression authored; grouped device batching and bulk evidence remain open |
| 9 | IO-011 | P1 | Replace unconditional readahead with useful range caching | IO-004, IO-006, IO-008 | wip | per-file sequential demand bootstraps one prefetch; observed cache hits permit gradual growth while misses hold the recommendation; extra queued bytes and read frame are reserved before backend dispatch; bounded stream history evicts only its least-recently-used file; a reusable multi-range policy and bulk evidence remain open |
| 9 | NET-108 | P1 | Implement adaptive request windows and deadline-driven refill | NET-104, NET-106, NET-107 | wip | pure module landed in `crates/rt-engine/src/transfer/` with unit tests (compiled and clippy-clean, NOT yet run); live integration pending NET-114 |
| 9 | NET-110 | P1 | Implement bounded shared upload service and eliminate valid-demand drops | NET-103, NET-105, NET-106, NET-107 | wip | pure module landed in `crates/rt-engine/src/transfer/`; invalid zero/oversized wire blocks are refused, byte cap has a one-block minimum and remains strict, duplicate connection generations cannot replace live reads, and jobs keep their charge across disconnect until backend completion; regression sources await bulk pass; live integration pending NET-114 |
| 9 | PERF-115 | P1 | Integrate hosted correctness and dedicated hardware regression lanes | PERF-110, PERF-111, PERF-112, PERF-113, PERF-114 | todo | |
| 10 | IO-010 | P0 | Bind resume persistence and crash recovery to barrier evidence | IO-009, SYS-009 | wip | storage returns opaque evidence only after a successful scheduler-wide barrier and retains write admission in the proof; `FastresumeStore` writes the evidence-bound state in its blocking atomic writer before releasing admission, including caller cancellation; persisted file-count/write-sequence fields default safely for legacy records; crash-cut qualification and SYS-009 persistence integration remain open |
| 10 | IO-014 | P1 | Schedule secure copies and verify each leaf once | IO-006, IO-009, IO-012 | wip | secure plan steps resolve source/destination device queues once and acquire them in stable order; regular-file copy/hash work runs in 64 KiB quanta with per-quantum working-set charges and cleanup under admission; a terminal scheduler rejection triggers root-confined cursor cleanup; Unix same-filesystem `ImportExisting` keeps the hard-link fast path; checkpointing stays outside device permits; unknown topology uses a stable dispatcher fallback; directory verification streams names and Unix deletion batches 256 entries; directory-tree preemption, final sync latency, and bulk qualification remain open |
| 10 | IO-015 | P1 | Add continuous io_uring with retained safe registrations | IO-006, IO-009 | todo | |
| 10 | NET-111 | P1 | Implement fair seed and download choking policies | NET-104, NET-107, NET-110 | wip | v1 and v2 use mode-aware regular/optimistic slots; seeded tie-breaking with a caller-supplied monotonic timeline, connection-specific v2 selection, and revoked-slot checks after storage/rate waits are authored, but the full swarm-trace input/output harness and bulk evidence remain open |
| 11 | IO-013 | P1 | Persist authenticated v2 Merkle nodes and serve bounded proofs | IO-007, IO-010, IO-012 | wip | proof layers proven against root + LRU; persisted node store not started |
| 11 | NET-114 | P1 | Move v1, pure-v2 and hybrid transfer execution onto the shared core | NET-104, NET-105, NET-108, NET-109, NET-110, IO-008, IO-010 | todo | |
| 12 | NET-115 | P1 | Unify source scheduling and batch progress publication | NET-106, NET-107, NET-108, NET-114, SYS-009 | todo | |
| 12 | SYS-013 | P0 | Integrate one-writer migration and durable rollback | NET-114, IO-010, SYS-005, SYS-007, SYS-009, SYS-012 | todo | |
| 13 | IO-016 | P0 | Gate rollout on live-engine parity, recovery and hardware workloads | IO-008, IO-009, IO-010, IO-011, IO-012, IO-013, IO-014, IO-015, PERF-110, SYS-013 | todo | |
| 13 | NET-116 | P1 | Gate capability rollout with independent correctness and performance evidence | NET-108, NET-109, NET-111, NET-113, NET-114, NET-115, PERF-112, PERF-113, SYS-013 | todo | |
| 13 | PERF-116 | P1 | Run candidate soak recovery and reversible canary qualification | PERF-112, PERF-114, PERF-115, SYS-013, SYS-014 | todo | |
| 14 | SYS-015 | P0 | Integrate final gates, independent review and reproducible release | SYS-013, SYS-014, NET-116, IO-016, PERF-116 | todo | |

### Status snapshot (2026-10-17)

4 of 63 tickets `done`, 43 `wip`, 16 `todo`. `wip` means a bounded
implementation or contract slice landed and the rest of its scope remains;
applicable regression sources are recorded in Evidence and may be awaiting the
owner-requested bulk pass. Documentation-only work does not claim runtime
evidence. No gate has passed: G0
still needs SYS-009, SYS-011, PERF-101, and a valid baseline controller with
pinned client profiles. Nothing here claims a performance result; the bulk
test pass establishes correctness of the landed slices only.

### Immediate code sequence

1. **SYS-012** — add production-shaped compatibility and platform fixtures
   before the shared actors become the only execution path.
2. **Close the current P0 contract slices** — finish the open parts of
   **IO-001/004/005/006/007**, **NET-101–105/110**, and
   **PERF-101–106**. In particular, IO-007 still needs the live hybrid dual
   view even though bounded SHA-1/v2 hashing already uses the shared pool.
3. **Complete integration prerequisites** — finish **SYS-004/005/009/011/012**
   and the P0 performance ceilings/fixtures needed for honest live comparisons.
4. Keep the single bulk validation pass deferred until implementation and
   documentation are complete; then run the required regression and workspace
   gates against the exact candidate revision.

### Progress log

| Date | Change |
|---|---|
| 2026-10-17 | SYS-012 started: added deterministic payload-derived v1/v2/hybrid fixtures to `rt-testkit`; regression sources exercise production parsing, v1 SHA-1 pieces, v2 roots/layers, hybrid BEP 47 padding, and piece-map content ranges. Wired the fixture suite into Windows smoke; no tests or checks were run. |
| 2026-10-17 | IO-001 fault harness hardening: scripted backend teardown now releases any held completion workers so an interrupted regression cannot strand a test thread. No tests or checks were run. |
| 2026-10-17 | Plan created from the audit; F01, F02, F10, F14 re-verified in source at local HEAD. |
| 2026-10-17 | SYS-002 landed: v2 block vs piece extent split in `handle_v2_piece`, multi-block fixture (32 KiB pieces, sentinel payload), within-piece request pipelining. Note: the 5 `storage_jobs`/`update_save_path` move tests fail when `TMPDIR` is on the NFS scratch pool and pass on tmpfs (environmental, unrelated). |
| 2026-10-17 | SYS-003 started: F10 fixed. Release-mode regression is `cargo test -p rt-engine --lib --release webseed_block_is_debited` (debug builds pass either way, so the release run is the real check). |
| 2026-10-17 | SYS-003: F09 fixed. Per-peer upload buckets and `PeerCommand::UpdateUploadLimit` removed; peers draw on one shared per-torrent limiter. Test-env note: run `cargo test -p rt-engine` once with `TMPDIR` on NFS and once on tmpfs; 2 v2 crash-safety tests only pass on NFS, 5 storage-move tests only pass on tmpfs. |
| 2026-10-17 | SYS-003: F11 fixed. Policy change: code for all tasks first, bulk test pass at the end (owner request). Bulk-test backlog: full `rt-engine` suite on both TMPDIRs, clippy, `--release` webseed test. Known pre-existing load-flaky crash-safety tests (base commit also flakes). |
| 2026-10-17 | F29 fixed: `StorageRuntime::initialized()`; `/metrics` no longer initialises the global runtime and exports `torrentng_storage_runtime_initialized`. Test deferred to bulk pass. |
| 2026-10-17 | F35 fixed: `TrackerWorkerEvent::{Announce,Scrape}`; v2 uses `TrackerWorkers::announce_only()` (it never consumed scrape stats). Test `announce_result_is_not_held_back_by_a_stalled_scrape` written, run deferred to bulk pass. |
| 2026-10-17 | F14 fixed, F15 partly: `rt-piece-picker` uses packed `PieceFlags` (also availability `retired`), `priority_count` skips the priority scan, and `memory_bytes_for_piece_count` now budgets bitsets + u32 counts + 12 B/piece buckets (about 16.6 B/piece, up from 4.4). Engine piece-index leases grow accordingly. |
| 2026-10-17 | F27 timer half: `ActivityTimerWheel::cancel` is O(1) with a stale counter and compaction; test `cancelling_a_large_cohort_with_one_deadline_is_linear_and_bounded`. |
| 2026-10-17 | F28 fixed: `TaskExits`/`spawn_supervised` in `subsystems.rs`; command path calls `reap_exited_torrent_tasks` (O(exits)), the 1 s health tick keeps the full scan. |
| 2026-10-17 | F19 partly: `sync_all_open_files` no longer unions the shared pool's writable files; `write_at` marks dirty at issue (also narrows F21's lost-dirty window). Test `checkpoint_sync_covers_only_this_schedulers_files` run deferred. |
| 2026-10-17 | F06 partly: removed the per-peer requests-per-10-seconds ceiling; admission is the bounded request queue + pending-read cap. |
| 2026-10-17 | F08 (v1): `UploadContext.fast_negotiated`; rejected requests while choked / piece not held / queue saturated / read failed / choke transition; rate-wait interruptions requeue instead of dropping. Aborting in-flight reads on choke is skipped for Fast peers so their result can be rejected. Test `fast_peer_request_while_choked_is_rejected_not_dropped`. Chose real Reject semantics over the stop-advertising interim. |
| 2026-10-17 | F18 partly: `PeerReadStreams` gates readahead on sequential continuation. Test `readahead_expands_only_continuing_streams`; existing readahead cache tests may need a sequential first read (check in bulk pass). |
| 2026-10-17 | F18: gate is runtime-toggleable (`[storage] peer_read_readahead_sequential_only`, default true; documented in CONFIGURATION.md). Existing readahead cache scheduler tests set it to false to keep their expansion semantics. `cargo check --workspace --all-targets` is clean. |
| 2026-10-17 | F25 fixed: nested copy entries skip verification; test `deep_copy_hashes_each_leaf_once_per_side` (thread-local hash counter). |
| 2026-10-17 | F26 partly: `SessionSnapshot` stores `Arc<TorrentEntry>`; test `changing_one_row_shares_every_other_snapshot_row`. The 711k-allocation probe should be re-run in the bulk pass to quantify. |
| 2026-10-17 | F13 partly: `enforce_piece_assembly_budget` spills evicted assemblies via `write_block` and marks the piece restored-partial; falls back to reject on write failure. Test `evicted_piece_assembly_is_spilled_to_disk_not_discarded`. |
| 2026-10-17 | F34: `scripts/memory_roadmap_certification.sh` only accepts source reports whose `- Commit:` matches HEAD, records worktree state, and labels stale evidence; `scripts/test_memory_roadmap_certification.py` (3 cases) passes. Other certification aggregators still select by mtime. |
| 2026-10-17 | F24 partly: `V2ProofCache`, `v2_proof_layers`, `read_v2_leaf_hashes`; serving now refuses hashes of content that does not match `pieces root` (a behaviour tightening). Tests `proof_layers_are_refused_for_unverified_content_then_cached`, `proof_cache_is_byte_bounded_and_evicts_least_recently_used`; existing v2 hash-exchange tests must be re-run in the bulk pass. |
| 2026-10-17 | F23 partly: `V2FileVerifier::CHUNK_LEAVES`, `MountScheduler::hash_v2_leaves`. Test `v2_file_verify_batches_leaf_hashing_per_chunk`. |
| 2026-10-17 | F12 partly: `Choker::run_with_mode(ChokeMode)`, optimistic history + seeded tie-break, engine passes decayed rates. Tests include mode ranking and fair rotation; replay coverage is authored but awaits the bulk pass. |
| 2026-10-17 | F05 partly: `adaptive_request_pipeline` (rate x 0.25 s, bounded). Uses delivery rate rather than measured RTT because the peer loop does not yet report per-request delay. Test `request_window_grows_only_for_fast_peers_and_never_under_pressure`. |
| 2026-10-17 | F17 partly: `FilePool::lookup_valid`, `sweep_idle_if_due_locked`, `mode_satisfies`; tests `write_descriptor_serves_reads_without_reopening`, `idle_sweep_runs_on_an_interval_not_on_every_lookup`. |
| 2026-10-17 | F27 estimate half: `TierController::estimated_owner_bytes`; not yet wired into the dormant budget check (PERF-109 live-owner fixtures decide the cap). Test `owner_bytes_cover_maps_and_timers_not_just_the_hash_string`. |
| 2026-10-17 | Code-first phase: `transfer/` core (coords, ledger, obligations, upload, rate, window, sim) committed. Its tests, and all tests added since the last bulk run, have not been executed. |
| 2026-10-17 | SYS-001 docs completed: added the native/shared-core capability matrix and a per-ticket specification template; linked both from `docs/README.md`. |
| 2026-10-17 | NET-109 implementation slice: replaced per-piece `BTreeSet` nodes with swap-remove vector buckets and piece positions, indexed only fresh-work candidates, randomized equal-rarity peer choices, moved endgame traversal to bounded in-progress pieces, and added picker memory accounting that includes bucket capacity and the configured peer ceiling. Regression tests are authored; no tests or checks were run in this code-first stretch. |
| 2026-10-17 | Architecture review: retained the native rebuild as a reversible candidate after reviewing `librqbit` 9.0.1 and libtorrent as maintained alternatives/references. `librqbit` remains a serious alternate-engine candidate; exact Rust 1.88/dependency compatibility, storage-adapter fit, and matched whole-client results are open. Recorded D5 and added `TRANSFER_CORE_REFERENCE_EVALUATION.md`; no implementation checks or benchmarks were run. |
| 2026-10-17 | PERF-101 started: added the measurement contract plus strict `transfer-trial-v1` and `transfer-manifest-v1` JSON Schemas under `certification/transfer-rebuild/`. Validation and scorecard tooling remain open; no tests or checks were run. |
| 2026-10-17 | PERF-102 started: added an independent Linux `/proc`/cgroup-v2 observer with raw CSV samples, separate RSS/PSS/cgroup memory domains, cumulative CPU and disk counters, and a JSON peak summary. No tests or checks were run. |
| 2026-10-17 | SYS-005 partly: `SessionRegistry::delta_since_page` caps each journal read at 1,024 records, coalesces repeated updates by torrent, and returns an explicit continuation cursor. SSE and qBittorrent sync publish bounded pages; native/qBittorrent snapshot-cache refresh folds pages into changed-hash sets instead of cloning the full journal. Added registry regression sources; bulk validation remains deferred. |
| 2026-10-17 | SYS-006 partly: SSE and qBittorrent sync now publish at most 1,000/1,024 journal changes per response and advance the client cursor through the emitted page. Corrected qBittorrent RID encoding so revision zero and revision one cannot alias; added route regression sources for backlog continuation and the first torrent after an empty snapshot. No tests or checks were run. |
| 2026-10-17 | IO-011 partly: per-file readahead growth now uses actual cache feedback: the first sequential continuation bootstraps one prefetch, a cache hit grows the recommendation gradually, and a miss does not expand speculative I/O. Added controller regression sources; reusable multi-range policy and bulk validation remain open. |
| 2026-10-17 | SYS-008 partly: promoted-torrent idle deadlines now distribute across one-second buckets over 10% of the delay (up to 30 seconds), limiting synchronized maintenance after bulk restore while keeping the timer wheel's deadline-key count bounded. Added a 1,000-torrent regression source; broader pacing and bulk validation remain open. |
| 2026-10-17 | IO-001 started: documented evidence states, per-file sequence barriers, cancellation ownership rules, and deterministic fault cases. Added a test-only scripted backend for read/write/sync errors, held completions, and partial-write failures, with regressions for cancelled waiters, incomplete writes, and a failed barrier retry. Tests are authored but unrun; file-replacement and crash-cut coverage remains open. |
| 2026-10-17 | IO-001 expanded: added scheduler regression sources for injected read failures (without dirtying payload state) and injected write failures (dirty until a covering barrier). Bulk validation remains deferred; operation-lease, short-I/O, replacement, and crash-cut coverage remain open. |
| 2026-10-17 | IO-004 expanded: public `HandleCache` now prefers a current read+write descriptor for reads and drops the cached read-only entry when publishing a writer; added regression source proving read-only descriptors never satisfy writes and active leases keep retired descriptors charged. Shared catalog unification remains open; bulk validation remains deferred. |
| 2026-10-17 | NET-109 follow-up: randomized equal candidates from a webseed's zero-availability set, including priority ties; added seeded candidate coverage. Corrected the in-progress state memory comment. Tests remain deferred. |
| 2026-10-17 | IO-002 started: v1 peer receive/upload and v2 receive/serve paths consume typed block extents for range validation; v2 assembly, read-back, and progress use the full typed piece extent. Existing multi-block regressions remain unrun; broader shared-ledger integration remains open. |
| 2026-10-17 | PERF-102 follow-up: descendant discovery includes non-leader process threads; missing FD observations remain null instead of partial sums; cgroup `inactive_file` is emitted separately from file-backed memory. Added synthetic thread-child coverage; bulk pass remains deferred. |
| 2026-10-17 | SYS-009 started: `DbExecutor::Worker` is now available in test builds, and the transfer-stat batching regression uses a file-backed supervised worker, checks the batched transaction metric, and reads the updated row after shutdown/reopen. The actor test and bulk worker suite remain unrun. |
| 2026-10-17 | IO-003 started: v1 peer and webseed block events now carry a private lease-owning wrapper through the actor's async storage handling; webseed body reservations no longer drop when fetch returns. Partial assembly flush copies take temporary peer-buffer leases, while budget evictions move the assembly allocation into shared immutable block slices so spill does not allocate a second piece-sized copy. Added lease-lifetime regression sources; no tests run. Shared cross-plane payload spans and storage-operation ownership remain open. |
| 2026-10-17 | SYS-011 continued: added a metric-owner inventory and passive-scrape contract. A descriptor-budget scrape used to initialize its lazy gate and could raise RLIMIT_NOFILE; metrics now use a passive accessor and expose an initialized gauge. Added a fresh-process regression source (unrun) and explicit storage-owner setup in the renderer-output test. Mixed-load evidence remains pending. |
| 2026-10-17 | NET-103 started: v1 peer request admission now acquires a PeerBuffer lease before queueing the request; the lease moves through the peer command and outstanding request, then into the matching immutable block event. Choke/reject/timeout/disconnect and failed-event paths release or return the coordinate without retaining an unowned payload. A denied reservation returns the picker slot and retries from the existing one-second peer-control tick after capacity becomes available. Added denial/retry and request-to-event lease regression sources; not run. Shared spans and v2/hybrid/webseed request admission remain open. |
| 2026-10-17 | NET-105 v2 protocol follow-up: pure-v2 upload rejection now emits Reject Request only when Fast was negotiated; Fast Choke preserves outstanding download obligations and their piece assemblies, while legacy Choke clears them. Added focused obligation/choke regression sources; not run. Outgoing Cancel and shared-core integration remain open. |
| 2026-10-17 | NET-105 v1 Choke follow-up: Fast Choke preserves on-wire requests and their response-byte leases; queued request commands carry a generation so the peer loop discards work queued before Choke, and the actor cancels queued-only picker reservations. Legacy Choke still releases all requests. Added focused lease and queue-reconciliation regression sources; not run. Outgoing Cancel and shared-core integration remain open. |
| 2026-10-17 | NET-105 v1 Cancel follow-up: a remote Cancel removes a request still waiting in the bounded upload-read queue; if Fast was negotiated it receives Reject, while a read already in service remains obligated to finish through the Piece/Reject path. Dedicated queued/in-service Cancel regression coverage and shared-core integration remain open. |
| 2026-10-17 | NET-105 v1 endgame follow-up: accepting a block queues Cancel to peers holding duplicate requests. The peer loop sends Cancel only for requests already on wire and retains those response leases until Piece, Reject, timeout, or disconnect. v2 and shared-ledger cancellation integration remain open; no tests run. |
| 2026-10-17 | IO-009 started: per-path shared write guards and exclusive sync fences now make a barrier wait for admitted backend completions and block later writes until fdatasync. Issued/synced sequence watermarks pair with dirty generations; the backend watcher retains fences if a caller is canceled, and preallocation carries its exclusive fence into the blocking job. New ordering regressions and grouped device batching remain open; no tests run. |
| 2026-10-17 | NET-103 pure-v2 follow-up: reserve PeerBuffer response capacity before each `Message::Request`, retain it with the outstanding coordinate through piece storage/hash handling, release it on Reject/legacy Choke/disconnect, and retry denied admissions on a one-second peer tick. Focused v2 admission coverage and hybrid/shared-span integration remain open; no tests run. |
| 2026-10-17 | IO-004 started: centralized platform-specific file identity in `open.rs` so the scheduler descriptor cache and `HandleCache` share replacement checks; both caches now perform path identity I/O outside their mutexes and verify the candidate pointer before refresh/removal. Replacing the two cache/catalog implementations and remaining lifecycle coverage are open; no tests run. |
| 2026-10-17 | PERF-101 follow-up: added a pinned Draft 2020-12 validator for schemas and manifest references; it checks relative non-symlink paths, artifact/trial SHA-256, trial schema, pair/outcome and commit/worktree identity. Complete-trial, malformed identity, and referenced-file regression sources are authored but unrun; scorecard tooling remains open. |
| 2026-10-17 | IO-005 started: queue/device permits and queued-byte charges now move into blocking file preparation and backend completion watchers, so canceling the caller cannot free admitted capacity while submitted work remains in flight. Strict writes retain admission through their data sync; helper-level cancellation owner regressions were added but remain unrun; the end-to-end fault/cancellation matrix remains open. |
| 2026-10-17 | IO-005 expanded: added end-to-end scripted-write regressions proving caller cancellation retains the scheduler queue slot and ResourceGovernor queued-disk lease until backend completion, rejects work at each limit, and restores admission afterward. Bulk validation remains deferred; the full read/write/sync fault and cancellation matrix remains open. |
| 2026-10-17 | PERF-104 started: added a fresh-directory file barrier, per-file interval-union accounting for unique and duplicate accepted payload bytes, and separate monotonic phase markers for transfer, flush, validation, and shutdown. Harness regression sources and contract notes are authored; client adapters, fixtures, paired execution, and bulk pass remain open. |
| 2026-10-17 | PERF-105 started: SHAKE-256 counter-based per-file payload generation writes through a private staging directory under a fresh caller-selected output path, records per-file digests plus an aggregate fixture identity, and verifies both manifest and payload content. Torrent metainfo, cache controls, workload profiles, and bulk-pass evidence remain open; no tests were run. |
| 2026-10-17 | PERF-105 metainfo follow-up: fixture generation can now create deterministic private v1 multi-file metainfo for a caller-supplied tracker URL. `torrent.json` binds tracker, name, piece length, info hash, torrent digest, and fixture digest; verification rebuilds the torrent from payload bytes. Added determinism and tamper regression sources; not run. V2/hybrid metainfo, cache controls and workload profiles remain open. |
| 2026-10-17 | NET-104 follow-up: bounded early-commit reconciliation history per connection, with a hard cap and simulator invariant checks. Added a regression source for history eviction; shared-core actors remain on their reference paths and the bulk pass is deferred. |
| 2026-10-17 | NET-110 follow-up: disconnect now ends protocol obligations without releasing capacity for reads already dispatched to storage; their job and byte budget remains charged until completion and the result is discarded. Added a regression source for disconnect/replacement admission; not run. |
| 2026-10-17 | NET-110 budget follow-up: invalid zero/oversized wire lengths are refused, and configured byte budgets normalize to one legal block then enforce a strict cap. Added Fast/legacy and undersized-budget regression sources; not run. |
| 2026-10-17 | NET-110 identity follow-up: a connection generation cannot be registered twice or reused while an earlier storage read is still live. Added a regression source for disconnect/reuse/completion ordering; not run. |
| 2026-10-17 | NET-111 follow-up: `Choker::run_with_mode_at` lets seeded traces replay optimistic rotation on a caller-owned monotonic timeline. Pure-v2 peers receive mode-aware slots from the same policy, selection is keyed by connection ID, and in-flight reads recheck slot membership after storage and rate waits; revoked Fast requests receive Reject. Added seeded replay, reconnect-identity, and v2 transition regression sources; none were run. |
| 2026-10-17 | IO-011 follow-up: peer-read readahead now grows per file only as sequential requests continue, beginning at one block and respecting the configured maximum; discontinuities reset the recommendation to an exact read. The existing queued-byte lease and frame reservation precede backend dispatch. Added a controller regression source; cache-hit feedback and the full range-cache policy remain open and untested. |
| 2026-10-17 | PERF-113 started: added manifest-bound descriptive scorecard generation with same-signature pairing, invalid/incomplete exclusion reasons, per-pair resource ratios, deterministic median percentile-bootstrap intervals, a minimum-sample indicator, and no performance thresholds. Added scorecard schema/validator support and regression sources. PERF-110's automated paired runner and the bulk pass remain open; no tests or checks were run. |
| 2026-10-17 | NET-104 resource bounds: ledger piece-state admission now accounts for a conservative logical byte estimate and releases it on verify, failure, have, and epoch transitions. Live connections cap at 4,096, peer request depth at 1,024, torrent-wide request owners at 65,536, and one block at four endgame owners. Regression sources cover admission/release and the hard caps; no tests or checks were run. |
| 2026-10-17 | NET-112 evaluation update: upstream `urtorrent-utp` remains a protocol/design reference, but Cargo metadata reports both reviewed releases (0.14.2 and 0.14.3) require Rust 1.98, above TorrentNG's Rust 1.88 floor. Do not add the crate or raise the native MSRV for it. Keep the bounded window, SACK, reorder, timeout, and shared-socket scheduling lessons; build the compatible core and manager in `rt-utp`, then compare correctness and whole-client efficiency before rollout. |
| 2026-10-17 | NET-113 integration update: added `UtpRuntime` with one lazily bound shared endpoint per family, installed the incoming listener endpoint into the runtime, and routed production v1, pure-v2, and metadata outbound connects through it. Listener rebind retires old endpoints so active routes can drain; the retained generation count is capped. Added a shared-socket/rebind regression source. No tests or builds run; deadline/ready scheduling, aggregate memory evidence, and performance qualification remain open. |
| 2026-10-17 | PERF-103 started: added read-only identity/settings adapters for TorrentNG, qBittorrent, Transmission, Deluge, rTorrent, and rqbit; pins immutable images or a native binary plus its live process/build, redacts credentials, and emits linked client-observation artifacts. Trial schema and manifest validation cross-check observation digest, settings digest, client version/runtime, and process start identity. Regression sources and docs are authored; nothing was run. Exact client profiles, rqbit's launch-configuration boundary, paired-run assembly, and bulk qualification remain open. |
| 2026-10-17 | PERF-103 hardening: native API ownership now checks the actual loopback listener address; container port publication rejects wildcard/non-loopback host bindings, and host-network containers must expose a loopback listener. Added regression sources for wildcard rejection and loopback acceptance; not run. |
| 2026-10-17 | PERF-103 identity hardening: native executable and PID-file reads now use no-follow regular-file descriptors; executable metadata is compared before and after hashing before the running process is matched to its inode. Added symlink-rejection regression source; not run. |
| 2026-10-17 | PERF-103 maintained-engine comparison: added a separate Linux-only `urtorrent = 0.14.3` runner pinned to Rust 1.98.1, process-bound settings snapshot, private v1 isolation, and v2/hybrid rejection. The runner binds to an explicit task-local IPv4 interface, dials one private IPv4 peer, removes trackers/web seeds, disables IPv6/discovery, and waits for the common start gate; the adapter independently validates the listener/peer and snapshot process identity. `hash_threads` is labeled as a launch input because the library settings API does not expose it. Architecture docs keep this a comparator and the native candidate reversible. Regression sources are authored; no build, tests, or benchmark run. Full paired execution and qualification remain open. |
| 2026-10-17 | IO-007 hash-pool completion: v2 Merkle accumulator finalization now runs as one bounded shared-pool job after the existing 64-leaf hash batches; a queue-full retry clones only the small streaming accumulator. The batch regression asserts exactly three leaf jobs plus the root job. Hybrid dual-view verification remains open because production hybrid metadata still routes through the v1 actor. No tests or checks run. |
| 2026-10-17 | IO-006 started: device admission is keyed only by physical device ID, so profile differences cannot split the same device's permit pool. Capacity follows shared storage I/O depth while per-mount `max_queue` remains separately enforced; engine queue summaries also deduplicate by device ID rather than profile. Added scheduler and aggregation regression sources; common priority dispatch and end-to-end completion ownership remain open. No tests run. |
| 2026-10-17 | IO-006 continued: replaced the shared device semaphore with a bounded per-device dispatcher. Active work uses one slot per portable worker or one bounded io_uring submission batch; pending classes use the `DeviceElevator` weights, canceled waiters release pending capacity, and operation IDs remain attached to device permits through completion. Scheduler positioned I/O, peer-read batches, preallocation and extent probes use the service; cache hits and file-open preparation do not occupy device slots. Added weighted-fairness and canceled-waiter sources and extended the held-write completion regression to assert that caller cancellation does not release the device slot early; no tests or checks run. Secure copy plans, explicit deadlines/choke-critical promotion, cross-mount coalescing, grouped barriers and full fault qualification remain open in IO-014, NET-114, IO-009 and the IO-006 bulk matrix. |
| 2026-10-17 | IO-006 follow-up: added `MountScheduler::run_device_operation` as the reusable bounded handoff for non-scheduler blocking work. It routes through the existing mount queue, byte budget, shared per-device dispatcher, and backend worker, retaining ownership through completion. Added `external_device_handoff_shares_admission_across_mounts` as a regression source. Secure-copy plans have not adopted the handoff; no tests or checks run. |
| 2026-10-17 | PERF-105 v2/hybrid and cache-control slice: added deterministic BEP 52 file roots and piece layers, BEP 47 hybrid padding, a versioned torrent-record envelope with legacy v1-record verification, strict LAN/v2/hybrid fixture profiles, and Linux per-file cache requests with mincore residency observations. Added bdecode-level regression sources for roots, layers, file ordering, padding, profile identity and cache record ownership. No tests or checks were run; filesystem-specific cache qualification and the requested bulk pass remain open. |
| 2026-10-17 | PERF-105 evidence linkage: added a strict cache-state result schema; cold/warm trial records now require the observation digest, and manifest validation checks the cache artifact's digest, measured status, fixture identity, requested state, and per-file totals. Added schema and manifest-link regression sources; all remain unrun. |
| 2026-10-17 | IO-009/IO-010 durability evidence slice: checkpoint sync now closes scheduler-wide write admission before taking its dirty/active path snapshot, so writes to paths first used during the barrier wait. Successful barriers return an opaque proof that retains admission through the fastresume atomic write, including cancellation after the blocking writer starts; the persisted record carries synced-file count and a stable scheduler write-sequence watermark, while fast/unsupported saves remain explicitly unsynced. Added held-sync new-path and real-scheduler fastresume-store regression sources; none were run. Grouped device barriers, crash-cut qualification, and SYS-009 persistence integration remain open. |
| 2026-10-17 | PERF-106 started: added pinned loss-free Linux egress profiles restricted to task-owned `tngt-*` namespaces and `tngt*` veths, with exact `tc -j` readback, immutable apply/reset artifacts, and identity-checked removal. Trial and manifest schemas now link applied profile evidence by digest. Added shaper and validator regression sources; none were run. Independent-swarm generation, paired endpoint orchestration, and deterministic loss/jitter controls remain open. |
| 2026-10-17 | PERF-107 started: added versioned independent calibration and ceiling-report schemas, directional network/disk limits, v1/v2/hybrid hash capacity, pipeline math from peer windows and measured RTT, immutable report generation, manifest raw-evidence linkage/recomputation, and optional trial linkage to matching host/device/network/protocol and process-bound client/version/settings readback. Pipeline peer, request-depth, and block counts are checked against the trial and exact settings pointer. Regression sources include invalid and out-of-range numeric inputs; no tests, schema checks, calibration, or benchmark runs. |
| 2026-10-17 | NET-109 picker memory follow-up: rarity buckets now trim capacity after any removal when retained slack exceeds four times live membership, including swap-removals from the middle; added a high-churn capacity/position regression source. Not run. |
| 2026-10-17 | IO-004 path-publication follow-up: scheduler and public handle-cache opens now recheck the opened descriptor against the current path before caching or returning it, retrying up to eight concurrent replacements; scheduler sync opens use the same check. Added a deterministic stale-identity regression source. Not run. |
| 2026-10-17 | PERF-102 artifact follow-up: observer CSV and summary outputs now use exclusive mode-0600 creation and file sync, so reruns cannot overwrite prior resource evidence; added output immutability regression source. Not run. |
| 2026-10-17 | SYS-012 continuation: compatibility fixtures now drive the production v2 piece map as well as the v1 map, checking aligned file offsets, a multi-piece file tail, the short final piece, empty-file omission, and hybrid logical-length parity. The Linux/Windows regression commands remain deferred. |
| 2026-10-17 | IO-011 continuation: a 1,024-file stream-history table now evicts one least-recently-used entry instead of clearing all histories when a new file arrives. Added a hot-stream retention regression source; no tests run. Cache-hit feedback and a reusable range policy remain open. |
| 2026-10-17 | IO-014 started: production storage-plan copy/import steps now receive a destination `MountScheduler` built with the engine's shared resource governor and configured I/O limits. The root-confined secure executor sends each copy step through `run_device_operation` with a 64 KiB streaming working-set charge; checkpoint writes remain outside the device permit. Cross-device source admission, directory-entry memory limits, and bulk qualification remain open; no tests or checks run. |
| 2026-10-17 | IO-014 follow-up: storage-plan steps now classify every source/destination path and acquire all touched physical-device queues in stable order, retaining both permits through the secure operation and cleanup. Unknown topology uses one stable dispatcher fallback instead of a new per-job identity. Added dual-device ownership and opposing-copy ordering regression sources; directory-entry bounds, copy/hash splitting, and the deferred bulk pass remain open. No tests or checks were run. |
| 2026-10-17 | IO-014 memory follow-up: Unix and Windows secure verification now stream directory names instead of retaining full source/destination vectors or hash sets; Unix copy, length accounting, and recursive deletion also stream, with deletion retaining at most 256 names per batch and restarting enumeration after removal. Added extra-entry, stream, and batch-bound regression sources. Long-copy preemption and the deferred bulk pass remain open; no tests or checks were run. |
| 2026-10-17 | IO-014 regular-file preemption slice: root-confined regular-file `CopyVerifyRename` and copied `ImportExisting` steps now advance copy, source hashing, and destination hashing in separate 64 KiB scheduler quanta, reacquiring source/destination device queues between quanta and yielding to queued work. The touched queue set is resolved once per plan step to avoid repeated topology discovery. Unix same-filesystem regular imports keep the hard-link fast path. Cursor state retains descriptors/hash state but no copy buffer; cancellation and operation errors remove the partial file before the active admission is released, while a terminal scheduler rejection performs root-confined cleanup so the cursor cannot orphan the partial output. Added Unix/Windows bounded-quantum and cleanup regression sources plus a Unix hard-link regression source. Directory-tree copies, final sync latency, and bulk qualification remain open; no tests or checks were run. |
| 2026-10-17 | IO-005 completion ownership follow-up: added a held-backend peer-read regression source proving the expanded readahead charge stays in the queued-disk governor after caller cancellation, rejects a later read at the memory cap, and becomes available after backend completion. The queue-slot and write-byte-budget regressions remain alongside it; the full injected-fault matrix and all bulk validation remain open. No tests or checks were run. |
| 2026-10-17 | IO-005 strict-write ownership follow-up: added a held-sync regression source showing caller cancellation preserves queue/byte admission through strict fdatasync, keeps dirty evidence for a later checkpoint, rejects over-budget work while held, and restores admission after completion. The full injected-fault matrix and bulk validation remain open; no tests or checks were run. |
| 2026-10-17 | IO-005 failure-release follow-up: pre-submission open errors and injected read, write, partial-write, and barrier failures now assert queued-byte and device-slot recovery while preserving dirty evidence where required. These regression sources remain unrun; the complete device-service fault matrix and bulk validation remain open. |
| 2026-10-17 | IO-005 strict-sync fault follow-up: added a regression source for an injected fdatasync error during a strict write, checking that queue/device/byte admission returns while dirty evidence survives until a successful barrier. Source remains unrun; the full device-service fault matrix and bulk validation remain open. |
| 2026-10-17 | SYS-004 tier-identity slice: tier and timer maps now use an 8-byte slot/generation ID; one interned key allocation backs the lookup index and reverse slot, while the public dormant snapshot keeps its separately accounted string. Slot reuse advances generation, stale timer events are covered by regression source, and retained map/queue capacities are included in owner accounting. Engine runtime maps and all regression/bulk evidence remain open; no tests or checks were run. |
| 2026-10-17 | NET-103 pure-v2 admission coverage: added a socket-backed regression source proving denied requests are not sent, capacity recovery retries the v2 window, each request coordinate retains its own byte lease through full Piece storage/verification, and disconnect drops remaining leases. Updated the Fast/legacy Choke regression to assert lease retention/release. Shared hybrid spans and live-ledger integration remain open; no tests were run. |
| 2026-10-17 | NET-103 pure-v2 Reject coverage: extracted the exact-coordinate/length response release check used by the v2 peer loop and added a regression source proving mismatched Rejects retain capacity while a matching Reject releases it. No tests were run; hybrid/shared-span integration and bulk validation remain open. |
| 2026-10-17 | IO-006 external cancellation ownership: added a regression source proving cancellation of `run_device_operation` after dispatch keeps its queued-byte lease and shared physical-device permit until the blocking closure finishes; a second mount is rejected while that work remains active, then succeeds after completion. Updated the IO-006 contract and tracker. No tests or checks were run. |
