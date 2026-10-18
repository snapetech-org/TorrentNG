/// Packed per-piece boolean flags: one bit per piece.
///
/// `Vec<bool>` stores one byte per element, eight times the memory the
/// picker's budget assumes. This keeps the same indexing semantics (out of
/// range reads are `false`, out of range writes are ignored) in 64-bit words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PieceFlags {
    words: Vec<u64>,
    len: usize,
}

impl PieceFlags {
    pub(crate) fn new(len: usize, value: bool) -> Self {
        let mut flags = Self {
            words: vec![0; len.div_ceil(64)],
            len,
        };
        if value {
            flags.fill(true);
        }
        flags
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
        self.words.capacity().saturating_mul(std::mem::size_of::<u64>())
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn get(&self, index: usize) -> bool {
        index < self.len && self.words[index / 64] & (1 << (index % 64)) != 0
    }

    pub(crate) fn set(&mut self, index: usize, value: bool) {
        if index >= self.len {
            return;
        }
        let mask = 1u64 << (index % 64);
        if value {
            self.words[index / 64] |= mask;
        } else {
            self.words[index / 64] &= !mask;
        }
    }

    pub(crate) fn fill(&mut self, value: bool) {
        self.words.fill(if value { u64::MAX } else { 0 });
        // Keep bits past `len` clear so word-wise counts stay exact.
        if value {
            let tail = self.len % 64;
            if tail != 0 {
                if let Some(last) = self.words.last_mut() {
                    *last = (1u64 << tail) - 1;
                }
            }
        }
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
        assert_eq!(std::mem::size_of_val(flags.words.as_slice()), 3 * 8);
    }

    #[test]
    fn from_bools_roundtrips() {
        let bools: Vec<bool> = (0..200).map(|i| i % 3 == 0).collect();
        let flags = PieceFlags::from_bools(&bools);
        assert_eq!(flags.iter().collect::<Vec<_>>(), bools);
    }
}
