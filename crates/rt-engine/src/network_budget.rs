use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, Notify, Semaphore};
// tokio's Instant, not std's: it respects the paused/mockable clock that
// `#[tokio::test(start_paused = true)]` and `tokio::time::advance()` use.
// Using std::time::Instant here would make the refill calculation below see
// real wall-clock time regardless of virtual time advances, which live-locks
// a paused-clock test into an infinite near-zero-wait retry loop.
use rt_metrics::{ResourceGovernor, ResourceGovernorConfig};
use rt_utp::{UtpEndpoint, UtpRuntime};
use tokio::time::Instant;

use crate::transfer::coords::{ConnectionId, TorrentKey};
use crate::transfer::rate::{Grant, RatePeerId, RateTree, CHANGE_ONLY_WAKE};
use crate::transfer::shared_upload::SharedUploadService;
use crate::transfer::upload_scheduler::UploadReadLimits;

/// Matches the v1 adaptive per-peer request-window ceiling.
const SOURCE_REQUESTS_PER_PEER_SLOT: usize = 128;
/// Keep one maximum-size peer window available even with a one-peer config.
const MIN_GLOBAL_SOURCE_REQUESTS: usize = 128;
/// Provides 512 MiB worth of slots at the standard 16 KiB block size; the
/// PeerBuffer governor separately enforces byte admission for actual lengths.
const MAX_GLOBAL_SOURCE_REQUESTS: usize = 32_768;
/// Matches the existing per-torrent adaptive request headroom.
const MAX_TORRENT_SOURCE_REQUESTS: usize = 2_048;
/// A source that stopped retrying cannot leave a stale round-robin turn at the
/// head of either admission queue.
const SOURCE_TURN_IDLE_TTL: Duration = Duration::from_secs(5);

/// Process-wide network admission and traffic budgets.
///
/// The peer semaphore counts live peer/metadata connections. A separate
/// shared request semaphore bounds outstanding peer and webseed payload work
/// at both process and torrent scope. When sources contend, one demand turn is
/// served at a time per source within its torrent and per torrent across the
/// process; an uncontended request stays on the semaphore-only fast path.
/// One shared upload service registers each live peer's BEP 6 obligations and
/// its process-wide read/byte budget as a single lifetime. Rate limiters count
/// payload bytes at the engine boundary; protocol framing and TCP/IP overhead
/// are outside the current accounting contract.
#[derive(Clone)]
pub(crate) struct GlobalNetworkBudget {
    peer_slots: Arc<Semaphore>,
    source_admission: Arc<SourceAdmissionService>,
    upload_service: SharedUploadService,
    download_tree: Arc<HierarchicalRateService>,
    upload_tree: Arc<HierarchicalRateService>,
    compatibility_download: Arc<HierarchicalRateLimiter>,
    compatibility_upload: Arc<HierarchicalRateLimiter>,
    listen_port: Arc<AtomicU16>,
    listener_rebind_tx: mpsc::Sender<PeerListenerRebindRequest>,
    listener_rebind_rx: Arc<Mutex<Option<mpsc::Receiver<PeerListenerRebindRequest>>>>,
    utp_runtime: UtpRuntime,
}

pub(crate) struct PeerListenerRebindRequest {
    pub(crate) port: u16,
    pub(crate) reply: oneshot::Sender<Result<(), String>>,
}

impl GlobalNetworkBudget {
    pub(crate) fn new(
        max_peers: usize,
        download_limit_bytes_per_sec: Option<u64>,
        upload_limit_bytes_per_sec: Option<u64>,
    ) -> Self {
        Self::new_with_resources(
            max_peers,
            download_limit_bytes_per_sec,
            upload_limit_bytes_per_sec,
            ResourceGovernor::new(ResourceGovernorConfig::default()),
        )
    }

    pub(crate) fn new_with_resources(
        max_peers: usize,
        download_limit_bytes_per_sec: Option<u64>,
        upload_limit_bytes_per_sec: Option<u64>,
        resources: ResourceGovernor,
    ) -> Self {
        let (listener_rebind_tx, listener_rebind_rx) = mpsc::channel(4);
        let upload_origin = Instant::now();
        let global_source_requests = max_peers
            .saturating_mul(SOURCE_REQUESTS_PER_PEER_SLOT)
            .clamp(MIN_GLOBAL_SOURCE_REQUESTS, MAX_GLOBAL_SOURCE_REQUESTS);
        let download_tree = Arc::new(HierarchicalRateService {
            tree: Mutex::new(RateTree::new(download_limit_bytes_per_sec, 0)),
            origin: upload_origin,
            changed: RateLimitCancellation::default(),
            demand_changes: Mutex::new(HashMap::new()),
            next_torrent: AtomicU64::new(1),
            next_demand: AtomicU64::new(1),
        });
        let upload_tree = Arc::new(HierarchicalRateService {
            tree: Mutex::new(RateTree::new(upload_limit_bytes_per_sec, 0)),
            origin: upload_origin,
            changed: RateLimitCancellation::default(),
            demand_changes: Mutex::new(HashMap::new()),
            next_torrent: AtomicU64::new(1),
            next_demand: AtomicU64::new(1),
        });
        // The reference actors still expose the former process-global rate
        // limiter API. Give each direction one stable synthetic peer in the
        // same rate tree so rollback traffic shares the candidate's global
        // ceiling instead of creating a second, independent token bucket.
        let compatibility_torrent = upload_tree.new_torrent_key();
        download_tree.register_torrent(compatibility_torrent, None);
        upload_tree.register_torrent(compatibility_torrent, None);
        Self {
            peer_slots: Arc::new(Semaphore::new(max_peers.max(1))),
            source_admission: Arc::new(SourceAdmissionService::new(
                global_source_requests,
                global_source_requests.min(MAX_TORRENT_SOURCE_REQUESTS),
            )),
            upload_service: SharedUploadService::new(
                max_peers.max(1),
                UploadReadLimits {
                    max_connections: max_peers.max(1),
                    ..UploadReadLimits::default()
                },
                resources,
            ),
            download_tree: Arc::clone(&download_tree),
            upload_tree: Arc::clone(&upload_tree),
            compatibility_download: Arc::new(HierarchicalRateLimiter {
                service: download_tree,
                torrent: compatibility_torrent,
                next_connection: AtomicU64::new(1),
            }),
            compatibility_upload: Arc::new(HierarchicalRateLimiter {
                service: upload_tree,
                torrent: compatibility_torrent,
                next_connection: AtomicU64::new(1),
            }),
            listen_port: Arc::new(AtomicU16::new(0)),
            listener_rebind_tx,
            listener_rebind_rx: Arc::new(Mutex::new(Some(listener_rebind_rx))),
            utp_runtime: UtpRuntime::default(),
        }
    }

    pub(crate) fn listen_port(&self) -> u16 {
        self.listen_port.load(Ordering::Acquire)
    }

    pub(crate) fn set_listen_port(&self, port: u16) {
        self.listen_port.store(port, Ordering::Release);
    }

    pub(crate) fn listen_port_shared(&self) -> Arc<AtomicU16> {
        Arc::clone(&self.listen_port)
    }

    pub(crate) fn take_listener_rebind_receiver(
        &self,
    ) -> Option<mpsc::Receiver<PeerListenerRebindRequest>> {
        self.listener_rebind_rx
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }

    pub(crate) async fn rebind_peer_listener(&self, port: u16) -> Result<(), String> {
        if port == 0 {
            return Err("listen_port must be between 1 and 65535".to_owned());
        }
        let (reply, response) = oneshot::channel();
        tokio::time::timeout(
            Duration::from_secs(5),
            self.listener_rebind_tx
                .send(PeerListenerRebindRequest { port, reply }),
        )
        .await
        .map_err(|_| "peer listener rebind queue timed out".to_owned())?
        .map_err(|_| "peer listener is not running".to_owned())?;
        tokio::time::timeout(Duration::from_secs(10), response)
            .await
            .map_err(|_| "peer listener rebind timed out".to_owned())?
            .map_err(|_| "peer listener dropped rebind reply".to_owned())?
    }

    #[allow(dead_code)]
    pub(crate) fn unlimited() -> Self {
        Self::new(1_000_000, None, None)
    }

    pub(crate) fn try_acquire_peer(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, tokio::sync::TryAcquireError> {
        Arc::clone(&self.peer_slots).try_acquire_owned()
    }

    pub(crate) fn download(&self) -> Arc<HierarchicalRateLimiter> {
        Arc::clone(&self.compatibility_download)
    }

    pub(crate) fn upload(&self) -> Arc<HierarchicalRateLimiter> {
        Arc::clone(&self.compatibility_upload)
    }

    pub(crate) fn download_available_credit_bytes_for(&self, peer_id: RatePeerId) -> u64 {
        self.download_tree.available_credit_bytes(peer_id)
    }

    pub(crate) fn utp_runtime(&self) -> UtpRuntime {
        self.utp_runtime.clone()
    }

    pub(crate) fn upload_service(&self) -> SharedUploadService {
        self.upload_service.clone()
    }

    pub(crate) async fn install_utp_endpoint(
        &self,
        endpoint: UtpEndpoint,
    ) -> Result<(), rt_utp::UtpError> {
        self.utp_runtime.install_endpoint(endpoint).await
    }

    pub(crate) async fn replace_utp_endpoint(
        &self,
        endpoint: UtpEndpoint,
    ) -> Result<(), rt_utp::UtpError> {
        self.utp_runtime.replace_endpoint(endpoint).await
    }

    pub(crate) async fn shutdown_utp(&self) -> Result<(), rt_utp::UtpError> {
        self.utp_runtime.shutdown().await
    }

    pub(crate) fn set_download_limit(&self, limit: Option<u64>) {
        self.download_tree.set_global_limit(limit);
    }

    pub(crate) fn download_limit_changes(&self) -> RateLimitCancellation {
        self.download_tree.limit_changes()
    }

    /// Wake only one torrent when its fairness waiters may have become
    /// eligible after another peer makes progress or retires demand.
    pub(crate) fn download_demand_changes(&self, torrent: TorrentKey) -> RateLimitCancellation {
        self.download_tree.demand_changes_for(torrent)
    }

    pub(crate) fn set_upload_limit(&self, limit: Option<u64>) {
        self.upload_tree.set_global_limit(limit);
    }

    pub(crate) fn new_transfer_torrent_key(&self) -> TorrentKey {
        self.upload_tree.new_torrent_key()
    }

    pub(crate) fn register_upload_torrent(&self, key: TorrentKey, limit: Option<u64>) {
        self.upload_tree.register_torrent(key, limit);
    }

    pub(crate) fn register_download_torrent(&self, key: TorrentKey, limit: Option<u64>) {
        self.download_tree.register_torrent(key, limit);
        self.source_admission.register_torrent(key);
    }

    pub(crate) fn set_upload_torrent_limit(&self, key: TorrentKey, limit: Option<u64>) {
        self.upload_tree.set_torrent_limit(key, limit);
    }

    pub(crate) fn set_download_torrent_limit(&self, key: TorrentKey, limit: Option<u64>) {
        self.download_tree.set_torrent_limit(key, limit);
    }

    pub(crate) fn remove_transfer_torrent(&self, key: TorrentKey) {
        self.source_admission.remove_torrent(key);
        self.upload_tree.remove_torrent(key);
        self.download_tree.remove_torrent(key);
    }

    pub(crate) fn register_transfer_peer(&self, peer_id: RatePeerId) {
        self.upload_tree.register_peer(peer_id);
        self.download_tree.register_peer(peer_id);
    }

    pub(crate) fn remove_transfer_peer(&self, peer_id: RatePeerId) {
        self.source_admission.remove_source(peer_id);
        self.upload_tree.remove_peer(peer_id);
        self.download_tree.remove_peer(peer_id);
    }

    /// Retire a synchronous download refill that no longer has runnable
    /// demand. The peer registration and any grants already attached to
    /// requests remain live.
    pub(crate) fn retire_download_demand(&self, peer_id: RatePeerId) {
        self.download_tree.retire_sync_demand(peer_id);
    }

    pub(crate) async fn acquire_upload(
        &self,
        peer_id: RatePeerId,
        bytes: u64,
        cancellation: &RateLimitCancellation,
        generation: u64,
    ) -> anyhow::Result<Option<RateAdmissionGrant>> {
        self.upload_tree
            .acquire(peer_id, bytes, cancellation, generation)
            .await
    }

    pub(crate) async fn acquire_download(
        &self,
        peer_id: RatePeerId,
        bytes: u64,
        cancellation: &RateLimitCancellation,
        generation: u64,
    ) -> anyhow::Result<Option<RateAdmissionGrant>> {
        self.download_tree
            .acquire(peer_id, bytes, cancellation, generation)
            .await
    }

    pub(crate) fn try_acquire_download(
        &self,
        peer_id: RatePeerId,
        bytes: u64,
    ) -> anyhow::Result<RateAdmissionAttempt> {
        self.download_tree.try_acquire(peer_id, bytes)
    }

    /// Admit one outstanding payload request through the same process and
    /// torrent caps regardless of whether its source is a peer or webseed.
    /// The returned permit stays with the request until response, rejection,
    /// expiry, cancellation, or failure.
    pub(crate) fn try_acquire_source_request(
        &self,
        peer_id: RatePeerId,
    ) -> Option<SourceRequestPermit> {
        self.source_admission_for_torrent(peer_id.torrent)?
            .try_acquire(peer_id)
    }

    /// Clone a torrent-scoped admission handle once per request-fill batch.
    /// The hot path avoids the process-wide torrent-map mutex; it uses atomic
    /// semaphore admission when uncontended and short FIFO turn locks only
    /// while sources are competing for capacity.
    pub(crate) fn source_admission_for_torrent(
        &self,
        key: TorrentKey,
    ) -> Option<SourceAdmissionTorrent> {
        self.source_admission.for_torrent(key)
    }
}

/// Compatibility view used by the reversible reference actors. It reserves
/// and settles against the same hierarchical process rate tree as candidate
/// peers; it is not a second global token bucket.
pub(crate) struct HierarchicalRateLimiter {
    service: Arc<HierarchicalRateService>,
    torrent: TorrentKey,
    next_connection: AtomicU64,
}

impl HierarchicalRateLimiter {
    fn register_operation(&self) -> RatePeerId {
        let mut current = self.next_connection.load(Ordering::Relaxed);
        loop {
            let next = current
                .checked_add(1)
                .expect("compatibility rate connection identity space exhausted");
            match self.next_connection.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => {
                    let peer_id = RatePeerId::new(
                        self.torrent,
                        ConnectionId {
                            slot: current as u32,
                            generation: (current >> 32) as u32,
                        },
                    );
                    self.service.register_peer(peer_id);
                    return peer_id;
                }
                Err(observed) => current = observed,
            }
        }
    }

    pub(crate) async fn acquire(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let cancellation = RateLimitCancellation::default();
        let generation = cancellation.generation();
        let _ = self
            .acquire_or_cancelled(bytes, &cancellation, generation)
            .await;
    }

    pub(crate) async fn acquire_or_cancelled(
        &self,
        bytes: u64,
        cancellation: &RateLimitCancellation,
        generation: u64,
    ) -> bool {
        if bytes == 0 {
            return cancellation.generation() == generation;
        }
        let peer_id = self.register_operation();
        let result = self
            .service
            .acquire(peer_id, bytes, cancellation, generation)
            .await;
        let admitted = match result {
            Ok(Some(grant)) if cancellation.generation() == generation => {
                grant.settle(bytes);
                true
            }
            Ok(Some(grant)) => {
                drop(grant);
                false
            }
            Ok(None) | Err(_) => false,
        };
        self.service.remove_peer(peer_id);
        admitted
    }
}

const LEGACY_RATE_BURST_BYTES: u64 = 64 * 1024;

#[derive(Debug)]
struct LegacyRateState {
    limit_bytes_per_sec: Option<u64>,
    tokens: u64,
    updated_at: Instant,
}

/// Retained only for the rollback actor's per-torrent upload ceiling. Global
/// limits use `HierarchicalRateLimiter` so both actor routes share one process
/// rate tree.
pub(crate) struct SharedRateLimiter {
    state: Mutex<LegacyRateState>,
    limit_changed: RateLimitCancellation,
}

impl SharedRateLimiter {
    pub(crate) fn new(limit_bytes_per_sec: Option<u64>) -> Self {
        let limit = limit_bytes_per_sec.filter(|limit| *limit > 0);
        let tokens = limit
            .map(|limit| limit.max(LEGACY_RATE_BURST_BYTES))
            .unwrap_or(u64::MAX);
        Self {
            state: Mutex::new(LegacyRateState {
                limit_bytes_per_sec: limit,
                tokens,
                updated_at: Instant::now(),
            }),
            limit_changed: RateLimitCancellation::default(),
        }
    }

    pub(crate) fn set_limit(&self, limit_bytes_per_sec: Option<u64>) {
        let limit = limit_bytes_per_sec.filter(|limit| *limit > 0);
        let mut state = self.lock_state();
        if state.limit_bytes_per_sec == limit {
            return;
        }
        Self::refill(&mut state, Instant::now());
        state.tokens = match (state.limit_bytes_per_sec, limit) {
            (None, Some(_)) => 0,
            (_, None) => u64::MAX,
            (Some(_), Some(new_limit)) => state.tokens.min(new_limit.max(LEGACY_RATE_BURST_BYTES)),
        };
        state.limit_bytes_per_sec = limit;
        state.updated_at = Instant::now();
        drop(state);
        self.limit_changed.cancel();
    }

    pub(crate) async fn acquire_or_cancelled(
        &self,
        bytes: u64,
        cancellation: &RateLimitCancellation,
        generation: u64,
    ) -> bool {
        if bytes == 0 {
            return cancellation.generation() == generation;
        }
        let mut remaining = bytes;
        let mut limit_generation = self.limit_changed.generation();
        loop {
            if cancellation.generation() != generation {
                return false;
            }
            let wait = {
                let mut state = self.lock_state();
                let Some(limit) = state.limit_bytes_per_sec else {
                    return true;
                };
                let now = Instant::now();
                Self::refill(&mut state, now);
                let chunk = remaining.min(limit.max(LEGACY_RATE_BURST_BYTES));
                if state.tokens >= chunk {
                    state.tokens -= chunk;
                    remaining -= chunk;
                    if remaining == 0 {
                        return cancellation.generation() == generation;
                    }
                    Duration::ZERO
                } else {
                    token_refill_delay(chunk - state.tokens, limit)
                }
            };
            if wait.is_zero() {
                tokio::select! {
                    _ = tokio::task::yield_now() => {}
                    _ = cancellation.wait_for_change(generation) => return false,
                    _ = self.limit_changed.wait_for_change(limit_generation) => {
                        limit_generation = self.limit_changed.generation();
                    }
                }
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = cancellation.wait_for_change(generation) => return false,
                    _ = self.limit_changed.wait_for_change(limit_generation) => {
                        limit_generation = self.limit_changed.generation();
                    }
                }
            }
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, LegacyRateState> {
        match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => {
                self.state.clear_poison();
                poisoned.into_inner()
            }
        }
    }

    fn refill(state: &mut LegacyRateState, now: Instant) {
        let Some(limit) = state.limit_bytes_per_sec else {
            state.tokens = u64::MAX;
            state.updated_at = now;
            return;
        };
        let elapsed = now.saturating_duration_since(state.updated_at);
        let refill = elapsed.as_nanos().saturating_mul(u128::from(limit)) / 1_000_000_000;
        let refill = refill.min(u128::from(u64::MAX)) as u64;
        state.tokens = state
            .tokens
            .saturating_add(refill)
            .min(limit.max(LEGACY_RATE_BURST_BYTES));
        state.updated_at = now;
    }
}

fn token_refill_delay(missing_bytes: u64, limit_bytes_per_sec: u64) -> Duration {
    if missing_bytes == 0 {
        return Duration::ZERO;
    }
    let rate = u128::from(limit_bytes_per_sec.max(1));
    let wait_nanos = u128::from(missing_bytes)
        .saturating_mul(1_000_000_000)
        .saturating_add(rate.saturating_sub(1))
        / rate;
    let seconds = wait_nanos / 1_000_000_000;
    if seconds > u128::from(u64::MAX) {
        return Duration::MAX;
    }
    Duration::new(seconds as u64, (wait_nanos % 1_000_000_000) as u32)
}

struct SourceAdmissionService {
    process_slots: Arc<Semaphore>,
    process_turns: Arc<Mutex<SourceTurnQueue<TorrentKey>>>,
    process_turn_pending: Arc<AtomicBool>,
    per_torrent_limit: usize,
    torrent_slots: Mutex<HashMap<TorrentKey, Arc<TorrentSourceSlots>>>,
}

struct TorrentSourceSlots {
    key: TorrentKey,
    slots: Arc<Semaphore>,
    active: AtomicBool,
    source_turns: Mutex<SourceTurnQueue<RatePeerId>>,
    source_turn_pending: AtomicBool,
}

struct SourceTurnQueue<T> {
    pending: VecDeque<SourceTurnEntry<T>>,
}

struct SourceTurnEntry<T> {
    key: T,
    requested_at: Instant,
    claimed: bool,
}

impl<T: Copy + Eq> SourceTurnQueue<T> {
    fn new() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }

    fn request(&mut self, key: T, now: Instant) {
        self.pending.retain(|entry| {
            now.saturating_duration_since(entry.requested_at) <= SOURCE_TURN_IDLE_TTL
        });
        if let Some(entry) = self.pending.iter_mut().find(|entry| entry.key == key) {
            entry.requested_at = now;
        } else {
            self.pending.push_back(SourceTurnEntry {
                key,
                requested_at: now,
                claimed: false,
            });
        }
    }

    fn is_turn(&self, key: T) -> bool {
        self.pending.front().is_some_and(|entry| entry.key == key)
    }

    fn claim_turn(&mut self, key: T) -> bool {
        let Some(entry) = self.pending.front_mut().filter(|entry| entry.key == key) else {
            return false;
        };
        if entry.claimed {
            return false;
        }
        entry.claimed = true;
        true
    }

    fn release_claim(&mut self, key: T) {
        if let Some(entry) = self.pending.front_mut().filter(|entry| entry.key == key) {
            entry.claimed = false;
        }
    }

    fn complete_turn(&mut self, key: T) {
        if self.is_turn(key) {
            self.pending.pop_front();
        }
    }

    fn remove(&mut self, key: T) {
        self.pending.retain(|entry| entry.key != key);
    }

    fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }
}

fn lock_source_turns<T>(turns: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    match turns.lock() {
        Ok(turns) => turns,
        Err(poisoned) => {
            turns.clear_poison();
            poisoned.into_inner()
        }
    }
}

impl SourceAdmissionService {
    fn new(process_limit: usize, per_torrent_limit: usize) -> Self {
        Self {
            process_slots: Arc::new(Semaphore::new(process_limit.max(1))),
            process_turns: Arc::new(Mutex::new(SourceTurnQueue::new())),
            process_turn_pending: Arc::new(AtomicBool::new(false)),
            per_torrent_limit: per_torrent_limit.max(1),
            torrent_slots: Mutex::new(HashMap::new()),
        }
    }

    fn register_torrent(&self, key: TorrentKey) {
        self.lock_torrents().entry(key).or_insert_with(|| {
            Arc::new(TorrentSourceSlots {
                key,
                slots: Arc::new(Semaphore::new(self.per_torrent_limit)),
                active: AtomicBool::new(true),
                source_turns: Mutex::new(SourceTurnQueue::new()),
                source_turn_pending: AtomicBool::new(false),
            })
        });
    }

    fn remove_torrent(&self, key: TorrentKey) {
        let mut torrents = self.lock_torrents();
        if let Some(torrent) = torrents.get(&key) {
            torrent.active.store(false, Ordering::Release);
            lock_source_turns(&torrent.source_turns).pending.clear();
            torrent.source_turn_pending.store(false, Ordering::Release);
        }
        torrents.remove(&key);
        let mut process_turns = lock_source_turns(&self.process_turns);
        process_turns.remove(key);
        self.process_turn_pending
            .store(!process_turns.is_empty(), Ordering::Release);
    }

    fn remove_source(&self, peer_id: RatePeerId) {
        if let Some(torrent) = self.lock_torrents().get(&peer_id.torrent).cloned() {
            let mut source_turns = lock_source_turns(&torrent.source_turns);
            source_turns.remove(peer_id);
            if source_turns.is_empty() {
                torrent.source_turn_pending.store(false, Ordering::Release);
                let mut process_turns = lock_source_turns(&self.process_turns);
                process_turns.remove(peer_id.torrent);
                self.process_turn_pending
                    .store(!process_turns.is_empty(), Ordering::Release);
            } else {
                torrent.source_turn_pending.store(true, Ordering::Release);
            }
        }
    }

    fn for_torrent(&self, key: TorrentKey) -> Option<SourceAdmissionTorrent> {
        let torrent_slots = self.lock_torrents().get(&key).cloned()?;
        Some(SourceAdmissionTorrent {
            process_slots: Arc::clone(&self.process_slots),
            process_turns: Arc::clone(&self.process_turns),
            process_turn_pending: Arc::clone(&self.process_turn_pending),
            torrent_slots,
        })
    }

    fn lock_torrents(&self) -> MutexGuard<'_, HashMap<TorrentKey, Arc<TorrentSourceSlots>>> {
        match self.torrent_slots.lock() {
            Ok(slots) => slots,
            Err(poisoned) => {
                let slots = poisoned.into_inner();
                self.torrent_slots.clear_poison();
                slots
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct SourceAdmissionTorrent {
    process_slots: Arc<Semaphore>,
    process_turns: Arc<Mutex<SourceTurnQueue<TorrentKey>>>,
    process_turn_pending: Arc<AtomicBool>,
    torrent_slots: Arc<TorrentSourceSlots>,
}

impl SourceAdmissionTorrent {
    pub(crate) fn try_acquire(&self, peer_id: RatePeerId) -> Option<SourceRequestPermit> {
        if peer_id.torrent != self.torrent_slots.key {
            return None;
        }
        if !self.torrent_slots.active.load(Ordering::Acquire) {
            return None;
        }
        if !self
            .torrent_slots
            .source_turn_pending
            .load(Ordering::Acquire)
            && !self.process_turn_pending.load(Ordering::Acquire)
        {
            if let Ok(torrent) = self.torrent_slots.slots.clone().try_acquire_owned() {
                if let Ok(process) = self.process_slots.clone().try_acquire_owned() {
                    if self.torrent_slots.active.load(Ordering::Acquire) {
                        return Some(SourceRequestPermit {
                            _process: process,
                            _torrent: torrent,
                        });
                    }
                    return None;
                }
            }
        }
        {
            let mut source_turns = lock_source_turns(&self.torrent_slots.source_turns);
            source_turns.request(peer_id, Instant::now());
            self.torrent_slots
                .source_turn_pending
                .store(!source_turns.is_empty(), Ordering::Release);
            if !source_turns.claim_turn(peer_id) {
                return None;
            }
        }
        let Some(torrent) = self.torrent_slots.slots.clone().try_acquire_owned().ok() else {
            lock_source_turns(&self.torrent_slots.source_turns).release_claim(peer_id);
            return None;
        };
        let process = {
            let mut process_turns = lock_source_turns(&self.process_turns);
            process_turns.request(peer_id.torrent, Instant::now());
            self.process_turn_pending
                .store(!process_turns.is_empty(), Ordering::Release);
            if !process_turns.claim_turn(peer_id.torrent) {
                drop(process_turns);
                lock_source_turns(&self.torrent_slots.source_turns).release_claim(peer_id);
                return None;
            }
            let Some(process) = self.process_slots.clone().try_acquire_owned().ok() else {
                process_turns.release_claim(peer_id.torrent);
                drop(process_turns);
                lock_source_turns(&self.torrent_slots.source_turns).release_claim(peer_id);
                return None;
            };
            if !self.torrent_slots.active.load(Ordering::Acquire) {
                process_turns.release_claim(peer_id.torrent);
                drop(process_turns);
                lock_source_turns(&self.torrent_slots.source_turns).release_claim(peer_id);
                return None;
            }
            process_turns.complete_turn(peer_id.torrent);
            self.process_turn_pending
                .store(!process_turns.is_empty(), Ordering::Release);
            process
        };
        {
            let mut source_turns = lock_source_turns(&self.torrent_slots.source_turns);
            source_turns.complete_turn(peer_id);
            self.torrent_slots
                .source_turn_pending
                .store(!source_turns.is_empty(), Ordering::Release);
        }
        Some(SourceRequestPermit {
            _process: process,
            _torrent: torrent,
        })
    }
}

/// A bounded source request owns one process slot and one torrent slot.
/// Peer and webseed adapters both keep this guard beside the ledger request.
pub(crate) struct SourceRequestPermit {
    _process: tokio::sync::OwnedSemaphorePermit,
    _torrent: tokio::sync::OwnedSemaphorePermit,
}

impl std::fmt::Debug for SourceRequestPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SourceRequestPermit")
    }
}

pub(crate) enum RateAdmissionAttempt {
    Granted(RateAdmissionGrant),
    Wait(Duration),
    /// Fairness is blocked on peer progress or demand retirement, with no
    /// useful clock deadline to poll.
    WaitForChange,
}

/// Live Tokio adapter for the deterministic transfer `RateTree`. The lock is
/// held only while applying one scheduler event; deadlines, cancellation,
/// and network I/O all happen after it is released.
struct HierarchicalRateService {
    tree: Mutex<RateTree>,
    origin: Instant,
    changed: RateLimitCancellation,
    demand_changes: Mutex<HashMap<TorrentKey, RateLimitCancellation>>,
    next_torrent: AtomicU64,
    next_demand: AtomicU64,
}

impl HierarchicalRateService {
    fn now(&self) -> u64 {
        self.origin.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
    }

    fn lock_tree(&self) -> MutexGuard<'_, RateTree> {
        match self.tree.lock() {
            Ok(tree) => tree,
            Err(poisoned) => {
                let tree = poisoned.into_inner();
                self.tree.clear_poison();
                tree
            }
        }
    }

    fn new_torrent_key(&self) -> TorrentKey {
        let mut current = self.next_torrent.load(Ordering::Relaxed);
        loop {
            let next = current
                .checked_add(1)
                .expect("transfer rate torrent identity space exhausted");
            match self.next_torrent.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return TorrentKey(current),
                Err(observed) => current = observed,
            }
        }
    }

    fn new_demand_id(&self) -> u64 {
        let mut current = self.next_demand.load(Ordering::Relaxed);
        loop {
            let next = current
                .checked_add(1)
                .expect("transfer rate demand identity space exhausted");
            match self.next_demand.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return current,
                Err(observed) => current = observed,
            }
        }
    }

    fn demand_changes_for(&self, torrent: TorrentKey) -> RateLimitCancellation {
        let mut changes = self
            .demand_changes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        changes.entry(torrent).or_default().clone()
    }

    fn notify_demand_change(&self, torrent: TorrentKey) {
        let changes = self
            .demand_changes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&torrent)
            .cloned();
        if let Some(changes) = changes {
            changes.cancel();
        }
    }

    fn remove_demand_changes(&self, torrent: TorrentKey) {
        let changes = self
            .demand_changes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&torrent);
        if let Some(changes) = changes {
            changes.cancel();
        }
    }

    fn retire_sync_demand(&self, peer_id: RatePeerId) {
        if self.lock_tree().retire_sync_demand(peer_id) {
            self.notify_demand_change(peer_id.torrent);
        }
    }

    fn retire_demand(&self, peer_id: RatePeerId, demand_id: u64) {
        if self.lock_tree().retire_demand(peer_id, demand_id) {
            self.notify_demand_change(peer_id.torrent);
        }
    }

    fn set_global_limit(&self, limit: Option<u64>) {
        self.lock_tree().set_global_limit(limit, self.now());
        self.changed.cancel();
    }

    fn limit_changes(&self) -> RateLimitCancellation {
        self.changed.clone()
    }

    fn available_credit_bytes(&self, peer_id: RatePeerId) -> u64 {
        self.lock_tree().available_credit_bytes(peer_id, self.now())
    }

    fn register_torrent(&self, key: TorrentKey, limit: Option<u64>) {
        self.demand_changes_for(key);
        self.lock_tree().add_torrent(key, limit, self.now());
        self.changed.cancel();
    }

    fn set_torrent_limit(&self, key: TorrentKey, limit: Option<u64>) {
        self.lock_tree().set_torrent_limit(key, limit, self.now());
        self.changed.cancel();
    }

    fn remove_torrent(&self, key: TorrentKey) {
        self.lock_tree().remove_torrent(key);
        self.remove_demand_changes(key);
        self.changed.cancel();
    }

    fn register_peer(&self, peer_id: RatePeerId) {
        self.lock_tree().add_peer(peer_id, 1);
        self.notify_demand_change(peer_id.torrent);
        self.changed.cancel();
    }

    fn remove_peer(&self, peer_id: RatePeerId) {
        self.lock_tree().remove_peer(peer_id);
        self.notify_demand_change(peer_id.torrent);
        self.changed.cancel();
    }

    fn settle_and_refund_unused(
        &self,
        peer_id: RatePeerId,
        grant_id: u64,
        transferred: u64,
        unused: u64,
    ) {
        let refunded = {
            let mut tree = self.lock_tree();
            tree.settle_grant(peer_id, grant_id, transferred);
            if unused == 0 {
                0
            } else {
                tree.refund_grant(peer_id, grant_id, unused, self.now())
            }
        };
        if refunded > 0 {
            self.changed.cancel();
        }
    }

    fn refund(&self, peer_id: RatePeerId, grant_id: u64, bytes: u64) {
        if self
            .lock_tree()
            .refund_grant(peer_id, grant_id, bytes, self.now())
            > 0
        {
            self.changed.cancel();
        }
    }

    fn try_acquire(
        self: &Arc<Self>,
        peer_id: RatePeerId,
        bytes: u64,
    ) -> anyhow::Result<RateAdmissionAttempt> {
        if bytes == 0 {
            anyhow::bail!("zero-byte payload cannot consume rate admission");
        }
        let now = self.now();
        let (grant, grant_id, wake_fairness_waiters) = {
            let mut tree = self.lock_tree();
            let (grant, grant_id) = tree.try_grant_sync_tracked(peer_id, bytes, now);
            let wake_fairness_waiters =
                grant == Grant::Granted && tree.has_change_waiters(peer_id.torrent);
            (grant, grant_id, wake_fairness_waiters)
        };
        if wake_fairness_waiters {
            self.notify_demand_change(peer_id.torrent);
        }
        match grant {
            Grant::Granted => Ok(RateAdmissionAttempt::Granted(RateAdmissionGrant {
                scheduler: Arc::clone(self),
                peer_id,
                grant_id: grant_id.expect("granted payload has a reservation identity"),
                bytes,
                active: true,
            })),
            Grant::Wait { until } if until == CHANGE_ONLY_WAKE => {
                Ok(RateAdmissionAttempt::WaitForChange)
            }
            Grant::Wait { until } => Ok(RateAdmissionAttempt::Wait(Duration::from_nanos(
                until.saturating_sub(self.now()),
            ))),
            Grant::Unknown => anyhow::bail!(
                "rate peer is not registered: torrent={} connection={}.{}",
                peer_id.torrent.0,
                peer_id.connection.slot,
                peer_id.connection.generation
            ),
        }
    }

    async fn acquire(
        self: &Arc<Self>,
        peer_id: RatePeerId,
        bytes: u64,
        cancellation: &RateLimitCancellation,
        generation: u64,
    ) -> anyhow::Result<Option<RateAdmissionGrant>> {
        if bytes == 0 {
            anyhow::bail!("zero-byte payload cannot consume rate admission");
        }
        let demand_changes = self.demand_changes_for(peer_id.torrent);
        let demand_id = self.new_demand_id();
        let _demand = RateDemandGuard {
            scheduler: Arc::clone(self),
            peer_id,
            demand_id,
        };
        loop {
            if cancellation.generation() != generation {
                return Ok(None);
            }
            let changed_generation = self.changed.generation();
            let demand_generation = demand_changes.generation();
            let now = self.now();
            let (grant, grant_id, wake_fairness_waiters) = {
                let mut tree = self.lock_tree();
                let (grant, grant_id) =
                    tree.try_grant_demand_tracked(peer_id, demand_id, bytes, now);
                let wake_fairness_waiters =
                    grant == Grant::Granted && tree.has_change_waiters(peer_id.torrent);
                (grant, grant_id, wake_fairness_waiters)
            };
            if wake_fairness_waiters {
                self.notify_demand_change(peer_id.torrent);
            }
            match grant {
                Grant::Granted => {
                    let grant = RateAdmissionGrant {
                        scheduler: Arc::clone(self),
                        peer_id,
                        grant_id: grant_id.expect("granted payload has a reservation identity"),
                        bytes,
                        active: true,
                    };
                    if cancellation.generation() != generation {
                        drop(grant);
                        return Ok(None);
                    }
                    return Ok(Some(grant));
                }
                Grant::Unknown => anyhow::bail!(
                    "rate peer is not registered: torrent={} connection={}.{}",
                    peer_id.torrent.0,
                    peer_id.connection.slot,
                    peer_id.connection.generation
                ),
                Grant::Wait { until } => {
                    if until == CHANGE_ONLY_WAKE {
                        tokio::select! {
                            _ = self.changed.wait_for_change(changed_generation) => {}
                            _ = demand_changes.wait_for_change(demand_generation) => {}
                            _ = cancellation.wait_for_change(generation) => return Ok(None),
                        }
                    } else {
                        let delay = Duration::from_nanos(until.saturating_sub(self.now()));
                        tokio::select! {
                            _ = tokio::time::sleep(delay) => {}
                            _ = self.changed.wait_for_change(changed_generation) => {}
                            _ = demand_changes.wait_for_change(demand_generation) => {}
                            _ = cancellation.wait_for_change(generation) => return Ok(None),
                        }
                    }
                }
            }
        }
    }
}

/// A pending rate demand is owned by one asynchronous request. Dropping or
/// cancelling that request clears only its own waiter, so another request
/// from the same connection remains represented in the fairness queue.
struct RateDemandGuard {
    scheduler: Arc<HierarchicalRateService>,
    peer_id: RatePeerId,
    demand_id: u64,
}

impl Drop for RateDemandGuard {
    fn drop(&mut self) {
        self.scheduler.retire_demand(self.peer_id, self.demand_id);
    }
}

/// Shared upload/download rate credit. Settlement records the bytes that
/// crossed the transfer boundary; dropping an unfinished grant refunds its
/// reserved credit.
pub(crate) struct RateAdmissionGrant {
    scheduler: Arc<HierarchicalRateService>,
    peer_id: RatePeerId,
    grant_id: u64,
    bytes: u64,
    active: bool,
}

impl std::fmt::Debug for RateAdmissionGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RateAdmissionGrant")
            .field("peer_id", &self.peer_id)
            .field("grant_id", &self.grant_id)
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl RateAdmissionGrant {
    pub(crate) fn settle(mut self, transferred: u64) {
        let transferred = transferred.min(self.bytes);
        self.scheduler.settle_and_refund_unused(
            self.peer_id,
            self.grant_id,
            transferred,
            self.bytes - transferred,
        );
        self.active = false;
    }
}

impl Drop for RateAdmissionGrant {
    fn drop(&mut self) {
        if self.active {
            self.scheduler
                .refund(self.peer_id, self.grant_id, self.bytes);
        }
    }
}

/// Out-of-band wakeup shared by a peer owner and its peer loop. Control
/// messages use a bounded mailbox, so a peer stuck in a rate wait must have a
/// separate cancellation path for choke, limit, and shutdown transitions.
#[derive(Clone, Debug, Default)]
pub(crate) struct RateLimitCancellation {
    generation: Arc<AtomicU64>,
    notify: Arc<Notify>,
}

impl RateLimitCancellation {
    pub(crate) fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    pub(crate) fn cancel(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.notify.notify_waiters();
    }

    /// Wait until the supplied generation is no longer current. Enabling the
    /// notification before the second generation check closes the race where
    /// a cancellation happens between checking the atomic and awaiting.
    pub(crate) async fn wait_for_change(&self, generation: u64) {
        if self.generation() != generation {
            return;
        }
        let notified = self.notify.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        if self.generation() == generation {
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rate_peer(torrent: TorrentKey, slot: u32) -> RatePeerId {
        RatePeerId::new(
            torrent,
            ConnectionId {
                slot,
                generation: 1,
            },
        )
    }

    #[test]
    fn peer_slots_are_shared_across_clones() {
        let budget = GlobalNetworkBudget::new(1, None, None);
        let clone = budget.clone();
        let permit = budget.try_acquire_peer().unwrap();
        assert!(clone.try_acquire_peer().is_err());
        drop(permit);
        assert!(clone.try_acquire_peer().is_ok());
    }

    #[test]
    fn peer_and_webseed_requests_share_torrent_and_process_admission() {
        let admission = Arc::new(SourceAdmissionService::new(3, 2));
        let first_torrent = TorrentKey(1);
        let second_torrent = TorrentKey(2);
        admission.register_torrent(first_torrent);
        admission.register_torrent(second_torrent);

        // These handles model a peer request and the actor's synthetic
        // webseed connection. Both enter through the same admission method.
        let first = admission.for_torrent(first_torrent).unwrap();
        let second = admission.for_torrent(second_torrent).unwrap();
        let peer = rate_peer(first_torrent, 0);
        let webseed = rate_peer(first_torrent, 1);
        let second_peer = rate_peer(second_torrent, 0);
        let peer_request = first.try_acquire(peer).unwrap();
        let webseed_request = first.try_acquire(webseed).unwrap();
        assert!(first.try_acquire(peer).is_none());

        let other_torrent_request = second.try_acquire(second_peer).unwrap();
        assert!(second.try_acquire(second_peer).is_none());

        drop(webseed_request);
        let reopened = second
            .try_acquire(second_peer)
            .expect("the torrent already waiting at the process turn gets the freed slot");
        assert!(first.try_acquire(peer).is_none());
        drop(reopened);
        drop(peer_request);
        drop(other_torrent_request);
    }

    #[test]
    fn removing_a_torrent_revokes_cached_source_admission_handles() {
        let budget = GlobalNetworkBudget::new(2, None, None);
        let torrent = budget.new_transfer_torrent_key();
        budget.register_download_torrent(torrent, None);
        let cached = budget
            .source_admission_for_torrent(torrent)
            .expect("registered torrent has a source-admission handle");

        budget.remove_transfer_torrent(torrent);

        assert!(cached.try_acquire(rate_peer(torrent, 0)).is_none());
        budget.register_download_torrent(torrent, None);
        assert!(budget
            .source_admission_for_torrent(torrent)
            .expect("re-registered torrent gets a fresh handle")
            .try_acquire(rate_peer(torrent, 0))
            .is_some());
    }

    #[test]
    fn global_network_budget_registers_and_releases_source_request_slots() {
        let budget = GlobalNetworkBudget::new(1, None, None);
        let first_torrent = budget.new_transfer_torrent_key();
        let second_torrent = budget.new_transfer_torrent_key();
        budget.register_download_torrent(first_torrent, None);
        budget.register_download_torrent(second_torrent, None);

        let mut first_torrent_requests = (0..MIN_GLOBAL_SOURCE_REQUESTS)
            .map(|_| {
                budget
                    .try_acquire_source_request(rate_peer(first_torrent, 0))
                    .expect("registered source request should fit")
            })
            .collect::<Vec<_>>();
        assert!(budget
            .try_acquire_source_request(rate_peer(second_torrent, 0))
            .is_none());
        assert!(budget
            .try_acquire_source_request(rate_peer(first_torrent, 0))
            .is_none());

        drop(first_torrent_requests.pop());
        let other_torrent_request = budget
            .try_acquire_source_request(rate_peer(second_torrent, 0))
            .expect("queued torrent receives the next released process slot");
        drop(other_torrent_request);
        assert!(budget
            .try_acquire_source_request(rate_peer(first_torrent, 0))
            .is_some());
    }

    #[test]
    fn source_turn_queue_rotates_demand_and_expires_idle_sources() {
        let now = Instant::now();
        let torrent = TorrentKey(7);
        let a = rate_peer(torrent, 1);
        let b = rate_peer(torrent, 2);
        let mut turns = SourceTurnQueue::new();

        turns.request(a, now);
        turns.request(b, now);
        assert!(turns.is_turn(a));
        assert!(turns.claim_turn(a));
        assert!(!turns.claim_turn(a));
        turns.release_claim(a);
        turns.complete_turn(a);
        assert!(turns.is_turn(b));
        turns.request(a, now + Duration::from_millis(1));
        turns.complete_turn(b);
        assert!(turns.is_turn(a));

        turns.request(b, now + SOURCE_TURN_IDLE_TTL + Duration::from_secs(1));
        assert!(turns.is_turn(b));
        assert!(!turns.pending.iter().any(|entry| entry.key == a));
    }

    #[test]
    fn global_download_limit_change_signal_tracks_runtime_updates() {
        let budget = GlobalNetworkBudget::new(1, Some(1), None);
        let changes = budget.download_limit_changes();
        let generation = changes.generation();

        budget.set_download_limit(Some(2));

        assert_ne!(changes.generation(), generation);
    }

    #[tokio::test(start_paused = true)]
    async fn hierarchical_download_tree_shares_global_credit_and_wakes_on_update() {
        let budget = GlobalNetworkBudget::new(4, Some(1), None);
        let first_torrent = budget.new_transfer_torrent_key();
        let second_torrent = budget.new_transfer_torrent_key();
        budget.register_download_torrent(first_torrent, None);
        budget.register_download_torrent(second_torrent, None);
        let first_peer = rate_peer(first_torrent, 0);
        let second_peer = rate_peer(second_torrent, 0);
        budget.register_transfer_peer(first_peer);
        budget.register_transfer_peer(second_peer);

        let control = RateLimitCancellation::default();
        let first_grant = budget
            .acquire_download(first_peer, 32 * 1024, &control, control.generation())
            .await
            .unwrap()
            .unwrap();
        first_grant.settle(32 * 1024);

        let waiting_budget = budget.clone();
        let waiting_control = control.clone();
        let generation = control.generation();
        let waiter = tokio::spawn(async move {
            waiting_budget
                .acquire_download(second_peer, 1, &waiting_control, generation)
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        budget.set_download_limit(None);
        waiter.await.unwrap().unwrap().unwrap().settle(1);
    }

    #[tokio::test(start_paused = true)]
    async fn reference_rate_compatibility_spends_the_candidate_global_bucket() {
        let budget = GlobalNetworkBudget::new(2, Some(1), None);
        let torrent = budget.new_transfer_torrent_key();
        budget.register_upload_torrent(torrent, None);
        budget.register_download_torrent(torrent, None);
        let peer = rate_peer(torrent, 7);
        budget.register_transfer_peer(peer);

        let compatibility = budget.download();
        let cancellation = RateLimitCancellation::default();
        assert!(
            compatibility
                .acquire_or_cancelled(32 * 1024, &cancellation, cancellation.generation(),)
                .await
        );

        let waiting_budget = budget.clone();
        let waiter = tokio::spawn(async move {
            let cancellation = RateLimitCancellation::default();
            waiting_budget
                .acquire_download(peer, 1, &cancellation, cancellation.generation())
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        budget.set_download_limit(None);
        waiter.await.unwrap().unwrap().unwrap().settle(1);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelled_reference_rate_wait_removes_its_temporary_peer() {
        let budget = GlobalNetworkBudget::new(1, Some(1), None);
        let compatibility = budget.download();
        let cancellation = RateLimitCancellation::default();
        let generation = cancellation.generation();
        assert!(
            compatibility
                .acquire_or_cancelled(32 * 1024, &cancellation, generation)
                .await
        );

        let waiting_limiter = Arc::clone(&compatibility);
        let waiting_cancellation = cancellation.clone();
        let waiter = tokio::spawn(async move {
            waiting_limiter
                .acquire_or_cancelled(1, &waiting_cancellation, generation)
                .await
        });
        tokio::task::yield_now().await;
        assert_eq!(compatibility.service.lock_tree().waiting_peers(8).len(), 1);

        cancellation.cancel();
        assert!(!waiter.await.unwrap());
        assert!(compatibility
            .service
            .lock_tree()
            .waiting_peers(8)
            .is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn retiring_sync_download_demand_unblocks_live_peer_without_removal() {
        const BLOCK: u64 = 16 * 1024;
        let budget = GlobalNetworkBudget::new(2, Some(32 * 1024 * 1024), None);
        let torrent = budget.new_transfer_torrent_key();
        budget.register_download_torrent(torrent, Some(32 * 1024 * 1024));
        let active = rate_peer(torrent, 0);
        let parked = rate_peer(torrent, 1);
        budget.register_transfer_peer(active);
        budget.register_transfer_peer(parked);

        while let RateAdmissionAttempt::Granted(grant) =
            budget.try_acquire_download(active, BLOCK).unwrap()
        {
            grant.settle(BLOCK);
        }
        assert!(matches!(
            budget.try_acquire_download(parked, BLOCK).unwrap(),
            RateAdmissionAttempt::Wait(_) | RateAdmissionAttempt::WaitForChange
        ));

        let demand_changes = budget.download_demand_changes(torrent);
        let demand_generation = demand_changes.generation();
        budget.set_download_limit(None);
        budget.set_download_torrent_limit(torrent, None);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(
            matches!(
                budget.try_acquire_download(active, BLOCK).unwrap(),
                RateAdmissionAttempt::WaitForChange
            ),
            "a stale live demand should be present until its owner retires it"
        );

        budget.retire_download_demand(parked);
        assert_ne!(demand_changes.generation(), demand_generation);
        let RateAdmissionAttempt::Granted(grant) =
            budget.try_acquire_download(active, BLOCK).unwrap()
        else {
            panic!("retiring parked demand should restore work-conserving service");
        };
        grant.settle(BLOCK);
        let RateAdmissionAttempt::Granted(grant) = budget.try_acquire_download(parked, 1).unwrap()
        else {
            panic!("retiring pending demand must keep the peer registration live");
        };
        grant.settle(1);
    }

    #[tokio::test(start_paused = true)]
    async fn cancelling_one_async_rate_demand_keeps_its_sibling_waiter() {
        let budget = GlobalNetworkBudget::new(3, Some(1), None);
        let torrent = budget.new_transfer_torrent_key();
        budget.register_download_torrent(torrent, None);
        let spender = rate_peer(torrent, 0);
        let waiting = rate_peer(torrent, 1);
        budget.register_transfer_peer(spender);
        budget.register_transfer_peer(waiting);

        let spender_control = RateLimitCancellation::default();
        budget
            .acquire_download(
                spender,
                32 * 1024,
                &spender_control,
                spender_control.generation(),
            )
            .await
            .unwrap()
            .unwrap()
            .settle(32 * 1024);

        let first_control = RateLimitCancellation::default();
        let first_task_control = first_control.clone();
        let first_generation = first_control.generation();
        let first_budget = budget.clone();
        let first_waiter = tokio::spawn(async move {
            first_budget
                .acquire_download(waiting, 1, &first_task_control, first_generation)
                .await
        });
        let second_control = RateLimitCancellation::default();
        let second_generation = second_control.generation();
        let second_budget = budget.clone();
        let second_waiter = tokio::spawn(async move {
            second_budget
                .acquire_download(waiting, 1, &second_control, second_generation)
                .await
        });
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert_eq!(
            budget.download_tree.lock_tree().waiting_peers(8),
            vec![waiting]
        );

        first_control.cancel();
        assert!(first_waiter.await.unwrap().unwrap().is_none());
        assert_eq!(
            budget.download_tree.lock_tree().waiting_peers(8),
            vec![waiting],
            "one cancelled request must not erase its sibling's demand"
        );

        budget.set_download_limit(None);
        let grant = second_waiter.await.unwrap().unwrap().unwrap();
        grant.settle(1);
        assert!(budget.download_tree.lock_tree().waiting_peers(8).is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn hierarchical_download_tree_shares_torrent_credit_and_reports_advisory() {
        let budget = GlobalNetworkBudget::new(4, None, None);
        let torrent = budget.new_transfer_torrent_key();
        budget.register_download_torrent(torrent, Some(1));
        let first_peer = rate_peer(torrent, 0);
        let second_peer = rate_peer(torrent, 1);
        budget.register_transfer_peer(first_peer);
        budget.register_transfer_peer(second_peer);
        assert_eq!(
            budget.download_available_credit_bytes_for(first_peer),
            16 * 1024
        );

        let control = RateLimitCancellation::default();
        budget
            .acquire_download(first_peer, 32 * 1024, &control, control.generation())
            .await
            .unwrap()
            .unwrap()
            .settle(32 * 1024);
        assert_eq!(budget.download_available_credit_bytes_for(first_peer), 0);
        assert_eq!(budget.download_available_credit_bytes_for(second_peer), 0);

        let waiting_budget = budget.clone();
        let waiting_control = control.clone();
        let generation = control.generation();
        let waiter = tokio::spawn(async move {
            waiting_budget
                .acquire_download(second_peer, 1, &waiting_control, generation)
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        budget.set_download_torrent_limit(torrent, None);
        waiter.await.unwrap().unwrap().unwrap().settle(1);
    }

    #[tokio::test(start_paused = true)]
    async fn hierarchical_upload_tree_shares_global_credit_and_wakes_on_limit_change() {
        let budget = GlobalNetworkBudget::new(4, None, Some(1));
        let first_torrent = budget.new_transfer_torrent_key();
        let second_torrent = budget.new_transfer_torrent_key();
        budget.register_upload_torrent(first_torrent, None);
        budget.register_upload_torrent(second_torrent, None);
        let first_peer = rate_peer(first_torrent, 0);
        let second_peer = rate_peer(second_torrent, 0);
        budget.register_transfer_peer(first_peer);
        budget.register_transfer_peer(second_peer);

        // Spend beyond the initial burst so the process bucket goes into
        // debt. A peer on another torrent must still wait on the same global
        // allowance.
        let control = RateLimitCancellation::default();
        let first_grant = budget
            .acquire_upload(first_peer, 32 * 1024, &control, control.generation())
            .await
            .unwrap()
            .unwrap();
        first_grant.settle(32 * 1024);

        let waiting_budget = budget.clone();
        let waiting_control = control.clone();
        let generation = control.generation();
        let waiter = tokio::spawn(async move {
            waiting_budget
                .acquire_upload(second_peer, 1, &waiting_control, generation)
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        budget.set_upload_limit(None);
        waiter.await.unwrap().unwrap().unwrap().settle(1);
    }

    #[tokio::test(start_paused = true)]
    async fn dropping_upload_rate_grant_refunds_unused_bytes() {
        let budget = GlobalNetworkBudget::new(4, None, Some(1));
        let torrent = budget.new_transfer_torrent_key();
        budget.register_upload_torrent(torrent, None);
        let peer = rate_peer(torrent, 0);
        budget.register_transfer_peer(peer);
        let control = RateLimitCancellation::default();

        let unused = budget
            .acquire_upload(peer, 32 * 1024, &control, control.generation())
            .await
            .unwrap()
            .unwrap();
        drop(unused);

        // The unused admission restores enough credit to issue the same
        // request immediately instead of waiting at one byte per second.
        budget
            .acquire_upload(peer, 32 * 1024, &control, control.generation())
            .await
            .unwrap()
            .unwrap()
            .settle(32 * 1024);
    }

    #[test]
    fn partial_upload_rate_settlement_refunds_only_the_unsent_bytes() {
        const BLOCK: u64 = 32 * 1024;
        let budget = GlobalNetworkBudget::new(4, None, Some(1));
        let torrent = budget.new_transfer_torrent_key();
        budget.register_upload_torrent(torrent, None);
        let peer = rate_peer(torrent, 0);
        budget.register_transfer_peer(peer);

        let RateAdmissionAttempt::Granted(grant) =
            budget.upload_tree.try_acquire(peer, BLOCK).unwrap()
        else {
            panic!("initial burst should grant a full block");
        };
        grant.settle(BLOCK / 2);

        // A one-byte-per-second bucket begins with 64 KiB of burst credit.
        // Half the 32 KiB grant was sent; the other half must be refundable.
        let RateAdmissionAttempt::Granted(grant) =
            budget.upload_tree.try_acquire(peer, 48 * 1024).unwrap()
        else {
            panic!("partial settlement should return its unsent half");
        };
        grant.settle(48 * 1024);

        assert!(
            matches!(
                budget.upload_tree.try_acquire(peer, 1).unwrap(),
                RateAdmissionAttempt::Wait(_) | RateAdmissionAttempt::WaitForChange
            ),
            "the sent half stays charged; only the unsent half is refunded"
        );
    }
}
