use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use tokio::sync::{mpsc, oneshot, Notify, Semaphore};
// tokio's Instant, not std's: it respects the paused/mockable clock that
// `#[tokio::test(start_paused = true)]` and `tokio::time::advance()` use.
// Using std::time::Instant here would make the refill calculation below see
// real wall-clock time regardless of virtual time advances, which live-locks
// a paused-clock test into an infinite near-zero-wait retry loop.
use rt_utp::{UtpEndpoint, UtpRuntime};
use tokio::time::Instant;

use crate::transfer::coords::{ConnectionId, TorrentKey, TransferPeerId};
use crate::transfer::rate::{Grant, RatePeerId, RateTree};

/// Process-wide network admission and traffic budgets.
///
/// The peer semaphore counts live peer/metadata connections. The rate
/// limiters count payload bytes at the engine boundary; protocol framing and
/// TCP/IP overhead are not included in the current accounting contract.
#[derive(Clone)]
pub(crate) struct GlobalNetworkBudget {
    peer_slots: Arc<Semaphore>,
    download_tree: Arc<HierarchicalRateService>,
    upload_tree: Arc<HierarchicalRateService>,
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
        let (listener_rebind_tx, listener_rebind_rx) = mpsc::channel(4);
        let upload_origin = Instant::now();
        Self {
            peer_slots: Arc::new(Semaphore::new(max_peers.max(1))),
            download_tree: Arc::new(HierarchicalRateService {
                tree: Mutex::new(RateTree::new(download_limit_bytes_per_sec, 0)),
                origin: upload_origin,
                changed: RateLimitCancellation::default(),
                next_torrent: AtomicU64::new(1),
            }),
            upload_tree: Arc::new(HierarchicalRateService {
                tree: Mutex::new(RateTree::new(upload_limit_bytes_per_sec, 0)),
                origin: upload_origin,
                changed: RateLimitCancellation::default(),
                next_torrent: AtomicU64::new(1),
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

    pub(crate) fn download_available_credit_bytes_for(&self, peer_id: RatePeerId) -> u64 {
        self.download_tree.available_credit_bytes(peer_id)
    }

    pub(crate) fn utp_runtime(&self) -> UtpRuntime {
        self.utp_runtime.clone()
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
    }

    pub(crate) fn set_upload_torrent_limit(&self, key: TorrentKey, limit: Option<u64>) {
        self.upload_tree.set_torrent_limit(key, limit);
    }

    pub(crate) fn set_download_torrent_limit(&self, key: TorrentKey, limit: Option<u64>) {
        self.download_tree.set_torrent_limit(key, limit);
    }

    pub(crate) fn remove_transfer_torrent(&self, key: TorrentKey) {
        self.upload_tree.remove_torrent(key);
        self.download_tree.remove_torrent(key);
    }

    pub(crate) fn register_transfer_peer(&self, peer_id: RatePeerId) {
        self.upload_tree.register_peer(peer_id);
        self.download_tree.register_peer(peer_id);
    }

    pub(crate) fn remove_transfer_peer(&self, peer_id: RatePeerId) {
        self.upload_tree.remove_peer(peer_id);
        self.download_tree.remove_peer(peer_id);
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
}

pub(crate) enum RateAdmissionAttempt {
    Granted(RateAdmissionGrant),
    Wait(Duration),
}

/// Live Tokio adapter for the deterministic transfer `RateTree`. The lock is
/// held only while applying one scheduler event; deadlines, cancellation,
/// and network I/O all happen after it is released.
struct HierarchicalRateService {
    tree: Mutex<RateTree>,
    origin: Instant,
    changed: RateLimitCancellation,
    next_torrent: AtomicU64,
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
        let value = self
            .next_torrent
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .expect("transfer rate torrent identity space exhausted");
        TorrentKey(value)
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
        self.lock_tree().add_torrent(key, limit, self.now());
        self.changed.cancel();
    }

    fn set_torrent_limit(&self, key: TorrentKey, limit: Option<u64>) {
        self.lock_tree().set_torrent_limit(key, limit, self.now());
        self.changed.cancel();
    }

    fn remove_torrent(&self, key: TorrentKey) {
        self.lock_tree().remove_torrent(key);
        self.changed.cancel();
    }

    fn register_peer(&self, peer_id: RatePeerId) {
        self.lock_tree().add_peer(peer_id, 1);
        self.changed.cancel();
    }

    fn remove_peer(&self, peer_id: RatePeerId) {
        self.lock_tree().remove_peer(peer_id);
        self.changed.cancel();
    }

    fn settle(&self, peer_id: RatePeerId, grant_id: u64, bytes: u64) {
        self.lock_tree().settle_grant(peer_id, grant_id, bytes);
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
        let (grant, grant_id) = self.lock_tree().try_grant_tracked(peer_id, bytes, now);
        match grant {
            Grant::Granted => Ok(RateAdmissionAttempt::Granted(RateAdmissionGrant {
                scheduler: Arc::clone(self),
                peer_id,
                grant_id: grant_id.expect("granted payload has a reservation identity"),
                bytes,
                active: true,
            })),
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
        loop {
            if cancellation.generation() != generation {
                return Ok(None);
            }
            let changed_generation = self.changed.generation();
            let now = self.now();
            let (grant, grant_id) = self.lock_tree().try_grant_tracked(peer_id, bytes, now);
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
                    let delay = Duration::from_nanos(until.saturating_sub(self.now()));
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        _ = self.changed.wait_for_change(changed_generation) => {}
                        _ = cancellation.wait_for_change(generation) => return Ok(None),
                    }
                }
            }
        }
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
        self.scheduler
            .settle(self.peer_id, self.grant_id, transferred);
        self.scheduler.refund(
            self.peer_id,
            self.grant_id,
            self.bytes.saturating_sub(transferred),
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
        TransferPeerId::new(
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
}
