//! Bounded Linux mapped upload backing, built with seed-mmap and enabled
//! through TNG_SEED_MMAP=1.
//!
//! Mappings are never exposed as Rust references. File truncation can revoke
//! pages at any time; only the kernel accesses them, through mlock
//! on a disk worker and sendmsg on the socket. Invalidated pages become I/O
//! errors instead of a userspace SIGBUS or an invalid Rust slice.

use std::collections::VecDeque;
use std::io;
use std::os::fd::{AsRawFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::file_handle::LeasedFileHandle;
use crate::open::FileIdentity;
use rt_metrics::{MemoryClass, MemoryLease, ResourceGovernor};

const WINDOW_BYTES: usize = 256 * 1024;
const MAX_CACHE_BYTES: usize = 512 * 1024;
const MAX_CACHE_ENTRIES: usize = 8;
pub const MAX_MAPPED_FRAMES: usize = 8;

pub fn enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("TNG_SEED_MMAP").is_ok_and(|v| v == "1"))
}

struct Window {
    address: usize,
    len: usize,
    offset: u64,
    _lease: Option<MemoryLease>,
    _budget: crate::scheduler::PeerReadCacheBudgetLease,
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: this is the unique owner of the mmap allocation. Every
        // outstanding syscall owns an Arc, and no Rust reference aliases it.
        unsafe {
            libc::munmap(self.address as *mut libc::c_void, self.len);
        }
    }
}

impl Window {
    fn prefault(&self, start: usize, len: usize) -> io::Result<()> {
        let page = page_size();
        let aligned = start / page * page;
        let end = start
            .checked_add(len)
            .filter(|end| *end <= self.len)
            .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))?;
        // mlock (without MLOCK_ONFAULT) populates AND locks the requested pages
        // before returning successfully. A separate MADV_POPULATE_READ would
        // duplicate that kernel walk on every prepared batch. Keep this on the
        // storage worker: population can block. Locks prevent ordinary reclaim
        // between preparation and sendmsg (external truncation can still revoke
        // backing); insufficient memlock allowance selects buffered reads.
        // SAFETY: the kernel validates the same bounded mapped range; no Rust
        // reference is created. munmap releases these locks with the window.
        if unsafe { libc::mlock((self.address + aligned) as *const _, end - aligned) } == -1 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

fn page_size() -> usize {
    // SAFETY: sysconf has no pointer arguments or process mutation.
    usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) })
        .unwrap_or(4096)
        .max(1)
}

/// A batch owns one mapping and one live descriptor identity. Cache entries
/// do not pin descriptors: normal descriptor admission remains authoritative.
pub struct MappedUploadRead {
    window: Arc<Window>,
    start: usize,
    len: usize,
    _source: Arc<LeasedFileHandle>,
}

impl MappedUploadRead {
    #[cfg(test)]
    pub(crate) fn assert_resident_and_locked(&self) {
        let page = page_size();
        let start = self.start / page * page;
        let len = self.start + self.len - start;
        let address = (self.window.address + start) as *mut libc::c_void;
        let mut residency = vec![0u8; len.div_ceil(page)];
        // SAFETY: the Arc keeps the checked mapping range live; mincore writes
        // only to the correctly sized result vector, never a payload reference.
        assert_eq!(
            unsafe { libc::mincore(address, len, residency.as_mut_ptr()) },
            0
        );
        assert!(residency.iter().all(|page| page & 1 != 0));
        // Linux rejects DONTNEED on locked pages. This verifies the requested
        // range remains locked, rather than only testing page-cache residency.
        // SAFETY: the same valid range is passed to the kernel, with no Rust load.
        for offset in (0..len).step_by(page) {
            let address = (self.window.address + start + offset) as *mut libc::c_void;
            assert_eq!(
                unsafe { libc::madvise(address, page, libc::MADV_DONTNEED) },
                -1
            );
            assert_eq!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::EINVAL)
            );
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Write frame headers plus mapped ranges without constructing a slice
    /// over mutable file-backed memory. Caller owns readiness and cursors.
    pub fn try_send_frames(
        &self,
        socket: BorrowedFd<'_>,
        frames: &[(&[u8], usize, usize)],
    ) -> io::Result<usize> {
        if frames.len() > MAX_MAPPED_FRAMES {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let mut vectors = [libc::iovec {
            iov_base: std::ptr::null_mut(),
            iov_len: 0,
        }; MAX_MAPPED_FRAMES * 2];
        let mut count = 0;
        for &(header, offset, len) in frames {
            if offset.checked_add(len).is_none_or(|end| end > self.len) {
                return Err(io::ErrorKind::InvalidInput.into());
            }
            if !header.is_empty() {
                vectors[count] = libc::iovec {
                    iov_base: header.as_ptr().cast_mut().cast(),
                    iov_len: header.len(),
                };
                count += 1;
            }
            if len != 0 {
                vectors[count] = libc::iovec {
                    iov_base: (self.window.address + self.start + offset) as *mut _,
                    iov_len: len,
                };
                count += 1;
            }
        }
        let message = libc::msghdr {
            msg_name: std::ptr::null_mut(),
            msg_namelen: 0,
            msg_iov: vectors.as_mut_ptr(),
            msg_iovlen: count,
            msg_control: std::ptr::null_mut(),
            msg_controllen: 0,
            msg_flags: 0,
        };
        // SAFETY: descriptor is borrowed for this call; all header pointers
        // remain valid, and mapped address extents were checked above. The
        // kernel copies from file pages, handling EFAULT/short writes. No
        // Rust dereference of externally mutable mapping memory occurs.
        let result = unsafe {
            libc::sendmsg(
                socket.as_raw_fd(),
                &message,
                libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
            )
        };
        if result < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(result as usize)
        }
    }
}

struct Entry {
    path: PathBuf,
    identity: FileIdentity,
    generation: u64,
    source: std::sync::Weak<crate::file_handle::LeasedFile>,
    window: Arc<Window>,
}

#[derive(Default)]
pub(crate) struct MappedUploadCache {
    entries: VecDeque<Entry>,
}

#[derive(Clone, Copy)]
pub(crate) struct MappedReadBudget<'a> {
    pub bytes: usize,
    pub entries: usize,
    pub resources: Option<&'a ResourceGovernor>,
    pub manager: &'a Arc<crate::scheduler::PeerReadCacheManager>,
}

impl std::fmt::Debug for MappedUploadCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MappedUploadCache")
            .field("bytes", &self.cached_bytes())
            .finish()
    }
}

impl MappedUploadCache {
    pub(crate) fn cached_bytes(&self) -> usize {
        self.entries.iter().map(|entry| entry.window.len).sum()
    }

    pub(crate) fn reclaim(&mut self, requested: u64) -> u64 {
        let mut released = 0;
        self.entries.retain(|entry| {
            if released < requested && Arc::strong_count(&entry.window) == 1 {
                released += entry.window.len as u64;
                false
            } else {
                true
            }
        });
        released
    }

    /// Must run on a storage worker: metadata, map and population may block.
    pub(crate) fn read(
        &mut self,
        file: Arc<LeasedFileHandle>,
        path: &Path,
        offset: u64,
        len: usize,
        generation: u64,
        budget: MappedReadBudget<'_>,
    ) -> io::Result<Option<MappedUploadRead>> {
        let MappedReadBudget {
            bytes: budget,
            entries,
            resources,
            manager,
        } = budget;
        let budget = budget.min(MAX_CACHE_BYTES);
        let entries = entries.min(MAX_CACHE_ENTRIES);
        let end = offset
            .checked_add(len as u64)
            .ok_or(io::ErrorKind::InvalidInput)?;
        let file_len = file.metadata()?.len();
        if end > file_len {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        if len == 0 || len > WINDOW_BYTES || budget < len || entries == 0 {
            return Ok(None);
        }
        let identity = file.identity();
        if let Some(index) = self.entries.iter().position(|entry| {
            entry.path == path
                && entry.identity == identity
                && entry.generation == generation
                && entry.source.upgrade().is_some()
                && entry.window.offset <= offset
                && end <= entry.window.offset + entry.window.len as u64
        }) {
            let entry = self.entries.remove(index).expect("matched cache entry");
            let start = (offset - entry.window.offset) as usize;
            match entry.window.prefault(start, len) {
                Ok(()) => {}
                Err(error) if permits_buffered_fallback(&error) => return Ok(None),
                Err(error) => return Err(error),
            }
            let read = MappedUploadRead {
                window: entry.window.clone(),
                start,
                len,
                _source: file,
            };
            self.entries.push_back(entry);
            return Ok(Some(read));
        }
        let page = page_size() as u64;
        let aligned = offset / page * page;
        let start = (offset - aligned) as usize;
        let useful_len = WINDOW_BYTES
            .min(budget)
            .min((file_len - aligned).min(usize::MAX as u64) as usize);
        // mmap and mlock consume whole pages even for a one-byte file. Charge
        // that actual extent, not logical file bytes, and separately bound
        // entry/path metadata for small-file swarms.
        let page = page as usize;
        let map_len = useful_len.div_ceil(page) * page;
        if start.checked_add(len).is_none_or(|end| end > map_len) {
            return Ok(None);
        }
        self.reclaim(
            self.cached_bytes()
                .saturating_add(map_len)
                .saturating_sub(budget) as u64,
        );
        while self.entries.len() >= entries {
            let Some(index) = self
                .entries
                .iter()
                .position(|entry| Arc::strong_count(&entry.window) == 1)
            else {
                break;
            };
            self.entries.remove(index);
        }
        if self.cached_bytes().saturating_add(map_len) > budget || self.entries.len() >= entries {
            return Ok(None);
        }
        let Some(budget_lease) = manager.reserve(map_len as u64) else {
            return Ok(None);
        };
        let lease = match resources {
            Some(resources) => match resources.try_acquire(MemoryClass::PeerBuffer, map_len as u64)
            {
                Some(lease) => Some(lease),
                None => return Ok(None),
            },
            None => None,
        };
        let file_offset =
            libc::off_t::try_from(aligned).map_err(|_| io::ErrorKind::InvalidInput)?;
        // SAFETY: checked fd/range; the resulting memory is owned by Window
        // and accessed exclusively by kernel syscalls, never by Rust loads.
        let address = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                map_len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                file_offset,
            )
        };
        if address == libc::MAP_FAILED {
            return Err(io::Error::last_os_error());
        }
        let window = Arc::new(Window {
            address: address as usize,
            len: map_len,
            offset: aligned,
            _lease: lease,
            _budget: budget_lease,
        });
        match window.prefault(start, len) {
            Ok(()) => {}
            Err(error) if permits_buffered_fallback(&error) => return Ok(None),
            Err(error) => return Err(error),
        }
        self.entries.push_back(Entry {
            path: path.to_owned(),
            identity,
            generation,
            source: file.source_weak(),
            window: window.clone(),
        });
        Ok(Some(MappedUploadRead {
            window,
            start,
            len,
            _source: file,
        }))
    }
}

fn permits_buffered_fallback(error: &io::Error) -> bool {
    matches!(
        error.raw_os_error(),
        Some(libc::EINVAL | libc::ENOMEM | libc::EPERM | libc::EAGAIN)
    )
}
