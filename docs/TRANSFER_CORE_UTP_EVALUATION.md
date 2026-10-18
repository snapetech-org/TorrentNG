# uTP Implementation Evaluation

Status: **TorrentNG-owned compatible implementation is in progress and wired
through production peer paths; protocol correctness and efficiency remain
unqualified until the requested bulk pass.** This is a source review and
architecture decision, not benchmark evidence.

## Decision

Keep the uTP protocol and runtime inside `rt-utp`, behind its peer-stream API.
Retain the existing transport controls and TCP fallback while replacing the
stop-and-wait send path, cumulative-only ACK accounting, and drop-on-reorder
receive path with bounded windowed state. The engine now routes v1, pure-v2,
and metadata outbound uTP through a shared runtime; incoming peers share the
listener endpoint when enabled. The runtime keeps a small retired-endpoint
window so established streams survive listener rebinds. Deadline scheduling,
aggregate admission evidence, protocol qualification, and matched efficiency
results remain open. Do not change rollout defaults before NET-116.

This is a compatibility and ownership decision, not a claim that an in-house
implementation is automatically faster or more correct. The aim is to remove
the observed one-packet send ceiling and repeated per-peer socket/receive-task
overhead without adding a dependency that raises the native workspace MSRV.
Use the maintained implementation below as a behavioral and performance
reference, then compare matched whole-client results before claiming a gain.

## Maintained implementation reviewed

`urtorrent-utp` exposes a sans-IO socket state machine with windowed sending,
SACK, reordering, retransmission, congestion control, and MTU discovery. Its
documentation says the protocol behavior was ported from libtorrent 2.0 and
wire shapes checked against captured libtorrent traffic. Its separate
`Manager` routes multiple peers through a UDP port, but the reviewed source
scans the connection table for timer and outgoing work. The socket API also
documents payload copies and defaults to a 1 MiB receive buffer per peer.
These are useful reference points for behavior, limits, and measurement.

Cargo metadata resolved both reviewed releases, 0.14.2 and 0.14.3, with
`rust-version = 1.98`. TorrentNG's native workspace floor is Rust 1.88. The
crate therefore cannot be used directly unless TorrentNG intentionally raises
its compiler floor, which is not justified for this feature. The package's
fresh release history is another reason to isolate the design behind
TorrentNG-owned APIs rather than make runtime compatibility depend on it.

The full `urtorrent` application is not a candidate runtime: its package
requires Linux 6.1+ and io_uring, while TorrentNG supports portable Tokio and
positioned-I/O paths. Learn from the protocol and manager behavior without
depending on or copying the application.

Primary references:

- [`urtorrent-utp` crate documentation](https://docs.rs/urtorrent-utp/latest/urtorrent_utp/)
- [`Socket` API](https://docs.rs/urtorrent-utp/latest/urtorrent_utp/socket/struct.Socket.html)
- [`Manager` API](https://docs.rs/urtorrent-utp/latest/urtorrent_utp/manager/struct.Manager.html)
- [`Socket` source and copy accounting](https://docs.rs/urtorrent-utp/latest/src/urtorrent_utp/socket.rs.html)
- [`Manager` source](https://docs.rs/urtorrent-utp/latest/src/urtorrent_utp/manager.rs.html)
- [`urtorrent` package requirements](https://docs.rs/crate/urtorrent/latest)
- [upstream repository and release activity](https://github.com/giejqf/urtorrent)

## Local implementation review

TorrentNG's implementation already supplies BEP 29 packet parsing, connection
IDs, a congestion controller, RTT/timeout estimates, configurable policy,
metrics, a bounded inbound endpoint, and peer-wire stream integration. The
review found these specific bottlenecks and correctness gaps:

| Area | Current `rt-utp` | Target design |
|---|---|---|
| Send progress | Window fill and stream-owned retransmission packets are implemented; bulk validation remains open | Keep the congestion/remote window full and retransmit bounded unacknowledged packets |
| ACK accounting | Per-sequence byte accounting and incoming SACK processing are implemented; bulk validation remains open | Debit each cumulative or selective ACK exactly once |
| Reordering | Bounded receive-window buffering and ordered delivery are implemented; bulk validation remains open | Deliver accepted payloads in sequence and keep their memory charged until read |
| Outgoing sockets | Production v1, pure-v2, and metadata paths use the shared runtime; endpoint generations are capped | One shared runtime/socket per address family in steady state |
| Receive execution | One endpoint receive loop demultiplexes bounded per-stream queues; each stream still owns its I/O and retransmit waits | Bound dispatch and timer work without per-peer UDP sockets or unbounded queues |
| Memory | Application receive buffer is bounded per stream | Aggregate runtime budget covers app reads, reorder bytes, retransmit bytes, queued writes, and packet metadata |
| Scheduling | Per-stream waits and receive routing | Earliest-deadline timer heap and bounded ready queue, with work quanta for fairness |

Windowing is not allowed to weaken TorrentNG's ownership rules. Bytes retained
for retransmission remain charged until ACK or connection teardown; received
out-of-order bytes remain charged until delivered or dropped; canceled writer
futures cannot release capacity while protocol state still owns the data.
Admission is bounded at both peer and runtime levels.

## Implementation and acceptance

1. Add a bounded send window keyed by wrap-safe sequence numbers. It tracks
   payload bytes per packet and distinguishes cumulative ACK from SACK so
   congestion and memory accounting debit only newly acknowledged bytes.
2. Add a fixed-capacity reorder window. It accepts packets only within the
   advertised byte/sequence window, exposes selective ACK bits, and releases
   contiguous payloads in wire order.
3. The stream adapter now fills its available window, keeps retransmit packets
   on the stream across canceled writes, and buffers reordering within bounded
   slots. The regression sources are authored but unrun.
4. NET-113 now has an engine-owned runtime, bounded endpoint admission, one
   receive loop per active family endpoint, and production v1, pure-v2, and
   metadata integration. Deadline/ready scheduling, rebind stress, and
   aggregate memory qualification remain open. Existing uTP controls and TCP
   fallback are preserved.
5. Keep the implementation behind existing uTP controls until deterministic protocol
   coverage and matched efficiency evidence accept it. Do not run two protocol
   writers for one peer.

Regression sources must cover handshake, multi-packet window refill, partial
cumulative ACK, SACK holes, packet loss/retransmission, out-of-order and
duplicate delivery, sequence wrap, zero-window recovery, duplicate SYN,
connection admission limits, cancellation, and multiple peers sharing one
port. The seeded transfer simulator and a lossy/reordered local UDP harness
must agree on byte and connection ownership.

Efficiency comparisons must use identical payloads, peer inputs, and settings,
and report verified goodput and completion time, CPU per verified GiB, process
and cgroup anonymous memory, socket/file-descriptor count, payload copies,
retransmissions, queue depth, and delivery latency. The current stream path is
the rollback/reference path until NET-116 accepts the candidate. Source review
does not establish a performance win.
