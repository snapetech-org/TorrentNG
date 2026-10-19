# Active Transfer Rebuild Ticket Specs

This file instantiates the template in
[`TRANSFER_CORE_TICKET_TEMPLATE.md`](TRANSFER_CORE_TICKET_TEMPLATE.md) for the
implementation slices currently in progress. A ticket stays `wip` until its
written acceptance and exact-revision evidence are complete.

## Fresh full validation — 2026-10-19

The tested production source is commit
`354ad504f75b33c712b5ccd827cfef3eb0cf4414`. At the start of this run, `main`
was at `14308ca143e6fe46957267ed88ed54080a5bcbf2`; all commits since the
tested source change documentation only, and the later working changes
recorded here are also documentation only. The full workspace passed on Rust
1.94.0 and 1.88.0, with
2,301 passed, 0 failed, and 12 ignored across 37 summaries on each toolchain.
The release `rt-engine` library passed 733 tests. All 21 transfer-rebuild
Python scripts passed. Workspace and engine Clippy exited 0 with warnings;
workspace formatting and runner formatting on Rust 1.94.0 passed. The
standalone comparator runner built under Rust 1.98.1, passed all six tests,
and passed its `--help` smoke. Rust 1.98.1 `rustfmt` was not installed, so the
runner formatting check used Rust 1.94.0. The ignored panic-hook child is
covered through its parent regression.

The workspace commands were:

- `cargo +1.94.0 test --workspace --all-targets --locked --no-fail-fast -- --test-threads=1 --format=terse`, and the same command with `+1.88.0`.
- `cargo +1.94.0 test --locked --release -p rt-engine --lib --jobs 1 -- --test-threads=1`.
- All 21 `python3 scripts/test_transfer_rebuild_*.py` scripts.
- `cargo +1.94.0 clippy --workspace --all-targets --locked --jobs 1` and `cargo +1.94.0 clippy --locked -p rt-engine --all-targets --jobs 1`.
- Workspace and standalone-runner `cargo fmt -- --check` on Rust 1.94.0.
- Rust 1.98.1 release build/test/help smoke for `scripts/transfer_rebuild/urtorrent_runner`.

All 11 runnable ignored test cases passed explicitly: two 100k catalog tests,
seven release storage tests, the 64-file/64 MiB move-import-delete test, and
the peer-wire encoder probe. The picker and API snapshot Criterion-style
benchmarks also completed. The 100k catalog restored in 2.026 s after 219.170
s admission; first sorted list was 735 ms and cached listing was below 1 ms.
Across 16 mixed-load rounds, list p95 was 757.512 ms and 782.177 ms, transfer
info p95 was 4.32 ms, and `/metrics` p95 was 2.21 ms. The cold first snapshot
drives the list p95; these are diagnostics without approved thresholds.

The storage suite passed 7/7. Its temp paths were on `/tmp` tmpfs and the
harness reported `profile=Unknown`, `fs=None`, and no device identity; the
HDD-only branch skipped as expected. The 256 MiB positioned-I/O probe reported
4,844 MiB/s writes and 6,872 MiB/s reads; readahead served 4,096 reads with 133
backend reads (30.8x fewer); the 4,096-piece recheck reported 126.38 MiB/s.
The 64 MiB move/import/delete probe passed. These results are functional
diagnostics, not physical-device or dedicated-ext4 qualification.

At 100,000 pieces, picker pick/cancel measured 1,161 ns per operation and
reported 5.80 MB of picker storage. The API snapshot probe measured a 502 ms
cold initial snapshot, 86 ns per cached read, and 151–313 µs for a one-row
change. The peer-wire microbenchmark measured 2.131 ms for owned encoding and
579 µs for direct reused encoding over 10,000 frames. These isolated probes
have no accepted threshold and do not establish whole-client efficiency.
Detailed logs are under
`/tmp/tng-transfer-core-final-20261019.GSdus5/`.

The `urtorrent` runner preflight was attempted on the current build (and on an
earlier build of the same runner source) and failed to start the pinned client
because `io_uring_setup` returned `Cannot allocate memory`. Runner build, unit
tests, and CLI smoke pass, but no paired client trial is qualified. The hosted
correctness and Windows Smoke runs
[39233029958](https://github.com/snapetech-org/TorrentNG/actions/runs/39233029958)
and
[39233812206](https://github.com/snapetech-org/TorrentNG/actions/runs/39233812206)
passed on exact commit `94c8c1b3ff14106db93ab1a9750a1bb00c808b7c`. The
protected hardware lane was skipped; its dedicated ext4 LVM evidence below is
from `eb5f3158`, and no protected-runner result has been collected for the
current source. Since `eb5f3158`, the engine/storage production paths are
unchanged; later commits adjust two test timeouts and add the standalone
comparator runner and its workflow.

The source audit found all 63 repository-owned implementation slices and
ticket contracts present with no remaining architecture boundary. The ledger
remains 12/63 `done`, 51/63 `wip`, 0 `todo`: the remaining status is for live
client, platform, crash-cut, matched efficiency, protected-runner, soak/canary,
independent-review, and release evidence. Ordinary “await bulk” notes below
are superseded for repository-local tests by the fresh run above. Detailed
logs are under `/tmp/tng-transfer-core-final-20261019.GSdus5/`.

The server storage target was not rerun: read-only preflight found `kspls0`
load averages of 147.29, 171.55, and 171.72. Earlier ext4 evidence remains
bound to its recorded source revision. The existing `urtorrent` preflight also
remains unable to start because `io_uring_setup` returns `Cannot allocate
memory`; no paired client trial is qualified. Hosted Transfer Rebuild run
[39233029958](https://github.com/snapetech-org/TorrentNG/actions/runs/39233029958)
and Windows Smoke run
[39233812206](https://github.com/snapetech-org/TorrentNG/actions/runs/39233812206)
passed on `94c8c1b3ff14106db93ab1a9750a1bb00c808b7c`; the protected hardware
lane was skipped because no runner is registered.

## Previous exact-source closeout — 2026-10-19 (`eb5f3158`)

The tested `main` source is commit `eb5f31588beb4cf9ec0fb4c3509745c49c16f500`,
recorded from a clean worktree. The source audit found every repository-owned implementation slice present and no
unowned architecture boundary. Eight tickets whose written acceptance was
limited to code and local evidence are now `done`; the ledger is 12/63 done,
51/63 wip, and 0 todo. The remaining `wip` tickets still have specific live,
hardware, platform, paired-client, recovery, soak, release, or independent
review requirements.

The full Rust workspace passed on both Rust 1.94.0 and 1.88.0: 2,301 passed,
0 failed, 12 ignored across 37 test-result summaries per toolchain. The
release-mode `rt-engine` library suite passed 733/733, including the transfer
simulator tests. All 21 `scripts/test_transfer_rebuild_*.py` regressions
passed. Formatting passed; Clippy exited 0 with warnings. Hosted Transfer Rebuild run
[39217933388](https://github.com/snapetech-org/TorrentNG/actions/runs/39217933388)
and Windows Smoke run
[39216147832](https://github.com/snapetech-org/TorrentNG/actions/runs/39216147832)
passed on this exact source. The hosted hardware job was not selected for this dispatch; no runner is registered
for its protected label; `hardware_lane.sh`
was run twice manually on `kspls0` against the confirmed `ext4` LVM mount, and
both runs passed picker, API snapshot, and storage move/import checks. The
release-mode `rt-storage::storage_real_device` suite also passed all 7 tests
on this source and the same ext4 LVM target. Its sequential readahead case
served 4,096 submitted reads from 133 backend reads (30.8x), over 64 MiB; this
is workload-specific storage evidence, not a whole-client result. The release
engine log is retained as
`rt-engine-release-eb5f3158.log` under the same task evidence directory
(SHA-256 `62fdcdb3d59fc8d3fcc41930201fefb750e56f1c2190b7247b8aa52be4dc8f06`).

SYS-014 is source-bound to these validated artifacts:

- Report SHA-256: `2daedbf535bbad767430651496b577d01a1c4549bf5904f66e6eb47c6dfdaa19`
- Raw log SHA-256: `07b34b70e44cd315e96a481d1a79b99f5d3dd2aca565ad584d98bffb2a10e245`
- Manifest SHA-256: `4c66cedf13a3ac856b1c633b3fbfa4fcd601148ead839e04a98218ce220ee309`
- Qualification SHA-256: `6797ed80bba22332e7154e88262910d597e23db781613b6d6117817e15bd761a`

For the 100k restored-paused catalog, list p50 was 0.325/0.337 ms and p95
0.838/0.860 s for default/name-sorted pages; the first cold snapshot drives
those p95 values. Transfer-info p50/p95 was 0.098/3.350 ms and `/metrics`
0.092/0.323 ms. The 1k list p95 was 7.44/7.41 ms. These are in-process
diagnostics, not thresholds or whole-client results. Two exact-source hardware
runs measured 100k pick/cancel at 14.95 and 14.84 µs/op, cached 100k snapshot
reads at 123 and 133 ns/sample, and each initial 100k snapshot at about 1.8 s.
All logs remain under `/tmp/tng-transfer-core-close-20261018/evidence/`.

Matched client trials, active-transfer/process resource measurements, live
v1/v2/hybrid parity, target-filesystem crash-cut recovery, the protected
hardware workflow artifact, release-matrix builds, soak/canary, independent
review, and the final release bundle remain open. No efficiency or
superiority claim is supported by these local probes.

## Prior completion audit — 2026-10-19 (`eb5f3158`)

The 63-row package table and all active ticket contracts have now been checked
against the current source and regression inventory. The implementation slices
are present across the pure transfer core, v1/v2/hybrid actors, storage,
supervision, rollback routing, and qualification tooling; this audit found no
missing owner boundary or placeholder implementation in the reviewed paths.
`wip` therefore remains an acceptance/evidence status, not a count of code that
has not been written.

At that point, source `eb5f3158` passed the following bulk validation:

- `cargo +1.94.0 test --workspace --all-targets --locked --no-fail-fast -- --test-threads=1 --format=terse`
  and the same command on Rust 1.88.0: 2,301 passed, 0 failed, 12 ignored
  across 37 test-result summaries per toolchain.
- All 21 `scripts/test_transfer_rebuild_*.py` regression scripts passed.
- Both 100k production-catalog cases passed. The release mixed-load probe used
  four Tokio workers and retained all 16 samples for each route; results and
  cold-snapshot limits are recorded in SYS-014.
- `cargo +1.94.0 fmt --all -- --check` passed. Clippy exited 0 with warnings.
- The two exact-source `hardware_lane.sh` runs passed on `kspls0`; the
  release-mode `storage_real_device` suite also passed all 7 tests on the
  verified ext4 LVM target. Sequential readahead served 4,096 submitted reads
  from 133 backend reads (30.8x) over 64 MiB. This is workload-specific
  evidence, not a whole-client efficiency measurement.
- Hosted Transfer Rebuild run
  [39217933388](https://github.com/snapetech-org/TorrentNG/actions/runs/39217933388)
  and Windows Smoke
  [39216147832](https://github.com/snapetech-org/TorrentNG/actions/runs/39216147832)
  passed on exact main `eb5f3158`. The hosted hardware job was not selected by
  the dispatch; no runner is registered for its protected label.

The first full pass exposed an invalid pure-v2 projection after a previously
completed torrent entered a durability hold: the entry could remain `Seeding`
with the one-byte incomplete marker. Both v2 persistence paths now downgrade
that stale state to `Downloading` and clear `completed_at`; the new
`v2_reopens_a_stale_seeding_projection_during_completion_hold` regression
passes in both final workspace runs.

The prior test logs are retained under the task-scoped
`/tmp/tng-transfer-core-close-20261018/evidence/` directory. Ordinary Rust and
Python regressions described as “await bulk pass” or “authored but unrun” in
older per-ticket notes are superseded by the current exact-source follow-up at
the top of this file.
Ticket acceptance remains `wip` where it requires runtime, platform,
filesystem fault, crash-cut, live parity, matched whole-client, soak/canary,
protected-runner artifact, or independent-review evidence.
Ticket-specific acceptance that needs hosted workflows, dedicated hardware,
filesystem fault qualification, crash cuts, live parity, or matched whole-
client trials remains open.

## NET-101 — Typed transfer coordinates and geometry adapters

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** `rt-engine/src/transfer/coords.rs` and the live
  protocol adapters.
- **Finding or capability:** one ownership core must validate requests across
  v1 concatenated-file geometry, v2 per-file geometry, and hybrid wire views.
- **Status:** `wip`

### Scope and invariant

Define distinct `BlockExtent`, `PieceExtent`, `ContentRange`, `Plane`,
generation-bearing `ConnectionId`, `TorrentKey`, and `TransferPeerId` types.
`PieceGeometry` adapts `PieceMap` and `V2PieceMap`; `HybridGeometry` retains
both prebuilt maps and can select the download view. Hybrid projection maps
wire extents to canonical content ranges, synthesizes or removes BEP 47
padding as required, and compares content ranges instead of equating numeric
piece indexes.

- **Invariant preserved:** a block is validated against its own protocol
  geometry and every hybrid alias resolves to the same payload bytes.
- **Must not:** treat a block extent as a whole-piece extent, infer content
  identity from matching piece numbers, or merge v1 and v2 rarity counts.

### Acceptance

- Reject zero-length, overflowing, out-of-piece, and noncanonical request
  extents; allow valid served tail extents that need not have request alignment.
- Map file-spanning v1 ranges, file-local v2 ranges, hybrid padding, and short
  final blocks without allocating duplicate piece maps.
- Compare hybrid blocks by normalized payload ranges and preserve generation
  identity when a connection slot is reused.
- Regression sources cover block/piece type separation, short tails, padding
  shifts, equal indexes with unequal content, selected hybrid plane, and shared
  map ownership.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/coords.rs`,
  `transfer/hybrid.rs`, `transfer/live.rs`, and `rt-piece-map` adapters.
- **State or wire compatibility:** no resume or wire-format change; coordinates
  are internal typed wrappers.
- **Migration / rollback:** callers can retain their current picker geometry
  while adopting adapters incrementally.
- **Feature flag or rollout boundary:** live ownership parity is NET-114.

### Evidence

- **Regression source added or updated:** geometry and hybrid adapter tests in
  `coords.rs`, `hybrid.rs`, and `live.rs`; simulator traces cover mixed planes.
- **Command or probe to run:** transfer-core simulator and engine suites in the
  requested bulk pass.
- **Expected result:** only equivalent payload ranges alias across hybrid
  planes, and stale connection generations cannot own new work.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** matched v1/v2/hybrid live parity.

## NET-102 — Seeded transfer simulator and invariant oracle

- **Priority / wave:** P0 / 3
- **Prerequisites:** NET-101
- **Owning component:** `rt-engine/src/transfer/sim.rs`.
- **Finding or capability:** ownership races and protocol transitions need a
  deterministic virtual-time oracle independent of sockets and storage.
- **Status:** `wip`

### Scope and invariant

Run a seeded swarm trace against the transfer ledger and hybrid adapter.
`SimConfig` controls the seed and event behavior; `SimReport` records terminal
completion, conservation, and invariant outcomes. Separate seeded traces
exercise reserve/commit/response order, loss, peer churn, endgame races,
late data, hybrid piece affinity, padding projection, and generation reuse.

- **Invariant preserved:** simulation time and randomness are reproducible;
  accepted content is accounted exactly once and every terminal path releases
  ownership.
- **Must not:** substitute simulation for the production actor, filesystem,
  or interoperability qualification.

### Acceptance

- Identical configuration and seed produce identical reports.
- Every trace checks ledger invariants and byte/request conservation at each
  transition and at termination.
- Seeded hybrid traces ensure one canonical piece is not split across planes,
  losing responses project cancellation to the losing peer's wire plane, and
  stale generations remain fenced after reconnect.
- Run bounded matrices for one-peer, high-loss/churn, and mixed hybrid cases.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/sim.rs` and the pure
  ownership core.
- **State or wire compatibility:** simulator-only; no production state or
  protocol change.
- **Migration / rollback:** not applicable.
- **Feature flag or rollout boundary:** oracle results support NET-114 and
  NET-116 but do not authorize rollout alone.

### Evidence

- **Regression source added or updated:** deterministic replay, loss/churn,
  commit/response race, degenerate swarm, and 64-seed hybrid trace tests.
- **Command or probe to run:** simulator module tests during the requested
  bulk pass.
- **Expected result:** fixed seeds preserve all invariants and complete where
  the configured source availability permits completion.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** production actor and live-protocol
  qualification.

## PERF-105 — Scoped deterministic fixture generation

- **Priority / wave:** P0 / 3
- **Prerequisites:** PERF-101
- **Owning component:** certification tooling.
- **Finding or capability:** repeatable fixture identity without writing
  generated payloads into the measured source checkout.
- **Status:** `wip`

### Scope

Generate deterministic per-file payloads beneath a fresh caller-selected
directory and record a manifest with size, SHA-256, generator, seed, and a
fixture identity digest. With a caller-supplied tracker URL, generate private
v1, v2, or hybrid metainfo and a separate identity record. Pinned workload
profiles define payload shape and torrent geometry. A Linux helper requests
cold or warm page-cache state for the fixture files and records measured
residency; it does not modify global cache settings. Callers own fixture
retention and may remove only their task-scoped output after preserving the
required result artifacts.

### Ownership and invariant

- **Current owner:** the trial runner chooses and owns the output directory;
  the generator writes only into a private staging directory beside it.
- **Target owner:** PERF-105 supplies fixture identities to paired trial
  execution and adapters.
- **Invariant preserved:** fixture identity is content-derived and each
  payload digest is independently recorded.
- **Must not:** merge into or clear an existing output directory, or call a
  fixture cold/warm based only on its generation state.

### Acceptance

- Same seed, file labels, sizes, and generator version reproduce the same
  payload bytes and manifest identity; verifier checks the manifest and every
  file digest before use.
- Names are flat filename-safe labels and sizes are nonnegative.
- Output must be new; generation is staged and task-scoped.
- Optional private v1/v2/hybrid metainfo derives every hash from the payload
  and records all applicable info hashes and the metainfo digest; verification
  rebuilds those values.
- V2 fixtures use BEP 52 16 KiB SHA-256 leaves, zero-hash padding, a mandatory
  piece-layers dictionary, and authenticated piece layers for files larger
  than the piece length. Hybrid fixtures preserve the same sorted file order
  in both views and use BEP 47 zero-filled padding files for piece alignment.
- Named profiles pin the file list, protocol kind, torrent name, and piece
  length into the fixture identity; profile options cannot be overridden.
- Cache preparation is scoped to fixture payload files and reports both the
  requested state and mincore-observed residency. A partial/unsupported state
  cannot be reported as an achieved cold/warm trial.
- Regression sources cover deterministic output, digest identity, and refusal
  to reuse an existing path, deterministic and tamper-checked v1/v2/hybrid
  metainfo, profile identity, and cache-state record shape.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/fixtures.py`,
  `fixture_profiles.json`, `cache_control.py`, the cache-state schema,
  artifact validator, trial schema, regression sources, and harness README.
- **State or wire compatibility:** certification fixture format only; payload
  identity remains `transfer-fixture-v1`, and torrent records use a versioned
  v2 envelope while the verifier retains support for v1-only records.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** a fixture is comparable only when its
  profile, generated metainfo, client settings, and cache-state observation
  are linked to the trial artifact.

### Evidence

- **Regression source added or updated:** `scripts/test_transfer_rebuild_fixtures.py`.
- **Validator regression source:** `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** `python3 scripts/test_transfer_rebuild_fixtures.py`
  during the requested bulk pass.
- **Expected result:** deterministic file bytes, correct v1/v2/hybrid
  metainfo, matching profile identity, and accurate cache-state outcome; no
  existing path or global cache setting is modified.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** review of the pinned workload sizes,
  and cache-control qualification on each dedicated benchmark filesystem.

## PERF-104 — Common trial timing and useful-byte accounting

- **Priority / wave:** P0 / 4
- **Prerequisites:** PERF-101, PERF-102, PERF-103
- **Owning component:** certification tooling.
- **Finding or capability:** matched paired starts and an explicit useful
  payload denominator for goodput.
- **Status:** `wip`

### Scope

Provide shared measurement primitives for a matched start barrier, unique
accepted payload range accounting, and separate monotonic barrier, transfer,
flush, validation, and shutdown intervals. Extend the trial artifact with
optional, observation-linked payload-copy deltas for clients that expose
instrumented counters. The paired executor now connects the adapters, fixture,
resource observer, trial records, and manifest; its real-client execution and
qualification remain open.

### Ownership and invariant

- **Current owner:** `paired_runner.py` owns the barrier lifecycle, measured
  interval, fixture, client counters, and trial assembly; the independent
  observer remains a separate process and `measurement.py` owns reusable
  interval/range primitives.
- **Target owner:** PERF-104 harness consumes pinned PERF-103 adapters and
  PERF-105 fixtures without controlling the client's resource observer.
- **Invariant preserved:** goodput uses unique verifier-accepted payload bytes
  divided by transfer time; padding, duplicate overlaps, flush, validation, and
  shutdown are accounted separately.
- **Must not:** infer a start time from process launch or count bytes merely
  offered by peers.

### Acceptance

- A fresh task-owned file barrier requires every declared participant and has
  a bounded timeout. Readiness and release markers are created exclusively as
  regular files; symlink and non-file markers are rejected without following
  them.
- Range accounting is per file, merges overlapping ranges, excludes padding,
  and reports new and duplicate bytes.
- When available, copy accounting links start/end counter observations from
  the same PID and process start time, and validates each reported delta.
  Coverage is explicit and site-scoped; missing counters are not zero and
  partial native counters are not presented as whole-client totals.
- Timeline markers use a monotonic clock, require all contract phases, and
  reject zero transfer duration and overlapping/out-of-order phases.
- Regressions cover participant release, symlinked readiness/release markers,
  unique/duplicate ranges, padding, and phase boundaries.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/measurement.py`, its
  regression source, `adapters.py`, the payload-copy observation and trial
  schemas, the artifact validator, and the measurement contract.
- **State or wire compatibility:** certification artifacts only; the existing
  `transfer-trial-v1` schema now requires an observation-linked client
  identity. No trial evidence has been qualified against the earlier draft.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** no scorecard or release comparison
  until the runner and adapters are qualified; partial copy counters are
  descriptive until another client reports matching coverage.

### Evidence

- **Regression source added or updated:** `scripts/test_transfer_rebuild_measurement.py`.
- **Command or probe to run:** `python3 scripts/test_transfer_rebuild_measurement.py`
  during the requested bulk pass.
- **Expected result:** a matched release, exact interval-union accounting, and
  separate ordered phase durations; marker symlinks cannot satisfy readiness
  or release.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** cross-process barrier characterization
  and paired-run reproducibility. The payload-copy snapshot and delta tooling
  passed local regressions; live counter captures under measured client runs
  remain open, and native-only coverage is limited to the three sites named in
  the measurement contract.

## NET-108 — Adaptive peer request windows

- **Priority / wave:** P1 / 9
- **Prerequisites:** NET-104, NET-106, NET-107
- **Owning component:** v1, hybrid-v1, and pure-v2 peer actors.
- **Finding or capability:** size request windows by measured delivery and
  available useful work while preserving bounded resource admission.
- **Status:** `wip`

### Scope

Both live actor paths use the shared adaptive controller. Its window is now
bounded by the number of fresh blocks this peer can supply, in addition to
rate credit, response buffers, device backlog, and negotiated `reqq`. V1 and
hybrid-v1 count picker-owned fresh blocks through packed peer availability;
the bitmap walk stops once the local 128-block ceiling is met. When fresh
work is exhausted, v1 endgame counts distinct outstanding coordinates held by
other peers so duplicate requests remain possible without reopening ordinary
work. Pure-v2 counts fresh blocks in active assemblies and eligible remote
pieces, excluding received and on-wire blocks, local completion, padding,
unwanted files, pending completion, foreign piece ownership, and per-peer
assembly/completion limits. Its count stops at the 32-block ceiling.

PeerBuffer credit is measured in bytes and rounded to full blocks for ordinary
window sizing. If positive headroom is smaller than one maximum block, retain
a one-request attempt so a short piece-tail block can use its exact lease; an
exact-size allocation still guards the byte cap. If there is no headroom, arm
the bounded memory retry even though the window avoids a doomed allocation.
This ensures a peer can resume when memory returns without waiting for another
protocol event.

### Ownership and invariant

- **Current owner:** `PiecePicker` tracks distinct fresh v1 blocks; each
  pure-v2 peer owns its assembly, response map, and request cursor.
- **Invariant preserved:** the controller never admits more new requests than
  fresh blocks available from that peer. Endgame duplicates remain a separate
  bounded path and do not count blocks already requested by the same peer.
- **Must not:** count received data or another peer's on-wire request as fresh
  work, or remove the one-block progress path used while rate credit refills.

### Acceptance

- Fresh v1 block counts stay correct through request, receipt, cancel, partial
  restore, and outstanding-request reset; peer availability traversal stops at
  the configured local ceiling.
- Endgame counts distinct unreceived blocks available to the peer and excludes
  its own requests.
- Pure-v2 counts only eligible blocks, subtracts received and on-wire work,
  and respects assembly and completion caps.
- Window limits remain 4–128 for v1/hybrid-v1 and 4–32 for pure-v2. The
  available-credit snapshot is advisory to the window; each actual request
  separately reserves global and torrent credit before enqueue. Rate changes
  and first-ready deadlines wake refill.
- Positive sub-block PeerBuffer headroom permits one exact-size attempt for a
  short tail block; zero headroom schedules the bounded retry path.
- Regression sources cover the picker, packed early-stop traversal, endgame
  duplicate accounting, and pure-v2 availability/assembly counts.

### Evidence

- **Regression sources:** `useful_missing_counts_only_fresh_peer_owned_enabled_blocks`,
  `useful_missing_preserves_peer_specific_endgame_duplicates`,
  `peer_bitmap_set_iteration_visits_only_valid_set_pieces`, and
  `v2_useful_missing_counts_only_remote_available_wanted_pieces` plus
  `v2_useful_missing_counts_only_unreceived_blocks_without_wire_requests`,
  `peer_request_admission_retries_after_payload_capacity_returns`.
- **Command or probe:** run with the owner-requested bulk pass.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** live-window parity and matched
  whole-client measurements; the local correctness suite passed at
  `354ad504`.

## NET-109 — Active-candidate piece picker

- **Priority / wave:** P1 / 6
- **Prerequisites:** NET-104
- **Owning component:** `rt-piece-picker`
- **Finding or capability:** F14/F15; bounded piece selection and honest index
  accounting.
- **Status:** `wip`

### Scope

Replace per-piece tree nodes and full-torrent fresh-work scans with
array-backed rarity buckets and an incrementally maintained candidate index.
NET-114 connects v1/hybrid ownership to `PiecePicker`; pure-v2 maintains the
same rarity policy in a separate v2-local `Availability` index. The indexes
never combine numeric piece IDs across protocol geometries. If the v2 index
cannot reserve its bounded memory estimate, the actor logs the degradation and
keeps the sparse cyclic fallback.

### Ownership and invariant

- **Current owner:** `PiecePicker` owns v1/hybrid rarity/priority selection
  and a compact received-block projection used for selection and fastresume.
  Pure-v2 owns a geometry-local rarity index and peer bitmaps. The live
  generation-aware ledger is authoritative for cross-peer wire reservations,
  response arbitration, and received-block ownership.
- **Target owner:** protocol adapters provide their own piece-space index to
  one bounded rarity policy; the shared ledger remains the sole cross-peer
  ownership authority. Picker and v2 local-have state remain derived
  selection/resume projections, never permission to reserve or publish
  content.
- **Invariant preserved:** cancellation, completion, rejection, restore, and
  enable/disable transitions update both the outstanding count and candidate
  membership consistently. Endgame sees only bounded in-progress pieces.
- **Must not:** report a piece as verified by selection alone, or include
  received bytes in a “have” result before verification.

### Acceptance

- Rarity and active-work indexes use dense arrays with swap-remove positions; after an interior removal, the position index points to the tail element left in that slot, not the removed element returned by `Vec::swap_remove`.
- Peer bitfield add/remove walks set pieces and skips empty bytes while
  preserving BitTorrent's MSB-first order and ignoring unused tail bits.
- Pure-v2 bitfield replacement, Have, HaveAll, HaveNone, and disconnect update
  the v2-local rarity counts by exact set-difference; completed pieces leave
  the fresh-candidate index and failed/rechecked pieces re-enter it.
- Pure-v2 peer request filling drains the current piece before choosing a new
  piece uniformly from the rarest peer-held bucket. A sparse cyclic fallback
  remains available only when the memory governor cannot admit the index.
- The picker exposes a constant-time fresh-work query so source schedulers can
  sleep while all wanted blocks are already requested.
- Selection chooses uniformly among matching peers in the rarest matching
  bucket; sampling is bounded for the common case and falls back to uniform
  reservoir sampling when matches are sparse.
- When probes miss in a large bucket, packed peer availability is traversed by
  set bits once to find the rarest requestable match, avoiding a
  candidate-by-candidate bitmap check for sparse peer bitfields. Small buckets
  are checked directly before selection advances to the next rarity.
- Sources with out-of-band full availability, including webseeds, randomize
  zero-availability ties too.
- In-progress pieces keep a forward cursor to the first block that may still
  be requested; cancellation rewinds it, and restore/reset skip received
  blocks without rescanning the requested prefix.
- Partial-piece receipt flags use a packed bitmap and a maintained count;
  completion checks stay O(1), and snapshots/bytes-left enumerate set bits.
- Priority, sequential, cancellation, partial restore, and endgame behavior
  remain covered by regression cases.
- Static and live estimates include packed flags, index arrays, bucket
  capacities, and the configured peer ceiling.
- The pure-v2 availability index reserves a conservative capacity bound from
  the configured connection ceiling before allocation; if admission fails,
  the actor records the fallback state instead of allocating uncharged index
  buckets.

### Files and compatibility

- **Expected code paths:** `crates/rt-piece-picker/src/{availability,flags,picker}.rs`,
  `crates/rt-engine/src/{torrent_task.rs,torrent_task_v2.rs}` packed peer
  availability, engine reservation accounting, picker README.
- **State or wire compatibility:** no resume or wire-format change.
- **Migration / rollback:** none; the reference API remains the current picker.
- **Feature flag or rollout boundary:** none; qualification is required before
  the new picker is relied on for release claims.

### Evidence

- **Regression source added or updated:** sparse bitfield MSB order and tail
  handling, availability bucket movement and candidate-tail position repair after repeated interior removals,
  seeded random tie, sparse peer fallback and equal-rarity selection,
  pure-v2 bitmap set-difference and rarest-peer-held selection,
  candidate lifecycle, source-scheduler fresh-work transitions, request
  cancellation, and picker memory estimate cases; block cursor advancement,
  cancellation rewind,
  partial restore, and reset cases; packed receipt-bit boundaries, duplicate
  counting, invalid tail indexes, and bitmap memory accounting.
- **Command or probe to run:** `cargo test -p rt-piece-picker` and the engine
  picker/accounting regressions during the requested bulk pass.
- **Expected result:** all cases pass; no requestable piece is lost from the
  candidate index and no inactive piece remains indexed.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** whole-engine qualification. The
  PERF-111 picker/snapshot probes ran twice on `kspls0` at `eb5f3158`; these
  diagnostic results do not establish a release threshold or whole-client win.

## IO-001 — Storage evidence contract and deterministic fault harness

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** `rt-storage`
- **Finding or capability:** explicit separation of write completion,
  servability, payload durability, and resume commit; cancellation-safe
  completion ownership.
- **Status:** `wip`

### Scope

Document storage evidence and introduce deterministic backend faults for the
portable scheduler. This slice covers cancellation after backend submission,
injected read/write failures, partial-write failure, retry after a failed data
barrier, and operation-level file identity fencing. Dirty path state binds each
typed `ContentEpoch` to the descriptor identity that received it. The admitted
`DiskSubmission` carries a `StorageOperationRecord` containing a process-wide
operation identity, the device-local operation sequence paired with its stable
device-queue key, the captured file identity and content epoch, the per-file
write sequence for dirty content, and payload read/write byte ranges where
applicable through backend completion; only that record can clear the dirty
state it covered. Coalesced peer reads also receive the sequence/key pair, an
operation identity, and range for tracing. A `DiskSubmission` directly retains
the opened descriptor handle and an immutable write-payload reference through
backend completion. Dispatch and owner-drop traces record logical queued bytes,
governor-backed queue credit, shared payload class/lease/view bytes, permit
counts, and retained descriptor/payload owners. Read dispatch also records the
frame-pool charge after size-class rounding; coalesced peer reads aggregate
their retained requests' queued credits and record their single frame charge
separately from governor-backed bytes.
Securely opened descriptor leases carry that captured identity through queued
and submitted operations. On Unix and Windows, reads, writes, and sync
operations check that the descriptor's original identity still matches the
current path at their completion boundary; a mismatch fails as uncertain, and
writes keep dirty evidence intact. Data-bearing hash jobs always retain their
queued-byte owner through closure completion. When trace-level logging is
enabled, they also capture a process-wide operation identity and record queue
and shared-payload credits at submission and release; normal operation avoids
that ID and snapshot work. Trace-enabled `MemoryLease` values now receive
stable process-wide IDs, preserve them across growth, and emit acquire/growth/
release events with consumer lifetime and post-transition governor snapshots.
Storage-operation credit records retain an exact lease ID for one contribution
and lossy, order-independent xor/sum diagnostics when reads coalesce several
contributions. The aggregate diagnostics can collide and are not an exact ID
set. A controlled local frame/queue/hash owner-chain regression checks
dispatch credit against live frame-pool and governor snapshots as owners are
retained and dropped. A second regression now holds an actual scheduler read
at backend completion and reconciles its dispatched queue credit, frame
charge, descriptor identity, and device permit with live scheduler and
governor state. New sources now hold two reads and a shared-payload write at
once, reconcile distinct storage/device IDs and per-operation credits against
live scheduler snapshots, and verify that two hash owners release their
credits independently. These sources passed in the exact-main bulk suite; restart cuts and platform
fault qualification remain open. Frame-pool charge attribution is present in
read-dispatch traces.

### Ownership and invariant

- **Current owner:** `MountScheduler` admits and sequences positioned I/O;
  backend workers own submitted operations until completion.
- **Target owner:** IO-006 device service, with per-operation identity and
  retained completion ownership.
- **Invariant preserved:** cancellation of a waiter cannot clear dirty evidence
  for a submitted write; failed partial writes cannot be reported as complete.
- **Must not:** equate successful write completion with durability or resume
  commitment.

### Acceptance

- Contract defines received, verified, write-complete, servable,
  payload-durable, and resume-committed separately.
- Test-only backend can inject read/write/sync failures, hold a completion, and
  write a prefix before returning an error.
- Cancellation while a submitted write is held leaves dirty state until a
  covering barrier completes.
- Partial-write failure is returned to the caller and remains dirty.
- Failed data sync leaves the dirty content epoch intact for a successful
  retry.
- Regression sources cover strict-write sync failure and a canceled
  multi-path checkpoint waiter whose held sync returns an injected error; a
  later write waits for the barrier to release and dirty evidence remains
  retryable. Platform-specific storage-failure qualification remains open.
- The positioned-I/O helpers retry `Interrupted` and positive short progress;
  regression sources simulate short reads and writes across multiple calls.
- Operation-record regressions prove local sequence values from distinct
  device queues remain distinguishable by their stable queue keys.
- Trace-enabled memory-lease regressions prove the lease ID survives growth,
  remains unique after release, and the resource snapshot returns to its
  pre-lease state. Storage-credit merge regressions preserve exact single IDs
  and lossy, order-independent diagnostics when contributions are coalesced.
- Controlled frame/queue/hash owner-chain regression sources reconcile rounded
  frame-pool charge and queued/shared-payload credit against live snapshots
  through hash-owner drop, with two independently retained hash jobs and
  before/after governor snapshots. Production-scheduler regressions hold real
  positioned reads at their backend boundaries and reconcile captured operation
  IDs, byte ranges, descriptor owners, frame charges, shared-payload leases, and
  device permits against live scheduler and governor snapshots. A three-operation
  source holds two reads and one shared-payload write concurrently, then checks
  each owner's credit through release. These sources passed in the exact-main bulk suite; physical
  crash cuts and platform-specific fault qualification remain open.
- Path-replacement regressions cover secure open, cached descriptor reopening,
  scheduler snapshots, and peer-read readahead (IO-004), plus held payload
  reads, writes, and checkpoint syncs. Dirty writes remain tied to the original
  file identity; replacement during an operation returns
  `FilesystemStateUncertain` before stale read bytes reach the caller and
  without clearing dirty evidence. Completion owners retain descriptor and
  immutable write-payload references; traces expose their queue credit and
  retained permit/owner counts. Read dispatch records the actual frame-pool
  charge, including size-class rounding, separately from governor-backed queue
  bytes. Data-bearing hash jobs retain queue/payload credit through the worker
  closure; at trace level they also record operation-scoped credit at
  submission and release without a global ID allocation on the untraced hot
  path. Regression sources cover frame-credit separation, rounded frame
  charge, hash queue credit lifetime with before/after governor snapshots,
  trace lease identity/lifetime, and
  coalesced credit diagnostics; deterministic before/after assertions across
  the complete storage-frame, queue, and hash pipeline remain open. Crash cuts
  and platform-specific fault qualification remain open.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/backend.rs`,
  `scheduler.rs`, test-only `test_support.rs`, and
  `certification/transfer-rebuild/storage-evidence-contract.md`.
- **State or wire compatibility:** no public storage or resume format change.
- **Migration / rollback:** none in this test-only slice.
- **Feature flag or rollout boundary:** injected backend is `cfg(test)` only.

### Evidence

- **Regression source added or updated:**
  `injected_read_failure_is_returned_without_marking_payload_dirty`,
  `scripted_zero_progress_read_and_write_are_reported_as_short_io`,
  `pre_submission_open_failures_release_storage_admission`,
  `injected_write_failure_is_returned_and_remains_dirty_until_barrier`,
  `strict_write_sync_failure_releases_admission_and_preserves_dirty_evidence`,
  `exact_read_retries_interrupted_and_positive_short_progress`,
  `write_all_retries_interrupted_and_positive_short_progress`,
  `cancelled_waiter_keeps_submitted_write_dirty_until_barrier`,
  `cancelled_strict_write_retains_admission_until_sync_completion`,
  `partial_write_then_error_is_not_reported_as_success`,
  `failed_barrier_keeps_dirty_evidence_for_retry`,
  `cancelled_checkpoint_waiter_keeps_dirty_evidence_after_sync_failure`,
  `dropping_backend_releases_and_joins_held_completion_workers`, and
  `replaced_path_after_held_read_is_not_returned_to_caller`, and
  `device_operation_record_disambiguates_local_sequences_by_queue`, plus
  `storage_credit_keeps_frame_pool_charge_separate_from_governor_bytes` and
  `hash_job_owner_retains_queued_credit_until_drop`, plus the size-class charge assertion in
  `into_bytes_releases_charge_without_copying_payload`, and
  `traced_lease_identity_is_stable_through_growth_and_release`, and
  `storage_frame_queue_multi_hash_credit_reconciles_live_snapshots`; the
  `production_scheduler_dispatch_credit_matches_live_owners_until_completion`
  regression executes an admitted scheduler read against a held backend and
  checks dispatch evidence against live queue, device, and governor ownership;
  `production_scheduler_multi_operation_credit_reconciles_until_completion`
  holds two reads and one shared-payload write and checks unique operation IDs,
  ranges, owners, per-class credits, and live resource snapshots before and
  after completion. All added sources passed in the exact-source suite at `354ad504`; order-independent
  credit-merge regression includes lease-ID diagnostics.
- **Command or probe to run:** `cargo test -p rt-storage` during the requested
  bulk pass.
- **Expected result:** injected read/write failures are returned; failed reads
  do not dirty payload state, failed writes remain dirty until a barrier,
  cancellation retains dirty ownership through a barrier, partial failure
  remains an error, and a failed barrier preserves dirty state for retry.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** real-device and filesystem-specific
  crash/replacement qualification.

## IO-002 — Typed v2 block and piece extents

- **Priority / wave:** P0 / 3
- **Prerequisites:** IO-001, SYS-002, NET-101
- **Owning component:** `rt-engine` v2 actor and transfer geometry adapters.
- **Finding or capability:** F01; wire-block coordinates must not size a
  whole-piece assembly.
- **Status:** `done`

### Scope

Use typed geometry for v1 and v2 receive/serve range validation, and for
whole-piece v2 assembly sizing. The existing piece maps remain the storage-path
adapters; shared request ownership and actor migration remain NET-104/NET-114.

### Ownership and invariant

- **Current owner:** `torrent_task.rs` and `torrent_task_v2.rs` own peer
  requests; the v2 task owns per-peer assemblies.
- **Target owner:** NET-114 shared ledger owns request/content state; geometry
  remains a pure adapter.
- **Invariant preserved:** block length bounds one wire response; piece length
  bounds full verification, read-back, and publication.
- **Must not:** use a block extent as a hash or persistence extent.

### Acceptance

- Outgoing/received block coordinates validate as aligned, nonempty, bounded
  blocks within the mapped piece.
- Peer upload requests validate as bounded served blocks; invalid ranges are
  rejected through the existing protocol behavior.
- Assembly capacity and whole-piece length use the typed piece extent.
- Multi-block memory and spill-path regressions cover full-piece completion,
  short final pieces, and request pipelining.
- v1 and hybrid cross-plane ownership are not claimed by this ticket.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/coords.rs`,
  `torrent_task.rs`, `torrent_task_v2.rs`, and the existing v2 tests.
- **State or wire compatibility:** no format or wire change.
- **Migration / rollback:** none; old storage map remains the reference.
- **Feature flag or rollout boundary:** live v1/hybrid picker and pure-v2 sparse
  traversal; SYS-013 keeps the reference route available until qualification.

### Evidence

- **Regression source added or updated:** existing `v2_*` multi-block memory
  and spill cases plus the within-piece request-window case.
- **Command or probe to run:** targeted v2 engine regressions during the
  requested bulk pass.
- **Expected result:** request coordinates agree with their geometry plane;
  all requested v2 blocks fill the piece and whole-piece assembly length
  matches the geometry adapter.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** Rust 1.88.0 and 1.94.0 full-workspace logs under `/tmp/tng-transfer-core-close-20261019/evidence/`.
- **External evidence still required:** none for this code slice; full shared
  actor parity remains a later gate.

## IO-003 — Lease-owning immutable receive events

- **Priority / wave:** P0 / 3
- **Prerequisites:** IO-001
- **Owning component:** rt-engine v1 peer and webseed receive paths.
- **Finding or capability:** F16; a queued payload and its memory reservation
  must share one lifetime across asynchronous boundaries.
- **Status:** wip

### Scope

Carry the existing receive allocation lease inside a private event wrapper
through peer mailboxes, actor processing, and storage waits. Keep the public
BlockEvent shape unchanged. Eviction spill must retain ownership of its
original allocation without creating a second piece-sized copy.

### Ownership and invariant

- **Current owner:** the v1 actor admits peer-response bytes before requests;
  the peer loop carries the lease into the received event; webseed fetch
  reserves its response body.
- **Core boundary:** the shared transfer ledger owns coordinate reservations
  and arbitration. The actor's `LeasedBlockEvent` owns the immutable payload
  and its `MemoryLease` together through actor and storage work. This keeps
  the deterministic ledger independent of the resource governor without
  splitting payload lifetime from its charge.
- **Target owner:** the private LeasedBlockEvent carries both content and
  reservation into storage, which retains the same lease through backend
  completion without charging a shared payload a second time.
- **Invariant preserved:** dropping a stale, rejected, cancelled, or completed
  event releases its reservation only after its bytes stop being retained.
- **Must not:** release a webseed body reservation when fetch returns, or lose
  the piece-assembly charge while an eviction slice still references its
  backing allocation.

### Acceptance

- Peer and webseed events retain their lease through actor handling.
- Leased block writes use immutable `Bytes` slices and keep the original
  payload lease through cancellation and backend finish; they add only the
  per-job queue overhead charge.
- A webseed fetch regression proves the body charge remains live after the
  fetch helper returns and releases when the event drops.
- Evicted spill ranges share the original immutable allocation and lease.
- Partial flush copies reserve their transient memory before allocation.
- The live v1, pure-v2, and hybrid paths carry the admitted immutable payload
  lease into actor handling; storage submission retains it through backend
  completion. Hybrid wire/canonical conversion reuses contiguous Bytes and uses
  bounded block-local copies only when file or padding spans are discontinuous.

### Files and compatibility

- **Expected code paths:** crates/rt-engine/src/torrent_task.rs,
  torrent_task/peer_session.rs, torrent_task/peer_transfer.rs.
- **State or wire compatibility:** no public BlockEvent, resume, or wire
  format change.
- **Migration / rollback:** none; the wrapper is internal to the actor.
- **Feature flag or rollout boundary:** live v1, pure-v2, and hybrid receive paths.

### Evidence

- **Regression source added or updated:** webseed event lease lifetime,
  piece-assembly copied-range lease accounting, and
  evicted_block_views_share_the_assembly_memory_lease, plus the NET-103
  request-to-event ownership regression and a shared-payload storage
  cancellation regression.
- **Command or probe to run:** targeted rt-engine lease and spill regressions
  during the requested bulk pass.
- **Expected result:** charges remain live through the final event/slice owner
  and return to zero after it drops.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** platform fault, cancellation, and
  crash/replacement qualification of storage-operation completion ownership.

## IO-004 — Shared secure file identity and descriptor catalog

- **Priority / wave:** P0 / 3
- **Prerequisites:** IO-001
- **Owning component:** `rt-storage` secure open path, scheduler cache, and
  public `HandleCache`.
- **Finding or capability:** F17; cached descriptors must remain tied to the
  filesystem object the authorized path currently names, and metadata I/O
  must not serialize unrelated cache operations.
- **Status:** `wip`

### Scope

Centralize path normalization, platform file identity, and replacement
detection in `open.rs`. `HandleCache` is the sole descriptor-cache
implementation; the scheduler's `FilePool` is a narrow stats/open-policy
adapter. Production `StorageRuntime` and scheduler resources share the one
process-global path map and descriptor limiter. Isolated caches remain
available to unit fixtures. The catalog tightens its process capacity and
idle lifetime to the strictest scheduler bounds observed during startup.

### Ownership and invariant

- **Current owner:** `HandleCache` owns secure normalized path keys, identity
  checks, descriptor leases, and cache lifecycle. `FilePool`, `StorageRuntime`,
  and scheduler resources resolve through the same process-global descriptor
  map; `open.rs` owns no-follow opens and platform identity operations.
- **Policy boundary:** all production owners report the same nonzero
  `catalog_id`; engine storage statistics merge its gauges and counters once
  even when scheduler worker resources differ. Each instance retains its own
  scheduler/backend identity and counters.
- **Invariant preserved:** unlink/recreate or symlink replacement invalidates
  a cached descriptor before it serves a later path-based operation.
- **Must not:** perform a filesystem identity check while holding the cache
  mutex or reuse a descriptor after its path has been replaced.

### Acceptance

- Scheduler and public cache share the same OS-specific identity helpers.
- Path identity checks happen outside the cache mutex, followed by a pointer
  check before refreshing an entry.
- A newly opened descriptor is checked against the current path before it is
  published; repeated replacement races fail after a bounded retry count.
- Cache-hit validation uses the stored descriptor identity plus one path
  identity lookup; it does not `fstat` the cached descriptor on every hit.
- Reads can reuse a read+write descriptor, while read-only descriptors never
  serve writes.
- Every leased descriptor retains the identity captured by secure open.
  Queued reads validate that identity after backend completion and before
  returning or caching bytes, so a path replacement cannot make an old
  descriptor's payload appear current.
- The LRU index shares each path allocation and expiry checks only the stale
  oldest prefix, avoiding a path-string clone and full-map scan on every hit.
- Replacement, unlink/recreate, descriptor-budget, and in-flight-lease
  regressions continue to cover the catalog lifecycle; a zero idle TTL keeps
  expiry disabled.
- The public `HandleCache` reuses a current read+write descriptor for reads,
  never reuses read-only descriptors for writes, and removes a cached
  read-only entry when a read+write entry is published.
- Scheduler and direct-runtime clones resolve the same absolute path to the
  same catalog entry and descriptor lease; engine aggregation counts that
  catalog once rather than once per scheduler resource.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/open.rs`,
  `scheduler.rs`, `handle_cache.rs`, and `file_handle.rs`.
- **State or wire compatibility:** no storage-path, resume, or wire-format
  change.
- **Migration / rollback:** none; path opening remains no-follow.
- **Feature flag or rollout boundary:** none; production runtime and scheduler
  use the shared catalog, with portability and replacement qualification still
  open.

### Evidence

- **Regression source added or updated:**
  `read_only_handles_do_not_serve_writes_and_reads_reuse_writable_handle`,
  `reopens_path_after_unlink_and_recreate`, `zero_idle_ttl_disables_expiry`,
  `write_handle_snapshot_drops_replaced_paths`,
  `process_global_handle_catalog_has_one_identity_and_owner_map`,
  `replaced_path_after_held_read_is_not_returned_to_caller`, plus existing
  descriptor-budget, in-flight-lease, and scheduler cache-stat regressions.
- **Command or probe to run:** `cargo test -p rt-storage` during the requested
  bulk pass.
- **Expected result:** scheduler and runtime use one cache implementation and
  identity policy; replaced files are reopened; identity checks do not hold
  the cache lock during filesystem access; reads reuse writable handles; and
  live leases continue to bound descriptors.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** NFS and real-device replacement
  behavior under concurrent move and transfer load.

## IO-005 — Cancellation-safe storage admission ownership

- **Priority / wave:** P0 / 4
- **Prerequisites:** IO-003, IO-004
- **Owning component:** `rt-storage::MountScheduler` and backend completion
  watchers.
- **Finding or capability:** admitted queue slots and byte budgets must remain
  owned until blocking preparation or submitted backend work stops retaining
  the operation.
- **Status:** `wip`

### Scope

Carry `DiskSubmission` through queued file preparation and backend completion
for reads, writes, and sync barriers. The completion watcher owns queue/device
permits and queued-byte charges if the caller cancels while backend work is
still running. Strict writes retain the same admission through their own sync.

The elevator worker already owns its independent request, queue, and device
leases through its backend read. The operation record binds device-local
sequence and stable queue identity with file/content evidence; shared
resource-credit transfer and the full injected-fault matrix remain open.

### Ownership and invariant

- **Current owner:** the scheduler admits queue and device slots plus queued
  bytes in `DiskSubmission`; its operation record carries the device-local
  sequence, stable queue identity, and file/content evidence through
  completion helpers.
- **Target owner:** IO-006's device service owns one bounded operation record
  from admission through exactly one backend completion result.
- **Invariant preserved:** dropping a waiter cannot make a queued slot or
  byte reservation available while its blocking or backend operation still
  retains work.
- **Must not:** leak admission on a pre-submission error or release it before
  an accepted backend operation completes.

### Acceptance

- File-open preparation closures retain admission while running after caller
  cancellation and release it promptly on error.
- Backend read, write, and sync completion watchers retain operation
  admission until their backend receiver resolves, even if their return
  channel is closed.
- Readahead's additional queued-byte charge follows its larger read through
  backend completion.
- Strict writes hold admission through their fdatasync; checkpoint writes
  release it only after the write completion.
- Pre-submission open failures and injected read, write, partial-write, and
  barrier failures assert that queued bytes and device slots are returned. A
  strict-write sync failure also returns admission while preserving dirty
  evidence for a later successful barrier.
- End-to-end scheduler regressions cancel submitted writes while completion is
  held, prove queue slots and queued-disk memory leases stay charged, verify
  later admission is rejected at each limit, and prove admission returns after
  backend completion. A strict-mode case cancels during the write's sync,
  retains dirty evidence, and recovers admission only after sync completion.
- Canceling a checkpoint barrier while its per-file sync is held retains the
  device and queued-byte admission until completion; because the canceled
  caller cannot consume barrier evidence, dirty state remains for a later
  successful barrier.
- The full fault-injected device-service matrix remains open.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`,
  `backend.rs`, `test_support.rs`, and IO-006's device service.
- **State or wire compatibility:** no resume, storage-path, or wire-format
  change.
- **Migration / rollback:** none; the portable backend remains selectable.
- **Feature flag or rollout boundary:** no behavior flag; storage service
  rollout remains later work.

### Evidence

- **Regression source added or updated:**
  `cancelled_submitted_write_retains_queue_slot_until_completion`,
  `cancelled_submitted_write_retains_memory_budget_until_completion`,
  `cancelled_submitted_readahead_read_retains_extra_bytes_until_completion`,
  `cancelled_strict_write_retains_admission_until_sync_completion`,
  `cancelled_checkpoint_barrier_retains_admission_until_sync_completion`,
  `strict_write_sync_failure_releases_admission_and_preserves_dirty_evidence`,
  `cancelled_submitted_read_retains_admission_until_failure_completion`,
  plus helper-level completion-owner lifetime cases for canceled read/write
  waiters and admission-release assertions for open and injected I/O failures.
  Canceled submitted read and write cases now receive delayed backend errors;
  the strict-write cancellation case does the same for sync after the submitted
  sync completes. These prove canceled callers retain admission until
  completion while failed results leave dirty evidence for retry where needed.
- **Command or probe to run:** `cargo test -p rt-storage` and the scripted
  backend cancellation matrix during the requested bulk pass.
- **Expected result:** accepted work holds its queue/device/byte owner until
  backend completion; queue-slot and memory-budget admission recover after
  completion, including strict-write sync and the extra charge for an expanded
  peer read, with no leak after pre-submission failure.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** cancellation and queue behavior under
  real-device and NFS latency.

## IO-006 — Shared device service over the portable backend

- **Priority / wave:** P0 / 5
- **Prerequisites:** IO-005
- **Owning component:** `rt-storage` device admission and positioned-I/O path.
- **Finding or capability:** F22; mounts on one physical device need one
  scheduling and admission authority.
- **Status:** `wip`

### Scope

Device admission identity is independent of each mount's storage-profile label
and per-mount queue limit. Schedulers that resolve to the same device share a
bounded `DeviceQueue`; schedulers without a device identity retain a
storage-root-scoped fallback. The storage-job dispatcher keeps one stable
fallback root identity across its copy schedulers, so unsupported platforms
do not create a fresh, unshared queue for every plan. The queue capacity comes
from the shared storage I/O depth, while active dispatch uses one slot per
portable worker or a continuously refilled 64-operation io_uring window.
Queued work uses the `DeviceElevator` class weights, then round-robins distinct
storage-root owners within each class. A physical-device label is not reused as
the owner key, so schedulers sharing one device remain distinct fairness
participants. A unique operation ID and permit travel through the existing
completion owner. Pending depth and availability use an O(1) counter; canceled
waiter guards remove their own entry, so normal admission does not scan the
pending queue. Cache hits and file-open preparation happen before device
admission so they do not occupy a physical-device slot.

The live service currently covers scheduler positioned reads/writes/syncs,
peer-read batches, preallocation, extent probes, and secure storage-plan
copy/import steps. Checkpoint barriers group ready file fences by resolved
device and dispatch bounded batches through that device's queue, preserving a
separate completion watermark for each path. Plan steps enter the destination
mount's `MoveCopy` lane; source and destination paths each contribute
physical-device admission, with all touched device queues acquired in stable
key order. The 64 KiB streaming buffer is charged once while the operation
owns mount/device admission. Live v1/hybrid and pure-v2 upload reads now pass a
response deadline into device admission; Fast allowed-request reads that must
be served while locally choked carry choke-critical priority. The dispatcher
selects choke-critical work immediately, then expired-deadline work by earliest
deadline, before normal weighted-class turns. These promotions occur only
while dispatching the bounded pending queue; uncontended admission stays on
the fast path. Urgent peer reads bypass the scheduler-local coalescer wait.
Normal peer reads coalesce within one scheduler before entering the shared
physical-device queue. Cross-mount coalescing is deferred as a performance
experiment: it is useful only when distinct schedulers issue adjacent reads to
the same file, and must preserve each caller's mount admission and completion
owner. It is not a gap in physical-device ownership or fairness. Full
injected-backend accounting remains open.

### Ownership and invariant

- **Current owner:** a weak registry shares one bounded, class-aware dispatcher
  by device ID. The dispatcher keeps a FIFO per storage-root owner and class,
  with one owner turn per class round. Per-mount semaphores still enforce local
  queue and class caps;
  `DiskSubmission` retains byte, mount, and device-operation ownership through
  the selected backend's completion path.
- **Target owner:** the device service owns every disk-bound operation record
  from dispatch through completion and applies class, deadline, and
  choke-critical policy across all mounts on that device.
- **Invariant preserved:** one physical device ID maps to one admission pool,
  independent of profile labels and per-mount `max_queue` values.
- **Must not:** create a second device gate because the same device was
  classified differently, or release a permit before accepted backend work
  completes.

### Acceptance

- Schedulers with one device ID share the same admission owner across profile
  and per-mount queue differences.
- Engine queue-capacity/available summaries count one shared device ID once,
  even when its mounts expose different profile labels.
- Unknown topology remains bounded by a storage-root fallback.
- Scheduler reads, writes, syncs, preallocation, extent probes, and coalesced
  peer reads pass through the device owner with one operation identity.
- Queued classes receive the documented weighted share; canceled waiters are
  removed and accepted operations retain the device slot through late backend
  completion. Within a class, distinct storage-root owners sharing a physical
  device receive round-robin turns. The service caps active operations to its
  backend dispatch window and bounds pending records by the remaining device
  queue depth.
- `MountScheduler::run_device_operation` is the reusable dispatch handoff for
  non-scheduler consumers. `run_device_operation_with_paths` admits every
  touched physical device in stable key order; secure copy/import steps use
  this path so reverse-direction copies cannot deadlock. Peer deadline and
  choke-critical inputs come from live v1/hybrid and pure-v2 upload-read
  adapters. Checkpoint barriers
  resolve path groups before admission and dispatch a bounded same-device
  batch while preserving per-path fences and operation identities.
- Canceling a caller after an external handoff starts does not release its
  queue slot, physical-device permit, or queued-byte lease before the blocking
  closure completes; another mount sharing the device remains backpressured
  until that completion.
- Cancellation and injected-backend regression sources cover the full
  scheduler permit and queued-byte accounting path.
- A skewed same-class queue from two storage-root owners verifies that a busy
  root cannot place all of its pending work ahead of another root sharing the
  same device. Regression source is authored; bulk execution remains deferred.
- Device-queue regression sources cover choke-critical and expired-deadline
  promotion ahead of normal work while ordinary traffic retains weighted
  class shares.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`,
  `elevator.rs`, the portable backend, and deterministic backend fixtures.
- **State or wire compatibility:** no resume, storage-path, or wire-format
  change.
- **Migration / rollback:** preserve the portable direct path until the
  service passes correctness and cancellation qualification.
- **Feature flag or rollout boundary:** no service rollout until all operations
  use the shared dispatcher.

### Evidence

- **Regression source added or updated:**
  `schedulers_on_same_device_share_global_queue_across_profiles` covers one
  physical ID with different profile and `max_queue` values;
  `engine_stats_does_not_double_count_shared_device_queue_across_profiles`
  covers its aggregate queue counters; `device_dispatch_uses_weighted_class_fairness`
  records the class share and `cancelled_device_waiter_releases_bounded_queue_capacity`
  covers pre-dispatch cancellation; `cancelled_waiter_keeps_submitted_write_dirty_until_barrier`
  now also checks that caller cancellation retains the device slot until the
  injected backend completion; `external_device_handoff_shares_admission_across_mounts`
  checks that an external blocking operation shares device admission across
  schedulers; `cancelled_external_handoff_retains_admission_until_completion`
  checks that a canceled external caller retains the shared device permit and
  queued-byte lease until the blocking closure returns an injected error;
  `multi_device_handoff_holds_source_and_destination_until_completion` checks
  both physical-device permits remain held through external work. These sources
  passed in the exact-source workspace suite; reverse-mapping and saturation
  qualification remain open.
- **Command or probe to run:** `cargo test -p rt-storage` in the requested bulk
  pass.
- **Expected result:** every device-bound operation uses the same bounded
  service while per-mount queue limits remain independent; operation ownership
  survives cancellation through backend completion.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** fairness and device saturation under
  HDD, NVMe, network mounts, and concurrent multi-mount workloads.

## IO-007 — Batched hash jobs and shared hybrid content views

- **Priority / wave:** P0 / 5
- **Prerequisites:** IO-002, IO-003, IO-005
- **Owning component:** `rt-storage` hashing and the production hybrid actor.
- **Finding or capability:** v1 and BEP 52 checks must reuse immutable payload
  bytes, submit bounded hash work in batches, and verify both integrity views
  for hybrid torrents.
- **Status:** `done`

### Scope

`MountScheduler` already shares a bounded hash worker pool across schedulers
with matching storage configuration. v1 recheck streams SHA-1 updates, v2
recheck hashes up to 64 leaves per job, and the final streaming Merkle fold now
runs as one hash-pool job. Hybrid metadata now enters the v1 actor with a
compact v2 projection containing aligned file ranges and authenticated piece
layers. The in-memory completion path computes SHA-1, the file root, and the
piece-layer root from one immutable piece buffer and one bounded pool job; the
disk fallback streams each file range once and checks both roots while keeping
BEP 47 padding only in the v1 view. Disk recheck now routes through the same
hybrid-aware verifier instead of directly invoking the v1-only verifier. The
projection maps v2 files back to v1 file indexes because synthetic v1 pad
files shift those indexes.

### Ownership and invariant

- **Current owner:** the v1 task owns hybrid piece/file geometry and its
  mutable assembly lease. Once complete, the task freezes that allocation
  into immutable `Bytes` whose owner carries the lease, shared by hashing,
  spill blocks, and file writes; storage queues retain each view through
  backend completion and charge only per-job overhead in addition to the
  existing payload reservation.
- **Target owner:** NET-114 moves coordinates and request ownership into the
  shared ledger; typed actor/storage events carry the immutable payload span
  and its `MemoryLease` through hashing and backend completion. The pure
  decision core remains independent of the runtime governor without changing
  v1/v2 root semantics.
- **Invariant preserved:** queue rejection cannot consume the caller's only
  hash state or release the payload lease early; v1 and v2 views refer to the
  same verified content while preserving their distinct padding rules. A
  completed in-memory piece has one payload allocation until its verification
  and aggregate writes finish.
- **Must not:** retain one leaf hash per byte range for a whole large file or
  duplicate payload buffers just to change hash geometry.

### Acceptance

- Hash workers remain bounded by the shared queue and owned resource limits.
- Large-file v2 verification batches leaf hashes and folds the root off the
  async executor without retaining all leaf hashes.
- Hybrid memory and disk paths check both v1 and authenticated v2 roots;
  regressions cover v1 padding, a short final piece, the one-piece file-root
  shape, v1/v2 index differences, a v2-only expected-root mismatch, and
  zero-copy freezing/slicing of the completed piece allocation. Full disk
  recheck must also reject a v1-valid piece whose authenticated v2 root differs.
- Hash-pool retries retain the immutable content and one queued-byte
  reservation across saturation; shared payload jobs carry the existing
  allocation lease and charge queue overhead once. A canceled queued closure
  retains both until dequeue, while the active closure owns them through
  finish.

### Evidence

- **Regression source added or updated:**
  `rt-storage::verify::v2_file_verify_batches_leaf_hashing_per_chunk` now
  asserts three leaf batches plus one root job; hybrid scheduler, streaming
  verifier, memory-completion, and compact metadata-projection regressions now
  cover shared payload roots, padding, short tails, v2-only mismatch, and the
  v1 index shift after a pad file. `completed_piece_freezes_and_reuses_its_assembly_allocation`
  checks pointer identity and confirms completed spill views do not allocate
  peer-buffer copies; `completed_payload_views_keep_piece_lease_after_assembly_is_dropped`
  verifies a queued immutable view keeps the original allocation charged
  after the actor-side assembly is gone.
  `cancelled_queued_hybrid_hash_retains_shared_payload_lease_until_dequeued`
  covers waiter cancellation while the accepted hash closure remains queued;
  the task-level hybrid completion/recheck regression verifies that a v1-valid
  disk piece with a mismatched v2 root is rejected. These regressions passed
  in both exact-main workspace runs.
- **Command or probe:** focused storage verifier, scheduler, and engine
  regressions; these passed in the exact-source bulk suite recorded above.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** Rust 1.88.0 and 1.94.0 full-workspace logs under `/tmp/tng-transfer-core-close-20261019/evidence/`.
- **External evidence still required:** none for the bounded hash contract;
  whole-client hash throughput and CPU remain PERF-107/110/113 evidence.

## IO-008 — Completion-driven transfer actors

- **Priority / wave:** P0 / 8
- **Prerequisites:** IO-002, IO-006, IO-007, NET-104, NET-106
- **Owning component:** v1/hybrid `TorrentTask` and pure-v2 torrent actor.
- **Finding or capability:** peer progress must continue while bounded storage
  writes and piece verification complete; only the actor may apply results to
  picker, completion, and resume state.
- **Status:** `wip`

### Scope

The authored v1/hybrid slice moves direct received-block writes, completion
verification, and aggregate writes into a bounded actor-owned job set. Picker
and progress state change only when the actor applies the typed completion.
Pure-v2 moves download-rate admission, direct block writes, complete-piece
storage reads, Merkle hashing, and aggregate writes into a bounded per-peer
job set; a piece stays unavailable until its completion is applied. Existing
request and assembly memory leases remain alive through their storage/hash
consumers.

This is still partial: failure/reset and actor-level lifecycle qualification
remain open. v1/hybrid block, verification, and aggregate-write jobs carry
monotonically increasing piece-attempt generations, and stale completions
cannot mutate a retry. Pure-v2 assemblies plus block and piece completions use
per-peer monotonically increasing generations with the same stale-result
fencing. v1 lifecycle snapshots drain accepted work before shutdown, pause,
recheck, and storage-move snapshots. Pure-v2 peer shutdown
uses a per-peer lifecycle generation to stop new request and upload work,
cancels download-rate and upload-rate waits, and drains dispatched upload
reads plus already-admitted storage and verification jobs before the torrent
task finishes joining peers. Queued upload requests are discarded at shutdown.
The same signal preempts framed sends and hash-proof waits owned by the peer
loop, so a blocked socket writer does not prevent the drain; failure/reset and
broader actor lifecycle qualification remain open.

### Ownership and invariant

- **Current owner:** each torrent actor owns its picker and transfer state;
  v1/hybrid storage and verification results are actor-applied, while pure-v2
  completed-piece results are applied by the peer loop that owns that peer.
- **Target owner:** bounded job sets own storage/hash futures and payload
  leases until completion; each actor remains the sole writer of its piece,
  peer, lifecycle, and durability state.
- **Invariant preserved:** write completion precedes verification; a piece
  remains pending until its integrity result is applied; accepted work is
  serialized with piece reset and drained before v1 quiesce, move, and clean
  shutdown snapshots. Pure-v2 drains peer-owned work on protocol exit and
  orderly torrent shutdown. Both implementations fence completions by
  piece-attempt generation; the broader failure/reset matrix remains open.
- **Must not:** spawn untracked work, drop a payload lease before backend
  completion, advertise or persist unverified bytes as complete, or let an old
  job commit into a replacement attempt.

### Acceptance

- Active peer controls and lifecycle commands continue to progress while a
  storage write or hash job is held.
- Per-torrent active work and deferred input have explicit bounds; saturation
  returns request ownership for retry without losing a valid piece.
- Direct-write and in-memory assembly paths both apply storage/hash results
  through the actor; hybrid pieces still verify both roots from shared bytes.
- Failure, cancellation, retry, piece reset, pause, storage move, and shutdown
  preserve picker accounting and memory leases through late backend completion.
- Pure-v2 shutdown preempts a pending framed send and hash-proof wait, then
  stops new per-peer work, cancels upload-rate waits, and drains dispatched
  upload reads plus already-admitted writes and verification before releasing
  peer state.
- A piece becomes locally available only after verification and, for the
  aggregate path, successful payload write; fastresume barriers cover all
  accepted writes that the snapshot claims.

### Files and compatibility

- **Expected code paths:** `torrent_task.rs`,
  `torrent_task/peer_transfer.rs`, `torrent_task_v2.rs`,
  `rt-piece-picker`, and the storage completion ownership helpers.
- **State or wire compatibility:** no wire-format change; resume state keeps
  the existing format and only records actor-applied piece results.
- **Migration / rollback:** no runtime flag or persistent-format change has
  been added. The old synchronous helpers remain available for recheck and
  other non-peer paths; reverting the actor job routing remains a source
  rollback until v1/hybrid and pure-v2 failure cases, actor-level lifecycle
  cases, and bulk evidence are reviewed.
- **Feature flag or rollout boundary:** no separate flag; this is the
  production actor completion contract required before NET-114 integration.

### Evidence

- **Regression source added or updated:** `pending_verification_does_not_publish_piece_as_have`
  checks picker state stays pending until verification is applied;
  `failed_block_write_releases_picker_request`,
  `hash_failure_does_not_persist_piece_as_valid`, and
  `restored_partial_piece_writes_new_blocks_directly` cover v1 failure and
  retry behavior; `v2_piece_is_not_available_until_completion_job_finishes`
  checks a pure-v2 piece remains unavailable and unwritten while its completion
  job is queued, then publishes after successful hash and write. Existing
  v2 multi-block and request-lease sources were adapted to drain the job;
  `v2_peer_charges_download_bytes_once_and_keeps_control_responsive` covers an
  upload rate wait, then makes the shared download tree go into debt after a
  request is admitted and confirms the received piece still completes without
  a second network-rate charge while the peer loop handles controls.
  `v2_peer_shutdown_cancels_a_backpressured_socket_send`
  covers a peer writer blocked by a non-reading socket. These sources passed
  in the exact-source workspace suite; failure/reset cases, actor-level
  disconnect/quiesce, and broader lifecycle qualification remain open.
  `v2_peer_shutdown_drains_admitted_upload_reads` holds a dispatched
  upload future and verifies shutdown drain waits for it to finish.
  `v2_torrent_shutdown_cancels_and_joins_peer_tasks` exercises the torrent
  owner’s cancellation-and-join path; command-loop, disconnect/quiesce, and
  broader lifecycle qualification remain open.
  `stale_piece_attempt_completions_cannot_mutate_a_retry` covers v1/hybrid
  block-write and verification completions from a replaced attempt, while
  `v2_stale_piece_attempt_completions_cannot_mutate_a_retry` covers pure-v2
  block-write and verification results from a replaced attempt.
- **Command or probe to run:** focused actor and picker regressions during the
  requested bulk pass.
- **Expected result:** actors process controls while accepted transfer work is
  pending and publish only verified, fully written pieces.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** saturated mixed-peer completion
  latency and resource bounds.

## NET-103 — Reserve receive capacity before requests

- **Priority / wave:** P0 / 4
- **Prerequisites:** NET-101, NET-102, IO-003
- **Owning component:** `rt-engine` v1 and pure-v2 peer request paths.
- **Finding or capability:** F13/F16; an admitted wire response needs a byte
  reservation before the request can cause the receive allocation.
- **Status:** `wip`

### Scope

Acquire `PeerBuffer` capacity before each v1, pure-v2, or hybrid peer request
reaches the wire. The v1/hybrid reservation is sized for its canonical extent,
so hybrid v2 zero-padding expansion remains covered by the same lease. The
matching response transfers its owner into actor handling; pure-v2 keeps each
block lease with its outstanding coordinate through storage and verification.
All paths retry after temporary capacity denial.

### Ownership and invariant

- **Current owner:** v1 `TorrentTask` and `V2TorrentTask` choose coordinates;
  peer loops own outstanding responses and their generation-safe byte leases.
- **Target owner:** the same admitted lease follows v1/hybrid responses into
  their block event and pure-v2 responses through their final block consumer.
- **Invariant preserved:** no v1, pure-v2, or hybrid request reaches the wire
  unless its expected canonical payload bytes have already been admitted under
  `MemoryClass::PeerBuffer`.
- **Must not:** reserve again after receiving an admitted response or strand a
  picker coordinate when a request command, socket write, peer event, or
  terminal path fails.

### Acceptance

- A denied reservation prevents command enqueue and returns the picker block.
- The peer command owns the reservation while queued; a choked or abandoned
  command releases it.
- A matching response transfers the reservation to its `LeasedBlockEvent`
  without an extra allocation charge.
- A pure-v2 request stores its lease with the outstanding coordinate until
  Piece, Reject, legacy Choke, or disconnect; temporary admission denial is
  retried on the one-second peer tick.
- Reject, legacy Choke, timeout, disconnect, and failed delivery release byte
  leases and return coordinates through the existing lifecycle events. Fast
  Choke retains on-wire requests and their leases until a terminal response.
- A failed memory admission returns the picker coordinate and schedules a
  bounded retry on the existing one-second peer-control tick.
- Unsolicited or mismatched blocks do not inherit a request lease.
- The live ledger and hybrid facade now share canonical request ownership;
  contiguous payload projections share Bytes, and only discontinuous hybrid
  file/padding views need a bounded block-local copy. IO-003/IO-005 own the
  immutable lease and device-operation lifetime.

### Files and compatibility

- **Expected code paths:** `torrent_task.rs`,
  `torrent_task/peer_transfer.rs`, `torrent_task_v2.rs`, and the pure
  `transfer::Ledger` contracts.
- **State or wire compatibility:** no wire or resume-state change; request
  admission may stop earlier under the existing memory cap.
- **Migration / rollback:** none; the actor remains on its current picker and
  request maps.
- **Feature flag or rollout boundary:** live v1, pure-v2, and hybrid request
  paths; reference rollback and qualification are tracked by SYS-013.

### Evidence

- **Regression source added or updated:**
  `drained_download_bucket_schedules_a_token_ready_wakeup` checks the queued
  request owns a byte lease;
  `peer_request_admission_retries_after_payload_capacity_returns` checks a
  denied request releases its picker slot and retries after capacity returns;
  `peer_loop_moves_request_reservation_into_piece_event` checks that the same
  reservation remains charged through the received event and releases at
  drop; `v2_request_admission_retries_after_capacity_returns_and_owns_each_block`
  checks v2 no-send under denial, retry, per-block ownership through full Piece
  storage/verification, and remaining-request disconnect release;
  `v2_choke_preserves_fast_request_obligations`
  checks the request lease survives Fast Choke and releases on legacy Choke;
  `v2_reject_releases_only_matching_response_lease` checks matching Reject
  release and mismatched Reject retention.
  These v2 and hybrid regression sources passed in the exact-source suite at `354ad504`; bulk
  qualification remains open.
- **Command or probe to run:** targeted request-admission and peer-loop lease
  regressions during the requested bulk pass.
- **Expected result:** request bytes are charged before enqueue, stay charged
  without a second lease while outstanding and through event handling, then
  return to zero on all terminal paths.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** bounded mixed-peer and v2/hybrid
  qualification.

## NET-104 — Authoritative request and content ownership ledger

- **Priority / wave:** P0 / 5
- **Prerequisites:** NET-101, NET-102, NET-103, IO-005
- **Owning component:** pure transfer core in `rt-engine`.
- **Finding or capability:** torrent-wide single ownership, bounded late
  responses, and generation-safe peer identity.
- **Status:** `wip`

### Scope

The `Ledger` owns per-piece block state and per-connection reservations,
reconciles response/commit event reordering, accepts useful late data, and
returns bounded protocol actions. A conservative 64 MiB logical piece-state
budget, a 4,096-live-connection ceiling, a 1,024-request per-peer ceiling, a
65,536-owner torrent ceiling, and a four-owner endgame cap keep its principal
retained collections bounded. NET-114 uses it for live v1, hybrid-v1, and pure-v2 outbound request ownership
and received-block arbitration. Pure-v2 keeps protocol-local response leases
and per-peer piece assemblies, with piece-affine reservations. The live hybrid
facade binds both wire planes to one canonical v1 ledger and projects accepted
data and cancellation actions back to each connection.

### Ownership and invariant

- **Current owner:** the ledger owns live v1/hybrid-v1/pure-v2 outbound
  reservations, hybrid dual-view arbitration, connection generations,
  fastresume-restored partial blocks, and applicable cancellation actions.
  `PiecePicker` remains the candidate-ranking and resume projection. Pure-v2
  actor maps retain response leases and per-peer assembly state, with atomic
  piece-affinity reservation.
- **Target owner:** NET-114 applies the same ownership contract to hybrid
  dual-view transfers. Geometry-specific piece assembly and verification remain
  with their actors; storage retains immutable payload owners through backend
  completion.
- **Invariant preserved:** each missing block is reserved once outside
  endgame; connection generations isolate stale events; received blocks are
  verified as complete pieces before `have` is set.
- **Must not:** infer a peer identity from an address or let a late response
  mutate a replacement connection or newer content epoch.

### Acceptance

- The per-piece slots move only through missing, reserved, received, and
  verification outcomes; invariants cross-check slot owners, connection maps,
  and in-flight totals.
- Acquisition, commit, Reject, and received events use the exact canonical
  aligned block extent; a shorter response cannot mark an entire ledger slot
  received or release a different live request.
- Response-before-commit is reconciled once; its pending reconciliation
  history and late-response tombstones have hard per-connection caps.
- Piece-state admission and release preserve the configured logical byte
  budget; live connections, peer request depth, torrent-wide in-flight
  ownership, and endgame duplicate owners have explicit hard ceilings.
- Fastresume partial blocks seed the ledger's received slots under the same
  PeerBuffer charge as live piece state; a complete partial bitmap is discarded
  and can never mark a piece available without verification.
- A useful tombstoned response may supersede a replacement request and
  returns cancellation actions for the replacement owners.
- Completion, hash failure, choke, reject, timeout, disconnect, resume-have,
  partial resume, and epoch advance have regression sources.
- Seeded simulation replays churn, loss, late replies, and hash failures with
  an invariant check after every event.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/{ledger,sim}.rs`,
  `crates/rt-engine/src/torrent_task/peer_transfer.rs`, and
  `crates/rt-engine/src/torrent_task/peer_session.rs`.
- **State or wire compatibility:** the ledger is in-memory only; no wire or
  fastresume format changed. The live request adapter is wired for v1, pure-v2,
  and hybrid wire planes.
- **Migration / rollback:** the picker remains the selection/resume projection;
  reverting the actor adapter returns request ownership to the prior actor path.
- **Feature flag or rollout boundary:** v1, hybrid-v1, and pure-v2 outbound
  request ownership, including pure-v2 timeout expiry, are wired but remain
  unqualified for live parity; simulator invariants and the exact-source bulk
  suite passed at `354ad504`.

### Evidence

- **Regression source added or updated:** ledger transition cases and seeded
  simulator replay; current additions include bounded commit history, late
  data superseding a replacement reservation, byte-budget lifecycle, bounded
  connection registration, request/endgame caps, and rejection of partial or
  mismatched request, commit, Reject, and response extents.
- **Command or probe to run:** transfer-core and simulator tests during the
  requested bulk pass.
- **Expected result:** all transitions preserve ledger conservation and all
  configured simulator seeds complete reproducibly.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** matched live actor parity and storage
  commit/recovery qualification.

## NET-105 — Negotiated peer request obligations

- **Priority / wave:** P0 / 6
- **Prerequisites:** NET-104
- **Owning component:** v1 and pure-v2 peer protocol handlers, then the shared
  transfer runtime.
- **Finding or capability:** F08; Fast negotiation changes request, choke,
  reject, and cancellation lifecycles.
- **Status:** `wip`

### Scope

Honor negotiated BEP 6 behavior in the existing handlers while moving the
complete obligation state machine into the live shared runtime. The
`transfer::InboundObligations` module owns the v1 and pure-v2 inbound upload
queue/service lifecycle. The generation-aware ledger also owns live
v1/hybrid-v1 and pure-v2 outbound reservations, received-block arbitration,
and pure-v2's 60-second request expiry; hybrid dual-view projection is wired through the canonical live ledger.

### Ownership and invariant

- **Current owner:** v1 and pure-v2 inbound upload queue, in-service, Cancel,
  choke, and completion transitions use `InboundObligations`; the peer loops
  own storage futures and wire writes. The ledger owns v1/hybrid-v1 and
  pure-v2 outbound reservation and response arbitration, including v2 timeout
  expiry; hybrid dual-view projection is wired through the canonical live ledger; qualification remains open.
- **Target owner:** `SharedUploadService` owns live inbound obligations and
  upload read/byte admission; the shared ledger owns outbound request state
  across both hybrid wire planes.
- **Invariant preserved:** when Fast is negotiated, a request already
  accepted for processing remains owed a Piece or Reject response across
  Choke and Cancel; Reject is sent only when the peer negotiated Fast.
- **Bound preserved:** the per-connection cap covers queued and in-service
  requests together. Duplicate coordinates are recognized before capacity
  refusal, so saturation cannot turn a duplicate into a second response.
- **Must not:** clear a Fast request merely because Choke arrived, send a
  Reject to a legacy peer, or answer one request more than once.

### Acceptance

- v1 and v2 emit Reject only when Fast was negotiated.
- A Fast Choke preserves on-wire v1 and v2 requests. The v1 peer loop retains
  their response-byte leases and its actor cancels commands that had not
  reached the wire; v2 retains pending request coordinates and assembly
  state. Legacy Choke may clear that state.
- A remote Cancel removes a v1 upload request only while it is still queued
  for storage admission. Under Fast, that canceled queued request gets a
  Reject; an in-service read keeps its Piece/Reject obligation.
- Both live upload adapters apply the shared total-work bound and suppress
  duplicates before checking capacity. Legacy Choke ends in-service
  obligations without sending Reject; Fast Choke rejects queued requests
  except pieces explicitly in the local allowed-fast set.
- Once a v1 block is accepted, the actor queues endgame Cancel for peers with
  duplicate requests. A peer loop sends it only if that request reached the
  wire and keeps the response lease until its terminal event.
- Request cancellation and endgame loser handling reconcile exactly once with
  queued reads, in-flight work, Piece delivery, and Reject delivery.
- Stale, duplicate, and unsolicited responses cannot mutate current content.
- Both v1 and pure-v2 inbound upload paths use the shared obligation owner.
  v1/hybrid-v1 and pure-v2 outbound ownership now uses the ledger, including
  v2 timeout expiry and hybrid dual-view arbitration. Regression and public-
  client parity qualification remain open.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/obligations.rs`,
  `torrent_task.rs`, `torrent_task_v2.rs`, and NET-114 peer-runtime adapters.
- **State or wire compatibility:** no wire-format change; negotiated Fast
  behavior follows the peer's handshake capabilities.
- **Migration / rollback:** no persisted state changes. The inbound owner is registered in the process-wide shared service; v1, pure-v2,
  and hybrid routes remain reversible through SYS-013 while parity/recovery
  evidence is unfinished.
- **Feature flag or rollout boundary:** no new advertisement or capability
  until the applicable live path satisfies its obligations.

### Evidence

- **Regression source added or updated:**
  `v2_reject_obligations_require_fast_negotiation`,
  `v2_choke_preserves_fast_request_obligations`,
  `interrupted_in_service_request_can_be_requeued_without_losing_its_obligation`,
  `fast_choke_keeps_wire_request_lease_and_legacy_choke_releases_it`,
  `fast_choke_cancels_queued_requests_but_keeps_wire_requests`, and the
  existing v1 `fast_peer_request_while_choked_is_rejected_not_dropped`.
- Focused v1 regression sources now cover queued-versus-in-service Cancel,
  late Piece after outbound Cancel, and exact endgame duplicate matching.
  Shared-ledger and hybrid sources passed in the exact-source workspace suite;
  public-client interoperability and live cancellation qualification remain
  open. Added shared-owner
  sources cover duplicate requests at capacity, total queued-plus-in-service
  bounds, allowed-fast dispatch while choked, and legacy in-service cleanup.
- **Command or probe to run:** targeted v1/v2 Fast lifecycle and shared-ledger
  simulations during the requested bulk pass.
- **Expected result:** no owed request is lost across Fast Choke/Cancel and
  each negotiated request reaches one terminal response.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** public-client interoperability.

## NET-106 — Independent peer progress during upload work

- **Priority / wave:** P1 / 7
- **Prerequisites:** NET-103, NET-104, NET-105
- **Owning component:** v1 and pure-v2 peer protocol loops.
- **Finding or capability:** keep protocol input and control handling responsive
  while bounded upload work waits on storage or rate admission.
- **Status:** `wip`; live actors route this work through bounded tasks and
  shared admission. Local regressions passed at `354ad504`; mixed-peer
  qualification remains open.

### Scope

Move peer-requested upload reads and rate-budget waits into bounded per-peer
work. The owning protocol loop continues to read and process incoming control
messages while those jobs wait. Socket writes remain serialized by the
connection owner; actor-side received-block writes, verification, and
progress persistence remain IO-008.

### Ownership and invariant

- **Current owner:** each protocol loop owns its socket and upload response
  ordering; bounded worker futures own storage payloads and leases until the
  loop sends or rejects the response.
- **Target owner:** NET-110/114 move admission and request obligations into the
  shared upload service while preserving a single socket writer per peer.
- **Invariant preserved:** each peer has bounded active upload reads and a
  bounded pending request queue; storage/rate waits never hold the protocol
  loop's message-polling future; choke or shutdown wakes rate waiters and
  invalidates stale responses before they are sent.
- **Must not:** release a payload lease while storage completion still owns
  the read, or send a block after its upload slot was revoked.

### Acceptance

- v1 and pure-v2 continue to process incoming interest/control messages while
  an upload read or rate-budget wait is pending.
- Active read count and pending request count have explicit per-peer bounds.
- Choke, slot revocation, cancellation, and shutdown release or reject work
  according to negotiated Fast semantics.
- A deterministic regression holds upload rate admission after the storage
  read completes, sends a control message, and observes that message before
  the budget is released.
- NET-114 now uses the shared upload service without adding a second socket
  writer or bypassing its ownership accounting.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/torrent_task.rs`,
  `torrent_task_v2.rs`, and the NET-114 peer adapters.
- **State or wire compatibility:** no wire-format change; only scheduling
  between already bounded local jobs changes.
- **Migration / rollback:** keep the reference actor selectable through SYS-013 until bulk parity and
  rollback coverage qualify a broader cutover.
- **Feature flag or rollout boundary:** NET-114 is integrated behind the per-torrent
  reference/shared route selected by SYS-013; keep both routes available until
  rollback and parity evidence qualify the cutover.

### Evidence

- **Regression source added or updated:**
  `peer_loop_keeps_reading_while_upload_waits_for_rate_budget` and
  `v2_peer_reads_control_messages_while_upload_waits_for_rate_budget`.
- **Command or probe to run:** targeted v1/v2 peer-loop regressions during the
  requested bulk pass.
- **Expected result:** each loop processes control input while its upload
  worker remains rate-limited, and shutdown releases all worker-owned state.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** mixed-speed peer control latency and
  bounded resource use under a saturated upload queue.

## NET-107 — Hierarchical fair payload rate scheduler

- **Priority / wave:** P1 / 8
- **Prerequisites:** NET-103, NET-104, NET-106
- **Owning component:** pure scheduler in `crates/rt-engine/src/transfer/rate.rs`,
  then the process-wide network budget and live peer adapters.
- **Finding or capability:** remove copied per-peer torrent buckets, avoid
  timer polling, and share configured limits fairly across live torrents and
  peers.
- **Status:** `wip`

### Scope

Use one deterministic `RateTree` per transfer direction to admit payload bytes
under process-wide and per-torrent limits, with weighted per-peer fairness.
Each core accepts an injected monotonic timestamp, keeps
fractional byte-nanosecond credit, and may carry debt so a configured rate
below one block per second still makes progress. Wait results identify the
first useful retry deadline, and a bounded due-peer query drains wake work
without scanning every connection; weighted start-time fairness limits how
far one peer runs ahead of waiting peers in its torrent. The engine owns
wall-clock timers, limit-change notifications, cancellation, and socket or
storage work.

Deadlines describe the last grant attempt. A limit change or refund can make
parked requests eligible sooner, so the live adapter must promptly reconsider
the affected requests instead of waiting for their previous deadlines.

`TorrentKey` is unique for a task generation while stale peer work may still
exist. `TransferPeerId` combines that key with the connection-local generation-safe
`ConnectionId`, because separate torrent ledgers can both register `conn#0.1`.
NET-114 and NET-115 own actor registration, payload admission, limit updates,
cancellation/refund, and shared source scheduling.

### Ownership and invariant

- **Current owner:** `GlobalNetworkBudget` owns global upload and download
  `RateTree` instances. v1/hybrid-v1 and pure-v2 peer uploads register
  generation-safe `(TorrentKey, ConnectionId)` identities, reserve before
  socket writes, and settle only after successful writes; dropped grants
  refund unused bytes. Both peer protocol families restore and apply their
  persisted per-torrent upload and download caps. v1/hybrid-v1 download
  requests reserve tree credit before enqueue and carry it through the
  outstanding request until a matching payload settles it; rejects, timeout,
  choke, cancellation, or teardown refund unused credit. Pure-v2 download
  requests now reserve tree credit before enqueue and carry it through the
  outstanding request until Piece receipt settles the bytes received; Reject,
  Choke, timeout, cancellation, and disconnect refund unused credit. Webseed
  reserves before HTTP ranges. Peer and torrent teardown remove scheduler
  identities and return unsettled grants without confusing reused connection
  slots across torrents. Pure-v2 settles a block's download grant at Piece
  receipt; its subsequent storage job uses the storage scheduler without
  charging the same network bytes again. Bulk qualification remains open.
- **Target owner:** one process-shared upload tree and one process-shared
  download tree each own global and per-torrent byte credit, peer fairness
  progress, unsettled grant/refund accounting, and the next-wakeup index.
- **Invariant preserved:** a grant debits all applicable levels atomically;
  a denial does not partially consume torrent or peer service. A rate update
  clamps existing credit and cannot mint another burst.
- **Must not:** key process-wide state by a `ConnectionId` alone, scan every
  peer on each grant/timer pass, or hold the tree lock while waiting or doing
  I/O.

### Acceptance

- Rate peer identity remains distinct when two torrent ledgers reuse the same
  connection slot and generation; removing one torrent leaves the other's
  rate state intact.
- Global and torrent limits are shared across peers and grants debit every
  limited level together or none.
- Fractional refill and debt make sub-block-per-second limits progress, with
  whole-nanosecond retry deadlines and no floating-point drift.
- Weighted peers share a saturated torrent allowance; a new peer starts at
  the current virtual floor rather than at zero.
- Updating a peer's weight preserves its fairness progress and unsettled
  grant accounting.
- Re-registering or updating a torrent limit preserves accumulated debt and
  never mints a burst; unlimited-to-limited starts without accumulated credit.
- Refunds restore only granted bytes that were not settled as transferred
  payload, and clamp at bucket capacity.
- Ordered per-torrent active/wait indexes and the global wake index stay
  coherent across grant, wait, refund, peer removal, and torrent removal.
- The due-peer query returns only indexed peers at or before the supplied
  deadline, respects its caller-provided batch bound, and leaves entries
  indexed until a grant attempt reconciles them.
- Limit changes and refunds can enumerate a bounded batch of parked peers even
  when their old deadlines are still in the future; each retry reconciles or
  updates its existing deadline index entry.
- Limit changes and refunds promptly reconsider affected parked requests, so
  an earlier eligibility time is not hidden by a stale timer deadline.
- Live v1, pure-v2, and webseed traffic uses the same directional global and
  per-torrent service. Request admission remains bounded by memory and device
  capacity.
- A received pure-v2 block consumes one download grant: the grant is reserved
  before Request enqueue and settled on Piece receipt; later storage admission
  does not charge the download tree again.

### Files and compatibility

- **Expected code paths:** `transfer/rate.rs`, `network_budget.rs`,
  `torrent_task/peer_transfer.rs`, `torrent_task/peer_session.rs`,
  `torrent_task_v2.rs`, and NET-114/115 integration.
- **State or wire compatibility:** no wire or persisted-state change; rates
  continue to count payload bytes at the engine boundary.
- **Migration / rollback:** all live transfer directions use the tree at the
  applicable global and per-torrent levels. Reverting the actor-adapter
  changes restores the previous limiter paths.
- **Feature flag or rollout boundary:** the tree is active without a runtime
  feature flag. Regression sources passed at exact source `354ad504`; mixed-
  torrent qualification remains open.

### Evidence

- **Regression sources:** exact deadlines, sub-block progress, shared
  global/torrent budgets, all-or-nothing grants, weighted fairness, no-burst
  limit updates, unsettled-grant refunds tied to the limit epoch that charged
  them, settled-payload protection, peer
  weight updates, index
  cleanup, deadline-filtered bounded wake batches, unknown peers, repeated
  torrent-limit updates, and
  `reused_ledger_connections_are_isolated_between_torrents`; bounded waiter
  batches also cover immediate reconsideration after a global limit change.
- **Live upload regression sources:** peers across different torrents share
  global credit; limit changes wake parked uploads; unused grants refund; v1
  peers share the configured torrent bucket; and pure-v2 per-torrent limit
  updates wake parked peer admission. These sources passed in the exact-source suite at `354ad504`.
- **Live download regression sources:** the global download tree shares credit
  across torrents and wakes parked admissions after a limit update; torrent
  credit constrains peers together and is exposed as request-window advisory.
  v1 request grants settle on matching payloads and refund on terminal request
  paths; pure-v2 reserves before request enqueue and settles on Piece receipt,
  while terminal outcomes refund (`v2_reject_refunds_reserved_download_rate_credit`).
  Pure-v2 piece storage uses no second download grant; the source regression
  `v2_peer_charges_download_bytes_once_and_keeps_control_responsive` checks
  that storage progresses while the shared download tree is in debt. Webseed
  reserves before issuing HTTP ranges. These sources passed in the exact-source suite at `354ad504`.
  Both v1/hybrid and pure-v2 peer loops subscribe to process-wide and
  per-torrent change signals.
- **Command or probe to run:** pure scheduler regressions and the engine rate
  integration suites during the requested bulk pass.
- **Expected result:** deterministic grants conserve each configured budget,
  return useful wake deadlines, and keep peer identities separate across
  torrent actors.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** exact-source Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external evidence remains listed below.
- **External evidence still required:** live mixed-torrent fairness and
  matched whole-client CPU, memory, and goodput measurements.

## NET-111 — Fair seed and download choking policies

- **Priority / wave:** P1 / 10
- **Prerequisites:** NET-104, NET-107, NET-110
- **Owning component:** `rt-peer-manager` policy, v1 and pure-v2 peer actors.
- **Finding or capability:** F12; choose upload slots from useful peer feedback
  and preserve exploration for peers without a recent history.
- **Status:** `wip`

### Scope

The shared `Choker` ranks peers by download contribution while downloading and
upload contribution while seeding. It assigns regular slots plus a rotating
optimistic slot. Pure-v2 peer loops now apply the same policy through a
task-owned watch keyed by connection identity, so reconnecting at the same
address does not inherit an old slot. The choker accepts a seed for
deterministic tie replay and an explicit monotonic timestamp for replay; the
replay fixture now covers downloading/seeding mode changes, stale rates,
interest changes, peer joins/departures, optimistic-slot fairness, and
transition-only outputs. Its regression passed in both exact-main workspace
runs.

### Ownership and invariant

- **Current owner:** v1 `TorrentTask` owns choker cadence and peer snapshots;
  pure-v2 now uses that policy but still owns its peer state separately.
- **Target owner:** one mode-aware policy ranks comparable peer snapshots;
  socket-owning peer loops serialize the resulting wire transitions.
- **Invariant preserved:** only interested peers compete, slot count stays
  bounded, optimistic rotation avoids repeatedly selecting the same waiting
  peer, ranking uses the rate appropriate to download or seed mode, and a
  revoked slot is rechecked after storage and rate waits before sending data.
  A replacement connection receives a fresh policy identity even when its
  socket address is reused.
- **Must not:** infer useful transfer from slot membership or let the choker
  write peer sockets directly.

### Acceptance

- Downloading mode rewards recent bytes received; seeding mode rewards recent
  bytes sent.
- Not-interested peers stay choked; regular and optimistic slots are bounded.
- Optimistic ties can be replayed from a fixed seed and waiting peers receive
  fair rotation.
- A caller-supplied monotonic timeline drives rotation without wall-clock
  sleeps.
- v1 and pure-v2 actors send only actual choke-state transitions.
- A pure-v2 upload that finishes after its slot is revoked is discarded and
  resolved according to the negotiated Fast behavior.
- Reconnecting at the same socket address does not inherit the prior
  connection's selected slot.
- A deterministic swarm trace covers mode changes, stale rates, interest
  changes, joins, departures, and optimistic-slot fairness, and replays the
  exact decision and choke-transition outputs from the same seed and timeline.

### Files and compatibility

- **Expected code paths:** `crates/rt-peer-manager/src/choker.rs`,
  `crates/rt-engine/src/torrent_task.rs`, `torrent_task_v2.rs`, and the
  deterministic trace-replay regression source.
- **State or wire compatibility:** no wire-format change; slot behavior changes
  only peer upload scheduling.
- **Migration / rollback:** retain the reference actor path until shared-core
  migration has parity and rollback coverage.
- **Feature flag or rollout boundary:** none; external swarm evidence remains
  qualification work.

### Evidence

- **Regression source added or updated:** mode-aware rankings, optimistic
  history, seeded tie replay on virtual monotonic time, interest transitions,
  v2 wire-state changes, connection identity across reconnect, and
  `seeded_swarm_trace_replays_mode_stale_interest_join_leave_and_fair_rotation`.
- **Command or probe to run:** peer-manager and v2 peer-policy tests during the
  requested bulk pass.
- **Expected result:** mode changes select the matching feedback rate, seeded
  traces replay identically, and v2 emits no duplicate choke transitions.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** real-client seed and download swarm
  behavior under mixed peer speeds.

## NET-110 — Bounded shared upload service

- **Priority / wave:** P1 / 9
- **Prerequisites:** NET-103, NET-105, NET-106, NET-107
- **Owning component:** pure transfer core, then live peer upload runtime.
- **Finding or capability:** F06/F08; valid upload demand must not be silently
  dropped, and accepted requests keep their negotiated terminal response.
- **Status:** `wip`

### Scope

The pure `UploadService` remains the deterministic policy reference. The live
`SharedUploadService` registers generation-safe BEP 6 obligations and
process-wide round-robin job/byte admission together for v1/hybrid and pure-v2.
The peer loop retains socket ownership and ordered responses. Composite
torrent/connection identities prevent collisions between torrent-local slots.
Requests with zero or greater-than-one-block lengths are refused. The configured byte ceiling has a
one-wire-block minimum, then acts as a hard cap. A disconnect ends protocol
obligations but does not release capacity for reads already dispatched to
storage. The live read-registration cap uses configured `network.max_peers`;
the live facade couples both registration lifetimes and derives a cloneable,
generation-scoped read handle from the registered peer. New reads cannot be
admitted through the process service without that handle; dropping the peer
registration closes new work while already-issued permits remain charged
through completion. Peer loops continue reading control input while bounded
storage/rate jobs progress, with one ordered socket writer per peer.

### Ownership and invariant

- **Current owner:** `SharedUploadService` owns live BEP 6 obligations and
  read/byte admission by generation-safe peer identity; worker tasks receive
  read admission only through a handle derived from that peer registration.
  Actor loops own the socket and ordered response path.
- **Target owner:** the same shared upload service owns accepted obligations
  and in-flight read accounting until the actor completes or rejects a response.
- **Invariant preserved:** live registrations stay within the process-wide
  connection cap; queued and active reads stay within per-peer queue, global
  job, per-peer job, and byte limits; a connection identity is not
  reused while a storage read from that generation is live; completed reads
  for a closed peer are discarded without reopening admission early.
- **Must not:** drop valid demand because of an arbitrary time window or reclaim
  an in-flight budget while its backend read may still be running.

### Acceptance

- Fast overload and choked/canceled requests receive the required Reject;
  legacy peers receive no unsupported Reject.
- Per-connection round robin prevents one peer's queue from occupying all
  dispatches.
- Job and byte budgets bound dispatched reads, including after disconnect;
  one valid block is admitted even when configuration is below one block.
- Read-job identities are monotonic and never reused; exhaustion refuses
  dispatch before removing an obligation from its per-peer queue.
- The process-wide live read-registration table never exceeds configured
  `network.max_peers`;
  disconnected generations with unfinished reads remain bounded by the job
  limit and do not consume a live-registration slot.
- Re-registering a live connection identity is refused, including while its
  disconnected generation still owns an unfinished read.
- Equal ledger connection slots and generations in separate torrents remain
  distinct registrations; disconnecting one cannot remove the other's work.
- Regression sources cover queue bounds, byte bounds, fairness, invalid wire
  lengths, connection-generation reuse, live registration capacity, choke,
  cancel, and disconnect completion ownership. They passed at exact source
  `354ad504`.
- NET-114 routes live v1, pure-v2, and hybrid upload obligations through the
  shared registration owner; code integration and local regressions passed at
  `354ad504`, with live parity and cancellation latency qualification open.
- The read scheduler's identity-based admission method is private to the
  shared-service module; production actor jobs cannot construct an unscoped
  read request directly from `GlobalNetworkBudget`.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/transfer/{coords,upload,upload_scheduler,obligations}.rs`
  and the NET-114 peer adapters.
- **State or wire compatibility:** no wire-format change.
- **Migration / rollback:** the reference actor remains selectable through SYS-013 until bulk parity and
  rollback coverage qualify a broader cutover.
- **Feature flag or rollout boundary:** no release capability claim until
  live paths and cancellation accounting qualify.

### Evidence

- **Regression source added or updated:** upload service queue, fairness,
  budgets, cross-torrent identity isolation, choked reads, disconnect-held
  budget, and exhausted-job-identity preservation cases.
- **Command or probe to run:** transfer-core upload tests during the requested
  bulk pass.
- **Expected result:** no accepted request disappears and disconnected reads
  remain accounted until completion.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** real-client Fast behavior and disk
  cancellation latency under load.

## IO-009 — Per-file durability watermarks

- **Priority / wave:** P0 / 9
- **Prerequisites:** IO-004, IO-006, IO-008
- **Owning component:** `rt-storage::MountScheduler` write and sync paths.
- **Finding or capability:** F19 and F21; barriers must cover the admitted
  file writes they claim to make durable and must not erase newer dirty
  evidence.
- **Status:** `wip`

### Scope

Order checkpoint barriers against writes to each normalized path and across
the scheduler's complete path set. Ordinary writes share scheduler admission
and each file gate, so they remain concurrent. A checkpoint takes scheduler
write admission exclusively, waits for admitted write/preallocation/sync
operations, snapshots dirty and active paths, then groups them by resolved
device and dispatches bounded batches of ready per-file fences. Later writes,
including writes to a new path, remain queued until the evidence is dropped
after the resume commit. Dirty generations remain separate from per-file
sequence watermarks so a newer generation cannot be accidentally cleared.

### Ownership and invariant

- **Current owner:** each `MountScheduler` clone shares scheduler-wide write
  admission, a global diagnostic write sequence, weakly registered per-path
  fences, its dirty-generation map, and completion watchers for submitted
  writes and syncs. Full checkpoint barriers run in an owned task so dropping
  a waiter does not cancel a partially completed path snapshot.
- **Target owner:** IO-006's shared device service receives bounded batches
  of ready per-file fences grouped by resolved device; each path retains its
  own completion watermark and device operation identity.
- **Invariant preserved:** a scheduler-wide checkpoint admits no later
  write/preallocation/sync operation until every path in its stable snapshot
  has completed `fdatasync`; per-file fences, the detached barrier task, and
  completion watchers retain ownership across caller cancellation. A
  successful result keeps scheduler write admission in its evidence until the
  resume commit drops it.
- **Must not:** clear a newer generation, sync a clean unrelated descriptor,
  open more descriptors than the bounded batch permits, or let
  preallocation/write completion pass a barrier queued behind it.

### Acceptance

- `sync_data` and checkpoint sync capture a sequence watermark after all
  earlier admitted writes to that path complete.
- A checkpoint cannot miss a write admitted on a new path after path
  enumeration begins; that write waits for the returned barrier and remains
  dirty for the next one.
- Canceling the barrier waiter cannot release write admission while any path
  in the snapshot is still syncing; the detached barrier finishes the snapshot
  and returns no evidence to a canceled waiter.
- A write queued behind a path's exclusive fence waits until its `fdatasync`
  finishes and remains dirty for a later barrier.
- Failed barriers and partial writes retain dirty evidence; a successful
  strict write clears only its own generation.
- `sync_all_open_files` keeps descriptor use bounded and syncs only its
  scheduler's dirty or active paths.
- Ready paths on one resolved device enter a bounded batch together; batch
  size does not exceed the device dispatch window, handle-pool capacity, or
  mount queue capacity, and each path retains an independent fence, dirty
  generation, storage-operation identity, and device-operation identity. A
  reused barrier admission cannot pin the prior path descriptor across batches.
- `sync_all_open_files_with_evidence` returns an opaque proof only after every
  path in the barrier succeeds; it reports synced-file count and the stable
  scheduler write-sequence watermark, and it retains write admission until
  the proof is dropped after its resume commit.
- Per-device batching is implemented as bounded path groups. The local
  regression suite passed at `354ad504`; cross-device crash and storage
  fault-matrix evidence remain open.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`, IO-006's
  device service, and crash-cut fixtures.
- **State or wire compatibility:** no protocol change; the evidence API is
  additive and does not alter storage payload layout.
- **Migration / rollback:** none; the reference backend stays selectable.
- **Feature flag or rollout boundary:** durability mode is unchanged.

### Evidence

- **Regression source added or updated:** existing canceled-writer,
  dirty-generation, and failed-barrier retry sources; held-sync regressions
  cover a new-path write queued behind the scheduler-wide checkpoint, a
  canceled waiter whose barrier must finish a multi-path snapshot, and
  same-device paths entering one bounded batch before either sync completes.
- **Command or probe to run:** storage fault matrix and barrier ordering
  regressions during the requested bulk pass.
- **Expected result:** a returned barrier covers every earlier admitted write
  to its path, preserves all later dirty generations, and leaves unrelated
  paths untouched.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** target-filesystem crash validation.

## IO-010 — Resume persistence tied to barrier evidence

- **Priority / wave:** P0 / 10
- **Prerequisites:** IO-009, SYS-009
- **Owning component:** `rt-engine` fast-resume save path and `rt-fastresume` durability watermark.
- **Finding or capability:** a persisted resume record must distinguish data
  proven durable by a completed storage barrier from a save that skipped or
  could not perform a data sync.
- **Status:** `wip`

### Scope

Consume `rt-storage::DurabilityBarrierEvidence` when a successful checkpoint is
attached to fast-resume state. The proof retains scheduler write admission
through the blocking atomic resume-file write, even if the async caller is
canceled. Persist the synced-file count and the scheduler write-sequence
watermark for audit and recovery diagnostics. A fast-mode save
or unsupported filesystem records `synced = false` and zeroes the current
barrier evidence; a failed barrier leaves the checkpoint unclean. Additive
watermark fields default to zero when reading existing records.

### Ownership and invariant

- **Current owner:** `MountScheduler` constructs opaque evidence; the
  `TorrentTask` save path passes it to `FastresumeStore`, which attaches the
  resulting watermark and keeps the proof alive in its blocking writer until
  the existing atomic fast-resume write returns.
- **Target owner:** the same storage-to-resume evidence boundary remains, with
  engine/database publication ordering integrated under SYS-009.
- **Invariant preserved:** a record can set `synced = true` only after
  consuming evidence returned by a fully successful scheduler barrier. A
  skipped, unsupported, failed, or missing proof cannot be converted into a
  synced record.
- Canceling the async waiter after the atomic writer starts does not release
  scheduler write admission before the resume write completes.
- **Cross-store boundary:** fastresume and its barrier proof own piece/block
  trust; `DbWorker` owns lifecycle and runtime/API projections. These stores do
  not expose a shared transaction. Startup validates resume state or rechecks
  payload, then republishes progress and lifecycle state from that result;
  SQLite `amount_left` is never accepted as proof that payload bytes exist.
  A crash between the fastresume rename and the following SQLite projection
  can leave the projection stale, but restore repairs it before the actor
  publishes its recovered running state. SYS-009 covers the worker commit
  boundary, while physical crash-cut reconciliation remains qualification.
- **Must not:** fabricate proof in `rt-fastresume`, infer durability from
  `clean_shutdown`, or break decoding of pre-evidence records.

### Acceptance

- Fast-resume state consumes evidence rather than accepting a no-argument
  “barrier completed” call.
- Synced records persist the evidence's file count and write-sequence
  watermark; legacy records decode with zero for those new fields.
- Fast/unsupported saves explicitly remain unsynced and carry no current
  barrier evidence.
- A storage barrier failure leaves the state unclean and does not clear the
  engine's dirty-piece watermark.
- Regression sources cover real storage evidence, fast-mode state, and forced
  completion barriers.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs`,
  `crates/rt-fastresume/src/state.rs`, `crates/rt-fastresume/src/store.rs`, and
  `crates/rt-engine/src/torrent_task.rs`.
- **State or wire compatibility:** additive JSON fields use serde defaults;
  `FASTRESUME_VERSION` and torrent protocol formats do not change.
- **Migration / rollback:** existing resume records remain readable; a rollback
  must retain the `synced` safety flag and legacy decode defaults.
- **Feature flag or rollout boundary:** none; durability modes are unchanged.

### Evidence

- **Regression source added or updated:** storage creates evidence only after a
  successful barrier; fast-resume consumes it and persists the diagnostic
  watermarks. `torrent_task.rs` also has a deterministic `power_cut_walk`
  model: 12 seeds each exercise checkpointed and fast-mode saves across 40
  receive/save/idle steps, rebuild the payload from its durable image, and
  assert that host-crash recovery never trusts missing bytes. A negative
  control confirms the model detects unsafe state when crash protection is
  disabled. The model and storage regressions passed at exact source
  `354ad504`; physical crash-cut qualification remains open.
- **Command or probe to run:** storage barrier-ordering and fast-resume
  durability suites during the bulk pass.
- **Expected result:** no save claims stable payload data without an opaque
  successful barrier proof, while legacy fast-resume records remain readable.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** crash-cut qualification on supported
  filesystems. SYS-009 separately qualifies the `DbWorker` actor path; no
  distributed transaction spans SQLite and the fastresume file.

## IO-011 — Reuse-driven peer-read readahead

- **Priority / wave:** P1 / 9
- **Prerequisites:** IO-004, IO-006, IO-008
- **Owning component:** `rt-storage::MountScheduler` peer-read path.
- **Finding or capability:** F18; speculative reads should follow observed
  sequential demand and remain within admitted memory/queue budgets.
- **Status:** `wip`

### Scope

Sequential-only readahead now starts at an exact first read and bootstraps one
prefetch on the first continuation. Later growth is driven by cache hits and is
gradual; a miss holds the current recommendation rather than expanding
speculative I/O. Recommendations remain bounded by the configured maximum, and
a discontinuity resets to the requested range. Stream history is bounded with
single-entry least-recently-used eviction, so admitting a new file does not
clear every warm stream. The cache also retains multiple independently useful
forward ranges per file up to the configured total entry cap, evicting the
least-recently reused range. Live-engine qualification remains open.

### Ownership and invariant

- **Current owner:** `MountScheduler` owns bounded stream history, per-file
  range entries, and per-read admission; the portable backend owns the
  submitted read.
- **Target owner:** IO-006 device service owns speculative capacity and IO-008
  actor progress consumes completed reads without waiting on unrelated work.
- **Invariant preserved:** readahead never exceeds the configured cap or file
  extent, and its additional queued-byte charge plus frame reservation are
  acquired before the backend read is submitted.
- **Must not:** expand an isolated seek or discard a requested range when
  speculative admission is denied.

### Acceptance

- The first read and any nonsequential jump remain exact when sequential-only
  mode is enabled.
- Repeated continuation grows only to the configured maximum; per-file
  histories remain bounded.
- Cache hits may grow the recommendation gradually; misses do not expand the
  speculative range.
- A file may retain multiple disjoint useful ranges; total range entries stay
  within the configured cap and least-recently-used ranges are evicted first.
- Replacing a path or completing a write invalidates every cached range for
  that path.
- Added buffer bytes are charged before allocation/submission and released only
  after completion.
- Toggle-off preserves the existing unconditional configured range behavior.
- Regression sources cover one-block start, growth, discontinuity reset,
  single-entry stream-history eviction, cache behavior, and queue/frame
  pressure.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/scheduler.rs` and storage
  configuration docs.
- **State or wire compatibility:** no persistent or wire-format change.
- **Migration / rollback:** the existing `peer_read_readahead_sequential_only`
  setting can disable sequential gating.
- **Feature flag or rollout boundary:** enabled by default through storage
  configuration; performance claims wait for PERF-109 and IO-016.

### Evidence

- **Regression source added or updated:** `readahead_expands_only_continuing_streams`
  covers bounded per-file growth and discontinuity reset;
  `readahead_stops_growing_when_prefetch_is_not_reused` covers hit/miss
  feedback; `peer_read_readahead_cache_retains_disjoint_ranges_for_one_file`
  covers multi-range reuse; scheduler cache regressions remain in the bulk
  pass.
- **Command or probe:** exact-main workspace tests on Rust 1.88.0 and 1.94.0;
  release `rt-storage::storage_real_device` suite on `kspls0` ext4 LVM.
- **Expected result:** exact reads for isolated requests, bounded growth for
  continuous streams, and no speculative budget leak.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** workspace logs are under
  `/tmp/tng-transfer-core-close-20261018/evidence/`; the release real-device
  log is `hardware-kspls0-eb5f3158-01/storage_real_device_release.log` in that
  evidence tree.
- **External evidence still required:** filesystem/backend read amplification
  and throughput under realistic peer request patterns.

## IO-012 — Chunked v2 file-root recheck

- **Priority / wave:** P1 / 6
- **Prerequisites:** IO-006, IO-007
- **Owning component:** `rt-storage::V2FileVerifier` and the shared hash pool.
- **Finding or capability:** v2 recheck must verify large and sparse files
  without one small I/O or hash-pool round trip per 16 KiB leaf.
- **Status:** `wip`

### Scope and invariant

Read BEP 52 file content as a Merkle stream of 16 KiB leaves. The verifier
uses chunks of 64 leaves (1 MiB), batches leaf hashing through the shared hash
pool, and keeps a bounded page of sparse extents across sequential chunks.
Sparse holes hash as zeroes; extent pages advance from an exclusive scan
offset, and a final tail read detects truncation during a scan. Hybrid disk
recheck uses IO-007's dual-view stream verifier to compute both protocol views
from one read stream.

- **Invariant preserved:** a file is valid only when its exact BEP 52 root
  matches metainfo; missing/truncated data is never silently zero-filled.
- **Must not:** materialize an entire large file, reread sparse-extent pages
  for each chunk, or include BEP 47 padding in a file root.

### Acceptance

- Hash 64 leaves per bounded job and preserve Merkle layer/root semantics for
  full chunks and final partial chunks.
- Reuse no more than 64 sparse extents per page across sequential chunks;
  advance the cursor monotonically and detect a truncated tail.
- Sparse holes produce the same root as their logical zero bytes, while a
  missing file, invalid root, or truncated file remains incomplete.
- Hybrid disk verification reads each needed range once and checks both v1
  and authenticated v2 views, excluding padding from the v2 root.
- Regression sources cover batched leaf jobs, persistent cursor reuse, sparse
  holes, truncation, empty files, invalid roots, and hybrid dual-view reads.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/verify.rs`,
  `rt-storage/src/scheduler.rs`, and the hybrid verifier integration.
- **State or wire compatibility:** verification output is unchanged; work is
  internally chunked.
- **Migration / rollback:** no state migration; verifier can retain the
  existing bounded positioned-I/O backend.
- **Feature flag or rollout boundary:** hardware/read-amplification claims
  remain gated by IO-016 and PERF-110.

### Evidence

- **Regression source added or updated:** `v2_file_verify_batches_leaf_hashing_per_chunk`,
  `v2_sparse_extent_cursor_reuses_pages_across_hash_chunks`, sparse-hole and
  truncation cases, and hybrid dual-view verifier sources are present.
- **Command or probe to run:** storage verification and engine restart suites
  during the requested bulk pass.
- **Expected result:** correct roots with bounded chunk and extent-page work;
  truncated data fails verification.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** target-filesystem read amplification,
  CPU, and recheck throughput.

## IO-013 — Persist authenticated v2 Merkle proof nodes

- **Priority / wave:** P1 / 11
- **Prerequisites:** IO-007, IO-010, IO-012
- **Owning component:** pure-v2 proof serving in `torrent_task_v2.rs`.
- **Finding or capability:** a BEP 52 hash request should not reread and hash
  a whole file after every restart when authenticated nodes are available.
- **Status:** `wip`

### Scope and invariant

Build a Merkle tree only from payload verified against the metainfo root.
Keep up to 64 MiB of governor-charged proof layers in the in-memory LRU and a
rebuildable per-torrent disk store capped at 512 MiB and 4,096 entries. Disk
entries use a versioned header binding torrent identity, file root, length,
and tree shape. Writes stage a unique no-follow temporary file, sync it, and
install it by no-follow rename. Warm requests read the requested layer range
and bounded authentication path, validate the header/shape, and authenticate
the returned hashes to the metainfo root. Cold misses rebuild from content;
oversized entries are skipped rather than weakening the cap.

- **Invariant preserved:** persisted data is only a cache; no proof is served
  unless its tree and requested path authenticate to the metainfo root.
- **Must not:** trust a file because its name matches, follow attacker-created
  symlinks, or make proof-cache durability a payload-completion condition.

### Acceptance

- Reject malformed headers, wrong torrent/root/length/tree shape, truncation,
  and any returned range whose authentication path does not reach the root.
- Enforce the memory and disk bounds per torrent; remove stale entries safely
  and leave oversized trees rebuildable after restart.
- A crash before rename leaves only a disposable temporary entry; restart can
  ignore it and rebuild from verified content.
- Charge retained trees and response buffers to their owning memory classes.
- Regression sources cover cold build, warm range read, restart reuse, tampered
  node rejection, cap eviction, and temporary-file cleanup.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/torrent_task_v2.rs` and
  `rt-storage` no-follow filesystem helpers.
- **State or wire compatibility:** cache format is versioned and rebuildable;
  BEP 52 wire replies are unchanged.
- **Migration / rollback:** remove or ignore cache entries to fall back to
  content rehashing.
- **Feature flag or rollout boundary:** proof serving remains on the existing
  pure-v2 peer path; durability/corruption qualification remains required.

### Evidence

- **Regression source added or updated:** cold-cache round trip,
  authenticated warm range, tampered-node rejection, size-cap, and
  no-follow/temporary-file sources are present.
- **Command or probe to run:** pure-v2 proof and restart suites during the
  requested bulk pass.
- **Expected result:** warm proofs match cold proofs, tampering is rejected,
  and cache cleanup preserves the size bounds.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** restart-heavy swarm workloads and
  storage-device qualification.

## IO-014 — Schedule secure copies and verify each leaf once

- **Priority / wave:** P1 / 10
- **Prerequisites:** IO-006, IO-009, IO-012
- **Owning component:** durable storage-plan worker and `rt-storage` secure
  filesystem executor.
- **Finding or capability:** secure copies must retain bounded admission while
  preserving descriptor-anchored paths and one content verification per leaf.
- **Status:** `wip`

### Scope

`CopyVerifyRename` and `ImportExisting` plan steps dispatch through the
destination mount's shared `DeviceQueue` in the `MoveCopy` lane. Source and
destination paths are resolved to physical-device queues and admitted in
stable key order, covering source reads, staging writes, final renames, source
deletion, and rollback. The worker passes the same `ResourceGovernor` and
storage-I/O settings used by torrent actors. Checkpoint persistence stays
outside device permits. The durable plan worker owns cursor state, while the
scheduler owns each blocking quantum and its in-quantum cleanup through
completion. The charged
working set is one 64 KiB streaming buffer, not the payload length. Regular
file `CopyVerifyRename` work is split into copy, source-hash, and destination-
hash quanta; each quantum reacquires all touched device queues before it does
I/O, then yields before the next admission. The physical queue set is
resolved once for the plan step and reused rather than rediscovering mount
topology for every chunk. The cursor keeps only confined descriptors and hash
state between quanta. A failed or cancelled quantum
closes its handles and removes the partial destination while device admission
is still held. If the scheduler rejects further admission, the retained cursor
uses its root-confined descriptors for terminal cleanup before returning the
admission error. F25's nested-entry copy-only behavior still verifies each
copied leaf once per side.

When physical-device IDs are unavailable, all copy jobs through the engine's
storage dispatcher share its stable fallback queue. This conservatively bounds
unknown devices without assuming two paths are independent.

Directory copy and verification now use resumable cursors on Unix and Windows.
They stream directory entries, advance one entry or one bounded file-copy,
source-hash, or destination-hash quantum per device admission, and compare
directory contents without materializing per-directory name vectors. Unix
recursive deletion retains at most 256 names per batch and restarts enumeration
after each batch, so removing entries cannot invalidate an active directory
cursor. Unix same-filesystem `ImportExisting` retains its one-operation
hard-link fast path; Windows imports use bounded quanta for regular-file copies.
An individual directory entry operation and each file/directory sync are still
non-preemptible within their admitted quantum; their latency and platform
behavior require qualification.

### Ownership and invariant

- **Current owner:** the durable storage worker checkpoints plan steps; the
  destination mount scheduler owns each quantum's queued bytes and the
  source/destination device permits until that blocking quantum and its local
  cleanup return. The cursor retains root-confined descriptors and hash state
  between admissions.
- **Target owner:** one bounded storage service accounts for every physical
  device touched by a copy while verification uses the shared hash service.
- **Invariant preserved:** root-confined descriptor operations, rollback on
  interrupted staging, and once-per-leaf content verification remain intact.
- **Must not:** charge the full file length as retained memory or release
  scheduler admission while a blocking copy can still mutate the destination.

### Acceptance

- Secure copy/import steps wait behind and share the configured destination
  device queue across mounts.
- Queued working-set bytes and the device permit remain owned until an admitted
  quantum and its cleanup finish. Cancellation between quanta is handled by
  the next admitted quantum before its permit is released.
- Regular-file copies and both verification reads process
  no more than 64 KiB per device admission, then release permits before the
  next quantum; one-block-per-quantum and cancellation cleanup regressions
  cover Unix and Windows.
- Directory traversal and per-file copy/hash work yield between bounded
  quanta. Individual directory-entry operations and file/directory syncs stay
  within one admitted quantum; Unix same-filesystem `ImportExisting` retains
  the single-operation hard-link fast path.
- Source reads, destination writes, renames, deletes, and rollback mutations
  acquire admission for every touched physical device in a global order.
- Opposing cross-device copies cannot deadlock while holding device permits.
- Nested directory copies hash each leaf once on each side and reject
  same-length corruption.
- Verification streams directory entries without a per-directory name vector;
  recursive deletion retains at most the fixed batch size.
- Regression sources cover scheduler contention, cancellation cleanup,
  cross-device admission, and once-per-leaf hashing.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/plan.rs`,
  `crates/rt-storage/src/secure_fs.rs`,
  `crates/rt-storage/src/windows_secure_fs.rs`,
  `crates/rt-storage/src/scheduler.rs`,
  `crates/rt-engine/src/storage_jobs.rs`, and engine scheduler composition.
- **State or wire compatibility:** no resume, storage-path, or peer-wire
  format change.
- **Migration / rollback:** the direct storage-plan path remains available to
  non-engine callers; production plan copies use the configured device queue.
- **Feature flag or rollout boundary:** no performance claim until bulk
  correctness and storage measurements pass.

### Evidence

- **Regression source added or updated:**
  `secure_copy_plan_waits_for_mount_device_admission` proves a queued plan does
  not create its destination while the mount queue is full;
  `external_device_handoff_shares_admission_across_mounts` covers shared device
  admission; `multi_device_handoff_holds_source_and_destination_until_completion`
  checks both physical permits remain held. The existing secure-copy hash-count
  and cleanup sources remain in place; `directory_name_stream_visits_all_entries`
  and `directory_deletion_batches_bound_retained_names` cover the streaming
  iterator and bounded Unix deletion batch;
  `regular_file_copy_cursor_advances_in_bounded_quanta` and
  `regular_file_copy_cursor_cancellation_removes_partial_destination` cover
  bounded file work and cancellation cleanup on Unix and Windows;
  `directory_copy_cursor_preempts_and_verifies_streamed_tree` and
  `directory_copy_cursor_cancellation_removes_partial_tree` cover tree
  preemption, content verification, and cancellation cleanup on Unix and
  Windows;
  `regular_file_import_keeps_the_hard_link_fast_path` protects the Unix
  same-filesystem import optimization;
  `streamed_tree_verification_rejects_destination_only_entries` preserves
  exact directory-content matching without a name set on Unix, and
  `windows_secure_fs::tests::streamed_directory_verification_rejects_destination_only_entries`
  covers the Windows handle-relative path.
- **Command or probe:** storage plan, scheduler admission, cancellation,
  secure-copy hash-count, and injected-backend suites; these passed in the
  exact-source bulk run recorded above.
- **Expected result:** no plan step begins before all touched device queues
  admit it, no permit or byte charge is released while accepted work remains
  active, directory verification/deletion retain bounded entry state, and
  rollback/reconciliation report uncertain filesystem state explicitly.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** matched cross-device throughput and
  memory results under realistic concurrent peer I/O.

## PERF-101 — Trial measurement contract and schemas

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** certification tooling and evidence format.
- **Finding or capability:** reproducible trials tied to immutable source,
  executable, fixture, settings, and environment identity.
- **Status:** `done`

### Scope

Define trial timing, byte accounting, result outcomes, provenance, strict
trial/manifest records, and schema validation. This does not implement client
adapters, fixture controls, paired-run analysis, or adopt the proposed
scorecard thresholds.

### Ownership and invariant

- **Current owner:** certification scripts and report authors.
- **Target owner:** a shared harness that emits immutable manifest-linked
  evidence.
- **Invariant preserved:** incomplete, invalid, failed, and unmeasured attempts
  remain visible; only complete trials with applicable passing integrity may
  enter scorecards.
- **Must not:** infer identity from a filename or treat missing counters as
  zero.

### Acceptance

- Contract separates barrier, transfer, flush, validation, and shutdown time.
- Useful payload excludes duplicates, protocol overhead, and padding.
- Schemas reject malformed identities and invalid complete-trial combinations.
- The client-observation schema pins runtime identity and redacted settings.
- The pinned validator checks trial, manifest, and client-observation
  schemas; manifest validation verifies non-symlink artifact/trial digests,
  source identity, and linked client/version/settings/process identity.
- Every external JSON artifact and referenced JSON record is parsed with
  duplicate-field, nonstandard-constant, and non-finite-number rejection
  before schema or semantic validation.
- Missing or unattributable platform counters remain explicit `null` values.
- Scorecard thresholds remain proposed until separately adopted.

### Files and compatibility

- **Expected code paths:** `certification/transfer-rebuild/` schemas and
  `measurement-contract.md`, plus `scripts/transfer_rebuild/` validation.
- **State or wire compatibility:** new evidence format `transfer-trial-v1` and
  `transfer-manifest-v1`; no product state change.
- **Migration / rollback:** schema versioning is additive; old reports remain
  historical evidence.
- **Feature flag or rollout boundary:** no benchmark claims until PERF-104 and
  PERF-113 build on this contract.

### Evidence

- **Regression source added or updated:** complete-trial rules, malformed
  hashes, unknown fields, manifest digest shape, and referenced-file/provenance
  checks in `scripts/test_transfer_rebuild_validator.py`; duplicate-field,
  non-finite numeric, and oversized-integer JSON inputs are rejected before
  schema validation. The shared strict decoder also covers catalog-load logs,
  rollout evidence, and paired-runner output.
- **Command or probe to run:** schema validation and contract fixtures during
  the requested bulk pass; install the pinned helper dependency first.
- **Expected result:** invalid types, hashes, and complete-trial omissions are
  rejected.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** exact-main Python regression log is under `/tmp/tng-transfer-core-close-20261018/evidence/`; no performance result is produced by this contract.
- **External evidence still required:** no performance evidence is produced by
  this contract.

## PERF-102 — Independent resource observer

- **Priority / wave:** P0 / 3
- **Prerequisites:** PERF-101
- **Owning component:** Linux certification tooling.
- **Finding or capability:** independent cumulative process-tree and cgroup-v2
  resource observation.
- **Status:** `wip`

### Scope

Sample process-tree RSS/PSS, cgroup memory, CPU, threads, descriptors, and
`/proc/<pid>/io` counters without retaining the sample series in memory.
For an isolated one-client network namespace, read the named interface from
`/proc/<pid>/net/dev`, retaining namespace identity and reporting byte deltas.
The versioned summary binds its CSV digest and root process start identity;
manifest validation checks trial totals against the summary and recomputes
values from the CSV. Fixture control and paired runner integration remain open.

### Ownership and invariant

- **Current owner:** external observer process; it does not control the client.
- **Target owner:** PERF-102 observer, consumed by PERF-104 trial adapters.
- **Invariant preserved:** RSS/PSS, cgroup anonymous/file/current, and process
  I/O remain separate accounting domains; `inactive_file` is kept distinct as
  reclaimable cache, and unavailable values stay null.
- **Must not:** signal or restart the measured process, or present
  shared-namespace interface bytes as traffic from one client.

### Acceptance

- Descendant discovery covers children spawned by any process thread.
- Cumulative samples are streamed to CSV; peak summary has constant memory.
- CSV and summary artifacts are created exclusively with private permissions;
  rerunning an observer cannot replace prior raw evidence.
- Missing process counters do not become partial undercounts.
- A linked observation contains at least one sample; every sample has a live
  process tree and strictly increasing monotonic elapsed time within the
  summary's start/finish interval.
- New manifests require a linked observer summary for every complete trial;
  validation verifies the root PID/start-time identity, CSV digest, row count,
  and resource totals. Historical trial-v1 records without this link remain
  readable but are excluded from scorecards. The observer checks the same PID
  start time before and after each sample so PID reuse cannot extend the
  observation into another process instance.
- Optional network counters come from one named interface in the root
  process's network namespace, not from a sum of process counters. The
  namespace inode must remain stable and counters must be monotonic across at
  least two samples; otherwise network totals stay null.
- The summary repeats that the interface counters include all namespace
  traffic, and records when network-byte observation was not requested.
- The interface is eligible as client traffic only when the runner isolates
  one client in that private namespace and links the network-control evidence.
- Observer exits clearly when the process was never observable.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/observe.py`,
  `validate_artifact.py`, the resource-observation schema, trial schema,
  observer README, and measurement contract.
- **State or wire compatibility:** adds the
  `transfer-resource-observation-v1` certification artifact; existing
  trial-v1 records remain readable.
- **Migration / rollback:** historical complete trials without observer
  provenance remain valid records but cannot enter a scorecard.
- **Feature flag or rollout boundary:** Linux process and cgroup v2 only;
  observer does not claim support for other platforms.

### Evidence

- **Regression source added or updated:** peak/null-counter fixtures and
  synthetic process-tree descendants, including a non-leader thread child;
  named-interface parsing, namespace identity, counter deltas, and reset/null
  behavior; unavailable-sample peak invalidation; observer CSV digest, schema,
  process identity/reuse fencing, and manifest-to-summary/CSV reconciliation.
- **Command or probe to run:** `python3 scripts/test_transfer_rebuild_observer.py`,
  `python3 scripts/test_transfer_rebuild_validator.py`, and
  `python3 scripts/test_transfer_rebuild_scorecard.py` during the requested
  bulk pass.
- **Expected result:** cumulative counters and process-tree attribution match
  synthetic fixtures without retaining samples; per-interface network deltas
  are scoped to the observed namespace and fail closed on missing/reset data.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** Linux cgroup-v2 permission and overhead
  characterization on the dedicated qualification host.

## PERF-103 — Pinned client adapters and settings readback

- **Priority / wave:** P0 / 3
- **Prerequisites:** PERF-101
- **Owning component:** `scripts/transfer_rebuild/adapters.py` and the client
  observation schema.
- **Finding or capability:** every paired trial identifies the client process
  and reads back the settings declared by its profile.
- **Status:** `wip`

### Scope

Capture version and effective settings for TorrentNG, qBittorrent,
Transmission, Deluge, rTorrent, rqbit, and a direct `urtorrent` library runner.
Pin container clients by immutable
image digest and runtime container/process identity; pin native clients by
binary digest, executable inode, and process start time. Also pin the source
commit, dirty state, lockfile, and toolchain when a source checkout is
available. Redact credentials before hashing or writing observations. Bind
every trial to its observation through the manifest.

rqbit's API exposes server identity and status, not resolved settings. Its
adapter pins its running command line, an allowlist of environment settings,
and declared configuration-file digests exactly; it does not claim to observe
hidden defaults. The full paired runner and settings-surface review remain
open.

The `urtorrent` comparator is a separate Linux-only binary pinned to library
0.14.3 and Rust 1.98.1; it is not a TorrentNG production dependency. It records
`Session::settings()` and per-torrent status against the live process identity.
The API does not expose the configured hash-pool thread count, so the snapshot
labels that value as a command-line input instead of claiming effective
readback. The runner requires private v1 metainfo, rejects v2/hybrid fixtures,
binds to one private, link-local, or loopback IPv4 interface, removes tracker
and web-seed endpoints before release, and dials one explicit private,
link-local, or loopback IPv4 peer. IPv6, DHT, PEX, and LSD are disabled, and
the adapter independently checks both addresses from the live process-bound
snapshot. Its control loop stays inside the measured process. Its
barrier-to-trial-end duration is diagnostic and does not replace the
measurement contract's `transfer_ns`.
This comparator covers v1 only; it cannot decide whether the native core should
be kept, refactored, or replaced until the broader paired comparison is
implemented and qualified.

### Ownership and invariant

- **Current owner:** the read-only adapter captures identity/settings; the
  manifest validator binds observations to trial records.
- **Target owner:** PERF-104 consumes a verified observation before a trial.
- **Invariant preserved:** the executable or image and API endpoint belong to
  the observed process/container; expected version/settings match before an
  artifact is emitted.
- **Must not:** store credentials, accept mutable image tags, or treat missing
  readback as a successful observation.

### Acceptance

- Pin profiles require expected version, expected settings, and immutable
  runtime identity pins.
- Native HTTP APIs are loopback-only and owned by the pinned process; container
  bridge mappings must bind only to loopback and map into the pinned container's
  network namespace. Host-network containers must expose the API only on a
  loopback listener.
- Failed login, unknown version, settings mismatch, unpinned identity, or an
  unobservable API endpoint returns an error without writing an artifact.
- The observation is created once with restrictive permissions and cannot
  overwrite an earlier record.
- Trial and manifest validation bind the observation digest, client/version,
  settings digest, runtime kind, and process start identity.
- The `urtorrent` profile requires the pinned native runner, an absolute live
  settings snapshot, the Linux task namespace, private metainfo, discovery
  disabled, an explicit private IPv4 listener and peer, and v1-only geometry;
  unobservable hash-thread count is labeled as a launch input. The runner
  rejects public peer addresses, duplicate CLI options, colliding control
  paths, existing start-gate files, and v2/hybrid metainfo.
- Regression sources cover auth text responses, credential redaction,
  endpoint/process binding, schema linkage, urtorrent snapshot/process binding,
  IPv4 listener and peer policy, fixture-isolation rejection, and mismatch
  rejection.

### Files and compatibility

- **Expected code paths:** adapter source, client-observation and trial
  schemas, validator, measurement contract, harness README, regressions, and
  the standalone `urtorrent_runner` crate with its own lockfile.
- **State or wire compatibility:** versioned certification artifacts only;
  `client_identity` is required in the current `transfer-trial-v1` draft.
- **Migration / rollback:** no trial evidence has been qualified on this branch.
- **Feature flag or rollout boundary:** no comparison is valid until each
  client has a reviewed pin profile and the exact revision passes bulk
  validation.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_adapters.py` (including runner snapshot
  identity and isolation binding), the runner's parser and local-peer policy
  unit cases, and `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** build and test the runner in release mode with
  `cargo +1.98.1 build --release --locked --manifest-path
  scripts/transfer_rebuild/urtorrent_runner/Cargo.toml` and the corresponding
  `cargo +1.98.1 test --release --locked` command; the hosted qualification
  workflow now runs the runner format check, both release commands, and a CLI
  help smoke check, followed by the adapter and validator regressions.
- **Expected result:** only identity-bound, version-matched, settings-matched
  readbacks produce an observation; mismatches are rejected.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** all seven client pin profiles, API
  surface review for locked versions, Linux io_uring and namespace
  qualification for urtorrent, and local process/container qualification.

## SYS-004 — Compact generation-checked tier identities

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** `rt-engine::TierController` and engine runtime maps.
- **Finding or capability:** F14/F27; compact metadata identities and honest
  whole-owner memory accounting.
- **Status:** `wip`

### Scope

Replace repeated string keys inside high-cardinality runtime owners with
compact identities while retaining the canonical info hash at string-facing
boundaries. `TierController` uses owner-local slot/generation IDs: its tier
maps and ordered deadline indexes use an 8-byte ID, while one interned `Arc<str>` identity
supports lookups. `EngineRuntimeState` channel, task, activity, and pending-
promotion maps, pending-add/delete sets, and supervised task-exit events use
inline decoded keys for canonical v1/v2 hashes; the map and set adapters keep
`&str` lookup boundaries, and noncanonical placeholder keys retain exact
text. No database, API, or wire representation changes.

### Ownership and invariant

- **Current owner:** `TierController` retains the canonical string in its
  interner and uses compact IDs in tier maps, timer indexes, and dormant
  projections.
- **Target owner:** an owner-local key interner retains the canonical hash;
  tier maps and timers carry only `(slot, generation)`.
- **Invariant preserved:** removing an identity cancels its exact deadline
  nodes before the slot can be reused; generation checks keep a recycled slot
  distinct from the removed identity. Tier counts and dormant snapshot
  behavior remain unchanged.
- **Must not:** persist the runtime ID, change hash canonicalization, or let a
  stale timer act on a newly added torrent.

### Acceptance

- Tier and timer indexes use an 8-byte slot/generation ID; the key index and
  slot share one interned string allocation.
- Slot reuse increments generation. A generation that would overflow retires
  that slot instead of wrapping and aliasing an old ID.
- Timer cancellation removes the exact ordered-index node; no canceled timer
  leaves a stale node or can target an ID that reuses the old slot.
- Whole-owner estimates include retained map capacities, slot/free-list
  capacities, ordered timer-index nodes, interned string bytes, and separately
  owned dormant snapshot strings.
- Regression sources cover generation-safe slot reuse, dormant projection
  accounting, and whole-owner memory accounting.
- Task-exit IDs, the four `EngineRuntimeState` maps, and the pending add/delete
  sets keep canonical hashes inline and preserve their existing string-facing
  lookup behavior.
- Compact map/set keys do not by themselves prove a whole-process memory or
  throughput improvement; matched-client and whole-owner memory evidence is
  still required.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/tier.rs`,
  `subsystems.rs`, engine lifecycle maps, and the rebuild ledger.
- **State or wire compatibility:** no persistence, API, or protocol format
  changes; public dormant snapshot hash remains a string.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** no flag; qualify with the bulk pass.

### Evidence

- **Regression source added or updated:**
  `reused_tier_identity_has_new_generation_and_removes_old_deadline`,
  `dormant_snapshot_is_accounted_and_cleared_on_promotion`, and
  `owner_bytes_cover_maps_and_timers_not_just_the_hash_string`, plus
  `compact_exit_ids_roundtrip_v1_v2_and_keep_non_hash_test_keys` and
  `runtime_torrent_maps_keep_canonical_keys_inline_and_accept_hash_strings`
  and `runtime_torrent_sets_keep_canonical_keys_inline_and_accept_hash_strings`.
- **Command or probe to run:** `cargo test -p rt-engine tier::tests` during
  the requested bulk pass.
- **Expected result:** a removed timer never fires for a reused slot and the
  owner estimate includes retained tier, snapshot, interner, and timer storage.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** PERF-109/SYS-014 catalog and mixed-load
  qualification.

## SYS-005 — Publish registry deltas and share immutable API rows

- **Priority / wave:** P1 / 3
- **Prerequisites:** SYS-004
- **Owning component:** `rt-session::SessionRegistry` and native API snapshot
  projection.
- **Finding or capability:** F26; updates should not clone a full registry
  journal or copy unrelated API row payloads.
- **Status:** `wip`

### Scope and invariant

Publish bounded, coalesced registry revision pages and apply them to cached
native API snapshots. Canonical v1/v2 hashes use fixed-size byte keys in the
snapshot hash index and change set. `ChunkedVec` shares immutable row owners
across snapshot revisions, including when copy-on-write duplicates the small
pointer array for a changed row's chunk. Structural add/remove changes rebuild
the positional projection from the deterministic registry snapshot. Dormant
torrents retain the registry's compact immutable projection until mutation
promotes them. With tiering enabled, fresh paused/stopped/queued additions use
that same taskless path; metainfo and piece-index reservations are reacquired
only when a lifecycle command promotes the row. Disabling tiering keeps the
configured actor behavior.

- **Invariant preserved:** every published snapshot is internally consistent
  at its revision; an incomplete or gapped delta page falls back to a full
  deterministic rebuild; noncanonical fixture keys retain exact string
  matching; public API fields and ordering are unchanged.
- **Must not:** report an efficiency win based solely on compact keys or shared
  rows. Per-row shared ownership has a fixed allocation cost; catalog and
  matched-client evidence must measure its tradeoff.

### Acceptance

- Delta reads remain capped and coalesce repeated changes per hash.
- Canonical v1/v2 snapshot indexes and delta sets use inline 20/32-byte keys;
  noncanonical values preserve exact text behavior.
- Updating one existing row allocates a replacement row while unchanged row
  payload allocations remain shared across cached snapshot revisions.
- Add/remove changes and incomplete delta history rebuild all positional
  indexes from the registry snapshot.
- Regression sources cover compact key behavior, shared unchanged row identity,
  incremental filter-index consistency, and adding a paused torrent without a
  task before verifying resume promotion reacquires runtime memory.
- Bulk scale and matched-client evidence remain required before claiming a
  whole-process memory or throughput improvement.

### Files and compatibility

- **Expected code paths:** `crates/rt-session/src/registry.rs`,
  `crates/rt-api-model/src/snapshot.rs`, and
  `crates/rt-api-native/src/state.rs`.
- **State or wire compatibility:** no persisted or API representation change.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** snapshot implementation remains behind
  the existing API cache; source regressions and 1k/100k catalog probes passed,
  while whole-client qualification remains open.

### Evidence

- **Regression source added or updated:** `replacing_one_item_shares_unchanged_rows_in_the_copied_chunk` and
  `snapshot_hash_keys_compact_canonical_hashes_and_preserve_other_text`, plus
  the existing registry delta-page and incremental filter-index regressions.
- **Command or probe to run:** API model, API native, engine, and session
  registry suites during the requested bulk pass; the 1k/100k live catalog
  probes exercise the persisted add/restore path.
- **Expected result:** unchanged row pointers remain shared in changed chunks,
  and API snapshots match a full rebuild at the same registry revision.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** SYS-014's validated 1k/100k report, raw log,
  manifest, and qualification are retained under the task-scoped evidence
  directory; matched whole-client comparisons remain open.
- **External evidence still required:** matched whole-client resource and
  throughput comparisons.

## SYS-006 — Bounded API pages and revision deltas

- **Priority / wave:** P1 / 4
- **Prerequisites:** SYS-005
- **Owning component:** native API snapshot projection, SSE handler, and
  qBittorrent sync facade.
- **Finding or capability:** list and change-feed endpoints must stay bounded
  while clients page through one consistent catalog revision.
- **Status:** `wip`

### Scope and invariant

Native torrent summaries are served from cached immutable snapshots, with
revision-pinned pagination, indexed filters/sorts, a capped response count,
and an estimated response-byte reservation before materializing the page.
SSE advances through bounded registry-delta pages and requests a fresh snapshot
when its cursor expires. qBittorrent `sync/maindata` projects the same bounded
registry journal and uses an opaque RID that distinguishes the empty revision
from the first mutation.

- **Invariant preserved:** a page belongs to one registry revision; a delta
  cursor advances only through rows included in that page; expired history
  becomes an explicit restart/full-snapshot path.
- **Must not:** allocate directly from an untrusted requested limit, return
  mixed-revision pages, or interpret RID zero as a valid returned revision.

### Acceptance

- Native pages clamp `limit` to 1–5,000, keep a stable snapshot cursor, and
  reserve the estimated response before cloning summaries.
- SSE emits at most 1,000 changed/removed rows per page and advances through
  `through_revision`; stale history resets the stream to a snapshot.
- qBittorrent deltas cap each page and encode revision `r` as `r + 1` within
  the supported integer range, so clients can distinguish initial state from
  revision one.
- Regression sources cover expired snapshots, pinned pages, delta continuation,
  empty revision versus first mutation, and bounded response estimates.

### Files and compatibility

- **Expected code paths:** `crates/rt-api-native/src/state.rs`,
  `rt-api-native/src/handlers.rs`, `rt-api-qbit/src/handlers.rs`, and
  `rt-session/src/registry.rs`.
- **State or wire compatibility:** additive page cursors and existing response
  fields remain compatible; qBittorrent RID remains opaque to clients.
- **Migration / rollback:** no persisted-state migration; handlers may fall
  back to full snapshot semantics when journal history is unavailable.
- **Feature flag or rollout boundary:** response budgets and 100k-catalog
  responsiveness require SYS-014 qualification.

### Evidence

- **Regression source added or updated:** native snapshot/SSE page tests and
  qBittorrent continuation, RID, and response-size tests are present.
- **Command or probe to run:** API native, API qBittorrent, and session suites
  during the requested bulk pass.
- **Expected result:** page sizes stay within bounds and every continuation
  either advances without a gap or explicitly requests a fresh snapshot.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** matched client load and mixed-transfer
  responsiveness.

## SYS-008 — Bounded timer maintenance and bulk-operation pacing

- **Priority / wave:** P1 / 3
- **Prerequisites:** SYS-004
- **Owning component:** `rt-engine` tier maintenance and the native API bulk
  action handler.
- **Status:** `done`

### Scope and invariant

Keep cancellation and due-timer maintenance bounded as the catalog grows, and
avoid turning a large API bulk request into either a serial tail or an
unbounded burst. An ordered `(deadline, sequence)` index removes canceled
timers in O(log N), preserves FIFO order for equal deadlines, and pops at most
the caller's per-tick limit without scanning tombstones. Bulk idle checks are
staggered. Independent native bulk stop,
recheck, label, and location operations run with at most 32 actions in flight,
while start and reannounce remain ordered and serial because their lifecycle
and fallback semantics are order-sensitive.

The bulk response preserves the original selected-hash order and reports each
selected hash exactly once as applied or failed. A failed operation does not
cancel unrelated operations in the same request.

### Regression and remaining evidence

- **Regression source:** `bulk_operation_runner_bounds_concurrency_and_preserves_input_order`
  checks the fixed concurrency ceiling, overlap, and ordered result accounting;
  deadline-index regressions cover exact cancellation, equal-deadline order,
  bounded due-pop after large cancellation cohorts, generation-safe slot reuse,
  and staggered idle deadlines.
- **Bulk evidence:** native API and engine tier suites passed at exact source
  `354ad504`. Catalog-scale and mixed-load evidence is recorded under SYS-014.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.

## SYS-009 — Production database worker actor coverage

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** `rt-engine` `DbExecutor` and `TorrentTask` progress
  persistence.
- **Finding or capability:** F30; engine actor tests should cover the same
  ordered, supervised SQLite boundary used in production.
- **Status:** `done`

### Scope

Keep fast in-memory direct executors for most state-machine fixtures, while
adding a file-backed actor regression that opts into the production
`DbExecutor::Worker` path for coalesced transfer-stat persistence.

### Ownership and invariant

- **Current owner:** engine actors order persistence requests; `DbWorker`
  owns its dedicated SQLite connection and commits queued work.
- **Target owner:** unchanged; SYS-009 adds evidence around the existing owner.
- **Invariant preserved:** in-memory progress is not considered persisted
  until the worker's transaction returns successfully; the reopened database
  reflects the last acknowledged row.
- **Must not:** weaken `DbWorker` supervision or make actor tests mutate its
  private SQLite connection.

### Acceptance

- A torrent progress flush runs through `DbExecutor::Worker` against a real
  file-backed database.
- The actor sees old durable values before the flush and updated values after
  it, including after worker shutdown and database reopen.
- Worker metrics show the batched-transaction path ran.
- Existing worker regressions continue to cover concurrent batched siblings,
  command isolation, rollback, panic containment, and shutdown.
- The broader engine fixture set may continue using direct in-memory DBs.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/db_worker.rs`,
  `torrent_task.rs`, and the existing `db_worker.rs` tests.
- **State or wire compatibility:** no database schema or public API change.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** tests opt into the worker; production
  behavior is unchanged.

### Evidence

- **Regression source added or updated:**
  `transfer_stats_are_batched_until_progress_flush_on_db_worker`.
- **Command or probe to run:** during the requested bulk pass, run:

  ```sh
  cargo test -p rt-engine --lib transfer_stats_are_batched_until_progress_flush_on_db_worker
  ```

  followed by the full `rt-engine` bulk suite.
- **Expected result:** the worker-backed progress update is visible after
  restart and increments the batched-transaction counter.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** Rust 1.88.0 and 1.94.0 full-workspace logs under `/tmp/tng-transfer-core-close-20261019/evidence/`.
- **External evidence still required:** none for SYS-009; current-main hosted
  CI is recorded under PERF-115.

## SYS-010 — Bounded discovery and control I/O

- **Priority / wave:** P1 / 4
- **Prerequisites:** SYS-004, SYS-008
- **Owning component:** tracker workers, outbound DNS admission, DHT bootstrap
  and registration, and engine metadata loading.
- **Finding or capability:** F35 plus bounded process-wide admission for
  discovery I/O and detached metadata work.
- **Status:** `wip`

### Scope

Bound tracker announces across torrents, share one no-wait DNS budget across
tracker/webseed egress and IPv4/IPv6 DHT bootstrap, and cap persisted metadata
projection loads that wait for blocking parsing. Keep optional scrape work
independent of announce delivery. Existing DHT registration, retry, sweep,
command-delivery, bootstrap node, answer-count, and deadline limits remain part
of the same control-I/O contract. Magnet completion, torrent promotion, DHT
registration, and pure-v2 recheck keep their own admission budgets; they do not
consume the projection-load semaphore.

### Ownership and invariant

- **Current owner:** `TrackerWorkers` owns each torrent's announce futures;
  process semaphores own global announce, scrape, and DNS admission; the DHT
  task owns bootstrap and registration; `load_torrent_metadata_bounded` owns
  persisted projection-load permits.
- **Invariant preserved:** accepted announce responses reach their torrent
  actor before optional scrape work; saturated budgets do not create an
  unbounded waiter or detached-task queue; DNS permits cover resolution only.
- **Overload behavior:** tracker work stays due for a later actor turn, optional
  scrapes are skipped, DNS lookups fail quickly, and projection-load requests
  are rejected when the bounded active-plus-waiting admission is full.
- **Must not:** hold a global announce permit while scraping, release projection
  permits when an async caller is cancelled but `spawn_blocking` is still
  running, or collect unbounded resolver answers.

### Acceptance

- At most 8 tracker announces run per torrent and 64 process-wide; announce
  result channels retain at most 8 items per torrent.
- Optional HTTP scrapes remain capped process-wide at 4 and use a separate
  result channel sized to that cap.
- Tracker, webseed, and DHT bootstrap share a non-queuing 32-lookup DNS budget;
  each lookup retains its existing deadline and 64-address ceiling.
- DHT bootstrap remains capped at 256 nodes, a 10-second aggregate window, and
  5 seconds per node. Registration parse concurrency is 32, detached
  registration/retry tasks are capped at 256, sweeps at 1, and command delivery
  at 5 seconds.
- Persisted metadata projection loads allow 4 active jobs and at most 32
  additional admitted callers; the blocking closure owns both permits until
  parsing exits, even if its async caller is cancelled. Other parser entry
  points remain governed by their separately documented admission budgets.
- Saturation and scrape progress have regression sources; bulk qualification
  confirms overload recovery and whole-session responsiveness.

### Files and compatibility

- **Expected code paths:** `outbound_resolver.rs`, `egress_policy.rs`,
  `tracker_runtime.rs`, `dht_task.rs`, `engine.rs`, and the rebuild ledger.
- **State or wire compatibility:** no tracker protocol, DHT wire, metadata, or
  database format changes; only local admission and retained queue bounds.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** existing bounds apply to all
  outbound-discovery paths.

### Evidence

- **Regression sources:** `dns_budget_rejects_excess_work_without_a_wait_queue`,
  `tracker_worker_availability_obeys_the_process_wide_limit`,
  `metadata_load_admission_rejects_when_the_bounded_queue_is_full`, and the
  existing bounded-answer and announce-before-scrape regressions.
- **Command or probe:** run the targeted resolver, tracker-worker, metadata
  loader, and DHT bootstrap suites during the requested bulk pass.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** mixed-load responsiveness and recovery
  under saturated tracker, DNS, DHT, and metadata budgets.

## SYS-011 — Passive, owner-specific metrics

- **Priority / wave:** P0 / 2
- **Prerequisites:** SYS-001
- **Owning component:** native metrics handler and the snapshots exposed by
  engine, storage, persistence, API, peer-ingress, transport, and crash-safety
  owners.
- **Finding or capability:** F29 and the need to identify which live owner
  supplies every exported metric family.
- **Status:** wip

### Scope

Keep /metrics observational: it reads existing owner snapshots and never
initializes a runtime or worker. Document the metric owner, scope, and limits.
This ticket does not make metrics an activation path for otherwise dormant
subsystems.

### Ownership and invariant

- **Current owner:** each subsystem owns its counters; the native handler
  combines snapshots into Prometheus exposition.
- **Target owner:** unchanged; metrics remain projections of subsystem-owned
  state.
- **Invariant preserved:** collection does not create owners or duplicate
  shared storage accounting; absent owners remain absent or unavailable.
- **Must not:** create StorageRuntime from a scrape, multiply shared storage
  gauges by torrent count, or expose raw torrent IDs by default.

### Acceptance

- The handler uses passive accessors for process-global storage/runtime state.
- The descriptor-budget scrape reports whether its gate exists and never
  raises RLIMIT_NOFILE.
- Metric families and their source owners, scope, privacy, and known limits
  are documented in TRANSFER_CORE_OBSERVABILITY.md.
- Renderer tests cover absent and initialized runtime output and ownership
  labels without requiring a live transfer.
- A fresh-process regression proves a scrape does not create the storage
  runtime or worker.
- Large-catalog and mixed-load scrape cost remains SYS-014/PERF-109 evidence.

### Files and compatibility

- **Expected code paths:** crates/rt-api-native/src/handlers.rs, engine and
  storage metric snapshots, TRANSFER_CORE_OBSERVABILITY.md.
- **State or wire compatibility:** Prometheus names remain stable unless a
  separately reviewed compatibility change is needed.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** existing native /metrics route.

### Evidence

- **Regression source added or updated:** F29 uses StorageRuntime::initialized
  in the renderer; the fresh-process descriptor-gate/RLIMIT passivity
  regression passed in the exact-source suite at `354ad504`.
- **Command or probe to run:** native metrics renderer and fresh-process
  passivity regressions during the requested bulk pass.
- **Expected result:** a scrape never creates the storage runtime and each
  shared metric is emitted once per owner/resource identity.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** large-catalog and mixed-load
  collection latency and resource impact.

## SYS-014 — Complete-catalog cost and mixed-load responsiveness

- **Priority / wave:** P1 / 6
- **Prerequisites:** SYS-005, SYS-006, SYS-007, SYS-008, SYS-009, SYS-010,
  PERF-109
- **Owning component:** live native-engine catalog probe in
  `crates/rt-metrics/tests/scale.rs`.
- **Status:** `done`

### Scope and invariant

Measure the native list and metrics paths with a catalog that entered through
production metainfo admission, persisted, shut down, and restored. Keep the
probe observational: all catalog torrents remain paused, API requests are
read-only, responses are drained, and no timing threshold is treated as a
release gate.

### Implementation slice

The mixed-load source runs 16 synchronized rounds of four concurrent requests:
two bounded/sorted list pages, transfer info, and `/metrics`. It reports each
route's raw latency samples and p50/p95 in a JSON log record. Both 1k and
opt-in 100k catalog cases use the same production-engine create/persist/
restore path as the existing live catalog probe. The 100k cases stay ignored
by default due to their storage and resource cost.

`catalog_load_report.py` assembles the retained full test log only for a clean,
committed worktree and exact source commit. It requires both catalog sizes,
validates all four routes and 16 positive raw samples per route, recomputes
nearest-rank p50/p95, and records host identity. Manifest validation links the
report to a separately retained `transfer-catalog-load-log-v1` artifact and
requires the report source commit to match the clean manifest source. The
manifest writer accepts this artifact-only evidence set directly, without a
synthetic transfer trial.

### Limits and evidence

- The exact clean source `eb5f31588beb4cf9ec0fb4c3509745c49c16f500` passed the
  1k mixed-load case and both opt-in 100k cases: production metainfo admission,
  persist/shutdown/restore, and the four-route mixed-load probe.
- The 100k case ran optimized on Rust 1.94.0 with four Tokio workers and 16
  rounds of two bounded list pages, transfer info, and `/metrics`. Nearest-rank
  list p50/p95 was 0.325/838.111 ms (default) and 0.337/859.572 ms
  (name-sorted). The first cold snapshot produced the p95 outlier; later list
  samples were sub-millisecond. Transfer-info p50/p95 was 0.098/3.350 ms;
  `/metrics` was 0.092/0.323 ms. The 1k list p50/p95 was 0.118/7.437 ms and
  0.108/7.415 ms for the two list routes. These in-process measurements are
  diagnostic only and apply no release threshold.
- The report, raw log, artifact-only manifest, and
  `transfer-catalog-load-v1` qualification all validate. Their SHA-256 values
  and retained paths are in the 2026-10-19 current-closeout record above; the
  report is bound to the clean source commit.
- **Artifact paths:**
  `/tmp/tng-transfer-core-close-20261018/evidence/catalog-load-report-eb5f3158.json`,
  `catalog-load-manifest-eb5f3158.json`,
  `catalog-load-qualification-eb5f3158.json`, and the linked
  `final-main-catalog-mixed-load-1k-100k-rust-1.94-release-eb5f3158.log`.
- **Executed on exact revision:** yes; all three catalog test cases and all
  report, manifest, and qualification validators passed on `eb5f3158`.
- This paused in-process probe does not represent daemon RSS, concurrent
  payload transfer, independent clients, or a hardware capacity guarantee.
  PERF-102/PERF-109 own process-level resource and active-transfer evidence;
  SYS-015 owns final release integration.
- **External evidence still required for those separate gates:** production
  binary 10k/100k restore with process-attributed samples, active transfer,
  repeated trials, and paired-client measurements.
## PERF-108 — Stable memory accounting domains

- **Priority / wave:** P0 / 4
- **Prerequisites:** PERF-101, PERF-102, SYS-011
- **Owning component:** `rt-engine` memory projections, the native Prometheus
  renderer, and the independent PERF-102 observer.
- **Finding or capability:** remove shadow-runtime metrics and keep governor
  leases, torrent-owner estimates, allocator values, process memory, and
  cgroup measurements distinct.
- **Status:** `wip`

### Scope

Keep existing Prometheus names stable while defining what each memory family
counts. The resource-governor family remains active leases, not RSS. The
top-torrent family remains a bounded estimate; expose outstanding-request
receive reservations separately from upload buffers because the reservation
can overlap materialized piece memory. PERF-102 remains the independent source
for process, allocator, and cgroup observations.

### Ownership and invariant

- **Current owner:** TorrentNG owners expose lease snapshots and bounded
  per-torrent counters; the external observer samples process and cgroup
  state.
- **Target owner:** unchanged; each metric reads its existing owner and each
  measurement domain remains separate.
- **Invariant preserved:** a scrape never initializes an owner, and no
  governor, RSS, allocator, cgroup, or estimate value is relabeled as another
  domain.
- **Must not:** add an aggregate that sums overlapping owner estimates and
  call it process memory, or treat unavailable measurements as zero.

### Acceptance

- Keep `torrentng_memory_used_bytes` and class usage explicitly defined as
  current resource-governor leases.
- Keep the top-torrent aggregate metric name, document its estimate and
  reservation overlap, and separately render receive reservations and upload
  buffers for each retained top torrent.
- Hash torrent identifiers by default and preserve the existing explicit
  opt-in for raw identifiers.
- Document metric meaning, source owner, and known overlap in `docs/API.md`
  and `docs/TRANSFER_CORE_OBSERVABILITY.md`.
- Renderer regressions verify the separate receive/upload values and the
  default identifier redaction.
- Process, allocator, and cgroup evidence continues to come from the
  identity-bound observer; large-catalog scrape cost remains SYS-014/PERF-109.

### Files and compatibility

- **Expected code paths:** `crates/rt-engine/src/command.rs`,
  `crates/rt-api-native/src/handlers.rs`, `docs/API.md`, and
  `docs/TRANSFER_CORE_OBSERVABILITY.md`.
- **State or wire compatibility:** additive gauges only; existing metric names
  and hash privacy defaults remain stable.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** no new flag; interpret owner estimates
  only within their documented accounting domain.

### Evidence

- **Regression source added or updated:** engine estimate projection and native
  metrics renderer now preserve the combined estimate while exposing receive
  reservations and upload buffers separately.
- **Command or probe to run:** engine aggregation and native metrics renderer
  regressions during the requested bulk pass.
- **Expected result:** values retain their owner and overlap semantics, with
  raw identifiers absent unless explicitly enabled.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** PERF-102 process/allocator/cgroup
  observations and SYS-014/PERF-109 collection-cost qualification.

## SYS-012 — Production-shaped protocol and platform compatibility fixtures

- **Priority / wave:** P0 / 3
- **Prerequisites:** SYS-001, SYS-002, SYS-003
- **Owning component:** `rt-testkit`, with production parsing and geometry
  supplied by `rt-metainfo` and `rt-piece-map`.
- **Finding or capability:** preserve v1, pure-v2, and hybrid file/piece
  semantics while the transfer implementation changes, and keep the portable
  product build working on every released platform.
- **Status:** `wip`

### Scope

Provide one deterministic, payload-derived Rust fixture corpus consumed by
production metainfo parsing and piece-map code. Exercise the v1 and v2 views of
the same multi-file torrent, including a multi-piece file, nested path, empty
file, authenticated v2 piece layer, and BEP 47 alignment padding in hybrid
metainfo. SYS-013 now provides a reversible per-torrent route between the
reference and shared actor paths; this fixture ticket does not enable a cutover
or claim equivalent runtime behavior across backends. Rollout remains gated by
parity, recovery, and qualification evidence.

The release build matrix remains Linux x86_64, Linux aarch64, macOS aarch64,
and Windows x86_64. The normal workspace suite covers the fixture and full
metainfo/piece-map crates on Linux; the Windows smoke workflow runs those same
crates on Windows and separately exercises the portable storage plan
executor. macOS and Linux aarch64 are release-build targets, not test runners
in this ticket. The fixtures themselves avoid platform-specific paths,
sockets, or filesystem timing.

### Ownership and invariant

- **Current owner:** metainfo and file geometry regressions have separate
  crate-local builders; Windows storage compatibility is covered by its smoke
  workflow.
- **Target owner:** `rt-testkit` owns reusable fixture bytes and payloads;
  parsers and piece maps remain the source of semantic truth.
- **Invariant preserved:** infohashes derive from the exact encoded `info`
  dictionary, while v1 piece hashes and v2 file roots derive from payload
  bytes; hybrid v1 padding is synthetic zero content and is absent from the v2
  payload view.
- **Must not:** duplicate production parsing/geometry rules inside the fixture
  crate, write padding to disk as real content, or describe an authored fixture
  as platform test evidence before it runs there.

### Acceptance

- A repeatable fixture matrix contains v1, pure-v2, and hybrid torrents with
  byte-for-byte stable metainfo and payload.
- V1 piece hashes cover the generated byte stream, including hybrid zero pads;
  v2 file roots and any required piece layer are computed from that file's
  payload blocks.
- Production parsing preserves file order, offsets, piece counts, private
  status, and both hybrid identity views.
- Piece-map regressions show hybrid padding as a synthetic region that does
  not become wanted file content.
- `rt-testkit`, `rt-metainfo`, and `rt-piece-map` run in Linux workspace tests
  and the Windows smoke workflow; existing release builds continue to cover
  the four shipped target combinations above.
- Feature-path changes retain a rollback route through the existing actors
  until SYS-013 qualifies migration and cutover.

### Files and compatibility

- **Expected code paths:** `crates/rt-testkit/src/transfer_fixtures.rs`, its
  dependencies and README, `docs/TRANSFER_CORE_CAPABILITIES.md`, and
  `.github/workflows/windows-smoke.yml`.
- **State or wire compatibility:** no product state, network protocol, or
  stored metainfo format change; fixtures use valid BEP 3, BEP 47, and BEP 52
  encodings.
- **Migration / rollback:** none; production routing remains unchanged.
- **Feature flag or rollout boundary:** protocol variants are fixture cases,
  not runtime feature toggles. Shared-core production cutover remains SYS-013.

### Evidence

- **Regression source added or updated:** `rt-testkit` deterministic corpus
  tests production parsing, v2 payload hashes, hybrid alignment, and piece-map
  treatment of padding.
- **Command or probe to run:** during the requested bulk pass,
  `cargo test --locked -p rt-testkit -p rt-metainfo -p rt-piece-map` on Linux
  and Windows; retain the existing release target builds for macOS aarch64 and
  Linux aarch64.
- **Expected result:** stable metainfo and hashes, consistent hybrid extents,
  and no padding file treated as payload content.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** exact-source Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external evidence remains listed below.
- **External evidence still required:** release-matrix results for the exact
  production source; hosted Windows Smoke passed at `94c8c1b3`, whose
  production source matches `354ad504`.

## NET-112 — Select and qualify the uTP protocol core

- **Priority / wave:** P1 / 4
- **Prerequisites:** NET-101, NET-102
- **Owning component:** `rt-utp` protocol adapter and the engine-owned shared
  peer runtime.
- **Finding or capability:** F07: the current outbound writer is stop-and-wait;
  the current receiver drops reordered DATA and does not apply SACK.
- **Status:** `wip`

### Scope

Keep the native runtime implementation in TorrentNG. Cargo metadata reports
that both reviewed `urtorrent-utp` releases, 0.14.2 and 0.14.3, require Rust
1.98; adopting either would break the workspace's Rust 1.88 floor. Use upstream
as a protocol and implementation reference for windowed sends, selective ACK,
reordering, retransmission, congestion response, MTU handling, and wire
behavior checked against libtorrent captures. Implement the protocol core,
connection manager, stream adapter, queue admission, and memory accounting in
TorrentNG. Target one UDP socket per address family and deadline plus ready
queues rather than per-peer sockets or unconditional full-table scans. Do not
adopt the full `urtorrent` engine or enable uTP by default before NET-116.

The reference's `Vec` API reports three user-space payload copies, its default
receive capacity is 1 MiB per peer, and its manager scans the connection table
for several operations. These are evaluation findings and design constraints,
not claims about the implementation TorrentNG will produce. Keep payload copy
count, admitted bytes, per-peer overhead, and scheduling work measurable.

### Ownership and invariant

- **Current owner:** `rt-utp::UtpStream`, `UtpEndpoint`, and the new
  `UtpRuntime`, with the engine's `GlobalNetworkBudget` owning and sharing the
  runtime across peer and metadata tasks.
- **Target owner:** one TorrentNG uTP runtime owns sockets, protocol state,
  connection identities, deadlines, readiness, and byte admission; each
  logical `UtpStream` is a bounded handle into that owner. Source now has shared
  socket demultiplexing, one deadline heap per endpoint, a deduplicated FIFO
  ready queue drained in 32-event turns, fixed per-stream reservations, and an
  aggregate endpoint datagram budget. Each admitted stream reserves one
  cancellation slot in the same bounded FIFO as its schedule commands, so a
  dropped stream cannot lose or overtake its final cancellation. The scheduler
  handles at most 32 due deadlines, ready signals, and queued commands per
  turn, and does no full-table liveness sweep. Bulk qualification remains open.
- **Invariant preserved:** per-stream reservations cover bounded receive and
  retransmit windows plus route/deadline metadata; queued datagrams retain
  endpoint-budget reservations until processing or discard. Cancellation cannot
  release capacity still owned by the runtime. Usage reports reservations and
  live packet charges, not process RSS.
- **Must not:** use an ambient blocking thread, silently fall back from a
  rejected uTP datagram into an unbounded queue, exceed aggregate or per-peer
  receive/retransmit budgets, raise the workspace MSRV for the reference
  crate, or claim performance from source inspection.

### Acceptance

- The native workspace remains Rust 1.88 compatible and has no direct or
  transitive dependency on the Rust-1.98-only reference crate.
- Outbound and inbound logical streams share the engine-owned UDP socket for
  their address family; no per-peer UDP socket is created.
- A SYN flood cannot exceed the configured stream cap. Read/write queues,
  receive capacity, retransmit storage, pending datagrams, and deadline/ready
  metadata have hard per-stream or aggregate endpoint budgets. The endpoint
  defaults to a 64 MiB reservation limit and exposes current reservation usage.
- Each admitted stream retains a bounded FIFO permit for cancellation. Saturating
  the schedule-command capacity cannot drop a cancellation; FIFO ordering keeps
  cancellation behind the stream's earlier deadline updates. Scheduler turns
  process at most 32 due deadlines, ready signals, and queued commands apiece
  and never scan the connection table.
- The adapter preserves the peer-wire byte-stream interface, configured TCP
  fallback, close/shutdown behavior, and current metrics ownership.
- Deterministic tests cover handshake, window refill, loss/retransmission,
  selective ACK, out-of-order delivery, sequence wrap, zero-window recovery,
  duplicate SYN, inbound admission cap, and two peers sharing a port.
- The seeded transfer simulator and a lossy/reordered local UDP harness agree
  on byte and connection ownership.
- Matched efficiency results report verified goodput, CPU/GiB, process and
  cgroup memory, socket count, payload copies, retransmissions, and queue
  latency. The current backend remains available as a test reference until
  NET-116 accepts the candidate.

### Files and compatibility

- **Expected code paths:** `crates/rt-utp/`, engine uTP endpoint setup,
  `GlobalNetworkBudget`, outgoing connect sites, `docs/protocol/UTP.md`, and
  the uTP evaluation note.
- **State or wire compatibility:** BEP 29 packet and peer-wire behavior must
  remain interoperable; no persisted state format changes.
- **Migration / rollback:** keep the current transport behind a test-only or
  explicit fallback path through correctness and performance qualification;
  do not run two writers for one peer.
- **Feature flag or rollout boundary:** existing incoming/outgoing uTP
  controls; default behavior stays unchanged until NET-116.

### Evidence

- **Regression source added or updated:** existing `rt-utp` transport tests
  remain the baseline; add deterministic state/window and adapter cases with
  NET-112/113. Deadline coverage includes schedule-queue saturation with FIFO
  cancellation and removal of canceled registered/ready state.
- **Command or probe to run:** `cargo test -p rt-utp` and the focused engine
  uTP integration cases during the requested bulk pass.
- **Expected result:** all protocol, ownership, socket-sharing, and boundedness
  cases pass on the exact integration revision.
- **Executed on exact revision:** yes; the full `rt-utp` and engine suites passed on Rust 1.88.0 and 1.94.0 at `354ad504`. Matched hardware/network evidence remains open.
- **Artifact or report path:** the exact-main workspace logs are under
  `/tmp/tng-transfer-core-close-20261018/evidence/`; matched efficiency reports
  remain pending under PERF-112 through PERF-116.
- **External evidence still required:** matched hardware/network trials under
  PERF-112 through PERF-116; source review is not a performance result.

## NET-113 — Bounded windowed uTP runtime integration

- **Priority / wave:** P1 / 8
- **Prerequisites:** NET-103, NET-106, NET-112
- **Owning component:** `rt-utp` plus the engine's shared network service.
- **Finding or capability:** use one shared UDP receive/demultiplexing path per
  address family for production uTP peers, while keeping protocol-owned bytes
  bounded and safe across async cancellation.
- **Status:** `wip`

### Scope

The source now contains a capped packet/byte send window, cumulative plus SACK
acknowledgement accounting, bounded receive reordering, stream-owned
retransmission payloads, and `UtpRuntime`. The production v1, pure-v2, and
metadata paths connect through that runtime. Incoming and outgoing IPv4 streams
share the listener endpoint when incoming uTP is enabled; outbound-only address
families lazily bind one shared endpoint. Listener rebind retires the old
endpoint for existing routes and limits retained generations. Central timer
scheduling now uses one heap per endpoint and a deduplicated FIFO ready queue
drained in 32-event turns. Per-stream reservations and live datagram charges
share a hard aggregate endpoint budget. The full loss/reorder acceptance
matrix and all validation remain open.

### Acceptance

- Every production outbound uTP call site uses the engine-owned runtime; direct
  per-peer `UtpStream::connect` remains only as a standalone crate API and test
  primitive.
- Streams in one endpoint generation share a local UDP socket. Rebinding
  changes the socket for new connections without silently replacing an active
  route; stale endpoints reject new SYNs and are bounded.
- Retransmission bytes survive canceled writes, and receive reorder bytes stay
  under the configured receive budget until delivered or rejected.
- Regression sources cover multi-packet window fill, SACK, reordering, wrap,
  cancellation, multi-stream socket sharing, rebind with an active stream, and
  route caps. These regressions passed in the exact-main workspace.
- Deadline scheduling and queue fairness regressions passed in the exact-main
  workspace. Resource and efficiency
  claims require matched evidence; a shared socket or memory limit alone is not
  a performance result.

### Evidence

- **Regression source added or updated:** `rt-utp` window and transport tests,
  including the shared runtime/rebind case.
- **Command or probe to run:** `cargo test -p rt-utp` and the engine's uTP
  integration tests during the requested bulk pass.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **External evidence still required:** matched whole-client trials under
  PERF-112 through PERF-116.

## PERF-106 — Deterministic independent swarm and network controls

- **Priority / wave:** P0 / 4
- **Prerequisites:** PERF-101, PERF-105
- **Owning component:** `scripts/transfer_rebuild/network_control.py`,
  network-control schema, trial/manifest validator, and later the paired-run
  harness.
- **Finding or capability:** every claimed network profile is applied to an
  isolated fixture link and bound to immutable readback evidence.
- **Status:** `wip`

### Scope

Control the local peer network and its link profile so capacity and matched
service runs do not depend on public-swarm availability. The helper applies
pinned Linux egress profiles to a named task-owned veth in a `tngt-*` network
namespace. Clean profiles provide fixed delay and rate. Seeded impairment
profiles add independent random loss and delay jitter. Every apply records
namespace/interface identity, iproute2 versions, requested delay/jitter/loss/
rate/queue/seed, and normalized `tc -j` qdisc readback. Reset removes only the
exact recorded qdisc and verifies the default root qdisc returns.

Profiles are one-way egress controls. `paired_runner.py` applies one profile
per endpoint when both directions need shaping. It derives distinct seeds from
the randomized pair identity, profile, and endpoint role, then records the
ordered target/seed values and both apply-artifact digests. The validator
recomputes those seeds and verifies that each artifact's profile, settings,
qdisc readback, and tool identity match. Configured one-way delay, jitter, and
loss are recorded separately; `rtt_ms` stays null because the runner does not
measure RTT. The helper does not create namespaces, shape host or loopback
interfaces, or accept a non-default root qdisc. `paired_runner.py` composes it
with an isolated tracker, seed/target endpoints, and deterministic peer-start
barrier; client execution and host qualification remain open.

### Ownership and invariant

- **Current owner:** `network_control.py` owns the task-namespace qdisc apply
  and reset lifecycle; `paired_runner.py` applies and resets it on both trial
  endpoints; the manifest validator owns artifact digest and trial linkage
  checks.
- **Target owner:** PERF-104/106's paired runner owns fixture startup,
  independent peers, two-sided shaping, trial start barrier, and cleanup.
- **Invariant preserved:** a profile claim is backed by exact pinned settings
  and qdisc readback on a stable task namespace and interface identity.
- **Must not:** replace an existing non-default root qdisc, overwrite a report,
  touch `lo` or a host interface, treat an unmeasured RTT as profile truth, or
  record randomized loss/jitter without deterministic seed semantics.

### Acceptance

- Only `tngt-*` namespaces and task-named veth interfaces are accepted; the
  helper refuses missing/ambiguous interfaces, non-veth links, changed
  namespace/interface identity, and non-default root qdiscs.
- Apply verifies the pinned profile through `tc -j` before writing a new
  mode-0600 artifact. Failure to persist the apply artifact removes only the
  exact qdisc that this operation installed.
- Reset requires the immutable apply record, rechecks namespace inode and
  interface index, verifies the handle/kind/settings, removes only that qdisc,
  and writes a separate reset artifact.
- Trials with a controlled profile link all apply-artifact digests. Manifest
  validation verifies the file digest, schema, profile settings, observed
  qdisc, tool identity digest, link-rate value, deterministic endpoint seeds,
  configured impairment summary, and reset-to-apply reference. Completed
  shaped trials include both endpoint apply and reset records.
- Stochastic profiles use independent random packet loss and jitter with zero
  correlation. Their explicit unsigned 64-bit PRNG seeds are read back from
  `tc -j`; records without a seed remain accepted only for historical clean
  profiles, so existing loss-free artifacts remain resettable.
- No performance result is accepted until the isolated swarm fixture and
  paired-run orchestration establish the same workload for each candidate.

### Files and compatibility

- **Expected code paths:** network control helper, `network-control.schema.json`,
  trial environment fields, manifest validator, harness README, measurement
  contract, and focused regressions.
- **State or wire compatibility:** versioned certification artifacts only;
  existing unshaped trial records remain valid.
- **Migration / rollback:** every apply has a distinct reset report; no
  production interface is changed by this helper.
- **Feature flag or rollout boundary:** network-control observations remain
  optional until a paired fixture claims a profile; no profile is a product
  runtime setting.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_network_control.py` and
  `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** network-control and validator regressions during
  the requested bulk pass; do not run `tc` on the desktop/server host as part of
  implementation.
- **Expected result:** apply/reset refuses unrelated qdiscs, records only
  verified settings, and manifest validation rejects unlinked or mismatched
  profile evidence.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** pending isolated-network qualification.
- **External evidence still required:** task-owned namespace lifecycle,
  two-ended control, independent swarm readiness, and repeatability on the
  qualification kernel/iproute2 matrix.

## PERF-107 — Independent network, hash, disk, and pipeline ceilings

- **Priority / wave:** P0 / 5
- **Prerequisites:** PERF-102, PERF-105, PERF-106
- **Owning component:** `scripts/transfer_rebuild/ceilings.py` and the
  versioned calibration/report schemas.
- **Finding or capability:** estimate the workload's independent service
  ceilings so a slow client run can be attributed to fixture limits, an
  undersized request pipeline, storage, hashing, or the client path.
- **Status:** `wip`

### Scope

Accept independently collected network, disk, SHA-1/SHA-256, and peer RTT
observations; record the exact host, filesystem/device, network profile, tool
identity, and raw evidence digests; then derive protocol-aware download and
upload bounds. The pipeline estimate is `peers × requests per peer × block
bytes / measured RTT`. Download is bounded by receive capacity, disk writes,
the required protocol hash rate, and the request pipeline. Upload is bounded
by transmit capacity, disk reads, and the request pipeline.

The output is a diagnostic model, not a candidate measurement or a performance
claim. Missing or invalid inputs fail closed rather than becoming zero or an
infinite ceiling. The model does not decide scorecard thresholds, normalize
unrelated hosts, or replace the matched whole-client trials.

### Ownership and invariant

- **Current owner:** `ceilings.py` validates the calibration input and emits an
  immutable derived report; manifest validation binds both to trial records.
- **Target owner:** PERF-110's paired harness requests a ceiling report for the
  same host, storage device, network profile, and protocol before trials run.
- **Invariant preserved:** every derived rate is traceable to independent raw
  artifacts and the exact tool/version; unknown resources remain unknown.
- **Must not:** derive calibration from the candidate's measured transfer,
  call a modeled ceiling observed client throughput, or treat a missing hash
  or RTT result as unlimited capacity.

### Acceptance

- Versioned input and output schemas reject missing identities, invalid rates,
  unsupported protocol values, malformed digests, and unmeasured hybrid hash
  capacity. Each probe records its PID and process-start ticks so the manifest
  can reject calibration collected inside the candidate process.
- The model uses SHA-1 for v1, SHA-256 for v2, and an independently measured
  combined hash rate for hybrid.
- Network receive/transmit and storage read/write ceilings retain separate
  directions; pipeline capacity uses measured RTT and explicit peer/request/
  block geometry. Its client, version, settings digest, and JSON pointer bind
  request depth to the process-bound settings readback; trial peer and block
  counts must match the calibration.
- The report records every component rate, exact limiting components, and a
  fixed diagnostic-only interpretation.
- Input and output are immutable artifacts; manifest validation checks all raw
  artifact digests and recomputes the report from the linked calibration.
- Trial linkage requires matching host, OS, kernel, CPU, filesystem, device,
  network profile, and protocol. It rejects calibration process identities
  that match the measured client.
- Regression sources cover hash/protocol selection, each direction's
  bottleneck, pipeline math, invalid values, symlinks, immutable writes, raw
  artifact linkage, report recomputation, and candidate-process reuse.

### Files and compatibility

- **Expected code paths:** `ceilings.py`, the ceiling input/report schemas,
  `validate_artifact.py`, trial schema, harness README, and focused regression
  sources.
- **State or wire compatibility:** new `transfer-ceiling-input-v1` and
  `transfer-ceiling-v1` certification artifacts; optional trial linkage only.
- **Migration / rollback:** existing trial records remain valid without a
  ceiling digest; no product runtime behavior changes.
- **Feature flag or rollout boundary:** PERF-110 may use the model only as
  diagnostic context; scorecard decisions remain gated by PERF-113.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_ceilings.py` and linked-artifact cases in
  `scripts/test_transfer_rebuild_validator.py`.
- **Command or probe to run:** ceiling and validator regressions during the
  requested bulk pass.
- **Expected result:** complete, identity-matched independent calibrations
  produce deterministic bounds; missing or conflicting evidence is rejected.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** pending independent calibration and paired
  measurements; repository regression sources passed at `354ad504`.
- **External evidence still required:** repeated network, disk, hash, and RTT
  calibration on the dedicated comparison host.

## PERF-109 — Live production catalog fixtures

- **Priority / wave:** P0 / 5
- **Prerequisites:** PERF-102, PERF-103, PERF-104, PERF-105, PERF-108
- **Owning component:** `rt-testkit` catalog metainfo fixtures and
  `rt-metrics/tests/scale.rs`.
- **Finding or capability:** current scale proxies populate API registries and
  databases with fabricated rows. Add a path through valid metainfo, engine
  persistence, runtime registry, and the native API before describing a
  catalog observation as production-shaped.
- **Status:** `wip`

### Scope

Generate deterministic, unique, valid private v1 metainfo with a real
content-derived piece hash. Populate an isolated engine with paused torrents
through `EngineHandle::add_torrent_raw_with_labels`, which parses the metainfo,
writes the torrent blob, persists the row, and registers the runtime entry.
Shut down the ingest engine cleanly, restart it from the same isolated database
and metadata blobs, then measure fixture generation, engine admission, restore,
and first/cached native API list requests while draining the response body.
Keep smaller direct-registry and synthetic-database cases as explicitly named
lower-level proxies.

The catalog fixture is metadata and persistence coverage. Its one-byte payload
does not exercise payload I/O, hashing throughput, swarm activity, or useful
goodput. The opt-in 100k live-engine case passed: all 100,000 paused rows
restored, and the native API returned the full count. Its in-process result is
not process-attributed capacity evidence.

### Ownership and invariant

- **Current owner:** `production_catalog_metainfo` builds deterministic
  bencoded metainfo; the scale harness sends it through the public engine API
  and observes the native API router.
- **Target owner:** PERF-110 launches the exact daemon and adapters as separate
  processes for resource-attributed, paired catalog and transfer trials.
- **Invariant preserved:** a live catalog observation contains distinct,
  parseable info hashes and paused entries admitted by the engine; synthetic
  registry and row tests remain labeled as proxies.
- **Must not:** treat in-process test RSS as daemon RSS, label a one-byte
  catalog as transfer capacity, or infer beyond the measured fixture.

### Acceptance

- Catalog metainfo is repeatable for one index, parseable as v1, private, and
  content-hash consistent; adjacent indexes have different info hashes.
- The live 1k case starts the production engine in task-owned temporary paths,
  admits each raw metainfo record through the engine API, verifies registry
  count and paused state, then calls the native list route and drains its
  response while checking the reported total.
- The 100k case uses the same fixture and engine path, is opt-in due to its
  resource cost, and reports an explicit setup failure if the production
  admission or restore path cannot reach the target count.
- Observations report fixture-generation, engine-admission, restore,
  first-list, and cached-list timings without applying a capacity threshold or
  claiming a performance win. “First” and “cached” describe the API snapshot;
  they do not imply cold or warm filesystem cache state.
- Synthetic API and database test names include `proxy`; comments and the
  harness documentation state that they are not production-capacity evidence.
- The audit, active specification, harness README, and ticket ledger distinguish
  authored source from executed evidence. PERF-102 and PERF-110 remain
  responsible for independent process resources and matched mixed-load trials.

### Files and compatibility

- **Expected code paths:** `crates/rt-testkit/src/transfer_fixtures.rs`,
  `crates/rt-metrics/tests/scale.rs`, `scripts/transfer_rebuild/README.md`,
  and the scale-claims audit.
- **State or wire compatibility:** test-fixture and regression-source changes
  only; no product data format or runtime behavior changes.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** the 100k observation remains ignored
  by default and is invoked explicitly during scale qualification.

### Evidence

- **Regression source added or updated:** `rt-testkit` verifies deterministic
  parseable catalog metainfo; `rt-metrics/tests/scale.rs` exercises 1k and
  opt-in 100k engine-backed API paths and labels synthetic cases as proxies.
- **Command or probe:** the live 1k case and explicit ignored 100k case passed
  on the closeout source with Rust 1.94.0.
- **Expected result:** each admitted fixture has a unique valid info hash and
  paused registry entry; restart restores the full catalog; API requests return
  successfully with separate setup, restore, first-page, and cached-page
  timings.
- **Executed on exact revision:** yes; the 1k/100k production-catalog tests
  passed at `eb5f3158`, and the clean-source report, manifest, and qualification
  validate at that revision. Process-attributed resource and active-transfer
  evidence remains open.
- **Artifact or report path:** exact report, raw log, manifest, and qualification
  paths are listed in SYS-014.
- **External evidence still required:** production-binary 10k/100k restore,
  process-attributed resource samples, mixed activity, and repeated trials.

## PERF-110 — Reproducible paired-run catalog and schedule

- **Priority / wave:** P0 / 6
- **Prerequisites:** PERF-103, PERF-104, PERF-105, PERF-106, PERF-107, PERF-109
- **Owning component:** certification tooling.
- **Finding or capability:** paired comparisons need a fixed workload catalog,
  randomized order, explicit exclusions, and retained execution evidence.
- **Status:** `wip`

### Scope

This spec interprets the ticket's “28-case catalog” as the seven checked-in
client adapters crossed with the four checked-in fixture profiles. This is a
separate efficiency catalog from the 28-row protocol correctness matrix in
`docs/INTEROP_MATRIX.md`. The plan generator records all 28 slots, including
known unsupported combinations, and produces a seeded schedule for pinned
clients. The `candidate` label means only “not known to be unsupported by this
catalog”; client/profile compatibility still needs a runtime preflight before
the trial is considered eligible.

The current known exclusions are the `urtorrent` runner's v2 and hybrid slots:
it accepts private v1 metainfo only. Unpinned clients and profile/repetition
pairs with fewer than two candidate clients are explicit exclusions. Every
scheduled pair holds profile and repetition fixed while randomizing client
order; `paired_runner.py` reuses one verified fixture for the paired trials.

### Ownership and invariant

- **Current owner:** `paired_plan.py` builds the catalog, seed-bound plan, and
  immutable output. `compatibility.py` probes each pinned candidate with a
  small private fixture, records add/paused/remove outcomes, and binds them to
  client identity and settings readback. `paired_runner.py` consumes the ready
  preflight, stages inputs, creates or reuses fixtures, launches native and
  container clients in isolated task namespaces, runs download and seed modes
  behind a shared barrier, observes resources, and assembles trials and
  manifests. `process_supervisor.py`, `client_control.py`, `task_network.py`,
  `task_tracker.py`, and `trial_record.py` provide its identity-bound pieces.
  `validate_artifact.py` validates the linked evidence, rehashes fixture
  payloads and the original torrent record, checks retargeted metainfo hashes
  against the original and retained binary, binds tracker addresses and hashes
  to the task topology, and verifies that network observation sampled the
  measured candidate namespace (target for downloads, seed for seeding). The
  runner requires its task-local staged-input directory to have exact mode
  `0700`; external JSON inputs and runner output records reject duplicate
  fields, nonstandard constants, and non-finite numeric values. The private
  tracker rejects malformed percent escapes, raw non-ASCII bytes, and
  unescaped control bytes before interpreting binary hashes or peer IDs.
  Complete trials require both tracker peers and a ready seed.
- **Target owner:** the paired executor and compatibility preflight remain
  separate commands; both must succeed before a trial can qualify.
- **Invariant preserved:** every included schedule slot is pinned and
  catalogued; the seed, schedule algorithm, adapter pins, and fixture-profile
  digest are retained with the plan; no pair compares different profiles.
- **Must not:** treat `candidate` as verified client support, label a plan as
  measured evidence, silently skip a failed or incompatible trial, or conflate
  this efficiency catalog with protocol interop correctness.

### Acceptance

- The catalog contains exactly seven adapter IDs by four profile IDs and has a
  canonical SHA-256 digest over its workload definitions and the profile-file
  digest.
- A caller supplies a valid pin file, unsigned 64-bit seed, and 1–100
  repetitions. The plan records the pin and catalog digests and the explicit
  `sha256-fisher-yates-v1` scheduling algorithm.
- Pair-group and client order are randomized reproducibly from the seed. Every
  pair uses one profile and repetition, has distinct clients, and includes all
  pinned candidate clients for that profile. Pairs with fewer than two clients
  are excluded with a reason.
- Unsupported and unpinned catalog slots are listed with reasons. A runtime
  compatibility preflight must reject or explicitly exclude unsupported
  candidates before any measurements begin.
- The compatibility preflight uses a private, profile-shaped probe under 3 MiB
  and a pin-declared disposable save path. It verifies paused/stopped state at
  admission for clients with pause-on-add behavior; rqbit is recorded as
  `after_add` because this adapter pauses it immediately after adding. It
  reconciles uncertain add responses by exact info hash, removes admitted
  probes, and preserves cleanup failures as blocking outcomes. The plan, pin
  file, profile digest, client identity, settings digest, pause mode, and all 28
  case outcomes are bound to an exclusive mode-`0600` artifact.
- A passed case is eligible only when add, paused-state verification, and exact
  removal all succeed. A scheduled pair is excluded when fewer than two of its
  clients pass. No failed or cleanup-failed case is silently dropped.
- Every attempted scheduled slot produces a trial record. A failed client
  startup remains `unmeasured`, with no client process identity, settings
  readback, or process resource observation; it is retained with its reason.
  A complete payload transfer accounts for the full expected useful payload,
  passes applicable integrity checks, and links the matching independent
  process observation.
- Plan output is exclusive, mode `0600`, schema-validated, and rejected if its
  catalog digest or deterministic schedule no longer matches its identity.
- Trial output is schema-validated, mode `0600`, exclusive, and syncs both file
  and parent directory. The source-bound manifest still requires a clean
  committed worktree and verifies each trial and evidence digest.
- Regression sources cover the 28 slots, balanced pairs, stable scheduling,
  exclusions, tamper rejection, bounds, and exclusive output.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/paired_plan.py`,
  `scripts/transfer_rebuild/compatibility.py`,
  `scripts/transfer_rebuild/process_supervisor.py`,
  `scripts/transfer_rebuild/paired_runner.py`,
  `scripts/transfer_rebuild/managed_client.py`,
  `scripts/transfer_rebuild/client_control.py`,
  `scripts/transfer_rebuild/task_network.py`,
  `scripts/transfer_rebuild/task_tracker.py`,
  `scripts/transfer_rebuild/trial_record.py`,
  `scripts/transfer_rebuild/fixtures.py`,
  `scripts/transfer_rebuild/validate_artifact.py`,
  `certification/transfer-rebuild/{paired-plan,compatibility-preflight}.schema.json`,
  harness README, and the paired-plan and compatibility regression sources.
- **State or wire compatibility:** additive `transfer-paired-plan-v1` and
  `transfer-compatibility-preflight-v1` certification artifacts; no runtime
  product format changes.
- **Migration / rollback:** none; plans are standalone artifacts and can be
  regenerated from pinned inputs and the recorded seed.
- **Feature flag or rollout boundary:** no comparison or performance claim is
  permitted from a plan, preflight, or authored runner alone. Local plan,
  observation, manifest, and validator regressions passed at `354ad504`; real
  client runs and independent observations remain required.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_paired_plan.py`,
  `scripts/test_transfer_rebuild_compatibility.py`,
  `scripts/test_transfer_rebuild_process_supervisor.py`,
  `scripts/test_transfer_rebuild_trial_record.py`,
  `scripts/test_transfer_rebuild_validator.py`, and semantic validation for
  the plan, observations, trial, and manifest artifacts.
- **Command or probe to run:** planner, compatibility preflight, schema
  validators, and regression cases during the requested bulk pass.
- **Expected result:** byte-identical plan content for identical inputs;
  schema-valid catalog/schedule; compatibility records retain every case,
  identify rqbit's after-add pause mode, reconcile uncertain adds, and reject
  incomplete removal evidence; tampered identity or ordering rejected; no
  output overwrite.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** runner, planner, preflight, process supervision,
  trial assembly, and validator regressions passed at `354ad504`. The pinned
  `urtorrent` runtime preflight was attempted but failed because
  `io_uring_setup` returned `Cannot allocate memory`; no paired client trial
  has completed.
- **External evidence still required:** successful pinned compatibility
  preflights, actual paired process execution, complete trial records,
  independent resources, and dedicated-host results.

## PERF-111 — Picker and changed-snapshot probes

- **Priority / wave:** P1 / 6
- **Prerequisites:** PERF-101, PERF-102, PERF-109
- **Owning component:** picker and native API performance probes.
- **Finding or capability:** F14's picker memory estimate and large-library
  snapshot work need repeatable, scale-shaped observations rather than a
  synthetic allocation claim.
- **Status:** `done`

### Scope

Add standalone, dependency-free Rust benchmark targets for two narrow paths.
The picker probe measures a warmed pick/cancel cycle at increasing piece
counts. The native API probe measures the initial, unchanged cached, and
single-row changed `AppState::torrent_snapshot` paths at increasing torrent
counts, including allocation calls and requested allocation bytes from a
process-local counting allocator. Results are diagnostic; they are not
threshold tests or whole-client performance evidence.

### Ownership and invariant

- **Current owner:** `rt-piece-picker` and `rt-api-native` benchmark targets;
  each prints CSV to standard output for retention in a task-owned artifact
  directory.
- **Target owner:** PERF-111 provides repeatable probes for the PERF-102 / PERF-109
  scale evidence and picker implementation review.
- **Invariant preserved:** cold/full snapshot, unchanged cache hit, and
  changed-row refresh remain separately labeled; allocation traffic is never
  described as retained heap or process RSS.
- **Must not:** introduce host-dependent pass/fail timing thresholds, mix
  fixture setup into the timed picker loop, or call a method-only probe a
  whole-daemon measurement.

### Acceptance

- Picker probe covers 1k, 12k, and 100k pieces with peer availability and
  prebuilt partial state up to the 16,384-piece in-progress cap; the 100k case
  uses a sparse peer bitfield over that admitted subset. It records the
  pick/cancel cycle duration and live picker memory estimate.
- API probe covers 1k, 10k, and 100k registry rows and separately reports one
  full initial snapshot, 1,000 unchanged cached reads, and three one-row
  incremental refreshes. Changed samples wait beyond the API snapshot TTL
  outside the timed region.
- The API probe runs on a single-thread runtime with its own counting global
  allocator, and reports allocation calls/bytes only during each measured
  snapshot operation. Those numbers mean cumulative allocation traffic.
- Both probes emit stable CSV headings, have no pass/fail thresholds, and
  document the need to retain commit, toolchain, and host identity with output.
- Regression source records the intended probe workloads. The local commands
  and two exact-source dedicated-host repeats have run; independent process
  RSS and whole-client evidence remain separate.

### Files and compatibility

- **Expected code paths:**
  `crates/rt-piece-picker/benches/perf.rs`,
  `crates/rt-api-native/benches/perf.rs`, their Cargo benchmark declarations,
  crate/harness READMEs, and this spec.
- **State or wire compatibility:** benchmark-only code; no runtime format or
  behavior changes.
- **Migration / rollback:** none; delete the standalone benchmark targets if
  later evidence shows they measure the wrong boundary.
- **Feature flag or rollout boundary:** no release gate is inferred from the
  output; whole-client matched trials remain PERF-110/113 and bulk qualification.

### Evidence

- **Regression source added or updated:** standalone picker and native API
  benchmark targets; both probes ran twice on the identified `kspls0` host.
- **Command or probe run:** `hardware_lane.sh` ran at clean source
  `eb5f3158` twice; each run invoked both release-mode probes and the configured
  storage move/import case.
- **Expected result:** valid CSV rows for each workload and explicitly
  separated initial/cache-hit/changed snapshot allocation traffic.
- **Executed on exact revision:** yes, twice on clean `eb5f3158` with Rust
  1.94.0. The 100k picker cycle measured 14.95 and 14.84 µs/op; 100k cached
  snapshot reads measured 123 and 133 ns/sample. Initial 100k snapshots were
  1.797 and 1.878 seconds.
- **Artifact or report path:**
  `/tmp/tng-transfer-core-close-20261018/evidence/hardware-kspls0-eb5f3158-01/`
  and `hardware-kspls0-eb5f3158-02/`.
- Independent process RSS and whole-client evidence remain separate
  PERF-102/PERF-110/113 qualification gates; they do not block completion of
  this diagnostic-probe ticket or support an efficiency claim.

## PERF-112 — V2 and persistence invariants in full engine paths

- **Priority / wave:** P0 / 5
- **Prerequisites:** PERF-104, PERF-105, PERF-106
- **Owning component:** production-shaped transfer fixtures and the
  `rt-metrics` engine integration target.
- **Finding or capability:** performance cases need protocol-correct payloads
  and must not count persisted completion state as valid after its content has
  changed.
- **Status:** `wip`

### Scope

Use the deterministic v1, pure-v2, and hybrid compatibility fixtures between
two production engine instances over loopback TCP. The receiver must obtain
all payload files through the real peer-wire, storage, and verification paths.
Persist paused completion state through a clean receiver shutdown and restore
it from the same database and metainfo blobs. Change one byte in the pure-v2
payload between process lifetimes, then require resume-time verification to
demote that torrent while the unchanged v1 and hybrid payloads remain
complete. This is a small correctness transfer and restart case, not a
transfer-rate or whole-client performance result.

### Ownership and invariant

- **Current owner:** `rt-testkit` defines payload-derived protocol fixtures;
  `rt-engine` owns real metainfo admission, peer transfer, storage
  verification, and registry updates; the `rt-metrics` integration target
  observes lifecycle results across restart.
- **Target owner:** PERF-110's paired runner must run clients against pinned
  v2/hybrid fixtures and invoke integrity/restart gates before a trial can be
  treated as comparable.
- **Invariant preserved:** the receiver gets exact expected payload bytes over
  the actual TCP peer path; a torrent that was previously complete is not
  trusted after restart until its current payload verifies; v1, pure-v2, and
  hybrid identities remain distinct, and hybrid pad bytes stay zero-filled.
- **Must not:** label local storage recheck as a peer transfer, accept the
  persisted `amount_left == 0` row without resumed verification, or infer
  filesystem durability beyond IO-009/IO-010 evidence.

### Acceptance

- Start isolated seed and receiver engines on task-scoped loopback ports.
- Materialize each seed fixture under its name-wrapped save path, leave
  receiver payloads absent, and create the expected hybrid padding file.
- Admit raw metainfo through `EngineHandle::add_torrent_raw_with_labels`,
  connect the receiver with `EngineHandle::add_peers`, and observe all three
  cases reach complete state with zero bytes remaining and exact payload-file
  bytes at the receiver. Use the public compatibility matrix and one
  initiating dial; private-torrent peer authorization is a separate policy
  regression and must not be simulated by simultaneous reciprocal dials.
- Pause the completed entries, shut down cleanly, and restore all three from
  the same database and metainfo blobs with their protocol identities and
  completed counters intact.
- Corrupt one pure-v2 payload byte between engine lifetimes. After resume, the
  pure-v2 entry must return to `Downloading` with bytes remaining; unchanged
  v1 and hybrid entries must reverify as `Seeding` with zero bytes remaining.
- Keep peer network transfer, matched goodput, resource attribution, crash-cut
  behavior, and strict durability qualification as separate evidence owned by
  PERF-110, PERF-102, IO-009, and IO-010.

### Files and compatibility

- **Expected code paths:**
  `crates/rt-metrics/tests/transfer_invariants.rs` and this spec.
- **State or wire compatibility:** regression-source changes only; no
  product format or runtime behavior changes.
- **Migration / rollback:** none.
- **Feature flag or rollout boundary:** none; paired performance results stay
  unqualified until their network payload and restart invariants pass.

### Evidence

- **Regression source added or updated:**
  `persisted_v2_completion_is_rechecked_after_restart` transfers the v1,
  pure-v2, and hybrid fixtures between production engine instances, verifies
  receiver bytes, and corrupts only pure-v2 content between restarts.
- **Command or probe to run:**
  `cargo test -p rt-metrics --test transfer_invariants -- --nocapture`
  during the requested bulk pass.
- **Expected result:** valid protocol fixtures become complete through the
  production storage path; persisted completion is restored; corrupted v2
  content is rejected after restart while the unchanged protocol cases remain
  complete.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** matched network transfer and payload
  integrity for all applicable PERF-110 client/profile pairs.

## PERF-113 — Descriptive paired scorecard

- **Priority / wave:** P1 / 7
- **Prerequisites:** PERF-101, PERF-102, PERF-104, PERF-110
- **Owning component:** certification analysis tooling.
- **Finding or capability:** comparisons must retain exact pairing, workload
  identity, accounting domains, exclusions, and uncertainty without turning
  proposed numbers into unapproved release thresholds.
- **Status:** `done`

### Scope

Read a validated manifest and its linked trial records, form comparison groups
only from identical workload and environment signatures, and calculate
pairwise client ratios with deterministic percentile-bootstrap 95% intervals
for their median. Preserve per-pair observations and reasons for excluded
attempts. The scorecard is descriptive; it does not make release decisions or
adopt the proposal in `TRANSFER_CORE_REBUILD.md`.

### Ownership and invariant

- **Current owner:** `scorecard.py` consumes immutable PERF-101 manifest
  evidence; PERF-110 remains responsible for producing randomized paired runs.
- **Target owner:** the paired runner and scorecard report remain separate
  stages so an analysis can be reproduced from a retained manifest.
- **Invariant preserved:** client comparisons use the same pair IDs and exact
  workload/environment identity; RSS, PSS, cgroup domains, CPU, I/O, and energy
  are not silently conflated.
- **Must not:** compare unpaired history, drop failed attempts silently, infer
  missing resources as zero, or treat the sample minimum as a pass/fail gate.

### Acceptance

- The input manifest and referenced trial/artifact digests validate before
  analysis.
- Only complete trials with passing applicable integrity enter comparisons;
  each excluded pair has an explicit reason.
- A paired comparison reports each ratio, median ratio, deterministic
  percentile-bootstrap 95% interval, paired count, and informational minimum
  sample status.
- Resource ratios retain their metric direction and accounting domain;
  missing counters remain `null` or are absent from that comparison.
- Client and resampling work are bounded; output is exclusive, mode `0600`, and
  conforms to `transfer-scorecard-v1`.
- The report states that no performance threshold was applied. Proposed
  thresholds require a separate decision before results are observed.
- Regression sources cover deterministic intervals, signature normalization,
  failed/incomplete exclusion, malformed pair membership, and work bounds.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/scorecard.py`, the
  scorecard schema and validator, measurement contract, harness README, and
  `scripts/test_transfer_rebuild_scorecard.py`.
- **State or wire compatibility:** additive `transfer-scorecard-v1` report;
  trial and manifest formats do not change.

## PERF-114 — Source-bound result manifest assembly

- **Priority / wave:** P0 / 8
- **Prerequisites:** PERF-101, PERF-113
- **Owning component:** certification evidence tooling.
- **Finding or capability:** text-grep certification can associate an older
  `PASS` with new code; qualification needs one immutable manifest that binds
  every attempt and referenced artifact to the exact measured source.
- **Status:** `wip`

### Scope

`manifest.py` accepts the exact clean committed worktree, every trial JSON
record, and each additional schema-kind artifact. It hashes the evidence,
validates the complete manifest and all linked files, rechecks input digests and
source identity, and publishes a new mode-`0600` manifest atomically. The
manifest records complete, incomplete, invalid, failed, and unmeasured
outcomes; it never reduces a run to a text summary.

### Ownership and invariant

- **Current owner:** `scripts/transfer_rebuild/manifest.py` assembles evidence;
  `validate_artifact.py` owns schema, link, and semantic validation.
- **Target owner:** PERF-110's paired executor emits trials and calls this
  writer; analysis and CI consume the retained manifest.
- **Invariant preserved:** every referenced file is a regular file below the
  manifest directory, outside the measured source checkout, with a checked
  SHA-256 and matching trial/source identity. A generated manifest describes
  one clean Git commit.
- **Must not:** overwrite evidence, silently omit failed attempts, accept
  symlinks/path escapes, or claim that a dirty flag identifies uncommitted
  source contents.

### Acceptance

- At least one trial is required; duplicate trial IDs and duplicate evidence
  paths are rejected.
- The writer rejects dirty worktrees, source mismatches, evidence under the
  measured checkout, non-regular or symlink paths, malformed/ambiguous JSON,
  digest changes during collection, and an existing output path.
- Before publication, trial and manifest schemas plus all referenced artifact
  digests and identities validate; source and input digests are checked again.
- Publication is exclusive, atomic, mode `0600`, and synced before success is
  reported.
- Regression sources cover all trial outcomes, clean-source binding, dirty
  source rejection, in-tree evidence, duplicate IDs, and overwrite rejection.
- Paired executor integration and local tests passed at exact source
  `354ad504`; cross-platform support, paired-client outputs, and qualification
  remain open.

### Files and compatibility

- **Expected code paths:** `scripts/transfer_rebuild/manifest.py`,
  `scripts/test_transfer_rebuild_validator.py`, the measurement contract,
  harness README, rebuild ledger, and capability summary.
- **State or wire compatibility:** no schema change; generates existing
  `transfer-manifest-v1` records.
- **Migration / rollback:** none; the writer creates a new artifact and never
  edits existing manifests.
- **Feature flag or rollout boundary:** none; paired measurements remain
  unqualified until PERF-110 produces complete, source-bound client trials.

### Evidence

- **Regression source added:** `TransferManifestWriterTests` in
  `scripts/test_transfer_rebuild_validator.py` covers all outcome values,
  digest-linked observation artifacts, source cleanliness, evidence location,
  duplicate JSON fields and trial IDs, and exclusive output.
- **Command or probe to run:** the focused manifest/validator suite during the
  requested bulk pass.
- **Executed on exact revision:** yes; the full repository Rust and transfer-harness regression suites passed at `354ad504`. Paired-run evidence remains open.
- **Artifact or report path:** paired-run manifest emission is integrated and
  its regression suite passed at `354ad504`; measured trial artifacts remain
  pending PERF-110 client execution.
- **External evidence still required:** exact matched client observations and
  trial outputs from the paired executor.
- **Migration / rollback:** none; scorecards can be regenerated from retained
  manifests and the pinned analysis revision.
- **Feature flag or rollout boundary:** no superiority or release claim until
  PERF-110 produces complete paired samples and PERF-113 analyzes them. No
  threshold is adopted here.

### Evidence

- **Regression source added or updated:**
  `scripts/test_transfer_rebuild_scorecard.py`.
- **Command or probe to run:** scorecard and artifact-validator regressions in
  the requested bulk pass.
- **Expected result:** deterministic paired ratios and intervals; malformed,
  unpaired, or incomplete evidence is rejected or explicitly excluded.
- **Executed on exact revision:** yes; the full repository Rust and transfer-harness regression suites passed at `354ad504`. Paired-run evidence remains open.
- **Artifact or report path:** pending PERF-110 paired-run artifacts; repository
  regression sources passed at `354ad504`.
- **External evidence still required:** a complete randomized paired workload
  set on the selected comparison host; scorecard source alone is not a
  performance result.

## PERF-115 — Hosted correctness and dedicated hardware lanes

- **Priority / wave:** P1 / 9
- **Prerequisites:** PERF-110, PERF-111, PERF-112, PERF-113, PERF-114
- **Owning component:** GitHub Actions qualification workflows and run scripts.
- **Status:** `wip`

### Scope

Provide repeatable hosted correctness coverage for the transfer core and its
artifact tooling, plus an explicitly opted-in dedicated-host lane for picker
and API snapshot diagnostics and the configured storage target exercise. The
hardware lane is limited to `main`, a protected GitHub environment, and the
dedicated `torrentng-transfer-perf` runner label. It retains the exact source
commit, worktree state, compiler, host, storage mount, command logs, and exit
statuses as run artifacts.

### Ownership and invariant

- **Current owner:** `.github/workflows/transfer-rebuild.yml` runs hosted
  Rust/Python correctness and, when explicitly selected, calls
  `scripts/transfer_rebuild/hardware_lane.sh` on the dedicated runner.
- **Target owner:** the same workflow publishes immutable CI evidence; the
  paired manifest remains the authority for actual PERF-110 client trials.
- **Invariant preserved:** routine hosted CI never writes to a physical
  storage target. The storage exercise requires an operator-supplied,
  disposable directory outside the checkout. Benchmark output is diagnostic
  until source, host, and environment are compared under PERF-113.
- **Must not:** expose self-hosted hardware to pull-request code, silently
  use a temporary filesystem as a physical disk, or turn benchmark output into
  a performance claim.

### Acceptance

- Hosted execution runs the DB, peer-wire, picker, uTP, storage-library,
  engine-library, native API, and 1k production-catalog mixed-load suites, plus
  every `scripts/test_transfer_rebuild_*.py` regression source.
- Dedicated execution is manual, main-only, environment-protected, and bound
  to the dedicated runner label. It records host, worktree, toolchain, and
  storage mount metadata before executing diagnostics.
- Picker and changed-snapshot benchmarks run with raw logs retained. The
  storage move/import exercise runs only against the explicit configured
  storage root; all three exit statuses are retained even when one fails.
- CI artifacts remain diagnostics. No gate, baseline, or comparative claim
  passes from workflow completion alone.
- Workflow and script changes receive syntax/security review before the first
  hosted execution; the dedicated run remains external evidence.

### Files and compatibility

- **Expected code paths:** `.github/workflows/transfer-rebuild.yml`,
  `scripts/transfer_rebuild/hardware_lane.sh`, this spec, harness README, and
  the rebuild ledger.
- **State or wire compatibility:** no runtime or artifact-schema changes.
- **Migration / rollback:** none; delete or disable this manual workflow
  without changing product behavior.
- **Feature flag or rollout boundary:** dedicated hardware execution remains
  opt-in and unavailable to untrusted pull requests.

### Evidence

- **Regression source added or updated:** workflow and hardware-lane source;
  workflow-policy and hardware-lane regressions passed in the exact 21-script
  Python suite.
- **Command or probe to run:** hosted lane first, then the protected hardware
  lane with an operator-provisioned storage root.
- **Executed on exact revision:** hosted run
  [39233029958](https://github.com/snapetech-org/TorrentNG/actions/runs/39233029958)
  passed, and Windows Smoke
  [39233812206](https://github.com/snapetech-org/TorrentNG/actions/runs/39233812206)
  passed on `94c8c1b3`; its production source matches the locally tested
  `354ad504`. The same hardware script passed twice manually on clean
  `eb5f3158`.
- **Artifact or report path:** manual hardware evidence is retained in the
  task-scoped `hardware-kspls0-eb5f3158-01/` and `-02/` directories. The
  protected hardware job was skipped because no runner is registered for its
  label, so no protected-runner artifact exists.
- **External evidence still required:** register the protected runner and
  execute the dedicated workflow job so the immutable CI hardware artifact is
  produced.

## IO-015 — Continuous io_uring completion and retained registrations

- **Priority / wave:** P1 / 10
- **Prerequisites:** IO-006, IO-009
- **Owning component:** `rt-storage::UringBackend` and its device workers.
- **Finding or capability:** F20; a slow operation must not hold completed
  I/O or newly admitted work behind a whole-batch wait.
- **Status:** `wip`

### Scope

Keep each Linux ring and its optional fixed-file and fixed-buffer registrations
alive for the worker lifetime. Refill a bounded outstanding window as CQEs
arrive, drain all currently ready completions, and preserve the portable
`pread` fallback and the existing per-device admission boundary.

### Ownership and invariant

- **Current owner:** each `UringWorker` owns its ring, pending operation map,
  fixed-file slot references, and any registered read-buffer lease; the
  `DeviceQueue` owns active and pending admission.
- **Target owner:** the same worker-local ownership remains, with CQEs driving
  progress instead of a batch-wide completion barrier.
- **Invariant preserved:** submitted reads, writes, and syncs retain their
  file, payload/frame, registration, and device ownership until their own CQE
  completes or the ring is destroyed after a fatal failure.
- **Must not:** reuse a registered file/buffer slot while a kernel operation
  may still reference it, cancel admission ownership when a caller drops its
  waiter, or raise the existing device and backend queue bounds.

### Acceptance

- A worker waits for at least one CQE, completes every ready CQE, and refills
  its outstanding set up to the 64-operation window without waiting for the
  slowest operation from a prior batch.
- An active worker uses a nonblocking receiver-lock attempt before waiting for
  ring progress, so an idle worker blocked on input cannot stall another
  worker's CQE processing.
- A closed job channel drains all submitted operations before worker exit.
- Fixed-buffer registration remains within the existing bounded memory-lock
  budget, and each fixed-file slot is cleared only after its final operation
  completes.
- Unsupported kernels, registration failures, and startup failures retain the
  positioned `pread` fallback. Short reads/writes and fatal-ring errors remain
  explicit failures with operation owners retained through ring teardown.
- Regression sources cover more than one outstanding window of concurrent
  reads, backend shutdown with pending work, fixed-slot reuse, and caller
  cancellation while an operation remains in flight.

### Files and compatibility

- **Expected code paths:** `crates/rt-storage/src/backend.rs`,
  `crates/rt-storage/src/device.rs`, `README.md`, and backend regression
  sources.
- **State or wire compatibility:** no storage file format or public wire
  changes; `TNG_STORAGE_BACKEND=auto|pread|uring` and fallback policy remain.
- **Migration / rollback:** select `pread` to use the existing portable
  worker pool; no payload or resume migration is involved.
- **Feature flag or rollout boundary:** the `uring` backend remains opt-in;
  performance and kernel-compatibility qualification are still required.

### Evidence

- **Regression source added or updated:**
  `forced_uring_recycles_its_bounded_submission_window`; existing tests cover
  read/write/sync roundtrips, registration limits, operation-held file slots,
  submission rejection, and fatal ring ownership.
- **Command or probe:** `cargo test -p rt-storage` passed in the exact-source
  workspace suite; PERF-115 hardware comparison remains open.
- **Expected result:** every submitted operation completes with exact bytes,
  registration and queue bounds remain respected, and CQE progress is not
  serialized behind the slowest operation in a prior batch.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Artifact or report path:** repository-local Rust and Python bulk logs are under `/tmp/tng-transfer-core-close-20261019/evidence/`; external acceptance remains listed below.
- **External evidence still required:** Linux kernel coverage and paired
  device workloads demonstrating a benefit without latency or durability
  regressions.

## NET-114 — Live transfer actor ownership migration

- **Priority / wave:** P1 / 11
- **Prerequisites:** NET-104, NET-105, NET-108, NET-109, NET-110,
  IO-008, IO-010
- **Owning component:** `rt-engine/src/transfer/` with v1, pure-v2, and
  hybrid peer-runtime adapters.
- **Status:** `wip`

### Scope

Move request reservation, response arbitration, and terminal request cleanup
from actor-local state into the shared transfer core in reversible protocol
slices. The live ledger owns v1/hybrid-v1 and pure-v2 peer acquisition, plus
the serial v1 webseed fallback through a temporary source connection. Partial
fastresume block state is restored into the live ledger and charged to
PeerBuffer alongside picker resume projection. Pure-v2 retains peer-local piece
assembly and uses atomic affinity checks so one connection owns each active
piece. A centralized 60-second ledger expiry routes Cancel actions to peer
tasks and drops matching actor requests. Hybrid live peers now negotiate v1 or
v2 identity and run requests, responses, upload obligations, and availability
through canonical v1 actor coordinates with wire-plane projection. Matched live
hybrid qualification remains open. Shared process/torrent request-turn admission, rate credit, and adaptive
windows are integrated across peers and webseeds. Candidate ranking remains
geometry-specific by design; hybrid parity qualification remains open.
Each pure-v2 assembly also keeps a forward request cursor over received and
on-wire blocks. Matching Cancel and Reject responses rewind the cursor. Its
received-block state is stored as a packed bitmap; both bitmap and cursor are
included in the PieceAssembly memory lease. The peer request filler prioritizes
its active assemblies, then selects uniformly among the rarest eligible pieces
in the memory-governed v2 availability index. If that index cannot be admitted,
it walks the remote bitmap with a bounded cyclic fallback.
Both active and reference pure-v2 actors prepare the target file through the
same per-file lock/cache helper before the first streamed block or verified
piece write. Storage-root changes clear that preparation cache before the
replacement actor resumes.

### Ownership and invariant

- **Current owner:** the generation-aware ledger owns live v1/hybrid-v1 and
  pure-v2 peer reservations, socket-write commits, received-block arbitration,
  timeout expiry, and completion/failure cleanup. Serial webseed requests use
  a temporary, memory-accounted ledger connection and enter the same response
  arbitration path. Fastresume partial blocks also populate ledger received
  slots under an explicit PeerBuffer lease; the picker retains its candidate
  and resume projection. V2 actor maps retain response leases and peer-local
  assemblies.
  Pure-v2 assemblies advance a request cursor past received/on-wire blocks and
  rewind it on Cancel/Reject. Their received-block bitmap is packed and both
  allocations are charged to `MemoryClass::PieceAssembly`.
- **Geometry foundation:** `PieceGeometry::block_content_ranges` projects a
  wire block into path/offset spans while retaining v1 synthetic padding;
  `HybridGeometry::blocks_match_content` identifies equivalent payload when a
  padded v1 block and its v2 block have different lengths, and
  `canonical_v1_block_for_v2`, `canonical_v2_block_for_v1`,
  `expand_v2_block_to_v1`, and `project_v1_block_to_v2` provide bidirectional
  wire and payload projection, mapped ledger extents, and zero-filled padding
  conversion. Contiguous upload payloads use a shared `Bytes` view; bounded
  copy is used only when projection spans multiple payload ranges. The pure
  `transfer::hybrid::HybridLedger` adapter now binds each connection to its
  wire plane, arbitrates both coordinate views in one canonical v1 ledger,
  returns accepted v2 bytes with v1 padding synthesized, and projects ledger
  cancellation actions back to each connection's plane. Its piece-affine
  reserve/receive methods apply the peer-local assembly rule after projecting
  coordinates, so a v1 and a v2 connection cannot split one canonical piece.
  Live hybrid actors use it through `LiveTransferLedger`: incoming peers bind
  their plane at admission, outgoing peers bind after the BEP 3 handshake, and
  the idle-only rebind guard prevents changing plane with requests outstanding.
  The actor maps Request, Cancel, Reject, and Piece coordinates at the peer
  boundary, synthesizes v1 padding blocks locally for v2 downloads, and
  projects canonical upload data and obligation replies back to v2 wire ranges.
  V2 availability bitfields omit alignment-gap pieces; download rate grants and
  transfer counters use actual wire payload lengths.
- **Target owner:** each protocol adapter submits candidates to one shared
  content owner, while immutable payload, storage completion, and durability
  evidence retain their existing explicit owners.
- **Invariant preserved:** peer requests are reserved before they enter a
  peer command queue, committed after their wire write succeeds, and reconciled
  against responses from the same connection generation. Webseed blocks are
  reserved before fetch and committed after exact response validation.
  Duplicate or stale blocks cannot enter storage; every lifecycle exit
  releases ownership.
- **Memory bound:** connection slots and per-piece ledger state are charged to
  `MemoryClass::PeerBuffer`; per-request response capacity remains charged by
  its existing lease. Pure-v2 received-block bitmap and cursor storage are
  included in the `MemoryClass::PieceAssembly` lease. The picker stays
  independently charged to PieceIndex.
- **Rollback:** remove the actor adapter and restore picker-owned request
  admission. No resume format or wire protocol change is required.

### Acceptance

- v1 uses the ledger for reserve, wire commit, response, Reject, Choke,
  timeout, disconnect, and piece completion/failure. Pure-v2 uses it for
  reserve, wire commit, response, Reject, Choke, 60-second timeout expiry,
  disconnect, and piece completion/failure.
- Pure-v2 request cursors skip received/on-wire blocks and rewind on matching
  Cancel/Reject responses.
- Pure-v2 request filling continues active assemblies before considering new
  pieces and selects uniformly among the rarest eligible remote pieces; its
  bounded cyclic fallback visits only set bits in the remote bitmap.
- Active and reference pure-v2 actors prepare parent directories and file
  length before their first block or verified-piece write; storage-root
  changes invalidate the per-file preparation cache.
- Serial v1 webseed requests reserve before fetch, commit after a validated
  response, and release on fetch failure, cancellation, or response handling;
  temporary connection and piece-state allocations are charged to
  `MemoryClass::PeerBuffer`. One transient ledger slot is reserved beyond the
  configured peer cap; peer admission itself is unchanged.
- Restored partial blocks occupy ledger received slots before live acquisition;
  a full or empty/malformed partial bitmap is not treated as verified, and
  failure to admit its state rolls the picker projection back so those blocks
  are requested again.
- Hybrid v1/v2 views arbitrate the same payload without duplicate writes or
  mismatched piece geometry.
- Hybrid piece-affine reservation and response handling apply ownership
  checks to canonical v1 pieces across both wire planes.
- The live facade admits v2-plane connections only after validated hybrid
  geometry is attached. Its plane-aware reserve, commit, response, choke,
  disconnect, expiry, verification-failure, and completion operations all
  route through the same canonical owner and return wire-plane actions. The
  production peer adapter translates request/cancel/reject/piece coordinates,
  fills v1-only padding locally, and serves projected v2 upload ranges.
- Peer request, ledger connection, piece state, source jobs, rate queues, and
  upload obligations stay within measured resource caps.
- On the live v1, hybrid-v1, hybrid-v2, and pure-v2 paths, negotiated BEP 10 `reqq`
  updates the actor request window and its ledger connection bound together,
  clamped to that actor's request-window maximum; an omitted value restores
  that local cap, while reducing the bound stops new reservations without
  revoking existing wire obligations.
- Simulator and actor regressions pass, then matched v1/v2/hybrid live
  workloads show correct payload and lifecycle parity before the reference
  paths are removed.

### Evidence

- **Regression sources:** partial-resume ledger completion and verification
  boundary, padded hybrid block-range projection and zero-fill,
  reverse mapping from canonical ledger extents to v2 wire actions and from
  canonical v1 upload payloads to shorter v2 wire payloads,
  shared hybrid ownership when v1 and v2 requests overlap one padded payload
  extent, one winning response canceling the other plane in its own
  coordinates, duplicate suppression after canonicalization, generation-safe
  plane reuse, legacy v1 disconnect isolation from v2 connections, malformed
  response lengths preserving ownership, and
  piece-affine reservation/response handling across the two planes,
  ledger failure/expiry cancellation, v1 actor lifecycle, webseed ledger
  ownership, and pure-v2
  reservation/response/action-routing/timeout sources and a request-cursor
  sources covering live-block skips, cancellation rewind, received-block word
  boundaries, packed bitmap/cursor memory accounting, and remote-availability
  set-bit iteration are present; the pure hybrid ledger adapter covers padded
  overlap, winner cancellation, zero-fill, and duplicate suppression, and
  `transfer/sim.rs` now carries 64 seeded traces for piece-affinity exclusion,
  endgame response arbitration, loser cancel projection, padding expansion,
  late responses, canonical assembly, and generation reuse. Live hybrid actor
  regressions passed in the exact-source workspace suite; matched hybrid live
  workloads remain open.
  `v1_compat_disconnect_cannot_orphan_v2_ledger_connection`
  protects a live v2 plane entry from the staged v1 cleanup adapter, and
  `peer_reqq_updates_the_live_v1_and_hybrid_ledger_bounds` and
  `v2_remote_reqq_bounds_the_peer_window_and_shared_ledger` cover negotiated
  connection limits in the live v1, hybrid-v1, and pure-v2 paths.
  `hybrid_v2_projection_maps_files_after_v1_padding` also covers v1/v2
  handshake-plane selection and rejects a truncated v2 hash without the v2
  capability bit. `hybrid_v2_upload_projects_canonical_padding_out_of_wire_payload`
  covers a canonical v1 upload extent projected to its shorter v2 payload and
  verifies the canonical buffer lease remains charged.
  `hybrid_v2_peer_loop_translates_download_and_upload_wire_ranges` covers the
  production peer loop's shortened v2 Request, zero-padded canonical Piece,
  upload obligation mapping, and projected v2 Piece response. These actor
  sources passed in the exact-source suite at `354ad504`; matched hybrid live
  workloads remain open.
  `live_hybrid_v2_plane_returns_data_in_canonical_v1_coordinates` covers the
  new live facade's v2 admission, reservation, wire commit, response, and
  canonical data projection with synthetic BEP 47 padding, then checks the
  second file's v2 piece after the preceding v1 pad extent. Live actor wiring
  regressions and simulator tests passed in the exact-main workspace.
  The transfer-invariant integration source also routes a pure-v2 receiver
  through the reference actor, removes its save tree, downloads again, checks
  the recreated payload, then restores the shared actor. Both actor paths use
  the common locked file-preparation helper; this regression passed in both
  exact-main workspace runs.
- **Command or probe:** full workspace tests on Rust 1.88.0 and 1.94.0;
  release-mode `rt-engine` library tests on Rust 1.94.0.
- **Executed on exact revision:** yes; the full Rust workspace, including live actor and simulator regression targets, passed on Rust 1.88.0 and 1.94.0 at `354ad504`. Matched live protocol workloads remain open.
- **External evidence:** matched live protocol workloads and efficiency
  comparisons remain pending.

## NET-115 — Unified source scheduling and batched progress

- **Priority / wave:** P1 / 12
- **Prerequisites:** NET-106, NET-107, NET-108, NET-114, SYS-009
- **Owning component:** v1 and pure-v2 torrent actors, then the shared transfer
  scheduler.
- **Status:** `wip`

### Scope

Schedule peer and webseed work through one bounded ownership and rate-control
path, then publish transfer progress in bounded batches. The v1 and pure-v2
actors each dispatch one bounded webseed request at a time, including while
peers are active, when fresh work is available. Both reserve through their
shared transfer ledger and global/torrent download budgets before HTTP; an
owned fetch task keeps actor commands and peer events responsive. Adapter
candidate walkers remain protocol-specific, while both sources compete for
piece ownership in the same ledger. `GlobalNetworkBudget` also caps outstanding
payload requests across the process at `clamp(max_peers * 128, 128, 32768)`
and across one torrent at `min(process_cap, 2048)`. Peer requests hold a slot
until response or terminal cleanup; webseed requests hold it until the body is
accepted or the attempt is cancelled. These non-waiting permits share one
admission path and use bounded actor retries when capacity is full. Peer
request-fill batches resolve one torrent-scoped handle. Uncontended requests
use semaphore atomics; under contention, a short demand queue grants one
admission turn per peer/webseed within a torrent and one turn per torrent
across the process. Demand expires after five seconds without retry and is
removed on source/torrent teardown. Torrent removal revokes cached handles
before dropping the registry entry. This protects a hot source from consuming
every newly freed slot while leaving protocol-specific piece ranking intact.
v1 maps multi-file blocks to bounded per-file HTTP ranges
and synthesizes BEP 47 padding locally. Pure-v2 maps its file paths to BEP 19
URLs and routes returned blocks through its normal assembly, readback, and
Merkle verification path.
Requests use the DNS-validated, address-pinned HTTP client and require an
exact partial-content range response. Cancellation and the 60-second ledger
timeout revoke the matching fetch and release its request ownership; timeout
processing wakes source selection when a peer's expired reservation makes a
piece available. Pure-v2 peer selection uses its memory-governed rarest-first
index, with bounded cyclic fallback if the index cannot be admitted; pure-v2
webseed selection has an independent cyclic sparse-index cursor. Both source
adapters share ledger ownership and active-piece priority. v1/hybrid-v1 retain
the rarest-first picker. All blocks in one active piece stay with one selected
seed, and a typed Merkle mismatch backs that seed off before retrying the piece
elsewhere.

v1 progress already batches at its five-second dirty-stat flush, and pure-v2
batches upload/download statistics at its existing five-second runtime flush.
Completion and lifecycle boundaries flush immediately.

### Ownership and invariant

- **Current owner:** each torrent actor owns its peer connections, one
  synthetic webseed ledger connection, the in-flight fetch, and piece
  verification. Fetches run in cancellable tasks while the actor remains
  available for peer and lifecycle events. Pure-v2 publishes only verified
  piece totals and persists them through `DbWorker`. `GlobalNetworkBudget`
  owns process/torrent admission turns across every peer and webseed adapter;
  each actor retains its own geometry-specific candidate walker.
- **Target owner:** `GlobalNetworkBudget` owns shared peer/webseed request
  admission and fair demand turns; the transfer ledger owns request/content
  ownership and cancellation; `RateTree` owns shared credit; actor timers own
  bounded retries. Torrent actors retain geometry-specific candidate ranking,
  verification, storage publication, and persistence ordering. These are
  cooperating bounded services rather than one scheduler with socket access.
- **Invariant preserved:** every admitted source request has one owner until
  response, cancellation, or failure. Progress totals advance only after the
  corresponding database transaction succeeds; a failed transaction restores
  the registry projection and leaves its pending counters available for retry.
- **Must not:** let one source bypass the aggregate or torrent rate budget,
  double-count retries, or hold actor lifecycle commands behind a long source
  wait. A fetch revoked by piece completion, pause, recheck, storage move, or
  shutdown must not publish its late body.

### Acceptance

- Peers and supported webseeds share bounded per-torrent and process-wide
  source admission, rate budgets, and cancellation behavior.
- Concurrent sources receive successive demand turns within their torrent,
  and torrents receive successive demand turns at the process cap; idle
  tickets expire and cannot reserve capacity.
- Source candidate ranking preserves protocol geometry, file priorities, and
  picker ownership: v1/hybrid-v1 retain rarest-first selection, pure-v2 peers
  use the v2 availability index (with cyclic fallback under memory pressure),
  and pure-v2 webseeds use an independent cyclic cursor. Common admission turns
  arbitrate contention while useful peer and webseed work can proceed
  concurrently.
- Transfer events coalesce into bounded progress publications; completion,
  pause, storage-move, recheck, and shutdown boundaries flush pending totals.
- A failed progress transaction restores the prior registry projection and
  retains each pending source total for exactly-once retry.
- Regression sources cover concurrent peer/webseed ownership, rate waits,
  cancellation including request expiry, actor responsiveness during a held
  HTTP body, v1 per-file mapping and padding synthesis, pure-v2 URL mapping,
  progress coalescing, retry, and lifecycle flushes.
- Matched live-source workloads show no missing or duplicate payload and
  bounded progress-publication latency.

### Current slice evidence

- **Regression source added or updated:** pure-v2 progress batching and
  database-failure retry remain in `torrent_task_v2.rs`. The v1 actor owns one
  bounded cancellable webseed fetch alongside peers; its held-body source
  queries and pauses the actor, then confirms a late response does not reach
  storage. v1 range mapping, padding, and file-fanout bounds remain covered in
  `torrent_task.rs`. Pure-v2 now executes one webseed request through its
  ledger, shared download budget, bounded body lease, and verified-piece
  pipeline; source tests cover single/multi-file URL mapping, live status
  snapshots, and timeout cancellation with ledger cleanup. Webseed traversal
  uses bitmap-word skips and round-robin cursor advancement; pure-v2 peer
  selection uses rarest-first availability with a memory-pressure cyclic
  fallback; peer-request expiry wakes the scheduler. A source-affinity regression source checks that
  a Merkle mismatch backs off the contributing seed. New pure-v2 regressions
  hold a real HTTP body while status and pause commands are served, then prove
  the late body does not reach storage; another sends a valid ranged response
  into a deliberately unwriteable file target and checks the failed piece
  releases ledger and assembly ownership without publishing local-have state.
  `network_budget.rs` covers common per-torrent/process request caps, source
  and torrent turn order, idle-ticket expiry, and revocation of cached handles
  after torrent removal.
  `transfer/selection.rs` covers cyclic wrap and unusable sparse candidates;
  the pure-v2 webseed path uses this policy with its own cursor, while the peer
  path uses the v2 availability index. All these
  sources passed in the exact-source bulk suite at `354ad504`.
- **Command or probe:** run with the requested bulk engine suite; do not run
  this source independently before that pass.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **Remaining:** live-source workloads. Candidate ranking
  remains protocol-specific by design: v1/hybrid-v1 use rarest-first from
  their wire availability, while pure-v2 uses its remote bitmap and piece
  geometry. The shared owner now arbitrates request turns, ledger ownership,
  rate credit, and capacity across those adapters.

## SYS-013 — Reversible live transfer-owner cutover

- **Priority / wave:** P0 / 12
- **Prerequisites:** NET-114, IO-010, SYS-005, SYS-007, SYS-009, SYS-012
- **Owning component:** `rt-db` transfer-owner journal and the engine actor
  lifecycle coordinator.
- **Status:** `wip`; the reversible path is implemented but unqualified.

### Scope

Keep the current actor available as a reference route while allowing one torrent
at a time to move to the shared-core route. The additive SQLite journal records
the owner, owner generation, transition phase, and rollback reason without
changing `user_version`. New torrents default to `shared_core`; interrupted
transitions recover to `reference` before any torrent actor is restored. The
native API exposes a mutation-authenticated
`PUT /api/v1/torrents/:hash/transfer-owner` endpoint.

For an active torrent, the engine persists `draining`, asks the current actor
to quiesce and finish its bounded work, stops it, persists `starting`, starts
the requested owner from durable metadata/state, waits for the ready signal,
then commits the new owner generation. Failure before durable commit stops the
candidate, persists rollback intent, restores the old owner, waits for it to be
ready, and only then returns the journal to stable. DHT registration follows
the stable route. A paused or dormant torrent changes its durable route without
starting an actor; later resume uses that route. Supervised exit generations
prevent a late old actor exit from removing the replacement.

### Ownership and invariant

- **Current owner:** `transfer_cutovers` is the durable selector; the engine
  lifecycle coordinator is the sole live transition writer. Both actor
  implementations remain available for rollback.
- **Target owner:** the same journal and coordinator remain authoritative; a
  candidate cannot serve payload until its actor is ready and the generation
  commit succeeds.
- **Invariant preserved:** at most one actor route owns a torrent's live
  payload at a time. Incomplete startup or cutover never silently selects the
  partially started candidate after restart.
- **Must not:** commit before the old actor drains, start both owners at once,
  discard the reference implementation, or advance SQLite `user_version` for
  this additive journal.

### Acceptance

- Route owner changes use generation-checked durable transitions and reject a
  stale or concurrent generation.
- Active switching quiesces and stops the old owner before candidate start;
  candidate readiness precedes commit.
- Injected drain, stop, spawn, ready, and commit failures restore the previous
  owner or leave an explicit rollback-required row for startup recovery.
- A lost commit reply is reconciled from the durable row before any rollback.
- Startup recovery precedes actor restoration and fences stale actor exits by
  incarnation generation.
- Paused/dormant route changes persist without starting network activity;
  subsequent activation uses the selected route.
- Native API route parsing is strict, mutation-authenticated, and bound to an
  existing torrent.

### Evidence

- **Regression sources:** database CAS/drain/commit/recovery tests, generation
  fence tests, native API route parsing/registration tests, and
  `failed_live_transfer_cutover_restores_the_previous_owner` are authored.
- **Executed on exact revision:** yes; the full Rust workspace, including the engine cutover and recovery regressions, passed on Rust 1.88.0 and 1.94.0 at `354ad504`. Live swarm crash-cut qualification remains open.
- **External qualification:** active v1, pure-v2, and hybrid cutover parity;
  injected failure/crash cuts; restart recovery; and rollback under real swarm
  activity remain required.

## Final rollout evidence contract

`qualification_record.py` creates strict, private, exclusive records for the
named qualification kinds. Each record is bound to a 40-character source
commit and lists required check IDs, each with one or more evidence digests.
The record itself is included in the immutable manifest; its check digests
must resolve to other files in that same manifest. `rollout_index.py` binds the
source commit, exact manifest bytes, exact scorecard bytes, selected candidate,
and four stage declarations outside the manifest to avoid a self-referential
digest.

The final evaluator validates the manifest and scorecard, verifies every
qualification record and cited proof digest, requires the stage-specific kinds
and checks, checks candidate completion and integrity across v1, v2, and hybrid
download and seed cases, and checks paired sample counts for goodput, RSS, and
CPU metrics. It emits `blocked` until all four stage records pass. It applies
no numeric performance threshold: decision D1 remains open, and the
descriptive scorecard must retain `performance_thresholds_applied: false`.

Qualification records must be schema-valid, source-bound, created exclusively
with mode `0600`, and link only to artifacts in the same manifest. Failed or
unrun checks cannot pass. Each SYS-014 catalog check must cite a
`transfer-catalog-load-report-v1`; that report must be source-bound, recompute
from its retained samples, and cite a `transfer-catalog-load-log-v1` in the
same manifest. Hashed declarations do not prove hardware, crash, soak, or
independent-review execution by themselves.

## IO-016 — Live-engine parity, recovery, and storage qualification

- **Priority / wave:** P0 / 13
- **Prerequisites:** IO-008–015, PERF-110, SYS-013
- **Owning component:** storage/runtime qualification and
  `scripts/transfer_rebuild/rollout_gate.py`.
- **Finding or capability:** demonstrate that the shared-core candidate
  preserves live v1/v2/hybrid behavior, storage ownership, and recovery before
  rollout.
- **Status:** `wip`; qualification artifacts have not been produced.

### Scope and invariant

Run exact-source live parity for v1, pure-v2, and hybrid-v1/v2 download and
seed cases, plus target-storage, injected-fault, crash-cut, and resume recovery
cases. Keep the reference actor selectable through each trial. The acceptance
boundary includes current storage and fast-resume ownership; it does not
authorize a format change or production default switch.

- **Invariant preserved:** only verified and durably published content is
  reported complete; a failed candidate or interrupted recovery leaves one
  known owner and a restartable state.
- **Must not:** treat loopback source tests or a local filesystem result as
  target-device or crash-recovery qualification.

### Acceptance

- `transfer-live-parity-v1` passes `live-v1-parity`, `live-v2-parity`, and
  `live-hybrid-v1-v2-parity` on the exact clean source revision.
- `transfer-storage-qualification-v1` passes `storage-fault-matrix`,
  `real-device-workload`, and `bounded-resource-ownership` with retained raw
  logs and device identity.
- `transfer-recovery-qualification-v1` passes `crash-cut-recovery`,
  `resume-restoration`, and `one-writer-recovery`; the reference route remains
  usable after each injected failure.
- `transfer-gate-policy-v1` records reviewed correctness checks and explicitly
  states that unapproved performance thresholds were not applied.
- The final evaluator accepts these proof kinds and reports IO-016 passed;
  failed, missing, or source-mismatched evidence reports blocked.

### Evidence

- **Regression source:** workspace storage/engine suites, live parity
  integration cases, crash-cut harness, and
  `scripts/test_transfer_rebuild_rollout_gate.py`.
- **Command or probe:** create source-bound qualification records, include
  them and raw evidence in the immutable manifest, assemble the rollout index,
  then run `scripts/transfer_rebuild/rollout_gate.py`.
- **Expected result:** all named checks and proof digests validate against the
  same clean source commit and the stage reports passed.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **External evidence still required:** target storage, crash-cut recovery,
  live three-protocol parity, and independently retained device evidence.

## NET-116 — Independent correctness and performance rollout gate

- **Priority / wave:** P1 / 13
- **Prerequisites:** NET-108, NET-109, NET-111, NET-113–115, PERF-112–113,
  SYS-013
- **Owning component:** transfer-core simulator, live actor adapters, and
  `scripts/transfer_rebuild/rollout_gate.py`.
- **Finding or capability:** decide whether the candidate can be enabled only
  after independent correctness and matched whole-client evidence.
- **Status:** `wip`; no matched catalog or qualification record is available.

### Scope and invariant

Combine deterministic simulator evidence, live protocol parity, and a paired
whole-client scorecard for the same clean revision. The scorecard remains
descriptive until D1 is explicitly decided.

- **Invariant preserved:** every candidate and baseline trial uses the same
  fixture, settings, resource limits, integrity rules, and observer contract.
- **Must not:** infer efficiency from a component microbenchmark, omit an
  unsupported client/profile silently, or apply thresholds after seeing data.

### Acceptance

- `transfer-live-parity-v1` passes live v1, pure-v2, and hybrid parity checks.
- `transfer-simulator-qualification-v1` passes seeded invariant replay,
  hybrid-race replay, and generation-reuse replay with the seed and artifact
  digest retained.
- `transfer-performance-policy-v1` records matched candidate/baseline
  execution, the required whole-client metrics, and the explicit
  no-unapproved-thresholds check.
- The evaluator verifies complete, integrity-passed download and seed trials
  for all three protocols and each declared candidate/baseline, plus required
  pair counts for goodput, RSS, and CPU.
- The descriptive report does not claim superiority or set
  `performance_thresholds_applied` to true without a recorded D1 decision.

### Evidence

- **Regression source:** `crates/rt-engine/src/transfer/sim.rs`, live transfer
  integration tests, PERF-110–114 scripts, and
  `scripts/test_transfer_rebuild_rollout_gate.py`.
- **Command or probe:** validate the source-bound manifest and scorecard, then
  run `scripts/transfer_rebuild/rollout_gate.py` against the matching rollout
  index.
- **Expected result:** required protocols, workloads, sample counts, proof
  kinds, source digests, and policy checks validate; otherwise the stage stays
  blocked with reasons.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **External evidence still required:** pinned client identities/settings,
  complete paired trials, and whole-client measurements.

## PERF-116 — Candidate soak, recovery, and reversible canary

- **Priority / wave:** P1 / 13
- **Prerequisites:** PERF-112, PERF-114–115, SYS-013–014
- **Owning component:** supervised runtime, owner-switch journal, and
  `scripts/transfer_rebuild/rollout_gate.py`.
- **Finding or capability:** demonstrate stable resource ownership and safe
  activation/rollback over a predeclared soak and canary.
- **Status:** `wip`; no soak or canary has been run.

### Scope and invariant

Run the declared soak profile, recovery scenarios, and reversible candidate
canary on one exact source build. Keep the reference route available and retain
the journal and process/resource observations throughout each transition.
Duration, resource limits, and workload are fixed in the qualification input
before execution; the run must not adopt an unapproved scorecard threshold.

- **Invariant preserved:** one writer owns each torrent throughout drain,
  candidate start, commit, rollback, and restart recovery.
- **Must not:** replace the reference fallback, silently restart a failed
  candidate, or call a short smoke run a soak.

### Acceptance

- `transfer-soak-qualification-v1` passes `soak-completed`,
  `integrity-preserved`, and `resource-accounting-complete` for the declared
  duration and workload, with raw samples and identity retained.
- `transfer-recovery-qualification-v1` passes crash-cut recovery, resume
  restoration, and one-writer recovery during and after the soak.
- `transfer-canary-qualification-v1` proves candidate cutover, reference
  rollback, and single-writer ownership throughout, including one injected
  failure after the old actor drains.
- `transfer-gate-policy-v1` records the reviewed correctness gate and that no
  unapproved performance threshold was applied.
- The stage evaluator accepts the same clean source, manifest, scorecard, and
  required qualification records; missing dwell or rollback evidence leaves
  it blocked.

### Evidence

- **Regression source:** SYS-013 cutover/recovery tests and
  `scripts/test_transfer_rebuild_rollout_gate.py`.
- **Command or probe:** run the predeclared soak and canary harnesses, build
  source-bound records, then evaluate the final rollout index.
- **Expected result:** all checks pass with complete raw resource samples and
  source-bound transition/recovery records.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **External evidence still required:** controlled dwell, operational recovery,
  and canary/rollback on the designated runtime environment.

## SYS-015 — Independent review and reproducible release integration

- **Priority / wave:** P0 / 14
- **Prerequisites:** SYS-013–014, NET-116, IO-016, PERF-116
- **Owning component:** release engineering, independent review, and
  `scripts/transfer_rebuild/rollout_gate.py`.
- **Finding or capability:** consolidate the completed gates into a
  reproducible, reviewable release decision.
- **Status:** `wip`; final evidence and independent signoff are absent.

### Scope and invariant

Rebuild the exact source commit, retain provenance and release notes, obtain an
independent review, and include complete-catalog mixed-load evidence. The
final decision consumes all four stage records and cannot infer a pass from
the presence of a release bundle alone.

- **Invariant preserved:** release artifacts identify the exact clean source
  and required capabilities; unresolved findings or failed gates remain
  visible and block the final decision.
- **Must not:** self-attest the independent review, bypass required checks, or
  bind stale evidence to a newer commit.

### Acceptance

- `transfer-independent-review-v1` passes `independent-reviewer-attested`,
  `review-scope-complete`, and `findings-resolved`, with reviewer identity and
  scope evidence.
- `transfer-release-bundle-v1` passes `clean-source-rebuild`,
  `artifact-provenance`, and `release-notes-and-bundle`; the release artifact
  digest is tied to the exact source commit.
- `transfer-catalog-load-v1` passes production catalog persist/restore,
  mixed native API latency retention, and host/source identity checks using a
  validated report and retained raw log.
- `transfer-gate-policy-v1` records that required correctness gates were
  reviewed and unapproved performance thresholds were not applied.
- The final evaluator reports all four stages passed and the release bundle
  is reproducible from the source-bound manifest.

### Evidence

- **Regression source:** `scripts/test_transfer_rebuild_rollout_gate.py`,
  `scripts/test_transfer_rebuild_catalog_load_report.py`, release build and
  provenance checks.
- **Command or probe:** create independent-review, release-bundle, catalog,
  and gate-policy records; assemble the final rollout index; run
  `scripts/transfer_rebuild/rollout_gate.py`.
- **Expected result:** the evaluator returns ready only when every stage
  artifact is valid, source-matched, and independently reviewed.
- **Executed on exact revision:** yes; repository Rust regressions passed on Rust 1.88.0 and 1.94.0 and all 21 transfer-rebuild Python regressions passed at `354ad504`. External acceptance remains listed below.
- **External evidence still required:** independent reviewer, reproducible
  release artifact, and complete-catalog mixed-load report.
