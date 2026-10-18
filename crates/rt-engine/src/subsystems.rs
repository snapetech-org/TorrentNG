//! Explicit ownership boundaries for the engine actor.
//!
//! `Engine` is still the single ordering authority, but it should not also
//! present every runtime concern as a flat bag of fields. These two state
//! objects make the ownership boundary visible:
//!
//! * `EngineRuntimeState` owns torrent actors, tier admission, and lifecycle
//!   bookkeeping that is mutated by the actor.
//! * `EngineSubsystems` owns detachable services and process-wide budgets.
//!
//! Keeping these objects free of cross-layer callbacks makes the next step—a
//! separately supervised actor or injected dependency—mechanical rather than
//! another rewrite of the command protocol.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::command::{EngineStats, TorrentPromotionAction};
use crate::dht_task::DhtCommand;
use crate::network_budget::GlobalNetworkBudget;
use crate::storage_jobs::StorageJobDispatcher;
use crate::tier::TierController;
use crate::torrent_task::TorrentCmd;
use rt_metrics::ResourceGovernor;

pub(super) struct EngineRuntimeState {
    pub(super) torrent_chans: HashMap<String, mpsc::Sender<TorrentCmd>>,
    pub(super) torrent_tasks: HashMap<String, JoinHandle<()>>,
    pub(super) tier_controller: TierController<String>,
    pub(super) tier_last_active: HashMap<String, Instant>,
    pub(super) pending_torrent_adds: HashSet<String>,
    pub(super) pending_torrent_deletes: HashSet<String>,
    pub(super) pending_torrent_promotions: HashMap<String, Vec<TorrentPromotionAction>>,
    /// Exit notifications from supervised torrent actors, so reaping does not
    /// have to scan every active task handle on each engine command.
    pub(super) task_exits: TaskExits,
}

/// Queue of info hashes whose actor future has ended, including by panic.
pub(super) struct TaskExits {
    tx: mpsc::UnboundedSender<TaskExitId>,
    rx: mpsc::UnboundedReceiver<TaskExitId>,
}

/// Completion identity kept inline while an actor runs and while its exit is
/// queued. Runtime APIs still use hex strings, but task guards need not retain
/// another heap-backed copy of a canonical v1 or v2 hash.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum TaskExitId {
    Compact { bytes: [u8; 32], byte_len: u8 },
    Text(String),
}

impl TaskExitId {
    fn from_info_hash(info_hash: &str) -> Self {
        let byte_len = match info_hash.len() {
            40 => 20,
            64 => 32,
            _ => return Self::Text(info_hash.to_owned()),
        };
        let mut bytes = [0; 32];
        for (index, pair) in info_hash.as_bytes().chunks_exact(2).enumerate() {
            let Some(high) = decode_hex_nibble(pair[0]) else {
                return Self::Text(info_hash.to_owned());
            };
            let Some(low) = decode_hex_nibble(pair[1]) else {
                return Self::Text(info_hash.to_owned());
            };
            bytes[index] = (high << 4) | low;
        }
        Self::Compact { bytes, byte_len }
    }

    pub(super) fn into_info_hash(self) -> String {
        match self {
            Self::Text(info_hash) => info_hash,
            Self::Compact { bytes, byte_len } => {
                let digits = b"0123456789abcdef";
                let mut hex = String::with_capacity(usize::from(byte_len) * 2);
                for byte in bytes.iter().take(usize::from(byte_len)) {
                    let byte = *byte;
                    hex.push(char::from(digits[usize::from(byte >> 4)]));
                    hex.push(char::from(digits[usize::from(byte & 0x0f)]));
                }
                hex
            }
        }
    }
}

fn decode_hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

impl Default for TaskExits {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx }
    }
}

impl TaskExits {
    /// A guard to move into the actor's future; dropping it (normal exit or
    /// unwinding panic) queues the hash.
    pub(super) fn guard(&self, info_hash: &str) -> TaskExitGuard {
        TaskExitGuard {
            info_hash: TaskExitId::from_info_hash(info_hash),
            tx: self.tx.clone(),
        }
    }

    /// Up to `limit` queued exits; O(exits), independent of active tasks.
    pub(super) fn drain(&mut self, limit: usize) -> Vec<TaskExitId> {
        let mut exits = Vec::new();
        while exits.len() < limit {
            match self.rx.try_recv() {
                Ok(info_hash) => exits.push(info_hash),
                Err(_) => break,
            }
        }
        exits
    }
}

pub(super) struct TaskExitGuard {
    info_hash: TaskExitId,
    tx: mpsc::UnboundedSender<TaskExitId>,
}

impl Drop for TaskExitGuard {
    fn drop(&mut self) {
        let info_hash = std::mem::replace(&mut self.info_hash, TaskExitId::Text(String::new()));
        let _ = self.tx.send(info_hash);
    }
}

/// Spawn `future` so its exit is reported through `guard`.
pub(super) fn spawn_supervised<F>(guard: TaskExitGuard, future: F) -> JoinHandle<()>
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    tokio::spawn(async move {
        let _guard = guard;
        future.await;
    })
}

impl EngineRuntimeState {
    pub(super) fn new(tier_controller: TierController<String>) -> Self {
        Self {
            torrent_chans: HashMap::new(),
            torrent_tasks: HashMap::new(),
            tier_controller,
            tier_last_active: HashMap::new(),
            pending_torrent_adds: HashSet::new(),
            pending_torrent_deletes: HashSet::new(),
            pending_torrent_promotions: HashMap::new(),
            task_exits: TaskExits::default(),
        }
    }
}

pub(super) struct EngineSubsystems {
    pub(super) dht_tx: Option<mpsc::Sender<DhtCommand>>,
    /// The DHT sender alone cannot cancel a task that is inside bootstrap
    /// DNS resolution. Keep the production join handle so shutdown can
    /// force-reap a task after its graceful command path times out.
    #[cfg(not(test))]
    pub(super) dht_task: Option<JoinHandle<()>>,
    pub(super) resources: ResourceGovernor,
    pub(super) network_budget: GlobalNetworkBudget,
    pub(super) storage_jobs: StorageJobDispatcher,
    pub(super) stats_cache: Option<EngineStatsCache>,
    /// Crash-safety settings and run verdict shared with every torrent task.
    /// Starts inert; `Engine::start` installs the real runtime.
    pub(super) crash_safety: std::sync::Arc<crate::crash_safety::CrashSafetyRuntime>,
}

pub(super) struct EngineStatsCache {
    pub(super) generated_at: Instant,
    pub(super) stats: EngineStats,
    /// The refresh is detached from the actor. Keep its start time so a
    /// dropped completion cannot permanently suppress future refreshes.
    pub(super) refresh_started_at: Option<Instant>,
}

impl EngineSubsystems {
    pub(super) fn new(
        dht_tx: Option<mpsc::Sender<DhtCommand>>,
        resources: ResourceGovernor,
        network_budget: GlobalNetworkBudget,
        storage_jobs: StorageJobDispatcher,
    ) -> Self {
        Self {
            dht_tx,
            #[cfg(not(test))]
            dht_task: None,
            resources,
            network_budget,
            storage_jobs,
            stats_cache: None,
            crash_safety: crate::crash_safety::CrashSafetyRuntime::inert(),
        }
    }
}

#[cfg(test)]
mod task_exit_tests {
    use super::*;

    #[tokio::test]
    async fn supervised_exit_is_reported_for_normal_return_and_panic() {
        let mut exits = TaskExits::default();
        let ok = spawn_supervised(exits.guard("ok"), async {});
        let boom = spawn_supervised(exits.guard("boom"), async {
            panic!("simulated torrent actor panic");
        });
        let _ = ok.await;
        assert!(boom.await.is_err());

        let mut reported = exits
            .drain(10)
            .into_iter()
            .map(TaskExitId::into_info_hash)
            .collect::<Vec<_>>();
        reported.sort();
        assert_eq!(reported, ["boom", "ok"]);
        assert!(exits.drain(10).is_empty(), "each exit is reported once");
    }

    #[tokio::test]
    async fn drain_is_bounded() {
        let mut exits = TaskExits::default();
        for index in 0..5 {
            drop(exits.guard(&format!("t{index}")));
        }
        assert_eq!(exits.drain(2).len(), 2);
        assert_eq!(exits.drain(10).len(), 3);
    }

    #[test]
    fn compact_exit_ids_roundtrip_v1_v2_and_keep_non_hash_test_keys() {
        for hash in ["01".repeat(20), "ab".repeat(32)] {
            let id = TaskExitId::from_info_hash(&hash);
            assert!(matches!(&id, TaskExitId::Compact { .. }));
            assert_eq!(id.into_info_hash(), hash);
        }

        let text = "test-hash";
        let id = TaskExitId::from_info_hash(text);
        assert!(matches!(&id, TaskExitId::Text(_)));
        assert_eq!(id.into_info_hash(), text);
    }
}
