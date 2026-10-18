//! Shared bounded traversal for sparse source-candidate indexes.

/// Return the first usable candidate at or after `start`, wrapping once to
/// the beginning of the index. `next_candidate` must return sorted indexes
/// greater than or equal to its argument.
pub(crate) fn first_cyclic_candidate<E>(
    len: usize,
    start: usize,
    mut next_candidate: impl FnMut(usize) -> Option<usize>,
    mut is_usable: impl FnMut(usize) -> Result<bool, E>,
) -> Result<Option<usize>, E> {
    if len == 0 {
        return Ok(None);
    }

    let start = start % len;
    let mut candidate = next_candidate(start);
    while let Some(index) = candidate.filter(|index| *index >= start && *index < len) {
        if is_usable(index)? {
            return Ok(Some(index));
        }
        candidate = next_candidate(index.saturating_add(1));
    }

    if start == 0 {
        return Ok(None);
    }

    let mut candidate = next_candidate(0);
    while let Some(index) = candidate.filter(|index| *index < start && *index < len) {
        if is_usable(index)? {
            return Ok(Some(index));
        }
        candidate = next_candidate(index.saturating_add(1));
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::first_cyclic_candidate;
    use std::convert::Infallible;

    #[test]
    fn sparse_candidates_wrap_once_and_skip_unusable_entries() {
        let candidates = [1, 4, 6, 9];
        let selected = first_cyclic_candidate(
            10,
            5,
            |start| candidates.iter().copied().find(|index| *index >= start),
            |index| Ok::<_, Infallible>(index == 1),
        )
        .unwrap();

        assert_eq!(selected, Some(1));
    }

    #[test]
    fn empty_candidate_index_returns_without_querying_it() {
        let queried = std::cell::Cell::new(false);
        let selected = first_cyclic_candidate(
            0,
            0,
            |_| {
                queried.set(true);
                None
            },
            |_| Ok::<_, Infallible>(true),
        )
        .unwrap();

        assert_eq!(selected, None);
        assert!(!queried.get());
    }
}
