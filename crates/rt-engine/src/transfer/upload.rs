//! Bounded, fair, byte-admitted upload service (NET-110).
//!
//! The previous guard classified demand with a message-count ceiling derived
//! from a 64 MiB fixture and silently dropped everything beyond it. Real
//! demand is bounded by what can be *held*: this service admits requests
//! against a per-connection queue and a service-wide in-flight read budget
//! (jobs and bytes), serves connections round-robin so one fast leecher
//! cannot starve the rest, and answers anything it will not serve according
//! to the negotiated protocol (see [`obligations`](super::obligations)).
//!
//! It never reads a file: it hands out [`ReadJob`]s and is told how each one
//! ended, so read progress stays independent of slow sockets and disks.

use std::collections::{HashMap, VecDeque};

use super::coords::{BlockExtent, TransferPeerId, BLOCK_SIZE};
use super::obligations::{InboundDecision, InboundObligations, Reject, Served};

/// Mirrors `NetworkConfig::max_peers`' current default. The live adapter must
/// override this with the configured daemon-wide peer cap when it is added.
const DEFAULT_MAX_UPLOAD_CONNECTIONS: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UploadLimits {
    /// Live torrent/connection identities retained by this process-wide
    /// service. Disconnected generations with unfinished reads remain bounded
    /// separately by `max_in_flight_jobs`.
    pub max_connections: usize,
    /// Maximum queued plus in-service request obligations per connection.
    pub max_queue_per_conn: usize,
    /// Reads in flight across all connections.
    pub max_in_flight_jobs: usize,
    /// Bytes of reads in flight across all connections. The effective
    /// minimum is one legal wire block, so every valid request can run.
    pub max_in_flight_bytes: u64,
    /// Reads in flight for one connection (fairness bound).
    pub max_in_flight_per_conn: usize,
}

impl Default for UploadLimits {
    fn default() -> Self {
        Self {
            max_connections: DEFAULT_MAX_UPLOAD_CONNECTIONS,
            max_queue_per_conn: 4_096,
            max_in_flight_jobs: 64,
            max_in_flight_bytes: 16 * 1024 * 1024,
            max_in_flight_per_conn: 8,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct JobId(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadJob {
    pub id: JobId,
    pub conn: TransferPeerId,
    pub block: BlockExtent,
}

/// What the caller must send in response to an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outgoing {
    pub conn: TransferPeerId,
    pub reject: Reject,
}

struct ConnUpload {
    obligations: InboundObligations,
    in_flight: usize,
}

pub struct UploadService {
    limits: UploadLimits,
    conns: HashMap<TransferPeerId, ConnUpload>,
    /// Round-robin order of connections that may have work.
    ring: VecDeque<TransferPeerId>,
    jobs: HashMap<JobId, ReadJob>,
    in_flight_bytes: u64,
    next_job: u64,
}

impl UploadService {
    pub fn new(mut limits: UploadLimits) -> Self {
        limits.max_in_flight_bytes = limits.max_in_flight_bytes.max(u64::from(BLOCK_SIZE));
        Self {
            limits,
            conns: HashMap::new(),
            ring: VecDeque::new(),
            jobs: HashMap::new(),
            in_flight_bytes: 0,
            next_job: 1,
        }
    }

    /// Register one fresh torrent/connection generation. Refuse a duplicate
    /// live identity, including one whose previous storage read has not
    /// completed, or refuse admission when the process-wide connection table
    /// is full.
    pub fn register(&mut self, conn: TransferPeerId, fast_negotiated: bool) -> bool {
        if self.conns.contains_key(&conn) || self.jobs.values().any(|job| job.conn == conn) {
            return false;
        }
        if self.conns.len() >= self.limits.max_connections {
            return false;
        }
        self.conns.insert(
            conn,
            ConnUpload {
                obligations: InboundObligations::new(
                    fast_negotiated,
                    self.limits.max_queue_per_conn,
                ),
                in_flight: 0,
            },
        );
        self.ring.push_back(conn);
        true
    }

    pub fn allow_fast(&mut self, conn: TransferPeerId, piece: u32) {
        if let Some(state) = self.conns.get_mut(&conn) {
            state.obligations.allow_fast(piece);
        }
    }

    pub fn in_flight_jobs(&self) -> usize {
        self.jobs.len()
    }

    pub fn in_flight_bytes(&self) -> u64 {
        self.in_flight_bytes
    }

    pub fn queued(&self, conn: TransferPeerId) -> usize {
        self.conns
            .get(&conn)
            .map_or(0, |state| state.obligations.queued_len())
    }

    /// A request arrived from `conn`.
    pub fn admit(
        &mut self,
        conn: TransferPeerId,
        block: BlockExtent,
        have_piece: bool,
    ) -> Option<InboundDecision> {
        self.conns
            .get_mut(&conn)
            .map(|state| state.obligations.on_request(block, have_piece))
    }

    pub fn on_cancel(&mut self, conn: TransferPeerId, block: BlockExtent) -> Option<Outgoing> {
        let reject = self.conns.get_mut(&conn)?.obligations.on_cancel(block)?;
        Some(Outgoing { conn, reject })
    }

    pub fn on_local_unchoke(&mut self, conn: TransferPeerId) {
        if let Some(state) = self.conns.get_mut(&conn) {
            state.obligations.on_local_unchoke();
        }
    }

    pub fn on_local_choke(&mut self, conn: TransferPeerId) -> Vec<Outgoing> {
        self.conns
            .get_mut(&conn)
            .map(|state| {
                state
                    .obligations
                    .on_local_choke()
                    .into_iter()
                    .map(|reject| Outgoing { conn, reject })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Next read to start, honouring the global and per-connection budgets
    /// and round-robin fairness. `None` when nothing is runnable.
    pub fn next_job(&mut self) -> Option<ReadJob> {
        if self.jobs.len() >= self.limits.max_in_flight_jobs {
            return None;
        }
        // Job IDs are never reused while the service is alive. Refuse new
        // work before mutating an obligation if the counter is exhausted.
        let next_job = self.next_job.checked_add(1)?;
        for _ in 0..self.ring.len() {
            let conn = self.ring.pop_front()?;
            let Some(state) = self.conns.get_mut(&conn) else {
                continue; // the connection went away; drop it from the ring
            };
            self.ring.push_back(conn);
            if state.in_flight >= self.limits.max_in_flight_per_conn
                || state.obligations.queued_len() == 0
            {
                continue;
            }
            // Look at the head before dequeuing so a byte-budget deferral
            // leaves the queue (and its order) untouched.
            let Some(head) = state.obligations.peek_request() else {
                continue;
            };
            if self.in_flight_bytes.saturating_add(u64::from(head.length))
                > self.limits.max_in_flight_bytes
            {
                continue;
            }
            let Some(block) = state.obligations.next_request() else {
                continue;
            };
            let id = JobId(self.next_job);
            self.next_job = next_job;
            state.in_flight += 1;
            self.in_flight_bytes += u64::from(block.length);
            let job = ReadJob { id, conn, block };
            self.jobs.insert(id, job);
            return Some(job);
        }
        None
    }

    /// Whether the data for `job` may still be sent.
    pub fn may_send(&self, job: &ReadJob) -> bool {
        self.conns
            .get(&job.conn)
            .is_some_and(|state| state.obligations.may_send(&job.block))
    }

    /// A read job ended. Returns the Reject owed to a Fast peer, if any.
    pub fn finish(&mut self, id: JobId, served: Served) -> Option<Outgoing> {
        let job = self.jobs.remove(&id)?;
        self.in_flight_bytes = self
            .in_flight_bytes
            .saturating_sub(u64::from(job.block.length));
        let state = self.conns.get_mut(&job.conn)?;
        state.in_flight = state.in_flight.saturating_sub(1);
        let reject = state.obligations.finish(job.block, served)?;
        Some(Outgoing {
            conn: job.conn,
            reject,
        })
    }

    /// The connection closed; its protocol obligations end with it. Reads
    /// already handed to the backend remain charged until `finish`, even
    /// though their results can no longer be sent.
    pub fn disconnect(&mut self, conn: TransferPeerId) {
        if let Some(mut state) = self.conns.remove(&conn) {
            state.obligations.close();
        }
        self.ring.retain(|candidate| *candidate != conn);
    }

    pub fn check_invariants(&self) -> Result<(), String> {
        let bytes: u64 = self
            .jobs
            .values()
            .map(|job| u64::from(job.block.length))
            .sum();
        if bytes != self.in_flight_bytes {
            return Err(format!(
                "in-flight bytes {} != jobs {bytes}",
                self.in_flight_bytes
            ));
        }
        for (conn, state) in &self.conns {
            let jobs = self.jobs.values().filter(|job| job.conn == *conn).count();
            if jobs != state.in_flight {
                return Err(format!(
                    "{conn:?} in-flight {} != jobs {jobs}",
                    state.in_flight
                ));
            }
            state.obligations.check_invariants()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::coords::{ConnectionId, TorrentKey};
    use super::super::obligations::RejectReason;
    use super::*;

    fn peer(torrent: u64, slot: u32) -> TransferPeerId {
        TransferPeerId::new(
            TorrentKey(torrent),
            ConnectionId {
                slot,
                generation: 1,
            },
        )
    }

    fn conn(slot: u32) -> TransferPeerId {
        peer(1, slot)
    }

    fn block(piece: u32, index: u32) -> BlockExtent {
        BlockExtent::new(piece, index * 16 * 1024, 16 * 1024)
    }

    fn ready(service: &mut UploadService, id: u32, fast: bool) -> TransferPeerId {
        let conn = conn(id);
        service.register(conn, fast);
        service.on_local_unchoke(conn);
        conn
    }

    #[test]
    fn ledger_connection_slots_are_isolated_between_torrents() {
        let mut service = UploadService::new(UploadLimits::default());
        let first_torrent = peer(1, 1);
        let second_torrent = peer(2, 1);
        assert_eq!(first_torrent.connection, second_torrent.connection);
        assert!(service.register(first_torrent, true));
        assert!(service.register(second_torrent, true));
        service.on_local_unchoke(first_torrent);
        service.on_local_unchoke(second_torrent);

        assert_eq!(
            service.admit(first_torrent, block(0, 0), true),
            Some(InboundDecision::Queue)
        );
        assert_eq!(
            service.admit(second_torrent, block(0, 0), true),
            Some(InboundDecision::Queue)
        );
        assert_eq!(service.queued(first_torrent), 1);
        assert_eq!(service.queued(second_torrent), 1);

        service.disconnect(first_torrent);
        assert_eq!(service.queued(second_torrent), 1);
        let job = service
            .next_job()
            .expect("the other torrent remains schedulable");
        assert_eq!(job.conn, second_torrent);
    }

    #[test]
    fn valid_demand_is_not_capped_by_a_request_count_window() {
        // F06: 10,000 valid requests from one peer must all be admitted as
        // long as the queue bound holds; none are dropped by a rate window.
        let mut service = UploadService::new(UploadLimits {
            max_queue_per_conn: 10_000,
            ..UploadLimits::default()
        });
        let conn = ready(&mut service, 1, true);
        for index in 0..10_000u32 {
            let decision = service.admit(conn, block(index, 0), true).unwrap();
            assert_eq!(decision, InboundDecision::Queue, "request {index}");
        }
        assert_eq!(service.queued(conn), 10_000);
    }

    #[test]
    fn exhausted_job_identity_does_not_consume_an_upload_obligation() {
        let mut service = UploadService::new(UploadLimits::default());
        let conn = ready(&mut service, 1, true);
        assert_eq!(
            service.admit(conn, block(0, 0), true),
            Some(InboundDecision::Queue)
        );
        service.next_job = u64::MAX;

        assert!(service.next_job().is_none());
        assert_eq!(service.queued(conn), 1);
        assert_eq!(service.in_flight_jobs(), 0);
        assert!(service.check_invariants().is_ok());
    }

    #[test]
    fn overload_is_rejected_for_fast_and_dropped_for_legacy() {
        let limits = UploadLimits {
            max_queue_per_conn: 2,
            ..UploadLimits::default()
        };
        let mut service = UploadService::new(limits);
        let fast = ready(&mut service, 1, true);
        let legacy = ready(&mut service, 2, false);
        for conn in [fast, legacy] {
            service.admit(conn, block(0, 0), true);
            service.admit(conn, block(1, 0), true);
        }
        assert_eq!(
            service.admit(fast, block(2, 0), true),
            Some(InboundDecision::Reject(RejectReason::QueueFull))
        );
        assert_eq!(
            service.admit(legacy, block(2, 0), true),
            Some(InboundDecision::Drop(RejectReason::QueueFull))
        );
    }

    #[test]
    fn connections_are_served_round_robin() {
        let mut service = UploadService::new(UploadLimits {
            max_in_flight_per_conn: 100,
            ..UploadLimits::default()
        });
        let greedy = ready(&mut service, 1, true);
        let modest = ready(&mut service, 2, true);
        for index in 0..20 {
            service.admit(greedy, block(index, 0), true);
        }
        for index in 0..3 {
            service.admit(modest, block(100 + index, 0), true);
        }
        let mut order = Vec::new();
        while let Some(job) = service.next_job() {
            order.push(job.conn);
            service.finish(job.id, Served::Sent);
        }
        // The modest peer's 3 requests are interleaved, not queued behind 20.
        let modest_positions: Vec<usize> = order
            .iter()
            .enumerate()
            .filter(|(_, c)| **c == modest)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(modest_positions.len(), 3);
        assert!(
            modest_positions[2] < 8,
            "served by position {modest_positions:?}"
        );
        assert_eq!(order.len(), 23);
        service.check_invariants().unwrap();
    }

    #[test]
    fn global_read_budget_bounds_what_is_in_flight() {
        let mut service = UploadService::new(UploadLimits {
            max_in_flight_jobs: 3,
            max_in_flight_per_conn: 100,
            ..UploadLimits::default()
        });
        let conn = ready(&mut service, 1, true);
        for index in 0..10 {
            service.admit(conn, block(index, 0), true);
        }
        let first = (0..3)
            .map(|_| service.next_job().unwrap())
            .collect::<Vec<_>>();
        assert!(service.next_job().is_none(), "job budget exhausted");
        assert_eq!(service.in_flight_jobs(), 3);
        service.finish(first[0].id, Served::Sent);
        assert!(service.next_job().is_some(), "a slot freed");
        service.check_invariants().unwrap();
    }

    #[test]
    fn byte_budget_defers_without_losing_the_request() {
        let mut service = UploadService::new(UploadLimits {
            max_in_flight_bytes: 16 * 1024,
            max_in_flight_per_conn: 100,
            ..UploadLimits::default()
        });
        let conn = ready(&mut service, 1, true);
        service.admit(conn, block(0, 0), true);
        service.admit(conn, block(1, 0), true);
        let first = service.next_job().unwrap();
        assert!(
            service.next_job().is_none(),
            "second would exceed the byte budget"
        );
        assert_eq!(
            service.queued(conn),
            1,
            "the deferred request is still queued"
        );
        service.finish(first.id, Served::Sent);
        assert!(service.next_job().is_some());
        service.check_invariants().unwrap();
    }

    #[test]
    fn configured_byte_budget_has_one_block_minimum_and_remains_a_hard_cap() {
        let mut service = UploadService::new(UploadLimits {
            max_in_flight_bytes: 1,
            max_in_flight_per_conn: 100,
            ..UploadLimits::default()
        });
        let conn = ready(&mut service, 1, true);
        service.admit(conn, block(0, 0), true);
        service.admit(conn, block(1, 0), true);

        let first = service.next_job().expect("one legal block can be read");
        assert_eq!(service.in_flight_bytes(), u64::from(BLOCK_SIZE));
        assert!(service.next_job().is_none(), "the normalized cap is strict");
        assert_eq!(service.queued(conn), 1);

        service.finish(first.id, Served::Sent);
        assert!(
            service.next_job().is_some(),
            "the completion frees its charge"
        );
        service.check_invariants().unwrap();
    }

    #[test]
    fn choking_mid_read_rejects_for_fast_and_blocks_the_send() {
        let mut service = UploadService::new(UploadLimits::default());
        let conn = ready(&mut service, 1, true);
        service.admit(conn, block(0, 0), true);
        service.admit(conn, block(1, 0), true);
        let job = service.next_job().unwrap();
        let rejects = service.on_local_choke(conn);
        assert_eq!(rejects.len(), 1, "the queued request is rejected");
        assert!(
            !service.may_send(&job),
            "the in-flight read must not be sent"
        );
        let owed = service.finish(job.id, Served::Discarded).unwrap();
        assert_eq!(owed.reject.reason, RejectReason::ChokeTransition);
        service.check_invariants().unwrap();
    }

    #[test]
    fn disconnect_keeps_submitted_jobs_charged_until_completion() {
        let mut service = UploadService::new(UploadLimits {
            max_in_flight_jobs: 1,
            max_in_flight_bytes: 16 * 1024,
            ..UploadLimits::default()
        });
        let conn = ready(&mut service, 1, true);
        service.admit(conn, block(0, 0), true);
        let job = service.next_job().unwrap();
        assert!(service.in_flight_bytes() > 0);
        service.disconnect(conn);
        assert!(!service.may_send(&job), "the disconnected peer is gone");
        let replacement = ready(&mut service, 2, true);
        service.admit(replacement, block(1, 0), true);
        assert_eq!(service.in_flight_jobs(), 1);
        assert_eq!(service.in_flight_bytes(), 16 * 1024);
        assert!(
            service.next_job().is_none(),
            "the backend read is still charged"
        );
        assert!(
            service.finish(job.id, Served::Sent).is_none(),
            "late result ignored"
        );
        assert_eq!(service.in_flight_bytes(), 0);
        assert_eq!(service.next_job().unwrap().conn, replacement);
        service.check_invariants().unwrap();
    }

    #[test]
    fn a_connection_generation_cannot_be_reused_while_its_read_is_live() {
        let mut service = UploadService::new(UploadLimits::default());
        let old = ready(&mut service, 1, true);
        service.admit(old, block(0, 0), true);
        let job = service.next_job().unwrap();
        service.disconnect(old);

        assert!(!service.register(old, true));
        assert_eq!(service.in_flight_jobs(), 1);
        assert_eq!(service.in_flight_bytes(), u64::from(BLOCK_SIZE));
        service.check_invariants().unwrap();

        service.finish(job.id, Served::Sent);
        assert!(service.register(old, true));
        service.check_invariants().unwrap();
    }
}
