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
/// Default bound on pieces being acquired at once.
pub const DEFAULT_MAX_IN_PROGRESS_PIECES: usize = 1024;

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
    /// This connection already owns the block.
    AlreadyOwned,
    ConnectionFull,
    TooManyPieces,
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
}

impl PieceState {
    fn new(blocks: u32) -> Self {
        Self {
            slots: vec![Slot::Missing; blocks as usize],
            sources: vec![None; blocks as usize],
            received: 0,
            awaiting_verification: false,
        }
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
    /// Blocks whose response was accepted before the writer's commit arrived.
    pending_commit_ignores: HashSet<BlockExtent>,
}

impl ConnState {
    fn new(reqq: usize) -> Self {
        Self {
            outstanding: HashSet::new(),
            reqq,
            tombstones: VecDeque::new(),
            tombstone_set: HashSet::new(),
            pending_commit_ignores: HashSet::new(),
        }
    }

    fn tombstone_cap(&self) -> usize {
        self.reqq.saturating_mul(2).max(64)
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
    max_in_progress: usize,
    in_flight: usize,
}

impl<G: PieceGeometry> Ledger<G> {
    pub fn new(geometry: G) -> Self {
        let pieces = geometry.piece_count();
        Self {
            geometry,
            epoch: Epoch::default(),
            conns: Vec::new(),
            free_slots: Vec::new(),
            pieces: HashMap::new(),
            have: HaveSet::new(pieces),
            max_in_progress: DEFAULT_MAX_IN_PROGRESS_PIECES,
            in_flight: 0,
        }
    }

    pub fn with_max_in_progress(mut self, max: usize) -> Self {
        self.max_in_progress = max.max(1);
        self
    }

    pub fn geometry(&self) -> &G {
        &self.geometry
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    // ---- connections ------------------------------------------------------

    pub fn connect(&mut self, remote_reqq: usize) -> ConnectionId {
        let state = ConnState::new(remote_reqq.max(1));
        if let Some(slot) = self.free_slots.pop() {
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
        }
    }

    pub fn set_remote_reqq(&mut self, conn: ConnectionId, reqq: usize) {
        if let Some(state) = self.conn_mut(conn) {
            state.reqq = reqq.max(1);
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
        self.pieces.insert(piece, PieceState::new(blocks));
        Ok(())
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
        if state.pending_commit_ignores.remove(&block) {
            return (CommitOutcome::Reconciled, Vec::new());
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
        if let Err(error) = self.geometry.validate_served_block(block) {
            return ReceiveOutcome::Invalid(error);
        }
        let index = block.block_index() as usize;
        let owned_by_conn = self
            .pieces
            .get(&block.piece)
            .and_then(|piece| piece.slots.get(index))
            .is_some_and(|slot| match slot {
                Slot::Reserved(owners) => owners.iter().any(|owner| owner.conn == conn),
                _ => false,
            });
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
            let state = self.conn_mut(conn).expect("connection checked above");
            if !state.take_tombstone(&block) {
                return ReceiveOutcome::Unsolicited;
            }
            // Re-open the block for this delivery.
            if self.begin_piece(block.piece).is_err() {
                return ReceiveOutcome::Unsolicited;
            }
            let piece = self.pieces.get_mut(&block.piece).expect("opened above");
            if piece.awaiting_verification || !matches!(piece.slots[index], Slot::Missing) {
                return ReceiveOutcome::Unsolicited;
            }
            return self.accept_block(conn, block, true, Vec::new());
        }
        self.accept_block(conn, block, false, Vec::new())
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
                state.pending_commit_ignores.insert(block);
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
        if self.conn_ref(conn).is_none() {
            return RejectOutcome::StaleConnection;
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

    /// The piece verified: it is now `have` and its acquisition state is freed.
    pub fn piece_verified(&mut self, piece: u32) {
        if let Some(state) = self.pieces.remove(&piece) {
            // No block of a verified piece can still be reserved.
            debug_assert!(state.is_complete());
        }
        self.have.set(piece);
    }

    /// Verification failed: every block returns to `Missing` so it is fetched
    /// again. Returns the contributors for blame.
    pub fn piece_failed(&mut self, piece: u32) -> Vec<ConnectionId> {
        let sources = self.contributors(piece);
        self.pieces.remove(&piece);
        self.have.clear(piece);
        sources
    }

    /// Mark a piece already held (resume data, recheck result).
    pub fn mark_have(&mut self, piece: u32) -> Vec<Action> {
        let mut actions = Vec::new();
        if let Some(state) = self.pieces.remove(&piece) {
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
        self.in_flight = 0;
        for entry in &mut self.conns {
            if let Some(state) = entry.state.as_mut() {
                state.outstanding.clear();
                state.tombstones.clear();
                state.tombstone_set.clear();
                state.pending_commit_ignores.clear();
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

    pub fn piece_progress(&self, piece: u32) -> Option<(u32, u32)> {
        self.pieces
            .get(&piece)
            .map(|state| (state.received, state.slots.len() as u32))
    }

    /// Verify the ledger's internal consistency. Used by the simulator's
    /// oracle after every step; cheap enough for tests, not for hot paths.
    pub fn check_invariants(&self) -> Result<(), String> {
        let mut owned: HashMap<ConnectionId, HashSet<BlockExtent>> = HashMap::new();
        let mut total = 0usize;
        for (&piece_index, piece) in &self.pieces {
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
            if state.outstanding.len() > state.reqq {
                return Err(format!("{conn} exceeds its request queue"));
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
