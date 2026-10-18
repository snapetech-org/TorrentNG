use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc,
};
use std::time::Instant;

pub const MEMORY_CLASS_COUNT: usize = 10;

static NEXT_MEMORY_LEASE_ID: AtomicU64 = AtomicU64::new(1);

fn allocate_memory_lease_id() -> Option<u64> {
    let mut current = NEXT_MEMORY_LEASE_ID.load(Ordering::Acquire);
    loop {
        let next = current.checked_add(1)?;
        match NEXT_MEMORY_LEASE_ID.compare_exchange_weak(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Some(current),
            Err(observed) => current = observed,
        }
    }
}

/// A deliberately narrow category for a payload copy observed at a known
/// engine copy site. These counters are not a whole-process copy total.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PayloadCopyPath {
    PeerV2Assembly,
    PeerFileSegment,
    WebseedReassembly,
}

/// Cumulative bytes copied at the currently instrumented transfer sites.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PayloadCopySnapshot {
    pub peer_v2_assembly_bytes: u64,
    pub peer_file_segment_bytes: u64,
    pub webseed_reassembly_bytes: u64,
}

#[derive(Debug, Default)]
struct PayloadCopyCounters {
    peer_v2_assembly_bytes: AtomicU64,
    peer_file_segment_bytes: AtomicU64,
    webseed_reassembly_bytes: AtomicU64,
}

impl PayloadCopyCounters {
    fn record(&self, path: PayloadCopyPath, bytes: u64) {
        let counter = match path {
            PayloadCopyPath::PeerV2Assembly => &self.peer_v2_assembly_bytes,
            PayloadCopyPath::PeerFileSegment => &self.peer_file_segment_bytes,
            PayloadCopyPath::WebseedReassembly => &self.webseed_reassembly_bytes,
        };
        counter.fetch_add(bytes, Ordering::Relaxed);
    }

    fn snapshot(&self) -> PayloadCopySnapshot {
        PayloadCopySnapshot {
            peer_v2_assembly_bytes: self.peer_v2_assembly_bytes.load(Ordering::Relaxed),
            peer_file_segment_bytes: self.peer_file_segment_bytes.load(Ordering::Relaxed),
            webseed_reassembly_bytes: self.webseed_reassembly_bytes.load(Ordering::Relaxed),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum MemoryClass {
    StorageFrame = 0,
    PieceAssembly = 1,
    PeerBuffer = 2,
    WebseedBody = 3,
    Metadata = 4,
    TrackerPeers = 5,
    DhtTable = 6,
    QueuedDisk = 7,
    ApiSnapshot = 8,
    PieceIndex = 9,
}

impl MemoryClass {
    pub const ALL: [MemoryClass; MEMORY_CLASS_COUNT] = [
        MemoryClass::StorageFrame,
        MemoryClass::PieceAssembly,
        MemoryClass::PeerBuffer,
        MemoryClass::WebseedBody,
        MemoryClass::Metadata,
        MemoryClass::TrackerPeers,
        MemoryClass::DhtTable,
        MemoryClass::QueuedDisk,
        MemoryClass::ApiSnapshot,
        MemoryClass::PieceIndex,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            MemoryClass::StorageFrame => "storage_frame",
            MemoryClass::PieceAssembly => "piece_assembly",
            MemoryClass::PeerBuffer => "peer_buffer",
            MemoryClass::WebseedBody => "webseed_body",
            MemoryClass::Metadata => "metadata",
            MemoryClass::TrackerPeers => "tracker_peers",
            MemoryClass::DhtTable => "dht_table",
            MemoryClass::QueuedDisk => "queued_disk",
            MemoryClass::ApiSnapshot => "api_snapshot",
            MemoryClass::PieceIndex => "piece_index",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryPressure {
    Normal,
    Constrained,
    Critical,
}

impl MemoryPressure {
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryPressure::Normal => "normal",
            MemoryPressure::Constrained => "constrained",
            MemoryPressure::Critical => "critical",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceGovernorConfig {
    pub total_cap_bytes: u64,
    pub class_caps_bytes: [u64; MEMORY_CLASS_COUNT],
    pub pressure_constrained_pct: u8,
    pub pressure_critical_pct: u8,
}

impl Default for ResourceGovernorConfig {
    fn default() -> Self {
        let mib = 1024 * 1024;
        Self {
            total_cap_bytes: 512 * mib,
            class_caps_bytes: [
                128 * mib,
                128 * mib,
                128 * mib,
                32 * mib,
                32 * mib,
                32 * mib,
                32 * mib,
                32 * mib,
                16 * mib,
                128 * mib,
            ],
            pressure_constrained_pct: 75,
            pressure_critical_pct: 90,
        }
    }
}

#[derive(Debug)]
struct ResourceGovernorInner {
    config: ResourceGovernorConfig,
    total_used: AtomicU64,
    class_used: [AtomicU64; MEMORY_CLASS_COUNT],
    denied: [AtomicU64; MEMORY_CLASS_COUNT],
    payload_copies: PayloadCopyCounters,
}

#[derive(Debug, Clone)]
pub struct ResourceGovernor {
    inner: Arc<ResourceGovernorInner>,
}

impl ResourceGovernor {
    pub fn new(config: ResourceGovernorConfig) -> Self {
        Self {
            inner: Arc::new(ResourceGovernorInner {
                config,
                total_used: AtomicU64::new(0),
                class_used: std::array::from_fn(|_| AtomicU64::new(0)),
                denied: std::array::from_fn(|_| AtomicU64::new(0)),
                payload_copies: PayloadCopyCounters::default(),
            }),
        }
    }

    /// Record bytes copied at one of the explicitly instrumented transfer
    /// sites. This relaxed counter does not participate in ownership or
    /// synchronization decisions.
    pub fn record_payload_copy(&self, path: PayloadCopyPath, bytes: u64) {
        self.inner.payload_copies.record(path, bytes);
    }

    /// Snapshot cumulative counts from known copy sites. Uninstrumented copies
    /// elsewhere in the process are intentionally excluded.
    pub fn payload_copy_snapshot(&self) -> PayloadCopySnapshot {
        self.inner.payload_copies.snapshot()
    }

    pub fn try_acquire(&self, class: MemoryClass, bytes: u64) -> Option<MemoryLease> {
        self.try_reserve_lease_bytes(class, bytes)
            .then(|| MemoryLease::new(self.clone(), class, bytes))
    }

    fn try_reserve_lease_bytes(&self, class: MemoryClass, bytes: u64) -> bool {
        if bytes == 0 {
            return true;
        }

        let class_idx = class.index();
        let class_cap = self.inner.config.class_caps_bytes[class_idx];
        if class_cap == 0 || bytes > class_cap || bytes > self.inner.config.total_cap_bytes {
            self.inner.denied[class_idx].fetch_add(1, Ordering::Relaxed);
            return false;
        }

        if !reserve(&self.inner.class_used[class_idx], bytes, class_cap) {
            self.inner.denied[class_idx].fetch_add(1, Ordering::Relaxed);
            return false;
        }

        if !reserve(
            &self.inner.total_used,
            bytes,
            self.inner.config.total_cap_bytes,
        ) {
            self.inner.class_used[class_idx].fetch_sub(bytes, Ordering::AcqRel);
            self.inner.denied[class_idx].fetch_add(1, Ordering::Relaxed);
            return false;
        }

        true
    }

    pub fn pressure(&self) -> MemoryPressure {
        pressure_for(
            self.inner.total_used.load(Ordering::Relaxed),
            self.inner.config.total_cap_bytes,
            self.inner.config.pressure_constrained_pct,
            self.inner.config.pressure_critical_pct,
        )
    }

    pub fn snapshot(&self) -> ResourceSnapshot {
        ResourceSnapshot {
            total_cap_bytes: self.inner.config.total_cap_bytes,
            total_used_bytes: self.inner.total_used.load(Ordering::Relaxed),
            pressure: self.pressure(),
            classes: std::array::from_fn(|idx| {
                let class = MemoryClass::ALL[idx];
                MemoryClassSnapshot {
                    class,
                    cap_bytes: self.inner.config.class_caps_bytes[idx],
                    used_bytes: self.inner.class_used[idx].load(Ordering::Relaxed),
                    denied_allocations: self.inner.denied[idx].load(Ordering::Relaxed),
                }
            }),
        }
    }

    fn release(&self, class: MemoryClass, bytes: u64) {
        if bytes == 0 {
            return;
        }
        self.inner.class_used[class.index()].fetch_sub(bytes, Ordering::AcqRel);
        self.inner.total_used.fetch_sub(bytes, Ordering::AcqRel);
    }
}

fn reserve(counter: &AtomicU64, bytes: u64, cap: u64) -> bool {
    loop {
        let current = counter.load(Ordering::Acquire);
        let Some(next) = current.checked_add(bytes) else {
            return false;
        };
        if next > cap {
            return false;
        }
        if counter
            .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            return true;
        }
    }
}

fn pressure_for(used: u64, cap: u64, constrained_pct: u8, critical_pct: u8) -> MemoryPressure {
    if cap == 0 {
        return MemoryPressure::Critical;
    }
    let used_pct = used.saturating_mul(100) / cap;
    if used_pct >= critical_pct as u64 {
        MemoryPressure::Critical
    } else if used_pct >= constrained_pct as u64 {
        MemoryPressure::Constrained
    } else {
        MemoryPressure::Normal
    }
}

#[derive(Debug)]
pub struct MemoryLease {
    governor: ResourceGovernor,
    class: MemoryClass,
    bytes: u64,
    /// Allocated only while trace logging is enabled. This keeps normal
    /// admission free of a per-lease heap allocation while giving storage
    /// evidence a stable identity and complete consumer lifetime when traced.
    trace: Option<Box<MemoryLeaseTrace>>,
}

#[derive(Debug)]
struct MemoryLeaseTrace {
    id: u64,
    acquired_at: Instant,
}

impl MemoryLease {
    fn new(governor: ResourceGovernor, class: MemoryClass, bytes: u64) -> Self {
        Self::new_with_trace(
            governor,
            class,
            bytes,
            tracing::enabled!(tracing::Level::TRACE),
        )
    }

    fn new_with_trace(
        governor: ResourceGovernor,
        class: MemoryClass,
        bytes: u64,
        trace_enabled: bool,
    ) -> Self {
        let trace = trace_enabled
            .then(allocate_memory_lease_id)
            .flatten()
            .map(|id| {
                let acquired_at = Instant::now();
                let snapshot = governor.snapshot();
                tracing::trace!(
                    resource_lease_id = id,
                    resource_lease_class = class.as_str(),
                    resource_lease_bytes = bytes,
                    resource_lease_total_used_bytes_after_acquire = snapshot.total_used_bytes,
                    resource_lease_class_used_bytes_after_acquire =
                        snapshot.classes[class.index()].used_bytes,
                    "resource lease acquired"
                );
                Box::new(MemoryLeaseTrace { id, acquired_at })
            });
        Self {
            governor,
            class,
            bytes,
            trace,
        }
    }

    pub fn class(&self) -> MemoryClass {
        self.class
    }

    pub fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Trace-only identity for correlating a reservation with operation and
    /// release evidence. Leases created without TRACE logging intentionally
    /// carry no identity and incur no identity allocation.
    pub fn identity(&self) -> Option<u64> {
        self.trace.as_ref().map(|trace| trace.id)
    }

    /// Extend an existing reservation while keeping ownership in one lease.
    /// This is used when a bounded protocol response reveals a second
    /// allocation whose size could not be known at admission time.
    pub fn try_grow(&mut self, additional: u64) -> bool {
        let Some(new_bytes) = self.bytes.checked_add(additional) else {
            return false;
        };
        if additional == 0 {
            return true;
        }
        if !self
            .governor
            .try_reserve_lease_bytes(self.class, additional)
        {
            return false;
        }
        let previous_bytes = self.bytes;
        self.bytes = new_bytes;
        if let Some(trace) = self.trace.as_ref() {
            let snapshot = self.governor.snapshot();
            tracing::trace!(
                resource_lease_id = trace.id,
                resource_lease_class = self.class.as_str(),
                resource_lease_previous_bytes = previous_bytes,
                resource_lease_growth_bytes = additional,
                resource_lease_bytes = new_bytes,
                resource_lease_total_used_bytes_after_growth = snapshot.total_used_bytes,
                resource_lease_class_used_bytes_after_growth =
                    snapshot.classes[self.class.index()].used_bytes,
                "resource lease grown"
            );
        }
        true
    }
}

impl Drop for MemoryLease {
    fn drop(&mut self) {
        self.governor.release(self.class, self.bytes);
        if let Some(trace) = self.trace.as_ref() {
            let snapshot = self.governor.snapshot();
            tracing::trace!(
                resource_lease_id = trace.id,
                resource_lease_class = self.class.as_str(),
                resource_lease_bytes = self.bytes,
                resource_lease_lifetime_micros = trace
                    .acquired_at
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX))
                    as u64,
                resource_lease_total_used_bytes_after_release = snapshot.total_used_bytes,
                resource_lease_class_used_bytes_after_release =
                    snapshot.classes[self.class.index()].used_bytes,
                "resource lease released"
            );
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryClassSnapshot {
    pub class: MemoryClass,
    pub cap_bytes: u64,
    pub used_bytes: u64,
    pub denied_allocations: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResourceSnapshot {
    pub total_cap_bytes: u64,
    pub total_used_bytes: u64,
    pub pressure: MemoryPressure,
    pub classes: [MemoryClassSnapshot; MEMORY_CLASS_COUNT],
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ResourceGovernorConfig {
        ResourceGovernorConfig {
            total_cap_bytes: 100,
            class_caps_bytes: [50, 80, 80, 80, 80, 80, 80, 80, 80, 80],
            pressure_constrained_pct: 60,
            pressure_critical_pct: 90,
        }
    }

    #[test]
    fn leases_release_on_drop() {
        let governor = ResourceGovernor::new(config());
        let lease = governor.try_acquire(MemoryClass::StorageFrame, 40).unwrap();
        assert_eq!(lease.bytes(), 40);
        assert_eq!(governor.snapshot().total_used_bytes, 40);
        drop(lease);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn lease_growth_is_bounded_and_releases_as_one_reservation() {
        let governor = ResourceGovernor::new(config());
        let mut lease = governor.try_acquire(MemoryClass::Metadata, 40).unwrap();

        assert!(lease.try_grow(30));
        assert_eq!(lease.bytes(), 70);
        assert_eq!(governor.snapshot().total_used_bytes, 70);
        assert!(!lease.try_grow(20));
        assert_eq!(lease.bytes(), 70);
        assert_eq!(governor.snapshot().total_used_bytes, 70);
        drop(lease);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn traced_lease_identity_is_stable_through_growth_and_release() {
        let governor = ResourceGovernor::new(config());
        assert!(governor.try_reserve_lease_bytes(MemoryClass::Metadata, 40));
        let mut lease =
            MemoryLease::new_with_trace(governor.clone(), MemoryClass::Metadata, 40, true);
        let identity = lease.identity().expect("trace lease has an identity");

        assert!(lease.try_grow(30));
        assert_eq!(lease.identity(), Some(identity));
        assert_eq!(lease.bytes(), 70);

        drop(lease);
        assert_eq!(governor.snapshot().total_used_bytes, 0);

        assert!(governor.try_reserve_lease_bytes(MemoryClass::Metadata, 1));
        let next_lease =
            MemoryLease::new_with_trace(governor.clone(), MemoryClass::Metadata, 1, true);
        assert_ne!(next_lease.identity(), Some(identity));
        drop(next_lease);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn untraced_lease_does_not_allocate_an_identity() {
        let governor = ResourceGovernor::new(config());
        assert!(governor.try_reserve_lease_bytes(MemoryClass::Metadata, 1));
        let lease = MemoryLease::new_with_trace(governor.clone(), MemoryClass::Metadata, 1, false);

        assert_eq!(lease.identity(), None);
        drop(lease);
        assert_eq!(governor.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn class_and_global_caps_are_enforced() {
        let governor = ResourceGovernor::new(config());
        let _a = governor.try_acquire(MemoryClass::StorageFrame, 40).unwrap();
        assert!(governor
            .try_acquire(MemoryClass::StorageFrame, 11)
            .is_none());
        let _b = governor.try_acquire(MemoryClass::PeerBuffer, 60).unwrap();
        assert!(governor.try_acquire(MemoryClass::Metadata, 1).is_none());
        let snap = governor.snapshot();
        assert_eq!(
            snap.classes[MemoryClass::StorageFrame.index()].denied_allocations,
            1
        );
        assert_eq!(
            snap.classes[MemoryClass::Metadata.index()].denied_allocations,
            1
        );
    }

    #[test]
    fn pressure_transitions_are_deterministic() {
        let governor = ResourceGovernor::new(config());
        assert_eq!(governor.pressure(), MemoryPressure::Normal);
        let constrained = governor.try_acquire(MemoryClass::PeerBuffer, 60).unwrap();
        assert_eq!(governor.pressure(), MemoryPressure::Constrained);
        drop(constrained);
        let _critical = governor.try_acquire(MemoryClass::PeerBuffer, 80).unwrap();
        let _more = governor.try_acquire(MemoryClass::StorageFrame, 10).unwrap();
        assert_eq!(governor.pressure(), MemoryPressure::Critical);
    }
}
