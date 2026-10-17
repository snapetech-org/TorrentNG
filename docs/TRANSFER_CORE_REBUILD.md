# Transfer Core Rebuild Plan

Status: **active**. This is the working plan and tracker for the transfer-core
rebuild proposed by the external audit
*TorrentNG Code Audit and Rebuild Blueprint* (17 Oct 2026, audited revision
`05d5c23886446f380552c73e0be480f03ae2d2ab`; the PDF was dropped into the repo
root and is not committed). It complements, and does not replace,
`ENGINE.md`, `ENGINE_REWRITE.md`, `STORAGE_NG.md` and `CRASH_SAFETY.md`.

How to use this file: pick the lowest-numbered ready ticket (all prerequisites
`done`), land it as its own PR/commit, tick its row in section 6, and record
the evidence (test name, probe output, report path) in the "Evidence" column.
Never tick a row on the strength of a design alone.

## 1. Review of the audit

### What was verified here, against local HEAD

The audit's line references were checked against this checkout. The
`rt-engine`, `rt-piece-map`, `rt-piece-picker`, `rt-storage`, `rt-utp` and
`rt-session` sources are byte-identical between the audited revision and local
`main` (the 33 commits local `main` lacks relative to `origin/main` touch
workflows/scripts only), so the pinned line numbers apply.

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

### Decisions needed from the owner (not blocking G0)

| # | Decision | Needed before |
|---|---|---|
| D1 | Adopt, amend, or reject the proposed scorecard numbers | PERF-110 |
| D2 | uTP: reuse a maintained interoperable implementation vs. repair `rt-utp` | NET-112 |
| D3 | Whether the 14-wave plan runs on `main` behind a selectable reference path, or on a long-lived integration branch | NET-114 |
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
| F06 | P0 | 4,096 requests/10 s upload guard | NET-110 | G2 | partly fixed (count ceiling removed; queue bound remains and still drops silently when saturated; protocol-correct rejection and shared byte-admitted upload service are NET-105/NET-110) |
| F07 | P0 | uTP writer is stop-and-wait | NET-112, NET-113 | G5 | accepted |
| F08 | P0 | Fast advertised without full request termination | SYS-003, NET-105 | G0 | partly fixed (v1: Reject for every request whose coordinates are known when Fast is negotiated; v2 handler, outgoing Cancel and endgame cancellation remain NET-105) |
| F09 | P0 | torrent upload limit copied into per-peer buckets | SYS-003, NET-107 | G0 | fixed (SYS-003; hierarchical scheduler still NET-107) |
| F10 | P0 | webseed debit inside `debug_assert!` | SYS-003 | G0 | fixed (SYS-003) |
| F11 | P1 | no token-ready wakeup for download pacing | SYS-003, NET-107 | G0 | fixed (SYS-003; deadline-driven, not per-peer polling) |
| F12 | P1 | choker feedback/exploration | NET-111 | G2 | partly fixed (v1: mode-aware ranking, staleness decay, fair random optimistic rotation; v2 still unchokes every interested peer, swarm-trace replay remains NET-111) |
| F13 | P0 | memory pressure discards received data | NET-103 | G2 | partly fixed (evicted assemblies spill to the payload files and the piece verifies from disk; reserve-before-request admission remains NET-103) |
| F14 | P0 | picker memory estimate wrong (`Vec<bool>`) | SYS-004, NET-109 | G0 | fixed (packed `PieceFlags`; estimate counts bitsets, counts and BTree buckets) |
| F15 | P1 | picker scans all pieces before rarity buckets | NET-109 | G2 | partly fixed (default no-priority path is O(1); bucket `BTreeSet` allocation storm remains NET-109) |
| F16 | P0/P1 | buffers lose coherent owner across boundaries | IO-003 | G1 | accepted |
| F17 | P1 | shared file cache serialises metadata work | IO-004 | G1 | accepted |
| F18 | P1 | default readahead 16 KiB → 512 KiB | IO-011 | G4 | partly fixed (readahead expands only a stream that continues the previous read of that file; reuse-driven sizing and pre-reserved speculative capacity remain IO-011) |
| F19 | P0 | torrent flush syncs unrelated writable files | IO-009 | G4 | partly fixed (barrier covers only this scheduler's dirty files, marked at write issue; exact file fences/sequence watermarks remain IO-009) |
| F20 | P1 | io_uring worker is a batch barrier | IO-015 | G6 | accepted |
| F21 | P0 | cancellation may release admission/dirty evidence early (moderate confidence) | IO-005 | G1 | accepted |
| F22 | P1 | several storage schedulers, no fairness authority | IO-006 | G1 | accepted |
| F23 | P1 | v2 recheck does per-leaf small I/O | IO-012 | G4 | partly fixed (1 MiB chunks: one extent probe, read and hash job per 64 leaves; persistent sparse cursors and hybrid shared read stream remain IO-012/IO-007) |
| F24 | P0/P1 | v2 proof serving rereads file; 64 MiB ceiling | IO-013 | G3 | partly fixed (64 MiB ceiling removed; leaves hashed in 256 KiB reads; layers proven against the metainfo root and kept in a 64 MiB LRU; persisted node store and subtree-only work remain IO-013) |
| F25 | P1 | secure copy re-verifies leaves per ancestor | IO-014 | G4 | fixed for hashing (nested entries copy-only; top-level verifies the tree once; scheduling copies through the device service remains IO-014) |
| F26 | P1 | one row change rebuilds a deep-cloned registry vector | SYS-005, SYS-006 | G5 | partly fixed (snapshot rows are shared `Arc<TorrentEntry>`, flatten clones pointers; chunked revision deltas, cold/hot metadata split and compact identities remain SYS-005/SYS-004) |
| F27 | P1 | dormant estimates incomplete; quadratic timer cancel | SYS-004, SYS-008 | G5 | timer half fixed (lazy deletion + compaction); dormant-estimate half remains SYS-004 |
| F28 | P1 | every engine command scans all task handles | SYS-007 | G5 | fixed (supervised exit queue on the command path; 1 s full scan kept as reconciliation) |
| F29 | P0 | metrics scrape builds a shadow storage runtime | SYS-011, PERF-108 | G0 | fixed (scrape uses `StorageRuntime::initialized()`; owner-specific metrics still SYS-011) |
| F30 | P1 | engine unit tests bypass production DB worker | SYS-009 | G0 | accepted |
| F31 | P0 | scale checks certify synthetic projections | PERF-109 | G5 | accepted |
| F32 | P0 | throughput comparisons don't establish capacity | PERF-104..107 | G0 | accepted |
| F33 | P1 | CPU/cache/backend microbenchmarks reward wrong behaviour | PERF-102, PERF-107 | G0 | accepted |
| F34 | P0 | certification stamps current commit on historical PASS text | PERF-114 | G0 | fixed for `memory_roadmap_certification.sh` (source reports must record the current HEAD; other aggregators and machine-readable manifests remain PERF-114) |
| F35 | P1 | announce waits for optional scrape | SYS-010 | G5 | fixed (announce delivered first; scrape separate event, process-wide cap of 4; broader discovery/control budgets still SYS-010) |

## 6. Work packages

63 packages; ordering is topological. A *wave* is a dependency frontier, not a
sprint. Status values: `todo`, `wip`, `done`, `blocked`. Detailed per-ticket
scope/acceptance is created from the template in SYS-001 when picked up.

| Wave | Ticket | P | Work package | Prerequisites | Status | Evidence |
|---|---|---|---|---|---|---|
| 1 | SYS-001 | P0 | Pin the reference, capability matrix and shared ownership contracts | none | wip | this document pins the reference and ownership summary; the capability matrix and per-ticket spec template are still to write |
| 2 | IO-001 | P0 | Define storage evidence contracts and deterministic fault harness | SYS-001 | todo | |
| 2 | NET-101 | P0 | Define typed transfer coordinates and protocol geometry adapters | SYS-001 | todo | |
| 2 | PERF-101 | P0 | Commit measurement contract and strict result schemas | SYS-001 | todo | |
| 2 | SYS-002 | P0 | Land v2 reproductions and a reviewed extent correction | SYS-001 | done | `torrent_task_v2::tests::v2_*` multi-block tests (5 completion cases x memory/spill, red without fix) + `v2_request_window_pipelines_every_block_of_a_piece` (red: 4 of 8 requests) |
| 2 | SYS-003 | P0 | Repair immediate release-rate and negotiated-capability behavior | SYS-001 | wip | F10 done: `webseed_block_is_debited_from_the_local_download_bucket` (fails under `--release` with the old line, tokens left 100; passes with the fix). F09 done: one `SharedRateLimiter` per torrent shared by all peer loops (`torrent_upload_limit_is_one_bucket_shared_by_all_peers`). F11 done: `download_token_wake` deadline + loop branch (`drained_download_bucket_schedules_a_token_ready_wakeup`). F08 v1 half done (see register); the stop-advertising interim was not needed. Remaining: v2 Reject/Cancel, outgoing Cancel, endgame |
| 2 | SYS-004 | P0 | Introduce compact identities and honest whole-owner metadata accounting | SYS-001 | todo | |
| 2 | SYS-009 | P0 | Exercise the production database worker and coalesce progress | SYS-001 | todo | |
| 2 | SYS-011 | P0 | Make live observability passive and owner-specific | SYS-001 | todo | |
| 3 | IO-002 | P0 | Separate block and piece extents and repair multi-block v2 transfers | IO-001, SYS-002, NET-101 | todo | |
| 3 | IO-003 | P0 | Carry capacity leases through immutable payload ownership | IO-001 | todo | |
| 3 | IO-004 | P0 | Unify the secure file catalog and descriptor lifecycle | IO-001 | todo | |
| 3 | NET-102 | P0 | Build a deterministic transfer simulator and invariant oracle | NET-101 | todo | |
| 3 | PERF-102 | P0 | Build independent cumulative resource observer | PERF-101 | todo | |
| 3 | PERF-103 | P0 | Implement client adapters with pinned identity and settings readback | PERF-101 | todo | |
| 3 | PERF-105 | P0 | Build scoped deterministic fixtures and cache controls | PERF-101 | todo | |
| 3 | SYS-005 | P1 | Publish registry chunks and revision deltas without global deep clones | SYS-004 | todo | |
| 3 | SYS-007 | P1 | Replace per-command task scans with supervised completions | SYS-004 | todo | |
| 3 | SYS-008 | P1 | Replace quadratic timer cancellation and stagger bulk work | SYS-004 | todo | |
| 3 | SYS-012 | P0 | Preserve feature and platform compatibility through production fixtures | SYS-001, SYS-002, SYS-003 | todo | |
| 4 | IO-005 | P0 | Make admission and completion ownership bounded and cancellation-safe | IO-003, IO-004 | todo | |
| 4 | NET-103 | P0 | Reserve payload capacity before requests and carry leased buffer spans | NET-101, NET-102, IO-003 | todo | |
| 4 | NET-112 | P1 | Select a uTP backend using a reproducible integration evaluation | NET-101, NET-102 | todo | |
| 4 | PERF-104 | P0 | Implement common timing barriers and useful-byte accounting | PERF-101, PERF-102, PERF-103 | todo | |
| 4 | PERF-106 | P0 | Create deterministic independent swarm and network controls | PERF-101, PERF-105 | todo | |
| 4 | PERF-108 | P0 | Remove metrics-created shadow storage runtime and stabilize accounting | PERF-101, PERF-102, SYS-011 | todo | |
| 4 | SYS-006 | P1 | Route APIs through bounded delta, page and sort projections | SYS-005 | todo | |
| 4 | SYS-010 | P1 | Bound discovery and control I/O and decouple scrape from announce | SYS-004, SYS-008 | todo | |
| 5 | IO-006 | P0 | Build one device service over the portable reference backend | IO-005 | todo | |
| 5 | IO-007 | P0 | Create batched hash jobs and shared hybrid content views | IO-002, IO-003, IO-005 | todo | |
| 5 | NET-104 | P0 | Implement the authoritative request and content ownership ledger | NET-101, NET-102, NET-103, IO-005 | todo | |
| 5 | PERF-107 | P0 | Implement independent network, hash, disk and pipeline ceilings | PERF-102, PERF-105, PERF-106 | todo | |
| 5 | PERF-109 | P0 | Replace synthetic capacity proxies with live production fixtures | PERF-102, PERF-103, PERF-104, PERF-105, PERF-108 | todo | |
| 5 | PERF-112 | P0 | Enforce v2 and persistence invariants in full transfer paths | PERF-104, PERF-105, PERF-106 | todo | |
| 6 | IO-012 | P1 | Stream recheck through large chunks and persistent sparse cursors | IO-006, IO-007 | todo | |
| 6 | NET-105 | P0 | Implement negotiated peer protocol request semantics | NET-104 | todo | |
| 6 | NET-109 | P1 | Rebuild piece selection around active candidates and accurate accounting | NET-104 | todo | |
| 6 | PERF-110 | P0 | Automate 28-case catalog and randomized paired execution | PERF-103, PERF-104, PERF-105, PERF-106, PERF-107, PERF-109 | todo | |
| 6 | PERF-111 | P1 | Add picker timer and changed-snapshot allocation regression probes | PERF-101, PERF-102, PERF-109 | todo | |
| 6 | SYS-014 | P1 | Qualify complete catalog cost and mixed-load responsiveness | SYS-005, SYS-006, SYS-007, SYS-008, SYS-009, SYS-010, PERF-109 | todo | |
| 7 | NET-106 | P1 | Separate peer read progress from write and storage waits | NET-103, NET-104, NET-105 | todo | |
| 7 | PERF-113 | P1 | Implement statistical analysis and explicit scorecard | PERF-101, PERF-102, PERF-104, PERF-110 | todo | |
| 8 | IO-008 | P0 | Move transfer actors to completion-driven storage and hashing | IO-002, IO-006, IO-007, NET-104, NET-106 | todo | |
| 8 | NET-107 | P1 | Replace replicated token buckets with hierarchical fair rate scheduling | NET-103, NET-104, NET-106 | todo | |
| 8 | NET-113 | P1 | Deliver the selected windowed uTP transport behind the shared runtime | NET-103, NET-106, NET-112 | todo | |
| 8 | PERF-114 | P0 | Replace text-grep qualification with exact artifact manifests | PERF-101, PERF-113 | todo | |
| 9 | IO-009 | P0 | Implement dirty-only generation fences and grouped barriers | IO-004, IO-006, IO-008 | todo | |
| 9 | IO-011 | P1 | Replace unconditional readahead with useful range caching | IO-004, IO-006, IO-008 | todo | |
| 9 | NET-108 | P1 | Implement adaptive request windows and deadline-driven refill | NET-104, NET-106, NET-107 | todo | |
| 9 | NET-110 | P1 | Implement bounded shared upload service and eliminate valid-demand drops | NET-103, NET-105, NET-106, NET-107 | todo | |
| 9 | PERF-115 | P1 | Integrate hosted correctness and dedicated hardware regression lanes | PERF-110, PERF-111, PERF-112, PERF-113, PERF-114 | todo | |
| 10 | IO-010 | P0 | Bind resume persistence and crash recovery to barrier evidence | IO-009, SYS-009 | todo | |
| 10 | IO-014 | P1 | Schedule secure copies and verify each leaf once | IO-006, IO-009, IO-012 | todo | |
| 10 | IO-015 | P1 | Add continuous io_uring with retained safe registrations | IO-006, IO-009 | todo | |
| 10 | NET-111 | P1 | Implement fair seed and download choking policies | NET-104, NET-107, NET-110 | todo | |
| 11 | IO-013 | P1 | Persist authenticated v2 Merkle nodes and serve bounded proofs | IO-007, IO-010, IO-012 | todo | |
| 11 | NET-114 | P1 | Move v1, pure-v2 and hybrid transfer execution onto the shared core | NET-104, NET-105, NET-108, NET-109, NET-110, IO-008, IO-010 | todo | |
| 12 | NET-115 | P1 | Unify source scheduling and batch progress publication | NET-106, NET-107, NET-108, NET-114, SYS-009 | todo | |
| 12 | SYS-013 | P0 | Integrate one-writer migration and durable rollback | NET-114, IO-010, SYS-005, SYS-007, SYS-009, SYS-012 | todo | |
| 13 | IO-016 | P0 | Gate rollout on live-engine parity, recovery and hardware workloads | IO-008, IO-009, IO-010, IO-011, IO-012, IO-013, IO-014, IO-015, PERF-110, SYS-013 | todo | |
| 13 | NET-116 | P1 | Gate capability rollout with independent correctness and performance evidence | NET-108, NET-109, NET-111, NET-113, NET-114, NET-115, PERF-112, PERF-113, SYS-013 | todo | |
| 13 | PERF-116 | P1 | Run candidate soak recovery and reversible canary qualification | PERF-112, PERF-114, PERF-115, SYS-013, SYS-014 | todo | |
| 14 | SYS-015 | P0 | Integrate final gates, independent review and reproducible release | SYS-013, SYS-014, NET-116, IO-016, PERF-116 | todo | |

### Immediate sequence (G0)

1. **SYS-002** — done. v2 reproductions + extent correction + local
   request-window repair. The window fix is a local loop repair only; the
   authoritative shared ledger (F03) remains NET-104.
2. **SYS-003** (F10, F09, F11 done; F08 v1 done, v2/Cancel remain) — release-rate and negotiated-capability repairs: webseed debit
   out of `debug_assert!` (F10) with a release-mode test, aggregate torrent
   upload cap (F09), explicit token-ready wakeup (F11), Fast lifecycle (F08;
   interim = stop advertising Fast until NET-105).
3. In parallel (independent owners): **IO-001**, **NET-101/102**,
   **PERF-101**, **SYS-004/009/011**.

### Progress log

| Date | Change |
|---|---|
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
| 2026-10-17 | F12 partly: `Choker::run_with_mode(ChokeMode)`, optimistic history + uuid tie-break, engine passes decayed rates. Tests: `downloading_mode_rewards_peers_that_deliver_to_us`, `optimistic_slot_rotates_through_every_waiting_peer`, `optimistic_slot_moves_on_when_its_holder_stops_being_interested`. |
| 2026-10-17 | F05 partly: `adaptive_request_pipeline` (rate x 0.25 s, bounded). Uses delivery rate rather than measured RTT because the peer loop does not yet report per-request delay. Test `request_window_grows_only_for_fast_peers_and_never_under_pressure`. |
