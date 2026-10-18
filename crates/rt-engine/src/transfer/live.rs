//! Actor-facing ownership ledger during the staged hybrid migration.
//!
//! Non-hybrid v1 tasks use the plain ledger. Once validated hybrid metadata is
//! attached, the same actor switches to `HybridLedger` before registering any
//! peer or restoring partial state. Its current v1 wire path stays source
//! compatible; plane-aware operations expose canonical ownership to the v2
//! path while the handshake and scheduling integration is pending.

use std::sync::Arc;

use bytes::Bytes;

use super::coords::{BlockExtent, ConnectionId, HybridGeometry, PieceGeometry, Plane};
use super::hybrid::{
    HybridAction, HybridBlockData, HybridLedger, HybridLedgerError, HybridReceiveOutcome,
    HybridWireBlock,
};
use super::ledger::{
    Action, CommitOutcome, Ledger, ReceiveOutcome, RejectOutcome, ReserveError, ReserveMode,
    MAX_CONNECTIONS,
};
use super::Nanos;

enum LiveLedgerInner<G: PieceGeometry> {
    V1(Ledger<G>),
    Hybrid(HybridLedger),
}

fn v1_actions(actions: Vec<Action>) -> Vec<HybridAction> {
    actions
        .into_iter()
        .map(|action| match action {
            Action::Cancel { conn, block } => HybridAction {
                conn,
                block: HybridWireBlock {
                    plane: Plane::V1,
                    extent: block,
                },
            },
        })
        .collect()
}

/// The single live request/content owner shared by plane-aware actor paths.
/// Legacy methods preserve the existing v1 call surface during migration.
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
        self.try_connect_on_plane(Plane::V1, remote_reqq)
    }

    /// Register a connection on the wire plane selected by its handshake.
    /// A plain v1 torrent cannot admit a v2-plane connection.
    pub fn try_connect_on_plane(
        &mut self,
        plane: Plane,
        remote_reqq: usize,
    ) -> Option<ConnectionId> {
        let conn = match (&mut self.inner, plane) {
            (LiveLedgerInner::V1(ledger), Plane::V1) => ledger.try_connect(remote_reqq),
            (LiveLedgerInner::V1(_), Plane::V2) => None,
            (LiveLedgerInner::Hybrid(ledger), plane) => ledger.try_connect(plane, remote_reqq),
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

    pub fn plane(&self, conn: ConnectionId) -> Option<Plane> {
        match &self.inner {
            LiveLedgerInner::V1(ledger) => ledger.is_live(conn).then_some(Plane::V1),
            LiveLedgerInner::Hybrid(ledger) => ledger.plane(conn),
        }
    }

    /// Keep the shared connection owner in step with the peer's advertised
    /// request queue. Callers should clamp this to their own request-window
    /// ceiling so retained request history stays bounded by live policy.
    pub fn set_remote_reqq(&mut self, conn: ConnectionId, remote_reqq: usize) {
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => ledger.set_remote_reqq(conn, remote_reqq),
            LiveLedgerInner::Hybrid(ledger) => ledger.set_remote_reqq(conn, remote_reqq),
        }
    }

    /// Reserve one block while keeping the complete canonical v1 piece on its
    /// connection. For a v2 connection, `block` uses v2 wire coordinates and
    /// the returned extent is the canonical v1 ledger coordinate.
    pub fn reserve_piece_affine_on_plane(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
    ) -> Result<BlockExtent, HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => {
                if !ledger.is_live(conn) {
                    return Err(HybridLedgerError::UnknownConnection(conn));
                }
                ledger
                    .reserve_piece_affine(conn, block)
                    .map_err(HybridLedgerError::Reserve)?;
                Ok(block)
            }
            LiveLedgerInner::Hybrid(ledger) => ledger.reserve_piece_affine(conn, block),
        }
    }

    pub fn commit_on_plane(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        now: Nanos,
    ) -> Result<(CommitOutcome, Vec<HybridAction>), HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => {
                if !ledger.is_live(conn) {
                    return Err(HybridLedgerError::UnknownConnection(conn));
                }
                let (outcome, actions) = ledger.commit(conn, block, now);
                Ok((outcome, v1_actions(actions)))
            }
            LiveLedgerInner::Hybrid(ledger) => ledger.commit(conn, block, now),
        }
    }

    pub fn on_reject_on_plane(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
    ) -> Result<RejectOutcome, HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => {
                if !ledger.is_live(conn) {
                    return Err(HybridLedgerError::UnknownConnection(conn));
                }
                Ok(ledger.on_reject(conn, block))
            }
            LiveLedgerInner::Hybrid(ledger) => ledger.on_reject(conn, block),
        }
    }

    pub fn on_block_piece_affine_on_plane(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        payload: Bytes,
    ) -> Result<(HybridReceiveOutcome, Option<HybridBlockData>), HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => {
                if !ledger.is_live(conn) {
                    return Err(HybridLedgerError::UnknownConnection(conn));
                }
                let expected = usize::try_from(block.length).unwrap_or(usize::MAX);
                if payload.len() != expected {
                    return Err(HybridLedgerError::InvalidPayloadLength {
                        expected,
                        actual: payload.len(),
                    });
                }
                match ledger.on_block_piece_affine(conn, block) {
                    ReceiveOutcome::Accepted {
                        completed,
                        late,
                        actions,
                    } => Ok((
                        HybridReceiveOutcome::Accepted {
                            completed,
                            late,
                            actions: v1_actions(actions),
                        },
                        Some(HybridBlockData {
                            canonical_extent: block,
                            payload,
                        }),
                    )),
                    ReceiveOutcome::Duplicate => Ok((HybridReceiveOutcome::Duplicate, None)),
                    ReceiveOutcome::IgnoredForPieceAffinity => {
                        Ok((HybridReceiveOutcome::IgnoredForPieceAffinity, None))
                    }
                    ReceiveOutcome::Unsolicited => Ok((HybridReceiveOutcome::Unsolicited, None)),
                    ReceiveOutcome::StaleConnection => {
                        Ok((HybridReceiveOutcome::StaleConnection, None))
                    }
                    ReceiveOutcome::Invalid(error) => {
                        Err(HybridLedgerError::InvalidGeometry(error))
                    }
                }
            }
            LiveLedgerInner::Hybrid(ledger) => ledger.on_block_piece_affine(conn, block, payload),
        }
    }

    pub fn on_choke_on_plane(
        &mut self,
        conn: ConnectionId,
        fast_negotiated: bool,
    ) -> Result<Vec<HybridWireBlock>, HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => {
                if !ledger.is_live(conn) {
                    return Err(HybridLedgerError::UnknownConnection(conn));
                }
                Ok(ledger
                    .on_choke(conn, fast_negotiated)
                    .into_iter()
                    .map(|extent| HybridWireBlock {
                        plane: Plane::V1,
                        extent,
                    })
                    .collect())
            }
            LiveLedgerInner::Hybrid(ledger) => ledger.on_choke(conn, fast_negotiated),
        }
    }

    pub fn disconnect_on_plane(
        &mut self,
        conn: ConnectionId,
    ) -> Result<Vec<HybridWireBlock>, HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => {
                if !ledger.is_live(conn) {
                    return Err(HybridLedgerError::UnknownConnection(conn));
                }
                Ok(ledger
                    .disconnect(conn)
                    .into_iter()
                    .map(|extent| HybridWireBlock {
                        plane: Plane::V1,
                        extent,
                    })
                    .collect())
            }
            LiveLedgerInner::Hybrid(ledger) => ledger.disconnect(conn),
        }
    }

    pub fn expire_on_plane(
        &mut self,
        now: Nanos,
        timeout: Nanos,
    ) -> Result<(Vec<(ConnectionId, HybridWireBlock)>, Vec<HybridAction>), HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => {
                let (expired, actions) = ledger.expire(now, timeout);
                Ok((
                    expired
                        .into_iter()
                        .map(|(conn, extent)| {
                            (
                                conn,
                                HybridWireBlock {
                                    plane: Plane::V1,
                                    extent,
                                },
                            )
                        })
                        .collect(),
                    v1_actions(actions),
                ))
            }
            LiveLedgerInner::Hybrid(ledger) => ledger.expire(now, timeout),
        }
    }

    pub fn piece_failed_with_actions_on_plane(
        &mut self,
        piece: u32,
    ) -> Result<(Vec<ConnectionId>, Vec<HybridAction>), HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => {
                let (contributors, actions) = ledger.piece_failed_with_actions(piece);
                Ok((contributors, v1_actions(actions)))
            }
            LiveLedgerInner::Hybrid(ledger) => ledger.piece_failed_with_actions(piece),
        }
    }

    pub fn mark_have_on_plane(
        &mut self,
        piece: u32,
    ) -> Result<Vec<HybridAction>, HybridLedgerError> {
        self.used = true;
        match &mut self.inner {
            LiveLedgerInner::V1(ledger) => Ok(v1_actions(ledger.mark_have(piece))),
            LiveLedgerInner::Hybrid(ledger) => ledger.mark_have(piece),
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

    #[test]
    fn peer_reqq_updates_the_live_v1_and_hybrid_ledger_bounds() {
        let piece_length = u64::from(BLOCK_SIZE) * 2;
        let path = SafeRelPath::from_name("payload.bin", false).unwrap();
        let v1 = Arc::new(
            PieceMap::new(
                piece_length,
                vec![FileSpan {
                    file_index: 0,
                    path: path.clone(),
                    content_offset: 0,
                    length: piece_length * 2,
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
                    length: piece_length * 2,
                    pad: false,
                }],
            )
            .unwrap(),
        );

        for hybrid in [false, true] {
            let mut ledger = LiveTransferLedger::new_bounded(Arc::clone(&v1), 2);
            if hybrid {
                assert!(ledger.attach_hybrid_geometry(Arc::new(HybridGeometry::new(
                    Arc::clone(&v1),
                    Arc::clone(&v2),
                ))));
            }
            let conn = ledger.try_connect(8).unwrap();
            ledger.set_remote_reqq(conn, 1);
            ledger
                .reserve(
                    conn,
                    BlockExtent::new(0, 0, BLOCK_SIZE),
                    &ReserveMode::Exclusive,
                )
                .unwrap();
            assert_eq!(
                ledger.reserve(
                    conn,
                    BlockExtent::new(0, BLOCK_SIZE, BLOCK_SIZE),
                    &ReserveMode::Exclusive,
                ),
                Err(ReserveError::ConnectionFull)
            );
        }
    }

    #[test]
    fn live_hybrid_v2_plane_returns_data_in_canonical_v1_coordinates() {
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
        let mut ledger = LiveTransferLedger::new_bounded(Arc::clone(&v1), 2);
        assert!(ledger.attach_hybrid_geometry(Arc::new(HybridGeometry::new(v1, v2))));

        let conn = ledger.try_connect_on_plane(Plane::V2, 2).unwrap();
        let wire_block = BlockExtent::new(0, 0, BLOCK_SIZE);
        assert_eq!(ledger.plane(conn), Some(Plane::V2));
        assert_eq!(
            ledger
                .reserve_piece_affine_on_plane(conn, wire_block)
                .unwrap(),
            wire_block
        );
        assert_eq!(
            ledger.commit_on_plane(conn, wire_block, 1).unwrap().0,
            CommitOutcome::OnWire
        );

        let payload = Bytes::from(vec![0xA5; BLOCK_SIZE as usize]);
        let (outcome, data) = ledger
            .on_block_piece_affine_on_plane(conn, wire_block, payload.clone())
            .unwrap();
        assert!(matches!(outcome, HybridReceiveOutcome::Accepted { .. }));
        let data = data.expect("accepted v2 response returns canonical data");
        assert_eq!(data.canonical_extent, wire_block);
        assert_eq!(data.payload, payload);
        assert_eq!(ledger.piece_progress(0), Some((1, 2)));
        ledger.check_invariants().unwrap();
    }
}
