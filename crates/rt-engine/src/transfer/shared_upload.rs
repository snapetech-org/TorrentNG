//! Process-wide owner for live inbound upload obligations.
//!
//! The actor loop still owns its socket, read futures, and wire projection.
//! This registry owns protocol state keyed by generation-safe transfer
//! identity, so a late completion cannot mutate a replacement connection.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard};

use rt_metrics::{MemoryClass, MemoryLease, ResourceGovernor};

use super::coords::{BlockExtent, TransferPeerId};
use super::obligations::{InboundDecision, InboundObligations, Reject, Served};
use super::upload_scheduler::{
    UploadReadAdmissionError, UploadReadConnection, UploadReadLimits, UploadReadPermit,
    UploadReadPermits, UploadReadScheduler,
};

/// One live, process-wide owner for inbound upload obligations and the
/// expensive read admission that serves them. The actor keeps the socket and
/// response tasks; this service couples their protocol lifetime to the global
/// job/byte budget so a connection cannot register only half of the upload
/// path.
#[derive(Clone)]
pub(crate) struct SharedUploadService {
    obligations: SharedUploadObligations,
    reads: UploadReadScheduler,
    resources: ResourceGovernor,
}

/// Amortized per-request charge for the retained FIFO entry and hash index
/// bucket. The charge follows the connection's high-water mark because
/// cancellation retains collection capacity until that peer disconnects.
const INBOUND_REQUEST_BOOKKEEPING_BASE_BYTES: u64 = 256;
const INBOUND_REQUEST_BOOKKEEPING_BYTES_PER_SLOT: u64 = 64;

fn inbound_request_bookkeeping_bytes(slots: usize) -> Option<u64> {
    if slots == 0 {
        return Some(0);
    }
    let slots = u64::try_from(slots).ok()?;
    INBOUND_REQUEST_BOOKKEEPING_BASE_BYTES
        .checked_add(slots.checked_mul(INBOUND_REQUEST_BOOKKEEPING_BYTES_PER_SLOT)?)
}

/// Resource-governed admission for request-coordinate metadata. Kept in the
/// runtime adapter so the pure BEP 6 state machine remains independent of
/// engine resource policy.
pub(crate) struct InboundBookkeepingBudget {
    resources: ResourceGovernor,
    lease: Option<MemoryLease>,
    reserved_slots: usize,
}

impl InboundBookkeepingBudget {
    pub(crate) fn new(resources: ResourceGovernor) -> Self {
        Self {
            resources,
            lease: None,
            reserved_slots: 0,
        }
    }

    pub(crate) fn admit(
        &mut self,
        obligations: &mut InboundObligations,
        block: BlockExtent,
        have_piece: bool,
    ) -> InboundDecision {
        obligations.on_request_with_admission(block, have_piece, |required_slots| {
            self.reserve_slots(required_slots)
        })
    }

    fn reserve_slots(&mut self, required_slots: usize) -> bool {
        if required_slots <= self.reserved_slots {
            return true;
        }
        let Some(required_bytes) = inbound_request_bookkeeping_bytes(required_slots) else {
            return false;
        };
        let Some(reserved_bytes) = inbound_request_bookkeeping_bytes(self.reserved_slots) else {
            return false;
        };
        let additional_bytes = required_bytes.saturating_sub(reserved_bytes);

        let admitted = match self.lease.as_mut() {
            Some(lease) => lease.try_grow(additional_bytes),
            None => {
                self.lease = self
                    .resources
                    .try_acquire(MemoryClass::PeerBuffer, additional_bytes);
                self.lease.is_some()
            }
        };
        if admitted {
            self.reserved_slots = required_slots;
        }
        admitted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiveUploadRegistrationError {
    Obligations(SharedUploadRegistrationError),
    Read(UploadReadAdmissionError),
}

/// A registered live upload connection. Deref keeps the protocol-obligation
/// API concise at actor call sites; the same registration owns the global
/// read slot until the peer loop exits.
pub(crate) struct SharedUploadPeer {
    // Rust drops struct fields in declaration order. Revoke read admission
    // before removing BEP 6 state so cloned worker handles cannot begin work
    // after the protocol registration starts closing.
    _read_registration: UploadReadConnection,
    obligations: SharedUploadConnection,
    service: SharedUploadService,
    identity: TransferPeerId,
}

/// Cloneable read capability scoped to one registered peer identity. A
/// worker may keep the handle while the peer loop owns the registration; once
/// the loop drops its `SharedUploadPeer`, the scheduler rejects new reads.
#[derive(Clone)]
pub(crate) struct SharedUploadReadHandle {
    service: SharedUploadService,
    identity: TransferPeerId,
}

impl SharedUploadService {
    pub(crate) fn new(
        max_connections: usize,
        read_limits: UploadReadLimits,
        resources: ResourceGovernor,
    ) -> Self {
        Self {
            obligations: SharedUploadObligations::new(max_connections),
            reads: UploadReadScheduler::new(read_limits),
            resources,
        }
    }

    pub(crate) fn register(
        &self,
        identity: TransferPeerId,
        fast_negotiated: bool,
        max_outstanding: usize,
    ) -> Result<SharedUploadPeer, LiveUploadRegistrationError> {
        let read_registration = self
            .reads
            .register_guard(identity)
            .map_err(LiveUploadRegistrationError::Read)?;
        let obligations = self
            .obligations
            .register(
                identity,
                fast_negotiated,
                max_outstanding,
                self.resources.clone(),
            )
            .map_err(LiveUploadRegistrationError::Obligations)?;
        Ok(SharedUploadPeer {
            _read_registration: read_registration,
            obligations,
            service: self.clone(),
            identity,
        })
    }

    #[cfg(test)]
    pub(crate) fn in_flight_read_jobs(&self) -> usize {
        self.reads.in_flight().0
    }

    async fn acquire_read(
        &self,
        identity: TransferPeerId,
        block: BlockExtent,
    ) -> Result<UploadReadPermit, UploadReadAdmissionError> {
        self.reads.acquire(identity, block).await
    }

    async fn acquire_read_batch(
        &self,
        identity: TransferPeerId,
        blocks: &[BlockExtent],
    ) -> Result<UploadReadPermits, UploadReadAdmissionError> {
        self.reads.acquire_batch(identity, blocks).await
    }
}

impl SharedUploadPeer {
    pub(crate) fn read_admission(&self) -> SharedUploadReadHandle {
        SharedUploadReadHandle {
            service: self.service.clone(),
            identity: self.identity,
        }
    }
}

impl SharedUploadReadHandle {
    pub(crate) async fn acquire_read(
        &self,
        block: BlockExtent,
    ) -> Result<UploadReadPermit, UploadReadAdmissionError> {
        self.service.acquire_read(self.identity, block).await
    }

    pub(crate) async fn acquire_read_batch(
        &self,
        blocks: &[BlockExtent],
    ) -> Result<UploadReadPermits, UploadReadAdmissionError> {
        self.service.acquire_read_batch(self.identity, blocks).await
    }

    #[cfg(test)]
    pub(crate) fn in_flight_read_jobs(&self) -> usize {
        self.service.in_flight_read_jobs()
    }
}

impl Deref for SharedUploadPeer {
    type Target = SharedUploadConnection;

    fn deref(&self) -> &Self::Target {
        &self.obligations
    }
}

#[derive(Clone)]
pub(crate) struct SharedUploadObligations {
    inner: Arc<Inner>,
}

struct Inner {
    max_connections: usize,
    connections: Mutex<HashMap<TransferPeerId, Arc<Mutex<SharedUploadConnectionState>>>>,
}

pub(crate) struct SharedUploadConnection {
    service: SharedUploadObligations,
    identity: TransferPeerId,
    state: Arc<Mutex<SharedUploadConnectionState>>,
}

struct SharedUploadConnectionState {
    obligations: InboundObligations,
    bookkeeping: InboundBookkeepingBudget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedUploadRegistrationError {
    AlreadyRegistered,
    ConnectionLimit,
}

impl SharedUploadObligations {
    pub(crate) fn new(max_connections: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                max_connections: max_connections.max(1),
                connections: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub(crate) fn register(
        &self,
        identity: TransferPeerId,
        fast_negotiated: bool,
        max_outstanding: usize,
        resources: ResourceGovernor,
    ) -> Result<SharedUploadConnection, SharedUploadRegistrationError> {
        let mut connections = lock(&self.inner.connections);
        if connections.contains_key(&identity) {
            return Err(SharedUploadRegistrationError::AlreadyRegistered);
        }
        if connections.len() >= self.inner.max_connections {
            return Err(SharedUploadRegistrationError::ConnectionLimit);
        }
        let state = Arc::new(Mutex::new(SharedUploadConnectionState {
            obligations: InboundObligations::new(fast_negotiated, max_outstanding),
            bookkeeping: InboundBookkeepingBudget::new(resources),
        }));
        connections.insert(identity, Arc::clone(&state));
        Ok(SharedUploadConnection {
            service: self.clone(),
            identity,
            state,
        })
    }

    pub(crate) fn active_connections(&self) -> usize {
        lock(&self.inner.connections).len()
    }

    fn unregister(
        &self,
        identity: TransferPeerId,
        expected: &Arc<Mutex<SharedUploadConnectionState>>,
    ) {
        let removed = {
            let mut connections = lock(&self.inner.connections);
            if connections
                .get(&identity)
                .is_some_and(|current| Arc::ptr_eq(current, expected))
            {
                connections.remove(&identity)
            } else {
                None
            }
        };
        if let Some(state) = removed {
            lock(&state).obligations.close();
        }
    }
}

impl SharedUploadConnection {
    pub(crate) fn identity(&self) -> TransferPeerId {
        self.identity
    }

    pub(crate) fn admit(&self, block: BlockExtent, have_piece: bool) -> InboundDecision {
        let mut state = lock(&self.state);
        let SharedUploadConnectionState {
            obligations,
            bookkeeping,
        } = &mut *state;
        bookkeeping.admit(obligations, block, have_piece)
    }

    pub(crate) fn on_cancel(&self, block: BlockExtent) -> Option<Reject> {
        lock(&self.state).obligations.on_cancel(block)
    }

    pub(crate) fn on_local_unchoke(&self) {
        lock(&self.state).obligations.on_local_unchoke();
    }

    pub(crate) fn on_local_choke(&self) -> Vec<Reject> {
        lock(&self.state).obligations.on_local_choke()
    }

    pub(crate) fn next_request(&self) -> Option<BlockExtent> {
        lock(&self.state).obligations.next_request()
    }

    pub(crate) fn peek_request(&self) -> Option<BlockExtent> {
        lock(&self.state).obligations.peek_request()
    }

    pub(crate) fn requeue_front(&self, block: BlockExtent) -> bool {
        lock(&self.state).obligations.requeue_front(block)
    }

    pub(crate) fn may_send(&self, block: &BlockExtent) -> bool {
        lock(&self.state).obligations.may_send(block)
    }

    pub(crate) fn is_allowed_fast(&self, piece: u32) -> bool {
        lock(&self.state).obligations.is_allowed_fast(piece)
    }

    pub(crate) fn finish(&self, block: BlockExtent, served: Served) -> Option<Reject> {
        lock(&self.state).obligations.finish(block, served)
    }

    pub(crate) fn close(&self) -> usize {
        lock(&self.state).obligations.close()
    }

    pub(crate) fn queued_len(&self) -> usize {
        lock(&self.state).obligations.queued_len()
    }

    pub(crate) fn is_choked(&self) -> bool {
        lock(&self.state).obligations.is_choked()
    }
}

impl Drop for SharedUploadConnection {
    fn drop(&mut self) {
        self.service.unregister(self.identity, &self.state);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            let guard = poisoned.into_inner();
            mutex.clear_poison();
            guard
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::coords::{ConnectionId, TorrentKey};
    use crate::transfer::obligations::RejectReason;
    use rt_metrics::ResourceGovernorConfig;

    fn identity(generation: u32) -> TransferPeerId {
        TransferPeerId::new(
            TorrentKey(9),
            ConnectionId {
                slot: 3,
                generation,
            },
        )
    }

    #[test]
    fn registration_and_protocol_state_are_shared_across_service_clones() {
        let service = SharedUploadObligations::new(1);
        let clone = service.clone();
        let connection = service
            .register(
                identity(1),
                true,
                2,
                ResourceGovernor::new(ResourceGovernorConfig::default()),
            )
            .unwrap();
        assert_eq!(clone.active_connections(), 1);
        assert!(matches!(
            clone.register(
                identity(2),
                true,
                2,
                ResourceGovernor::new(ResourceGovernorConfig::default())
            ),
            Err(SharedUploadRegistrationError::ConnectionLimit)
        ));
        assert_eq!(
            connection.admit(BlockExtent::new(4, 0, 16 * 1024), true),
            InboundDecision::Reject(RejectReason::Choked)
        );
        drop(connection);
        assert_eq!(clone.active_connections(), 0);
        assert!(clone
            .register(
                identity(2),
                true,
                2,
                ResourceGovernor::new(ResourceGovernorConfig::default()),
            )
            .is_ok());
    }

    #[tokio::test]
    async fn live_upload_registration_couples_obligations_and_read_budget() {
        let service = SharedUploadService::new(
            1,
            UploadReadLimits {
                max_connections: 2,
                ..UploadReadLimits::default()
            },
            ResourceGovernor::new(ResourceGovernorConfig::default()),
        );
        let peer = service.register(identity(1), true, 2).unwrap();
        peer.on_local_unchoke();
        assert_eq!(
            peer.admit(BlockExtent::new(4, 0, 16 * 1024), true),
            InboundDecision::Queue
        );
        let permit = peer
            .read_admission()
            .acquire_read(BlockExtent::new(4, 0, 16 * 1024))
            .await
            .unwrap();
        drop(permit);
        let stale_read_admission = peer.read_admission();

        // A partial registration must release the read-side slot when the
        // obligation table refuses the connection.
        assert!(matches!(
            service.register(identity(2), true, 2),
            Err(LiveUploadRegistrationError::Obligations(
                SharedUploadRegistrationError::ConnectionLimit
            ))
        ));
        drop(peer);
        assert!(matches!(
            stale_read_admission
                .acquire_read(BlockExtent::new(4, 0, 16 * 1024))
                .await,
            Err(UploadReadAdmissionError::NotRegistered)
        ));
        assert!(service.register(identity(2), true, 2).is_ok());
    }

    #[test]
    fn inbound_obligation_metadata_is_charged_to_peer_buffer_budget() {
        let mut config = ResourceGovernorConfig::default();
        let two_slots = inbound_request_bookkeeping_bytes(2).unwrap();
        config.total_cap_bytes = two_slots;
        config.class_caps_bytes[MemoryClass::PeerBuffer as usize] = two_slots;
        let resources = ResourceGovernor::new(config);
        let service = SharedUploadService::new(1, UploadReadLimits::default(), resources.clone());
        let peer = service.register(identity(1), true, 8).unwrap();
        peer.on_local_unchoke();

        assert_eq!(
            peer.admit(BlockExtent::new(1, 0, 16 * 1024), true),
            InboundDecision::Queue
        );
        assert_eq!(
            peer.admit(BlockExtent::new(1, 16 * 1024, 16 * 1024), true),
            InboundDecision::Queue
        );
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            two_slots
        );

        let first = peer.next_request().unwrap();
        assert_eq!(
            peer.admit(BlockExtent::new(1, 2 * 16 * 1024, 16 * 1024), true),
            InboundDecision::Reject(RejectReason::QueueFull),
            "the next retained slot is refused when its memory charge cannot grow"
        );
        assert_eq!(peer.queued_len(), 1);
        assert_eq!(peer.finish(first, Served::Sent), None);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            two_slots,
            "capacity remains charged while the connection retains it"
        );
        drop(peer);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            0
        );
    }

    #[test]
    fn denied_request_metadata_is_rejected_before_queue_allocation() {
        let mut config = ResourceGovernorConfig::default();
        let one_slot = inbound_request_bookkeeping_bytes(1).unwrap();
        config.total_cap_bytes = one_slot - 1;
        config.class_caps_bytes[MemoryClass::PeerBuffer as usize] = one_slot - 1;
        let resources = ResourceGovernor::new(config);
        let service = SharedUploadService::new(1, UploadReadLimits::default(), resources.clone());
        let peer = service.register(identity(1), true, 8).unwrap();
        peer.on_local_unchoke();

        assert_eq!(
            peer.admit(BlockExtent::new(1, 0, 16 * 1024), true),
            InboundDecision::Reject(RejectReason::QueueFull)
        );
        assert_eq!(peer.queued_len(), 0);
        assert_eq!(peer.close(), 0);
        drop(peer);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].denied_allocations,
            1
        );

        let legacy_peer = service.register(identity(2), false, 8).unwrap();
        legacy_peer.on_local_unchoke();
        assert_eq!(
            legacy_peer.admit(BlockExtent::new(2, 0, 16 * 1024), true),
            InboundDecision::Drop(RejectReason::QueueFull),
            "legacy peers do not receive a Fast reject when memory is unavailable"
        );
        assert_eq!(legacy_peer.queued_len(), 0);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].denied_allocations,
            2
        );
    }
}
