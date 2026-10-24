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

use std::collections::{hash_map, HashMap, HashSet};
use std::ops::Index;
use std::sync::atomic::{AtomicU64, Ordering};
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
    pub(super) torrent_chans: RuntimeTorrentMap<mpsc::Sender<TorrentCmd>>,
    pub(super) torrent_tasks: RuntimeTorrentMap<JoinHandle<()>>,
    pub(super) tier_controller: TierController<String>,
    pub(super) tier_last_active: RuntimeTorrentMap<Instant>,
    pub(super) pending_torrent_adds: RuntimeTorrentSet,
    pub(super) pending_torrent_deletes: RuntimeTorrentSet,
    pub(super) pending_torrent_promotions: RuntimeTorrentMap<Vec<TorrentPromotionAction>>,
    /// Exit notifications from supervised torrent actors, so reaping does not
    /// have to scan every active task handle on each engine command.
    pub(super) task_exits: TaskExits,
}

/// Queue of info hashes whose actor future has ended, including by panic.
pub(super) struct TaskExits {
    tx: mpsc::UnboundedSender<RuntimeTorrentExit>,
    rx: mpsc::UnboundedReceiver<RuntimeTorrentExit>,
    next_generation: AtomicU64,
    /// Current actor incarnation, owned alongside the exit queue it fences.
    current_generations: RuntimeTorrentMap<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RuntimeTorrentExit {
    pub(super) info_hash: RuntimeTorrentKey,
    pub(super) generation: u64,
}

/// Canonical runtime identity kept inline instead of as a heap-backed hex
/// string. Noncanonical keys remain exact strings for placeholder/test paths.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(super) enum RuntimeTorrentKey {
    Compact { bytes: [u8; 32], byte_len: u8 },
    Text(String),
}

impl RuntimeTorrentKey {
    pub(super) fn from_info_hash(info_hash: &str) -> Self {
        let byte_len = match info_hash.len() {
            40 => 20,
            64 => 32,
            _ => return Self::Text(info_hash.to_owned()),
        };
        let mut bytes = [0; 32];
        for (index, pair) in info_hash.as_bytes().as_chunks::<2>().0.iter().enumerate() {
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

pub(super) fn exit_matches_current_generation(
    notice: &RuntimeTorrentExit,
    current_generations: &RuntimeTorrentMap<u64>,
) -> bool {
    current_generations.get(&notice.info_hash.clone().into_info_hash()) == Some(&notice.generation)
}

/// Hash maps for engine-owned torrent state. Lookups keep accepting `&str`
/// at the actor/API boundary, while canonical v1/v2 keys live inline as hash
/// bytes and avoid one heap allocation per map entry.
pub(super) struct RuntimeTorrentMap<V> {
    entries: HashMap<RuntimeTorrentKey, V>,
}

impl<V> Default for RuntimeTorrentMap<V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<V> RuntimeTorrentMap<V> {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn insert<K: AsRef<str>>(&mut self, info_hash: K, value: V) -> Option<V> {
        self.entries
            .insert(RuntimeTorrentKey::from_info_hash(info_hash.as_ref()), value)
    }

    pub(super) fn get(&self, info_hash: &str) -> Option<&V> {
        self.entries
            .get(&RuntimeTorrentKey::from_info_hash(info_hash))
    }

    pub(super) fn get_mut(&mut self, info_hash: &str) -> Option<&mut V> {
        self.entries
            .get_mut(&RuntimeTorrentKey::from_info_hash(info_hash))
    }

    pub(super) fn contains_key(&self, info_hash: &str) -> bool {
        self.entries
            .contains_key(&RuntimeTorrentKey::from_info_hash(info_hash))
    }

    pub(super) fn remove(&mut self, info_hash: &str) -> Option<V> {
        self.entries
            .remove(&RuntimeTorrentKey::from_info_hash(info_hash))
    }

    pub(super) fn entry<K: AsRef<str>>(
        &mut self,
        info_hash: K,
    ) -> hash_map::Entry<'_, RuntimeTorrentKey, V> {
        self.entries
            .entry(RuntimeTorrentKey::from_info_hash(info_hash.as_ref()))
    }

    pub(super) fn values(&self) -> hash_map::Values<'_, RuntimeTorrentKey, V> {
        self.entries.values()
    }

    #[cfg(test)]
    pub(super) fn into_values(self) -> hash_map::IntoValues<RuntimeTorrentKey, V> {
        self.entries.into_values()
    }

    pub(super) fn iter(&self) -> hash_map::Iter<'_, RuntimeTorrentKey, V> {
        self.entries.iter()
    }

    pub(super) fn keys(&self) -> hash_map::Keys<'_, RuntimeTorrentKey, V> {
        self.entries.keys()
    }

    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    pub(super) fn into_entries(self) -> impl Iterator<Item = (String, V)> {
        self.entries
            .into_iter()
            .map(|(key, value)| (key.into_info_hash(), value))
    }
}

/// Set counterpart to [`RuntimeTorrentMap`] for engine-owned pending work.
#[derive(Default)]
pub(super) struct RuntimeTorrentSet {
    entries: HashSet<RuntimeTorrentKey>,
}

impl RuntimeTorrentSet {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn insert<K: AsRef<str>>(&mut self, info_hash: K) -> bool {
        self.entries
            .insert(RuntimeTorrentKey::from_info_hash(info_hash.as_ref()))
    }

    pub(super) fn contains(&self, info_hash: &str) -> bool {
        self.entries
            .contains(&RuntimeTorrentKey::from_info_hash(info_hash))
    }

    pub(super) fn remove(&mut self, info_hash: &str) -> bool {
        self.entries
            .remove(&RuntimeTorrentKey::from_info_hash(info_hash))
    }
}

impl<K, const N: usize> From<[K; N]> for RuntimeTorrentSet
where
    K: AsRef<str>,
{
    fn from(info_hashes: [K; N]) -> Self {
        let mut set = Self::new();
        for info_hash in info_hashes {
            set.insert(info_hash);
        }
        set
    }
}

impl<K, V, const N: usize> From<[(K, V); N]> for RuntimeTorrentMap<V>
where
    K: AsRef<str>,
{
    fn from(entries: [(K, V); N]) -> Self {
        let mut map = Self::new();
        for (info_hash, value) in entries {
            map.insert(info_hash, value);
        }
        map
    }
}

impl<V> Index<&String> for RuntimeTorrentMap<V> {
    type Output = V;

    fn index(&self, info_hash: &String) -> &Self::Output {
        self.get(info_hash)
            .expect("runtime torrent map key must exist")
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
        Self {
            tx,
            rx,
            next_generation: AtomicU64::new(0),
            current_generations: RuntimeTorrentMap::new(),
        }
    }
}

impl TaskExits {
    /// A guard to move into the actor's future; dropping it (normal exit or
    /// unwinding panic) queues the hash.
    #[allow(deprecated)] // AtomicU64::try_update is newer than the Rust 1.88 MSRV.
    pub(super) fn guard(&mut self, info_hash: &str) -> TaskExitGuard {
        let generation = self
            .next_generation
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
                current.checked_add(1)
            })
            .expect("torrent task exit generation exhausted")
            + 1;
        self.current_generations.insert(info_hash, generation);
        TaskExitGuard {
            notice: Some(RuntimeTorrentExit {
                info_hash: RuntimeTorrentKey::from_info_hash(info_hash),
                generation,
            }),
            tx: self.tx.clone(),
        }
    }

    /// Up to `limit` queued exits; O(exits), independent of active tasks.
    pub(super) fn drain(&mut self, limit: usize) -> Vec<RuntimeTorrentExit> {
        let mut exits = Vec::new();
        while exits.len() < limit {
            match self.rx.try_recv() {
                Ok(info_hash) => exits.push(info_hash),
                Err(_) => break,
            }
        }
        exits
    }

    pub(super) fn remove_current_generation(&mut self, info_hash: &str) {
        self.current_generations.remove(info_hash);
    }

    pub(super) fn is_current(&self, notice: &RuntimeTorrentExit) -> bool {
        exit_matches_current_generation(notice, &self.current_generations)
    }
}

pub(super) struct TaskExitGuard {
    notice: Option<RuntimeTorrentExit>,
    tx: mpsc::UnboundedSender<RuntimeTorrentExit>,
}

#[cfg(test)]
impl TaskExitGuard {
    pub(super) fn generation(&self) -> u64 {
        self.notice
            .as_ref()
            .expect("task exit guard must retain its notice until drop")
            .generation
    }
}

impl Drop for TaskExitGuard {
    fn drop(&mut self) {
        if let Some(notice) = self.notice.take() {
            let _ = self.tx.send(notice);
        }
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
            torrent_chans: RuntimeTorrentMap::new(),
            torrent_tasks: RuntimeTorrentMap::new(),
            tier_controller,
            tier_last_active: RuntimeTorrentMap::new(),
            pending_torrent_adds: RuntimeTorrentSet::new(),
            pending_torrent_deletes: RuntimeTorrentSet::new(),
            pending_torrent_promotions: RuntimeTorrentMap::new(),
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
    pub(super) collected_since: Option<Instant>,
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
            .map(|notice| notice.info_hash.into_info_hash())
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
    fn actor_replacements_receive_distinct_exit_generations() {
        let mut exits = TaskExits::default();
        let first = exits.guard("same-torrent");
        let first_generation = first.generation();
        drop(first);
        let second = exits.guard("same-torrent");
        let second_generation = second.generation();
        assert!(second_generation > first_generation);
        drop(second);

        let mut exits = exits;
        let notices = exits.drain(4);
        assert_eq!(notices.len(), 2);
        assert_eq!(notices[0].info_hash, notices[1].info_hash);
        assert_eq!(notices[0].generation, first_generation);
        assert_eq!(notices[1].generation, second_generation);
    }

    #[test]
    fn stale_actor_exit_cannot_match_its_replacement() {
        let mut exits = TaskExits::default();
        let old = exits.guard("same-torrent");
        let old_notice = RuntimeTorrentExit {
            info_hash: RuntimeTorrentKey::from_info_hash("same-torrent"),
            generation: old.generation(),
        };
        drop(old);
        let new = exits.guard("same-torrent");
        let new_notice = RuntimeTorrentExit {
            info_hash: RuntimeTorrentKey::from_info_hash("same-torrent"),
            generation: new.generation(),
        };
        assert!(!exits.is_current(&old_notice));
        assert!(exits.is_current(&new_notice));
    }

    #[test]
    fn compact_exit_ids_roundtrip_v1_v2_and_keep_non_hash_test_keys() {
        for hash in ["01".repeat(20), "ab".repeat(32)] {
            let id = RuntimeTorrentKey::from_info_hash(&hash);
            assert!(matches!(&id, RuntimeTorrentKey::Compact { .. }));
            assert_eq!(id.into_info_hash(), hash);
        }

        let text = "test-hash";
        let id = RuntimeTorrentKey::from_info_hash(text);
        assert!(matches!(&id, RuntimeTorrentKey::Text(_)));
        assert_eq!(id.into_info_hash(), text);
    }

    #[test]
    fn runtime_torrent_maps_keep_canonical_keys_inline_and_accept_hash_strings() {
        let mut map = RuntimeTorrentMap::new();
        for (hash, byte_len) in [("cd".repeat(20), 20), ("ab".repeat(32), 32)] {
            map.insert(hash.as_str(), byte_len);
            assert_eq!(map.get(&hash), Some(&byte_len));
            assert!(matches!(
                map.entries.keys().next(),
                Some(RuntimeTorrentKey::Compact {
                    byte_len: stored_len,
                    ..
                }) if usize::from(*stored_len) == byte_len
            ));
            assert_eq!(map.remove(&hash), Some(byte_len));
        }
        assert!(map.is_empty());

        let placeholder = "not-a-canonical-hash";
        map.insert(placeholder, 11);
        assert_eq!(map.get(placeholder), Some(&11));
        assert_eq!(map.get("NOT-A-CANONICAL-HASH"), None);
    }

    #[test]
    fn runtime_torrent_sets_keep_canonical_keys_inline_and_accept_hash_strings() {
        let mut set = RuntimeTorrentSet::new();
        let v1_hash = "cd".repeat(20);
        let v2_hash = "ab".repeat(32);
        assert!(set.insert(&v1_hash));
        assert!(set.insert(&v2_hash));
        assert!(!set.insert(&v1_hash));
        assert!(set.contains(&v1_hash));
        assert!(set.contains(&v2_hash));
        assert!(set
            .entries
            .iter()
            .any(|key| matches!(key, RuntimeTorrentKey::Compact { byte_len: 20, .. })));
        assert!(set
            .entries
            .iter()
            .any(|key| matches!(key, RuntimeTorrentKey::Compact { byte_len: 32, .. })));
        assert!(set.remove(&v1_hash));
        assert!(set.remove(&v2_hash));
        assert!(!set.contains(&v1_hash));

        let placeholder = "not-a-canonical-hash";
        assert!(set.insert(placeholder));
        assert!(set.contains(placeholder));
        assert!(!set.contains("NOT-A-CANONICAL-HASH"));
    }
}
