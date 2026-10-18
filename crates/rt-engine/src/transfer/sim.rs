//! Deterministic transfer simulator and invariant oracle (NET-102).
//!
//! Replays a swarm against the [`Ledger`] on virtual time with a seeded PRNG:
//! peers with different latencies and protocols (Fast and legacy), chokes,
//! disconnect/reconnect, replies that are lost, writer commits that race the
//! responses, late data after a choke, endgame duplicates, and pieces whose
//! hash fails. After every event the oracle re-checks the ledger's invariants
//! and the simulator's own conservation laws, so a regression anywhere in the
//! ownership rules fails with a seed that reproduces it exactly. A second
//! seeded trace in the test module interleaves v1/v2 hybrid views, piece
//! affinity, late responses, and connection-slot reuse.

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
                        ReceiveOutcome::IgnoredForPieceAffinity => {
                            unreachable!("simulator uses the non-affine receive path")
                        }
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
    use bytes::Bytes;

    use super::super::coords::{HybridGeometry, Plane};
    use super::super::hybrid::{HybridLedger, HybridLedgerError, HybridReceiveOutcome};
    use super::super::ledger::{CommitOutcome, ReserveError};
    use rt_path::SafeRelPath;
    use rt_piece_map::{FileSpan, PieceMap, V2FileSpan, V2PieceMap};

    const HYBRID_BLOCK: u32 = 16 * 1024;

    fn hybrid_geometry() -> HybridGeometry {
        let payload = SafeRelPath::from_name("payload.bin", false).unwrap();
        let padding = SafeRelPath::from_name("padding.bin", false).unwrap();
        let v1 = PieceMap::new_with_padding(
            32 * 1024,
            vec![
                FileSpan {
                    file_index: 0,
                    path: payload.clone(),
                    content_offset: 0,
                    length: 40 * 1024,
                },
                FileSpan {
                    file_index: 1,
                    path: padding,
                    content_offset: 40 * 1024,
                    length: 24 * 1024,
                },
                FileSpan {
                    file_index: 2,
                    path: SafeRelPath::from_name("tail.bin", false).unwrap(),
                    content_offset: 64 * 1024,
                    length: 24 * 1024,
                },
            ],
            [1],
        )
        .unwrap();
        let v2 = V2PieceMap::new(
            32 * 1024,
            vec![
                V2FileSpan {
                    file_index: 0,
                    path: payload,
                    piece_offset: 0,
                    length: 40 * 1024,
                    pad: false,
                },
                V2FileSpan {
                    file_index: 1,
                    path: SafeRelPath::from_name("tail.bin", false).unwrap(),
                    piece_offset: 64 * 1024,
                    length: 24 * 1024,
                    pad: false,
                },
            ],
        )
        .unwrap();
        HybridGeometry::new(v1, v2)
    }

    fn run_hybrid_affinity_trace(seed: u64) {
        let mut rng = Rng::new(seed);
        let mut ledger = HybridLedger::new(hybrid_geometry());
        let first_plane = if rng.below(2) == 0 {
            Plane::V1
        } else {
            Plane::V2
        };
        let second_plane = if first_plane == Plane::V1 {
            Plane::V2
        } else {
            Plane::V1
        };
        let first = ledger.connect(first_plane, 8);
        let second = ledger.connect(second_plane, 8);
        let first_begin = u32::try_from(rng.below(2)).unwrap() * HYBRID_BLOCK;
        let second_begin = HYBRID_BLOCK - first_begin;
        let first_block = BlockExtent::new(0, first_begin, HYBRID_BLOCK);
        let second_block = BlockExtent::new(0, second_begin, HYBRID_BLOCK);

        assert_eq!(
            ledger.reserve_piece_affine(first, first_block),
            Ok(first_block),
            "seed {seed}: first plane should acquire the canonical piece"
        );
        assert_eq!(
            ledger.reserve_piece_affine(second, second_block),
            Err(HybridLedgerError::Reserve(ReserveError::PieceOwnedByOther)),
            "seed {seed}: the other plane must not split the piece assembly"
        );
        ledger.check_invariants().unwrap();
        let started_at = rng.next_u64() % SEC;
        assert_eq!(
            ledger.commit(first, first_block, started_at).unwrap().0,
            CommitOutcome::OnWire,
            "seed {seed}: the winning request should enter flight"
        );
        ledger.check_invariants().unwrap();

        for _ in 0..1 + rng.below(4) {
            let (outcome, data) = ledger
                .on_block_piece_affine(
                    second,
                    second_block,
                    Bytes::from(vec![0x22; HYBRID_BLOCK as usize]),
                )
                .unwrap();
            assert_eq!(outcome, HybridReceiveOutcome::IgnoredForPieceAffinity);
            assert!(data.is_none());
            ledger.check_invariants().unwrap();
        }

        let (first_outcome, first_data) = ledger
            .on_block_piece_affine(
                first,
                first_block,
                Bytes::from(vec![seed as u8; HYBRID_BLOCK as usize]),
            )
            .unwrap();
        assert!(matches!(
            first_outcome,
            HybridReceiveOutcome::Accepted {
                completed: None,
                ..
            }
        ));
        assert_eq!(
            first_data.unwrap().canonical_extent,
            first_block,
            "seed {seed}: accepted data must be in the canonical v1 plane"
        );
        ledger.check_invariants().unwrap();

        assert_eq!(
            ledger.reserve_piece_affine(first, second_block),
            Ok(second_block),
            "seed {seed}: the same connection may continue its assembly"
        );
        assert_eq!(
            ledger
                .commit(first, second_block, started_at.saturating_add(MS))
                .unwrap()
                .0,
            CommitOutcome::OnWire
        );
        ledger.check_invariants().unwrap();
        let (second_outcome, second_data) = ledger
            .on_block_piece_affine(
                first,
                second_block,
                Bytes::from(vec![seed.wrapping_add(1) as u8; HYBRID_BLOCK as usize]),
            )
            .unwrap();
        assert!(matches!(
            second_outcome,
            HybridReceiveOutcome::Accepted {
                completed: Some(_),
                ..
            }
        ));
        assert_eq!(
            second_data.unwrap().canonical_extent,
            second_block,
            "seed {seed}: completion must preserve canonical coordinates"
        );
        ledger.piece_verified(0);
        assert!(
            ledger.has_piece(0),
            "seed {seed}: verified state must settle"
        );
        ledger.check_invariants().unwrap();

        let stale_plane = if second_plane == Plane::V1 {
            Plane::V2
        } else {
            Plane::V1
        };
        let disconnected = ledger.disconnect(second).unwrap();
        assert!(disconnected.is_empty());
        let replacement = ledger.connect(stale_plane, 8);
        assert_eq!(replacement.slot, second.slot);
        assert_ne!(replacement.generation, second.generation);
        assert_eq!(
            ledger.on_block_piece_affine(
                second,
                second_block,
                Bytes::from(vec![0x33; HYBRID_BLOCK as usize]),
            ),
            Err(HybridLedgerError::UnknownConnection(second)),
            "seed {seed}: a reused slot must reject the previous plane generation"
        );
        assert!(ledger.check_invariants().is_ok(), "seed {seed}");
        assert!(ledger.disconnect(first).unwrap().is_empty());
        assert!(ledger.disconnect(replacement).unwrap().is_empty());
        ledger.check_invariants().unwrap();
    }

    fn run_hybrid_response_race(seed: u64) {
        let mut rng = Rng::new(seed ^ 0xA5A5_5A5A_0123_4567);
        let mut ledger = HybridLedger::new(hybrid_geometry());
        let first_plane = if rng.below(2) == 0 {
            Plane::V1
        } else {
            Plane::V2
        };
        let second_plane = if first_plane == Plane::V1 {
            Plane::V2
        } else {
            Plane::V1
        };
        let first = ledger.connect(first_plane, 8);
        let second = ledger.connect(second_plane, 8);
        let v1_block = BlockExtent::new(1, 0, HYBRID_BLOCK);
        let v2_block = BlockExtent::new(1, 0, HYBRID_BLOCK / 2);
        let (first_block, second_block) = if first_plane == Plane::V1 {
            (v1_block, v2_block)
        } else {
            (v2_block, v1_block)
        };

        assert_eq!(
            ledger.reserve(first, first_block, &ReserveMode::Exclusive),
            Ok(v1_block),
            "seed {seed}: the first wire view should reserve canonical payload"
        );
        assert_eq!(
            ledger.reserve(second, second_block, &ReserveMode::Endgame),
            Ok(v1_block),
            "seed {seed}: endgame duplicate must map to the same canonical range"
        );
        ledger.check_invariants().unwrap();
        let now = rng.next_u64() % SEC;
        if rng.below(2) == 0 {
            assert_eq!(
                ledger.commit(first, first_block, now).unwrap().0,
                CommitOutcome::OnWire
            );
            assert_eq!(
                ledger
                    .commit(second, second_block, now.saturating_add(1))
                    .unwrap()
                    .0,
                CommitOutcome::OnWire
            );
        } else {
            assert_eq!(
                ledger.commit(second, second_block, now).unwrap().0,
                CommitOutcome::OnWire
            );
            assert_eq!(
                ledger
                    .commit(first, first_block, now.saturating_add(1))
                    .unwrap()
                    .0,
                CommitOutcome::OnWire
            );
        }
        ledger.check_invariants().unwrap();

        let second_wins = rng.below(2) == 1;
        let (winner, winner_plane, winner_block, loser, loser_plane, loser_block) = if second_wins {
            (
                second,
                second_plane,
                second_block,
                first,
                first_plane,
                first_block,
            )
        } else {
            (
                first,
                first_plane,
                first_block,
                second,
                second_plane,
                second_block,
            )
        };
        let marker = seed as u8;
        let wire_payload = match winner_plane {
            Plane::V1 => {
                let mut bytes = vec![marker; (HYBRID_BLOCK / 2) as usize];
                bytes.resize(HYBRID_BLOCK as usize, 0);
                Bytes::from(bytes)
            }
            Plane::V2 => Bytes::from(vec![marker; (HYBRID_BLOCK / 2) as usize]),
        };
        let (outcome, data) = ledger.on_block(winner, winner_block, wire_payload).unwrap();
        let HybridReceiveOutcome::Accepted { actions, .. } = outcome else {
            panic!("seed {seed}: first valid response should win endgame ownership");
        };
        assert_eq!(
            actions.len(),
            1,
            "seed {seed}: the losing wire request should be canceled"
        );
        assert_eq!(actions[0].conn, loser);
        assert_eq!(actions[0].block.plane, loser_plane);
        assert_eq!(actions[0].block.extent, loser_block);
        let data = data.expect("winning response returns canonical writer payload");
        assert_eq!(data.canonical_extent, v1_block);
        assert!(data.payload[..(HYBRID_BLOCK / 2) as usize]
            .iter()
            .all(|byte| *byte == marker));
        assert!(data.payload[(HYBRID_BLOCK / 2) as usize..]
            .iter()
            .all(|byte| *byte == 0));
        ledger.check_invariants().unwrap();

        let losing_payload = match loser_plane {
            Plane::V1 => {
                let mut bytes = vec![marker; (HYBRID_BLOCK / 2) as usize];
                bytes.resize(HYBRID_BLOCK as usize, 0);
                Bytes::from(bytes)
            }
            Plane::V2 => Bytes::from(vec![marker; (HYBRID_BLOCK / 2) as usize]),
        };
        let (late_outcome, late_data) =
            ledger.on_block(loser, loser_block, losing_payload).unwrap();
        assert_eq!(late_outcome, HybridReceiveOutcome::Duplicate);
        assert!(late_data.is_none());
        assert_eq!(ledger.in_flight(), 0);
        ledger.check_invariants().unwrap();
        let (contributors, actions) = ledger.piece_failed_with_actions(1).unwrap();
        assert!(contributors.contains(&winner));
        assert!(actions.is_empty());
        assert_eq!(ledger.piece_progress(1), None);
        ledger.check_invariants().unwrap();
        assert!(ledger.disconnect(first).unwrap().is_empty());
        assert!(ledger.disconnect(second).unwrap().is_empty());
        ledger.check_invariants().unwrap();
    }

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

    #[test]
    fn seeded_hybrid_plane_traces_preserve_piece_affinity_and_generations() {
        for seed in 0..64 {
            run_hybrid_affinity_trace(seed);
            run_hybrid_response_race(seed);
        }
    }
}
