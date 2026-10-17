//! Hierarchical global / torrent / peer rate scheduler (NET-107).
//!
//! The previous design copied the torrent limit into a token bucket per peer,
//! so N peers could each spend the full torrent allowance; refilling had no
//! wakeup when tokens became available again; and a limit update re-minted a
//! full burst in every connection. This scheduler replaces all of that with
//! one tree:
//!
//! * a grant succeeds **atomically** at every level (global, torrent, peer
//!   fairness) or at none, and a denial reports the exact monotonic instant
//!   at which it can succeed, so callers sleep until then instead of polling;
//! * credit is kept in fractional byte-nanoseconds, and a grant may drive a
//!   bucket into debt, so a rate below one block per second still progresses
//!   at exactly its configured long-run rate;
//! * changing a limit never *adds* credit: it can only clamp it, so a limit
//!   update cannot mint a fresh burst anywhere in the tree;
//! * peers under one torrent share its bucket by weighted start-time fair
//!   queueing, so one fast peer cannot starve the others;
//! * time is injected (`Nanos`), never read.

use std::collections::HashMap;

use super::coords::ConnectionId;
use super::Nanos;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TorrentKey(pub u32);

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
    /// Credit in bytes scaled by 1e9. May be negative (debt).
    credit: i128,
    last: Nanos,
}

impl Bucket {
    fn new(limit: Option<u64>, now: Nanos) -> Self {
        let limit = limit.filter(|limit| *limit > 0);
        let mut bucket = Self {
            limit,
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
            self.last = now;
            return;
        };
        let elapsed = now.saturating_sub(self.last);
        self.last = self.last.max(now);
        self.credit = (self.credit + i128::from(limit) * i128::from(elapsed)).min(self.capacity());
    }

    /// Set a new limit. Credit is clamped, never raised, so an update cannot
    /// mint a burst; going from unlimited to limited starts from zero.
    fn set_limit(&mut self, limit: Option<u64>, now: Nanos) {
        self.refill(now);
        let limit = limit.filter(|limit| *limit > 0);
        let was_limited = self.limit.is_some();
        self.limit = limit;
        self.credit = if was_limited {
            self.credit.min(self.capacity())
        } else {
            0
        };
        self.last = now;
    }

    fn ready(&self) -> bool {
        self.limit.is_none() || self.credit >= 0
    }

    /// Instant at which credit returns to zero.
    fn ready_at(&self, now: Nanos) -> Nanos {
        match self.limit {
            Some(limit) if self.credit < 0 => {
                let deficit = -self.credit;
                let nanos = (deficit + i128::from(limit) - 1) / i128::from(limit);
                now.saturating_add(nanos.max(1) as u64)
            }
            _ => now,
        }
    }

    fn debit(&mut self, bytes: u64) {
        if self.limit.is_some() {
            self.credit -= i128::from(bytes) * NANOS_PER_SEC;
        }
    }

    fn credit_back(&mut self, bytes: u64) {
        if self.limit.is_some() {
            self.credit = (self.credit + i128::from(bytes) * NANOS_PER_SEC).min(self.capacity());
        }
    }
}

struct TorrentNode {
    bucket: Bucket,
}

struct PeerNode {
    torrent: TorrentKey,
    weight: u64,
    /// Virtual finish time of the last grant (start-time fair queueing).
    vfinish: u128,
    /// Denied and still wanting service.
    waiting: bool,
    wake_at: Nanos,
}

pub struct RateTree {
    global: Bucket,
    torrents: HashMap<TorrentKey, TorrentNode>,
    peers: HashMap<ConnectionId, PeerNode>,
}

impl RateTree {
    pub fn new(global_limit: Option<u64>, now: Nanos) -> Self {
        Self {
            global: Bucket::new(global_limit, now),
            torrents: HashMap::new(),
            peers: HashMap::new(),
        }
    }

    pub fn set_global_limit(&mut self, limit: Option<u64>, now: Nanos) {
        self.global.set_limit(limit, now);
    }

    pub fn add_torrent(&mut self, key: TorrentKey, limit: Option<u64>, now: Nanos) {
        self.torrents.insert(
            key,
            TorrentNode {
                bucket: Bucket::new(limit, now),
            },
        );
    }

    pub fn set_torrent_limit(&mut self, key: TorrentKey, limit: Option<u64>, now: Nanos) {
        if let Some(node) = self.torrents.get_mut(&key) {
            node.bucket.set_limit(limit, now);
        }
    }

    pub fn remove_torrent(&mut self, key: TorrentKey) {
        self.torrents.remove(&key);
        self.peers.retain(|_, peer| peer.torrent != key);
    }

    /// Register a peer under `torrent` with a fairness weight (at least 1). A
    /// newcomer starts at the torrent's current virtual time, not zero, so it
    /// neither starves nor is starved by existing peers.
    pub fn add_peer(&mut self, torrent: TorrentKey, conn: ConnectionId, weight: u64) {
        let floor = self.virtual_floor(torrent).unwrap_or(0);
        self.peers.insert(
            conn,
            PeerNode {
                torrent,
                weight: weight.max(1),
                vfinish: floor,
                waiting: false,
                wake_at: 0,
            },
        );
    }

    pub fn remove_peer(&mut self, conn: ConnectionId) {
        self.peers.remove(&conn);
    }

    /// Smallest virtual finish time among waiting peers of `torrent`.
    fn virtual_floor(&self, torrent: TorrentKey) -> Option<u128> {
        self.peers
            .values()
            .filter(|peer| peer.torrent == torrent && peer.waiting)
            .map(|peer| peer.vfinish)
            .min()
            .or_else(|| {
                self.peers
                    .values()
                    .filter(|peer| peer.torrent == torrent)
                    .map(|peer| peer.vfinish)
                    .min()
            })
    }

    /// Try to spend `bytes` for `conn` at every level at once.
    pub fn try_grant(&mut self, conn: ConnectionId, bytes: u64, now: Nanos) -> Grant {
        let Some(peer) = self.peers.get(&conn) else {
            return Grant::Unknown;
        };
        let torrent_key = peer.torrent;
        let (weight, vfinish) = (peer.weight, peer.vfinish);
        if !self.torrents.contains_key(&torrent_key) {
            return Grant::Unknown;
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
        // torrent's bucket.
        if ready {
            let floor = self
                .peers
                .iter()
                .filter(|(other, node)| {
                    **other != conn && node.torrent == torrent_key && node.waiting
                })
                .map(|(_, node)| node.vfinish)
                .min();
            if let Some(floor) = floor {
                let lag = FAIRNESS_LAG_BYTES * VTIME_SCALE / u128::from(weight);
                if vfinish > floor.saturating_add(lag) {
                    ready = false;
                    // Retry once the bucket could have served the laggard.
                    wake = wake.max(now.saturating_add(1_000_000));
                }
            }
        }

        let peer = self.peers.get_mut(&conn).expect("peer checked above");
        if !ready {
            peer.waiting = true;
            peer.wake_at = wake;
            return Grant::Wait { until: wake };
        }

        self.global.debit(bytes);
        self.torrents
            .get_mut(&torrent_key)
            .expect("checked above")
            .bucket
            .debit(bytes);
        let peer = self.peers.get_mut(&conn).expect("peer checked above");
        peer.waiting = false;
        peer.vfinish = peer
            .vfinish
            .saturating_add(u128::from(bytes) * VTIME_SCALE / u128::from(peer.weight));
        Grant::Granted
    }

    /// Return reserved bytes that will not be used (request cancelled or
    /// rejected). Never raises credit past the bucket capacity.
    pub fn refund(&mut self, conn: ConnectionId, bytes: u64, now: Nanos) {
        let Some(peer) = self.peers.get(&conn) else {
            return;
        };
        let torrent_key = peer.torrent;
        self.global.refill(now);
        self.global.credit_back(bytes);
        if let Some(torrent) = self.torrents.get_mut(&torrent_key) {
            torrent.bucket.refill(now);
            torrent.bucket.credit_back(bytes);
        }
        if let Some(peer) = self.peers.get_mut(&conn) {
            peer.vfinish = peer
                .vfinish
                .saturating_sub(u128::from(bytes) * VTIME_SCALE / u128::from(peer.weight));
        }
    }

    /// Earliest instant any waiting peer could be granted, for a single
    /// timer; `None` when nobody is waiting.
    pub fn next_wake(&self) -> Option<Nanos> {
        self.peers
            .values()
            .filter(|peer| peer.waiting)
            .map(|peer| peer.wake_at)
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEC: Nanos = 1_000_000_000;
    const BLOCK: u64 = 16 * 1024;

    fn conn(slot: u32) -> ConnectionId {
        ConnectionId {
            slot,
            generation: 1,
        }
    }

    /// Drive `conn` for `seconds`, waiting exactly as told; return bytes granted.
    fn drain(tree: &mut RateTree, conn: ConnectionId, start: Nanos, seconds: u64) -> u64 {
        let end = start + seconds * SEC;
        let mut now = start;
        let mut granted = 0;
        while now < end {
            match tree.try_grant(conn, BLOCK, now) {
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
        tree.add_peer(TorrentKey(1), conn(1), 1);
        let granted = drain(&mut tree, conn(1), 0, 20);
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
        tree.add_peer(TorrentKey(1), conn(1), 1);
        let granted = drain(&mut tree, conn(1), 0, 120);
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
        tree.add_peer(TorrentKey(1), conn(1), 1);
        // Spend the burst, then the next grant is owed exactly after the debt.
        let now = 0;
        loop {
            match tree.try_grant(conn(1), BLOCK, now) {
                Grant::Granted => {}
                Grant::Wait { until } => {
                    // Retrying one nanosecond early must still be denied.
                    assert!(matches!(
                        tree.try_grant(conn(1), BLOCK, until - 1),
                        Grant::Wait { .. }
                    ));
                    assert_eq!(tree.try_grant(conn(1), BLOCK, until), Grant::Granted);
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
    fn peers_share_one_torrent_allowance_instead_of_each_getting_it() {
        // F09: N peers must together stay at the torrent limit.
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        for slot in 1..=4 {
            tree.add_peer(TorrentKey(1), conn(slot), 1);
        }
        let mut now = 0;
        let mut total = 0u64;
        while now < 10 * SEC {
            let mut next = now + SEC / 100;
            for slot in 1..=4 {
                match tree.try_grant(conn(slot), BLOCK, now) {
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
    fn a_grant_is_all_or_nothing_across_levels() {
        let mut tree = RateTree::new(Some(10 * BLOCK), 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_torrent(TorrentKey(2), None, 0);
        tree.add_peer(TorrentKey(1), conn(1), 1);
        tree.add_peer(TorrentKey(2), conn(2), 1);
        // Exhaust the global bucket through the unlimited torrent.
        let mut now = 0;
        while matches!(tree.try_grant(conn(2), BLOCK, now), Grant::Granted) {
            now += 1;
        }
        // Torrent 1 has plenty of its own credit, but the global level denies,
        // and the denial must not have debited torrent 1's bucket.
        let before = tree.torrents[&TorrentKey(1)].bucket.credit;
        assert!(matches!(
            tree.try_grant(conn(1), BLOCK, now),
            Grant::Wait { .. }
        ));
        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, before);
    }

    #[test]
    fn a_limit_update_never_mints_a_burst() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_peer(TorrentKey(1), conn(1), 1);
        // Drain the bucket into debt.
        let now = 0;
        while matches!(tree.try_grant(conn(1), BLOCK, now), Grant::Granted) {}
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
    fn weighted_peers_split_a_saturated_bucket_by_weight() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(2_000_000), 0);
        tree.add_peer(TorrentKey(1), conn(1), 1);
        tree.add_peer(TorrentKey(1), conn(2), 3);
        let mut now = 0u64;
        let mut totals = [0u64; 2];
        while now < 30 * SEC {
            let mut next = now + 1_000_000;
            for (index, slot) in [1u32, 2].iter().enumerate() {
                if tree.try_grant(conn(*slot), BLOCK, now) == Grant::Granted {
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
        tree.add_peer(TorrentKey(1), conn(1), 1);
        let mut now = 0;
        // The veteran runs for a while and builds up virtual time.
        while now < 5 * SEC {
            if let Grant::Wait { until } = tree.try_grant(conn(1), BLOCK, now) {
                now = until;
            }
        }
        tree.add_peer(TorrentKey(1), conn(2), 1);
        let veteran = tree.peers[&conn(1)].vfinish;
        let newcomer = tree.peers[&conn(2)].vfinish;
        assert!(
            newcomer + FAIRNESS_LAG_BYTES * VTIME_SCALE >= veteran,
            "newcomer starts near the floor"
        );
    }

    #[test]
    fn refund_returns_credit_without_exceeding_capacity() {
        let mut tree = RateTree::new(None, 0);
        tree.add_torrent(TorrentKey(1), Some(1_000_000), 0);
        tree.add_peer(TorrentKey(1), conn(1), 1);
        assert_eq!(tree.try_grant(conn(1), BLOCK, 0), Grant::Granted);
        let spent = tree.torrents[&TorrentKey(1)].bucket.credit;
        tree.refund(conn(1), BLOCK, 0);
        assert!(tree.torrents[&TorrentKey(1)].bucket.credit > spent);
        tree.refund(conn(1), 100 * BLOCK, 0);
        let capacity = tree.torrents[&TorrentKey(1)].bucket.capacity();
        assert_eq!(tree.torrents[&TorrentKey(1)].bucket.credit, capacity);
    }

    #[test]
    fn unknown_and_removed_peers_are_reported() {
        let mut tree = RateTree::new(None, 0);
        assert_eq!(tree.try_grant(conn(9), 1, 0), Grant::Unknown);
        tree.add_torrent(TorrentKey(1), None, 0);
        tree.add_peer(TorrentKey(1), conn(1), 1);
        tree.remove_peer(conn(1));
        assert_eq!(tree.try_grant(conn(1), 1, 0), Grant::Unknown);
        tree.add_peer(TorrentKey(1), conn(2), 1);
        tree.remove_torrent(TorrentKey(1));
        assert_eq!(tree.try_grant(conn(2), 1, 0), Grant::Unknown);
    }
}
