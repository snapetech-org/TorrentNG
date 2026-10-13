//! UI-side window of the torrent list.
//!
//! The table never holds the whole library: it keeps one page of rows around
//! the viewport (server-side sort/filter/paginate) and patches them in place
//! from SSE deltas. This is what keeps memory flat at 100k+ torrents.

use std::collections::HashMap;

use crate::model::{LiveRow, Row, TorrentSummary, ViewSpec};

/// Rows requested per page.
pub const PAGE: usize = 200;
/// Re-window when the viewport gets this close to the loaded edge.
const MARGIN: usize = 40;

#[derive(Debug, Default)]
pub struct ViewState {
    pub spec: ViewSpec,
    /// Bumped on every spec change so late responses for an old view are dropped.
    pub epoch: u64,
    pub offset: usize,
    pub rows: Vec<Row>,
    pub total: usize,
    pub revision: u64,
    pub loaded: bool,
}

impl ViewState {
    /// Change the view definition. Existing rows stay on screen (no flicker)
    /// until the first page for the new epoch arrives.
    pub fn set_spec(&mut self, spec: ViewSpec) -> bool {
        if spec == self.spec {
            return false;
        }
        self.spec = spec;
        self.epoch += 1;
        self.offset = 0;
        true
    }

    /// Accept a page. Returns `false` (and ignores it) for a stale epoch.
    pub fn apply_page(
        &mut self,
        epoch: u64,
        offset: usize,
        total: usize,
        revision: u64,
        rows: Vec<TorrentSummary>,
    ) -> bool {
        if epoch != self.epoch {
            return false;
        }
        // Keep the last known live rates so a refresh does not flash dashes.
        let rates: HashMap<&str, (i64, i64)> = self
            .rows
            .iter()
            .map(|r| (r.t.info_hash.as_str(), (r.down_rate, r.up_rate)))
            .collect();
        let new_rows: Vec<Row> = rows
            .into_iter()
            .map(|t| {
                let (d, u) = rates.get(t.info_hash.as_str()).copied().unwrap_or((0, 0));
                Row {
                    t,
                    down_rate: d,
                    up_rate: u,
                }
            })
            .collect();
        self.offset = offset;
        self.total = total;
        self.revision = revision;
        self.rows = new_rows;
        self.loaded = true;
        true
    }

    /// Replace summaries for rows that are in the window. Returns how many changed.
    pub fn apply_patch(&mut self, patch: &[TorrentSummary]) -> usize {
        let mut n = 0;
        for p in patch {
            if let Some(r) = self.rows.iter_mut().find(|r| r.t.info_hash == p.info_hash) {
                if r.t != *p {
                    r.t = p.clone();
                    n += 1;
                }
            }
        }
        n
    }

    pub fn apply_live(&mut self, live: &[LiveRow]) {
        for l in live {
            if let Some(r) = self.rows.iter_mut().find(|r| r.t.info_hash == l.hash) {
                r.down_rate = l.download_rate;
                r.up_rate = l.upload_rate;
                r.t.amount_left = l.amount_left as i64;
            }
        }
    }

    /// Row for absolute index `idx` in the full (server-ordered) list.
    pub fn row(&self, idx: usize) -> Option<&Row> {
        idx.checked_sub(self.offset).and_then(|i| self.rows.get(i))
    }

    /// New window offset to request for the viewport `first..last`, or `None`
    /// if the loaded rows still cover it with margin.
    pub fn wanted_window(&self, first: usize, last: usize) -> Option<usize> {
        if !self.loaded {
            return None;
        }
        let loaded_end = self.offset + self.rows.len();
        let covers_start = self.offset == 0 || first >= self.offset + MARGIN;
        let covers_end = loaded_end >= self.total || last + MARGIN <= loaded_end;
        if covers_start && covers_end {
            return None;
        }
        // Centre the page on the viewport.
        let mid = (first + last) / 2;
        let off = mid.saturating_sub(PAGE / 2);
        // Round to a multiple of 50 so scrolling does not request every frame.
        Some(off / 50 * 50)
    }

    /// Hashes of the rows inside the viewport (bounded; used for `/live`).
    pub fn visible_hashes(&self, first: usize, last: usize) -> Vec<String> {
        (first..=last)
            .filter_map(|i| self.row(i))
            .take(128)
            .map(|r| r.t.info_hash.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(n: usize) -> TorrentSummary {
        TorrentSummary {
            info_hash: format!("{n:040x}"),
            name: format!("t{n}"),
            state: "seeding".into(),
            total_length: 10,
            downloaded: 10,
            amount_left: 0,
            uploaded: 0,
            ratio: 0.0,
            save_path: String::new(),
            category: None,
            tags: vec![],
            added_at: 0,
            completed_at: None,
            num_peers: 0,
            num_seeds: 0,
            tracker_message: None,
            finalizing: false,
        }
    }

    fn page(v: &mut ViewState, offset: usize, n: usize, total: usize) {
        let e = v.epoch;
        assert!(v.apply_page(e, offset, total, 1, (offset..offset + n).map(t).collect()));
    }

    #[test]
    fn stale_epoch_pages_are_dropped() {
        let mut v = ViewState::default();
        let old = v.epoch;
        assert!(v.set_spec(ViewSpec {
            filter: "x".into(),
            ..Default::default()
        }));
        assert!(!v.apply_page(old, 0, 1, 1, vec![t(1)]));
        assert!(!v.loaded);
        assert!(
            !v.set_spec(v.spec.clone()),
            "identical spec must not bump the epoch"
        );
    }

    #[test]
    fn rows_are_addressed_by_absolute_index() {
        let mut v = ViewState::default();
        page(&mut v, 100, 200, 10_000);
        assert!(v.row(99).is_none());
        assert_eq!(v.row(100).unwrap().t.name, "t100");
        assert_eq!(v.row(299).unwrap().t.name, "t299");
        assert!(v.row(300).is_none());
    }

    #[test]
    fn window_only_moves_near_the_edge() {
        let mut v = ViewState::default();
        page(&mut v, 0, 200, 10_000);
        assert_eq!(v.wanted_window(0, 30), None);
        assert_eq!(v.wanted_window(100, 130), None);
        let off = v.wanted_window(150, 185).expect("near end of loaded page");
        assert!(off <= 150 && off % 50 == 0);
        let off = v.wanted_window(5000, 5030).unwrap();
        assert_eq!(off, 4900);
        // Small libraries never re-window.
        let mut s = ViewState::default();
        page(&mut s, 0, 20, 20);
        assert_eq!(s.wanted_window(0, 19), None);
    }

    #[test]
    fn patch_and_live_update_in_place_and_survive_refresh() {
        let mut v = ViewState::default();
        page(&mut v, 0, 3, 3);
        let mut p = t(1);
        p.name = "renamed".into();
        assert_eq!(v.apply_patch(&[p.clone(), t(99)]), 1);
        assert_eq!(v.row(1).unwrap().t.name, "renamed");
        v.apply_live(&[LiveRow {
            hash: p.info_hash.clone(),
            amount_left: 5,
            download_rate: 7,
            upload_rate: 9,
        }]);
        let r = v.row(1).unwrap();
        assert_eq!((r.down_rate, r.up_rate, r.t.amount_left), (7, 9, 5));
        // A fresh page keeps the last known rates for the same hash.
        let e = v.epoch;
        v.apply_page(e, 0, 3, 2, vec![t(0), p, t(2)]);
        assert_eq!(v.row(1).unwrap().down_rate, 7);
    }

    #[test]
    fn visible_hashes_are_bounded() {
        let mut v = ViewState::default();
        page(&mut v, 0, 200, 200);
        assert_eq!(v.visible_hashes(0, 199).len(), 128);
    }
}
