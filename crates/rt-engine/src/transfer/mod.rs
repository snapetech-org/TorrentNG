//! Pure transfer core (`docs/TRANSFER_CORE_REBUILD.md`, gate G1-G3).
//!
//! This module owns the *decisions* of a transfer and none of its I/O: it
//! consumes timestamped events and returns bounded actions. It holds no
//! sockets, file descriptors, database connections, sleeping futures or
//! unbounded task spawns, so every behaviour here is deterministic and can be
//! replayed by `sim`.
//!
//! * [`coords`]      typed block/piece extents, torrent-scoped peer identities,
//!   epochs and the v1 / v2 / hybrid geometry adapters
//! * [`ledger`]      the one authoritative request and content ownership ledger
//! * [`hybrid`]      a dual-wire-view adapter over one canonical hybrid ledger
//! * [`obligations`] negotiated peer-protocol request termination (BEP 6)
//! * [`upload`]      bounded, fair, byte-admitted upload service keyed by the
//!   shared torrent/connection identity
//! * [`rate`]        hierarchical global/torrent/peer rate scheduler
//! * [`window`]      adaptive request windows and refill deadlines
//! * [`sim`]         deterministic simulator and invariant oracle
//!
//! Integration into the live torrent actors is staged (NET-114). v1 and
//! pure-v2 inbound upload requests use [`obligations::InboundObligations`],
//! and v1/hybrid-v1 plus pure-v2 outbound acquisition use the generation-aware
//! ledger; the serial v1 webseed fallback also uses the ledger with a
//! temporary source connection. V1/hybrid-v1 and pure-v2 use [`window`] for
//! bounded per-peer request admission, while concurrent shared-source
//! scheduling, hybrid dual-view arbitration, and shared content completion
//! remain open. Hybrid v2 verification now resolves piece file ranges through
//! [`coords::HybridGeometry`], and live hybrid-v1 tasks use `HybridLedger` as
//! their single v1-coordinate owner. Hybrid v2-plane handshake/scheduling and
//! dual-view request ownership remain open. Unused-code lints are allowed for
//! the parts that are still only exercised by replay tests.
#![allow(dead_code)]

/// Injected monotonic time in nanoseconds. The core never reads a clock.
pub(crate) type Nanos = u64;

pub(crate) mod coords;
pub(crate) mod hybrid;
pub(crate) mod ledger;
pub(crate) mod live;
pub(crate) mod obligations;
pub(crate) mod rate;
#[cfg(test)]
pub(crate) mod sim;
pub(crate) mod upload;
pub(crate) mod window;
