//! The one authoritative request and content ownership ledger (NET-104).
//!
//! Per-peer assemblies and per-peer outstanding maps let two peers fetch the
//! same missing block with no torrent-wide owner, and let a disconnect, choke
//! or timeout lose track of what was in flight. This ledger replaces them for
//! every protocol plane: each block of an in-progress piece is `Missing`,
//! `Reserved` by one or more connections (more than one only in endgame) or
//! `Received`, and every transition that changes ownership is atomic.
//!
//! Key rules (see `docs/TRANSFER_CORE_REBUILD.md` section 2 and the audit's
//! F03/F04/F08):
//!
//! * A socket address is not a connection identity; [`ConnectionId`] carries a
//!   generation, so events from a replaced connection cannot mutate its
//!   successor.
//! * The writer task reports `commit` once a request is actually on the wire,
//!   and the reader may deliver the response *before* that notification
//!   reaches the core. The ledger accepts the data and swallows the later
//!   commit exactly once; it never double-debits and never rejects legitimate
//!   data because two task notifications raced.
//! * Internal generations are not transmitted: a late reply with the same
//!   coordinates cannot say which retry it belongs to. Released blocks keep a
//!   bounded per-connection *tombstone* so useful late data is accepted while
//!   a stranger's unsolicited data is not.
//! * An epoch change (move, recheck, content change) quiesces all outstanding
//!   work; nothing issued under the old epoch completes into the new one.

use std::collections::{HashMap, HashSet, VecDeque};

use super::coords::{BlockExtent, ConnectionId, Epoch, GeometryError, PieceExtent, PieceGeometry};
use super::Nanos;

/// Default ceiling on requests one connection may have outstanding.
pub const DEFAULT_REMOTE_REQQ: usize = 250;
/// Hard ceiling on request and tombstone state retained for one connection.
pub const MAX_REMOTE_REQQ: usize = 1_024;
/// Hard ceiling on live connection state retained by one ledger.
pub const MAX_CONNECTIONS: usize = 4_096;
/// Default bound on pieces being acquired at once.
pub const DEFAULT_MAX_IN_PROGRESS_PIECES: usize = 1024;
/// Hard logical-memory ceiling for in-progress piece state.
pub const MAX_IN_PROGRESS_BYTES: usize = 64 * 1024 * 1024;
/// Hard ceiling on request-owner entries across the torrent.
pub const MAX_IN_FLIGHT: usize = 65_536;
/// Bound the number of connections that may own one endgame block.
const MAX_ENDGAME_OWNERS_PER_BLOCK: usize = 4;
/// Hard ceiling for per-connection late-event history, even when a peer
/// advertises an implausibly large request queue.
const MAX_RECENT_BLOCKS_PER_CONNECTION: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Tell `conn` we no longer want `block` (a Cancel message, or dropping a
    /// request that has not been written yet).
    Cancel {
        conn: ConnectionId,
        block: BlockExtent,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveError {
    UnknownConnection,
    Invalid(GeometryError),
    /// The piece was already received (or completed) in full.
    AlreadyReceived,
    /// Another connection owns it and this reservation is not endgame.
    OwnedByOther,
    /// Another connection has activity in this piece. Protocol adapters with
    /// peer-local whole-piece assemblies must keep the piece on that peer.
    PieceOwnedByOther,
    /// This connection already owns the block.
    AlreadyOwned,
    ConnectionFull,
    GlobalRequestLimit,
    TooManyOwners,
    TooManyPieces,
    InProgressByteLimit,
    /// The piece is complete and awaiting verification.
    AwaitingVerification,
    /// The piece is already complete and verified.
    Have,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReceiveOutcome {
    /// New content. `late` is true when it arrived for a block this
    /// connection no longer owned but had a tombstone for.
    Accepted {
        completed: Option<PieceExtent>,
        late: bool,
        actions: Vec<Action>,
    },
    /// Valid, but the block was already received: wasted traffic.
    Duplicate,
    /// A valid late response was dropped to preserve a protocol adapter's
    /// connection-affine whole-piece assembly while another connection owns
    /// activity in that piece.
    IgnoredForPieceAffinity,
    /// Nobody asked this connection for it.
    Unsolicited,
    StaleConnection,
    Invalid(GeometryError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitOutcome {
    /// The request is now on the wire.
    OnWire,
    /// The response had already been accepted; this commit was swallowed.
    Reconciled,
    /// The ledger no longer owns this request (cancelled, released). The
    /// returned actions cancel it at the remote.
    Orphaned,
    StaleConnection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectOutcome {
    Released,
    Unknown,
    StaleConnection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReserveMode {
    /// Only blocks nobody owns.
    Exclusive,
    /// Endgame: a block already requested from other connections may also be
    /// requested here (never twice from the same connection).
    Endgame,
}

#[derive(Debug, Clone)]
struct Owner {
    conn: ConnectionId,
    /// `None` until the writer confirms the request is on the wire.
    wire_at: Option<Nanos>,
}

#[derive(Debug, Clone)]
enum Slot {
    Missing,
    Reserved(Vec<Owner>),
    Received,
}

#[derive(Debug)]
struct PieceState {
    slots: Vec<Slot>,
    sources: Vec<Option<ConnectionId>>,
    received: u32,
    awaiting_verification: bool,
    budget_bytes: usize,
}

impl PieceState {
    fn new(blocks: u32) -> Self {
        let blocks = blocks as usize;
        Self {
            slots: vec![Slot::Missing; blocks],
            sources: vec![None; blocks],
            received: 0,
            awaiting_verification: false,
            budget_bytes: Self::estimated_bytes(blocks),
        }
    }

    fn estimated_bytes(blocks: usize) -> usize {
        std::mem::size_of::<Self>()
            .saturating_add(blocks.saturating_mul(std::mem::size_of::<Slot>()))
            .saturating_add(blocks.saturating_mul(std::mem::size_of::<Option<ConnectionId>>()))
            // A reserved slot owns a small Vec. Charge for its maximum bounded
            // endgame owner capacity, not only its inline Vec header.
            .saturating_add(
                blocks
                    .saturating_mul(MAX_ENDGAME_OWNERS_PER_BLOCK)
                    .saturating_mul(std::mem::size_of::<Owner>()),
            )
            // Include a conservative hash-table/key allowance for this state.
            .saturating_add(std::mem::size_of::<(u32, PieceState)>() + 32)
    }

    fn is_complete(&self) -> bool {
        self.received as usize == self.slots.len()
    }
}

#[derive(Debug)]
struct ConnState {
    outstanding: HashSet<BlockExtent>,
    reqq: usize,
    tombstones: VecDeque<BlockExtent>,
    tombstone_set: HashSet<BlockExtent>,
    /// Bounded blocks whose response was accepted before the writer's commit.
    pending_commit_ignores: VecDeque<BlockExtent>,
    pending_commit_ignore_set: HashSet<BlockExtent>,
}

impl ConnState {
    fn new(reqq: usize) -> Self {
        Self {
            outstanding: HashSet::new(),
            reqq,
            tombstones: VecDeque::new(),
            tombstone_set: HashSet::new(),
            pending_commit_ignores: VecDeque::new(),
            pending_commit_ignore_set: HashSet::new(),
        }
    }

    fn tombstone_cap(&self) -> usize {
        self.reqq
            .saturating_mul(2)
            .clamp(64, MAX_RECENT_BLOCKS_PER_CONNECTION)
    }

    fn add_pending_commit_ignore(&mut self, block: BlockExtent) {
        if !self.pending_commit_ignore_set.insert(block) {
            return;
        }
        self.pending_commit_ignores.push_back(block);
        while self.pending_commit_ignores.len() > self.tombstone_cap() {
            if let Some(old) = self.pending_commit_ignores.pop_front() {
                self.pending_commit_ignore_set.remove(&old);
            }
        }
    }

    fn take_pending_commit_ignore(&mut self, block: &BlockExtent) -> bool {
        if self.pending_commit_ignore_set.remove(block) {
            self.pending_commit_ignores
                .retain(|candidate| candidate != block);
            true
        } else {
            false
        }
    }

    fn trim_history(&mut self) {
        while self.tombstones.len() > self.tombstone_cap() {
            if let Some(old) = self.tombstones.pop_front() {
                self.tombstone_set.remove(&old);
            }
        }
        while self.pending_commit_ignores.len() > self.tombstone_cap() {
            if let Some(old) = self.pending_commit_ignores.pop_front() {
                self.pending_commit_ignore_set.remove(&old);
            }
        }
    }

    fn add_tombstone(&mut self, block: BlockExtent) {
        if !self.tombstone_set.insert(block) {
            return;
        }
        self.tombstones.push_back(block);
        while self.tombstones.len() > self.tombstone_cap() {
            if let Some(old) = self.tombstones.pop_front() {
                self.tombstone_set.remove(&old);
            }
        }
    }

    fn take_tombstone(&mut self, block: &BlockExtent) -> bool {
        if self.tombstone_set.remove(block) {
            self.tombstones.retain(|candidate| candidate != block);
            true
        } else {
            false
        }
    }
}

struct ConnEntry {
    generation: u32,
    state: Option<ConnState>,
}

/// Packed per-piece flags (verified-have).
#[derive(Debug, Default)]
struct HaveSet {
    words: Vec<u64>,
}

impl HaveSet {
    fn new(len: u32) -> Self {
        Self {
            words: vec![0; (len as usize).div_ceil(64)],
        }
    }

    fn get(&self, piece: u32) -> bool {
        self.words
            .get(piece as usize / 64)
            .is_some_and(|word| word & (1 << (piece % 64)) != 0)
    }

    fn set(&mut self, piece: u32) {
        if let Some(word) = self.words.get_mut(piece as usize / 64) {
            *word |= 1 << (piece % 64);
        }
    }

    fn clear(&mut self, piece: u32) {
        if let Some(word) = self.words.get_mut(piece as usize / 64) {
            *word &= !(1 << (piece % 64));
        }
    }
}

pub struct Ledger<G: PieceGeometry> {
    geometry: G,
    epoch: Epoch,
    conns: Vec<ConnEntry>,
    free_slots: Vec<u32>,
    pieces: HashMap<u32, PieceState>,
    have: HaveSet,
    max_connections: usize,
    active_connections: usize,
    max_in_progress: usize,
    max_in_progress_bytes: usize,
    max_in_flight: usize,
    in_progress_bytes: usize,
    in_flight: usize,
}

impl<G: PieceGeometry> Ledger<G> {
    /// Conservative fixed state for one live connection slot. Per-request
    /// owner and tombstone entries are covered by the peer response leases;
    /// runtime adapters can include this baseline in their peer admission.
    pub fn estimated_connection_state_bytes() -> usize {
        std::mem::size_of::<ConnEntry>()
            .saturating_add(std::mem::size_of::<ConnState>())
            .saturating_add(std::mem::size_of::<u32>())
    }

    pub fn new(geometry: G) -> Self {
        let pieces = geometry.piece_count();
        Self {
            geometry,
            epoch: Epoch::default(),
            conns: Vec::new(),
            free_slots: Vec::new(),
            pieces: HashMap::new(),
            have: HaveSet::new(pieces),
            max_connections: MAX_CONNECTIONS,
            active_connections: 0,
            max_in_progress: DEFAULT_MAX_IN_PROGRESS_PIECES,
            max_in_progress_bytes: MAX_IN_PROGRESS_BYTES,
            max_in_flight: MAX_IN_FLIGHT,
            in_progress_bytes: 0,
            in_flight: 0,
        }
    }

    pub fn with_max_in_progress(mut self, max: usize) -> Self {
        self.max_in_progress = max.clamp(1, DEFAULT_MAX_IN_PROGRESS_PIECES);
        self
    }

    /// Set a lower live-connection bound before registering peers.
    pub fn with_max_connections(mut self, max: usize) -> Self {
        self.max_connections = max.min(MAX_CONNECTIONS);
        self
    }

    /// Set a lower logical state budget before opening pieces.
    pub fn with_max_in_progress_bytes(mut self, max: usize) -> Self {
        self.max_in_progress_bytes = max.min(MAX_IN_PROGRESS_BYTES);
        self
    }

    /// Set a lower torrent-wide bound for in-flight request ownership.
    pub fn with_max_in_flight(mut self, max: usize) -> Self {
        self.max_in_flight = max.min(MAX_IN_FLIGHT);
        self
    }

    pub fn geometry(&self) -> &G {
        &self.geometry
    }

    /// Conservative logical allocation for the block state of `piece`.
    /// Runtime adapters can reserve this amount with their memory governor
    /// before the first request opens the piece in the ledger.
    pub fn estimated_piece_state_bytes(&self, piece: u32) -> Option<usize> {
        self.geometry
            .block_count(piece)
            .map(Self::estimated_piece_state_bytes_for_block_count)
    }

    pub fn estimated_piece_state_bytes_for_block_count(block_count: u32) -> usize {
        PieceState::estimated_bytes(block_count as usize)
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    // ---- connections ------------------------------------------------------

    /// Register a peer when the bounded connection table has capacity.
    pub fn try_connect(&mut self, remote_reqq: usize) -> Option<ConnectionId> {
        if self.active_connections >= self.max_connections {
            return None;
        }
        let state = ConnState::new(remote_reqq.clamp(1, MAX_REMOTE_REQQ));
        let conn = if let Some(slot) = self.free_slots.pop() {
            let entry = &mut self.conns[slot as usize];
            entry.generation = entry.generation.wrapping_add(1);
            entry.state = Some(state);
            ConnectionId {
                slot,
                generation: entry.generation,
            }
        } else {
            let slot = self.conns.len() as u32;
            self.conns.push(ConnEntry {
                generation: 1,
                state: Some(state),
            });
            ConnectionId {
                slot,
                generation: 1,
            }
        };
        self.active_connections += 1;
        Some(conn)
    }

    /// Infallible convenience for bounded simulations and fixtures.
    /// Runtime adapters should use [`Self::try_connect`] and handle saturation.
    pub fn connect(&mut self, remote_reqq: usize) -> ConnectionId {
        self.try_connect(remote_reqq)
            .expect("transfer ledger connection limit reached")
    }

    pub fn set_remote_reqq(&mut self, conn: ConnectionId, reqq: usize) {
        if let Some(state) = self.conn_mut(conn) {
            state.reqq = reqq.clamp(1, MAX_REMOTE_REQQ);
            state.trim_history();
        }
    }

    pub fn is_live(&self, conn: ConnectionId) -> bool {
        self.conn_ref(conn).is_some()
    }

    fn conn_ref(&self, conn: ConnectionId) -> Option<&ConnState> {
        let entry = self.conns.get(conn.slot as usize)?;
        (entry.generation == conn.generation)
            .then_some(())
            .and(entry.state.as_ref())
    }

    fn conn_mut(&mut self, conn: ConnectionId) -> Option<&mut ConnState> {
        let entry = self.conns.get_mut(conn.slot as usize)?;
        if entry.generation == conn.generation {
            entry.state.as_mut()
        } else {
            None
        }
    }

    /// Drop a connection and release everything it owned. Returns the blocks
    /// that returned to `Missing`.
    pub fn disconnect(&mut self, conn: ConnectionId) -> Vec<BlockExtent> {
        let Some(entry) = self.conns.get_mut(conn.slot as usize) else {
            return Vec::new();
        };
        if entry.generation != conn.generation {
            return Vec::new();
        }
        let Some(state) = entry.state.take() else {
            return Vec::new();
        };
        self.active_connections = self.active_connections.saturating_sub(1);
        self.free_slots.push(conn.slot);
        let mut released = Vec::new();
        for block in state.outstanding {
            if self.release_owner(conn, block) {
                released.push(block);
            }
        }
        released
    }

    // ---- selection and reservation ---------------------------------------

    pub fn has_piece(&self, piece: u32) -> bool {
        self.have.get(piece)
    }

    /// Open `piece` for acquisition without reserving anything.
    pub fn begin_piece(&mut self, piece: u32) -> Result<(), ReserveError> {
        if self.have.get(piece) {
            return Err(ReserveError::Have);
        }
        if self.pieces.contains_key(&piece) {
            return Ok(());
        }
        let blocks = self.geometry.block_count(piece).ok_or_else(|| {
            ReserveError::Invalid(GeometryError::PieceOutOfRange {
                piece,
                piece_count: self.geometry.piece_count(),
            })
        })?;
        if self.pieces.len() >= self.max_in_progress {
            return Err(ReserveError::TooManyPieces);
        }
        let state = PieceState::new(blocks);
        if self.in_progress_bytes.saturating_add(state.budget_bytes) > self.max_in_progress_bytes {
            return Err(ReserveError::InProgressByteLimit);
        }
        self.in_progress_bytes = self.in_progress_bytes.saturating_add(state.budget_bytes);
        self.pieces.insert(piece, state);
        Ok(())
    }

    /// Restore unverified block indexes from fastresume into the authoritative
    /// piece ledger. A complete restored bitmap is deliberately discarded:
    /// partial resume data is never sufficient evidence to mark a piece `have`.
    /// Invalid and duplicate indexes are ignored, matching the picker's
    /// defensive restore behavior.
    pub fn restore_partial_blocks(
        &mut self,
        piece: u32,
        block_indexes: &[u32],
    ) -> Result<bool, ReserveError> {
        if self.have.get(piece) {
            return Err(ReserveError::Have);
        }
        let block_count = self.geometry.block_count(piece).ok_or_else(|| {
            ReserveError::Invalid(GeometryError::PieceOutOfRange {
                piece,
                piece_count: self.geometry.piece_count(),
            })
        })?;
        if self.pieces.get(&piece).is_some_and(|state| {
            state.awaiting_verification
                || state
                    .slots
                    .iter()
                    .any(|slot| !matches!(slot, Slot::Missing))
        }) {
            return Ok(false);
        }
        self.begin_piece(piece)?;
        let state = self.pieces.get_mut(&piece).expect("piece opened above");
        debug_assert_eq!(state.slots.len(), block_count as usize);
        for &block_index in block_indexes {
            let restored = state
                .slots
                .get_mut(block_index as usize)
                .is_some_and(|slot| {
                    if matches!(slot, Slot::Missing) {
                        *slot = Slot::Received;
                        true
                    } else {
                        false
                    }
                });
            if restored {
                state.received = state.received.saturating_add(1);
            }
        }
        let restored_partial = state.received > 0 && !state.is_complete();
        if !restored_partial {
            self.take_piece_state(piece);
        }
        Ok(restored_partial)
    }

    /// Reserve one specific block for `conn`.
    pub fn reserve(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        mode: &ReserveMode,
    ) -> Result<(), ReserveError> {
        self.geometry
            .validate_request_block(block)
            .map_err(ReserveError::Invalid)?;
        let outstanding_len = self
            .conn_ref(conn)
            .map(|state| (state.outstanding.len(), state.reqq))
            .ok_or(ReserveError::UnknownConnection)?;
        if outstanding_len.0 >= outstanding_len.1 {
            return Err(ReserveError::ConnectionFull);
        }
        if self.in_flight >= self.max_in_flight {
            return Err(ReserveError::GlobalRequestLimit);
        }
        self.begin_piece(block.piece)?;
        let state = self
            .pieces
            .get_mut(&block.piece)
            .expect("piece opened above");
        if state.awaiting_verification {
            return Err(ReserveError::AwaitingVerification);
        }
        let index = block.block_index() as usize;
        match &mut state.slots[index] {
            Slot::Received => return Err(ReserveError::AlreadyReceived),
            Slot::Missing => {
                state.slots[index] = Slot::Reserved(vec![Owner {
                    conn,
                    wire_at: None,
                }])
            }
            Slot::Reserved(owners) => {
                if owners.iter().any(|owner| owner.conn == conn) {
                    return Err(ReserveError::AlreadyOwned);
                }
                if *mode != ReserveMode::Endgame {
                    return Err(ReserveError::OwnedByOther);
                }
                if owners.len() >= MAX_ENDGAME_OWNERS_PER_BLOCK {
                    return Err(ReserveError::TooManyOwners);
                }
                owners.push(Owner {
                    conn,
                    wire_at: None,
                });
            }
        }
        let conn_state = self.conn_mut(conn).expect("connection checked above");
        conn_state.outstanding.insert(block);
        // A fresh reservation supersedes any tombstone for the same block.
        conn_state.take_tombstone(&block);
        self.in_flight += 1;
        Ok(())
    }

    /// Reserve a block while keeping all activity for its piece on one
    /// connection. This check and reservation happen under the same mutable
    /// ledger borrow, so peer-local piece assemblers cannot race into a split
    /// piece when two peers select it at once.
    pub fn reserve_piece_affine(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
    ) -> Result<(), ReserveError> {
        if self.conn_ref(conn).is_none() {
            return Err(ReserveError::UnknownConnection);
        }
        self.geometry
            .validate_request_block(block)
            .map_err(ReserveError::Invalid)?;
        if self.piece_has_foreign_activity(conn, block.piece) {
            return Err(ReserveError::PieceOwnedByOther);
        }
        self.reserve(conn, block, &ReserveMode::Exclusive)
    }

    /// The first block of `piece` this connection could take: `Missing`, or
    /// (endgame) requested elsewhere but not from `conn`. This is the repair
    /// for the audit's F02: a block that is already on the wire from this
    /// connection is skipped and the *next* block of the same piece is used,
    /// so a piece can have several blocks in flight at once.
    pub fn next_block(
        &self,
        conn: ConnectionId,
        piece: u32,
        mode: &ReserveMode,
    ) -> Option<BlockExtent> {
        if self.have.get(piece) {
            return None;
        }
        let blocks = self.geometry.block_count(piece)?;
        let state = self.pieces.get(&piece);
        for index in 0..blocks {
            let block = self.geometry.block_extent(piece, index)?;
            let usable = match state.map(|state| &state.slots[index as usize]) {
                None | Some(Slot::Missing) => true,
                Some(Slot::Received) => false,
                Some(Slot::Reserved(owners)) => {
                    *mode == ReserveMode::Endgame && !owners.iter().any(|owner| owner.conn == conn)
                }
            };
            if usable {
                return Some(block);
            }
        }
        None
    }

    /// Reserve the next usable block of `piece` for `conn` in one step.
    pub fn reserve_next(
        &mut self,
        conn: ConnectionId,
        piece: u32,
        mode: &ReserveMode,
    ) -> Result<Option<BlockExtent>, ReserveError> {
        let Some(block) = self.next_block(conn, piece, mode) else {
            return Ok(None);
        };
        self.reserve(conn, block, mode)?;
        Ok(Some(block))
    }

    // ---- wire events -----------------------------------------------------

    /// The writer confirms `block` was written to the socket at `now`.
    pub fn commit(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        now: Nanos,
    ) -> (CommitOutcome, Vec<Action>) {
        let Some(state) = self.conn_mut(conn) else {
            return (CommitOutcome::StaleConnection, Vec::new());
        };
        if state.take_pending_commit_ignore(&block) {
            return (CommitOutcome::Reconciled, Vec::new());
        }
        if !state.outstanding.contains(&block) {
            return (
                CommitOutcome::Orphaned,
                vec![Action::Cancel { conn, block }],
            );
        }
        if let Some(piece) = self.pieces.get_mut(&block.piece) {
            if let Some(Slot::Reserved(owners)) = piece.slots.get_mut(block.block_index() as usize)
            {
                if let Some(owner) = owners.iter_mut().find(|owner| owner.conn == conn) {
                    if owner.wire_at.is_none() {
                        owner.wire_at = Some(now);
                    }
                    return (CommitOutcome::OnWire, Vec::new());
                }
            }
        }
        (
            CommitOutcome::Orphaned,
            vec![Action::Cancel { conn, block }],
        )
    }

    /// A block's bytes arrived on `conn`.
    pub fn on_block(&mut self, conn: ConnectionId, block: BlockExtent) -> ReceiveOutcome {
        if self.conn_ref(conn).is_none() {
            return ReceiveOutcome::StaleConnection;
        }
        if let Err(error) = self.geometry.validate_request_block(block) {
            return ReceiveOutcome::Invalid(error);
        }
        let index = block.block_index() as usize;
        let has_outstanding = self
            .conn_ref(conn)
            .is_some_and(|state| state.outstanding.contains(&block));
        let slot_owned_by_conn = self
            .pieces
            .get(&block.piece)
            .and_then(|piece| piece.slots.get(index))
            .is_some_and(|slot| match slot {
                Slot::Reserved(owners) => owners.iter().any(|owner| owner.conn == conn),
                _ => false,
            });
        let owned_by_conn = has_outstanding && slot_owned_by_conn;
        let received_already = self
            .pieces
            .get(&block.piece)
            .and_then(|piece| piece.slots.get(index))
            .is_some_and(|slot| matches!(slot, Slot::Received))
            || self.have.get(block.piece);

        if !owned_by_conn {
            if received_already {
                // Whatever the connection was told, the bytes add nothing.
                let state = self.conn_mut(conn).expect("connection checked above");
                state.take_tombstone(&block);
                return ReceiveOutcome::Duplicate;
            }
            // Not owned and not received: only a tombstoned (recently
            // released) request makes this useful late data.
            let has_tombstone = self
                .conn_ref(conn)
                .is_some_and(|state| state.tombstone_set.contains(&block));
            if !has_tombstone {
                return ReceiveOutcome::Unsolicited;
            }
            // Re-open the block for this delivery.
            if self.begin_piece(block.piece).is_err() {
                return ReceiveOutcome::Unsolicited;
            }
            let piece = self.pieces.get_mut(&block.piece).expect("opened above");
            if piece.awaiting_verification || matches!(piece.slots[index], Slot::Received) {
                return ReceiveOutcome::Unsolicited;
            }
            self.conn_mut(conn)
                .expect("connection checked above")
                .take_tombstone(&block);
            return self.accept_block(conn, block, true, Vec::new());
        }
        self.accept_block(conn, block, false, Vec::new())
    }

    /// Receive a block for an adapter that cannot merge partial piece
    /// assemblies across connections. Normal ownership follows `on_block`;
    /// a useful tombstoned late block is ignored when another connection has
    /// since taken activity in the piece, preventing the ledger from mixing
    /// connections that have separate payload assemblies.
    pub fn on_block_piece_affine(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
    ) -> ReceiveOutcome {
        if !self.is_live(conn) {
            return ReceiveOutcome::StaleConnection;
        }
        if let Err(error) = self.geometry.validate_request_block(block) {
            return ReceiveOutcome::Invalid(error);
        }
        if self.piece_has_foreign_activity(conn, block.piece) {
            return ReceiveOutcome::IgnoredForPieceAffinity;
        }
        self.on_block(conn, block)
    }

    fn accept_block(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        late: bool,
        mut actions: Vec<Action>,
    ) -> ReceiveOutcome {
        let index = block.block_index() as usize;
        let piece = self.pieces.get_mut(&block.piece).expect("piece is open");
        let previous = std::mem::replace(&mut piece.slots[index], Slot::Received);
        piece.sources[index] = Some(conn);
        piece.received += 1;
        let completed = piece.is_complete();
        if completed {
            piece.awaiting_verification = true;
        }
        let mut owners = match previous {
            Slot::Reserved(owners) => owners,
            _ => Vec::new(),
        };
        let mut early_commit = false;
        for owner in owners.drain(..) {
            self.in_flight = self.in_flight.saturating_sub(1);
            if owner.conn == conn {
                // The response beat the writer's commit if it was never
                // confirmed on the wire; that commit must now be swallowed.
                early_commit = owner.wire_at.is_none();
                if let Some(state) = self.conn_mut(conn) {
                    state.outstanding.remove(&block);
                }
            } else {
                if let Some(state) = self.conn_mut(owner.conn) {
                    state.outstanding.remove(&block);
                    // The loser's reply is still expected until the cancel
                    // lands, so remember the block for tombstone matching.
                    state.add_tombstone(block);
                }
                actions.push(Action::Cancel {
                    conn: owner.conn,
                    block,
                });
            }
        }
        if early_commit {
            if let Some(state) = self.conn_mut(conn) {
                state.add_pending_commit_ignore(block);
            }
        }
        let extent = completed.then(|| {
            self.geometry
                .piece_extent(block.piece)
                .expect("piece exists in geometry")
        });
        ReceiveOutcome::Accepted {
            completed: extent,
            late,
            actions,
        }
    }

    /// The peer answered a request with Reject Request.
    pub fn on_reject(&mut self, conn: ConnectionId, block: BlockExtent) -> RejectOutcome {
        let Some(state) = self.conn_ref(conn) else {
            return RejectOutcome::StaleConnection;
        };
        if !state.outstanding.contains(&block)
            || self.geometry.validate_request_block(block).is_err()
        {
            return RejectOutcome::Unknown;
        }
        if self.release_owner(conn, block) {
            if let Some(state) = self.conn_mut(conn) {
                state.outstanding.remove(&block);
            }
            RejectOutcome::Released
        } else {
            RejectOutcome::Unknown
        }
    }

    /// The remote peer choked us. Without Fast every outstanding request is
    /// implicitly dropped (and a late reply is still possible, so each is
    /// tombstoned); with Fast, requests stay owned until Piece or Reject.
    pub fn on_choke(&mut self, conn: ConnectionId, fast_negotiated: bool) -> Vec<BlockExtent> {
        if fast_negotiated {
            return Vec::new();
        }
        let blocks: Vec<BlockExtent> = match self.conn_ref(conn) {
            Some(state) => state.outstanding.iter().copied().collect(),
            None => return Vec::new(),
        };
        let mut released = Vec::new();
        for block in blocks {
            if self.release_owner(conn, block) {
                released.push(block);
            }
        }
        if let Some(state) = self.conn_mut(conn) {
            state.outstanding.clear();
            for block in &released {
                state.add_tombstone(*block);
            }
        }
        released
    }

    /// Release requests on the wire longer than `timeout`. Returns the cancel
    /// actions and the expired blocks.
    pub fn expire(
        &mut self,
        now: Nanos,
        timeout: Nanos,
    ) -> (Vec<(ConnectionId, BlockExtent)>, Vec<Action>) {
        let mut expired = Vec::new();
        for (&piece_index, piece) in &self.pieces {
            for (index, slot) in piece.slots.iter().enumerate() {
                if let Slot::Reserved(owners) = slot {
                    for owner in owners {
                        if let Some(at) = owner.wire_at {
                            if now.saturating_sub(at) >= timeout {
                                if let Some(block) =
                                    self.geometry.block_extent(piece_index, index as u32)
                                {
                                    expired.push((owner.conn, block));
                                }
                            }
                        }
                    }
                }
            }
        }
        let mut actions = Vec::new();
        for &(conn, block) in &expired {
            if self.release_owner(conn, block) {
                if let Some(state) = self.conn_mut(conn) {
                    state.outstanding.remove(&block);
                    state.add_tombstone(block);
                }
                actions.push(Action::Cancel { conn, block });
            }
        }
        (expired, actions)
    }

    /// Remove `conn`'s ownership of `block`, returning it to `Missing` when no
    /// owner remains. Returns whether an owner was removed.
    fn release_owner(&mut self, conn: ConnectionId, block: BlockExtent) -> bool {
        let Some(piece) = self.pieces.get_mut(&block.piece) else {
            return false;
        };
        let Some(slot) = piece.slots.get_mut(block.block_index() as usize) else {
            return false;
        };
        let Slot::Reserved(owners) = slot else {
            return false;
        };
        let before = owners.len();
        owners.retain(|owner| owner.conn != conn);
        if owners.len() == before {
            return false;
        }
        self.in_flight = self.in_flight.saturating_sub(1);
        if owners.is_empty() {
            *slot = Slot::Missing;
        }
        true
    }

    // ---- piece lifecycle --------------------------------------------------

    /// Connections that supplied the blocks of `piece`, for blame on a failed
    /// hash. Blocks received before a restart have no source.
    pub fn contributors(&self, piece: u32) -> Vec<ConnectionId> {
        let mut seen = HashSet::new();
        self.pieces
            .get(&piece)
            .map(|state| {
                state
                    .sources
                    .iter()
                    .flatten()
                    .copied()
                    .filter(|conn| seen.insert(*conn))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn take_piece_state(&mut self, piece: u32) -> Option<PieceState> {
        let state = self.pieces.remove(&piece)?;
        self.in_progress_bytes = self.in_progress_bytes.saturating_sub(state.budget_bytes);
        Some(state)
    }

    /// The piece verified: it is now `have` and its acquisition state is freed.
    pub fn piece_verified(&mut self, piece: u32) {
        if let Some(state) = self.take_piece_state(piece) {
            // No block of a verified piece can still be reserved.
            debug_assert!(state.is_complete());
        }
        self.have.set(piece);
    }

    /// Verification failed: every block returns to `Missing` so it is fetched
    /// again. Returns the contributors for blame. Reserved owners are removed
    /// from their connection indexes and receive Cancel actions before the
    /// piece state is discarded.
    pub fn piece_failed(&mut self, piece: u32) -> Vec<ConnectionId> {
        self.piece_failed_with_actions(piece).0
    }

    pub fn piece_failed_with_actions(&mut self, piece: u32) -> (Vec<ConnectionId>, Vec<Action>) {
        let sources = self.contributors(piece);
        let mut actions = Vec::new();
        if let Some(state) = self.take_piece_state(piece) {
            for (index, slot) in state.slots.into_iter().enumerate() {
                let Slot::Reserved(owners) = slot else {
                    continue;
                };
                let Some(block) = self.geometry.block_extent(piece, index as u32) else {
                    continue;
                };
                for owner in owners {
                    self.in_flight = self.in_flight.saturating_sub(1);
                    if let Some(conn_state) = self.conn_mut(owner.conn) {
                        conn_state.outstanding.remove(&block);
                        conn_state.add_tombstone(block);
                    }
                    actions.push(Action::Cancel {
                        conn: owner.conn,
                        block,
                    });
                }
            }
        }
        self.have.clear(piece);
        (sources, actions)
    }

    /// Mark a piece already held (resume data, recheck result).
    pub fn mark_have(&mut self, piece: u32) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(state) = self.take_piece_state(piece) {
            for (index, slot) in state.slots.into_iter().enumerate() {
                if let Slot::Reserved(owners) = slot {
                    if let Some(block) = self.geometry.block_extent(piece, index as u32) {
                        for owner in owners {
                            self.in_flight = self.in_flight.saturating_sub(1);
                            if let Some(conn_state) = self.conn_mut(owner.conn) {
                                conn_state.outstanding.remove(&block);
                                conn_state.add_tombstone(block);
                            }
                            actions.push(Action::Cancel {
                                conn: owner.conn,
                                block,
                            });
                        }
                    }
                }
            }
        }
        self.have.set(piece);
        actions
    }

    /// Quiesce everything: the payload epoch changed. Outstanding requests
    /// are cancelled, partial pieces dropped, tombstones cleared (a reply
    /// issued under the old epoch is simply unsolicited now).
    pub fn advance_epoch(&mut self) -> (Epoch, Vec<Action>) {
        let mut actions = Vec::new();
        for (&piece_index, piece) in &self.pieces {
            for (index, slot) in piece.slots.iter().enumerate() {
                if let Slot::Reserved(owners) = slot {
                    if let Some(block) = self.geometry.block_extent(piece_index, index as u32) {
                        actions.extend(owners.iter().map(|owner| Action::Cancel {
                            conn: owner.conn,
                            block,
                        }));
                    }
                }
            }
        }
        self.pieces.clear();
        self.in_progress_bytes = 0;
        self.in_flight = 0;
        for entry in &mut self.conns {
            if let Some(state) = entry.state.as_mut() {
                state.outstanding.clear();
                state.tombstones.clear();
                state.tombstone_set.clear();
                state.pending_commit_ignores.clear();
                state.pending_commit_ignore_set.clear();
            }
        }
        self.epoch = self.epoch.next();
        (self.epoch, actions)
    }

    // ---- inspection -------------------------------------------------------

    /// Whether `conn` currently owns `block`.
    pub fn owns(&self, conn: ConnectionId, block: BlockExtent) -> bool {
        self.conn_ref(conn)
            .is_some_and(|state| state.outstanding.contains(&block))
    }

    pub fn outstanding(&self, conn: ConnectionId) -> usize {
        self.conn_ref(conn)
            .map_or(0, |state| state.outstanding.len())
    }

    /// Requests owned across the whole torrent.
    pub fn in_flight(&self) -> usize {
        self.in_flight
    }

    pub fn pieces_in_progress(&self) -> usize {
        self.pieces.len()
    }

    /// Conservative logical allocation reserved by in-progress piece state.
    pub fn in_progress_bytes(&self) -> usize {
        self.in_progress_bytes
    }

    pub fn piece_progress(&self, piece: u32) -> Option<(u32, u32)> {
        self.pieces
            .get(&piece)
            .map(|state| (state.received, state.slots.len() as u32))
    }

    /// Remove an unused piece shell after its last request expires or is
    /// canceled. Any received block or live reservation keeps the state.
    pub fn discard_empty_piece(&mut self, piece: u32) -> bool {
        let empty = self.pieces.get(&piece).is_some_and(|state| {
            state.received == 0
                && !state.awaiting_verification
                && state.slots.iter().all(|slot| matches!(slot, Slot::Missing))
        });
        empty && self.take_piece_state(piece).is_some()
    }

    /// Whether `piece` has received blocks or live reservations from a
    /// different connection. Protocol adapters with peer-local piece
    /// assemblies can use this to keep all blocks for a piece on one peer
    /// while still sharing request ownership across the torrent.
    pub fn piece_has_foreign_activity(&self, conn: ConnectionId, piece: u32) -> bool {
        self.pieces
            .get(&piece)
            .is_some_and(|state| Self::has_foreign_activity(state, conn))
    }

    /// Snapshot pieces that a peer-local assembly must leave to another
    /// connection. The ledger bounds this result by its in-progress piece
    /// ceiling, so adapters can avoid locking or scanning the whole torrent
    /// once per candidate piece.
    pub fn foreign_activity_pieces(&self, conn: ConnectionId) -> Vec<u32> {
        let mut pieces = self
            .pieces
            .iter()
            .filter_map(|(&piece, state)| Self::has_foreign_activity(state, conn).then_some(piece))
            .collect::<Vec<_>>();
        pieces.sort_unstable();
        pieces
    }

    fn has_foreign_activity(state: &PieceState, conn: ConnectionId) -> bool {
        state.sources.iter().flatten().any(|source| *source != conn)
            || state.slots.iter().any(|slot| match slot {
                Slot::Reserved(owners) => owners.iter().any(|owner| owner.conn != conn),
                Slot::Missing | Slot::Received => false,
            })
    }

    /// Verify the ledger's internal consistency. Used by the simulator's
    /// oracle after every step; cheap enough for tests, not for hot paths.
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut owned: HashMap<ConnectionId, HashSet<BlockExtent>> = HashMap::new();
        let mut total = 0usize;
        let mut total_state_bytes = 0usize;
        for (&piece_index, piece) in &self.pieces {
            let expected_bytes = PieceState::estimated_bytes(piece.slots.len());
            if piece.budget_bytes != expected_bytes {
                return Err(format!("piece {piece_index} has a stale byte estimate"));
            }
            total_state_bytes = total_state_bytes.saturating_add(piece.budget_bytes);
            let mut received = 0u32;
            for (index, slot) in piece.slots.iter().enumerate() {
                let block = self
                    .geometry
                    .block_extent(piece_index, index as u32)
                    .ok_or_else(|| format!("piece {piece_index} slot {index} has no block"))?;
                match slot {
                    Slot::Received => received += 1,
                    Slot::Reserved(owners) => {
                        if owners.is_empty() {
                            return Err(format!("{block:?} reserved with no owner"));
                        }
                        if owners.len() > MAX_ENDGAME_OWNERS_PER_BLOCK {
                            return Err(format!("{block:?} exceeds its endgame owner limit"));
                        }
                        let mut seen = HashSet::new();
                        for owner in owners {
                            if !seen.insert(owner.conn) {
                                return Err(format!("{block:?} owned twice by {}", owner.conn));
                            }
                            owned.entry(owner.conn).or_default().insert(block);
                            total += 1;
                        }
                    }
                    Slot::Missing => {}
                }
            }
            if received != piece.received {
                return Err(format!(
                    "piece {piece_index} received count {} != {received}",
                    piece.received
                ));
            }
            if piece.awaiting_verification && !piece.is_complete() {
                return Err(format!(
                    "piece {piece_index} awaiting verification but incomplete"
                ));
            }
            if self.have.get(piece_index) {
                return Err(format!("piece {piece_index} is both have and in progress"));
            }
        }
        if total != self.in_flight {
            return Err(format!("in_flight {} != owners {total}", self.in_flight));
        }
        if self.in_flight > self.max_in_flight {
            return Err(format!(
                "in-flight request count {} exceeds {}",
                self.in_flight, self.max_in_flight
            ));
        }
        let active_connections = self
            .conns
            .iter()
            .filter(|entry| entry.state.is_some())
            .count();
        if active_connections != self.active_connections
            || self.active_connections > self.max_connections
        {
            return Err(format!(
                "active connection accounting {} != {active_connections} or exceeds {}",
                self.active_connections, self.max_connections
            ));
        }
        if total_state_bytes != self.in_progress_bytes
            || self.in_progress_bytes > self.max_in_progress_bytes
        {
            return Err(format!(
                "in-progress byte accounting {} != {total_state_bytes} or exceeds {}",
                self.in_progress_bytes, self.max_in_progress_bytes
            ));
        }
        for (slot, entry) in self.conns.iter().enumerate() {
            let Some(state) = entry.state.as_ref() else {
                continue;
            };
            let conn = ConnectionId {
                slot: slot as u32,
                generation: entry.generation,
            };
            let expected = owned.remove(&conn).unwrap_or_default();
            if expected != state.outstanding {
                return Err(format!(
                    "{conn} outstanding set disagrees with slot owners ({} vs {})",
                    state.outstanding.len(),
                    expected.len()
                ));
            }
            if state.outstanding.len() > MAX_REMOTE_REQQ {
                return Err(format!("{conn} exceeds the hard request queue ceiling"));
            }
            if !(1..=MAX_REMOTE_REQQ).contains(&state.reqq) {
                return Err(format!("{conn} has an invalid request queue limit"));
            }
            if state.tombstones.len() != state.tombstone_set.len()
                || state.tombstones.len() > state.tombstone_cap()
            {
                return Err(format!("{conn} has inconsistent or unbounded tombstones"));
            }
            if state.pending_commit_ignores.len() != state.pending_commit_ignore_set.len()
                || state.pending_commit_ignores.len() > state.tombstone_cap()
            {
                return Err(format!(
                    "{conn} has inconsistent or unbounded pending commit history"
                ));
            }
        }
        if let Some((conn, blocks)) = owned.into_iter().next() {
            return Err(format!(
                "{} blocks owned by dead connection {conn}",
                blocks.len()
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::coords::UniformGeometry;
    use super::*;

    const KIB: u32 = 1024;

    fn ledger() -> Ledger<UniformGeometry> {
        // 4 pieces of 32 KiB (two blocks each).
        Ledger::new(UniformGeometry::new(32 * KIB, 4))
    }

    fn block(piece: u32, index: u32) -> BlockExtent {
        BlockExtent::new(piece, index * 16 * KIB, 16 * KIB)
    }

    #[test]
    fn in_progress_state_obeys_byte_budget_and_releases_on_lifecycle_events() {
        let state_bytes = PieceState::estimated_bytes(2);
        let mut ledger = ledger().with_max_in_progress_bytes(state_bytes);
        let conn = ledger.connect(8);

        ledger
            .reserve(conn, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(ledger.in_progress_bytes(), state_bytes);
        assert_eq!(
            ledger.reserve(conn, block(1, 0), &ReserveMode::Exclusive),
            Err(ReserveError::InProgressByteLimit)
        );

        assert!(matches!(
            ledger.on_block(conn, block(0, 0)),
            ReceiveOutcome::Accepted { .. }
        ));
        ledger
            .reserve(conn, block(0, 1), &ReserveMode::Exclusive)
            .unwrap();
        assert!(matches!(
            ledger.on_block(conn, block(0, 1)),
            ReceiveOutcome::Accepted {
                completed: Some(_),
                ..
            }
        ));
        ledger.piece_verified(0);
        assert_eq!(ledger.in_progress_bytes(), 0);

        ledger
            .reserve(conn, block(1, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger.piece_failed(1);
        assert_eq!(ledger.in_progress_bytes(), 0);

        ledger
            .reserve(conn, block(2, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger.advance_epoch();
        assert_eq!(ledger.in_progress_bytes(), 0);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn advertised_request_depth_and_endgame_owners_are_hard_capped() {
        let mut ledger = ledger();
        let too_deep = ledger.connect(usize::MAX);
        assert_eq!(ledger.conn_ref(too_deep).unwrap().reqq, MAX_REMOTE_REQQ);
        ledger.set_remote_reqq(too_deep, usize::MAX);
        assert_eq!(ledger.conn_ref(too_deep).unwrap().reqq, MAX_REMOTE_REQQ);

        let mut owners = vec![too_deep];
        owners.extend((0..MAX_ENDGAME_OWNERS_PER_BLOCK).map(|_| ledger.connect(8)));
        for (index, conn) in owners
            .iter()
            .copied()
            .take(MAX_ENDGAME_OWNERS_PER_BLOCK)
            .enumerate()
        {
            let mode = if index == 0 {
                ReserveMode::Exclusive
            } else {
                ReserveMode::Endgame
            };
            ledger.reserve(conn, block(0, 0), &mode).unwrap();
        }
        assert_eq!(
            ledger.reserve(*owners.last().unwrap(), block(0, 0), &ReserveMode::Endgame),
            Err(ReserveError::TooManyOwners)
        );
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn connection_table_is_bounded_and_reuses_a_new_generation() {
        let mut ledger = ledger().with_max_connections(1);
        let first = ledger.try_connect(8).unwrap();
        assert!(ledger.try_connect(8).is_none());

        ledger.disconnect(first);
        let replacement = ledger.try_connect(8).unwrap();

        assert_eq!(replacement.slot, first.slot);
        assert_ne!(replacement.generation, first.generation);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn global_request_budget_reopens_after_an_obligation_is_released() {
        let mut ledger = ledger().with_max_in_flight(1);
        let conn = ledger.connect(8);
        let first = block(0, 0);
        let second = block(0, 1);

        ledger
            .reserve(conn, first, &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(
            ledger.reserve(conn, second, &ReserveMode::Exclusive),
            Err(ReserveError::GlobalRequestLimit)
        );
        assert_eq!(ledger.on_reject(conn, first), RejectOutcome::Released);
        ledger
            .reserve(conn, second, &ReserveMode::Exclusive)
            .unwrap();
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn a_piece_can_have_every_block_in_flight_at_once() {
        // F02: skipping a piece because its first block is outstanding.
        let mut ledger = ledger();
        let conn = ledger.connect(32);
        let first = ledger
            .reserve_next(conn, 0, &ReserveMode::Exclusive)
            .unwrap();
        let second = ledger
            .reserve_next(conn, 0, &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(first, Some(block(0, 0)));
        assert_eq!(second, Some(block(0, 1)));
        assert_eq!(
            ledger
                .reserve_next(conn, 0, &ReserveMode::Exclusive)
                .unwrap(),
            None
        );
        assert_eq!(ledger.outstanding(conn), 2);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn exclusive_ownership_is_shared_across_connections() {
        // F03: two peers must not both fetch the same missing block.
        let mut ledger = ledger();
        let (a, b) = (ledger.connect(8), ledger.connect(8));
        ledger
            .reserve(a, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(
            ledger.reserve(b, block(0, 0), &ReserveMode::Exclusive),
            Err(ReserveError::OwnedByOther)
        );
        // b is steered to the next free block of the same piece instead.
        assert_eq!(
            ledger.next_block(b, 0, &ReserveMode::Exclusive),
            Some(block(0, 1))
        );
        // Endgame permits the duplicate, but never twice on one connection.
        ledger
            .reserve(b, block(0, 0), &ReserveMode::Endgame)
            .unwrap();
        assert_eq!(
            ledger.reserve(b, block(0, 0), &ReserveMode::Endgame),
            Err(ReserveError::AlreadyOwned)
        );
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn endgame_winner_cancels_the_other_owners() {
        let mut ledger = ledger();
        let (a, b) = (ledger.connect(8), ledger.connect(8));
        ledger
            .reserve(a, block(1, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger
            .reserve(b, block(1, 0), &ReserveMode::Endgame)
            .unwrap();
        ledger.commit(a, block(1, 0), 10);
        ledger.commit(b, block(1, 0), 11);
        match ledger.on_block(b, block(1, 0)) {
            ReceiveOutcome::Accepted { actions, late, .. } => {
                assert!(!late);
                assert_eq!(
                    actions,
                    vec![Action::Cancel {
                        conn: a,
                        block: block(1, 0)
                    }]
                );
            }
            other => panic!("expected acceptance, got {other:?}"),
        }
        // The loser's reply, if the cancel loses the race, is a duplicate.
        assert_eq!(ledger.on_block(a, block(1, 0)), ReceiveOutcome::Duplicate);
        assert_eq!(ledger.in_flight(), 0);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn a_response_can_beat_the_writers_commit_notification() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        ledger
            .reserve(conn, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        // The reader delivers the data before the writer reports the write.
        assert!(matches!(
            ledger.on_block(conn, block(0, 0)),
            ReceiveOutcome::Accepted { .. }
        ));
        // The late commit is reconciled exactly once...
        assert_eq!(
            ledger.commit(conn, block(0, 0), 5),
            (CommitOutcome::Reconciled, Vec::new())
        );
        // ...and a second one is an orphan, not another reconciliation.
        assert_eq!(
            ledger.commit(conn, block(0, 0), 6).0,
            CommitOutcome::Orphaned
        );
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn early_commit_reconciliation_history_is_bounded_per_connection() {
        let mut ledger = Ledger::new(UniformGeometry::new(32 * KIB, 128));
        let conn = ledger.connect(1);
        let first = block(0, 0);
        let latest = block(99, 0);

        for piece in 0..100 {
            let request = block(piece, 0);
            ledger
                .reserve(conn, request, &ReserveMode::Exclusive)
                .unwrap();
            assert!(matches!(
                ledger.on_block(conn, request),
                ReceiveOutcome::Accepted { .. }
            ));
        }

        assert_eq!(
            ledger.commit(conn, first, 1).0,
            CommitOutcome::Orphaned,
            "evicted commit history degrades to a harmless orphan"
        );
        assert_eq!(ledger.commit(conn, latest, 1).0, CommitOutcome::Reconciled);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn piece_completes_verifies_and_failure_refetches_everything() {
        let mut ledger = ledger();
        let a = ledger.connect(8);
        let b = ledger.connect(8);
        ledger
            .reserve(a, block(2, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger
            .reserve(b, block(2, 1), &ReserveMode::Exclusive)
            .unwrap();
        assert!(matches!(
            ledger.on_block(a, block(2, 0)),
            ReceiveOutcome::Accepted {
                completed: None,
                ..
            }
        ));
        let ReceiveOutcome::Accepted { completed, .. } = ledger.on_block(b, block(2, 1)) else {
            panic!("second block accepted");
        };
        assert_eq!(completed.map(|piece| piece.length), Some(32 * KIB));
        assert_eq!(
            ledger.reserve(a, block(2, 0), &ReserveMode::Exclusive),
            Err(ReserveError::AwaitingVerification)
        );
        let mut blame = ledger.piece_failed(2);
        blame.sort();
        assert_eq!(blame, vec![a, b]);
        // Everything is Missing again and can be reserved afresh.
        ledger
            .reserve(a, block(2, 0), &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(ledger.piece_progress(2), Some((0, 2)));
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn failed_piece_releases_all_reserved_owners_and_emits_cancels() {
        let mut ledger = ledger();
        let first = ledger.connect(8);
        let second = ledger.connect(8);
        let requested = block(0, 0);
        ledger
            .reserve(first, requested, &ReserveMode::Exclusive)
            .unwrap();
        ledger
            .reserve(second, requested, &ReserveMode::Endgame)
            .unwrap();

        let (_, actions) = ledger.piece_failed_with_actions(0);
        assert_eq!(ledger.in_flight(), 0);
        assert_eq!(ledger.outstanding(first), 0);
        assert_eq!(ledger.outstanding(second), 0);
        assert_eq!(
            actions,
            vec![
                Action::Cancel {
                    conn: first,
                    block: requested,
                },
                Action::Cancel {
                    conn: second,
                    block: requested,
                },
            ]
        );
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn peer_local_assembly_can_detect_foreign_piece_activity() {
        let mut ledger = ledger();
        let first = ledger.connect(8);
        let second = ledger.connect(8);
        let requested = block(0, 0);
        assert!(!ledger.piece_has_foreign_activity(first, 0));
        ledger
            .reserve(first, requested, &ReserveMode::Exclusive)
            .unwrap();
        assert!(!ledger.piece_has_foreign_activity(first, 0));
        assert!(ledger.piece_has_foreign_activity(second, 0));
        assert_eq!(ledger.foreign_activity_pieces(second), vec![0]);

        ledger.on_block(first, requested);
        assert!(!ledger.piece_has_foreign_activity(first, 0));
        assert!(ledger.piece_has_foreign_activity(second, 0));
    }

    #[test]
    fn piece_affinity_reservation_is_atomic_across_connections() {
        let mut ledger = ledger();
        let first = ledger.connect(8);
        let second = ledger.connect(8);

        ledger.reserve_piece_affine(first, block(0, 0)).unwrap();
        assert_eq!(
            ledger.reserve_piece_affine(second, block(0, 1)),
            Err(ReserveError::PieceOwnedByOther)
        );
        ledger.reserve_piece_affine(first, block(0, 1)).unwrap();
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn verified_pieces_are_have_and_cannot_be_reserved() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        ledger
            .reserve(conn, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger
            .reserve(conn, block(0, 1), &ReserveMode::Exclusive)
            .unwrap();
        ledger.on_block(conn, block(0, 0));
        ledger.on_block(conn, block(0, 1));
        ledger.piece_verified(0);
        assert!(ledger.has_piece(0));
        assert_eq!(
            ledger.reserve(conn, block(0, 0), &ReserveMode::Exclusive),
            Err(ReserveError::Have)
        );
        assert_eq!(ledger.pieces_in_progress(), 0);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn legacy_choke_releases_requests_but_keeps_tombstones_for_late_data() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        ledger
            .reserve(conn, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger.commit(conn, block(0, 0), 1);
        let released = ledger.on_choke(conn, false);
        assert_eq!(released, vec![block(0, 0)]);
        assert_eq!(ledger.outstanding(conn), 0);
        // The block is free for anyone else...
        let other = ledger.connect(8);
        assert_eq!(
            ledger.next_block(other, 0, &ReserveMode::Exclusive),
            Some(block(0, 0))
        );
        // ...but the choked peer's late reply is still useful once.
        match ledger.on_block(conn, block(0, 0)) {
            ReceiveOutcome::Accepted { late, .. } => assert!(late),
            other => panic!("late useful data must be accepted: {other:?}"),
        }
        // An identical second delivery is merely a duplicate.
        assert_eq!(
            ledger.on_block(conn, block(0, 0)),
            ReceiveOutcome::Duplicate
        );
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn useful_late_data_wins_over_a_replacement_request() {
        let mut ledger = ledger();
        let late_peer = ledger.connect(8);
        let replacement_peer = ledger.connect(8);
        let requested = block(0, 0);

        ledger
            .reserve(late_peer, requested, &ReserveMode::Exclusive)
            .unwrap();
        ledger.commit(late_peer, requested, 1);
        ledger.on_choke(late_peer, false);
        ledger
            .reserve(replacement_peer, requested, &ReserveMode::Exclusive)
            .unwrap();

        match ledger.on_block(late_peer, requested) {
            ReceiveOutcome::Accepted {
                late: true,
                actions,
                ..
            } => assert_eq!(
                actions,
                vec![Action::Cancel {
                    conn: replacement_peer,
                    block: requested,
                }]
            ),
            other => panic!("useful late data must replace the new request: {other:?}"),
        }
        assert_eq!(ledger.outstanding(replacement_peer), 0);
        assert_eq!(ledger.in_flight(), 0);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn late_tombstone_survives_a_temporary_in_progress_piece_limit() {
        let mut ledger = ledger().with_max_in_progress(1);
        let late_peer = ledger.connect(8);
        let busy_peer = ledger.connect(8);
        let late_block = block(0, 0);

        ledger
            .reserve(late_peer, late_block, &ReserveMode::Exclusive)
            .unwrap();
        ledger.commit(late_peer, late_block, 1);
        ledger.on_choke(late_peer, false);
        ledger.piece_failed(0);

        ledger
            .reserve(busy_peer, block(1, 0), &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(
            ledger.on_block(late_peer, late_block),
            ReceiveOutcome::Unsolicited
        );
        ledger.check_invariants().unwrap();

        ledger
            .reserve(busy_peer, block(1, 1), &ReserveMode::Exclusive)
            .unwrap();
        ledger.on_block(busy_peer, block(1, 0));
        assert!(matches!(
            ledger.on_block(busy_peer, block(1, 1)),
            ReceiveOutcome::Accepted {
                completed: Some(_),
                ..
            }
        ));
        ledger.piece_verified(1);

        assert!(matches!(
            ledger.on_block(late_peer, late_block),
            ReceiveOutcome::Accepted { late: true, .. }
        ));
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn fast_choke_keeps_ownership_until_piece_or_reject() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        ledger
            .reserve(conn, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        assert!(ledger.on_choke(conn, true).is_empty());
        assert_eq!(ledger.outstanding(conn), 1);
        assert_eq!(ledger.on_reject(conn, block(0, 0)), RejectOutcome::Released);
        assert_eq!(ledger.on_reject(conn, block(0, 0)), RejectOutcome::Unknown);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn unsolicited_data_is_rejected_and_never_accepted_late() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        assert_eq!(
            ledger.on_block(conn, block(0, 0)),
            ReceiveOutcome::Unsolicited
        );
        assert!(matches!(
            ledger.on_block(conn, BlockExtent::new(9, 0, 16 * KIB)),
            ReceiveOutcome::Invalid(GeometryError::PieceOutOfRange { .. })
        ));
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn partial_or_mismatched_extents_cannot_complete_a_reserved_block() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        let requested = block(0, 0);
        let partial = BlockExtent::new(0, 0, 8 * KIB);

        assert_eq!(
            ledger.reserve(conn, partial, &ReserveMode::Exclusive),
            Err(ReserveError::Invalid(GeometryError::NonCanonicalBlock {
                piece: 0,
                begin: 0,
                length: 8 * KIB,
            }))
        );
        ledger
            .reserve(conn, requested, &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(ledger.on_reject(conn, partial), RejectOutcome::Unknown);
        assert!(ledger.owns(conn, requested));
        assert_eq!(ledger.commit(conn, partial, 1).0, CommitOutcome::Orphaned);
        assert_eq!(ledger.commit(conn, requested, 2).0, CommitOutcome::OnWire);
        assert_eq!(
            ledger.on_block(conn, partial),
            ReceiveOutcome::Invalid(GeometryError::NonCanonicalBlock {
                piece: 0,
                begin: 0,
                length: 8 * KIB,
            })
        );
        assert!(ledger.owns(conn, requested));
        assert_eq!(ledger.piece_progress(0), Some((0, 2)));
        assert!(matches!(
            ledger.on_block(conn, requested),
            ReceiveOutcome::Accepted { .. }
        ));
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn a_reused_slot_cannot_be_mutated_by_the_old_connection() {
        let mut ledger = ledger();
        let old = ledger.connect(8);
        ledger
            .reserve(old, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        let released = ledger.disconnect(old);
        assert_eq!(released, vec![block(0, 0)]);
        let replacement = ledger.connect(8);
        assert_eq!(replacement.slot, old.slot);
        assert_ne!(replacement.generation, old.generation);
        ledger
            .reserve(replacement, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        // Late events from the dead generation touch nothing.
        assert_eq!(
            ledger.on_block(old, block(0, 0)),
            ReceiveOutcome::StaleConnection
        );
        assert_eq!(
            ledger.commit(old, block(0, 0), 1).0,
            CommitOutcome::StaleConnection
        );
        assert_eq!(
            ledger.on_reject(old, block(0, 0)),
            RejectOutcome::StaleConnection
        );
        assert_eq!(ledger.outstanding(replacement), 1);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn timeouts_release_and_cancel_only_requests_that_were_on_the_wire() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        ledger
            .reserve(conn, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger
            .reserve(conn, block(0, 1), &ReserveMode::Exclusive)
            .unwrap();
        ledger.commit(conn, block(0, 0), 100);
        // block(0,1) was reserved but the writer never put it on the wire.
        let (expired, actions) = ledger.expire(100 + 60, 60);
        assert_eq!(expired, vec![(conn, block(0, 0))]);
        assert_eq!(
            actions,
            vec![Action::Cancel {
                conn,
                block: block(0, 0)
            }]
        );
        assert_eq!(ledger.outstanding(conn), 1);
        // A reply after the expiry is late-but-useful, not unsolicited.
        assert!(matches!(
            ledger.on_block(conn, block(0, 0)),
            ReceiveOutcome::Accepted { late: true, .. }
        ));
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn piece_affine_adapter_ignores_late_block_after_another_peer_takes_piece() {
        let mut ledger = ledger();
        let (first, second) = (ledger.connect(8), ledger.connect(8));
        let late = block(0, 0);
        ledger.reserve_piece_affine(first, late).unwrap();
        ledger.commit(first, late, 100);
        let (expired, actions) = ledger.expire(160, 60);
        assert_eq!(expired, vec![(first, late)]);
        assert_eq!(
            actions,
            vec![Action::Cancel {
                conn: first,
                block: late
            }]
        );

        let active = block(0, 1);
        ledger.reserve_piece_affine(second, active).unwrap();
        assert_eq!(
            ledger.on_block_piece_affine(first, late),
            ReceiveOutcome::IgnoredForPieceAffinity
        );
        assert!(ledger.owns(second, active));
        assert_eq!(ledger.piece_progress(0), Some((0, 2)));
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn restored_partial_blocks_count_toward_shared_piece_completion() {
        let mut ledger = ledger();
        assert!(ledger.restore_partial_blocks(0, &[0, 0, u32::MAX]).unwrap());
        assert_eq!(ledger.piece_progress(0), Some((1, 2)));
        assert!(!ledger.has_piece(0));

        let conn = ledger.connect(8);
        let missing = block(0, 1);
        ledger
            .reserve(conn, missing, &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(ledger.commit(conn, missing, 10).0, CommitOutcome::OnWire);
        assert_eq!(
            ledger.on_block(conn, missing),
            ReceiveOutcome::Accepted {
                completed: Some(PieceExtent {
                    piece: 0,
                    length: 32 * KIB,
                }),
                late: false,
                actions: Vec::new(),
            }
        );
        assert!(
            !ledger.has_piece(0),
            "received data still requires verification"
        );
        ledger.piece_verified(0);
        assert!(ledger.has_piece(0));

        // A stale partial record that claims every block is not verification
        // evidence and must not make the piece available.
        assert!(!ledger.restore_partial_blocks(1, &[0, 1, 1]).unwrap());
        assert!(!ledger.has_piece(1));
        assert_eq!(ledger.piece_progress(1), None);
        assert!(!ledger.restore_partial_blocks(2, &[u32::MAX]).unwrap());
        assert_eq!(ledger.piece_progress(2), None);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn empty_piece_shell_can_be_released_without_touching_foreign_ownership() {
        let mut ledger = ledger();
        let (first, second) = (ledger.connect(8), ledger.connect(8));
        let expired = block(0, 0);
        ledger.reserve_piece_affine(first, expired).unwrap();
        ledger.commit(first, expired, 100);
        ledger.expire(160, 60);
        assert!(ledger.discard_empty_piece(0));
        assert_eq!(ledger.pieces_in_progress(), 0);

        ledger.reserve_piece_affine(second, block(1, 0)).unwrap();
        assert!(!ledger.discard_empty_piece(1));
        assert_eq!(ledger.pieces_in_progress(), 1);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn commit_for_a_released_request_is_an_orphan_that_gets_cancelled() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        ledger
            .reserve(conn, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger.on_reject(conn, block(0, 0));
        let (outcome, actions) = ledger.commit(conn, block(0, 0), 1);
        assert_eq!(outcome, CommitOutcome::Orphaned);
        assert_eq!(
            actions,
            vec![Action::Cancel {
                conn,
                block: block(0, 0)
            }]
        );
    }

    #[test]
    fn epoch_change_quiesces_all_outstanding_work() {
        let mut ledger = ledger();
        let (a, b) = (ledger.connect(8), ledger.connect(8));
        ledger
            .reserve(a, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger
            .reserve(b, block(1, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger.on_block(a, block(0, 0));
        let (epoch, actions) = ledger.advance_epoch();
        assert_eq!(epoch, Epoch(1));
        assert_eq!(
            actions,
            vec![Action::Cancel {
                conn: b,
                block: block(1, 0)
            }]
        );
        assert_eq!(ledger.in_flight(), 0);
        assert_eq!(ledger.pieces_in_progress(), 0);
        // A reply issued under the old epoch is not accepted.
        assert_eq!(ledger.on_block(b, block(1, 0)), ReceiveOutcome::Unsolicited);
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn per_connection_and_per_torrent_bounds_hold() {
        let mut ledger = Ledger::new(UniformGeometry::new(32 * KIB, 8)).with_max_in_progress(2);
        let conn = ledger.connect(3);
        ledger
            .reserve(conn, block(0, 0), &ReserveMode::Exclusive)
            .unwrap();
        ledger
            .reserve(conn, block(0, 1), &ReserveMode::Exclusive)
            .unwrap();
        ledger
            .reserve(conn, block(1, 0), &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(
            ledger.reserve(conn, block(1, 1), &ReserveMode::Exclusive),
            Err(ReserveError::ConnectionFull)
        );
        let other = ledger.connect(8);
        assert_eq!(
            ledger.reserve(other, block(2, 0), &ReserveMode::Exclusive),
            Err(ReserveError::TooManyPieces)
        );
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn mark_have_cancels_what_was_in_flight_for_that_piece() {
        let mut ledger = ledger();
        let conn = ledger.connect(8);
        ledger
            .reserve(conn, block(3, 0), &ReserveMode::Exclusive)
            .unwrap();
        let actions = ledger.mark_have(3);
        assert_eq!(
            actions,
            vec![Action::Cancel {
                conn,
                block: block(3, 0)
            }]
        );
        assert!(ledger.has_piece(3));
        assert_eq!(ledger.outstanding(conn), 0);
        ledger.check_invariants().unwrap();
    }
}
