//! Typed transfer coordinates and protocol geometry adapters (NET-101).
//!
//! A *block* extent (what a peer requests and sends) and a *piece* extent
//! (what is hashed and persisted as a unit) are different types, because
//! confusing them is exactly the pure-v2 defect this rebuild started from.
//! Equal piece indexes also do not imply equal content ranges across the v1
//! and v2 views of a hybrid torrent, so geometry exposes canonical content
//! ranges and callers compare those, never indexes.

use std::fmt;
use std::sync::Arc;

use bytes::Bytes;
use rt_path::SafeRelPath;
use rt_piece_map::{PieceMap, V2PieceMap};

/// BEP 3 block size the core requests; also the largest block it will serve.
pub const BLOCK_SIZE: u32 = 16 * 1024;

/// A wire block: bytes `[begin, begin + length)` of one piece.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BlockExtent {
    pub piece: u32,
    pub begin: u32,
    pub length: u32,
}

impl BlockExtent {
    pub fn new(piece: u32, begin: u32, length: u32) -> Self {
        Self {
            piece,
            begin,
            length,
        }
    }

    /// Index of this block within its piece when aligned to [`BLOCK_SIZE`].
    pub fn block_index(&self) -> u32 {
        self.begin / BLOCK_SIZE
    }

    pub fn end(&self) -> u64 {
        u64::from(self.begin) + u64::from(self.length)
    }
}

/// A complete piece: the unit that is verified, written and advertised.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PieceExtent {
    pub piece: u32,
    pub length: u32,
}

/// A canonical content range, independent of piece numbering.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ContentRange {
    /// Root-confined identity used to compare ranges across v1 and v2 views.
    /// Numeric file indexes are retained for storage lookup but can differ
    /// when v1 includes BEP 47 padding entries.
    pub path: Option<SafeRelPath>,
    pub file_index: u32,
    pub file_offset: u64,
    pub length: u64,
    /// BEP 47 padding: synthetic zero content, no payload on disk.
    pub pad: bool,
}

/// A file range as it appears inside one wire block.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlockContentRange {
    /// Byte offset within the peer-wire block.
    pub block_offset: u32,
    pub content: ContentRange,
}

/// Which piece numbering a geometry describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Plane {
    V1,
    V2,
}

/// Identity of one peer connection. A socket address is not an identity: a
/// reconnect reuses it. The generation distinguishes the replacement, so a
/// late event from the old connection can never mutate the new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConnectionId {
    pub slot: u32,
    pub generation: u32,
}

/// Runtime identity for one live torrent task. It is process-local and must
/// not be reused while work from an earlier task generation may still exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TorrentKey(pub u64);

/// Process-wide identity for a peer connection. `ConnectionId` is only unique
/// within one torrent ledger, so shared transfer services use this composite
/// key when they cover multiple torrent tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TransferPeerId {
    pub torrent: TorrentKey,
    pub connection: ConnectionId,
}

impl TransferPeerId {
    pub const fn new(torrent: TorrentKey, connection: ConnectionId) -> Self {
        Self {
            torrent,
            connection,
        }
    }
}

impl fmt::Display for ConnectionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "conn#{}.{}", self.slot, self.generation)
    }
}

/// Content epoch of a torrent's payload. It advances whenever the payload
/// identity changes (move, recheck, content change, pause/resume of writes);
/// work started in an older epoch is quiesced, never completed into the new
/// one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Epoch(pub u64);

impl Epoch {
    pub fn next(self) -> Self {
        Epoch(self.0.wrapping_add(1))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GeometryError {
    PieceOutOfRange {
        piece: u32,
        piece_count: u32,
    },
    ZeroLength,
    BlockTooLarge {
        length: u32,
    },
    InvalidContentMap {
        piece: u32,
    },
    OutOfBounds {
        piece: u32,
        end: u64,
        piece_len: u32,
    },
    Unaligned {
        begin: u32,
    },
    NonCanonicalBlock {
        piece: u32,
        begin: u32,
        length: u32,
    },
}

impl fmt::Display for GeometryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PieceOutOfRange { piece, piece_count } => {
                write!(f, "piece {piece} outside 0..{piece_count}")
            }
            Self::ZeroLength => write!(f, "zero-length block"),
            Self::BlockTooLarge { length } => write!(f, "block of {length} bytes is too large"),
            Self::InvalidContentMap { piece } => {
                write!(
                    f,
                    "piece {piece} content map does not cover its full extent"
                )
            }
            Self::OutOfBounds {
                piece,
                end,
                piece_len,
            } => write!(
                f,
                "block ends at {end} past piece {piece} length {piece_len}"
            ),
            Self::Unaligned { begin } => write!(f, "block begin {begin} is not block aligned"),
            Self::NonCanonicalBlock {
                piece,
                begin,
                length,
            } => write!(
                f,
                "block ({piece}, {begin}, {length}) is not a full canonical block extent"
            ),
        }
    }
}

impl std::error::Error for GeometryError {}

/// Piece geometry for one protocol plane.
pub trait PieceGeometry {
    fn plane(&self) -> Plane;
    fn piece_count(&self) -> u32;
    /// Length of `piece`, or `None` when out of range.
    fn piece_len(&self, piece: u32) -> Option<u32>;
    /// The content `piece` covers, in piece order.
    fn content_ranges(&self, piece: u32) -> Vec<ContentRange>;

    /// Project a wire block onto its file ranges, preserving synthetic pad
    /// spans. Piece maps return ranges in piece order, so their lengths also
    /// identify where a block intersects each file when v1 padding shifts the
    /// v2 view's block coordinates.
    fn block_content_ranges(
        &self,
        block: BlockExtent,
    ) -> Result<Vec<BlockContentRange>, GeometryError> {
        self.validate_served_block(block)?;
        let piece_len = u64::from(self.piece_len(block.piece).ok_or(
            GeometryError::PieceOutOfRange {
                piece: block.piece,
                piece_count: self.piece_count(),
            },
        )?);
        let block_start = u64::from(block.begin);
        let block_end = block.end();
        let mut piece_offset = 0u64;
        let mut projected = Vec::new();

        for mut range in self.content_ranges(block.piece) {
            let range_end = piece_offset
                .checked_add(range.length)
                .filter(|end| *end <= piece_len)
                .ok_or(GeometryError::InvalidContentMap { piece: block.piece })?;
            let overlap_start = block_start.max(piece_offset);
            let overlap_end = block_end.min(range_end);
            if overlap_start < overlap_end {
                let block_offset = u32::try_from(overlap_start - block_start)
                    .map_err(|_| GeometryError::InvalidContentMap { piece: block.piece })?;
                range.file_offset = range
                    .file_offset
                    .checked_add(overlap_start - piece_offset)
                    .ok_or(GeometryError::InvalidContentMap { piece: block.piece })?;
                range.length = overlap_end - overlap_start;
                projected.push(BlockContentRange {
                    block_offset,
                    content: range,
                });
            }
            piece_offset = range_end;
        }

        if piece_offset != piece_len {
            return Err(GeometryError::InvalidContentMap { piece: block.piece });
        }
        Ok(projected)
    }

    fn piece_extent(&self, piece: u32) -> Option<PieceExtent> {
        self.piece_len(piece)
            .map(|length| PieceExtent { piece, length })
    }

    /// Number of [`BLOCK_SIZE`] blocks in `piece`.
    fn block_count(&self, piece: u32) -> Option<u32> {
        self.piece_len(piece).map(|len| len.div_ceil(BLOCK_SIZE))
    }

    /// The `index`-th aligned block of `piece` (the last one may be short).
    fn block_extent(&self, piece: u32, index: u32) -> Option<BlockExtent> {
        let len = self.piece_len(piece)?;
        let begin = index.checked_mul(BLOCK_SIZE)?;
        if begin >= len {
            return None;
        }
        Some(BlockExtent::new(
            piece,
            begin,
            (len - begin).min(BLOCK_SIZE),
        ))
    }

    /// Validate an acquisition block: it must equal the full aligned extent
    /// used by the ledger, including a short final block when the piece ends.
    fn validate_request_block(&self, block: BlockExtent) -> Result<(), GeometryError> {
        self.validate_served_block(block)?;
        if !block.begin.is_multiple_of(BLOCK_SIZE) {
            return Err(GeometryError::Unaligned { begin: block.begin });
        }
        if self.block_extent(block.piece, block.block_index()) != Some(block) {
            return Err(GeometryError::NonCanonicalBlock {
                piece: block.piece,
                begin: block.begin,
                length: block.length,
            });
        }
        Ok(())
    }

    /// Validate a block a peer asks us to serve: in range, non-empty, at most
    /// [`BLOCK_SIZE`]; alignment is the peer's business.
    fn validate_served_block(&self, block: BlockExtent) -> Result<(), GeometryError> {
        let piece_len = self
            .piece_len(block.piece)
            .ok_or(GeometryError::PieceOutOfRange {
                piece: block.piece,
                piece_count: self.piece_count(),
            })?;
        if block.length == 0 {
            return Err(GeometryError::ZeroLength);
        }
        if block.length > BLOCK_SIZE {
            return Err(GeometryError::BlockTooLarge {
                length: block.length,
            });
        }
        if block.end() > u64::from(piece_len) {
            return Err(GeometryError::OutOfBounds {
                piece: block.piece,
                end: block.end(),
                piece_len,
            });
        }
        Ok(())
    }
}

impl PieceGeometry for PieceMap {
    fn plane(&self) -> Plane {
        Plane::V1
    }

    fn piece_count(&self) -> u32 {
        self.piece_count
    }

    fn piece_len(&self, piece: u32) -> Option<u32> {
        PieceMap::piece_len(self, piece).ok()
    }

    fn content_ranges(&self, piece: u32) -> Vec<ContentRange> {
        self.piece_to_file_regions(piece)
            .map(|regions| {
                regions
                    .into_iter()
                    .map(|region| ContentRange {
                        path: Some(region.path),
                        file_index: region.file_index,
                        file_offset: region.file_offset,
                        length: region.length,
                        pad: region.pad,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl<G: PieceGeometry + ?Sized> PieceGeometry for std::sync::Arc<G> {
    fn plane(&self) -> Plane {
        self.as_ref().plane()
    }

    fn piece_count(&self) -> u32 {
        self.as_ref().piece_count()
    }

    fn piece_len(&self, piece: u32) -> Option<u32> {
        self.as_ref().piece_len(piece)
    }

    fn content_ranges(&self, piece: u32) -> Vec<ContentRange> {
        self.as_ref().content_ranges(piece)
    }
}

impl PieceGeometry for V2PieceMap {
    fn plane(&self) -> Plane {
        Plane::V2
    }

    fn piece_count(&self) -> u32 {
        self.piece_count
    }

    fn piece_len(&self, piece: u32) -> Option<u32> {
        V2PieceMap::piece_len(self, piece).ok()
    }

    fn content_ranges(&self, piece: u32) -> Vec<ContentRange> {
        self.piece_to_file(piece)
            .map(|region| {
                vec![ContentRange {
                    path: Some(region.path),
                    file_index: region.file_index,
                    file_offset: region.file_offset,
                    length: u64::from(region.length),
                    pad: region.pad,
                }]
            })
            .unwrap_or_default()
    }
}

/// A hybrid torrent carries both views. Acquisition initially runs on one
/// plane while verification must satisfy both; mixed-plane acquisition is a
/// later capability gate, so nothing here assumes `v1` piece `n` and `v2`
/// piece `n` are the same bytes.
#[derive(Debug, Clone)]
pub struct HybridGeometry {
    v1: Arc<PieceMap>,
    v2: Arc<V2PieceMap>,
    download_plane: Plane,
}

impl HybridGeometry {
    pub fn new(v1: impl Into<Arc<PieceMap>>, v2: impl Into<Arc<V2PieceMap>>) -> Self {
        Self {
            v1: v1.into(),
            v2: v2.into(),
            download_plane: Plane::V1,
        }
    }

    /// Select which protocol view supplies piece coordinates to
    /// [`PieceGeometry`]. V1 remains the default for callers that are still
    /// migrating from the legacy hybrid actor.
    pub fn with_download_plane(mut self, plane: Plane) -> Self {
        self.download_plane = plane;
        self
    }

    pub fn v1(&self) -> &PieceMap {
        self.v1.as_ref()
    }

    pub fn v2(&self) -> &V2PieceMap {
        self.v2.as_ref()
    }

    /// The plane requests are scheduled in.
    pub fn download_plane(&self) -> Plane {
        self.download_plane
    }

    /// Canonical content ranges of a v1 piece that no v2 file range covers
    /// identically; callers use it to decide whether one read can feed both
    /// hash views (see the batched hash service).
    pub fn ranges_match(&self, v1_piece: u32, v2_piece: u32) -> bool {
        let v1 = self
            .v1
            .content_ranges(v1_piece)
            .into_iter()
            .filter(|range| !range.pad)
            .collect::<Vec<_>>();
        let v2 = self
            .v2
            .content_ranges(v2_piece)
            .into_iter()
            .filter(|range| !range.pad)
            .collect::<Vec<_>>();
        same_content_ranges(&v1, &v2)
    }

    /// Whether two wire blocks from different planes cover the same real
    /// payload bytes. Padding bytes are omitted because they have no v2 file
    /// range and are synthesized by the hybrid v1 view.
    pub fn blocks_match_content(&self, v1: BlockExtent, v2: BlockExtent) -> bool {
        let Ok(v1_ranges) = self.v1.block_content_ranges(v1) else {
            return false;
        };
        let Ok(v2_ranges) = self.v2.block_content_ranges(v2) else {
            return false;
        };
        same_block_payload_ranges(&v1_ranges, &v2_ranges)
    }

    /// Return the v1 ledger extent that represents a valid aligned v2 request.
    /// In a padded hybrid, the v1 extent can be longer because its trailing
    /// bytes include synthetic padding.
    pub fn canonical_v1_block_for_v2(&self, v2_block: BlockExtent) -> Option<BlockExtent> {
        self.v2.validate_request_block(v2_block).ok()?;
        let v1_block = self
            .v1
            .block_extent(v2_block.piece, v2_block.block_index())?;
        self.blocks_match_content(v1_block, v2_block)
            .then_some(v1_block)
    }

    /// Return the v2 wire extent corresponding to a canonical v1 ledger
    /// extent. A v1 extent may contain synthetic padding, so its length cannot
    /// be copied directly into a v2 Cancel or request coordinate.
    pub fn canonical_v2_block_for_v1(&self, v1_block: BlockExtent) -> Option<BlockExtent> {
        self.v1.validate_request_block(v1_block).ok()?;
        let v2_block = self
            .v2
            .block_extent(v1_block.piece, v1_block.block_index())?;
        self.blocks_match_content(v1_block, v2_block)
            .then_some(v2_block)
    }

    /// Expand a v2 payload block into its v1 ledger extent, filling BEP 47
    /// padding with zeros. The caller must hold a lease for the returned
    /// v1-sized buffer before calling this bounded, pure conversion.
    pub fn expand_v2_block_to_v1(
        &self,
        v2_block: BlockExtent,
        payload: &[u8],
    ) -> Option<(BlockExtent, Vec<u8>)> {
        if payload.len() != usize::try_from(v2_block.length).ok()? {
            return None;
        }
        let v1_block = self.canonical_v1_block_for_v2(v2_block)?;
        let source_ranges = self.v2.block_content_ranges(v2_block).ok()?;
        let target_ranges = self.v1.block_content_ranges(v1_block).ok()?;
        if !same_block_payload_ranges(&target_ranges, &source_ranges) {
            return None;
        }

        let mut expanded = vec![0; usize::try_from(v1_block.length).ok()?];
        let mut source_payload = source_ranges.iter().filter(|range| !range.content.pad);
        for target in target_ranges.iter().filter(|range| !range.content.pad) {
            let source = source_payload.next()?;
            let length = usize::try_from(target.content.length).ok()?;
            let target_start = usize::try_from(target.block_offset).ok()?;
            let source_start = usize::try_from(source.block_offset).ok()?;
            let target_end = target_start.checked_add(length)?;
            let source_end = source_start.checked_add(length)?;
            expanded
                .get_mut(target_start..target_end)?
                .copy_from_slice(payload.get(source_start..source_end)?);
        }
        Some((v1_block, expanded))
    }

    /// Project a canonical v1 block payload onto the corresponding v2 wire
    /// extent, omitting synthetic BEP 47 padding. The caller must reserve
    /// capacity for any copied output before calling. When the real v2 bytes
    /// are contiguous inside the v1 block, the returned `Bytes` is a zero-copy
    /// view and keeps the original allocation alive.
    pub fn project_v1_block_to_v2(
        &self,
        v1_block: BlockExtent,
        payload: Bytes,
    ) -> Option<(BlockExtent, Bytes)> {
        if payload.len() != usize::try_from(v1_block.length).ok()? {
            return None;
        }
        let v2_block = self.canonical_v2_block_for_v1(v1_block)?;
        let source_ranges = self.v1.block_content_ranges(v1_block).ok()?;
        let target_ranges = self.v2.block_content_ranges(v2_block).ok()?;
        if !same_block_payload_ranges(&source_ranges, &target_ranges) {
            return None;
        }

        let mut source_payload = source_ranges.iter().filter(|range| !range.content.pad);
        let source_fast = source_payload.next();
        let source_is_single = source_payload.next().is_none();
        let mut target_payload = target_ranges.iter().filter(|range| !range.content.pad);
        let target_fast = target_payload.next();
        let target_is_single = target_payload.next().is_none();
        let target_len = usize::try_from(v2_block.length).ok()?;
        if source_is_single && target_is_single {
            let source = source_fast?;
            let target = target_fast?;
            let source_start = usize::try_from(source.block_offset).ok()?;
            let target_start = usize::try_from(target.block_offset).ok()?;
            let source_end = source_start.checked_add(target_len)?;
            let target_end = target_start.checked_add(target_len)?;
            if source.content.length == u64::try_from(target_len).ok()?
                && target.content.length == u64::try_from(target_len).ok()?
                && target_start == 0
                && target_end == target_len
                && source_end <= payload.len()
            {
                return Some((v2_block, payload.slice(source_start..source_end)));
            }
        }

        let mut projected = vec![0; target_len];
        let mut source_payload = source_ranges.iter().filter(|range| !range.content.pad);
        for target in target_ranges.iter().filter(|range| !range.content.pad) {
            let source = source_payload.next()?;
            let length = usize::try_from(target.content.length).ok()?;
            let source_start = usize::try_from(source.block_offset).ok()?;
            let target_start = usize::try_from(target.block_offset).ok()?;
            let source_end = source_start.checked_add(length)?;
            let target_end = target_start.checked_add(length)?;
            projected
                .get_mut(target_start..target_end)?
                .copy_from_slice(payload.get(source_start..source_end)?);
        }
        if source_payload.next().is_some() {
            return None;
        }
        Some((v2_block, Bytes::from(projected)))
    }
}

fn same_content_ranges(left: &[ContentRange], right: &[ContentRange]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| same_content_range(left, right))
}

fn same_content_range(left: &ContentRange, right: &ContentRange) -> bool {
    let same_file = match (&left.path, &right.path) {
        (Some(left), Some(right)) => left == right,
        (None, None) => left.file_index == right.file_index,
        _ => false,
    };
    same_file && left.file_offset == right.file_offset && left.length == right.length
}

fn same_block_payload_ranges(left: &[BlockContentRange], right: &[BlockContentRange]) -> bool {
    let mut left = left.iter().filter(|range| !range.content.pad);
    let mut right = right.iter().filter(|range| !range.content.pad);
    let mut matched_payload = false;
    loop {
        match (left.next(), right.next()) {
            (Some(left), Some(right)) if same_content_range(&left.content, &right.content) => {
                matched_payload = true;
            }
            (None, None) => return matched_payload,
            _ => return false,
        }
    }
}

impl PieceGeometry for HybridGeometry {
    fn plane(&self) -> Plane {
        self.download_plane
    }

    fn piece_count(&self) -> u32 {
        match self.download_plane {
            Plane::V1 => self.v1.piece_count,
            Plane::V2 => self.v2.piece_count,
        }
    }

    fn piece_len(&self, piece: u32) -> Option<u32> {
        match self.download_plane {
            Plane::V1 => PieceGeometry::piece_len(&self.v1, piece),
            Plane::V2 => PieceGeometry::piece_len(&self.v2, piece),
        }
    }

    fn content_ranges(&self, piece: u32) -> Vec<ContentRange> {
        match self.download_plane {
            Plane::V1 => self.v1.content_ranges(piece),
            Plane::V2 => self.v2.content_ranges(piece),
        }
    }
}

/// A fixed-size synthetic geometry for tests and the simulator: `piece_count`
/// pieces of `piece_len` bytes, with an optional shorter final piece.
#[derive(Debug, Clone)]
pub struct UniformGeometry {
    pub plane: Plane,
    pub piece_len: u32,
    pub piece_count: u32,
    pub last_piece_len: u32,
}

impl UniformGeometry {
    pub fn new(piece_len: u32, piece_count: u32) -> Self {
        Self {
            plane: Plane::V1,
            piece_len,
            piece_count,
            last_piece_len: piece_len,
        }
    }

    pub fn with_short_last(mut self, last_piece_len: u32) -> Self {
        self.last_piece_len = last_piece_len;
        self
    }
}

impl PieceGeometry for UniformGeometry {
    fn plane(&self) -> Plane {
        self.plane
    }

    fn piece_count(&self) -> u32 {
        self.piece_count
    }

    fn piece_len(&self, piece: u32) -> Option<u32> {
        if piece >= self.piece_count {
            None
        } else if piece + 1 == self.piece_count {
            Some(self.last_piece_len)
        } else {
            Some(self.piece_len)
        }
    }

    fn content_ranges(&self, piece: u32) -> Vec<ContentRange> {
        self.piece_len(piece)
            .map(|length| {
                vec![ContentRange {
                    path: None,
                    file_index: 0,
                    file_offset: u64::from(piece) * u64::from(self.piece_len),
                    length: u64::from(length),
                    pad: false,
                }]
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rt_path::SafeRelPath;
    use rt_piece_map::{FileSpan, V2FileSpan};

    fn v1_map() -> PieceMap {
        // Two files of 40 KiB and 24 KiB over 32 KiB pieces: piece 1 spans both.
        PieceMap::new(
            32 * 1024,
            vec![
                FileSpan {
                    file_index: 0,
                    path: SafeRelPath::from_name("a.bin", false).unwrap(),
                    content_offset: 0,
                    length: 40 * 1024,
                },
                FileSpan {
                    file_index: 1,
                    path: SafeRelPath::from_name("b.bin", false).unwrap(),
                    content_offset: 40 * 1024,
                    length: 24 * 1024,
                },
            ],
        )
        .unwrap()
    }

    fn v2_map() -> V2PieceMap {
        V2PieceMap::new(
            32 * 1024,
            vec![
                V2FileSpan {
                    file_index: 0,
                    path: SafeRelPath::from_name("a.bin", false).unwrap(),
                    piece_offset: 0,
                    length: 40 * 1024,
                    pad: false,
                },
                V2FileSpan {
                    file_index: 1,
                    path: SafeRelPath::from_name("b.bin", false).unwrap(),
                    piece_offset: 64 * 1024,
                    length: 24 * 1024,
                    pad: false,
                },
            ],
        )
        .unwrap()
    }

    #[test]
    fn blocks_are_aligned_and_the_last_may_be_short() {
        let geometry = UniformGeometry::new(40_000, 3).with_short_last(20_000);
        assert_eq!(geometry.block_count(0), Some(3));
        assert_eq!(
            geometry.block_extent(0, 2),
            Some(BlockExtent::new(0, 32_768, 7_232))
        );
        assert_eq!(geometry.block_count(2), Some(2));
        assert_eq!(
            geometry.block_extent(2, 1),
            Some(BlockExtent::new(2, 16_384, 3_616))
        );
        assert_eq!(geometry.block_extent(2, 2), None);
        assert_eq!(geometry.piece_extent(3), None);
    }

    #[test]
    fn block_and_piece_extents_are_distinct_types() {
        // The v2 defect: a block's extent used where the piece's was required.
        let geometry = UniformGeometry::new(32 * 1024, 4);
        let block = geometry.block_extent(1, 1).unwrap();
        let piece = geometry.piece_extent(1).unwrap();
        assert_eq!(block.length, 16 * 1024);
        assert_eq!(piece.length, 32 * 1024);
    }

    #[test]
    fn served_blocks_are_validated_but_not_required_aligned() {
        let geometry = UniformGeometry::new(32 * 1024, 2);
        assert!(geometry
            .validate_served_block(BlockExtent::new(0, 100, 4096))
            .is_ok());
        assert_eq!(
            geometry.validate_request_block(BlockExtent::new(0, 100, 4096)),
            Err(GeometryError::Unaligned { begin: 100 })
        );
        assert_eq!(
            geometry.validate_served_block(BlockExtent::new(0, 0, 0)),
            Err(GeometryError::ZeroLength)
        );
        assert_eq!(
            geometry.validate_served_block(BlockExtent::new(0, 0, BLOCK_SIZE + 1)),
            Err(GeometryError::BlockTooLarge {
                length: BLOCK_SIZE + 1
            })
        );
        assert!(matches!(
            geometry.validate_served_block(BlockExtent::new(0, 20 * 1024, BLOCK_SIZE)),
            Err(GeometryError::OutOfBounds { .. })
        ));
        assert!(matches!(
            geometry.validate_served_block(BlockExtent::new(9, 0, 1)),
            Err(GeometryError::PieceOutOfRange { .. })
        ));
    }

    #[test]
    fn acquisition_blocks_must_match_the_canonical_extent() {
        let geometry = UniformGeometry::new(32 * 1024, 2);
        assert_eq!(
            geometry.validate_request_block(BlockExtent::new(0, 0, 8 * 1024)),
            Err(GeometryError::NonCanonicalBlock {
                piece: 0,
                begin: 0,
                length: 8 * 1024,
            })
        );
        assert!(geometry
            .validate_request_block(BlockExtent::new(0, 0, BLOCK_SIZE))
            .is_ok());
        assert!(geometry
            .validate_request_block(BlockExtent::new(1, BLOCK_SIZE, BLOCK_SIZE))
            .is_ok());
    }

    #[test]
    fn v1_piece_can_span_files_while_v2_piece_never_does() {
        let v1 = v1_map();
        let ranges = PieceGeometry::content_ranges(&v1, 1);
        assert_eq!(ranges.len(), 2, "v1 piece 1 straddles both files");
        let v2 = v2_map();
        for piece in 0..PieceGeometry::piece_count(&v2) {
            assert_eq!(PieceGeometry::content_ranges(&v2, piece).len(), 1);
        }
    }

    #[test]
    fn equal_piece_indexes_do_not_imply_equal_content() {
        let hybrid = HybridGeometry::new(v1_map(), v2_map());
        assert_eq!(hybrid.download_plane(), Plane::V1);
        // v1 piece 0 and v2 piece 0 are both the first 32 KiB of a.bin...
        assert!(hybrid.ranges_match(0, 0));
        // ...but v1 piece 1 spans a.bin's tail and b.bin, while v2 piece 1 is
        // only a.bin's tail: same index, different content.
        assert!(!hybrid.ranges_match(1, 1));
    }

    #[test]
    fn hybrid_geometry_can_select_the_v2_download_view() {
        let hybrid = HybridGeometry::new(v1_map(), v2_map()).with_download_plane(Plane::V2);

        assert_eq!(hybrid.download_plane(), Plane::V2);
        assert_eq!(hybrid.piece_count(), 3);
        assert_eq!(hybrid.piece_len(2), Some(24 * 1024));
        assert_eq!(
            hybrid.content_ranges(2)[0]
                .path
                .as_ref()
                .unwrap()
                .as_display(),
            "b.bin"
        );
    }

    #[test]
    fn hybrid_geometry_shares_prebuilt_piece_maps() {
        let v1 = Arc::new(v1_map());
        let v2 = Arc::new(v2_map());
        let hybrid = HybridGeometry::new(Arc::clone(&v1), Arc::clone(&v2));

        assert_eq!(Arc::strong_count(&v1), 2);
        assert_eq!(Arc::strong_count(&v2), 2);
        assert_eq!(
            hybrid.piece_count(),
            PieceGeometry::piece_count(v1.as_ref())
        );
    }

    #[test]
    fn hybrid_ranges_match_across_v1_padding_index_shift() {
        let a_path = SafeRelPath::from_name("a.bin", false).unwrap();
        let b_path = SafeRelPath::from_name("b.bin", false).unwrap();
        let v1 = PieceMap::new_with_padding(
            32 * 1024,
            vec![
                FileSpan {
                    file_index: 0,
                    path: a_path.clone(),
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
                    path: b_path.clone(),
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
                    path: a_path,
                    piece_offset: 0,
                    length: 40 * 1024,
                    pad: false,
                },
                V2FileSpan {
                    file_index: 1,
                    path: b_path,
                    piece_offset: 64 * 1024,
                    length: 24 * 1024,
                    pad: false,
                },
            ],
        )
        .unwrap();
        let hybrid = HybridGeometry::new(v1, v2);

        assert!(hybrid.ranges_match(0, 0));
        assert!(hybrid.ranges_match(1, 1));
        assert!(hybrid.ranges_match(2, 2));
        assert!(!hybrid.ranges_match(1, 2));

        // The v1 wire block includes 8 KiB of real data plus 8 KiB of
        // synthetic padding; the v2 plane requests only the real 8 KiB.
        assert!(hybrid.blocks_match_content(
            BlockExtent::new(1, 0, BLOCK_SIZE),
            BlockExtent::new(1, 0, 8 * 1024),
        ));
        assert!(hybrid.blocks_match_content(
            BlockExtent::new(2, 0, BLOCK_SIZE),
            BlockExtent::new(2, 0, BLOCK_SIZE),
        ));
        assert!(hybrid.blocks_match_content(
            BlockExtent::new(2, BLOCK_SIZE, 8 * 1024),
            BlockExtent::new(2, BLOCK_SIZE, 8 * 1024),
        ));
        let v2_tail = BlockExtent::new(1, 0, 8 * 1024);
        let v2_tail_data = vec![0x5a; 8 * 1024];
        let (v1_tail, expanded) = hybrid
            .expand_v2_block_to_v1(v2_tail, &v2_tail_data)
            .expect("v2 tail maps to its padded v1 wire block");
        assert_eq!(v1_tail, BlockExtent::new(1, 0, BLOCK_SIZE));
        assert_eq!(hybrid.canonical_v2_block_for_v1(v1_tail), Some(v2_tail));
        assert!(expanded[..8 * 1024].iter().all(|byte| *byte == 0x5a));
        assert!(expanded[8 * 1024..].iter().all(|byte| *byte == 0));
        let expanded = Bytes::from(expanded);
        let (projected_tail, projected) = hybrid
            .project_v1_block_to_v2(v1_tail, expanded.clone())
            .expect("padded v1 block projects back to its shorter v2 payload");
        assert_eq!(projected_tail, v2_tail);
        assert_eq!(projected.as_ref(), v2_tail_data.as_slice());
        assert_eq!(projected.as_ptr(), expanded.as_ptr());

        let second_file_v1 = BlockExtent::new(2, 0, 16 * 1024);
        let second_file_v2 = BlockExtent::new(2, 0, 16 * 1024);
        let second_file_payload = vec![0xA6; 16 * 1024];
        let second_file_payload = Bytes::from(second_file_payload);
        let (projected_second_file, projected) = hybrid
            .project_v1_block_to_v2(second_file_v1, second_file_payload.clone())
            .expect("second-file v1 block projects after the preceding pad extent");
        assert_eq!(projected_second_file, second_file_v2);
        assert_eq!(projected.as_ref(), second_file_payload.as_ref());
        assert_eq!(projected.as_ptr(), second_file_payload.as_ptr());
        assert_eq!(
            hybrid.canonical_v2_block_for_v1(BlockExtent::new(1, BLOCK_SIZE, BLOCK_SIZE)),
            None,
            "a padding-only v1 extent has no v2 wire range"
        );
        assert!(!hybrid.blocks_match_content(
            BlockExtent::new(1, BLOCK_SIZE, BLOCK_SIZE),
            BlockExtent::new(1, 0, 8 * 1024),
        ));
        assert!(!hybrid.blocks_match_content(
            BlockExtent::new(1, 0, BLOCK_SIZE),
            BlockExtent::new(2, 0, 16 * 1024),
        ));
    }

    #[test]
    fn connection_generations_distinguish_reused_slots() {
        let first = ConnectionId {
            slot: 7,
            generation: 1,
        };
        let second = ConnectionId {
            slot: 7,
            generation: 2,
        };
        assert_ne!(first, second);
        assert_eq!(Epoch::default().next(), Epoch(1));
    }
}
