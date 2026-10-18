//! Production-shaped v1, v2, and hybrid torrent fixtures.
//!
//! The corpus deliberately includes a multi-piece file, a nested file, an
//! empty file, and (for hybrid) BEP 47 padding. Payload bytes and all hashes
//! are content-derived so parser, geometry, and storage tests exercise the
//! same relationships they receive from real metainfo.

use rt_bencode::{encode, BValue};
use rt_hash::{merkle_root, piece_layer_hash, BlockHash, InfoHashV2, V2_BLOCK_SIZE};
use rt_metainfo::{parse_torrent, torrent_info_bytes, TorrentMeta};
use rt_piece_map::{FileSpan, PieceMap, V2FileSpan, V2PieceMap};
use sha1::{Digest, Sha1};

/// Geometry shared by the fixture's v1 and v2 views.
pub const TRANSFER_FIXTURE_PIECE_LENGTH: usize = 32 * 1024;

/// Protocol shape represented by a compatibility fixture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferProtocol {
    V1,
    V2,
    Hybrid,
}

impl TransferProtocol {
    fn has_v1(self) -> bool {
        matches!(self, Self::V1 | Self::Hybrid)
    }

    fn has_v2(self) -> bool {
        matches!(self, Self::V2 | Self::Hybrid)
    }
}

/// One actual payload file in a transfer fixture. Synthetic hybrid padding
/// is present in metainfo but intentionally absent from this payload list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferFixtureFile {
    pub path: Vec<&'static str>,
    pub bytes: Vec<u8>,
}

/// Deterministic torrent bytes plus the payload needed to verify their hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransferCompatibilityFixture {
    pub protocol: TransferProtocol,
    pub torrent: Vec<u8>,
    pub payload_files: Vec<TransferFixtureFile>,
    pub piece_length: usize,
}

/// Build one repeatable, private transfer fixture for a protocol shape.
pub fn transfer_compatibility_fixture(protocol: TransferProtocol) -> TransferCompatibilityFixture {
    let payload_files = vec![
        TransferFixtureFile {
            path: vec!["alpha.bin"],
            bytes: deterministic_payload(0, 33_000),
        },
        TransferFixtureFile {
            path: vec!["empty.bin"],
            bytes: Vec::new(),
        },
        TransferFixtureFile {
            path: vec!["nested", "beta.bin"],
            bytes: deterministic_payload(1, 9_001),
        },
    ];
    let torrent = build_torrent(protocol, &payload_files);

    TransferCompatibilityFixture {
        protocol,
        torrent,
        payload_files,
        piece_length: TRANSFER_FIXTURE_PIECE_LENGTH,
    }
}

/// Build the v1, pure-v2, and hybrid cases in a stable order.
pub fn transfer_compatibility_matrix() -> Vec<TransferCompatibilityFixture> {
    [
        TransferProtocol::V1,
        TransferProtocol::V2,
        TransferProtocol::Hybrid,
    ]
    .into_iter()
    .map(transfer_compatibility_fixture)
    .collect()
}

/// Build valid, deterministic one-file v1 metainfo for a dormant production
/// catalog fixture. The info dictionary contains a unique display name and a
/// content-derived piece hash, so each result has a distinct real info hash.
/// The one-byte payload keeps large catalog setup bounded; these fixtures are
/// for metadata, persistence, restore, and API paths rather than transfer-rate
/// measurements.
pub fn production_catalog_metainfo(index: usize) -> Vec<u8> {
    let name = format!("catalog-{index:010}");
    let payload_files = [TransferFixtureFile {
        path: vec!["payload.bin"],
        bytes: deterministic_payload(index, 1),
    }];
    build_torrent_named(TransferProtocol::V1, name.as_bytes(), &payload_files)
}

#[derive(Debug)]
struct V1FileEntry {
    path: Vec<String>,
    length: usize,
    padding: bool,
}

#[derive(Debug)]
struct V2FileHashes {
    pieces_root: Option<[u8; 32]>,
    piece_layer: Option<Vec<u8>>,
}

fn deterministic_payload(file_index: usize, length: usize) -> Vec<u8> {
    (0..length)
        .map(|offset| ((offset.wrapping_mul(37) + file_index * 101) % 251) as u8)
        .collect()
}

fn build_torrent(protocol: TransferProtocol, payload_files: &[TransferFixtureFile]) -> Vec<u8> {
    build_torrent_named(protocol, b"transfer-compatibility", payload_files)
}

fn build_torrent_named(
    protocol: TransferProtocol,
    torrent_name: &[u8],
    payload_files: &[TransferFixtureFile],
) -> Vec<u8> {
    let mut info_fields = vec![
        (b"name".as_slice(), BValue::Bytes(torrent_name)),
        (
            b"piece length".as_slice(),
            BValue::Int(TRANSFER_FIXTURE_PIECE_LENGTH as i64),
        ),
        (b"private".as_slice(), BValue::Int(1)),
    ];

    let mut v1_entries = None;
    let mut v1_pieces = None;
    if protocol.has_v1() {
        let (entries, stream) = build_v1_view(payload_files, protocol == TransferProtocol::Hybrid);
        v1_entries = Some(entries);
        v1_pieces = Some(sha1_piece_hashes(&stream));
        info_fields.push((
            b"files".as_slice(),
            v1_entries_value(v1_entries.as_deref().expect("v1 entries were set")),
        ));
        info_fields.push((
            b"pieces".as_slice(),
            BValue::Bytes(v1_pieces.as_deref().expect("v1 hashes were set")),
        ));
    }

    let v2_hashes = if protocol.has_v2() {
        payload_files
            .iter()
            .map(|file| v2_file_hashes(&file.bytes))
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };

    let piece_layers = if protocol.has_v2() {
        let tree = v2_file_tree(&v2_hashes);
        info_fields.push((b"file tree".as_slice(), tree));
        info_fields.push((b"meta version".as_slice(), BValue::Int(2)));
        Some(v2_piece_layers(&v2_hashes))
    } else {
        None
    };

    // Keep the owned buffers above alive until the bencode tree has been
    // encoded; the bencode values borrow their bytes.
    let info = sorted_dict(info_fields);
    let mut root_fields = vec![
        (
            b"announce".as_slice(),
            BValue::Bytes(b"http://tracker.fixture.invalid/announce"),
        ),
        (b"info".as_slice(), info),
    ];
    if let Some(layers) = piece_layers {
        root_fields.push((b"piece layers".as_slice(), layers));
    }
    encode(&sorted_dict(root_fields))
}

fn build_v1_view(
    payload_files: &[TransferFixtureFile],
    align_files: bool,
) -> (Vec<V1FileEntry>, Vec<u8>) {
    let mut entries = Vec::new();
    let mut stream = Vec::new();

    for file in payload_files {
        if align_files && !file.bytes.is_empty() {
            let remainder = stream.len() % TRANSFER_FIXTURE_PIECE_LENGTH;
            let padding_length =
                (TRANSFER_FIXTURE_PIECE_LENGTH - remainder) % TRANSFER_FIXTURE_PIECE_LENGTH;
            if padding_length != 0 {
                entries.push(V1FileEntry {
                    path: vec![".pad".to_owned(), padding_length.to_string()],
                    length: padding_length,
                    padding: true,
                });
                stream.resize(stream.len() + padding_length, 0);
            }
        }

        entries.push(V1FileEntry {
            path: file.path.iter().map(|part| (*part).to_owned()).collect(),
            length: file.bytes.len(),
            padding: false,
        });
        stream.extend_from_slice(&file.bytes);
    }

    (entries, stream)
}

fn v1_entries_value<'a>(entries: &'a [V1FileEntry]) -> BValue<'a> {
    let files = entries
        .iter()
        .map(|entry| {
            let mut fields = vec![
                (b"length".as_slice(), BValue::Int(entry.length as i64)),
                (
                    b"path".as_slice(),
                    BValue::List(
                        entry
                            .path
                            .iter()
                            .map(|part| BValue::Bytes(part.as_bytes()))
                            .collect(),
                    ),
                ),
            ];
            if entry.padding {
                fields.push((b"attr".as_slice(), BValue::Bytes(b"p")));
            }
            sorted_dict(fields)
        })
        .collect();
    BValue::List(files)
}

fn sha1_piece_hashes(stream: &[u8]) -> Vec<u8> {
    let mut pieces = Vec::with_capacity(stream.len().div_ceil(TRANSFER_FIXTURE_PIECE_LENGTH) * 20);
    for piece in stream.chunks(TRANSFER_FIXTURE_PIECE_LENGTH) {
        pieces.extend_from_slice(&Sha1::digest(piece));
    }
    pieces
}

fn v2_file_hashes(data: &[u8]) -> V2FileHashes {
    if data.is_empty() {
        return V2FileHashes {
            pieces_root: None,
            piece_layer: None,
        };
    }

    if data.len() > TRANSFER_FIXTURE_PIECE_LENGTH {
        let piece_hashes = data
            .chunks(TRANSFER_FIXTURE_PIECE_LENGTH)
            .map(|piece| {
                piece_layer_hash(piece, TRANSFER_FIXTURE_PIECE_LENGTH)
                    .expect("fixture piece geometry is valid")
            })
            .collect::<Vec<_>>();
        let root = merkle_root(&piece_hashes);
        let mut layer = Vec::with_capacity(piece_hashes.len() * 32);
        for hash in piece_hashes {
            layer.extend_from_slice(&hash);
        }
        V2FileHashes {
            pieces_root: Some(root),
            piece_layer: Some(layer),
        }
    } else {
        let leaves = data
            .chunks(V2_BLOCK_SIZE)
            .map(|block| BlockHash::of(block).0)
            .collect::<Vec<_>>();
        V2FileHashes {
            pieces_root: Some(merkle_root(&leaves)),
            piece_layer: None,
        }
    }
}

fn v2_file_tree<'a>(hashes: &'a [V2FileHashes]) -> BValue<'a> {
    let alpha = file_tree_node(33_000, hashes[0].pieces_root.as_ref());
    let empty = file_tree_node(0, hashes[1].pieces_root.as_ref());
    let beta = file_tree_node(9_001, hashes[2].pieces_root.as_ref());
    let nested = sorted_dict(vec![(b"beta.bin".as_slice(), beta)]);
    sorted_dict(vec![
        (b"alpha.bin".as_slice(), alpha),
        (b"empty.bin".as_slice(), empty),
        (b"nested".as_slice(), nested),
    ])
}

fn file_tree_node<'a>(length: i64, pieces_root: Option<&'a [u8; 32]>) -> BValue<'a> {
    let mut fields = vec![(b"length".as_slice(), BValue::Int(length))];
    if let Some(root) = pieces_root {
        fields.push((b"pieces root".as_slice(), BValue::Bytes(root)));
    }
    sorted_dict(vec![(b"".as_slice(), sorted_dict(fields))])
}

fn v2_piece_layers<'a>(hashes: &'a [V2FileHashes]) -> BValue<'a> {
    let layers = hashes
        .iter()
        .filter_map(|file| {
            Some((
                file.pieces_root.as_ref()?.as_slice(),
                BValue::Bytes(file.piece_layer.as_deref()?),
            ))
        })
        .collect();
    sorted_dict(layers)
}

fn sorted_dict<'a>(mut fields: Vec<(&'a [u8], BValue<'a>)>) -> BValue<'a> {
    fields.sort_by(|left, right| left.0.cmp(right.0));
    BValue::Dict(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_matrix_is_deterministic_and_parses_all_protocol_views() {
        let fixtures = transfer_compatibility_matrix();

        assert_eq!(
            fixtures
                .iter()
                .map(|fixture| fixture.protocol)
                .collect::<Vec<_>>(),
            vec![
                TransferProtocol::V1,
                TransferProtocol::V2,
                TransferProtocol::Hybrid
            ]
        );
        for fixture in fixtures {
            let repeated = transfer_compatibility_fixture(fixture.protocol);
            assert_eq!(fixture.torrent, repeated.torrent);
            assert_eq!(fixture.payload_files, repeated.payload_files);
            assert_eq!(fixture.piece_length, TRANSFER_FIXTURE_PIECE_LENGTH);

            let parsed = parse_torrent(&fixture.torrent).unwrap();
            assert!(parsed.is_private());
            assert_eq!(parsed.name(), "transfer-compatibility");
            match (fixture.protocol, parsed) {
                (TransferProtocol::V1, TorrentMeta::V1(v1)) => {
                    assert_eq!(v1.files.iter().filter(|file| !file.pad).count(), 3);
                    assert_eq!(v1.files.iter().filter(|file| file.pad).count(), 0);
                    assert_eq!(v1.pieces.len(), 2);
                    assert_eq!(v1.files[2].path.as_display(), "nested/beta.bin");
                    assert_eq!(v1.files[2].offset, 33_000);
                    assert_info_hashes_match_encoded_info(&fixture, Some(&v1.info_hash), None);
                    assert_v1_hashes_match_payload(&fixture, &v1, false);
                    let map = piece_map_from_v1(&v1.files, v1.piece_length);
                    assert_eq!(map.total_length, 42_001);
                    assert_eq!(map.piece_count, 2);
                }
                (TransferProtocol::V2, TorrentMeta::V2(v2)) => {
                    assert_eq!(v2.files.len(), 3);
                    assert_eq!(v2.total_length(), 42_001);
                    assert_eq!(v2.piece_count(), 3);
                    assert_eq!(v2.files[0].path.as_display(), "alpha.bin");
                    assert_eq!(v2.files[0].piece_offset, 0);
                    assert_eq!(v2.files[1].length, 0);
                    assert_eq!(v2.files[2].path.as_display(), "nested/beta.bin");
                    assert_eq!(v2.files[2].piece_offset, 65_536);
                    assert_eq!(v2.piece_layers.len(), 1);
                    assert_info_hashes_match_encoded_info(&fixture, None, Some(&v2.info_hash_v2));
                    assert_v2_hashes_match_payload(&fixture, &v2);
                    let map = piece_map_from_v2(&v2.files, v2.piece_length);
                    assert_eq!(map.logical_length, 74_537);
                    assert_eq!(map.piece_count, 3);
                    let alpha_tail = map.piece_to_file(1).unwrap();
                    assert_eq!(alpha_tail.file_index, v2.files[0].index);
                    assert_eq!(alpha_tail.file_offset, TRANSFER_FIXTURE_PIECE_LENGTH as u64);
                    assert_eq!(alpha_tail.length, 232);
                    let beta_piece = map.piece_to_file(2).unwrap();
                    assert_eq!(beta_piece.file_index, v2.files[2].index);
                    assert_eq!(beta_piece.file_offset, 0);
                    assert_eq!(beta_piece.length, 9_001);
                    assert!(map
                        .piece_ranges_for_file_indices(&[v2.files[1].index])
                        .unwrap()
                        .is_empty());
                }
                (TransferProtocol::Hybrid, TorrentMeta::Hybrid(v1, v2)) => {
                    assert_eq!(v1.pieces.len(), 3);
                    assert_eq!(v1.files.iter().filter(|file| file.pad).count(), 1);
                    assert_eq!(v1.files[2].path.as_display(), ".pad/32536");
                    assert!(v1.files[2].pad);
                    assert_eq!(v1.files[2].length, 32_536);
                    assert_eq!(v1.files[3].offset, 65_536);
                    assert_info_hashes_match_encoded_info(
                        &fixture,
                        Some(&v1.info_hash),
                        Some(&v2.info_hash_v2),
                    );
                    assert_v1_hashes_match_payload(&fixture, &v1, true);
                    assert_eq!(v2.total_length(), 42_001);
                    assert_eq!(v2.logical_length(), 74_537);
                    assert_eq!(v2.piece_count(), 3);
                    assert_v2_hashes_match_payload(&fixture, &v2);
                    let v2_map = piece_map_from_v2(&v2.files, v2.piece_length);
                    assert_eq!(v2_map.logical_length, map.total_length);
                    assert_eq!(v2_map.piece_count, 3);
                    assert!(v2_map
                        .piece_ranges_for_file_indices(&[v2.files[1].index])
                        .unwrap()
                        .is_empty());

                    let pad_indexes = v1
                        .files
                        .iter()
                        .filter(|file| file.pad)
                        .map(|file| file.index)
                        .collect::<Vec<_>>();
                    let map = piece_map_from_v1(&v1.files, v1.piece_length);
                    assert_eq!(map.total_length, v2.logical_length());
                    assert!(map
                        .piece_ranges_for_file_indices(&pad_indexes)
                        .unwrap()
                        .is_empty());
                    for pad_index in pad_indexes {
                        assert!(map
                            .piece_to_file_regions(1)
                            .unwrap()
                            .iter()
                            .any(|region| region.file_index == pad_index && region.pad));
                    }
                }
                (protocol, parsed) => panic!("fixture {protocol:?} parsed as {parsed:?}"),
            }
        }
    }

    #[test]
    fn production_catalog_metainfo_has_unique_content_derived_identity() {
        let first = production_catalog_metainfo(0);
        let repeated = production_catalog_metainfo(0);
        let second = production_catalog_metainfo(1);

        assert_eq!(first, repeated);
        let first_meta = parse_torrent(&first).expect("first catalog metainfo parses");
        let second_meta = parse_torrent(&second).expect("second catalog metainfo parses");

        assert_eq!(first_meta.name(), "catalog-0000000000");
        assert_eq!(first_meta.v1_info_hash().unwrap().len(), 20);
        assert_ne!(first_meta.v1_info_hash(), second_meta.v1_info_hash());
        assert!(first_meta.is_private());
        let TorrentMeta::V1(first_v1) = first_meta else {
            panic!("catalog fixture must be v1");
        };
        assert_eq!(first_v1.total_length(), 1);
        assert_eq!(first_v1.files.len(), 1);
        assert_eq!(first_v1.pieces.len(), 1);
    }

    fn piece_map_from_v1(files: &[rt_metainfo::TorrentFileV1], piece_length: u64) -> PieceMap {
        let spans = files
            .iter()
            .map(|file| FileSpan {
                file_index: file.index,
                path: file.path.clone(),
                content_offset: file.offset,
                length: file.length,
            })
            .collect::<Vec<_>>();
        let padding = files
            .iter()
            .filter(|file| file.pad)
            .map(|file| file.index)
            .collect::<Vec<_>>();
        PieceMap::new_with_padding(piece_length, spans, padding).unwrap()
    }

    fn piece_map_from_v2(files: &[rt_metainfo::TorrentFileV2], piece_length: u64) -> V2PieceMap {
        let spans = files
            .iter()
            .map(|file| V2FileSpan {
                file_index: file.index,
                path: file.path.clone(),
                piece_offset: file.piece_offset,
                length: file.length,
                pad: file.pad,
            })
            .collect();
        V2PieceMap::new(piece_length, spans).unwrap()
    }

    fn assert_v1_hashes_match_payload(
        fixture: &TransferCompatibilityFixture,
        v1: &rt_metainfo::TorrentMetaV1,
        align_files: bool,
    ) {
        let (_, stream) = build_v1_view(&fixture.payload_files, align_files);
        let expected = sha1_piece_hashes(&stream)
            .chunks_exact(20)
            .map(|hash| hash.try_into().expect("SHA-1 digest width is fixed"))
            .collect::<Vec<[u8; 20]>>();
        assert_eq!(v1.pieces, expected);
    }

    fn assert_info_hashes_match_encoded_info(
        fixture: &TransferCompatibilityFixture,
        v1_hash: Option<&[u8; 20]>,
        v2_hash: Option<&[u8; 32]>,
    ) {
        let info = torrent_info_bytes(&fixture.torrent).unwrap();
        if let Some(expected) = v1_hash {
            let actual: [u8; 20] = Sha1::digest(&info).into();
            assert_eq!(&actual, expected);
        }
        if let Some(expected) = v2_hash {
            assert_eq!(InfoHashV2::of_info_bytes(&info).0, *expected);
        }
    }

    fn assert_v2_hashes_match_payload(
        fixture: &TransferCompatibilityFixture,
        v2: &rt_metainfo::TorrentMetaV2,
    ) {
        let long_file = &fixture.payload_files[0];
        let piece_hashes = long_file
            .bytes
            .chunks(fixture.piece_length)
            .map(|piece| piece_layer_hash(piece, fixture.piece_length).unwrap())
            .collect::<Vec<_>>();
        let expected_root = merkle_root(&piece_hashes);
        assert_eq!(v2.files[0].pieces_root, Some(expected_root));
        assert_eq!(v2.piece_layers.get(&expected_root), Some(&piece_hashes));

        let short_file = &fixture.payload_files[2];
        let short_leaves = short_file
            .bytes
            .chunks(V2_BLOCK_SIZE)
            .map(|block| BlockHash::of(block).0)
            .collect::<Vec<_>>();
        assert_eq!(v2.files[2].pieces_root, Some(merkle_root(&short_leaves)));
    }
}
