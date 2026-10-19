//! Process-wide owner for live inbound upload obligations.
//!
//! The actor loop still owns its socket, read futures, and wire projection.
//! This registry owns protocol state keyed by generation-safe transfer
//! identity, so a late completion cannot mutate a replacement connection.

use std::collections::HashMap;
use std::ops::Deref;
use std::sync::{Arc, Mutex, MutexGuard};

use super::coords::{BlockExtent, TransferPeerId};
use super::obligations::{InboundDecision, InboundObligations, Reject, Served};
use super::upload_scheduler::{
    UploadReadAdmissionError, UploadReadConnection, UploadReadLimits, UploadReadPermit,
    UploadReadScheduler,
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
    pub(crate) fn new(max_connections: usize, read_limits: UploadReadLimits) -> Self {
        Self {
            obligations: SharedUploadObligations::new(max_connections),
            reads: UploadReadScheduler::new(read_limits),
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
            .register(identity, fast_negotiated, max_outstanding)
            .map_err(LiveUploadRegistrationError::Obligations)?;
        Ok(SharedUploadPeer {
            _read_registration: read_registration,
            obligations,
            service: self.clone(),
            identity,
        })
    }

    async fn acquire_read(
        &self,
        identity: TransferPeerId,
        block: BlockExtent,
    ) -> Result<UploadReadPermit, UploadReadAdmissionError> {
        self.reads.acquire(identity, block).await
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
    connections: Mutex<HashMap<TransferPeerId, Arc<Mutex<InboundObligations>>>>,
}

pub(crate) struct SharedUploadConnection {
    service: SharedUploadObligations,
    identity: TransferPeerId,
    state: Arc<Mutex<InboundObligations>>,
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
    ) -> Result<SharedUploadConnection, SharedUploadRegistrationError> {
        let state = Arc::new(Mutex::new(InboundObligations::new(
            fast_negotiated,
            max_outstanding,
        )));
        let mut connections = lock(&self.inner.connections);
        if connections.contains_key(&identity) {
            return Err(SharedUploadRegistrationError::AlreadyRegistered);
        }
        if connections.len() >= self.inner.max_connections {
            return Err(SharedUploadRegistrationError::ConnectionLimit);
        }
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

    fn unregister(&self, identity: TransferPeerId, expected: &Arc<Mutex<InboundObligations>>) {
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
            lock(&state).close();
        }
    }
}

impl SharedUploadConnection {
    pub(crate) fn identity(&self) -> TransferPeerId {
        self.identity
    }

    pub(crate) fn admit(&self, block: BlockExtent, have_piece: bool) -> InboundDecision {
        lock(&self.state).on_request(block, have_piece)
    }

    pub(crate) fn on_cancel(&self, block: BlockExtent) -> Option<Reject> {
        lock(&self.state).on_cancel(block)
    }

    pub(crate) fn on_local_unchoke(&self) {
        lock(&self.state).on_local_unchoke();
    }

    pub(crate) fn on_local_choke(&self) -> Vec<Reject> {
        lock(&self.state).on_local_choke()
    }

    pub(crate) fn next_request(&self) -> Option<BlockExtent> {
        lock(&self.state).next_request()
    }

    pub(crate) fn requeue_front(&self, block: BlockExtent) -> bool {
        lock(&self.state).requeue_front(block)
    }

    pub(crate) fn may_send(&self, block: &BlockExtent) -> bool {
        lock(&self.state).may_send(block)
    }

    pub(crate) fn is_allowed_fast(&self, piece: u32) -> bool {
        lock(&self.state).is_allowed_fast(piece)
    }

    pub(crate) fn finish(&self, block: BlockExtent, served: Served) -> Option<Reject> {
        lock(&self.state).finish(block, served)
    }

    pub(crate) fn close(&self) -> usize {
        lock(&self.state).close()
    }

    pub(crate) fn queued_len(&self) -> usize {
        lock(&self.state).queued_len()
    }

    pub(crate) fn is_choked(&self) -> bool {
        lock(&self.state).is_choked()
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
        let connection = service.register(identity(1), true, 2).unwrap();
        assert_eq!(clone.active_connections(), 1);
        assert!(matches!(
            clone.register(identity(2), true, 2),
            Err(SharedUploadRegistrationError::ConnectionLimit)
        ));
        assert_eq!(
            connection.admit(BlockExtent::new(4, 0, 16 * 1024), true),
            InboundDecision::Reject(RejectReason::Choked)
        );
        drop(connection);
        assert_eq!(clone.active_connections(), 0);
        assert!(clone.register(identity(2), true, 2).is_ok());
    }

    #[tokio::test]
    async fn live_upload_registration_couples_obligations_and_read_budget() {
        let service = SharedUploadService::new(
            1,
            UploadReadLimits {
                max_connections: 2,
                ..UploadReadLimits::default()
            },
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
}
