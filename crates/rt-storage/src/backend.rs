//! Disk backend abstraction.
//!
//! All positioned I/O goes through a [`DiskBackend`]. Storage NG ships the
//! [`PreadBackend`] (a dedicated, bounded blocking thread pool calling
//! `pread`/`pwrite` via positioned I/O) as the portable default and a Linux
//! [`UringBackend`] for positioned `io_uring` reads, writes, and data sync.
//! Backend selection is explicit so older kernels and restricted containers
//! fall back cleanly.
//!
//! The pool is deliberately *separate* from Tokio's generic blocking pool
//! so disk I/O can neither starve nor be starved by unrelated
//! `spawn_blocking` work.

use std::any::Any;
#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io;
#[cfg(all(unix, not(target_os = "linux")))]
use std::os::unix::fs::FileExt;
#[cfg(target_os = "linux")]
use std::os::unix::fs::FileExt;
#[cfg(target_os = "linux")]
use std::os::unix::io::AsRawFd;
#[cfg(windows)]
use std::os::windows::fs::FileExt;
#[cfg(target_os = "linux")]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::atomic::{AtomicU8, Ordering as AtomicOrdering};
use std::sync::{mpsc, Arc, Mutex, MutexGuard};
use std::thread;

#[cfg(target_os = "linux")]
use io_uring::{opcode, types, IoUring};
use tokio::sync::oneshot;

use crate::file_handle::LeasedFileHandle;
use crate::frame::Frame;
#[cfg(target_os = "linux")]
use crate::frame::{RegisteredFrameSlot, RegisteredFrameSlots};
use crate::StorageError;

fn lock_job_receiver<T>(receiver: &Mutex<T>) -> MutexGuard<'_, T> {
    match receiver.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            // The receiver is the authoritative bounded work queue. Keep its
            // pending jobs and let another worker continue after a panic.
            let guard = poisoned.into_inner();
            receiver.clear_poison();
            guard
        }
    }
}

#[cfg(target_os = "linux")]
fn try_lock_job_receiver<T>(receiver: &Mutex<T>) -> Option<MutexGuard<'_, T>> {
    match receiver.try_lock() {
        Ok(guard) => Some(guard),
        Err(std::sync::TryLockError::WouldBlock) => None,
        Err(std::sync::TryLockError::Poisoned(poisoned)) => {
            // The receiver remains authoritative after a worker panic. Do not
            // block a ring worker behind another thread waiting for new work.
            let guard = poisoned.into_inner();
            receiver.clear_poison();
            Some(guard)
        }
    }
}

const DEFAULT_BACKEND_QUEUE_DEPTH: usize = 1024;
#[cfg(target_os = "linux")]
const URING_ENTRIES: u32 = 256;
#[cfg(target_os = "linux")]
const URING_BATCH_LIMIT: usize = 64;
#[cfg(target_os = "linux")]
const URING_FILE_SLOTS: u32 = URING_ENTRIES;
/// Keep fixed buffer registration below common 8 MiB `RLIMIT_MEMLOCK`
/// defaults. The backend can still batch more submissions than it has
/// fixed-buffer slots; overflow submissions use ordinary read/write buffers.
#[cfg(target_os = "linux")]
const URING_FIXED_BUFFER_SLOTS: usize = 4;
#[cfg(target_os = "linux")]
const URING_FIXED_BUFFER_LEN: usize = 256 * 1024;

/// Ownership that must outlive a submitted backend operation. The scheduler
/// keeps a typed handle to the same state so a successful completion can
/// return its guards without a forwarding Tokio task or a second channel.
pub(crate) struct BackendJobOwnership {
    _keepalive: Option<Arc<dyn Any + Send + Sync>>,
    on_completion: Option<Box<dyn FnOnce() + Send>>,
}

impl BackendJobOwnership {
    pub(crate) fn new<T: Any + Send + Sync>(keepalive: Arc<T>) -> Self {
        Self {
            _keepalive: Some(keepalive),
            on_completion: None,
        }
    }

    pub(crate) fn untracked() -> Self {
        Self {
            _keepalive: None,
            on_completion: None,
        }
    }

    pub(crate) fn after_completion(mut self, action: impl FnOnce() + Send + 'static) -> Self {
        self.on_completion = Some(Box::new(action));
        self
    }

    pub(crate) fn complete(&mut self) {
        if let Some(action) = self.on_completion.take() {
            action();
        }
    }
}

impl Drop for BackendJobOwnership {
    fn drop(&mut self) {
        self.complete();
    }
}

/// Backend requested by configuration or environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendRequest {
    /// Probe for the best supported backend.
    Auto,
    /// Force the portable positioned-I/O worker pool.
    Pread,
    /// Request the Linux `io_uring` backend, falling back when unavailable.
    Uring,
}

impl BackendRequest {
    /// Parse a user-facing backend name. Unknown values use `Auto` so typos do
    /// not break startup; the selected backend reason records the fallback.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "pread" | "threadpool" | "thread-pool" => Self::Pread,
            "uring" | "io_uring" | "io-uring" => Self::Uring,
            "auto" | "" => Self::Auto,
            _ => Self::Auto,
        }
    }
}

/// Concrete backend selected after probing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Pread,
    Uring,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pread => "pread",
            Self::Uring => "uring",
        }
    }
}

/// Probe result exposed for diagnostics and tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackendSelection {
    pub requested: BackendRequest,
    pub selected: BackendKind,
    pub reason: String,
}

/// How the selected backend uses registered fixed buffers, when available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixedBufferStrategy {
    /// No fixed buffers are registered or used.
    Disabled,
    /// The backend registers private worker buffers and copies between those
    /// buffers and application frames at completion/submission time.
    WorkerCopy,
    /// The backend can submit application frame-pool slots directly.
    FramePoolSlots,
}

impl FixedBufferStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::WorkerCopy => "worker_copy",
            Self::FramePoolSlots => "frame_pool_slots",
        }
    }

    pub fn uses_worker_copy(self) -> bool {
        matches!(self, Self::WorkerCopy)
    }

    pub fn uses_frame_pool_slots(self) -> bool {
        matches!(self, Self::FramePoolSlots)
    }
}

/// Probe-selected disk backend.
///
/// The enum keeps the scheduler/runtime selection decision narrow while the
/// Linux backend grows from positioned SQEs and registered file slots to fixed
/// buffers.
pub struct SelectedDiskBackend {
    selection: BackendSelection,
    inner: SelectedDiskBackendInner,
}

enum SelectedDiskBackendInner {
    Pread(PreadBackend),
    #[cfg(target_os = "linux")]
    Uring(UringBackend),
    #[cfg(test)]
    Injected(Arc<dyn DiskBackend>),
}

impl fmt::Debug for SelectedDiskBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SelectedDiskBackend")
            .field("selection", &self.selection)
            .finish_non_exhaustive()
    }
}

impl SelectedDiskBackend {
    /// Select and construct a backend with an explicit worker-thread budget.
    pub fn select(requested: BackendRequest, threads: usize) -> Self {
        Self::select_with_queue_depth(requested, threads, DEFAULT_BACKEND_QUEUE_DEPTH)
    }

    /// Select and construct a backend with explicit worker and queue budgets.
    pub fn select_with_queue_depth(
        requested: BackendRequest,
        threads: usize,
        queue_depth: usize,
    ) -> Self {
        match requested {
            BackendRequest::Pread => Self::pread(
                requested,
                threads,
                queue_depth,
                "forced by storage backend configuration".to_string(),
            ),
            // `Auto` now takes the identical probe-and-fallback path as an
            // explicit `Uring` request: probe, use io_uring when the probe
            // reports it usable, otherwise fall back to the portable pread
            // pool. This reuses the same tested fallback logic that the
            // explicit request already exercised, so a kernel/container
            // where the probe fails (old kernel, io_uring disabled, no
            // registered-file support) is unaffected either way. Validated
            // via the `rt-storage` backend test suite plus a real-device
            // NVMe/btrfs roundtrip and throughput comparison (see
            // `scripts/storage_real_device_benchmark.sh`); this is
            // functional-correctness and parity evidence on the hardware
            // available in-repo, not the broader target-hardware fleet soak
            // `docs/STORAGE_NG.md` originally described — operators who need
            // the conservative baseline can still force `pread` explicitly.
            BackendRequest::Auto | BackendRequest::Uring => match UringBackend::probe() {
                Ok(probe) if probe.usable => {
                    #[cfg(not(target_os = "linux"))]
                    {
                        Self::pread(requested, threads, queue_depth, probe.reason)
                    }
                    #[cfg(target_os = "linux")]
                    Self::uring(requested, threads, queue_depth, probe.reason)
                }
                Ok(probe) => Self::pread(requested, threads, queue_depth, probe.reason),
                Err(reason) => Self::pread(requested, threads, queue_depth, reason),
            },
        }
    }

    /// Select using the default worker sizing for the fallback path.
    pub fn select_default(requested: BackendRequest) -> Self {
        let threads = default_worker_threads();
        Self::select(requested, threads)
    }

    pub fn selection(&self) -> &BackendSelection {
        &self.selection
    }

    pub(crate) fn uses_pread_workers(&self) -> bool {
        matches!(&self.inner, SelectedDiskBackendInner::Pread(_))
    }

    /// Execute one complete path-backed operation in the portable disk
    /// worker queue. io_uring keeps its own submission/completion lifecycle.
    pub(crate) async fn run_prepared_pread<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, StorageError> + Send + 'static,
    {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.run_prepared(f).await,
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(_) => Err(StorageError::io(
                "prepared pread",
                io::Error::other("prepared path operation requires the portable backend"),
            )),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(_) => Err(StorageError::io(
                "prepared pread",
                io::Error::other("prepared path operation is unavailable for injected backends"),
            )),
        }
    }

    /// Construct a scheduler backend around a scripted test implementation.
    /// The adapter is test-only so production cannot bypass the selected
    /// backend or descriptor ownership rules.
    #[cfg(test)]
    pub(crate) fn with_test_backend(backend: Arc<dyn DiskBackend>) -> Self {
        Self {
            selection: BackendSelection {
                requested: BackendRequest::Pread,
                selected: BackendKind::Pread,
                reason: "scripted test backend".to_string(),
            },
            inner: SelectedDiskBackendInner::Injected(backend),
        }
    }

    pub(crate) fn pread_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        frame: Frame,
        offset: u64,
        ownership: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<Frame>> {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => {
                backend.pread_leased(file, frame, offset, ownership)
            }
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => {
                backend.pread_leased(file, frame, offset, ownership)
            }
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => match test_file_clone(&file) {
                Ok(file) => {
                    retain_test_read_ownership(backend.pread(file, frame, offset), ownership)
                }
                Err(error) => {
                    let mut ownership = ownership;
                    ownership.complete();
                    completed_error(error)
                }
            },
        }
    }

    pub(crate) fn pwrite_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        data: bytes::Bytes,
        offset: u64,
        ownership: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<()>> {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => {
                backend.pwrite_leased(file, data, offset, ownership)
            }
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => {
                backend.pwrite_leased(file, data, offset, ownership)
            }
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => match test_file_clone(&file) {
                Ok(file) => {
                    retain_test_backend_ownership(backend.pwrite(file, data, offset), ownership)
                }
                Err(error) => {
                    let mut ownership = ownership;
                    ownership.complete();
                    completed_error(error)
                }
            },
        }
    }

    pub(crate) fn fdatasync_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        ownership: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<()>> {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.fdatasync_leased(file, ownership),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.fdatasync_leased(file, ownership),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => match test_file_clone(&file) {
                Ok(file) => retain_test_backend_ownership(backend.fdatasync(file), ownership),
                Err(error) => {
                    let mut ownership = ownership;
                    ownership.complete();
                    completed_error(error)
                }
            },
        }
    }

    pub fn kind(&self) -> BackendKind {
        self.selection.selected
    }

    /// Bound the active window admitted by the per-device dispatcher while
    /// leaving the remainder of its queue available for class-aware
    /// selection. The portable worker pool progresses one job per worker;
    /// io_uring continuously refills a bounded set of outstanding operations.
    pub(crate) fn device_dispatch_window(
        &self,
        worker_threads: usize,
        queue_capacity: usize,
    ) -> usize {
        let preferred = match self.kind() {
            BackendKind::Pread => worker_threads.max(1),
            BackendKind::Uring => {
                #[cfg(target_os = "linux")]
                {
                    URING_BATCH_LIMIT
                }
                #[cfg(not(target_os = "linux"))]
                {
                    worker_threads.max(1)
                }
            }
        };
        preferred.min(queue_capacity.max(1))
    }

    fn pread(
        requested: BackendRequest,
        threads: usize,
        queue_depth: usize,
        reason: String,
    ) -> Self {
        let (backend, reason) = match PreadBackend::try_new_with_queue_depth(threads, queue_depth) {
            Ok(backend) => (backend, reason),
            Err(error) => {
                tracing::error!(
                    component = "storage",
                    operation = "spawn_pread_workers",
                    result = "degraded",
                    error = %error,
                    "pread worker startup failed; storage operations will fail closed"
                );
                (
                    PreadBackend::unavailable(),
                    format!("{reason}; pread worker startup failed: {error}"),
                )
            }
        };
        let selection = BackendSelection {
            requested,
            selected: BackendKind::Pread,
            reason,
        };
        Self {
            selection,
            inner: SelectedDiskBackendInner::Pread(backend),
        }
    }

    #[cfg(target_os = "linux")]
    fn uring(
        requested: BackendRequest,
        threads: usize,
        queue_depth: usize,
        reason: String,
    ) -> Self {
        let backend = match UringBackend::try_new_with_queue_depth(threads, queue_depth) {
            Ok(backend) => backend,
            Err(error) => {
                return Self::pread(
                    requested,
                    threads,
                    queue_depth,
                    format!("{reason}; io_uring worker startup failed: {error}"),
                );
            }
        };
        let selection = BackendSelection {
            requested,
            selected: BackendKind::Uring,
            reason,
        };
        Self {
            selection,
            inner: SelectedDiskBackendInner::Uring(backend),
        }
    }
}

#[cfg(test)]
fn test_file_clone(file: &LeasedFileHandle) -> io::Result<Arc<File>> {
    file.as_file().try_clone().map(Arc::new)
}

#[cfg(test)]
fn completed_error<T>(error: io::Error) -> oneshot::Receiver<io::Result<T>> {
    let (sender, receiver) = oneshot::channel();
    let _ = sender.send(Err(error));
    receiver
}

#[cfg(test)]
fn retain_test_backend_ownership<T: Send + 'static>(
    receiver: oneshot::Receiver<io::Result<T>>,
    mut ownership: BackendJobOwnership,
) -> oneshot::Receiver<io::Result<T>> {
    let (sender, result) = oneshot::channel();
    tokio::spawn(async move {
        let result = receiver.await.unwrap_or_else(|_| {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "test storage backend completion channel closed",
            ))
        });
        ownership.complete();
        let _ = sender.send(result);
    });
    result
}

#[cfg(test)]
fn retain_test_read_ownership(
    receiver: oneshot::Receiver<io::Result<Frame>>,
    mut ownership: BackendJobOwnership,
) -> oneshot::Receiver<io::Result<Frame>> {
    let (sender, result) = oneshot::channel();
    tokio::spawn(async move {
        let result = receiver
            .await
            .unwrap_or_else(|_| {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "test storage backend completion channel closed",
                ))
            })
            .map(|mut frame| {
                frame.mark_initialized();
                frame
            });
        ownership.complete();
        let _ = sender.send(result);
    });
    result
}

impl DiskBackend for SelectedDiskBackend {
    fn pread(
        &self,
        file: Arc<File>,
        frame: Frame,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<Frame>> {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.pread(file, frame, offset),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.pread(file, frame, offset),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => backend.pread(file, frame, offset),
        }
    }

    fn pwrite(
        &self,
        file: Arc<File>,
        data: bytes::Bytes,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<()>> {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.pwrite(file, data, offset),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.pwrite(file, data, offset),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => backend.pwrite(file, data, offset),
        }
    }

    fn fdatasync(&self, file: Arc<File>) -> oneshot::Receiver<io::Result<()>> {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.fdatasync(file),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.fdatasync(file),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => backend.fdatasync(file),
        }
    }

    fn supports_fixed_buffers(&self) -> bool {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.supports_fixed_buffers(),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.supports_fixed_buffers(),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => backend.supports_fixed_buffers(),
        }
    }

    fn supports_registered_files(&self) -> bool {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.supports_registered_files(),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.supports_registered_files(),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => backend.supports_registered_files(),
        }
    }

    fn max_batch_len(&self) -> usize {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.max_batch_len(),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.max_batch_len(),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => backend.max_batch_len(),
        }
    }

    fn fixed_buffer_len(&self) -> usize {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.fixed_buffer_len(),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.fixed_buffer_len(),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => backend.fixed_buffer_len(),
        }
    }

    fn fixed_buffer_strategy(&self) -> FixedBufferStrategy {
        match &self.inner {
            SelectedDiskBackendInner::Pread(backend) => backend.fixed_buffer_strategy(),
            #[cfg(target_os = "linux")]
            SelectedDiskBackendInner::Uring(backend) => backend.fixed_buffer_strategy(),
            #[cfg(test)]
            SelectedDiskBackendInner::Injected(backend) => backend.fixed_buffer_strategy(),
        }
    }
}

/// Probe information for the `io_uring` backend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UringProbe {
    pub usable: bool,
    pub registered_files: bool,
    pub fixed_buffers: bool,
    pub reason: String,
}

/// Linux `io_uring` backend.
#[cfg(target_os = "linux")]
pub struct UringBackend {
    tx: mpsc::SyncSender<Job>,
    _workers: Vec<thread::JoinHandle<()>>,
    registered_files_supported: Arc<AtomicBool>,
    fixed_buffers_supported: Arc<AtomicBool>,
}

#[cfg(not(target_os = "linux"))]
pub struct UringBackend;

#[cfg(target_os = "linux")]
impl UringBackend {
    pub fn try_new(threads: usize) -> io::Result<Self> {
        Self::try_new_with_queue_depth(threads, DEFAULT_BACKEND_QUEUE_DEPTH)
    }

    pub fn try_new_with_queue_depth(threads: usize, queue_depth: usize) -> io::Result<Self> {
        let threads = threads.max(1);
        let (tx, rx) = mpsc::sync_channel::<Job>(queue_depth.max(1));
        let rx = Arc::new(Mutex::new(rx));
        let registered_files_supported = Arc::new(AtomicBool::new(false));
        let fixed_buffers_supported = Arc::new(AtomicBool::new(false));
        let mut workers: Vec<thread::JoinHandle<()>> = Vec::with_capacity(threads);
        for i in 0..threads {
            let rx = Arc::clone(&rx);
            let registered_files_supported = Arc::clone(&registered_files_supported);
            let fixed_buffers_supported = Arc::clone(&fixed_buffers_supported);
            let (ready_tx, ready_rx) = mpsc::channel();
            let handle =
                match thread::Builder::new()
                    .name(format!("tng-uring-{i}"))
                    .spawn(move || {
                        match UringWorker::new(
                            rx,
                            registered_files_supported,
                            fixed_buffers_supported,
                        ) {
                            Ok(worker) => {
                                let _ = ready_tx.send(Ok(()));
                                worker.run();
                            }
                            Err(error) => {
                                let _ = ready_tx.send(Err(error));
                            }
                        }
                    }) {
                    Ok(handle) => handle,
                    Err(error) => {
                        drop(tx);
                        for worker in workers {
                            let _ = worker.join();
                        }
                        return Err(error);
                    }
                };
            match ready_rx.recv() {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    drop(tx);
                    let _ = handle.join();
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(error);
                }
                Err(_) => {
                    drop(tx);
                    let _ = handle.join();
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "io_uring worker exited during startup",
                    ));
                }
            }
            workers.push(handle);
        }
        Ok(Self {
            tx,
            _workers: workers,
            registered_files_supported,
            fixed_buffers_supported,
        })
    }

    fn pread_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        frame: Frame,
        offset: u64,
        owner: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<Frame>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Read {
            file: BackendFile::Leased(file),
            frame,
            offset,
            reply,
            owner: Some(owner),
        }));
        rx
    }

    fn pwrite_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        data: bytes::Bytes,
        offset: u64,
        owner: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<()>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Write {
            file: BackendFile::Leased(file),
            data,
            offset,
            reply,
            owner: Some(owner),
        }));
        rx
    }

    fn fdatasync_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        owner: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<()>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Sync {
            file: BackendFile::Leased(file),
            reply,
            owner: Some(owner),
        }));
        rx
    }

    pub fn probe() -> Result<UringProbe, String> {
        #[cfg(target_os = "linux")]
        {
            let disabled = std::fs::read_to_string("/proc/sys/kernel/io_uring_disabled")
                .ok()
                .and_then(|value| value.trim().parse::<u32>().ok())
                .unwrap_or(0);
            if disabled != 0 {
                return Ok(UringProbe {
                    usable: false,
                    registered_files: false,
                    fixed_buffers: false,
                    reason: format!("kernel reports io_uring disabled ({disabled})"),
                });
            }
            let ring = match IoUring::new(8) {
                Ok(ring) => ring,
                Err(e) => {
                    return Ok(UringProbe {
                        usable: false,
                        registered_files: false,
                        fixed_buffers: false,
                        reason: format!("io_uring probe failed: {e}"),
                    });
                }
            };
            let registered_files = probe_registered_files(&ring).is_ok();
            let fixed_buffers = probe_fixed_buffers(&ring).is_ok();
            let reason = format!(
                "io_uring probe succeeded; registered_files={registered_files} fixed_buffers={fixed_buffers}"
            );
            Ok(UringProbe {
                usable: true,
                registered_files,
                fixed_buffers,
                reason,
            })
        }

        #[cfg(not(target_os = "linux"))]
        {
            Ok(UringProbe {
                usable: false,
                registered_files: false,
                fixed_buffers: false,
                reason: "io_uring is only available on Linux; using pread fallback".to_string(),
            })
        }
    }
}

#[cfg(not(target_os = "linux"))]
impl UringBackend {
    pub fn probe() -> Result<UringProbe, String> {
        Ok(UringProbe {
            usable: false,
            registered_files: false,
            fixed_buffers: false,
            reason: "io_uring is only available on Linux; using pread fallback".to_string(),
        })
    }
}

#[cfg(target_os = "linux")]
fn probe_registered_files(ring: &IoUring) -> io::Result<()> {
    ring.submitter().register_files_sparse(1)?;
    ring.submitter().unregister_files()
}

#[cfg(target_os = "linux")]
fn probe_fixed_buffers(ring: &IoUring) -> io::Result<()> {
    let mut buffers = (0..URING_FIXED_BUFFER_SLOTS)
        .map(|_| vec![0u8; URING_FIXED_BUFFER_LEN])
        .collect::<Vec<_>>();
    let iovecs = buffers
        .iter_mut()
        .map(|buf| libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        })
        .collect::<Vec<_>>();
    unsafe {
        ring.submitter().register_buffers(&iovecs)?;
    }
    ring.submitter().unregister_buffers()
}

#[cfg(target_os = "linux")]
impl DiskBackend for UringBackend {
    fn pread(
        &self,
        file: Arc<File>,
        frame: Frame,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<Frame>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Read {
            file: BackendFile::Unleased(file),
            frame,
            offset,
            reply,
            owner: None,
        }));
        rx
    }

    fn pwrite(
        &self,
        file: Arc<File>,
        data: bytes::Bytes,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<()>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Write {
            file: BackendFile::Unleased(file),
            data,
            offset,
            reply,
            owner: None,
        }));
        rx
    }

    fn fdatasync(&self, file: Arc<File>) -> oneshot::Receiver<io::Result<()>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Sync {
            file: BackendFile::Unleased(file),
            reply,
            owner: None,
        }));
        rx
    }

    fn supports_fixed_buffers(&self) -> bool {
        self.fixed_buffers_supported.load(Ordering::Relaxed)
    }

    fn supports_registered_files(&self) -> bool {
        self.registered_files_supported.load(Ordering::Relaxed)
    }

    fn max_batch_len(&self) -> usize {
        URING_BATCH_LIMIT
    }

    fn fixed_buffer_len(&self) -> usize {
        if self.supports_fixed_buffers() {
            URING_FIXED_BUFFER_LEN
        } else {
            0
        }
    }

    fn fixed_buffer_strategy(&self) -> FixedBufferStrategy {
        if self.supports_fixed_buffers() {
            FixedBufferStrategy::FramePoolSlots
        } else {
            FixedBufferStrategy::Disabled
        }
    }
}

#[cfg(target_os = "linux")]
struct UringWorker {
    rx: Arc<Mutex<mpsc::Receiver<Job>>>,
    ring: IoUring,
    fixed: Option<Arc<RegisteredFrameSlots>>,
    next_id: u64,
    registered_files: bool,
    file_slots: HashMap<FileIdentity, u32>,
    slot_files: Vec<Option<FileIdentity>>,
    file_slot_refs: Vec<usize>,
    pending: HashMap<u64, PendingUring>,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
struct FileIdentity {
    // The handle cache keeps a read-only and a read/write descriptor for the
    // same path. io_uring's fixed-file table preserves the descriptor's access
    // mode, so inode-only identity could route a write through a read-only
    // descriptor and fail with EBADF.
    //
    // A descriptor remains live while its slot is referenced by `pending`, so
    // its number cannot be reused until the slot is released.
    fd: i32,
}

#[cfg(target_os = "linux")]
#[derive(Debug, Clone, Copy)]
struct RegisteredFileSlot {
    index: u32,
}

#[cfg(target_os = "linux")]
fn file_identity(file: &File) -> FileIdentity {
    FileIdentity {
        fd: file.as_raw_fd(),
    }
}

#[cfg(target_os = "linux")]
enum PendingUring {
    Read {
        file: BackendFile,
        frame: Frame,
        expected: usize,
        fixed_slot: Option<RegisteredFrameSlot>,
        file_slot: Option<RegisteredFileSlot>,
        reply: Option<oneshot::Sender<io::Result<Frame>>>,
        owner: Option<BackendJobOwnership>,
    },
    Write {
        file: BackendFile,
        data: bytes::Bytes,
        expected: usize,
        file_slot: Option<RegisteredFileSlot>,
        reply: Option<oneshot::Sender<io::Result<()>>>,
        owner: Option<BackendJobOwnership>,
    },
    Sync {
        file: BackendFile,
        file_slot: Option<RegisteredFileSlot>,
        reply: Option<oneshot::Sender<io::Result<()>>>,
        owner: Option<BackendJobOwnership>,
    },
}

#[cfg(target_os = "linux")]
impl UringWorker {
    fn new(
        rx: Arc<Mutex<mpsc::Receiver<Job>>>,
        registered_files_supported: Arc<AtomicBool>,
        fixed_buffers_supported: Arc<AtomicBool>,
    ) -> io::Result<Self> {
        let ring = IoUring::new(URING_ENTRIES)?;
        let registered_files = match ring.submitter().register_files_sparse(URING_FILE_SLOTS) {
            Ok(()) => {
                registered_files_supported.store(true, Ordering::Relaxed);
                true
            }
            Err(e) => {
                tracing::debug!(
                    component = "storage",
                    operation = "io_uring_register_files",
                    result = "unavailable",
                    error = %e,
                    "io_uring fixed-file table unavailable"
                );
                false
            }
        };
        let fixed = {
            let slots = RegisteredFrameSlots::new(URING_FIXED_BUFFER_SLOTS, URING_FIXED_BUFFER_LEN);
            let iovecs = slots.iovecs();
            match unsafe { ring.submitter().register_buffers(&iovecs) } {
                Ok(()) => {
                    fixed_buffers_supported.store(true, Ordering::Relaxed);
                    Some(slots)
                }
                Err(e) => {
                    tracing::debug!(
                        component = "storage",
                        operation = "io_uring_register_buffers",
                        result = "unavailable",
                        error = %e,
                        "io_uring fixed-buffer registration unavailable"
                    );
                    None
                }
            }
        };
        Ok(Self {
            rx,
            ring,
            fixed,
            next_id: 1,
            registered_files,
            file_slots: HashMap::new(),
            slot_files: vec![None; URING_FILE_SLOTS as usize],
            file_slot_refs: vec![0; URING_FILE_SLOTS as usize],
            pending: HashMap::new(),
        })
    }

    fn run(mut self) {
        let mut receiver_disconnected = false;
        loop {
            if self.pending.is_empty() {
                if receiver_disconnected {
                    return;
                }
                let Some(jobs) = self.recv_batch() else {
                    return;
                };
                self.submit_jobs(jobs);
            } else if !receiver_disconnected {
                let capacity = URING_BATCH_LIMIT.saturating_sub(self.pending.len());
                let (jobs, disconnected) = self.try_recv_batch(capacity);
                receiver_disconnected = disconnected;
                self.submit_jobs(jobs);
            }
            if self.pending.is_empty() {
                continue;
            }

            // Wait for the first completion, then drain every CQE already
            // ready. Waiting for the whole prior batch made one slow disk
            // operation hold up unrelated completed work and prevented the
            // ring from accepting newly queued work until the slowest CQE.
            loop {
                match self.ring.submit_and_wait(1) {
                    Ok(_) => break,
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        self.fail_all(e);
                        return;
                    }
                }
            }
            self.complete_ready();
        }
    }

    fn recv_batch(&self) -> Option<Vec<Job>> {
        let guard = lock_job_receiver(&self.rx);
        let mut jobs = Vec::with_capacity(URING_BATCH_LIMIT);
        jobs.push(guard.recv().ok()?);
        while jobs.len() < URING_BATCH_LIMIT {
            match guard.try_recv() {
                Ok(job) => jobs.push(job),
                Err(mpsc::TryRecvError::Empty) | Err(mpsc::TryRecvError::Disconnected) => break,
            }
        }
        Some(jobs)
    }

    fn try_recv_batch(&self, limit: usize) -> (Vec<Job>, bool) {
        if limit == 0 {
            return (Vec::new(), false);
        }
        let Some(guard) = try_lock_job_receiver(&self.rx) else {
            return (Vec::new(), false);
        };
        let mut jobs = Vec::with_capacity(limit);
        let disconnected = match guard.try_recv() {
            Ok(job) => {
                jobs.push(job);
                false
            }
            Err(mpsc::TryRecvError::Empty) => return (jobs, false),
            Err(mpsc::TryRecvError::Disconnected) => return (jobs, true),
        };
        let mut disconnected = disconnected;
        while jobs.len() < limit {
            match guard.try_recv() {
                Ok(job) => jobs.push(job),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        (jobs, disconnected)
    }

    fn submit_jobs(&mut self, jobs: Vec<Job>) {
        for job in jobs {
            if let Err(error) = self.submit_job(job) {
                tracing::warn!(
                    component = "storage",
                    operation = "io_uring_submit",
                    result = "error",
                    error = %error,
                    "io_uring submission failed"
                );
            }
        }
    }

    fn submit_job(&mut self, job: Job) -> io::Result<()> {
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        match job {
            Job::Prepared(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "prepared operations must use the portable disk worker pool",
                ));
            }
            Job::Read {
                file,
                mut frame,
                offset,
                reply,
                mut owner,
            } => {
                let len = frame.len();
                let file_slot = self.register_file_slot(file.as_file());
                let fixed_slot = self.fixed.as_ref().and_then(|fixed| fixed.acquire(len));
                let entry = if let Some(buf_slot) = fixed_slot.as_ref() {
                    let ptr = buf_slot.ptr_mut();
                    match file_slot.as_ref().map(|slot| slot.index) {
                        Some(file_slot) => opcode::ReadFixed::new(
                            types::Fixed(file_slot),
                            ptr,
                            len as _,
                            buf_slot.index(),
                        ),
                        None => opcode::ReadFixed::new(
                            types::Fd(file.as_file().as_raw_fd()),
                            ptr,
                            len as _,
                            buf_slot.index(),
                        ),
                    }
                    .offset(offset)
                    .build()
                    .user_data(id)
                } else {
                    let ptr = frame.as_mut_slice().as_mut_ptr();
                    match file_slot.as_ref().map(|slot| slot.index) {
                        Some(slot) => opcode::Read::new(types::Fixed(slot), ptr, len as _),
                        None => {
                            opcode::Read::new(types::Fd(file.as_file().as_raw_fd()), ptr, len as _)
                        }
                    }
                    .offset(offset)
                    .build()
                    .user_data(id)
                };
                if let Err(e) = self.push_entry(entry) {
                    self.release_submission_resources(file_slot, fixed_slot);
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    let _ = reply.send(Err(io::Error::new(e.kind(), e.to_string())));
                    return Err(e);
                }
                self.pending.insert(
                    id,
                    PendingUring::Read {
                        file,
                        frame,
                        expected: len,
                        fixed_slot,
                        file_slot,
                        reply: Some(reply),
                        owner,
                    },
                );
            }
            Job::Write {
                file,
                data,
                offset,
                reply,
                mut owner,
            } => {
                let len = data.len();
                let file_slot = self.register_file_slot(file.as_file());
                let ptr = data.as_ptr();
                let entry = match file_slot.as_ref().map(|slot| slot.index) {
                    Some(slot) => opcode::Write::new(types::Fixed(slot), ptr, len as _),
                    None => {
                        opcode::Write::new(types::Fd(file.as_file().as_raw_fd()), ptr, len as _)
                    }
                }
                .offset(offset)
                .build()
                .user_data(id);
                if let Err(e) = self.push_entry(entry) {
                    self.release_submission_resources(file_slot, None);
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    let _ = reply.send(Err(io::Error::new(e.kind(), e.to_string())));
                    return Err(e);
                }
                self.pending.insert(
                    id,
                    PendingUring::Write {
                        file,
                        data,
                        expected: len,
                        file_slot,
                        reply: Some(reply),
                        owner,
                    },
                );
            }
            Job::Sync {
                file,
                reply,
                mut owner,
            } => {
                let file_slot = self.register_file_slot(file.as_file());
                let entry = match file_slot.as_ref().map(|slot| slot.index) {
                    Some(slot) => opcode::Fsync::new(types::Fixed(slot)),
                    None => opcode::Fsync::new(types::Fd(file.as_file().as_raw_fd())),
                }
                .flags(types::FsyncFlags::DATASYNC)
                .build()
                .user_data(id);
                if let Err(e) = self.push_entry(entry) {
                    self.release_submission_resources(file_slot, None);
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    let _ = reply.send(Err(io::Error::new(e.kind(), e.to_string())));
                    return Err(e);
                }
                self.pending.insert(
                    id,
                    PendingUring::Sync {
                        file,
                        file_slot,
                        reply: Some(reply),
                        owner,
                    },
                );
            }
        }
        Ok(())
    }

    fn register_file_slot(&mut self, file: &File) -> Option<RegisteredFileSlot> {
        if !self.registered_files {
            return None;
        }
        let identity = file_identity(file);
        if let Some(slot) = self.file_slots.get(&identity) {
            self.file_slot_refs[*slot as usize] += 1;
            return Some(RegisteredFileSlot { index: *slot });
        }
        let slot = self.slot_files.iter().position(Option::is_none)? as u32;
        match self
            .ring
            .submitter()
            .register_files_update(slot, &[file.as_raw_fd()])
        {
            Ok(1) => {
                self.slot_files[slot as usize] = Some(identity);
                self.file_slots.insert(identity, slot);
                self.file_slot_refs[slot as usize] = 1;
                Some(RegisteredFileSlot { index: slot })
            }
            Ok(_) => None,
            Err(e) => {
                tracing::debug!(
                    component = "storage",
                    operation = "io_uring_update_fixed_file",
                    slot,
                    result = "error",
                    error = %e,
                    "io_uring fixed-file update failed"
                );
                None
            }
        }
    }

    fn release_submission_resources(
        &mut self,
        file_slot: Option<RegisteredFileSlot>,
        fixed_slot: Option<RegisteredFrameSlot>,
    ) {
        drop(fixed_slot);
        self.release_file_slot(file_slot);
    }

    fn release_file_slot(&mut self, file_slot: Option<RegisteredFileSlot>) {
        let Some(file_slot) = file_slot else {
            return;
        };
        let index = file_slot.index as usize;
        let Some(refs) = self.file_slot_refs.get_mut(index) else {
            return;
        };
        debug_assert!(*refs > 0);
        if *refs == 0 {
            return;
        }
        *refs -= 1;
        if *refs != 0 {
            return;
        }
        let Some(identity) = self.slot_files.get_mut(index).and_then(Option::take) else {
            return;
        };
        if self.file_slots.get(&identity).copied() == Some(file_slot.index) {
            self.file_slots.remove(&identity);
        }
        // The operation no longer has an SQE that can reference this slot.
        // Clear the sparse entry as well as the worker bookkeeping so
        // completed operations, inode reuse, and a transient full submission
        // queue cannot consume or misdirect the finite fixed-file table.
        if let Err(error) = self
            .ring
            .submitter()
            .register_files_update(file_slot.index, &[-1])
        {
            tracing::debug!(
                component = "storage",
                operation = "io_uring_release_fixed_file",
                result = "error",
                slot = file_slot.index,
                error = %error,
                    "io_uring fixed-file slot could not be cleared after operation release"
            );
        }
    }

    fn push_entry(&mut self, entry: io_uring::squeue::Entry) -> io::Result<()> {
        unsafe {
            self.ring
                .submission()
                .push(&entry)
                .map_err(|_| io::Error::new(io::ErrorKind::WouldBlock, "io_uring SQ full"))
        }
    }

    fn complete_ready(&mut self) {
        let completions = self
            .ring
            .completion()
            .map(|cqe| (cqe.user_data(), cqe.result()));
        for (id, result) in completions.collect::<Vec<_>>() {
            let Some(pending) = self.pending.remove(&id) else {
                continue;
            };
            match pending {
                PendingUring::Read {
                    file,
                    mut frame,
                    expected,
                    fixed_slot,
                    file_slot,
                    reply,
                    mut owner,
                } => {
                    let _keepalive = file;
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(match uring_result(result) {
                            Ok(n) if n == expected => {
                                frame.mark_initialized();
                                if let Some(slot) = fixed_slot {
                                    Ok(frame.into_registered_slot(slot))
                                } else {
                                    Ok(frame)
                                }
                            }
                            Ok(_) => Err(io::Error::new(
                                io::ErrorKind::UnexpectedEof,
                                "short io_uring read",
                            )),
                            Err(e) => Err(e),
                        });
                    }
                    self.release_file_slot(file_slot);
                }
                PendingUring::Write {
                    file,
                    data,
                    expected,
                    file_slot,
                    reply,
                    mut owner,
                } => {
                    let _keepalive_file = file;
                    let _keepalive = data;
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(match uring_result(result) {
                            Ok(n) if n == expected => Ok(()),
                            Ok(_) => Err(io::Error::new(
                                io::ErrorKind::WriteZero,
                                "short io_uring write",
                            )),
                            Err(e) => Err(e),
                        });
                    }
                    self.release_file_slot(file_slot);
                }
                PendingUring::Sync {
                    file,
                    file_slot,
                    reply,
                    mut owner,
                } => {
                    let _keepalive = file;
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    if let Some(reply) = reply {
                        let _ = reply.send(uring_result(result).map(|_| ()));
                    }
                    self.release_file_slot(file_slot);
                }
            }
        }
    }

    fn fail_all(&mut self, err: io::Error) {
        // A submission failure does not prove that the kernel has stopped
        // consuming the SQEs already published to the ring. Keep every
        // PendingUring value in place until `UringWorker` drops the ring; its
        // buffers, payloads, and file descriptors must remain valid for that
        // whole interval. The worker exits immediately after this method.
        let message = err.to_string();
        for pending in self.pending.values_mut() {
            let e = || io::Error::new(err.kind(), message.clone());
            match pending {
                PendingUring::Read { reply, .. } => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(Err(e()));
                    }
                }
                PendingUring::Write { reply, .. } => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(Err(e()));
                    }
                }
                PendingUring::Sync { reply, .. } => {
                    if let Some(reply) = reply.take() {
                        let _ = reply.send(Err(e()));
                    }
                }
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn uring_result(result: i32) -> io::Result<usize> {
    if result < 0 {
        Err(io::Error::from_raw_os_error(-result))
    } else {
        Ok(result as usize)
    }
}

/// Positioned disk operations. Implementors must be safe to call
/// concurrently against the same file handle (true for `pread`/`pwrite`).
pub trait DiskBackend: Send + Sync {
    /// Read exactly `frame.len()` bytes at `offset` into `frame`.
    fn pread(
        &self,
        file: Arc<File>,
        frame: Frame,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<Frame>>;

    /// Write all of `data` at `offset`.
    fn pwrite(
        &self,
        file: Arc<File>,
        data: bytes::Bytes,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<()>>;

    /// Flush file data (not necessarily metadata) to the device.
    fn fdatasync(&self, file: Arc<File>) -> oneshot::Receiver<io::Result<()>>;

    /// Whether the backend can register fixed buffers (io_uring). The
    /// pread backend cannot; the elevator uses this to decide pinning.
    fn supports_fixed_buffers(&self) -> bool {
        false
    }

    /// Whether the backend can use registered file slots.
    fn supports_registered_files(&self) -> bool {
        false
    }

    /// Maximum number of jobs the backend submits as one batch.
    fn max_batch_len(&self) -> usize {
        1
    }

    /// Size of each registered fixed buffer, or 0 when unsupported.
    fn fixed_buffer_len(&self) -> usize {
        0
    }

    /// Whether fixed-buffer submissions use backend-private copy buffers or
    /// application frame-pool slots directly.
    fn fixed_buffer_strategy(&self) -> FixedBufferStrategy {
        FixedBufferStrategy::Disabled
    }
}

enum BackendFile {
    Unleased(Arc<File>),
    Leased(Arc<LeasedFileHandle>),
}

impl BackendFile {
    fn as_file(&self) -> &File {
        match self {
            Self::Unleased(file) => file.as_ref(),
            Self::Leased(file) => file.as_file(),
        }
    }
}

enum Job {
    Prepared(PreparedJob),
    Read {
        file: BackendFile,
        frame: Frame,
        offset: u64,
        reply: oneshot::Sender<io::Result<Frame>>,
        owner: Option<BackendJobOwnership>,
    },
    Write {
        file: BackendFile,
        data: bytes::Bytes,
        offset: u64,
        reply: oneshot::Sender<io::Result<()>>,
        owner: Option<BackendJobOwnership>,
    },
    Sync {
        file: BackendFile,
        reply: oneshot::Sender<io::Result<()>>,
        owner: Option<BackendJobOwnership>,
    },
}

type PreparedJob = Box<dyn FnOnce() + Send + 'static>;

const PREPARED_JOB_QUEUED: u8 = 0;
const PREPARED_JOB_STARTED: u8 = 1;
const PREPARED_JOB_CANCELLED: u8 = 2;

struct PreparedJobCancelGuard(Arc<AtomicU8>);

impl Drop for PreparedJobCancelGuard {
    fn drop(&mut self) {
        let _ = self.0.compare_exchange(
            PREPARED_JOB_QUEUED,
            PREPARED_JOB_CANCELLED,
            AtomicOrdering::AcqRel,
            AtomicOrdering::Acquire,
        );
    }
}

fn read_exact_with(
    mut buf: &mut [u8],
    mut offset: u64,
    mut read_at: impl FnMut(&mut [u8], u64) -> io::Result<usize>,
) -> io::Result<()> {
    while !buf.is_empty() {
        match read_at(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => {
                if n > buf.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "positioned read reported more bytes than requested",
                    ));
                }
                offset = checked_io_offset(offset, n)?;
                let (_, rest) = buf.split_at_mut(n);
                buf = rest;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn write_all_with(
    mut buf: &[u8],
    mut offset: u64,
    mut write_at: impl FnMut(&[u8], u64) -> io::Result<usize>,
) -> io::Result<()> {
    while !buf.is_empty() {
        match write_at(buf, offset) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => {
                if n > buf.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "positioned write reported more bytes than requested",
                    ));
                }
                offset = checked_io_offset(offset, n)?;
                buf = &buf[n..];
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    read_exact_with(buf, offset, |buffer, position| {
        file.read_at(buffer, position)
    })
}

#[cfg(any(unix, windows))]
pub(crate) fn read_frame_leased(
    file: &LeasedFileHandle,
    mut frame: Frame,
    offset: u64,
) -> io::Result<Frame> {
    read_exact_at(file.as_file(), frame.as_mut_slice(), offset)?;
    frame.mark_initialized();
    Ok(frame)
}

#[cfg(windows)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    read_exact_with(buf, offset, |buffer, position| {
        file.seek_read(buffer, position)
    })
}

#[cfg(unix)]
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    write_all_with(buf, offset, |buffer, position| {
        file.write_at(buffer, position)
    })
}

#[cfg(windows)]
fn write_all_at(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    write_all_with(buf, offset, |buffer, position| {
        file.seek_write(buffer, position)
    })
}

fn checked_io_offset(offset: u64, bytes: usize) -> io::Result<u64> {
    offset
        .checked_add(u64::try_from(bytes).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "I/O byte count does not fit in u64",
            )
        })?)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "I/O offset overflow"))
}

/// `pread`/`pwrite` backend backed by a fixed pool of dedicated OS
/// threads. Threads block on real syscalls; the async caller awaits a
/// `oneshot`.
pub struct PreadBackend {
    tx: mpsc::SyncSender<Job>,
    _workers: Vec<thread::JoinHandle<()>>,
}

impl PreadBackend {
    /// Spawn `threads` dedicated I/O workers (clamped to at least 1).
    pub fn new(threads: usize) -> Self {
        Self::new_with_queue_depth(threads, DEFAULT_BACKEND_QUEUE_DEPTH)
    }

    pub fn new_with_queue_depth(threads: usize, queue_depth: usize) -> Self {
        match Self::try_new_with_queue_depth(threads, queue_depth) {
            Ok(backend) => backend,
            Err(error) => {
                tracing::error!(
                    component = "storage",
                    operation = "spawn_pread_workers",
                    result = "degraded",
                    error = %error,
                    "pread worker startup failed; storage operations will fail closed"
                );
                Self::unavailable()
            }
        }
    }

    pub fn try_new_with_queue_depth(threads: usize, queue_depth: usize) -> io::Result<Self> {
        let threads = threads.max(1);
        let (tx, rx) = mpsc::sync_channel::<Job>(queue_depth.max(1));
        let rx = Arc::new(Mutex::new(rx));
        let mut workers: Vec<thread::JoinHandle<()>> = Vec::with_capacity(threads);
        for i in 0..threads {
            let rx = Arc::clone(&rx);
            let handle = match thread::Builder::new()
                .name(format!("tng-disk-{i}"))
                .spawn(move || Self::worker(rx))
            {
                Ok(handle) => handle,
                Err(error) => {
                    drop(tx);
                    for worker in workers {
                        let _ = worker.join();
                    }
                    return Err(error);
                }
            };
            workers.push(handle);
        }
        Ok(PreadBackend {
            tx,
            _workers: workers,
        })
    }

    fn unavailable() -> Self {
        let (tx, rx) = mpsc::sync_channel(1);
        drop(rx);
        PreadBackend {
            tx,
            _workers: Vec::new(),
        }
    }

    /// Default sizing: scale with cores but stay modest — disk, not CPU,
    /// is the constraint. The elevator above bounds true concurrency.
    pub fn with_default_threads() -> Self {
        Self::new(default_worker_threads())
    }

    fn pread_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        frame: Frame,
        offset: u64,
        owner: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<Frame>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Read {
            file: BackendFile::Leased(file),
            frame,
            offset,
            reply,
            owner: Some(owner),
        }));
        rx
    }

    fn pwrite_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        data: bytes::Bytes,
        offset: u64,
        owner: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<()>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Write {
            file: BackendFile::Leased(file),
            data,
            offset,
            reply,
            owner: Some(owner),
        }));
        rx
    }

    fn fdatasync_leased(
        &self,
        file: Arc<LeasedFileHandle>,
        owner: BackendJobOwnership,
    ) -> oneshot::Receiver<io::Result<()>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Sync {
            file: BackendFile::Leased(file),
            reply,
            owner: Some(owner),
        }));
        rx
    }

    /// Run a complete prepared storage operation on the same bounded worker
    /// pool that performs positioned I/O. This lets the scheduler keep path
    /// normalization, descriptor lookup, cache checks, the read, and final
    /// identity validation behind one queue and one completion.
    pub(crate) async fn run_prepared<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T, StorageError> + Send + 'static,
    {
        let (reply, result) = oneshot::channel();
        let state = Arc::new(AtomicU8::new(PREPARED_JOB_QUEUED));
        let worker_state = Arc::clone(&state);
        let job: PreparedJob = Box::new(move || {
            if worker_state
                .compare_exchange(
                    PREPARED_JOB_QUEUED,
                    PREPARED_JOB_STARTED,
                    AtomicOrdering::AcqRel,
                    AtomicOrdering::Acquire,
                )
                .is_err()
            {
                return;
            }
            let _ = reply.send(f());
        });
        let _cancel_guard = PreparedJobCancelGuard(state);
        match self.tx.try_send(Job::Prepared(job)) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {
                return Err(StorageError::QueueFull {
                    mount: "disk-backend".to_string(),
                });
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                return Err(StorageError::io(
                    "disk-backend",
                    io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "portable disk worker pool is closed",
                    ),
                ));
            }
        }
        result.await.map_err(|_| StorageError::Cancelled)?
    }

    fn worker(rx: Arc<Mutex<mpsc::Receiver<Job>>>) {
        loop {
            // Lock only to dequeue; the actual syscall runs unlocked so
            // workers process jobs in parallel.
            let job = {
                let guard = lock_job_receiver(&rx);
                guard.recv()
            };
            let Ok(job) = job else {
                return; // sender dropped → shutdown
            };
            match job {
                Job::Prepared(job) => job(),
                Job::Read {
                    file,
                    mut frame,
                    offset,
                    reply,
                    mut owner,
                } => {
                    let res =
                        read_exact_at(file.as_file(), frame.as_mut_slice(), offset).map(|()| {
                            frame.mark_initialized();
                            frame
                        });
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    let _ = reply.send(res);
                }
                Job::Write {
                    file,
                    data,
                    offset,
                    reply,
                    mut owner,
                } => {
                    let res = write_all_at(file.as_file(), &data, offset);
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    let _ = reply.send(res);
                }
                Job::Sync {
                    file,
                    reply,
                    mut owner,
                } => {
                    let res = file.as_file().sync_data();
                    if let Some(owner) = owner.as_mut() {
                        owner.complete();
                    }
                    let _ = reply.send(res);
                }
            }
        }
    }
}

pub fn default_worker_threads() -> usize {
    let cores = thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    (cores / 2).clamp(2, 8)
}

impl DiskBackend for PreadBackend {
    fn pread(
        &self,
        file: Arc<File>,
        frame: Frame,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<Frame>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Read {
            file: BackendFile::Unleased(file),
            frame,
            offset,
            reply,
            owner: None,
        }));
        rx
    }

    fn pwrite(
        &self,
        file: Arc<File>,
        data: bytes::Bytes,
        offset: u64,
    ) -> oneshot::Receiver<io::Result<()>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Write {
            file: BackendFile::Unleased(file),
            data,
            offset,
            reply,
            owner: None,
        }));
        rx
    }

    fn fdatasync(&self, file: Arc<File>) -> oneshot::Receiver<io::Result<()>> {
        let (reply, rx) = oneshot::channel();
        fail_job_on_full(self.tx.try_send(Job::Sync {
            file: BackendFile::Unleased(file),
            reply,
            owner: None,
        }));
        rx
    }
}

fn fail_job_on_full(result: Result<(), mpsc::TrySendError<Job>>) {
    match result {
        Ok(()) => {}
        Err(mpsc::TrySendError::Full(job)) => {
            fail_job(job, io::ErrorKind::WouldBlock, "storage backend queue full")
        }
        Err(mpsc::TrySendError::Disconnected(job)) => fail_job(
            job,
            io::ErrorKind::BrokenPipe,
            "storage backend queue closed",
        ),
    }
}

fn fail_job(job: Job, kind: io::ErrorKind, message: &'static str) {
    let error = || io::Error::new(kind, message);
    match job {
        Job::Prepared(_) => {}
        Job::Read {
            reply, mut owner, ..
        } => {
            if let Some(owner) = owner.as_mut() {
                owner.complete();
            }
            let _ = reply.send(Err(error()));
        }
        Job::Write {
            reply, mut owner, ..
        }
        | Job::Sync {
            reply, mut owner, ..
        } => {
            if let Some(owner) = owner.as_mut() {
                owner.complete();
            }
            let _ = reply.send(Err(error()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::FramePool;
    use crate::handle_cache::HandleCache;
    use std::time::Duration;

    struct PreparedDropProbe {
        dropped: Arc<std::sync::atomic::AtomicBool>,
        notify: Arc<tokio::sync::Notify>,
    }

    impl Drop for PreparedDropProbe {
        fn drop(&mut self) {
            self.dropped.store(true, AtomicOrdering::Release);
            self.notify.notify_one();
        }
    }

    #[test]
    fn queued_job_retains_descriptor_lease_after_caller_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queued-lease.bin");
        std::fs::write(&path, b"payload").unwrap();
        let cache = HandleCache::new(0, Duration::from_secs(30));
        let operation = cache.get_or_open(&path, false, false).unwrap();
        let handle = operation.leased_handle();
        let (tx, queued) = mpsc::sync_channel(1);
        let backend = PreadBackend {
            tx,
            _workers: Vec::new(),
        };

        let completion =
            backend.fdatasync_leased(Arc::clone(&handle), BackendJobOwnership::untracked());
        drop(completion); // Simulate a caller cancelling after queue admission.
        drop(handle);
        drop(operation);
        assert_eq!(cache.active_descriptors(), 1);

        let job = queued.try_recv().unwrap();
        assert_eq!(cache.active_descriptors(), 1);
        drop(job); // Backend completion releases the final descriptor owner.
        assert_eq!(cache.active_descriptors(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn pending_uring_job_retains_descriptor_lease_until_completion() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uring-lease.bin");
        std::fs::write(&path, b"payload").unwrap();
        let cache = HandleCache::new(0, Duration::from_secs(30));
        let operation = cache.get_or_open(&path, false, false).unwrap();
        let handle = operation.leased_handle();
        let pending = PendingUring::Sync {
            file: BackendFile::Leased(Arc::clone(&handle)),
            file_slot: None,
            reply: None,
            owner: None,
        };

        drop(handle);
        drop(operation);
        assert_eq!(cache.active_descriptors(), 1);
        drop(pending);
        assert_eq!(cache.active_descriptors(), 0);
    }

    #[test]
    fn poisoned_backend_receiver_preserves_queued_work() {
        let (sender, receiver) = mpsc::channel();
        sender.send(41_u8).unwrap();
        let receiver = Arc::new(Mutex::new(receiver));

        let poisoner = Arc::clone(&receiver);
        assert!(std::thread::spawn(move || {
            let _guard = poisoner.lock().unwrap();
            panic!("poison the bounded disk-job receiver");
        })
        .join()
        .is_err());
        assert!(receiver.is_poisoned());

        assert_eq!(lock_job_receiver(&receiver).recv().unwrap(), 41);
        assert!(!receiver.is_poisoned());
    }

    #[test]
    fn backend_request_parses_user_values() {
        assert_eq!(BackendRequest::parse("auto"), BackendRequest::Auto);
        assert_eq!(BackendRequest::parse("pread"), BackendRequest::Pread);
        assert_eq!(BackendRequest::parse("thread-pool"), BackendRequest::Pread);
        assert_eq!(BackendRequest::parse("io_uring"), BackendRequest::Uring);
        assert_eq!(BackendRequest::parse("surprise"), BackendRequest::Auto);
    }

    #[test]
    fn forcing_pread_selects_pread() {
        let backend = SelectedDiskBackend::select(BackendRequest::Pread, 1);
        assert_eq!(backend.kind(), BackendKind::Pread);
        assert_eq!(backend.selection().requested, BackendRequest::Pread);
        assert!(!backend.supports_fixed_buffers());
        assert!(!backend.supports_registered_files());
        assert_eq!(backend.max_batch_len(), 1);
        assert_eq!(backend.fixed_buffer_len(), 0);
    }

    #[test]
    fn uring_request_has_clean_probe_fallback() {
        let backend = SelectedDiskBackend::select(BackendRequest::Uring, 1);
        assert_eq!(backend.selection().requested, BackendRequest::Uring);
        if backend.kind() == BackendKind::Pread {
            assert!(!backend.selection().reason.is_empty());
        }
    }

    #[test]
    fn uring_probe_is_diagnostic_not_panic() {
        let probe = UringBackend::probe().unwrap();
        assert!(!probe.reason.is_empty());
        if !probe.usable {
            assert!(!probe.fixed_buffers);
            assert!(!probe.registered_files);
        }
    }

    #[tokio::test]
    async fn pwrite_then_pread_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.bin");
        std::fs::write(&path, vec![0u8; 64]).unwrap();

        let backend = PreadBackend::new(2);
        let pool = FramePool::new(1 << 20);

        let file = Arc::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap(),
        );

        backend
            .pwrite(file.clone(), bytes::Bytes::from_static(b"hello"), 8)
            .await
            .unwrap()
            .unwrap();

        let mut frame = pool.try_acquire(5).unwrap();
        frame = backend.pread(file, frame, 8).await.unwrap().unwrap();
        assert_eq!(frame.as_slice(), b"hello");
    }

    #[tokio::test]
    async fn prepared_storage_operation_runs_on_bounded_disk_worker() {
        let backend = PreadBackend::new_with_queue_depth(1, 1);
        let worker_name = backend
            .run_prepared(|| {
                Ok(std::thread::current()
                    .name()
                    .unwrap_or_default()
                    .to_string())
            })
            .await
            .unwrap();
        assert!(worker_name.starts_with("tng-disk-"));
    }

    #[tokio::test]
    async fn prepared_operation_keeps_owners_until_started_work_finishes() {
        let backend = Arc::new(PreadBackend::new_with_queue_depth(1, 1));
        let (started_tx, started_rx) = oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let dropped = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let drop_notify = Arc::new(tokio::sync::Notify::new());
        let operation_dropped = Arc::clone(&dropped);
        let operation_notify = Arc::clone(&drop_notify);

        let operation = tokio::spawn(async move {
            let probe = PreparedDropProbe {
                dropped: operation_dropped,
                notify: operation_notify,
            };
            backend
                .run_prepared(move || {
                    let _probe = probe;
                    let _ = started_tx.send(());
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .await
        });

        started_rx.await.unwrap();
        operation.abort();
        assert!(operation.await.is_err());
        assert!(!dropped.load(AtomicOrdering::Acquire));

        release_tx.send(()).unwrap();
        drop_notify.notified().await;
        assert!(dropped.load(AtomicOrdering::Acquire));
    }

    #[tokio::test]
    async fn pread_past_eof_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.bin");
        std::fs::write(&path, b"abc").unwrap();
        let backend = PreadBackend::new(1);
        let pool = FramePool::new(1 << 20);
        let file = Arc::new(File::open(&path).unwrap());
        let frame = pool.try_acquire(16).unwrap();
        let res = backend.pread(file, frame, 0).await.unwrap();
        assert!(res.is_err()); // read_exact_at past EOF
    }

    #[test]
    fn checked_io_offset_rejects_u64_overflow() {
        assert_eq!(checked_io_offset(u64::MAX - 1, 1).unwrap(), u64::MAX);
        let error = checked_io_offset(u64::MAX, 1).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn exact_read_retries_interrupted_and_positive_short_progress() {
        let expected = b"positioned short reads";
        let mut output = vec![0; expected.len()];
        let base = 37_u64;
        let mut calls = 0;

        read_exact_with(&mut output, base, |buffer, offset| {
            calls += 1;
            if calls == 1 {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let start = usize::try_from(offset - base).unwrap();
            let count = buffer.len().min(3);
            buffer[..count].copy_from_slice(&expected[start..start + count]);
            Ok(count)
        })
        .unwrap();

        assert_eq!(output.as_slice(), &expected[..]);
        assert_eq!(calls, 1 + expected.len().div_ceil(3));
    }

    #[test]
    fn write_all_retries_interrupted_and_positive_short_progress() {
        let payload = b"positioned short writes";
        let mut output = vec![0; payload.len()];
        let base = 53_u64;
        let mut calls = 0;

        write_all_with(payload, base, |buffer, offset| {
            calls += 1;
            if calls == 1 {
                return Err(io::ErrorKind::Interrupted.into());
            }
            let start = usize::try_from(offset - base).unwrap();
            let count = buffer.len().min(2);
            output[start..start + count].copy_from_slice(&buffer[..count]);
            Ok(count)
        })
        .unwrap();

        assert_eq!(output.as_slice(), &payload[..]);
        assert_eq!(calls, 1 + payload.len().div_ceil(2));
    }

    #[tokio::test]
    async fn unavailable_pread_backend_fails_closed_without_workers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("unavailable.bin");
        std::fs::write(&path, vec![0u8; 16]).unwrap();
        let backend = PreadBackend::unavailable();
        let file = Arc::new(File::open(&path).unwrap());

        let error = backend
            .pwrite(file, bytes::Bytes::from_static(b"fail"), 0)
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
    }

    #[tokio::test]
    async fn pread_backend_queue_fails_closed_when_full() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("queued.bin");
        std::fs::write(&path, vec![0u8; 64]).unwrap();
        let file = Arc::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap(),
        );
        let (tx, _rx) = mpsc::sync_channel(1);
        let backend = PreadBackend {
            tx,
            _workers: Vec::new(),
        };

        let _queued = backend.pwrite(file.clone(), bytes::Bytes::from_static(b"first"), 0);
        let rejected = backend
            .pwrite(file, bytes::Bytes::from_static(b"second"), 8)
            .await
            .unwrap()
            .unwrap_err();

        assert_eq!(rejected.kind(), io::ErrorKind::WouldBlock);
    }

    #[tokio::test]
    async fn selected_backend_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("selected.bin");
        std::fs::write(&path, vec![0u8; 64]).unwrap();

        let backend = SelectedDiskBackend::select(BackendRequest::Auto, 1);
        let pool = FramePool::new(1 << 20);
        let file = Arc::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap(),
        );

        backend
            .pwrite(file.clone(), bytes::Bytes::from_static(b"backend"), 16)
            .await
            .unwrap()
            .unwrap();
        let frame = pool.try_acquire(7).unwrap();
        let frame = backend.pread(file, frame, 16).await.unwrap().unwrap();
        assert_eq!(frame.as_slice(), b"backend");
        assert_eq!(
            backend.fixed_buffer_strategy().uses_frame_pool_slots(),
            backend.kind() == BackendKind::Uring && backend.supports_fixed_buffers()
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn forced_uring_read_only_roundtrip() {
        let probe = UringBackend::probe().unwrap();
        if !probe.usable {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uring-read-only.bin");
        std::fs::write(&path, vec![0_u8; 16 * 1024]).unwrap();
        let backend = SelectedDiskBackend::select(BackendRequest::Uring, 2);
        if backend.kind() != BackendKind::Uring {
            return;
        }
        let pool = FramePool::new(1 << 20);
        let file = Arc::new(crate::open::open_path_no_follow(&path, false, false).unwrap());
        let frame = pool.try_acquire(16 * 1024).unwrap();
        let frame = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            backend.pread(file, frame, 0),
        )
        .await
        .expect("io_uring read-only request timed out")
        .unwrap()
        .unwrap();
        assert_eq!(frame.as_slice(), vec![0_u8; 16 * 1024].as_slice());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn forced_uring_recycles_its_bounded_submission_window() {
        let probe = UringBackend::probe().unwrap();
        if !probe.usable {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uring-window.bin");
        let block_len = 4 * 1024;
        let block_count = URING_BATCH_LIMIT * 2;
        let mut data = vec![0_u8; block_len * block_count];
        for index in 0..block_count {
            let start = index * block_len;
            data[start..start + block_len].fill(index as u8);
        }
        std::fs::write(&path, data).unwrap();

        let backend = SelectedDiskBackend::select(BackendRequest::Uring, 1);
        if backend.kind() != BackendKind::Uring {
            return;
        }
        let file = Arc::new(crate::open::open_path_no_follow(&path, false, false).unwrap());
        // 4 KiB requests are backed by the pool's 16 KiB frame class.
        // Budget backing capacity, not just the logical request lengths, for
        // every frame held while this test builds the bounded submission set.
        let pool = FramePool::new((16 * 1024 * block_count) as u64);
        let reads = (0..block_count)
            .map(|index| {
                let frame = pool.try_acquire(block_len).unwrap();
                backend.pread(Arc::clone(&file), frame, (index * block_len) as u64)
            })
            .collect::<Vec<_>>();

        for (index, read) in reads.into_iter().enumerate() {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(5), read)
                .await
                .expect("io_uring window refill stalled")
                .unwrap()
                .unwrap();
            assert!(frame.as_slice().iter().all(|byte| *byte == index as u8));
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn forced_uring_roundtrip_when_kernel_supports_it() {
        let probe = UringBackend::probe().unwrap();
        if !probe.usable {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uring.bin");
        std::fs::write(&path, vec![0u8; 128]).unwrap();

        let backend = SelectedDiskBackend::select(BackendRequest::Uring, 1);
        if backend.kind() != BackendKind::Uring {
            eprintln!(
                "tng_storage_backend forced_uring_skipped reason=\"{}\"",
                backend.selection().reason
            );
            return;
        }

        let pool = FramePool::new(1 << 20);
        let file = Arc::new(
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap(),
        );

        backend
            .pwrite(file.clone(), bytes::Bytes::from_static(b"real-uring"), 32)
            .await
            .unwrap()
            .unwrap();
        backend.fdatasync(file.clone()).await.unwrap().unwrap();

        let frame = pool.try_acquire(10).unwrap();
        let frame = backend.pread(file, frame, 32).await.unwrap().unwrap();
        assert_eq!(frame.as_slice(), b"real-uring");
        assert_eq!(backend.max_batch_len(), URING_BATCH_LIMIT);
        if backend.supports_fixed_buffers() {
            assert_eq!(backend.fixed_buffer_len(), URING_FIXED_BUFFER_LEN);
            assert!(frame.is_registered_slot());
            assert_eq!(
                backend.fixed_buffer_strategy(),
                FixedBufferStrategy::FramePoolSlots
            );
        } else {
            assert_eq!(backend.fixed_buffer_len(), 0);
            assert!(!frame.is_registered_slot());
            assert_eq!(
                backend.fixed_buffer_strategy(),
                FixedBufferStrategy::Disabled
            );
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn fixed_file_identity_keeps_read_and_write_descriptors_separate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("access-mode.bin");
        std::fs::write(&path, b"data").unwrap();
        let read_file = File::open(&path).unwrap();
        let write_file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();

        assert_ne!(
            file_identity(&read_file),
            file_identity(&write_file),
            "io_uring fixed-file slots must not alias handles with different access modes"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn uring_fixed_buffer_registration_budget_stays_below_common_memlock_limit() {
        const {
            assert!(URING_FIXED_BUFFER_SLOTS < URING_BATCH_LIMIT);
            assert!(URING_FIXED_BUFFER_SLOTS * URING_FIXED_BUFFER_LEN <= 4 * 1024 * 1024);
        }
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn uring_rejected_submission_releases_new_fixed_file_slot() {
        let probe = UringBackend::probe().unwrap();
        if !probe.usable || !probe.registered_files {
            return;
        }
        let (_tx, rx) = mpsc::sync_channel(1);
        let registered_files_supported = Arc::new(AtomicBool::new(false));
        let fixed_buffers_supported = Arc::new(AtomicBool::new(false));
        let Ok(mut worker) = UringWorker::new(
            Arc::new(Mutex::new(rx)),
            registered_files_supported,
            fixed_buffers_supported,
        ) else {
            return;
        };
        if !worker.registered_files {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uring-full.bin");
        std::fs::write(&path, vec![0u8; 16]).unwrap();
        let file = Arc::new(File::open(&path).unwrap());

        let mut pushed = 0;
        while worker.push_entry(opcode::Nop::new().build()).is_ok() {
            pushed += 1;
        }
        assert!(pushed > 0);

        let (reply, response) = oneshot::channel();
        let result = worker.submit_job(Job::Sync {
            file: BackendFile::Unleased(file),
            reply,
            owner: None,
        });
        assert_eq!(
            result.unwrap_err().kind(),
            io::ErrorKind::WouldBlock,
            "test setup must leave the submission queue full"
        );
        assert_eq!(
            response.await.unwrap().unwrap_err().kind(),
            io::ErrorKind::WouldBlock,
            "a rejected io_uring submission must reach the operation caller"
        );
        assert!(worker.file_slots.is_empty());
        assert!(worker.slot_files.iter().all(Option::is_none));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn uring_completed_operations_release_shared_fixed_file_slot() {
        let probe = UringBackend::probe().unwrap();
        if !probe.usable || !probe.registered_files {
            return;
        }
        let (_tx, rx) = mpsc::sync_channel(1);
        let registered_files_supported = Arc::new(AtomicBool::new(false));
        let fixed_buffers_supported = Arc::new(AtomicBool::new(false));
        let Ok(mut worker) = UringWorker::new(
            Arc::new(Mutex::new(rx)),
            registered_files_supported,
            fixed_buffers_supported,
        ) else {
            return;
        };
        if !worker.registered_files {
            return;
        }

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uring-complete.bin");
        std::fs::write(&path, vec![0u8; 16]).unwrap();
        let file = Arc::new(File::open(&path).unwrap());
        let (reply_a, response_a) = oneshot::channel();
        let (reply_b, response_b) = oneshot::channel();

        worker
            .submit_job(Job::Sync {
                file: BackendFile::Unleased(file.clone()),
                reply: reply_a,
                owner: None,
            })
            .unwrap();
        worker
            .submit_job(Job::Sync {
                file: BackendFile::Unleased(file),
                reply: reply_b,
                owner: None,
            })
            .unwrap();
        assert_eq!(worker.file_slots.len(), 1);
        assert_eq!(worker.file_slot_refs[0], 2);

        worker.ring.submit_and_wait(2).unwrap();
        worker.complete_ready();
        assert!(response_a.await.unwrap().is_ok());
        assert!(response_b.await.unwrap().is_ok());
        assert!(worker.file_slots.is_empty());
        assert!(worker.slot_files.iter().all(Option::is_none));
        assert!(worker.file_slot_refs.iter().all(|refs| *refs == 0));
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn uring_fatal_submission_preserves_pending_io_lifetimes() {
        let probe = UringBackend::probe().unwrap();
        if !probe.usable {
            return;
        }
        let (_tx, rx) = mpsc::sync_channel(1);
        let registered_files_supported = Arc::new(AtomicBool::new(false));
        let fixed_buffers_supported = Arc::new(AtomicBool::new(false));
        let Ok(mut worker) = UringWorker::new(
            Arc::new(Mutex::new(rx)),
            registered_files_supported,
            fixed_buffers_supported,
        ) else {
            return;
        };

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uring-fatal.bin");
        std::fs::write(&path, vec![0u8; 16]).unwrap();
        let file = Arc::new(File::open(&path).unwrap());
        let frame = FramePool::new(1 << 20).try_acquire(16).unwrap();
        let (reply, response) = oneshot::channel();

        worker
            .submit_job(Job::Read {
                file: BackendFile::Unleased(file),
                frame,
                offset: 0,
                reply,
                owner: None,
            })
            .unwrap();
        assert_eq!(worker.pending.len(), 1);

        worker.fail_all(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "simulated io_uring submission failure",
        ));
        assert_eq!(worker.pending.len(), 1);
        assert_eq!(
            response.await.unwrap().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );

        // The worker must be dropped only after the ring has stopped seeing
        // the SQE's buffer and file references.
        drop(worker);
    }

    #[test]
    fn fixed_buffer_strategy_names_are_stable_for_metrics() {
        assert_eq!(FixedBufferStrategy::Disabled.as_str(), "disabled");
        assert_eq!(FixedBufferStrategy::WorkerCopy.as_str(), "worker_copy");
        assert_eq!(
            FixedBufferStrategy::FramePoolSlots.as_str(),
            "frame_pool_slots"
        );
        assert!(FixedBufferStrategy::WorkerCopy.uses_worker_copy());
        assert!(!FixedBufferStrategy::WorkerCopy.uses_frame_pool_slots());
        assert!(!FixedBufferStrategy::FramePoolSlots.uses_worker_copy());
        assert!(FixedBufferStrategy::FramePoolSlots.uses_frame_pool_slots());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn uring_strategy_reports_frame_pool_slots_only_with_registered_buffers() {
        let backend = UringBackend::try_new_with_queue_depth(1, 1);
        let Ok(backend) = backend else {
            return;
        };
        let strategy = backend.fixed_buffer_strategy();
        if backend.supports_fixed_buffers() {
            assert_eq!(strategy, FixedBufferStrategy::FramePoolSlots);
        } else {
            assert_eq!(strategy, FixedBufferStrategy::Disabled);
        }
    }
}
