//! One ownership ledger for the v1 and v2 wire views of a hybrid torrent.
//!
//! The shared ledger stores canonical v1 extents because the legacy hybrid
//! actor writes and verifies v1 pieces. Requests and responses from a v2 view
//! are projected to that same extent before arbitration. A v2 payload is staged
//! into a bounded, v1-sized block before ledger mutation, so a geometry
//! conversion failure cannot consume ownership. Staged bytes are returned only
//! when the response wins arbitration.

use std::collections::HashMap;

use bytes::Bytes;

use super::coords::{
    BlockExtent, ConnectionId, Epoch, GeometryError, HybridGeometry, PieceGeometry, Plane,
};
use super::ledger::{
    Action, CommitOutcome, Ledger, ReceiveOutcome, RejectOutcome, ReserveError, ReserveMode,
    MAX_CONNECTIONS,
};
use super::Nanos;

/// A block in the coordinate system spoken by one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HybridWireBlock {
    pub plane: Plane,
    pub extent: BlockExtent,
}

/// A cancel action translated back to the target connection's wire plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HybridAction {
    pub conn: ConnectionId,
    pub block: HybridWireBlock,
}

/// Canonical v1-sized data accepted from either hybrid wire view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HybridBlockData {
    pub canonical_extent: BlockExtent,
    pub payload: Bytes,
}

/// Response arbitration result with actions translated to each peer's plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HybridReceiveOutcome {
    Accepted {
        completed: Option<super::coords::PieceExtent>,
        late: bool,
        actions: Vec<HybridAction>,
    },
    Duplicate,
    IgnoredForPieceAffinity,
    Unsolicited,
    StaleConnection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HybridLedgerError {
    UnknownConnection(ConnectionId),
    InvalidGeometry(GeometryError),
    /// A valid wire block does not map to the same payload in the other view.
    NoContentMapping(HybridWireBlock),
    /// The response body must exactly match the wire request length.
    InvalidPayloadLength {
        expected: usize,
        actual: usize,
    },
    Reserve(ReserveError),
}

impl std::fmt::Display for HybridLedgerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownConnection(conn) => write!(f, "unknown hybrid connection {conn}"),
            Self::InvalidGeometry(error) => write!(f, "invalid hybrid block: {error}"),
            Self::NoContentMapping(block) => write!(
                f,
                "hybrid {:?} block {:?} has no matching payload extent",
                block.plane, block.extent
            ),
            Self::InvalidPayloadLength { expected, actual } => write!(
                f,
                "hybrid response length {actual} does not match request length {expected}"
            ),
            Self::Reserve(error) => write!(f, "hybrid ledger refused reservation: {error:?}"),
        }
    }
}

impl std::error::Error for HybridLedgerError {}

/// Dual-plane wire adapter around one canonical ownership ledger.
///
/// A connection is permanently bound to the plane used for its handshake.
/// The underlying ledger remains in v1 coordinates so a v1/hybrid writer has
/// one piece assembly and one owner for every payload block, regardless of
/// which view supplied it.
pub struct HybridLedger {
    ledger: Ledger<HybridGeometry>,
    connection_planes: HashMap<ConnectionId, Plane>,
}

impl HybridLedger {
    pub fn new(geometry: impl Into<std::sync::Arc<HybridGeometry>>) -> Self {
        Self::new_bounded(geometry, MAX_CONNECTIONS)
    }

    pub fn new_bounded(
        geometry: impl Into<std::sync::Arc<HybridGeometry>>,
        max_connections: usize,
    ) -> Self {
        // The ledger's keys are always canonical v1 extents. The geometry's
        // selected download plane must not change that internal coordinate
        // system when an adapter is reused by a v2 wire connection.
        let geometry = geometry.into();
        let geometry = geometry.as_ref().clone().with_download_plane(Plane::V1);
        Self {
            ledger: Ledger::new(geometry)
                .with_max_connections(max_connections.min(MAX_CONNECTIONS)),
            connection_planes: HashMap::new(),
        }
    }

    /// Conservative per-connection charge for the ledger slot and its plane
    /// index. The map-entry allowance includes its hash-table control/slack.
    pub fn estimated_connection_state_bytes() -> usize {
        Ledger::<HybridGeometry>::estimated_connection_state_bytes()
            .saturating_add(std::mem::size_of::<(ConnectionId, Plane)>() + 24)
    }

    pub fn try_connect(&mut self, plane: Plane, remote_reqq: usize) -> Option<ConnectionId> {
        let conn = self.ledger.try_connect(remote_reqq)?;
        self.connection_planes.insert(conn, plane);
        Some(conn)
    }

    pub fn connect(&mut self, plane: Plane, remote_reqq: usize) -> ConnectionId {
        self.try_connect(plane, remote_reqq)
            .expect("hybrid transfer ledger connection limit reached")
    }

    pub fn is_live(&self, conn: ConnectionId) -> bool {
        self.ledger.is_live(conn) && self.connection_planes.contains_key(&conn)
    }

    pub fn plane(&self, conn: ConnectionId) -> Option<Plane> {
        self.is_live(conn)
            .then(|| self.connection_planes.get(&conn).copied())
            .flatten()
    }

    pub fn set_remote_reqq(&mut self, conn: ConnectionId, reqq: usize) {
        if self.is_live(conn) {
            self.ledger.set_remote_reqq(conn, reqq);
        }
    }

    pub fn reserve(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        mode: &ReserveMode,
    ) -> Result<BlockExtent, HybridLedgerError> {
        let canonical = self.to_canonical(conn, block)?;
        self.ledger
            .reserve(conn, canonical, mode)
            .map_err(HybridLedgerError::Reserve)?;
        Ok(canonical)
    }

    /// Confirm that the writer placed the request on the wire.
    pub fn commit(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        now: Nanos,
    ) -> Result<(CommitOutcome, Vec<HybridAction>), HybridLedgerError> {
        let canonical = self.to_canonical(conn, block)?;
        let (outcome, actions) = self.ledger.commit(conn, canonical, now);
        Ok((outcome, self.translate_actions(actions)?))
    }

    pub fn on_reject(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
    ) -> Result<RejectOutcome, HybridLedgerError> {
        let canonical = self.to_canonical(conn, block)?;
        Ok(self.ledger.on_reject(conn, canonical))
    }

    /// Reconcile a response and return its canonical v1 payload only when the
    /// ledger accepts ownership. Callers must reserve enough response memory
    /// for the expanded v1 extent before passing a v2 payload here.
    pub fn on_block(
        &mut self,
        conn: ConnectionId,
        block: BlockExtent,
        payload: Bytes,
    ) -> Result<(HybridReceiveOutcome, Option<HybridBlockData>), HybridLedgerError> {
        let plane = self
            .plane(conn)
            .ok_or(HybridLedgerError::UnknownConnection(conn))?;
        let wire = HybridWireBlock {
            plane,
            extent: block,
        };
        let expected = usize::try_from(block.length).unwrap_or(usize::MAX);
        if payload.len() != expected {
            return Err(HybridLedgerError::InvalidPayloadLength {
                expected,
                actual: payload.len(),
            });
        }
        let canonical = self.to_canonical(conn, block)?;

        // Prepare the bounded expansion before mutating ledger state. It is
        // at most one wire block and the caller has already reserved its
        // response-capacity lease.
        let canonical_payload = match plane {
            Plane::V1 => payload,
            Plane::V2 => {
                let (extent, bytes) = self
                    .ledger
                    .geometry()
                    .expand_v2_block_to_v1(block, &payload)
                    .ok_or(HybridLedgerError::NoContentMapping(wire))?;
                if extent != canonical {
                    return Err(HybridLedgerError::NoContentMapping(wire));
                }
                Bytes::from(bytes)
            }
        };

        let outcome = self.ledger.on_block(conn, canonical);
        match outcome {
            ReceiveOutcome::Accepted {
                completed,
                late,
                actions,
            } => {
                let actions = self.translate_actions(actions)?;
                Ok((
                    HybridReceiveOutcome::Accepted {
                        completed,
                        late,
                        actions,
                    },
                    Some(HybridBlockData {
                        canonical_extent: canonical,
                        payload: canonical_payload,
                    }),
                ))
            }
            ReceiveOutcome::Duplicate => Ok((HybridReceiveOutcome::Duplicate, None)),
            ReceiveOutcome::IgnoredForPieceAffinity => {
                Ok((HybridReceiveOutcome::IgnoredForPieceAffinity, None))
            }
            ReceiveOutcome::Unsolicited => Ok((HybridReceiveOutcome::Unsolicited, None)),
            ReceiveOutcome::StaleConnection => Ok((HybridReceiveOutcome::StaleConnection, None)),
            ReceiveOutcome::Invalid(error) => Err(HybridLedgerError::InvalidGeometry(error)),
        }
    }

    pub fn on_choke(
        &mut self,
        conn: ConnectionId,
        fast_negotiated: bool,
    ) -> Result<Vec<HybridWireBlock>, HybridLedgerError> {
        let plane = self
            .plane(conn)
            .ok_or(HybridLedgerError::UnknownConnection(conn))?;
        self.ledger
            .on_choke(conn, fast_negotiated)
            .into_iter()
            .map(|canonical| self.to_wire(plane, canonical))
            .collect()
    }

    pub fn disconnect(
        &mut self,
        conn: ConnectionId,
    ) -> Result<Vec<HybridWireBlock>, HybridLedgerError> {
        let plane = self
            .connection_planes
            .remove(&conn)
            .ok_or(HybridLedgerError::UnknownConnection(conn))?;
        self.ledger
            .disconnect(conn)
            .into_iter()
            .map(|canonical| self.to_wire(plane, canonical))
            .collect()
    }

    pub fn begin_piece(&mut self, piece: u32) -> Result<(), ReserveError> {
        self.ledger.begin_piece(piece)
    }

    pub fn restore_partial_blocks(
        &mut self,
        piece: u32,
        received_blocks: &[u32],
    ) -> Result<bool, ReserveError> {
        self.ledger.restore_partial_blocks(piece, received_blocks)
    }

    pub fn discard_empty_piece(&mut self, piece: u32) -> bool {
        self.ledger.discard_empty_piece(piece)
    }

    pub fn expire(
        &mut self,
        now: Nanos,
        timeout: Nanos,
    ) -> Result<(Vec<(ConnectionId, HybridWireBlock)>, Vec<HybridAction>), HybridLedgerError> {
        let (expired, actions) = self.ledger.expire(now, timeout);
        let expired = expired
            .into_iter()
            .map(|(conn, canonical)| {
                let plane = self
                    .plane(conn)
                    .ok_or(HybridLedgerError::UnknownConnection(conn))?;
                Ok((conn, self.to_wire(plane, canonical)?))
            })
            .collect::<Result<Vec<_>, HybridLedgerError>>()?;
        Ok((expired, self.translate_actions(actions)?))
    }

    pub fn piece_failed_with_actions(
        &mut self,
        piece: u32,
    ) -> Result<(Vec<ConnectionId>, Vec<HybridAction>), HybridLedgerError> {
        let (contributors, actions) = self.ledger.piece_failed_with_actions(piece);
        Ok((contributors, self.translate_actions(actions)?))
    }

    pub fn mark_have(&mut self, piece: u32) -> Result<Vec<HybridAction>, HybridLedgerError> {
        let actions = self.ledger.mark_have(piece);
        self.translate_actions(actions)
    }

    pub fn piece_verified(&mut self, piece: u32) {
        self.ledger.piece_verified(piece);
    }

    pub fn advance_epoch(&mut self) -> Result<(Epoch, Vec<HybridAction>), HybridLedgerError> {
        let (epoch, actions) = self.ledger.advance_epoch();
        Ok((epoch, self.translate_actions(actions)?))
    }

    pub fn has_piece(&self, piece: u32) -> bool {
        self.ledger.has_piece(piece)
    }

    pub fn outstanding(&self, conn: ConnectionId) -> usize {
        self.ledger.outstanding(conn)
    }

    pub fn owns(&self, conn: ConnectionId, block: BlockExtent) -> Result<bool, HybridLedgerError> {
        let canonical = self.to_canonical(conn, block)?;
        Ok(self.ledger.owns(conn, canonical))
    }

    pub fn in_flight(&self) -> usize {
        self.ledger.in_flight()
    }

    pub fn pieces_in_progress(&self) -> usize {
        self.ledger.pieces_in_progress()
    }

    pub fn in_progress_bytes(&self) -> usize {
        self.ledger.in_progress_bytes()
    }

    pub fn piece_progress(&self, piece: u32) -> Option<(u32, u32)> {
        self.ledger.piece_progress(piece)
    }

    pub fn contributors(&self, piece: u32) -> Vec<ConnectionId> {
        self.ledger.contributors(piece)
    }

    pub fn estimated_piece_state_bytes(&self, piece: u32) -> Option<usize> {
        self.ledger.estimated_piece_state_bytes(piece)
    }

    pub fn epoch(&self) -> Epoch {
        self.ledger.epoch()
    }

    pub fn check_invariants(&self) -> Result<(), String> {
        self.ledger.check_invariants()?;
        if self
            .connection_planes
            .keys()
            .any(|conn| !self.ledger.is_live(*conn))
        {
            return Err("hybrid connection plane outlived its ledger generation".to_owned());
        }
        Ok(())
    }

    fn to_canonical(
        &self,
        conn: ConnectionId,
        block: BlockExtent,
    ) -> Result<BlockExtent, HybridLedgerError> {
        let plane = self
            .plane(conn)
            .ok_or(HybridLedgerError::UnknownConnection(conn))?;
        self.to_canonical_on(plane, block)
    }

    fn to_canonical_on(
        &self,
        plane: Plane,
        block: BlockExtent,
    ) -> Result<BlockExtent, HybridLedgerError> {
        let wire = HybridWireBlock {
            plane,
            extent: block,
        };
        match plane {
            Plane::V1 => self
                .ledger
                .geometry()
                .v1()
                .validate_request_block(block)
                .map(|()| block)
                .map_err(HybridLedgerError::InvalidGeometry),
            Plane::V2 => {
                self.ledger
                    .geometry()
                    .v2()
                    .validate_request_block(block)
                    .map_err(HybridLedgerError::InvalidGeometry)?;
                self.ledger
                    .geometry()
                    .canonical_v1_block_for_v2(block)
                    .ok_or(HybridLedgerError::NoContentMapping(wire))
            }
        }
    }

    fn to_wire(
        &self,
        plane: Plane,
        canonical: BlockExtent,
    ) -> Result<HybridWireBlock, HybridLedgerError> {
        let wire_extent = match plane {
            Plane::V1 => {
                self.ledger
                    .geometry()
                    .v1()
                    .validate_request_block(canonical)
                    .map_err(HybridLedgerError::InvalidGeometry)?;
                canonical
            }
            Plane::V2 => self
                .ledger
                .geometry()
                .canonical_v2_block_for_v1(canonical)
                .ok_or(HybridLedgerError::NoContentMapping(HybridWireBlock {
                    plane,
                    extent: canonical,
                }))?,
        };
        Ok(HybridWireBlock {
            plane,
            extent: wire_extent,
        })
    }

    fn translate_actions(
        &self,
        actions: Vec<Action>,
    ) -> Result<Vec<HybridAction>, HybridLedgerError> {
        actions
            .into_iter()
            .map(|action| match action {
                Action::Cancel { conn, block } => {
                    let plane = self
                        .plane(conn)
                        .ok_or(HybridLedgerError::UnknownConnection(conn))?;
                    Ok(HybridAction {
                        conn,
                        block: self.to_wire(plane, block)?,
                    })
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rt_path::SafeRelPath;
    use rt_piece_map::{FileSpan, PieceMap, V2FileSpan, V2PieceMap};

    fn geometry() -> HybridGeometry {
        let a = SafeRelPath::from_name("a.bin", false).unwrap();
        let b = SafeRelPath::from_name("b.bin", false).unwrap();
        let v1 = PieceMap::new_with_padding(
            32 * 1024,
            vec![
                FileSpan {
                    file_index: 0,
                    path: a.clone(),
                    content_offset: 0,
                    length: 40 * 1024,
                },
                FileSpan {
                    file_index: 1,
                    path: SafeRelPath::from_name("padding.bin", false).unwrap(),
                    content_offset: 40 * 1024,
                    length: 24 * 1024,
                },
                FileSpan {
                    file_index: 2,
                    path: b.clone(),
                    content_offset: 64 * 1024,
                    length: 24 * 1024,
                },
            ],
            [1],
        )
        .unwrap();
        let v2 = V2PieceMap::new(
            32 * 1024,
            vec![
                V2FileSpan {
                    file_index: 0,
                    path: a,
                    piece_offset: 0,
                    length: 40 * 1024,
                    pad: false,
                },
                V2FileSpan {
                    file_index: 1,
                    path: b,
                    piece_offset: 64 * 1024,
                    length: 24 * 1024,
                    pad: false,
                },
            ],
        )
        .unwrap();
        HybridGeometry::new(v1, v2)
    }

    #[test]
    fn v1_and_v2_requests_arbitrate_one_padded_payload_extent() {
        let mut ledger = HybridLedger::new(geometry());
        let v1 = ledger.connect(Plane::V1, 8);
        let v2 = ledger.connect(Plane::V2, 8);
        let v1_block = BlockExtent::new(1, 0, 16 * 1024);
        let v2_block = BlockExtent::new(1, 0, 8 * 1024);

        assert_eq!(
            ledger.reserve(v1, v1_block, &ReserveMode::Exclusive),
            Ok(v1_block)
        );
        assert_eq!(
            ledger.reserve(v2, v2_block, &ReserveMode::Exclusive),
            Err(HybridLedgerError::Reserve(ReserveError::OwnedByOther))
        );
        assert_eq!(
            ledger.reserve(v2, v2_block, &ReserveMode::Endgame),
            Ok(v1_block)
        );
        assert_eq!(
            ledger.commit(v2, v2_block, 10).unwrap().0,
            CommitOutcome::OnWire
        );

        let (outcome, data) = ledger
            .on_block(v2, v2_block, Bytes::from(vec![0x5a; 8 * 1024]))
            .unwrap();
        let HybridReceiveOutcome::Accepted { actions, .. } = outcome else {
            panic!("the v2 response should win shared ownership");
        };
        let data = data.expect("accepted payload is returned to the writer");
        assert_eq!(data.canonical_extent, v1_block);
        assert_eq!(data.payload.len(), 16 * 1024);
        assert!(data.payload[..8 * 1024].iter().all(|byte| *byte == 0x5a));
        assert!(data.payload[8 * 1024..].iter().all(|byte| *byte == 0));
        assert_eq!(
            actions,
            vec![HybridAction {
                conn: v1,
                block: HybridWireBlock {
                    plane: Plane::V1,
                    extent: v1_block,
                },
            }]
        );
        assert_eq!(
            ledger
                .on_block(v1, v1_block, Bytes::from(vec![0x5a; 16 * 1024]))
                .unwrap()
                .0,
            HybridReceiveOutcome::Duplicate
        );
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn ownership_actions_are_projected_back_to_the_requesting_plane() {
        let mut ledger = HybridLedger::new(geometry());
        let v2 = ledger.connect(Plane::V2, 8);
        let v2_block = BlockExtent::new(1, 0, 8 * 1024);
        ledger
            .reserve(v2, v2_block, &ReserveMode::Exclusive)
            .unwrap();

        let actions = ledger.mark_have(1).unwrap();
        assert_eq!(
            actions,
            vec![HybridAction {
                conn: v2,
                block: HybridWireBlock {
                    plane: Plane::V2,
                    extent: v2_block,
                },
            }]
        );
    }

    #[test]
    fn v2_requests_cannot_smuggle_v1_padding_coordinates() {
        let mut ledger = HybridLedger::new(geometry());
        let v2 = ledger.connect(Plane::V2, 8);
        let padding_only = BlockExtent::new(1, 16 * 1024, 16 * 1024);

        assert!(matches!(
            ledger.reserve(v2, padding_only, &ReserveMode::Exclusive),
            Err(HybridLedgerError::InvalidGeometry(_))
                | Err(HybridLedgerError::NoContentMapping(_))
        ));
        assert_eq!(ledger.in_flight(), 0);
    }

    #[test]
    fn reusing_a_connection_slot_does_not_reuse_its_wire_plane() {
        let mut ledger = HybridLedger::new(geometry());
        let old = ledger.connect(Plane::V1, 8);
        ledger.disconnect(old).unwrap();
        let new = ledger.connect(Plane::V2, 8);

        assert_eq!(old.slot, new.slot);
        assert_ne!(old.generation, new.generation);
        assert_eq!(ledger.plane(old), None);
        assert_eq!(ledger.plane(new), Some(Plane::V2));
        ledger.check_invariants().unwrap();
    }

    #[test]
    fn malformed_response_length_does_not_consume_the_reservation() {
        let mut ledger = HybridLedger::new(geometry());
        let v2 = ledger.connect(Plane::V2, 8);
        let block = BlockExtent::new(1, 0, 8 * 1024);
        ledger.reserve(v2, block, &ReserveMode::Exclusive).unwrap();

        assert_eq!(
            ledger.on_block(v2, block, Bytes::from_static(b"short")),
            Err(HybridLedgerError::InvalidPayloadLength {
                expected: 8 * 1024,
                actual: 5,
            })
        );
        assert_eq!(ledger.outstanding(v2), 1);
        assert_eq!(ledger.in_flight(), 1);
        ledger.check_invariants().unwrap();
    }
}
