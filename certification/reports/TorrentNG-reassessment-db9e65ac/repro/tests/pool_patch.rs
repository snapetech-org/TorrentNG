use rt_storage::FramePool;
const LARGE:usize=512*1024;
#[test]
fn aliases_keep_charge_until_last_drop() {
    let pool=FramePool::new(LARGE as u64);
    let frame=pool.try_acquire(LARGE).unwrap();
    let data=frame.into_bytes();let alias=data.slice(..16384);drop(data);
    assert_eq!(pool.in_use_bytes(),LARGE as u64);
    assert!(pool.try_acquire(1).is_none());
    assert_eq!(pool.retained_bytes(),0);
    drop(alias);
    assert_eq!(pool.in_use_bytes(),0);
    assert_eq!(pool.retained_bytes(),LARGE as u64);
}
#[test]
fn large_idle_retention_stays_bounded_after_burst() {
    let pool=FramePool::new(4*LARGE as u64);
    let frames:Vec<_>=(0..4).map(|_|pool.try_acquire(LARGE).unwrap()).collect();
    assert!(pool.try_acquire(1).is_none());
    drop(frames);
    assert_eq!(pool.in_use_bytes(),0);
    assert_eq!(pool.retained_bytes(),2*LARGE as u64);
}
#[test]
fn generic_reuse_clears_previous_contents() {
    let pool=FramePool::new(LARGE as u64);
    let mut frame=pool.try_acquire(LARGE).unwrap();frame.as_mut_slice().fill(0xa5);drop(frame);
    let reused=pool.try_acquire(LARGE).unwrap();assert!(reused.as_slice().iter().all(|b|*b==0));
}
