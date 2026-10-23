use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use futures::future::join_all;
use once_cell::sync::Lazy;
use tokio::sync::{
    mpsc::{self as tokio_mpsc, error::TrySendError},
    oneshot, OwnedSemaphorePermit, RwLock as AsyncRwLock, Semaphore, TryAcquireError,
};
use tracing::instrument;

use rt_hash::{merkle_root, BlockHash, MerkleAccumulator, V2_BLOCK_SIZE};
use rt_metrics::{MemoryClass, MemoryLease, ResourceGovernor};
use rt_path::{StorageProfile, StorageRootId};
use sha1::{Digest, Sha1};

#[cfg(any(unix, windows))]
use crate::open::identity_matches_path;
use crate::{
    backend::{
        read_frame_leased, BackendJobOwnership, BackendKind, BackendRequest, DiskBackend,
        FixedBufferStrategy, SelectedDiskBackend,
    },
    device::{detect_storage_topology, StorageTopology},
    elevator::elevator_class_weight,
    error::StorageError,
    fd_limit::process_handle_cache_capacity,
    file_handle::{LeasedFile, LeasedFileHandle},
    frame::{global_frame_pool, try_global_frame_pool, Frame},
    handle_cache::HandleCache,
    io_class::IoClass,
    open::{create_dir_all_no_follow, normalized_storage_path, FileIdentity},
};

/// Optional urgency carried by a disk operation while it waits for shared
/// device admission. Expired deadlines are promoted ahead of weighted normal
/// traffic; choke-critical obligations are promoted immediately. A future
/// deadline remains ordinary work so it can use batching and readahead paths.
/// Once a backend operation is dispatched, its completion ownership is unchanged.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IoRequestPriority {
    deadline: Option<Instant>,
    choke_critical: bool,
}

impl IoRequestPriority {
    pub const fn normal() -> Self {
        Self {
            deadline: None,
            choke_critical: false,
        }
    }

    pub const fn deadline(deadline: Instant) -> Self {
        Self {
            deadline: Some(deadline),
            choke_critical: false,
        }
    }

    pub const fn choke_critical(deadline: Instant) -> Self {
        Self {
            deadline: Some(deadline),
            choke_critical: true,
        }
    }

    pub fn is_urgent(self) -> bool {
        self.choke_critical
            || self
                .deadline
                .is_some_and(|deadline| deadline <= Instant::now())
    }
}

#[cfg(test)]
use crate::open::file_identity;

pub const STORAGE_LATENCY_BUCKETS_NS: [u64; 8] = [
    100_000,
    500_000,
    1_000_000,
    5_000_000,
    10_000_000,
    50_000_000,
    100_000_000,
    u64::MAX,
];
pub const STORAGE_LATENCY_BUCKET_COUNT: usize = STORAGE_LATENCY_BUCKETS_NS.len();

const QUEUED_DISK_JOB_OVERHEAD_BYTES: u64 = 1024;
const MAX_HASH_QUEUE_RETRIES: usize = 10_000;

fn checked_file_range_end(path: &Path, offset: u64, len: u64) -> Result<u64, StorageError> {
    offset.checked_add(len).ok_or_else(|| {
        StorageError::io(
            path.display().to_string(),
            io::Error::new(io::ErrorKind::InvalidInput, "file range overflows u64"),
        )
    })
}

fn lock_recover<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // Preserve authoritative state such as dirty-write generations,
            // shared device semaphores, and queued I/O receiver state.
            let guard = poisoned.into_inner();
            mutex.clear_poison();
            guard
        }
    }
}

fn lock_reset_derived_cache<T: Default>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    match mutex.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // Only use this for reconstructible caches whose backing state is
            // authoritative elsewhere.
            let mut guard = poisoned.into_inner();
            *guard = T::default();
            mutex.clear_poison();
            guard
        }
    }
}

/// A pending read or write operation against a storage root.
#[derive(Debug)]
pub struct IoRequest {
    pub class: IoClass,
    pub storage_root: StorageRootId,
    pub file_path: PathBuf,
    pub offset: u64,
    pub len: usize,
}

/// Hash results for one immutable hybrid piece payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HybridPieceHashes {
    pub v1_sha1: [u8; 20],
    /// Root over the content's actual 16 KiB leaves, suitable for a file that
    /// fits in one v2 piece.
    pub v2_file_root: [u8; 32],
    /// Root padded to the full piece's leaf count, suitable for a v2 piece
    /// layer entry, including a short final file piece.
    pub v2_piece_root: [u8; 32],
}

/// Opaque evidence that a scheduler-wide payload durability barrier completed.
///
/// The evidence can only be created by `MountScheduler` after all writes
/// admitted before the barrier have completed and every dirty or active path
/// owned by that scheduler has been synced. It retains write admission until
/// dropped, so callers can keep later payload writes behind the resume commit
/// that consumes this evidence.
#[must_use = "durability evidence should be attached to the state it protects"]
pub struct DurabilityBarrierEvidence {
    synced_file_count: u64,
    write_sequence_watermark: u64,
    _write_admission_guard: tokio::sync::OwnedRwLockWriteGuard<()>,
}

impl std::fmt::Debug for DurabilityBarrierEvidence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurabilityBarrierEvidence")
            .field("synced_file_count", &self.synced_file_count)
            .field("write_sequence_watermark", &self.write_sequence_watermark)
            .finish_non_exhaustive()
    }
}

impl DurabilityBarrierEvidence {
    /// Number of distinct files synced by this barrier.
    pub fn synced_file_count(&self) -> u64 {
        self.synced_file_count
    }

    /// Highest scheduler-wide payload write sequence admitted before the
    /// barrier. Sequences are diagnostic watermarks and saturate at `u64::MAX`.
    pub fn write_sequence_watermark(&self) -> u64 {
        self.write_sequence_watermark
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreallocationMode {
    Off,
    Auto,
    Sparse,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DurabilityMode {
    Fast,
    Checkpoint,
    Strict,
}

#[derive(Debug, Clone)]
pub struct StorageIoConfig {
    pub file_pool_size: usize,
    pub idle_file_ttl_secs: u64,
    pub io_worker_threads: usize,
    pub io_queue_depth: usize,
    pub hash_worker_threads: usize,
    pub hash_queue_depth: usize,
    pub preallocation_mode: PreallocationMode,
    pub durability_mode: DurabilityMode,
    pub peer_read_readahead_bytes: usize,
    /// Expand readahead only for reads that continue the previous read of the
    /// same file. Off restores unconditional expansion on every miss.
    pub peer_read_readahead_sequential_only: bool,
    /// Maximum cached forward ranges across this scheduler's peer-read cache.
    pub peer_read_cache_entries: usize,
    /// Maximum payload bytes retained by this scheduler's peer-read cache.
    pub peer_read_cache_bytes: usize,
    pub peer_read_elevator_budget_ms: u64,
}

impl Default for StorageIoConfig {
    fn default() -> Self {
        Self {
            file_pool_size: 512,
            idle_file_ttl_secs: 300,
            io_worker_threads: 2,
            io_queue_depth: 256,
            hash_worker_threads: 1,
            hash_queue_depth: 256,
            preallocation_mode: PreallocationMode::Auto,
            durability_mode: DurabilityMode::Checkpoint,
            peer_read_readahead_bytes: 512 * 1024,
            peer_read_readahead_sequential_only: true,
            peer_read_cache_entries: 64,
            peer_read_cache_bytes: 8 * 1024 * 1024,
            peer_read_elevator_budget_ms: 25,
        }
    }
}

/// Per-mount concurrency configuration.
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    pub profile: StorageProfile,
    /// Maximum queue depth across all classes (backpressure point).
    pub max_queue: usize,
    pub recheck_concurrency: usize,
    pub peer_read_concurrency: usize,
    pub storage_io: StorageIoConfig,
    pub resources: Option<ResourceGovernor>,
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        SchedulerConfig {
            profile: StorageProfile::Unknown,
            max_queue: 256,
            recheck_concurrency: 0,
            peer_read_concurrency: 0,
            storage_io: StorageIoConfig::default(),
            resources: None,
        }
    }
}

fn effective_io_config_for_topology(
    io_config: &StorageIoConfig,
    topology: Option<&StorageTopology>,
) -> StorageIoConfig {
    let mut effective = io_config.clone();
    if effective.preallocation_mode == PreallocationMode::Auto {
        effective.preallocation_mode = preallocation_mode_for_topology(topology);
    }
    effective
}

pub fn preallocation_mode_for_topology(topology: Option<&StorageTopology>) -> PreallocationMode {
    match topology {
        Some(StorageTopology {
            profile: StorageProfile::Hdd,
            cow: false,
            ..
        }) => PreallocationMode::Full,
        _ => PreallocationMode::Sparse,
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FilePoolStats {
    pub capacity: usize,
    /// File descriptors retained by cache entries.
    pub open_files: usize,
    /// Descriptor leases held by cached files, open attempts, or active operations.
    pub active_descriptors: usize,
    pub memory_bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub idle_closes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageIoStats {
    /// Identity for deduplicating snapshots of shared scheduler resources.
    /// Zero means the identity is unavailable and must not be deduplicated.
    #[doc(hidden)]
    pub shared_resource_id: u64,
    /// Identity for deduplicating the descriptor catalog independently from
    /// scheduler workers. Zero means unavailable and must not be deduplicated.
    #[doc(hidden)]
    pub file_catalog_id: u64,
    pub device_id: Option<String>,
    pub profile: StorageProfile,
    pub file_pool: FilePoolStats,
    pub io_queue_depth: usize,
    pub hash_queue_depth: usize,
    /// Total device admission slots for the shared per-device service.
    pub device_queue_capacity: usize,
    /// Slots not held by either an active operation or a queued waiter.
    pub device_queue_available: usize,
    pub queued_disk_bytes: u64,
    pub queue_full: u64,
    pub dirty_files: usize,
    pub read_ops_by_class: [u64; 6],
    pub write_ops_by_class: [u64; 6],
    pub bytes_read_by_class: [u64; 6],
    pub bytes_written_by_class: [u64; 6],
    pub backend_read_ops_by_class: [u64; 6],
    pub backend_bytes_read_by_class: [u64; 6],
    pub read_latency_ns_by_class: [u64; 6],
    pub write_latency_ns_by_class: [u64; 6],
    pub read_latency_buckets: [u64; STORAGE_LATENCY_BUCKET_COUNT],
    pub write_latency_buckets: [u64; STORAGE_LATENCY_BUCKET_COUNT],
    pub sync_latency_buckets: [u64; STORAGE_LATENCY_BUCKET_COUNT],
    pub hash_latency_buckets: [u64; STORAGE_LATENCY_BUCKET_COUNT],
    pub sync_latency_ns: u64,
    pub hash_latency_ns: u64,
    pub sync_ops: u64,
    pub hash_ops: u64,
    pub preallocation_failures: u64,
    pub preallocation_fallbacks: u64,
    pub preallocation_shrink_refused: u64,
    /// Number of forward ranges currently retained across all files.
    pub peer_read_cache_entries: usize,
    /// Payload bytes currently retained across all peer-read cache ranges.
    pub peer_read_cache_bytes: u64,
    /// Backing allocation bytes held by peer-read cache owners process-wide,
    /// including returned slices that outlive their cache entry.
    pub peer_read_cache_global_backing_bytes: u64,
    /// Process-wide cache backing allocation budget.
    pub peer_read_cache_global_budget_bytes: u64,
    pub peer_read_cache_hits: u64,
    pub peer_read_cache_misses: u64,
    pub peer_read_cache_evictions: u64,
    pub peer_read_elevator_enabled: bool,
    pub peer_read_elevator_queue_depth: usize,
    pub peer_read_elevator_queued: usize,
    pub peer_read_elevator_queue_full: u64,
    pub peer_read_elevator_batches: u64,
    pub peer_read_elevator_coalesced_requests: u64,
    pub page_cache_advise_sequential: u64,
    pub page_cache_advise_willneed: u64,
    pub page_cache_advise_dontneed: u64,
    pub page_cache_advise_failures: u64,
    pub sparse_data_extents: u64,
    pub sparse_hole_bytes: u64,
    pub sparse_seek_fallbacks: u64,
}

/// Read payload that can keep ownership of a pooled storage frame.
///
/// `read_at` and `scheduled_read` remain `Bytes` compatibility wrappers.
/// New call sites that can process borrowed data should use
/// `read_owned_at` / `scheduled_read_owned` so exact backend reads can avoid
/// copying out of the process-wide frame pool.
#[derive(Debug)]
pub enum StorageRead {
    Frame(Frame),
    Bytes(bytes::Bytes),
}

impl StorageRead {
    pub fn len(&self) -> usize {
        match self {
            StorageRead::Frame(frame) => frame.len(),
            StorageRead::Bytes(bytes) => bytes.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn as_slice(&self) -> &[u8] {
        match self {
            StorageRead::Frame(frame) => frame.as_slice(),
            StorageRead::Bytes(bytes) => bytes.as_ref(),
        }
    }

    pub fn into_bytes(self) -> bytes::Bytes {
        match self {
            StorageRead::Frame(frame) => frame.into_bytes(),
            StorageRead::Bytes(bytes) => bytes,
        }
    }
}

impl Default for StorageIoStats {
    fn default() -> Self {
        Self {
            shared_resource_id: 0,
            file_catalog_id: 0,
            device_id: None,
            profile: StorageProfile::Unknown,
            file_pool: FilePoolStats::default(),
            io_queue_depth: 0,
            hash_queue_depth: 0,
            device_queue_capacity: 0,
            device_queue_available: 0,
            queued_disk_bytes: 0,
            queue_full: 0,
            dirty_files: 0,
            read_ops_by_class: [0; 6],
            write_ops_by_class: [0; 6],
            bytes_read_by_class: [0; 6],
            bytes_written_by_class: [0; 6],
            backend_read_ops_by_class: [0; 6],
            backend_bytes_read_by_class: [0; 6],
            read_latency_ns_by_class: [0; 6],
            write_latency_ns_by_class: [0; 6],
            read_latency_buckets: [0; STORAGE_LATENCY_BUCKET_COUNT],
            write_latency_buckets: [0; STORAGE_LATENCY_BUCKET_COUNT],
            sync_latency_buckets: [0; STORAGE_LATENCY_BUCKET_COUNT],
            hash_latency_buckets: [0; STORAGE_LATENCY_BUCKET_COUNT],
            sync_latency_ns: 0,
            hash_latency_ns: 0,
            sync_ops: 0,
            hash_ops: 0,
            preallocation_failures: 0,
            preallocation_fallbacks: 0,
            preallocation_shrink_refused: 0,
            peer_read_cache_entries: 0,
            peer_read_cache_bytes: 0,
            peer_read_cache_global_backing_bytes: 0,
            peer_read_cache_global_budget_bytes: 0,
            peer_read_cache_hits: 0,
            peer_read_cache_misses: 0,
            peer_read_cache_evictions: 0,
            peer_read_elevator_enabled: false,
            peer_read_elevator_queue_depth: 0,
            peer_read_elevator_queued: 0,
            peer_read_elevator_queue_full: 0,
            peer_read_elevator_batches: 0,
            peer_read_elevator_coalesced_requests: 0,
            page_cache_advise_sequential: 0,
            page_cache_advise_willneed: 0,
            page_cache_advise_dontneed: 0,
            page_cache_advise_failures: 0,
            sparse_data_extents: 0,
            sparse_hole_bytes: 0,
            sparse_seek_fallbacks: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataExtent {
    pub offset: u64,
    pub len: u64,
}

/// One bounded page from a sparse-file extent scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataExtentPage {
    pub extents: Vec<DataExtent>,
    /// Exclusive logical offset through which the scan has completed.
    pub scanned_to: u64,
    /// Whether `scanned_to` reached the requested range end.
    pub complete: bool,
    /// The platform/filesystem did not support sparse seeks; this page treats
    /// the requested range as allocated data.
    pub used_fallback: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OpenMode {
    Read,
    Write,
}

#[derive(Debug)]
struct FilePool {
    catalog: HandleCache,
}

impl FilePool {
    fn new(capacity: usize, idle_ttl: Duration) -> Self {
        let catalog = if cfg!(test) {
            // Keep unit-test eviction fixtures hermetic. Production schedulers
            // and StorageRuntime share the process-wide catalog.
            HandleCache::new(capacity, idle_ttl)
        } else {
            HandleCache::process_global_with_bounds(capacity, idle_ttl)
        };
        Self { catalog }
    }

    fn catalog_id(&self) -> u64 {
        self.catalog.catalog_id()
    }

    fn get_or_open(
        &self,
        path: &Path,
        mode: OpenMode,
        create: bool,
    ) -> Result<Arc<LeasedFileHandle>, StorageError> {
        let key = normalized_key(path);
        self.get_or_open_normalized(&key, mode, create)
    }

    fn get_or_open_normalized(
        &self,
        key: &Path,
        mode: OpenMode,
        create: bool,
    ) -> Result<Arc<LeasedFileHandle>, StorageError> {
        self.get_or_open_normalized_owned(key, mode, create)
            .map(Arc::new)
    }

    fn get_or_open_normalized_owned(
        &self,
        key: &Path,
        mode: OpenMode,
        create: bool,
    ) -> Result<LeasedFileHandle, StorageError> {
        self.catalog
            .get_or_open_normalized_lease(key, mode == OpenMode::Write, create)
            .map_err(|error| StorageError::io(key.display().to_string(), error))
    }

    fn open_for_sync(&self, path: &Path) -> Result<Arc<LeasedFileHandle>, StorageError> {
        let key = normalized_key(path);
        self.catalog
            .open_uncached_leased(&key, true, false)
            .map_err(|error| StorageError::io(key.display().to_string(), error))
    }

    /// Writable cached paths, evicting entries whose file was replaced. Only
    /// tests enumerate the pool now; production syncs are scoped to the
    /// scheduler's own dirty paths.
    #[cfg(test)]
    fn write_paths(&self) -> Vec<PathBuf> {
        self.catalog.cached_write_paths()
    }

    fn stats(&self) -> FilePoolStats {
        let stats = self.catalog.stats();
        FilePoolStats {
            capacity: stats.capacity,
            open_files: stats.open_files,
            active_descriptors: stats.active_descriptors,
            memory_bytes: stats.memory_bytes,
            hits: stats.hits,
            misses: stats.misses,
            evictions: stats.evictions,
            idle_closes: stats.idle_closes,
        }
    }

    #[cfg(test)]
    fn active_descriptors(&self) -> usize {
        self.catalog.active_descriptors()
    }
}

type BlockingJob = Box<dyn FnOnce() + Send + 'static>;

#[derive(Debug)]
struct BlockingJobCancelGuard(Arc<AtomicU8>);

impl Drop for BlockingJobCancelGuard {
    fn drop(&mut self) {
        let _ = self.0.compare_exchange(
            BLOCKING_JOB_QUEUED,
            BLOCKING_JOB_CANCELLED,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }
}

const BLOCKING_JOB_QUEUED: u8 = 0;
const BLOCKING_JOB_STARTED: u8 = 1;
const BLOCKING_JOB_CANCELLED: u8 = 2;

#[derive(Debug)]
struct BlockingPool {
    queue_name: &'static str,
    sender: mpsc::SyncSender<BlockingJob>,
    queued: Arc<AtomicUsize>,
}

impl BlockingPool {
    fn new(name_prefix: &'static str, worker_threads: usize, queue_depth: usize) -> Self {
        let (sender, receiver) = mpsc::sync_channel::<BlockingJob>(queue_depth.max(1));
        let receiver = Arc::new(Mutex::new(receiver));
        let queued = Arc::new(AtomicUsize::new(0));
        let requested_workers = worker_threads.max(1);
        let mut started_workers = 0;
        for index in 0..requested_workers {
            let receiver = receiver.clone();
            let queued = queued.clone();
            let name = format!("{name_prefix}-{index}");
            match std::thread::Builder::new().name(name).spawn(move || loop {
                let job = {
                    let rx = lock_recover(&receiver);
                    rx.recv()
                };
                match job {
                    Ok(job) => {
                        queued.fetch_sub(1, Ordering::Relaxed);
                        job();
                    }
                    Err(_) => break,
                }
            }) {
                Ok(_) => started_workers += 1,
                Err(error) => {
                    tracing::error!(
                        component = "storage",
                        operation = "spawn_blocking_workers",
                        pool = name_prefix,
                        requested_workers,
                        started_workers,
                        result = if started_workers == 0 {
                            "unavailable"
                        } else {
                            "degraded"
                        },
                        error = %error,
                        "storage blocking pool started with fewer workers"
                    );
                    break;
                }
            }
        }
        Self {
            queue_name: name_prefix,
            sender,
            queued,
        }
    }

    async fn run<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, StorageError> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let job_state = Arc::new(AtomicU8::new(BLOCKING_JOB_QUEUED));
        let worker_job_state = job_state.clone();
        let _cancel_guard = BlockingJobCancelGuard(job_state);
        self.queued.fetch_add(1, Ordering::Relaxed);
        if self
            .sender
            .try_send(Box::new(move || {
                if worker_job_state
                    .compare_exchange(
                        BLOCKING_JOB_QUEUED,
                        BLOCKING_JOB_STARTED,
                        Ordering::AcqRel,
                        Ordering::Acquire,
                    )
                    .is_err()
                {
                    return;
                }
                let _ = tx.send(f());
            }))
            .is_err()
        {
            self.queued.fetch_sub(1, Ordering::Relaxed);
            return Err(StorageError::QueueFull {
                mount: self.queue_name.to_string(),
            });
        }
        rx.await.map_err(|_| StorageError::Cancelled)?
    }

    fn queued(&self) -> usize {
        self.queued.load(Ordering::Relaxed)
    }
}

/// The bounded resources shared by path-backed schedulers in one process.
///
/// Scheduling policy remains per mount/torrent, but the expensive workers and
/// descriptor cache do not need to be duplicated for every torrent. Keeping
/// this seam separate from `MountScheduler` also means each scheduler retains
/// its own class permits, dirty generations, read cache, counters, and device
/// queue identity.
#[derive(Debug)]
struct SharedSchedulerResources {
    id: u64,
    file_pool: Arc<FilePool>,
    io_pool: Arc<BlockingPool>,
    disk_backend: Arc<SelectedDiskBackend>,
    hash_pool: Arc<BlockingPool>,
}

/// Selected backend capabilities for a backend used by live mount schedulers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveStorageBackend {
    /// Identity of the shared worker/backend resource group in this process.
    pub scheduler_group_id: u64,
    pub selected: BackendKind,
    pub fixed_buffers_supported: bool,
    pub registered_files_supported: bool,
    pub max_batch_len: usize,
    pub fixed_buffer_len: usize,
    pub fixed_buffer_strategy: FixedBufferStrategy,
}

/// Passive snapshot of backend owners currently held by live schedulers.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActiveStorageBackendSnapshot {
    pub scheduler_groups: usize,
    pub backends: Vec<ActiveStorageBackend>,
}

fn active_storage_backend(backend: &SelectedDiskBackend) -> ActiveStorageBackend {
    ActiveStorageBackend {
        scheduler_group_id: 0,
        selected: backend.kind(),
        fixed_buffers_supported: backend.supports_fixed_buffers(),
        registered_files_supported: backend.supports_registered_files(),
        max_batch_len: backend.max_batch_len(),
        fixed_buffer_len: backend.fixed_buffer_len(),
        fixed_buffer_strategy: backend.fixed_buffer_strategy(),
    }
}

/// Snapshot the backend groups already retained by live `MountScheduler`s.
/// This never initializes a backend; metrics can therefore observe the actual
/// transfer owner without constructing a separate `StorageRuntime`.
pub fn active_storage_backend_snapshot() -> ActiveStorageBackendSnapshot {
    let active: Vec<_> = {
        let resources = lock_recover(&SHARED_SCHEDULER_RESOURCES);
        resources.values().filter_map(Weak::upgrade).collect()
    };
    let mut backends = Vec::with_capacity(active.len());
    for resource in &active {
        let mut backend = active_storage_backend(&resource.disk_backend);
        backend.scheduler_group_id = resource.id;
        backends.push(backend);
    }
    backends.sort_by(|left, right| {
        left.selected
            .as_str()
            .cmp(right.selected.as_str())
            .then_with(|| left.scheduler_group_id.cmp(&right.scheduler_group_id))
    });
    ActiveStorageBackendSnapshot {
        scheduler_groups: active.len(),
        backends,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct SharedSchedulerKey {
    file_pool_size: usize,
    idle_file_ttl_secs: u64,
    io_worker_threads: usize,
    io_queue_depth: usize,
    hash_worker_threads: usize,
    hash_queue_depth: usize,
    backend_request: BackendRequest,
}

static SHARED_SCHEDULER_RESOURCES: Lazy<
    Mutex<HashMap<SharedSchedulerKey, Weak<SharedSchedulerResources>>>,
> = Lazy::new(|| Mutex::new(HashMap::new()));
static NEXT_SHARED_SCHEDULER_RESOURCE_ID: AtomicU64 = AtomicU64::new(1);

fn next_shared_scheduler_resource_id() -> u64 {
    loop {
        let id = NEXT_SHARED_SCHEDULER_RESOURCE_ID.load(Ordering::Relaxed);
        let next = id
            .checked_add(1)
            .expect("shared scheduler resource identity exhausted");
        if NEXT_SHARED_SCHEDULER_RESOURCE_ID
            .compare_exchange_weak(id, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return id;
        }
    }
}

fn backend_request_from_env() -> BackendRequest {
    std::env::var("TNG_STORAGE_BACKEND")
        .ok()
        .map(|value| BackendRequest::parse(&value))
        .unwrap_or(BackendRequest::Auto)
}

fn shared_scheduler_key(io_config: &StorageIoConfig) -> SharedSchedulerKey {
    SharedSchedulerKey {
        file_pool_size: effective_file_pool_size(
            io_config.file_pool_size,
            process_handle_cache_capacity(),
        ),
        idle_file_ttl_secs: io_config.idle_file_ttl_secs,
        io_worker_threads: io_config.io_worker_threads.max(1),
        io_queue_depth: io_config.io_queue_depth.max(1),
        hash_worker_threads: io_config.hash_worker_threads.max(1),
        hash_queue_depth: io_config.hash_queue_depth.max(1),
        backend_request: backend_request_from_env(),
    }
}

fn create_scheduler_resources(key: SharedSchedulerKey) -> Arc<SharedSchedulerResources> {
    Arc::new(SharedSchedulerResources {
        id: next_shared_scheduler_resource_id(),
        file_pool: Arc::new(FilePool::new(
            key.file_pool_size,
            Duration::from_secs(key.idle_file_ttl_secs),
        )),
        io_pool: Arc::new(BlockingPool::new(
            "rt-storage-io",
            key.io_worker_threads,
            key.io_queue_depth,
        )),
        disk_backend: Arc::new(SelectedDiskBackend::select_with_queue_depth(
            key.backend_request,
            key.io_worker_threads,
            key.io_queue_depth,
        )),
        hash_pool: Arc::new(BlockingPool::new(
            "rt-storage-hash",
            key.hash_worker_threads,
            key.hash_queue_depth,
        )),
    })
}

fn shared_scheduler_resources(io_config: &StorageIoConfig) -> Arc<SharedSchedulerResources> {
    let key = shared_scheduler_key(io_config);
    let mut resources = lock_recover(&SHARED_SCHEDULER_RESOURCES);
    if let Some(existing) = resources.get(&key).and_then(Weak::upgrade) {
        return existing;
    }
    resources.retain(|_, resource| resource.strong_count() > 0);

    // Construct while holding the registry lock so concurrent torrent
    // promotions cannot briefly create duplicate worker pools for one key.
    let shared = create_scheduler_resources(key);
    resources.insert(key, Arc::downgrade(&shared));
    shared
}

fn local_scheduler_resources(io_config: &StorageIoConfig) -> Arc<SharedSchedulerResources> {
    create_scheduler_resources(shared_scheduler_key(io_config))
}

/// Bounded record of sequential peer-read streams and their useful readahead
/// history.
#[derive(Debug, Default)]
struct PeerReadStreams {
    files: HashMap<PathBuf, PeerReadStream>,
    current: Option<(PathBuf, PeerReadStream)>,
    sequence: u64,
}

#[derive(Debug, Clone, Copy)]
struct PeerReadStream {
    next_offset: u64,
    sequential_reads: u8,
    recommended_bytes: usize,
    last_used: u64,
}

impl PeerReadStreams {
    const MAX_TRACKED_FILES: usize = 1024;

    /// Record a read and return whether it continued plus its current
    /// reuse-driven readahead recommendation.
    fn observe(
        &mut self,
        key: &Path,
        offset: u64,
        len: usize,
        max_readahead_bytes: usize,
        cache_hit: bool,
    ) -> (bool, usize) {
        self.sequence = self.sequence.saturating_add(1).max(1);
        let last_used = self.sequence;
        let end = offset.saturating_add(len as u64);
        if let Some((current_key, stream)) = self.current.as_mut() {
            if current_key.as_path() == key {
                return observe_peer_read_stream(
                    stream,
                    offset,
                    end,
                    len,
                    max_readahead_bytes,
                    cache_hit,
                    last_used,
                );
            }
        }

        // Promote this path into the one-entry hot slot. The previous hot
        // stream becomes an ordinary LRU entry only when another path is
        // observed, avoiding a hash lookup and PathBuf clone on the common
        // same-file sequential read.
        if let Some((current_key, current_stream)) = self.current.take() {
            self.files.insert(current_key, current_stream);
        }
        let (current_key, mut stream, is_new) = match self.files.remove_entry(key) {
            Some((path, stream)) => (path, stream, false),
            None => {
                if self.files.len() >= Self::MAX_TRACKED_FILES {
                    self.evict_oldest();
                }
                (
                    key.to_path_buf(),
                    PeerReadStream {
                        next_offset: end,
                        sequential_reads: 0,
                        recommended_bytes: len,
                        last_used,
                    },
                    true,
                )
            }
        };
        let result = if is_new {
            // A newly observed path already contains the state for this read.
            (false, stream.recommended_bytes)
        } else {
            observe_peer_read_stream(
                &mut stream,
                offset,
                end,
                len,
                max_readahead_bytes,
                cache_hit,
                last_used,
            )
        };
        self.current = Some((current_key, stream));
        result
    }

    fn evict_oldest(&mut self) {
        if let Some(oldest) = self
            .files
            .iter()
            .min_by_key(|(_, stream)| stream.last_used)
            .map(|(path, _)| path.clone())
        {
            self.files.remove(&oldest);
        }
    }

    #[cfg(test)]
    fn tracked_file_count(&self) -> usize {
        self.files.len() + usize::from(self.current.is_some())
    }
}

fn observe_peer_read_stream(
    stream: &mut PeerReadStream,
    offset: u64,
    end: u64,
    len: usize,
    max_readahead_bytes: usize,
    cache_hit: bool,
    last_used: u64,
) -> (bool, usize) {
    let sequential = stream.next_offset == offset;
    if sequential {
        let next = if stream.sequential_reads == 0 {
            len.saturating_mul(2)
        } else if cache_hit {
            stream
                .recommended_bytes
                .saturating_add((stream.recommended_bytes / 4).max(len))
        } else {
            stream.recommended_bytes
        };
        stream.sequential_reads = stream.sequential_reads.saturating_add(1);
        stream.recommended_bytes = next.min(max_readahead_bytes.max(len));
    } else {
        stream.sequential_reads = 0;
        stream.recommended_bytes = len;
    }
    stream.next_offset = end;
    stream.last_used = last_used;
    (sequential, stream.recommended_bytes)
}

fn next_saturating_sequence(sequence: &AtomicU64) -> u64 {
    let mut current = sequence.load(Ordering::Acquire);
    loop {
        let next = current.saturating_add(1);
        match sequence.compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => return next,
            Err(observed) => current = observed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContentEpoch(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DeviceOperationId(u64);

/// Process-wide identity for one admitted storage operation. Unlike the
/// device sequence, this identity remains unique when work crosses mounts or
/// is dispatched through a different physical-device queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StorageOperationId(u64);

static NEXT_STORAGE_OPERATION_ID: AtomicU64 = AtomicU64::new(1);

fn allocate_storage_operation_id() -> Option<StorageOperationId> {
    let mut current = NEXT_STORAGE_OPERATION_ID.load(Ordering::Acquire);
    loop {
        let next = current.checked_add(1)?;
        match NEXT_STORAGE_OPERATION_ID.compare_exchange_weak(
            current,
            next,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return Some(StorageOperationId(current)),
            Err(observed) => current = observed,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StorageByteRange {
    offset: u64,
    length: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DirtyPathState {
    content_epoch: ContentEpoch,
    identity: FileIdentity,
    write_sequence: u64,
}

fn mark_dirty_path(
    dirty_paths: &Mutex<HashMap<PathBuf, DirtyPathState>>,
    dirty_path_epoch: &AtomicU64,
    path: PathBuf,
    identity: FileIdentity,
    write_sequence: u64,
) -> DirtyPathState {
    let mut dirty = lock_recover(dirty_paths);
    let state = DirtyPathState {
        content_epoch: ContentEpoch(dirty_path_epoch.fetch_add(1, Ordering::Relaxed)),
        identity,
        write_sequence,
    };
    dirty.insert(path, state);
    state
}

fn clear_dirty_path_if_operation(
    dirty_paths: &Mutex<HashMap<PathBuf, DirtyPathState>>,
    path: &Path,
    operation: &StorageOperationRecord,
) {
    if operation.storage_operation_id.is_none()
        || operation.device_operation_id.is_none()
        || operation.device_queue_key.is_none()
        || operation.content_epoch.is_none()
        || operation.file_identity.is_none()
        || operation.write_sequence.is_none()
    {
        return;
    }
    let mut dirty = lock_recover(dirty_paths);
    if dirty
        .get(path)
        .is_some_and(|state| operation.covers_dirty_path(*state))
    {
        dirty.remove(path);
    }
}

fn current_file_identity(
    path: &Path,
    file: &LeasedFileHandle,
    operation: &'static str,
) -> Result<FileIdentity, StorageError> {
    // The identity is captured by the secure open and travels with the
    // descriptor lease through queued and submitted work. Re-reading the fd
    // identity here would add an fstat to every boundary without strengthening
    // the binding between this descriptor and the path generation.
    verify_path_identity(path, file.identity(), operation)
}

fn verify_path_identity(
    path: &Path,
    identity: FileIdentity,
    operation: &'static str,
) -> Result<FileIdentity, StorageError> {
    #[cfg(any(unix, windows))]
    match identity_matches_path(path, identity) {
        Ok(true) => {}
        Ok(false) => {
            return Err(StorageError::FilesystemStateUncertain {
                step: operation,
                reason: format!(
                    "file path {} was replaced during storage I/O",
                    path.display()
                ),
            });
        }
        Err(error) => {
            return Err(StorageError::FilesystemStateUncertain {
                step: operation,
                reason: format!("cannot verify file path {}: {error}", path.display()),
            });
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = (path, operation);
    Ok(identity)
}

fn ensure_dirty_identity(
    path: &Path,
    operation: &'static str,
    dirty: Option<DirtyPathState>,
    actual: FileIdentity,
) -> Result<(), StorageError> {
    if let Some(dirty) = dirty {
        if dirty.identity != actual {
            return Err(StorageError::FilesystemStateUncertain {
                step: operation,
                reason: format!(
                    "dirty writes for {} belong to a replaced file identity",
                    path.display()
                ),
            });
        }
    }
    Ok(())
}

/// Per-path write admission and durability fence. Writers share the gate, so
/// block writes remain concurrent; a sync takes it exclusively, waits for all
/// admitted writers, records their sequence watermark, and blocks later
/// writes until fdatasync completes.
#[derive(Debug, Default)]
struct FileSyncFence {
    gate: Arc<AsyncRwLock<()>>,
    issued_sequence: AtomicU64,
    synced_sequence: AtomicU64,
}

enum FileFenceGuard {
    Shared {
        _guard: tokio::sync::OwnedRwLockReadGuard<()>,
    },
    Exclusive {
        _guard: tokio::sync::OwnedRwLockWriteGuard<()>,
    },
}

impl FileSyncFence {
    fn issue_write_sequence(&self) -> u64 {
        next_saturating_sequence(&self.issued_sequence)
    }

    fn watermark(&self) -> u64 {
        self.issued_sequence.load(Ordering::Acquire)
    }

    fn mark_synced(&self, watermark: u64) {
        let _ = self.synced_sequence.fetch_max(watermark, Ordering::Release);
    }
}

/// Weakly retains path fences so a torrent with a large file set does not
/// keep one lock allocation per path after all I/O and barriers have ended.
#[derive(Debug, Default)]
struct FileSyncFenceRegistry {
    paths: Mutex<HashMap<PathBuf, Weak<FileSyncFence>>>,
}

impl FileSyncFenceRegistry {
    fn for_path(&self, path: &Path) -> Arc<FileSyncFence> {
        let mut paths = lock_recover(&self.paths);
        if paths.len() >= 256 {
            paths.retain(|_, fence| fence.strong_count() != 0);
        }
        if let Some(fence) = paths.get(path).and_then(Weak::upgrade) {
            return fence;
        }
        let fence = Arc::new(FileSyncFence::default());
        paths.insert(path.to_path_buf(), Arc::downgrade(&fence));
        fence
    }

    fn active_paths(&self) -> Vec<PathBuf> {
        let mut paths = lock_recover(&self.paths);
        paths.retain(|_, fence| fence.strong_count() != 0);
        paths.keys().cloned().collect()
    }
}

/// Per-mount I/O scheduler.
#[derive(Debug, Clone)]
pub struct MountScheduler {
    storage_root: StorageRootId,
    recheck_sem: Arc<Semaphore>,
    move_copy_sem: Arc<Semaphore>,
    peer_write_sem: Arc<Semaphore>,
    peer_read_sem: Arc<Semaphore>,
    foreground_sem: Arc<Semaphore>,
    metadata_sem: Arc<Semaphore>,
    queue_sem: Arc<Semaphore>,
    device_queue: Arc<DeviceQueue>,
    device_queue_capacity: usize,
    io_config: StorageIoConfig,
    _shared_resources: Arc<SharedSchedulerResources>,
    file_pool: Arc<FilePool>,
    io_pool: Arc<BlockingPool>,
    disk_backend: Arc<SelectedDiskBackend>,
    hash_pool: Arc<BlockingPool>,
    /// Dirty paths carry both the write generation and descriptor identity.
    /// Per-path fences order admitted writes before fdatasync and keep later
    /// writes behind the barrier; a sync may clear only the content epoch and
    /// file identity it actually covered.
    dirty_paths: Arc<Mutex<HashMap<PathBuf, DirtyPathState>>>,
    dirty_path_epoch: Arc<AtomicU64>,
    file_sync_fences: Arc<FileSyncFenceRegistry>,
    /// Shared write admission lets a full checkpoint take a stable snapshot
    /// across paths while ordinary writes remain concurrent with each other.
    write_admission_gate: Arc<AsyncRwLock<()>>,
    write_sequence: Arc<AtomicU64>,
    peer_read_cache: Arc<Mutex<PeerReadCache>>,
    peer_read_cache_manager: Arc<PeerReadCacheManager>,
    peer_read_cache_epoch: Arc<AtomicU64>,
    /// Where each recently read file's next sequential read would start, so
    /// readahead only expands streams that actually continue.
    peer_read_streams: Arc<Mutex<PeerReadStreams>>,
    peer_read_elevator: Arc<Mutex<Option<PeerReadElevator>>>,
    peer_read_limit: usize,
    recheck_limit: usize,
    peer_read_elevator_enabled: bool,
    peer_read_elevator_queue_depth: usize,
    device_id: Option<String>,
    profile: StorageProfile,
    resources: Option<ResourceGovernor>,
    counters: Arc<StorageCounters>,
}

#[derive(Debug)]
struct StorageCounters {
    read_ops_by_class: [AtomicU64; 6],
    write_ops_by_class: [AtomicU64; 6],
    bytes_read_by_class: [AtomicU64; 6],
    bytes_written_by_class: [AtomicU64; 6],
    backend_read_ops_by_class: [AtomicU64; 6],
    backend_bytes_read_by_class: [AtomicU64; 6],
    read_latency_ns_by_class: [AtomicU64; 6],
    write_latency_ns_by_class: [AtomicU64; 6],
    read_latency_buckets: [AtomicU64; STORAGE_LATENCY_BUCKET_COUNT],
    write_latency_buckets: [AtomicU64; STORAGE_LATENCY_BUCKET_COUNT],
    sync_latency_buckets: [AtomicU64; STORAGE_LATENCY_BUCKET_COUNT],
    hash_latency_buckets: [AtomicU64; STORAGE_LATENCY_BUCKET_COUNT],
    queued_disk_bytes: AtomicU64,
    queue_full: AtomicU64,
    sync_latency_ns: AtomicU64,
    hash_latency_ns: AtomicU64,
    sync_ops: AtomicU64,
    hash_ops: AtomicU64,
    preallocation_failures: AtomicU64,
    preallocation_fallbacks: AtomicU64,
    preallocation_shrink_refused: AtomicU64,
    peer_read_cache_hits: AtomicU64,
    peer_read_cache_misses: AtomicU64,
    peer_read_cache_evictions: AtomicU64,
    peer_read_elevator_queue_full: AtomicU64,
    peer_read_elevator_batches: AtomicU64,
    peer_read_elevator_coalesced_requests: AtomicU64,
    page_cache_advise_sequential: AtomicU64,
    page_cache_advise_willneed: AtomicU64,
    page_cache_advise_dontneed: AtomicU64,
    page_cache_advise_failures: AtomicU64,
    sparse_data_extents: AtomicU64,
    sparse_hole_bytes: AtomicU64,
    sparse_seek_fallbacks: AtomicU64,
    #[cfg(test)]
    operation_dispatches: Mutex<Vec<StorageDispatchTestSnapshot>>,
    #[cfg(test)]
    prepared_pread_operations: AtomicU64,
}

impl Default for StorageCounters {
    fn default() -> Self {
        Self {
            read_ops_by_class: std::array::from_fn(|_| AtomicU64::new(0)),
            write_ops_by_class: std::array::from_fn(|_| AtomicU64::new(0)),
            bytes_read_by_class: std::array::from_fn(|_| AtomicU64::new(0)),
            bytes_written_by_class: std::array::from_fn(|_| AtomicU64::new(0)),
            backend_read_ops_by_class: std::array::from_fn(|_| AtomicU64::new(0)),
            backend_bytes_read_by_class: std::array::from_fn(|_| AtomicU64::new(0)),
            read_latency_ns_by_class: std::array::from_fn(|_| AtomicU64::new(0)),
            write_latency_ns_by_class: std::array::from_fn(|_| AtomicU64::new(0)),
            read_latency_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            write_latency_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            sync_latency_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            hash_latency_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            queued_disk_bytes: AtomicU64::new(0),
            queue_full: AtomicU64::new(0),
            sync_latency_ns: AtomicU64::new(0),
            hash_latency_ns: AtomicU64::new(0),
            sync_ops: AtomicU64::new(0),
            hash_ops: AtomicU64::new(0),
            preallocation_failures: AtomicU64::new(0),
            preallocation_fallbacks: AtomicU64::new(0),
            preallocation_shrink_refused: AtomicU64::new(0),
            peer_read_cache_hits: AtomicU64::new(0),
            peer_read_cache_misses: AtomicU64::new(0),
            peer_read_cache_evictions: AtomicU64::new(0),
            peer_read_elevator_queue_full: AtomicU64::new(0),
            peer_read_elevator_batches: AtomicU64::new(0),
            peer_read_elevator_coalesced_requests: AtomicU64::new(0),
            page_cache_advise_sequential: AtomicU64::new(0),
            page_cache_advise_willneed: AtomicU64::new(0),
            page_cache_advise_dontneed: AtomicU64::new(0),
            page_cache_advise_failures: AtomicU64::new(0),
            sparse_data_extents: AtomicU64::new(0),
            sparse_hole_bytes: AtomicU64::new(0),
            sparse_seek_fallbacks: AtomicU64::new(0),
            #[cfg(test)]
            operation_dispatches: Mutex::new(Vec::new()),
            #[cfg(test)]
            prepared_pread_operations: AtomicU64::new(0),
        }
    }
}

#[derive(Debug)]
struct PeerReadCacheEntry {
    identity: FileIdentity,
    source: Weak<LeasedFile>,
    offset: u64,
    data: bytes::Bytes,
    consumed_prefix: usize,
    consumed_suffix: usize,
    consumed_granules: u64,
    last_used: Instant,
}

impl PeerReadCacheEntry {
    /// Remember full granules even when they arrive before either edge. One
    /// word covers a normal read-ahead window without allocating per hit.
    /// Partial, out-of-order granules remain conservatively cached.
    fn consume(&mut self, offset: u64, len: usize) -> bool {
        if len == 0 {
            return false;
        }
        let Some(request_end) = offset.checked_add(len as u64) else {
            return false;
        };
        let relative = offset
            .saturating_sub(self.offset)
            .min(self.data.len() as u64) as usize;
        let end = request_end
            .saturating_sub(self.offset)
            .min(self.data.len() as u64) as usize;
        if relative >= end {
            return false;
        }
        if relative <= self.consumed_prefix {
            self.consumed_prefix = self.consumed_prefix.max(end);
        }
        if end >= self.consumed_suffix {
            self.consumed_suffix = self.consumed_suffix.min(relative);
        }
        let granule = self
            .data
            .len()
            .div_ceil(64)
            .next_power_of_two()
            .max(16 * 1024);
        for index in relative.div_ceil(granule)..end / granule {
            self.consumed_granules |= 1u64 << index;
        }
        while self.consumed_prefix < self.consumed_suffix {
            let index = self.consumed_prefix / granule;
            if index >= 64 || self.consumed_granules & (1u64 << index) == 0 {
                break;
            }
            self.consumed_prefix = (index + 1) * granule;
        }
        while self.consumed_suffix > self.consumed_prefix {
            let index = (self.consumed_suffix - 1) / granule;
            if index >= 64 || self.consumed_granules & (1u64 << index) == 0 {
                break;
            }
            self.consumed_suffix = index * granule;
        }
        self.consumed_prefix >= self.consumed_suffix
    }
}

/// Bounded multi-range cache for peer upload reads. A single hot file may
/// retain several independently useful forward ranges without evicting its
/// previous range on every seek.
#[derive(Debug, Default)]
struct PeerReadCache {
    files: HashMap<PathBuf, Vec<PeerReadCacheEntry>>,
    entries: usize,
    cached_bytes: usize,
    registered_with_manager: bool,
}

impl PeerReadCache {
    fn entry_count(&self) -> usize {
        self.entries
    }

    fn cached_bytes(&self) -> usize {
        self.cached_bytes
    }

    #[cfg(test)]
    fn contains_file(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn remove_file(&mut self, path: &Path) -> usize {
        let Some(entries) = self.files.remove(path) else {
            return 0;
        };
        let removed = entries.len();
        let removed_bytes = entries.iter().map(|entry| entry.data.len()).sum::<usize>();
        self.entries = self.entries.saturating_sub(removed);
        self.cached_bytes = self.cached_bytes.saturating_sub(removed_bytes);
        removed
    }

    fn hit(
        &mut self,
        path: &Path,
        identity: FileIdentity,
        offset: u64,
        len: usize,
    ) -> Option<bytes::Bytes> {
        if self
            .files
            .get(path)
            .is_some_and(|entries| entries.iter().any(|entry| entry.identity != identity))
        {
            self.remove_file(path);
            return None;
        }
        self.candidate(path, offset, len).map(|(_, bytes, _)| bytes)
    }

    /// Take a candidate and its captured identity together. A caller without
    /// an already-validated descriptor MUST validate this identity against
    /// the current path before exposing the bytes. No filesystem I/O runs
    /// while this cache is locked.
    fn candidate(
        &mut self,
        path: &Path,
        offset: u64,
        len: usize,
    ) -> Option<(FileIdentity, bytes::Bytes, Weak<LeasedFile>)> {
        let result = {
            let entries = self.files.get_mut(path)?;
            let (index, relative, end) = entries
                .iter()
                .enumerate()
                .filter_map(|(index, entry)| {
                    let relative = usize::try_from(offset.checked_sub(entry.offset)?).ok()?;
                    let end = relative.checked_add(len)?;
                    (end <= entry.data.len()).then_some((index, relative, end, entry.data.len()))
                })
                .min_by_key(|(_, _, _, range_bytes)| *range_bytes)
                .map(|(index, relative, end, _)| (index, relative, end))?;
            let entry = &mut entries[index];
            entry.last_used = Instant::now();
            (
                entry.identity,
                entry.data.slice(relative..end),
                entry.source.clone(),
            )
        };
        // The same requested bytes also consume overlapping windows, even
        // when the smallest covering window supplied the returned view.
        self.consume(path, result.0, offset, len);
        Some(result)
    }

    fn consume(&mut self, path: &Path, identity: FileIdentity, offset: u64, len: usize) {
        if len == 0 {
            return;
        }
        let Some(entries) = self.files.get_mut(path) else {
            return;
        };
        let mut removed = 0;
        let mut removed_bytes = 0;
        entries.retain_mut(|entry| {
            if entry.identity != identity || entry.consume(offset, len) {
                removed += 1;
                removed_bytes += entry.data.len();
                false
            } else {
                true
            }
        });
        let empty = entries.is_empty();
        self.entries = self.entries.saturating_sub(removed);
        self.cached_bytes = self.cached_bytes.saturating_sub(removed_bytes);
        if empty {
            self.files.remove(path);
        }
    }

    fn remove_entry(&mut self, path: &Path, index: usize) -> bool {
        let remove_path = {
            let Some(entries) = self.files.get_mut(path) else {
                return false;
            };
            if index >= entries.len() {
                return false;
            }
            let removed = entries.remove(index);
            self.entries = self.entries.saturating_sub(1);
            self.cached_bytes = self.cached_bytes.saturating_sub(removed.data.len());
            entries.is_empty()
        };
        if remove_path {
            self.files.remove(path);
        }
        true
    }

    fn evict_oldest(&mut self) -> bool {
        let oldest = self
            .files
            .iter()
            .flat_map(|(path, entries)| {
                entries
                    .iter()
                    .enumerate()
                    .map(move |(index, entry)| (path, index, entry.last_used))
            })
            .min_by_key(|(_, _, last_used)| *last_used)
            .map(|(path, index, _)| (path.clone(), index));
        oldest.is_some_and(|(path, index)| self.remove_entry(&path, index))
    }

    /// Insert one useful range while keeping both entry count and payload
    /// bytes within their configured limits.
    fn insert(
        &mut self,
        path: PathBuf,
        entry: PeerReadCacheEntry,
        max_entries: usize,
        max_bytes: usize,
    ) -> Option<bool> {
        let entry_bytes = entry.data.len();
        if max_entries == 0 || max_bytes == 0 || entry_bytes > max_bytes {
            return None;
        }
        if let Some(index) = self.files.get(&path).and_then(|entries| {
            entries
                .iter()
                .position(|value| value.offset == entry.offset)
        }) {
            self.remove_entry(&path, index);
        }

        let mut evicted = false;
        while self.entries >= max_entries
            || self.cached_bytes.saturating_add(entry_bytes) > max_bytes
        {
            if !self.evict_oldest() {
                return None;
            }
            evicted = true;
        }
        self.files.entry(path).or_default().push(entry);
        self.entries = self.entries.saturating_add(1);
        self.cached_bytes = self.cached_bytes.saturating_add(entry_bytes);
        Some(evicted)
    }
}

const MAX_GLOBAL_PEER_READ_CACHE_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Default)]
struct PeerReadCacheRegistry {
    caches: Vec<PeerReadCacheOwner>,
    cache_ids: HashSet<usize>,
    cursor: usize,
}

#[derive(Debug, Clone)]
struct PeerReadCacheOwner {
    id: usize,
    cache: Weak<Mutex<PeerReadCache>>,
    counters: Weak<StorageCounters>,
    reclaim_scope_id: Option<usize>,
}

/// Process-wide peer read-cache budget. Per-torrent byte and entry limits
/// still apply; this manager caps their combined retained backing memory and
/// evicts one oldest range per cache turn when memory is needed.
#[derive(Debug)]
struct PeerReadCacheManager {
    budget_bytes: u64,
    used_bytes: AtomicU64,
    registry: Mutex<PeerReadCacheRegistry>,
}

impl PeerReadCacheManager {
    fn new(budget_bytes: u64) -> Self {
        Self {
            budget_bytes,
            used_bytes: AtomicU64::new(0),
            registry: Mutex::new(PeerReadCacheRegistry::default()),
        }
    }

    fn reserve(self: &Arc<Self>, bytes: u64) -> Option<PeerReadCacheBudgetLease> {
        if bytes > self.budget_bytes {
            return None;
        }
        loop {
            let current = self.used_bytes.load(Ordering::Acquire);
            let next = current.checked_add(bytes)?;
            if next <= self.budget_bytes {
                if self
                    .used_bytes
                    .compare_exchange_weak(current, next, Ordering::AcqRel, Ordering::Acquire)
                    .is_ok()
                {
                    return Some(PeerReadCacheBudgetLease {
                        manager: Arc::clone(self),
                        bytes,
                    });
                }
                continue;
            }

            let needed = next.saturating_sub(self.budget_bytes);
            let reclaimed = self.reclaim(needed);
            let trimmed = try_global_frame_pool()
                .map(|pool| pool.trim_retained(needed))
                .unwrap_or(0);
            if reclaimed == 0 && trimmed == 0 {
                return None;
            }
        }
    }

    fn register(
        &self,
        cache: &Arc<Mutex<PeerReadCache>>,
        counters: &Arc<StorageCounters>,
        reclaim_scope_id: Option<usize>,
    ) {
        lock_reset_derived_cache(cache).registered_with_manager = true;
        let id = Arc::as_ptr(cache) as usize;
        let mut registry = lock_recover(&self.registry);
        if registry.cache_ids.contains(&id) {
            if registry.caches.iter().any(|owner| {
                owner.id == id
                    && owner
                        .cache
                        .upgrade()
                        .is_some_and(|registered| Arc::ptr_eq(&registered, cache))
            }) {
                return;
            }
            registry.caches.retain(|owner| owner.id != id);
            registry.cache_ids.remove(&id);
        }
        registry.cache_ids.insert(id);
        registry.caches.push(PeerReadCacheOwner {
            id,
            cache: Arc::downgrade(cache),
            counters: Arc::downgrade(counters),
            reclaim_scope_id,
        });
        if registry.cursor >= registry.caches.len() {
            registry.cursor = 0;
        }
    }

    fn reclaim(&self, target_bytes: u64) -> u64 {
        self.reclaim_matching(target_bytes, None)
    }

    fn reclaim_for_scope(&self, target_bytes: u64, reclaim_scope_id: usize) -> u64 {
        self.reclaim_matching(target_bytes, Some(reclaim_scope_id))
    }

    fn reclaim_matching(&self, target_bytes: u64, reclaim_scope_id: Option<usize>) -> u64 {
        if target_bytes == 0 {
            return 0;
        }
        let before = self.used_bytes.load(Ordering::Acquire);
        let mut registry = lock_recover(&self.registry);
        let mut remaining_in_round = registry.caches.len();
        let mut released_in_round = false;
        while before.saturating_sub(self.used_bytes.load(Ordering::Acquire)) < target_bytes
            && !registry.caches.is_empty()
        {
            if remaining_in_round == 0 {
                if !released_in_round {
                    break;
                }
                remaining_in_round = registry.caches.len();
                released_in_round = false;
            }
            let index = registry.cursor % registry.caches.len();
            let owner = registry.caches[index].clone();
            remaining_in_round -= 1;
            if reclaim_scope_id.is_some_and(|id| owner.reclaim_scope_id != Some(id)) {
                registry.cursor = (index + 1) % registry.caches.len();
                continue;
            }
            let Some(cache_owner) = owner.cache.upgrade() else {
                registry.caches.remove(index);
                registry.cache_ids.remove(&owner.id);
                registry.cursor = if registry.caches.is_empty() {
                    0
                } else {
                    index % registry.caches.len()
                };
                continue;
            };
            let mut cache = lock_reset_derived_cache(&cache_owner);
            if cache.entry_count() == 0 {
                cache.registered_with_manager = false;
                drop(cache);
                registry.caches.remove(index);
                registry.cache_ids.remove(&owner.id);
                registry.cursor = if registry.caches.is_empty() {
                    0
                } else {
                    index % registry.caches.len()
                };
                continue;
            }

            let used_before = self.used_bytes.load(Ordering::Acquire);
            let evicted = cache.evict_oldest();
            if evicted {
                if let Some(counters) = owner.counters.upgrade() {
                    counters
                        .peer_read_cache_evictions
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
            let became_empty = cache.entry_count() == 0;
            if became_empty {
                cache.registered_with_manager = false;
            }
            drop(cache);

            if became_empty {
                registry.caches.remove(index);
                registry.cache_ids.remove(&owner.id);
                registry.cursor = if registry.caches.is_empty() {
                    0
                } else {
                    index % registry.caches.len()
                };
            } else {
                registry.cursor = (index + 1) % registry.caches.len();
            }
            if self.used_bytes.load(Ordering::Acquire) < used_before {
                released_in_round = true;
            }
        }
        if registry.caches.is_empty() {
            registry.cursor = 0;
        }
        before.saturating_sub(self.used_bytes.load(Ordering::Acquire))
    }

    fn used_bytes(&self) -> u64 {
        self.used_bytes.load(Ordering::Relaxed)
    }
}

static GLOBAL_PEER_READ_CACHE_MANAGER: Lazy<Arc<PeerReadCacheManager>> =
    Lazy::new(|| Arc::new(PeerReadCacheManager::new(MAX_GLOBAL_PEER_READ_CACHE_BYTES)));

#[derive(Debug)]
struct PeerReadCacheBudgetLease {
    manager: Arc<PeerReadCacheManager>,
    bytes: u64,
}

impl Drop for PeerReadCacheBudgetLease {
    fn drop(&mut self) {
        self.manager
            .used_bytes
            .fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct CachedPeerBytes {
    bytes: bytes::Bytes,
    _resource_lease: Option<MemoryLease>,
    _budget_lease: PeerReadCacheBudgetLease,
}

impl AsRef<[u8]> for CachedPeerBytes {
    fn as_ref(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

#[derive(Debug, Clone)]
struct PeerReadElevator {
    sender: tokio_mpsc::Sender<PeerReadRequest>,
    counters: Arc<StorageCounters>,
    resources: Option<ResourceGovernor>,
}

#[derive(Debug)]
struct PeerReadRequest {
    path: PathBuf,
    offset: u64,
    len: usize,
    _queued_bytes: Option<QueuedDiskBytes>,
    tx: oneshot::Sender<Result<bytes::Bytes, StorageError>>,
}

#[derive(Debug)]
struct PeerReadBatch {
    path: PathBuf,
    offset: u64,
    len: usize,
    requests: Vec<PeerReadRequest>,
}

#[derive(Debug)]
struct QueuedDiskBytes {
    bytes: u64,
    counters: Arc<StorageCounters>,
    _lease: Option<MemoryLease>,
    /// A shared payload lease accounts for bytes already charged to another
    /// owner (for example a completed piece assembly) while keeping that
    /// owner alive through queued or active backend work.
    _shared_payload_lease: Option<Arc<MemoryLease>>,
    shared_payload_view_bytes: Option<u64>,
}

#[derive(Debug)]
struct DiskSubmission {
    operation: StorageOperationRecord,
    _queued_bytes: Arc<QueuedDiskBytes>,
    _queue: OwnedSemaphorePermit,
    _device_operation: Option<DeviceOperationPermit>,
    _additional_device_operations: Option<Box<[DeviceOperationPermit]>>,
    _file_owner: Option<Arc<LeasedFileHandle>>,
    _payload_owner: Option<bytes::Bytes>,
}

impl Drop for DiskSubmission {
    fn drop(&mut self) {
        let credit = self.operation.resource_credit;
        tracing::trace!(
            storage_operation_id = ?self.operation.storage_operation_id.map(|id| id.0),
            device_operation_id = ?self.operation.device_operation_id.map(|id| id.0),
            device_queue_key = ?self.operation.device_queue_key.as_deref(),
            mount_queue_permits_released = 1,
            device_queue_permits_released = self.device_queue_permit_count(),
            descriptor_owner_released = self._file_owner.is_some(),
            write_payload_owner_released = self._payload_owner.is_some(),
            resource_credit_governor_leases = credit.governor_credit_leases,
            resource_credit_governor_bytes = credit.governor_credit_bytes,
            resource_credit_governor_lease_id = credit.governor_credit_lease_id,
            resource_credit_governor_lease_ids_xor = credit.governor_credit_lease_ids_xor,
            resource_credit_governor_lease_ids_sum = credit.governor_credit_lease_ids_sum,
            resource_credit_logical_queued_bytes = credit.logical_queued_bytes,
            resource_credit_shared_payload_class = ?credit.shared_payload_class,
            resource_credit_shared_payload_classes_mixed = credit.shared_payload_classes_mixed,
            resource_credit_shared_payload_leases = credit.shared_payload_lease_count,
            resource_credit_shared_payload_lease_id = credit.shared_payload_lease_id,
            resource_credit_shared_payload_lease_ids_xor =
                credit.shared_payload_lease_ids_xor,
            resource_credit_shared_payload_lease_ids_sum =
                credit.shared_payload_lease_ids_sum,
            resource_credit_shared_payload_lease_bytes = credit.shared_payload_lease_bytes,
            resource_credit_shared_payload_view_bytes = credit.shared_payload_view_bytes,
            resource_credit_frame_pool_charge_bytes_at_dispatch =
                credit.frame_pool_charge_bytes_at_dispatch,
            "storage operation owner released"
        );
    }
}

/// Resource credit attached to a submitted storage operation. Logical queued
/// bytes are retained for existing scheduler accounting; governor-backed
/// bytes and any shared immutable payload lease are reported separately so
/// callers can distinguish a retained view from a second allocation.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct StorageCreditEvidence {
    governor_credit_leases: u64,
    governor_credit_bytes: u64,
    /// Exact ID when one lease is represented. Coalesced operations retain
    /// lossy, order-independent xor/sum diagnostics over ID contributions.
    governor_credit_lease_id: Option<u64>,
    governor_credit_lease_ids_xor: u64,
    governor_credit_lease_ids_sum: u64,
    logical_queued_bytes: u64,
    shared_payload_class: Option<MemoryClass>,
    shared_payload_classes_mixed: bool,
    shared_payload_lease_count: u64,
    shared_payload_lease_id: Option<u64>,
    shared_payload_lease_ids_xor: u64,
    shared_payload_lease_ids_sum: u64,
    shared_payload_lease_bytes: u64,
    shared_payload_view_bytes: u64,
    frame_pool_charge_bytes_at_dispatch: u64,
}

#[cfg(test)]
#[derive(Debug, Clone)]
struct StorageDispatchTestSnapshot {
    storage_operation_id: Option<u64>,
    device_operation_id: Option<u64>,
    device_queue_key: Option<String>,
    operation_kind: &'static str,
    credit: StorageCreditEvidence,
    byte_range: Option<StorageByteRange>,
    has_file_identity: bool,
    file_owner_retained: bool,
    payload_owner_retained: bool,
    device_queue_permits: usize,
}

/// Evidence carried with an admitted operation until its backend completion.
/// The device sequence is local to the retained device queue and is therefore
/// paired with that queue's key; file identity, content epoch, and write
/// sequence bind dirty-path clearing. Payload reads and writes also retain
/// their submitted byte range.
#[derive(Debug, Default)]
struct StorageOperationRecord {
    storage_operation_id: Option<StorageOperationId>,
    device_operation_id: Option<DeviceOperationId>,
    device_queue_key: Option<Arc<str>>,
    file_identity: Option<FileIdentity>,
    content_epoch: Option<ContentEpoch>,
    byte_range: Option<StorageByteRange>,
    write_sequence: Option<u64>,
    resource_credit: StorageCreditEvidence,
}

impl StorageOperationRecord {
    fn bind_device_operation(&mut self, permit: &DeviceOperationPermit) {
        debug_assert!(self.device_operation_id.is_none());
        debug_assert!(self.device_queue_key.is_none());
        self.device_operation_id = Some(permit.operation_id);
        self.device_queue_key = Some(Arc::clone(&permit.queue.key));
    }

    fn bind_file_identity(&mut self, identity: FileIdentity) {
        debug_assert!(self.file_identity.is_none_or(|current| current == identity));
        self.file_identity = Some(identity);
    }

    fn bind_dirty_path(&mut self, state: DirtyPathState) {
        self.bind_file_identity(state.identity);
        debug_assert!(self
            .content_epoch
            .is_none_or(|epoch| epoch == state.content_epoch));
        self.content_epoch = Some(state.content_epoch);
        debug_assert!(self
            .write_sequence
            .is_none_or(|sequence| sequence == state.write_sequence));
        self.write_sequence = Some(state.write_sequence);
    }

    fn bind_byte_range(&mut self, offset: u64, length: u64) {
        debug_assert!(offset.checked_add(length).is_some());
        let range = StorageByteRange { offset, length };
        debug_assert!(self.byte_range.is_none_or(|current| current == range));
        self.byte_range = Some(range);
    }

    fn bind_write_sequence(&mut self, sequence: u64) {
        debug_assert!(self
            .write_sequence
            .is_none_or(|current| current == sequence));
        self.write_sequence = Some(sequence);
    }

    fn covers_dirty_path(&self, state: DirtyPathState) -> bool {
        self.file_identity == Some(state.identity)
            && self.content_epoch == Some(state.content_epoch)
            && self.write_sequence == Some(state.write_sequence)
    }
}

impl DiskSubmission {
    fn start_device_operation(&mut self, permit: DeviceOperationPermit) {
        debug_assert!(self._device_operation.is_none());
        self.operation.bind_device_operation(&permit);
        self._device_operation = Some(permit);
    }

    fn finish_device_operation(&mut self) {
        self._device_operation.take();
    }

    fn retain_additional_device_operations(&mut self, permits: Vec<DeviceOperationPermit>) {
        if !permits.is_empty() {
            self._additional_device_operations = Some(permits.into_boxed_slice());
        }
    }

    fn device_queue_permit_count(&self) -> usize {
        (if self._device_operation.is_some() {
            1usize
        } else {
            0usize
        })
        .saturating_add(
            self._additional_device_operations
                .as_ref()
                .map_or(0, |permits| permits.len()),
        )
    }

    fn trace_dispatch(&self, operation_kind: &'static str, class: IoClass, device_count: usize) {
        let range = self.operation.byte_range;
        let credit = self.operation.resource_credit;
        #[cfg(test)]
        let mut operation_dispatches = self
            ._queued_bytes
            .counters
            .operation_dispatches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        #[cfg(test)]
        if operation_dispatches.len() < 64 {
            operation_dispatches.push(StorageDispatchTestSnapshot {
                storage_operation_id: self.operation.storage_operation_id.map(|id| id.0),
                device_operation_id: self.operation.device_operation_id.map(|id| id.0),
                device_queue_key: self
                    .operation
                    .device_queue_key
                    .as_ref()
                    .map(ToString::to_string),
                operation_kind,
                credit,
                byte_range: range,
                has_file_identity: self.operation.file_identity.is_some(),
                file_owner_retained: self._file_owner.is_some(),
                payload_owner_retained: self._payload_owner.is_some(),
                device_queue_permits: self.device_queue_permit_count(),
            });
        }
        #[cfg(test)]
        drop(operation_dispatches);
        tracing::trace!(
            storage_operation_id = ?self.operation.storage_operation_id.map(|id| id.0),
            device_operation_id = ?self.operation.device_operation_id.map(|id| id.0),
            device_queue_key = ?self.operation.device_queue_key.as_deref(),
            byte_offset = ?range.map(|range| range.offset),
            byte_length = ?range.map(|range| range.length),
            file_identity = ?self.operation.file_identity,
            content_epoch = ?self.operation.content_epoch,
            write_sequence = ?self.operation.write_sequence,
            resource_credit_governor_bytes = credit.governor_credit_bytes,
            resource_credit_governor_leases = credit.governor_credit_leases,
            resource_credit_governor_lease_id = credit.governor_credit_lease_id,
            resource_credit_governor_lease_ids_xor = credit.governor_credit_lease_ids_xor,
            resource_credit_governor_lease_ids_sum = credit.governor_credit_lease_ids_sum,
            resource_credit_logical_queued_bytes = credit.logical_queued_bytes,
            resource_credit_shared_payload_class = ?credit.shared_payload_class,
            resource_credit_shared_payload_classes_mixed = credit.shared_payload_classes_mixed,
            resource_credit_shared_payload_leases = credit.shared_payload_lease_count,
            resource_credit_shared_payload_lease_id = credit.shared_payload_lease_id,
            resource_credit_shared_payload_lease_ids_xor =
                credit.shared_payload_lease_ids_xor,
            resource_credit_shared_payload_lease_ids_sum =
                credit.shared_payload_lease_ids_sum,
            resource_credit_shared_payload_lease_bytes = credit.shared_payload_lease_bytes,
            resource_credit_shared_payload_view_bytes = credit.shared_payload_view_bytes,
            resource_credit_frame_pool_charge_bytes_at_dispatch =
                credit.frame_pool_charge_bytes_at_dispatch,
            descriptor_owner_retained = self._file_owner.is_some(),
            write_payload_owner_retained = self._payload_owner.is_some(),
            operation_kind,
            class = ?class,
            mount_queue_permits = 1,
            device_count,
            "storage operation dispatched"
        );
    }

    fn bind_file_identity(&mut self, identity: FileIdentity) {
        self.operation.bind_file_identity(identity);
    }

    fn retain_file(&mut self, file: Arc<LeasedFileHandle>) {
        self.bind_file_identity(file.identity());
        self._file_owner = Some(file);
    }

    fn retain_payload(&mut self, payload: bytes::Bytes) {
        self._payload_owner = Some(payload);
    }

    fn release_payload(&mut self) {
        self._payload_owner = None;
    }

    /// Reuse only this barrier's mount admission and byte charge for the next
    /// sequential path. Per-path identity, descriptor ownership, and device
    /// admission must start fresh so a checkpoint never attributes two files
    /// to one storage operation or pins an evicted descriptor across paths.
    fn reset_for_next_checkpoint_path(&mut self) -> Result<(), StorageError> {
        debug_assert!(self._device_operation.is_none());
        debug_assert!(self._additional_device_operations.is_none());
        self._file_owner = None;
        self._payload_owner = None;
        let storage_operation_id = allocate_storage_operation_id().ok_or_else(|| {
            StorageError::io(
                "storage operation identity",
                io::Error::other("process-wide storage operation identity space exhausted"),
            )
        })?;
        self.operation = StorageOperationRecord {
            storage_operation_id: Some(storage_operation_id),
            resource_credit: self._queued_bytes.resource_credit(),
            ..StorageOperationRecord::default()
        };
        Ok(())
    }

    fn bind_dirty_path(&mut self, state: DirtyPathState) {
        self.operation.bind_dirty_path(state);
    }

    fn record_frame_pool_charge(&mut self, frame: &Frame) {
        self.operation
            .resource_credit
            .include_frame_pool_charge(frame.charged_bytes());
    }
}

#[derive(Debug)]
enum ReadPreparation {
    CacheHit {
        key: PathBuf,
        identity: FileIdentity,
        bytes: bytes::Bytes,
    },
    BackendRead {
        key: PathBuf,
        file: Arc<LeasedFileHandle>,
        read_len: usize,
    },
}

struct PreparedPread {
    device_queue: Arc<DeviceQueue>,
    file_pool: Arc<FilePool>,
    peer_read_cache: Arc<Mutex<PeerReadCache>>,
    peer_read_cache_manager: Arc<PeerReadCacheManager>,
    peer_read_cache_epoch: Arc<AtomicU64>,
    peer_read_streams: Arc<Mutex<PeerReadStreams>>,
    counters: Arc<StorageCounters>,
    resources: Option<ResourceGovernor>,
    storage_root: StorageRootId,
    cache_generation: Option<u64>,
    readahead_bytes: usize,
    sequential_only: bool,
    readahead_cache_entries: usize,
    readahead_cache_bytes: usize,
    submission: Option<DiskSubmission>,
}

enum PreadPreparation {
    Complete(StorageRead),
    /// Only a contended cache miss allocates this continuation. Never block
    /// a disk worker waiting for a device slot held by another worker job.
    AwaitDevice(Box<DeferredPread>),
}

struct DeferredPread {
    prepared: PreparedPread,
    class: IoClass,
    key: PathBuf,
    offset: u64,
    len: usize,
    started: Instant,
}

impl DeferredPread {
    fn execute(self) -> Result<StorageRead, StorageError> {
        self.prepared
            .execute_miss(self.class, self.key, self.offset, self.len, self.started)
    }
}

impl PreparedPread {
    fn execute(
        mut self,
        class: IoClass,
        path: PathBuf,
        offset: u64,
        len: usize,
        started: Instant,
    ) -> Result<PreadPreparation, StorageError> {
        #[cfg(test)]
        self.counters
            .prepared_pread_operations
            .fetch_add(1, Ordering::Relaxed);

        let key = if path.is_absolute() {
            path
        } else {
            normalized_key(&path)
        };
        if class == IoClass::PeerRead && self.readahead_bytes > len {
            let candidate =
                lock_reset_derived_cache(&self.peer_read_cache).candidate(&key, offset, len);
            // Upgrade only the original descriptor: an inode number can be
            // reused after its last descriptor closes. A weak source neither
            // pins an idle descriptor nor consumes extra descriptor quota.
            // No catalog lookup, open, or LRU update is needed on a hit.
            let candidate = candidate.and_then(|(identity, bytes, source)| {
                let source = LeasedFileHandle::new(source.upgrade()?);
                (source.identity() == identity).then_some((source, bytes))
            });
            if let Some((source, bytes)) = candidate.filter(|(source, _)| {
                identity_matches_path(&key, source.identity()).unwrap_or(false)
            }) {
                lock_recover(&self.peer_read_streams).observe(
                    &key,
                    offset,
                    len,
                    self.readahead_bytes,
                    true,
                );
                self.counters
                    .peer_read_cache_hits
                    .fetch_add(1, Ordering::Relaxed);
                self.counters.read_ops_by_class[class_index(class)].fetch_add(1, Ordering::Relaxed);
                self.counters.bytes_read_by_class[class_index(class)]
                    .fetch_add(len as u64, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                self.counters.read_latency_ns_by_class[class_index(class)]
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&self.counters.read_latency_buckets, latency_ns);
                let mut submission = self.submission.take().ok_or_else(|| {
                    StorageError::io(
                        "prepared pread",
                        io::Error::other("storage admission owner was already released"),
                    )
                })?;
                submission.bind_file_identity(source.identity());
                submission.operation.bind_byte_range(offset, len as u64);
                drop(submission);
                return Ok(PreadPreparation::Complete(StorageRead::Bytes(bytes)));
            }
            self.counters
                .peer_read_cache_misses
                .fetch_add(1, Ordering::Relaxed);
        }

        // A cache lookup uses worker/mount capacity, but does not occupy a
        // physical device operation. An uncontended miss still completes in
        // this same worker turn; only contention returns to async admission.
        if self
            .submission
            .as_ref()
            .is_some_and(|owner| owner._device_operation.is_some())
        {
            return self
                .execute_miss(class, key, offset, len, started)
                .map(PreadPreparation::Complete);
        }
        if let Some(permit) = self.device_queue.try_acquire_uncontended() {
            self.submission
                .as_mut()
                .expect("prepared pread owns its mount admission")
                .start_device_operation(permit);
            self.execute_miss(class, key, offset, len, started)
                .map(PreadPreparation::Complete)
        } else {
            // No descriptor has been opened or retained before admission.
            // A queued operation holding the device slot may itself need
            // descriptor credit; avoid that resource-ordering cycle.
            Ok(PreadPreparation::AwaitDevice(Box::new(DeferredPread {
                prepared: self,
                class,
                key,
                offset,
                len,
                started,
            })))
        }
    }

    fn execute_miss(
        mut self,
        class: IoClass,
        key: PathBuf,
        offset: u64,
        len: usize,
        started: Instant,
    ) -> Result<StorageRead, StorageError> {
        let file = self
            .file_pool
            .get_or_open_normalized(&key, OpenMode::Read, false)?;
        let read_len =
            if class == IoClass::PeerRead && self.readahead_bytes > len {
                let file_len = file
                    .metadata()
                    .map_err(|error| StorageError::io(key.display().to_string(), error))?
                    .len();
                let (sequential, recommended_bytes) = lock_recover(&self.peer_read_streams)
                    .observe(&key, offset, len, self.readahead_bytes, false);
                if offset >= file_len || (self.sequential_only && !sequential) {
                    len
                } else {
                    (if self.sequential_only {
                        recommended_bytes
                    } else {
                        self.readahead_bytes
                    })
                    .min(file_len.saturating_sub(offset) as usize)
                    .max(len)
                }
            } else {
                len
            };
        advise_for_read_class(file.as_ref(), class, offset, read_len, &self.counters);

        let mut submission = self.submission.take().ok_or_else(|| {
            StorageError::io(
                "prepared pread",
                io::Error::other("storage admission owner was already released"),
            )
        })?;
        submission.retain_file(Arc::clone(&file));
        submission
            .operation
            .bind_byte_range(offset, read_len as u64);
        let requested_charge = queued_disk_charge(len as u64);
        let read_buffer_charge = queued_disk_charge(read_len as u64);
        let read_buffer_queued_bytes = read_buffer_charge
            .checked_sub(requested_charge)
            .filter(|bytes| *bytes > 0)
            .map(|bytes| {
                QueuedDiskBytes::reserve_charged(
                    self.counters.clone(),
                    self.resources.as_ref(),
                    bytes,
                    format_args!("storage-root-{}-readahead", self.storage_root.0),
                )
            })
            .transpose()?;
        if let Some(extra_charge) = read_buffer_queued_bytes.as_ref() {
            submission
                .operation
                .resource_credit
                .include(extra_charge.resource_credit());
        }
        let frame = global_frame_pool()
            .try_acquire_for_overwrite(read_len)
            .ok_or_else(|| StorageError::QueueFull {
                mount: "scheduler-read-frame".to_string(),
            })?;
        submission.record_frame_pool_charge(&frame);
        submission.trace_dispatch("positioned_read", class, 1);
        let frame = read_frame_leased(file.as_ref(), frame, offset).map_err(|error| {
            if matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::WriteZero
            ) {
                StorageError::ShortIo {
                    path: key.display().to_string(),
                    expected: len,
                    actual: 0,
                }
            } else {
                StorageError::io(key.display().to_string(), error)
            }
        })?;
        drop(submission);
        drop(read_buffer_queued_bytes);

        current_file_identity(&key, file.as_ref(), "payload_read")?;
        if class == IoClass::PeerRead {
            lock_reset_derived_cache(&self.peer_read_cache).consume(
                &key,
                file.identity(),
                offset,
                len,
            );
        }
        self.counters.read_ops_by_class[class_index(class)].fetch_add(1, Ordering::Relaxed);
        self.counters.bytes_read_by_class[class_index(class)]
            .fetch_add(len as u64, Ordering::Relaxed);
        self.counters.backend_read_ops_by_class[class_index(class)].fetch_add(1, Ordering::Relaxed);
        self.counters.backend_bytes_read_by_class[class_index(class)]
            .fetch_add(read_len as u64, Ordering::Relaxed);
        advise_after_read_class(file.as_ref(), class, offset, read_len, &self.counters);
        let latency_ns = latency_ns_since(started);
        self.counters.read_latency_ns_by_class[class_index(class)]
            .fetch_add(latency_ns, Ordering::Relaxed);
        record_latency_bucket(&self.counters.read_latency_buckets, latency_ns);

        if class == IoClass::PeerRead && read_len > len {
            let backing_bytes = frame.charged_bytes();
            let bytes = frame.into_bytes();
            let Some(cache_generation) = self.cache_generation else {
                return Err(StorageError::io(
                    "peer read cache",
                    io::Error::other("read-ahead completed without a cache generation"),
                ));
            };
            let exact = bytes.slice(..len);
            let cached_exact = peer_read_cache_store_parts(
                &self.peer_read_cache,
                &self.peer_read_cache_manager,
                &self.peer_read_cache_epoch,
                self.resources.as_ref(),
                &self.counters,
                PeerReadCacheInsert {
                    generation: cache_generation,
                    key,
                    identity: file.identity(),
                    source: file.source_weak(),
                    offset,
                    data: bytes,
                    max_entries: self.readahead_cache_entries,
                    max_bytes: self.readahead_cache_bytes,
                    backing_bytes,
                    result_len: len,
                },
            );
            Ok(StorageRead::Bytes(cached_exact.unwrap_or(exact)))
        } else {
            Ok(StorageRead::Frame(frame))
        }
    }
}

impl QueuedDiskBytes {
    fn reserve(
        counters: Arc<StorageCounters>,
        resources: Option<&ResourceGovernor>,
        bytes: u64,
        mount: impl std::fmt::Display,
    ) -> Result<Self, StorageError> {
        Self::reserve_charged(counters, resources, queued_disk_charge(bytes), mount)
    }

    fn reserve_charged(
        counters: Arc<StorageCounters>,
        resources: Option<&ResourceGovernor>,
        bytes: u64,
        mount: impl std::fmt::Display,
    ) -> Result<Self, StorageError> {
        let lease = if let Some(resources) = resources {
            let Some(lease) = resources.try_acquire(MemoryClass::QueuedDisk, bytes) else {
                counters.queue_full.fetch_add(1, Ordering::Relaxed);
                return Err(StorageError::QueueFull {
                    mount: mount.to_string(),
                });
            };
            Some(lease)
        } else {
            None
        };
        if bytes > 0 {
            counters
                .queued_disk_bytes
                .fetch_add(bytes, Ordering::Relaxed);
        }
        Ok(Self {
            bytes,
            counters,
            _lease: lease,
            _shared_payload_lease: None,
            shared_payload_view_bytes: None,
        })
    }

    fn reserve_shared_payload(
        counters: Arc<StorageCounters>,
        resources: Option<&ResourceGovernor>,
        payload_bytes: u64,
        payload_lease: Arc<MemoryLease>,
        mount: impl std::fmt::Display,
    ) -> Result<Self, StorageError> {
        if payload_lease.bytes() < payload_bytes {
            return Err(StorageError::io(
                mount.to_string(),
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "shared payload lease is smaller than the queued view",
                ),
            ));
        }

        // The payload allocation is already charged by its original owner.
        // Reserve only per-job queue overhead here, while keeping the logical
        // queued-byte counter comparable with ordinary owned submissions.
        let overhead = QUEUED_DISK_JOB_OVERHEAD_BYTES;
        let lease = if let Some(resources) = resources {
            let Some(lease) = resources.try_acquire(MemoryClass::QueuedDisk, overhead) else {
                counters.queue_full.fetch_add(1, Ordering::Relaxed);
                return Err(StorageError::QueueFull {
                    mount: mount.to_string(),
                });
            };
            Some(lease)
        } else {
            None
        };
        let bytes = queued_disk_charge(payload_bytes);
        counters
            .queued_disk_bytes
            .fetch_add(bytes, Ordering::Relaxed);
        Ok(Self {
            bytes,
            counters,
            _lease: lease,
            _shared_payload_lease: Some(payload_lease),
            shared_payload_view_bytes: Some(payload_bytes),
        })
    }

    fn resource_credit(&self) -> StorageCreditEvidence {
        let shared_payload = self._shared_payload_lease.as_deref();
        let governor_lease_id = self._lease.as_ref().and_then(MemoryLease::identity);
        let shared_payload_lease_id = shared_payload.and_then(MemoryLease::identity);
        StorageCreditEvidence {
            governor_credit_leases: if self._lease.is_some() { 1 } else { 0 },
            governor_credit_bytes: self._lease.as_ref().map_or(0, MemoryLease::bytes),
            governor_credit_lease_id: governor_lease_id,
            governor_credit_lease_ids_xor: governor_lease_id.unwrap_or_default(),
            governor_credit_lease_ids_sum: governor_lease_id.unwrap_or_default(),
            logical_queued_bytes: self.bytes,
            shared_payload_class: shared_payload.map(MemoryLease::class),
            shared_payload_classes_mixed: false,
            shared_payload_lease_count: if shared_payload.is_some() { 1 } else { 0 },
            shared_payload_lease_id,
            shared_payload_lease_ids_xor: shared_payload_lease_id.unwrap_or_default(),
            shared_payload_lease_ids_sum: shared_payload_lease_id.unwrap_or_default(),
            shared_payload_lease_bytes: shared_payload.map_or(0, MemoryLease::bytes),
            shared_payload_view_bytes: self.shared_payload_view_bytes.unwrap_or_default(),
            frame_pool_charge_bytes_at_dispatch: 0,
        }
    }
}

impl StorageCreditEvidence {
    fn include_frame_pool_charge(&mut self, bytes: u64) {
        self.frame_pool_charge_bytes_at_dispatch = self
            .frame_pool_charge_bytes_at_dispatch
            .saturating_add(bytes);
    }

    fn include(&mut self, other: Self) {
        let governor_lease_count = self
            .governor_credit_leases
            .saturating_add(other.governor_credit_leases);
        let shared_payload_lease_count = self
            .shared_payload_lease_count
            .saturating_add(other.shared_payload_lease_count);
        self.governor_credit_lease_id = if governor_lease_count == 1 {
            self.governor_credit_lease_id
                .or(other.governor_credit_lease_id)
        } else {
            None
        };
        self.governor_credit_lease_ids_xor ^= other.governor_credit_lease_ids_xor;
        self.governor_credit_lease_ids_sum = self
            .governor_credit_lease_ids_sum
            .wrapping_add(other.governor_credit_lease_ids_sum);
        self.shared_payload_lease_id = if shared_payload_lease_count == 1 {
            self.shared_payload_lease_id
                .or(other.shared_payload_lease_id)
        } else {
            None
        };
        self.shared_payload_lease_ids_xor ^= other.shared_payload_lease_ids_xor;
        self.shared_payload_lease_ids_sum = self
            .shared_payload_lease_ids_sum
            .wrapping_add(other.shared_payload_lease_ids_sum);
        self.governor_credit_leases = self
            .governor_credit_leases
            .saturating_add(other.governor_credit_leases);
        self.governor_credit_bytes = self
            .governor_credit_bytes
            .saturating_add(other.governor_credit_bytes);
        self.logical_queued_bytes = self
            .logical_queued_bytes
            .saturating_add(other.logical_queued_bytes);
        self.shared_payload_lease_bytes = self
            .shared_payload_lease_bytes
            .saturating_add(other.shared_payload_lease_bytes);
        self.shared_payload_lease_count = self
            .shared_payload_lease_count
            .saturating_add(other.shared_payload_lease_count);
        self.shared_payload_view_bytes = self
            .shared_payload_view_bytes
            .saturating_add(other.shared_payload_view_bytes);
        self.frame_pool_charge_bytes_at_dispatch = self
            .frame_pool_charge_bytes_at_dispatch
            .saturating_add(other.frame_pool_charge_bytes_at_dispatch);
        if self.shared_payload_classes_mixed || other.shared_payload_classes_mixed {
            self.shared_payload_class = None;
            self.shared_payload_classes_mixed = true;
        } else if let (Some(current), Some(next)) =
            (self.shared_payload_class, other.shared_payload_class)
        {
            if current != next {
                self.shared_payload_class = None;
                self.shared_payload_classes_mixed = true;
            }
        } else if self.shared_payload_class.is_none() {
            self.shared_payload_class = other.shared_payload_class;
        }
    }
}

/// Retains and describes the queued-memory credits owned by a hash job. The
/// job closure owns this value, so cancellation of its waiter cannot make the
/// data lease available while queued or running hash work still retains bytes.
#[derive(Debug)]
enum HashQueueCredit {
    Owned(QueuedDiskBytes),
    Shared(Arc<QueuedDiskBytes>),
}

impl HashQueueCredit {
    fn evidence(&self) -> StorageCreditEvidence {
        match self {
            Self::Owned(queued_bytes) => queued_bytes.resource_credit(),
            Self::Shared(queued_bytes) => queued_bytes.resource_credit(),
        }
    }
}

#[derive(Debug)]
struct HashJobCreditOwner {
    operation_id: Option<StorageOperationId>,
    operation_kind: &'static str,
    input_bytes: u64,
    credit: StorageCreditEvidence,
    _queued_bytes: HashQueueCredit,
}

impl HashJobCreditOwner {
    fn new(operation_kind: &'static str, input_bytes: u64, queued_bytes: HashQueueCredit) -> Self {
        // Hash jobs are on a hot path. Avoid a global identity atomic and
        // duplicate credit snapshot work unless trace output can consume it.
        let trace_enabled = tracing::enabled!(tracing::Level::TRACE);
        let operation_id = trace_enabled.then(allocate_storage_operation_id).flatten();
        let credit = if operation_id.is_some() {
            queued_bytes.evidence()
        } else {
            StorageCreditEvidence::default()
        };
        Self {
            operation_id,
            operation_kind,
            input_bytes,
            credit,
            _queued_bytes: queued_bytes,
        }
    }

    fn trace_submission(&self) {
        let Some(operation_id) = self.operation_id else {
            return;
        };
        let credit = self.credit;
        tracing::trace!(
            storage_operation_id = operation_id.0,
            hash_operation = self.operation_kind,
            hash_input_bytes = self.input_bytes,
            resource_credit_governor_leases = credit.governor_credit_leases,
            resource_credit_governor_bytes = credit.governor_credit_bytes,
            resource_credit_governor_lease_id = credit.governor_credit_lease_id,
            resource_credit_governor_lease_ids_xor = credit.governor_credit_lease_ids_xor,
            resource_credit_governor_lease_ids_sum = credit.governor_credit_lease_ids_sum,
            resource_credit_logical_queued_bytes = credit.logical_queued_bytes,
            resource_credit_shared_payload_class = ?credit.shared_payload_class,
            resource_credit_shared_payload_classes_mixed = credit.shared_payload_classes_mixed,
            resource_credit_shared_payload_leases = credit.shared_payload_lease_count,
            resource_credit_shared_payload_lease_id = credit.shared_payload_lease_id,
            resource_credit_shared_payload_lease_ids_xor =
                credit.shared_payload_lease_ids_xor,
            resource_credit_shared_payload_lease_ids_sum =
                credit.shared_payload_lease_ids_sum,
            resource_credit_shared_payload_lease_bytes = credit.shared_payload_lease_bytes,
            resource_credit_shared_payload_view_bytes = credit.shared_payload_view_bytes,
            "hash job submission requested"
        );
    }
}

impl Drop for HashJobCreditOwner {
    fn drop(&mut self) {
        let Some(operation_id) = self.operation_id else {
            return;
        };
        let credit = self.credit;
        tracing::trace!(
            storage_operation_id = operation_id.0,
            hash_operation = self.operation_kind,
            hash_input_bytes = self.input_bytes,
            resource_credit_governor_leases = credit.governor_credit_leases,
            resource_credit_governor_bytes = credit.governor_credit_bytes,
            resource_credit_governor_lease_id = credit.governor_credit_lease_id,
            resource_credit_governor_lease_ids_xor = credit.governor_credit_lease_ids_xor,
            resource_credit_governor_lease_ids_sum = credit.governor_credit_lease_ids_sum,
            resource_credit_logical_queued_bytes = credit.logical_queued_bytes,
            resource_credit_shared_payload_class = ?credit.shared_payload_class,
            resource_credit_shared_payload_classes_mixed = credit.shared_payload_classes_mixed,
            resource_credit_shared_payload_leases = credit.shared_payload_lease_count,
            resource_credit_shared_payload_lease_id = credit.shared_payload_lease_id,
            resource_credit_shared_payload_lease_ids_xor =
                credit.shared_payload_lease_ids_xor,
            resource_credit_shared_payload_lease_ids_sum =
                credit.shared_payload_lease_ids_sum,
            resource_credit_shared_payload_lease_bytes = credit.shared_payload_lease_bytes,
            resource_credit_shared_payload_view_bytes = credit.shared_payload_view_bytes,
            "hash job credit owner released"
        );
    }
}

impl Drop for QueuedDiskBytes {
    fn drop(&mut self) {
        if self.bytes > 0 {
            self.counters
                .queued_disk_bytes
                .fetch_sub(self.bytes, Ordering::Relaxed);
        }
    }
}

fn queued_disk_charge(payload_bytes: u64) -> u64 {
    payload_bytes.max(QUEUED_DISK_JOB_OVERHEAD_BYTES)
}

fn validate_hybrid_piece_hash_geometry(
    data: &bytes::Bytes,
    v2_content_len: usize,
    piece_length: usize,
) -> Result<(), StorageError> {
    if v2_content_len == 0
        || v2_content_len > data.len()
        || data.len() > piece_length
        || piece_length < V2_BLOCK_SIZE
        || !piece_length.is_power_of_two()
        || !piece_length.is_multiple_of(V2_BLOCK_SIZE)
    {
        return Err(StorageError::io(
            "rt-storage-hash",
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid hybrid piece hash geometry",
            ),
        ));
    }
    Ok(())
}

type BackendOperationOwnership<G, O> = Arc<Mutex<Option<(G, O)>>>;

fn backend_operation_ownership<G: Send + 'static, O: Send + 'static>(
    fence_guard: G,
    operation_owner: O,
) -> (BackendOperationOwnership<G, O>, BackendJobOwnership) {
    let ownership = Arc::new(Mutex::new(Some((fence_guard, operation_owner))));
    let backend_owner = BackendJobOwnership::new(Arc::clone(&ownership));
    (ownership, backend_owner)
}

async fn await_backend_io_with_fence<T, G, O>(
    path: &Path,
    rx: oneshot::Receiver<io::Result<T>>,
    ownership: BackendOperationOwnership<G, O>,
    expected_len: Option<usize>,
) -> Result<(T, G, O), StorageError> {
    let result = rx.await.map_err(|_| StorageError::Cancelled)?;
    match result {
        Ok(value) => {
            let (fence_guard, operation_owner) = lock_recover(&ownership)
                .take()
                .ok_or(StorageError::Cancelled)?;
            Ok((value, fence_guard, operation_owner))
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Err(StorageError::QueueFull {
            mount: "storage-backend".to_string(),
        }),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::UnexpectedEof | io::ErrorKind::WriteZero
            ) =>
        {
            Err(StorageError::ShortIo {
                path: path.display().to_string(),
                expected: expected_len.unwrap_or(0),
                actual: 0,
            })
        }
        Err(error) => Err(StorageError::io(path.display().to_string(), error)),
    }
}

const DEVICE_CLASS_ORDER: [IoClass; 6] = [
    IoClass::Foreground,
    IoClass::PeerRead,
    IoClass::PeerWrite,
    IoClass::MoveCopy,
    IoClass::Recheck,
    IoClass::Metadata,
];
const DEVICE_DISPATCH_SLOT_COUNT: usize = 32;

struct DeviceWaiter {
    id: u64,
    priority: IoRequestPriority,
    sender: oneshot::Sender<DeviceOperationPermit>,
}

struct DeviceMountWaiters {
    by_class: [VecDeque<DeviceWaiter>; 6],
}

impl DeviceMountWaiters {
    fn new() -> Self {
        Self {
            by_class: std::array::from_fn(|_| VecDeque::new()),
        }
    }

    fn is_empty(&self) -> bool {
        self.by_class.iter().all(|queue| queue.is_empty())
    }
}

struct DeviceQueueState {
    active: usize,
    /// Maintained on enqueue/dequeue so depth checks do not scan owner lanes.
    pending_count: usize,
    /// Pending operations are partitioned by storage-root owner, then class.
    pending_by_mount: HashMap<Arc<str>, DeviceMountWaiters>,
    /// Each class gives one turn at a time to every storage-root owner.
    mount_turns: [VecDeque<Arc<str>>; 6],
    next_waiter_id: u64,
    next_operation_id: u64,
    dispatch_cursor: usize,
}

struct DeviceQueue {
    /// Stable identity of the shared physical-device queue, or its bounded
    /// storage-root fallback when topology could not be resolved.
    key: Arc<str>,
    capacity: usize,
    active_limit: usize,
    state: Mutex<DeviceQueueState>,
}

impl std::fmt::Debug for DeviceQueue {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceQueue")
            .field("key", &self.key)
            .field("capacity", &self.capacity)
            .field("active_limit", &self.active_limit)
            .finish_non_exhaustive()
    }
}

/// Resolved device queues for a multi-path operation. Long-running cursors
/// prepare this once so each bounded quantum does not repeat filesystem
/// topology discovery.
pub(crate) struct DeviceQueueSet {
    additional_queues: Vec<(String, String, Arc<DeviceQueue>)>,
}

struct DeviceOperationPermit {
    queue: Arc<DeviceQueue>,
    operation_id: DeviceOperationId,
    armed: bool,
}

impl std::fmt::Debug for DeviceOperationPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceOperationPermit")
            .field("operation_id", &self.operation_id)
            .field("armed", &self.armed)
            .finish_non_exhaustive()
    }
}

struct DeviceWaiterGuard {
    queue: Weak<DeviceQueue>,
    id: u64,
    armed: bool,
}

impl DeviceQueue {
    fn new(key: Arc<str>, capacity: usize, active_limit: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            key,
            capacity,
            active_limit: active_limit.max(1).min(capacity),
            state: Mutex::new(DeviceQueueState {
                active: 0,
                pending_count: 0,
                pending_by_mount: HashMap::new(),
                mount_turns: std::array::from_fn(|_| VecDeque::new()),
                next_waiter_id: 1,
                next_operation_id: 1,
                dispatch_cursor: 0,
            }),
        }
    }

    /// Reserve only when there are no earlier waiters. Used inside a disk
    /// worker after a cache miss; contended callers use normal async
    /// admission, preserving class/owner fairness and cancellation.
    fn try_acquire_uncontended(self: &Arc<Self>) -> Option<DeviceOperationPermit> {
        let mut state = lock_recover(&self.state);
        if state.active >= self.active_limit || state.pending_count != 0 {
            return None;
        }
        state.active += 1;
        Some(DeviceOperationPermit {
            queue: self.clone(),
            operation_id: DeviceOperationId(take_device_sequence(&mut state.next_operation_id)),
            armed: true,
        })
    }

    async fn acquire(
        self: &Arc<Self>,
        class: IoClass,
        mount: String,
    ) -> Result<DeviceOperationPermit, StorageError> {
        self.acquire_with_priority(class, mount, IoRequestPriority::normal())
            .await
    }

    async fn acquire_with_priority(
        self: &Arc<Self>,
        class: IoClass,
        mount: String,
        priority: IoRequestPriority,
    ) -> Result<DeviceOperationPermit, StorageError> {
        let (waiter_id, immediate, grant, receiver) = {
            let mut state = lock_recover(&self.state);
            let grant = collect_device_grant(self, &mut state);

            if state.active < self.active_limit && state.pending_count == 0 {
                state.active += 1;
                let operation_id =
                    DeviceOperationId(take_device_sequence(&mut state.next_operation_id));
                (
                    None,
                    Some(DeviceOperationPermit {
                        queue: self.clone(),
                        operation_id,
                        armed: true,
                    }),
                    grant,
                    None,
                )
            } else {
                let pending_limit = self.capacity.saturating_sub(self.active_limit);
                if state.pending_count >= pending_limit {
                    drop(state);
                    dispatch_device_grant(grant);
                    return Err(StorageError::QueueFull { mount });
                }

                let (sender, receiver) = oneshot::channel();
                let id = take_device_sequence(&mut state.next_waiter_id);
                enqueue_device_waiter(
                    &mut state,
                    class,
                    Arc::from(mount),
                    DeviceWaiter {
                        id,
                        priority,
                        sender,
                    },
                );
                (Some(id), None, grant, Some(receiver))
            }
        };
        dispatch_device_grant(grant);

        if let Some(permit) = immediate {
            return Ok(permit);
        }

        let waiter_id = waiter_id.expect("queued device admission has an ID");
        let mut waiter_guard = DeviceWaiterGuard {
            queue: Arc::downgrade(self),
            id: waiter_id,
            armed: true,
        };
        let permit = receiver
            .expect("queued device admission has a reply channel")
            .await
            .map_err(|_| StorageError::Cancelled)?;
        waiter_guard.armed = false;
        Ok(permit)
    }

    fn available(&self) -> usize {
        let state = lock_recover(&self.state);
        self.capacity
            .saturating_sub(state.active.saturating_add(state.pending_count))
    }

    fn cancel_waiter(self: &Arc<Self>, id: u64) {
        let grant = {
            let mut state = lock_recover(&self.state);
            'mounts: for mount_waiters in state.pending_by_mount.values_mut() {
                for queue in &mut mount_waiters.by_class {
                    if let Some(position) = queue.iter().position(|waiter| waiter.id == id) {
                        queue.remove(position);
                        break 'mounts;
                    }
                }
            }
            prune_closed_device_waiters(&mut state);
            collect_device_grant(self, &mut state)
        };
        dispatch_device_grant(grant);
    }

    fn release(self: &Arc<Self>) {
        dispatch_device_grant(self.release_slot());
    }

    fn release_slot(self: &Arc<Self>) -> Option<DeviceGrant> {
        {
            let mut state = lock_recover(&self.state);
            state.active = state.active.saturating_sub(1);
            collect_device_grant(self, &mut state)
        }
    }
}

impl Drop for DeviceOperationPermit {
    fn drop(&mut self) {
        if self.armed {
            self.queue.release();
        }
    }
}

impl DeviceOperationPermit {
    fn disarm(mut self) -> Arc<DeviceQueue> {
        self.armed = false;
        Arc::clone(&self.queue)
    }
}

impl Drop for DeviceWaiterGuard {
    fn drop(&mut self) {
        if self.armed {
            if let Some(queue) = self.queue.upgrade() {
                queue.cancel_waiter(self.id);
            }
        }
    }
}

type DeviceGrant = (
    oneshot::Sender<DeviceOperationPermit>,
    DeviceOperationPermit,
);

fn dispatch_device_grant(grant: Option<DeviceGrant>) {
    let mut grant = grant;
    while let Some((sender, permit)) = grant {
        // Sending outside the queue mutex matters: a receiver may have been
        // cancelled after its slot was reserved, and dropping the returned
        // permit must be able to release that slot without reentrant locking.
        match sender.send(permit) {
            Ok(()) => return,
            Err(permit) => {
                let queue = permit.disarm();
                grant = queue.release_slot();
            }
        }
    }
}

fn collect_device_grant(
    queue: &Arc<DeviceQueue>,
    state: &mut DeviceQueueState,
) -> Option<DeviceGrant> {
    while state.active < queue.active_limit {
        let (slot, waiter) = pop_next_device_waiter(state, Instant::now())?;
        state.dispatch_cursor = next_device_cursor(slot);
        if waiter.sender.is_closed() {
            continue;
        }
        state.active += 1;
        let operation_id = DeviceOperationId(take_device_sequence(&mut state.next_operation_id));
        let grant = (
            waiter.sender,
            DeviceOperationPermit {
                queue: queue.clone(),
                operation_id,
                armed: true,
            },
        );
        return Some(grant);
    }
    None
}

fn enqueue_device_waiter(
    state: &mut DeviceQueueState,
    class: IoClass,
    mount: Arc<str>,
    waiter: DeviceWaiter,
) {
    let class_index = device_class_index(class);
    let mount_waiters = state
        .pending_by_mount
        .entry(mount.clone())
        .or_insert_with(DeviceMountWaiters::new);
    let queue = &mut mount_waiters.by_class[class_index];
    let add_mount_turn = queue.is_empty();
    queue.push_back(waiter);
    state.pending_count = state.pending_count.saturating_add(1);
    if add_mount_turn {
        state.mount_turns[class_index].push_back(mount);
    }
}

fn pop_mount_waiter(state: &mut DeviceQueueState, class_index: usize) -> Option<DeviceWaiter> {
    while let Some(mount) = state.mount_turns[class_index].pop_front() {
        let Some(mount_waiters) = state.pending_by_mount.get_mut(&mount) else {
            continue;
        };
        let Some(waiter) = mount_waiters.by_class[class_index].pop_front() else {
            continue;
        };
        state.pending_count = state.pending_count.saturating_sub(1);
        let mount_has_more = !mount_waiters.by_class[class_index].is_empty();
        let mount_is_empty = mount_waiters.is_empty();
        if mount_has_more {
            state.mount_turns[class_index].push_back(Arc::clone(&mount));
        }
        if mount_is_empty {
            state.pending_by_mount.remove(&mount);
        }
        return Some(waiter);
    }
    None
}

fn pop_next_device_waiter(
    state: &mut DeviceQueueState,
    now: Instant,
) -> Option<(usize, DeviceWaiter)> {
    if let Some(urgent) = pop_urgent_device_waiter(state, now) {
        return Some(urgent);
    }
    for offset in 0..DEVICE_DISPATCH_SLOT_COUNT {
        let slot = (state.dispatch_cursor + offset) % DEVICE_DISPATCH_SLOT_COUNT;
        let class_index = device_class_index(device_dispatch_class(slot));
        if let Some(waiter) = pop_mount_waiter(state, class_index) {
            return Some((slot, waiter));
        }
    }
    None
}

/// Promote choke-critical obligations immediately and expired-deadline work
/// by earliest deadline. Only this bounded, already-admitted wait queue is
/// scanned, and only when a device slot is being dispatched; enqueue and
/// availability checks remain constant-time.
fn pop_urgent_device_waiter(
    state: &mut DeviceQueueState,
    now: Instant,
) -> Option<(usize, DeviceWaiter)> {
    let mut selected: Option<(bool, Instant, u64, Arc<str>, usize, usize)> = None;
    for (mount, mount_waiters) in &state.pending_by_mount {
        for (class_index, queue) in mount_waiters.by_class.iter().enumerate() {
            for (position, waiter) in queue.iter().enumerate() {
                let expired_deadline = waiter.priority.deadline.filter(|deadline| *deadline <= now);
                if !waiter.priority.choke_critical && expired_deadline.is_none() {
                    continue;
                }
                let critical = waiter.priority.choke_critical;
                let deadline = expired_deadline.unwrap_or(now);
                let candidate = (
                    critical,
                    deadline,
                    waiter.id,
                    Arc::clone(mount),
                    class_index,
                    position,
                );
                let is_better = selected.as_ref().is_none_or(|current| {
                    (candidate.0 && !current.0)
                        || (candidate.0 == current.0
                            && (candidate.1, candidate.2) < (current.1, current.2))
                });
                if is_better {
                    selected = Some(candidate);
                }
            }
        }
    }
    let (_, _, _, mount, class_index, position) = selected?;
    let mount_waiters = state.pending_by_mount.get_mut(&mount)?;
    let queue = &mut mount_waiters.by_class[class_index];
    let waiter = queue.remove(position)?;
    state.pending_count = state.pending_count.saturating_sub(1);
    let mount_has_more = !queue.is_empty();
    let mount_is_empty = mount_waiters.is_empty();
    if !mount_has_more {
        state.mount_turns[class_index].retain(|turn| turn != &mount);
    }
    if mount_is_empty {
        state.pending_by_mount.remove(&mount);
    }
    let slot = (0..DEVICE_DISPATCH_SLOT_COUNT)
        .map(|offset| (state.dispatch_cursor + offset) % DEVICE_DISPATCH_SLOT_COUNT)
        .find(|slot| device_class_index(device_dispatch_class(*slot)) == class_index)
        .unwrap_or(state.dispatch_cursor);
    Some((slot, waiter))
}

fn next_device_cursor(slot: usize) -> usize {
    (slot + 1) % DEVICE_DISPATCH_SLOT_COUNT
}

fn device_dispatch_class(slot: usize) -> IoClass {
    let mut position = slot % DEVICE_DISPATCH_SLOT_COUNT;
    for class in DEVICE_CLASS_ORDER {
        let weight = elevator_class_weight(class);
        if position < weight {
            return class;
        }
        position -= weight;
    }
    IoClass::Metadata
}

fn prune_closed_device_waiters(state: &mut DeviceQueueState) {
    let mut pending_count = 0usize;
    for mount_waiters in state.pending_by_mount.values_mut() {
        for queue in &mut mount_waiters.by_class {
            queue.retain(|waiter| !waiter.sender.is_closed());
            pending_count = pending_count.saturating_add(queue.len());
        }
    }
    state.pending_count = pending_count;

    state
        .pending_by_mount
        .retain(|_, mount_waiters| !mount_waiters.is_empty());
    let pending_by_mount = &state.pending_by_mount;
    for class_index in 0..state.mount_turns.len() {
        state.mount_turns[class_index].retain(|mount| {
            pending_by_mount
                .get(mount)
                .is_some_and(|waiters| !waiters.by_class[class_index].is_empty())
        });
    }
}

fn device_class_index(class: IoClass) -> usize {
    match class {
        IoClass::Metadata => 0,
        IoClass::Recheck => 1,
        IoClass::MoveCopy => 2,
        IoClass::PeerWrite => 3,
        IoClass::PeerRead => 4,
        IoClass::Foreground => 5,
    }
}

fn take_device_sequence(next: &mut u64) -> u64 {
    let current = (*next).max(1);
    *next = current.wrapping_add(1).max(1);
    current
}

static DEVICE_QUEUES: Lazy<Mutex<HashMap<String, Weak<DeviceQueue>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

fn device_queue_for(
    storage_root: StorageRootId,
    device_id: Option<&str>,
    capacity: usize,
    active_limit: usize,
) -> Arc<DeviceQueue> {
    let key = device_queue_key(storage_root, device_id);
    let mut queues = lock_recover(&DEVICE_QUEUES);
    if let Some(queue) = queues.get(&key).and_then(Weak::upgrade) {
        return queue;
    }
    queues.retain(|_, queue| queue.strong_count() > 0);
    let queue = Arc::new(DeviceQueue::new(
        Arc::from(key.as_str()),
        capacity,
        active_limit,
    ));
    queues.insert(key, Arc::downgrade(&queue));
    queue
}

fn device_queue_key(storage_root: StorageRootId, device_id: Option<&str>) -> String {
    match device_id {
        Some(device_id) => format!("device:{device_id}"),
        None => format!("storage-root:{}", storage_root.0),
    }
}

impl PeerReadElevator {
    // This constructor wires independent resource pools owned by the storage
    // runtime. Keeping the arguments explicit makes ownership and scheduling
    // boundaries visible; a context object would hide those dependencies.
    #[allow(clippy::too_many_arguments)]
    fn spawn(
        storage_root: StorageRootId,
        queue_depth: usize,
        inflight_limit: usize,
        budget: Duration,
        file_pool: Arc<FilePool>,
        io_pool: Arc<BlockingPool>,
        disk_backend: Arc<SelectedDiskBackend>,
        queue_sem: Arc<Semaphore>,
        device_queue: Arc<DeviceQueue>,
        counters: Arc<StorageCounters>,
        resources: Option<ResourceGovernor>,
    ) -> Self {
        let (sender, receiver) = tokio_mpsc::channel(queue_depth.max(1));
        tokio::spawn(peer_read_elevator_worker(
            storage_root,
            receiver,
            inflight_limit,
            budget,
            file_pool,
            io_pool,
            disk_backend,
            queue_sem,
            device_queue,
            counters.clone(),
        ));
        Self {
            sender,
            counters,
            resources,
        }
    }

    async fn read(
        &self,
        path: PathBuf,
        offset: u64,
        len: usize,
    ) -> Result<bytes::Bytes, StorageError> {
        self.try_enqueue(path, offset, len)?
            .await
            .map_err(|_| StorageError::Cancelled)?
    }

    fn try_enqueue(
        &self,
        path: PathBuf,
        offset: u64,
        len: usize,
    ) -> Result<oneshot::Receiver<Result<bytes::Bytes, StorageError>>, StorageError> {
        let (tx, rx) = oneshot::channel();
        let queued_bytes = QueuedDiskBytes::reserve(
            self.counters.clone(),
            self.resources.as_ref(),
            len as u64,
            "peer-read-elevator",
        )?;
        self.sender
            .try_send(PeerReadRequest {
                path,
                offset,
                len,
                _queued_bytes: Some(queued_bytes),
                tx,
            })
            .map_err(|err| match err {
                TrySendError::Full(_) => {
                    self.counters
                        .peer_read_elevator_queue_full
                        .fetch_add(1, Ordering::Relaxed);
                    StorageError::QueueFull {
                        mount: "peer-read-elevator".to_string(),
                    }
                }
                TrySendError::Closed(_) => StorageError::Cancelled,
            })?;
        Ok(rx)
    }

    fn queued_len(&self) -> usize {
        self.sender
            .max_capacity()
            .saturating_sub(self.sender.capacity())
    }
}

#[allow(clippy::too_many_arguments)]
async fn peer_read_elevator_worker(
    _storage_root: StorageRootId,
    mut receiver: tokio_mpsc::Receiver<PeerReadRequest>,
    inflight_limit: usize,
    budget: Duration,
    file_pool: Arc<FilePool>,
    io_pool: Arc<BlockingPool>,
    disk_backend: Arc<SelectedDiskBackend>,
    queue_sem: Arc<Semaphore>,
    device_queue: Arc<DeviceQueue>,
    counters: Arc<StorageCounters>,
) {
    let inflight = Arc::new(Semaphore::new(inflight_limit.max(1)));
    while let Some(first) = receiver.recv().await {
        let requests = collect_peer_read_batch(&mut receiver, first, budget).await;
        let batches = peer_read_batches(requests);
        for batch in batches {
            let permit = match inflight.clone().acquire_owned().await {
                Ok(permit) => permit,
                Err(_) => return,
            };
            let file_pool = file_pool.clone();
            let io_pool = io_pool.clone();
            let disk_backend = disk_backend.clone();
            let queue_sem = queue_sem.clone();
            let device_queue = device_queue.clone();
            let counters = counters.clone();
            tokio::spawn(async move {
                let _permit = permit;
                let results = dispatch_peer_read_batch(
                    batch,
                    file_pool,
                    io_pool,
                    disk_backend,
                    queue_sem,
                    device_queue,
                    counters,
                )
                .await;
                for (tx, result) in results {
                    let _ = tx.send(result);
                }
            });
        }
    }
}

async fn collect_peer_read_batch(
    receiver: &mut tokio_mpsc::Receiver<PeerReadRequest>,
    first: PeerReadRequest,
    budget: Duration,
) -> Vec<PeerReadRequest> {
    let started = Instant::now();
    let max_wait = budget.saturating_mul(4).max(budget);
    let quiet = Duration::from_millis(2);
    let mut requests = vec![first];

    loop {
        while let Ok(request) = receiver.try_recv() {
            requests.push(request);
        }

        let elapsed = started.elapsed();
        if budget.is_zero() || elapsed >= max_wait {
            break;
        }

        let wait = quiet.min(max_wait - elapsed);

        match tokio::time::timeout(wait, receiver.recv()).await {
            Ok(Some(request)) => requests.push(request),
            Ok(None) => break,
            Err(_) => break,
        }
    }

    requests
}

fn peer_read_batches(mut requests: Vec<PeerReadRequest>) -> Vec<PeerReadBatch> {
    requests.sort_by(|a, b| {
        normalized_key(&a.path)
            .cmp(&normalized_key(&b.path))
            .then_with(|| a.offset.cmp(&b.offset))
    });

    let mut batches: Vec<PeerReadBatch> = Vec::with_capacity(requests.len());
    for request in requests {
        let Some(request_end) = request.offset.checked_add(request.len as u64) else {
            batches.push(PeerReadBatch {
                path: request.path.clone(),
                offset: request.offset,
                len: request.len,
                requests: vec![request],
            });
            continue;
        };
        if let Some(last) = batches.last_mut() {
            let last_end = last.offset.checked_add(last.len as u64);
            if normalized_key(&last.path) == normalized_key(&request.path)
                && last_end.is_some_and(|end| request.offset <= end)
            {
                let Some(merged_len) = request_end
                    .checked_sub(last.offset)
                    .and_then(|length| usize::try_from(length).ok())
                else {
                    batches.push(PeerReadBatch {
                        path: request.path.clone(),
                        offset: request.offset,
                        len: request.len,
                        requests: vec![request],
                    });
                    continue;
                };
                last.len = merged_len.max(last.len);
                last.requests.push(request);
                continue;
            }
        }
        batches.push(PeerReadBatch {
            path: request.path.clone(),
            offset: request.offset,
            len: request.len,
            requests: vec![request],
        });
    }
    batches
}

async fn dispatch_peer_read_batch(
    mut batch: PeerReadBatch,
    file_pool: Arc<FilePool>,
    io_pool: Arc<BlockingPool>,
    disk_backend: Arc<SelectedDiskBackend>,
    queue_sem: Arc<Semaphore>,
    device_queue: Arc<DeviceQueue>,
    counters: Arc<StorageCounters>,
) -> Vec<(
    oneshot::Sender<Result<bytes::Bytes, StorageError>>,
    Result<bytes::Bytes, StorageError>,
)> {
    // A caller can drop the read future while its request is still waiting in
    // the elevator. Do not acquire queue permits or perform filesystem work
    // for a result nobody can receive.
    batch.requests.retain(|request| !request.tx.is_closed());
    if batch.requests.is_empty() {
        return Vec::new();
    }

    let _queue = match queue_sem.acquire_owned().await {
        Ok(permit) => permit,
        Err(_) => {
            return batch
                .requests
                .into_iter()
                .map(|request| (request.tx, Err(StorageError::Cancelled)))
                .collect();
        }
    };
    let path = batch.path.clone();
    let offset = batch.offset;
    let len = batch.len;
    let read_counters = counters.clone();
    let read = match io_pool
        .run(move || {
            let key = normalized_key(&path);
            let file = file_pool.get_or_open(&key, OpenMode::Read, false)?;
            advise_for_read_class(&file, IoClass::PeerRead, offset, len, &read_counters);
            Ok::<_, StorageError>((key, file))
        })
        .await
    {
        Ok((key, file)) => {
            let _device_queue = match device_queue
                .acquire(IoClass::PeerRead, "storage-peer-read-elevator".to_owned())
                .await
            {
                Ok(permit) => permit,
                Err(error) => {
                    if matches!(error, StorageError::QueueFull { .. }) {
                        counters.queue_full.fetch_add(1, Ordering::Relaxed);
                    }
                    return batch
                        .requests
                        .into_iter()
                        .map(|request| (request.tx, Err(error.clone_for_waiter())))
                        .collect();
                }
            };
            batch.requests.retain(|request| !request.tx.is_closed());
            if batch.requests.is_empty() {
                return Vec::new();
            }
            let Some(storage_operation_id) = allocate_storage_operation_id() else {
                let error = StorageError::io(
                    "storage operation identity",
                    io::Error::other("process-wide storage operation identity space exhausted"),
                );
                return batch
                    .requests
                    .into_iter()
                    .map(|request| (request.tx, Err(error.clone_for_waiter())))
                    .collect();
            };
            let mut operation_record = StorageOperationRecord {
                storage_operation_id: Some(storage_operation_id),
                ..StorageOperationRecord::default()
            };
            operation_record.bind_device_operation(&_device_queue);
            operation_record.bind_file_identity(file.identity());
            operation_record.bind_byte_range(offset, len as u64);
            for request in &batch.requests {
                operation_record.resource_credit.include(
                    request
                        ._queued_bytes
                        .as_ref()
                        .expect("queued read owns its credit until backend submission")
                        .resource_credit(),
                );
            }
            let frame = match global_frame_pool().try_acquire_for_overwrite(len) {
                Some(frame) => frame,
                None => {
                    counters.queue_full.fetch_add(1, Ordering::Relaxed);
                    return batch
                        .requests
                        .into_iter()
                        .map(|request| {
                            (
                                request.tx,
                                Err(StorageError::QueueFull {
                                    mount: "scheduler-read-frame".to_string(),
                                }),
                            )
                        })
                        .collect();
                }
            };
            operation_record
                .resource_credit
                .include_frame_pool_charge(frame.charged_bytes());
            let credit = operation_record.resource_credit;
            tracing::trace!(
                storage_operation_id = operation_record.storage_operation_id.unwrap().0,
                device_operation_id = operation_record.device_operation_id.unwrap().0,
                device_queue_key = operation_record.device_queue_key.as_deref().unwrap_or("unknown"),
                byte_offset = offset,
                byte_length = len as u64,
                queued_request_count = batch.requests.len(),
                resource_credit_governor_leases = credit.governor_credit_leases,
                resource_credit_governor_bytes = credit.governor_credit_bytes,
                resource_credit_governor_lease_id = credit.governor_credit_lease_id,
                resource_credit_governor_lease_ids_xor =
                    credit.governor_credit_lease_ids_xor,
                resource_credit_governor_lease_ids_sum =
                    credit.governor_credit_lease_ids_sum,
                resource_credit_logical_queued_bytes = credit.logical_queued_bytes,
                resource_credit_shared_payload_class = ?credit.shared_payload_class,
                resource_credit_shared_payload_classes_mixed = credit.shared_payload_classes_mixed,
                resource_credit_shared_payload_leases = credit.shared_payload_lease_count,
                resource_credit_shared_payload_lease_id = credit.shared_payload_lease_id,
                resource_credit_shared_payload_lease_ids_xor =
                    credit.shared_payload_lease_ids_xor,
                resource_credit_shared_payload_lease_ids_sum =
                    credit.shared_payload_lease_ids_sum,
                resource_credit_shared_payload_lease_bytes = credit.shared_payload_lease_bytes,
                resource_credit_shared_payload_view_bytes = credit.shared_payload_view_bytes,
                resource_credit_frame_pool_charge_bytes_at_dispatch =
                    credit.frame_pool_charge_bytes_at_dispatch,
                class = ?IoClass::PeerRead,
                mount_queue_permits = 1,
                device_queue_permits = 1,
                "dispatching coalesced peer read"
            );
            let queued_byte_owners = batch
                .requests
                .iter_mut()
                .filter_map(|request| request._queued_bytes.take())
                .collect::<Vec<_>>();
            let (ownership, backend_owner) =
                backend_operation_ownership((_queue, _device_queue), queued_byte_owners);
            let completion = disk_backend.pread_leased(file.clone(), frame, offset, backend_owner);
            match await_backend_io_with_fence(&key, completion, ownership, Some(len)).await {
                Ok((frame, (queue_permit, device_permit), queued_byte_owners)) => {
                    let bytes = frame.into_bytes();
                    advise_after_read_class(&file, IoClass::PeerRead, offset, len, &counters);
                    drop((queue_permit, device_permit, queued_byte_owners));
                    Ok(bytes)
                }
                Err(error) => {
                    if matches!(error, StorageError::QueueFull { .. }) {
                        counters.queue_full.fetch_add(1, Ordering::Relaxed);
                    }
                    Err(error)
                }
            }
        }
        Err(error) => Err(error),
    };

    match read {
        Ok(bytes) => {
            counters
                .peer_read_elevator_batches
                .fetch_add(1, Ordering::Relaxed);
            counters.peer_read_elevator_coalesced_requests.fetch_add(
                batch.requests.len().saturating_sub(1) as u64,
                Ordering::Relaxed,
            );
            counters.backend_read_ops_by_class[class_index(IoClass::PeerRead)]
                .fetch_add(1, Ordering::Relaxed);
            counters.backend_bytes_read_by_class[class_index(IoClass::PeerRead)]
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            batch
                .requests
                .into_iter()
                .map(|request| {
                    let relative = request
                        .offset
                        .checked_sub(batch.offset)
                        .and_then(|relative| usize::try_from(relative).ok());
                    let end = relative.and_then(|relative| relative.checked_add(request.len));
                    match (relative, end) {
                        (Some(relative), Some(end)) if end <= bytes.len() => {
                            (request.tx, Ok(bytes.slice(relative..end)))
                        }
                        _ => {
                            let actual = relative
                                .map(|relative| bytes.len().saturating_sub(relative))
                                .unwrap_or(0);
                            (
                                request.tx,
                                Err(StorageError::ShortIo {
                                    path: request.path.to_string_lossy().into_owned(),
                                    expected: request.len,
                                    actual,
                                }),
                            )
                        }
                    }
                })
                .collect()
        }
        Err(error) => {
            if matches!(error, StorageError::QueueFull { .. }) {
                counters.queue_full.fetch_add(1, Ordering::Relaxed);
            }
            let mut requests = batch.requests.into_iter();
            let Some(first) = requests.next() else {
                return Vec::new();
            };
            let mut out = vec![(first.tx, Err(error))];
            out.extend(requests.map(|request| (request.tx, Err(StorageError::Cancelled))));
            out
        }
    }
}

impl MountScheduler {
    pub fn new(storage_root: StorageRootId, config: &SchedulerConfig) -> Self {
        Self::new_with_profile_and_io_config(
            storage_root,
            config,
            config.profile.clone(),
            effective_io_config_for_topology(&config.storage_io, None),
            None,
        )
    }

    pub fn new_for_path(
        storage_root: StorageRootId,
        path: &Path,
        config: &SchedulerConfig,
    ) -> Self {
        let topology = detect_storage_topology(path);
        let profile = match &config.profile {
            StorageProfile::Unknown => topology.profile.clone(),
            profile => profile.clone(),
        };
        Self::new_with_profile_and_io_config_and_resources(
            storage_root,
            config,
            profile,
            effective_io_config_for_topology(&config.storage_io, Some(&topology)),
            Some(topology),
            Some(shared_scheduler_resources(&config.storage_io)),
        )
    }

    fn new_with_profile_and_io_config(
        storage_root: StorageRootId,
        config: &SchedulerConfig,
        profile: StorageProfile,
        io_config: StorageIoConfig,
        topology: Option<StorageTopology>,
    ) -> Self {
        let local_resources = local_scheduler_resources(&io_config);
        Self::new_with_profile_and_io_config_and_resources(
            storage_root,
            config,
            profile,
            io_config,
            topology,
            Some(local_resources),
        )
    }

    fn new_with_profile_and_io_config_and_resources(
        storage_root: StorageRootId,
        config: &SchedulerConfig,
        profile: StorageProfile,
        io_config: StorageIoConfig,
        topology: Option<StorageTopology>,
        shared_resources: Option<Arc<SharedSchedulerResources>>,
    ) -> Self {
        let ssd = matches!(profile, StorageProfile::Ssd | StorageProfile::Nvme);
        let recheck_limit = if config.recheck_concurrency > 0 {
            config.recheck_concurrency
        } else if ssd {
            IoClass::Recheck.ssd_concurrency()
        } else {
            IoClass::Recheck.hdd_concurrency()
        };
        let peer_read_limit = if config.peer_read_concurrency > 0 {
            config.peer_read_concurrency
        } else if ssd {
            IoClass::PeerRead.ssd_concurrency()
        } else {
            IoClass::PeerRead.hdd_concurrency()
        };
        let mv = if ssd {
            IoClass::MoveCopy.ssd_concurrency()
        } else {
            IoClass::MoveCopy.hdd_concurrency()
        };
        let pw = if ssd {
            IoClass::PeerWrite.ssd_concurrency()
        } else {
            IoClass::PeerWrite.hdd_concurrency()
        };
        let fg = if ssd {
            IoClass::Foreground.ssd_concurrency()
        } else {
            IoClass::Foreground.hdd_concurrency()
        };
        let md = if ssd {
            IoClass::Metadata.ssd_concurrency()
        } else {
            IoClass::Metadata.hdd_concurrency()
        };
        let shared_resources =
            shared_resources.unwrap_or_else(|| local_scheduler_resources(&io_config));
        let file_pool = shared_resources.file_pool.clone();
        let io_pool = shared_resources.io_pool.clone();
        let disk_backend = shared_resources.disk_backend.clone();
        let hash_pool = shared_resources.hash_pool.clone();
        let queue_sem = Arc::new(Semaphore::new(
            config.max_queue.min(io_config.io_queue_depth).max(1),
        ));
        let device_id = topology
            .as_ref()
            .and_then(|topology| topology.device_id.as_ref().map(|device| device.0.clone()));
        // Device admission is shared by every mount on the physical device.
        // Per-mount max_queue remains enforced by queue_sem; it must not split
        // or resize the shared device service according to which mount starts
        // first.
        let requested_device_queue_capacity = io_config.io_queue_depth.max(1);
        let device_dispatch_window = disk_backend
            .device_dispatch_window(io_config.io_worker_threads, requested_device_queue_capacity);
        let device_queue = device_queue_for(
            storage_root,
            device_id.as_deref(),
            requested_device_queue_capacity,
            device_dispatch_window,
        );
        let device_queue_capacity = device_queue.capacity;
        let counters = Arc::new(StorageCounters::default());
        let peer_read_elevator_enabled =
            matches!(profile, StorageProfile::Hdd) && io_config.peer_read_elevator_budget_ms > 0;
        let peer_read_elevator_queue_depth = io_config.io_queue_depth.max(peer_read_limit);
        // Read-ahead and urgent uploads bypass the elevator. Create its task
        // and queue only when an eligible read actually uses them, rather
        // than retaining an unused actor for every HDD storage scheduler.
        let peer_read_elevator = Arc::new(Mutex::new(None));
        let peer_read_cache = Arc::new(Mutex::new(PeerReadCache::default()));
        let peer_read_cache_manager = Arc::clone(&GLOBAL_PEER_READ_CACHE_MANAGER);
        if let Some(resources) = config.resources.as_ref() {
            let manager = Arc::clone(&peer_read_cache_manager);
            resources.register_reclaimer(MemoryClass::PeerBuffer, "rt-storage.peer-read-cache", {
                let reclaim_scope_id = resources.reclaim_scope_id();
                move |needed| {
                    let cache_reclaimed = manager.reclaim_for_scope(needed, reclaim_scope_id);
                    let frame_reclaimed = try_global_frame_pool()
                        .map(|pool| pool.trim_retained(needed))
                        .unwrap_or(0);
                    cache_reclaimed.max(frame_reclaimed)
                }
            });
            resources.register_reclaimer(
                MemoryClass::StorageFrame,
                "rt-storage.frame-pool",
                |needed| {
                    try_global_frame_pool()
                        .map(|pool| pool.trim_retained(needed))
                        .unwrap_or(0)
                },
            );
        }

        MountScheduler {
            storage_root,
            recheck_sem: Arc::new(Semaphore::new(recheck_limit)),
            move_copy_sem: Arc::new(Semaphore::new(mv)),
            peer_write_sem: Arc::new(Semaphore::new(pw)),
            peer_read_sem: Arc::new(Semaphore::new(peer_read_limit)),
            foreground_sem: Arc::new(Semaphore::new(fg)),
            metadata_sem: Arc::new(Semaphore::new(md)),
            queue_sem,
            device_queue,
            device_queue_capacity,
            file_pool,
            io_pool,
            disk_backend,
            _shared_resources: shared_resources,
            hash_pool,
            dirty_paths: Arc::new(Mutex::new(HashMap::new())),
            dirty_path_epoch: Arc::new(AtomicU64::new(0)),
            file_sync_fences: Arc::new(FileSyncFenceRegistry::default()),
            write_admission_gate: Arc::new(AsyncRwLock::new(())),
            write_sequence: Arc::new(AtomicU64::new(0)),
            peer_read_cache,
            peer_read_cache_manager,
            peer_read_cache_epoch: Arc::new(AtomicU64::new(0)),
            peer_read_streams: Arc::new(Mutex::new(PeerReadStreams::default())),
            peer_read_elevator,
            peer_read_limit,
            recheck_limit,
            peer_read_elevator_enabled,
            peer_read_elevator_queue_depth,
            device_id,
            profile,
            resources: config.resources.clone(),
            counters,
            io_config,
        }
    }

    pub fn storage_root(&self) -> StorageRootId {
        self.storage_root
    }

    pub fn io_config(&self) -> &StorageIoConfig {
        &self.io_config
    }

    pub fn file_pool_stats(&self) -> FilePoolStats {
        self.file_pool.stats()
    }

    pub fn io_queue_depth(&self) -> usize {
        self.io_pool.queued()
    }

    /// Current shared device-queue occupancy as `(capacity, available)`.
    /// This avoids collecting the full storage statistics snapshot when a
    /// transfer scheduler only needs a cheap congestion signal.
    pub fn device_queue_load(&self) -> (usize, usize) {
        (self.device_queue_capacity, self.device_queue.available())
    }

    pub fn stats(&self) -> StorageIoStats {
        let dirty_files = lock_recover(&self.dirty_paths).len();
        let peer_read_cache = lock_reset_derived_cache(&self.peer_read_cache);
        let peer_read_cache_entries = peer_read_cache.entry_count();
        let peer_read_cache_bytes = peer_read_cache.cached_bytes() as u64;
        drop(peer_read_cache);
        let peer_read_elevator_queued = lock_recover(&self.peer_read_elevator)
            .as_ref()
            .map(PeerReadElevator::queued_len)
            .unwrap_or(0);
        StorageIoStats {
            shared_resource_id: self._shared_resources.id,
            file_catalog_id: self.file_pool.catalog_id(),
            device_id: self.device_id.clone(),
            profile: self.profile.clone(),
            file_pool: self.file_pool.stats(),
            io_queue_depth: self.io_pool.queued(),
            hash_queue_depth: self.hash_pool.queued(),
            device_queue_capacity: self.device_queue_capacity,
            device_queue_available: self.device_queue.available(),
            queued_disk_bytes: self.counters.queued_disk_bytes.load(Ordering::Relaxed),
            queue_full: self.counters.queue_full.load(Ordering::Relaxed),
            dirty_files,
            read_ops_by_class: load_atomic_array(&self.counters.read_ops_by_class),
            write_ops_by_class: load_atomic_array(&self.counters.write_ops_by_class),
            bytes_read_by_class: load_atomic_array(&self.counters.bytes_read_by_class),
            bytes_written_by_class: load_atomic_array(&self.counters.bytes_written_by_class),
            backend_read_ops_by_class: load_atomic_array(&self.counters.backend_read_ops_by_class),
            backend_bytes_read_by_class: load_atomic_array(
                &self.counters.backend_bytes_read_by_class,
            ),
            read_latency_ns_by_class: load_atomic_array(&self.counters.read_latency_ns_by_class),
            write_latency_ns_by_class: load_atomic_array(&self.counters.write_latency_ns_by_class),
            read_latency_buckets: load_atomic_array(&self.counters.read_latency_buckets),
            write_latency_buckets: load_atomic_array(&self.counters.write_latency_buckets),
            sync_latency_buckets: load_atomic_array(&self.counters.sync_latency_buckets),
            hash_latency_buckets: load_atomic_array(&self.counters.hash_latency_buckets),
            sync_latency_ns: self.counters.sync_latency_ns.load(Ordering::Relaxed),
            hash_latency_ns: self.counters.hash_latency_ns.load(Ordering::Relaxed),
            sync_ops: self.counters.sync_ops.load(Ordering::Relaxed),
            hash_ops: self.counters.hash_ops.load(Ordering::Relaxed),
            preallocation_failures: self.counters.preallocation_failures.load(Ordering::Relaxed),
            preallocation_fallbacks: self
                .counters
                .preallocation_fallbacks
                .load(Ordering::Relaxed),
            preallocation_shrink_refused: self
                .counters
                .preallocation_shrink_refused
                .load(Ordering::Relaxed),
            peer_read_cache_entries,
            peer_read_cache_bytes,
            peer_read_cache_global_backing_bytes: self.peer_read_cache_manager.used_bytes(),
            peer_read_cache_global_budget_bytes: self.peer_read_cache_manager.budget_bytes,
            peer_read_cache_hits: self.counters.peer_read_cache_hits.load(Ordering::Relaxed),
            peer_read_cache_misses: self.counters.peer_read_cache_misses.load(Ordering::Relaxed),
            peer_read_cache_evictions: self
                .counters
                .peer_read_cache_evictions
                .load(Ordering::Relaxed),
            peer_read_elevator_enabled: self.peer_read_elevator_enabled,
            peer_read_elevator_queue_depth: self.peer_read_elevator_queue_depth,
            peer_read_elevator_queued,
            peer_read_elevator_queue_full: self
                .counters
                .peer_read_elevator_queue_full
                .load(Ordering::Relaxed),
            peer_read_elevator_batches: self
                .counters
                .peer_read_elevator_batches
                .load(Ordering::Relaxed),
            peer_read_elevator_coalesced_requests: self
                .counters
                .peer_read_elevator_coalesced_requests
                .load(Ordering::Relaxed),
            page_cache_advise_sequential: self
                .counters
                .page_cache_advise_sequential
                .load(Ordering::Relaxed),
            page_cache_advise_willneed: self
                .counters
                .page_cache_advise_willneed
                .load(Ordering::Relaxed),
            page_cache_advise_dontneed: self
                .counters
                .page_cache_advise_dontneed
                .load(Ordering::Relaxed),
            page_cache_advise_failures: self
                .counters
                .page_cache_advise_failures
                .load(Ordering::Relaxed),
            sparse_data_extents: self.counters.sparse_data_extents.load(Ordering::Relaxed),
            sparse_hole_bytes: self.counters.sparse_hole_bytes.load(Ordering::Relaxed),
            sparse_seek_fallbacks: self.counters.sparse_seek_fallbacks.load(Ordering::Relaxed),
        }
    }

    #[instrument(skip(self), fields(class = ?class))]
    pub async fn acquire(&self, class: IoClass) -> Result<OwnedSemaphorePermit, StorageError> {
        let sem = match class {
            IoClass::Recheck => &self.recheck_sem,
            IoClass::MoveCopy => &self.move_copy_sem,
            IoClass::PeerWrite => &self.peer_write_sem,
            IoClass::PeerRead => &self.peer_read_sem,
            IoClass::Foreground => &self.foreground_sem,
            IoClass::Metadata => &self.metadata_sem,
        };
        sem.clone()
            .acquire_owned()
            .await
            .map_err(|_| StorageError::Cancelled)
    }

    pub fn try_acquire(&self, class: IoClass) -> Option<OwnedSemaphorePermit> {
        let sem = match class {
            IoClass::Recheck => &self.recheck_sem,
            IoClass::MoveCopy => &self.move_copy_sem,
            IoClass::PeerWrite => &self.peer_write_sem,
            IoClass::PeerRead => &self.peer_read_sem,
            IoClass::Foreground => &self.foreground_sem,
            IoClass::Metadata => &self.metadata_sem,
        };
        sem.clone().try_acquire_owned().ok()
    }

    /// Configured concurrency budget for the `IoClass::Recheck` lane on this
    /// mount: `SchedulerConfig::recheck_concurrency` when set explicitly, or
    /// the profile default (`IoClass::Recheck::{ssd,hdd}_concurrency`)
    /// otherwise. This is the same limit that sizes `recheck_sem`, i.e. the
    /// number of recheck reads the elevator already allows in flight at
    /// once on this mount.
    ///
    /// Recheck/import/migration verification pipelines that parallelize
    /// across pieces or files should size their in-flight batch to this
    /// value so they share the existing recheck lane budget instead of
    /// introducing a second, independent concurrency limit.
    pub fn recheck_concurrency(&self) -> usize {
        self.recheck_limit
    }

    pub fn available_permits(&self, class: IoClass) -> usize {
        match class {
            IoClass::Recheck => self.recheck_sem.available_permits(),
            IoClass::MoveCopy => self.move_copy_sem.available_permits(),
            IoClass::PeerWrite => self.peer_write_sem.available_permits(),
            IoClass::PeerRead => self.peer_read_sem.available_permits(),
            IoClass::Foreground => self.foreground_sem.available_permits(),
            IoClass::Metadata => self.metadata_sem.available_permits(),
        }
    }

    async fn run_queued_blocking<T, F>(
        &self,
        class: IoClass,
        queued_bytes: u64,
        f: F,
    ) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, StorageError> + Send + 'static,
    {
        let mut submission = self.reserve_mount_submission(queued_bytes)?;
        submission.start_device_operation(self.acquire_device_operation(class).await?);
        submission.trace_dispatch("storage_blocking", class, 1);
        let result = self
            .io_pool
            .run(move || {
                let _submission = submission;
                f()
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Run disk-bound work owned by another subsystem through this mount's
    /// bounded queue and the shared physical-device dispatcher.
    ///
    /// The caller supplies the retained byte charge and an operation class.
    /// Both the mount and device permits, plus the byte charge, remain owned
    /// by the blocking operation until it returns, even if this future is
    /// cancelled after dispatch. Use this handoff for storage work that does
    /// not naturally enter through `read_at`, `write_at`, or `sync_data`.
    ///
    /// The closure runs on the configured storage blocking pool. It must not
    /// perform asynchronous work or wait for another operation queued on the
    /// same blocking pool.
    pub async fn run_device_operation<T, F>(
        &self,
        class: IoClass,
        queued_bytes: u64,
        operation: F,
    ) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, StorageError> + Send + 'static,
    {
        self.run_queued_blocking(class, queued_bytes, operation)
            .await
    }

    /// Run disk-bound work that touches paths on this mount and any additional
    /// physical devices. Every device queue is admitted in stable key order,
    /// so concurrent copies in opposite directions cannot hold one device
    /// while waiting for the other in the reverse order.
    ///
    /// The current scheduler remains the mount and byte-accounting owner.
    /// Additional paths contribute physical-device admission; their path
    /// mounts do not acquire a second queue-byte charge for the same working
    /// buffer.
    pub async fn run_device_operation_with_paths<T, F>(
        &self,
        class: IoClass,
        queued_bytes: u64,
        additional_paths: &[PathBuf],
        operation: F,
    ) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, StorageError> + Send + 'static,
    {
        let queues = self.device_queue_set_for_paths(additional_paths);
        self.run_device_operation_with_queue_set(class, queued_bytes, &queues, operation)
            .await
    }

    pub(crate) fn device_queue_set_for_paths(&self, paths: &[PathBuf]) -> DeviceQueueSet {
        let mut queues = Vec::with_capacity(paths.len());
        for path in paths {
            let topology = detect_storage_topology(path);
            let device_id = topology.device_id.map(|device| device.0);
            let device_key = device_queue_key(self.storage_root, device_id.as_deref());
            let mount = format!("storage-root-{}", self.storage_root.0);
            let queue = device_queue_for(
                self.storage_root,
                device_id.as_deref(),
                self.device_queue_capacity,
                self.device_queue.active_limit,
            );
            queues.push((device_key, mount, queue));
        }
        queues.sort_by(|left, right| left.0.cmp(&right.0));
        let mut seen = HashSet::with_capacity(queues.len());
        queues.retain(|(_, _, queue)| seen.insert(Arc::as_ptr(queue) as usize));
        DeviceQueueSet {
            additional_queues: queues,
        }
    }

    pub(crate) async fn run_device_operation_with_queue_set<T, F>(
        &self,
        class: IoClass,
        queued_bytes: u64,
        queue_set: &DeviceQueueSet,
        operation: F,
    ) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, StorageError> + Send + 'static,
    {
        self.run_device_operation_with_queues(
            class,
            queued_bytes,
            queue_set.additional_queues.clone(),
            operation,
        )
        .await
    }

    async fn run_device_operation_with_queues<T, F>(
        &self,
        class: IoClass,
        queued_bytes: u64,
        additional_queues: Vec<(String, String, Arc<DeviceQueue>)>,
        operation: F,
    ) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, StorageError> + Send + 'static,
    {
        let mut queues = additional_queues;
        let mount = format!("storage-root-{}", self.storage_root.0);
        queues.push((
            device_queue_key(self.storage_root, self.device_id.as_deref()),
            mount,
            Arc::clone(&self.device_queue),
        ));
        queues.sort_by(|left, right| left.0.cmp(&right.0));
        let mut seen = HashSet::with_capacity(queues.len());
        queues.retain(|(_, _, queue)| seen.insert(Arc::as_ptr(queue) as usize));

        let mut submission = self.reserve_mount_submission(queued_bytes)?;
        let mut permits = Vec::with_capacity(queues.len());
        for (_, mount, queue) in queues {
            match queue.acquire(class, mount).await {
                Ok(permit) => permits.push(permit),
                Err(error) => {
                    if matches!(error, StorageError::QueueFull { .. }) {
                        self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
                    }
                    return Err(error);
                }
            }
        }
        let mut permits = permits.into_iter();
        let Some(primary) = permits.next() else {
            return Err(StorageError::Cancelled);
        };
        submission.start_device_operation(primary);
        let additional_permits = permits.collect::<Vec<_>>();
        submission.retain_additional_device_operations(additional_permits);
        submission.trace_dispatch(
            "multi_device_blocking",
            class,
            submission.device_queue_permit_count(),
        );
        let result = self
            .io_pool
            .run(move || {
                let _submission = submission;
                operation()
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn reserve_mount_submission(&self, queued_bytes: u64) -> Result<DiskSubmission, StorageError> {
        self.reserve_mount_submission_with_shared_payload_lease(queued_bytes, None)
    }

    fn reserve_mount_submission_with_shared_payload_lease(
        &self,
        queued_bytes: u64,
        shared_payload_lease: Option<Arc<MemoryLease>>,
    ) -> Result<DiskSubmission, StorageError> {
        let queued_bytes = match shared_payload_lease {
            Some(lease) => Arc::new(QueuedDiskBytes::reserve_shared_payload(
                self.counters.clone(),
                self.resources.as_ref(),
                queued_bytes,
                lease,
                format_args!("storage-root-{}", self.storage_root.0),
            )?),
            None => Arc::new(QueuedDiskBytes::reserve(
                self.counters.clone(),
                self.resources.as_ref(),
                queued_bytes,
                format_args!("storage-root-{}", self.storage_root.0),
            )?),
        };
        self.reserve_mount_submission_with_queued_bytes(queued_bytes)
    }

    fn reserve_mount_submission_with_queued_bytes(
        &self,
        queued_bytes: Arc<QueuedDiskBytes>,
    ) -> Result<DiskSubmission, StorageError> {
        self.try_reserve_mount_submission_with_queued_bytes(queued_bytes)?
            .ok_or_else(|| {
                self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
                StorageError::QueueFull {
                    mount: format!("storage-root-{}", self.storage_root.0),
                }
            })
    }

    fn try_reserve_mount_submission_with_queued_bytes(
        &self,
        queued_bytes: Arc<QueuedDiskBytes>,
    ) -> Result<Option<DiskSubmission>, StorageError> {
        let queue = match self.queue_sem.clone().try_acquire_owned() {
            Ok(queue) => queue,
            Err(TryAcquireError::NoPermits) => return Ok(None),
            Err(TryAcquireError::Closed) => return Err(StorageError::Cancelled),
        };
        let storage_operation_id = allocate_storage_operation_id().ok_or_else(|| {
            StorageError::io(
                "storage operation identity",
                io::Error::other("process-wide storage operation identity space exhausted"),
            )
        })?;
        let resource_credit = queued_bytes.resource_credit();
        Ok(Some(DiskSubmission {
            operation: StorageOperationRecord {
                storage_operation_id: Some(storage_operation_id),
                resource_credit,
                ..StorageOperationRecord::default()
            },
            _queued_bytes: queued_bytes,
            _queue: queue,
            _device_operation: None,
            _additional_device_operations: None,
            _file_owner: None,
            _payload_owner: None,
        }))
    }

    async fn acquire_device_operation(
        &self,
        class: IoClass,
    ) -> Result<DeviceOperationPermit, StorageError> {
        self.acquire_device_operation_with_priority(class, IoRequestPriority::normal())
            .await
    }

    async fn acquire_device_operation_with_priority(
        &self,
        class: IoClass,
        priority: IoRequestPriority,
    ) -> Result<DeviceOperationPermit, StorageError> {
        let device_operation = self
            .device_queue
            .acquire_with_priority(
                class,
                format!("storage-root-{}", self.storage_root.0),
                priority,
            )
            .await;
        match device_operation {
            Ok(permit) => Ok(permit),
            Err(error) => {
                if matches!(error, StorageError::QueueFull { .. }) {
                    self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
                }
                Err(error)
            }
        }
    }

    fn peer_read_elevator(&self) -> Option<PeerReadElevator> {
        if !self.peer_read_elevator_enabled {
            return None;
        }
        let mut elevator = lock_recover(&self.peer_read_elevator);
        if elevator.is_none() && tokio::runtime::Handle::try_current().is_ok() {
            *elevator = Some(PeerReadElevator::spawn(
                self.storage_root,
                self.peer_read_elevator_queue_depth,
                self.peer_read_limit,
                Duration::from_millis(self.io_config.peer_read_elevator_budget_ms),
                self.file_pool.clone(),
                self.io_pool.clone(),
                self.disk_backend.clone(),
                self.queue_sem.clone(),
                self.device_queue.clone(),
                self.counters.clone(),
                self.resources.clone(),
            ));
        }
        elevator.clone()
    }

    pub async fn read_at(
        &self,
        class: IoClass,
        path: &Path,
        offset: u64,
        len: usize,
    ) -> Result<bytes::Bytes, StorageError> {
        self.read_owned_at(class, path, offset, len)
            .await
            .map(StorageRead::into_bytes)
    }

    pub async fn read_owned_at(
        &self,
        class: IoClass,
        path: &Path,
        offset: u64,
        len: usize,
    ) -> Result<StorageRead, StorageError> {
        self.read_owned_at_with_priority(class, path, offset, len, IoRequestPriority::normal())
            .await
    }

    pub async fn read_owned_at_with_priority(
        &self,
        class: IoClass,
        path: &Path,
        offset: u64,
        len: usize,
        priority: IoRequestPriority,
    ) -> Result<StorageRead, StorageError> {
        checked_file_range_end(path, offset, len as u64)?;
        let _permit = self.acquire(class).await?;
        let pool = self.file_pool.clone();
        let disk_backend = self.disk_backend.clone();
        let counters = self.counters.clone();
        let peer_read_cache = self.peer_read_cache.clone();
        let peer_read_cache_epoch = self.peer_read_cache_epoch.clone();
        let peer_read_streams = self.peer_read_streams.clone();
        let path = path.to_path_buf();
        let configured_readahead_bytes = self.io_config.peer_read_readahead_bytes;
        let sequential_only = self.io_config.peer_read_readahead_sequential_only;
        let readahead_cache_entries = self.io_config.peer_read_cache_entries;
        let readahead_cache_bytes = self.io_config.peer_read_cache_bytes;
        let readahead_enabled = readahead_cache_entries > 0 && readahead_cache_bytes > 0;
        let readahead_bytes = if readahead_enabled {
            configured_readahead_bytes.min(readahead_cache_bytes)
        } else {
            0
        };
        let started = Instant::now();
        let cache_generation = (class == IoClass::PeerRead && readahead_bytes > len)
            .then(|| peer_read_cache_epoch.load(Ordering::Acquire));
        if class == IoClass::PeerRead && readahead_bytes <= len && !priority.is_urgent() {
            if let Some(elevator) = self.peer_read_elevator() {
                let bytes = elevator.read(path.to_path_buf(), offset, len).await?;
                counters.read_ops_by_class[class_index(class)].fetch_add(1, Ordering::Relaxed);
                counters.bytes_read_by_class[class_index(class)]
                    .fetch_add(bytes.len() as u64, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                counters.read_latency_ns_by_class[class_index(class)]
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.read_latency_buckets, latency_ns);
                return Ok(StorageRead::Bytes(bytes));
            }
        }

        if disk_backend.uses_pread_workers() {
            let mut submission = self.reserve_mount_submission(len as u64)?;
            if class != IoClass::PeerRead || readahead_bytes <= len {
                // Non-cacheable operations keep direct admission, avoiding
                // a preparation round-trip when the device is contended.
                submission.start_device_operation(
                    self.acquire_device_operation_with_priority(class, priority)
                        .await?,
                );
            }
            let prepared = PreparedPread {
                device_queue: self.device_queue.clone(),
                file_pool: pool,
                peer_read_cache,
                peer_read_cache_manager: self.peer_read_cache_manager.clone(),
                peer_read_cache_epoch,
                peer_read_streams,
                counters: counters.clone(),
                resources: self.resources.clone(),
                storage_root: self.storage_root,
                cache_generation,
                readahead_bytes,
                sequential_only,
                readahead_cache_entries,
                readahead_cache_bytes,
                submission: Some(submission),
            };
            let failed_counter = counters.clone();
            let preparation = disk_backend
                .run_prepared_pread(move || prepared.execute(class, path, offset, len, started))
                .await;
            let result = match preparation {
                Ok(PreadPreparation::Complete(read)) => Ok(read),
                Ok(PreadPreparation::AwaitDevice(mut pending)) => {
                    let permit = self
                        .acquire_device_operation_with_priority(class, priority)
                        .await?;
                    pending
                        .prepared
                        .submission
                        .as_mut()
                        .expect("deferred pread owns its mount admission")
                        .start_device_operation(permit);
                    disk_backend
                        .run_prepared_pread(move || pending.execute())
                        .await
                }
                Err(error) => Err(error),
            };
            if matches!(
                &result,
                Err(StorageError::QueueFull { mount })
                    if mount == "disk-backend" || mount == "scheduler-read-frame"
            ) {
                failed_counter.queue_full.fetch_add(1, Ordering::Relaxed);
            }
            return result;
        }

        let submission = self.reserve_mount_submission(len as u64)?;
        let preparation_counters = counters.clone();
        let preparation_peer_read_cache = peer_read_cache.clone();
        let preparation = self
            .io_pool
            .run(move || {
                let prepared = (|| -> Result<ReadPreparation, StorageError> {
                    let key = normalized_key(&path);
                    let path_str = key.display().to_string();
                    let file = pool.get_or_open(&key, OpenMode::Read, false)?;
                    if class == IoClass::PeerRead && readahead_bytes > len {
                        if let Some(bytes) = peer_read_cache_hit(
                            &preparation_peer_read_cache,
                            &key,
                            file.identity(),
                            offset,
                            len,
                        ) {
                            lock_recover(&peer_read_streams).observe(
                                &key,
                                offset,
                                len,
                                readahead_bytes,
                                true,
                            );
                            preparation_counters
                                .peer_read_cache_hits
                                .fetch_add(1, Ordering::Relaxed);
                            preparation_counters.read_ops_by_class[class_index(class)]
                                .fetch_add(1, Ordering::Relaxed);
                            preparation_counters.bytes_read_by_class[class_index(class)]
                                .fetch_add(len as u64, Ordering::Relaxed);
                            let latency_ns = latency_ns_since(started);
                            preparation_counters.read_latency_ns_by_class[class_index(class)]
                                .fetch_add(latency_ns, Ordering::Relaxed);
                            record_latency_bucket(
                                &preparation_counters.read_latency_buckets,
                                latency_ns,
                            );
                            return Ok(ReadPreparation::CacheHit {
                                key,
                                identity: file.identity(),
                                bytes,
                            });
                        }
                        preparation_counters
                            .peer_read_cache_misses
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    let read_len = if class == IoClass::PeerRead && readahead_bytes > len {
                        let file_len = file
                            .metadata()
                            .map_err(|e| StorageError::io(&path_str, e))?
                            .len();
                        // Expand only a stream that continues the previous read of
                        // this file. An isolated 16 KiB request stays 16 KiB
                        // instead of fetching 512 KiB it will never reuse.
                        let (sequential, recommended_bytes) = lock_recover(&peer_read_streams)
                            .observe(&key, offset, len, readahead_bytes, false);
                        if offset >= file_len || (sequential_only && !sequential) {
                            len
                        } else {
                            (if sequential_only {
                                recommended_bytes
                            } else {
                                readahead_bytes
                            })
                            .min(file_len.saturating_sub(offset) as usize)
                            .max(len)
                        }
                    } else {
                        len
                    };
                    advise_for_read_class(&file, class, offset, read_len, &preparation_counters);
                    Ok(ReadPreparation::BackendRead {
                        key,
                        file,
                        read_len,
                    })
                })();
                prepared.map(|prepared| (prepared, submission))
            })
            .await;
        if matches!(preparation, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        let (preparation, mut submission) = preparation?;
        match preparation {
            ReadPreparation::CacheHit {
                key,
                identity,
                bytes,
            } => {
                submission.bind_file_identity(identity);
                submission.operation.bind_byte_range(offset, len as u64);
                verify_path_identity(&key, identity, "payload_read")?;
                let _submission = submission;
                Ok(StorageRead::Bytes(bytes))
            }
            ReadPreparation::BackendRead {
                key,
                file,
                read_len,
            } => {
                submission.retain_file(Arc::clone(&file));
                submission
                    .operation
                    .bind_byte_range(offset, read_len as u64);
                let requested_charge = queued_disk_charge(len as u64);
                let read_buffer_charge = queued_disk_charge(read_len as u64);
                let read_buffer_queued_bytes = read_buffer_charge
                    .checked_sub(requested_charge)
                    .filter(|bytes| *bytes > 0)
                    .map(|bytes| {
                        QueuedDiskBytes::reserve_charged(
                            counters.clone(),
                            self.resources.as_ref(),
                            bytes,
                            format_args!("storage-root-{}-readahead", self.storage_root.0),
                        )
                    })
                    .transpose()?;
                if let Some(extra_charge) = read_buffer_queued_bytes.as_ref() {
                    submission
                        .operation
                        .resource_credit
                        .include(extra_charge.resource_credit());
                }
                submission.start_device_operation(
                    self.acquire_device_operation_with_priority(class, priority)
                        .await?,
                );
                let frame = global_frame_pool()
                    .try_acquire_for_overwrite(read_len)
                    .ok_or_else(|| StorageError::QueueFull {
                        mount: "scheduler-read-frame".to_string(),
                    })?;
                submission.record_frame_pool_charge(&frame);
                submission.trace_dispatch("positioned_read", class, 1);
                let (ownership, backend_owner) =
                    backend_operation_ownership((), (submission, read_buffer_queued_bytes));
                let completion =
                    disk_backend.pread_leased(file.clone(), frame, offset, backend_owner);
                let frame =
                    match await_backend_io_with_fence(&key, completion, ownership, Some(read_len))
                        .await
                    {
                        Ok((frame, (), operation_owner)) => {
                            drop(operation_owner);
                            frame
                        }
                        Err(error) => {
                            if matches!(error, StorageError::QueueFull { .. }) {
                                counters.queue_full.fetch_add(1, Ordering::Relaxed);
                            }
                            return Err(error);
                        }
                    };
                // A path can be atomically replaced while the backend read
                // is in flight. The submitted descriptor remains valid, but
                // its bytes must not be returned or cached as the current
                // path's payload after that replacement.
                current_file_identity(&key, file.as_ref(), "payload_read")?;
                if class == IoClass::PeerRead {
                    lock_reset_derived_cache(&peer_read_cache).consume(
                        &key,
                        file.identity(),
                        offset,
                        len,
                    );
                }
                counters.read_ops_by_class[class_index(class)].fetch_add(1, Ordering::Relaxed);
                counters.bytes_read_by_class[class_index(class)]
                    .fetch_add(len as u64, Ordering::Relaxed);
                counters.backend_read_ops_by_class[class_index(class)]
                    .fetch_add(1, Ordering::Relaxed);
                counters.backend_bytes_read_by_class[class_index(class)]
                    .fetch_add(read_len as u64, Ordering::Relaxed);
                advise_after_read_class(&file, class, offset, read_len, &counters);
                let latency_ns = latency_ns_since(started);
                counters.read_latency_ns_by_class[class_index(class)]
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.read_latency_buckets, latency_ns);
                if class == IoClass::PeerRead && read_len > len {
                    let backing_bytes = frame.charged_bytes();
                    let bytes = frame.into_bytes();
                    let exact = bytes.slice(..len);
                    let identity = file.identity();
                    let cached_exact = self.peer_read_cache_store(PeerReadCacheInsert {
                        generation: cache_generation.expect("peer-read cache generation is set"),
                        key,
                        identity,
                        source: file.source_weak(),
                        offset,
                        data: bytes,
                        max_entries: self.io_config.peer_read_cache_entries,
                        max_bytes: self.io_config.peer_read_cache_bytes,
                        backing_bytes,
                        result_len: len,
                    });
                    Ok(StorageRead::Bytes(cached_exact.unwrap_or(exact)))
                } else {
                    Ok(StorageRead::Frame(frame))
                }
            }
        }
    }

    fn peer_read_cache_store(&self, insert: PeerReadCacheInsert) -> Option<bytes::Bytes> {
        peer_read_cache_store_parts(
            &self.peer_read_cache,
            &self.peer_read_cache_manager,
            &self.peer_read_cache_epoch,
            self.resources.as_ref(),
            &self.counters,
            PeerReadCacheInsert {
                max_entries: self.io_config.peer_read_cache_entries,
                max_bytes: self.io_config.peer_read_cache_bytes,
                ..insert
            },
        )
    }

    pub async fn write_at(
        &self,
        class: IoClass,
        path: &Path,
        offset: u64,
        data: bytes::Bytes,
        create: bool,
    ) -> Result<(), StorageError> {
        self.write_at_inner(class, path, offset, data, create, None)
            .await
    }

    /// Write a view of an allocation already covered by `payload_lease`.
    /// The lease remains owned by the submitted operation through backend
    /// completion without charging the shared payload a second time.
    pub async fn write_at_with_shared_payload_lease(
        &self,
        class: IoClass,
        path: &Path,
        offset: u64,
        data: bytes::Bytes,
        create: bool,
        payload_lease: Arc<MemoryLease>,
    ) -> Result<(), StorageError> {
        self.write_at_inner(class, path, offset, data, create, Some(payload_lease))
            .await
    }

    async fn write_at_inner(
        &self,
        class: IoClass,
        path: &Path,
        offset: u64,
        data: bytes::Bytes,
        create: bool,
        shared_payload_lease: Option<Arc<MemoryLease>>,
    ) -> Result<(), StorageError> {
        checked_file_range_end(path, offset, data.len() as u64)?;
        let write_admission_guard = Arc::clone(&self.write_admission_gate).read_owned().await;
        let key = normalized_key(path);
        let strict = self.io_config.durability_mode == DurabilityMode::Strict;
        let file_sync_fence = self.file_sync_fences.for_path(&key);
        let file_write_guard = if strict {
            FileFenceGuard::Exclusive {
                _guard: Arc::clone(&file_sync_fence.gate).write_owned().await,
            }
        } else {
            FileFenceGuard::Shared {
                _guard: Arc::clone(&file_sync_fence.gate).read_owned().await,
            }
        };
        let _permit = self.acquire(class).await?;
        let pool = self.file_pool.clone();
        let disk_backend = self.disk_backend.clone();
        let dirty_paths = self.dirty_paths.clone();
        let dirty_path_epoch = self.dirty_path_epoch.clone();
        let peer_read_cache = self.peer_read_cache.clone();
        let peer_read_cache_epoch = self.peer_read_cache_epoch.clone();
        let counters = self.counters.clone();
        let started = Instant::now();
        let submission = self.reserve_mount_submission_with_shared_payload_lease(
            data.len() as u64,
            shared_payload_lease,
        )?;
        // Invalidate both before and after the write. A concurrent read that
        // started before the write may finish with old bytes after the first
        // invalidation; the epoch check in `peer_read_cache_store` prevents it
        // from repopulating the cache after the second one.
        peer_read_cache_invalidate(&peer_read_cache, &peer_read_cache_epoch, &key);
        let (file, mut submission) = match self
            .io_pool
            .run({
                let pool = pool.clone();
                let key = key.clone();
                move || {
                    pool.get_or_open(&key, OpenMode::Write, create)
                        .map(|file| (file, submission))
                }
            })
            .await
        {
            Ok(file_and_submission) => file_and_submission,
            Err(error) => {
                if matches!(error, StorageError::QueueFull { .. }) {
                    counters.queue_full.fetch_add(1, Ordering::Relaxed);
                }
                return Err(error);
            }
        };
        let file_identity = current_file_identity(&key, file.as_ref(), "payload_write")?;
        let written = data.len();
        submission.retain_file(Arc::clone(&file));
        submission.retain_payload(data.clone());
        submission.operation.bind_byte_range(offset, written as u64);
        submission.start_device_operation(self.acquire_device_operation(class).await?);
        submission.trace_dispatch("positioned_write", class, 1);
        // Record the path as needing a sync when the write is *issued*, not
        // only after it completes. The backend can finish a submitted write
        // after this future is dropped; marking only afterwards would let a
        // cancelled caller leave written bytes that no barrier covers. A
        // failed write costs one redundant fdatasync, which is harmless.
        let write_sequence = file_sync_fence.issue_write_sequence();
        submission.operation.bind_write_sequence(write_sequence);
        let _write_watermark = next_saturating_sequence(&self.write_sequence);
        let dirty_state = mark_dirty_path(
            &dirty_paths,
            &dirty_path_epoch,
            key.clone(),
            file_identity,
            write_sequence,
        );
        submission.bind_dirty_path(dirty_state);
        let (ownership, backend_owner) =
            backend_operation_ownership((file_write_guard, write_admission_guard), submission);
        let completion_cache = peer_read_cache.clone();
        let completion_epoch = peer_read_cache_epoch.clone();
        let completion_key = key.clone();
        let backend_owner = backend_owner.after_completion(move || {
            peer_read_cache_invalidate(&completion_cache, &completion_epoch, &completion_key);
        });
        let completion = disk_backend.pwrite_leased(file.clone(), data, offset, backend_owner);
        let (file_write_guard, admission_guard, mut submission) =
            match await_backend_io_with_fence(&key, completion, ownership, Some(written)).await {
                Ok(((), (fence_guard, admission_guard), submission)) => {
                    (fence_guard, admission_guard, submission)
                }
                Err(error) => {
                    if matches!(error, StorageError::QueueFull { .. }) {
                        counters.queue_full.fetch_add(1, Ordering::Relaxed);
                    }
                    return Err(error);
                }
            };
        current_file_identity(&key, file.as_ref(), "payload_write")?;
        submission.release_payload();
        let submission = if strict {
            let sync_started = Instant::now();
            let strict_sync_guard = match file_write_guard {
                FileFenceGuard::Exclusive { _guard } => _guard,
                FileFenceGuard::Shared { .. } => {
                    unreachable!("strict writes hold the exclusive per-file fence")
                }
            };
            let (ownership, backend_owner) =
                backend_operation_ownership((strict_sync_guard, admission_guard), submission);
            let completion = disk_backend.fdatasync_leased(file.clone(), backend_owner);
            let sync_completion =
                await_backend_io_with_fence(&key, completion, ownership, None).await;
            let (strict_sync_guard, admission_guard, submission) = match sync_completion {
                Ok(((), (fence_guard, admission_guard), submission)) => {
                    (fence_guard, admission_guard, submission)
                }
                Err(error) => {
                    if matches!(error, StorageError::QueueFull { .. }) {
                        counters.queue_full.fetch_add(1, Ordering::Relaxed);
                    }
                    return Err(error);
                }
            };
            current_file_identity(&key, file.as_ref(), "strict_data_sync")?;
            record_sync_completion(&counters, sync_started);
            file_sync_fence.mark_synced(write_sequence);
            clear_dirty_path_if_operation(&dirty_paths, &key, &submission.operation);
            drop(strict_sync_guard);
            drop(admission_guard);
            submission
        } else {
            drop(file_write_guard);
            drop(admission_guard);
            submission
        };
        counters.write_ops_by_class[class_index(class)].fetch_add(1, Ordering::Relaxed);
        counters.bytes_written_by_class[class_index(class)]
            .fetch_add(written as u64, Ordering::Relaxed);
        let latency_ns = latency_ns_since(started);
        counters.write_latency_ns_by_class[class_index(class)]
            .fetch_add(latency_ns, Ordering::Relaxed);
        record_latency_bucket(&counters.write_latency_buckets, latency_ns);
        drop(submission);
        Ok(())
    }

    pub async fn prepare_file(
        &self,
        path: &Path,
        len: u64,
        mode: PreallocationMode,
    ) -> Result<(), StorageError> {
        let write_admission_guard = Arc::clone(&self.write_admission_gate).read_owned().await;
        let pool = self.file_pool.clone();
        let dirty_paths = self.dirty_paths.clone();
        let dirty_path_epoch = self.dirty_path_epoch.clone();
        let write_sequence = Arc::clone(&self.write_sequence);
        let counters = self.counters.clone();
        let path = path.to_path_buf();
        let fence = self.file_sync_fences.for_path(&normalized_key(&path));
        let fence_guard = Arc::clone(&fence.gate).write_owned().await;
        let fence_for_job = Arc::clone(&fence);
        self.run_queued_blocking(IoClass::Foreground, 0, move || {
            let _write_admission_guard = write_admission_guard;
            let _fence_guard = fence_guard;
            let key = normalized_key(&path);
            let path_str = key.display().to_string();
            if let Some(parent) = key.parent() {
                create_dir_all_no_follow(parent)
                    .map_err(|e| StorageError::io(parent.display().to_string(), e))?;
            }
            let file = pool.get_or_open(&key, OpenMode::Write, true)?;
            let identity = current_file_identity(&key, file.as_ref(), "prepare_file")?;
            if mode != PreallocationMode::Off {
                let file_sequence = fence_for_job.issue_write_sequence();
                let _write_watermark = next_saturating_sequence(&write_sequence);
                mark_dirty_path(
                    &dirty_paths,
                    &dirty_path_epoch,
                    key.clone(),
                    identity,
                    file_sequence,
                );
            }
            if mode != PreallocationMode::Off {
                let current_len = file
                    .metadata()
                    .map_err(|e| StorageError::io(&path_str, e))?
                    .len();
                if current_len > len {
                    counters
                        .preallocation_shrink_refused
                        .fetch_add(1, Ordering::Relaxed);
                    tracing::error!(
                        component = "storage",
                        operation = "prepare_file",
                        path = %path_str,
                        current_len,
                        requested_len = len,
                        "refusing to shrink an existing file below its current on-disk size"
                    );
                    return Err(StorageError::RefuseShrink {
                        path: path_str,
                        current_len,
                        requested_len: len,
                    });
                }
            }
            match mode {
                PreallocationMode::Off => {}
                PreallocationMode::Auto => {
                    file.set_len(len)
                        .map_err(|e| StorageError::io(&path_str, e))?;
                }
                PreallocationMode::Sparse => {
                    file.set_len(len)
                        .map_err(|e| StorageError::io(&path_str, e))?;
                }
                PreallocationMode::Full => {
                    if let Err(e) = full_preallocate(&file, len) {
                        counters
                            .preallocation_fallbacks
                            .fetch_add(1, Ordering::Relaxed);
                        tracing::warn!(
                            component = "storage",
                            operation = "preallocate",
                            mode = "full",
                            result = "fallback",
                            error = %e,
                            "full preallocation failed; falling back to sparse length"
                        );
                        file.set_len(len).map_err(|fallback| {
                            counters
                                .preallocation_failures
                                .fetch_add(1, Ordering::Relaxed);
                            StorageError::io(&path_str, fallback)
                        })?;
                    }
                }
            }
            current_file_identity(&key, file.as_ref(), "prepare_file")?;
            Ok(())
        })
        .await
    }

    pub async fn sync_data(&self, path: &Path) -> Result<(), StorageError> {
        let write_admission_guard = Arc::clone(&self.write_admission_gate).read_owned().await;
        let pool = self.file_pool.clone();
        let disk_backend = self.disk_backend.clone();
        let dirty_paths = self.dirty_paths.clone();
        let counters = self.counters.clone();
        let path = path.to_path_buf();
        let key = normalized_key(&path);
        let file_sync_fence = self.file_sync_fences.for_path(&key);
        let fence_guard = Arc::clone(&file_sync_fence.gate).write_owned().await;
        let watermark = file_sync_fence.watermark();
        let started = Instant::now();
        let submission = self.reserve_mount_submission(0)?;
        let expected_state = lock_recover(&dirty_paths).get(&key).copied();
        let (file, mut submission) = match self
            .io_pool
            .run({
                let pool = pool.clone();
                let key = key.clone();
                move || pool.open_for_sync(&key).map(|file| (file, submission))
            })
            .await
        {
            Ok(file_and_submission) => file_and_submission,
            Err(error) => {
                if matches!(error, StorageError::QueueFull { .. }) {
                    counters.queue_full.fetch_add(1, Ordering::Relaxed);
                }
                return Err(error);
            }
        };
        let opened_identity = current_file_identity(&key, file.as_ref(), "data_sync")?;
        ensure_dirty_identity(&key, "data_sync", expected_state, opened_identity)?;
        submission.retain_file(Arc::clone(&file));
        if let Some(expected_state) = expected_state {
            submission.bind_dirty_path(expected_state);
        }
        submission
            .start_device_operation(self.acquire_device_operation(IoClass::Foreground).await?);
        submission.trace_dispatch("file_data_sync", IoClass::Foreground, 1);
        let (ownership, backend_owner) =
            backend_operation_ownership((fence_guard, write_admission_guard), submission);
        let completion = disk_backend.fdatasync_leased(file.clone(), backend_owner);
        let (fence_guard, admission_guard, submission) =
            match await_backend_io_with_fence(&key, completion, ownership, None).await {
                Ok(((), (fence_guard, admission_guard), submission)) => {
                    (fence_guard, admission_guard, submission)
                }
                Err(error) => {
                    if matches!(error, StorageError::QueueFull { .. }) {
                        counters.queue_full.fetch_add(1, Ordering::Relaxed);
                    }
                    return Err(error);
                }
            };
        let completed_identity = current_file_identity(&key, file.as_ref(), "data_sync")?;
        ensure_dirty_identity(&key, "data_sync", expected_state, completed_identity)?;
        record_sync_completion(&counters, started);
        file_sync_fence.mark_synced(watermark);
        if expected_state.is_some() {
            clear_dirty_path_if_operation(&dirty_paths, &key, &submission.operation);
        }
        drop(fence_guard);
        drop(admission_guard);
        drop(submission);
        Ok(())
    }

    pub async fn sync_all_open_files(&self) -> Result<(), StorageError> {
        drop(self.sync_all_open_files_with_evidence().await?);
        Ok(())
    }

    /// Sync every dirty or active file after closing scheduler-wide write
    /// admission. The returned opaque evidence can be attached to durable
    /// resume state to prove the payload barrier completed. The barrier runs
    /// in an owned task so dropping the caller's future cannot stop a partial
    /// checkpoint and admit new writes before the complete path snapshot has
    /// been synced.
    pub async fn sync_all_open_files_with_evidence(
        &self,
    ) -> Result<DurabilityBarrierEvidence, StorageError> {
        let scheduler = self.clone();
        tokio::spawn(async move { scheduler.sync_all_open_files_with_evidence_inner().await })
            .await
            .map_err(|_| StorageError::Cancelled)?
    }

    async fn sync_all_open_files_with_evidence_inner(
        &self,
    ) -> Result<DurabilityBarrierEvidence, StorageError> {
        let dirty_paths = self.dirty_paths.clone();
        let counters = self.counters.clone();
        let started = Instant::now();
        let mut submission = Some(self.reserve_mount_submission(0)?);
        let barrier_queued_bytes = Arc::clone(
            &submission
                .as_ref()
                .expect("checkpoint admission is reserved")
                ._queued_bytes,
        );
        let write_admission_guard = Arc::clone(&self.write_admission_gate).write_owned().await;
        let mut paths_to_sync: BTreeSet<PathBuf> = {
            let dirty = lock_recover(&dirty_paths);
            dirty.keys().cloned().collect()
        };
        paths_to_sync.extend(self.file_sync_fences.active_paths());
        // Sync exactly the files this scheduler wrote. The descriptor pool is
        // shared across torrents, so enumerating its writable files here used
        // to make one torrent's checkpoint fsync every other torrent's (and
        // clean) files, serially and possibly across devices. Include active
        // fences as well as dirty paths so a write still opening its file is
        // ordered before this barrier even though it has not marked dirty yet.
        // Resolve each path once and collect ready barriers by physical
        // device. A batch is bounded by the device dispatch window, shared
        // handle-cache capacity, and mount queue. File handles are still
        // opened before device admission, and each path keeps its own fence
        // and dirty generation. This keeps descriptor use bounded while
        // avoiding a device-queue round trip for every path in a large
        // checkpoint.
        let mut device_groups: BTreeMap<String, (String, Arc<DeviceQueue>, Vec<PathBuf>)> =
            BTreeMap::new();
        let scheduler_device_key = device_queue_key(self.storage_root, self.device_id.as_deref());
        for path in paths_to_sync {
            let key = normalized_key(&path);
            let topology = detect_storage_topology(&key);
            let device_id = topology.device_id.map(|device| device.0);
            let device_key = device_queue_key(self.storage_root, device_id.as_deref());
            let mount = format!("storage-root-{}", self.storage_root.0);
            let queue = if device_key == scheduler_device_key {
                Arc::clone(&self.device_queue)
            } else {
                device_queue_for(
                    self.storage_root,
                    device_id.as_deref(),
                    self.device_queue_capacity,
                    self.device_queue.active_limit,
                )
            };
            let group = device_groups
                .entry(device_key)
                .or_insert_with(|| (mount, queue, Vec::new()));
            group.2.push(key);
        }

        let mut synced_file_count = 0u64;
        let mut reusable_submissions = Vec::with_capacity(1);
        if let Some(submission) = submission.take() {
            reusable_submissions.push(submission);
        }
        for (_, (mount, queue, paths)) in device_groups {
            let mut path_index = 0;
            while path_index < paths.len() {
                let cache_capacity = self.file_pool.stats().capacity.max(1);
                let in_flight_mount_capacity = self
                    .queue_sem
                    .available_permits()
                    .saturating_add(reusable_submissions.len())
                    .max(1);
                let batch_limit = queue
                    .active_limit
                    .min(cache_capacity)
                    .min(self.io_config.io_queue_depth.max(1))
                    .min(in_flight_mount_capacity)
                    .max(1);
                let mut batch = Vec::with_capacity(batch_limit);
                while batch.len() < batch_limit && path_index < paths.len() {
                    let path = paths[path_index].clone();
                    let current_submission = match reusable_submissions.pop() {
                        Some(submission) => submission,
                        None => match self.try_reserve_mount_submission_with_queued_bytes(
                            Arc::clone(&barrier_queued_bytes),
                        )? {
                            Some(submission) => submission,
                            None if !batch.is_empty() => break,
                            None => {
                                self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
                                return Err(StorageError::QueueFull {
                                    mount: format!("storage-root-{}", self.storage_root.0),
                                });
                            }
                        },
                    };
                    batch.push((path, current_submission));
                    path_index += 1;
                }

                let outcomes = join_all(batch.into_iter().map(|(path, submission)| {
                    self.sync_checkpoint_file(path, mount.clone(), Arc::clone(&queue), submission)
                }))
                .await;
                let mut first_error = None;
                for outcome in outcomes {
                    match outcome {
                        Ok(submission) => {
                            synced_file_count = synced_file_count.saturating_add(1);
                            reusable_submissions.push(submission);
                        }
                        Err(error) if first_error.is_none() => first_error = Some(error),
                        Err(_) => {}
                    }
                }
                if let Some(error) = first_error {
                    return Err(error);
                }
            }
        }
        record_sync_completion(&counters, started);
        let write_sequence_watermark = self.write_sequence.load(Ordering::Acquire);
        drop(reusable_submissions);
        drop(submission);
        drop(barrier_queued_bytes);
        Ok(DurabilityBarrierEvidence {
            synced_file_count,
            write_sequence_watermark,
            _write_admission_guard: write_admission_guard,
        })
    }

    async fn sync_checkpoint_file(
        &self,
        key: PathBuf,
        mount: String,
        queue: Arc<DeviceQueue>,
        mut submission: DiskSubmission,
    ) -> Result<DiskSubmission, StorageError> {
        submission.reset_for_next_checkpoint_path()?;
        let file_sync_fence = self.file_sync_fences.for_path(&key);
        let fence_guard = Arc::clone(&file_sync_fence.gate).write_owned().await;
        let watermark = file_sync_fence.watermark();
        let expected_state = lock_recover(&self.dirty_paths).get(&key).copied();
        let (file, mut submission) = match self
            .io_pool
            .run({
                let pool = Arc::clone(&self.file_pool);
                let key = key.clone();
                move || pool.open_for_sync(&key).map(|file| (file, submission))
            })
            .await
        {
            Ok(file_and_submission) => file_and_submission,
            Err(error) => {
                if matches!(error, StorageError::QueueFull { .. }) {
                    self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
                }
                return Err(error);
            }
        };
        let opened_identity = current_file_identity(&key, file.as_ref(), "checkpoint_sync")?;
        ensure_dirty_identity(&key, "checkpoint_sync", expected_state, opened_identity)?;
        submission.retain_file(Arc::clone(&file));
        if let Some(expected_state) = expected_state {
            submission.bind_dirty_path(expected_state);
        }
        let device_operation = queue.acquire(IoClass::Foreground, mount).await;
        let device_operation = match device_operation {
            Ok(permit) => permit,
            Err(error) => {
                if matches!(error, StorageError::QueueFull { .. }) {
                    self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
                }
                return Err(error);
            }
        };
        submission.start_device_operation(device_operation);
        submission.trace_dispatch("checkpoint_file_sync", IoClass::Foreground, 1);
        let (ownership, backend_owner) = backend_operation_ownership(fence_guard, submission);
        let completion = self
            .disk_backend
            .fdatasync_leased(file.clone(), backend_owner);
        let (fence_guard, mut submission) =
            match await_backend_io_with_fence(&key, completion, ownership, None).await {
                Ok(((), fence_guard, submission)) => (fence_guard, submission),
                Err(error) => {
                    if matches!(error, StorageError::QueueFull { .. }) {
                        self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
                    }
                    return Err(error);
                }
            };
        let completed_identity = current_file_identity(&key, file.as_ref(), "checkpoint_sync")?;
        ensure_dirty_identity(&key, "checkpoint_sync", expected_state, completed_identity)?;
        file_sync_fence.mark_synced(watermark);
        if expected_state.is_some() {
            clear_dirty_path_if_operation(&self.dirty_paths, &key, &submission.operation);
        }
        drop(fence_guard);
        submission.finish_device_operation();
        Ok(submission)
    }

    /// Whether this platform has a page-cache drop primitive at all
    /// (`posix_fadvise(DONTNEED)` on Linux and FreeBSD).
    pub const fn page_cache_drop_supported() -> bool {
        cfg!(any(target_os = "linux", target_os = "freebsd"))
    }

    /// Best-effort request that the OS drop cached pages for `path`, so the
    /// next read of it is served by the device instead of the page cache. Used
    /// after a durability barrier to make a verification pass read what is on
    /// disk. Returns whether the platform accepted the hint; platforms with no
    /// such primitive return `false` and callers must treat the read-back as
    /// possibly cache-served.
    pub async fn drop_page_cache(&self, path: &Path) -> bool {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        {
            let pool = self.file_pool.clone();
            let key = normalized_key(path);
            let counters = self.counters.clone();
            let result = self
                .run_queued_blocking(IoClass::Recheck, 0, move || {
                    let file = pool.get_or_open(&key, OpenMode::Read, false)?;
                    Ok(advise_page_cache(&file, 0, 0, PageCacheAdvice::DontNeed))
                })
                .await;
            match result {
                Ok(true) => {
                    counters
                        .page_cache_advise_dontneed
                        .fetch_add(1, Ordering::Relaxed);
                    true
                }
                _ => {
                    counters
                        .page_cache_advise_failures
                        .fetch_add(1, Ordering::Relaxed);
                    false
                }
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        {
            let _ = path;
            false
        }
    }

    pub async fn data_extents(
        &self,
        path: &Path,
        offset: u64,
        len: u64,
    ) -> Result<Vec<DataExtent>, StorageError> {
        self.data_extents_page(path, offset, len, usize::MAX)
            .await
            .map(|page| page.extents)
    }

    /// Scan a range for at most `max_extents` allocated extents. Callers can
    /// continue from `scanned_to` without retaining an unbounded extent list
    /// for a highly fragmented file.
    pub async fn data_extents_page(
        &self,
        path: &Path,
        offset: u64,
        len: u64,
        max_extents: usize,
    ) -> Result<DataExtentPage, StorageError> {
        if len == 0 {
            return Ok(DataExtentPage {
                extents: Vec::new(),
                scanned_to: offset,
                complete: true,
                used_fallback: false,
            });
        }
        checked_file_range_end(path, offset, len)?;
        let pool = self.file_pool.clone();
        let counters = self.counters.clone();
        let path = path.to_path_buf();
        let max_extents = max_extents.max(1);
        self.run_queued_blocking(IoClass::Recheck, 0, move || {
            let key = normalized_key(&path);
            let path_str = key.display().to_string();
            let file = pool.get_or_open(&key, OpenMode::Read, false)?;
            let file_len = file
                .metadata()
                .map_err(|e| StorageError::io(&path_str, e))?
                .len();
            let requested_end = offset.checked_add(len).ok_or_else(|| {
                StorageError::io(
                    &path_str,
                    io::Error::new(io::ErrorKind::InvalidInput, "file range overflows u64"),
                )
            })?;
            if offset >= file_len || requested_end > file_len {
                return Err(StorageError::ShortIo {
                    path: path_str,
                    expected: len as usize,
                    actual: file_len.saturating_sub(offset) as usize,
                });
            }
            let page = seek_data_extent_page(&file, offset, len, max_extents)
                .map_err(|e| StorageError::io(&path_str, e))?;
            if page.used_fallback {
                counters
                    .sparse_seek_fallbacks
                    .fetch_add(1, Ordering::Relaxed);
            }
            let scanned_bytes = page.scanned_to.saturating_sub(offset).min(len);
            let data_bytes = page
                .extents
                .iter()
                .map(|extent| extent.len)
                .fold(0u64, u64::saturating_add);
            let hole_bytes = scanned_bytes.saturating_sub(data_bytes);
            counters
                .sparse_data_extents
                .fetch_add(page.extents.len() as u64, Ordering::Relaxed);
            counters
                .sparse_hole_bytes
                .fetch_add(hole_bytes, Ordering::Relaxed);
            Ok(page)
        })
        .await
    }

    pub async fn hash_sha1(&self, data: bytes::Bytes) -> Result<[u8; 20], StorageError> {
        self.hash_sha1_inner(data, None).await
    }

    /// Hash a shared immutable payload whose allocation is already charged
    /// to `payload_lease`.
    pub async fn hash_sha1_with_shared_payload_lease(
        &self,
        data: bytes::Bytes,
        payload_lease: Arc<MemoryLease>,
    ) -> Result<[u8; 20], StorageError> {
        self.hash_sha1_inner(data, Some(payload_lease)).await
    }

    async fn hash_sha1_inner(
        &self,
        data: bytes::Bytes,
        shared_payload_lease: Option<Arc<MemoryLease>>,
    ) -> Result<[u8; 20], StorageError> {
        let counters = self.counters.clone();
        let started = Instant::now();
        let queued_bytes = match shared_payload_lease {
            Some(lease) => QueuedDiskBytes::reserve_shared_payload(
                counters.clone(),
                self.resources.as_ref(),
                data.len() as u64,
                lease,
                "rt-storage-hash",
            )?,
            None => QueuedDiskBytes::reserve(
                counters.clone(),
                self.resources.as_ref(),
                data.len() as u64,
                "rt-storage-hash",
            )?,
        };
        let hash_owner = HashJobCreditOwner::new(
            "sha1_piece",
            data.len() as u64,
            HashQueueCredit::Owned(queued_bytes),
        );
        hash_owner.trace_submission();
        let result = self
            .hash_pool
            .run(move || {
                let _hash_owner = hash_owner;
                let mut hasher = Sha1::new();
                hasher.update(&data);
                counters.hash_ops.fetch_add(1, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                counters
                    .hash_latency_ns
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.hash_latency_buckets, latency_ns);
                Ok(hasher.finalize().into())
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Hash an input while retrying transient hash-queue pressure.
    ///
    /// The ordinary [`Self::hash_sha1`] operation remains fail-fast so
    /// callers that use `QueueFull` as backpressure keep that behavior. A
    /// caller that retains a correctness-critical input, such as a complete
    /// peer piece assembled in memory, can use this bounded retry path
    /// instead of discarding valid data because another hash is briefly
    /// occupying the dedicated pool.
    pub async fn hash_sha1_retry_queue_full(
        &self,
        data: bytes::Bytes,
    ) -> Result<[u8; 20], StorageError> {
        let mut attempts = 0;
        loop {
            match self.hash_sha1(data.clone()).await {
                Err(StorageError::QueueFull { .. }) if attempts < MAX_HASH_QUEUE_RETRIES => {
                    attempts += 1;
                    if attempts.is_multiple_of(16) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    } else {
                        tokio::task::yield_now().await;
                    }
                }
                result => return result,
            }
        }
    }

    /// Retry transient queue pressure while a shared payload lease stays
    /// attached to every submitted or attempted immutable view.
    pub async fn hash_sha1_retry_queue_full_with_shared_payload_lease(
        &self,
        data: bytes::Bytes,
        payload_lease: Arc<MemoryLease>,
    ) -> Result<[u8; 20], StorageError> {
        let mut attempts = 0;
        loop {
            match self
                .hash_sha1_with_shared_payload_lease(data.clone(), Arc::clone(&payload_lease))
                .await
            {
                Err(StorageError::QueueFull { .. }) if attempts < MAX_HASH_QUEUE_RETRIES => {
                    attempts += 1;
                    if attempts.is_multiple_of(16) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    } else {
                        tokio::task::yield_now().await;
                    }
                }
                result => return result,
            }
        }
    }

    /// Compute both integrity views of a hybrid piece from the same immutable
    /// bytes in one bounded hash-pool job. `v2_content_len` excludes synthetic
    /// BEP 47 padding present in the v1 piece. The two v2 roots cover the
    /// single-file root shape and the full piece-layer shape; the caller picks
    /// the one authenticated by that file's metainfo.
    pub async fn hash_hybrid_piece(
        &self,
        data: bytes::Bytes,
        v2_content_len: usize,
        piece_length: usize,
    ) -> Result<HybridPieceHashes, StorageError> {
        validate_hybrid_piece_hash_geometry(&data, v2_content_len, piece_length)?;
        let queued_bytes = Arc::new(QueuedDiskBytes::reserve(
            self.counters.clone(),
            self.resources.as_ref(),
            data.len() as u64,
            "rt-storage-hash",
        )?);
        self.hash_hybrid_piece_with_reserved_bytes(data, v2_content_len, piece_length, queued_bytes)
            .await
    }

    async fn hash_hybrid_piece_with_reserved_bytes(
        &self,
        data: bytes::Bytes,
        v2_content_len: usize,
        piece_length: usize,
        queued_bytes: Arc<QueuedDiskBytes>,
    ) -> Result<HybridPieceHashes, StorageError> {
        let counters = self.counters.clone();
        let started = Instant::now();
        let piece_leaf_count = piece_length / V2_BLOCK_SIZE;
        let hash_owner = HashJobCreditOwner::new(
            "hybrid_piece",
            data.len() as u64,
            HashQueueCredit::Shared(queued_bytes),
        );
        hash_owner.trace_submission();
        let result = self
            .hash_pool
            .run(move || {
                let _hash_owner = hash_owner;
                let v1_sha1 = Sha1::digest(&data).into();
                let mut accumulator = MerkleAccumulator::new();
                for leaf in data[..v2_content_len].chunks(V2_BLOCK_SIZE) {
                    accumulator.push(BlockHash::of(leaf).0);
                }
                let v2_file_root = accumulator.clone().finish();
                let v2_piece_root =
                    accumulator
                        .finish_padded_to(piece_leaf_count)
                        .ok_or_else(|| {
                            StorageError::io(
                                "rt-storage-hash",
                                io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    "hybrid piece leaf count exceeds v2 piece geometry",
                                ),
                            )
                        })?;
                counters.hash_ops.fetch_add(1, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                counters
                    .hash_latency_ns
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.hash_latency_buckets, latency_ns);
                Ok(HybridPieceHashes {
                    v1_sha1,
                    v2_file_root,
                    v2_piece_root,
                })
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Retry transient saturation while the original immutable payload and
    /// one queued-byte reservation remain owned across every attempt.
    pub async fn hash_hybrid_piece_retry_queue_full(
        &self,
        data: bytes::Bytes,
        v2_content_len: usize,
        piece_length: usize,
    ) -> Result<HybridPieceHashes, StorageError> {
        self.hash_hybrid_piece_retry_queue_full_inner(data, v2_content_len, piece_length, None)
            .await
    }

    /// Retry a shared hybrid payload without charging its backing allocation
    /// to the queued-disk class a second time.
    pub async fn hash_hybrid_piece_retry_queue_full_with_shared_payload_lease(
        &self,
        data: bytes::Bytes,
        v2_content_len: usize,
        piece_length: usize,
        payload_lease: Arc<MemoryLease>,
    ) -> Result<HybridPieceHashes, StorageError> {
        self.hash_hybrid_piece_retry_queue_full_inner(
            data,
            v2_content_len,
            piece_length,
            Some(payload_lease),
        )
        .await
    }

    async fn hash_hybrid_piece_retry_queue_full_inner(
        &self,
        data: bytes::Bytes,
        v2_content_len: usize,
        piece_length: usize,
        shared_payload_lease: Option<Arc<MemoryLease>>,
    ) -> Result<HybridPieceHashes, StorageError> {
        validate_hybrid_piece_hash_geometry(&data, v2_content_len, piece_length)?;
        let mut attempts = 0;
        let queued_bytes = loop {
            let reservation = match shared_payload_lease.as_ref() {
                Some(lease) => QueuedDiskBytes::reserve_shared_payload(
                    self.counters.clone(),
                    self.resources.as_ref(),
                    data.len() as u64,
                    Arc::clone(lease),
                    "rt-storage-hash",
                ),
                None => QueuedDiskBytes::reserve(
                    self.counters.clone(),
                    self.resources.as_ref(),
                    data.len() as u64,
                    "rt-storage-hash",
                ),
            };
            match reservation {
                Ok(reservation) => break Arc::new(reservation),
                Err(StorageError::QueueFull { .. }) if attempts < MAX_HASH_QUEUE_RETRIES => {
                    attempts += 1;
                    if attempts.is_multiple_of(16) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    } else {
                        tokio::task::yield_now().await;
                    }
                }
                Err(error) => return Err(error),
            }
        };

        attempts = 0;
        loop {
            match self
                .hash_hybrid_piece_with_reserved_bytes(
                    data.clone(),
                    v2_content_len,
                    piece_length,
                    queued_bytes.clone(),
                )
                .await
            {
                Err(StorageError::QueueFull { .. }) if attempts < MAX_HASH_QUEUE_RETRIES => {
                    attempts += 1;
                    if attempts.is_multiple_of(16) {
                        tokio::time::sleep(Duration::from_millis(1)).await;
                    } else {
                        tokio::task::yield_now().await;
                    }
                }
                result => return result,
            }
        }
    }

    /// Apply one bounded chunk to a SHA-1 state on the dedicated hash pool.
    ///
    /// Recheck deliberately streams large pieces instead of retaining the
    /// whole piece in memory. Keep the state cloneable at this boundary so a
    /// queue-full rejection leaves the caller's current state intact and can
    /// be retried without losing already-hashed chunks.
    pub(crate) async fn hash_sha1_update(
        &self,
        hasher: &Sha1,
        data: bytes::Bytes,
    ) -> Result<Sha1, StorageError> {
        let counters = self.counters.clone();
        let started = Instant::now();
        let queued_bytes = QueuedDiskBytes::reserve(
            counters.clone(),
            self.resources.as_ref(),
            data.len() as u64,
            "rt-storage-hash",
        )?;
        let next_hasher = hasher.clone();
        let hash_owner = HashJobCreditOwner::new(
            "sha1_recheck_chunk",
            data.len() as u64,
            HashQueueCredit::Owned(queued_bytes),
        );
        hash_owner.trace_submission();
        let result = self
            .hash_pool
            .run(move || {
                let _hash_owner = hash_owner;
                let mut next_hasher = next_hasher;
                next_hasher.update(&data);
                counters.hash_ops.fetch_add(1, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                counters
                    .hash_latency_ns
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.hash_latency_buckets, latency_ns);
                Ok(next_hasher)
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    pub async fn hash_v2_leaf(&self, data: bytes::Bytes) -> Result<[u8; 32], StorageError> {
        let counters = self.counters.clone();
        let started = Instant::now();
        let queued_bytes = QueuedDiskBytes::reserve(
            counters.clone(),
            self.resources.as_ref(),
            data.len() as u64,
            "rt-storage-hash",
        )?;
        let hash_owner = HashJobCreditOwner::new(
            "v2_leaf",
            data.len() as u64,
            HashQueueCredit::Owned(queued_bytes),
        );
        hash_owner.trace_submission();
        let result = self
            .hash_pool
            .run(move || {
                let _hash_owner = hash_owner;
                counters.hash_ops.fetch_add(1, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                counters
                    .hash_latency_ns
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.hash_latency_buckets, latency_ns);
                Ok(BlockHash::of(&data).0)
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Hash `data` as consecutive `leaf_size` BEP 52 leaves in one hash-pool
    /// job. A recheck used to submit one job per 16 KiB leaf (65,536 per GiB);
    /// batching amortises the queueing and wakeup cost over a whole chunk.
    pub async fn hash_v2_leaves(
        &self,
        data: bytes::Bytes,
        leaf_size: usize,
    ) -> Result<Vec<[u8; 32]>, StorageError> {
        let counters = self.counters.clone();
        let started = Instant::now();
        let queued_bytes = QueuedDiskBytes::reserve(
            counters.clone(),
            self.resources.as_ref(),
            data.len() as u64,
            "rt-storage-hash",
        )?;
        let hash_owner = HashJobCreditOwner::new(
            "v2_leaf_batch",
            data.len() as u64,
            HashQueueCredit::Owned(queued_bytes),
        );
        hash_owner.trace_submission();
        let result = self
            .hash_pool
            .run(move || {
                let _hash_owner = hash_owner;
                counters.hash_ops.fetch_add(1, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                counters
                    .hash_latency_ns
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.hash_latency_buckets, latency_ns);
                Ok(data
                    .chunks(leaf_size.max(1))
                    .map(|leaf| BlockHash::of(leaf).0)
                    .collect())
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    pub async fn hash_v2_root(&self, leaves: Vec<[u8; 32]>) -> Result<[u8; 32], StorageError> {
        let counters = self.counters.clone();
        let started = Instant::now();
        let input_bytes = (leaves.len() * 32) as u64;
        let queued_bytes = QueuedDiskBytes::reserve(
            counters.clone(),
            self.resources.as_ref(),
            input_bytes,
            "rt-storage-hash",
        )?;
        let hash_owner =
            HashJobCreditOwner::new("v2_root", input_bytes, HashQueueCredit::Owned(queued_bytes));
        hash_owner.trace_submission();
        let result = self
            .hash_pool
            .run(move || {
                let _hash_owner = hash_owner;
                counters.hash_ops.fetch_add(1, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                counters
                    .hash_latency_ns
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.hash_latency_buckets, latency_ns);
                Ok(merkle_root(&leaves))
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    /// Finish a streaming BEP 52 tree on the shared hash pool. The accumulator
    /// holds one node per tree level, so retrying a full queue only clones a
    /// small bounded state rather than retaining a file's leaf hashes.
    pub async fn finish_v2_merkle_root(
        &self,
        accumulator: rt_hash::MerkleAccumulator,
    ) -> Result<[u8; 32], StorageError> {
        let counters = self.counters.clone();
        let started = Instant::now();
        let result = self
            .hash_pool
            .run(move || {
                let root = accumulator.finish();
                counters.hash_ops.fetch_add(1, Ordering::Relaxed);
                let latency_ns = latency_ns_since(started);
                counters
                    .hash_latency_ns
                    .fetch_add(latency_ns, Ordering::Relaxed);
                record_latency_bucket(&counters.hash_latency_buckets, latency_ns);
                Ok(root)
            })
            .await;
        if matches!(result, Err(StorageError::QueueFull { .. })) {
            self.counters.queue_full.fetch_add(1, Ordering::Relaxed);
        }
        result
    }
}

#[instrument(skip(scheduler), fields(path = %path.display(), offset, len))]
pub async fn scheduled_read(
    scheduler: &MountScheduler,
    class: IoClass,
    path: &Path,
    offset: u64,
    len: usize,
) -> Result<bytes::Bytes, StorageError> {
    scheduler.read_at(class, path, offset, len).await
}

#[instrument(skip(scheduler), fields(path = %path.display(), offset, len))]
pub async fn scheduled_read_owned(
    scheduler: &MountScheduler,
    class: IoClass,
    path: &Path,
    offset: u64,
    len: usize,
) -> Result<StorageRead, StorageError> {
    scheduler.read_owned_at(class, path, offset, len).await
}

#[instrument(skip(scheduler, data), fields(path = %path.display(), offset, len = data.len()))]
pub async fn scheduled_write(
    scheduler: &MountScheduler,
    class: IoClass,
    path: &Path,
    offset: u64,
    data: bytes::Bytes,
    create: bool,
) -> Result<(), StorageError> {
    scheduler.write_at(class, path, offset, data, create).await
}

#[instrument(skip(scheduler, data, payload_lease), fields(path = %path.display(), offset, len = data.len()))]
pub async fn scheduled_write_with_shared_payload_lease(
    scheduler: &MountScheduler,
    class: IoClass,
    path: &Path,
    offset: u64,
    data: bytes::Bytes,
    create: bool,
    payload_lease: Arc<MemoryLease>,
) -> Result<(), StorageError> {
    scheduler
        .write_at_with_shared_payload_lease(class, path, offset, data, create, payload_lease)
        .await
}

fn normalized_key(path: &Path) -> PathBuf {
    normalized_storage_path(path).unwrap_or_else(|_| PathBuf::from(".").join(path))
}

fn class_index(class: IoClass) -> usize {
    class as usize
}

fn load_atomic_array<const N: usize>(values: &[AtomicU64; N]) -> [u64; N] {
    std::array::from_fn(|index| values[index].load(Ordering::Relaxed))
}

fn latency_ns_since(started: Instant) -> u64 {
    started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64
}

fn record_latency_bucket(buckets: &[AtomicU64; STORAGE_LATENCY_BUCKET_COUNT], latency_ns: u64) {
    for (index, upper_bound) in STORAGE_LATENCY_BUCKETS_NS.iter().enumerate() {
        if latency_ns <= *upper_bound {
            buckets[index].fetch_add(1, Ordering::Relaxed);
        }
    }
}

fn record_sync_completion(counters: &StorageCounters, started: Instant) {
    counters.sync_ops.fetch_add(1, Ordering::Relaxed);
    let latency_ns = latency_ns_since(started);
    counters
        .sync_latency_ns
        .fetch_add(latency_ns, Ordering::Relaxed);
    record_latency_bucket(&counters.sync_latency_buckets, latency_ns);
}

fn peer_read_cache_hit(
    cache: &Mutex<PeerReadCache>,
    key: &Path,
    identity: FileIdentity,
    offset: u64,
    len: usize,
) -> Option<bytes::Bytes> {
    lock_reset_derived_cache(cache).hit(key, identity, offset, len)
}

fn peer_read_cache_invalidate(cache: &Mutex<PeerReadCache>, epoch: &AtomicU64, key: &Path) {
    epoch.fetch_add(1, Ordering::AcqRel);
    lock_reset_derived_cache(cache).remove_file(key);
}

struct PeerReadCacheInsert {
    generation: u64,
    key: PathBuf,
    identity: FileIdentity,
    source: Weak<LeasedFile>,
    offset: u64,
    data: bytes::Bytes,
    max_entries: usize,
    max_bytes: usize,
    backing_bytes: u64,
    result_len: usize,
}

fn peer_read_cache_store_parts(
    cache: &Arc<Mutex<PeerReadCache>>,
    manager: &Arc<PeerReadCacheManager>,
    epoch: &AtomicU64,
    resources: Option<&ResourceGovernor>,
    counters: &Arc<StorageCounters>,
    insert: PeerReadCacheInsert,
) -> Option<bytes::Bytes> {
    let PeerReadCacheInsert {
        generation,
        key,
        identity,
        source,
        offset,
        data,
        max_entries,
        max_bytes,
        backing_bytes,
        result_len,
    } = insert;
    if max_entries == 0 || max_bytes == 0 || data.len() > max_bytes || result_len > data.len() {
        return None;
    }
    let budget_lease = manager.reserve(backing_bytes)?;
    let lease = if let Some(resources) = resources {
        match resources.try_acquire(MemoryClass::PeerBuffer, backing_bytes) {
            Some(lease) => Some(lease),
            None => {
                counters.queue_full.fetch_add(1, Ordering::Relaxed);
                return None;
            }
        }
    } else {
        None
    };
    let mut cache_guard = lock_reset_derived_cache(cache);
    if epoch.load(Ordering::Acquire) != generation {
        // A write completed while this read was in flight. Do not publish
        // stale backend bytes into the cache.
        return None;
    }
    let data = bytes::Bytes::from_owner(CachedPeerBytes {
        bytes: data,
        _resource_lease: lease,
        _budget_lease: budget_lease,
    });
    let result = data.slice(..result_len);
    let consumed_prefix = result_len.min(data.len());
    let consumed_suffix = data.len();
    let evicted = cache_guard.insert(
        key,
        PeerReadCacheEntry {
            identity,
            source,
            offset,
            data,
            consumed_prefix,
            consumed_suffix,
            consumed_granules: 0,
            last_used: Instant::now(),
        },
        max_entries,
        max_bytes,
    );
    let evicted = evicted?;
    if evicted {
        counters
            .peer_read_cache_evictions
            .fetch_add(1, Ordering::Relaxed);
    }
    let should_register = !cache_guard.registered_with_manager;
    cache_guard.registered_with_manager = true;
    drop(cache_guard);
    if should_register {
        manager.register(
            cache,
            counters,
            resources.map(ResourceGovernor::reclaim_scope_id),
        );
    }
    Some(result)
}

fn advise_for_read_class(
    file: &File,
    class: IoClass,
    offset: u64,
    len: usize,
    counters: &StorageCounters,
) {
    if len < 256 * 1024 {
        return;
    }
    match class {
        IoClass::PeerRead | IoClass::Recheck => {
            if advise_page_cache(file, offset, len, PageCacheAdvice::Sequential) {
                counters
                    .page_cache_advise_sequential
                    .fetch_add(1, Ordering::Relaxed);
            } else {
                counters
                    .page_cache_advise_failures
                    .fetch_add(1, Ordering::Relaxed);
            }
            if class == IoClass::PeerRead {
                if advise_page_cache(file, offset, len, PageCacheAdvice::WillNeed) {
                    counters
                        .page_cache_advise_willneed
                        .fetch_add(1, Ordering::Relaxed);
                } else {
                    counters
                        .page_cache_advise_failures
                        .fetch_add(1, Ordering::Relaxed);
                }
            }
        }
        _ => {}
    }
}

fn advise_after_read_class(
    file: &File,
    class: IoClass,
    offset: u64,
    len: usize,
    counters: &StorageCounters,
) {
    if class != IoClass::Recheck || len < 256 * 1024 {
        return;
    }
    if advise_page_cache(file, offset, len, PageCacheAdvice::DontNeed) {
        counters
            .page_cache_advise_dontneed
            .fetch_add(1, Ordering::Relaxed);
    } else {
        counters
            .page_cache_advise_failures
            .fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy)]
enum PageCacheAdvice {
    Sequential,
    WillNeed,
    DontNeed,
}

fn advise_page_cache(file: &File, offset: u64, len: usize, advice: PageCacheAdvice) -> bool {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        use std::os::fd::AsRawFd;

        let fd = file.as_raw_fd();
        let offset = offset.min(i64::MAX as u64) as libc::off_t;
        let len = len.min(i64::MAX as usize) as libc::off_t;
        let advice = match advice {
            PageCacheAdvice::Sequential => libc::POSIX_FADV_SEQUENTIAL,
            PageCacheAdvice::WillNeed => libc::POSIX_FADV_WILLNEED,
            PageCacheAdvice::DontNeed => libc::POSIX_FADV_DONTNEED,
        };
        unsafe { libc::posix_fadvise(fd, offset, len, advice) == 0 }
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = (file, offset, len, advice);
        true
    }
}

fn seek_data_extent_page(
    file: &File,
    offset: u64,
    len: u64,
    max_extents: usize,
) -> std::io::Result<DataExtentPage> {
    #[cfg(not(target_os = "linux"))]
    let _ = (file, max_extents);

    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;

        let end = offset.checked_add(len).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "file range overflows u64")
        })?;
        if len == 0 {
            return Ok(DataExtentPage {
                extents: Vec::new(),
                scanned_to: offset,
                complete: true,
                used_fallback: false,
            });
        }

        let fd = file.as_raw_fd();
        let mut cursor = offset;
        let mut extents = Vec::new();
        while cursor < end {
            let data = unsafe {
                libc::lseek(
                    fd,
                    cursor.min(i64::MAX as u64) as libc::off_t,
                    libc::SEEK_DATA,
                )
            };
            if data < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::ENXIO) {
                    break;
                }
                if matches!(err.raw_os_error(), Some(libc::EINVAL)) {
                    return Ok(DataExtentPage {
                        extents: vec![DataExtent { offset, len }],
                        scanned_to: end,
                        complete: true,
                        used_fallback: true,
                    });
                }
                return Err(err);
            }
            let data = (data as u64).max(cursor);
            if data >= end {
                break;
            }
            let hole = unsafe {
                libc::lseek(
                    fd,
                    data.min(i64::MAX as u64) as libc::off_t,
                    libc::SEEK_HOLE,
                )
            };
            if hole < 0 {
                let err = std::io::Error::last_os_error();
                if matches!(err.raw_os_error(), Some(libc::EINVAL)) {
                    return Ok(DataExtentPage {
                        extents: vec![DataExtent { offset, len }],
                        scanned_to: end,
                        complete: true,
                        used_fallback: true,
                    });
                }
                return Err(err);
            }
            let extent_end = (hole as u64).min(end);
            if extent_end <= data {
                // SEEK_DATA/SEEK_HOLE did not make forward progress. Treat
                // the filesystem as unsupported instead of risking a loop
                // or silently skipping bytes.
                return Ok(DataExtentPage {
                    extents: vec![DataExtent { offset, len }],
                    scanned_to: end,
                    complete: true,
                    used_fallback: true,
                });
            }
            extents.push(DataExtent {
                offset: data,
                len: extent_end - data,
            });
            cursor = extent_end;
            if extents.len() >= max_extents && cursor < end {
                return Ok(DataExtentPage {
                    extents,
                    scanned_to: cursor,
                    complete: false,
                    used_fallback: false,
                });
            }
        }
        Ok(DataExtentPage {
            extents,
            scanned_to: end,
            complete: true,
            used_fallback: false,
        })
    }

    #[cfg(not(target_os = "linux"))]
    {
        if len == 0 {
            Ok(DataExtentPage {
                extents: Vec::new(),
                scanned_to: offset,
                complete: true,
                used_fallback: true,
            })
        } else {
            Ok(DataExtentPage {
                extents: vec![DataExtent { offset, len }],
                scanned_to: offset.saturating_add(len),
                complete: true,
                used_fallback: true,
            })
        }
    }
}

fn effective_file_pool_size(configured: usize, process_budget: usize) -> usize {
    configured.min(process_budget)
}

/// Reserve `len` bytes of real storage for `file`.
///
/// This calls `fallocate(2)` directly instead of `posix_fallocate(3)` on
/// purpose. When the filesystem has no native preallocation, glibc's
/// `posix_fallocate` *emulates* it by writing zero bytes into every block.
/// Those blocks are indistinguishable from written payload, so after a host
/// crash a never-written region would look like valid, fully allocated data
/// and defeat the allocation audit (`alloc_audit`). A native `fallocate`
/// instead leaves unwritten extents the filesystem can report, and on a
/// filesystem without support it fails with `EOPNOTSUPP`; the caller then
/// falls back to a sparse length, which is honest about holes.
fn full_preallocate(file: &File, len: u64) -> Result<(), StorageError> {
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::AsRawFd;
        // SAFETY: `file` owns a live descriptor for the duration of the call.
        let rc = unsafe { libc::fallocate(file.as_raw_fd(), 0, 0, len as libc::off_t) };
        if rc == 0 {
            return Ok(());
        }
        Err(StorageError::Io {
            path: "<preallocate>".to_owned(),
            source: std::io::Error::last_os_error(),
        })
    }
    #[cfg(not(target_os = "linux"))]
    {
        file.set_len(len)
            .map_err(|e| StorageError::io("<preallocate>", e))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::elevator::DeviceId;
    use crate::test_support::{Effect, Operation, ScriptedDiskBackend};
    use crate::verify::{PieceVerifier, VerifyResult};
    use rt_path::SafeRelPath;
    use rt_path::StorageRootId;
    use rt_piece_map::{FileSpan, PieceMap};

    fn insert_coverage_range(
        cache: &mut PeerReadCache,
        path: &Path,
        identity: FileIdentity,
        offset: u64,
        length: usize,
    ) {
        cache
            .insert(
                path.to_path_buf(),
                PeerReadCacheEntry {
                    source: Weak::new(),
                    identity,
                    offset,
                    data: bytes::Bytes::from(vec![0x5a; length]),
                    consumed_prefix: 0,
                    consumed_suffix: length,
                    consumed_granules: 0,
                    last_used: Instant::now(),
                },
                8,
                1024 * 1024,
            )
            .unwrap();
    }

    #[test]
    fn completed_wire_blocks_retire_cache_in_every_completion_order() {
        const BLOCK: usize = 16 * 1024;
        let path = Path::new("coverage.bin");
        let identity = test_file_identity();
        for a in 0..4 {
            for b in 0..4 {
                for c in 0..4 {
                    for d in 0..4 {
                        let order = [a, b, c, d];
                        if (0..4).any(|i| order[..i].contains(&order[i])) {
                            continue;
                        }
                        let mut cache = PeerReadCache::default();
                        insert_coverage_range(&mut cache, path, identity, 0, 4 * BLOCK);
                        for (served, block) in order.into_iter().enumerate() {
                            assert_eq!(
                                cache
                                    .hit(path, identity, (block * BLOCK) as u64, BLOCK)
                                    .unwrap()
                                    .len(),
                                BLOCK
                            );
                            assert_eq!(
                                cache.entry_count(),
                                usize::from(served != 3),
                                "order {order:?}"
                            );
                        }
                        assert_eq!(cache.cached_bytes(), 0);
                    }
                }
            }
        }
    }

    #[test]
    fn consuming_one_window_retires_other_fully_consumed_overlaps() {
        const BLOCK: usize = 16 * 1024;
        let path = Path::new("overlap.bin");
        let identity = test_file_identity();
        let mut cache = PeerReadCache::default();
        insert_coverage_range(&mut cache, path, identity, 0, 4 * BLOCK);
        insert_coverage_range(&mut cache, path, identity, BLOCK as u64, 3 * BLOCK);
        assert!(cache.hit(path, identity, 0, BLOCK).is_some());
        let response = cache.hit(path, identity, BLOCK as u64, 3 * BLOCK).unwrap();
        assert_eq!(cache.entry_count(), 0);
        assert_eq!(cache.cached_bytes(), 0);
        assert!(response.iter().all(|byte| *byte == 0x5a));
    }

    #[test]
    fn partial_granule_keeps_its_unread_gap() {
        const BLOCK: usize = 16 * 1024;
        let path = Path::new("gap.bin");
        let identity = test_file_identity();
        let mut cache = PeerReadCache::default();
        insert_coverage_range(&mut cache, path, identity, 0, 4 * BLOCK);
        assert!(cache.hit(path, identity, BLOCK as u64, BLOCK / 2).is_some());
        assert!(cache.hit(path, identity, 0, BLOCK).is_some());
        assert!(cache
            .hit(path, identity, (2 * BLOCK) as u64, 2 * BLOCK)
            .is_some());
        assert_eq!(
            cache.entry_count(),
            1,
            "half of the middle block is still unread"
        );
        assert!(cache.hit(path, identity, BLOCK as u64, BLOCK).is_some());
        assert_eq!(cache.entry_count(), 0);
    }

    #[tokio::test]
    async fn read_crossing_a_cached_window_consumes_its_remaining_tail() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("straddling.bin");
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwx").unwrap();
        let scheduler = MountScheduler::new_for_path(
            StorageRootId::new(),
            dir.path(),
            &SchedulerConfig {
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 16,
                    peer_read_readahead_sequential_only: false,
                    peer_read_cache_entries: 8,
                    peer_read_cache_bytes: 64,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        assert_eq!(
            &scheduled_read_owned(&scheduler, IoClass::PeerRead, &path, 0, 4)
                .await
                .unwrap()
                .as_slice(),
            &b"abcd"
        );
        assert_eq!(
            &scheduled_read_owned(&scheduler, IoClass::PeerRead, &path, 4, 8)
                .await
                .unwrap()
                .as_slice(),
            &b"efghijkl"
        );
        assert_eq!(
            &scheduled_read_owned(&scheduler, IoClass::PeerRead, &path, 12, 8)
                .await
                .unwrap()
                .as_slice(),
            &b"mnopqrst"
        );
        assert_eq!(scheduler.stats().peer_read_cache_entries, 1);
        assert_eq!(scheduler.stats().peer_read_cache_bytes, 12);
    }

    fn insert_manager_test_range(
        manager: &Arc<PeerReadCacheManager>,
        cache: &Arc<Mutex<PeerReadCache>>,
        counters: &Arc<StorageCounters>,
        key: PathBuf,
        identity: FileIdentity,
        payload: &[u8],
    ) {
        let budget_lease = manager.reserve(payload.len() as u64).unwrap();
        let data = bytes::Bytes::from_owner(CachedPeerBytes {
            bytes: bytes::Bytes::copy_from_slice(payload),
            _resource_lease: None,
            _budget_lease: budget_lease,
        });
        let consumed_suffix = data.len();
        let inserted = lock_reset_derived_cache(cache).insert(
            key,
            PeerReadCacheEntry {
                source: Weak::new(),
                identity,
                offset: 0,
                data,
                consumed_prefix: 0,
                consumed_suffix,
                consumed_granules: 0,
                last_used: Instant::now(),
            },
            8,
            1024,
        );
        assert!(inserted.is_some());
        manager.register(cache, counters, None);
    }

    #[test]
    fn global_peer_cache_budget_reclaims_ranges_across_torrents() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first-cache-owner.bin");
        let second_path = dir.path().join("second-cache-owner.bin");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();
        let first_identity = file_identity(&File::open(&first_path).unwrap()).unwrap();
        let second_identity = file_identity(&File::open(&second_path).unwrap()).unwrap();
        let manager = Arc::new(PeerReadCacheManager::new(8));
        let first_cache = Arc::new(Mutex::new(PeerReadCache::default()));
        let second_cache = Arc::new(Mutex::new(PeerReadCache::default()));
        let first_counters = Arc::new(StorageCounters::default());
        let second_counters = Arc::new(StorageCounters::default());

        insert_manager_test_range(
            &manager,
            &first_cache,
            &first_counters,
            first_path,
            first_identity,
            b"1234",
        );
        insert_manager_test_range(
            &manager,
            &second_cache,
            &second_counters,
            second_path,
            second_identity,
            b"5678",
        );
        assert_eq!(manager.used_bytes(), 8);

        let new_owner = manager.reserve(6).unwrap();

        assert_eq!(manager.used_bytes(), 6);
        assert_eq!(lock_reset_derived_cache(&first_cache).entry_count(), 0);
        assert_eq!(lock_reset_derived_cache(&second_cache).entry_count(), 0);
        assert_eq!(
            first_counters
                .peer_read_cache_evictions
                .load(Ordering::Relaxed),
            1
        );
        assert_eq!(
            second_counters
                .peer_read_cache_evictions
                .load(Ordering::Relaxed),
            1
        );
        drop(new_owner);
        assert_eq!(manager.used_bytes(), 0);
    }

    #[test]
    fn cache_alias_keeps_both_memory_budgets_until_last_slice_drops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cache-alias-owner.bin");
        std::fs::write(&path, b"payload").unwrap();
        let identity = file_identity(&File::open(&path).unwrap()).unwrap();
        let manager = Arc::new(PeerReadCacheManager::new(32));
        let cache = Arc::new(Mutex::new(PeerReadCache::default()));
        let counters = Arc::new(StorageCounters::default());
        let governor = ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: 32,
            class_caps_bytes: [32; rt_metrics::MEMORY_CLASS_COUNT],
            ..Default::default()
        });
        let epoch = AtomicU64::new(0);
        let response = peer_read_cache_store_parts(
            &cache,
            &manager,
            &epoch,
            Some(&governor),
            &counters,
            PeerReadCacheInsert {
                source: Weak::new(),
                generation: 0,
                key: path,
                identity,
                offset: 0,
                data: bytes::Bytes::from_static(b"payload"),
                max_entries: 4,
                max_bytes: 32,
                backing_bytes: 7,
                result_len: 3,
            },
        )
        .expect("bounded cache admission succeeds");
        assert_eq!(&response[..], b"pay");
        assert_eq!(manager.reclaim(7), 0);
        assert_eq!(lock_reset_derived_cache(&cache).entry_count(), 0);
        assert_eq!(manager.used_bytes(), 7);
        assert_eq!(
            governor.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            7
        );

        drop(response);
        assert_eq!(manager.used_bytes(), 0);
        assert_eq!(
            governor.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            0
        );
    }

    #[test]
    fn governor_reclaim_does_not_evict_another_governors_cache() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("governor-one.bin");
        let second_path = dir.path().join("governor-two.bin");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();
        let first_identity = file_identity(&File::open(&first_path).unwrap()).unwrap();
        let second_identity = file_identity(&File::open(&second_path).unwrap()).unwrap();
        let manager = Arc::new(PeerReadCacheManager::new(8));
        let first_cache = Arc::new(Mutex::new(PeerReadCache::default()));
        let second_cache = Arc::new(Mutex::new(PeerReadCache::default()));
        let first_counters = Arc::new(StorageCounters::default());
        let second_counters = Arc::new(StorageCounters::default());
        let config = rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: 8,
            class_caps_bytes: [8; rt_metrics::MEMORY_CLASS_COUNT],
            ..Default::default()
        };
        let first_governor = ResourceGovernor::new(config);
        let second_governor = ResourceGovernor::new(config);

        let first_response = peer_read_cache_store_parts(
            &first_cache,
            &manager,
            &AtomicU64::new(0),
            Some(&first_governor),
            &first_counters,
            PeerReadCacheInsert {
                source: Weak::new(),
                generation: 0,
                key: first_path,
                identity: first_identity,
                offset: 0,
                data: bytes::Bytes::from_static(b"1111"),
                max_entries: 4,
                max_bytes: 8,
                backing_bytes: 4,
                result_len: 1,
            },
        )
        .unwrap();
        let second_response = peer_read_cache_store_parts(
            &second_cache,
            &manager,
            &AtomicU64::new(0),
            Some(&second_governor),
            &second_counters,
            PeerReadCacheInsert {
                source: Weak::new(),
                generation: 0,
                key: second_path,
                identity: second_identity,
                offset: 0,
                data: bytes::Bytes::from_static(b"2222"),
                max_entries: 4,
                max_bytes: 8,
                backing_bytes: 4,
                result_len: 1,
            },
        )
        .unwrap();
        drop((first_response, second_response));

        assert_eq!(
            manager.reclaim_for_scope(4, first_governor.reclaim_scope_id()),
            4
        );
        assert_eq!(lock_reset_derived_cache(&first_cache).entry_count(), 0);
        assert_eq!(lock_reset_derived_cache(&second_cache).entry_count(), 1);
        assert_eq!(manager.used_bytes(), 4);
        assert_eq!(
            first_governor.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            0
        );
        assert_eq!(
            second_governor.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            4
        );
    }

    #[test]
    fn default_storage_pools_use_bounded_worker_counts() {
        let config = StorageIoConfig::default();
        assert_eq!(config.io_worker_threads, 2);
        assert_eq!(config.hash_worker_threads, 1);
    }

    #[test]
    fn storage_credit_keeps_frame_pool_charge_separate_from_governor_bytes() {
        let mut first = StorageCreditEvidence {
            governor_credit_bytes: 4096,
            ..StorageCreditEvidence::default()
        };
        first.include_frame_pool_charge(64 * 1024);

        let mut coalesced = StorageCreditEvidence::default();
        coalesced.include_frame_pool_charge(16 * 1024);
        first.include(coalesced);

        assert_eq!(first.governor_credit_bytes, 4096);
        assert_eq!(first.frame_pool_charge_bytes_at_dispatch, 80 * 1024);
    }

    #[test]
    fn storage_credit_merge_is_order_independent_across_resource_domains() {
        let governor_and_peer_payload = StorageCreditEvidence {
            governor_credit_leases: 1,
            governor_credit_bytes: 4096,
            governor_credit_lease_id: Some(11),
            governor_credit_lease_ids_xor: 11,
            governor_credit_lease_ids_sum: 11,
            logical_queued_bytes: 8192,
            shared_payload_class: Some(MemoryClass::PeerBuffer),
            shared_payload_lease_count: 1,
            shared_payload_lease_id: Some(21),
            shared_payload_lease_ids_xor: 21,
            shared_payload_lease_ids_sum: 21,
            shared_payload_lease_bytes: 16 * 1024,
            shared_payload_view_bytes: 8192,
            frame_pool_charge_bytes_at_dispatch: 64 * 1024,
            ..StorageCreditEvidence::default()
        };
        let queue_and_webseed_payload = StorageCreditEvidence {
            governor_credit_leases: 1,
            governor_credit_bytes: 2048,
            governor_credit_lease_id: Some(12),
            governor_credit_lease_ids_xor: 12,
            governor_credit_lease_ids_sum: 12,
            logical_queued_bytes: 4096,
            shared_payload_class: Some(MemoryClass::WebseedBody),
            shared_payload_lease_count: 1,
            shared_payload_lease_id: Some(22),
            shared_payload_lease_ids_xor: 22,
            shared_payload_lease_ids_sum: 22,
            shared_payload_lease_bytes: 4096,
            shared_payload_view_bytes: 4096,
            frame_pool_charge_bytes_at_dispatch: 16 * 1024,
            ..StorageCreditEvidence::default()
        };

        let mut forward = governor_and_peer_payload;
        forward.include(queue_and_webseed_payload);
        let mut reverse = queue_and_webseed_payload;
        reverse.include(governor_and_peer_payload);

        assert_eq!(forward, reverse);
        assert_eq!(forward.governor_credit_leases, 2);
        assert_eq!(forward.governor_credit_bytes, 6144);
        assert_eq!(forward.governor_credit_lease_id, None);
        assert_eq!(forward.governor_credit_lease_ids_xor, 11 ^ 12);
        assert_eq!(forward.governor_credit_lease_ids_sum, 23);
        assert_eq!(forward.logical_queued_bytes, 12 * 1024);
        assert_eq!(forward.shared_payload_class, None);
        assert!(forward.shared_payload_classes_mixed);
        assert_eq!(forward.shared_payload_lease_count, 2);
        assert_eq!(forward.shared_payload_lease_id, None);
        assert_eq!(forward.shared_payload_lease_ids_xor, 21 ^ 22);
        assert_eq!(forward.shared_payload_lease_ids_sum, 43);
        assert_eq!(forward.shared_payload_lease_bytes, 20 * 1024);
        assert_eq!(forward.shared_payload_view_bytes, 12 * 1024);
        assert_eq!(forward.frame_pool_charge_bytes_at_dispatch, 80 * 1024);

        let mut single = StorageCreditEvidence::default();
        single.include(governor_and_peer_payload);
        assert_eq!(single.governor_credit_lease_id, Some(11));
        assert_eq!(single.shared_payload_lease_id, Some(21));

        let mut same_class = governor_and_peer_payload;
        same_class.include(governor_and_peer_payload);
        assert_eq!(
            same_class.shared_payload_class,
            Some(MemoryClass::PeerBuffer)
        );
        assert!(!same_class.shared_payload_classes_mixed);
        assert_eq!(same_class.shared_payload_lease_id, None);
        assert_eq!(same_class.shared_payload_lease_ids_xor, 0);
        assert_eq!(same_class.shared_payload_lease_ids_sum, 42);
    }

    #[test]
    fn hash_job_owner_retains_queued_credit_until_drop() {
        let counters = Arc::new(StorageCounters::default());
        let resources = ResourceGovernor::new(Default::default());
        let queued_bytes =
            QueuedDiskBytes::reserve(counters.clone(), Some(&resources), 4096, "hash-owner-test")
                .unwrap();
        let owner =
            HashJobCreditOwner::new("sha1_piece", 4096, HashQueueCredit::Owned(queued_bytes));
        owner.trace_submission();

        assert!(matches!(
            &owner._queued_bytes,
            HashQueueCredit::Owned(queued) if queued.bytes == queued_disk_charge(4096)
        ));
        assert_eq!(
            counters.queued_disk_bytes.load(Ordering::Relaxed),
            queued_disk_charge(4096)
        );
        assert_eq!(
            resources.snapshot().total_used_bytes,
            queued_disk_charge(4096)
        );

        drop(owner);

        assert_eq!(counters.queued_disk_bytes.load(Ordering::Relaxed), 0);
        assert_eq!(resources.snapshot().total_used_bytes, 0);
    }

    #[test]
    fn storage_frame_queue_multi_hash_credit_reconciles_live_snapshots() {
        let counters = Arc::new(StorageCounters::default());
        let resources = ResourceGovernor::new(Default::default());
        let frames = crate::frame::FramePool::new(64 * 1024);
        let baseline = resources.snapshot();
        assert_eq!(baseline.total_used_bytes, 0);
        assert_eq!(frames.in_use_bytes(), 0);

        let frame = frames.try_acquire(1).expect("frame fits the local pool");
        let frame_charge = frame.charged_bytes();
        assert_eq!(frame_charge, 16 * 1024);
        assert_eq!(frames.in_use_bytes(), frame_charge);

        let payload_lease = Arc::new(
            resources
                .try_acquire(MemoryClass::PeerBuffer, 4096)
                .expect("payload lease fits the resource governor"),
        );
        let queued_bytes = Arc::new(
            QueuedDiskBytes::reserve_shared_payload(
                counters.clone(),
                Some(&resources),
                4096,
                Arc::clone(&payload_lease),
                "frame-queue-hash-test",
            )
            .unwrap(),
        );
        drop(payload_lease);

        let mut operation_record = StorageOperationRecord::default();
        operation_record
            .resource_credit
            .include(queued_bytes.resource_credit());
        operation_record
            .resource_credit
            .include_frame_pool_charge(frame.charged_bytes());
        let operation_credit = operation_record.resource_credit;
        assert_eq!(operation_credit.governor_credit_leases, 1);
        assert_eq!(
            operation_credit.governor_credit_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(operation_credit.shared_payload_lease_count, 1);
        assert_eq!(operation_credit.shared_payload_lease_bytes, 4096);
        assert_eq!(operation_credit.shared_payload_view_bytes, 4096);
        assert_eq!(
            operation_credit.frame_pool_charge_bytes_at_dispatch,
            frame_charge
        );

        let owner = HashJobCreditOwner::new(
            "sha1_piece",
            4096,
            HashQueueCredit::Shared(Arc::clone(&queued_bytes)),
        );
        let second_queued_bytes = QueuedDiskBytes::reserve(
            counters.clone(),
            Some(&resources),
            2048,
            "second-hash-owner-test",
        )
        .unwrap();
        let second_job_credit = second_queued_bytes.resource_credit();
        let second_owner =
            HashJobCreditOwner::new("v2_leaf", 2048, HashQueueCredit::Owned(second_queued_bytes));
        let mut pipeline_credit = operation_credit;
        pipeline_credit.include(second_job_credit);
        drop(queued_bytes);
        drop(frame);

        let retained = resources.snapshot();
        assert_eq!(frames.in_use_bytes(), 0);
        assert_eq!(
            retained.classes[MemoryClass::PeerBuffer as usize].used_bytes,
            4096
        );
        assert_eq!(
            retained.classes[MemoryClass::QueuedDisk as usize].used_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES + 2048
        );
        assert_eq!(
            pipeline_credit.governor_credit_bytes + pipeline_credit.shared_payload_lease_bytes,
            retained.total_used_bytes
        );
        assert_eq!(
            counters.queued_disk_bytes.load(Ordering::Relaxed),
            queued_disk_charge(4096) + queued_disk_charge(2048)
        );

        drop(owner);
        let first_released = resources.snapshot();
        assert_eq!(first_released.total_used_bytes, queued_disk_charge(2048));
        assert_eq!(
            first_released.classes[MemoryClass::PeerBuffer as usize].used_bytes,
            0
        );
        assert_eq!(
            first_released.classes[MemoryClass::QueuedDisk as usize].used_bytes,
            queued_disk_charge(2048)
        );
        assert_eq!(
            counters.queued_disk_bytes.load(Ordering::Relaxed),
            queued_disk_charge(2048)
        );

        drop(second_owner);

        let released = resources.snapshot();
        assert_eq!(released.total_used_bytes, baseline.total_used_bytes);
        assert_eq!(
            released.classes[MemoryClass::PeerBuffer as usize].used_bytes,
            baseline.classes[MemoryClass::PeerBuffer as usize].used_bytes
        );
        assert_eq!(
            released.classes[MemoryClass::QueuedDisk as usize].used_bytes,
            baseline.classes[MemoryClass::QueuedDisk as usize].used_bytes
        );
        assert_eq!(frames.in_use_bytes(), 0);
        assert_eq!(counters.queued_disk_bytes.load(Ordering::Relaxed), 0);
    }

    #[tokio::test]
    async fn production_scheduler_dispatch_credit_matches_live_owners_until_completion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("scheduler-owner-chain.bin");
        std::fs::write(&path, b"payload").unwrap();
        let resources = ResourceGovernor::new(Default::default());
        let scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        let (scheduler, backend) = scheduler_with_scripted_backend_for(scheduler);
        backend.inject(Operation::Read, Effect::HoldCompletion);

        let read_scheduler = scheduler.clone();
        let read_path = path.clone();
        let read = tokio::spawn(async move {
            read_scheduler
                .read_at(IoClass::Recheck, &read_path, 0, 7)
                .await
        });
        backend.wait_until_completion_is_held().await;

        let dispatches = scheduler
            .counters
            .operation_dispatches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(dispatches.len(), 1);
        let dispatch = dispatches[0].clone();
        assert_eq!(
            dispatch.byte_range,
            Some(StorageByteRange {
                offset: 0,
                length: 7
            })
        );
        assert!(dispatch.has_file_identity);
        assert!(dispatch.file_owner_retained);
        assert!(!dispatch.payload_owner_retained);
        assert_eq!(dispatch.device_queue_permits, 1);
        assert_eq!(
            dispatch.credit.frame_pool_charge_bytes_at_dispatch,
            16 * 1024
        );

        let stats = scheduler.stats();
        let resource_snapshot = resources.snapshot();
        let queued_disk_used =
            resource_snapshot.classes[MemoryClass::QueuedDisk as usize].used_bytes;
        assert_eq!(dispatch.credit.governor_credit_leases, 1);
        assert_eq!(dispatch.credit.governor_credit_bytes, queued_disk_used);
        assert_eq!(
            dispatch.credit.logical_queued_bytes,
            stats.queued_disk_bytes
        );
        assert_eq!(stats.queued_disk_bytes, queued_disk_used);
        assert_eq!(
            stats.device_queue_capacity - stats.device_queue_available,
            1
        );

        assert!(backend.release_next());
        assert_eq!(&read.await.unwrap().unwrap()[..], b"payload");
        assert_storage_admission_released(&scheduler);
        assert_eq!(resources.snapshot().total_used_bytes, 0);
    }

    #[tokio::test]
    async fn production_scheduler_multi_operation_credit_reconciles_until_completion() {
        let dir = tempfile::tempdir().unwrap();
        let first_read_path = dir.path().join("multi-read-first.bin");
        let second_read_path = dir.path().join("multi-read-second.bin");
        let write_path = dir.path().join("multi-write.bin");
        std::fs::write(&first_read_path, b"read-one").unwrap();
        std::fs::write(&second_read_path, b"second-data").unwrap();
        std::fs::write(&write_path, b"--------").unwrap();

        let resources = ResourceGovernor::new(Default::default());
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                max_queue: 4,
                recheck_concurrency: 2,
                storage_io: StorageIoConfig {
                    io_worker_threads: 2,
                    io_queue_depth: 4,
                    peer_read_elevator_budget_ms: 0,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        scheduler.device_queue = Arc::new(DeviceQueue::new(
            Arc::from("device:io001-multi-operation"),
            4,
            3,
        ));
        scheduler.device_queue_capacity = 4;
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        backend.inject(Operation::Read, Effect::HoldCompletion);
        backend.inject(Operation::Read, Effect::HoldCompletion);
        backend.inject(Operation::Write, Effect::HoldCompletion);

        let first_scheduler = scheduler.clone();
        let first_path = first_read_path.clone();
        let first_read = tokio::spawn(async move {
            first_scheduler
                .read_at(IoClass::Recheck, &first_path, 0, 8)
                .await
        });
        backend.wait_until_completion_is_held().await;

        let second_scheduler = scheduler.clone();
        let second_path = second_read_path.clone();
        let second_read = tokio::spawn(async move {
            second_scheduler
                .read_at(IoClass::Recheck, &second_path, 7, 4)
                .await
        });
        backend.wait_until_completion_is_held().await;

        let write_scheduler = scheduler.clone();
        let lease = Arc::new(
            resources
                .try_acquire(MemoryClass::PeerBuffer, 8)
                .expect("shared write payload lease fits the resource budget"),
        );
        let write = tokio::spawn(async move {
            write_scheduler
                .write_at_with_shared_payload_lease(
                    IoClass::PeerWrite,
                    &write_path,
                    0,
                    bytes::Bytes::from_static(b"payload!"),
                    false,
                    lease,
                )
                .await
        });
        backend.wait_until_completion_is_held().await;

        assert_eq!(backend.pending_releases(), 3);
        let dispatches = scheduler
            .counters
            .operation_dispatches
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        assert_eq!(dispatches.len(), 3);

        let mut storage_operation_ids = dispatches
            .iter()
            .map(|dispatch| dispatch.storage_operation_id.expect("storage operation ID"))
            .collect::<Vec<_>>();
        storage_operation_ids.sort_unstable();
        storage_operation_ids.dedup();
        assert_eq!(storage_operation_ids.len(), 3);

        let mut device_operation_ids = dispatches
            .iter()
            .map(|dispatch| dispatch.device_operation_id.expect("device operation ID"))
            .collect::<Vec<_>>();
        device_operation_ids.sort_unstable();
        device_operation_ids.dedup();
        assert_eq!(device_operation_ids.len(), 3);

        let mut read_dispatches = 0;
        let mut write_dispatches = 0;
        for dispatch in &dispatches {
            assert_eq!(
                dispatch.device_queue_key.as_deref(),
                Some("device:io001-multi-operation")
            );
            assert!(dispatch.has_file_identity);
            assert!(dispatch.file_owner_retained);
            assert_eq!(dispatch.device_queue_permits, 1);
            assert_eq!(dispatch.credit.governor_credit_leases, 1);
            assert_eq!(
                dispatch.credit.governor_credit_bytes,
                QUEUED_DISK_JOB_OVERHEAD_BYTES
            );

            match dispatch.operation_kind {
                "positioned_read" => {
                    read_dispatches += 1;
                    assert!(!dispatch.payload_owner_retained);
                    assert_eq!(
                        dispatch.credit.frame_pool_charge_bytes_at_dispatch,
                        16 * 1024
                    );
                    assert_eq!(dispatch.credit.shared_payload_lease_count, 0);
                    assert!(matches!(
                        dispatch.byte_range,
                        Some(StorageByteRange { length: 8 | 4, .. })
                    ));
                }
                "positioned_write" => {
                    write_dispatches += 1;
                    assert!(dispatch.payload_owner_retained);
                    assert_eq!(dispatch.credit.frame_pool_charge_bytes_at_dispatch, 0);
                    assert_eq!(dispatch.credit.shared_payload_lease_count, 1);
                    assert_eq!(
                        dispatch.credit.shared_payload_class,
                        Some(MemoryClass::PeerBuffer)
                    );
                    assert_eq!(dispatch.credit.shared_payload_lease_bytes, 8);
                    assert_eq!(dispatch.credit.shared_payload_view_bytes, 8);
                    assert_eq!(
                        dispatch.byte_range,
                        Some(StorageByteRange {
                            offset: 0,
                            length: 8
                        })
                    );
                }
                other => panic!("unexpected storage operation kind {other}"),
            }
            assert_eq!(
                dispatch.credit.logical_queued_bytes,
                queued_disk_charge(
                    dispatch
                        .byte_range
                        .expect("payload operation has a byte range")
                        .length
                )
            );
        }
        assert_eq!(read_dispatches, 2);
        assert_eq!(write_dispatches, 1);

        let expected_logical_queue_bytes = dispatches
            .iter()
            .map(|dispatch| dispatch.credit.logical_queued_bytes)
            .sum::<u64>();
        let live_stats = scheduler.stats();
        let live_resources = resources.snapshot();
        assert_eq!(live_stats.queued_disk_bytes, expected_logical_queue_bytes);
        assert_eq!(
            live_resources.classes[MemoryClass::QueuedDisk as usize].used_bytes,
            3 * QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(
            live_resources.classes[MemoryClass::PeerBuffer as usize].used_bytes,
            8
        );
        assert_eq!(
            live_stats.device_queue_capacity - live_stats.device_queue_available,
            3
        );

        for _ in 0..3 {
            assert!(backend.release_next());
        }
        assert_eq!(&first_read.await.unwrap().unwrap()[..], b"read-one");
        assert_eq!(&second_read.await.unwrap().unwrap()[..], b"data");
        write.await.unwrap().unwrap();
        assert_storage_admission_released(&scheduler);
        assert_eq!(resources.snapshot().total_used_bytes, 0);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PeerBuffer as usize].used_bytes,
            0
        );
    }

    fn hdd_scheduler() -> MountScheduler {
        MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    file_pool_size: 2,
                    idle_file_ttl_secs: 3600,
                    io_worker_threads: 2,
                    io_queue_depth: 8,
                    ..Default::default()
                },
                ..Default::default()
            },
        )
    }

    fn test_file_identity() -> FileIdentity {
        let file = tempfile::tempfile().unwrap();
        file_identity(&file).unwrap()
    }

    fn ssd_scheduler() -> MountScheduler {
        MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Ssd,
                ..Default::default()
            },
        )
    }

    #[tokio::test]
    async fn acquire_and_release_recheck() {
        let sched = hdd_scheduler();
        let permit = sched.acquire(IoClass::Recheck).await.unwrap();
        assert_eq!(sched.available_permits(IoClass::Recheck), 0);
        drop(permit);
        assert_eq!(sched.available_permits(IoClass::Recheck), 1);
    }

    #[tokio::test]
    async fn peer_read_not_starved_by_recheck() {
        let sched = hdd_scheduler();
        let _r = sched.acquire(IoClass::Recheck).await.unwrap();
        let permit = sched.try_acquire(IoClass::PeerRead);
        assert!(permit.is_some());
    }

    #[tokio::test]
    async fn full_mount_queue_fails_closed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queued.bin");
        std::fs::write(&path, b"queued").unwrap();
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                max_queue: 1,
                storage_io: StorageIoConfig {
                    io_queue_depth: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        let _held_queue = sched.queue_sem.clone().try_acquire_owned().unwrap();

        let result = sched.read_at(IoClass::Foreground, &path, 0, 1).await;

        assert!(matches!(
            result,
            Err(StorageError::QueueFull { mount }) if mount.starts_with("storage-root-")
        ));
        assert_eq!(sched.stats().queue_full, 1);
        assert_eq!(sched.stats().queued_disk_bytes, 0);
    }

    #[tokio::test]
    async fn blocking_pool_full_queue_fails_closed() {
        let (sender, _receiver) = mpsc::sync_channel::<BlockingJob>(1);
        sender.send(Box::new(|| {})).unwrap();
        let pool = BlockingPool {
            queue_name: "test-blocking-pool",
            sender,
            queued: Arc::new(AtomicUsize::new(1)),
        };

        let result = pool.run(|| Ok::<_, StorageError>(())).await;

        assert!(matches!(
            result,
            Err(StorageError::QueueFull { mount }) if mount == "test-blocking-pool"
        ));
        assert_eq!(pool.queued(), 1);
    }

    #[tokio::test]
    async fn blocking_pool_closed_queue_fails_closed() {
        let (sender, receiver) = mpsc::sync_channel::<BlockingJob>(1);
        drop(receiver);
        let pool = BlockingPool {
            queue_name: "test-closed-blocking-pool",
            sender,
            queued: Arc::new(AtomicUsize::new(0)),
        };

        let result = pool.run(|| Ok::<_, StorageError>(())).await;

        assert!(matches!(
            result,
            Err(StorageError::QueueFull { mount }) if mount == "test-closed-blocking-pool"
        ));
        assert_eq!(pool.queued(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_queued_blocking_job_is_skipped() {
        let pool = Arc::new(BlockingPool::new("test-cancelled-blocking-pool", 1, 1));
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first = tokio::spawn({
            let pool = pool.clone();
            async move {
                pool.run(move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok::<_, StorageError>(())
                })
                .await
            }
        });

        started_rx.recv().unwrap();

        let ran = Arc::new(AtomicBool::new(false));
        let second = tokio::spawn({
            let pool = pool.clone();
            let ran = ran.clone();
            async move {
                pool.run(move || {
                    ran.store(true, Ordering::SeqCst);
                    Ok::<_, StorageError>(())
                })
                .await
            }
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            while pool.queued() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second job was not queued");

        second.abort();
        assert!(second.await.unwrap_err().is_cancelled());

        release_tx.send(()).unwrap();
        first.await.unwrap().unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            while pool.queued() != 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cancelled job was not drained");
        assert!(!ran.load(Ordering::SeqCst));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn queued_disk_bytes_track_active_blocking_job_payload() {
        let sched = hdd_scheduler();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let task = tokio::spawn({
            let sched = sched.clone();
            async move {
                sched
                    .run_queued_blocking(IoClass::PeerWrite, 4096, move || {
                        started_tx.send(()).unwrap();
                        release_rx.recv().unwrap();
                        Ok::<_, StorageError>(())
                    })
                    .await
            }
        });

        started_rx.recv().unwrap();
        assert_eq!(sched.stats().queued_disk_bytes, 4096);

        release_tx.send(()).unwrap();
        task.await.unwrap().unwrap();
        assert_eq!(sched.stats().queued_disk_bytes, 0);
    }

    #[tokio::test]
    async fn queued_disk_governor_denies_before_enqueue() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queued-cap.bin");
        std::fs::write(&path, vec![1u8; 4096]).unwrap();
        let mut caps = [1024 * 1024; rt_metrics::MEMORY_CLASS_COUNT];
        caps[MemoryClass::QueuedDisk as usize] = 1024;
        let resources = ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: 1024 * 1024,
            class_caps_bytes: caps,
            ..Default::default()
        });
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Ssd,
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );

        let result = sched.read_at(IoClass::Foreground, &path, 0, 2048).await;

        assert!(matches!(
            result,
            Err(StorageError::QueueFull { mount }) if mount.starts_with("storage-root-")
        ));
        assert_eq!(sched.stats().queued_disk_bytes, 0);
        let snapshot = resources.snapshot();
        assert_eq!(
            snapshot.classes[MemoryClass::QueuedDisk as usize].denied_allocations,
            1
        );
    }

    #[tokio::test]
    async fn owned_read_returns_pooled_frame_for_exact_backend_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("owned-read.bin");
        std::fs::write(&path, b"owned-frame").unwrap();
        let sched = MountScheduler::new(StorageRootId::new(), &SchedulerConfig::default());

        let read = scheduled_read_owned(&sched, IoClass::Foreground, &path, 0, 11)
            .await
            .unwrap();

        assert!(matches!(read, StorageRead::Frame(_)));
        assert_eq!(read.as_slice(), b"owned-frame");
    }

    #[tokio::test]
    async fn compatibility_read_still_returns_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("compat-read.bin");
        std::fs::write(&path, b"compat").unwrap();
        let sched = MountScheduler::new(StorageRootId::new(), &SchedulerConfig::default());

        let read = scheduled_read(&sched, IoClass::Foreground, &path, 0, 6)
            .await
            .unwrap();

        assert_eq!(read, bytes::Bytes::from_static(b"compat"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn runtime_file_pool_rejects_an_ancestor_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("payload.bin");
        std::fs::write(&outside_file, b"outside").unwrap();
        symlink(outside.path(), root.path().join("alias")).unwrap();
        let path = root.path().join("alias/payload.bin");
        let sched = MountScheduler::new(StorageRootId::new(), &SchedulerConfig::default());

        let error = scheduled_read(&sched, IoClass::Foreground, &path, 0, 7)
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            StorageError::Io { source, .. }
                if matches!(source.raw_os_error(), Some(libc::ELOOP | libc::ENOTDIR))
        ));
        assert_eq!(std::fs::read(outside_file).unwrap(), b"outside");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn prepare_file_does_not_create_through_an_ancestor_symlink() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("alias")).unwrap();
        let path = root.path().join("alias/new/nested/file.bin");
        let sched = MountScheduler::new(StorageRootId::new(), &SchedulerConfig::default());

        assert!(sched
            .prepare_file(&path, 16, PreallocationMode::Sparse)
            .await
            .is_err());
        assert!(!outside.path().join("new").exists());
    }

    #[tokio::test]
    async fn schedulers_on_same_device_share_global_queue_across_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("payload.bin");
        std::fs::write(&path, b"payload").unwrap();
        let topology = StorageTopology {
            device_id: Some(DeviceId("shared-device".to_owned())),
            profile: StorageProfile::Hdd,
            fs_type: Some("ext4".to_owned()),
            cow: false,
        };
        let config = SchedulerConfig {
            profile: StorageProfile::Hdd,
            max_queue: 1,
            storage_io: StorageIoConfig {
                io_queue_depth: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let first_root = StorageRootId::new();
        let first = MountScheduler::new_with_profile_and_io_config(
            first_root,
            &config,
            StorageProfile::Hdd,
            config.storage_io.clone(),
            Some(topology.clone()),
        );
        let second_config = SchedulerConfig {
            profile: StorageProfile::Ssd,
            max_queue: 8,
            ..config.clone()
        };
        let second_root = StorageRootId::new();
        let second = MountScheduler::new_with_profile_and_io_config(
            second_root,
            &second_config,
            StorageProfile::Ssd,
            second_config.storage_io.clone(),
            Some(topology),
        );
        assert!(Arc::ptr_eq(&first.device_queue, &second.device_queue));
        assert!(!Arc::ptr_eq(&first.queue_sem, &second.queue_sem));
        let _held = first
            .device_queue
            .acquire(
                IoClass::Foreground,
                format!("storage-root-{}", first.storage_root.0),
            )
            .await
            .unwrap();

        let err = second
            .read_at(IoClass::Foreground, &path, 0, b"payload".len())
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            StorageError::QueueFull { mount }
                if mount == format!("storage-root-{}", second_root.0)
        ));
        let stats = second.stats();
        assert_eq!(stats.queue_full, 1);
        assert_eq!(stats.device_queue_capacity, 1);
        assert_eq!(stats.device_queue_available, 0);
    }

    #[test]
    fn device_dispatch_uses_weighted_class_fairness() {
        let mut state = DeviceQueueState {
            active: 0,
            pending_count: 0,
            pending_by_mount: HashMap::new(),
            mount_turns: std::array::from_fn(|_| VecDeque::new()),
            next_waiter_id: 1,
            next_operation_id: 1,
            dispatch_cursor: 0,
        };
        let mut receivers = Vec::new();
        for class in [
            IoClass::Foreground,
            IoClass::PeerRead,
            IoClass::PeerWrite,
            IoClass::MoveCopy,
            IoClass::Recheck,
            IoClass::Metadata,
        ] {
            for _ in 0..64 {
                let (sender, receiver) = oneshot::channel();
                receivers.push(receiver);
                let id = take_device_sequence(&mut state.next_waiter_id);
                enqueue_device_waiter(
                    &mut state,
                    class,
                    Arc::from(format!("mount-{class:?}")),
                    DeviceWaiter {
                        id,
                        priority: IoRequestPriority::normal(),
                        sender,
                    },
                );
            }
        }

        let mut selected = [0usize; 6];
        for _ in 0..DEVICE_DISPATCH_SLOT_COUNT {
            let (slot, waiter) = pop_next_device_waiter(&mut state, Instant::now()).unwrap();
            let class = device_dispatch_class(slot);
            selected[device_class_index(class)] += 1;
            enqueue_device_waiter(
                &mut state,
                class,
                Arc::from(format!("mount-{class:?}")),
                waiter,
            );
            state.dispatch_cursor = next_device_cursor(slot);
        }

        assert_eq!(selected[device_class_index(IoClass::Foreground)], 16);
        assert_eq!(selected[device_class_index(IoClass::PeerRead)], 8);
        assert_eq!(selected[device_class_index(IoClass::PeerWrite)], 4);
        assert_eq!(selected[device_class_index(IoClass::MoveCopy)], 2);
        assert_eq!(selected[device_class_index(IoClass::Recheck)], 1);
        assert_eq!(selected[device_class_index(IoClass::Metadata)], 1);
        assert_eq!(state.pending_count, 6 * 64);
        drop(receivers);
    }

    #[test]
    fn device_dispatch_round_robins_mounts_within_each_class() {
        let mut state = DeviceQueueState {
            active: 0,
            pending_count: 0,
            pending_by_mount: HashMap::new(),
            mount_turns: std::array::from_fn(|_| VecDeque::new()),
            next_waiter_id: 1,
            next_operation_id: 1,
            dispatch_cursor: 0,
        };
        let mut receivers = Vec::new();
        for (mount, count) in [("busy-mount", 3), ("quiet-mount", 1)] {
            for _ in 0..count {
                let (sender, receiver) = oneshot::channel();
                receivers.push(receiver);
                let id = take_device_sequence(&mut state.next_waiter_id);
                enqueue_device_waiter(
                    &mut state,
                    IoClass::PeerRead,
                    Arc::from(mount),
                    DeviceWaiter {
                        id,
                        priority: IoRequestPriority::normal(),
                        sender,
                    },
                );
            }
        }

        let mut selected = Vec::new();
        for _ in 0..4 {
            let (slot, waiter) = pop_next_device_waiter(&mut state, Instant::now()).unwrap();
            selected.push(waiter.id);
            state.dispatch_cursor = next_device_cursor(slot);
        }

        assert_eq!(selected, vec![1, 4, 2, 3]);
        assert_eq!(state.pending_count, 0);
        drop(receivers);
    }

    #[test]
    fn device_dispatch_promotes_expired_and_choke_critical_requests() {
        let mut state = DeviceQueueState {
            active: 0,
            pending_count: 0,
            pending_by_mount: HashMap::new(),
            mount_turns: std::array::from_fn(|_| VecDeque::new()),
            next_waiter_id: 1,
            next_operation_id: 1,
            dispatch_cursor: 0,
        };
        let mut receivers = Vec::new();
        let now = Instant::now();
        let priorities = [
            (IoRequestPriority::normal(), 1),
            (IoRequestPriority::deadline(now - Duration::from_secs(1)), 2),
            (
                IoRequestPriority::choke_critical(now + Duration::from_secs(10)),
                3,
            ),
        ];
        for (priority, id) in priorities {
            let (sender, receiver) = oneshot::channel();
            receivers.push(receiver);
            enqueue_device_waiter(
                &mut state,
                IoClass::PeerRead,
                Arc::from("upload-owner"),
                DeviceWaiter {
                    id,
                    priority,
                    sender,
                },
            );
        }

        let critical = pop_next_device_waiter(&mut state, now).unwrap().1;
        assert_eq!(critical.id, 3);
        let expired = pop_next_device_waiter(&mut state, now).unwrap().1;
        assert_eq!(expired.id, 2);
        let normal = pop_next_device_waiter(&mut state, now).unwrap().1;
        assert_eq!(normal.id, 1);
        assert_eq!(state.pending_count, 0);
        drop(receivers);
    }

    #[test]
    fn future_io_deadline_stays_eligible_for_batching() {
        let now = Instant::now();

        assert!(!IoRequestPriority::normal().is_urgent());
        assert!(!IoRequestPriority::deadline(now + Duration::from_secs(5)).is_urgent());
        assert!(IoRequestPriority::deadline(now - Duration::from_secs(1)).is_urgent());
        assert!(IoRequestPriority::choke_critical(now + Duration::from_secs(5)).is_urgent());
    }

    #[tokio::test]
    async fn cancelled_device_waiter_releases_bounded_queue_capacity() {
        let queue = Arc::new(DeviceQueue::new(Arc::from("device:test"), 2, 1));
        let active = queue
            .acquire(IoClass::Metadata, "device-test".to_owned())
            .await
            .unwrap();
        let waiting_queue = queue.clone();
        let waiter = tokio::spawn(async move {
            waiting_queue
                .acquire(IoClass::Recheck, "device-test".to_owned())
                .await
        });

        while queue.available() != 0 {
            tokio::task::yield_now().await;
        }
        waiter.abort();
        assert!(matches!(waiter.await, Err(error) if error.is_cancelled()));
        assert_eq!(queue.available(), 1);

        drop(active);
        assert_eq!(queue.available(), 2);
    }

    #[tokio::test]
    async fn device_operation_record_disambiguates_local_sequences_by_queue() {
        let first_queue = Arc::new(DeviceQueue::new(Arc::from("device:first"), 1, 1));
        let second_queue = Arc::new(DeviceQueue::new(Arc::from("device:second"), 1, 1));
        let first_permit = first_queue
            .acquire(IoClass::PeerRead, "first-device".to_owned())
            .await
            .unwrap();
        let second_permit = second_queue
            .acquire(IoClass::PeerRead, "second-device".to_owned())
            .await
            .unwrap();

        let mut first_record = StorageOperationRecord {
            storage_operation_id: Some(StorageOperationId(10)),
            ..StorageOperationRecord::default()
        };
        first_record.bind_device_operation(&first_permit);
        let mut second_record = StorageOperationRecord {
            storage_operation_id: Some(StorageOperationId(11)),
            ..StorageOperationRecord::default()
        };
        second_record.bind_device_operation(&second_permit);

        assert_eq!(
            first_record.device_operation_id,
            second_record.device_operation_id
        );
        assert_eq!(
            first_record.device_queue_key.as_deref(),
            Some("device:first")
        );
        assert_eq!(
            second_record.device_queue_key.as_deref(),
            Some("device:second")
        );
    }

    #[tokio::test]
    async fn external_device_handoff_shares_admission_across_mounts() {
        let topology = StorageTopology {
            device_id: Some(DeviceId("external-handoff-device".to_owned())),
            profile: StorageProfile::Hdd,
            fs_type: Some("ext4".to_owned()),
            cow: false,
        };
        let config = SchedulerConfig {
            profile: StorageProfile::Hdd,
            max_queue: 1,
            storage_io: StorageIoConfig {
                io_worker_threads: 1,
                io_queue_depth: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let first = MountScheduler::new_with_profile_and_io_config(
            StorageRootId::new(),
            &config,
            StorageProfile::Hdd,
            config.storage_io.clone(),
            Some(topology.clone()),
        );
        let second = MountScheduler::new_with_profile_and_io_config(
            StorageRootId::new(),
            &config,
            StorageProfile::Hdd,
            config.storage_io.clone(),
            Some(topology),
        );
        assert!(Arc::ptr_eq(&first.device_queue, &second.device_queue));

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let operation = tokio::spawn(async move {
            first
                .run_device_operation(IoClass::MoveCopy, 64, move || {
                    started_tx.send(()).unwrap();
                    finish_rx.recv().unwrap();
                    Ok(())
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .expect("external operation did not start on the storage pool")
            .expect("external operation start notification was dropped");

        let rejected = second
            .run_device_operation(IoClass::PeerRead, 8, || Ok(()))
            .await;
        assert!(matches!(rejected, Err(StorageError::QueueFull { .. })));

        finish_tx.send(()).unwrap();
        operation.await.unwrap().unwrap();
        assert_eq!(second.device_queue.available(), 1);
    }

    #[tokio::test]
    async fn cancelled_external_handoff_retains_admission_until_completion() {
        let resources = queued_disk_governor(4 * QUEUED_DISK_JOB_OVERHEAD_BYTES);
        let topology = StorageTopology {
            device_id: Some(DeviceId("external-cancel-device".to_owned())),
            profile: StorageProfile::Hdd,
            fs_type: Some("ext4".to_owned()),
            cow: false,
        };
        let config = SchedulerConfig {
            profile: StorageProfile::Hdd,
            max_queue: 1,
            storage_io: StorageIoConfig {
                io_worker_threads: 1,
                io_queue_depth: 1,
                ..Default::default()
            },
            resources: Some(resources.clone()),
            ..Default::default()
        };
        let first = MountScheduler::new_with_profile_and_io_config(
            StorageRootId::new(),
            &config,
            StorageProfile::Hdd,
            config.storage_io.clone(),
            Some(topology.clone()),
        );
        let second = MountScheduler::new_with_profile_and_io_config(
            StorageRootId::new(),
            &config,
            StorageProfile::Hdd,
            config.storage_io.clone(),
            Some(topology),
        );
        assert!(Arc::ptr_eq(&first.device_queue, &second.device_queue));

        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let first_operation_scheduler = first.clone();
        let operation = tokio::spawn(async move {
            first_operation_scheduler
                .run_device_operation(
                    IoClass::MoveCopy,
                    64,
                    move || -> Result<(), StorageError> {
                        started_tx.send(()).unwrap();
                        finish_rx.recv().unwrap();
                        Err(StorageError::io(
                            "external-cancel-test",
                            io::Error::other("injected external operation fault"),
                        ))
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .expect("external operation did not start on the storage pool")
            .expect("external operation start notification was dropped");

        assert_eq!(
            first.stats().queued_disk_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(first.device_queue.available(), 0);
        operation.abort();
        assert!(operation.await.unwrap_err().is_cancelled());

        assert_eq!(
            first.stats().queued_disk_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES,
            "canceling the caller must retain the external operation's byte lease"
        );
        assert_eq!(first.device_queue.available(), 0);

        let denied = second
            .run_device_operation(IoClass::PeerRead, 8, || Ok(()))
            .await;
        assert!(matches!(denied, Err(StorageError::QueueFull { .. })));
        assert_eq!(
            first.stats().queued_disk_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );

        finish_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while first.stats().queued_disk_bytes != 0 || first.device_queue.available() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("external completion should return byte and device admission");
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            0
        );
        second
            .run_device_operation(IoClass::PeerRead, 8, || Ok(()))
            .await
            .unwrap();
        assert_eq!(second.device_queue.available(), 1);
    }

    #[tokio::test]
    async fn multi_device_handoff_holds_source_and_destination_until_completion() {
        let config = SchedulerConfig {
            profile: StorageProfile::Hdd,
            max_queue: 1,
            storage_io: StorageIoConfig {
                io_worker_threads: 1,
                io_queue_depth: 1,
                ..Default::default()
            },
            ..Default::default()
        };
        let destination = MountScheduler::new_with_profile_and_io_config(
            StorageRootId::new(),
            &config,
            StorageProfile::Hdd,
            config.storage_io.clone(),
            Some(StorageTopology {
                device_id: Some(DeviceId("multi-device-destination".to_owned())),
                profile: StorageProfile::Hdd,
                fs_type: Some("ext4".to_owned()),
                cow: false,
            }),
        );
        let source = MountScheduler::new_with_profile_and_io_config(
            StorageRootId::new(),
            &config,
            StorageProfile::Hdd,
            config.storage_io.clone(),
            Some(StorageTopology {
                device_id: Some(DeviceId("multi-device-source".to_owned())),
                profile: StorageProfile::Hdd,
                fs_type: Some("ext4".to_owned()),
                cow: false,
            }),
        );
        let source_check = source.clone();
        let source_key = device_queue_key(source.storage_root, source.device_id.as_deref());
        let source_mount = format!("storage-root-{}", source.storage_root.0);
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let operation = tokio::spawn(async move {
            destination
                .run_device_operation_with_queues(
                    IoClass::MoveCopy,
                    64 * 1024,
                    vec![(source_key, source_mount, source.device_queue.clone())],
                    move || {
                        started_tx.send(()).unwrap();
                        finish_rx.recv().unwrap();
                        Ok(())
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .expect("multi-device operation did not start")
            .expect("multi-device operation start notification was dropped");

        let blocked = source_check
            .run_device_operation(IoClass::PeerRead, 0, || Ok(()))
            .await;
        assert!(matches!(blocked, Err(StorageError::QueueFull { .. })));

        finish_tx.send(()).unwrap();
        operation.await.unwrap().unwrap();
        assert_eq!(source_check.device_queue.available(), 1);
    }

    #[tokio::test]
    async fn opposing_multi_device_handoffs_use_one_queue_order() {
        let config = SchedulerConfig {
            profile: StorageProfile::Hdd,
            max_queue: 2,
            storage_io: StorageIoConfig {
                io_worker_threads: 1,
                io_queue_depth: 2,
                ..Default::default()
            },
            ..Default::default()
        };
        let make_scheduler = |device: &str| {
            let mut scheduler = MountScheduler::new_with_profile_and_io_config(
                StorageRootId::new(),
                &config,
                StorageProfile::Hdd,
                config.storage_io.clone(),
                Some(StorageTopology {
                    device_id: Some(DeviceId(device.to_owned())),
                    profile: StorageProfile::Hdd,
                    fs_type: Some("ext4".to_owned()),
                    cow: false,
                }),
            );
            let key = device_queue_key(scheduler.storage_root, scheduler.device_id.as_deref());
            // This test asserts cross-device lock order with one in-flight
            // operation per device. Pin that admission limit explicitly so
            // the result is independent of whether Auto selected io_uring.
            scheduler.device_queue = Arc::new(DeviceQueue::new(Arc::from(key), 2, 1));
            scheduler.device_queue_capacity = 2;
            scheduler
        };
        let device_a = make_scheduler("multi-device-order-a");
        let device_b = make_scheduler("multi-device-order-b");
        let queue_for = |scheduler: &MountScheduler| {
            (
                device_queue_key(scheduler.storage_root, scheduler.device_id.as_deref()),
                format!("storage-root-{}", scheduler.storage_root.0),
                Arc::clone(&scheduler.device_queue),
            )
        };
        let (started_tx, mut started_rx) = tokio::sync::mpsc::unbounded_channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let release_rx = Arc::new(Mutex::new(release_rx));

        let first_scheduler = device_a.clone();
        let first_other_queue = queue_for(&device_b);
        let first_started = started_tx.clone();
        let first_release = Arc::clone(&release_rx);
        let first = tokio::spawn(async move {
            first_scheduler
                .run_device_operation_with_queues(
                    IoClass::MoveCopy,
                    64,
                    vec![first_other_queue],
                    move || {
                        first_started.send(()).unwrap();
                        lock_recover(&first_release).recv().unwrap();
                        Ok(())
                    },
                )
                .await
        });
        tokio::task::yield_now().await;
        tokio::time::timeout(Duration::from_secs(2), started_rx.recv())
            .await
            .expect("first cross-device operation did not start")
            .expect("first cross-device operation start notification was dropped");

        let second_scheduler = device_b.clone();
        let second_other_queue = queue_for(&device_a);
        let second_started = started_tx;
        let second_release = Arc::clone(&release_rx);
        let second = tokio::spawn(async move {
            second_scheduler
                .run_device_operation_with_queues(
                    IoClass::MoveCopy,
                    64,
                    vec![second_other_queue],
                    move || {
                        second_started.send(()).unwrap();
                        lock_recover(&second_release).recv().unwrap();
                        Ok(())
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let has_pending = {
                    let state = lock_recover(&device_a.device_queue.state);
                    state.pending_count != 0
                };
                if has_pending {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("opposing operation did not wait on the first ordered device");

        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), started_rx.recv())
            .await
            .expect("opposing cross-device operation deadlocked")
            .expect("opposing cross-device start notification was dropped");
        release_tx.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            first.await.unwrap().unwrap();
            second.await.unwrap().unwrap();
        })
        .await
        .expect("ordered cross-device operations did not finish");
    }

    #[tokio::test]
    async fn path_schedulers_share_bounded_worker_resources() {
        let dir = tempfile::tempdir().unwrap();
        let config = SchedulerConfig {
            profile: StorageProfile::Ssd,
            storage_io: StorageIoConfig {
                file_pool_size: 3,
                io_worker_threads: 2,
                io_queue_depth: 7,
                hash_worker_threads: 1,
                hash_queue_depth: 5,
                ..Default::default()
            },
            ..Default::default()
        };
        let first = MountScheduler::new_for_path(StorageRootId::new(), dir.path(), &config);
        let second = MountScheduler::new_for_path(StorageRootId::new(), dir.path(), &config);

        assert!(Arc::ptr_eq(&first.file_pool, &second.file_pool));
        assert!(Arc::ptr_eq(&first.io_pool, &second.io_pool));
        assert!(Arc::ptr_eq(&first.disk_backend, &second.disk_backend));
        assert!(Arc::ptr_eq(&first.hash_pool, &second.hash_pool));
        let first_stats = first.stats();
        let second_stats = second.stats();
        assert_ne!(first_stats.shared_resource_id, 0);
        assert_eq!(
            first_stats.shared_resource_id,
            second_stats.shared_resource_id
        );
        assert_eq!(
            first.file_pool_stats().capacity,
            effective_file_pool_size(3, process_handle_cache_capacity())
        );
    }

    #[test]
    fn active_backend_snapshot_reports_live_path_scheduler() {
        let dir = tempfile::tempdir().unwrap();
        let config = SchedulerConfig::default();
        let scheduler = MountScheduler::new_for_path(StorageRootId::new(), dir.path(), &config);
        let mut expected = active_storage_backend(&scheduler.disk_backend);
        expected.scheduler_group_id = scheduler.stats().shared_resource_id;

        let snapshot = active_storage_backend_snapshot();

        assert!(snapshot.scheduler_groups > 0);
        assert!(snapshot.backends.contains(&expected));
    }

    #[test]
    fn ssd_has_higher_concurrency() {
        let hdd = hdd_scheduler();
        let ssd = ssd_scheduler();
        assert!(
            ssd.available_permits(IoClass::PeerRead) > hdd.available_permits(IoClass::PeerRead)
        );
    }

    #[test]
    fn auto_preallocation_policy_uses_full_only_for_non_cow_hdd() {
        let hdd = StorageTopology {
            device_id: Some(DeviceId("sda".to_owned())),
            profile: StorageProfile::Hdd,
            fs_type: Some("xfs".to_owned()),
            cow: false,
        };
        let hdd_cow = StorageTopology {
            device_id: Some(DeviceId("sdb".to_owned())),
            profile: StorageProfile::Hdd,
            fs_type: Some("btrfs".to_owned()),
            cow: true,
        };
        let nvme = StorageTopology {
            device_id: Some(DeviceId("nvme0n1".to_owned())),
            profile: StorageProfile::Nvme,
            fs_type: Some("ext4".to_owned()),
            cow: false,
        };

        assert_eq!(
            preallocation_mode_for_topology(Some(&hdd)),
            PreallocationMode::Full
        );
        assert_eq!(
            preallocation_mode_for_topology(Some(&hdd_cow)),
            PreallocationMode::Sparse
        );
        assert_eq!(
            preallocation_mode_for_topology(Some(&nvme)),
            PreallocationMode::Sparse
        );
        assert_eq!(
            preallocation_mode_for_topology(None),
            PreallocationMode::Sparse
        );
    }

    #[test]
    fn scheduler_new_resolves_auto_to_sparse_without_path_topology() {
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    preallocation_mode: PreallocationMode::Auto,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        assert_eq!(
            sched.io_config().preallocation_mode,
            PreallocationMode::Sparse
        );
    }

    #[tokio::test]
    async fn read_nonexistent_file_does_not_create() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.bin");
        let sched = hdd_scheduler();
        let result = scheduled_read(&sched, IoClass::Foreground, &path, 0, 16).await;
        assert!(matches!(result, Err(StorageError::FileNotFound { .. })));
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn short_positioned_read_maps_to_storage_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short-read.bin");
        std::fs::write(&path, b"tiny").unwrap();
        let sched = hdd_scheduler();

        let result = scheduled_read(&sched, IoClass::Foreground, &path, 0, 8).await;

        assert!(
            matches!(
                result,
                Err(StorageError::ShortIo {
                    expected: 8,
                    actual: 0,
                    ..
                })
            ),
            "unexpected short positioned read result: {result:?}"
        );
    }

    #[tokio::test]
    async fn read_and_write_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.bin");
        std::fs::write(&path, b"hello world!").unwrap();
        let sched = ssd_scheduler();
        let data = scheduled_read(&sched, IoClass::Foreground, &path, 6, 5)
            .await
            .unwrap();
        assert_eq!(&data[..], b"world");
    }

    #[tokio::test]
    async fn write_does_not_create_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("does_not_exist.bin");
        let sched = hdd_scheduler();
        let result = scheduled_write(
            &sched,
            IoClass::PeerWrite,
            &path,
            0,
            bytes::Bytes::from_static(b"data"),
            false,
        )
        .await;
        assert!(result.is_err());
        assert!(!path.exists());
        assert_storage_admission_released(&sched);
    }

    #[tokio::test]
    async fn file_range_overflow_is_rejected_before_storage_access() {
        let sched = ssd_scheduler();
        let path = PathBuf::from("range-overflow.bin");

        let read = sched.read_at(IoClass::Foreground, &path, u64::MAX, 1).await;
        assert!(matches!(
            read,
            Err(StorageError::Io { source, .. })
                if source.kind() == io::ErrorKind::InvalidInput
        ));

        let write = sched
            .write_at(
                IoClass::Foreground,
                &path,
                u64::MAX,
                bytes::Bytes::from_static(b"x"),
                true,
            )
            .await;
        assert!(matches!(
            write,
            Err(StorageError::Io { source, .. })
                if source.kind() == io::ErrorKind::InvalidInput
        ));

        let extents = sched.data_extents(&path, u64::MAX, 1).await;
        assert!(matches!(
            extents,
            Err(StorageError::Io { source, .. })
                if source.kind() == io::ErrorKind::InvalidInput
        ));
    }

    #[tokio::test]
    async fn file_pool_records_hits_and_evictions() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.bin");
        let b = dir.path().join("b.bin");
        let c = dir.path().join("c.bin");
        std::fs::write(&a, b"aaaa").unwrap();
        std::fs::write(&b, b"bbbb").unwrap();
        std::fs::write(&c, b"cccc").unwrap();
        let sched = hdd_scheduler();

        scheduled_read(&sched, IoClass::Foreground, &a, 0, 1)
            .await
            .unwrap();
        scheduled_read(&sched, IoClass::Foreground, &a, 1, 1)
            .await
            .unwrap();
        scheduled_read(&sched, IoClass::Foreground, &b, 0, 1)
            .await
            .unwrap();
        scheduled_read(&sched, IoClass::Foreground, &c, 0, 1)
            .await
            .unwrap();

        let stats = sched.file_pool_stats();
        assert!(stats.hits >= 1);
        assert!(stats.misses >= 3);
        assert!(stats.evictions >= 1);
        assert_eq!(stats.open_files, 2);
    }

    #[test]
    fn configured_file_pool_is_clamped_to_the_process_fd_budget() {
        assert_eq!(effective_file_pool_size(512, 6), 6);
        assert_eq!(effective_file_pool_size(3, 6), 3);
        assert_eq!(effective_file_pool_size(512, 0), 0);
    }

    #[test]
    fn zero_capacity_file_pool_opens_without_retaining_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uncached.bin");
        std::fs::write(&path, b"data").unwrap();
        let pool = FilePool::new(0, Duration::from_secs(30));

        let file = pool.get_or_open(&path, OpenMode::Read, false).unwrap();

        assert_eq!(file.metadata().unwrap().len(), 4);
        assert_eq!(pool.active_descriptors(), 1);
        let stats = pool.stats();
        assert_eq!(stats.capacity, 0);
        assert_eq!(stats.open_files, 0);
        drop(file);
        assert_eq!(pool.active_descriptors(), 0);
    }

    #[test]
    fn file_pool_waits_for_an_evicted_in_flight_descriptor_lease() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first.bin");
        let second_path = dir.path().join("second.bin");
        std::fs::write(&first_path, b"first").unwrap();
        std::fs::write(&second_path, b"second").unwrap();
        let pool = Arc::new(FilePool::new(1, Duration::from_secs(30)));
        let first = pool
            .get_or_open(&first_path, OpenMode::Read, false)
            .unwrap();
        assert_eq!(pool.active_descriptors(), 1);

        let (opened_tx, opened_rx) = mpsc::channel();
        let opener = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                let second = pool
                    .get_or_open(&second_path, OpenMode::Read, false)
                    .unwrap();
                opened_tx.send(second).unwrap();
            })
        };

        assert!(matches!(
            opened_rx.recv_timeout(Duration::from_millis(50)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        assert_eq!(pool.active_descriptors(), 1);

        drop(first);
        let second = opened_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("descriptor waiter did not resume after the in-flight handle dropped");
        opener.join().unwrap();
        assert_eq!(second.metadata().unwrap().len(), 6);
        assert_eq!(pool.active_descriptors(), 1);
        drop(second);
        assert_eq!(pool.active_descriptors(), 1, "the cache retains one handle");
    }

    #[tokio::test]
    async fn file_pool_reopens_a_path_after_unlink_and_recreate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced.bin");
        std::fs::write(&path, b"old payload").unwrap();
        let sched = hdd_scheduler();

        assert_eq!(
            scheduled_read(&sched, IoClass::Foreground, &path, 0, 11)
                .await
                .unwrap(),
            bytes::Bytes::from_static(b"old payload")
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"new payload").unwrap();

        assert_eq!(
            scheduled_read(&sched, IoClass::Foreground, &path, 0, 11)
                .await
                .unwrap(),
            bytes::Bytes::from_static(b"new payload")
        );
    }

    #[tokio::test]
    async fn file_pool_writes_to_the_recreated_path_not_the_unlinked_inode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced-write.bin");
        std::fs::write(&path, b"old payload").unwrap();
        let sched = hdd_scheduler();

        scheduled_write(
            &sched,
            IoClass::PeerWrite,
            &path,
            0,
            bytes::Bytes::from_static(b"old payload"),
            false,
        )
        .await
        .unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"new payload").unwrap();

        scheduled_write(
            &sched,
            IoClass::PeerWrite,
            &path,
            0,
            bytes::Bytes::from_static(b"fresh data!"),
            false,
        )
        .await
        .unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"fresh data!");
    }

    #[tokio::test]
    async fn large_peer_and_recheck_reads_emit_page_cache_advice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("large.bin");
        std::fs::write(&path, vec![7u8; 512 * 1024]).unwrap();
        let sched = hdd_scheduler();

        let peer = scheduled_read(&sched, IoClass::PeerRead, &path, 0, 300 * 1024)
            .await
            .unwrap();
        assert_eq!(peer.len(), 300 * 1024);
        let recheck = scheduled_read(&sched, IoClass::Recheck, &path, 0, 300 * 1024)
            .await
            .unwrap();
        assert_eq!(recheck.len(), 300 * 1024);

        let stats = sched.stats();
        assert!(stats.page_cache_advise_sequential >= 2);
        assert!(stats.page_cache_advise_willneed >= 1);
        assert!(stats.page_cache_advise_dontneed >= 1);
    }

    #[tokio::test]
    async fn concurrent_positioned_writes_do_not_share_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("positioned.bin");
        let sched = hdd_scheduler();
        sched
            .prepare_file(&path, 8, PreallocationMode::Sparse)
            .await
            .unwrap();
        let a = scheduled_write(
            &sched,
            IoClass::PeerWrite,
            &path,
            0,
            bytes::Bytes::from_static(b"aaaa"),
            false,
        );
        let b = scheduled_write(
            &sched,
            IoClass::PeerWrite,
            &path,
            4,
            bytes::Bytes::from_static(b"bbbb"),
            false,
        );
        let (ra, rb) = tokio::join!(a, b);
        ra.unwrap();
        rb.unwrap();
        let data = scheduled_read(&sched, IoClass::Foreground, &path, 0, 8)
            .await
            .unwrap();
        assert_eq!(&data[..], b"aaaabbbb");
    }

    #[tokio::test]
    async fn sparse_prepare_creates_parent_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("file.bin");
        let sched = hdd_scheduler();
        sched
            .prepare_file(&path, 12, PreallocationMode::Sparse)
            .await
            .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 12);
    }

    #[tokio::test]
    async fn prepare_file_refuses_to_shrink_existing_data() {
        // Regression test: `prepare_file` must never truncate a file that
        // already holds more data than the requested length, whatever the
        // upstream reason for the mismatched length is (metainfo parsing
        // bug, stale hint, wrong path, ...). Silent truncation here is what
        // destroys already-downloaded data on import.
        for mode in [
            PreallocationMode::Auto,
            PreallocationMode::Sparse,
            PreallocationMode::Full,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("complete.bin");
            let original = vec![0xABu8; 4096];
            std::fs::write(&path, &original).unwrap();
            let sched = hdd_scheduler();

            let err = sched
                .prepare_file(&path, 1024, mode)
                .await
                .expect_err("shrink must be refused");
            assert!(
                matches!(
                    err,
                    StorageError::RefuseShrink {
                        current_len: 4096,
                        requested_len: 1024,
                        ..
                    }
                ),
                "unexpected error for {mode:?}: {err:?}"
            );

            let stats = sched.stats();
            assert_eq!(
                stats.preallocation_shrink_refused, 1,
                "shrink-refused counter should record the refusal for {mode:?}"
            );

            let on_disk = std::fs::read(&path).unwrap();
            assert_eq!(
                on_disk, original,
                "file bytes must be untouched after a refused shrink for {mode:?}"
            );
        }
    }

    /// The reason `full_preallocate` uses raw `fallocate(2)`: preallocated
    /// but never-written space must stay *visibly* unwritten to the
    /// filesystem, so the allocation audit can flag it after a crash. If this
    /// ever regressed to zero-filling (glibc's `posix_fallocate` emulation),
    /// the audit would see ordinary data blocks and the guarantee would be
    /// silently gone.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn full_preallocation_leaves_unwritten_extents_not_zero_blocks() {
        use crate::alloc_audit::{audit_file_allocation, AllocationAudit, AuditMethod};

        let len = 8u64 << 20;
        // Use the working directory's filesystem: /tmp is often tmpfs, which
        // cannot report unwritten extents.
        let dir = tempfile::Builder::new()
            .prefix("prealloc-audit-")
            .tempdir_in(std::env::current_dir().unwrap())
            .unwrap();
        let path = dir.path().join("prealloc.bin");
        let sched = hdd_scheduler();
        sched
            .prepare_file(&path, len, PreallocationMode::Full)
            .await
            .unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), len);

        let fs_type = crate::device::detect_storage_topology(&path).fs_type;
        let extent_fs = matches!(fs_type.as_deref(), Some("ext4" | "xfs" | "btrfs"));
        let stats = sched.stats();
        match audit_file_allocation(&path, len).unwrap() {
            AllocationAudit::Gaps {
                gaps,
                method: AuditMethod::Fiemap,
            } if extent_fs => {
                assert_eq!(stats.preallocation_fallbacks, 0);
                assert_eq!(
                    gaps.merged(),
                    vec![(0, len)],
                    "full preallocation on {fs_type:?} must remain unwritten, not zero-filled"
                );
            }
            other => eprintln!("fs {fs_type:?} exposes no unwritten extents ({other:?}); skipping"),
        }
    }

    #[tokio::test]
    async fn prepare_file_extends_without_disturbing_existing_bytes() {
        // A length request >= current size (normal preallocation for a
        // growing/partial file) must still work exactly as before.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial.bin");
        let original = vec![0xCDu8; 1024];
        std::fs::write(&path, &original).unwrap();
        let sched = hdd_scheduler();

        sched
            .prepare_file(&path, 4096, PreallocationMode::Sparse)
            .await
            .unwrap();

        let on_disk = std::fs::read(&path).unwrap();
        assert_eq!(on_disk.len(), 4096);
        assert_eq!(&on_disk[..1024], &original[..]);
        assert_eq!(sched.stats().preallocation_shrink_refused, 0);
    }

    #[tokio::test]
    async fn stats_track_io_sync_and_hash_work() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("stats.bin");
        let sched = hdd_scheduler();

        scheduled_write(
            &sched,
            IoClass::PeerWrite,
            &path,
            0,
            bytes::Bytes::from_static(b"abcd"),
            true,
        )
        .await
        .unwrap();
        let data = scheduled_read(&sched, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let hash = sched.hash_sha1(data).await.unwrap();
        assert_ne!(hash, [0; 20]);
        sched.sync_all_open_files().await.unwrap();

        let stats = sched.stats();
        assert_eq!(stats.write_ops_by_class[class_index(IoClass::PeerWrite)], 1);
        assert_eq!(stats.read_ops_by_class[class_index(IoClass::PeerRead)], 1);
        assert_eq!(
            stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            1
        );
        assert_eq!(
            stats.bytes_written_by_class[class_index(IoClass::PeerWrite)],
            4
        );
        assert_eq!(stats.bytes_read_by_class[class_index(IoClass::PeerRead)], 4);
        assert_eq!(
            stats.backend_bytes_read_by_class[class_index(IoClass::PeerRead)],
            4
        );
        assert!(stats.read_latency_ns_by_class[class_index(IoClass::PeerRead)] > 0);
        assert!(stats.write_latency_ns_by_class[class_index(IoClass::PeerWrite)] > 0);
        assert_eq!(
            stats.read_latency_buckets[STORAGE_LATENCY_BUCKET_COUNT - 1],
            1
        );
        assert_eq!(
            stats.write_latency_buckets[STORAGE_LATENCY_BUCKET_COUNT - 1],
            1
        );
        assert_eq!(stats.hash_ops, 1);
        assert!(stats.hash_latency_ns > 0);
        assert_eq!(
            stats.hash_latency_buckets[STORAGE_LATENCY_BUCKET_COUNT - 1],
            1
        );
        assert_eq!(stats.queued_disk_bytes, 0);
        assert_eq!(stats.sync_ops, 1);
        assert!(stats.sync_latency_ns > 0);
        assert_eq!(
            stats.sync_latency_buckets[STORAGE_LATENCY_BUCKET_COUNT - 1],
            1
        );
        assert_eq!(stats.dirty_files, 0);
    }

    #[tokio::test]
    async fn hybrid_piece_hashes_share_payload_and_separate_padding_views() {
        let content: Vec<u8> = (0..(V2_BLOCK_SIZE + 17))
            .map(|index| (index % 251) as u8)
            .collect();
        let piece_length = 4 * V2_BLOCK_SIZE;
        let mut piece = content.clone();
        piece.resize(piece_length, 0);
        let expected_v1: [u8; 20] = Sha1::digest(&piece).into();
        let v2_leaves: Vec<[u8; 32]> = content
            .chunks(V2_BLOCK_SIZE)
            .map(|leaf| BlockHash::of(leaf).0)
            .collect();
        let expected_file_root = merkle_root(&v2_leaves);
        let expected_piece_root = rt_hash::piece_layer_hash(&content, piece_length).unwrap();
        let scheduler = hdd_scheduler();

        let hashes = scheduler
            .hash_hybrid_piece(bytes::Bytes::from(piece), content.len(), piece_length)
            .await
            .unwrap();

        assert_eq!(hashes.v1_sha1, expected_v1);
        assert_eq!(hashes.v2_file_root, expected_file_root);
        assert_eq!(hashes.v2_piece_root, expected_piece_root);
        assert_eq!(scheduler.stats().hash_ops, 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_queued_hybrid_hash_retains_shared_payload_lease_until_dequeued() {
        let resources = queued_disk_governor(1024 * 1024);
        let scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Ssd,
                storage_io: StorageIoConfig {
                    file_pool_size: 37,
                    idle_file_ttl_secs: 3601,
                    io_worker_threads: 1,
                    io_queue_depth: 8,
                    hash_worker_threads: 1,
                    hash_queue_depth: 1,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::spawn({
            let pool = scheduler.hash_pool.clone();
            async move {
                pool.run(move || {
                    started_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok::<_, StorageError>(())
                })
                .await
            }
        });
        started_rx.recv().unwrap();

        let data = bytes::Bytes::from(vec![0x5a; V2_BLOCK_SIZE * 2]);
        let payload_len = data.len() as u64;
        let payload_lease = Arc::new(
            resources
                .try_acquire(MemoryClass::PieceAssembly, payload_len)
                .unwrap(),
        );
        let hash = tokio::spawn({
            let scheduler = scheduler.clone();
            async move {
                scheduler
                    .hash_hybrid_piece_retry_queue_full_with_shared_payload_lease(
                        data,
                        V2_BLOCK_SIZE * 2,
                        V2_BLOCK_SIZE * 2,
                        payload_lease,
                    )
                    .await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.hash_pool.queued() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("hybrid hash did not reach the bounded worker queue");
        assert_eq!(scheduler.stats().queued_disk_bytes, payload_len);
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PieceAssembly as usize].used_bytes,
            payload_len
        );

        hash.abort();
        assert!(hash.await.unwrap_err().is_cancelled());
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES,
            "canceling the waiter must not release queue overhead owned by a queued closure"
        );
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PieceAssembly as usize].used_bytes,
            payload_len,
            "canceling the waiter must not release the shared payload lease"
        );

        release_tx.send(()).unwrap();
        blocker.await.unwrap().unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while queued_disk_used_bytes(&resources) != 0
                || resources.snapshot().classes[MemoryClass::PieceAssembly as usize].used_bytes != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("canceled queued hash did not release its bytes after dequeue");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn verifier_retries_transient_hash_queue_full() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hash-queue.bin");
        let content = vec![0x5au8; 1024 * 1024];
        std::fs::write(&path, &content).unwrap();

        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Ssd,
                storage_io: StorageIoConfig {
                    file_pool_size: 3,
                    idle_file_ttl_secs: 3601,
                    io_worker_threads: 2,
                    io_queue_depth: 8,
                    hash_worker_threads: 1,
                    hash_queue_depth: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let (first_started_tx, first_started_rx) = std::sync::mpsc::channel();
        let (first_release_tx, first_release_rx) = std::sync::mpsc::channel();
        let first = tokio::spawn({
            let pool = sched.hash_pool.clone();
            async move {
                pool.run(move || {
                    first_started_tx.send(()).unwrap();
                    first_release_rx.recv().unwrap();
                    Ok::<(), StorageError>(())
                })
                .await
            }
        });
        first_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        let (second_started_tx, second_started_rx) = std::sync::mpsc::channel();
        let (second_release_tx, second_release_rx) = std::sync::mpsc::channel();
        let second = tokio::spawn({
            let pool = sched.hash_pool.clone();
            async move {
                pool.run(move || {
                    second_started_tx.send(()).unwrap();
                    second_release_rx.recv().unwrap();
                    Ok::<(), StorageError>(())
                })
                .await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while sched.hash_pool.queued() != 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("hash worker queue was not saturated");

        let expected_hash: [u8; 20] = Sha1::digest(&content).into();
        let complete_piece_hash = tokio::spawn({
            let sched = sched.clone();
            let data = bytes::Bytes::from(content.clone());
            async move { sched.hash_sha1_retry_queue_full(data).await }
        });

        let piece_map = PieceMap::new(
            content.len() as u64,
            vec![FileSpan {
                file_index: 0,
                path: SafeRelPath::from_name("hash-queue.bin", false).unwrap(),
                content_offset: 0,
                length: content.len() as u64,
            }],
        )
        .unwrap();
        let storage_root = dir.path().to_path_buf();
        let verify_task = tokio::spawn({
            let sched = sched.clone();
            async move {
                let expected = [expected_hash];
                let verifier = PieceVerifier::new(&storage_root, &sched, &piece_map, &expected);
                verifier.verify_piece(0).await
            }
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            while sched.stats().queue_full == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("verification did not observe the saturated hash queue");

        first_release_tx.send(()).unwrap();
        first.await.unwrap().unwrap();
        second_started_rx
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        second_release_tx.send(()).unwrap();
        second.await.unwrap().unwrap();

        assert_eq!(complete_piece_hash.await.unwrap().unwrap(), expected_hash);
        assert_eq!(verify_task.await.unwrap(), VerifyResult::Valid);
    }

    #[tokio::test]
    async fn sync_all_open_files_syncs_dirty_paths_after_fd_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.bin");
        let second = dir.path().join("second.bin");
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    file_pool_size: 1,
                    durability_mode: DurabilityMode::Checkpoint,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        sched
            .write_at(
                IoClass::PeerWrite,
                &first,
                0,
                bytes::Bytes::from_static(b"dirty-a"),
                true,
            )
            .await
            .unwrap();
        sched
            .write_at(
                IoClass::PeerWrite,
                &second,
                0,
                bytes::Bytes::from_static(b"dirty-b"),
                true,
            )
            .await
            .unwrap();

        let before_sync = sched.stats();
        assert_eq!(before_sync.file_pool.capacity, 1);
        assert_eq!(before_sync.file_pool.open_files, 1);
        assert!(before_sync.file_pool.evictions >= 1);
        assert_eq!(before_sync.dirty_files, 2);

        sched.sync_all_open_files().await.unwrap();

        let after_sync = sched.stats();
        assert_eq!(after_sync.dirty_files, 0);
        assert_eq!(after_sync.sync_ops, 1);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn sync_all_open_files_stays_within_low_descriptor_limit() {
        const CHILD_ENV: &str = "TNG_TEST_SYNC_ALL_OPEN_FILES_LOW_FD_CHILD";

        if std::env::var_os(CHILD_ENV).is_some() {
            let mut current = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: `current` is a valid writable `rlimit` for getrlimit.
            assert_eq!(
                unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut current) },
                0
            );
            let limit = current.rlim_max.clamp(1, 64);
            let constrained = libc::rlimit {
                rlim_cur: limit,
                rlim_max: limit,
            };
            // SAFETY: lowering this child process's soft and hard descriptor
            // limits is local to the subprocess; it cannot affect the parent.
            assert_eq!(
                unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &constrained) },
                0
            );
            let dir = tempfile::tempdir().unwrap();
            let sched = MountScheduler::new(
                StorageRootId::new(),
                &SchedulerConfig {
                    profile: StorageProfile::Ssd,
                    storage_io: StorageIoConfig {
                        file_pool_size: 1,
                        io_worker_threads: 1,
                        io_queue_depth: 8,
                        hash_worker_threads: 1,
                        hash_queue_depth: 8,
                        durability_mode: DurabilityMode::Checkpoint,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );
            for index in 0..96 {
                let path = dir.path().join(format!("dirty-{index}.bin"));
                scheduled_write(
                    &sched,
                    IoClass::PeerWrite,
                    &path,
                    0,
                    bytes::Bytes::from_static(b"payload"),
                    true,
                )
                .await
                .unwrap_or_else(|error| panic!("write {path:?} failed: {error}"));
            }
            assert_eq!(sched.stats().dirty_files, 96);
            sched.sync_all_open_files().await.unwrap();
            assert_eq!(sched.stats().dirty_files, 0);
            return;
        }

        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "scheduler::tests::sync_all_open_files_stays_within_low_descriptor_limit",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .env("TNG_STORAGE_BACKEND", "pread")
            .output()
            .unwrap();
        let transcript = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success() && transcript.contains("1 passed"),
            "low-fd child failed or did not run the test:\n{transcript}"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn concurrent_file_pool_opens_stay_within_low_descriptor_limit() {
        const CHILD_ENV: &str = "TNG_TEST_FILE_POOL_CONCURRENT_LOW_FD_CHILD";

        if std::env::var_os(CHILD_ENV).is_some() {
            let mut current = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            // SAFETY: `current` is a valid writable `rlimit` for getrlimit.
            assert_eq!(
                unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut current) },
                0
            );
            let limit = current.rlim_max.clamp(1, 64);
            let constrained = libc::rlimit {
                rlim_cur: limit,
                rlim_max: limit,
            };
            // SAFETY: lowering this child process's limits cannot affect the
            // parent test process.
            assert_eq!(
                unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &constrained) },
                0
            );

            const OPERATIONS: usize = 72;
            const ACTIVE_LIMIT: usize = 8;
            let dir = tempfile::tempdir().unwrap();
            let paths = (0..OPERATIONS)
                .map(|index| {
                    let path = dir.path().join(format!("concurrent-{index}.bin"));
                    std::fs::write(&path, b"payload").unwrap();
                    path
                })
                .collect::<Vec<_>>();
            let pool = Arc::new(FilePool::new(ACTIVE_LIMIT, Duration::from_secs(30)));
            let start = Arc::new(std::sync::Barrier::new(OPERATIONS));
            let max_active = Arc::new(AtomicUsize::new(0));
            let workers = paths
                .into_iter()
                .map(|path| {
                    let pool = Arc::clone(&pool);
                    let start = Arc::clone(&start);
                    let max_active = Arc::clone(&max_active);
                    std::thread::spawn(move || {
                        start.wait();
                        let file = pool
                            .get_or_open(&path, OpenMode::Read, false)
                            .unwrap_or_else(|error| panic!("open {path:?} failed: {error}"));
                        max_active.fetch_max(pool.active_descriptors(), Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(30));
                        assert_eq!(file.metadata().unwrap().len(), 7);
                    })
                })
                .collect::<Vec<_>>();

            for worker in workers {
                worker.join().expect("file-pool worker panicked");
            }
            assert!(max_active.load(Ordering::Relaxed) <= ACTIVE_LIMIT);
            assert!(pool.active_descriptors() <= ACTIVE_LIMIT);
            assert!(pool.stats().open_files <= ACTIVE_LIMIT);
            return;
        }

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "scheduler::tests::concurrent_file_pool_opens_stay_within_low_descriptor_limit",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let output = child.wait_with_output().unwrap();
                let transcript = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );
                panic!("concurrent low-fd child timed out:\n{transcript}");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let output = child.wait_with_output().unwrap();
        let transcript = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.status.success() && transcript.contains("1 passed"),
            "concurrent low-fd child failed or did not run the test:\n{transcript}"
        );
    }

    #[test]
    fn write_handle_snapshot_drops_replaced_paths() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced.bin");
        std::fs::write(&path, b"old").unwrap();
        let pool = FilePool::new(16, Duration::from_secs(30));

        let old = pool.get_or_open(&path, OpenMode::Write, false).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"new-content").unwrap();

        assert!(pool.write_paths().is_empty());
        let current = pool.open_for_sync(&path).unwrap();
        assert_ne!(
            file_identity(&old).unwrap(),
            file_identity(&current).unwrap()
        );
        assert_eq!(current.metadata().unwrap().len(), 11);
    }

    #[test]
    fn poisoned_file_pool_discards_cached_handles_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("poisoned-pool.bin");
        std::fs::write(&path, b"payload").unwrap();
        let pool = Arc::new(FilePool::new(16, Duration::from_secs(30)));
        let old = pool.get_or_open(&path, OpenMode::Read, false).unwrap();

        pool.catalog.poison_for_test();
        assert!(pool.catalog.cache_is_poisoned());

        let reopened = pool.get_or_open(&path, OpenMode::Read, false).unwrap();
        assert!(!Arc::ptr_eq(&old, &reopened));
        assert!(!pool.catalog.cache_is_poisoned());
    }

    #[test]
    fn poisoned_peer_read_cache_is_discarded_as_a_miss() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("poisoned-peer-cache.bin");
        std::fs::write(&path, b"fresh").unwrap();
        let file = File::open(&path).unwrap();
        let cache = Arc::new(Mutex::new(PeerReadCache::default()));
        assert!(cache
            .lock()
            .unwrap()
            .insert(
                path.clone(),
                PeerReadCacheEntry {
                    source: Weak::new(),
                    identity: file_identity(&file).unwrap(),
                    offset: 0,
                    data: bytes::Bytes::from_static(b"stale"),
                    consumed_prefix: 0,
                    consumed_suffix: 5,
                    consumed_granules: 0,
                    last_used: Instant::now(),
                },
                1,
                1024,
            )
            .is_some());

        let poisoner = Arc::clone(&cache);
        assert!(std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the derived peer read cache");
        })
        .join()
        .is_err());

        assert!(peer_read_cache_hit(&cache, &path, file_identity(&file).unwrap(), 0, 5).is_none());
        assert!(!cache.is_poisoned());
        assert!(lock_reset_derived_cache(&cache).files.is_empty());
    }

    #[test]
    fn operation_record_clears_only_its_admitted_content_epoch() {
        let dirty_paths = Mutex::new(HashMap::new());
        let dirty_path_epoch = AtomicU64::new(0);
        let path = PathBuf::from("payload.bin");
        let identity = test_file_identity();

        let first_state =
            mark_dirty_path(&dirty_paths, &dirty_path_epoch, path.clone(), identity, 1);
        let mut first_operation = StorageOperationRecord {
            storage_operation_id: Some(StorageOperationId(10)),
            device_operation_id: Some(DeviceOperationId(1)),
            device_queue_key: Some(Arc::from("device:test")),
            ..StorageOperationRecord::default()
        };
        first_operation.bind_byte_range(0, 16);
        first_operation.bind_dirty_path(first_state);
        mark_dirty_path(&dirty_paths, &dirty_path_epoch, path.clone(), identity, 2);

        clear_dirty_path_if_operation(&dirty_paths, &path, &first_operation);
        assert_eq!(dirty_paths.lock().unwrap().len(), 1);

        let second_state = dirty_paths.lock().unwrap().get(&path).copied().unwrap();
        let mut unadmitted_operation = StorageOperationRecord::default();
        unadmitted_operation.bind_dirty_path(second_state);
        clear_dirty_path_if_operation(&dirty_paths, &path, &unadmitted_operation);
        assert_eq!(dirty_paths.lock().unwrap().len(), 1);

        let mut stale_write_sequence = StorageOperationRecord {
            storage_operation_id: Some(StorageOperationId(12)),
            device_operation_id: Some(DeviceOperationId(3)),
            device_queue_key: Some(Arc::from("device:test")),
            ..StorageOperationRecord::default()
        };
        stale_write_sequence.bind_dirty_path(second_state);
        stale_write_sequence.write_sequence = Some(1);
        clear_dirty_path_if_operation(&dirty_paths, &path, &stale_write_sequence);
        assert_eq!(dirty_paths.lock().unwrap().len(), 1);

        let mut second_operation = StorageOperationRecord {
            storage_operation_id: Some(StorageOperationId(11)),
            device_operation_id: Some(DeviceOperationId(2)),
            device_queue_key: Some(Arc::from("device:test")),
            ..StorageOperationRecord::default()
        };
        second_operation.bind_byte_range(0, 16);
        second_operation.bind_dirty_path(second_state);
        clear_dirty_path_if_operation(&dirty_paths, &path, &second_operation);
        assert!(dirty_paths.lock().unwrap().is_empty());
        assert_eq!(
            second_operation.byte_range,
            Some(StorageByteRange {
                offset: 0,
                length: 16
            })
        );
    }

    #[test]
    fn storage_operation_ids_are_process_wide_and_monotonic() {
        let first = allocate_storage_operation_id().unwrap();
        let second = allocate_storage_operation_id().unwrap();
        assert!(second.0 > first.0);
    }

    #[test]
    fn poisoned_dirty_path_state_is_preserved_and_unpoisoned() {
        let original_path = PathBuf::from("already-dirty.bin");
        let new_path = PathBuf::from("new-dirty.bin");
        let identity = test_file_identity();
        let original_state = DirtyPathState {
            content_epoch: ContentEpoch(7),
            identity,
            write_sequence: 8,
        };
        let dirty_paths = Arc::new(Mutex::new(HashMap::from([(
            original_path.clone(),
            original_state,
        )])));

        let poisoner = Arc::clone(&dirty_paths);
        assert!(std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison authoritative dirty-path state");
        })
        .join()
        .is_err());

        mark_dirty_path(
            &dirty_paths,
            &AtomicU64::new(8),
            new_path.clone(),
            identity,
            9,
        );
        let dirty = lock_recover(&dirty_paths);
        assert_eq!(dirty.get(&original_path), Some(&original_state));
        assert!(dirty.contains_key(&new_path));
        assert!(!dirty_paths.is_poisoned());
    }

    #[tokio::test]
    async fn strict_write_sync_is_counted_and_not_left_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("strict.bin");
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Ssd,
                storage_io: StorageIoConfig {
                    durability_mode: DurabilityMode::Strict,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        scheduled_write(
            &sched,
            IoClass::PeerWrite,
            &path,
            0,
            bytes::Bytes::from_static(b"strict"),
            true,
        )
        .await
        .unwrap();

        let stats = sched.stats();
        assert_eq!(stats.dirty_files, 0);
        assert_eq!(stats.sync_ops, 1);
        assert!(stats.sync_latency_ns > 0);
    }

    #[tokio::test]
    async fn peer_read_readahead_cache_returns_exact_requested_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readahead.bin");
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 16,
                    peer_read_readahead_sequential_only: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let first = scheduled_read(&sched, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        assert_eq!(&first[..], b"abcd");
        let second = scheduled_read(&sched, IoClass::PeerRead, &path, 4, 4)
            .await
            .unwrap();
        assert_eq!(&second[..], b"efgh");

        let stats = sched.stats();
        assert_eq!(stats.peer_read_cache_misses, 1);
        assert_eq!(stats.peer_read_cache_hits, 1);
        assert_eq!(stats.peer_read_cache_entries, 1);
        assert_eq!(stats.peer_read_cache_bytes, 16);
        assert_eq!(stats.bytes_read_by_class[class_index(IoClass::PeerRead)], 8);
        assert_eq!(
            stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            1
        );
        assert_eq!(
            stats.backend_bytes_read_by_class[class_index(IoClass::PeerRead)],
            16
        );
    }

    #[tokio::test]
    async fn peer_read_readahead_window_is_removed_after_its_last_bytes_are_served() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consumed-readahead.bin");
        std::fs::write(&path, b"abcdefghijklmnop").unwrap();
        let scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 16,
                    peer_read_readahead_sequential_only: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let first = scheduled_read(&scheduler, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let middle = scheduled_read(&scheduler, IoClass::PeerRead, &path, 4, 4)
            .await
            .unwrap();
        assert_eq!(scheduler.stats().peer_read_cache_entries, 1);
        let last = scheduled_read(&scheduler, IoClass::PeerRead, &path, 8, 8)
            .await
            .unwrap();

        assert_eq!(&first[..], b"abcd");
        assert_eq!(&middle[..], b"efgh");
        assert_eq!(&last[..], b"ijklmnop");
        let stats = scheduler.stats();
        assert_eq!(stats.peer_read_cache_hits, 2);
        assert_eq!(stats.peer_read_cache_entries, 0);
        assert_eq!(stats.peer_read_cache_bytes, 0);
        assert_eq!(
            stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            1
        );
    }

    #[tokio::test]
    async fn peer_read_readahead_tail_hit_preserves_unread_prefix_and_middle() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tail-first-readahead.bin");
        std::fs::write(&path, b"abcdefghijklmnop").unwrap();
        let scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 16,
                    peer_read_readahead_sequential_only: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let first = scheduled_read(&scheduler, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let duplicate = scheduled_read(&scheduler, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let tail = scheduled_read(&scheduler, IoClass::PeerRead, &path, 12, 4)
            .await
            .unwrap();
        let after_tail = scheduler.stats();
        assert_eq!(after_tail.peer_read_cache_entries, 1);
        assert_eq!(after_tail.peer_read_cache_hits, 2);
        assert_eq!(
            after_tail.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            1
        );

        // A second reader can consume the remaining contiguous prefix. The
        // earlier tail hit must leave all unread bytes available here.
        let middle = scheduled_read(&scheduler, IoClass::PeerRead, &path, 4, 8)
            .await
            .unwrap();
        assert_eq!(&first[..], b"abcd");
        assert_eq!(&duplicate[..], b"abcd");
        assert_eq!(&tail[..], b"mnop");
        assert_eq!(&middle[..], b"efghijkl");
        let after_middle = scheduler.stats();
        assert_eq!(after_middle.peer_read_cache_hits, 3);
        assert_eq!(after_middle.peer_read_cache_entries, 0);
        assert_eq!(after_middle.peer_read_cache_bytes, 0);
        assert_eq!(
            after_middle.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            1
        );
    }

    fn saturated_pread_test_scheduler() -> MountScheduler {
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 16,
                    peer_read_readahead_sequential_only: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::select(BackendRequest::Pread, 1));
        scheduler.device_queue = Arc::new(DeviceQueue::new(Arc::from("pread-test"), 2, 1));
        scheduler.device_queue_capacity = 2;
        scheduler
    }

    async fn wait_for_pending_device_read(scheduler: &MountScheduler) {
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if lock_recover(&scheduler.device_queue.state).pending_count == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("cache miss must reach async device admission");
    }

    #[tokio::test]
    async fn portable_cached_read_does_not_wait_for_or_reserve_a_device_slot() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cached.bin");
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        let scheduler = saturated_pread_test_scheduler();
        scheduled_read(&scheduler, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let held = scheduler
            .acquire_device_operation(IoClass::PeerWrite)
            .await
            .unwrap();
        let next_id = lock_recover(&scheduler.device_queue.state).next_operation_id;
        let descriptor_hits = scheduler.file_pool.stats().hits;
        assert!(lock_recover(&scheduler.peer_read_elevator).is_none());
        let bytes = tokio::time::timeout(
            Duration::from_secs(2),
            scheduled_read(&scheduler, IoClass::PeerRead, &path, 4, 4),
        )
        .await
        .expect("cached data must not wait for disk capacity")
        .unwrap();
        assert_eq!(&bytes[..], b"efgh");
        assert_eq!(scheduler.file_pool.stats().hits, descriptor_hits);
        assert!(lock_recover(&scheduler.peer_read_elevator).is_none());
        let state = lock_recover(&scheduler.device_queue.state);
        assert_eq!(state.active, 1);
        assert_eq!(state.pending_count, 0);
        assert_eq!(state.next_operation_id, next_id);
        drop(state);
        assert_eq!(
            scheduler.stats().backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            1
        );
        drop(held);
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn cached_payload_does_not_pin_descriptors_or_reuse_a_closed_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("closed-source.bin");
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        let mut scheduler = saturated_pread_test_scheduler();
        // Use a catalog owned only by this scheduler; its normal shared
        // resource bundle retains a separate process-lifetime catalog.
        scheduler.file_pool = Arc::new(FilePool::new(1, Duration::from_secs(60)));
        scheduled_read(&scheduler, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let source = lock_reset_derived_cache(&scheduler.peer_read_cache)
            .files
            .get(&path)
            .unwrap()[0]
            .source
            .clone();
        assert!(source.upgrade().is_some());
        // Close the original catalog. Cached payloads retain only a weak
        // descriptor identity and cannot prevent descriptor reclamation.
        scheduler.file_pool = Arc::new(FilePool::new(1, Duration::from_secs(60)));
        assert!(source.upgrade().is_none());
        let bytes = scheduled_read(&scheduler, IoClass::PeerRead, &path, 4, 4)
            .await
            .unwrap();
        assert_eq!(&bytes[..], b"efgh");
        let stats = scheduler.stats();
        assert_eq!(stats.peer_read_cache_hits, 0);
        assert_eq!(
            stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            2
        );
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn portable_cache_miss_waits_without_opening_a_file_or_blocking_a_worker() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("miss.bin");
        std::fs::write(&path, b"abcdefgh").unwrap();
        let scheduler = saturated_pread_test_scheduler();
        let held = scheduler
            .acquire_device_operation(IoClass::PeerWrite)
            .await
            .unwrap();
        let reader = scheduler.clone();
        let task =
            tokio::spawn(
                async move { scheduled_read(&reader, IoClass::PeerRead, &path, 0, 4).await },
            );
        wait_for_pending_device_read(&scheduler).await;
        assert_eq!(scheduler.file_pool.stats().open_files, 0);
        assert_eq!(
            scheduler.stats().backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            0
        );
        assert!(scheduler.device_queue.try_acquire_uncontended().is_none());
        tokio::time::timeout(
            Duration::from_secs(2),
            scheduler.disk_backend.run_prepared_pread(|| Ok(())),
        )
        .await
        .expect("the sole pread worker must remain runnable")
        .unwrap();
        drop(held);
        let bytes = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(&bytes[..], b"abcd");
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn cancelled_deferred_pread_releases_mount_bytes_and_device_waiter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancelled.bin");
        std::fs::write(&path, b"abcdefgh").unwrap();
        let scheduler = saturated_pread_test_scheduler();
        let held = scheduler
            .acquire_device_operation(IoClass::PeerWrite)
            .await
            .unwrap();
        let reader = scheduler.clone();
        let task =
            tokio::spawn(
                async move { scheduled_read(&reader, IoClass::PeerRead, &path, 0, 4).await },
            );
        wait_for_pending_device_read(&scheduler).await;
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert_eq!(lock_recover(&scheduler.device_queue.state).pending_count, 0);
        assert_eq!(scheduler.file_pool.stats().open_files, 0);
        drop(held);
        assert_storage_admission_released(&scheduler);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn deferred_pread_revalidates_file_replaced_during_device_wait() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced.bin");
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        let scheduler = saturated_pread_test_scheduler();
        scheduled_read(&scheduler, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let held = scheduler
            .acquire_device_operation(IoClass::PeerWrite)
            .await
            .unwrap();
        let reader = scheduler.clone();
        let read_path = path.clone();
        let task = tokio::spawn(async move {
            scheduled_read(&reader, IoClass::PeerRead, &read_path, 20, 4).await
        });
        wait_for_pending_device_read(&scheduler).await;
        let replacement = dir.path().join("replacement.bin");
        std::fs::write(&replacement, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ").unwrap();
        std::fs::rename(replacement, path).unwrap();
        drop(held);
        let bytes = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(&bytes[..], b"UVWX");
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn portable_peer_read_fuses_cache_lookup_and_backend_completion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("prepared-pread.bin");
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 16,
                    peer_read_readahead_sequential_only: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::select(BackendRequest::Pread, 1));

        let first = scheduled_read(&scheduler, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let second = scheduled_read(&scheduler, IoClass::PeerRead, &path, 4, 4)
            .await
            .unwrap();

        assert_eq!(&first[..], b"abcd");
        assert_eq!(&second[..], b"efgh");
        assert_eq!(
            scheduler
                .counters
                .prepared_pread_operations
                .load(Ordering::Relaxed),
            2
        );
        let stats = scheduler.stats();
        assert_eq!(stats.peer_read_cache_misses, 1);
        assert_eq!(stats.peer_read_cache_hits, 1);
        assert_eq!(
            stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            1
        );
    }

    #[tokio::test]
    async fn peer_read_readahead_cache_retains_disjoint_ranges_for_one_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("multi-range-readahead.bin");
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        let scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 8,
                    peer_read_readahead_sequential_only: false,
                    peer_read_cache_entries: 2,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let first = scheduled_read(&scheduler, IoClass::PeerRead, &path, 0, 4)
            .await
            .unwrap();
        let disjoint = scheduled_read(&scheduler, IoClass::PeerRead, &path, 12, 4)
            .await
            .unwrap();
        let reused = scheduled_read(&scheduler, IoClass::PeerRead, &path, 4, 4)
            .await
            .unwrap();

        assert_eq!(&first[..], b"abcd");
        assert_eq!(&disjoint[..], b"mnop");
        assert_eq!(&reused[..], b"efgh");
        let stats = scheduler.stats();
        assert_eq!(stats.peer_read_cache_entries, 1);
        assert_eq!(stats.peer_read_cache_bytes, 8);
        assert_eq!(stats.peer_read_cache_misses, 2);
        assert_eq!(stats.peer_read_cache_hits, 1);
        assert_eq!(
            stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            2
        );
    }

    #[tokio::test]
    async fn peer_read_readahead_cache_evicts_to_its_byte_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readahead-byte-budget.bin");
        std::fs::write(&path, vec![0xABu8; 64]).unwrap();
        let scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 16,
                    peer_read_readahead_sequential_only: false,
                    peer_read_cache_entries: 64,
                    peer_read_cache_bytes: 24,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        for offset in [0, 32, 0] {
            let read = scheduled_read(&scheduler, IoClass::PeerRead, &path, offset, 4)
                .await
                .unwrap();
            assert_eq!(read.len(), 4);
        }

        let stats = scheduler.stats();
        assert_eq!(stats.peer_read_cache_entries, 1);
        assert_eq!(stats.peer_read_cache_bytes, 16);
        assert_eq!(stats.peer_read_cache_misses, 3);
        assert_eq!(stats.peer_read_cache_hits, 0);
        assert_eq!(stats.peer_read_cache_evictions, 2);
    }

    #[tokio::test]
    async fn peer_read_readahead_is_admitted_for_actual_backend_buffer() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readahead-cap.bin");
        std::fs::write(&path, vec![0xABu8; 2048]).unwrap();
        let mut caps = [1024 * 1024; rt_metrics::MEMORY_CLASS_COUNT];
        caps[MemoryClass::QueuedDisk as usize] = 1024;
        let resources = ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: 1024 * 1024,
            class_caps_bytes: caps,
            ..Default::default()
        });
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Ssd,
                resources: Some(resources.clone()),
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 2048,
                    peer_read_readahead_sequential_only: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let result = sched.read_at(IoClass::PeerRead, &path, 0, 4).await;

        assert!(matches!(
            result,
            Err(StorageError::QueueFull { mount }) if mount.ends_with("-readahead")
        ));
        assert_eq!(sched.stats().queued_disk_bytes, 0);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].denied_allocations,
            1
        );
    }

    #[tokio::test]
    async fn peer_read_readahead_cache_is_invalidated_by_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("coherent.bin");
        std::fs::write(&path, b"old-data").unwrap();
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 8,
                    peer_read_readahead_sequential_only: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let first = scheduled_read(&sched, IoClass::PeerRead, &path, 0, 3)
            .await
            .unwrap();
        assert_eq!(&first[..], b"old");

        scheduled_write(
            &sched,
            IoClass::PeerWrite,
            &path,
            0,
            bytes::Bytes::from_static(b"new"),
            false,
        )
        .await
        .unwrap();

        let after_write = scheduled_read(&sched, IoClass::PeerRead, &path, 0, 3)
            .await
            .unwrap();
        assert_eq!(&after_write[..], b"new");
        assert_eq!(sched.stats().peer_read_cache_hits, 0);
    }

    #[test]
    fn dropped_write_waiter_invalidates_cache_after_backend_completion() {
        struct CompletionOwner(Arc<AtomicBool>);

        impl Drop for CompletionOwner {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cancelled-write.bin");
        std::fs::write(&path, b"payload").unwrap();
        let file = File::open(&path).unwrap();
        let key = path.clone();
        let cache = Arc::new(Mutex::new(PeerReadCache::default()));
        assert!(cache
            .lock()
            .unwrap()
            .insert(
                key.clone(),
                PeerReadCacheEntry {
                    source: Weak::new(),
                    identity: file_identity(&file).unwrap(),
                    offset: 0,
                    data: bytes::Bytes::from_static(b"stale"),
                    consumed_prefix: 0,
                    consumed_suffix: 5,
                    consumed_granules: 0,
                    last_used: Instant::now(),
                },
                1,
                1024,
            )
            .is_some());
        let epoch = Arc::new(AtomicU64::new(0));
        let owner_dropped = Arc::new(AtomicBool::new(false));
        let (caller_ownership, backend_owner) =
            backend_operation_ownership((), CompletionOwner(Arc::clone(&owner_dropped)));
        let cache_for_completion = Arc::clone(&cache);
        let epoch_for_completion = Arc::clone(&epoch);
        let key_for_completion = key.clone();
        let mut backend_owner = backend_owner.after_completion(move || {
            peer_read_cache_invalidate(
                &cache_for_completion,
                &epoch_for_completion,
                &key_for_completion,
            );
        });
        // Simulate the writer future being cancelled while the submitted
        // backend job still owns its syscall guards and invalidation callback.
        drop(caller_ownership);
        assert!(!owner_dropped.load(Ordering::Acquire));
        backend_owner.complete();
        drop(backend_owner);
        assert_eq!(epoch.load(Ordering::Acquire), 1);
        assert!(owner_dropped.load(Ordering::Acquire));
        assert!(!cache.lock().unwrap().contains_file(&key));
    }

    #[test]
    fn dropped_backend_waiter_keeps_operation_owner_until_completion() {
        struct CompletionOwner(Arc<AtomicBool>);

        impl Drop for CompletionOwner {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let owner_dropped = Arc::new(AtomicBool::new(false));
        let (caller_ownership, backend_owner) =
            backend_operation_ownership((), CompletionOwner(Arc::clone(&owner_dropped)));
        drop(caller_ownership);
        assert!(!owner_dropped.load(Ordering::Acquire));
        drop(backend_owner);
        assert!(owner_dropped.load(Ordering::Acquire));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn peer_read_readahead_cache_reopens_a_replaced_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced.bin");
        std::fs::write(&path, b"old-data").unwrap();
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 8,
                    peer_read_readahead_sequential_only: false,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let first = scheduled_read(&sched, IoClass::PeerRead, &path, 0, 3)
            .await
            .unwrap();
        assert_eq!(&first[..], b"old");

        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"new-data").unwrap();

        let replaced = scheduled_read(&sched, IoClass::PeerRead, &path, 0, 3)
            .await
            .unwrap();
        assert_eq!(&replaced[..], b"new");
        assert_eq!(sched.stats().peer_read_cache_hits, 0);
    }

    #[tokio::test]
    async fn peer_read_readahead_cache_is_config_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first.bin");
        let second_path = dir.path().join("second.bin");
        std::fs::write(&first_path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        std::fs::write(&second_path, b"ABCDEFGHIJKLMNOPQRSTUVWXYZ").unwrap();
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 16,
                    peer_read_readahead_sequential_only: false,
                    peer_read_cache_entries: 1,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        scheduled_read(&sched, IoClass::PeerRead, &first_path, 0, 4)
            .await
            .unwrap();
        scheduled_read(&sched, IoClass::PeerRead, &second_path, 0, 4)
            .await
            .unwrap();
        let first_after_eviction = scheduled_read(&sched, IoClass::PeerRead, &first_path, 4, 4)
            .await
            .unwrap();

        assert_eq!(&first_after_eviction[..], b"efgh");
        let stats = sched.stats();
        assert_eq!(stats.peer_read_cache_entries, 1);
        assert_eq!(stats.peer_read_cache_evictions, 2);
        assert_eq!(stats.peer_read_cache_misses, 3);
        assert_eq!(stats.peer_read_cache_hits, 0);
        assert_eq!(
            stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
            3
        );
    }

    #[tokio::test]
    async fn peer_read_readahead_cache_can_be_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("disabled-readahead.bin");
        std::fs::write(&path, b"abcdefghijklmnopqrstuvwxyz").unwrap();
        for (entries, cache_bytes) in [(0, 8 * 1024 * 1024), (64, 0)] {
            let sched = MountScheduler::new(
                StorageRootId::new(),
                &SchedulerConfig {
                    profile: StorageProfile::Hdd,
                    storage_io: StorageIoConfig {
                        peer_read_readahead_bytes: 16,
                        peer_read_readahead_sequential_only: false,
                        peer_read_cache_entries: entries,
                        peer_read_cache_bytes: cache_bytes,
                        ..Default::default()
                    },
                    ..Default::default()
                },
            );

            let first = scheduled_read(&sched, IoClass::PeerRead, &path, 0, 4)
                .await
                .unwrap();
            let second = scheduled_read(&sched, IoClass::PeerRead, &path, 4, 4)
                .await
                .unwrap();

            assert_eq!(&first[..], b"abcd");
            assert_eq!(&second[..], b"efgh");
            let stats = sched.stats();
            assert_eq!(stats.peer_read_cache_entries, 0);
            assert_eq!(stats.peer_read_cache_bytes, 0);
            assert_eq!(stats.peer_read_cache_evictions, 0);
            assert_eq!(stats.peer_read_cache_hits, 0);
            assert_eq!(stats.peer_read_cache_misses, 0);
            assert_eq!(
                stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)],
                2
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn hdd_peer_read_elevator_batches_shuffled_adjacent_reads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("elevator.bin");
        let block_len = 1024usize;
        let blocks = 64usize;
        let data: Vec<u8> = (0..block_len * blocks).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &data).unwrap();
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                peer_read_concurrency: blocks,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 0,
                    peer_read_elevator_budget_ms: 5,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let mut set = tokio::task::JoinSet::new();
        for i in (0..blocks).rev() {
            let sched = sched.clone();
            let path = path.clone();
            set.spawn(async move {
                let offset = (i * block_len) as u64;
                let bytes = sched
                    .read_at(IoClass::PeerRead, &path, offset, block_len)
                    .await
                    .unwrap();
                (i, bytes)
            });
        }

        while let Some(joined) = set.join_next().await {
            let (i, bytes) = joined.unwrap();
            let start = i * block_len;
            assert_eq!(&bytes[..], &data[start..start + block_len]);
        }

        let stats = sched.stats();
        assert_eq!(
            stats.read_ops_by_class[class_index(IoClass::PeerRead)],
            blocks as u64
        );
        assert!(
            stats.backend_read_ops_by_class[class_index(IoClass::PeerRead)] * 5 <= blocks as u64,
            "expected elevator to reduce backend reads; stats={stats:?}"
        );
        assert!(stats.peer_read_elevator_enabled);
        assert!(stats.peer_read_elevator_queue_depth >= blocks);
        assert_eq!(stats.peer_read_elevator_queued, 0);
        assert!(stats.peer_read_elevator_batches > 0);
        assert!(
            stats.peer_read_elevator_coalesced_requests > 0,
            "expected elevator to coalesce requests; stats={stats:?}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hdd_peer_read_elevator_dispatches_after_quiet_slice() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("quiet-elevator.bin");
        std::fs::write(&path, vec![42_u8; 4096]).unwrap();
        let sched = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    peer_read_readahead_bytes: 0,
                    peer_read_elevator_budget_ms: 250,
                    ..Default::default()
                },
                ..Default::default()
            },
        );

        let started = Instant::now();
        let bytes = sched
            .read_at(IoClass::PeerRead, &path, 0, 4096)
            .await
            .unwrap();

        assert_eq!(bytes.len(), 4096);
        assert!(
            started.elapsed() < Duration::from_millis(100),
            "single peer read should not wait the full elevator budget"
        );
        let stats = sched.stats();
        assert_eq!(stats.peer_read_elevator_batches, 1);
        assert_eq!(stats.peer_read_elevator_coalesced_requests, 0);
    }

    #[test]
    fn peer_read_elevator_full_queue_fails_closed() {
        let (sender, _receiver) = tokio_mpsc::channel(1);
        let counters = Arc::new(StorageCounters::default());
        let elevator = PeerReadElevator {
            sender,
            counters: counters.clone(),
            resources: None,
        };
        let path = PathBuf::from("queued.bin");
        let first = elevator.try_enqueue(path.clone(), 0, 1024);
        assert!(first.is_ok());
        let second = elevator.try_enqueue(path, 1024, 1024);
        assert!(matches!(
            second,
            Err(StorageError::QueueFull { mount }) if mount == "peer-read-elevator"
        ));
        assert_eq!(
            counters
                .peer_read_elevator_queue_full
                .load(Ordering::Relaxed),
            1
        );
    }

    #[tokio::test]
    async fn canceled_queued_peer_read_is_not_dispatched() {
        // Regression: dropping a peer read while it is queued in the HDD
        // elevator must not still open the file and consume an I/O slot.
        let sched = hdd_scheduler();
        let (tx, rx) = oneshot::channel();
        drop(rx);
        let request = PeerReadRequest {
            path: PathBuf::from("canceled-peer-read.bin"),
            offset: 0,
            len: 1024,
            _queued_bytes: Some(
                QueuedDiskBytes::reserve(sched.counters.clone(), None, 1024, "peer-read-elevator")
                    .unwrap(),
            ),
            tx,
        };
        let batch = PeerReadBatch {
            path: request.path.clone(),
            offset: request.offset,
            len: request.len,
            requests: vec![request],
        };

        let results = dispatch_peer_read_batch(
            batch,
            sched.file_pool.clone(),
            sched.io_pool.clone(),
            sched.disk_backend.clone(),
            sched.queue_sem.clone(),
            sched.device_queue.clone(),
            sched.counters.clone(),
        )
        .await;

        assert!(results.is_empty());
        assert_eq!(sched.stats().file_pool.misses, 0);
        assert_eq!(sched.stats().queued_disk_bytes, 0);
    }

    #[tokio::test]
    async fn malformed_peer_read_batch_returns_short_io_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("malformed-peer-read.bin");
        std::fs::write(&path, b"data").unwrap();
        let sched = hdd_scheduler();
        let (tx, rx) = oneshot::channel();
        let request = PeerReadRequest {
            path: path.clone(),
            offset: 8,
            len: 4,
            _queued_bytes: Some(
                QueuedDiskBytes::reserve(sched.counters.clone(), None, 4, "peer-read-elevator")
                    .unwrap(),
            ),
            tx,
        };
        let batch = PeerReadBatch {
            path,
            offset: 0,
            len: 4,
            requests: vec![request],
        };

        let results = dispatch_peer_read_batch(
            batch,
            sched.file_pool.clone(),
            sched.io_pool.clone(),
            sched.disk_backend.clone(),
            sched.queue_sem.clone(),
            sched.device_queue.clone(),
            sched.counters.clone(),
        )
        .await;

        let (reply, result) = results.into_iter().next().expect("request should complete");
        reply.send(result).expect("caller should still be waiting");
        assert!(matches!(
            rx.await.unwrap(),
            Err(StorageError::ShortIo {
                expected: 4,
                actual: 0,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn checkpoint_sync_covers_only_this_schedulers_files() {
        // Two schedulers with the same configuration share one descriptor
        // pool. A checkpoint on one must not sync (or clear) the other's
        // dirty file.
        let dir = tempfile::tempdir().unwrap();
        let config = SchedulerConfig {
            profile: StorageProfile::Hdd,
            storage_io: StorageIoConfig {
                durability_mode: DurabilityMode::Checkpoint,
                ..Default::default()
            },
            ..Default::default()
        };
        let a = MountScheduler::new(StorageRootId::new(), &config);
        let b = MountScheduler::new(StorageRootId::new(), &config);
        a.write_at(
            IoClass::PeerWrite,
            &dir.path().join("a.bin"),
            0,
            bytes::Bytes::from_static(b"a-data"),
            true,
        )
        .await
        .unwrap();
        b.write_at(
            IoClass::PeerWrite,
            &dir.path().join("b.bin"),
            0,
            bytes::Bytes::from_static(b"b-data"),
            true,
        )
        .await
        .unwrap();
        assert_eq!((a.stats().dirty_files, b.stats().dirty_files), (1, 1));

        b.sync_all_open_files().await.unwrap();
        assert_eq!(b.stats().dirty_files, 0);
        assert_eq!(
            a.stats().dirty_files,
            1,
            "another torrent's checkpoint must leave this one's dirty state alone"
        );
        a.sync_all_open_files().await.unwrap();
        assert_eq!(a.stats().dirty_files, 0);
    }

    fn scheduler_with_scripted_backend_for(
        mut scheduler: MountScheduler,
    ) -> (MountScheduler, Arc<ScriptedDiskBackend>) {
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        (scheduler, backend)
    }

    fn scheduler_with_scripted_backend() -> (MountScheduler, Arc<ScriptedDiskBackend>) {
        scheduler_with_scripted_backend_for(hdd_scheduler())
    }

    fn set_test_device_queue_for_path(
        scheduler: &mut MountScheduler,
        path: &Path,
        capacity: usize,
        active_limit: usize,
    ) {
        // Checkpoint barriers resolve admission from each path's physical
        // device. Keep the scheduler's observed queue keyed to that same
        // topology, including when TMPDIR is on a different mount than the
        // process working directory (as it is on hosted CI).
        scheduler.device_id = detect_storage_topology(path)
            .device_id
            .map(|device| device.0);
        let key = device_queue_key(scheduler.storage_root, scheduler.device_id.as_deref());
        scheduler.device_queue = Arc::new(DeviceQueue::new(Arc::from(key), capacity, active_limit));
        scheduler.device_queue_capacity = capacity;
    }

    fn queued_disk_governor(cap_bytes: u64) -> ResourceGovernor {
        let mut class_caps = [1024 * 1024; rt_metrics::MEMORY_CLASS_COUNT];
        class_caps[MemoryClass::QueuedDisk as usize] = cap_bytes;
        ResourceGovernor::new(rt_metrics::ResourceGovernorConfig {
            total_cap_bytes: 1024 * 1024,
            class_caps_bytes: class_caps,
            ..Default::default()
        })
    }

    fn queued_disk_used_bytes(resources: &ResourceGovernor) -> u64 {
        resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes
    }

    fn assert_storage_admission_released(scheduler: &MountScheduler) {
        let stats = scheduler.stats();
        assert_eq!(stats.queued_disk_bytes, 0);
        assert_eq!(stats.device_queue_available, stats.device_queue_capacity);
    }

    #[tokio::test]
    async fn injected_read_failure_is_returned_without_marking_payload_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("read-failure.bin");
        std::fs::write(&path, b"payload").unwrap();
        let (scheduler, backend) = scheduler_with_scripted_backend();
        backend.inject(Operation::Read, Effect::Fail(io::ErrorKind::Other));

        let read = scheduler.read_at(IoClass::Recheck, &path, 0, 7).await;

        assert!(read.is_err());
        assert_eq!(scheduler.stats().dirty_files, 0);
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn scripted_zero_progress_read_and_write_are_reported_as_short_io() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("zero-progress.bin");
        std::fs::write(&path, b"existing").unwrap();
        let (scheduler, backend) = scheduler_with_scripted_backend();

        backend.inject(Operation::Read, Effect::Fail(io::ErrorKind::UnexpectedEof));
        let read = scheduler.read_at(IoClass::Recheck, &path, 0, 8).await;
        assert!(matches!(
            read,
            Err(StorageError::ShortIo {
                expected: 8,
                actual: 0,
                ..
            })
        ));
        assert_eq!(scheduler.stats().dirty_files, 0);
        assert_storage_admission_released(&scheduler);

        backend.inject(Operation::Write, Effect::Fail(io::ErrorKind::WriteZero));
        let write = scheduler
            .write_at(
                IoClass::PeerWrite,
                &path,
                0,
                bytes::Bytes::from_static(b"replacement"),
                false,
            )
            .await;
        assert!(matches!(
            write,
            Err(StorageError::ShortIo {
                expected: 11,
                actual: 0,
                ..
            })
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"existing");
        assert_eq!(scheduler.stats().dirty_files, 1);
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn pre_submission_open_failures_release_storage_admission() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing-parent").join("payload.bin");
        let (scheduler, backend) = scheduler_with_scripted_backend();
        let queue_capacity = scheduler.queue_sem.available_permits();

        let read = scheduler.read_at(IoClass::Recheck, &path, 0, 1).await;
        assert!(read.is_err());
        assert_eq!(scheduler.queue_sem.available_permits(), queue_capacity);
        assert_storage_admission_released(&scheduler);

        let write = scheduler
            .write_at(
                IoClass::PeerWrite,
                &path,
                0,
                bytes::Bytes::from_static(b"x"),
                false,
            )
            .await;
        assert!(write.is_err());
        assert_eq!(scheduler.queue_sem.available_permits(), queue_capacity);
        assert_eq!(scheduler.stats().dirty_files, 0);
        assert_storage_admission_released(&scheduler);
        assert_eq!(backend.pending_releases(), 0);
    }

    #[tokio::test]
    async fn injected_write_failure_is_returned_and_remains_dirty_until_barrier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("write-failure.bin");
        let (scheduler, backend) = scheduler_with_scripted_backend();
        backend.inject(Operation::Write, Effect::Fail(io::ErrorKind::Other));

        let write = scheduler
            .write_at(
                IoClass::PeerWrite,
                &path,
                0,
                bytes::Bytes::from_static(b"payload"),
                true,
            )
            .await;

        assert!(write.is_err());
        assert_eq!(scheduler.stats().dirty_files, 1);
        assert_storage_admission_released(&scheduler);
        scheduler.sync_all_open_files().await.unwrap();
        assert_eq!(scheduler.stats().dirty_files, 0);
    }

    #[tokio::test]
    async fn strict_write_sync_failure_releases_admission_and_preserves_dirty_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("strict-sync-failure.bin");
        let resources = queued_disk_governor(QUEUED_DISK_JOB_OVERHEAD_BYTES);
        let scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    durability_mode: DurabilityMode::Strict,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        let (scheduler, backend) = scheduler_with_scripted_backend_for(scheduler);
        backend.inject(Operation::Sync, Effect::Fail(io::ErrorKind::Other));

        let write = scheduler
            .write_at(
                IoClass::PeerWrite,
                &path,
                0,
                bytes::Bytes::from_static(b"strict-payload"),
                true,
            )
            .await;

        assert!(write.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"strict-payload");
        assert_eq!(scheduler.stats().dirty_files, 1);
        assert_storage_admission_released(&scheduler);
        assert_eq!(queued_disk_used_bytes(&resources), 0);

        scheduler.sync_all_open_files().await.unwrap();
        assert_eq!(scheduler.stats().dirty_files, 0);
        assert_storage_admission_released(&scheduler);
        assert_eq!(queued_disk_used_bytes(&resources), 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replaced_path_after_held_write_keeps_old_identity_dirty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced-during-write.bin");
        let (scheduler, backend) = scheduler_with_scripted_backend();
        backend.inject(Operation::Write, Effect::HoldCompletion);

        let write_scheduler = scheduler.clone();
        let write_path = path.clone();
        let write = tokio::spawn(async move {
            write_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &write_path,
                    0,
                    bytes::Bytes::from_static(b"old-generation"),
                    true,
                )
                .await
        });

        backend.wait_until_completion_is_held().await;
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"replacement-generation").unwrap();
        assert!(backend.release_next());

        let error = write.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            StorageError::FilesystemStateUncertain {
                step: "payload_write",
                ..
            }
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement-generation");
        assert_eq!(scheduler.stats().dirty_files, 1);

        let barrier_error = scheduler.sync_all_open_files().await.unwrap_err();
        assert!(matches!(
            barrier_error,
            StorageError::FilesystemStateUncertain {
                step: "checkpoint_sync",
                ..
            }
        ));
        assert_eq!(scheduler.stats().dirty_files, 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replaced_path_after_held_read_is_not_returned_to_caller() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced-during-read.bin");
        std::fs::write(&path, b"old-generation").unwrap();
        let (scheduler, backend) = scheduler_with_scripted_backend();
        backend.inject(Operation::Read, Effect::HoldCompletion);

        let read_scheduler = scheduler.clone();
        let read_path = path.clone();
        let read = tokio::spawn(async move {
            read_scheduler
                .read_at(IoClass::Foreground, &read_path, 0, 14)
                .await
        });

        backend.wait_until_completion_is_held().await;
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"new-generation").unwrap();
        assert!(backend.release_next());

        let error = read.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            StorageError::FilesystemStateUncertain {
                step: "payload_read",
                ..
            }
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"new-generation");
        assert_storage_admission_released(&scheduler);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn replaced_path_after_held_checkpoint_sync_is_not_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("replaced-during-sync.bin");
        let (scheduler, backend) = scheduler_with_scripted_backend();
        scheduler
            .write_at(
                IoClass::PeerWrite,
                &path,
                0,
                bytes::Bytes::from_static(b"dirty-generation"),
                true,
            )
            .await
            .unwrap();
        backend.inject(Operation::Sync, Effect::HoldCompletion);

        let sync_scheduler = scheduler.clone();
        let sync = tokio::spawn(async move { sync_scheduler.sync_all_open_files().await });

        backend.wait_until_completion_is_held().await;
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, b"replacement-generation").unwrap();
        assert!(backend.release_next());

        let error = sync.await.unwrap().unwrap_err();
        assert!(matches!(
            error,
            StorageError::FilesystemStateUncertain {
                step: "checkpoint_sync",
                ..
            }
        ));
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement-generation");
        assert_eq!(scheduler.stats().dirty_files, 1);
    }

    #[tokio::test]
    async fn cancelled_waiter_keeps_submitted_write_dirty_until_barrier() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("late-write.bin");
        let (scheduler, backend) = scheduler_with_scripted_backend();
        backend.inject(
            Operation::Write,
            Effect::HoldCompletionThenFail(io::ErrorKind::Other),
        );

        let write_scheduler = scheduler.clone();
        let write_path = path.clone();
        let write = tokio::spawn(async move {
            write_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &write_path,
                    0,
                    bytes::Bytes::from_static(b"verified-payload"),
                    true,
                )
                .await
        });

        backend.wait_until_completion_is_held().await;
        assert_eq!(backend.pending_releases(), 1);
        let capacity = scheduler.stats().device_queue_capacity;
        let available_while_in_flight = scheduler.stats().device_queue_available;
        assert_eq!(available_while_in_flight, capacity - 1);
        write.abort();
        let _ = write.await;

        assert_eq!(
            scheduler.stats().device_queue_available,
            available_while_in_flight,
            "caller cancellation must not release the device slot before backend completion"
        );
        assert_eq!(scheduler.stats().dirty_files, 1);
        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.stats().device_queue_available != capacity {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("late backend completion should release the device operation");
        scheduler.sync_all_open_files().await.unwrap();
        assert_eq!(scheduler.stats().dirty_files, 0);
        assert_eq!(std::fs::read(path).unwrap(), b"verified-payload");
    }

    #[tokio::test]
    async fn cancelled_submitted_read_retains_admission_until_failure_completion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("read-failure-held.bin");
        std::fs::write(&path, b"payload").unwrap();
        let (scheduler, backend) = scheduler_with_scripted_backend();
        backend.inject(
            Operation::Read,
            Effect::HoldCompletionThenFail(io::ErrorKind::Other),
        );

        let read_scheduler = scheduler.clone();
        let read_path = path.clone();
        let queue_capacity = scheduler.queue_sem.available_permits();
        let read = tokio::spawn(async move {
            read_scheduler
                .read_at(IoClass::Recheck, &read_path, 0, 7)
                .await
        });

        backend.wait_until_completion_is_held().await;
        assert_eq!(backend.pending_releases(), 1);
        assert_eq!(scheduler.queue_sem.available_permits(), queue_capacity - 1);
        assert_eq!(
            scheduler.stats().queued_disk_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        read.abort();
        let _ = read.await;

        assert_eq!(scheduler.queue_sem.available_permits(), queue_capacity - 1);
        assert_eq!(
            scheduler.stats().queued_disk_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(scheduler.stats().dirty_files, 0);
        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.queue_sem.available_permits() != queue_capacity
                || scheduler.stats().queued_disk_bytes != 0
                || scheduler.stats().device_queue_available
                    != scheduler.stats().device_queue_capacity
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("failed read completion should release all storage admission");
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn cancelled_submitted_write_retains_queue_slot_until_completion() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("queue-held.bin");
        let later_path = dir.path().join("queue-later.bin");
        let resources = queued_disk_governor(4096);
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                max_queue: 1,
                storage_io: StorageIoConfig {
                    io_worker_threads: 1,
                    io_queue_depth: 1,
                    peer_read_elevator_budget_ms: 0,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        set_test_device_queue_for_path(&mut scheduler, &first_path, 4, 4);
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        backend.inject(Operation::Write, Effect::HoldCompletion);

        let write_scheduler = scheduler.clone();
        let write_path = first_path.clone();
        let write = tokio::spawn(async move {
            write_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &write_path,
                    0,
                    bytes::Bytes::from_static(b"payload"),
                    true,
                )
                .await
        });

        backend.wait_until_completion_is_held().await;
        let charged = QUEUED_DISK_JOB_OVERHEAD_BYTES;
        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            charged
        );
        write.abort();
        let _ = write.await;

        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            charged,
            "caller cancellation must not release the queued-byte lease"
        );
        let blocked = scheduler
            .write_at(
                IoClass::PeerWrite,
                &later_path,
                0,
                bytes::Bytes::from_static(b"later"),
                true,
            )
            .await;
        assert!(matches!(blocked, Err(StorageError::QueueFull { .. })));
        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            charged
        );

        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.stats().queued_disk_bytes != 0
                || resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("backend completion should release queue and byte admission");

        scheduler
            .write_at(
                IoClass::PeerWrite,
                &later_path,
                0,
                bytes::Bytes::from_static(b"later"),
                true,
            )
            .await
            .unwrap();
        assert_eq!(scheduler.stats().queued_disk_bytes, 0);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            0
        );
    }

    #[tokio::test]
    async fn cancelled_shared_payload_write_retains_owner_lease_until_completion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared-payload-held.bin");
        let payload = bytes::Bytes::from_static(b"payload");
        let resources = queued_disk_governor(QUEUED_DISK_JOB_OVERHEAD_BYTES);
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                max_queue: 1,
                storage_io: StorageIoConfig {
                    io_worker_threads: 1,
                    io_queue_depth: 1,
                    peer_read_elevator_budget_ms: 0,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        backend.inject(Operation::Write, Effect::HoldCompletion);
        let payload_lease = Arc::new(
            resources
                .try_acquire(MemoryClass::PieceAssembly, payload.len() as u64)
                .unwrap(),
        );

        let write_scheduler = scheduler.clone();
        let write_path = path.clone();
        let write_lease = Arc::clone(&payload_lease);
        let write = tokio::spawn(async move {
            write_scheduler
                .write_at_with_shared_payload_lease(
                    IoClass::PeerWrite,
                    &write_path,
                    0,
                    payload,
                    true,
                    write_lease,
                )
                .await
        });

        backend.wait_until_completion_is_held().await;
        drop(payload_lease);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PieceAssembly as usize].used_bytes,
            b"payload".len() as u64
        );
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        write.abort();
        let _ = write.await;
        assert_eq!(
            resources.snapshot().classes[MemoryClass::PieceAssembly as usize].used_bytes,
            b"payload".len() as u64,
            "the submitted backend operation must retain the shared owner lease"
        );

        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), async {
            while resources.snapshot().classes[MemoryClass::PieceAssembly as usize].used_bytes != 0
                || queued_disk_used_bytes(&resources) != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("backend completion should release shared payload and queue leases");
        assert_eq!(std::fs::read(path).unwrap(), b"payload");
    }

    #[tokio::test]
    async fn cancelled_submitted_write_retains_memory_budget_until_completion() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("memory-held.bin");
        let later_path = dir.path().join("memory-later.bin");
        let resources = queued_disk_governor(QUEUED_DISK_JOB_OVERHEAD_BYTES);
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    io_worker_threads: 1,
                    io_queue_depth: 2,
                    peer_read_elevator_budget_ms: 0,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        backend.inject(Operation::Write, Effect::HoldCompletion);

        let write_scheduler = scheduler.clone();
        let write_path = first_path.clone();
        let write = tokio::spawn(async move {
            write_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &write_path,
                    0,
                    bytes::Bytes::from_static(b"payload"),
                    true,
                )
                .await
        });

        backend.wait_until_completion_is_held().await;
        let charged = QUEUED_DISK_JOB_OVERHEAD_BYTES;
        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            charged
        );
        write.abort();
        let _ = write.await;

        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            charged,
            "caller cancellation must not release memory admission"
        );
        let denied = scheduler
            .write_at(
                IoClass::PeerWrite,
                &later_path,
                0,
                bytes::Bytes::from_static(b"later"),
                true,
            )
            .await;
        assert!(matches!(denied, Err(StorageError::QueueFull { .. })));
        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        let snapshot = resources.snapshot();
        assert_eq!(
            snapshot.classes[MemoryClass::QueuedDisk as usize].used_bytes,
            charged
        );
        assert_eq!(
            snapshot.classes[MemoryClass::QueuedDisk as usize].denied_allocations,
            1
        );

        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.stats().queued_disk_bytes != 0
                || resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("backend completion should release the memory lease");

        scheduler
            .write_at(
                IoClass::PeerWrite,
                &later_path,
                0,
                bytes::Bytes::from_static(b"later"),
                true,
            )
            .await
            .unwrap();
        assert_eq!(scheduler.stats().queued_disk_bytes, 0);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].used_bytes,
            0
        );
    }

    #[tokio::test]
    async fn cancelled_submitted_readahead_read_retains_extra_bytes_until_completion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readahead-held.bin");
        std::fs::write(&path, vec![0xAB_u8; 4096]).unwrap();
        let readahead_bytes = 2048;
        let resources = queued_disk_governor(readahead_bytes as u64);
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    io_worker_threads: 1,
                    io_queue_depth: 2,
                    peer_read_readahead_bytes: readahead_bytes,
                    peer_read_readahead_sequential_only: false,
                    peer_read_cache_entries: 1,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        backend.inject(Operation::Read, Effect::HoldCompletion);

        let read_scheduler = scheduler.clone();
        let read_path = path.clone();
        let read = tokio::spawn(async move {
            read_scheduler
                .read_at(IoClass::PeerRead, &read_path, 0, 4)
                .await
        });

        backend.wait_until_completion_is_held().await;
        let charged = readahead_bytes as u64;
        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        assert_eq!(queued_disk_used_bytes(&resources), charged);
        read.abort();
        let _ = read.await;

        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        assert_eq!(
            queued_disk_used_bytes(&resources),
            charged,
            "caller cancellation must retain the expanded read charge"
        );
        let denied = scheduler.read_at(IoClass::Foreground, &path, 0, 4).await;
        assert!(matches!(denied, Err(StorageError::QueueFull { .. })));
        assert_eq!(scheduler.stats().queued_disk_bytes, charged);
        assert_eq!(queued_disk_used_bytes(&resources), charged);
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].denied_allocations,
            1
        );

        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.stats().queued_disk_bytes != 0
                || queued_disk_used_bytes(&resources) != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("backend completion should release the expanded read charge");

        let bytes = scheduler
            .read_at(IoClass::Foreground, &path, 0, 4)
            .await
            .unwrap();
        assert_eq!(bytes.as_ref(), &[0xAB; 4]);
        assert_eq!(scheduler.stats().queued_disk_bytes, 0);
        assert_eq!(queued_disk_used_bytes(&resources), 0);
    }

    #[tokio::test]
    async fn cancelled_strict_write_retains_admission_until_sync_completion() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("strict-held.bin");
        let later_path = dir.path().join("strict-later.bin");
        let resources = queued_disk_governor(QUEUED_DISK_JOB_OVERHEAD_BYTES);
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                max_queue: 4,
                storage_io: StorageIoConfig {
                    io_worker_threads: 1,
                    io_queue_depth: 2,
                    durability_mode: DurabilityMode::Strict,
                    peer_read_elevator_budget_ms: 0,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        backend.inject(
            Operation::Sync,
            Effect::HoldCompletionThenFail(io::ErrorKind::Other),
        );

        let write_scheduler = scheduler.clone();
        let write_path = first_path.clone();
        let write = tokio::spawn(async move {
            write_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &write_path,
                    0,
                    bytes::Bytes::from_static(b"strict-payload"),
                    true,
                )
                .await
        });

        backend.wait_until_completion_is_held().await;
        assert_eq!(
            scheduler.stats().queued_disk_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        write.abort();
        let _ = write.await;

        assert_eq!(scheduler.stats().dirty_files, 1);
        assert_eq!(
            scheduler.stats().queued_disk_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES,
            "caller cancellation must retain admission through strict sync"
        );
        let denied = scheduler
            .write_at(
                IoClass::PeerWrite,
                &later_path,
                0,
                bytes::Bytes::from_static(b"later"),
                true,
            )
            .await;
        assert!(matches!(denied, Err(StorageError::QueueFull { .. })));
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(
            resources.snapshot().classes[MemoryClass::QueuedDisk as usize].denied_allocations,
            1
        );

        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.stats().queued_disk_bytes != 0
                || queued_disk_used_bytes(&resources) != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("strict-sync completion should release queue and byte admission");
        assert_eq!(scheduler.stats().dirty_files, 1);

        scheduler.sync_all_open_files().await.unwrap();
        assert_eq!(scheduler.stats().dirty_files, 0);
        scheduler
            .write_at(
                IoClass::PeerWrite,
                &later_path,
                0,
                bytes::Bytes::from_static(b"later"),
                true,
            )
            .await
            .unwrap();
        assert_eq!(scheduler.stats().queued_disk_bytes, 0);
        assert_eq!(queued_disk_used_bytes(&resources), 0);
        assert_eq!(std::fs::read(first_path).unwrap(), b"strict-payload");
    }

    #[tokio::test]
    async fn checkpoint_barrier_blocks_new_path_writes_until_evidence_is_returned() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("first.bin");
        let later_path = dir.path().join("later.bin");
        let (scheduler, backend) = scheduler_with_scripted_backend();

        scheduler
            .write_at(
                IoClass::PeerWrite,
                &first_path,
                0,
                bytes::Bytes::from_static(b"before-barrier"),
                true,
            )
            .await
            .unwrap();
        backend.inject(Operation::Sync, Effect::HoldCompletion);

        let barrier_scheduler = scheduler.clone();
        let barrier =
            tokio::spawn(
                async move { barrier_scheduler.sync_all_open_files_with_evidence().await },
            );
        backend.wait_until_completion_is_held().await;
        assert_eq!(backend.pending_releases(), 1);

        let later_scheduler = scheduler.clone();
        let later_write_path = later_path.clone();
        let (started_tx, started_rx) = oneshot::channel();
        let later_write = tokio::spawn(async move {
            started_tx.send(()).unwrap();
            later_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &later_write_path,
                    0,
                    bytes::Bytes::from_static(b"after-barrier"),
                    true,
                )
                .await
        });
        started_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert_eq!(
            backend.pending_releases(),
            1,
            "a new path must not reach the backend while the checkpoint owns admission"
        );

        assert!(backend.release_next());
        let evidence = barrier.await.unwrap().unwrap();
        assert_eq!(evidence.synced_file_count(), 1);
        assert_eq!(evidence.write_sequence_watermark(), 1);
        tokio::task::yield_now().await;
        assert_eq!(
            scheduler.stats().dirty_files,
            0,
            "the returned proof must keep later writes out until the resume commit releases it"
        );
        drop(evidence);
        later_write.await.unwrap().unwrap();
        assert_eq!(scheduler.stats().dirty_files, 1);
        assert_eq!(std::fs::read(first_path).unwrap(), b"before-barrier");
        assert_eq!(std::fs::read(later_path).unwrap(), b"after-barrier");
    }

    #[tokio::test]
    async fn checkpoint_barrier_dispatches_same_device_paths_as_a_bounded_batch() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("batch-first.bin");
        let second_path = dir.path().join("batch-second.bin");
        let mut scheduler = MountScheduler::new_for_path(
            StorageRootId::new(),
            dir.path(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                max_queue: 4,
                storage_io: StorageIoConfig {
                    file_pool_size: 2,
                    io_worker_threads: 2,
                    io_queue_depth: 4,
                    durability_mode: DurabilityMode::Checkpoint,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        set_test_device_queue_for_path(&mut scheduler, &first_path, 4, 2);
        let backend = Arc::new(ScriptedDiskBackend::new(2, 4));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        scheduler
            .write_at(
                IoClass::PeerWrite,
                &first_path,
                0,
                bytes::Bytes::from_static(b"first-dirty-path"),
                true,
            )
            .await
            .unwrap();
        scheduler
            .write_at(
                IoClass::PeerWrite,
                &second_path,
                0,
                bytes::Bytes::from_static(b"second-dirty-path"),
                true,
            )
            .await
            .unwrap();
        assert_eq!(scheduler.stats().dirty_files, 2);
        backend.inject(Operation::Sync, Effect::HoldCompletion);
        backend.inject(Operation::Sync, Effect::HoldCompletion);

        let barrier_scheduler = scheduler.clone();
        let barrier =
            tokio::spawn(
                async move { barrier_scheduler.sync_all_open_files_with_evidence().await },
            );
        tokio::time::timeout(Duration::from_secs(1), async {
            backend.wait_until_completion_is_held().await;
            backend.wait_until_completion_is_held().await;
        })
        .await
        .expect("both same-device paths should enter the bounded checkpoint batch");
        assert_eq!(backend.pending_releases(), 2);
        assert_eq!(scheduler.stats().device_queue_available, 2);

        assert!(backend.release_next());
        assert!(backend.release_next());
        let evidence = barrier.await.unwrap().unwrap();
        assert_eq!(evidence.synced_file_count(), 2);
        assert_eq!(scheduler.stats().dirty_files, 0);
        let checkpoint_operations: Vec<_> = lock_recover(&scheduler.counters.operation_dispatches)
            .iter()
            .filter(|operation| operation.operation_kind == "checkpoint_file_sync")
            .cloned()
            .collect();
        assert_eq!(checkpoint_operations.len(), 2);
        assert_ne!(
            checkpoint_operations[0].storage_operation_id,
            checkpoint_operations[1].storage_operation_id,
            "each file sync gets a distinct operation identity"
        );
        assert_ne!(
            checkpoint_operations[0].device_operation_id,
            checkpoint_operations[1].device_operation_id,
            "each file sync gets a distinct device admission identity"
        );
        drop(evidence);
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn cancelled_checkpoint_waiter_keeps_admission_until_full_barrier_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("a-checkpoint-held.bin");
        let second_path = dir.path().join("b-checkpoint-held.bin");
        let later_path = dir.path().join("c-checkpoint-later.bin");
        let resources = queued_disk_governor(QUEUED_DISK_JOB_OVERHEAD_BYTES);
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    durability_mode: DurabilityMode::Checkpoint,
                    io_worker_threads: 1,
                    io_queue_depth: 2,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        set_test_device_queue_for_path(&mut scheduler, &first_path, 4, 4);
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        scheduler
            .write_at(
                IoClass::PeerWrite,
                &first_path,
                0,
                bytes::Bytes::from_static(b"checkpoint-payload"),
                true,
            )
            .await
            .unwrap();
        scheduler
            .write_at(
                IoClass::PeerWrite,
                &second_path,
                0,
                bytes::Bytes::from_static(b"second-checkpoint-payload"),
                true,
            )
            .await
            .unwrap();
        assert_eq!(scheduler.stats().dirty_files, 2);
        backend.inject(Operation::Sync, Effect::HoldCompletion);

        let barrier_scheduler = scheduler.clone();
        let barrier = tokio::spawn(async move { barrier_scheduler.sync_all_open_files().await });
        backend.wait_until_completion_is_held().await;

        let capacity = scheduler.stats().device_queue_capacity;
        let available_during_barrier = scheduler.stats().device_queue_available;
        assert!(
            (capacity.saturating_sub(2)..capacity).contains(&available_during_barrier),
            "one or both snapshot syncs should retain device admission"
        );
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        barrier.abort();
        let _ = barrier.await;

        let later_scheduler = scheduler.clone();
        let later_write_path = later_path.clone();
        let (started_tx, started_rx) = oneshot::channel();
        let later_write = tokio::spawn(async move {
            started_tx.send(()).unwrap();
            later_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &later_write_path,
                    0,
                    bytes::Bytes::from_static(b"later-write"),
                    true,
                )
                .await
        });
        started_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert_eq!(backend.pending_releases(), 1);
        let available_after_later_write = scheduler.stats().device_queue_available;
        assert!(
            (capacity.saturating_sub(2)..capacity).contains(&available_after_later_write),
            "the later write must not acquire device admission before the snapshot barrier"
        );
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        assert_eq!(
            scheduler.stats().queued_disk_bytes,
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );

        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), async {
            while scheduler.stats().sync_ops < 1 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("detached checkpoint should finish every path in its snapshot");
        tokio::time::timeout(Duration::from_secs(1), async {
            later_write.await.unwrap().unwrap();
            while scheduler.stats().queued_disk_bytes != 0
                || queued_disk_used_bytes(&resources) != 0
                || scheduler.stats().device_queue_available != capacity
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("later write should run after the complete checkpoint releases admission");

        assert_eq!(scheduler.stats().dirty_files, 1);
        assert_eq!(std::fs::read(first_path).unwrap(), b"checkpoint-payload");
        assert_eq!(
            std::fs::read(second_path).unwrap(),
            b"second-checkpoint-payload"
        );
        assert_eq!(std::fs::read(later_path).unwrap(), b"later-write");
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn partial_write_then_error_is_not_reported_as_success() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("partial-write.bin");
        let (scheduler, backend) = scheduler_with_scripted_backend();
        backend.inject(
            Operation::Write,
            Effect::WritePrefixThenFail {
                prefix_bytes: 3,
                kind: io::ErrorKind::Other,
            },
        );

        let write_scheduler = scheduler.clone();
        let write_path = path.clone();
        let write = tokio::spawn(async move {
            write_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &write_path,
                    0,
                    bytes::Bytes::from_static(b"verified-payload"),
                    true,
                )
                .await
        });
        backend.wait_until_completion_is_held().await;
        assert!(backend.release_next());

        assert!(write.await.unwrap().is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"ver");
        assert_eq!(scheduler.stats().dirty_files, 1);
        assert_storage_admission_released(&scheduler);
    }

    #[tokio::test]
    async fn cancelled_checkpoint_waiter_keeps_dirty_evidence_after_sync_failure() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = dir.path().join("a-checkpoint-failure.bin");
        let second_path = dir.path().join("b-checkpoint-failure.bin");
        let later_path = dir.path().join("c-checkpoint-after-failure.bin");
        let resources = queued_disk_governor(QUEUED_DISK_JOB_OVERHEAD_BYTES);
        let mut scheduler = MountScheduler::new(
            StorageRootId::new(),
            &SchedulerConfig {
                profile: StorageProfile::Hdd,
                storage_io: StorageIoConfig {
                    durability_mode: DurabilityMode::Checkpoint,
                    io_worker_threads: 1,
                    io_queue_depth: 2,
                    ..Default::default()
                },
                resources: Some(resources.clone()),
                ..Default::default()
            },
        );
        set_test_device_queue_for_path(&mut scheduler, &first_path, 4, 4);
        let backend = Arc::new(ScriptedDiskBackend::new(1, 8));
        scheduler.disk_backend = Arc::new(SelectedDiskBackend::with_test_backend(backend.clone()));
        for (path, payload) in [
            (&first_path, &b"first-dirty"[..]),
            (&second_path, &b"second-dirty"[..]),
        ] {
            scheduler
                .write_at(
                    IoClass::PeerWrite,
                    path,
                    0,
                    bytes::Bytes::copy_from_slice(payload),
                    true,
                )
                .await
                .unwrap();
        }
        assert_eq!(scheduler.stats().dirty_files, 2);
        backend.inject(
            Operation::Sync,
            Effect::HoldCompletionThenFail(io::ErrorKind::Other),
        );

        let barrier_scheduler = scheduler.clone();
        let barrier =
            tokio::spawn(
                async move { barrier_scheduler.sync_all_open_files_with_evidence().await },
            );
        backend.wait_until_completion_is_held().await;
        let capacity = scheduler.stats().device_queue_capacity;
        assert!(
            scheduler.stats().device_queue_available < capacity,
            "the held checkpoint sync retains its device admission"
        );
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES
        );
        barrier.abort();
        let _ = barrier.await;

        let later_scheduler = scheduler.clone();
        let later_write_path = later_path.clone();
        let (started_tx, started_rx) = oneshot::channel();
        let later_write = tokio::spawn(async move {
            started_tx.send(()).unwrap();
            later_scheduler
                .write_at(
                    IoClass::PeerWrite,
                    &later_write_path,
                    0,
                    bytes::Bytes::from_static(b"after-failure"),
                    true,
                )
                .await
        });
        started_rx.await.unwrap();
        tokio::task::yield_now().await;
        assert_eq!(backend.pending_releases(), 1);
        assert!(
            scheduler.stats().device_queue_available < capacity,
            "the detached barrier retains device admission through the failed sync"
        );
        assert!(!later_write.is_finished());
        assert!(
            !later_path.exists(),
            "the later write waits for the barrier"
        );
        assert_eq!(
            queued_disk_used_bytes(&resources),
            QUEUED_DISK_JOB_OVERHEAD_BYTES,
            "the detached barrier must retain its admission through a failed sync"
        );

        assert!(backend.release_next());
        tokio::time::timeout(Duration::from_secs(1), later_write)
            .await
            .expect("the later write should run after the failed barrier finishes")
            .unwrap()
            .unwrap();
        assert!(
            scheduler.stats().dirty_files >= 2,
            "a failed path and the later write must remain dirty"
        );
        assert_eq!(std::fs::read(&first_path).unwrap(), b"first-dirty");
        assert_eq!(std::fs::read(&second_path).unwrap(), b"second-dirty");
        assert_eq!(std::fs::read(&later_path).unwrap(), b"after-failure");

        scheduler.sync_all_open_files().await.unwrap();
        assert_eq!(scheduler.stats().dirty_files, 0);
        assert_storage_admission_released(&scheduler);
        assert_eq!(queued_disk_used_bytes(&resources), 0);
    }

    #[tokio::test]
    async fn failed_barrier_keeps_dirty_evidence_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sync-retry.bin");
        let (scheduler, backend) = scheduler_with_scripted_backend();

        scheduler
            .write_at(
                IoClass::PeerWrite,
                &path,
                0,
                bytes::Bytes::from_static(b"durable-later"),
                true,
            )
            .await
            .unwrap();
        assert_eq!(scheduler.stats().dirty_files, 1);

        backend.inject(Operation::Sync, Effect::Fail(io::ErrorKind::Other));
        assert!(scheduler.sync_all_open_files().await.is_err());
        assert_eq!(scheduler.stats().dirty_files, 1);
        assert_storage_admission_released(&scheduler);

        scheduler.sync_all_open_files().await.unwrap();
        assert_eq!(scheduler.stats().dirty_files, 0);
    }

    #[test]
    fn readahead_expands_only_continuing_streams() {
        let mut streams = PeerReadStreams::default();
        let file = Path::new("/payload/a.bin");
        assert_eq!(
            streams.observe(file, 0, 16_384, 64 * 1024, false),
            (false, 16_384),
            "first read is exact"
        );
        assert_eq!(
            streams.observe(file, 16_384, 16_384, 64 * 1024, false),
            (true, 32_768),
            "first continuation expands by one block"
        );
        assert_eq!(
            streams.observe(file, 32_768, 16_384, 64 * 1024, true),
            (true, 49_152),
            "a cache hit grows the range gradually"
        );
        assert_eq!(
            streams.observe(file, 49_152, 16_384, 64 * 1024, false),
            (true, 49_152),
            "an exhausted range holds the recommendation until reuse is observed"
        );
        assert_eq!(
            streams.observe(file, 65_536, 16_384, 64 * 1024, true),
            (true, 64 * 1024),
            "growth stops at the configured cap"
        );
        assert_eq!(
            streams.observe(file, 81_920, 16_384, 64 * 1024, true),
            (true, 64 * 1024),
            "a stream at its cap stays bounded"
        );
        assert_eq!(
            streams.observe(file, 1_048_576, 16_384, 64 * 1024, false),
            (false, 16_384),
            "a jump resets to an exact read"
        );
        assert_eq!(
            streams.observe(
                Path::new("/payload/b.bin"),
                16_384,
                16_384,
                64 * 1024,
                false,
            ),
            (false, 16_384),
            "another file starts fresh"
        );
    }

    #[test]
    fn readahead_stops_growing_when_prefetch_is_not_reused() {
        let mut streams = PeerReadStreams::default();
        let file = Path::new("/payload/a.bin");
        streams.observe(file, 0, 16_384, 128 * 1024, false);
        assert_eq!(
            streams.observe(file, 16_384, 16_384, 128 * 1024, false),
            (true, 32_768),
            "sequential demand bootstraps one prefetch"
        );
        assert_eq!(
            streams.observe(file, 32_768, 16_384, 128 * 1024, false),
            (true, 32_768),
            "a cache miss does not increase speculative range"
        );
        assert_eq!(
            streams.observe(file, 49_152, 16_384, 128 * 1024, true),
            (true, 49_152),
            "a later cache hit resumes bounded growth"
        );
    }

    #[test]
    fn current_peer_read_stream_bypasses_map_and_promotes_with_history() {
        let mut streams = PeerReadStreams::default();
        let hot = Path::new("/payload/hot.bin");
        let other = Path::new("/payload/other.bin");

        assert_eq!(streams.observe(hot, 0, 16, 128, false), (false, 16));
        assert_eq!(streams.observe(hot, 16, 16, 128, false), (true, 32));
        assert!(
            streams.files.is_empty(),
            "the current stream stays outside the map"
        );

        assert_eq!(streams.observe(other, 0, 16, 128, false), (false, 16));
        assert!(streams.files.contains_key(hot));
        assert_eq!(streams.observe(hot, 32, 16, 128, false), (true, 32));
        assert!(streams.files.contains_key(other));
        assert_eq!(streams.tracked_file_count(), 2);
    }

    #[test]
    fn peer_read_stream_bound_evicts_one_least_recent_stream() {
        let mut streams = PeerReadStreams::default();
        let hot = Path::new("/payload/hot.bin");
        streams.observe(hot, 0, 16, 128, false);
        streams.observe(hot, 16, 16, 128, false);

        for index in 0..PeerReadStreams::MAX_TRACKED_FILES - 1 {
            let path = PathBuf::from(format!("/payload/{index}.bin"));
            streams.observe(&path, 0, 16, 128, false);
        }
        assert_eq!(
            streams.tracked_file_count(),
            PeerReadStreams::MAX_TRACKED_FILES
        );

        assert!(streams.observe(hot, 32, 16, 128, false).0);
        streams.observe(Path::new("/payload/overflow.bin"), 0, 16, 128, false);

        assert_eq!(
            streams.tracked_file_count(),
            PeerReadStreams::MAX_TRACKED_FILES
        );
        assert!(streams.observe(hot, 48, 16, 128, false).0);
        assert!(
            !streams
                .observe(Path::new("/payload/0.bin"), 16, 16, 128, false)
                .0
        );
    }

    #[test]
    fn write_descriptor_serves_reads_without_reopening() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shared.bin");
        std::fs::write(&path, b"payload").unwrap();
        let pool = FilePool::new(8, Duration::from_secs(300));

        let _writer = pool.get_or_open(&path, OpenMode::Write, true).unwrap();
        let _reader = pool.get_or_open(&path, OpenMode::Read, false).unwrap();

        let stats = pool.stats();
        assert_eq!(stats.misses, 1, "the read reused the write descriptor");
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.open_files, 1);
        assert_eq!(stats.evictions, 0);

        // The reverse is not allowed: a read-only descriptor cannot write.
        let other = dir.path().join("readonly.bin");
        std::fs::write(&other, b"x").unwrap();
        let _read = pool.get_or_open(&other, OpenMode::Read, false).unwrap();
        let _write = pool.get_or_open(&other, OpenMode::Write, false).unwrap();
        assert_eq!(pool.stats().evictions, 1, "upgrade replaces the read entry");
    }

    #[test]
    fn idle_sweep_runs_on_an_interval_not_on_every_lookup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sweep.bin");
        std::fs::write(&path, b"x").unwrap();
        let pool = FilePool::new(8, Duration::from_secs(3600));
        let _first = pool.get_or_open(&path, OpenMode::Read, false).unwrap();
        let swept_at = pool.catalog.sweep_marker_ns();
        assert_ne!(swept_at, 0, "the first lookup sweeps");
        for _ in 0..50 {
            let _hit = pool.get_or_open(&path, OpenMode::Read, false).unwrap();
        }
        assert_eq!(
            pool.catalog.sweep_marker_ns(),
            swept_at,
            "warm hits within the sweep interval must not rescan the cache"
        );
    }
}
