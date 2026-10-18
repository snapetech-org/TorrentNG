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

const MAX_INITIAL_BURST_BYTES: u64 = 64 * 1024;

/// Convert elapsed time into whole byte tokens without floating-point drift.
pub(crate) fn refilled_bytes(elapsed: Duration, limit_bytes_per_sec: u64) -> u64 {
    let refill = elapsed
        .as_nanos()
        .saturating_mul(u128::from(limit_bytes_per_sec))
        / 1_000_000_000;
    refill.min(u128::from(u64::MAX)) as u64
}

/// Return the first whole-nanosecond deadline at which `missing_bytes` can be
/// refilled. Limits are normalized to a positive value before they reach the
/// rate-limiter paths; the lower bound keeps this helper safe at boundaries.
pub(crate) fn token_refill_delay(missing_bytes: u64, limit_bytes_per_sec: u64) -> Duration {
    if missing_bytes == 0 {
        return Duration::ZERO;
    }
    let rate = u128::from(limit_bytes_per_sec.max(1));
    let wait_nanos = u128::from(missing_bytes)
        .saturating_mul(1_000_000_000)
        .saturating_add(rate.saturating_sub(1))
        / rate;
    duration_from_nanos(wait_nanos.max(1))
}

fn duration_from_nanos(nanos: u128) -> Duration {
    let seconds = nanos / 1_000_000_000;
    if seconds > u128::from(u64::MAX) {
        return Duration::MAX;
    }
    Duration::new(seconds as u64, (nanos % 1_000_000_000) as u32)
}

/// Process-wide network admission and traffic budgets.
///
/// The peer semaphore counts live peer/metadata connections. The rate
/// limiters count payload bytes at the engine boundary; protocol framing and
/// TCP/IP overhead are not included in the current accounting contract.
#[derive(Clone)]
pub(crate) struct GlobalNetworkBudget {
    peer_slots: Arc<Semaphore>,
    download: Arc<SharedRateLimiter>,
    upload: Arc<SharedRateLimiter>,
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
        Self {
            peer_slots: Arc::new(Semaphore::new(max_peers.max(1))),
            download: Arc::new(SharedRateLimiter::new(download_limit_bytes_per_sec)),
            upload: Arc::new(SharedRateLimiter::new(upload_limit_bytes_per_sec)),
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

    pub(crate) fn download(&self) -> Arc<SharedRateLimiter> {
        Arc::clone(&self.download)
    }

    pub(crate) fn upload(&self) -> Arc<SharedRateLimiter> {
        Arc::clone(&self.upload)
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
        self.download.set_limit(limit);
    }

    pub(crate) fn set_upload_limit(&self, limit: Option<u64>) {
        self.upload.set_limit(limit);
    }
}

#[derive(Debug)]
struct RateState {
    limit_bytes_per_sec: Option<u64>,
    tokens: u64,
    updated_at: Instant,
}

/// A small shared token bucket. The mutex is held only for arithmetic; waits
/// happen outside it so a slow torrent cannot block unrelated tasks.
pub(crate) struct SharedRateLimiter {
    state: Mutex<RateState>,
    /// Wake waiters when the runtime limit changes. Without this, a waiter
    /// can sleep against the old rate for an arbitrarily long interval after
    /// the engine has already raised or removed the limit.
    limit_changed: RateLimitCancellation,
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

impl SharedRateLimiter {
    pub(crate) fn new(limit_bytes_per_sec: Option<u64>) -> Self {
        let capacity = limit_bytes_per_sec
            .filter(|limit| *limit > 0)
            .map(|limit| limit.max(MAX_INITIAL_BURST_BYTES))
            .unwrap_or(u64::MAX);
        Self {
            state: Mutex::new(RateState {
                limit_bytes_per_sec: limit_bytes_per_sec.filter(|limit| *limit > 0),
                tokens: capacity,
                updated_at: Instant::now(),
            }),
            limit_changed: RateLimitCancellation::default(),
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, RateState> {
        match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                // Keep the existing token balance and rate limit. Resetting
                // this state would grant an attacker a fresh burst.
                let guard = poisoned.into_inner();
                self.state.clear_poison();
                guard
            }
        }
    }

    pub(crate) fn set_limit(&self, limit: Option<u64>) {
        let limit = limit.filter(|limit| *limit > 0);
        let mut state = self.lock_state();
        if state.limit_bytes_per_sec == limit {
            return;
        }
        let capacity = limit
            .map(|limit| limit.max(MAX_INITIAL_BURST_BYTES))
            .unwrap_or(u64::MAX);
        state.limit_bytes_per_sec = limit;
        state.tokens = capacity;
        state.updated_at = Instant::now();
        drop(state);
        self.limit_changed.cancel();
    }

    /// Snapshot currently available download credit without reserving it.
    ///
    /// Adaptive request windows use this as an admission hint so they shrink
    /// peer pipelines to the one-block progress floor while the process-wide
    /// bucket is empty.
    /// It is deliberately not a reservation: multiple torrents may observe
    /// the same credit, and received payload bytes are still charged by
    /// `acquire` when they arrive.
    pub(crate) fn available_credit_bytes(&self) -> Option<u64> {
        let mut state = self.lock_state();
        let limit = state.limit_bytes_per_sec?;
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(state.updated_at);
        state.updated_at = now;
        let refill = refilled_bytes(elapsed, limit);
        let capacity = limit.max(MAX_INITIAL_BURST_BYTES);
        state.tokens = state.tokens.saturating_add(refill).min(capacity);
        Some(state.tokens)
    }

    pub(crate) async fn acquire(&self, bytes: u64) {
        if bytes == 0 {
            return;
        }
        let mut remaining = bytes;
        let mut limit_generation = self.limit_changed.generation();
        loop {
            let wait = {
                let mut state = self.lock_state();
                let Some(limit) = state.limit_bytes_per_sec else {
                    return;
                };
                let now = Instant::now();
                let elapsed = now.saturating_duration_since(state.updated_at);
                state.updated_at = now;
                let refill = refilled_bytes(elapsed, limit);
                state.tokens = state
                    .tokens
                    .saturating_add(refill)
                    .min(limit.max(MAX_INITIAL_BURST_BYTES));
                // A protocol frame can be larger than the initial burst
                // (for example, a future metadata/data path may account for
                // a whole bounded frame). Consume it in bucket-sized chunks;
                // asking the bucket for more than its capacity would
                // otherwise wait forever because tokens can never reach that
                // request size.
                let requested = remaining.min(limit.max(MAX_INITIAL_BURST_BYTES));
                if state.tokens >= requested {
                    state.tokens -= requested;
                    remaining -= requested;
                    if remaining == 0 {
                        return;
                    }
                    Duration::ZERO
                } else {
                    let missing = requested.saturating_sub(state.tokens);
                    token_refill_delay(missing, limit)
                }
            };
            if wait.is_zero() {
                tokio::task::yield_now().await;
            } else {
                tokio::select! {
                    _ = tokio::time::sleep(wait) => {}
                    _ = self.limit_changed.wait_for_change(limit_generation) => {
                        limit_generation = self.limit_changed.generation();
                    }
                }
            }
        }
    }

    /// Acquire shared traffic budget while allowing the owning peer to stop
    /// waiting when its control state changes. A peer command must not wait
    /// behind a low global rate limit before it can observe Choke or Shutdown.
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
                let elapsed = now.saturating_duration_since(state.updated_at);
                state.updated_at = now;
                let refill = refilled_bytes(elapsed, limit);
                state.tokens = state
                    .tokens
                    .saturating_add(refill)
                    .min(limit.max(MAX_INITIAL_BURST_BYTES));
                let requested = remaining.min(limit.max(MAX_INITIAL_BURST_BYTES));
                if state.tokens >= requested {
                    state.tokens -= requested;
                    remaining -= requested;
                    if remaining == 0 {
                        return cancellation.generation() == generation;
                    }
                    Duration::ZERO
                } else {
                    let missing = requested.saturating_sub(state.tokens);
                    token_refill_delay(missing, limit)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_refill_math_uses_exact_integer_nanosecond_deadlines() {
        let limit = 2_000_000;
        assert_eq!(token_refill_delay(1, limit), Duration::from_nanos(500));
        assert_eq!(refilled_bytes(Duration::from_nanos(499), limit), 0);
        assert_eq!(refilled_bytes(Duration::from_nanos(500), limit), 1);
    }

    #[tokio::test]
    async fn unlimited_budget_does_not_wait() {
        let limiter = SharedRateLimiter::new(None);
        tokio::time::timeout(Duration::from_millis(50), limiter.acquire(1_000_000))
            .await
            .expect("unlimited limiter should not wait");
    }

    #[tokio::test(start_paused = true)]
    async fn limited_budget_refills_after_wait() {
        let limiter = Arc::new(SharedRateLimiter::new(Some(1_000)));
        limiter.acquire(MAX_INITIAL_BURST_BYTES).await;
        let waiter_limiter = Arc::clone(&limiter);
        let waiter = tokio::spawn(async move { waiter_limiter.acquire(1_000).await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        tokio::time::advance(Duration::from_secs(1)).await;
        waiter.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn available_credit_snapshot_tracks_spend_without_reserving() {
        let limiter = SharedRateLimiter::new(Some(1_000));
        assert_eq!(
            limiter.available_credit_bytes(),
            Some(MAX_INITIAL_BURST_BYTES)
        );

        limiter.acquire(8 * 1024).await;
        assert_eq!(
            limiter.available_credit_bytes(),
            Some(MAX_INITIAL_BURST_BYTES - 8 * 1024)
        );
        assert_eq!(
            SharedRateLimiter::new(None).available_credit_bytes(),
            None,
            "an unlimited bucket must not constrain a request window"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn limited_budget_accepts_request_larger_than_bucket_capacity() {
        let limiter = Arc::new(SharedRateLimiter::new(Some(1_000)));
        let waiter_limiter = Arc::clone(&limiter);
        let waiter = tokio::spawn(async move {
            waiter_limiter
                .acquire(MAX_INITIAL_BURST_BYTES.saturating_mul(2))
                .await;
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        tokio::time::advance(Duration::from_secs(65)).await;
        waiter.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn cancellable_budget_wakes_when_peer_control_changes() {
        let limiter = Arc::new(SharedRateLimiter::new(Some(1)));
        limiter.acquire(MAX_INITIAL_BURST_BYTES).await;
        let cancellation = RateLimitCancellation::default();
        let generation = cancellation.generation();
        let waiter_limiter = Arc::clone(&limiter);
        let waiter_cancellation = cancellation.clone();
        let waiter = tokio::spawn(async move {
            waiter_limiter
                .acquire_or_cancelled(1, &waiter_cancellation, generation)
                .await
        });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        cancellation.cancel();
        assert!(!waiter.await.unwrap());
    }

    #[tokio::test(start_paused = true)]
    async fn changing_global_limit_wakes_existing_waiters() {
        let limiter = Arc::new(SharedRateLimiter::new(Some(1)));
        limiter.acquire(MAX_INITIAL_BURST_BYTES).await;
        let waiter_limiter = Arc::clone(&limiter);
        let waiter = tokio::spawn(async move { waiter_limiter.acquire(1).await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        limiter.set_limit(None);
        waiter.await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn reapplying_same_limit_does_not_restore_spent_burst_tokens() {
        let limiter = Arc::new(SharedRateLimiter::new(Some(1)));
        limiter.acquire(MAX_INITIAL_BURST_BYTES).await;

        // Runtime API updates may persist and reapply an unchanged effective
        // limit. That is not a grant of a new token-bucket burst.
        limiter.set_limit(Some(1));
        let waiter_limiter = Arc::clone(&limiter);
        let waiter = tokio::spawn(async move { waiter_limiter.acquire(1).await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());

        tokio::time::advance(Duration::from_secs(1)).await;
        waiter.await.unwrap();
    }

    #[tokio::test]
    async fn poisoned_rate_limiter_preserves_tokens_and_recovers() {
        let limiter = Arc::new(SharedRateLimiter::new(Some(1_000_000)));
        limiter.acquire(1_000).await;
        let tokens_before_poison = limiter.lock_state().tokens;

        let poisoner = Arc::clone(&limiter);
        assert!(std::thread::spawn(move || {
            let _guard = poisoner.state.lock().unwrap();
            panic!("poison the shared network token bucket");
        })
        .join()
        .is_err());
        assert!(limiter.state.is_poisoned());

        limiter.set_limit(Some(1_000_000));
        assert_eq!(limiter.lock_state().tokens, tokens_before_poison);
        assert!(!limiter.state.is_poisoned());
        limiter.acquire(1).await;
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
}
