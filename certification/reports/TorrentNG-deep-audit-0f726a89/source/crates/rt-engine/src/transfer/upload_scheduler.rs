//! Process-wide round-robin admission for live upload reads (NET-110).
//!
//! Peer loops retain protocol obligations and socket ownership. This broker
//! limits the expensive read/rate-wait work across those loops, and admits at
//! most one job per connection per turn. Permits remain live through the
//! serialized Piece response so both job and byte charges include retained
//! upload payloads.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::oneshot;

use super::coords::{BlockExtent, BLOCK_SIZE};
use super::rate::RatePeerId;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UploadReadLimits {
    pub max_connections: usize,
    pub max_in_flight_jobs: usize,
    pub max_in_flight_bytes: u64,
    pub max_in_flight_per_connection: usize,
    pub max_queued_per_connection: usize,
}

impl Default for UploadReadLimits {
    fn default() -> Self {
        Self {
            max_connections: 200,
            max_in_flight_jobs: 64,
            max_in_flight_bytes: 16 * 1024 * 1024,
            max_in_flight_per_connection: 8,
            max_queued_per_connection: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UploadReadAdmissionError {
    NotRegistered,
    AlreadyRegistered,
    ConnectionLimit,
    QueueFull,
    InvalidBlock,
    IdentityExhausted,
}

#[derive(Clone)]
pub(crate) struct UploadReadScheduler {
    inner: Arc<Inner>,
}

struct Inner {
    limits: UploadReadLimits,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    connections: HashMap<RatePeerId, ConnectionState>,
    next_connection_order: u64,
    last_served_order: Option<u64>,
    round_robin: VecDeque<RatePeerId>,
    next_waiter: u64,
    in_flight_jobs: usize,
    in_flight_bytes: u64,
}

#[derive(Default)]
struct ConnectionState {
    order: u64,
    queue: VecDeque<Waiter>,
    in_flight_jobs: usize,
    in_flight_bytes: u64,
    closed: bool,
    queued_in_round_robin: bool,
}

struct Waiter {
    id: u64,
    bytes: u64,
    reply: oneshot::Sender<UploadReadPermit>,
}

pub(crate) struct UploadReadPermit {
    inner: Weak<Inner>,
    peer: RatePeerId,
    bytes: u64,
}

struct WaiterGuard {
    inner: Weak<Inner>,
    peer: RatePeerId,
    id: Option<u64>,
}

impl UploadReadScheduler {
    pub(crate) fn new(mut limits: UploadReadLimits) -> Self {
        limits.max_connections = limits.max_connections.max(1);
        limits.max_in_flight_jobs = limits.max_in_flight_jobs.max(1);
        limits.max_in_flight_bytes = limits.max_in_flight_bytes.max(u64::from(BLOCK_SIZE));
        limits.max_in_flight_per_connection = limits.max_in_flight_per_connection.max(1);
        limits.max_queued_per_connection = limits.max_queued_per_connection.max(1);
        Self {
            inner: Arc::new(Inner {
                limits,
                state: Mutex::new(State {
                    next_waiter: 1,
                    ..State::default()
                }),
            }),
        }
    }

    /// Register one generation-safe connection before its peer loop starts.
    pub(crate) fn register(&self, peer: RatePeerId) -> Result<(), UploadReadAdmissionError> {
        let mut state = lock_state(&self.inner);
        if state.connections.contains_key(&peer) {
            return Err(UploadReadAdmissionError::AlreadyRegistered);
        }
        if state
            .connections
            .values()
            .filter(|connection| !connection.closed)
            .count()
            >= self.inner.limits.max_connections
        {
            return Err(UploadReadAdmissionError::ConnectionLimit);
        }
        let order = state.next_connection_order;
        state.next_connection_order = order.saturating_add(1);
        state.connections.insert(
            peer,
            ConnectionState {
                order,
                ..ConnectionState::default()
            },
        );
        Ok(())
    }

    pub(crate) fn register_guard(
        &self,
        peer: RatePeerId,
    ) -> Result<UploadReadConnection, UploadReadAdmissionError> {
        self.register(peer)?;
        Ok(UploadReadConnection {
            scheduler: self.clone(),
            peer,
        })
    }

    /// Close a connection and discard waiters that have not received a read
    /// permit. Running jobs keep their global charge until their permit drops.
    pub(crate) fn unregister(&self, peer: RatePeerId) {
        let mut state = lock_state(&self.inner);
        let remove = if let Some(connection) = state.connections.get_mut(&peer) {
            connection.closed = true;
            connection.queue.clear();
            connection.queued_in_round_robin = false;
            connection.in_flight_jobs == 0
        } else {
            false
        };
        state.round_robin.retain(|candidate| *candidate != peer);
        if remove {
            state.connections.remove(&peer);
        }
    }

    /// Wait for fair process-wide job and byte admission. Dropping this future
    /// removes a queued waiter; dropping the returned permit releases the
    /// process and connection charges and advances the round-robin queue.
    pub(crate) async fn acquire(
        &self,
        peer: RatePeerId,
        block: BlockExtent,
    ) -> Result<UploadReadPermit, UploadReadAdmissionError> {
        if block.length == 0 || block.length > BLOCK_SIZE {
            return Err(UploadReadAdmissionError::InvalidBlock);
        }
        let bytes = u64::from(block.length);
        let (waiter_id, receive) = {
            let mut state = lock_state(&self.inner);
            let (closed, queue_len, in_flight_jobs, connection_order) = state
                .connections
                .get(&peer)
                .map(|connection| {
                    (
                        connection.closed,
                        connection.queue.len(),
                        connection.in_flight_jobs,
                        connection.order,
                    )
                })
                .ok_or(UploadReadAdmissionError::NotRegistered)?;
            if closed {
                return Err(UploadReadAdmissionError::NotRegistered);
            }
            if queue_len.saturating_add(in_flight_jobs)
                >= self
                    .inner
                    .limits
                    .max_queued_per_connection
                    .saturating_add(self.inner.limits.max_in_flight_per_connection)
            {
                return Err(UploadReadAdmissionError::QueueFull);
            }

            // The common uncontended case needs only this mutex acquisition:
            // no waiter ID, oneshot channel, or scheduler pump is created.
            // Any already-pending peer gets a turn before a new immediate grant.
            if state.round_robin.is_empty()
                && queue_len == 0
                && state.in_flight_jobs < self.inner.limits.max_in_flight_jobs
                && state.in_flight_bytes.saturating_add(bytes)
                    <= self.inner.limits.max_in_flight_bytes
                && in_flight_jobs < self.inner.limits.max_in_flight_per_connection
            {
                state.in_flight_jobs += 1;
                state.in_flight_bytes = state.in_flight_bytes.saturating_add(bytes);
                state.last_served_order = Some(connection_order);
                let connection = state
                    .connections
                    .get_mut(&peer)
                    .expect("connection checked above");
                connection.in_flight_jobs += 1;
                connection.in_flight_bytes = connection.in_flight_bytes.saturating_add(bytes);
                return Ok(UploadReadPermit {
                    inner: Arc::downgrade(&self.inner),
                    peer,
                    bytes,
                });
            }

            let id = state.next_waiter;
            state.next_waiter = id
                .checked_add(1)
                .ok_or(UploadReadAdmissionError::IdentityExhausted)?;
            let (reply, receive) = oneshot::channel();
            let connection = state
                .connections
                .get_mut(&peer)
                .expect("connection checked above");
            let add_to_round_robin =
                connection.queue.is_empty() && !connection.queued_in_round_robin;
            connection.queue.push_back(Waiter { id, bytes, reply });
            if add_to_round_robin {
                connection.queued_in_round_robin = true;
                queue_pending_peer(&mut state, peer);
            }
            (id, receive)
        };
        let mut waiter_guard = WaiterGuard {
            inner: Arc::downgrade(&self.inner),
            peer,
            id: Some(waiter_id),
        };
        pump(&self.inner);
        let permit = receive
            .await
            .map_err(|_| UploadReadAdmissionError::NotRegistered)?;
        waiter_guard.id = None;
        Ok(permit)
    }

    #[cfg(test)]
    fn in_flight(&self) -> (usize, u64) {
        let state = lock_state(&self.inner);
        (state.in_flight_jobs, state.in_flight_bytes)
    }

    #[cfg(test)]
    fn queued(&self) -> usize {
        lock_state(&self.inner)
            .connections
            .values()
            .map(|connection| connection.queue.len())
            .sum()
    }

    #[cfg(test)]
    fn pending_connection_count(&self) -> usize {
        lock_state(&self.inner).round_robin.len()
    }
}

pub(crate) struct UploadReadConnection {
    scheduler: UploadReadScheduler,
    peer: RatePeerId,
}

impl Drop for UploadReadConnection {
    fn drop(&mut self) {
        self.scheduler.unregister(self.peer);
    }
}

impl Drop for UploadReadPermit {
    fn drop(&mut self) {
        let Some(inner) = self.inner.upgrade() else {
            return;
        };
        {
            let mut state = lock_state(&inner);
            state.in_flight_jobs = state.in_flight_jobs.saturating_sub(1);
            state.in_flight_bytes = state.in_flight_bytes.saturating_sub(self.bytes);
            let remove = if let Some(connection) = state.connections.get_mut(&self.peer) {
                connection.in_flight_jobs = connection.in_flight_jobs.saturating_sub(1);
                connection.in_flight_bytes = connection.in_flight_bytes.saturating_sub(self.bytes);
                connection.closed && connection.in_flight_jobs == 0
            } else {
                false
            };
            if remove {
                state.connections.remove(&self.peer);
            }
        }
        pump(&inner);
    }
}

impl Drop for WaiterGuard {
    fn drop(&mut self) {
        let (Some(inner), Some(id)) = (self.inner.upgrade(), self.id.take()) else {
            return;
        };
        let mut state = lock_state(&inner);
        let mut remove_from_round_robin = false;
        if let Some(connection) = state.connections.get_mut(&self.peer) {
            connection.queue.retain(|waiter| waiter.id != id);
            if connection.queue.is_empty() && connection.queued_in_round_robin {
                connection.queued_in_round_robin = false;
                remove_from_round_robin = true;
            }
        }
        if remove_from_round_robin {
            state
                .round_robin
                .retain(|candidate| *candidate != self.peer);
        }
        drop(state);
        pump(&inner);
    }
}

fn pump(inner: &Arc<Inner>) {
    let failed_permits = {
        let mut state = lock_state(inner);
        let mut failed_permits = Vec::new();
        let mut idle_turns = 0usize;
        while state.in_flight_jobs < inner.limits.max_in_flight_jobs
            && !state.round_robin.is_empty()
        {
            let ring_len = state.round_robin.len();
            let Some(peer) = state.round_robin.pop_front() else {
                break;
            };
            let global_bytes = state.in_flight_bytes;
            let (closed, has_waiters, eligible, connection_order) = {
                let Some(connection) = state.connections.get_mut(&peer) else {
                    continue;
                };
                connection.queued_in_round_robin = false;
                (
                    connection.closed,
                    !connection.queue.is_empty(),
                    !connection.closed
                        && connection.in_flight_jobs < inner.limits.max_in_flight_per_connection
                        && connection.queue.front().is_some_and(|waiter| {
                            global_bytes.saturating_add(waiter.bytes)
                                <= inner.limits.max_in_flight_bytes
                        }),
                    connection.order,
                )
            };
            if closed || !has_waiters {
                continue;
            }
            if !eligible {
                if let Some(connection) = state.connections.get_mut(&peer) {
                    connection.queued_in_round_robin = true;
                }
                state.round_robin.push_back(peer);
                idle_turns += 1;
                if idle_turns >= ring_len {
                    break;
                }
                continue;
            }
            let waiter = {
                let connection = state
                    .connections
                    .get_mut(&peer)
                    .expect("eligible connection remains registered");
                let waiter = connection
                    .queue
                    .pop_front()
                    .expect("eligible connection has a queued waiter");
                connection.in_flight_jobs += 1;
                connection.in_flight_bytes =
                    connection.in_flight_bytes.saturating_add(waiter.bytes);
                if !connection.queue.is_empty() {
                    connection.queued_in_round_robin = true;
                }
                waiter
            };
            if state
                .connections
                .get(&peer)
                .is_some_and(|connection| connection.queued_in_round_robin)
            {
                state.round_robin.push_back(peer);
            }
            state.in_flight_jobs += 1;
            state.in_flight_bytes = state.in_flight_bytes.saturating_add(waiter.bytes);
            state.last_served_order = Some(connection_order);
            idle_turns = 0;
            let permit = UploadReadPermit {
                inner: Arc::downgrade(inner),
                peer,
                bytes: waiter.bytes,
            };
            if let Err(error) = waiter.reply.send(permit) {
                failed_permits.push(error);
            }
        }
        failed_permits
    };
    // A failed oneshot send returns an RAII permit. Drop it after unlocking so
    // its accounting callback cannot recursively lock the scheduler.
    drop(failed_permits);
}

fn queue_pending_peer(state: &mut State, peer: RatePeerId) {
    let order = state
        .connections
        .get(&peer)
        .expect("queued peer remains registered")
        .order;
    let rank = pending_rank(order, state.last_served_order);
    let insert_at = state.round_robin.iter().position(|queued_peer| {
        let queued_order = state
            .connections
            .get(queued_peer)
            .map(|connection| connection.order)
            .unwrap_or(u64::MAX);
        pending_rank(queued_order, state.last_served_order) > rank
    });
    match insert_at {
        Some(index) => state.round_robin.insert(index, peer),
        None => state.round_robin.push_back(peer),
    }
}

fn pending_rank(order: u64, last_served_order: Option<u64>) -> u128 {
    match last_served_order {
        Some(last) if order > last => u128::from(order - last),
        Some(last) => u128::from(u64::MAX - last) + u128::from(order) + 1,
        None => u128::from(order),
    }
}

fn lock_state(inner: &Inner) -> std::sync::MutexGuard<'_, State> {
    inner
        .state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::coords::{ConnectionId, TorrentKey};

    fn peer(torrent: u64, slot: u32) -> RatePeerId {
        RatePeerId::new(
            TorrentKey(torrent),
            ConnectionId {
                slot,
                generation: 1,
            },
        )
    }

    fn block(length: u32) -> BlockExtent {
        BlockExtent::new(0, 0, length)
    }

    #[tokio::test]
    async fn uncontended_admission_ignores_registered_idle_peers() {
        let scheduler = UploadReadScheduler::new(UploadReadLimits {
            max_connections: 4,
            max_in_flight_jobs: 1,
            max_in_flight_bytes: 16 * 1024,
            max_in_flight_per_connection: 1,
            max_queued_per_connection: 1,
        });
        let first = peer(1, 0);
        let second = peer(2, 0);
        let third = peer(3, 0);
        let _first_connection = scheduler.register_guard(first).unwrap();
        let _second_connection = scheduler.register_guard(second).unwrap();
        let _third_connection = scheduler.register_guard(third).unwrap();
        assert_eq!(scheduler.pending_connection_count(), 0);

        let permit = scheduler.acquire(first, block(16 * 1024)).await.unwrap();
        assert_eq!(scheduler.queued(), 0);
        assert_eq!(scheduler.pending_connection_count(), 0);
        drop(permit);
    }

    #[tokio::test]
    async fn read_admission_is_process_wide_and_round_robin() {
        let scheduler = UploadReadScheduler::new(UploadReadLimits {
            max_connections: 2,
            max_in_flight_jobs: 1,
            max_in_flight_bytes: 16 * 1024,
            max_in_flight_per_connection: 1,
            max_queued_per_connection: 2,
        });
        let first = peer(1, 0);
        let second = peer(2, 0);
        let _first_connection = scheduler.register_guard(first).unwrap();
        let _second_connection = scheduler.register_guard(second).unwrap();
        let first_permit = scheduler.acquire(first, block(16 * 1024)).await.unwrap();
        let waiting_first = {
            let scheduler = scheduler.clone();
            tokio::spawn(async move { scheduler.acquire(first, block(16 * 1024)).await })
        };
        let waiting_second = {
            let scheduler = scheduler.clone();
            tokio::spawn(async move { scheduler.acquire(second, block(16 * 1024)).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while scheduler.queued() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both peers should be waiting for the global slot");
        assert_eq!(scheduler.in_flight(), (1, 16 * 1024));
        drop(first_permit);
        let second_permit = waiting_second.await.unwrap().unwrap();
        assert_eq!(scheduler.in_flight(), (1, 16 * 1024));
        drop(second_permit);
        let first_waiting_permit = waiting_first.await.unwrap().unwrap();
        assert_eq!(scheduler.in_flight(), (1, 16 * 1024));
        drop(first_waiting_permit);
        assert_eq!(scheduler.in_flight(), (0, 0));
    }

    #[tokio::test]
    async fn dropping_waiter_and_disconnect_release_or_reject_admission() {
        let scheduler = UploadReadScheduler::new(UploadReadLimits {
            max_connections: 1,
            max_in_flight_jobs: 1,
            max_in_flight_bytes: 16 * 1024,
            max_in_flight_per_connection: 1,
            max_queued_per_connection: 1,
        });
        let peer = peer(1, 0);
        let _connection = scheduler.register_guard(peer).unwrap();
        let permit = scheduler.acquire(peer, block(16 * 1024)).await.unwrap();
        let queued = {
            let scheduler = scheduler.clone();
            tokio::spawn(async move { scheduler.acquire(peer, block(16 * 1024)).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while scheduler.queued() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second request should wait behind the active slot");
        queued.abort();
        let _ = queued.await;
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while scheduler.queued() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("canceled waiter should leave the shared queue");
        drop(_connection);
        drop(permit);
        assert!(matches!(
            scheduler.acquire(peer, block(16 * 1024)).await,
            Err(UploadReadAdmissionError::NotRegistered)
        ));
        assert_eq!(scheduler.in_flight(), (0, 0));
    }

    #[tokio::test]
    async fn disconnected_read_keeps_job_charge_but_not_live_connection_slot() {
        let scheduler = UploadReadScheduler::new(UploadReadLimits {
            max_connections: 1,
            max_in_flight_jobs: 1,
            max_in_flight_bytes: 16 * 1024,
            max_in_flight_per_connection: 1,
            max_queued_per_connection: 1,
        });
        let old_peer = peer(1, 0);
        let replacement_peer = peer(2, 0);
        let old_connection = scheduler.register_guard(old_peer).unwrap();
        assert!(matches!(
            scheduler.register_guard(replacement_peer),
            Err(UploadReadAdmissionError::ConnectionLimit)
        ));
        let old_permit = scheduler.acquire(old_peer, block(16 * 1024)).await.unwrap();
        drop(old_connection);

        let replacement = scheduler.register_guard(replacement_peer).unwrap();
        let replacement_read = {
            let scheduler = scheduler.clone();
            tokio::spawn(async move { scheduler.acquire(replacement_peer, block(16 * 1024)).await })
        };
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while scheduler.queued() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("replacement read should wait for the old storage job");
        assert_eq!(scheduler.in_flight(), (1, 16 * 1024));

        drop(old_permit);
        let replacement_permit = replacement_read.await.unwrap().unwrap();
        assert_eq!(scheduler.in_flight(), (1, 16 * 1024));
        drop(replacement_permit);
        drop(replacement);
        assert_eq!(scheduler.in_flight(), (0, 0));
    }
}
