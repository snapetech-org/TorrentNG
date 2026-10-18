# rt-utp

uTP packet and transport-state primitives for the TorrentNG client.

## Status: Owned windowed transport and shared runtime authored; bulk pass pending

This crate currently provides BEP 29 fixed-header parsing/encoding, full packet
payload framing, extension-chain parsing/encoding with truncation and
oversized-extension validation, selective ACK helpers, connection ID derivation,
state transitions, send-window accounting, RTT sampling, retransmit timeout
tracking, delay-based congestion-window adjustment, and an async UDP
`UtpListener`/`UtpStream` transport with SYN/STATE handshake, DATA/ACK exchange,
FIN close, bounded retransmission attempts, a capped packet/byte send window,
cumulative/SACK accounting, bounded receive reordering, and byte-stream
`read_exact` / `write_all` helpers over uTP DATA payloads. `UtpRuntime` shares
one UDP endpoint per address family across production outbound v1, pure-v2,
and metadata paths; when incoming uTP is enabled, the listener endpoint is
shared with outgoing streams. Existing routes survive a listener rebind while
the old endpoint drains, with a cap on retained endpoint generations.

TorrentNG-client uTP integration is partially active: outbound peer-wire sessions
use the shared runtime when enabled by `TNG_UTP_OUTGOING=prefer|only` or the legacy
`TNG_ENABLE_UTP_OUTGOING` flag. Prefer mode attempts uTP first and falls back to
TCP if the uTP connect fails. Pure-v2 and metadata uTP connects use the same
runtime. Incoming peer-wire sessions can use the shared listener endpoint when
enabled by `TNG_UTP_INCOMING=1`. The modified source and regression tests have not
been built or run in this worktree; protocol, cancellation, rebind, and matched
efficiency evidence remain open.

See `docs/protocol/UTP.md` for the module inventory, regression-source
inventory, public API example, and engine-integration status.
