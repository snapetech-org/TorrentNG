//! Open-handle cache.
//!
//! A path-keyed cache of open file descriptors. A per-cache descriptor lease
//! bounds cached files and in-flight operations together; LRU eviction plus a
//! time-based idle sweep releases unused descriptors promptly. All storage
//! caches also draw from one shared managed-storage quota; unrelated process
//! descriptors remain outside it.
//!
//! Positioned I/O (`pread`/`pwrite`) is what makes a shared fd safe: no
//! per-op `seek`, so concurrent readers/writers do not race a file cursor.

use std::collections::{BTreeMap, HashMap};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use once_cell::sync::Lazy;

use crate::fd_limit::process_handle_cache_capacity;
use crate::file_handle::{DescriptorLimiter, DescriptorPermit, LeasedFile, LeasedFileHandle};
use crate::open::{
    identity_matches_path, normalized_storage_path, open_path_no_follow_current_identity,
    FileIdentity,
};

/// Whether the cached handle is read-only or read+write. A read-only cache
/// entry is removed when a read+write entry is published; existing operation
/// leases keep its descriptor alive until their work completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Access {
    Read,
    Write,
}

fn access_satisfies(cached: Access, requested: Access) -> bool {
    cached == requested || cached == Access::Write
}

/// An operation's shareable reference to an open file. Cloning this wrapper
/// keeps its cache's descriptor permit alive with the actual descriptor.
#[derive(Debug)]
pub struct OpenFile {
    file: Arc<LeasedFile>,
}

impl OpenFile {
    /// Borrow the file while retaining this operation handle.
    pub fn file(&self) -> &File {
        &self.file
    }

    pub(crate) fn leased_handle(&self) -> Arc<LeasedFileHandle> {
        Arc::new(LeasedFileHandle::new(Arc::clone(&self.file)))
    }
}

impl Drop for OpenFile {
    fn drop(&mut self) {
        self.file.notify_waiters();
    }
}

#[derive(Debug)]
struct Entry {
    handle: Arc<LeasedFile>,
    identity: FileIdentity,
    access: Access,
    tick: u64,
    last_used: Instant,
}

#[derive(Debug)]
struct Inner {
    map: HashMap<Arc<Path>, Entry>,
    /// LRU index: tick → shared path key. Lowest tick is least-recently-used.
    lru: BTreeMap<u64, Arc<Path>>,
    next_tick: u64,
}

#[derive(Debug, Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    evictions: AtomicU64,
    idle_closes: AtomicU64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HandleCacheStats {
    pub catalog_id: u64,
    pub capacity: usize,
    pub open_files: usize,
    pub active_descriptors: usize,
    pub memory_bytes: u64,
    pub hits: u64,
    pub misses: u64,
    pub evictions: u64,
    pub idle_closes: u64,
}

/// Bounded LRU cache of open file handles. Cheap to clone (`Arc` inside).
#[derive(Debug, Clone)]
pub struct HandleCache {
    inner: Arc<Mutex<Inner>>,
    cap: Arc<AtomicUsize>,
    idle_ttl_ns: Arc<AtomicU64>,
    descriptors: Arc<DescriptorLimiter>,
    counters: Arc<Counters>,
    epoch: Instant,
    last_sweep_ns: Arc<AtomicU64>,
    /// Zero is reserved for isolated or otherwise unidentified catalogs.
    catalog_id: u64,
}

const PROCESS_CATALOG_ID: u64 = 1;
const DEFAULT_PROCESS_HANDLE_IDLE_SECS: u64 = 30;

static PROCESS_HANDLE_CATALOG: Lazy<HandleCache> = Lazy::new(|| {
    let idle_secs = std::env::var("TNG_STORAGE_HANDLE_IDLE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(DEFAULT_PROCESS_HANDLE_IDLE_SECS);
    HandleCache::new_with_catalog_id(
        process_handle_cache_capacity(),
        Duration::from_secs(idle_secs),
        PROCESS_CATALOG_ID,
    )
});

fn duration_nanos(duration: Duration) -> u64 {
    duration.as_nanos().min(u128::from(u64::MAX)) as u64
}

impl HandleCache {
    pub fn new(cap: usize, idle_ttl: Duration) -> Self {
        Self::new_with_catalog_id(cap, idle_ttl, 0)
    }

    fn new_with_catalog_id(cap: usize, idle_ttl: Duration, catalog_id: u64) -> Self {
        HandleCache {
            inner: Arc::new(Mutex::new(Inner {
                map: HashMap::new(),
                lru: BTreeMap::new(),
                next_tick: 0,
            })),
            cap: Arc::new(AtomicUsize::new(cap)),
            idle_ttl_ns: Arc::new(AtomicU64::new(duration_nanos(idle_ttl))),
            descriptors: DescriptorLimiter::new(cap),
            counters: Arc::new(Counters::default()),
            epoch: Instant::now(),
            last_sweep_ns: Arc::new(AtomicU64::new(0)),
            catalog_id,
        }
    }

    /// The single path-keyed descriptor catalog shared by storage schedulers
    /// and the direct storage runtime in a production process.
    pub fn process_global() -> Self {
        PROCESS_HANDLE_CATALOG.clone()
    }

    pub(crate) fn process_global_with_bounds(capacity: usize, idle_ttl: Duration) -> Self {
        let cache = Self::process_global();
        cache.tighten_bounds(capacity, idle_ttl);
        cache
    }

    pub(crate) fn catalog_id(&self) -> u64 {
        self.catalog_id
    }

    pub fn capacity(&self) -> usize {
        self.cap.load(Ordering::Acquire)
    }

    pub(crate) fn idle_ttl(&self) -> Duration {
        Duration::from_nanos(self.idle_ttl_ns.load(Ordering::Acquire))
    }

    fn tighten_bounds(&self, capacity: usize, idle_ttl: Duration) {
        self.cap.fetch_min(capacity, Ordering::AcqRel);
        self.descriptors.tighten_limit(capacity);
        let requested_ttl = duration_nanos(idle_ttl);
        if requested_ttl == 0 {
            self.idle_ttl_ns.store(0, Ordering::Release);
        } else {
            let current_ttl = self.idle_ttl_ns.load(Ordering::Acquire);
            if current_ttl != 0 {
                self.idle_ttl_ns.fetch_min(requested_ttl, Ordering::AcqRel);
            }
        }
        let mut inner = self.lock_inner();
        self.evict_to_cap(&mut inner, self.capacity());
    }

    pub(crate) fn active_descriptors(&self) -> usize {
        self.descriptors.active()
    }

    pub(crate) fn stats(&self) -> HandleCacheStats {
        let inner = self.lock_inner();
        let path_bytes = inner
            .map
            .keys()
            .map(|path| path.as_os_str().as_encoded_bytes().len())
            .fold(0usize, usize::saturating_add);
        HandleCacheStats {
            catalog_id: self.catalog_id,
            capacity: self.capacity(),
            open_files: inner.map.len(),
            active_descriptors: self.descriptors.active(),
            // The map and LRU each retain an Arc path key, sharing the path
            // allocation rather than cloning the path on every LRU touch.
            memory_bytes: path_bytes.saturating_add(inner.map.len().saturating_mul(
                2 * std::mem::size_of::<Arc<Path>>()
                    + std::mem::size_of::<Entry>()
                    + std::mem::size_of::<u64>(),
            )) as u64,
            hits: self.counters.hits.load(Ordering::Relaxed),
            misses: self.counters.misses.load(Ordering::Relaxed),
            evictions: self.counters.evictions.load(Ordering::Relaxed),
            idle_closes: self.counters.idle_closes.load(Ordering::Relaxed),
        }
    }

    pub fn len(&self) -> usize {
        self.lock_inner().map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn lock_inner(&self) -> MutexGuard<'_, Inner> {
        match self.inner.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                // The map and LRU are derived cache state. In-flight I/O owns
                // a separate leased handle, and every miss reopens through
                // the no-follow path, so dropping cached entries is safe.
                let mut guard = poisoned.into_inner();
                guard.map.clear();
                guard.lru.clear();
                guard.next_tick = 0;
                self.inner.clear_poison();
                guard
            }
        }
    }

    /// Get a cached handle for `path`, opening (and inserting) one on miss.
    ///
    /// `write` selects a read+write handle (optionally `create`ing the
    /// file); otherwise a read-only handle that never creates. The open
    /// syscall runs outside the lock; a concurrent opener racing the same
    /// key is resolved by keeping whichever landed first.
    pub fn get_or_open(&self, path: &Path, write: bool, create: bool) -> io::Result<Arc<OpenFile>> {
        self.get_or_open_file(path, write, create)
            .map(Self::operation_handle)
    }

    /// Scheduler-facing operation handle. It retains the same catalog lease
    /// without allocating an intermediate public wrapper.
    pub(crate) fn get_or_open_leased(
        &self,
        path: &Path,
        write: bool,
        create: bool,
    ) -> io::Result<Arc<LeasedFileHandle>> {
        self.get_or_open_file(path, write, create)
            .map(|file| Arc::new(LeasedFileHandle::new(file)))
    }

    /// Open a leased operation descriptor through the shared catalog without
    /// retaining a cache entry on a miss. Checkpoint syncs use this path so a
    /// one-off dirty-file sync does not fill the warm-read cache.
    pub(crate) fn open_uncached_leased(
        &self,
        path: &Path,
        write: bool,
        create: bool,
    ) -> io::Result<Arc<LeasedFileHandle>> {
        let (path, access) = Self::cache_key(path, write)?;
        if let Some(file) = self.touch_for_access(&path, access) {
            return Ok(Arc::new(LeasedFileHandle::new(file)));
        }
        let permit = self.acquire_descriptor();
        if let Some(file) = self.touch_for_access(&path, access) {
            return Ok(Arc::new(LeasedFileHandle::new(file)));
        }
        let (file, identity) = open_path_no_follow_current_identity(&path, write, create)?;
        Ok(Arc::new(LeasedFileHandle::new(Arc::new(LeasedFile::new(
            file, identity, permit,
        )))))
    }

    fn get_or_open_file(
        &self,
        path: &Path,
        write: bool,
        create: bool,
    ) -> io::Result<Arc<LeasedFile>> {
        let (path, access) = Self::cache_key(path, write)?;
        if let Some(file) = self.touch_for_access(&path, access) {
            return Ok(file);
        }

        self.counters.misses.fetch_add(1, Ordering::Relaxed);
        // Reserve capacity before opening so concurrent misses cannot exceed
        // this cache's live-descriptor budget while the opens race.
        let permit = self.acquire_descriptor();
        if let Some(file) = self.touch_for_access(&path, access) {
            return Ok(file);
        }
        self.open_and_insert(path, access, create, permit)
    }

    /// Async counterpart to [`HandleCache::get_or_open`]. Descriptor
    /// backpressure awaits a notification instead of blocking an executor
    /// worker while another operation owns the final lease.
    pub async fn get_or_open_async(
        &self,
        path: &Path,
        write: bool,
        create: bool,
    ) -> io::Result<Arc<OpenFile>> {
        self.get_or_open_file_async(path, write, create)
            .await
            .map(Self::operation_handle)
    }

    async fn get_or_open_file_async(
        &self,
        path: &Path,
        write: bool,
        create: bool,
    ) -> io::Result<Arc<LeasedFile>> {
        let (path, access) = Self::cache_key(path, write)?;
        if let Some(file) = self.touch_for_access(&path, access) {
            return Ok(file);
        }

        self.counters.misses.fetch_add(1, Ordering::Relaxed);
        let permit = self.acquire_descriptor_async().await;
        if let Some(file) = self.touch_for_access(&path, access) {
            return Ok(file);
        }
        self.open_and_insert(path, access, create, permit)
    }

    fn cache_key(path: &Path, write: bool) -> io::Result<(PathBuf, Access)> {
        let path = normalized_storage_path(path)?;
        let access = if write { Access::Write } else { Access::Read };
        Ok((path, access))
    }

    fn open_and_insert(
        &self,
        path: PathBuf,
        access: Access,
        create: bool,
        permit: DescriptorPermit,
    ) -> io::Result<Arc<LeasedFile>> {
        // Miss: open without holding the cache lock. On Unix this walks from `/`
        // through directory descriptors with O_NOFOLLOW, so a replaced
        // ancestor cannot redirect peer I/O outside the already-authorized
        // path. The final component is no-follow as well.
        let (file, identity) =
            open_path_no_follow_current_identity(&path, access == Access::Write, create)?;
        let file = Arc::new(LeasedFile::new(file, identity, permit));

        loop {
            let existing = {
                let inner = self.lock_inner();
                inner.map.get(path.as_path()).and_then(|entry| {
                    access_satisfies(entry.access, access)
                        .then(|| (Arc::clone(&entry.handle), entry.identity))
                })
            };
            if let Some((existing, existing_identity)) = existing {
                // Path identity is filesystem I/O. Check it without holding
                // the catalog mutex, then verify the candidate still owns the
                // key before refreshing or removing it.
                let still_same_path =
                    identity_matches_path(&path, existing_identity).unwrap_or(false);
                let mut inner = self.lock_inner();
                let is_current = inner
                    .map
                    .get(path.as_path())
                    .is_some_and(|entry| Arc::ptr_eq(&entry.handle, &existing));
                if !is_current {
                    continue;
                }
                if still_same_path {
                    self.refresh_locked(&mut inner, &path);
                    self.counters.hits.fetch_add(1, Ordering::Relaxed);
                    return Ok(existing);
                }
                if let Some(entry) = inner.map.remove(path.as_path()) {
                    inner.lru.remove(&entry.tick);
                    self.counters.evictions.fetch_add(1, Ordering::Relaxed);
                }
                continue;
            }

            // Revalidate our own miss descriptor after any race-resolution
            // work and immediately before publication. Path I/O stays
            // outside the catalog mutex.
            #[cfg(any(unix, windows))]
            if !identity_matches_path(&path, identity)? {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "runtime storage path changed before catalog publication",
                ));
            }

            let mut inner = self.lock_inner();
            if inner
                .map
                .get(path.as_path())
                .is_some_and(|entry| access_satisfies(entry.access, access))
            {
                continue;
            }
            return Ok(self.insert_locked(&mut inner, path, Arc::clone(&file), identity, access));
        }
    }

    fn insert_locked(
        &self,
        inner: &mut Inner,
        path: PathBuf,
        file: Arc<LeasedFile>,
        identity: FileIdentity,
        access: Access,
    ) -> Arc<LeasedFile> {
        if let Some(entry) = inner.map.remove(path.as_path()) {
            inner.lru.remove(&entry.tick);
            self.counters.evictions.fetch_add(1, Ordering::Relaxed);
        }
        let key: Arc<Path> = Arc::from(path.into_boxed_path());
        let tick = inner.next_tick;
        inner.next_tick += 1;
        inner.lru.insert(tick, Arc::clone(&key));
        inner.map.insert(
            key,
            Entry {
                handle: Arc::clone(&file),
                identity,
                access,
                tick,
                last_used: Instant::now(),
            },
        );
        self.evict_to_cap(inner, self.capacity());
        file
    }

    fn operation_handle(file: Arc<LeasedFile>) -> Arc<OpenFile> {
        Arc::new(OpenFile { file })
    }

    fn touch(&self, path: &Path, access: Access) -> Option<Arc<LeasedFile>> {
        let (file, identity) = {
            let now = Instant::now();
            let mut inner = self.lock_inner();
            self.sweep_idle_if_due_locked(&mut inner, now);
            let entry = inner.map.get(path)?;
            if !access_satisfies(entry.access, access) {
                return None;
            }
            (Arc::clone(&entry.handle), entry.identity)
        };
        // Do not hold the cache mutex across a filesystem identity check; a
        // slow stat on a network mount must not serialize unrelated paths.
        let still_same_path = identity_matches_path(path, identity).unwrap_or(false);
        let mut inner = self.lock_inner();
        if !inner
            .map
            .get(path)
            .is_some_and(|entry| Arc::ptr_eq(&entry.handle, &file))
        {
            return None;
        }
        if !still_same_path {
            if let Some(entry) = inner.map.remove(path) {
                inner.lru.remove(&entry.tick);
                self.counters.evictions.fetch_add(1, Ordering::Relaxed);
            }
            return None;
        }
        self.refresh_locked(&mut inner, path);
        self.counters.hits.fetch_add(1, Ordering::Relaxed);
        Some(file)
    }

    fn refresh_locked(&self, inner: &mut Inner, path: &Path) {
        let Some(key) = inner
            .map
            .get_key_value(path)
            .map(|(key, _)| Arc::clone(key))
        else {
            return;
        };
        let new_tick = inner.next_tick;
        let Some(entry) = inner.map.get_mut(path) else {
            return;
        };
        let old_tick = entry.tick;
        entry.tick = new_tick;
        entry.last_used = Instant::now();
        inner.next_tick += 1;
        inner.lru.remove(&old_tick);
        inner.lru.insert(new_tick, key);
    }

    /// Reads may reuse a read+write descriptor; write requests only accept a
    /// read+write entry.
    fn touch_for_access(&self, path: &Path, access: Access) -> Option<Arc<LeasedFile>> {
        self.touch(path, access)
    }

    /// Close handles untouched for longer than the idle TTL. Returns the
    /// number removed from the cache; active operation leases keep their file
    /// descriptor open until backend ownership ends.
    pub fn sweep_idle(&self) -> usize {
        let now = Instant::now();
        let mut inner = self.lock_inner();
        self.sweep_idle_locked(&mut inner, now)
    }

    fn sweep_idle_if_due_locked(&self, inner: &mut Inner, now: Instant) {
        let idle_ttl = self.idle_ttl();
        if idle_ttl.is_zero() {
            return;
        }
        let interval = (idle_ttl / 4).clamp(Duration::from_millis(1), Duration::from_secs(1));
        let elapsed_ns = now
            .saturating_duration_since(self.epoch)
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let last = self.last_sweep_ns.load(Ordering::Relaxed);
        if last != 0 && elapsed_ns.saturating_sub(last) < interval.as_nanos() as u64 {
            return;
        }
        self.last_sweep_ns
            .store(elapsed_ns.max(1), Ordering::Relaxed);
        self.sweep_idle_locked(inner, now);
    }

    fn sweep_idle_locked(&self, inner: &mut Inner, now: Instant) -> usize {
        let idle_ttl = self.idle_ttl();
        if idle_ttl.is_zero() {
            return 0;
        }
        let mut closed = 0usize;
        while let Some((&tick, key)) = inner.lru.first_key_value() {
            let expired = inner
                .map
                .get(key.as_ref())
                .is_none_or(|entry| now.duration_since(entry.last_used) >= idle_ttl);
            if !expired {
                break;
            }
            let key = inner
                .lru
                .remove(&tick)
                .expect("LRU key present for iterated tick");
            if inner.map.remove(key.as_ref()).is_some() {
                closed += 1;
            }
        }
        if closed > 0 {
            self.counters
                .idle_closes
                .fetch_add(closed as u64, Ordering::Relaxed);
        }
        closed
    }

    fn evict_to_cap(&self, inner: &mut Inner, cap: usize) {
        while inner.map.len() > cap {
            if !self.evict_oldest(inner) {
                break;
            }
        }
    }

    fn evict_oldest(&self, inner: &mut Inner) -> bool {
        let Some((&tick, _)) = inner.lru.iter().next() else {
            return false;
        };
        let key = inner
            .lru
            .remove(&tick)
            .expect("lru key present for iterated tick");
        inner.map.remove(key.as_ref());
        self.counters.evictions.fetch_add(1, Ordering::Relaxed);
        true
    }

    #[cfg(test)]
    pub(crate) fn cached_write_paths(&self) -> Vec<PathBuf> {
        let candidates = {
            let inner = self.lock_inner();
            inner
                .map
                .iter()
                .filter(|(_, entry)| entry.access == Access::Write)
                .map(|(key, entry)| (Arc::clone(key), Arc::clone(&entry.handle), entry.identity))
                .collect::<Vec<_>>()
        };
        let mut paths = Vec::new();
        for (key, file, identity) in candidates {
            let still_same = identity_matches_path(&key, identity).unwrap_or(false);
            let mut inner = self.lock_inner();
            let current = inner
                .map
                .get(key.as_ref())
                .is_some_and(|entry| Arc::ptr_eq(&entry.handle, &file));
            if !current {
                continue;
            }
            if still_same {
                paths.push(key.to_path_buf());
            } else if let Some(entry) = inner.map.remove(key.as_ref()) {
                inner.lru.remove(&entry.tick);
                self.counters.evictions.fetch_add(1, Ordering::Relaxed);
            }
        }
        paths
    }

    #[cfg(test)]
    pub(crate) fn poison_for_test(&self) {
        let inner = Arc::clone(&self.inner);
        let _ = std::thread::spawn(move || {
            let _guard = inner.lock().unwrap();
            panic!("poison the derived handle catalog");
        })
        .join();
    }

    #[cfg(test)]
    pub(crate) fn cache_is_poisoned(&self) -> bool {
        self.inner.is_poisoned()
    }

    #[cfg(test)]
    pub(crate) fn sweep_marker_ns(&self) -> u64 {
        self.last_sweep_ns.load(Ordering::Relaxed)
    }

    fn acquire_descriptor(&self) -> DescriptorPermit {
        loop {
            if let Some(permit) = self.descriptors.try_acquire() {
                return permit;
            }

            let generation = self.descriptors.generation();
            {
                let mut inner = self.lock_inner();
                if let Some(permit) = self.descriptors.try_acquire() {
                    return permit;
                }
                self.sweep_idle_locked(&mut inner, Instant::now());
                // Evicting an in-flight entry may not free its descriptor
                // yet. Operation/backend handle drops wake us to evict again
                // once that file becomes idle.
                self.evict_oldest(&mut inner);
            }
            self.descriptors.wait_for_change(generation);
        }
    }

    async fn acquire_descriptor_async(&self) -> DescriptorPermit {
        loop {
            if let Some(permit) = self.descriptors.try_acquire() {
                return permit;
            }

            let generation = self.descriptors.generation();
            {
                let mut inner = self.lock_inner();
                if let Some(permit) = self.descriptors.try_acquire() {
                    return permit;
                }
                self.sweep_idle_locked(&mut inner, Instant::now());
                self.evict_oldest(&mut inner);
            }
            self.descriptors.wait_for_change_async(generation).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::sync::atomic::AtomicUsize;
    use std::sync::atomic::Ordering;

    #[test]
    fn process_global_handle_catalog_has_one_identity_and_owner_map() {
        let first = HandleCache::process_global();
        let second = HandleCache::process_global();
        assert_ne!(first.catalog_id(), 0);
        assert_eq!(first.catalog_id(), second.catalog_id());
        assert!(Arc::ptr_eq(&first.inner, &second.inner));
        assert!(Arc::ptr_eq(&first.descriptors, &second.descriptors));
    }

    fn tmp_file(dir: &Path, name: &str, content: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn reuses_same_handle_for_repeated_opens() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_file(dir.path(), "a.bin", b"data");
        let cache = HandleCache::new(16, Duration::from_secs(30));

        let h1 = cache.get_or_open(&p, false, false).unwrap();
        let h2 = cache.get_or_open(&p, false, false).unwrap();
        assert!(Arc::ptr_eq(&h1.file, &h2.file));
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.descriptors.active(), 1);
    }

    #[test]
    fn reopens_path_after_unlink_and_recreate() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_file(dir.path(), "replaced.bin", b"old");
        let cache = HandleCache::new(16, Duration::from_secs(30));

        let old = cache.get_or_open(&p, false, false).unwrap();
        std::fs::remove_file(&p).unwrap();
        std::fs::write(&p, b"new").unwrap();

        let new = cache.get_or_open(&p, false, false).unwrap();

        assert!(!Arc::ptr_eq(&old.file, &new.file));
    }

    #[test]
    fn read_only_handles_do_not_serve_writes_and_reads_reuse_writable_handle() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_file(dir.path(), "rw.bin", b"data");
        let cache = HandleCache::new(16, Duration::from_secs(30));

        let read_only = cache.get_or_open(&p, false, false).unwrap();
        let writable = cache.get_or_open(&p, true, false).unwrap();
        assert!(!Arc::ptr_eq(&read_only.file, &writable.file));
        assert_eq!(
            cache.len(),
            1,
            "the writable entry replaces the read cache entry"
        );
        assert_eq!(cache.descriptors.active(), 2);

        let read_after_write = cache.get_or_open(&p, false, false).unwrap();
        assert!(Arc::ptr_eq(&writable.file, &read_after_write.file));
        assert_eq!(cache.descriptors.active(), 2);

        drop(read_only);
        assert_eq!(cache.descriptors.active(), 1);
    }

    #[test]
    fn zero_capacity_returns_open_files_without_retaining_them() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_file(dir.path(), "uncached.bin", b"data");
        let cache = HandleCache::new(0, Duration::from_secs(30));

        let handle = cache.get_or_open(&path, false, false).unwrap();

        assert_eq!(cache.capacity(), 0);
        assert_eq!(cache.len(), 0);
        assert_eq!(handle.file().metadata().unwrap().len(), 4);
        assert_eq!(cache.descriptors.active(), 1);
        drop(handle);
        assert_eq!(cache.descriptors.active(), 0);
    }

    #[test]
    fn waits_for_an_evicted_in_flight_descriptor_lease() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = tmp_file(dir.path(), "first.bin", b"first");
        let second_path = tmp_file(dir.path(), "second.bin", b"second");
        let cache = Arc::new(HandleCache::new(1, Duration::from_secs(30)));
        let first = cache.get_or_open(&first_path, false, false).unwrap();
        assert_eq!(cache.descriptors.active(), 1);

        let worker_cache = Arc::clone(&cache);
        let (started_tx, started_rx) = std::sync::mpsc::sync_channel(0);
        let (opened_tx, opened_rx) = std::sync::mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let second = worker_cache
                .get_or_open(&second_path, false, false)
                .unwrap();
            opened_tx.send(second).unwrap();
        });

        started_rx.recv().unwrap();
        assert!(opened_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);

        let second = opened_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("waiter should open after the evicted operation releases its lease");
        assert_eq!(second.file().metadata().unwrap().len(), 6);
        assert_eq!(cache.descriptors.active(), 1);
        worker.join().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn async_descriptor_wait_does_not_block_executor_progress() {
        let dir = tempfile::tempdir().unwrap();
        let first_path = tmp_file(dir.path(), "async-first.bin", b"first");
        let second_path = tmp_file(dir.path(), "async-second.bin", b"second");
        let cache = Arc::new(HandleCache::new(1, Duration::from_secs(30)));
        let first = cache
            .get_or_open_async(&first_path, false, false)
            .await
            .unwrap();

        let waiter_cache = Arc::clone(&cache);
        let waiter = tokio::spawn(async move {
            waiter_cache
                .get_or_open_async(&second_path, false, false)
                .await
        });
        tokio::task::yield_now().await;

        let progressed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let progressed_task = Arc::clone(&progressed);
        tokio::spawn(async move {
            progressed_task.store(true, Ordering::Release);
        });
        tokio::task::yield_now().await;
        assert!(progressed.load(Ordering::Acquire));

        drop(first);
        let second = tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("async descriptor waiter should wake")
            .unwrap()
            .unwrap();
        assert_eq!(second.file().metadata().unwrap().len(), 6);
        assert_eq!(cache.descriptors.active(), 1);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn concurrent_cache_opens_stay_within_low_descriptor_limit() {
        const CHILD_ENV: &str = "TNG_TEST_HANDLE_CACHE_CONCURRENT_LOW_FD_CHILD";

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
                    let path = dir.path().join(format!("runtime-{index}.bin"));
                    std::fs::write(&path, b"payload").unwrap();
                    path
                })
                .collect::<Vec<_>>();
            let cache = Arc::new(HandleCache::new(ACTIVE_LIMIT, Duration::from_secs(30)));
            let start = Arc::new(std::sync::Barrier::new(OPERATIONS));
            let max_active = Arc::new(AtomicUsize::new(0));
            let workers = paths
                .into_iter()
                .map(|path| {
                    let cache = Arc::clone(&cache);
                    let start = Arc::clone(&start);
                    let max_active = Arc::clone(&max_active);
                    std::thread::spawn(move || {
                        start.wait();
                        let file = cache
                            .get_or_open(&path, false, false)
                            .unwrap_or_else(|error| panic!("open {path:?} failed: {error}"));
                        max_active.fetch_max(cache.descriptors.active(), Ordering::Relaxed);
                        std::thread::sleep(Duration::from_millis(30));
                        assert_eq!(file.file().metadata().unwrap().len(), 7);
                    })
                })
                .collect::<Vec<_>>();

            for worker in workers {
                worker.join().expect("handle-cache worker panicked");
            }
            assert!(max_active.load(Ordering::Relaxed) <= ACTIVE_LIMIT);
            assert!(cache.descriptors.active() <= ACTIVE_LIMIT);
            assert!(cache.len() <= ACTIVE_LIMIT);
            return;
        }

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "handle_cache::tests::concurrent_cache_opens_stay_within_low_descriptor_limit",
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
                panic!("handle-cache low-fd child timed out:\n{transcript}");
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
            "low-fd child failed or did not run the test:\n{transcript}"
        );
    }

    #[test]
    fn missing_file_read_errors_and_is_not_cached() {
        let dir = tempfile::tempdir().unwrap();
        let cache = HandleCache::new(16, Duration::from_secs(30));
        let res = cache.get_or_open(&dir.path().join("nope.bin"), false, false);
        assert_eq!(res.err().map(|e| e.kind()), Some(io::ErrorKind::NotFound));
        assert_eq!(cache.len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn final_component_symlink_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let target = tmp_file(dir.path(), "target.bin", b"secret");
        let link = dir.path().join("link.bin");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let cache = HandleCache::new(16, Duration::from_secs(30));

        let error = cache.get_or_open(&link, false, false).unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::ELOOP));
        assert!(cache.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn ancestor_symlink_is_rejected_before_opening_the_file() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(outside.path(), &alias).unwrap();
        let path = alias.join("payload.bin");
        std::fs::write(outside.path().join("payload.bin"), b"secret").unwrap();
        let cache = HandleCache::new(16, Duration::from_secs(30));

        let error = cache.get_or_open(&path, false, false).unwrap_err();
        assert!(matches!(
            error.raw_os_error(),
            Some(libc::ELOOP | libc::ENOTDIR)
        ));
        assert!(cache.is_empty());
    }

    #[test]
    fn lru_evicts_least_recently_used() {
        let dir = tempfile::tempdir().unwrap();
        let cache = HandleCache::new(2, Duration::from_secs(30));
        let a = tmp_file(dir.path(), "a", b"a");
        let b = tmp_file(dir.path(), "b", b"b");
        let c = tmp_file(dir.path(), "c", b"c");

        cache.get_or_open(&a, false, false).unwrap();
        cache.get_or_open(&b, false, false).unwrap();
        // Touch `a` so `b` becomes LRU.
        cache.get_or_open(&a, false, false).unwrap();
        cache.get_or_open(&c, false, false).unwrap(); // evicts `b`
        assert_eq!(cache.len(), 2);

        // `b` was evicted → reopening it is a fresh insert (still len 2,
        // now `a` is LRU and gets evicted).
        cache.get_or_open(&b, false, false).unwrap();
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn idle_sweep_closes_stale_handles() {
        let dir = tempfile::tempdir().unwrap();
        let p = tmp_file(dir.path(), "idle.bin", b"x");
        let cache = HandleCache::new(16, Duration::from_millis(20));
        cache.get_or_open(&p, false, false).unwrap();
        assert_eq!(cache.len(), 1);
        std::thread::sleep(Duration::from_millis(40));
        assert_eq!(cache.sweep_idle(), 1);
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn zero_idle_ttl_disables_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_file(dir.path(), "no-idle-expiry.bin", b"x");
        let cache = HandleCache::new(16, Duration::ZERO);
        let first = cache.get_or_open(&path, false, false).unwrap();

        assert_eq!(cache.sweep_idle(), 0);
        assert_eq!(cache.len(), 1);

        let second = cache.get_or_open(&path, false, false).unwrap();
        assert!(Arc::ptr_eq(&first.file, &second.file));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn poisoned_handle_cache_discards_lru_and_reopens_safely() {
        let dir = tempfile::tempdir().unwrap();
        let path = tmp_file(dir.path(), "poisoned.bin", b"data");
        let cache = HandleCache::new(16, Duration::from_secs(30));
        let in_flight = cache.get_or_open(&path, false, false).unwrap();

        let inner = Arc::clone(&cache.inner);
        assert!(std::thread::spawn(move || {
            let _guard = inner.lock().unwrap();
            panic!("poison the derived file-handle cache");
        })
        .join()
        .is_err());
        assert!(cache.inner.is_poisoned());

        assert_eq!(cache.len(), 0);
        assert!(!cache.inner.is_poisoned());
        assert_eq!(in_flight.file().metadata().unwrap().len(), 4);

        let reopened = cache.get_or_open(&path, false, false).unwrap();
        assert!(!Arc::ptr_eq(&in_flight.file, &reopened.file));
        assert_eq!(cache.len(), 1);
    }
}
