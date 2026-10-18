# Transfer Rebuild Observability Ownership

This inventory records who owns each live metric family and what a scrape is
allowed to observe. It describes current sources, not evidence that the
transfer rebuild is qualified.

## Collection contract

The native /metrics handler renders snapshots from the live owners. It must not
construct a runtime, start a worker, open a torrent, or create a second cache
just to satisfy a scrape.

- Storage runtime state comes from StorageRuntime::initialized(). The handler
  reports whether the runtime exists and reads it only when another path has
  already initialized it.
- The descriptor-budget snapshot also has a passive accessor. Its initialized
  gauge distinguishes an absent gate; its value gauges are zero before a live
  storage path creates the gate. The scrape does not raise RLIMIT_NOFILE.
- Resource-governor state is copied from EngineStats.resources. The engine
  reads an already-created frame pool with try_global_frame_pool().
- Peer ingress, database worker, subsystem health, crash safety, API snapshot,
  egress-policy, and uTP values come from their owning snapshots or counters.
- Hot-torrent labels hash infohashes by default. Raw identifiers require the
  explicit metrics.include_torrent_ids setting.
- Missing or not-yet-created owners stay absent or report unavailable. They
  are not initialized by observation.

## Owner inventory

| Owner | Source | Metric families | Scope and known limits |
|---|---|---|---|
| Engine and torrent tasks | EngineStats from the engine read model | Torrent states, active tasks, transfer totals/rates, peer counts, tracker and DHT gauges, storage queues and operation totals, request-window changes, top hot-torrent memory | Aggregate task stats; top-torrent attribution is bounded and only covers active rows |
| Resource governor | EngineStats.resources | Total and per-class memory cap, use, pressure, and denied allocations | Counts leased allocations; it is not process RSS or cgroup memory |
| Peer ingress | PeerIngressStats snapshot | Admitted handshakes, budget rejects, read errors, timeouts, malformed handshakes | No peer-address labels |
| Database worker | EngineHandle database-worker snapshot | Queue depth/capacity, command outcomes, timeouts, cumulative queue/command/batch durations | Duration totals do not provide database latency percentiles |
| Storage schedulers | EngineStats storage snapshot | Device queueing, dirty files, read/write/hash/sync counters and latency summaries, per-class operations | Shared resources are aggregated by resource identity, not multiplied per torrent |
| Storage runtime and descriptor budget | Initialized StorageRuntime and passive process descriptor-budget snapshot | Backend capabilities, handles, frame pool, active/capacity/wait counters, descriptor-budget initialized gauge | The scrape does not initialize the runtime or gate and does not raise RLIMIT_NOFILE; descriptor values are zero until the gate exists |
| API snapshot and SSE owner | ApiRuntimeMetrics snapshot | Snapshot refresh/update/expiry, active SSE clients, estimated response bytes, disconnects | Native API owner only; compatibility API counters have their own paths |
| Crash-safety owner | CrashSafetyView | Previous-run verdict and recovery/audit counters | Read-only view; location-specific details are exposed through diagnostics |
| Egress-policy owner | egress_policy_metrics snapshot | Rejections and cache activity for outbound target policy | Global policy scope |
| uTP transport | rt-utp stats snapshot | Connects, accepts, sent/received bytes, timeouts, retransmits, routes, RTT | Transport-wide counters; no torrent label |

Storage device queue capacity and available admission slots are counted once
per physical device ID, even if mounts use different profile labels. Available
slots exclude both active operations and queued waiters; they do not report
idle backend worker count. Per-device latency projections retain profile
labels because they describe observed work, not the identity of the admission
pool.

## Current gaps

- The pure transfer ledger, upload service, and hierarchical rate scheduler
  are not wired to live actors, so their state is not represented here.
- The external PERF-102 observer supplies process and cgroup measurements;
  those values are deliberately separate from resource-governor leases.
- Exact API scrape cost under a large active catalog, mixed control load, and
  repeated scrapes still requires the SYS-014/PERF-109 measurements.
- The metrics surface has cumulative counters and selected storage buckets,
  but it does not provide tail histograms for every owner.

Memory totals have separate meanings. Governor-used bytes count active leases.
The top-torrent estimate combines owner-reported buffers, metadata estimates,
and outstanding-request receive reservations; the receive reservation can
overlap with materialized piece memory. The hot-torrent metrics expose receive
reservations and upload buffers separately so readers can inspect that overlap.
Neither metric family is a substitute for PERF-102 process, allocator, or
cgroup observations.

SYS-011 remains in progress until the fresh-process passivity regression and
mixed-load collection behavior are reviewed together. This inventory does not
turn the current metric families into a gate or capacity claim.
