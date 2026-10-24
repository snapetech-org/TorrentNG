use std::{collections::VecDeque, path::Path};

use futures::future;
use rt_hash::{MerkleAccumulator, V2_BLOCK_SIZE};
use rt_path::SafeRelPath;
use sha1::{Digest as Sha1Digest, Sha1};
use tracing::instrument;

use rt_piece_map::{FileRegion, PieceMap};

use crate::{
    error::StorageError,
    io_class::IoClass,
    scheduler::{scheduled_read_owned, DataExtent, MountScheduler},
};

const VERIFY_READ_CHUNK_BYTES: u64 = 1024 * 1024;

/// Result of verifying a single piece.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyResult {
    /// SHA-1 matches the expected hash.
    Valid,
    /// SHA-1 does not match.
    Invalid,
    /// One or more files could not be read (missing, permission denied, etc.)
    Missing { file_index: u32, reason: String },
}

#[derive(Debug, Clone)]
pub struct V2FileHash {
    pub file_index: u32,
    pub path: SafeRelPath,
    pub length: u64,
    pub pieces_root: Option<[u8; 32]>,
}

/// The authenticated v2 view associated with one hybrid v1 piece. Hybrid
/// metadata validation guarantees that the file range begins on the same
/// piece boundary in both address spaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HybridV2PieceExpectation {
    pub file_index: u32,
    pub file_offset: u64,
    pub content_len: u64,
    pub expected_root: [u8; 32],
    /// `Some` for a piece-layer entry, whose final partial file piece is
    /// padded with zero-hash leaves to the full v2 piece size. `None` for a
    /// file no larger than one piece, whose file root pads only to the next
    /// power-of-two leaf count.
    pub piece_leaf_count: Option<usize>,
}

/// Verifies piece data against expected SHA-1 hashes by reading directly
/// from disk. Never uses mmap.
pub struct PieceVerifier<'a> {
    storage_root: &'a Path,
    scheduler: &'a MountScheduler,
    piece_map: &'a PieceMap,
    expected_hashes: &'a [[u8; 20]],
}

impl<'a> PieceVerifier<'a> {
    pub fn new(
        storage_root: &'a Path,
        scheduler: &'a MountScheduler,
        piece_map: &'a PieceMap,
        expected_hashes: &'a [[u8; 20]],
    ) -> Self {
        PieceVerifier {
            storage_root,
            scheduler,
            piece_map,
            expected_hashes,
        }
    }

    /// Verify a single piece. Reads all file regions that compose the piece.
    ///
    /// Chunk reads and chunk hashing are pipelined (double-buffered): while
    /// chunk N is being hashed on the dedicated hash pool, chunk N+1 is
    /// already being read from disk, so the wall-clock cost of a piece is
    /// closer to `max(read_time, hash_time)` than `sum(read_time, hash_time)`.
    /// The very first read and the very last hash are not overlapped with
    /// anything (pipeline prologue/epilogue), which is the normal shape of
    /// software pipelining.
    pub async fn verify_piece(&self, piece: u32) -> VerifyResult {
        self.verify_piece_inner(piece, None).await
    }

    /// Verify a hybrid piece's v1 hash and authenticated v2 file/piece root
    /// while each payload range is read only once.
    pub async fn verify_hybrid_piece(
        &self,
        piece: u32,
        expectation: &HybridV2PieceExpectation,
    ) -> VerifyResult {
        self.verify_piece_inner(piece, Some(expectation)).await
    }

    #[instrument(skip(self, v2), fields(piece))]
    async fn verify_piece_inner(
        &self,
        piece: u32,
        v2: Option<&HybridV2PieceExpectation>,
    ) -> VerifyResult {
        let _frames = crate::frame::global_frame_pool().verification_scope();
        let regions = match self.piece_map.piece_to_file_regions(piece) {
            Ok(r) => r,
            Err(e) => {
                return VerifyResult::Missing {
                    file_index: 0,
                    reason: e.to_string(),
                }
            }
        };

        let expected = match self.expected_hashes.get(piece as usize) {
            Some(h) => h,
            None => {
                return VerifyResult::Missing {
                    file_index: 0,
                    reason: format!("no hash for piece {piece}"),
                }
            }
        };

        if let Some(expectation) = v2 {
            let mut payload_regions = regions.iter().filter(|region| !region.pad);
            let matches_geometry = payload_regions.next().is_some_and(|region| {
                region.file_index == expectation.file_index
                    && region.file_offset == expectation.file_offset
                    && region.length == expectation.content_len
                    && payload_regions.next().is_none()
            });
            let valid_padding = expectation.piece_leaf_count.is_none_or(|leaf_count| {
                leaf_count.is_power_of_two()
                    && expectation.content_len.div_ceil(V2_BLOCK_SIZE as u64) <= leaf_count as u64
            });
            if !matches_geometry
                || expectation.content_len == 0
                || !expectation.file_offset.is_multiple_of(V2_BLOCK_SIZE as u64)
                || !valid_padding
            {
                return VerifyResult::Missing {
                    file_index: expectation.file_index,
                    reason: "hybrid v2 file range does not match the v1 piece geometry".to_owned(),
                };
            }
        }

        // Flatten the piece's file regions into a fixed read-chunk plan up
        // front (region index, file offset, length) so the pipeline below
        // can look one chunk ahead, including across a file-region boundary.
        let mut chunks: Vec<(usize, u64, u64)> = Vec::new();
        for (region_index, region) in regions.iter().enumerate() {
            let mut region_offset = 0u64;
            while region_offset < region.length {
                let len = (region.length - region_offset).min(VERIFY_READ_CHUNK_BYTES);
                let file_offset = match region.file_offset.checked_add(region_offset) {
                    Some(offset) => offset,
                    None => {
                        return VerifyResult::Missing {
                            file_index: region.file_index,
                            reason: "piece region offset overflow".to_owned(),
                        };
                    }
                };
                chunks.push((region_index, file_offset, len));
                region_offset += len;
            }
        }

        let mut hasher = Sha1::new();
        let mut v2_accumulator = v2.map(|_| MerkleAccumulator::new());
        let mut v2_hashed_bytes = 0u64;

        // Prologue: start the first chunk's read. There is nothing to
        // overlap it with yet.
        let mut pending = match chunks.first() {
            Some(&(region_index, file_offset, len)) => {
                let region = &regions[region_index];
                match self.read_region_chunk(region, file_offset, len).await {
                    Ok(data) => Some(data),
                    Err(e) => {
                        self.log_chunk_read_failure(piece, region.file_index, &e);
                        return VerifyResult::Missing {
                            file_index: region.file_index,
                            reason: e.to_string(),
                        };
                    }
                }
            }
            None => None,
        };

        for (i, &(region_index, chunk_offset, chunk_len)) in chunks.iter().enumerate() {
            let data = pending
                .take()
                .expect("verify pipeline lost its buffered chunk");
            match chunks.get(i + 1) {
                Some(&(next_region_index, next_offset, next_len)) => {
                    // Steady state: hash the chunk in hand while prefetching
                    // the next one. For hybrid content, submit its v2 leaf
                    // batch beside the v1 update so both views use this read.
                    let next_region = &regions[next_region_index];
                    let hash_fut = recheck_hash_sha1_update(self.scheduler, &hasher, &data);
                    let v2_region = v2.is_some_and(|expectation| {
                        !regions[region_index].pad
                            && regions[region_index].file_index == expectation.file_index
                            && byte_range_within(
                                chunk_offset,
                                chunk_len,
                                expectation.file_offset,
                                expectation.content_len,
                            )
                    });
                    let v2_hash_fut = async {
                        if v2_region {
                            recheck_hash_v2_leaves(self.scheduler, &data, V2_BLOCK_SIZE).await
                        } else {
                            Ok(Vec::new())
                        }
                    };
                    let read_fut = self.read_region_chunk(next_region, next_offset, next_len);
                    let (hash_result, v2_result, read_result) =
                        future::join3(hash_fut, v2_hash_fut, read_fut).await;
                    hasher = match hash_result {
                        Ok(updated) => updated,
                        Err(e) => {
                            return VerifyResult::Missing {
                                file_index: regions[region_index].file_index,
                                reason: e.to_string(),
                            };
                        }
                    };
                    if v2_region {
                        let leaves = match v2_result {
                            Ok(leaves) => leaves,
                            Err(error) => {
                                return VerifyResult::Missing {
                                    file_index: regions[region_index].file_index,
                                    reason: error.to_string(),
                                };
                            }
                        };
                        if let Some(accumulator) = v2_accumulator.as_mut() {
                            for leaf in leaves {
                                accumulator.push(leaf);
                            }
                        }
                        v2_hashed_bytes = v2_hashed_bytes.saturating_add(data.len() as u64);
                    }
                    pending = match read_result {
                        Ok(data) => Some(data),
                        Err(e) => {
                            self.log_chunk_read_failure(piece, next_region.file_index, &e);
                            return VerifyResult::Missing {
                                file_index: next_region.file_index,
                                reason: e.to_string(),
                            };
                        }
                    };
                }
                None => {
                    // Epilogue: the last chunk has nothing left to prefetch.
                    let v2_region = v2.is_some_and(|expectation| {
                        !regions[region_index].pad
                            && regions[region_index].file_index == expectation.file_index
                            && byte_range_within(
                                chunk_offset,
                                chunk_len,
                                expectation.file_offset,
                                expectation.content_len,
                            )
                    });
                    let hash_fut = recheck_hash_sha1_update(self.scheduler, &hasher, &data);
                    let v2_hash_fut = async {
                        if v2_region {
                            recheck_hash_v2_leaves(self.scheduler, &data, V2_BLOCK_SIZE).await
                        } else {
                            Ok(Vec::new())
                        }
                    };
                    let (hash_result, v2_result) = future::join(hash_fut, v2_hash_fut).await;
                    match hash_result {
                        Ok(updated) => hasher = updated,
                        Err(e) => {
                            return VerifyResult::Missing {
                                file_index: regions[region_index].file_index,
                                reason: e.to_string(),
                            };
                        }
                    }
                    if v2_region {
                        let leaves = match v2_result {
                            Ok(leaves) => leaves,
                            Err(error) => {
                                return VerifyResult::Missing {
                                    file_index: regions[region_index].file_index,
                                    reason: error.to_string(),
                                };
                            }
                        };
                        if let Some(accumulator) = v2_accumulator.as_mut() {
                            for leaf in leaves {
                                accumulator.push(leaf);
                            }
                        }
                        v2_hashed_bytes = v2_hashed_bytes.saturating_add(data.len() as u64);
                    }
                }
            }
        }

        let actual: [u8; 20] = hasher.finalize().into();
        let v1_matches = &actual == expected;
        let v2_matches = match v2 {
            Some(expectation) if v2_hashed_bytes == expectation.content_len => {
                let root = match (v2_accumulator, expectation.piece_leaf_count) {
                    (Some(accumulator), Some(leaf_count)) => {
                        accumulator.finish_padded_to(leaf_count)
                    }
                    (Some(accumulator), None) => Some(accumulator.finish()),
                    (None, _) => None,
                };
                root == Some(expectation.expected_root)
            }
            Some(expectation) => {
                return VerifyResult::Missing {
                    file_index: expectation.file_index,
                    reason: "hybrid v2 hashing did not cover the authenticated file range"
                        .to_owned(),
                };
            }
            None => true,
        };
        if v1_matches && v2_matches {
            tracing::debug!(
                component = "storage",
                operation = "verify_piece",
                piece,
                result = "valid",
                "piece valid"
            );
            VerifyResult::Valid
        } else {
            tracing::warn!(
                component = "storage",
                operation = "verify_piece",
                piece,
                result = "invalid",
                v1_matches,
                v2_matches,
                "piece hash mismatch"
            );
            VerifyResult::Invalid
        }
    }

    /// Verify all pieces, returning a bitset of valid pieces.
    pub async fn verify_all(&self) -> Vec<VerifyResult> {
        let _frames = crate::frame::global_frame_pool().verification_scope();
        let mut results = Vec::with_capacity(self.piece_map.piece_count as usize);
        for piece in 0..self.piece_map.piece_count {
            results.push(self.verify_piece(piece).await);
        }
        results
    }

    /// Verify a range of pieces (for resumable recheck).
    pub async fn verify_range(
        &self,
        start: u32,
        end: u32,
    ) -> Result<Vec<(u32, VerifyResult)>, StorageError> {
        let _frames = crate::frame::global_frame_pool().verification_scope();
        let start = start.min(self.piece_map.piece_count);
        let end = end.min(self.piece_map.piece_count);
        if start >= end {
            return Ok(Vec::new());
        }
        let mut results = Vec::with_capacity((end - start) as usize);
        for piece in start..end {
            let result = self.verify_piece(piece).await;
            results.push((piece, result));
        }
        Ok(results)
    }

    async fn read_region_chunk(
        &self,
        region: &FileRegion,
        file_offset: u64,
        len: u64,
    ) -> Result<bytes::Bytes, StorageError> {
        if region.pad {
            // BEP 47 padding participates in v1 piece hashes as zero bytes,
            // but compliant clients need not create a corresponding file.
            return Ok(bytes::Bytes::from(vec![0; len as usize]));
        }
        let file_path = region.path.resolve(self.storage_root);
        read_sparse_range(self.scheduler, &file_path, file_offset, len).await
    }

    fn log_chunk_read_failure(&self, piece: u32, file_index: u32, error: &StorageError) {
        tracing::warn!(
            component = "storage",
            operation = "verify_piece",
            piece,
            file_index,
            result = "error",
            error = %error,
            "file read failed during verify"
        );
    }
}

/// Verifies BEP 52 file roots by reading each file in 16 KiB leaves and
/// comparing the computed merkle root with the metainfo `pieces root`.
pub struct V2FileVerifier<'a> {
    storage_root: &'a Path,
    scheduler: &'a MountScheduler,
    files: &'a [V2FileHash],
}

impl<'a> V2FileVerifier<'a> {
    pub const LEAF_SIZE: usize = 16 * 1024;
    /// Leaves read and hashed per round trip (1 MiB). The sparse-extent probe,
    /// read and hash job are each issued once per chunk instead of per leaf.
    pub const CHUNK_LEAVES: usize = 64;

    pub fn new(
        storage_root: &'a Path,
        scheduler: &'a MountScheduler,
        files: &'a [V2FileHash],
    ) -> Self {
        Self {
            storage_root,
            scheduler,
            files,
        }
    }

    pub async fn verify_all(&self) -> Vec<(u32, VerifyResult)> {
        let _frames = crate::frame::global_frame_pool().verification_scope();
        let mut out = Vec::with_capacity(self.files.len());
        for file in self.files {
            out.push((file.file_index, self.verify_file(file).await));
        }
        out
    }

    #[instrument(skip(self, file), fields(file_index = file.file_index))]
    pub async fn verify_file(&self, file: &V2FileHash) -> VerifyResult {
        let _frames = crate::frame::global_frame_pool().verification_scope();
        let actual = match self.file_root(file).await {
            Ok(root) => root,
            Err(e) => {
                tracing::warn!(
                    component = "storage",
                    operation = "verify_v2_file",
                    file_index = file.file_index,
                    result = "error",
                    error = %e,
                    "file read failed during v2 verify"
                );
                return VerifyResult::Missing {
                    file_index: file.file_index,
                    reason: e.to_string(),
                };
            }
        };
        if file.pieces_root.is_none() && file.length == 0 {
            // BEP 52 omits `pieces root` for empty files. `file_root` still
            // read the path, so a missing file is reported as Missing above.
            VerifyResult::Valid
        } else if Some(actual) == file.pieces_root {
            VerifyResult::Valid
        } else {
            tracing::warn!(
                component = "storage",
                operation = "verify_v2_file",
                file_index = file.file_index,
                result = "invalid",
                "v2 file root mismatch"
            );
            VerifyResult::Invalid
        }
    }

    async fn file_root(&self, file: &V2FileHash) -> Result<[u8; 32], StorageError> {
        let path = file.path.resolve(self.storage_root);
        if file.length == 0 {
            // Empty BEP 52 files omit `pieces root`, but the payload path must
            // still exist. A zero-length scheduled read validates that the
            // path can be opened without allocating a data buffer.
            let _ = recheck_read_owned(self.scheduler, &path, 0, 0).await?;
        }
        let mut accumulator = MerkleAccumulator::new();
        let mut offset = 0u64;
        let chunk_bytes = (Self::LEAF_SIZE * Self::CHUNK_LEAVES) as u64;
        let mut sparse_cursor = SparseExtentCursor::new(self.scheduler, &path, file.length);
        while offset < file.length {
            let len = (file.length - offset).min(chunk_bytes) as usize;
            let data = sparse_cursor.read_next(len).await?;
            for leaf in recheck_hash_v2_leaves(self.scheduler, &data, Self::LEAF_SIZE).await? {
                accumulator.push(leaf);
            }
            offset += len as u64;
        }
        if file.length > 0 {
            // Sparse holes need no data read, but a final one-byte check keeps
            // truncation during the streamed scan from being mistaken for
            // zero-filled content.
            let _ = recheck_read_owned(self.scheduler, &path, file.length - 1, 1).await?;
        }
        if file.length == 0 {
            let empty = bytes::Bytes::new();
            accumulator.push(recheck_hash_v2_leaf(self.scheduler, &empty).await?);
        }
        recheck_finish_v2_merkle_root(self.scheduler, accumulator).await
    }
}

const SPARSE_EXTENTS_PER_PAGE: usize = 64;

/// Sequential sparse-file reader whose bounded extent pages survive across
/// 1 MiB verification chunks. A data extent that spans several chunks is
/// discovered once and consumed as the Merkle stream advances.
struct SparseExtentCursor<'a> {
    scheduler: &'a MountScheduler,
    path: &'a Path,
    file_length: u64,
    read_offset: u64,
    scan_offset: u64,
    scan_complete: bool,
    extents: VecDeque<DataExtent>,
    #[cfg(test)]
    extent_page_count: usize,
}

impl<'a> SparseExtentCursor<'a> {
    fn new(scheduler: &'a MountScheduler, path: &'a Path, file_length: u64) -> Self {
        Self {
            scheduler,
            path,
            file_length,
            read_offset: 0,
            scan_offset: 0,
            scan_complete: false,
            extents: VecDeque::new(),
            #[cfg(test)]
            extent_page_count: 0,
        }
    }

    async fn read_next(&mut self, len: usize) -> Result<bytes::Bytes, StorageError> {
        if len == 0 {
            return Ok(bytes::Bytes::new());
        }
        let len_u64 = u64::try_from(len).map_err(|_| {
            StorageError::io(
                self.path.display().to_string(),
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "read length overflows u64",
                ),
            )
        })?;
        let end = self.read_offset.checked_add(len_u64).ok_or_else(|| {
            StorageError::io(
                self.path.display().to_string(),
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "read range overflows u64"),
            )
        })?;
        if end > self.file_length {
            return Err(StorageError::ShortIo {
                path: self.path.display().to_string(),
                expected: len,
                actual: self.file_length.saturating_sub(self.read_offset) as usize,
            });
        }

        let mut output = vec![0; len];
        let mut cursor = self.read_offset;
        while cursor < end {
            while self
                .extents
                .front()
                .is_some_and(|extent| extent.offset.saturating_add(extent.len) <= cursor)
            {
                self.extents.pop_front();
            }

            if let Some(extent) = self.extents.front().copied() {
                if extent.offset > cursor {
                    // `output` is zero-initialized; this interval is a hole.
                    cursor = extent.offset.min(end);
                    continue;
                }
                let extent_end = extent.offset.saturating_add(extent.len);
                let read_end = extent_end.min(end);
                let read_len = usize::try_from(read_end.saturating_sub(cursor)).map_err(|_| {
                    StorageError::io(
                        self.path.display().to_string(),
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "extent read length overflows usize",
                        ),
                    )
                })?;
                if read_len == 0 {
                    self.extents.pop_front();
                    continue;
                }
                let data = recheck_read_owned(self.scheduler, self.path, cursor, read_len).await?;
                let output_start = usize::try_from(cursor - self.read_offset).map_err(|_| {
                    StorageError::io(
                        self.path.display().to_string(),
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "extent output offset overflows usize",
                        ),
                    )
                })?;
                let output_end = output_start.checked_add(read_len).ok_or_else(|| {
                    StorageError::io(
                        self.path.display().to_string(),
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "extent output range overflows usize",
                        ),
                    )
                })?;
                if output_end > output.len() {
                    return Err(StorageError::io(
                        self.path.display().to_string(),
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "extent exceeds the requested read chunk",
                        ),
                    ));
                }
                if data.len() != output_end.saturating_sub(output_start) {
                    return Err(StorageError::ShortIo {
                        path: self.path.display().to_string(),
                        expected: output_end.saturating_sub(output_start),
                        actual: data.len(),
                    });
                }
                output[output_start..output_end].copy_from_slice(&data);
                cursor = read_end;
                if read_end >= extent_end {
                    self.extents.pop_front();
                } else if let Some(front) = self.extents.front_mut() {
                    front.len = extent_end - read_end;
                    front.offset = read_end;
                }
                continue;
            }

            if cursor < self.scan_offset {
                // The current page has proved this interval contains no data.
                cursor = self.scan_offset.min(end);
                continue;
            }
            if self.scan_complete {
                // The remainder of a fully scanned file is a sparse hole.
                cursor = end;
                continue;
            }

            let remaining = self.file_length.saturating_sub(self.scan_offset);
            if remaining == 0 {
                self.scan_complete = true;
                cursor = end;
                continue;
            }
            let page = self
                .scheduler
                .data_extents_page(
                    self.path,
                    self.scan_offset,
                    remaining,
                    SPARSE_EXTENTS_PER_PAGE,
                )
                .await?;
            if page.scanned_to <= self.scan_offset && !page.complete {
                return Err(StorageError::io(
                    self.path.display().to_string(),
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "sparse extent scan did not advance",
                    ),
                ));
            }
            self.scan_offset = page.scanned_to;
            self.scan_complete = page.complete;
            self.extents.extend(page.extents);
            #[cfg(test)]
            {
                self.extent_page_count = self.extent_page_count.saturating_add(1);
            }
        }

        self.read_offset = end;
        Ok(bytes::Bytes::from(output))
    }
}

async fn read_sparse_range(
    scheduler: &MountScheduler,
    path: &Path,
    offset: u64,
    len: u64,
) -> Result<bytes::Bytes, StorageError> {
    let extents = recheck_data_extents(scheduler, path, offset, len).await?;
    if extents.is_empty() {
        return recheck_read_owned(scheduler, path, offset, len as usize).await;
    }

    let range_end = offset.saturating_add(len);
    // A single dense extent can use the owned read directly. Avoid assembling
    // a second full-range Vec and copying the read into it.
    if extents.len() == 1 {
        let extent = extents[0];
        if extent.offset <= offset
            && extent
                .offset
                .checked_add(extent.len)
                .is_some_and(|extent_end| extent_end >= range_end)
        {
            return recheck_read_owned(scheduler, path, offset, len as usize).await;
        }
    }

    let mut out = Vec::with_capacity(len as usize);
    let mut cursor = offset;
    for extent in extents {
        if extent.offset > cursor {
            out.resize(out.len() + (extent.offset - cursor) as usize, 0);
        }
        let extent_end = extent.offset.saturating_add(extent.len).min(range_end);
        if extent_end > extent.offset {
            let data = recheck_read_owned(
                scheduler,
                path,
                extent.offset,
                (extent_end - extent.offset) as usize,
            )
            .await?;
            out.extend_from_slice(&data);
            cursor = extent_end;
        }
    }
    if cursor < range_end {
        out.resize(out.len() + (range_end - cursor) as usize, 0);
    }
    Ok(bytes::Bytes::from(out))
}

async fn recheck_read_owned(
    scheduler: &MountScheduler,
    path: &Path,
    offset: u64,
    len: usize,
) -> Result<bytes::Bytes, StorageError> {
    let mut attempts = 0;
    loop {
        match scheduled_read_owned(scheduler, IoClass::Recheck, path, offset, len).await {
            Ok(read) => return Ok(read.into_bytes()),
            Err(error @ StorageError::QueueFull { .. }) => {
                if !retry_recheck_queue_full(&mut attempts).await {
                    return Err(error);
                }
            }
            Err(err) => return Err(err),
        }
    }
}

fn byte_range_within(start: u64, len: u64, outer_start: u64, outer_len: u64) -> bool {
    let Some(end) = start.checked_add(len) else {
        return false;
    };
    let Some(outer_end) = outer_start.checked_add(outer_len) else {
        return false;
    };
    start >= outer_start && end <= outer_end
}

async fn recheck_hash_sha1_update(
    scheduler: &MountScheduler,
    hasher: &Sha1,
    data: &bytes::Bytes,
) -> Result<Sha1, StorageError> {
    let mut attempts = 0;
    loop {
        match scheduler.hash_sha1_update(hasher, data.clone()).await {
            Ok(updated) => return Ok(updated),
            Err(error @ StorageError::QueueFull { .. }) => {
                if !retry_recheck_queue_full(&mut attempts).await {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

async fn recheck_hash_v2_leaves(
    scheduler: &MountScheduler,
    data: &bytes::Bytes,
    leaf_size: usize,
) -> Result<Vec<[u8; 32]>, StorageError> {
    let mut attempts = 0;
    loop {
        match scheduler.hash_v2_leaves(data.clone(), leaf_size).await {
            Ok(hashes) => return Ok(hashes),
            Err(error @ StorageError::QueueFull { .. }) => {
                if !retry_recheck_queue_full(&mut attempts).await {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

async fn recheck_hash_v2_leaf(
    scheduler: &MountScheduler,
    data: &bytes::Bytes,
) -> Result<[u8; 32], StorageError> {
    let mut attempts = 0;
    loop {
        match scheduler.hash_v2_leaf(data.clone()).await {
            Ok(hash) => return Ok(hash),
            Err(error @ StorageError::QueueFull { .. }) => {
                if !retry_recheck_queue_full(&mut attempts).await {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

async fn recheck_finish_v2_merkle_root(
    scheduler: &MountScheduler,
    accumulator: MerkleAccumulator,
) -> Result<[u8; 32], StorageError> {
    let mut attempts = 0;
    loop {
        match scheduler.finish_v2_merkle_root(accumulator.clone()).await {
            Ok(root) => return Ok(root),
            Err(error @ StorageError::QueueFull { .. }) => {
                if !retry_recheck_queue_full(&mut attempts).await {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

const MAX_RECHECK_QUEUE_RETRIES: usize = 10_000;

async fn retry_recheck_queue_full(attempts: &mut usize) -> bool {
    if *attempts >= MAX_RECHECK_QUEUE_RETRIES {
        return false;
    }
    *attempts += 1;
    if (*attempts).is_multiple_of(16) {
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    } else {
        tokio::task::yield_now().await;
    }
    true
}

async fn recheck_data_extents(
    scheduler: &MountScheduler,
    path: &Path,
    offset: u64,
    len: u64,
) -> Result<Vec<crate::scheduler::DataExtent>, StorageError> {
    let mut attempts = 0;
    loop {
        match scheduler.data_extents(path, offset, len).await {
            Ok(extents) => return Ok(extents),
            Err(error @ StorageError::QueueFull { .. }) => {
                if !retry_recheck_queue_full(&mut attempts).await {
                    return Err(error);
                }
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scheduler::scheduled_write;
    use rt_hash::{merkle_root, piece_layer_hash, BlockHash, V2_BLOCK_SIZE};
    use rt_path::SafeRelPath;
    use rt_piece_map::{FileSpan, PieceMap};
    use sha1::{Digest, Sha1};

    use crate::scheduler::{MountScheduler, SchedulerConfig};
    use rt_path::{StorageProfile, StorageRootId};

    fn ssd_scheduler() -> MountScheduler {
        MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Ssd,
                ..Default::default()
            },
        )
    }

    fn piece_hash(data: &[u8]) -> [u8; 20] {
        Sha1::digest(data).into()
    }

    fn v2_file_root(content: &[u8]) -> [u8; 32] {
        let mut leaves = Vec::new();
        for chunk in content.chunks(V2FileVerifier::LEAF_SIZE) {
            leaves.push(BlockHash::of(chunk).0);
        }
        if leaves.is_empty() {
            leaves.push(BlockHash::of(&[]).0);
        }
        merkle_root(&leaves)
    }

    #[tokio::test]
    async fn verify_valid_piece() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"hello world - this is exactly one piece of data!";
        let fname = "data.bin";
        std::fs::write(dir.path().join(fname), content).unwrap();

        let piece_length = content.len() as u64;
        let hash = piece_hash(content);

        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name(fname, false).unwrap(),
            content_offset: 0,
            length: piece_length,
        }];
        let pm = PieceMap::new(piece_length, files).unwrap();
        let sched = ssd_scheduler();
        let hashes = [hash];
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        assert_eq!(verifier.verify_piece(0).await, VerifyResult::Valid);
    }

    #[tokio::test]
    async fn verify_piece_treats_missing_bep47_padding_as_zeroes() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("payload.bin"), b"abc").unwrap();
        let files = vec![
            FileSpan {
                file_index: 0,
                path: SafeRelPath::from_name("payload.bin", false).unwrap(),
                content_offset: 0,
                length: 3,
            },
            FileSpan {
                file_index: 1,
                path: SafeRelPath::from_components(&[".pad", "13"], false).unwrap(),
                content_offset: 3,
                length: 13,
            },
        ];
        let map = PieceMap::new_with_padding(16, files, [1]).unwrap();
        let mut expected = b"abc".to_vec();
        expected.resize(16, 0);
        let hash = piece_hash(&expected);
        let hashes = [hash];
        let scheduler = ssd_scheduler();
        let verifier = PieceVerifier::new(dir.path(), &scheduler, &map, &hashes);

        assert_eq!(verifier.verify_piece(0).await, VerifyResult::Valid);
        assert!(!dir.path().join(".pad/13").exists());
    }

    #[tokio::test]
    async fn verify_hybrid_short_final_piece_excludes_bep47_padding_from_v2_root() {
        let dir = tempfile::tempdir().unwrap();
        let piece_length = V2_BLOCK_SIZE * 2;
        let content: Vec<u8> = (0..piece_length + 17)
            .map(|index| (index % 239) as u8)
            .collect();
        std::fs::write(dir.path().join("payload.bin"), &content).unwrap();
        let padding_length = piece_length - 17;
        let mut final_v1_piece = content[piece_length..].to_vec();
        final_v1_piece.resize(piece_length, 0);
        let hashes = [
            piece_hash(&content[..piece_length]),
            piece_hash(&final_v1_piece),
        ];
        let files = vec![
            FileSpan {
                file_index: 0,
                path: SafeRelPath::from_name("payload.bin", false).unwrap(),
                content_offset: 0,
                length: content.len() as u64,
            },
            FileSpan {
                file_index: 1,
                path: SafeRelPath::from_components(&[".pad", "32751"], false).unwrap(),
                content_offset: content.len() as u64,
                length: padding_length as u64,
            },
        ];
        let piece_map = PieceMap::new_with_padding(piece_length as u64, files, [1]).unwrap();
        let scheduler = ssd_scheduler();
        let verifier = PieceVerifier::new(dir.path(), &scheduler, &piece_map, &hashes);
        let valid_expectation = HybridV2PieceExpectation {
            file_index: 0,
            file_offset: piece_length as u64,
            content_len: 17,
            expected_root: piece_layer_hash(&content[piece_length..], piece_length).unwrap(),
            piece_leaf_count: Some(2),
        };

        assert_eq!(
            verifier.verify_hybrid_piece(1, &valid_expectation).await,
            VerifyResult::Valid
        );
        let mut wrong_v2_root = valid_expectation;
        wrong_v2_root.expected_root[0] ^= 1;
        assert_eq!(
            verifier.verify_hybrid_piece(1, &wrong_v2_root).await,
            VerifyResult::Invalid,
            "a valid v1 hash must not hide a mismatch in the authenticated v2 view"
        );
        assert!(!dir.path().join(".pad/32751").exists());
    }

    #[tokio::test]
    async fn verify_hybrid_single_piece_file_uses_its_file_root() {
        let dir = tempfile::tempdir().unwrap();
        let piece_length = V2_BLOCK_SIZE * 2;
        let content = b"short v2 file";
        std::fs::write(dir.path().join("short.bin"), content).unwrap();
        let mut v1_piece = content.to_vec();
        v1_piece.resize(piece_length, 0);
        let hashes = [piece_hash(&v1_piece)];
        let files = vec![
            FileSpan {
                file_index: 0,
                path: SafeRelPath::from_name("short.bin", false).unwrap(),
                content_offset: 0,
                length: content.len() as u64,
            },
            FileSpan {
                file_index: 1,
                path: SafeRelPath::from_components(&[".pad", "32755"], false).unwrap(),
                content_offset: content.len() as u64,
                length: (piece_length - content.len()) as u64,
            },
        ];
        let piece_map = PieceMap::new_with_padding(piece_length as u64, files, [1]).unwrap();
        let scheduler = ssd_scheduler();
        let verifier = PieceVerifier::new(dir.path(), &scheduler, &piece_map, &hashes);
        let expectation = HybridV2PieceExpectation {
            file_index: 0,
            file_offset: 0,
            content_len: content.len() as u64,
            expected_root: v2_file_root(content),
            piece_leaf_count: None,
        };

        assert_eq!(
            verifier.verify_hybrid_piece(0, &expectation).await,
            VerifyResult::Valid
        );
    }

    #[tokio::test]
    async fn read_sparse_range_returns_dense_file_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("dense-range.bin");
        let content: Vec<u8> = (0..1024 * 1024).map(|index| (index % 251) as u8).collect();
        std::fs::write(&path, &content).unwrap();
        let scheduler = ssd_scheduler();

        let read = read_sparse_range(&scheduler, &path, 0, content.len() as u64)
            .await
            .unwrap();

        assert_eq!(read.as_ref(), content.as_slice());
    }

    #[tokio::test]
    async fn read_sparse_range_reports_truncated_dense_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("truncated-dense-range.bin");
        std::fs::write(&path, [0x5au8; 64 * 1024]).unwrap();
        let scheduler = ssd_scheduler();

        let result = read_sparse_range(&scheduler, &path, 0, 128 * 1024).await;

        assert!(matches!(result, Err(StorageError::ShortIo { .. })));
    }

    #[tokio::test]
    async fn verify_sparse_piece_hashes_holes_as_zeroes() {
        let dir = tempfile::tempdir().unwrap();
        let fname = "sparse.bin";
        let path = dir.path().join(fname);
        let piece_len = 512 * 1024usize;
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(piece_len as u64).unwrap();
        drop(file);
        let data_offset = 128 * 1024usize;
        let data = vec![0x5au8; 4096];
        scheduled_write(
            &ssd_scheduler(),
            IoClass::PeerWrite,
            &path,
            data_offset as u64,
            bytes::Bytes::from(data.clone()),
            false,
        )
        .await
        .unwrap();

        let mut expected_piece = vec![0u8; piece_len];
        expected_piece[data_offset..data_offset + data.len()].copy_from_slice(&data);
        let hash = piece_hash(&expected_piece);
        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name(fname, false).unwrap(),
            content_offset: 0,
            length: piece_len as u64,
        }];
        let pm = PieceMap::new(piece_len as u64, files).unwrap();
        let sched = ssd_scheduler();
        let hashes = [hash];
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        assert_eq!(verifier.verify_piece(0).await, VerifyResult::Valid);
    }

    #[tokio::test]
    async fn verify_piece_streams_large_piece() {
        let dir = tempfile::tempdir().unwrap();
        let fname = "large-sparse.bin";
        let piece_len = VERIFY_READ_CHUNK_BYTES * 2 + 17;
        let file = std::fs::File::create(dir.path().join(fname)).unwrap();
        file.set_len(piece_len).unwrap();
        drop(file);

        let zero_chunk = [0u8; 4096];
        let mut expected_hasher = Sha1::new();
        let mut remaining = piece_len;
        while remaining > 0 {
            let len = remaining.min(zero_chunk.len() as u64) as usize;
            expected_hasher.update(&zero_chunk[..len]);
            remaining -= len as u64;
        }
        let hash: [u8; 20] = expected_hasher.finalize().into();
        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name(fname, false).unwrap(),
            content_offset: 0,
            length: piece_len,
        }];
        let pm = PieceMap::new(piece_len, files).unwrap();
        let sched = ssd_scheduler();
        let hashes = [hash];
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        assert_eq!(verifier.verify_piece(0).await, VerifyResult::Valid);
    }

    #[tokio::test]
    async fn verify_truncated_sparse_file_is_missing_not_zero_filled() {
        let dir = tempfile::tempdir().unwrap();
        let fname = "truncated.bin";
        let path = dir.path().join(fname);
        std::fs::write(&path, [0u8; 1024]).unwrap();
        let piece_len = 128 * 1024usize;
        let expected = piece_hash(&vec![0u8; piece_len]);
        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name(fname, false).unwrap(),
            content_offset: 0,
            length: piece_len as u64,
        }];
        let pm = PieceMap::new(piece_len as u64, files).unwrap();
        let sched = ssd_scheduler();
        let hashes = [expected];
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        assert!(matches!(
            verifier.verify_piece(0).await,
            VerifyResult::Missing { file_index: 0, .. }
        ));
    }

    #[tokio::test]
    async fn verify_invalid_piece() {
        let dir = tempfile::tempdir().unwrap();
        let content = b"corrupted data!!!";
        let fname = "data.bin";
        std::fs::write(dir.path().join(fname), content).unwrap();

        let wrong_hash = [0u8; 20]; // definitely wrong
        let piece_length = content.len() as u64;

        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name(fname, false).unwrap(),
            content_offset: 0,
            length: piece_length,
        }];
        let pm = PieceMap::new(piece_length, files).unwrap();
        let sched = ssd_scheduler();
        let hashes = [wrong_hash];
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        assert_eq!(verifier.verify_piece(0).await, VerifyResult::Invalid);
    }

    #[tokio::test]
    async fn verify_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        // don't create the file

        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name("missing.bin", false).unwrap(),
            content_offset: 0,
            length: 256,
        }];
        let pm = PieceMap::new(256, files).unwrap();
        let sched = ssd_scheduler();
        let hashes = [[0u8; 20]];
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        assert!(matches!(
            verifier.verify_piece(0).await,
            VerifyResult::Missing { .. }
        ));
    }

    #[tokio::test]
    async fn verify_all_reports_per_piece() {
        let dir = tempfile::tempdir().unwrap();
        // Two-piece file: piece 0 valid, piece 1 has wrong hash
        let piece_len = 64usize;
        let content: Vec<u8> = (0..piece_len * 2).map(|i| i as u8).collect();
        std::fs::write(dir.path().join("data.bin"), &content).unwrap();

        let hash0 = piece_hash(&content[..piece_len]);
        let hash1_wrong = [0u8; 20];

        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name("data.bin", false).unwrap(),
            content_offset: 0,
            length: content.len() as u64,
        }];
        let pm = PieceMap::new(piece_len as u64, files).unwrap();
        let sched = ssd_scheduler();
        let hashes = [hash0, hash1_wrong];
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        let results = verifier.verify_all().await;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0], VerifyResult::Valid);
        assert_eq!(results[1], VerifyResult::Invalid);
    }

    #[tokio::test]
    async fn verify_range_resumable() {
        let dir = tempfile::tempdir().unwrap();
        let piece_len = 32usize;
        let n_pieces = 4usize;
        let content: Vec<u8> = (0..(piece_len * n_pieces)).map(|i| i as u8).collect();
        std::fs::write(dir.path().join("data.bin"), &content).unwrap();

        let hashes: Vec<[u8; 20]> = (0..n_pieces)
            .map(|i| piece_hash(&content[i * piece_len..(i + 1) * piece_len]))
            .collect();

        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name("data.bin", false).unwrap(),
            content_offset: 0,
            length: content.len() as u64,
        }];
        let pm = PieceMap::new(piece_len as u64, files).unwrap();
        let sched = ssd_scheduler();
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        // Verify only pieces 1..3 (simulating resume from piece 1)
        let results = verifier.verify_range(1, 3).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, 1);
        assert_eq!(results[0].1, VerifyResult::Valid);
        assert_eq!(results[1].0, 2);
        assert_eq!(results[1].1, VerifyResult::Valid);
    }

    #[tokio::test]
    async fn verify_range_returns_empty_for_reversed_or_out_of_range_bounds() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![FileSpan {
            file_index: 0,
            path: SafeRelPath::from_name("data.bin", false).unwrap(),
            content_offset: 0,
            length: 32,
        }];
        let pm = PieceMap::new(32, files).unwrap();
        let sched = ssd_scheduler();
        let hashes = [[0u8; 20]];
        let verifier = PieceVerifier::new(dir.path(), &sched, &pm, &hashes);

        assert!(verifier.verify_range(1, 0).await.unwrap().is_empty());
        assert!(verifier
            .verify_range(u32::MAX, u32::MAX)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn v2_file_verify_accepts_matching_file_root() {
        let dir = tempfile::tempdir().unwrap();
        let content: Vec<u8> = (0..(V2FileVerifier::LEAF_SIZE + 17))
            .map(|idx| idx as u8)
            .collect();
        std::fs::write(dir.path().join("data.bin"), &content).unwrap();

        let file = V2FileHash {
            file_index: 3,
            path: SafeRelPath::from_name("data.bin", false).unwrap(),
            length: content.len() as u64,
            pieces_root: Some(v2_file_root(&content)),
        };
        let sched = ssd_scheduler();
        let verifier = V2FileVerifier::new(dir.path(), &sched, std::slice::from_ref(&file));

        assert_eq!(verifier.verify_file(&file).await, VerifyResult::Valid);
        assert_eq!(
            verifier.verify_all().await,
            vec![(file.file_index, VerifyResult::Valid)]
        );
    }

    #[tokio::test]
    async fn v2_file_verify_batches_leaf_hashing_per_chunk() {
        let dir = tempfile::tempdir().unwrap();
        // 130 leaves: two full 64-leaf chunks plus a 2-leaf tail.
        let leaves = V2FileVerifier::CHUNK_LEAVES * 2 + 2;
        let content: Vec<u8> = (0..leaves * V2FileVerifier::LEAF_SIZE)
            .map(|idx| (idx % 251) as u8)
            .collect();
        std::fs::write(dir.path().join("chunked.bin"), &content).unwrap();
        let file = V2FileHash {
            file_index: 0,
            path: SafeRelPath::from_name("chunked.bin", false).unwrap(),
            length: content.len() as u64,
            pieces_root: Some(v2_file_root(&content)),
        };
        let sched = ssd_scheduler();
        let before = sched.stats().hash_ops;
        let verifier = V2FileVerifier::new(dir.path(), &sched, std::slice::from_ref(&file));

        assert_eq!(verifier.verify_file(&file).await, VerifyResult::Valid);
        let jobs = sched.stats().hash_ops - before;
        // Three leaf-batch jobs plus the offloaded root fold, not 130
        // per-leaf jobs or a root fold on the async executor.
        assert_eq!(jobs, 4, "expected three leaf batches and one root job");
    }

    #[tokio::test]
    async fn v2_sparse_extent_cursor_reuses_pages_across_hash_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sparse-cursor.bin");
        let file_len = VERIFY_READ_CHUNK_BYTES * 3 + 17;
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(file_len).unwrap();
        drop(file);

        let data = vec![0x6du8; 8192];
        let scheduler = ssd_scheduler();
        scheduled_write(
            &scheduler,
            IoClass::PeerWrite,
            &path,
            0,
            bytes::Bytes::from(data.clone()),
            false,
        )
        .await
        .unwrap();
        let mut expected = vec![0; file_len as usize];
        expected[..data.len()].copy_from_slice(&data);

        let mut cursor = SparseExtentCursor::new(&scheduler, &path, file_len);
        let mut actual = Vec::with_capacity(expected.len());
        let chunk_bytes = VERIFY_READ_CHUNK_BYTES as usize;
        while actual.len() < expected.len() {
            let len = (expected.len() - actual.len()).min(chunk_bytes);
            actual.extend_from_slice(&cursor.read_next(len).await.unwrap());
        }

        assert_eq!(actual, expected);
        assert_eq!(
            cursor.extent_page_count, 1,
            "one persistent scan page should serve all sequential chunks"
        );
    }

    #[tokio::test]
    async fn v2_file_verify_hashes_sparse_holes_as_zeroes() {
        let dir = tempfile::tempdir().unwrap();
        let fname = "sparse-v2.bin";
        let path = dir.path().join(fname);
        let file_len = (V2FileVerifier::LEAF_SIZE * 2) as u64;
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(file_len).unwrap();
        drop(file);
        let data_offset = V2FileVerifier::LEAF_SIZE + 2048;
        let data = vec![0xa5u8; 4096];
        scheduled_write(
            &ssd_scheduler(),
            IoClass::PeerWrite,
            &path,
            data_offset as u64,
            bytes::Bytes::from(data.clone()),
            false,
        )
        .await
        .unwrap();

        let mut expected = vec![0u8; file_len as usize];
        expected[data_offset..data_offset + data.len()].copy_from_slice(&data);
        let file = V2FileHash {
            file_index: 7,
            path: SafeRelPath::from_name(fname, false).unwrap(),
            length: file_len,
            pieces_root: Some(v2_file_root(&expected)),
        };
        let sched = ssd_scheduler();
        let verifier = V2FileVerifier::new(dir.path(), &sched, std::slice::from_ref(&file));

        assert_eq!(verifier.verify_file(&file).await, VerifyResult::Valid);
    }

    #[tokio::test]
    async fn v2_file_verify_rejects_wrong_file_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("data.bin"), b"content").unwrap();
        let file = V2FileHash {
            file_index: 0,
            path: SafeRelPath::from_name("data.bin", false).unwrap(),
            length: 7,
            pieces_root: Some([0u8; 32]),
        };
        let sched = ssd_scheduler();
        let verifier = V2FileVerifier::new(dir.path(), &sched, std::slice::from_ref(&file));

        assert_eq!(verifier.verify_file(&file).await, VerifyResult::Invalid);
    }

    #[tokio::test]
    async fn v2_file_verify_accepts_empty_file_without_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("empty.bin"), []).unwrap();
        let file = V2FileHash {
            file_index: 2,
            path: SafeRelPath::from_name("empty.bin", false).unwrap(),
            length: 0,
            pieces_root: None,
        };
        let sched = ssd_scheduler();
        let verifier = V2FileVerifier::new(dir.path(), &sched, std::slice::from_ref(&file));

        assert_eq!(verifier.verify_file(&file).await, VerifyResult::Valid);
    }

    #[tokio::test]
    async fn v2_file_verify_rejects_missing_empty_file_without_root() {
        let dir = tempfile::tempdir().unwrap();
        let file = V2FileHash {
            file_index: 3,
            path: SafeRelPath::from_name("missing-empty.bin", false).unwrap(),
            length: 0,
            pieces_root: None,
        };
        let sched = ssd_scheduler();
        let verifier = V2FileVerifier::new(dir.path(), &sched, std::slice::from_ref(&file));

        assert!(matches!(
            verifier.verify_file(&file).await,
            VerifyResult::Missing { file_index: 3, .. }
        ));
    }
}
