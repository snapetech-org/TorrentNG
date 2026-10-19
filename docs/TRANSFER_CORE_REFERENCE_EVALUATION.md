# Transfer Core Reference and Reuse Evaluation

Status: **native rebuild retained as a reversible candidate; reuse is not
ruled out.** This is an architecture review, not a correctness or performance
result. The workspace source bulk pass has run; matched whole-client comparison
and qualification are still pending.

## Decision

Do not restart the rebuild or make `librqbit`/libtorrent the production engine
based on source review alone. Continue the code and documentation work on the
current local branch, keep the current production actors selectable through
the integration and rollback stages, and evaluate maintained clients against
the native candidate with the same fixtures, settings, resource limits, and
measurement contract.

### Direct recommendation

No: we cannot be 100% sure that TorrentNG should keep its own transfer engine.
The best current call is to keep this implementation as a reversible
candidate long enough to compare it fairly, while borrowing concrete design
lessons from maintained clients. That is not a commitment to finish a rewrite
at any cost. If a maintained implementation wins on the matched workload and
can preserve TorrentNG's storage and persistence contracts with less total
complexity, replace the smallest failing boundary; restart the engine only if
the evidence supports a whole-engine change.

The current maintainers suggest different roles for the comparison. `rqbit`
9.0.1 is the closest Rust baseline for v1; its published feature list still
describes sequential selection as its only picker mode and does not list BEP
52. `libtorrent` 2.1.2 documents v2/hybrid transfer and keeps pieces grouped
by rarity with randomized ordering, making it the stronger full-protocol
behavior reference, with C++ build and adapter cost included in any comparison.
The `urtorrent` upstream `main` currently identifies itself as 0.14.3, requires
Rust 1.98, pins toolchain 1.98.1, and targets Linux with io_uring and no
portable I/O fallback. Our direct runner pins 0.14.3 and deliberately supports
the v1 fixture profile only. These are suitability observations, not
performance results. The paired harness source is integrated, but exact client
profiles, compatibility preflight, and real paired runs are still missing, so
there is no measured winner.

This is a provisional decision. It does **not** assume that TorrentNG's new
core is faster, better, or the final architecture. Keep subsystem boundaries
replaceable. If the native candidate fails the correctness, portability,
ownership, or matched-efficiency gates, prefer reusing a suitable maintained
implementation for that subsystem or as an alternate engine over continuing a
rewrite for its own sake. Reconsider a whole-engine replacement only after the
same comparison also shows that it can preserve TorrentNG's storage safety,
supervised persistence, API/migration behavior, Rust 1.88 floor, and rollback
requirements.

## What was reviewed

### `librqbit`

The `rqbit` project exposes `librqbit` as its main library and has an active
Rust implementation with an in-process `Session` API. Release 9.0.1's library
manifest declares Rust edition 2024 and Apache-2.0. It does not declare a
`rust-version`; edition compatibility alone does not prove that its complete
dependency graph builds at TorrentNG's Rust 1.88 floor. Its v9 release notes
also describe recently added uTP and IPv6 support, among other changes.

This makes `librqbit` a serious comparator and a possible alternate-engine
candidate, rather than an implementation to dismiss. Its public `AddTorrent`
options include a `storage_factory` hook, so adapting its storage boundary is
at least worth a targeted feasibility review. That hook does not by itself
prove that TorrentNG's confined paths, per-location crash policy, device
admission, resume evidence, hybrid semantics, and `DbWorker` ownership can be
preserved without parallel state or duplicated work. The dependency's exact
Rust 1.88 buildability, storage adapter contract, torrent feature parity, and
whole-client efficiency remain unverified here.

There are also current feature gaps to account for: rqbit's supported-BEP list
does not include BEP 52, its v9 release notes describe SHA-256 foundations
rather than completed v2 transfer, and its README describes sequential
download as the only picker mode. The upstream BEP 52 issue is still a design
proposal. Treat rqbit as a strong v1 efficiency and integration comparator,
not as a drop-in replacement for TorrentNG's v1/v2/hybrid capability target.

### libtorrent-rasterbar

libtorrent is a mature C++17 BitTorrent library under BSD-3-Clause. Its manual
documents a session-level event model, separate checking/downloading/seeding
queues, and a contiguous piece-priority representation intended to improve
locality and avoid per-piece tree allocation. These are useful design and
behavior references. Embedding it would add a C++ toolchain and FFI boundary,
and would still require an adapter around TorrentNG's persistence, secure
storage, API, and rollback contracts. No matched evidence currently shows that
adopting it improves TorrentNG's end-to-end result.

### uTorrent / BitTorrent Classic and `urtorrent`

Keep three names separate. µTorrent/BitTorrent Classic is a black-box
interoperability and behavior reference here. `urtorrent` is a separate,
actively developed Rust BitTorrent library, and `urtorrent-utp` is one of its
internal protocol crates.

The reviewed `urtorrent` release is `0.14.3` (Apache-2.0, Rust 1.98). Its
maintainer requires Linux 6.1+ and io_uring for every data-path operation,
with no epoll or thread-pool fallback, and excludes BEP 52 v2/hybrid from its
declared scope. It is a useful Linux v1 performance comparator and a poor
production replacement for TorrentNG's portable runtime and v1/v2/hybrid
scope. Its protocol state machines are sans-IO; the project pins qBittorrent
as a wire-behavior oracle, replays captured sessions, uses isolated
integration fixtures, and makes in-flight buffer ownership explicit. Those
are concrete techniques to adopt in TorrentNG's own boundaries and
qualification without adopting its runtime or raising TorrentNG's Rust 1.88
floor. The crate's published copy counters and documented copy budget also
make a useful benchmark lesson: count data movement explicitly, then compare
it under matched workloads instead of inferring efficiency from architecture.

Do not copy qBittorrent source while learning from its wire behavior. The
`urtorrent` project records qBittorrent's GPL boundary and uses libtorrent's
BSD-3-Clause source only with retained notices. Any code reuse from a
maintained project still needs file-level license and provenance review
before it enters TorrentNG.

For uTP specifically, reviewed `urtorrent-utp` releases require Rust 1.98.
Its window, SACK, reorder, timeout, MTU, and shared-port behavior remain
design references; see
[`TRANSFER_CORE_UTP_EVALUATION.md`](TRANSFER_CORE_UTP_EVALUATION.md). The
existing `rt-utp` candidate remains replaceable and must earn rollout with
correctness and matched whole-client efficiency evidence.

## Design lessons worth benchmarking

- `urtorrent` keeps protocol state machines sans-IO and drives them with
  recorded qBittorrent oracle captures and isolated local peers. TorrentNG's
  pure transfer modules and seeded simulator follow the same useful boundary;
  the simulator still needs to pass its bulk correctness gate before we trust
  it.
- Both libtorrent and `urtorrent` use locality-aware picker ideas. Libtorrent
  documents one contiguous vector shuffled within rarity classes and lazy
  rebuilding after expensive peer inventory changes. `urtorrent` adds an
  optional 4 MiB piece-extent affinity for small-piece torrents to reduce
  scattered writes. TorrentNG's current picker instead updates compact
  per-rarity arrays incrementally. Keep all three as measurable designs:
  compare picker CPU/allocation cost, partial-piece count, physical I/O, and
  verified goodput before changing the default or replacing NET-109.
- `urtorrent`'s completion-owned buffers and explicit qdisc/runtime limits are
  useful ownership examples. TorrentNG already has cancellation-retained
  storage and transfer leases; the bulk pass must establish that the full
  actor paths preserve those charges across late completion.
- `urtorrent` records payload copies and hash readback bytes as runtime
  counters and documents expected copy counts by transport. TorrentNG now
  exposes identity-bound counters for pure-v2 assembly, peer file-segment
  copying, and webseed range reassembly. These are intentionally scoped to
  those observed sites; the trial contract must not present them as a complete
  copy total or compare them with another client's broader counter. Measure
  their relationship to CPU and verified goodput once paired execution is
  available.

These are hypotheses and implementation lessons, not claimed TorrentNG wins.
PERF-103 now includes a direct `urtorrent` 0.14.3 library runner as a
separately identified Linux 6.1+ v1 comparator. It records effective session
and torrent settings against its live process; hash threads are labeled as a
configured launch input because the public settings API does not report them.
It binds to a task-local IPv4 interface, accepts a private fixture, removes
tracker and web-seed endpoints, disables IPv6 and discovery, and dials one
explicitly pinned private, link-local, or loopback IPv4 peer from a task-owned
network namespace. The adapter independently checks the addresses in the live
snapshot. Its control loop is part of the measured process. The runner and
adapter are authored but unbuilt and unrun. The paired harness, workload
qualification, and broader client comparison remain incomplete, so this adds
decision evidence without endorsing either architecture.

## Why keep the current branch as a candidate

- The new pure modules and storage work are already authored but unvalidated;
  restarting would discard that work without comparative evidence.
- A major part of TorrentNG's differentiator is integration with its existing
  secure storage and persistence owners. Replacing only the transfer engine
  may move complexity into adapters and duplicate state instead of removing
  it.
- `librqbit` is close enough in language and product scope to compare directly,
  and the PERF-103 adapter already includes it. The direct `urtorrent` runner
  adds a second Rust implementation with a different runtime and feature
  envelope. The paired harness can make this decision with evidence after
  implementation is complete.
- libtorrent remains a practical protocol and scheduling reference, while
  qBittorrent and other maintained clients provide whole-client baselines.
- Keeping the production fallback selectable makes a later replacement
  reversible and prevents an unqualified rewrite from becoming the only path.

These are reasons to defer the architecture choice until evidence exists, not
reasons to assume the native candidate will win.

## Reuse and replacement gate

Before adopting a maintained engine or subsystem, compare exact pinned builds
and settings on the same verified torrent payloads and peer/network workload.
Record completion and hash integrity, useful goodput, CPU per verified GiB,
process and cgroup memory, payload-copy and I/O amplification, socket/fd count,
queue latency, and recovery behavior. Keep unavailable measurements `null`.
Use the existing PERF-101 through PERF-116 contract and do not change the
acceptance thresholds after seeing results.

An alternate implementation is eligible only if it also preserves or
explicitly adapts:

1. TorrentNG's one-writer rule and bounded ownership under cancellation and
   late completion.
2. Confined/no-follow storage operations and per-location durability policy.
3. `DbWorker` authority and resume commits tied to storage barrier evidence.
4. v1, v2, hybrid, migration, identity, and API compatibility requirements.
5. The Rust 1.88 product floor and portable positioned-I/O fallback, unless
   an explicit product decision changes either constraint.
6. A staged rollout with a selectable reference path and durable rollback.

Start with the subsystem that misses a measured gate; do not replace the full
engine if a smaller boundary solves the problem. A full restart remains an
option only if repeated evidence shows the current architecture cannot meet
the gates and the alternate passes them with lower total ownership and
integration cost.

## Primary references

- [`urtorrent` upstream workspace version and MSRV](https://raw.githubusercontent.com/giejqf/urtorrent/main/Cargo.toml)
- [`urtorrent` pinned toolchain](https://raw.githubusercontent.com/giejqf/urtorrent/main/rust-toolchain.toml)
- [`urtorrent` current project scope](https://github.com/giejqf/urtorrent)
- [`rqbit` current supported BEPs and picker modes](https://raw.githubusercontent.com/ikatson/rqbit/main/README.md)
- [`rqbit` 9.0.1 library manifest](https://github.com/ikatson/rqbit/blob/v9.0.1/crates/librqbit/Cargo.toml)
- [`libtorrent` 2.1.2 release](https://github.com/arvidn/libtorrent/releases/tag/libtorrent-2_1_2)
- [`libtorrent` 2.1 manual](https://github.com/arvidn/libtorrent/blob/RC_2_1/docs/manual.rst)
- [rqbit repository](https://github.com/ikatson/rqbit)
- [rqbit upstream feature list](https://github.com/ikatson/rqbit#features)
- [`librqbit` 9.0.1 manifest](https://github.com/ikatson/rqbit/blob/v9.0.1/crates/librqbit/Cargo.toml)
- [`librqbit` 9.0.1 session API](https://github.com/ikatson/rqbit/blob/v9.0.1/crates/librqbit/src/session.rs)
- [rqbit v9.0.1 release notes](https://github.com/ikatson/rqbit/releases/tag/v9.0.1)
- [rqbit supported features](https://github.com/ikatson/rqbit#features)
- [rqbit BEP 52 implementation design issue](https://github.com/ikatson/rqbit/issues/546)
- [`urtorrent` 0.14.3 project charter and engineering rules](https://github.com/giejqf/urtorrent/blob/v0.14.3/AGENTS.md)
- [`urtorrent` 0.14.3 workspace manifest](https://github.com/giejqf/urtorrent/blob/v0.14.3/Cargo.toml)
- [`urtorrent` 0.14.3 pinned toolchain](https://github.com/giejqf/urtorrent/blob/v0.14.3/rust-toolchain.toml)
- [`urtorrent` 0.14.3 public API and scope](https://docs.rs/urtorrent/0.14.3/urtorrent/)
- [`urtorrent` picker extent-affinity implementation](https://docs.rs/urtorrent-picker/0.14.3/urtorrent_picker/)
- [`urtorrent` 0.14.3 copy counters](https://docs.rs/urtorrent/0.14.3/urtorrent/struct.SessionStats.html)
- [`urtorrent` 0.14.3 performance and copy budget](https://github.com/giejqf/urtorrent/blob/v0.14.3/docs/perf.md)
- [libtorrent repository and license](https://github.com/arvidn/libtorrent)
- [libtorrent 2.1 manual](https://github.com/arvidn/libtorrent/blob/RC_2_1/docs/manual.rst)
- [libtorrent BitTorrent v2 and hybrid support](https://libtorrent.org/manual-ref.html)
- [TorrentNG uTP evaluation](TRANSFER_CORE_UTP_EVALUATION.md)
- [TorrentNG transfer rebuild harness](../scripts/transfer_rebuild/README.md)

## Evidence state

The TorrentNG workspace and harness regression sources passed the bulk run
recorded in `TRANSFER_CORE_ACTIVE_SPECS.md`; that pass does not exercise the
direct `urtorrent` runner as a live peer. The runner pins crate version 0.14.3
and requires Rust 1.98.1, a private v1 fixture, a task-local IPv4 listener and
peer, and an isolated namespace on a compatible Linux 6.1+ host. Its build/run
status is tracked separately from TorrentNG workspace tests. No matched
performance result or architecture win has been established.
PERF-103/104/110/113 remain the path to a reproducible comparison; this
document does not mark any ticket done or any gate passed.
