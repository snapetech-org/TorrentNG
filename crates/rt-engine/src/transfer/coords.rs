//! Typed transfer coordinates and protocol geometry adapters (NET-101).
//!
//! A *block* extent (what a peer requests and sends) and a *piece* extent
//! (what is hashed and persisted as a unit) are different types, because
//! confusing them is exactly the pure-v2 defect this rebuild started from.
//! Equal piece indexes also do not imply equal content ranges across the v1
//! and v2 views of a hybrid torrent, so geometry exposes canonical content
//! ranges and callers compare those, never indexes.

use std::fmt;

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentRange {
    pub file_index: u32,
    pub file_offset: u64,
    pub length: u64,
    /// BEP 47 padding: synthetic zero content, no payload on disk.
    pub pad: bool,
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
    OutOfBounds {
        piece: u32,
        end: u64,
        piece_len: u32,
    },
    Unaligned {
        begin: u32,
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
            Self::OutOfBounds {
                piece,
                end,
                piece_len,
            } => write!(
                f,
                "block ends at {end} past piece {piece} length {piece_len}"
            ),
            Self::Unaligned { begin } => write!(f, "block begin {begin} is not block aligned"),
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

    /// Validate a block we are about to request: in range and aligned.
    fn validate_request_block(&self, block: BlockExtent) -> Result<(), GeometryError> {
        self.validate_served_block(block)?;
        if !block.begin.is_multiple_of(BLOCK_SIZE) {
            return Err(GeometryError::Unaligned { begin: block.begin });
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
pub struct HybridGeometry {
    v1: PieceMap,
    v2: V2PieceMap,
    download_plane: Plane,
}

impl HybridGeometry {
    pub fn new(v1: PieceMap, v2: V2PieceMap) -> Self {
        Self {
            v1,
            v2,
            download_plane: Plane::V1,
        }
    }

    pub fn v1(&self) -> &PieceMap {
        &self.v1
    }

    pub fn v2(&self) -> &V2PieceMap {
        &self.v2
    }

    /// The plane requests are scheduled in.
    pub fn download_plane(&self) -> Plane {
        self.download_plane
    }

    /// Canonical content ranges of a v1 piece that no v2 file range covers
    /// identically; callers use it to decide whether one read can feed both
    /// hash views (see the batched hash service).
    pub fn ranges_match(&self, v1_piece: u32, v2_piece: u32) -> bool {
        let strip = |ranges: Vec<ContentRange>| {
            ranges
                .into_iter()
                .filter(|range| !range.pad)
                .collect::<Vec<_>>()
        };
        strip(self.v1.content_ranges(v1_piece)) == strip(self.v2.content_ranges(v2_piece))
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
