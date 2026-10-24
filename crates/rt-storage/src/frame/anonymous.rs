//! Exclusively owned, zero-initialized anonymous memory; never maps a file.
use std::ptr::NonNull;

#[derive(Debug)]
pub(super) struct AnonymousBuffer {
    ptr: NonNull<u8>,
    len: usize,
}

// SAFETY: this allocation has one owner; shared access only exposes immutable
// bytes, and mutable slices require exclusive access to the owner. Ownership
// can move between workers; no thread-affine state or external mapping exists.
unsafe impl Send for AnonymousBuffer {}
unsafe impl Sync for AnonymousBuffer {}

impl AnonymousBuffer {
    pub(super) fn len(&self) -> usize {
        self.len
    }
    pub(super) fn rounded_len(len: usize) -> Option<usize> {
        // SAFETY: sysconf reads the process page-size constant.
        let page = usize::try_from(unsafe { libc::sysconf(libc::_SC_PAGESIZE) }).ok()?;
        if page == 0 || len == 0 {
            return None;
        }
        let rounded = len
            .checked_add(page - 1)?
            .checked_div(page)?
            .checked_mul(page)?;
        (rounded <= isize::MAX as usize).then_some(rounded)
    }

    pub(super) fn new(len: usize) -> Option<Self> {
        if Self::rounded_len(len)? != len {
            return None;
        }
        // SAFETY: anonymous private mapping creates len zeroed readable/writable
        // bytes owned solely by this object. No file, alias, fixed address or
        // external truncation can invalidate it. Failure is explicitly checked.
        let raw = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANONYMOUS,
                -1,
                0,
            )
        };
        if raw == libc::MAP_FAILED {
            return None;
        }
        let Some(ptr) = NonNull::new(raw.cast::<u8>()) else {
            // SAFETY: even a successful address-zero mapping must be released;
            // Rust slices require non-null pointers, so it cannot be exposed.
            unsafe {
                libc::munmap(raw, len);
            }
            return None;
        };
        Some(Self { ptr, len })
    }
    pub(super) fn as_slice(&self) -> &[u8] {
        // SAFETY: the complete allocation is zero-initialized, live, and bounded
        // by isize::MAX; shared borrowing prevents simultaneous mutable access.
        unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), self.len) }
    }
    pub(super) fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: exclusive borrow of the sole mapping owner.
        unsafe { std::slice::from_raw_parts_mut(self.ptr.as_ptr(), self.len) }
    }
}
impl Drop for AnonymousBuffer {
    fn drop(&mut self) {
        // SAFETY: exactly the original owned mapping; all slices borrow this
        // owner, and Bytes retains the owning Frame until its last reference.
        unsafe {
            libc::munmap(self.ptr.as_ptr().cast(), self.len);
        }
    }
}
