//! Hierarchical global / torrent / peer rate scheduler (NET-107).
//!
//! The previous design copied the torrent limit into a token bucket per peer,
//! so N peers could each spend the full torrent allowance; refilling had no
//! wakeup when tokens became available again; and a limit update re-minted a
//! full burst in every connection. This scheduler replaces all of that with
//! one tree:
//!
//! * a grant succeeds **atomically** at every level (global, torrent, peer
//!   fairness) or at none, and a denial reports the next useful retry deadline
//!   from rate credit or fairness progress, so callers do not poll on a tick;
//! * credit is kept in fractional byte-nanoseconds, and a grant may drive a
//!   bucket into debt, so a rate below one block per second still progresses
//!   at exactly its configured long-run rate;
//! * changing a limit never *adds* credit: it can only clamp it, so a limit
//!   update cannot mint a fresh burst anywhere in the tree;
//! * peers under one torrent share its bucket by weighted start-time fair
//!   queueing, so one fast peer cannot starve the others;
//! * time is injected (`Nanos`), never read.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::coords::{TorrentKey, TransferPeerId};
use super::Nanos;

/// Rate-specific name retained for callers that work directly with the
/// hierarchical scheduler. The identity is shared with other transfer-core
/// services that span multiple torrent ledgers.
pub type RatePeerId = TransferPeerId;

/// A fairness-blocked demand has no clock deadline; it wakes when a peer
/// makes progress, is removed, or retires its pending demand.
pub const CHANGE_ONLY_WAKE: Nanos = Nanos::MAX;

const NANOS_PER_SEC: i128 = 1_000_000_000;
/// Burst a limited bucket may accumulate: this fraction of a second of rate.
const BURST_DIVISOR: u64 = 10;
/// A bucket can always bank at least one protocol block.
const MIN_BURST_BYTES: u64 = 16 * 1024;
/// How far ahead (in bytes at weight 1) a peer may run before waiting for
/// slower peers under the same torrent.
const FAIRNESS_LAG_BYTES: u128 = 2 * 16 * 1024;
/// Virtual-time resolution: bytes scaled by this before dividing by weight.
const VTIME_SCALE: u128 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grant {
    Granted,
    /// Not before `until` (monotonic nanoseconds).
    Wait {
        until: Nanos,
    },
    /// The peer is not registered.
    Unknown,
}

#[derive(Debug, Clone)]
struct Bucket {
    limit: Option<u64>,
    /// Changes only when the bucket switches between limited and unlimited.
    /// Outstanding grants can then be refunded after a rate adjustment but
    /// not into a later bucket epoch that never debited them.
    epoch: u64,
    /// Credit in bytes scaled by 1e9. May be negative (debt).
    credit: i128,
    last: Nanos,
}

impl Bucket {
    fn new(limit: Option<u64>, now: Nanos) -> Self {
        let limit = limit.filter(|limit| *limit > 0);
        let mut bucket = Self {
            limit,
            epoch: 0,
            credit: 0,
            last: now,
        };
        bucket.credit = bucket.capacity();
        bucket
    }

    fn capacity(&self) -> i128 {
        match self.limit {
            Some(limit) => {
                let burst = (limit / BURST_DIVISOR).max(MIN_BURST_BYTES);
                i128::from(burst) * NANOS_PER_SEC
            }
            None => 0,
        }
    }

    fn refill(&mut self, now: Nanos) {
        let Some(limit) = self.limit else {
            self.last = self.last.max(now);
            return;
        };
        let elapsed = now.saturating_sub(self.last);
        self.last = self.last.max(now);
        let refill = i128::from(limit).saturating_mul(i128::from(elapsed));
        self.credit = self.credit.saturating_add(refill).min(self.capacity());
    }

    /// Set a new limit. Credit is clamped, never raised, so an update cannot
    /// mint a burst; going from unlimited to limited starts from zero.
    fn set_limit(&mut self, limit: Option<u64>, now: Nanos) {
        self.refill(now);
        let limit = limit.filter(|limit| *limit > 0);
        let was_limited = self.limit.is_some();
        if was_limited != limit.is_some() {
            self.epoch = self
                .epoch
                .checked_add(1)
                .expect("rate bucket epoch space exhausted");
        }
        self.limit = limit;
        self.credit = if was_limited {
            self.credit.min(self.capacity())
        } else {
            0
        };
        self.last = self.last.max(now);
    }

    fn ready(&self) -> bool {
        self.limit.is_none() || self.credit >= 0
    }

    /// Instant at which credit returns to zero.
    fn ready_at(&self, now: Nanos) -> Nanos {
        match self.limit {
            Some(limit) if self.credit < 0 => {
                let deficit = self.credit.saturating_neg();
                let rate = i128::from(limit);
                let nanos = deficit.saturating_add(rate - 1) / rate;
                now.saturating_add(u64::try_from(nanos.max(1)).unwrap_or(u64::MAX))
            }
            _ => now,
        }
    }

    fn debit(&mut self, bytes: u64) {
        if self.limit.is_some() {
            let cost = i128::from(bytes).saturating_mul(NANOS_PER_SEC);
            self.credit = self.credit.saturating_sub(cost);
        }
    }

    fn credit_back(&mut self, bytes: u64) {
        if self.limit.is_some() {
            let refund = i128::from(bytes).saturating_mul(NANOS_PER_SEC);
            self.credit = self.credit.saturating_add(refund).min(self.capacity());
        }
    }
}

struct TorrentNode {
    bucket: Bucket,
    /// Active peer progress, ordered so new peers start near the fair floor.
    peers: BTreeSet<(u128, RatePeerId)>,
    /// Waiting peers ordered by progress then retry deadline.
    waiting: BTreeSet<(u128, Nanos, RatePeerId)>,
    /// Number of registered peers with at least one event-only waiter.
    change_waiters: usize,
}

struct PeerNode {
    torrent: TorrentKey,
    weight: u64,
    /// Virtual finish time of the last grant (start-time fair queueing).
    vfinish: u128,
    /// Granted bytes that may still be refunded if the payload is not used.
    refundable_bytes: u64,
    /// Limit levels debited by each unsettled grant, keyed by grant identity.
    outstanding_grants: BTreeMap<u64, OutstandingGrant>,
    /// Aggregate fairness-index state across the legacy, synchronous and
    /// independently-owned asynchronous demand lifetimes below.
    waiting: bool,
    wake_at: Nanos,
    change_waiting: bool,
    /// Pending demand registered through the pure-core compatibility API.
    legacy_wake_at: Option<Nanos>,
    /// One serialized pending demand for synchronous actor refill paths.
    sync_wake_at: Option<Nanos>,
    /// Pending asynchronous admissions, each removed by its own owner.
    demand_wake_at: BTreeMap<u64, Nanos>,
}

#[derive(Clone, Copy)]
enum WaitOwner {
    Legacy,
    Sync,
    Demand(u64),
}

#[derive(Clone, Copy)]
struct OutstandingGrant {
    bytes: u64,
    fairness_cost: u128,
    global_epoch: Option<u64>,
    torrent_epoch: Option<u64>,
}

impl OutstandingGrant {
    fn fairness_cost_for(&self, bytes: u64) -> u128 {
        if bytes >= self.bytes {
            self.fairness_cost
        } else {
            self.fairness_cost.saturating_mul(u128::from(bytes)) / u128::from(self.bytes)
        }
    }
}

pub struct RateTree {
    global: Bucket,
    torrents: HashMap<TorrentKey, TorrentNode>,
    peers: HashMap<RatePeerId, PeerNode>,
    /// Earliest waiting retry is always the first entry.
    wake_queue: BTreeSet<(Nanos, RatePeerId)>,
    next_grant_id: u64,
}

impl RateTree {
    pub fn new(global_limit: Option<u64>, now: Nanos) -> Self {
        Self {
            global: Bucket::new(global_limit, now),
            torrents: HashMap::new(),
            peers: HashMap::new(),
            wake_queue: BTreeSet::new(),
            next_grant_id: 1,
        }
    }

    /// Update the process-wide limit without minting credit.
    ///
    /// Existing wait deadlines reflect the last grant attempt. The live
    /// adapter must promptly reconsider parked requests after a limit change,
    /// since a higher limit can make them eligible before those deadlines.
    pub fn set_global_limit(&mut self, limit: Option<u64>, now: Nanos) {
        self.global.set_limit(limit, now);
    }

    pub fn add_torrent(&mut self, key: TorrentKey, limit: Option<u64>, now: Nanos) {
        if let Some(node) = self.torrents.get_mut(&key) {
            node.bucket.set_limit(limit, now);
        } else {
            let bucket = Bucket::new(limit, now);
            // Preserve peers registered before the torrent node was created.
            let peers = self
                .peers
                .iter()
                .filter(|(_, peer)| peer.torrent == key)
                .map(|(peer_id, peer)| (peer.vfinish, *peer_id))
                .collect();
            let waiting = self
                .peers
                .iter()
                .filter(|(_, peer)| peer.torrent == key && peer.waiting)
                .map(|(peer_id, peer)| (peer.vfinish, peer.wake_at, *peer_id))
                .collect();
            let change_waiters = self
                .peers
                .values()
                .filter(|peer| peer.torrent == key && peer.change_waiting)
                .count();
            self.torrents.insert(
                key,
                TorrentNode {
                    bucket,
                    peers,
                    waiting,
                    change_waiters,
                },
            );
        }
    }

    /// Update one torrent limit without minting credit. The live adapter must
    /// promptly reconsider that torrent's parked requests after a limit
    /// change because their previous deadlines may now be late.
    pub fn set_torrent_limit(&mut self, key: TorrentKey, limit: Option<u64>, now: Nanos) {
        if let Some(node) = self.torrents.get_mut(&key) {
            node.bucket.set_limit(limit, now);
        }
    }

    /// Remove a torrent and all its peer registrations after the caller has
    /// settled transferred payload. Any unsettled grants are returned to the
    /// process bucket so task teardown cannot strand global credit.
    pub fn remove_torrent(&mut self, key: TorrentKey) {
        let peer_ids: Vec<RatePeerId> = self
            .torrents
            .remove(&key)
            .map(|torrent| {
                torrent
                    .peers
                    .into_iter()
                    .map(|(_, peer_id)| peer_id)
                    .collect()
            })
            .unwrap_or_else(|| {
                self.peers
                    .iter()
                    .filter(|(_, peer)| peer.torrent == key)
                    .map(|(peer_id, _)| *peer_id)
                    .collect()
            });
        for peer_id in peer_ids {
            if let Some(peer) = self.peers.remove(&peer_id) {
                if peer.waiting {
                    self.wake_queue.remove(&(peer.wake_at, peer_id));
                }
                let refundable_global = peer
                    .outstanding_grants
                    .values()
                    .filter(|grant| {
                        grant.global_epoch == Some(self.global.epoch) && self.global.limit.is_some()
                    })
                    .fold(0u64, |total, grant| total.saturating_add(grant.bytes));
                self.global.credit_back(refundable_global);
            }
        }
    }

    /// Register a peer with a fairness weight (at least 1), or update the
    /// weight of an existing registration without discarding its progress or
    /// unsettled grants. A newcomer starts at the torrent's current virtual
    /// time, not zero, so it neither starves nor is starved by existing peers.
    pub fn add_peer(&mut self, peer_id: RatePeerId, weight: u64) {
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.weight = weight.max(1);
            return;
        }
        let floor = self.virtual_floor(peer_id.torrent).unwrap_or(0);
        self.peers.insert(
            peer_id,
            PeerNode {
                torrent: peer_id.torrent,
                weight: weight.max(1),
                vfinish: floor,
                refundable_bytes: 0,
                outstanding_grants: BTreeMap::new(),
                waiting: false,
                wake_at: 0,
                change_waiting: false,
                legacy_wake_at: None,
                sync_wake_at: None,
                demand_wake_at: BTreeMap::new(),
            },
        );
        if let Some(node) = self.torrents.get_mut(&peer_id.torrent) {
            node.peers.insert((floor, peer_id));
        }
    }

    /// Remove a registration after settling transferred payload. Any
    /// unsettled grants are returned to both applicable buckets.
    pub fn remove_peer(&mut self, peer_id: RatePeerId) {
        let Some(peer) = self.peers.remove(&peer_id) else {
            return;
        };
        let refundable_global = peer
            .outstanding_grants
            .values()
            .filter(|grant| {
                grant.global_epoch == Some(self.global.epoch) && self.global.limit.is_some()
            })
            .fold(0u64, |total, grant| total.saturating_add(grant.bytes));
        let torrent_epoch = self
            .torrents
            .get(&peer.torrent)
            .filter(|torrent| torrent.bucket.limit.is_some())
            .map(|torrent| torrent.bucket.epoch);
        let refundable_torrent = peer
            .outstanding_grants
            .values()
            .filter(|grant| grant.torrent_epoch == torrent_epoch && torrent_epoch.is_some())
            .fold(0u64, |total, grant| total.saturating_add(grant.bytes));
        self.global.credit_back(refundable_global);
        if let Some(torrent) = self.torrents.get_mut(&peer.torrent) {
            torrent.bucket.credit_back(refundable_torrent);
            torrent.peers.remove(&(peer.vfinish, peer_id));
            if peer.waiting {
                if peer.change_waiting {
                    torrent.change_waiters = torrent.change_waiters.saturating_sub(1);
                }
                torrent
                    .waiting
                    .remove(&(peer.vfinish, peer.wake_at, peer_id));
                self.wake_queue.remove(&(peer.wake_at, peer_id));
            }
        }
    }

    /// Smallest virtual finish time among waiting peers of `torrent`.
    fn virtual_floor(&self, torrent: TorrentKey) -> Option<u128> {
        let node = self.torrents.get(&torrent)?;
        node.waiting
            .first()
            .map(|(vfinish, _, _)| *vfinish)
            .or_else(|| node.peers.first().map(|(vfinish, _)| *vfinish))
    }

    fn refresh_waiting_index(&mut self, peer_id: RatePeerId) {
        let Some(peer) = self.peers.get(&peer_id) else {
            return;
        };
        let torrent_key = peer.torrent;
        let vfinish = peer.vfinish;
        let was_waiting = peer.waiting;
        let old_wake_at = peer.wake_at;
        let was_change_waiting = peer.change_waiting;
        let mut next_wake_at = peer.legacy_wake_at;
        for wake_at in peer
            .sync_wake_at
            .into_iter()
            .chain(peer.demand_wake_at.values().copied())
        {
            next_wake_at = Some(next_wake_at.map_or(wake_at, |current| current.min(wake_at)));
        }
        let is_waiting = next_wake_at.is_some();
        let wake_at = next_wake_at.unwrap_or(0);
        let is_change_waiting = peer.legacy_wake_at == Some(CHANGE_ONLY_WAKE)
            || peer.sync_wake_at == Some(CHANGE_ONLY_WAKE)
            || peer
                .demand_wake_at
                .values()
                .any(|wake_at| *wake_at == CHANGE_ONLY_WAKE);

        if was_waiting {
            if let Some(torrent) = self.torrents.get_mut(&torrent_key) {
                if was_change_waiting {
                    torrent.change_waiters = torrent.change_waiters.saturating_sub(1);
                }
                torrent.waiting.remove(&(vfinish, old_wake_at, peer_id));
            }
            self.wake_queue.remove(&(old_wake_at, peer_id));
        }
        if is_waiting {
            if let Some(torrent) = self.torrents.get_mut(&torrent_key) {
                if is_change_waiting {
                    torrent.change_waiters = torrent.change_waiters.saturating_add(1);
                }
                torrent.waiting.insert((vfinish, wake_at, peer_id));
            }
            self.wake_queue.insert((wake_at, peer_id));
        }
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.waiting = is_waiting;
            peer.wake_at = wake_at;
            peer.change_waiting = is_change_waiting;
        }
    }

    fn set_waiting_for(&mut self, peer_id: RatePeerId, owner: WaitOwner, wake_at: Nanos) {
        let Some(peer) = self.peers.get_mut(&peer_id) else {
            return;
        };
        match owner {
            WaitOwner::Legacy => peer.legacy_wake_at = Some(wake_at),
            WaitOwner::Sync => peer.sync_wake_at = Some(wake_at),
            WaitOwner::Demand(demand_id) => {
                peer.demand_wake_at.insert(demand_id, wake_at);
            }
        }
        self.refresh_waiting_index(peer_id);
    }

    fn clear_waiting_for(&mut self, peer_id: RatePeerId, owner: WaitOwner) -> bool {
        let Some(peer) = self.peers.get_mut(&peer_id) else {
            return false;
        };
        let removed = match owner {
            WaitOwner::Legacy => peer.legacy_wake_at.take().is_some(),
            WaitOwner::Sync => peer.sync_wake_at.take().is_some(),
            WaitOwner::Demand(demand_id) => peer.demand_wake_at.remove(&demand_id).is_some(),
        };
        if removed {
            self.refresh_waiting_index(peer_id);
        }
        removed
    }

    /// Retire the serialized synchronous refill demand without unregistering
    /// the live peer or touching any outstanding grant.
    pub fn retire_sync_demand(&mut self, peer_id: RatePeerId) -> bool {
        self.clear_waiting_for(peer_id, WaitOwner::Sync)
    }

    /// Retire one asynchronous admission's demand without clearing sibling
    /// waiters owned by concurrent requests from the same peer.
    pub fn retire_demand(&mut self, peer_id: RatePeerId, demand_id: u64) -> bool {
        self.clear_waiting_for(peer_id, WaitOwner::Demand(demand_id))
    }

    fn update_peer_vfinish(&mut self, peer_id: RatePeerId, vfinish: u128) {
        let Some(peer) = self.peers.get(&peer_id) else {
            return;
        };
        let (torrent_key, previous, waiting, wake_at) =
            (peer.torrent, peer.vfinish, peer.waiting, peer.wake_at);
        if previous == vfinish {
            return;
        }
        if let Some(torrent) = self.torrents.get_mut(&torrent_key) {
            torrent.peers.remove(&(previous, peer_id));
            torrent.peers.insert((vfinish, peer_id));
            if waiting {
                torrent.waiting.remove(&(previous, wake_at, peer_id));
                torrent.waiting.insert((vfinish, wake_at, peer_id));
            }
        }
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.vfinish = vfinish;
        }
    }

    /// Try to spend `bytes` for `peer_id` at every level at once.
    pub fn try_grant(&mut self, peer_id: RatePeerId, bytes: u64, now: Nanos) -> Grant {
        self.try_grant_tracked(peer_id, bytes, now).0
    }

    /// Try to spend `bytes` and return an identity for any refundable grant.
    /// The identity preserves which buckets were debited if limits change
    /// before the caller settles or refunds the payload.
    pub fn try_grant_tracked(
        &mut self,
        peer_id: RatePeerId,
        bytes: u64,
        now: Nanos,
    ) -> (Grant, Option<u64>) {
        self.try_grant_for(peer_id, bytes, now, WaitOwner::Legacy)
    }

    /// Runtime refill attempt with one persistent, serialized demand lifetime
    /// per peer. The owner retires it when the peer is choked or has no useful
    /// request to admit.
    pub fn try_grant_sync_tracked(
        &mut self,
        peer_id: RatePeerId,
        bytes: u64,
        now: Nanos,
    ) -> (Grant, Option<u64>) {
        self.try_grant_for(peer_id, bytes, now, WaitOwner::Sync)
    }

    /// Runtime asynchronous attempt with an independently-owned demand ID.
    pub fn try_grant_demand_tracked(
        &mut self,
        peer_id: RatePeerId,
        demand_id: u64,
        bytes: u64,
        now: Nanos,
    ) -> (Grant, Option<u64>) {
        self.try_grant_for(peer_id, bytes, now, WaitOwner::Demand(demand_id))
    }

    fn try_grant_for(
        &mut self,
        peer_id: RatePeerId,
        bytes: u64,
        now: Nanos,
        wait_owner: WaitOwner,
    ) -> (Grant, Option<u64>) {
        let Some(peer) = self.peers.get(&peer_id) else {
            return (Grant::Unknown, None);
        };
        let torrent_key = peer_id.torrent;
        let (weight, vfinish) = (peer.weight, peer.vfinish);
        if !self.torrents.contains_key(&torrent_key) {
            return (Grant::Unknown, None);
        }
        if bytes == 0 {
            return (Grant::Granted, None);
        }

        self.global.refill(now);
        let torrent = self.torrents.get_mut(&torrent_key).expect("checked above");
        torrent.bucket.refill(now);

        // Rate eligibility at every level; the wake time is the latest of the
        // levels that are not ready, because the grant needs all of them.
        let mut wake = now;
        let mut ready = true;
        if !self.global.ready() {
            ready = false;
            wake = wake.max(self.global.ready_at(now));
        }
        if !torrent.bucket.ready() {
            ready = false;
            wake = wake.max(torrent.bucket.ready_at(now));
        }

        // Fairness: do not run far ahead of peers still waiting on this
        // torrent's bucket. The ordered index avoids scanning every live peer
        // for each grant attempt.
        if ready {
            let laggard = self.torrents.get(&torrent_key).and_then(|torrent| {
                torrent
                    .waiting
                    .iter()
                    .find(|(_, _, other)| *other != peer_id)
                    .copied()
            });
            if let Some((floor, laggard_wake, _)) = laggard {
                let lag = FAIRNESS_LAG_BYTES * VTIME_SCALE / u128::from(weight);
                if vfinish > floor.saturating_add(lag) {
                    ready = false;
                    // A live laggard with a future refill deadline gives us a
                    // real timer. Once that deadline is due, wait for demand
                    // progress or retirement instead of re-arming at now+1ns.
                    wake = if laggard_wake > now && laggard_wake != CHANGE_ONLY_WAKE {
                        wake.max(laggard_wake)
                    } else {
                        CHANGE_ONLY_WAKE
                    };
                }
            }
        }

        if !ready {
            self.set_waiting_for(peer_id, wait_owner, wake);
            return (Grant::Wait { until: wake }, None);
        }

        let global_epoch = self.global.limit.map(|_| self.global.epoch);
        let torrent_epoch = self
            .torrents
            .get(&torrent_key)
            .and_then(|torrent| torrent.bucket.limit.map(|_| torrent.bucket.epoch));
        self.clear_waiting_for(peer_id, wait_owner);
        self.global.debit(bytes);
        self.torrents
            .get_mut(&torrent_key)
            .expect("checked above")
            .bucket
            .debit(bytes);
        let peer = self.peers.get(&peer_id).expect("peer checked above");
        let fairness_cost = u128::from(bytes) * VTIME_SCALE / u128::from(peer.weight);
        let next_vfinish = peer.vfinish.saturating_add(fairness_cost);
        self.update_peer_vfinish(peer_id, next_vfinish);
        let grant_id = self.next_grant_id;
        self.next_grant_id = self
            .next_grant_id
            .checked_add(1)
            .expect("rate grant identity space exhausted");
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.refundable_bytes = peer.refundable_bytes.saturating_add(bytes);
            peer.outstanding_grants.insert(
                grant_id,
                OutstandingGrant {
                    bytes,
                    fairness_cost,
                    global_epoch,
                    torrent_epoch,
                },
            );
            return (Grant::Granted, Some(grant_id));
        }
        (Grant::Unknown, None)
    }

    /// Return reserved bytes that will not be used (request cancelled or
    /// rejected), up to the grants not already settled or refunded. Never
    /// raises credit past the bucket capacity. A refund can make parked
    /// requests eligible before their recorded deadlines; the live adapter
    /// must promptly reconsider them.
    pub fn refund(&mut self, peer_id: RatePeerId, bytes: u64, now: Nanos) -> u64 {
        let Some(peer) = self.peers.get(&peer_id) else {
            return 0;
        };
        let grant_ids = peer.outstanding_grants.keys().copied().collect::<Vec<_>>();
        let mut remaining = bytes;
        for grant_id in grant_ids {
            if remaining == 0 {
                break;
            }
            remaining -= self.refund_grant(peer_id, grant_id, remaining, now);
        }
        bytes - remaining
    }

    /// Refund bytes from one specific grant without restoring credit to a
    /// level that was unlimited when that grant was issued.
    pub fn refund_grant(
        &mut self,
        peer_id: RatePeerId,
        grant_id: u64,
        bytes: u64,
        now: Nanos,
    ) -> u64 {
        let Some(peer) = self.peers.get(&peer_id) else {
            return 0;
        };
        let Some(grant) = peer.outstanding_grants.get(&grant_id).copied() else {
            return 0;
        };
        let refunded = bytes.min(grant.bytes);
        if refunded == 0 {
            return 0;
        }
        let torrent_key = peer.torrent;
        let fairness_cost = grant.fairness_cost_for(refunded);
        let next_vfinish = peer.vfinish.saturating_sub(fairness_cost);
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.refundable_bytes -= refunded;
            if refunded == grant.bytes {
                peer.outstanding_grants.remove(&grant_id);
            } else if let Some(grant) = peer.outstanding_grants.get_mut(&grant_id) {
                grant.bytes -= refunded;
                grant.fairness_cost = grant.fairness_cost.saturating_sub(fairness_cost);
            }
        }
        self.global.refill(now);
        if grant.global_epoch == Some(self.global.epoch) {
            self.global.credit_back(refunded);
        }
        if let Some(torrent) = self.torrents.get_mut(&torrent_key) {
            torrent.bucket.refill(now);
            if grant.torrent_epoch == Some(torrent.bucket.epoch) {
                torrent.bucket.credit_back(refunded);
            }
        }
        self.update_peer_vfinish(peer_id, next_vfinish);
        refunded
    }

    /// Mark bytes from one grant as payload sent. The grant identity keeps
    /// settlement paired with its own future refund.
    pub fn settle_grant(&mut self, peer_id: RatePeerId, grant_id: u64, bytes: u64) -> u64 {
        let Some(peer) = self.peers.get_mut(&peer_id) else {
            return 0;
        };
        let Some(grant) = peer.outstanding_grants.get(&grant_id).copied() else {
            return 0;
        };
        let settled = bytes.min(grant.bytes);
        let fairness_cost = grant.fairness_cost_for(settled);
        peer.refundable_bytes -= settled;
        if settled == grant.bytes {
            peer.outstanding_grants.remove(&grant_id);
        } else if let Some(grant) = peer.outstanding_grants.get_mut(&grant_id) {
            grant.bytes -= settled;
            grant.fairness_cost = grant.fairness_cost.saturating_sub(fairness_cost);
        }
        settled
    }

    /// Mark granted bytes as actually used so a later cancellation cannot
    /// refund service already spent on transferred payload.
    pub fn settle(&mut self, peer_id: RatePeerId, bytes: u64) -> u64 {
        let Some(peer) = self.peers.get(&peer_id) else {
            return 0;
        };
        let grant_ids = peer.outstanding_grants.keys().copied().collect::<Vec<_>>();
        let mut remaining = bytes;
        for grant_id in grant_ids {
            if remaining == 0 {
                break;
            }
            remaining -= self.settle_grant(peer_id, grant_id, remaining);
        }
        bytes - remaining
    }

    /// Whole-byte credit at the process and peer's torrent levels. This is an
    /// advisory value for request-window sizing; payload admission still
    /// reserves both levels atomically.
    pub fn available_credit_bytes(&mut self, peer_id: RatePeerId, now: Nanos) -> u64 {
        let Some(torrent_key) = self.peers.get(&peer_id).map(|peer| peer.torrent) else {
            return 0;
        };
        self.global.refill(now);
        let global_credit = bucket_available_bytes(&self.global);
        let Some(torrent) = self.torrents.get_mut(&torrent_key) else {
            return 0;
        };
        torrent.bucket.refill(now);
        global_credit.min(bucket_available_bytes(&torrent.bucket))
    }

    /// Earliest retry deadline among waiting peers, for a single timer;
    /// `None` when nobody is waiting.
    pub fn next_wake(&self) -> Option<Nanos> {
        self.wake_queue
            .iter()
            .find_map(|(wake_at, _)| (*wake_at != CHANGE_ONLY_WAKE).then_some(*wake_at))
    }

    /// Whether progress in `torrent` should notify parked fairness waiters.
    pub fn has_change_waiters(&self, torrent: TorrentKey) -> bool {
        self.torrents
            .get(&torrent)
            .is_some_and(|node| node.change_waiters > 0)
    }

    /// Return at most `limit` peers whose deadlines are due. The entries stay
    /// indexed until `try_grant` updates their wait state, so a caller can
    /// safely batch wake notifications without losing a peer if delivery is
    /// delayed.
    pub fn due_peers(&self, now: Nanos, limit: usize) -> Vec<RatePeerId> {
        self.wake_queue
            .iter()
            .take_while(|(wake_at, _)| *wake_at <= now)
            .take(limit)
            .map(|(_, peer_id)| *peer_id)
            .collect()
    }

    /// Return a bounded batch of all parked peers, ordered by retry deadline.
    /// A live adapter uses this after a limit change or refund, which can make
    /// a peer eligible before its old deadline. Entries remain indexed until
    /// `try_grant` reconciles each wait.
    pub fn waiting_peers(&self, limit: usize) -> Vec<RatePeerId> {
        self.wake_queue
            .iter()
            .take(limit)
            .map(|(_, peer_id)| *peer_id)
            .collect()
    }
}

fn bucket_available_bytes(bucket: &Bucket) -> u64 {
    match bucket.limit {
        None => u64::MAX,
        Some(_) => u64::try_from(bucket.credit.max(0) / NANOS_PER_SEC).unwrap_or(u64::MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: Nanos = 1_000_000_000;
    const BLOCK: u64 = 16 * 1024;

    fn conn(slot: u32) -> super::super::coords::ConnectionId {
        super::super::coords::ConnectionId {
            slot,
            generation: 1,
        }
    }

    fn peer(torrent: u64, slot: u32) -> RatePeerId {
        RatePeerId::new(TorrentKey(torrent), conn(slot))
    }

    /// Drive one peer for `seconds`, waiting exactly as told; return bytes granted.
    fn drain(tree: &mut RateTree, peer_id: RatePeerId, start: Nanos, seconds: u64) -> u64 {
        let end = start + seconds * SEC;
        let mut now = start;
        let mut granted = 0;
        while now < end {
            match tree.try_grant(peer_id, BLOCK, now) {
                Grant::Granted => granted += BLOCK,
                Grant::Wait { until } => {
                    assert!(until > now, "a wait must move time forward");
                    now = until;
                }
                Grant::Unknown => panic!("unknown peer"),
            }
        }
        granted
    }

    #[test]
    fn long_run_rate_matches_the_limit_exactly() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_peer(peer(1, 1), 1);
        let granted = drain(&mut tree, peer(1, 1), 0, 20);
        let expected = 20_000_000u64;
        // The initial burst (100 ms of rate, at least one block) plus one
        // block of debt are the only deviations.
        assert!(
            granted.abs_diff(expected) <= 2 * 100_000 + 2 * BLOCK,
            "granted {granted} vs {expected}"
        );
    }

    #[test]
    fn a_rate_below_one_block_per_second_still_progresses() {
        // F11: 1 KiB/s with 16 KiB blocks. The old bucket could not hold a
        // block and waited forever for tokens that could never accumulate.
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1024), 0);
        tree.add_peer(peer(1, 1), 1);
        let granted = drain(&mut tree, peer(1, 1), 0, 120);
        // About 120 KiB over two minutes: within a couple of blocks.
        assert!(
            (100 * 1024..=150 * 1024).contains(&granted),
            "granted {granted}"
        );
    }

    #[test]
    fn a_denial_names_the_exact_instant_to_retry() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(BLOCK), 0);
        tree.add_peer(peer(1, 1), 1);
        // Spend the burst, then the next grant is owed exactly after the debt.
        let now = 0;
        loop {
            match tree.try_grant(peer(1, 1), BLOCK, now) {
                Grant::Granted => {}
                Grant::Wait { until } => {
                    // Retrying one nanosecond early must still be denied.
                    assert!(matches!(
                        tree.try_grant(peer(1, 1), BLOCK, until - 1),
                        Grant::Wait { .. }
                    ));
                    assert!(tree.due_peers(until - 1, 8).is_empty());
                    assert_eq!(tree.due_peers(until, 8), vec![peer(1, 1)]);
                    assert_eq!(tree.try_grant(peer(1, 1), BLOCK, until), Grant::Granted);
                    assert!(tree.due_peers(until, 8).is_empty());
                    break;
                }
                Grant::Unknown => panic!(),
            }
        }
        assert_eq!(
            tree.next_wake(),
            None,
            "a satisfied waiter clears the timer"
        );
    }

    #[test]
    fn due_peer_batches_respect_the_bound_and_keep_unprocessed_waiters() {
        let first_waiter = peer(1, 3);
        let second_waiter = peer(2, 4);
        let mut tree = RateTree::new(Some(1024), 0);
        tree.add_torrent(TorrentKey(1), None, 0);
        tree.add_torrent(TorrentKey(2), None, 0);
        for peer_id in [peer(1, 1), peer(1, 2), first_waiter, second_waiter] {
            tree.add_peer(peer_id, 1);
        }

        assert_eq!(tree.try_grant(peer(1, 1), BLOCK, 0), Grant::Granted);
        assert_eq!(tree.try_grant(peer(1, 2), BLOCK, 0), Grant::Granted);
        let deadline = match tree.try_grant(first_waiter, BLOCK, 0) {
            Grant::Wait { until } => until,
            other => panic!("expected first waiter to be rate limited, got {other:?}"),
        };
        assert_eq!(
            tree.try_grant(second_waiter, BLOCK, 0),
            Grant::Wait { until: deadline }
        );

        assert!(tree.due_peers(deadline, 0).is_empty());
        assert_eq!(tree.due_peers(deadline, 1), vec![first_waiter]);
        assert_eq!(
            tree.due_peers(deadline, 2),
            vec![first_waiter, second_waiter]
        );

        assert_eq!(
            tree.try_grant(first_waiter, BLOCK, deadline),
            Grant::Granted
        );
        assert_eq!(tree.due_peers(deadline, 1), vec![second_waiter]);
    }

    #[test]
    fn parked_peers_can_be_reconsidered_in_bounded_batches_after_limit_change() {
        let first_waiter = peer(1, 1);
        let second_waiter = peer(2, 1);
        let mut tree = RateTree::new(Some(1024), 0);
        tree.add_torrent(TorrentKey(1), None, 0);
        tree.add_torrent(TorrentKey(2), None, 0);
        tree.add_peer(first_waiter, 1);
        tree.add_peer(second_waiter, 1);

        assert_eq!(tree.try_grant(first_waiter, BLOCK, 0), Grant::Granted);
        assert_eq!(tree.try_grant(second_waiter, BLOCK, 0), Grant::Granted);
        assert!(matches!(
            tree.try_grant(first_waiter, BLOCK, 0),
            Grant::Wait { .. }
        ));
        assert!(matches!(
            tree.try_grant(second_waiter, BLOCK, 0),
            Grant::Wait { .. }
        ));

        tree.set_global_limit(None, 0);
        assert!(tree.due_peers(0, 2).is_empty(), "old deadlines are stale");
        assert!(tree.waiting_peers(0).is_empty());
        assert_eq!(tree.waiting_peers(1), vec![first_waiter]);
        assert_eq!(tree.waiting_peers(2), vec![first_waiter, second_waiter]);

        assert_eq!(tree.try_grant(first_waiter, BLOCK, 0), Grant::Granted);
        assert_eq!(tree.waiting_peers(1), vec![second_waiter]);
        assert_eq!(tree.try_grant(second_waiter, BLOCK, 0), Grant::Granted);
        assert!(tree.waiting_peers(2).is_empty());
    }

    #[test]
    fn peers_share_one_torrent_allowance_instead_of_each_getting_it() {
        // F09: N peers must together stay at the torrent limit.
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        for slot in 1..=4 {
            tree.add_peer(peer(1, slot), 1);
        }
        let mut now = 0;
        let mut total = 0u64;
        while now < 10 * SEC {
            let mut next = now + SEC / 100;
            for slot in 1..=4 {
                match tree.try_grant(peer(1, slot), BLOCK, now) {
                    Grant::Granted => total += BLOCK,
                    Grant::Wait { until } => next = next.min(until.max(now + 1)),
                    Grant::Unknown => panic!(),
                }
            }
            now = next;
        }
        assert!(
            total <= 10 * 1_000_000 + 4 * BLOCK + 100_000,
            "4 peers moved {total} bytes against a 1 MB/s torrent limit"
        );
        assert!(total >= 8_000_000, "and still used most of it: {total}");
    }

    #[test]
    fn reused_ledger_connections_are_isolated_between_torrents() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), None, 0);
        tree.add_torrent(TorrentKey(2), None, 0);
        let first_torrent_peer = peer(1, 1);
        let second_torrent_peer = peer(2, 1);
        assert_eq!(
            first_torrent_peer.connection, second_torrent_peer.connection,
            "each torrent's local ledger may use the same slot and generation"
        );
        tree.add_peer(first_torrent_peer, 1);
        tree.add_peer(second_torrent_peer, 1);

        assert_eq!(tree.peers.len(), 2);
        assert_eq!(tree.try_grant(first_torrent_peer, BLOCK, 0), Grant::Granted);
        assert_eq!(
            tree.try_grant(second_torrent_peer, BLOCK, 0),
            Grant::Granted
        );

        tree.remove_torrent(TorrentKey(1));
        assert_eq!(tree.try_grant(first_torrent_peer, 1, 0), Grant::Unknown);
        assert_eq!(tree.try_grant(second_torrent_peer, 1, 0), Grant::Granted);
    }

    #[test]
    fn a_grant_is_all_or_nothing_across_levels() {
        let mut tree = RateTree::new(Some(10 * BLOCK), 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_torrent(TorrentKey(2), None, 0);
        tree.add_peer(peer(1, 1), 1);
        tree.add_peer(peer(2, 2), 1);
        // Exhaust the global bucket through the unlimited torrent.
        let mut now = 0;
        while matches!(tree.try_grant(peer(2, 2), BLOCK, now), Grant::Granted) {
            now += 1;
        }
        // Torrent 1 has plenty of its own credit, but the global level denies,
        // and the denial must not have debited torrent 1's bucket.
        let before = tree.torrents[&TorrentKey(1)].bucket.credit;
        assert!(matches!(
            tree.try_grant(peer(1, 1), BLOCK, now),
            Grant::Wait { .. }
        ));
        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, before);
    }

    #[test]
    fn a_limit_update_never_mints_a_burst() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_peer(peer(1, 1), 1);
        // Drain the bucket into debt.
        let now = 0;
        while matches!(tree.try_grant(peer(1, 1), BLOCK, now), Grant::Granted) {}
        let before = tree.torrents[&TorrentKey(1)].bucket.credit;
        assert!(before < 0);
        // Raising the limit must not forgive the debt or add credit.
        tree.set_torrent_limit(TorrentKey(1), Some(50_000_000), now);
        assert!(tree.torrents[&TorrentKey(1)].bucket.credit <= 0);
        // Removing the limit and re-imposing it starts from zero, not full.
        tree.set_torrent_limit(TorrentKey(1), None, now);
        tree.set_torrent_limit(TorrentKey(1), Some(1_000_000), now);
        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, 0);
    }

    #[test]
    fn an_unlimited_grant_cannot_refund_into_a_new_limited_epoch() {
        let peer_id = peer(1, 1);
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), None, 0);
        tree.add_peer(peer_id, 1);
        let (grant, grant_id) = tree.try_grant_tracked(peer_id, BLOCK, 0);
        assert_eq!(grant, Grant::Granted);

        tree.set_global_limit(Some(BLOCK), 0);
        tree.set_torrent_limit(TorrentKey(1), Some(BLOCK), 0);
        let global_credit = tree.global.credit;
        let torrent_credit = tree.torrents[&TorrentKey(1)].bucket.credit;

        assert_eq!(
            tree.refund_grant(peer_id, grant_id.unwrap(), BLOCK, 0),
            BLOCK
        );
        assert_eq!(tree.global.credit, global_credit);
        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, torrent_credit);
        assert!(matches!(tree.try_grant(peer_id, BLOCK, 0), Grant::Granted));
        assert!(matches!(tree.try_grant(peer_id, 1, 0), Grant::Wait { .. }));
    }

    #[test]
    fn removing_peer_does_not_refund_old_grants_into_a_new_limit_epoch() {
        let peer_id = peer(1, 1);
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), None, 0);
        tree.add_peer(peer_id, 1);
        assert_eq!(tree.try_grant(peer_id, BLOCK, 0), Grant::Granted);

        tree.set_global_limit(Some(BLOCK), 0);
        tree.set_torrent_limit(TorrentKey(1), Some(BLOCK), 0);
        tree.remove_peer(peer_id);

        assert_eq!(tree.global.credit, 0);
        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, 0);
    }

    #[test]
    fn readding_a_torrent_limit_does_not_reset_existing_debt() {
        let torrent = TorrentKey(1);
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(torrent, Some(BLOCK), 0);
        tree.add_peer(peer(1, 1), 1);
        assert_eq!(tree.try_grant(peer(1, 1), BLOCK, 0), Grant::Granted);
        assert_eq!(tree.try_grant(peer(1, 1), BLOCK, 0), Grant::Granted);
        let debt = tree.torrents[&torrent].bucket.credit;
        assert!(debt < 0);

        tree.add_torrent(torrent, Some(50 * BLOCK), 0);

        assert_eq!(tree.torrents[&torrent].bucket.credit, debt);
    }

    #[test]
    fn updating_a_peer_weight_preserves_unsettled_grants_and_progress() {
        let peer_id = peer(1, 1);
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_peer(peer_id, 1);
        assert_eq!(tree.try_grant(peer_id, BLOCK, 0), Grant::Granted);
        let vfinish = tree.peers[&peer_id].vfinish;

        tree.add_peer(peer_id, 3);

        assert_eq!(tree.peers[&peer_id].weight, 3);
        assert_eq!(tree.peers[&peer_id].vfinish, vfinish);
        assert_eq!(tree.peers[&peer_id].refundable_bytes, BLOCK);
        tree.refund(peer_id, BLOCK, 0);
        assert_eq!(tree.peers[&peer_id].refundable_bytes, 0);
        assert_eq!(tree.peers[&peer_id].vfinish, 0);
    }

    #[test]
    fn weighted_peers_split_a_saturated_bucket_by_weight() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(2_000_000), 0);
        tree.add_peer(peer(1, 1), 1);
        tree.add_peer(peer(1, 2), 3);
        let mut now = 0u64;
        let mut totals = [0u64; 2];
        while now < 30 * SEC {
            let mut next = now + 1_000_000;
            for (index, slot) in [1u32, 2].iter().enumerate() {
                if tree.try_grant(peer(1, *slot), BLOCK, now) == Grant::Granted {
                    totals[index] += BLOCK;
                }
            }
            if let Some(wake) = tree.next_wake() {
                next = next.min(wake.max(now + 1));
            }
            now = next.max(now + 1);
        }
        let ratio = totals[1] as f64 / totals[0] as f64;
        assert!(
            (2.4..=3.6).contains(&ratio),
            "weights 1:3 gave {totals:?} (ratio {ratio})"
        );
    }

    #[test]
    fn a_new_peer_does_not_get_a_head_start_or_penalty() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_peer(peer(1, 1), 1);
        let mut now = 0;
        // The veteran runs for a while and builds up virtual time.
        while now < 5 * SEC {
            if let Grant::Wait { until } = tree.try_grant(peer(1, 1), BLOCK, now) {
                now = until;
            }
        }
        tree.add_peer(peer(1, 2), 1);
        let veteran = tree.peers[&peer(1, 1)].vfinish;
        let newcomer = tree.peers[&peer(1, 2)].vfinish;
        assert!(
            newcomer + FAIRNESS_LAG_BYTES * VTIME_SCALE >= veteran,
            "newcomer starts near the floor"
        );
    }

    #[test]
    fn refund_returns_credit_without_exceeding_capacity() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_peer(peer(1, 1), 1);
        assert_eq!(tree.try_grant(peer(1, 1), BLOCK, 0), Grant::Granted);
        let spent = tree.torrents[&TorrentKey(1)].bucket.credit;
        tree.refund(peer(1, 1), BLOCK, 0);
        assert!(tree.torrents[&TorrentKey(1)].bucket.credit > spent);
        let capacity = tree.torrents[&TorrentKey(1)].bucket.capacity();
        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, capacity);
        tree.refund(peer(1, 1), 100 * BLOCK, 0);
        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, capacity);
        let vfinish = tree.peers[&peer(1, 1)].vfinish;
        assert!(tree.torrents[&TorrentKey(1)]
            .peers
            .contains(&(vfinish, peer(1, 1))));
    }

    #[test]
    fn settled_payload_bytes_cannot_be_refunded() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_peer(peer(1, 1), 1);
        assert_eq!(tree.try_grant(peer(1, 1), BLOCK, 0), Grant::Granted);
        assert_eq!(tree.settle(peer(1, 1), BLOCK), BLOCK);
        let spent = tree.torrents[&TorrentKey(1)].bucket.credit;

        tree.refund(peer(1, 1), 100 * BLOCK, 0);

        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, spent);
        assert_eq!(tree.peers[&peer(1, 1)].refundable_bytes, 0);
    }

    #[test]
    fn fairness_wait_uses_the_laggards_deadline_and_indexes_cleanup() {
        let torrent = TorrentKey(1);
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(torrent, Some(1_000_000), 0);
        tree.add_peer(peer(torrent.0, 1), 1);
        tree.add_peer(peer(torrent.0, 2), 1);

        // Let the first peer run more than the two-block fairness allowance.
        for _ in 0..7 {
            assert_eq!(tree.try_grant(peer(1, 1), BLOCK, 0), Grant::Granted);
        }
        let laggard_wake = match tree.try_grant(peer(1, 2), BLOCK, 0) {
            Grant::Wait { until } => until,
            other => panic!("expected a rate wait, got {other:?}"),
        };
        assert_eq!(tree.next_wake(), Some(laggard_wake));

        assert_eq!(
            tree.try_grant(peer(1, 1), BLOCK, laggard_wake),
            Grant::Wait {
                until: CHANGE_ONLY_WAKE
            },
            "fairness waits for peer progress instead of polling on a timer"
        );
        assert_eq!(tree.next_wake(), Some(laggard_wake));
        assert!(tree.has_change_waiters(torrent));
        assert_eq!(
            tree.try_grant(peer(1, 2), BLOCK, laggard_wake),
            Grant::Granted
        );
        assert!(matches!(
            tree.try_grant(peer(1, 1), BLOCK, laggard_wake),
            Grant::Wait { .. }
        ));

        tree.remove_peer(peer(1, 1));
        assert_eq!(tree.next_wake(), None);
        let vfinish = tree.peers[&peer(1, 2)].vfinish;
        assert!(tree.torrents[&torrent]
            .peers
            .contains(&(vfinish, peer(1, 2))));
    }

    #[test]
    fn retiring_a_live_sync_demand_unblocks_funded_peers_without_disconnect() {
        let torrent = TorrentKey(1);
        let active = peer(1, 1);
        let parked = peer(1, 2);
        let mut tree = RateTree::new(Some(32 * 1024 * 1024), 0);
        tree.add_torrent(torrent, Some(32 * 1024 * 1024), 0);
        tree.add_peer(active, 1);
        tree.add_peer(parked, 1);

        while let (Grant::Granted, Some(grant_id)) = tree.try_grant_sync_tracked(active, BLOCK, 0) {
            assert_eq!(tree.settle_grant(active, grant_id, BLOCK), BLOCK);
        }
        assert!(matches!(
            tree.try_grant_sync_tracked(parked, BLOCK, 0).0,
            Grant::Wait { .. }
        ));
        assert!(tree.waiting_peers(8).contains(&parked));

        // Make rate credit available while both peers remain registered. The
        // parked peer must stop participating once its owner retires demand.
        tree.set_global_limit(None, 1_000_000_000);
        tree.set_torrent_limit(torrent, None, 1_000_000_000);
        assert!(matches!(
            tree.try_grant_sync_tracked(active, BLOCK, 1_000_000_000).0,
            Grant::Wait { .. }
        ));
        assert!(tree.retire_sync_demand(parked));
        for _ in 0..1_000 {
            let (grant, grant_id) = tree.try_grant_sync_tracked(active, BLOCK, 1_000_000_000);
            assert_eq!(grant, Grant::Granted);
            assert_eq!(tree.settle_grant(active, grant_id.unwrap(), BLOCK), BLOCK);
        }
        assert!(tree.peers.contains_key(&parked));
        assert!(!tree.waiting_peers(8).contains(&parked));
    }

    #[test]
    fn retiring_one_demand_keeps_another_demand_for_the_same_peer() {
        let torrent = TorrentKey(1);
        let peer_id = peer(1, 1);
        let mut tree = RateTree::new(Some(1), 0);
        tree.add_torrent(torrent, Some(1), 0);
        tree.add_peer(peer_id, 1);
        assert_eq!(tree.try_grant(peer_id, BLOCK + 1, 0), Grant::Granted);

        assert!(matches!(
            tree.try_grant_demand_tracked(peer_id, 10, 1, 0).0,
            Grant::Wait { .. }
        ));
        assert!(matches!(
            tree.try_grant_demand_tracked(peer_id, 11, 1, 0).0,
            Grant::Wait { .. }
        ));
        assert!(tree.retire_demand(peer_id, 10));
        assert_eq!(tree.waiting_peers(8), vec![peer_id]);
        assert!(tree.retire_demand(peer_id, 11));
        assert!(tree.waiting_peers(8).is_empty());
    }

    #[test]
    fn a_timer_wait_does_not_hide_a_sibling_event_only_demand() {
        let torrent = TorrentKey(1);
        let peer_id = peer(1, 1);
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(torrent, None, 0);
        tree.add_peer(peer_id, 1);

        tree.set_waiting_for(peer_id, WaitOwner::Demand(10), CHANGE_ONLY_WAKE);
        tree.set_waiting_for(peer_id, WaitOwner::Demand(11), 100);

        assert!(tree.has_change_waiters(torrent));
        assert_eq!(tree.next_wake(), Some(100));
        assert!(tree.retire_demand(peer_id, 10));
        assert!(!tree.has_change_waiters(torrent));
        assert_eq!(tree.next_wake(), Some(100));
        assert_eq!(tree.waiting_peers(1), vec![peer_id]);
    }

    #[test]
    fn unknown_and_removed_peers_are_reported() {
        let mut tree = RateTree::new(None, 0);
        assert_eq!(tree.try_grant(peer(1, 9), 1, 0), Grant::Unknown);
        tree.add_torrent(TorrentKey(1), None, 0);
        tree.add_peer(peer(1, 1), 1);
        tree.remove_peer(peer(1, 1));
        assert_eq!(tree.try_grant(peer(1, 1), 1, 0), Grant::Unknown);
        tree.add_peer(peer(1, 2), 1);
        tree.remove_torrent(TorrentKey(1));
        assert_eq!(tree.try_grant(peer(1, 2), 1, 0), Grant::Unknown);
    }
}
