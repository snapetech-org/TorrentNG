/// Upload slot manager: BEP 3 choking algorithm.
///
/// Slots are earned by the feedback that fits what the client is doing:
/// while downloading, peers that deliver the most to us (reciprocation);
/// while seeding, peers we can serve fastest. One extra optimistic unchoke
/// slot rotates every 30 seconds, round-robin over peers that have waited
/// longest with random tie-breaks, so exploration does not always pick the
/// first idle peer.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::peer::{ChokeState, PeerId};

/// Maximum regular unchoke slots (excludes the optimistic slot).
pub const DEFAULT_MAX_UNCHOKED: usize = 4;

/// Interval at which the optimistic unchoke slot rotates.
pub const OPTIMISTIC_ROTATE_INTERVAL: Duration = Duration::from_secs(30);

/// Decision produced by the choker for a single peer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChokeDecision {
    Unchoke,
    Choke,
}

/// Lightweight snapshot of a peer used for choking decisions.
#[derive(Debug, Clone)]
pub struct PeerSnapshot {
    pub id: PeerId,
    pub interested: bool,
    /// Recent bytes/s we sent this peer (decayed by the caller when idle).
    pub upload_rate: f64,
    /// Recent bytes/s this peer delivered to us (decayed by the caller when idle).
    pub download_rate: f64,
    pub current_choke: ChokeState,
}

/// What the client is doing, which decides which rate earns a slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChokeMode {
    /// Still downloading: reward peers that upload to us.
    Downloading,
    /// Complete: reward peers we serve fastest.
    Seeding,
}

/// Choking algorithm state.
pub struct Choker {
    max_unchoked: usize,
    optimistic_id: Option<PeerId>,
    last_optimistic_rotate: Instant,
    /// When each peer last held the optimistic slot, for fair rotation.
    optimistic_history: HashMap<PeerId, Instant>,
    random_state: u64,
}

impl Choker {
    pub fn new(max_unchoked: usize) -> Self {
        Choker {
            max_unchoked,
            optimistic_id: None,
            last_optimistic_rotate: Instant::now(),
            optimistic_history: HashMap::new(),
            random_state: Uuid::new_v4().as_u128() as u64,
        }
    }

    /// Construct a repeatable choker for deterministic swarm-trace replay.
    pub fn with_seed(max_unchoked: usize, seed: u64) -> Self {
        Self {
            max_unchoked,
            optimistic_id: None,
            last_optimistic_rotate: Instant::now(),
            optimistic_history: HashMap::new(),
            random_state: seed,
        }
    }

    fn random_index(&mut self, upper_bound: usize) -> usize {
        debug_assert!(upper_bound > 0);
        // SplitMix64 keeps the policy dependency-free and makes seeded traces
        // replayable across runs.
        self.random_state = self.random_state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = self.random_state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        value ^= value >> 31;
        ((u128::from(value) * upper_bound as u128) >> 64) as usize
    }

    /// Run one choking round in seeding mode (rank by upload rate).
    pub fn run(&mut self, peers: &[PeerSnapshot]) -> HashMap<PeerId, ChokeDecision> {
        self.run_with_mode(peers, ChokeMode::Seeding)
    }

    /// Run one choking round. Returns a map of peer → decision.
    ///
    /// Only peers that are `interested` compete for unchoke slots.
    /// Peers that are not interested are always choked.
    pub fn run_with_mode(
        &mut self,
        peers: &[PeerSnapshot],
        mode: ChokeMode,
    ) -> HashMap<PeerId, ChokeDecision> {
        self.run_with_mode_at(peers, mode, Instant::now())
    }

    /// Run one round at a caller-supplied monotonic time. Production callers
    /// use [`run_with_mode`](Self::run_with_mode); trace replayers provide a
    /// fixed timeline so rotation and fairness decisions are reproducible.
    pub fn run_with_mode_at(
        &mut self,
        peers: &[PeerSnapshot],
        mode: ChokeMode,
        now: Instant,
    ) -> HashMap<PeerId, ChokeDecision> {
        // Only interested peers compete.
        let mut interested: Vec<&PeerSnapshot> = peers.iter().filter(|p| p.interested).collect();

        // `upload_rate` is a public field on a struct producers outside this
        // crate can construct; a NaN rate must not panic the choke loop, so
        // treat it as unordered rather than unwrapping the comparison.
        let rank = |peer: &PeerSnapshot| match mode {
            ChokeMode::Downloading => (peer.download_rate, peer.upload_rate),
            ChokeMode::Seeding => (peer.upload_rate, peer.download_rate),
        };
        interested.sort_by(|a, b| {
            let (a_primary, a_secondary) = rank(a);
            let (b_primary, b_secondary) = rank(b);
            b_primary
                .partial_cmp(&a_primary)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(
                    b_secondary
                        .partial_cmp(&a_secondary)
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
        });

        // Pick top N for regular slots.
        let unchoked_ids: Vec<PeerId> = interested
            .iter()
            .take(self.max_unchoked)
            .map(|p| p.id)
            .collect();

        // Forget peers that left so the history stays bounded by the swarm.
        self.optimistic_history
            .retain(|id, _| peers.iter().any(|peer| peer.id == *id));

        // Rotate the optimistic slot on interval, or at once when its holder
        // is gone, no longer interested, or earned a regular slot.
        let holder_valid = self.optimistic_id.is_some_and(|id| {
            interested.iter().any(|peer| peer.id == id) && !unchoked_ids.contains(&id)
        });
        if now.saturating_duration_since(self.last_optimistic_rotate)
            >= OPTIMISTIC_ROTATE_INTERVAL
            || !holder_valid
        {
            let candidates: Vec<&PeerSnapshot> =
                interested.iter().skip(self.max_unchoked).copied().collect();
            // Prefer peers that have never held the slot, then the one that
            // held it longest ago; break remaining ties randomly.
            let oldest = candidates
                .iter()
                .map(|peer| self.optimistic_history.get(&peer.id).copied())
                .min();
            if let Some(oldest) = oldest {
                let tied: Vec<&PeerSnapshot> = candidates
                    .into_iter()
                    .filter(|peer| self.optimistic_history.get(&peer.id).copied() == oldest)
                    .collect();
                let pick = self.random_index(tied.len());
                let id = tied[pick].id;
                self.optimistic_id = Some(id);
                self.optimistic_history.insert(id, now);
                self.last_optimistic_rotate = now;
            } else {
                self.optimistic_id = None;
            }
        }

        // Build decision map.
        let mut decisions = HashMap::new();
        for p in peers {
            let in_regular = unchoked_ids.contains(&p.id);
            let in_optimistic = self.optimistic_id == Some(p.id);
            let decision = if (in_regular || in_optimistic) && p.interested {
                ChokeDecision::Unchoke
            } else {
                ChokeDecision::Choke
            };
            decisions.insert(p.id, decision);
        }
        decisions
    }
}

impl Default for Choker {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_UNCHOKED)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peer::ChokeState;

    fn snap(id: PeerId, interested: bool, rate: f64) -> PeerSnapshot {
        PeerSnapshot {
            id,
            interested,
            upload_rate: rate,
            download_rate: 0.0,
            current_choke: ChokeState::Choked,
        }
    }

    #[test]
    fn top_n_peers_unchoked() {
        let mut choker = Choker::new(2);
        let ids: Vec<PeerId> = (0..5).map(|_| PeerId::new()).collect();
        let peers = vec![
            snap(ids[0], true, 10.0),
            snap(ids[1], true, 50.0),   // best — always in regular slot
            snap(ids[2], true, 30.0),   // second — always in regular slot
            snap(ids[3], true, 5.0),    // may get optimistic on first run
            snap(ids[4], false, 100.0), // not interested
        ];
        let decisions = choker.run(&peers);
        // Top-2 by rate always unchoked.
        assert_eq!(decisions[&ids[1]], ChokeDecision::Unchoke);
        assert_eq!(decisions[&ids[2]], ChokeDecision::Unchoke);
        // Not interested → always choked regardless of rate.
        assert_eq!(decisions[&ids[4]], ChokeDecision::Choke);
        // ids[0] or ids[3] may receive the optimistic slot — both together cannot be unchoked.
        let extra_unchoked = [ids[0], ids[3]]
            .iter()
            .filter(|id| decisions[id] == ChokeDecision::Unchoke)
            .count();
        assert!(extra_unchoked <= 1, "at most one optimistic slot");
    }

    #[test]
    fn not_interested_always_choked() {
        let mut choker = Choker::new(4);
        let id = PeerId::new();
        let peers = vec![snap(id, false, 9999.0)];
        let decisions = choker.run(&peers);
        assert_eq!(decisions[&id], ChokeDecision::Choke);
    }

    #[test]
    fn fewer_peers_than_slots_all_unchoked() {
        let mut choker = Choker::new(4);
        let ids: Vec<PeerId> = (0..3).map(|_| PeerId::new()).collect();
        let peers: Vec<PeerSnapshot> = ids.iter().map(|&id| snap(id, true, 1.0)).collect();
        let decisions = choker.run(&peers);
        // All 3 should be unchoked (fewer than 4 slots)
        for id in &ids {
            assert_eq!(decisions[id], ChokeDecision::Unchoke);
        }
    }

    #[test]
    fn optimistic_slot_assigned() {
        let mut choker = Choker::new(1);
        let ids: Vec<PeerId> = (0..3).map(|_| PeerId::new()).collect();
        // ids[0] has best rate → regular slot
        // ids[1] or ids[2] should get optimistic
        let peers = vec![
            snap(ids[0], true, 100.0),
            snap(ids[1], true, 10.0),
            snap(ids[2], true, 5.0),
        ];
        let decisions = choker.run(&peers);
        // ids[0] is in regular slot
        assert_eq!(decisions[&ids[0]], ChokeDecision::Unchoke);
        // At least one of the remaining gets the optimistic slot
        let opt_unchoked = decisions[&ids[1]] == ChokeDecision::Unchoke
            || decisions[&ids[2]] == ChokeDecision::Unchoke;
        assert!(opt_unchoked);
    }

    #[test]
    fn seeded_optimistic_ties_replay_and_visit_every_candidate() {
        let ids: Vec<PeerId> = (0..5).map(|_| PeerId::new()).collect();
        let peers: Vec<PeerSnapshot> = ids.iter().map(|&id| snap(id, true, 1.0)).collect();
        let mut first = Choker::with_seed(1, 0x5EED);
        let mut replay = Choker::with_seed(1, 0x5EED);
        let mut first_trace = Vec::new();
        let mut replay_trace = Vec::new();

        let start = Instant::now();
        for round in 0..4 {
            let at = start + OPTIMISTIC_ROTATE_INTERVAL * round;
            let first_decisions = first.run_with_mode_at(&peers, ChokeMode::Seeding, at);
            let replay_decisions = replay.run_with_mode_at(&peers, ChokeMode::Seeding, at);
            let first_choice = ids
                .iter()
                .skip(1)
                .find(|id| first_decisions[id] == ChokeDecision::Unchoke)
                .copied()
                .unwrap();
            let replay_choice = ids
                .iter()
                .skip(1)
                .find(|id| replay_decisions[id] == ChokeDecision::Unchoke)
                .copied()
                .unwrap();
            first_trace.push(first_choice);
            replay_trace.push(replay_choice);
        }

        assert_eq!(first_trace, replay_trace);
        let visited = first_trace.iter().copied().collect::<std::collections::HashSet<_>>();
        assert_eq!(visited.len(), 4);
    }

    #[test]
    fn nan_upload_rate_does_not_panic() {
        // upload_rate is a public field; a producer that ever divides 0.0/0.0
        // (e.g. a reset EMA on a genuinely idle peer) must not crash the
        // choke loop just because it sorts by rate.
        let mut choker = Choker::new(2);
        let ids: Vec<PeerId> = (0..3).map(|_| PeerId::new()).collect();
        let peers = vec![
            snap(ids[0], true, f64::NAN),
            snap(ids[1], true, 10.0),
            snap(ids[2], true, f64::NAN),
        ];
        let decisions = choker.run(&peers);
        assert_eq!(decisions.len(), 3);
    }

    #[test]
    fn downloading_mode_rewards_peers_that_deliver_to_us() {
        let mut choker = Choker::new(1);
        let (generous, fast_sink, idle) = (PeerId::new(), PeerId::new(), PeerId::new());
        let mut a = snap(generous, true, 1.0);
        a.download_rate = 900.0;
        let mut b = snap(fast_sink, true, 5_000.0);
        b.download_rate = 0.0;
        let c = snap(idle, true, 0.0);
        let peers = vec![a, b, c];

        let downloading = choker.run_with_mode(&peers, ChokeMode::Downloading);
        assert_eq!(downloading[&generous], ChokeDecision::Unchoke);

        let mut seeding_choker = Choker::new(1);
        let seeding = seeding_choker.run_with_mode(&peers, ChokeMode::Seeding);
        assert_eq!(seeding[&fast_sink], ChokeDecision::Unchoke);
    }

    #[test]
    fn optimistic_slot_rotates_through_every_waiting_peer() {
        // One regular slot, four waiting peers: forcing a rotation each round
        // must visit all of them before repeating any.
        let mut choker = Choker::new(1);
        let leader = PeerId::new();
        let waiting: Vec<PeerId> = (0..4).map(|_| PeerId::new()).collect();
        let mut peers = vec![snap(leader, true, 1_000.0)];
        peers.extend(waiting.iter().map(|id| snap(*id, true, 1.0)));

        let mut seen = std::collections::HashSet::new();
        for _ in 0..4 {
            choker.last_optimistic_rotate =
                Instant::now() - OPTIMISTIC_ROTATE_INTERVAL - Duration::from_secs(1);
            let decisions = choker.run(&peers);
            let holder = waiting
                .iter()
                .find(|id| decisions[*id] == ChokeDecision::Unchoke)
                .copied()
                .expect("one waiting peer holds the optimistic slot");
            assert!(seen.insert(holder), "a peer repeated before all were tried");
        }
        assert_eq!(seen.len(), waiting.len());
    }

    #[test]
    fn optimistic_slot_moves_on_when_its_holder_stops_being_interested() {
        let mut choker = Choker::new(1);
        let (leader, a, b) = (PeerId::new(), PeerId::new(), PeerId::new());
        let peers = vec![
            snap(leader, true, 100.0),
            snap(a, true, 1.0),
            snap(b, true, 1.0),
        ];
        let first = choker.run(&peers);
        let holder = if first[&a] == ChokeDecision::Unchoke {
            a
        } else {
            b
        };
        let other = if holder == a { b } else { a };

        let peers = vec![
            snap(leader, true, 100.0),
            snap(holder, false, 1.0),
            snap(other, true, 1.0),
        ];
        let next = choker.run(&peers);
        assert_eq!(next[&holder], ChokeDecision::Choke);
        assert_eq!(next[&other], ChokeDecision::Unchoke);
    }
}
