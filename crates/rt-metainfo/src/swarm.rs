//! Public counterparts retain content proofs, but have their own swarm identity.
//! Never enable discovery on a private infohash or copy private discovery URLs.

use rt_bencode::{decode_torrent_info_span_with_allocation_reservation, encode, BValue};

use crate::{parse_torrent_with_allocation_reservation, TorrentMeta, TorrentMetaV1, TorrentMetaV2};

/// Bound the extra decode/compare work independently of ordinary torrent adds.
pub const MAX_SWARM_METAINFO_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicVariant {
    RemovePrivate,
    PrivateZero,
    RemovePrivateAndSource,
    PrivateZeroWithoutSource,
}

/// Construct only an info dictionary and (for v2) its authenticated piece layers.
/// Uploaded public metainfo keeps its exact info bytes and therefore its identity.
pub fn public_counterpart(
    raw: &[u8],
    variant: Option<PublicVariant>,
) -> Result<TorrentMeta, String> {
    let mut allocated = 0usize;
    public_counterpart_with_allocation_reservation(raw, variant, &mut |bytes| {
        allocated = allocated.saturating_add(bytes);
        allocated <= crate::MAX_CONVENIENCE_METAINFO_ALLOCATION_BYTES
    })
}

pub fn public_counterpart_with_allocation_reservation(
    raw: &[u8],
    variant: Option<PublicVariant>,
    reserve: &mut impl FnMut(usize) -> bool,
) -> Result<TorrentMeta, String> {
    if raw.len() > MAX_SWARM_METAINFO_BYTES {
        return Err("Swarm Switch metainfo exceeds 8 MiB".into());
    }
    let meta =
        parse_torrent_with_allocation_reservation(raw, reserve).map_err(|e| e.to_string())?;
    if variant.is_some() != meta.is_private() {
        return Err(if variant.is_some() {
            "Only private torrents can create a public counterpart"
        } else {
            "The supplied torrent is private; choose a public .torrent"
        }
        .into());
    }
    drop(meta);
    let (mut root, span) = decode_torrent_info_span_with_allocation_reservation(raw, reserve)
        .map_err(|e| e.to_string())?;
    let span = span.ok_or("Missing info dictionary")?;
    if !reserve(raw.len().saturating_add(16).saturating_mul(4)) {
        return Err("Swarm metainfo memory budget exhausted".into());
    }
    let mut output = Vec::with_capacity(raw.len() + 16);
    output.extend_from_slice(b"d4:info");
    if let Some(variant) = variant {
        let BValue::Dict(ref mut top) = root else {
            return Err("Invalid metainfo dictionary".into());
        };
        let index = top
            .iter()
            .position(|(key, _)| *key == b"info")
            .ok_or("Missing info dictionary")?;
        let mut info = top.remove(index).1;
        let BValue::Dict(ref mut fields) = info else {
            return Err("Invalid info dictionary".into());
        };
        fields.retain(|(key, _)| {
            *key != b"private"
                && (!matches!(
                    variant,
                    PublicVariant::RemovePrivateAndSource | PublicVariant::PrivateZeroWithoutSource
                ) || *key != b"source")
        });
        if matches!(
            variant,
            PublicVariant::PrivateZero | PublicVariant::PrivateZeroWithoutSource
        ) {
            fields.push((b"private", BValue::Int(0)));
        }
        fields.sort_by(|a, b| a.0.cmp(b.0));
        output.extend(encode(&info));
    } else {
        output.extend_from_slice(&raw[span]);
    }
    if let Some(layers) = root.get(b"piece layers") {
        output.extend_from_slice(b"12:piece layers");
        output.extend(encode(layers));
    }
    output.push(b'e');
    parse_torrent_with_allocation_reservation(&output, reserve).map_err(|e| e.to_string())
}

/// Sharing a writeable payload requires proofs of identical bytes, not names or sizes.
/// V1 needs matching piece boundaries/hashes. V2 file roots prove equality even
/// across different piece lengths. Cross-version and path remapping are rejected.
pub fn validate_swarm_payload(original: &TorrentMeta, public: &TorrentMeta) -> Result<(), String> {
    if !original.is_private() || public.is_private() {
        return Err("Swarm Switch requires a private original and a public counterpart".into());
    }
    match (original, public) {
        (TorrentMeta::V1(a), TorrentMeta::V1(b)) => compare_v1(a, b),
        (TorrentMeta::V2(a), TorrentMeta::V2(b)) => compare_v2(a, b),
        (TorrentMeta::Hybrid(a, av2), TorrentMeta::Hybrid(b, bv2)) => {
            compare_v1(a, b)?;
            compare_v2(av2, bv2)
        }
        _ => Err(
            "Torrent versions differ; use a counterpart with the same v1/v2/hybrid format".into(),
        ),
    }
}

fn compare_v1(a: &TorrentMetaV1, b: &TorrentMetaV1) -> Result<(), String> {
    if a.piece_length != b.piece_length {
        return Err("V1 piece sizes differ; the shared payload cannot be proven identical".into());
    }
    if a.pieces != b.pieces {
        return Err("Piece hashes differ; this public torrent is not an identical payload".into());
    }
    if a.files.len() != b.files.len()
        || a.files.iter().zip(&b.files).any(|(a, b)| {
            a.path != b.path || a.length != b.length || a.offset != b.offset || a.pad != b.pad
        })
    {
        return Err(
            "File paths, order, sizes, or padding differ; shared storage requires an exact layout"
                .into(),
        );
    }
    Ok(())
}

fn compare_v2(a: &TorrentMetaV2, b: &TorrentMetaV2) -> Result<(), String> {
    if a.files.len() != b.files.len()
        || a.files.iter().zip(&b.files).any(|(a, b)| {
            a.path != b.path
                || a.length != b.length
                || a.pad != b.pad
                || a.pieces_root != b.pieces_root
        })
    {
        return Err("V2 file paths, sizes, padding, or content roots differ".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_torrent;

    fn fixture(private: bool, piece_length: i64) -> Vec<u8> {
        let mut info = vec![
            (b"length".as_slice(), BValue::Int(4)),
            (b"name", BValue::Bytes(b"file")),
            (b"piece length", BValue::Int(piece_length)),
            (b"pieces", BValue::Bytes(b"aaaaaaaaaaaaaaaaaaaa")),
            (b"source", BValue::Bytes(b"tracker-name")),
        ];
        if private {
            info.push((b"private", BValue::Int(1)));
        }
        info.sort_by(|a, b| a.0.cmp(b.0));
        encode(&BValue::Dict(vec![
            (
                b"announce",
                BValue::Bytes(b"https://private.invalid/secret/announce"),
            ),
            (b"comment", BValue::Bytes(b"account-secret")),
            (b"info", BValue::Dict(info)),
            (
                b"url-list",
                BValue::Bytes(b"https://private.invalid/secret/file"),
            ),
        ]))
    }

    #[test]
    fn variants_are_distinct_and_do_not_export_private_discovery() {
        let raw = fixture(true, 16384);
        let original = parse_torrent(&raw).unwrap();
        let mut hashes = std::collections::HashSet::new();
        for variant in [
            PublicVariant::RemovePrivate,
            PublicVariant::PrivateZero,
            PublicVariant::RemovePrivateAndSource,
            PublicVariant::PrivateZeroWithoutSource,
        ] {
            let public = public_counterpart(&raw, Some(variant)).unwrap();
            validate_swarm_payload(&original, &public).unwrap();
            assert!(hashes.insert(public.v1_info_hash().unwrap()));
            assert_ne!(original.v1_info_hash(), public.v1_info_hash());
            let TorrentMeta::V1(public) = public else {
                panic!()
            };
            assert!(
                public.announce.is_none()
                    && public.announce_list.is_empty()
                    && public.webseeds.is_empty()
            );
            assert!(public.comment.is_none());
            assert!(!String::from_utf8_lossy(&public.raw).contains("secret"));
        }
        assert!(parse_torrent(&raw).unwrap().is_private());
    }

    #[test]
    fn supplied_public_identity_is_preserved_but_discovery_is_stripped() {
        let raw = fixture(false, 16384);
        let public = public_counterpart(&raw, None).unwrap();
        assert_eq!(
            parse_torrent(&raw).unwrap().v1_info_hash(),
            public.v1_info_hash()
        );
        assert!(public_counterpart(&fixture(true, 16384), None).is_err());
        assert!(public_counterpart(&raw, Some(PublicVariant::RemovePrivate)).is_err());
    }

    #[test]
    fn mismatched_piece_size_and_hashes_are_rejected() {
        let original = parse_torrent(&fixture(true, 16384)).unwrap();
        let mut public = public_counterpart(&fixture(false, 32768), None).unwrap();
        assert!(validate_swarm_payload(&original, &public)
            .unwrap_err()
            .contains("sizes differ"));
        if let TorrentMeta::V1(ref mut meta) = public {
            meta.piece_length = 16384;
            meta.pieces[0][0] ^= 1;
        }
        assert!(validate_swarm_payload(&original, &public)
            .unwrap_err()
            .contains("hashes differ"));
    }
    fn v2_fixture(hybrid: bool, private: bool, piece_length: i64) -> Vec<u8> {
        let mut fields = vec![
            (
                b"file tree".as_slice(),
                BValue::Dict(vec![(
                    b"file",
                    BValue::Dict(vec![(
                        b"",
                        BValue::Dict(vec![
                            (b"length", BValue::Int(4)),
                            (b"pieces root", BValue::Bytes(&[1; 32])),
                        ]),
                    )]),
                )]),
            ),
            (b"meta version", BValue::Int(2)),
            (b"name", BValue::Bytes(b"file")),
            (b"piece length", BValue::Int(piece_length)),
        ];
        if hybrid {
            fields.push((b"length", BValue::Int(4)));
            fields.push((b"pieces", BValue::Bytes(&[2; 20])));
        }
        if private {
            fields.push((b"private", BValue::Int(1)));
        }
        fields.sort_by(|a, b| a.0.cmp(b.0));
        encode(&BValue::Dict(vec![(b"info", BValue::Dict(fields))]))
    }

    #[test]
    fn v2_and_hybrid_round_trip_requires_authenticated_roots_and_exact_layout() {
        for hybrid in [false, true] {
            let raw = v2_fixture(hybrid, true, 16384);
            let original = parse_torrent(&raw).unwrap();
            let mut public = public_counterpart(&raw, Some(PublicVariant::RemovePrivate)).unwrap();
            validate_swarm_payload(&original, &public).unwrap();
            assert_ne!(original.v2_info_hash(), public.v2_info_hash());
            match &mut public {
                TorrentMeta::V2(m) | TorrentMeta::Hybrid(_, m) => {
                    m.files[0].pieces_root = Some([3; 32])
                }
                _ => unreachable!(),
            }
            assert!(validate_swarm_payload(&original, &public).is_err());
        }
        let original = parse_torrent(&v2_fixture(false, true, 16384)).unwrap();
        let public = parse_torrent(&v2_fixture(false, false, 32768)).unwrap();
        validate_swarm_payload(&original, &public).unwrap(); // file root is independent of piece size
        let hybrid = parse_torrent(&v2_fixture(true, false, 16384)).unwrap();
        assert!(validate_swarm_payload(&original, &hybrid)
            .unwrap_err()
            .contains("versions differ"));
    }

    #[test]
    fn layout_mismatch_and_allocation_denial_fail_closed() {
        let raw = fixture(true, 16384);
        let original = parse_torrent(&raw).unwrap();
        let mut public = public_counterpart(&raw, Some(PublicVariant::RemovePrivate)).unwrap();
        if let TorrentMeta::V1(m) = &mut public {
            m.files[0].path = rt_path::SafeRelPath::from_name("different", false).unwrap();
        }
        assert!(validate_swarm_payload(&original, &public)
            .unwrap_err()
            .contains("layout"));
        assert!(public_counterpart_with_allocation_reservation(
            &raw,
            Some(PublicVariant::RemovePrivate),
            &mut |_| false
        )
        .is_err());
        assert!(
            public_counterpart(&vec![0; MAX_SWARM_METAINFO_BYTES + 1], None)
                .unwrap_err()
                .contains("8 MiB")
        );
    }
}
