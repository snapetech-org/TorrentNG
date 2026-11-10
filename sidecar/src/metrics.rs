use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

/// Global metric counters. Cheap atomics — no external crate needed for Phase 2.
/// Replace with prometheus crate when the metric set grows enough to warrant it.
#[derive(Default)]
pub struct Metrics {
    pub torrents_total: AtomicI64,
    pub torrents_seeding: AtomicI64,
    pub torrents_downloading: AtomicI64,
    pub torrents_stopped: AtomicI64,
    pub torrents_errored: AtomicI64,
    pub peers_connected: AtomicI64,
    pub api_requests_total: AtomicU64,
    pub sync_cycles_total: AtomicU64,
    pub sync_errors_total: AtomicU64,
    /// Result of the most recently completed backend sync: 0 unknown,
    /// 1 connected, 2 unreachable. Health endpoints read this instead of
    /// adding a second SCGI probe while rTorrent is under storage pressure.
    pub sync_health_state: AtomicU64,
    pub sync_last_completed_at_unix_secs: AtomicU64,
}

pub type SharedMetrics = Arc<Metrics>;

impl Metrics {
    pub fn new() -> SharedMetrics {
        Arc::new(Self::default())
    }

    pub fn record_sync_result(&self, healthy: bool) {
        let completed_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        self.sync_last_completed_at_unix_secs
            .store(completed_at, Ordering::Relaxed);
        self.sync_health_state
            .store(if healthy { 1 } else { 2 }, Ordering::Release);
    }

    pub fn sync_is_healthy_and_fresh(&self, max_age: std::time::Duration) -> bool {
        if self.sync_health_state.load(Ordering::Acquire) != 1 {
            return false;
        }
        let last_completed = self
            .sync_last_completed_at_unix_secs
            .load(Ordering::Relaxed);
        if last_completed == 0 {
            return false;
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        now.saturating_sub(last_completed) <= max_age.as_secs()
    }

    /// Render Prometheus text exposition format.
    pub fn render(&self) -> String {
        let mut out = String::with_capacity(1024);

        gauge(
            &mut out,
            "torrentng_torrents_total",
            "Total torrents in session",
            self.torrents_total.load(Ordering::Relaxed),
        );
        gauge(
            &mut out,
            "torrentng_torrents_seeding",
            "Torrents currently seeding",
            self.torrents_seeding.load(Ordering::Relaxed),
        );
        gauge(
            &mut out,
            "torrentng_torrents_downloading",
            "Torrents currently downloading",
            self.torrents_downloading.load(Ordering::Relaxed),
        );
        gauge(
            &mut out,
            "torrentng_torrents_stopped",
            "Torrents stopped",
            self.torrents_stopped.load(Ordering::Relaxed),
        );
        gauge(
            &mut out,
            "torrentng_torrents_errored",
            "Torrents in error state",
            self.torrents_errored.load(Ordering::Relaxed),
        );
        gauge(
            &mut out,
            "torrentng_peers_connected",
            "Connected peers across all torrents",
            self.peers_connected.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "torrentng_api_requests_total",
            "Total API requests served",
            self.api_requests_total.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "torrentng_sync_cycles_total",
            "Total compatible-client backend sync cycles completed",
            self.sync_cycles_total.load(Ordering::Relaxed),
        );
        counter(
            &mut out,
            "torrentng_sync_errors_total",
            "Total compatible-client backend sync cycle errors",
            self.sync_errors_total.load(Ordering::Relaxed),
        );
        out
    }
}

fn gauge(out: &mut String, name: &str, help: &str, value: i64) {
    out.push_str(&format!(
        "# HELP {name} {help}\n# TYPE {name} gauge\n{name} {value}\n"
    ));
}

fn counter(out: &mut String, name: &str, help: &str, value: u64) {
    out.push_str(&format!(
        "# HELP {name} {help}\n# TYPE {name} counter\n{name} {value}\n"
    ));
}
