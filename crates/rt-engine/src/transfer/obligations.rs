//! Negotiated peer-protocol request obligations (NET-105).
//!
//! Advertising the Fast extension (BEP 6) is a contract, not a bandwidth
//! hint: every request a peer sends us must end in a Piece or a Reject, a
//! choke no longer implicitly discards requests, and requests for
//! allowed-fast pieces survive a choke. Without Fast, the legacy rules apply:
//! excess or choked requests are dropped silently and a choke clears the
//! queue. This module is the pure state machine for the *inbound* half (what
//! peers ask of us); the outbound half (what we asked) lives in the ledger.
//!
//! The wire obligation does not depend on the hash plane. Live v1, pure-v2,
//! and hybrid peer loops use this owner after translating v2 requests to the
//! actor's canonical v1 coordinates and projecting replies back to the wire.

use std::collections::{HashMap, HashSet, VecDeque};

use super::coords::{BlockExtent, BLOCK_SIZE};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    Choked,
    PieceNotHeld,
    QueueFull,
    /// The requested range cannot be served (read failed, content changed).
    Unavailable,
    /// We started choking this peer.
    ChokeTransition,
    /// The peer cancelled it.
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundDecision {
    /// Admitted: it will be served.
    Queue,
    /// Fast negotiated: answer with Reject Request.
    Reject(RejectReason),
    /// Legacy peer: no answer exists; drop silently.
    Drop(RejectReason),
    /// Already queued or being served; answered once, not again.
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reject {
    pub block: BlockExtent,
    pub reason: RejectReason,
}

/// Outcome of attempting to serve a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Served {
    /// A Piece was sent.
    Sent,
    /// It could not be served (read error, content moved, shutdown).
    Failed,
    /// We started choking while it was in flight and the data was dropped.
    Discarded,
}

#[derive(Debug)]
pub struct InboundObligations {
    fast: bool,
    /// We are choking this peer.
    choked: bool,
    allowed_fast: HashSet<u32>,
    queue: VecDeque<BlockExtent>,
    /// One index for both queued and in-service requests. Keeping separate
    /// queued and in-service sets retained two hash tables for one bounded
    /// obligation set, while the FIFO already stores queued coordinates.
    outstanding: HashMap<BlockExtent, RequestState>,
    in_service_len: usize,
    max_outstanding: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RequestState {
    Queued,
    InService,
}

impl InboundObligations {
    pub fn new(fast_negotiated: bool, max_outstanding: usize) -> Self {
        Self {
            fast: fast_negotiated,
            choked: true,
            allowed_fast: HashSet::new(),
            queue: VecDeque::new(),
            outstanding: HashMap::new(),
            in_service_len: 0,
            max_outstanding,
        }
    }

    pub fn fast_negotiated(&self) -> bool {
        self.fast
    }

    pub fn is_choked(&self) -> bool {
        self.choked
    }

    pub fn queued_len(&self) -> usize {
        self.queue.len()
    }

    pub fn in_service_len(&self) -> usize {
        self.in_service_len
    }

    /// Number of unique queued and in-service request obligations.
    pub fn outstanding_len(&self) -> usize {
        self.outstanding.len()
    }

    pub fn allow_fast(&mut self, piece: u32) {
        self.allowed_fast.insert(piece);
    }

    pub fn is_allowed_fast(&self, piece: u32) -> bool {
        self.fast && self.allowed_fast.contains(&piece)
    }

    fn refuse(&self, reason: RejectReason) -> InboundDecision {
        if self.fast {
            InboundDecision::Reject(reason)
        } else {
            InboundDecision::Drop(reason)
        }
    }

    fn request_decision(&self, block: BlockExtent, have_piece: bool) -> InboundDecision {
        if self.outstanding.contains_key(&block) {
            return InboundDecision::Duplicate;
        }
        if block.length == 0 || block.length > BLOCK_SIZE {
            return self.refuse(RejectReason::Unavailable);
        }
        if self.choked && !self.is_allowed_fast(block.piece) {
            return self.refuse(RejectReason::Choked);
        }
        if !have_piece {
            return self.refuse(RejectReason::PieceNotHeld);
        }
        if self.outstanding.len() >= self.max_outstanding {
            return self.refuse(RejectReason::QueueFull);
        }
        InboundDecision::Queue
    }

    /// Whether this request would add a new queued obligation. Runtime
    /// adapters use this to reserve metadata memory before mutating the FIFO
    /// and request index.
    pub fn would_queue(&self, block: BlockExtent, have_piece: bool) -> bool {
        self.request_decision(block, have_piece) == InboundDecision::Queue
    }

    /// A request arrived. `have_piece` is whether we can serve its piece.
    pub fn on_request(&mut self, block: BlockExtent, have_piece: bool) -> InboundDecision {
        self.on_request_with_memory_admission(block, have_piece, true)
    }

    /// Runtime adapters can refuse a logically valid request when its bounded
    /// bookkeeping allocation could not be admitted. The protocol response
    /// matches the ordinary queue-full path for Fast and legacy peers.
    pub fn on_request_with_memory_admission(
        &mut self,
        block: BlockExtent,
        have_piece: bool,
        memory_admitted: bool,
    ) -> InboundDecision {
        self.on_request_with_admission(block, have_piece, |_| memory_admitted)
    }

    /// Admit a request while allowing the runtime adapter to lease its
    /// bookkeeping capacity. The callback runs only for a valid new request,
    /// after protocol checks and duplicate detection have completed.
    pub(crate) fn on_request_with_admission(
        &mut self,
        block: BlockExtent,
        have_piece: bool,
        admit_memory: impl FnOnce(usize) -> bool,
    ) -> InboundDecision {
        let decision = self.request_decision(block, have_piece);
        if decision != InboundDecision::Queue {
            return decision;
        }
        let required_slots = self.outstanding.len().saturating_add(1);
        if !admit_memory(required_slots) {
            return self.refuse(RejectReason::QueueFull);
        }
        // Reserve both bounded containers before mutating either. Runtime
        // adapters lease the expected bytes first; these fallible reserves
        // also turn allocator failure into the protocol's bounded queue-full
        // outcome instead of aborting the peer task.
        if self.queue.try_reserve(1).is_err() || self.outstanding.try_reserve(1).is_err() {
            return self.refuse(RejectReason::QueueFull);
        }
        self.queue.push_back(block);
        self.outstanding.insert(block, RequestState::Queued);
        InboundDecision::Queue
    }

    /// The peer cancelled. A queued request is removed; under Fast it is
    /// answered with a Reject (harmless if the peer already forgot it, and
    /// it keeps the one-answer-per-request rule unconditional). A request
    /// already being served finishes normally.
    pub fn on_cancel(&mut self, block: BlockExtent) -> Option<Reject> {
        if self.outstanding.get(&block) != Some(&RequestState::Queued) {
            return None;
        }
        self.outstanding.remove(&block);
        self.queue.retain(|queued| *queued != block);
        self.fast.then_some(Reject {
            block,
            reason: RejectReason::Cancelled,
        })
    }

    /// We now choke the peer. Returns the rejects to send (Fast) for queued
    /// requests that do not survive; allowed-fast requests stay queued.
    /// Requests already in service are resolved by their `finish` result.
    pub fn on_local_choke(&mut self) -> Vec<Reject> {
        self.choked = true;
        let mut rejects = Vec::new();
        let queued_before_choke = self.queue.len();
        for _ in 0..queued_before_choke {
            let block = self
                .queue
                .pop_front()
                .expect("the queue length was captured before draining");
            if self.is_allowed_fast(block.piece) {
                // Rotate surviving requests through the existing allocation
                // instead of building a second FIFO during the choke pass.
                self.queue.push_back(block);
                continue;
            }
            self.outstanding.remove(&block);
            if self.fast {
                rejects.push(Reject {
                    block,
                    reason: RejectReason::ChokeTransition,
                });
            }
        }
        if !self.fast {
            // Legacy Choke implicitly ends even reads that have started. The
            // actor cancels those jobs; a late completion then finds no live
            // obligation and cannot produce a response.
            self.outstanding
                .retain(|_, state| *state != RequestState::InService);
            self.in_service_len = 0;
        }
        rejects
    }

    pub fn on_local_unchoke(&mut self) {
        self.choked = false;
    }

    /// The request `next_request` would return, without dequeuing it.
    pub fn peek_request(&self) -> Option<BlockExtent> {
        self.queue.front().copied()
    }

    /// Next request to serve, if any.
    pub fn next_request(&mut self) -> Option<BlockExtent> {
        self.next_request_with_choke_priority()
            .map(|(block, _)| block)
    }

    /// Dequeue one request and return whether it remains eligible because of
    /// allowed-fast while choked. The caller can carry this scheduling fact
    /// forward without re-locking the peer state to ask the same question.
    pub(crate) fn next_request_with_choke_priority(&mut self) -> Option<(BlockExtent, bool)> {
        let next = self.queue.front().copied()?;
        let choke_critical = self.choked && self.is_allowed_fast(next.piece);
        if self.choked && !choke_critical {
            return None;
        }
        let block = self.queue.pop_front()?;
        let state = self.outstanding.get_mut(&block)?;
        *state = RequestState::InService;
        self.in_service_len = self.in_service_len.saturating_add(1);
        Some((block, choke_critical))
    }

    /// Return an in-service request to the front of the queue when a
    /// cancellable rate wait ends before storage work can finish. Its
    /// obligation stays live and it keeps its original priority relative to
    /// requests accepted later.
    pub fn requeue_front(&mut self, block: BlockExtent) -> bool {
        if self.outstanding.get(&block) != Some(&RequestState::InService)
            || self.queue.len() >= self.max_outstanding
        {
            return false;
        }
        if let Some(state) = self.outstanding.get_mut(&block) {
            *state = RequestState::Queued;
        }
        self.in_service_len = self.in_service_len.saturating_sub(1);
        self.queue.push_front(block);
        true
    }

    /// Whether the data for an in-service request may still be sent: we must
    /// not send a Piece to a peer we have since choked (except allowed-fast).
    pub fn may_send(&self, block: &BlockExtent) -> bool {
        self.outstanding.get(block) == Some(&RequestState::InService)
            && (!self.choked || self.is_allowed_fast(block.piece))
    }

    /// Resolve an in-service request. A Fast peer gets a Reject for anything
    /// that did not end in a Piece.
    pub fn finish(&mut self, block: BlockExtent, served: Served) -> Option<Reject> {
        if self.outstanding.get(&block) != Some(&RequestState::InService) {
            return None;
        }
        self.outstanding.remove(&block);
        self.in_service_len = self.in_service_len.saturating_sub(1);
        match (served, self.fast) {
            (Served::Sent, _) => None,
            (Served::Failed, true) => Some(Reject {
                block,
                reason: RejectReason::Unavailable,
            }),
            (Served::Discarded, true) => Some(Reject {
                block,
                reason: RejectReason::ChokeTransition,
            }),
            (_, false) => None,
        }
    }

    /// Connection closed: every outstanding obligation ends with it.
    pub fn close(&mut self) -> usize {
        let outstanding = self.outstanding.len();
        self.queue.clear();
        self.outstanding.clear();
        self.in_service_len = 0;
        outstanding
    }

    /// Every admitted request is in exactly one of queued / in service.
    pub fn check_invariants(&self) -> Result<(), String> {
        let queued_count = self
            .outstanding
            .values()
            .filter(|state| **state == RequestState::Queued)
            .count();
        if self.queue.len() != queued_count {
            return Err("queue and request index disagree".into());
        }
        let mut queued_index = HashSet::with_capacity(self.queue.len());
        if self.queue.iter().any(|block| {
            self.outstanding.get(block) != Some(&RequestState::Queued)
                || !queued_index.insert(*block)
        }) || self
            .outstanding
            .iter()
            .any(|(block, state)| *state == RequestState::Queued && !queued_index.contains(block))
        {
            return Err("queue contents and request index disagree".into());
        }
        let indexed_in_service = self
            .outstanding
            .values()
            .filter(|state| **state == RequestState::InService)
            .count();
        if indexed_in_service != self.in_service_len {
            return Err("in-service count and request index disagree".into());
        }
        if self.outstanding.len() > self.max_outstanding {
            return Err("outstanding request limit exceeded".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(piece: u32) -> BlockExtent {
        BlockExtent::new(piece, 0, 16 * 1024)
    }

    #[test]
    fn fast_peer_gets_an_answer_for_every_refusal() {
        let mut fast = InboundObligations::new(true, 2);
        // Choked: rejected, not dropped.
        assert_eq!(
            fast.on_request(b(1), true),
            InboundDecision::Reject(RejectReason::Choked)
        );
        fast.on_local_unchoke();
        assert_eq!(
            fast.on_request(b(1), false),
            InboundDecision::Reject(RejectReason::PieceNotHeld)
        );
        assert_eq!(fast.on_request(b(1), true), InboundDecision::Queue);
        assert_eq!(fast.on_request(b(2), true), InboundDecision::Queue);
        assert_eq!(
            fast.on_request(b(3), true),
            InboundDecision::Reject(RejectReason::QueueFull)
        );
    }

    #[test]
    fn legacy_peer_refusals_are_silent_drops() {
        let mut legacy = InboundObligations::new(false, 1);
        assert_eq!(
            legacy.on_request(b(1), true),
            InboundDecision::Drop(RejectReason::Choked)
        );
        legacy.on_local_unchoke();
        assert_eq!(legacy.on_request(b(1), true), InboundDecision::Queue);
        assert_eq!(
            legacy.on_request(b(2), true),
            InboundDecision::Drop(RejectReason::QueueFull)
        );
        // A choke clears the queue and sends nothing.
        assert!(legacy.on_local_choke().is_empty());
        assert_eq!(legacy.queued_len(), 0);
    }

    #[test]
    fn invalid_wire_lengths_are_rejected_for_fast_and_dropped_for_legacy() {
        let mut fast = InboundObligations::new(true, 8);
        let mut legacy = InboundObligations::new(false, 8);
        fast.on_local_unchoke();
        legacy.on_local_unchoke();

        for length in [0, BLOCK_SIZE + 1] {
            let block = BlockExtent::new(3, 0, length);
            assert_eq!(
                fast.on_request(block, true),
                InboundDecision::Reject(RejectReason::Unavailable)
            );
            assert_eq!(
                legacy.on_request(block, true),
                InboundDecision::Drop(RejectReason::Unavailable)
            );
        }
        assert_eq!(fast.queued_len(), 0);
        assert_eq!(legacy.queued_len(), 0);
    }

    #[test]
    fn duplicates_are_answered_once() {
        let mut ob = InboundObligations::new(true, 8);
        ob.on_local_unchoke();
        assert_eq!(ob.on_request(b(1), true), InboundDecision::Queue);
        assert_eq!(ob.on_request(b(1), true), InboundDecision::Duplicate);
        let (block, choke_critical) = ob.next_request_with_choke_priority().unwrap();
        assert!(
            !choke_critical,
            "an unchoked request needs no priority boost"
        );
        assert_eq!(ob.on_request(block, true), InboundDecision::Duplicate);
        assert_eq!(ob.finish(block, Served::Sent), None);
        // Once served, the same coordinates are a fresh request.
        assert_eq!(ob.on_request(block, true), InboundDecision::Queue);
    }

    #[test]
    fn admission_leases_memory_once_after_protocol_and_duplicate_checks() {
        let mut ob = InboundObligations::new(true, 2);
        ob.on_local_unchoke();

        let mut reservations = Vec::new();
        assert_eq!(
            ob.on_request_with_admission(b(1), true, |slots| {
                reservations.push(slots);
                true
            }),
            InboundDecision::Queue
        );
        assert_eq!(reservations, [1]);

        let mut duplicate_requested_memory = false;
        assert_eq!(
            ob.on_request_with_admission(b(1), true, |_| {
                duplicate_requested_memory = true;
                true
            }),
            InboundDecision::Duplicate
        );
        assert!(!duplicate_requested_memory);

        let mut rejected_requested_memory = false;
        assert_eq!(
            ob.on_request_with_admission(b(3), false, |_| {
                rejected_requested_memory = true;
                true
            }),
            InboundDecision::Reject(RejectReason::PieceNotHeld)
        );
        assert!(!rejected_requested_memory);
        ob.check_invariants().unwrap();
    }

    #[test]
    fn duplicate_at_capacity_stays_a_duplicate_and_total_work_is_bounded() {
        let mut ob = InboundObligations::new(true, 2);
        ob.on_local_unchoke();
        assert_eq!(ob.on_request(b(1), true), InboundDecision::Queue);
        assert_eq!(ob.on_request(b(2), true), InboundDecision::Queue);
        assert_eq!(ob.next_request(), Some(b(1)));
        assert_eq!(ob.on_request(b(1), true), InboundDecision::Duplicate);
        assert_eq!(
            ob.on_request(b(3), true),
            InboundDecision::Reject(RejectReason::QueueFull)
        );
        ob.check_invariants().unwrap();
    }

    #[test]
    fn choke_rejects_queued_requests_but_keeps_allowed_fast() {
        let mut ob = InboundObligations::new(true, 8);
        ob.allow_fast(7);
        ob.on_local_unchoke();
        ob.on_request(b(1), true);
        ob.on_request(b(7), true);
        let rejects = ob.on_local_choke();
        assert_eq!(
            rejects,
            vec![Reject {
                block: b(1),
                reason: RejectReason::ChokeTransition
            }]
        );
        assert_eq!(ob.queued_len(), 1, "the allowed-fast request survives");
        // While choked, allowed-fast pieces are still admitted.
        // The allowed-fast request is still queued, so a repeat is a duplicate;
        // a *new* allowed-fast request is admitted even while choked.
        assert_eq!(ob.on_request(b(7), true), InboundDecision::Duplicate);
        ob.allow_fast(8);
        assert_eq!(ob.on_request(b(8), true), InboundDecision::Queue);
        assert_eq!(ob.next_request_with_choke_priority(), Some((b(7), true)));
        assert!(ob.may_send(&b(7)));
        ob.check_invariants().unwrap();
    }

    #[test]
    fn data_is_not_sent_to_a_peer_choked_mid_read() {
        let mut ob = InboundObligations::new(true, 8);
        ob.on_local_unchoke();
        ob.on_request(b(1), true);
        let block = ob.next_request().unwrap();
        assert!(ob.may_send(&block));
        ob.on_local_choke();
        assert!(!ob.may_send(&block));
        // The discarded read is still answered under Fast.
        assert_eq!(
            ob.finish(block, Served::Discarded),
            Some(Reject {
                block,
                reason: RejectReason::ChokeTransition
            })
        );
    }

    #[test]
    fn legacy_choke_ends_in_service_obligations_without_rejects() {
        let mut ob = InboundObligations::new(false, 8);
        ob.on_local_unchoke();
        assert_eq!(ob.on_request(b(1), true), InboundDecision::Queue);
        let block = ob.next_request().unwrap();
        assert_eq!(ob.in_service_len(), 1);

        assert!(ob.on_local_choke().is_empty());
        assert_eq!(ob.in_service_len(), 0);
        assert_eq!(ob.finish(block, Served::Failed), None);
        ob.check_invariants().unwrap();
    }

    #[test]
    fn failed_reads_are_rejected_for_fast_and_silent_for_legacy() {
        let mut fast = InboundObligations::new(true, 8);
        let mut legacy = InboundObligations::new(false, 8);
        for ob in [&mut fast, &mut legacy] {
            ob.on_local_unchoke();
            ob.on_request(b(1), true);
        }
        let fb = fast.next_request().unwrap();
        let lb = legacy.next_request().unwrap();
        assert_eq!(
            fast.finish(fb, Served::Failed),
            Some(Reject {
                block: fb,
                reason: RejectReason::Unavailable
            })
        );
        assert_eq!(legacy.finish(lb, Served::Failed), None);
    }

    #[test]
    fn cancel_removes_a_queued_request_and_fast_still_answers_it() {
        let mut fast = InboundObligations::new(true, 8);
        fast.on_local_unchoke();
        fast.on_request(b(1), true);
        assert_eq!(
            fast.on_cancel(b(1)),
            Some(Reject {
                block: b(1),
                reason: RejectReason::Cancelled
            })
        );
        assert_eq!(fast.on_cancel(b(1)), None);
        assert_eq!(fast.queued_len(), 0);
        fast.check_invariants().unwrap();
    }

    #[test]
    fn interrupted_in_service_request_can_be_requeued_without_losing_its_obligation() {
        let mut ob = InboundObligations::new(true, 8);
        ob.on_local_unchoke();
        assert_eq!(ob.on_request(b(1), true), InboundDecision::Queue);
        assert_eq!(ob.on_request(b(2), true), InboundDecision::Queue);

        let first = ob.next_request().unwrap();
        assert_eq!(first, b(1));
        assert_eq!(ob.on_cancel(first), None, "in-service work remains owed");
        assert!(ob.requeue_front(first));
        assert_eq!(ob.queued_len(), 2);
        assert_eq!(ob.in_service_len(), 0);
        assert_eq!(ob.next_request(), Some(first));
        assert_eq!(ob.finish(first, Served::Sent), None);
        assert_eq!(ob.next_request(), Some(b(2)));
        ob.check_invariants().unwrap();
    }

    #[test]
    fn closing_ends_every_obligation() {
        let mut ob = InboundObligations::new(true, 8);
        ob.on_local_unchoke();
        ob.on_request(b(1), true);
        ob.on_request(b(2), true);
        ob.next_request();
        assert_eq!(ob.close(), 2);
        assert_eq!(ob.queued_len() + ob.in_service_len(), 0);
    }
}
