//! Wire types and derived display state.
//!
//! `TorrentSummary`/`TorrentDetail` come straight from `rt-api-model`, so the
//! desktop client and the daemon cannot drift apart on the core contract.

use std::collections::HashMap;

use serde::Deserialize;

pub use rt_api_model::{FileInfo, TorrentDetail, TorrentSummary};

/// `GET /api/v1/torrents` response.
#[derive(Debug, Clone, Deserialize)]
pub struct ListResponse {
    #[serde(default)]
    pub snapshot: u64,
    #[serde(default)]
    pub total: usize,
    #[serde(default)]
    pub torrents: Vec<TorrentSummary>,
}

/// One `torrent_delta` SSE payload.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Delta {
    #[serde(default)]
    pub cursor: u64,
    #[serde(default)]
    pub snapshot: bool,
    #[serde(default)]
    pub snapshot_complete: bool,
    #[serde(default)]
    pub torrents: Vec<TorrentSummary>,
    #[serde(default)]
    pub removed: Vec<String>,
}

/// Fast-changing counters from `GET /api/v1/torrents/live`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct LiveRow {
    pub hash: String,
    #[serde(default)]
    pub amount_left: u64,
    #[serde(default)]
    pub download_rate: i64,
    #[serde(default)]
    pub upload_rate: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LiveResponse {
    #[serde(default)]
    pub torrents: Vec<LiveRow>,
}

/// `GET /api/v1/transfer/info`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct TransferInfo {
    #[serde(default)]
    pub dl_info_speed: i64,
    #[serde(default)]
    pub up_info_speed: i64,
    #[serde(default)]
    pub dl_rate_limit: i64,
    #[serde(default)]
    pub up_rate_limit: i64,
    #[serde(default)]
    pub dht_nodes: i64,
    #[serde(default)]
    pub connection_status: String,
}

/// `GET /api/v1/sidebar-facets` (only the status counts are used).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Facets {
    #[serde(default)]
    pub status: HashMap<String, u64>,
}

/// Coarse lifecycle bucket shown in the Status column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Downloading,
    Seeding,
    Stopped,
    Checking,
    Error,
    Queued,
    Metadata,
    /// Download finished but held until the data is durable (crash safety).
    Finalizing,
}

impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Status::Downloading => "Downloading",
            Status::Seeding => "Seeding",
            Status::Stopped => "Stopped",
            Status::Checking => "Checking",
            Status::Error => "Error",
            Status::Queued => "Queued",
            Status::Metadata => "Metadata",
            Status::Finalizing => "Finalizing",
        }
    }
}

pub fn is_complete(t: &TorrentSummary) -> bool {
    t.amount_left <= 0 && t.total_length > 0 && !t.finalizing
}

pub fn status(t: &TorrentSummary) -> Status {
    if t.finalizing {
        return Status::Finalizing;
    }
    match t.state.as_str() {
        "error" => Status::Error,
        "stopped" | "paused" => Status::Stopped,
        "checking" => Status::Checking,
        "metadata_pending" | "metadata" => Status::Metadata,
        "queued" => Status::Queued,
        "seeding" => Status::Seeding,
        "downloading" if is_complete(t) => Status::Seeding,
        "downloading" => Status::Downloading,
        _ => Status::Queued,
    }
}

/// Fraction complete in `0.0..=1.0` from live payload accounting.
pub fn progress(t: &TorrentSummary) -> f32 {
    if t.total_length <= 0 {
        return 0.0;
    }
    let done = (t.total_length - t.amount_left.max(0)).clamp(0, t.total_length);
    (done as f64 / t.total_length as f64) as f32
}

/// A table row: the summary plus the overlaid live counters.
#[derive(Debug, Clone)]
pub struct Row {
    pub t: TorrentSummary,
    pub down_rate: i64,
    pub up_rate: i64,
}

impl Row {
    pub fn new(t: TorrentSummary) -> Self {
        Self {
            t,
            down_rate: 0,
            up_rate: 0,
        }
    }

    pub fn eta_secs(&self) -> Option<u64> {
        if self.t.amount_left > 0 && self.down_rate > 0 {
            Some((self.t.amount_left / self.down_rate) as u64)
        } else {
            None
        }
    }
}

/// Server-side sortable columns (`sort=` values accepted by the daemon).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    #[default]
    Added,
    Name,
    Size,
    Progress,
    Ratio,
}

impl SortKey {
    pub fn param(self) -> &'static str {
        match self {
            SortKey::Added => "added",
            SortKey::Name => "name",
            SortKey::Size => "size",
            SortKey::Progress => "progress",
            SortKey::Ratio => "ratio",
        }
    }
}

/// Everything that selects which torrents the table shows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ViewSpec {
    pub filter: String,
    /// Server status filter (`downloading`, `seeding`, …); empty = all.
    pub status: String,
    pub sort: SortKey,
    pub descending: bool,
}

impl ViewSpec {
    pub fn query(
        &self,
        offset: usize,
        limit: usize,
        snapshot: Option<u64>,
    ) -> Vec<(&'static str, String)> {
        let mut q = vec![
            ("sort", self.sort.param().to_owned()),
            (
                "dir",
                if self.descending { "desc" } else { "asc" }.to_owned(),
            ),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        if !self.filter.trim().is_empty() {
            q.push(("filter", self.filter.trim().to_owned()));
        }
        if !self.status.is_empty() {
            q.push(("status", self.status.clone()));
        }
        if let Some(s) = snapshot {
            q.push(("snapshot", s.to_string()));
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn summary(state: &str, total: i64, left: i64) -> TorrentSummary {
        TorrentSummary {
            info_hash: "a".repeat(40),
            name: "n".into(),
            state: state.into(),
            total_length: total,
            downloaded: total - left,
            amount_left: left,
            uploaded: 0,
            ratio: 0.0,
            save_path: "/d".into(),
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

    #[test]
    fn status_buckets() {
        assert_eq!(status(&summary("seeding", 10, 0)), Status::Seeding);
        assert_eq!(status(&summary("downloading", 10, 4)), Status::Downloading);
        assert_eq!(status(&summary("downloading", 10, 0)), Status::Seeding);
        assert_eq!(status(&summary("paused", 10, 4)), Status::Stopped);
        assert_eq!(status(&summary("metadata_pending", 0, 0)), Status::Metadata);
        let mut t = summary("downloading", 10, 1);
        t.finalizing = true;
        assert_eq!(status(&t), Status::Finalizing);
        assert!(!is_complete(&t));
    }

    #[test]
    fn progress_uses_amount_left_not_cumulative_downloaded() {
        let mut t = summary("downloading", 1000, 250);
        t.downloaded = 99_999; // cumulative transfer after a recheck must not matter
        assert!((progress(&t) - 0.75).abs() < 1e-6);
        assert_eq!(progress(&summary("queued", 0, 0)), 0.0);
    }

    #[test]
    fn eta_requires_rate_and_remaining() {
        let mut r = Row::new(summary("downloading", 1000, 500));
        assert_eq!(r.eta_secs(), None);
        r.down_rate = 100;
        assert_eq!(r.eta_secs(), Some(5));
    }

    /// Contract test: whatever the daemon serializes must deserialize here and
    /// in the list/delta envelopes.
    #[test]
    fn daemon_json_roundtrips_into_envelopes() {
        let t = summary("seeding", 10, 0);
        let list = serde_json::json!({"snapshot": 7, "total": 1, "torrents": [t]});
        let parsed: ListResponse = serde_json::from_value(list).unwrap();
        assert_eq!(parsed.snapshot, 7);
        assert_eq!(parsed.torrents[0], t);

        let delta = serde_json::json!({
            "seq": 3, "cursor": 9, "snapshot": false, "snapshot_complete": false,
            "torrents": [t], "removed": ["b".repeat(40)]
        });
        let d: Delta = serde_json::from_value(delta).unwrap();
        assert_eq!(d.cursor, 9);
        assert_eq!(d.removed.len(), 1);
        assert_eq!(d.torrents.len(), 1);
    }

    #[test]
    fn query_includes_only_set_filters() {
        let mut v = ViewSpec::default();
        let q = v.query(100, 200, None);
        assert!(q.iter().any(|(k, val)| *k == "offset" && val == "100"));
        assert!(!q.iter().any(|(k, _)| *k == "filter" || *k == "status"));
        v.filter = " ubuntu ".into();
        v.status = "seeding".into();
        v.descending = true;
        let q = v.query(0, 50, Some(4));
        assert!(q.contains(&("filter", "ubuntu".to_owned())));
        assert!(q.contains(&("status", "seeding".to_owned())));
        assert!(q.contains(&("dir", "desc".to_owned())));
        assert!(q.contains(&("snapshot", "4".to_owned())));
    }
}
