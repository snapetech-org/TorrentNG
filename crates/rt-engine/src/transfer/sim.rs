//! Deterministic transfer simulator and invariant oracle (NET-102).
//!
//! Replays a swarm against the [`Ledger`] on virtual time with a seeded PRNG:
//! peers with different latencies and protocols (Fast and legacy), chokes,
//! disconnect/reconnect, replies that are lost, writer commits that race the
//! responses, late data after a choke, endgame duplicates, and pieces whose
//! hash fails. After every event the oracle re-checks the ledger's invariants
//! and the simulator's own conservation laws, so a regression anywhere in the
//! ownership rules fails with a seed that reproduces it exactly.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use super::coords::{BlockExtent, ConnectionId, PieceGeometry, UniformGeometry};
use super::ledger::{Action, Ledger, ReceiveOutcome, ReserveMode};
use super::Nanos;

const MS: Nanos = 1_000_000;
const SEC: Nanos = 1_000 * MS;

/// splitmix64: tiny, fast and fully deterministic.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_add(0x9E37_79B9_7F4A_7C15))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound.max(1)
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Event {
    Commit {
        conn: ConnectionId,
        block: BlockExtent,
    },
    Deliver {
        conn: ConnectionId,
        block: BlockExtent,
    },
    RejectArrives {
        conn: ConnectionId,
        block: BlockExtent,
    },
    Choke {
        peer: usize,
    },
    Unchoke {
        peer: usize,
    },
    Drop {
        peer: usize,
    },
    Reconnect {
        peer: usize,
    },
    Verify {
        piece: u32,
        ok: bool,
    },
    Tick,
}

struct SimPeer {
    conn: Option<ConnectionId>,
    latency: Nanos,
    fast: bool,
    choked: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SimReport {
    pub finished_at: Nanos,
    pub events: u64,
    pub accepted_blocks: u64,
    pub late_accepted: u64,
    pub duplicates: u64,
    pub unsolicited: u64,
    pub stale: u64,
    pub failed_pieces: u64,
    pub reconciled_commits: u64,
}

pub struct SimConfig {
    pub seed: u64,
    pub pieces: u32,
    pub blocks_per_piece: u32,
    pub peers: usize,
    pub window: usize,
    pub loss_percent: u64,
    pub hash_fail_percent: u64,
    pub churn: bool,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            seed: 1,
            pieces: 24,
            blocks_per_piece: 4,
            peers: 6,
            window: 12,
            loss_percent: 3,
            hash_fail_percent: 5,
            churn: true,
        }
    }
}

pub fn run(config: SimConfig) -> SimReport {
    let mut rng = Rng::new(config.seed);
    let piece_len = config.blocks_per_piece * 16 * 1024;
    let mut ledger = Ledger::new(UniformGeometry::new(piece_len, config.pieces));
    let mut queue: BinaryHeap<Reverse<(Nanos, u64, Event)>> = BinaryHeap::new();
    let mut seq = 0u64;
    let mut now: Nanos = 0;
    let mut report = SimReport::default();

    let mut peers: Vec<SimPeer> = (0..config.peers)
        .map(|index| SimPeer {
            conn: None,
            latency: (10 + rng.below(70)) * MS,
            fast: index % 2 == 0,
            choked: true,
        })
        .collect();

    macro_rules! at {
        ($delay:expr, $event:expr) => {{
            seq += 1;
            queue.push(Reverse((now + $delay, seq, $event)));
        }};
    }

    for (index, peer) in peers.iter_mut().enumerate() {
        peer.conn = Some(ledger.connect(config.window * 2));
        at!(rng.below(20) * MS, Event::Unchoke { peer: index });
    }
    at!(SEC, Event::Tick);

    let total_blocks = u64::from(config.pieces) * u64::from(config.blocks_per_piece);
    let mut verified = 0u32;

    // Reserve and issue requests for one peer up to the window.
    fn fill(
        ledger: &mut Ledger<UniformGeometry>,
        peer_index: usize,
        peers: &[SimPeer],
        window: usize,
        rng: &mut Rng,
        schedule: &mut Vec<(Nanos, Event)>,
    ) {
        let peer = &peers[peer_index];
        let Some(conn) = peer.conn else { return };
        if peer.choked {
            return;
        }
        let pieces = ledger.geometry().piece_count();
        'outer: while ledger.outstanding(conn) < window {
            for mode in [ReserveMode::Exclusive, ReserveMode::Endgame] {
                for piece in 0..pieces {
                    if ledger.has_piece(piece) {
                        continue;
                    }
                    match ledger.reserve_next(conn, piece, &mode) {
                        Ok(Some(block)) => {
                            // The writer's commit notification races the reply.
                            let commit_delay = rng.below(peer.latency + 1);
                            schedule.push((commit_delay, Event::Commit { conn, block }));
                            schedule.push((peer.latency, Event::Deliver { conn, block }));
                            continue 'outer;
                        }
                        Ok(None) | Err(_) => continue,
                    }
                }
                // Endgame duplicates only once nothing is left to request fresh.
            }
            break;
        }
    }

    let limit = 20 * 60 * SEC;
    while let Some(Reverse((time, _, event))) = queue.pop() {
        now = time;
        if now > limit {
            panic!(
                "simulation did not finish within the virtual time limit (seed {})",
                config.seed
            );
        }
        report.events += 1;
        let mut scheduled: Vec<(Nanos, Event)> = Vec::new();
        let mut refill: Vec<usize> = Vec::new();

        match event {
            Event::Commit { conn, block } => {
                let (outcome, actions) = ledger.commit(conn, block, now);
                if outcome == super::ledger::CommitOutcome::Reconciled {
                    report.reconciled_commits += 1;
                }
                let _ = actions;
            }
            Event::Deliver { conn, block } => {
                let lost = rng.chance(config.loss_percent);
                if !lost {
                    match ledger.on_block(conn, block) {
                        ReceiveOutcome::Accepted {
                            completed,
                            late,
                            actions,
                        } => {
                            report.accepted_blocks += 1;
                            if late {
                                report.late_accepted += 1;
                            }
                            for Action::Cancel { .. } in actions {
                                // The cancel races the duplicate reply; both
                                // outcomes are legal and handled above.
                            }
                            if let Some(extent) = completed {
                                let ok = !rng.chance(config.hash_fail_percent);
                                scheduled.push((
                                    (1 + rng.below(5)) * MS,
                                    Event::Verify {
                                        piece: extent.piece,
                                        ok,
                                    },
                                ));
                            }
                        }
                        ReceiveOutcome::Duplicate => report.duplicates += 1,
                        ReceiveOutcome::Unsolicited => report.unsolicited += 1,
                        ReceiveOutcome::StaleConnection => report.stale += 1,
                        ReceiveOutcome::Invalid(error) => panic!("invalid block from sim: {error}"),
                    }
                }
                if let Some(index) = peers.iter().position(|peer| peer.conn == Some(conn)) {
                    refill.push(index);
                }
            }
            Event::RejectArrives { conn, block } => {
                ledger.on_reject(conn, block);
                if let Some(index) = peers.iter().position(|peer| peer.conn == Some(conn)) {
                    refill.push(index);
                }
            }
            Event::Choke { peer } => {
                if let Some(conn) = peers[peer].conn {
                    peers[peer].choked = true;
                    let fast = peers[peer].fast;
                    let released = ledger.on_choke(conn, fast);
                    if fast {
                        // A Fast peer must answer every request: reject all.
                        for block in ledger_outstanding(
                            &ledger,
                            conn,
                            config.pieces,
                            config.blocks_per_piece,
                        ) {
                            scheduled.push((
                                peers[peer].latency / 2,
                                Event::RejectArrives { conn, block },
                            ));
                        }
                    }
                    let _ = released;
                    scheduled.push(((50 + rng.below(400)) * MS, Event::Unchoke { peer }));
                }
            }
            Event::Unchoke { peer } => {
                peers[peer].choked = false;
                refill.push(peer);
                if config.churn && rng.chance(20) {
                    scheduled.push(((200 + rng.below(1500)) * MS, Event::Choke { peer }));
                }
                if config.churn && rng.chance(5) {
                    scheduled.push(((300 + rng.below(1500)) * MS, Event::Drop { peer }));
                }
            }
            Event::Drop { peer } => {
                if let Some(conn) = peers[peer].conn.take() {
                    ledger.disconnect(conn);
                    scheduled.push(((100 + rng.below(800)) * MS, Event::Reconnect { peer }));
                }
            }
            Event::Reconnect { peer } => {
                if peers[peer].conn.is_none() {
                    peers[peer].conn = Some(ledger.connect(config.window * 2));
                    peers[peer].choked = false;
                    refill.push(peer);
                }
            }
            Event::Verify { piece, ok } => {
                if ok {
                    ledger.piece_verified(piece);
                    verified += 1;
                } else {
                    ledger.piece_failed(piece);
                    report.failed_pieces += 1;
                }
                refill.extend(0..peers.len());
            }
            Event::Tick => {
                let (expired, _actions) = ledger.expire(now, 5 * SEC);
                if !expired.is_empty() {
                    refill.extend(0..peers.len());
                }
                // Keep every connected peer topped up even if its replies
                // were all lost.
                refill.extend(0..peers.len());
                scheduled.push((SEC, Event::Tick));
            }
        }

        for index in refill {
            fill(
                &mut ledger,
                index,
                &peers,
                config.window,
                &mut rng,
                &mut scheduled,
            );
        }
        for (delay, event) in scheduled {
            at!(delay, event);
        }

        // ---- oracle -------------------------------------------------------
        if let Err(error) = ledger.check_invariants() {
            panic!(
                "ledger invariant violated at t={now}ns after {event:?} (seed {}): {error}",
                config.seed
            );
        }
        let have = (0..config.pieces)
            .filter(|piece| ledger.has_piece(*piece))
            .count() as u32;
        assert_eq!(
            have, verified,
            "verified count drifted (seed {})",
            config.seed
        );
        if verified == config.pieces {
            break;
        }
    }

    assert_eq!(
        verified, config.pieces,
        "swarm did not complete (seed {})",
        config.seed
    );
    assert_eq!(
        ledger.in_flight(),
        0,
        "nothing may remain in flight at completion"
    );
    assert!(
        report.accepted_blocks >= total_blocks,
        "every block must have been accepted at least once"
    );
    report.finished_at = now;
    report
}

/// Blocks currently owned by `conn` (for the Fast-choke reject fan-out).
fn ledger_outstanding(
    ledger: &Ledger<UniformGeometry>,
    conn: ConnectionId,
    pieces: u32,
    blocks_per_piece: u32,
) -> Vec<BlockExtent> {
    let mut owned = Vec::new();
    for piece in 0..pieces {
        for index in 0..blocks_per_piece {
            if let Some(block) = ledger.geometry().block_extent(piece, index) {
                if ledger.owns(conn, block) {
                    owned.push(block);
                }
            }
        }
    }
    owned
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn many_seeds_complete_with_every_invariant_holding() {
        for seed in 0..40 {
            let report = run(SimConfig {
                seed,
                ..SimConfig::default()
            });
            assert!(report.finished_at > 0, "seed {seed}");
        }
    }

    #[test]
    fn the_simulation_is_deterministic_for_a_seed() {
        let a = run(SimConfig {
            seed: 7,
            ..SimConfig::default()
        });
        let b = run(SimConfig {
            seed: 7,
            ..SimConfig::default()
        });
        assert_eq!(a, b);
        let c = run(SimConfig {
            seed: 8,
            ..SimConfig::default()
        });
        assert_ne!(a, c, "different seeds exercise different histories");
    }

    #[test]
    fn heavy_loss_and_churn_still_complete() {
        for seed in 100..110 {
            let report = run(SimConfig {
                seed,
                loss_percent: 25,
                hash_fail_percent: 15,
                peers: 4,
                ..SimConfig::default()
            });
            assert!(
                report.failed_pieces
                    + report.unsolicited
                    + report.late_accepted
                    + report.duplicates
                    > 0,
                "heavy loss and failures should exercise the recovery paths (seed {seed})"
            );
        }
    }

    #[test]
    fn races_between_commit_and_response_are_exercised_and_reconciled() {
        let mut reconciled = 0;
        for seed in 0..30 {
            reconciled += run(SimConfig {
                seed,
                ..SimConfig::default()
            })
            .reconciled_commits;
        }
        assert!(
            reconciled > 0,
            "the commit/response race never occurred in 30 seeds"
        );
    }

    #[test]
    fn a_single_peer_swarm_is_a_valid_degenerate_case() {
        let report = run(SimConfig {
            seed: 3,
            peers: 1,
            churn: false,
            loss_percent: 0,
            hash_fail_percent: 0,
            ..SimConfig::default()
        });
        assert_eq!(report.duplicates, 0);
        assert_eq!(report.unsolicited, 0);
        assert_eq!(report.failed_pieces, 0);
    }
}
