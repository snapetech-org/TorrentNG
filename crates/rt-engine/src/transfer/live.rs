//! Actor-facing ownership ledger during the staged hybrid migration.
//!
//! Non-hybrid v1 tasks use the plain ledger. Once validated hybrid metadata is
//! attached, the same actor switches to `HybridLedger` before registering any
//! peer or restoring partial state. Its current v1 wire path stays source
//! compatible while the dual-plane handshake and scheduling work is pending.

use std::sync::Arc;

use super::coords::{BlockExtent, ConnectionId, HybridGeometry, PieceGeometry};
use super::hybrid::HybridLedger;
use super::ledger::{
    Action, CommitOutcome, Ledger, ReceiveOutcome, RejectOutcome, ReserveError, ReserveMode,
    MAX_CONNECTIONS,
};
use super::Nanos;

enum LiveLedgerInner<G: PieceGeometry> {
    V1(Ledger<G>),
    Hybrid(HybridLedger),
}

/// The single live request/content owner used by a v1 or hybrid-v1 actor.
pub(crate) struct LiveTransferLedger<G: PieceGeometry> {
    inner: LiveLedgerInner<G>,
    max_connections: usize,
    used: bool,
}

impl<G: PieceGeometry> LiveTransferLedger<G> {
    pub fn new_bounded(geometry: G, max_connections: usize) -> Self {
        let max_connections = max_connections.min(MAX_CONNECTIONS);
        Self {
            inner: LiveLedgerInner::V1(Ledger::new(geometry).with_max_connections(max_connections)),
            max_connections,
            used: false,
        }
    }

    /// Replace the empty v1 ledger with its dual-view hybrid adapter.
    /// Metadata is attached before the task starts, so any prior use indicates
    /// an integration-order bug and leaves the existing owner untouched.
    pub fn attach_hybrid_geometry(&mut self, geometry: Arc<HybridGeometry>) -> bool {
        if self.used {
            return false;
        }
        self.inner =
            LiveLedgerInner::Hybrid(HybridLedger::new_bounded(geometry, self.max_connections));
        true
    }

    #[cfg(test)]
    pub fn is_hybrid(&self) -> bool {
        matches!(&self.inner, LiveLedgerInner::Hybrid(_))
    }

    /// The legacy estimate remains available for fixtures that size the
    /// ordinary v1 adapter. Live admission uses `connection_state_bytes`,
    /// which includes the hybrid plane index when one is present.
    pub fn estimated_connection_state_bytes() -> usize {
        Ledger::<G>::estimated_connection_state_bytes()
    }

    pub fn connection_state_bytes(&self) -> usize {
        match &self.inner {
            LiveLedgerInner::V1(_) => Ledger::<G>::estimated_connection_state_bytes(),
            LiveLedgerInner::Hybrid(_) => HybridLedger::estimated_connection_state_bytes(),
        }
    }

    pub fn estimated_piece_state_bytes_for_block_count(block_count: u32) -> usize {
        Ledger::<G>::estimated_piece_state_bytes_for_block_count(block_count)
    }

    pub fn estimated_piece_state_bytes(&self, piece: u32) -> Option<usize> {
        match &self.inner {
            LiveLedgerInner::V1(ledger) => ledger.estimated_piece_state_bytes(piece),
            LiveLedgerInner::Hybrid(ledger) => ledger.estimated_piece_state_bytes(piece),
        }
    }

    pub fn try_connect(&mut self, remote_reqq: usize) -> Option<ConnectionId> {
        let conn = match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.try_connect(remote_reqq),
            LiveLedgerInner::Hybrid(ledger) => {
                ledger.try_connect(super::coords::Plane::V1, remote_reqq)
            }
        };
        self.used |= conn.is_some();
        conn
    }

    pub fn is_live(&self, conn: ConnectionId) -> bool {
        match &self.inner {
            LiveLedgerInner::V1(ledger) => ledger.is_live(conn),
            LiveLedgerInner::Hybrid(ledger) => ledger.is_live(conn),
        }
    }

    pub fn reserve(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        mode: &ReserveMode,
    ) -> Result<(), ReserveError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.reserve(conn, block, mode),
            LiveLedgerInner::Hybrid(ledger) => ledger.reserve_v1(conn, block, mode),
        }
    }

    pub fn commit(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        now: Nanos,
    ) -> (CommitOutcome, Vec<Action>) {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.commit(conn, block, now),
            LiveLedgerInner::Hybrid(ledger) => ledger.commit_v1(conn, block, now),
        }
    }

    pub fn on_reject(&mut self, conn: ConnectionId, block: BlockExtent) -> RejectOutcome {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.on_reject(conn, block),
            LiveLedgerInner::Hybrid(ledger) => ledger.on_reject_v1(conn, block),
        }
    }

    pub fn on_block(&mut self, conn: ConnectionId, block: BlockExtent) -> ReceiveOutcome {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.on_block(conn, block),
            LiveLedgerInner::Hybrid(ledger) => ledger.on_block_v1(conn, block),
        }
    }

    pub fn on_choke(&mut self, conn: ConnectionId, fast_negotiated: bool) -> Vec<BlockExtent> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.on_choke(conn, fast_negotiated),
            LiveLedgerInner::Hybrid(ledger) => ledger.on_choke_v1(conn, fast_negotiated),
        }
    }

    pub fn disconnect(&mut self, conn: ConnectionId) -> Vec<BlockExtent> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.disconnect(conn),
            LiveLedgerInner::Hybrid(ledger) => ledger.disconnect_v1(conn),
        }
    }

    pub fn restore_partial_blocks(
        &mut self,
        piece: u32,
        received_blocks: &[u32],
    ) -> Result<bool, ReserveError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.restore_partial_blocks(piece, received_blocks),
            LiveLedgerInner::Hybrid(ledger) => {
                ledger.restore_partial_blocks(piece, received_blocks)
            }
        }
    }

    pub fn discard_empty_piece(&mut self, piece: u32) -> bool {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.discard_empty_piece(piece),
            LiveLedgerInner::Hybrid(ledger) => ledger.discard_empty_piece(piece),
        }
    }

    pub fn piece_progress(&self, piece: u32) -> Option<(u32, u32)> {
        match &self.inner {
            LiveLedgerInner::V1(ledger) => ledger.piece_progress(piece),
            LiveLedgerInner::Hybrid(ledger) => ledger.piece_progress(piece),
        }
    }

    pub fn has_piece(&self, piece: u32) -> bool {
        match &self.inner {
            LiveLedgerInner::V1(ledger) => ledger.has_piece(piece),
            LiveLedgerInner::Hybrid(ledger) => ledger.has_piece(piece),
        }
    }

    pub fn mark_have(&mut self, piece: u32) -> Vec<Action> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.mark_have(piece),
            LiveLedgerInner::Hybrid(ledger) => ledger.mark_have_v1(piece),
        }
    }

    pub fn piece_failed_with_actions(&mut self, piece: u32) -> (Vec<ConnectionId>, Vec<Action>) {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.piece_failed_with_actions(piece),
            LiveLedgerInner::Hybrid(ledger) => ledger.piece_failed_with_actions_v1(piece),
        }
    }

    pub fn outstanding(&self, conn: ConnectionId) -> usize {
        match &self.inner {
            LiveLedgerInner::V1(ledger) => ledger.outstanding(conn),
            LiveLedgerInner::Hybrid(ledger) => ledger.outstanding(conn),
        }
    }

    pub fn in_flight(&self) -> usize {
        match &self.inner {
            LiveLedgerInner::V1(ledger) => ledger.in_flight(),
            LiveLedgerInner::Hybrid(ledger) => ledger.in_flight(),
        }
    }

    pub fn pieces_in_progress(&self) -> usize {
        match &self.inner {
            LiveLedgerInner::V1(ledger) => ledger.pieces_in_progress(),
            LiveLedgerInner::Hybrid(ledger) => ledger.pieces_in_progress(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::transfer::coords::BLOCK_SIZE;
    use crate::transfer::hybrid::HybridLedger;
    use rt_path::SafeRelPath;
    use rt_piece_map::{FileSpan, PieceMap, V2FileSpan, V2PieceMap};
    use std::sync::Arc;

    #[test]
    fn hybrid_actor_uses_one_plane_bound_ledger_owner() {
        let piece_length = u64::from(BLOCK_SIZE) * 2;
        let path = SafeRelPath::from_name("payload.bin", false).unwrap();
        let v1 = Arc::new(
            PieceMap::new(
                piece_length,
                vec![FileSpan {
                    file_index: 0,
                    path: path.clone(),
                    content_offset: 0,
                    length: piece_length,
                }],
            )
            .unwrap(),
        );
        let v2 = Arc::new(
            V2PieceMap::new(
                piece_length,
                vec![V2FileSpan {
                    file_index: 0,
                    path,
                    piece_offset: 0,
                    length: piece_length,
                    pad: false,
                }],
            )
            .unwrap(),
        );
        let geometry = Arc::new(HybridGeometry::new(Arc::clone(&v1), v2));
        let mut ledger = LiveTransferLedger::new_bounded(Arc::clone(&v1), 4);

        assert!(ledger.attach_hybrid_geometry(Arc::clone(&geometry)));
        assert_eq!(
            ledger.connection_state_bytes(),
            HybridLedger::estimated_connection_state_bytes()
        );

        let conn = ledger.try_connect(4).unwrap();
        let block = BlockExtent::new(0, 0, BLOCK_SIZE);
        ledger
            .reserve(conn, block, &ReserveMode::Exclusive)
            .unwrap();
        assert_eq!(ledger.commit(conn, block, 10).0, CommitOutcome::OnWire);
        assert!(matches!(
            ledger.on_block(conn, block),
            ReceiveOutcome::Accepted { .. }
        ));
        assert_eq!(ledger.piece_progress(0), Some((1, 2)));

        // A live hybrid ledger cannot be replaced after ownership has begun.
        assert!(!ledger.attach_hybrid_geometry(geometry));
        assert!(ledger.is_live(conn));
    }
}
