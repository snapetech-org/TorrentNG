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
//! It is shared by v1, pure-v2 and hybrid peers: the wire obligation does not
//! depend on the hash plane.

use std::collections::{HashSet, VecDeque};

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
    queued: HashSet<BlockExtent>,
    in_service: HashSet<BlockExtent>,
    max_queue: usize,
}

impl InboundObligations {
    pub fn new(fast_negotiated: bool, max_queue: usize) -> Self {
        Self {
            fast: fast_negotiated,
            choked: true,
            allowed_fast: HashSet::new(),
            queue: VecDeque::new(),
            queued: HashSet::new(),
            in_service: HashSet::new(),
            max_queue,
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
        self.in_service.len()
    }

    pub fn allow_fast(&mut self, piece: u32) {
        self.allowed_fast.insert(piece);
    }

    fn refuse(&self, reason: RejectReason) -> InboundDecision {
        if self.fast {
            InboundDecision::Reject(reason)
        } else {
            InboundDecision::Drop(reason)
        }
    }

    /// A request arrived. `have_piece` is whether we can serve its piece.
    pub fn on_request(&mut self, block: BlockExtent, have_piece: bool) -> InboundDecision {
        if self.queued.contains(&block) || self.in_service.contains(&block) {
            return InboundDecision::Duplicate;
        }
        if block.length == 0 || block.length > BLOCK_SIZE {
            return self.refuse(RejectReason::Unavailable);
        }
        if self.choked && !(self.fast && self.allowed_fast.contains(&block.piece)) {
            return self.refuse(RejectReason::Choked);
        }
        if !have_piece {
            return self.refuse(RejectReason::PieceNotHeld);
        }
        if self.queue.len() >= self.max_queue {
            return self.refuse(RejectReason::QueueFull);
        }
        self.queue.push_back(block);
        self.queued.insert(block);
        InboundDecision::Queue
    }

    /// The peer cancelled. A queued request is removed; under Fast it is
    /// answered with a Reject (harmless if the peer already forgot it, and
    /// it keeps the one-answer-per-request rule unconditional). A request
    /// already being served finishes normally.
    pub fn on_cancel(&mut self, block: BlockExtent) -> Option<Reject> {
        if !self.queued.remove(&block) {
            return None;
        }
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
        let mut kept = VecDeque::new();
        while let Some(block) = self.queue.pop_front() {
            if self.fast && self.allowed_fast.contains(&block.piece) {
                kept.push_back(block);
                continue;
            }
            self.queued.remove(&block);
            if self.fast {
                rejects.push(Reject {
                    block,
                    reason: RejectReason::ChokeTransition,
                });
            }
        }
        self.queue = kept;
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
        let block = self.queue.pop_front()?;
        self.queued.remove(&block);
        self.in_service.insert(block);
        Some(block)
    }

    /// Whether the data for an in-service request may still be sent: we must
    /// not send a Piece to a peer we have since choked (except allowed-fast).
    pub fn may_send(&self, block: &BlockExtent) -> bool {
        self.in_service.contains(block)
            && (!self.choked || (self.fast && self.allowed_fast.contains(&block.piece)))
    }

    /// Resolve an in-service request. A Fast peer gets a Reject for anything
    /// that did not end in a Piece.
    pub fn finish(&mut self, block: BlockExtent, served: Served) -> Option<Reject> {
        if !self.in_service.remove(&block) {
            return None;
        }
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
        let outstanding = self.queue.len() + self.in_service.len();
        self.queue.clear();
        self.queued.clear();
        self.in_service.clear();
        outstanding
    }

    /// Every admitted request is in exactly one of queued / in service.
    pub fn check_invariants(&self) -> Result<(), String> {
        if self.queue.len() != self.queued.len() {
            return Err("queue and index disagree".into());
        }
        if self
            .queue
            .iter()
            .any(|block| self.in_service.contains(block))
        {
            return Err("a request is both queued and in service".into());
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
        let block = ob.next_request().unwrap();
        assert_eq!(ob.on_request(block, true), InboundDecision::Duplicate);
        assert_eq!(ob.finish(block, Served::Sent), None);
        // Once served, the same coordinates are a fresh request.
        assert_eq!(ob.on_request(block, true), InboundDecision::Queue);
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
