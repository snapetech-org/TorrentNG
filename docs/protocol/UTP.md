# BEP 29 uTP Status

TorrentNG now has a real `rt-utp` protocol crate, but the TorrentNG client does
not treat the packet codec alone as full application support. The distinction
matters: the crate can open and exchange uTP packets over UDP, while the
TorrentNG client capability only reports full transport when torrent peer-wire or
metadata paths can actually use those streams.

## Implemented In `rt-utp`

- BEP 29 fixed header parsing and encoding for SYN, STATE, DATA, FIN, and RESET.
- Packet payload framing and extension-chain parsing/encoding.
- Selective ACK extension bitset helpers.
- Initiator and acceptor connection ID derivation.
- Connection state transitions for SYN, STATE, DATA, FIN, and RESET.
- ACK validation, in-flight byte accounting, advertised-window handling, and
  RTT/retransmit-timeout sampling.
- Delay-based congestion-window adjustment with a minimum MTU floor and timeout
  backoff.
- Async UDP `UtpListener` and `UtpStream` primitives:
  - listener bind and accept;
  - client connect;
  - SYN/STATE handshake;
  - bounded windowed DATA send and cumulative/SACK acknowledgement;
  - payload receive and ACK response;
  - FIN close;
  - bounded retransmission attempts.
- Shared UDP `UtpEndpoint` and engine-owned `UtpRuntime`:
  - binds one UDP socket for the peer port;
  - demultiplexes packets by remote address and receive connection ID;
  - multiplexes outbound v1, pure-v2, and metadata streams on one endpoint per
    address family;
  - shares the incoming listener endpoint with outbound connections when
    incoming uTP is enabled;
  - retains a capped number of retired endpoint generations during listener
    rebind so established streams can drain;
  - accepts multiple incoming uTP streams without consuming the listener socket;
  - routes DATA/FIN/STATE packets into per-stream bounded queues;
  - shares one ordered deadline queue and timer task per endpoint, with
    per-stream ready notifications for routed receive deadlines.
- Process-level uTP counters and gauges exported through TorrentNG Prometheus
  metrics: connects, accepts, sent/received bytes, send/receive timeouts,
  retransmissions, route drops, RTT, RTT variance, retransmit timeout,
  congestion window, delay samples, and bytes in flight.
- Byte-stream helpers over DATA payloads:
  - `write_all` chunks arbitrary byte slices across uTP DATA packets;
  - `read_exact` buffers received DATA payloads and can satisfy reads across
    packet boundaries, which is the bridge needed for peer-wire handshakes and
    length-prefixed messages.

## Regression Source Inventory

The local `rt-utp` test suite covers:

- header and packet roundtrips;
- malformed header, version, type, extension header, extension payload, and
  oversized extension errors;
- selective ACK bitset generation and decoding;
- initiator/acceptor connection ID rules;
- SYN-to-STATE establishment;
- DATA sequence advancement and receive ACK updates;
- FIN/RESET state handling through the connection state machine;
- ACK out-of-window rejection and zero-ACK startup handling;
- congestion-window growth, reduction, and timeout floor behavior;
- loopback UDP transport handshake, payload exchange, ACK, and close;
- byte-stream reads spanning multiple uTP payload chunks;
- capped packet/byte window accounting, cumulative and selective ACK, and
  wrap-safe sequence handling;
- bounded out-of-order delivery, multi-packet window fill, and shared-runtime
  socket reuse across listener rebind.
- deadline queue replacement, cancellation, and expiry ordering.

These regression sources have not been run against the current worktree. The
updated stream path fills a bounded send window, applies SACK accounting,
buffers in-window reordered payloads, and retains retransmission bytes across
canceled writes. Production outbound connects use `UtpRuntime`; standalone
callers can still use `UtpStream::connect` directly. The central deadline and
ready-queue scheduler is now authored for shared endpoints; standalone streams
retain their direct timeout path. Scheduler runtime behavior remains
unqualified until the planned bulk pass. Matched efficiency evidence also
remains open.

## Transfer Rebuild Backend Decision

NET-112 keeps the protocol core in TorrentNG. The reviewed `urtorrent-utp`
releases (0.14.2 and 0.14.3) require Rust 1.98 according to Cargo metadata,
while the native workspace supports Rust 1.88. They are not dependencies.
Their window, SACK, reorder, timeout, and MTU behavior remain useful protocol
references. TorrentNG will retain control of UDP sockets, peer scheduling,
buffer admission, memory accounting, and runtime portability. The full
`urtorrent` engine is also not selected because its package requires Linux
6.1+ and io_uring. The owned implementation and shared-socket runtime are not
qualified yet; correctness and efficiency comparison are deferred to the
requested bulk pass. See
[`TRANSFER_CORE_UTP_EVALUATION.md`](../TRANSFER_CORE_UTP_EVALUATION.md) for the
source comparison and acceptance criteria.

Run:

```sh
cargo test -p rt-utp
```

## Public API

Minimal loopback-style usage:

```rust
use rt_utp::{UtpListener, UtpStream};

let listener = UtpListener::bind("127.0.0.1:0".parse().unwrap()).await?;
let addr = listener.local_addr()?;

let server = tokio::spawn(async move {
    let mut stream = listener.accept().await?;
    let payload = stream.recv().await?;
    stream.send(&payload).await?;
    stream.close().await
});

let mut client = UtpStream::connect(addr).await?;
client.send(b"piece block").await?;
let echoed = client.recv().await?;
client.close().await?;
server.await??;
```

For peer-wire integration, callers can use the byte-stream helpers:

```rust
let mut hs = [0u8; 68];
stream.write_all(&handshake_bytes).await?;
stream.read_exact(&mut hs).await?;
```

`UtpTransportConfig` exposes handshake timeout, I/O timeout, max datagram size,
and retransmission-attempt bounds.

## `/health` uTP Transport Capability

The runtime capability is intentionally stricter than crate capability. It
means peers can transfer torrent data or metadata through the TorrentNG
client, not merely that the protocol crate can exchange UDP packets.

`/health` now reports `networking.utp_transport=true` whenever at least one
runtime path can use uTP:

- outbound peer-wire is enabled by `TNG_UTP_OUTGOING` or by the default `auto`
  policy;
- metadata fetch is enabled by `TNG_UTP_METADATA` or the legacy outgoing flag;
- incoming uTP accepts are enabled by `TNG_UTP_INCOMING=1`.

If operators force TCP-only mode with `TNG_UTP_OUTGOING=tcp-only`, leave
metadata uTP off, and do not enable incoming uTP, `/health` reports
`networking.utp_transport=false`.

The TorrentNG-client health capability surface also reports:

- `networking.utp_udp_stream=true`: `rt-utp` has async UDP stream primitives.
- `networking.utp_outgoing_opt_in=true`: the engine contains an explicit
  outbound uTP peer-wire path.
- `networking.utp_incoming_opt_in=true`: the engine can bind a shared incoming
  uTP peer endpoint when `TNG_UTP_INCOMING=1` is set.
- `networking.utp_outgoing_policy`: `TNG_UTP_OUTGOING` when set, otherwise
  `prefer` for the legacy `TNG_ENABLE_UTP_OUTGOING` flag or `auto`.
- `networking.utp_outgoing_enabled`: whether outbound uTP may be attempted for
  at least one peer source under the current policy.
- `networking.utp_metadata_policy`: `TNG_UTP_METADATA` when set, otherwise
  `TNG_UTP_OUTGOING`, legacy `prefer`, or `off`.
- `networking.utp_metadata_enabled`: whether metadata fetch can attempt uTP.
- `networking.utp_incoming_enabled`: whether `TNG_UTP_INCOMING` is currently
  enabled for the engine process. This is a boolean listener switch; use
  `1`, `true`, `yes`, or `on`.
- `networking.utp_transport_paths`: the runtime paths that can currently use
  uTP, drawn from `outgoing_peer_wire`, `metadata_fetch`, and
  `incoming_peer_wire`.

## Engine Integration Progress

The engine peer loop now has a transport-neutral `PeerIo` adapter and an
outbound `UtpStream` peer-wire path. The default `TNG_UTP_OUTGOING=auto`
policy keeps tracker-discovered peers on TCP, attempts uTP first for DHT, PEX,
and manually added peers, and forces TCP for private torrents. Explicit
`TNG_UTP_OUTGOING=prefer` attempts uTP for eligible peers and falls back to TCP
if the uTP connect fails or does not complete within three seconds;
`TNG_UTP_OUTGOING=only` disables that fallback and returns a bounded connection
failure after the same three-second attempt. The legacy
`TNG_ENABLE_UTP_OUTGOING` flag maps to `prefer`. The same connect deadline is
used by v1, pure-v2, reference-actor, and metadata-fetch paths so a silent UDP
peer cannot hold a transfer slot indefinitely before TCP fallback.
The engine also has an incoming shared `UtpEndpoint` peer listener behind
`TNG_UTP_INCOMING=1`; accepted streams are routed by peer-wire handshake to the
same torrent tasks as TCP peers. Magnet metadata fetch can use uTP through the
same byte-stream adapter when `TNG_UTP_METADATA=prefer|only` is set; if that is
not set it follows `TNG_UTP_OUTGOING`, then the legacy
`TNG_ENABLE_UTP_OUTGOING` flag.

Production interop, alert thresholds, dashboards, and tracker/DHT transport
preference evidence remain release-quality work, but they no longer hide the
implemented app-level uTP transport from the runtime capability surface.
