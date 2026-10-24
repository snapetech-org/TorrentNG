use super::*;

const LARGE: usize = 512 * 1024 + 1;
fn pool(cap: u64) -> FramePool {
    let mut pool = FramePool::new(cap);
    pool.anonymous_large_frames = true;
    pool
}

// Scope the admission, then let ownership outlive it for the low-level tests.
fn acquire(pool: &FramePool, len: usize, overwrite: bool) -> Option<Frame> {
    let _scope = pool.verification_scope();
    pool.try_acquire_inner(len, overwrite)
}

#[test]
fn anonymous_zeroed_and_page_rounded_without_idle_retention() {
    let charge = anonymous::AnonymousBuffer::rounded_len(LARGE).unwrap() as u64;
    let pool = pool(charge);
    let mut frame = acquire(&pool, LARGE, false).unwrap();
    assert_eq!(frame.len(), LARGE);
    assert_eq!(frame.charged_bytes(), charge);
    assert!(frame.as_slice().iter().all(|&byte| byte == 0));
    frame.as_mut_slice().fill(0xAC);
    drop(frame);
    assert_eq!(pool.in_use_bytes(), 0);
    assert_eq!(pool.retained_bytes(), 0);
    assert!(pool
        .try_acquire(LARGE)
        .unwrap()
        .iter()
        .all(|&byte| byte == 0));
    // The largest pooled class remains a Vec even with the experiment enabled.
    let frame = acquire(&pool, 512 * 1024, false).unwrap();
    assert!(matches!(frame.backing, FrameBacking::Vec { .. }));
    drop(frame);
    assert_eq!(pool.retained_bytes(), 512 * 1024);
}

#[test]
fn anonymous_overwrite_completion_and_cancel_release() {
    let pool = pool(2 * 1024 * 1024);
    let mut frame = acquire(&pool, LARGE, true).unwrap();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| frame.as_slice())).is_err());
    frame.as_mut_slice().fill(0xA7);
    frame.mark_initialized();
    let bytes = frame.into_bytes();
    assert!(bytes.iter().all(|&byte| byte == 0xA7));
    drop(bytes);
    assert_eq!(pool.in_use_bytes(), 0);
    let mut cancelled = acquire(&pool, LARGE, true).unwrap();
    cancelled.as_mut_slice()[..10].fill(1);
    drop(cancelled);
    assert_eq!(pool.in_use_bytes(), 0);
    let incomplete = acquire(&pool, LARGE, true).unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| incomplete.into_bytes())).is_err()
    );
    assert_eq!(pool.in_use_bytes(), 0);
}

#[test]
fn anonymous_admission_invalid_length_and_failure_roll_back() {
    let charge = anonymous::AnonymousBuffer::rounded_len(LARGE).unwrap() as u64;
    let insufficient = pool(LARGE as u64);
    assert!(acquire(&insufficient, LARGE, false).is_none());
    assert_eq!(insufficient.in_use_bytes(), 0);
    let mut pool = pool(charge);
    pool.fail_anonymous_allocation = true;
    assert!(acquire(&pool, LARGE, false).is_none());
    assert_eq!(pool.in_use_bytes(), 0);
    assert_eq!(pool.denied_allocations(), 1);
    pool.fail_anonymous_allocation = false;
    assert!(acquire(&pool, usize::MAX, false).is_none());
    assert!(acquire(&pool, isize::MAX as usize, false).is_none());
    assert_eq!(pool.in_use_bytes(), 0);
    assert_eq!(pool.denied_allocations(), 3);
    assert!(anonymous::AnonymousBuffer::new(0).is_none());
    assert!(anonymous::AnonymousBuffer::new(LARGE).is_none());
    let frame = acquire(&pool, LARGE, false).unwrap();
    assert!(acquire(&pool, LARGE, false).is_none());
    assert_eq!(pool.in_use_bytes(), charge);
    drop(frame);
    assert_eq!(pool.in_use_bytes(), 0);
}

// Use a fresh single-test process so another test's mmap cannot reuse the
// just-unmapped address between drop and mincore. mincore never dereferences it.
#[test]
fn anonymous_last_owner_unmaps_and_registered_conversion_transfers_charge() {
    const CHILD: &str = "TNG_TEST_ANON_FRAME_LIFETIME_CHILD";
    const MARKER: &str = "anonymous-frame-lifetime-complete";
    if std::env::var_os(CHILD).is_none() {
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "frame::anonymous_tests::anonymous_last_owner_unmaps_and_registered_conversion_transfers_charge", "--nocapture", "--test-threads=1"])
            .env(CHILD, "1").output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains(MARKER));
        return;
    }
    fn mapped(ptr: *const u8) -> bool {
        let mut residency = 0u8;
        // SAFETY: mincore only queries mapping state; one page and one output
        // byte suffice. The pointer is a page-aligned mmap base, never read.
        let result = unsafe { libc::mincore(ptr.cast_mut().cast(), 1, &mut residency) };
        if result == 0 {
            true
        } else {
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::ENOMEM)
            );
            false
        }
    }
    let charge = anonymous::AnonymousBuffer::rounded_len(LARGE).unwrap() as u64;
    let pool = pool(charge);
    let mut frame = acquire(&pool, LARGE, false).unwrap();
    frame.as_mut_slice()[1..5].copy_from_slice(b"test");
    let ptr = frame.as_slice().as_ptr();
    let bytes = frame.into_bytes();
    let clone = bytes.clone();
    let slice = bytes.slice(1..5);
    drop(bytes);
    drop(clone);
    assert!(mapped(ptr));
    assert_eq!(&slice[..], b"test");
    assert_eq!(pool.in_use_bytes(), charge);
    drop(slice);
    assert!(!mapped(ptr));
    assert_eq!(pool.in_use_bytes(), 0);

    let slots = RegisteredFrameSlots::new(1, LARGE);
    let slot = slots.acquire(LARGE).unwrap();
    let frame = acquire(&pool, LARGE, false).unwrap();
    let ptr = frame.as_slice().as_ptr();
    let frame = frame.into_registered_slot(slot);
    assert!(!mapped(ptr));
    assert_eq!(pool.in_use_bytes(), charge);
    let bytes = frame.into_bytes();
    let slice = bytes.slice(..1);
    drop(bytes);
    assert!(slots.acquire(LARGE).is_none());
    assert_eq!(pool.in_use_bytes(), charge);
    drop(slice);
    assert_eq!(pool.in_use_bytes(), 0);
    assert!(slots.acquire(LARGE).is_some());
    let scope = pool.verification_scope();
    let frame = acquire(&pool, LARGE, false).unwrap();
    let ptr = frame.as_slice().as_ptr();
    drop(frame);
    assert!(mapped(ptr));
    assert_eq!(pool.in_use_bytes(), charge);
    drop(scope);
    assert!(!mapped(ptr));
    assert_eq!(pool.in_use_bytes(), 0);
    println!("{MARKER}");
}

#[test]
fn verification_scopes_reuse_charge_and_release_after_last_scope() {
    let charge = anonymous::AnonymousBuffer::rounded_len(LARGE).unwrap() as u64;
    let pool = pool(charge * 2);
    let outer = pool.verification_scope();
    let inner = pool.verification_scope();
    let mut first = acquire(&pool, LARGE, true).unwrap();
    first.as_mut_slice().fill(0xA3);
    first.mark_initialized();
    let pointer = first.as_slice().as_ptr();
    drop(first.into_bytes());
    assert_eq!(pool.in_use_bytes(), charge);
    let mut reused = acquire(&pool, LARGE, true).unwrap();
    assert_eq!(reused.as_mut_slice().as_ptr(), pointer);
    assert_eq!(reused.as_mut_slice()[0], 0xA3);
    drop(reused);
    let clean = acquire(&pool, LARGE, false).unwrap();
    assert_eq!(clean.as_slice().as_ptr(), pointer);
    assert!(clean.iter().all(|&b| b == 0));
    drop(clean);
    drop(outer);
    assert_eq!(pool.in_use_bytes(), charge);
    let live = acquire(&pool, LARGE, false).unwrap().into_bytes();
    let other = acquire(&pool, LARGE, false).unwrap();
    drop(other);
    assert_eq!(pool.in_use_bytes(), charge * 2);
    drop(inner);
    // The idle map is released; outstanding Bytes survives cancellation.
    assert_eq!(pool.in_use_bytes(), charge);
    assert_eq!(live.len(), LARGE);
    drop(live);
    assert_eq!(pool.in_use_bytes(), 0);
    assert!(pool.lock_inner().anonymous_idle.is_empty());
}

#[test]
fn verification_idle_cache_is_bounded_and_evicts_under_cap_pressure() {
    let charge = anonymous::AnonymousBuffer::rounded_len(LARGE).unwrap() as u64;
    let pool = pool(charge * 3);
    let scope = pool.verification_scope();
    let frames: Vec<_> = (0..3)
        .map(|_| acquire(&pool, LARGE, false).unwrap())
        .collect();
    drop(frames);
    assert_eq!(pool.in_use_bytes(), charge * 2);
    assert_eq!(pool.lock_inner().anonymous_idle.len(), 2);
    // A different mapping length must evict the idle cache to fit, not fail.
    let larger = acquire(&pool, charge as usize * 3, false).unwrap();
    assert_eq!(pool.in_use_bytes(), charge * 3);
    drop(larger); // >1 MiB is not eligible for idle reuse.
    assert_eq!(pool.in_use_bytes(), 0);
    drop(scope);
}

#[test]
fn verification_scope_unwind_releases_idle_buffers() {
    let pool = pool(2 * 1024 * 1024);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _scope = pool.verification_scope();
        drop(acquire(&pool, LARGE, false).unwrap());
        panic!("cancel verification via unwind");
    }))
    .is_err());
    assert_eq!(pool.in_use_bytes(), 0);
    assert!(pool.lock_inner().anonymous_idle.is_empty());
}

#[test]
fn concurrent_verification_scopes_share_one_bounded_cache() {
    let pool = pool(4 * 1024 * 1024);
    let parent_scope = pool.verification_scope();
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let pool = pool.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                let _scope = pool.verification_scope();
                let frame = acquire(&pool, LARGE, false).unwrap();
                barrier.wait(); // all four live frames must have distinct ownership
                drop(frame);
                barrier.wait();
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    let charge = anonymous::AnonymousBuffer::rounded_len(LARGE).unwrap() as u64;
    assert_eq!(pool.in_use_bytes(), 2 * charge);
    assert_eq!(pool.lock_inner().anonymous_idle.len(), 2);
    drop(parent_scope);
    assert_eq!(pool.in_use_bytes(), 0);
}

#[test]
fn anonymous_backing_requires_scope_and_old_owner_can_join_new_scope() {
    let charge = anonymous::AnonymousBuffer::rounded_len(LARGE).unwrap() as u64;
    let pool = pool(4 * charge);
    let ordinary = pool.try_acquire(LARGE).unwrap();
    assert!(matches!(ordinary.backing, FrameBacking::Vec { .. }));
    assert_eq!(ordinary.charged_bytes(), LARGE as u64);
    drop(ordinary);
    let old_scope = pool.verification_scope();
    let old = pool.try_acquire(LARGE).unwrap().into_bytes();
    drop(old_scope);
    assert_eq!(pool.in_use_bytes(), charge);
    let new_scope = pool.verification_scope();
    let second = pool.try_acquire(LARGE).unwrap();
    let third = pool.try_acquire(LARGE).unwrap();
    drop(old);
    drop(second);
    drop(third);
    assert_eq!(pool.in_use_bytes(), 2 * charge);
    assert_eq!(pool.lock_inner().anonymous_idle.len(), 2);
    drop(new_scope);
    assert_eq!(pool.in_use_bytes(), 0);
}
