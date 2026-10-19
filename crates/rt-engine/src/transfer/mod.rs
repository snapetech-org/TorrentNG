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
//! * [`upload_scheduler`] process-wide fair admission for live upload reads
//! * [`rate`]        hierarchical global/torrent/peer rate scheduler
//! * [`selection`]   cyclic traversal of sparse source-candidate indexes
//! * [`window`]      adaptive request windows and refill deadlines
//! * [`sim`]         deterministic simulator and invariant oracle
//!
//! The live v1/hybrid and pure-v2 torrent actors use the shared owners directly.
//! Their inbound upload lifecycle and process-wide read/byte admission are
//! registered together through [`shared_upload::SharedUploadService`]; the
//! actor retains only its socket and ordered response work. The pure
//! [`upload::UploadService`] remains a deterministic policy reference for
//! replay and parity checks. v1 and pure-v2 outbound acquisition use the generation-aware
//! ledger; the serial v1 webseed fallback also uses the ledger with a
//! temporary source connection. Peer and webseed blocks now also hold common
//! process/torrent request permits and hierarchical download credit until the
//! response or a terminal outcome. V1/hybrid-v1 and pure-v2 use [`window`]
//! for bounded per-peer request admission. Peer ranking uses an array-backed
//! rarity index in each protocol's own piece space, with no cross-plane index
//! equivalence. Pure-v2 webseeds use [`selection::first_cyclic_candidate`]
//! over their sparse local-have bitmap. `GlobalNetworkBudget` grants
//! peer/webseed request turns per torrent and torrent turns process-wide when
//! source admission is contended.
//! Piece assembly and integrity verification remain actor-owned because v1,
//! hybrid, and pure-v2 use different geometry and completion rules; each live
//! actor applies storage/hash results behind the same generation-fenced ledger.
//! Hybrid v2 verification resolves piece file ranges through
//! [`coords::HybridGeometry`], and live hybrid actors use
//! `HybridLedger` to map both wire planes to canonical v1 ownership. Hybrid
//! handshake and request scheduling are wired for the live adapter. Their
//! regression sources and bulk qualification remain open. Unused-code lints
//! are allowed for the parts that are still only exercised by replay tests.
#![allow(dead_code)]

/// Injected monotonic time in nanoseconds. The core never reads a clock.
pub(crate) type Nanos = u64;

pub(crate) mod coords;
pub(crate) mod hybrid;
pub(crate) mod ledger;
pub(crate) mod live;
pub(crate) mod obligations;
pub(crate) mod rate;
pub(crate) mod selection;
pub(crate) mod shared_upload;
#[cfg(test)]
pub(crate) mod sim;
pub(crate) mod upload;
pub(crate) mod upload_scheduler;
pub(crate) mod window;
