//! Pure transfer core (`docs/TRANSFER_CORE_REBUILD.md`, gate G1-G3).
//!
//! This module owns the *decisions* of a transfer and none of its I/O: it
//! consumes timestamped events and returns bounded actions. It holds no
//! sockets, file descriptors, database connections, sleeping futures or
//! unbounded task spawns, so every behaviour here is deterministic and can be
//! replayed by `sim`.
//!
//! * [`coords`]      typed block/piece extents, connection identities, epochs and
//!   the v1 / v2 / hybrid geometry adapters
//! * [`ledger`]      the one authoritative request and content ownership ledger
//! * [`obligations`] negotiated peer-protocol request termination (BEP 6)
//! * [`upload`]      bounded, fair, byte-admitted upload service
//! * [`rate`]        hierarchical global/torrent/peer rate scheduler
//! * [`window`]      adaptive request windows and refill deadlines
//! * [`sim`]         deterministic simulator and invariant oracle
//!
//! Integration into the live torrent actors is staged (NET-114). v1 and
//! pure-v2 inbound upload requests use [`obligations::InboundObligations`],
//! and v1/hybrid-v1 plus pure-v2 outbound acquisition use the generation-aware
//! ledger. V1/hybrid-v1 and pure-v2 also use [`window`] for bounded per-peer
//! request admission, while hybrid dual-view arbitration and shared content
//! completion remain open. Unused-code lints are allowed for the parts that
//! are still only exercised by replay tests.
#![allow(dead_code)]

/// Injected monotonic time in nanoseconds. The core never reads a clock.
pub(crate) type Nanos = u64;

pub(crate) mod coords;
pub(crate) mod ledger;
pub(crate) mod obligations;
pub(crate) mod rate;
#[cfg(test)]
pub(crate) mod sim;
pub(crate) mod upload;
pub(crate) mod window;
