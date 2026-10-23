/// Tracks how many peers have each piece (availability count per piece).
///
/// Used by the rarest-first picker. Pieces are kept in compact, array-backed
/// buckets by availability count so the picker can walk from the rarest
/// nonzero bucket upward without one tree allocation per piece.
use rand::{Rng, RngExt};

use crate::flags::PieceFlags;
use crate::picker::PieceAvailability;

#[derive(Debug, Clone)]
pub struct Availability {
    /// counts[i] = number of peers that have piece i.
    counts: Vec<u32>,
    selection: SelectionState,
    /// Pieces excluded from candidate indexes. Their swarm counts remain
    /// live for diagnostics and endgame selection.
    retired: PieceFlags,
}

/// Only downloading torrents retain candidate and rarity indexes. Seed mode
/// keeps counts and retired flags in Availability so peer churn remains exact.
#[derive(Debug, Clone)]
enum SelectionState {
    Seed { piece_count: usize },
    Download(Box<DownloadSelection>),
}

#[derive(Debug, Clone)]
struct DownloadSelection {
    /// buckets[c] holds every active piece index currently at availability
    /// count `c`. Zero-availability pieces have no rarity-bucket entry.
    buckets: Vec<Vec<u32>>,
    /// Piece position in its rarity bucket, or `NO_POSITION` if it has no
    /// bucket entry.
    bucket_positions: Vec<u32>,
    /// Active fresh-request candidates, including pieces at zero peer
    /// availability (for sources such as webseeds).
    candidates: Vec<u32>,
    /// Piece position in `candidates`, or `NO_POSITION` if it is inactive.
    candidate_positions: Vec<u32>,
}

impl std::ops::Deref for SelectionState {
    type Target = DownloadSelection;
    fn deref(&self) -> &DownloadSelection {
        static EMPTY: DownloadSelection = DownloadSelection {
            buckets: Vec::new(),
            bucket_positions: Vec::new(),
            candidates: Vec::new(),
            candidate_positions: Vec::new(),
        };
        match self {
            Self::Seed { .. } => &EMPTY,
            Self::Download(index) => index,
        }
    }
}

impl std::ops::DerefMut for SelectionState {
    fn deref_mut(&mut self) -> &mut DownloadSelection {
        if let Self::Seed { piece_count } = self {
            *self = Self::Download(Box::new(DownloadSelection {
                buckets: Vec::new(),
                bucket_positions: vec![NO_POSITION; *piece_count],
                candidates: Vec::new(),
                candidate_positions: vec![NO_POSITION; *piece_count],
            }));
        }
        match self {
            Self::Download(index) => index,
            Self::Seed { .. } => unreachable!(),
        }
    }
}

const NO_POSITION: u32 = u32::MAX;
const RANDOM_CANDIDATE_PROBES: usize = 32;
const SPARSE_FALLBACK_BUCKET_MIN: usize = RANDOM_CANDIDATE_PROBES * 4;

/// Visit only set bits in a BitTorrent MSB-first bitfield. Empty bytes are
/// common for sparse peers, so skip their eight per-piece checks entirely.
fn for_each_set_piece(bitfield: &[u8], piece_count: usize, mut visit: impl FnMut(usize)) {
    for (byte_index, byte) in bitfield.iter().copied().enumerate() {
        let first_piece = byte_index.saturating_mul(u8::BITS as usize);
        if first_piece >= piece_count {
            break;
        }
        let mut remaining = byte;
        while remaining != 0 {
            let bit_offset = remaining.leading_zeros() as usize;
            let piece = first_piece.saturating_add(bit_offset);
            if piece >= piece_count {
                break;
            }
            visit(piece);
            remaining &= !(0x80 >> bit_offset);
        }
    }
}

impl Availability {
    pub fn new(piece_count: usize) -> Self {
        Availability {
            counts: vec![0; piece_count],
            selection: SelectionState::Download(Box::new(DownloadSelection {
                buckets: Vec::new(),
                bucket_positions: vec![NO_POSITION; piece_count],
                candidates: (0..piece_count).map(|piece| piece as u32).collect(),
                candidate_positions: (0..piece_count).map(|piece| piece as u32).collect(),
            })),
            retired: PieceFlags::new(piece_count, false),
        }
    }

    /// Create an availability index sized for a known connection ceiling.
    /// Pre-sizing the bucket headers avoids repeated table growth as peers
    /// advertise progressively rarer pieces; the per-bucket member vectors
    /// remain proportional to pieces actually present at each rarity level.
    pub fn new_bounded(piece_count: usize, max_peers: usize) -> Self {
        let mut availability = Self::new(piece_count);
        let bucket_count = max_peers.min(u32::MAX as usize).saturating_add(1);
        availability.selection.buckets.reserve(bucket_count);
        availability
            .selection
            .buckets
            .resize_with(bucket_count, Vec::new);
        availability
    }

    /// Conservative owner charge for an index with `piece_count` entries and
    /// at most `max_peers` live availability contributions. Bucket member
    /// capacity is bounded at four times live membership by `bucket_remove`;
    /// table-header capacity includes vector growth slack.
    pub fn memory_bytes_upper_bound(piece_count: usize, max_peers: usize) -> usize {
        let piece_slots = piece_count
            .saturating_mul(4)
            .saturating_mul(std::mem::size_of::<u32>());
        let bucket_count = max_peers
            .min(u32::MAX as usize)
            .saturating_add(1)
            .saturating_mul(2);
        std::mem::size_of::<Self>()
            .saturating_add(std::mem::size_of::<DownloadSelection>())
            .saturating_add(piece_slots)
            .saturating_add(
                piece_count
                    .saturating_mul(4)
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(
                piece_count
                    .div_ceil(u64::BITS as usize)
                    .saturating_mul(std::mem::size_of::<u64>()),
            )
            .saturating_add(bucket_count.saturating_mul(std::mem::size_of::<Vec<u32>>()))
    }

    pub fn piece_count(&self) -> usize {
        self.counts.len()
    }

    /// Record that a peer has all pieces in `bitfield` (one bit per piece, MSB first).
    pub fn add_bitfield(&mut self, bitfield: &[u8]) {
        for_each_set_piece(bitfield, self.counts.len(), |piece| self.add_have(piece));
    }

    /// Record that a peer has a single piece (Have message).
    pub fn add_have(&mut self, piece: usize) {
        let Some(c) = self.counts.get_mut(piece) else {
            return;
        };
        let old = *c;
        let new = c.saturating_add(1);
        *c = new;
        if new != old && !self.retired.get(piece) {
            let selection = &mut *self.selection;
            Self::bucket_remove(
                &mut selection.buckets,
                &mut selection.bucket_positions,
                old,
                piece as u32,
            );
            Self::bucket_insert(
                &mut selection.buckets,
                &mut selection.bucket_positions,
                new,
                piece as u32,
            );
        }
    }

    /// Remove one peer's ownership of a single piece.
    pub fn remove_have(&mut self, piece: usize) {
        let Some(c) = self.counts.get_mut(piece) else {
            return;
        };
        let old = *c;
        let new = c.saturating_sub(1);
        *c = new;
        if new != old && !self.retired.get(piece) {
            let selection = &mut *self.selection;
            Self::bucket_remove(
                &mut selection.buckets,
                &mut selection.bucket_positions,
                old,
                piece as u32,
            );
            Self::bucket_insert(
                &mut selection.buckets,
                &mut selection.bucket_positions,
                new,
                piece as u32,
            );
        }
    }

    /// A peer disconnected; remove its bitfield from availability.
    pub fn remove_bitfield(&mut self, bitfield: &[u8]) {
        for_each_set_piece(bitfield, self.counts.len(), |piece| self.remove_have(piece));
    }

    pub fn count(&self, piece: usize) -> u32 {
        self.counts.get(piece).copied().unwrap_or(0)
    }

    /// Pieces grouped by availability ascending (rarest first).
    /// Only returns pieces for which `want[piece]` is true and count > 0.
    ///
    /// Walks the availability buckets (already grouped and ordered by
    /// count) instead of sorting every piece on each call.
    pub fn rarest_first(&self, want: &[bool]) -> Vec<usize> {
        self.selection.buckets
            .iter()
            .skip(1) // bucket 0 is never populated (see field doc)
            .flat_map(|bucket| bucket.iter().copied())
            .map(|piece| piece as usize)
            .filter(|&piece| want.get(piece).copied().unwrap_or(false))
            .collect()
    }

    /// Walk availability buckets from the rarest nonzero count upward,
    /// returning the first piece index accepted by `predicate`.
    ///
    /// Within a bucket, pieces have no stable order because removal uses
    /// swap-remove. Callers that need fair tie handling should use
    /// `rarest_matching_random`.
    #[cfg(test)]
    pub(crate) fn rarest_matching<F>(&self, mut predicate: F) -> Option<usize>
    where
        F: FnMut(usize) -> bool,
    {
        for bucket in self.selection.buckets.iter().skip(1) {
            for &piece in bucket {
                let piece = piece as usize;
                if predicate(piece) {
                    return Some(piece);
                }
            }
        }
        None
    }

    /// Choose uniformly from requestable peer-held pieces at the lowest
    /// availability count. Small buckets are fully checked after bounded
    /// probes miss. For a large bucket, walk the peer's set bits once and use
    /// reservoir sampling across its actually available requestable pieces
    /// instead of testing every torrent candidate against the peer bitmap.
    pub fn rarest_matching_random_from_peer<A, F, R>(
        &self,
        peer_has: &A,
        predicate: F,
        rng: &mut R,
    ) -> Option<usize>
    where
        A: PieceAvailability + ?Sized,
        F: FnMut(usize) -> bool,
        R: Rng + ?Sized,
    {
        self.rarest_matching_from_peer_with_preference(peer_has, predicate, None, rng)
    }

    /// Prefer an already-started piece among equally rare candidates. Rarer
    /// pieces still win, and newly opened pieces retain random tie breaking.
    pub fn rarest_matching_from_peer_with_preference<A, F, R>(
        &self,
        peer_has: &A,
        mut predicate: F,
        preferred: Option<usize>,
        rng: &mut R,
    ) -> Option<usize>
    where
        A: PieceAvailability + ?Sized,
        F: FnMut(usize) -> bool,
        R: Rng + ?Sized,
    {
        let preferred = preferred.filter(|&piece| {
            self.selection
                .candidate_positions
                .get(piece)
                .is_some_and(|&pos| pos != NO_POSITION)
                && self.count(piece) > 0
                && peer_has.has_piece(piece)
                && predicate(piece)
        });
        for (count, bucket) in self
            .selection
            .buckets
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, bucket)| !bucket.is_empty())
        {
            if let Some(piece) = preferred.filter(|&piece| self.count(piece) as usize == count) {
                return Some(piece);
            }
            for _ in 0..bucket.len().min(RANDOM_CANDIDATE_PROBES) {
                let piece = bucket[rng.random_range(0..bucket.len())] as usize;
                if peer_has.has_piece(piece) && predicate(piece) {
                    return Some(piece);
                }
            }

            if bucket.len() >= SPARSE_FALLBACK_BUCKET_MIN {
                let selected = self.rarest_matching_from_peer_set(peer_has, &mut predicate, rng)?;
                return Some(
                    preferred
                        .filter(|&piece| self.count(piece) == self.count(selected))
                        .unwrap_or(selected),
                );
            }

            let mut selected = None;
            let mut matched = 0usize;
            for &piece in bucket {
                let piece = piece as usize;
                if peer_has.has_piece(piece) && predicate(piece) {
                    matched = matched.saturating_add(1);
                    if rng.random_range(0..matched) == 0 {
                        selected = Some(piece);
                    }
                }
            }
            if selected.is_some() {
                return selected;
            }
        }
        None
    }

    fn rarest_matching_from_peer_set<A, F, R>(
        &self,
        peer_has: &A,
        predicate: &mut F,
        rng: &mut R,
    ) -> Option<usize>
    where
        A: PieceAvailability + ?Sized,
        F: FnMut(usize) -> bool,
        R: Rng + ?Sized,
    {
        let mut selected = None;
        let mut selected_count = u32::MAX;
        let mut tied = 0usize;
        peer_has.for_each_set_piece(self.counts.len(), &mut |piece| {
            if piece >= self.counts.len()
                || self.selection.candidate_positions[piece] == NO_POSITION
                || !predicate(piece)
            {
                return;
            }
            let count = self.counts[piece];
            if count == 0 {
                return;
            }
            if count < selected_count {
                selected = Some(piece);
                selected_count = count;
                tied = 1;
            } else if count == selected_count {
                tied = tied.saturating_add(1);
                if rng.random_range(0..tied) == 0 {
                    selected = Some(piece);
                }
            }
        });
        selected
    }

    /// Select uniformly from matching active candidates. Unlike the rarity
    /// buckets, this also includes zero-availability pieces for sources whose
    /// complete inventory is known out of band, such as webseeds.
    pub(crate) fn candidate_matching_random<F, R>(
        &self,
        mut predicate: F,
        rng: &mut R,
    ) -> Option<usize>
    where
        F: FnMut(usize) -> bool,
        R: Rng + ?Sized,
    {
        if self.selection.candidates.is_empty() {
            return None;
        }
        for _ in 0..self.selection.candidates.len().min(RANDOM_CANDIDATE_PROBES) {
            let piece = self.selection.candidates
                [rng.random_range(0..self.selection.candidates.len())]
                as usize;
            if predicate(piece) {
                return Some(piece);
            }
        }
        let mut matched = 0usize;
        let mut selected = None;
        for &piece in &self.selection.candidates {
            let piece = piece as usize;
            if predicate(piece) {
                matched = matched.saturating_add(1);
                if rng.random_range(0..matched) == 0 {
                    selected = Some(piece);
                }
            }
        }
        selected
    }

    /// Return the first active candidate accepted by `predicate` without
    /// scanning every piece or allocating a filtered list. Used for sources
    /// with complete availability, such as webseeds.
    #[cfg(test)]
    pub(crate) fn first_candidate_matching<F>(&self, mut predicate: F) -> Option<usize>
    where
        F: FnMut(usize) -> bool,
    {
        self.selection
            .candidates
            .iter()
            .map(|piece| *piece as usize)
            .find(|piece| predicate(*piece))
    }

    pub(crate) fn candidate_count(&self) -> usize {
        self.selection.candidates.len()
    }

    /// Exclude a locally complete or otherwise unavailable piece from fresh
    /// candidate selection while retaining its swarm count for diagnostics.
    pub fn retire_candidate(&mut self, piece: usize) {
        self.retire(piece);
    }

    /// Make a piece eligible for fresh candidate selection again after local
    /// verification or file policy marks it incomplete and wanted.
    pub fn reinstate_candidate(&mut self, piece: usize) {
        self.reinstate(piece);
    }

    pub(crate) fn candidate_min_by_key<F>(&self, mut key: F) -> Option<usize>
    where
        F: FnMut(usize) -> Option<usize>,
    {
        self.selection
            .candidates
            .iter()
            .map(|piece| *piece as usize)
            .filter_map(|piece| key(piece).map(|key| (key, piece)))
            .min()
            .map(|(_, piece)| piece)
    }

    /// Report the allocated backing storage for availability indexes.
    /// Capacities are used so owner accounting includes vector growth slack.
    pub fn memory_bytes(&self) -> usize {
        let bucket_entries = self
            .selection
            .buckets
            .iter()
            .map(|bucket| bucket.capacity().saturating_mul(std::mem::size_of::<u32>()))
            .fold(0usize, usize::saturating_add);
        self.counts
            .capacity()
            .saturating_mul(std::mem::size_of::<u32>())
            .saturating_add(
                self.selection
                    .bucket_positions
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(bucket_entries)
            .saturating_add(
                self.selection
                    .buckets
                    .capacity()
                    .saturating_mul(std::mem::size_of::<Vec<u32>>()),
            )
            .saturating_add(
                self.selection
                    .candidates
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(
                self.selection
                    .candidate_positions
                    .capacity()
                    .saturating_mul(std::mem::size_of::<u32>()),
            )
            .saturating_add(self.retired.memory_bytes())
            .saturating_add(if matches!(self.selection, SelectionState::Download(_)) {
                std::mem::size_of::<DownloadSelection>()
            } else {
                0
            })
    }

    /// Remove a piece from the active candidate indexes while preserving its
    /// swarm count. The owner calls this when no fresh block can be requested
    /// for the piece. Idempotent.
    pub(crate) fn retire(&mut self, piece: usize) {
        if piece >= self.retired.len() || self.retired.get(piece) {
            return;
        }
        self.retired.set(piece, true);
        let count = self.count(piece);
        let selection = &mut *self.selection;
        Self::bucket_remove(
            &mut selection.buckets,
            &mut selection.bucket_positions,
            count,
            piece as u32,
        );
        Self::candidate_remove(
            &mut selection.candidates,
            &mut selection.candidate_positions,
            piece as u32,
        );
    }

    /// Reverse of `retire`: restore a piece to the active candidate indexes
    /// at its current swarm count. Used when a piece can accept fresh work
    /// again (for example after a cancelled request).
    /// Idempotent.
    pub(crate) fn reinstate(&mut self, piece: usize) {
        if piece >= self.retired.len() || !self.retired.get(piece) {
            return;
        }
        self.retired.set(piece, false);
        let count = self.count(piece);
        let selection = &mut *self.selection;
        Self::bucket_insert(
            &mut selection.buckets,
            &mut selection.bucket_positions,
            count,
            piece as u32,
        );
        selection.candidate_positions[piece] = selection.candidates.len() as u32;
        selection.candidates.push(piece as u32);
    }

    /// Release only download selection storage. Swarm counts remain exact
    /// while seeding, including peer have/bitfield/disconnect updates.
    pub(crate) fn compact_for_seed(&mut self) {
        debug_assert!(self.selection.candidates.is_empty());
        self.selection = SelectionState::Seed {
            piece_count: self.counts.len(),
        };
    }

    fn bucket_insert(buckets: &mut Vec<Vec<u32>>, positions: &mut [u32], count: u32, piece: u32) {
        if count == 0 {
            return;
        }
        let idx = count as usize;
        if idx >= buckets.len() {
            buckets.resize_with(idx + 1, Vec::new);
        }
        let Some(position) = positions.get_mut(piece as usize) else {
            return;
        };
        *position = buckets[idx].len() as u32;
        buckets[idx].push(piece);
    }

    fn bucket_remove(buckets: &mut [Vec<u32>], positions: &mut [u32], count: u32, piece: u32) {
        if count == 0 {
            return;
        }
        let Some(bucket) = buckets.get_mut(count as usize) else {
            return;
        };
        let Some(position) = positions.get(piece as usize).copied() else {
            return;
        };
        if position == NO_POSITION {
            return;
        }
        let index = position as usize;
        if index >= bucket.len() || bucket[index] != piece {
            positions[piece as usize] = NO_POSITION;
            return;
        }
        bucket.swap_remove(index);
        positions[piece as usize] = NO_POSITION;
        if let Some(&moved) = bucket.get(index) {
            positions[moved as usize] = index as u32;
        }
        if bucket.is_empty() {
            // Empty rarity buckets do not retain an allocation. This keeps
            // historical availability churn from accumulating one spare
            // buffer per count level.
            *bucket = Vec::new();
        } else if bucket.capacity() > bucket.len().saturating_mul(4) {
            // A bucket can become much smaller after peer churn. Keep
            // retained capacity proportional to live membership so moving
            // pieces among rarity levels cannot retain a torrent-sized
            // buffer at every historical level.
            bucket.shrink_to_fit();
        }
    }

    fn candidate_remove(candidates: &mut Vec<u32>, positions: &mut [u32], piece: u32) {
        let Some(position) = positions.get(piece as usize).copied() else {
            return;
        };
        if position == NO_POSITION {
            return;
        }
        let index = position as usize;
        if index >= candidates.len() || candidates[index] != piece {
            positions[piece as usize] = NO_POSITION;
            return;
        }
        candidates.swap_remove(index);
        positions[piece as usize] = NO_POSITION;
        if let Some(&moved) = candidates.get(index) {
            positions[moved as usize] = index as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingPeerAvailability {
        pieces: Vec<bool>,
        set_walks: std::cell::Cell<usize>,
    }

    impl PieceAvailability for CountingPeerAvailability {
        fn has_piece(&self, piece: usize) -> bool {
            self.pieces.get(piece).copied().unwrap_or(false)
        }

        fn for_each_set_piece(&self, piece_count: usize, visitor: &mut dyn FnMut(usize)) {
            self.set_walks.set(self.set_walks.get() + 1);
            for piece in 0..piece_count {
                if self.has_piece(piece) {
                    visitor(piece);
                }
            }
        }
    }

    #[test]
    fn add_bitfield_increments_counts() {
        let mut av = Availability::new(8);
        // bitfield: 11110000 → pieces 0,1,2,3
        av.add_bitfield(&[0b1111_0000]);
        assert_eq!(av.count(0), 1);
        assert_eq!(av.count(3), 1);
        assert_eq!(av.count(4), 0);
    }

    #[test]
    fn sparse_bitfield_visits_msb_pieces_and_ignores_unused_tail_bits() {
        let mut av = Availability::new(11);
        let bitfield = [0b0000_0101, 0b0100_0001];

        av.add_bitfield(&bitfield);
        assert_eq!(av.count(5), 1);
        assert_eq!(av.count(7), 1);
        assert_eq!(av.count(9), 1);
        assert_eq!(av.count(10), 0);
        av.remove_bitfield(&bitfield);
        for piece in 0..av.piece_count() {
            assert_eq!(av.count(piece), 0);
        }
    }

    #[test]
    fn add_have_increments_single() {
        let mut av = Availability::new(4);
        av.add_have(2);
        assert_eq!(av.count(2), 1);
        assert_eq!(av.count(0), 0);
    }

    #[test]
    fn remove_bitfield_decrements() {
        let mut av = Availability::new(8);
        av.add_bitfield(&[0xFF]);
        av.remove_bitfield(&[0xFF]);
        for i in 0..8 {
            assert_eq!(av.count(i), 0);
        }
    }

    #[test]
    fn remove_does_not_underflow() {
        let mut av = Availability::new(4);
        av.remove_bitfield(&[0xFF]); // should not panic or underflow
        assert_eq!(av.count(0), 0);
    }

    #[test]
    fn remove_have_decrements_single_piece() {
        let mut av = Availability::new(4);
        av.add_have(2);
        av.add_have(2);

        av.remove_have(2);

        assert_eq!(av.count(2), 1);
        assert_eq!(av.count(0), 0);
    }

    #[test]
    fn rarest_first_orders_ascending() {
        let mut av = Availability::new(4);
        // piece 0: 3 peers, piece 1: 1 peer, piece 2: 2 peers, piece 3: 0 peers
        for _ in 0..3 {
            av.add_have(0);
        }
        av.add_have(1);
        av.add_have(1);
        av.add_have(2);
        av.add_have(2);
        av.add_have(2);
        // Recount: 0→3, 1→2, 2→3, 3→0
        // Rarest first (excluding unavailable): piece 1 (2), then 0 or 2 (both 3)
        let want = vec![true, true, true, true];
        let order = av.rarest_first(&want);
        assert_eq!(order[0], 1); // rarest
        assert!(order.contains(&0));
        assert!(order.contains(&2));
        assert!(!order.contains(&3)); // count=0
    }

    #[test]
    fn rarest_first_respects_want_filter() {
        let mut av = Availability::new(4);
        av.add_have(0);
        av.add_have(1);
        let want = vec![true, false, false, false];
        let order = av.rarest_first(&want);
        assert_eq!(order, vec![0]);
    }

    #[test]
    fn bitfield_out_of_bounds_ignored() {
        let mut av = Availability::new(4);
        // Bitfield has 8 bits but only 4 pieces exist — extra bits silently ignored.
        av.add_bitfield(&[0xFF]);
        assert_eq!(av.piece_count(), 4);
        for i in 0..4 {
            assert_eq!(av.count(i), 1);
        }
    }

    #[test]
    fn rarest_matching_walks_buckets_in_rarity_order() {
        let mut av = Availability::new(4);
        av.add_have(0);
        av.add_have(0);
        av.add_have(1);
        av.add_have(2);
        av.add_have(2);
        // counts: 0→2, 1→1, 2→2, 3→0
        let mut visited = Vec::new();
        let found = av.rarest_matching(|piece| {
            visited.push(piece);
            piece == 2
        });
        // Rarest bucket (count 1) is piece 1 — visited but rejected.
        // The next bucket (count 2) contains pieces 0 and 2. Bucket order is
        // intentionally unstable after swap-remove.
        assert_eq!(visited.len(), 3);
        assert_eq!(visited[0], 1);
        assert!(visited[1..].contains(&0));
        assert!(visited[1..].contains(&2));
        assert_eq!(found, Some(2));
    }

    #[test]
    fn rarest_matching_skips_zero_availability_pieces() {
        let av = Availability::new(4);
        assert_eq!(av.rarest_matching(|_| true), None);
    }

    #[test]
    fn retire_removes_piece_from_bucket_walk_but_keeps_count() {
        let mut av = Availability::new(2);
        av.add_have(0);
        av.add_have(1);

        av.retire(0);

        assert_eq!(av.count(0), 1); // count() unaffected by retirement
        let mut visited = Vec::new();
        av.rarest_matching(|piece| {
            visited.push(piece);
            false
        });
        assert_eq!(visited, vec![1]);
    }

    #[test]
    fn reinstate_returns_a_retired_piece_to_its_current_bucket() {
        let mut av = Availability::new(1);
        av.add_have(0);
        av.retire(0);
        assert_eq!(av.rarest_matching(|_| true), None);

        av.reinstate(0);

        assert_eq!(av.rarest_matching(|_| true), Some(0));
    }

    #[test]
    fn retiring_a_piece_suppresses_further_bucket_moves_until_reinstated() {
        let mut av = Availability::new(1);
        av.add_have(0);
        av.retire(0);

        // Count keeps tracking swarm changes while retired...
        av.add_have(0);
        av.remove_have(0);
        assert_eq!(av.count(0), 1);

        // ...but the piece stays out of the walk until reinstated, then
        // reappears at its current (up to date) count.
        assert_eq!(av.rarest_matching(|_| true), None);
        av.reinstate(0);
        assert_eq!(av.rarest_matching(|_| true), Some(0));
    }

    #[test]
    fn retire_and_reinstate_are_idempotent() {
        let mut av = Availability::new(1);
        av.add_have(0);
        av.retire(0);
        av.retire(0); // no-op, must not double-remove or panic
        assert_eq!(av.rarest_matching(|_| true), None);

        av.reinstate(0);
        av.reinstate(0); // no-op
        assert_eq!(av.rarest_matching(|_| true), Some(0));
    }

    #[test]
    fn moving_between_buckets_does_not_leak_stale_membership() {
        let mut av = Availability::new(2);
        av.add_have(0);
        av.add_have(0); // count 2
        av.remove_have(0); // back to count 1
        let mut visited = Vec::new();
        av.rarest_matching(|piece| {
            visited.push(piece);
            false
        });
        // Piece 0 should show up exactly once (in the count-1 bucket), not
        // linger in a stale count-2 bucket entry.
        assert_eq!(visited, vec![0]);
    }

    #[test]
    fn array_bucket_swap_remove_keeps_piece_positions_coherent() {
        let mut av = Availability::new(4);
        for piece in 0..4 {
            av.add_have(piece);
        }

        // Remove an interior bucket element, causing the tail to move into
        // its slot, then move that tail piece to a different rarity bucket.
        av.retire(1);
        av.add_have(3);
        let mut found = Vec::new();
        av.rarest_matching(|piece| {
            found.push(piece);
            false
        });

        assert_eq!(found, vec![0, 2, 3]);
        assert_eq!(av.count(0), 1);
        assert_eq!(av.count(2), 1);
        assert_eq!(av.count(3), 2);
        av.reinstate(1);
        assert_eq!(av.rarest_matching(|piece| piece == 1), Some(1));
    }

    #[test]
    fn active_candidate_index_excludes_retired_work_and_restores_it() {
        let mut av = Availability::new(3);
        av.add_have(1);
        assert_eq!(av.candidate_count(), 3);

        av.retire(1);
        assert_eq!(av.candidate_count(), 2);
        assert_eq!(av.first_candidate_matching(|piece| piece == 1), None);

        av.reinstate(1);
        assert_eq!(av.candidate_count(), 3);
        assert_eq!(av.first_candidate_matching(|piece| piece == 1), Some(1));
        assert_eq!(av.count(1), 1);
    }

    #[test]
    fn candidate_swap_remove_updates_the_tail_piece_position() {
        let mut av = Availability::new(4);

        av.retire(1);
        av.retire(3);

        assert_eq!(av.candidate_count(), 2);
        assert_eq!(av.first_candidate_matching(|piece| piece == 1), None);
        assert_eq!(av.first_candidate_matching(|piece| piece == 3), None);
        assert_eq!(av.first_candidate_matching(|piece| piece == 2), Some(2));
    }

    #[test]
    fn random_rarest_tie_break_reaches_each_equal_candidate() {
        use rand::SeedableRng;

        let mut av = Availability::new(8);
        for piece in 0..8 {
            av.add_have(piece);
        }
        let peer = vec![true; 8];
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x5eed);
        let selected: std::collections::BTreeSet<_> = (0..128)
            .filter_map(|_| av.rarest_matching_random_from_peer(&peer, |_| true, &mut rng))
            .collect();

        assert_eq!(selected, (0..8).collect());
    }

    #[test]
    fn sparse_peer_fallback_selects_uniformly_from_its_rarest_matches() {
        use rand::SeedableRng;

        let mut av = Availability::new(512);
        for piece in 0..512 {
            av.add_have(piece);
        }
        av.add_have(400);
        av.add_have(401);

        // The large first bucket contains no peer-held pieces, so its bounded
        // probes fall through to the set-bit walk.
        let mut pieces = vec![false; 512];
        pieces[400] = true;
        pieces[401] = true;
        let peer = CountingPeerAvailability {
            pieces,
            set_walks: std::cell::Cell::new(0),
        };
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x5eed);
        let selected: std::collections::BTreeSet<_> = (0..128)
            .filter_map(|_| av.rarest_matching_random_from_peer(&peer, |_| true, &mut rng))
            .collect();

        assert_eq!(selected, [400, 401].into_iter().collect());
        assert_eq!(peer.set_walks.get(), 128);
    }

    #[test]
    fn sparse_peer_fallback_skips_retired_or_unrequestable_matches() {
        use rand::SeedableRng;

        let mut av = Availability::new(512);
        for piece in 0..512 {
            av.add_have(piece);
        }
        av.add_have(400);
        av.add_have(401);
        av.retire(400);

        let mut peer = vec![false; 512];
        peer[400] = true;
        peer[401] = true;
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x5eed);

        assert_eq!(
            av.rarest_matching_random_from_peer(&peer, |piece| piece != 401, &mut rng),
            None
        );
        assert_eq!(
            av.rarest_matching_random_from_peer(&peer, |_| true, &mut rng),
            Some(401)
        );
    }

    #[test]
    fn small_empty_rarity_bucket_does_not_force_a_peer_set_walk() {
        use rand::SeedableRng;

        let mut av = Availability::new(64);
        av.add_have(0);
        for piece in 1..64 {
            av.add_have(piece);
            av.add_have(piece);
        }
        let mut pieces = vec![false; 64];
        pieces[32] = true;
        let peer = CountingPeerAvailability {
            pieces,
            set_walks: std::cell::Cell::new(0),
        };
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x5eed);

        assert_eq!(
            av.rarest_matching_random_from_peer(&peer, |_| true, &mut rng),
            Some(32)
        );
        assert_eq!(peer.set_walks.get(), 0);
    }

    #[test]
    fn random_seed_candidate_tie_break_reaches_zero_availability_pieces() {
        use rand::SeedableRng;

        let av = Availability::new(8);
        let mut rng = rand::rngs::StdRng::seed_from_u64(0x5eed);
        let selected: std::collections::BTreeSet<_> = (0..128)
            .filter_map(|_| av.candidate_matching_random(|_| true, &mut rng))
            .collect();

        assert_eq!(selected, (0..8).collect());
    }

    #[test]
    fn rarity_bucket_capacity_tracks_live_members_after_peer_churn() {
        let piece_count = 512;
        let remaining_in_first_bucket = 32;
        let mut av = Availability::new(piece_count);

        for piece in 0..piece_count {
            av.add_have(piece);
        }
        for _ in remaining_in_first_bucket..piece_count {
            // Repeatedly remove the first entry. swap_remove fills that slot
            // from the bucket tail, so this leaves an over-sized allocation
            // unless trimming considers interior removals too.
            let piece = av.selection.buckets[1][0] as usize;
            av.add_have(piece);
        }

        let rarity_one_bucket = &av.selection.buckets[1];
        assert_eq!(rarity_one_bucket.len(), remaining_in_first_bucket);
        assert!(rarity_one_bucket.capacity() <= rarity_one_bucket.len() * 4);
        assert!(av
            .selection
            .bucket_positions
            .iter()
            .all(|position| *position != NO_POSITION));
        assert!(av
            .selection
            .candidate_positions
            .iter()
            .all(|position| *position != NO_POSITION));
    }

    #[test]
    fn bounded_index_reserves_header_and_member_growth_budget() {
        let piece_count = 257;
        let max_peers = 17;
        let mut av = Availability::new_bounded(piece_count, max_peers);
        for peer in 0..max_peers {
            for piece in (peer..piece_count).step_by(3) {
                av.add_have(piece);
            }
        }

        assert!(av.selection.buckets.capacity() > max_peers);
        assert!(
            Availability::memory_bytes_upper_bound(piece_count, max_peers) >= av.memory_bytes(),
            "reserved bound must cover every live index allocation"
        );
    }
}
