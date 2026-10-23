//! Reusable typed completion records for portable prepared reads.
//!
//! A generation accompanies every queued job. Cancelling a queued request
//! can recycle its slot immediately: the old queue entry cannot execute a
//! new request in that slot. Running work keeps ownership until it exits.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::task::{Context, Poll, Waker};
use std::time::Instant;

use crate::StorageError;

pub(crate) trait PreparedWork: Send + 'static {
    type Output: Send + 'static;
    fn execute(self) -> Result<Self::Output, StorageError>;
}

pub(crate) trait Runnable: Send + Sync {
    fn run(self: Arc<Self>, generation: u64);
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

pub(crate) struct CompletionPool<W: PreparedWork>(Arc<Pool<W>>);

struct Pool<W: PreparedWork> {
    free: Mutex<Vec<Arc<Slot<W>>>>,
    retained: usize,
    allocations: AtomicU64,
    reuses: AtomicU64,
    active: AtomicU64,
    queue_wait_ns: AtomicU64,
    run_ns: AtomicU64,
}

impl<W: PreparedWork> std::fmt::Debug for CompletionPool<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompletionPool")
            .field("retained_limit", &self.0.retained)
            .field("idle", &lock(&self.0.free).len())
            .finish()
    }
}

impl<W: PreparedWork> CompletionPool<W> {
    pub(crate) fn new(retained: usize) -> Self {
        Self(Arc::new(Pool {
            free: Mutex::new(Vec::new()),
            retained,
            allocations: AtomicU64::new(0),
            reuses: AtomicU64::new(0),
            active: AtomicU64::new(0),
            queue_wait_ns: AtomicU64::new(0),
            run_ns: AtomicU64::new(0),
        }))
    }

    pub(crate) fn stats(&self) -> [u64; 5] {
        [
            &self.0.allocations,
            &self.0.reuses,
            &self.0.active,
            &self.0.queue_wait_ns,
            &self.0.run_ns,
        ]
        .map(|value| value.load(Ordering::Relaxed))
    }

    pub(crate) fn prepare(&self, work: W) -> (Arc<dyn Runnable>, u64, Completion<W>) {
        let recycled = lock(&self.0.free).pop();
        if recycled.is_some() {
            self.0.reuses.fetch_add(1, Ordering::Relaxed);
        } else {
            self.0.allocations.fetch_add(1, Ordering::Relaxed);
        }
        let slot = recycled.unwrap_or_else(|| {
            Arc::new(Slot {
                pool: Arc::downgrade(&self.0),
                state: Mutex::new(State {
                    queued: Instant::now(),
                    generation: 0,
                    work: None,
                    result: None,
                    waker: None,
                    running: false,
                    attached: false,
                    recycled: false,
                }),
            })
        });
        let generation = {
            let mut state = lock(&slot.state);
            // Exhausted slots are never returned to the pool.
            state.generation += 1;
            state.work = Some(work);
            state.queued = Instant::now();
            state.result = None;
            state.waker = None;
            state.running = false;
            state.attached = true;
            state.recycled = false;
            state.generation
        };
        (slot.clone(), generation, Completion { slot, generation })
    }
}

struct State<W: PreparedWork> {
    queued: Instant,
    generation: u64,
    work: Option<W>,
    result: Option<Result<W::Output, StorageError>>,
    waker: Option<Waker>,
    running: bool,
    attached: bool,
    recycled: bool,
}

struct Slot<W: PreparedWork> {
    pool: Weak<Pool<W>>,
    state: Mutex<State<W>>,
}

impl<W: PreparedWork> Slot<W> {
    fn recycle(self: &Arc<Self>, generation: u64) {
        {
            let mut state = lock(&self.state);
            if state.generation != generation || state.attached || state.running || state.recycled {
                return;
            }
            state.recycled = true;
            if generation == u64::MAX {
                return;
            }
        }
        if let Some(pool) = self.pool.upgrade() {
            let mut free = lock(&pool.free);
            if free.len() < pool.retained {
                free.push(self.clone());
            }
        }
    }
}

impl<W: PreparedWork> Runnable for Slot<W> {
    fn run(self: Arc<Self>, generation: u64) {
        let started = Instant::now();
        let pool = self.pool.upgrade();
        let work = {
            let mut state = lock(&self.state);
            if state.generation != generation || !state.attached {
                return;
            }
            let Some(work) = state.work.take() else {
                return;
            };
            state.running = true;
            if let Some(pool) = &pool {
                pool.active.fetch_add(1, Ordering::Relaxed);
                pool.queue_wait_ns.fetch_add(
                    started
                        .saturating_duration_since(state.queued)
                        .as_nanos()
                        .min(u64::MAX as u128) as u64,
                    Ordering::Relaxed,
                );
            }
            work
        };
        // A malformed operation must not kill a disk worker or strand the
        // awaiting peer. Its ownership drops during unwinding.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work.execute()))
            .unwrap_or(Err(StorageError::Cancelled));
        if let Some(pool) = &pool {
            pool.active.fetch_sub(1, Ordering::Relaxed);
            pool.run_ns.fetch_add(
                started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                Ordering::Relaxed,
            );
        }
        let wake = {
            let mut state = lock(&self.state);
            debug_assert_eq!(state.generation, generation);
            state.running = false;
            if state.attached {
                state.result = Some(result);
                state.waker.take()
            } else {
                drop(state);
                drop(result);
                self.recycle(generation);
                return;
            }
        };
        if let Some(waker) = wake {
            waker.wake();
        }
    }
}

pub(crate) struct Completion<W: PreparedWork> {
    slot: Arc<Slot<W>>,
    generation: u64,
}

impl<W: PreparedWork> Future for Completion<W> {
    type Output = Result<W::Output, StorageError>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = lock(&self.slot.state);
        debug_assert_eq!(state.generation, self.generation);
        if let Some(result) = state.result.take() {
            return Poll::Ready(result);
        }
        if !state
            .waker
            .as_ref()
            .is_some_and(|w| w.will_wake(cx.waker()))
        {
            state.waker = Some(cx.waker().clone());
        }
        Poll::Pending
    }
}

impl<W: PreparedWork> Drop for Completion<W> {
    fn drop(&mut self) {
        let (work, result, waker) = {
            let mut state = lock(&self.slot.state);
            debug_assert_eq!(state.generation, self.generation);
            state.attached = false;
            (state.work.take(), state.result.take(), state.waker.take())
        };
        drop((work, result, waker));
        self.slot.recycle(self.generation);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Work(u64);
    impl PreparedWork for Work {
        type Output = u64;
        fn execute(self) -> Result<u64, StorageError> {
            Ok(self.0)
        }
    }

    #[tokio::test]
    async fn stale_queued_completion_cannot_run_recycled_work() {
        let pool = CompletionPool::new(1);
        let (old, old_generation, cancelled) = pool.prepare(Work(7));
        drop(cancelled);
        let (current, generation, completion) = pool.prepare(Work(11));
        assert!(
            Arc::ptr_eq(&old, &current),
            "the test must actually reuse the slot"
        );
        assert_ne!(old_generation, generation);
        old.run(old_generation);
        current.run(generation);
        assert_eq!(completion.await.unwrap(), 11);
        assert_eq!(lock(&pool.0.free).len(), 1);
    }

    #[tokio::test]
    async fn completion_wakes_a_parked_waiter_and_reuses_storage() {
        let pool = CompletionPool::new(1);
        let (job, generation, completion) = pool.prepare(Work(23));
        let waiter = tokio::spawn(completion);
        tokio::task::yield_now().await;
        std::thread::spawn(move || job.run(generation))
            .join()
            .unwrap();
        assert_eq!(waiter.await.unwrap().unwrap(), 23);
        assert_eq!(lock(&pool.0.free).len(), 1);
    }

    #[test]
    fn idle_retention_is_bounded() {
        let pool = CompletionPool::new(2);
        let completions: Vec<_> = (0..12).map(|v| pool.prepare(Work(v)).2).collect();
        drop(completions);
        assert_eq!(lock(&pool.0.free).len(), 2);
    }

    #[tokio::test]
    async fn cancelling_running_work_cannot_recycle_its_live_ownership() {
        struct Blocking(Arc<std::sync::Barrier>, Arc<std::sync::Barrier>);
        impl PreparedWork for Blocking {
            type Output = ();
            fn execute(self) -> Result<(), StorageError> {
                self.0.wait();
                self.1.wait();
                Ok(())
            }
        }
        let pool = CompletionPool::new(1);
        let entered = Arc::new(std::sync::Barrier::new(2));
        let release = Arc::new(std::sync::Barrier::new(2));
        let (job, generation, completion) =
            pool.prepare(Blocking(entered.clone(), release.clone()));
        let worker = std::thread::spawn(move || job.run(generation));
        entered.wait();
        drop(completion);
        assert!(lock(&pool.0.free).is_empty());
        release.wait();
        worker.join().unwrap();
        assert_eq!(lock(&pool.0.free).len(), 1);
        assert_eq!(pool.stats()[2], 0);
    }

    #[tokio::test]
    async fn panic_returns_a_terminal_completion_and_recycles() {
        struct Panic;
        impl PreparedWork for Panic {
            type Output = ();
            fn execute(self) -> Result<(), StorageError> {
                panic!("injected worker panic")
            }
        }
        let pool = CompletionPool::new(1);
        let (job, generation, completion) = pool.prepare(Panic);
        job.run(generation);
        assert!(matches!(completion.await, Err(StorageError::Cancelled)));
        assert_eq!(lock(&pool.0.free).len(), 1);
    }
}
