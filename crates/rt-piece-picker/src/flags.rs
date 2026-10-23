/// Packed per-piece boolean flags: one bit per piece.
///
/// `Vec<bool>` stores one byte per element, eight times the memory the
/// picker's budget assumes. This keeps the same indexing semantics (out of
/// range reads are `false`, out of range writes are ignored) in 64-bit words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PieceFlags {
    words: Vec<u64>,
    len: usize,
    ones: usize,
}

impl PieceFlags {
    pub(crate) fn new(len: usize, value: bool) -> Self {
        Self {
            words: Vec::new(),
            len,
            ones: if value { len } else { 0 },
        }
    }

    pub(crate) fn from_bools(bools: &[bool]) -> Self {
        let mut flags = Self::new(bools.len(), false);
        for (index, value) in bools.iter().enumerate() {
            if *value {
                flags.set(index, true);
            }
        }
        flags
    }

    pub(crate) fn memory_bytes(&self) -> usize {
        self.words
            .capacity()
            .saturating_mul(std::mem::size_of::<u64>())
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn get(&self, index: usize) -> bool {
        index < self.len
            && if self.words.is_empty() {
                self.ones != 0
            } else {
                self.words[index / 64] & (1 << (index % 64)) != 0
            }
    }

    pub(crate) fn set(&mut self, index: usize, value: bool) {
        if index >= self.len || self.get(index) == value {
            return;
        }
        if self.words.is_empty() {
            self.words = vec![if self.ones == 0 { 0 } else { u64::MAX }; self.len.div_ceil(64)];
            let tail = self.len % 64;
            if tail != 0 {
                if let Some(last) = self.words.last_mut() {
                    *last &= (1u64 << tail) - 1;
                }
            }
        }
        self.ones = if value { self.ones + 1 } else { self.ones - 1 };
        let mask = 1u64 << (index % 64);
        if value {
            self.words[index / 64] |= mask;
        } else {
            self.words[index / 64] &= !mask;
        }
        if self.ones == 0 || self.ones == self.len {
            self.words = Vec::new();
        }
    }

    pub(crate) fn fill(&mut self, value: bool) {
        self.words = Vec::new();
        self.ones = if value { self.len } else { 0 };
    }

    pub(crate) fn is_empty_set(&self) -> bool {
        self.ones == 0
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = bool> + '_ {
        (0..self.len).map(|index| self.get(index))
    }
}

#[cfg(test)]
mod tests {
    use super::PieceFlags;

    #[test]
    fn flags_are_bit_packed_and_bounds_checked() {
        let mut flags = PieceFlags::new(130, true);
        assert!(flags.get(0) && flags.get(129));
        assert!(!flags.get(130), "out of range reads are false");
        flags.set(64, false);
        flags.set(500, true);
        assert!(!flags.get(64) && !flags.get(500));
        assert_eq!(flags.iter().filter(|set| *set).count(), 129);
        flags.fill(false);
        assert_eq!(flags.iter().filter(|set| *set).count(), 0);
        assert_eq!(
            flags.memory_bytes(),
            0,
            "uniform flags release backing storage"
        );
    }

    #[test]
    fn from_bools_roundtrips() {
        let bools: Vec<bool> = (0..200).map(|i| i % 3 == 0).collect();
        let flags = PieceFlags::from_bools(&bools);
        assert_eq!(flags.iter().collect::<Vec<_>>(), bools);
    }
}
